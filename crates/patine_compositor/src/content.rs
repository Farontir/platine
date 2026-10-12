//! Content cache of the procedural layers and mask effects: one texture per (layer,
//! channel) of a procedural fill and per procedural mask fill, evaluated again only when
//! its procedural, its parameters, its projection or the resolution change.
//!
//! The procedural passes read each texture set's *surface map* (see
//! [`patine_procedural::surface`]), built once per resolution, and the mesh in storage
//! buffers, uploaded once per mesh. Every texture here is discardable in the pool.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use patine_core::channel::Channel;
use patine_core::ids::{EffectId, LayerId};
use patine_core::layer::{FillSource, Layer, LayerKind, ProceduralRef, Projection};
use patine_core::mask::MaskEffect;
use patine_core::mesh::MeshAsset;
use patine_core::project::{TextureSet, TextureSetId};
use patine_gpu::pool::{Residency, TextureId, TexturePool, texture_bytes};
use patine_gpu::{GpuContext, wgpu};
use patine_procedural::def::{Output, ProceduralDef};
use patine_procedural::evaluate::{EvaluationUniforms, SurfaceInput};
use patine_procedural::library;
use patine_procedural::surface::{MeshBuffers, surface_descriptor};

use crate::compositor::{Compositor, format_of};
use crate::reference::ChannelKind;

/// What a content texture holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ContentId {
    /// Channel `1` of the procedural fill layer `0`.
    Layer(LayerId, Channel),
    /// The procedural fill effect `1` of the mask of layer `0`.
    Mask(LayerId, EffectId),
}

/// What a content texture was evaluated from.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    procedural: ProceduralRef,
    projection: Projection,
    output: u32,
    size: u32,
    /// Generation of the surface map it was evaluated on.
    map: u64,
}

#[derive(Debug)]
struct Content {
    key: Key,
    texture: TextureId,
    view: wgpu::TextureView,
    format: wgpu::TextureFormat,
}

#[derive(Debug)]
struct SurfaceMap {
    size: u32,
    texture: TextureId,
    view: wgpu::TextureView,
    /// Distinguishes the successive maps of a texture set.
    generation: u64,
}

/// One content to evaluate.
struct Wanted {
    id: ContentId,
    def: &'static ProceduralDef,
    key: Key,
    format: wgpu::TextureFormat,
}

/// The procedural contents of a project's layers and masks.
#[derive(Debug, Default)]
pub struct Contents {
    mesh: Option<(Arc<MeshAsset>, MeshBuffers)>,
    surfaces: HashMap<TextureSetId, SurfaceMap>,
    textures: HashMap<ContentId, Content>,
    /// Procedurals already reported as missing from the library.
    missing: HashSet<String>,
    /// Generation of the next surface map.
    generation: u64,
}

fn descriptor(format: wgpu::TextureFormat, size: u32) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("procedural content"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    }
}

impl Contents {
    /// The evaluated content `id`, if any.
    pub fn view(&self, id: ContentId) -> Option<&wgpu::TextureView> {
        self.textures.get(&id).map(|content| &content.view)
    }

    /// The surface map of texture set `set` (an `R32Uint` texture), if built.
    pub fn surface_map(&self, set: TextureSetId) -> Option<TextureId> {
        self.surfaces.get(&set).map(|map| map.texture)
    }

    /// Memory of the content textures and surface maps, in bytes.
    pub fn memory(&self) -> u64 {
        let contents: u64 = self
            .textures
            .values()
            .map(|c| texture_bytes(&descriptor(c.format, c.key.size)))
            .sum();
        let maps: u64 = self
            .surfaces
            .values()
            .map(|m| texture_bytes(&surface_descriptor(m.size)))
            .sum();
        contents + maps
    }

    /// Releases every texture.
    pub fn clear(&mut self, pool: &mut TexturePool) {
        for (_, content) in self.textures.drain() {
            pool.release(content.texture);
        }
        for (_, map) in self.surfaces.drain() {
            pool.release(map.texture);
        }
        self.mesh = None;
    }

    /// The definition of `procedural`, warning once when the library lacks it.
    fn find(&mut self, procedural: &ProceduralRef) -> Option<&'static ProceduralDef> {
        let def = library::find(&procedural.id);
        if def.is_none() && self.missing.insert(procedural.id.clone()) {
            tracing::warn!(id = %procedural.id, "unknown procedural: its layers do not contribute");
        }
        def
    }

    /// The contents `set` needs: each channel of its procedural fills (that the
    /// procedural produces), each procedural fill of its masks.
    fn wanted(&mut self, set: &TextureSet) -> Vec<Wanted> {
        let size = set.resolution.pixels();
        let mut layers = Vec::new();
        for layer in &set.stack.layers {
            layer.visit(&mut |layer: &Layer| layers.push(layer.clone()));
        }
        let mut wanted = Vec::new();
        for layer in &layers {
            if let LayerKind::Fill {
                source: FillSource::Procedural(procedural),
                projection,
            } = &layer.kind
                && let Some(def) = self.find(procedural)
            {
                for channel in layer.channels.keys() {
                    let output = Output::Channel(*channel);
                    if !def.outputs.contains(&output) {
                        continue;
                    }
                    let Some(index) = output.index() else {
                        continue;
                    };
                    wanted.push(Wanted {
                        id: ContentId::Layer(layer.id, *channel),
                        def,
                        key: Key {
                            procedural: procedural.clone(),
                            projection: *projection,
                            output: index,
                            size,
                            map: 0,
                        },
                        format: format_of(ChannelKind::of(*channel)),
                    });
                }
            }
            for entry in layer.mask.iter().flat_map(|mask| &mask.effects) {
                if let MaskEffect::Fill {
                    source: FillSource::Procedural(procedural),
                } = &entry.effect
                    && let Some(def) = self.find(procedural)
                    && def.outputs.contains(&Output::Gray)
                {
                    wanted.push(Wanted {
                        id: ContentId::Mask(layer.id, entry.id),
                        def,
                        key: Key {
                            procedural: procedural.clone(),
                            projection: Projection::default(),
                            output: 0,
                            size,
                            map: 0,
                        },
                        format: wgpu::TextureFormat::R16Float,
                    });
                }
            }
        }
        wanted
    }

    /// Brings the contents of `set` in line with it, evaluating what changed. Adds the
    /// contents in use to `used`.
    #[allow(clippy::too_many_arguments)]
    pub fn sync_set(
        &mut self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        compositor: &Compositor,
        encoder: &mut wgpu::CommandEncoder,
        mesh: &Arc<MeshAsset>,
        set: &TextureSet,
        used: &mut HashSet<ContentId>,
    ) {
        let mut wanted = self.wanted(set);
        if wanted.is_empty() {
            // No procedural content: the surface map is not needed any more.
            if let Some(map) = self.surfaces.remove(&set.id) {
                pool.release(map.texture);
            }
            return;
        }
        // The mesh: a new one invalidates every surface map and content.
        if !self
            .mesh
            .as_ref()
            .is_some_and(|(current, _)| Arc::ptr_eq(current, mesh))
        {
            self.clear(pool);
            self.mesh = Some((mesh.clone(), MeshBuffers::new(ctx, mesh)));
        }
        let size = set.resolution.pixels();
        let map_alive = self
            .surfaces
            .get(&set.id)
            .is_some_and(|map| map.size == size && pool.get(map.texture).is_some());
        if !map_alive {
            if let Some(old) = self.surfaces.remove(&set.id) {
                pool.release(old.texture);
            }
            let texture = pool.allocate(ctx, &surface_descriptor(size), Residency::Discardable);
            let gpu = pool
                .peek(texture)
                .expect("a texture that was just allocated");
            let buffers = &self.mesh.as_ref().expect("uploaded above").1;
            compositor
                .surfaces()
                .build(ctx, encoder, buffers, set.material_index, gpu);
            let view = gpu.create_view(&Default::default());
            self.generation += 1;
            self.surfaces.insert(
                set.id,
                SurfaceMap {
                    size,
                    texture,
                    view,
                    generation: self.generation,
                },
            );
        }
        let generation = self.surfaces[&set.id].generation;
        for item in &mut wanted {
            item.key.map = generation;
        }
        for item in wanted {
            used.insert(item.id);
            let current = self.textures.remove(&item.id);
            let mut content = match current {
                Some(content)
                    if content.format == item.format
                        && content.key.size == item.key.size
                        && pool.get(content.texture).is_some() =>
                {
                    if content.key == item.key {
                        self.textures.insert(item.id, content);
                        continue;
                    }
                    content
                }
                other => {
                    if let Some(old) = other {
                        pool.release(old.texture);
                    }
                    let texture = pool.allocate(
                        ctx,
                        &descriptor(item.format, item.key.size),
                        Residency::Discardable,
                    );
                    let view = pool
                        .peek(texture)
                        .expect("a texture that was just allocated")
                        .create_view(&Default::default());
                    Content {
                        key: item.key.clone(),
                        texture,
                        view,
                        format: item.format,
                    }
                }
            };
            content.key = item.key.clone();
            let (mesh, map) = (
                &self.mesh.as_ref().expect("uploaded above").1,
                &self.surfaces[&set.id],
            );
            let uniforms = EvaluationUniforms::new(
                &item.key.projection,
                size,
                mesh.center,
                mesh.extent,
                false,
                item.key.output,
            );
            let result = compositor.procedurals().evaluate(
                ctx,
                encoder,
                item.def,
                &item.def.resolve(&item.key.procedural),
                &uniforms,
                SurfaceInput::Mesh {
                    mesh,
                    material: set.material_index,
                    map: &map.view,
                },
                &content.view,
                item.format,
            );
            if let Err(error) = result {
                tracing::error!(id = %item.def.id, %error, "procedural evaluation failed");
                pool.release(content.texture);
                continue;
            }
            self.textures.insert(item.id, content);
        }
    }

    /// Releases the contents not in `used` and the surface maps of texture sets that no
    /// longer exist.
    pub fn release_unused(
        &mut self,
        pool: &mut TexturePool,
        used: &HashSet<ContentId>,
        sets: &[TextureSetId],
    ) {
        self.textures.retain(|id, content| {
            let keep = used.contains(id);
            if !keep {
                pool.release(content.texture);
            }
            keep
        });
        self.surfaces.retain(|id, map| {
            let keep = sets.contains(id);
            if !keep {
                pool.release(map.texture);
            }
            keep
        });
    }
}
