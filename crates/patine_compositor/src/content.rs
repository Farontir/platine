//! Content cache of the procedural layers and mask effects: one texture per output of a
//! procedural fill (all its channels, written by one pass) and per procedural mask fill,
//! evaluated again only when its procedural, its parameters, its projection or the
//! resolution change.
//!
//! The procedural passes read each texture set's *surface map* (see
//! [`patine_procedural::surface`]), built once per resolution, and the mesh in storage
//! buffers, uploaded once per mesh. Every texture here is discardable in the pool.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use patine_core::channel::Channel;
use patine_core::ids::{EffectId, LayerId};
use patine_core::layer::{FillSource, Layer, LayerKind, ProceduralRef, Projection};
use patine_core::mask::{MASK_VALUE_CHANNEL, MaskEffect};
use patine_core::mesh::MeshAsset;
use patine_core::project::{TextureSet, TextureSetId};
use patine_gpu::pool::{Residency, TextureId, TexturePool, texture_bytes};
use patine_gpu::{GpuContext, wgpu};
use patine_procedural::def::{Output, ProceduralDef};
use patine_procedural::evaluate::{EvaluationUniforms, SurfaceInput, output_formats};
use patine_procedural::library;
use patine_procedural::surface::{MeshBuffers, surface_descriptor};

use crate::compositor::Compositor;

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

/// One evaluation: every output of a procedural, one content each.
struct Job {
    /// Content of each output, in the order of `def.outputs`.
    ids: Vec<ContentId>,
    /// Format of each output.
    formats: Vec<wgpu::TextureFormat>,
    def: &'static ProceduralDef,
    key: Key,
}

/// Brush materials kept evaluated.
pub const BRUSH_MATERIALS: usize = 4;

/// What a brush material was evaluated for.
#[derive(Clone, Debug, PartialEq)]
struct BrushKey {
    set: TextureSetId,
    key: Key,
}

/// A procedural material evaluated for brushes: one texture per output.
#[derive(Debug)]
struct BrushMaterial {
    key: BrushKey,
    textures: Vec<(Channel, TextureId, wgpu::TextureView)>,
}

/// The procedural contents of a project's layers and masks.
#[derive(Debug, Default)]
pub struct Contents {
    /// Materials brushes paint with, the most recently used last.
    brushes: Vec<BrushMaterial>,
    mesh: Option<(Arc<MeshAsset>, Arc<MeshBuffers>)>,
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

    /// The mesh in storage buffers and the surface map of `set`, if built.
    pub fn surface(&self, set: TextureSetId) -> Option<(Arc<MeshBuffers>, wgpu::TextureView)> {
        Some((
            self.mesh.as_ref()?.1.clone(),
            self.surfaces.get(&set)?.view.clone(),
        ))
    }

    /// The procedural material `procedural` evaluated on texture set `set` with
    /// `projection`, for a brush painting it: one texture per output, by channel (a gray
    /// output for masks under [`MASK_VALUE_CHANNEL`]). Evaluated once, then kept while
    /// memory allows (the last [`BRUSH_MATERIALS`] used). `None` when the library lacks
    /// the procedural or the set has no surface map yet.
    #[allow(clippy::too_many_arguments)]
    pub fn brush_material(
        &mut self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        compositor: &Compositor,
        encoder: &mut wgpu::CommandEncoder,
        set: &TextureSet,
        procedural: &ProceduralRef,
        projection: &Projection,
    ) -> Option<Vec<(Channel, wgpu::TextureView)>> {
        let def = self.find(procedural)?;
        let size = set.resolution.pixels();
        let map = self.surfaces.get(&set.id)?;
        let key = BrushKey {
            set: set.id,
            key: Key {
                procedural: procedural.clone(),
                projection: *projection,
                size,
                map: map.generation,
            },
        };
        let channel_of = |output: &Output| match output {
            Output::Channel(channel) => *channel,
            Output::Gray => MASK_VALUE_CHANNEL,
        };
        if let Some(index) = self.brushes.iter().position(|brush| brush.key == key) {
            let brush = self.brushes.remove(index);
            if brush
                .textures
                .iter()
                .all(|(_, id, _)| pool.get(*id).is_some())
            {
                let views = brush
                    .textures
                    .iter()
                    .map(|(channel, _, view)| (*channel, view.clone()))
                    .collect();
                self.brushes.push(brush);
                return Some(views);
            }
            for (_, id, _) in brush.textures {
                pool.release(id);
            }
        }
        let mesh = self.mesh.as_ref()?.1.clone();
        let map_view = map.view.clone();
        let formats = output_formats(def);
        let textures: Vec<(Channel, TextureId, wgpu::TextureView)> = def
            .outputs
            .iter()
            .zip(&formats)
            .map(|(output, format)| {
                let id = pool.allocate(ctx, &descriptor(*format, size), Residency::Discardable);
                let view = pool
                    .peek(id)
                    .expect("a texture that was just allocated")
                    .create_view(&Default::default());
                (channel_of(output), id, view)
            })
            .collect();
        let targets: Vec<(&wgpu::TextureView, wgpu::TextureFormat)> = textures
            .iter()
            .zip(&formats)
            .map(|((_, _, view), format)| (view, *format))
            .collect();
        let uniforms = EvaluationUniforms::new(projection, size, mesh.center, mesh.extent, false);
        let result = compositor.procedurals().evaluate(
            ctx,
            encoder,
            def,
            &def.resolve(procedural),
            &uniforms,
            SurfaceInput::Mesh {
                mesh: &mesh,
                material: set.material_index,
                map: &map_view,
            },
            &targets,
        );
        if let Err(error) = result {
            tracing::error!(id = %def.id, %error, "brush material evaluation failed");
            for (_, id, _) in textures {
                pool.release(id);
            }
            return None;
        }
        let views = textures
            .iter()
            .map(|(channel, _, view)| (*channel, view.clone()))
            .collect();
        self.brushes.push(BrushMaterial { key, textures });
        if self.brushes.len() > BRUSH_MATERIALS {
            for (_, id, _) in self.brushes.remove(0).textures {
                pool.release(id);
            }
        }
        Some(views)
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
        for brush in self.brushes.drain(..) {
            for (_, id, _) in brush.textures {
                pool.release(id);
            }
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

    /// The evaluations `set` needs: every procedural fill (all the channels its material
    /// produces, whether the layer uses them or not: one pass writes them all) and every
    /// procedural fill of its masks.
    fn jobs(&mut self, set: &TextureSet) -> Vec<Job> {
        let size = set.resolution.pixels();
        let mut layers = Vec::new();
        for layer in &set.stack.layers {
            layer.visit(&mut |layer: &Layer| layers.push(layer.clone()));
        }
        let mut jobs = Vec::new();
        for layer in &layers {
            if let LayerKind::Fill {
                source: FillSource::Procedural(procedural),
                projection,
            } = &layer.kind
                && let Some(def) = self.find(procedural)
            {
                let channels: Option<Vec<Channel>> = def
                    .outputs
                    .iter()
                    .map(|output| match output {
                        Output::Channel(channel) => Some(*channel),
                        Output::Gray => None,
                    })
                    .collect();
                if let Some(channels) = channels {
                    jobs.push(Job {
                        ids: channels
                            .iter()
                            .map(|channel| ContentId::Layer(layer.id, *channel))
                            .collect(),
                        formats: output_formats(def),
                        def,
                        key: Key {
                            procedural: procedural.clone(),
                            projection: *projection,
                            size,
                            map: 0,
                        },
                    });
                }
            }
            for entry in layer.mask.iter().flat_map(|mask| &mask.effects) {
                if let MaskEffect::Fill {
                    source: FillSource::Procedural(procedural),
                } = &entry.effect
                    && let Some(def) = self.find(procedural)
                    && def.outputs == [Output::Gray]
                {
                    jobs.push(Job {
                        ids: vec![ContentId::Mask(layer.id, entry.id)],
                        formats: vec![wgpu::TextureFormat::R16Float],
                        def,
                        key: Key {
                            procedural: procedural.clone(),
                            projection: Projection::default(),
                            size,
                            map: 0,
                        },
                    });
                }
            }
        }
        jobs
    }

    /// Brings the contents of `set` in line with it, evaluating what changed. Adds the
    /// contents in use to `used`. The surface map is kept when the set has procedural
    /// content or when `needs_surface` (it has paint).
    #[allow(clippy::too_many_arguments)]
    pub fn sync_set(
        &mut self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        compositor: &Compositor,
        encoder: &mut wgpu::CommandEncoder,
        mesh: &Arc<MeshAsset>,
        set: &TextureSet,
        needs_surface: bool,
        used: &mut HashSet<ContentId>,
    ) {
        let mut jobs = self.jobs(set);
        if jobs.is_empty() && !needs_surface {
            // No procedural content nor paint: the surface map is not needed any more.
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
            self.mesh = Some((mesh.clone(), Arc::new(MeshBuffers::new(ctx, mesh))));
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
        for job in &mut jobs {
            job.key.map = generation;
        }
        for job in jobs {
            used.extend(job.ids.iter().copied());
            let current = job.ids.iter().zip(&job.formats).all(|(id, format)| {
                self.textures.get(id).is_some_and(|content| {
                    content.key == job.key
                        && content.format == *format
                        && pool.get(content.texture).is_some()
                })
            });
            if current {
                continue;
            }
            // Reuse the textures of the right size and format, allocate the others.
            let mut contents = Vec::with_capacity(job.ids.len());
            for (id, format) in job.ids.iter().zip(&job.formats) {
                let content = match self.textures.remove(id) {
                    Some(content)
                        if content.format == *format
                            && content.key.size == job.key.size
                            && pool.get(content.texture).is_some() =>
                    {
                        content
                    }
                    other => {
                        if let Some(old) = other {
                            pool.release(old.texture);
                        }
                        let texture = pool.allocate(
                            ctx,
                            &descriptor(*format, job.key.size),
                            Residency::Discardable,
                        );
                        let view = pool
                            .peek(texture)
                            .expect("a texture that was just allocated")
                            .create_view(&Default::default());
                        Content {
                            key: job.key.clone(),
                            texture,
                            view,
                            format: *format,
                        }
                    }
                };
                contents.push(content);
            }
            let (mesh, map) = (
                &self.mesh.as_ref().expect("uploaded above").1,
                &self.surfaces[&set.id],
            );
            let uniforms =
                EvaluationUniforms::new(&job.key.projection, size, mesh.center, mesh.extent, false);
            let targets: Vec<(&wgpu::TextureView, wgpu::TextureFormat)> = contents
                .iter()
                .map(|content| (&content.view, content.format))
                .collect();
            let result = compositor.procedurals().evaluate(
                ctx,
                encoder,
                job.def,
                &job.def.resolve(&job.key.procedural),
                &uniforms,
                SurfaceInput::Mesh {
                    mesh,
                    material: set.material_index,
                    map: &map.view,
                },
                &targets,
            );
            if let Err(error) = result {
                tracing::error!(id = %job.def.id, %error, "procedural evaluation failed");
                for content in contents {
                    pool.release(content.texture);
                }
                continue;
            }
            for (id, mut content) in job.ids.into_iter().zip(contents) {
                content.key = job.key.clone();
                self.textures.insert(id, content);
            }
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
        // Brush materials of a surface map that changed or went away.
        let generations: Vec<(TextureSetId, u64)> = self
            .surfaces
            .iter()
            .map(|(id, map)| (*id, map.generation))
            .collect();
        self.brushes.retain(|brush| {
            let keep = generations.contains(&(brush.key.set, brush.key.key.map));
            if !keep {
                for (_, id, _) in &brush.textures {
                    pool.release(*id);
                }
            }
            keep
        });
    }
}
