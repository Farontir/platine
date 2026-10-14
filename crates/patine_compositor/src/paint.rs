//! Paint contents of the paint layers and paint mask effects: one texture per painted
//! channel (a single scalar for a mask), holding the strokes of the log rasterized.
//!
//! The rasters follow the stroke logs by pointer. At each sync, the strokes the log
//! shares with the raster (its prefix) stay; the raster's other strokes were undone, and
//! their saved tiles bring back the content from before them; the log's other strokes
//! are new (or redone) and are replayed, saving their tiles first so that they can be
//! undone in turn. When saved tiles are missing (evicted, or never saved), the raster is
//! replayed from scratch. A stroke painted live arrives already rasterized
//! ([`PaintRasters::adopt`]).
//!
//! Contents are pinned: only a full replay can rebuild them. Saved tiles are
//! discardable.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use patine_core::channel::Channel;
use patine_core::commands::PaintTarget;
use patine_core::layer::{FillSource, Layer, LayerKind};
use patine_core::mask::{MASK_VALUE_CHANNEL, MaskEffect};
use patine_core::mesh::MeshAsset;
use patine_core::project::{Project, TextureSet, TextureSetId};
use patine_core::resource::{Resource, ResourceHash};
use patine_core::stroke::{Stroke, StrokeLog};
use patine_gpu::pool::{Residency, TextureId, TexturePool, texture_bytes};
use patine_gpu::{GpuContext, wgpu};
use patine_paint::alphas::{self, AlphaImage};
use patine_paint::projection::{GpuDab, SetGeometry, StrokeProjection, gpu_dab};
use patine_paint::raster::{Content, Painter, SetSurface, TileSnapshot, content_descriptor};
use patine_paint::reference::ContentLayout;

use crate::compositor::Region;
use crate::reference::ChannelKind;

/// Layout of the paint content of `channel`.
pub fn layout_of(channel: Channel) -> ContentLayout {
    match ChannelKind::of(channel) {
        ChannelKind::Scalar => ContentLayout::Scalar,
        _ => ContentLayout::Vector,
    }
}

/// The value `stroke` deposits on `channel` of `target`: its material's value for a
/// layer, its first value (gray) for a mask; `None` when it does not paint there.
pub fn stroke_value(stroke: &Stroke, target: PaintTarget, channel: Channel) -> Option<[f32; 4]> {
    let FillSource::Uniform(values) = &stroke.brush.material else {
        return None;
    };
    match target {
        PaintTarget::Layer(_) => values.get(&channel).copied(),
        PaintTarget::Mask(..) => values
            .get(&MASK_VALUE_CHANNEL)
            .or_else(|| values.values().next())
            .copied(),
    }
}

/// One paint content texture.
#[derive(Debug)]
pub(crate) struct PaintContent {
    pub(crate) texture: TextureId,
    pub(crate) view: wgpu::TextureView,
    pub(crate) layout: ContentLayout,
}

/// Tiles from before one stroke, per content channel.
#[derive(Debug)]
pub(crate) struct StrokeUndo {
    pub(crate) tiles: Vec<[u32; 2]>,
    pub(crate) before: Vec<(Channel, TileSnapshot)>,
}

impl StrokeUndo {
    fn alive(&self, pool: &TexturePool) -> bool {
        self.before.iter().all(|(_, snapshot)| snapshot.alive(pool))
    }

    fn release(self, pool: &mut TexturePool) {
        for (_, snapshot) in self.before {
            snapshot.release(pool);
        }
    }
}

/// The rasterized strokes of one paint layer or paint mask effect.
#[derive(Debug)]
pub(crate) struct Raster {
    pub(crate) set: TextureSetId,
    pub(crate) size: u32,
    /// Strokes rasterized, in order.
    pub(crate) strokes: Vec<Arc<Stroke>>,
    pub(crate) contents: BTreeMap<Channel, PaintContent>,
    /// Tiles from before each stroke that can be undone without a replay, by stroke.
    pub(crate) undo: HashMap<*const Stroke, StrokeUndo>,
}

impl Raster {
    fn release(self, pool: &mut TexturePool) {
        for (_, content) in self.contents {
            pool.release(content.texture);
        }
        for (_, undo) in self.undo {
            undo.release(pool);
        }
    }
}

/// Most strokes a sync replays while saving their tiles for undo: beyond, it is a load
/// or a replay from scratch, where saving would only churn memory.
const SAVED_REPLAYS: usize = 8;

/// The paint contents of a project.
#[derive(Debug, Default)]
pub struct PaintRasters {
    pub(crate) rasters: HashMap<PaintTarget, Raster>,
    /// The CPU triangles of each texture set, for the mesh they were built from.
    geometry: HashMap<TextureSetId, (Arc<MeshAsset>, u32, Arc<SetGeometry>)>,
    /// Decoded brush alphas.
    alphas: HashMap<ResourceHash, Option<AlphaImage>>,
    /// Strokes replayed since creation (tests of the undo tiles).
    replays: u64,
}

/// Every paint layer and paint mask effect of `layers`, with the channels they may
/// paint and their stroke logs.
pub(crate) fn paint_targets(layers: &[Layer]) -> Vec<(PaintTarget, Vec<Channel>, Arc<StrokeLog>)> {
    let mut found = Vec::new();
    for layer in layers {
        layer.visit(&mut |layer| {
            if let LayerKind::Paint { strokes } = &layer.kind {
                let channels = layer.channels.keys().copied().collect();
                found.push((PaintTarget::Layer(layer.id), channels, strokes.clone()));
            }
            for entry in layer.mask.iter().flat_map(|mask| &mask.effects) {
                if let MaskEffect::Paint { strokes } = &entry.effect {
                    found.push((
                        PaintTarget::Mask(layer.id, entry.id),
                        vec![MASK_VALUE_CHANNEL],
                        strokes.clone(),
                    ));
                }
            }
        });
    }
    found
}

/// A content texture of `layout` at `size`², cleared to 0 (no coverage).
fn new_content(
    ctx: &GpuContext,
    pool: &mut TexturePool,
    encoder: &mut wgpu::CommandEncoder,
    layout: ContentLayout,
    size: u32,
) -> PaintContent {
    let texture = pool.allocate(ctx, &content_descriptor(layout, size), Residency::Pinned);
    let view = pool
        .peek(texture)
        .expect("a texture that was just allocated")
        .create_view(&Default::default());
    clear(encoder, &view);
    PaintContent {
        texture,
        view,
        layout,
    }
}

fn clear(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("paint content clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

impl PaintRasters {
    /// The content of `channel` of `target`, if painted.
    pub fn view(&self, target: PaintTarget, channel: Channel) -> Option<&wgpu::TextureView> {
        self.rasters
            .get(&target)?
            .contents
            .get(&channel)
            .map(|content| &content.view)
    }

    /// The content texture of `channel` of `target`, if painted.
    pub fn texture(&self, target: PaintTarget, channel: Channel) -> Option<TextureId> {
        Some(self.rasters.get(&target)?.contents.get(&channel)?.texture)
    }

    /// Strokes replayed since creation.
    pub fn replays(&self) -> u64 {
        self.replays
    }

    /// Memory of the contents and saved tiles, in bytes.
    pub fn memory(&self) -> u64 {
        self.rasters
            .values()
            .flat_map(|raster| {
                raster
                    .contents
                    .values()
                    .map(|content| texture_bytes(&content_descriptor(content.layout, raster.size)))
            })
            .sum()
    }

    /// Whether texture set `set` has paint targets (it then needs a surface map).
    pub fn needs_surface(set: &TextureSet) -> bool {
        !paint_targets(&set.stack.layers).is_empty()
    }

    /// The CPU triangles of `set`.
    pub(crate) fn geometry(&mut self, mesh: &Arc<MeshAsset>, set: &TextureSet) -> Arc<SetGeometry> {
        let current = self.geometry.get(&set.id).is_some_and(|(m, material, _)| {
            Arc::ptr_eq(m, mesh) && *material == set.material_index
        });
        if !current {
            self.geometry.insert(
                set.id,
                (
                    mesh.clone(),
                    set.material_index,
                    Arc::new(SetGeometry::new(mesh, set.material_index)),
                ),
            );
        }
        self.geometry[&set.id].2.clone()
    }

    /// Decodes and keeps `resource` as the alpha `hash` (an alpha a stroke uses before
    /// its command brings it into the project).
    pub(crate) fn provide_alpha(&mut self, hash: ResourceHash, resource: &Resource) {
        self.alphas
            .entry(hash)
            .or_insert_with(|| Self::decode(hash, resource));
    }

    fn decode(hash: ResourceHash, resource: &Resource) -> Option<AlphaImage> {
        match alphas::decode(&resource.bytes) {
            Ok(image) => Some(image),
            Err(error) => {
                tracing::warn!(%hash, %error, "brush alpha unreadable: round dabs instead");
                None
            }
        }
    }

    /// The decoded alpha of `stroke`, from the project's resources (or provided); `None`
    /// for a round dab, or for an alpha that is missing (with a warning) or unreadable.
    pub(crate) fn alpha(&mut self, project: &Project, stroke: &Stroke) -> Option<AlphaImage> {
        let hash = stroke.brush.alpha?;
        if let Some(decoded) = self.alphas.get(&hash) {
            return decoded.clone();
        }
        let Some(resource) = project.resources.get(&hash) else {
            tracing::warn!(%hash, "brush alpha missing from the project: round dabs instead");
            return None;
        };
        let decoded = Self::decode(hash, resource);
        self.alphas.insert(hash, decoded.clone());
        decoded
    }

    /// Makes sure `target` has a content for each of `channels` (allocated empty).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ensure_contents(
        &mut self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        encoder: &mut wgpu::CommandEncoder,
        target: PaintTarget,
        set: TextureSetId,
        size: u32,
        channels: &[Channel],
    ) {
        let raster = self.rasters.entry(target).or_insert_with(|| Raster {
            set,
            size,
            strokes: Vec::new(),
            contents: BTreeMap::new(),
            undo: HashMap::new(),
        });
        for channel in channels {
            raster
                .contents
                .entry(*channel)
                .or_insert_with(|| new_content(ctx, pool, encoder, layout_of(*channel), size));
        }
    }

    /// Registers `stroke`, already rasterized live into `target` with the tiles from
    /// before it, as the raster's last stroke.
    pub(crate) fn adopt(&mut self, target: PaintTarget, stroke: Arc<Stroke>, undo: StrokeUndo) {
        if let Some(raster) = self.rasters.get_mut(&target) {
            raster.undo.insert(Arc::as_ptr(&stroke), undo);
            raster.strokes.push(stroke);
        }
    }

    /// Brings the rasters of `set` in line with its stroke logs. Adds the targets in use
    /// to `used`. Returns the tiles whose paint changed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sync_set(
        &mut self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        painter: &Painter,
        encoder: &mut wgpu::CommandEncoder,
        project: &Project,
        set: &TextureSet,
        surface: Option<(&patine_procedural::surface::MeshBuffers, &wgpu::TextureView)>,
        used: &mut HashSet<PaintTarget>,
    ) -> Region {
        let size = set.resolution.pixels();
        let mut dirty = Region::empty(size);
        let targets = paint_targets(&set.stack.layers);
        let Some((mesh, map)) = surface else {
            return dirty;
        };
        let set_channels: Vec<Channel> = set.channels.iter().map(|def| def.channel).collect();
        // Shared, so that rasters can be changed while it is borrowed.
        let geometry = self.geometry(&project.mesh, set);
        let surface = SetSurface {
            mesh,
            material: set.material_index,
            surface_map: map,
            geometry: &geometry,
            size,
        };
        for (target, channels, log) in targets {
            used.insert(target);
            let channels: Vec<Channel> = channels
                .into_iter()
                .filter(|channel| match target {
                    PaintTarget::Layer(_) => set_channels.contains(channel),
                    PaintTarget::Mask(..) => true,
                })
                .collect();
            if self
                .rasters
                .get(&target)
                .is_some_and(|raster| raster.size != size || raster.set != set.id)
                && let Some(old) = self.rasters.remove(&target)
            {
                old.release(pool);
            }
            self.ensure_contents(ctx, pool, encoder, target, set.id, size, &[]);
            let raster = self.rasters.get_mut(&target).expect("ensured above");
            let prefix = raster
                .strokes
                .iter()
                .zip(&log.strokes)
                .take_while(|(a, b)| Arc::ptr_eq(a, b))
                .count();
            if prefix == raster.strokes.len() && prefix == log.strokes.len() {
                continue;
            }

            // Undone strokes: their saved tiles, newest first.
            let mut full = false;
            while raster.strokes.len() > prefix {
                let stroke = raster.strokes.pop().expect("longer than the prefix");
                match raster.undo.remove(&Arc::as_ptr(&stroke)) {
                    Some(undo) if undo.alive(pool) => {
                        for (channel, snapshot) in &undo.before {
                            if let Some(content) = raster.contents.get(channel)
                                && let Some(texture) = pool.get(content.texture).cloned()
                            {
                                snapshot.restore(pool, encoder, &texture, None);
                            }
                        }
                        for tile in &undo.tiles {
                            dirty.add_tile(tile[0], tile[1]);
                        }
                        undo.release(pool);
                    }
                    other => {
                        if let Some(undo) = other {
                            undo.release(pool);
                        }
                        full = true;
                        break;
                    }
                }
            }
            let mut start = prefix;
            if full {
                for content in raster.contents.values() {
                    clear(encoder, &content.view);
                }
                raster.strokes.clear();
                for (_, undo) in raster.undo.drain() {
                    undo.release(pool);
                }
                dirty = Region::full(size);
                start = 0;
            }

            // New or redone strokes.
            let save = log.strokes.len() - start <= SAVED_REPLAYS;
            for stroke in &log.strokes[start..] {
                let painted: Vec<Channel> = channels
                    .iter()
                    .copied()
                    .filter(|channel| stroke_value(stroke, target, *channel).is_some())
                    .collect();
                self.ensure_contents(ctx, pool, encoder, target, set.id, size, &painted);
                let alpha = self.alpha(project, stroke);
                let raster = self.rasters.get_mut(&target).expect("ensured above");
                let textures: Vec<(Channel, wgpu::Texture, ContentLayout)> = painted
                    .iter()
                    .filter_map(|channel| {
                        let content = raster.contents.get(channel)?;
                        Some((*channel, pool.get(content.texture)?.clone(), content.layout))
                    })
                    .collect();
                let contents: Vec<Content<'_>> = textures
                    .iter()
                    .map(|(channel, texture, layout)| Content {
                        texture,
                        layout: *layout,
                        value: stroke_value(stroke, target, *channel).unwrap_or_default(),
                    })
                    .collect();
                if save {
                    let projection = StrokeProjection::new(&stroke.camera);
                    let dabs: Vec<GpuDab> =
                        stroke.dabs.iter().map(|dab| gpu_dab(stroke, dab)).collect();
                    let tiles = geometry.touched_tiles(&projection, &dabs, size);
                    let before = textures
                        .iter()
                        .map(|(channel, texture, _)| {
                            let mut snapshot = TileSnapshot::new(texture.format());
                            snapshot.save(ctx, pool, encoder, texture, &tiles);
                            snapshot.set_residency(pool, Residency::Discardable);
                            (*channel, snapshot)
                        })
                        .collect();
                    raster.undo.insert(
                        Arc::as_ptr(stroke),
                        StrokeUndo {
                            tiles: tiles.clone(),
                            before,
                        },
                    );
                }
                let tiles = painter.replay_stroke(
                    ctx,
                    pool,
                    encoder,
                    stroke,
                    &surface,
                    alpha.as_ref(),
                    &contents,
                );
                for tile in tiles {
                    dirty.add_tile(tile[0], tile[1]);
                }
                raster.strokes.push(stroke.clone());
                self.replays += 1;
            }
        }
        dirty
    }

    /// Releases the rasters not in `used`.
    pub(crate) fn release_unused(&mut self, pool: &mut TexturePool, used: &HashSet<PaintTarget>) {
        let unused: Vec<PaintTarget> = self
            .rasters
            .keys()
            .filter(|target| !used.contains(target))
            .copied()
            .collect();
        for target in unused {
            if let Some(raster) = self.rasters.remove(&target) {
                raster.release(pool);
            }
        }
    }

    /// Releases every texture.
    pub(crate) fn clear(&mut self, pool: &mut TexturePool) {
        for (_, raster) in self.rasters.drain() {
            raster.release(pool);
        }
        self.geometry.clear();
        self.alphas.clear();
    }
}
