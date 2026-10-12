//! GPU reflection of the document: the final texture of every channel of every texture
//! set, kept in step with the project by [`GpuDocument::sync`].
//!
//! The GPU is only a cache: everything here is derived from the project. `sync` compares
//! the project with what was last evaluated:
//! - allocation: textures follow the resolution and the channels of each texture set;
//! - contents: each procedural fill has a texture per channel (see [`crate::content`]),
//!   evaluated again only when its procedural, parameters or projection change;
//! - masks: each masked layer has an `R16Float` texture, evaluated again only when its
//!   mask stack changes;
//! - evaluation: for each channel, the first top-level layer whose contribution to that
//!   channel changed gives the starting point. The composition below the layer being
//!   edited is kept in a *lower cache*, so editing a layer only blends it and the layers
//!   above it again.
//!
//! The ping-pong textures, folder textures, lower caches and masks are discardable in the
//! pool: if evicted, they are rebuilt.

use std::collections::{HashMap, HashSet};

use patine_core::channel::{Channel, ChannelDef, ChannelFormat};
use patine_core::ids::LayerId;
use patine_core::layer::{BlendMode, FillSource, Layer, LayerKind};
use patine_core::mask::MaskStack;
use patine_core::project::{Project, TextureSetId};
use patine_gpu::pool::{Residency, TextureId, TexturePool, full_mip_count, texture_bytes};
use patine_gpu::{GpuContext, wgpu};

use crate::compositor::{BlendStep, Compositor, Ping, Region, Source};
use crate::content::{ContentId, Contents};
use crate::reference::ChannelKind;

/// GPU format of a channel format.
pub fn gpu_format(format: ChannelFormat) -> wgpu::TextureFormat {
    match format {
        ChannelFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        ChannelFormat::R16Float => wgpu::TextureFormat::R16Float,
    }
}

const USAGE: wgpu::TextureUsages = wgpu::TextureUsages::TEXTURE_BINDING
    .union(wgpu::TextureUsages::RENDER_ATTACHMENT)
    .union(wgpu::TextureUsages::COPY_SRC)
    .union(wgpu::TextureUsages::COPY_DST);

/// Descriptor of the final texture of a channel at `size`², with a full mip chain.
pub fn channel_descriptor(format: ChannelFormat, size: u32) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("channel"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: full_mip_count(size),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: gpu_format(format),
        usage: USAGE,
        view_formats: &[],
    }
}

/// Descriptor of a working texture (ping-pong, folder, lower cache): one level.
fn scratch_descriptor(format: wgpu::TextureFormat, size: u32) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("compositor scratch"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: USAGE,
        view_formats: &[],
    }
}

/// Memory the final textures of `project` take, in bytes.
pub fn expected_memory(project: &Project) -> u64 {
    project
        .texture_sets
        .iter()
        .flat_map(|set| {
            set.channels
                .iter()
                .map(|def| texture_bytes(&channel_descriptor(def.format, set.resolution.pixels())))
        })
        .sum()
}

/// The evaluated mask of one layer.
#[derive(Debug)]
struct MaskTexture {
    /// Texture set of the layer.
    set: TextureSetId,
    /// Stack it was evaluated from.
    stack: MaskStack,
    /// Side in pixels.
    size: u32,
    /// `R16Float` texture with its mips, discardable in the pool.
    texture: TextureId,
    /// View of every mip level.
    view: wgpu::TextureView,
}

/// Composition below a top-level layer, kept while that layer is being edited.
#[derive(Debug)]
struct LowerCache {
    /// Index of the first top-level layer not included.
    start: usize,
    texture: TextureId,
}

/// The final texture of one channel.
#[derive(Debug)]
pub struct ChannelTexture {
    /// Definition it was built from.
    pub def: ChannelDef,
    /// Side in pixels.
    pub size: u32,
    /// Texture in the pool.
    pub texture: TextureId,
    /// View of every mip level, for sampling.
    pub view: wgpu::TextureView,
    /// Top-level layers the texture was evaluated from; `None` before the first evaluation.
    evaluated: Option<Vec<Layer>>,
    cache: Option<LowerCache>,
}

/// The final textures of one texture set.
#[derive(Debug)]
pub struct GpuTextureSet {
    /// Texture set.
    pub id: TextureSetId,
    /// One texture per active channel, in the order of the texture set.
    pub channels: Vec<ChannelTexture>,
}

impl GpuTextureSet {
    /// The texture of `channel`, if the texture set has it.
    pub fn channel(&self, channel: Channel) -> Option<&ChannelTexture> {
        self.channels
            .iter()
            .find(|texture| texture.def.channel == channel)
    }
}

/// Working textures, by size, format and slot (two slots per folder depth).
#[derive(Debug, Default)]
struct Scratch {
    textures: HashMap<(u32, wgpu::TextureFormat, usize), TextureId>,
}

impl Scratch {
    fn get(
        &mut self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        size: u32,
        format: wgpu::TextureFormat,
        slot: usize,
    ) -> TextureId {
        let key = (size, format, slot);
        if let Some(id) = self.textures.get(&key)
            && pool.get(*id).is_some()
        {
            return *id;
        }
        let id = pool.allocate(
            ctx,
            &scratch_descriptor(format, size),
            Residency::Discardable,
        );
        self.textures.insert(key, id);
        id
    }

    fn release(&mut self, pool: &mut TexturePool) {
        for (_, id) in self.textures.drain() {
            pool.release(id);
        }
    }
}

/// The final textures of every texture set.
#[derive(Debug, Default)]
pub struct GpuDocument {
    sets: Vec<GpuTextureSet>,
    /// Masks of the masked layers of every texture set (identifiers are project-wide).
    masks: HashMap<LayerId, MaskTexture>,
    /// Contents of the procedural fills and mask fills.
    contents: Contents,
    scratch: Scratch,
}

/// Clears mip levels `levels` of `texture` to `value` (scalar formats use the first
/// component).
fn clear_levels(
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    value: [f32; 4],
    levels: std::ops::Range<u32>,
) {
    let [r, g, b, a] = value.map(f64::from);
    for level in levels {
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("channel clear"),
            base_mip_level: level,
            mip_level_count: Some(1),
            ..Default::default()
        });
        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("channel clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
}

/// Whether a layer contributes to `channel`.
fn affects(layer: &Layer, channel: Channel) -> bool {
    layer.visible && layer.channels.contains_key(&channel)
}

/// Whether two layers contribute the same way to `channel`.
fn same_for_channel(a: &Layer, b: &Layer, channel: Channel) -> bool {
    match (affects(a, channel), affects(b, channel)) {
        (false, false) => return true,
        (true, true) => {}
        _ => return false,
    }
    if a.channels.get(&channel) != b.channels.get(&channel) || a.mask != b.mask {
        return false;
    }
    match (&a.kind, &b.kind) {
        (
            LayerKind::Fill {
                source: FillSource::Uniform(x),
                ..
            },
            LayerKind::Fill {
                source: FillSource::Uniform(y),
                ..
            },
        ) => x.get(&channel) == y.get(&channel),
        (LayerKind::Folder { children: x }, LayerKind::Folder { children: y }) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|(p, q)| same_for_channel(p, q, channel))
        }
        (x, y) => x == y,
    }
}

/// Index of the first top-level layer whose contribution to `channel` differs between
/// `old` and `new`; `None` when the channel's composition is unchanged.
pub fn first_difference(old: &[Layer], new: &[Layer], channel: Channel) -> Option<usize> {
    let common = old.len().min(new.len());
    (0..common)
        .find(|&i| !same_for_channel(&old[i], &new[i], channel))
        .or((old.len() != new.len()).then_some(common))
}

fn view(pool: &TexturePool, id: TextureId) -> wgpu::TextureView {
    pool.peek(id)
        .expect("working textures are alive during an evaluation")
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// What an evaluation of one channel works with.
struct Evaluation<'a> {
    ctx: &'a GpuContext,
    compositor: &'a Compositor,
    masks: &'a HashMap<LayerId, MaskTexture>,
    contents: &'a Contents,
    channel: Channel,
    kind: ChannelKind,
    size: u32,
    format: wgpu::TextureFormat,
    region: Region,
}

impl Evaluation<'_> {
    /// Applies `layers` (bottom to top) to the composition in `pair.0`; the result ends in
    /// `pair.0`. Folders use the slots of `depth + 1`.
    fn apply(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pool: &mut TexturePool,
        scratch: &mut Scratch,
        layers: &[Layer],
        pair: &mut (TextureId, TextureId),
        depth: usize,
    ) {
        for layer in layers.iter().filter(|layer| affects(layer, self.channel)) {
            let blend = layer.channels[&self.channel];
            let mask = layer
                .mask
                .as_ref()
                .and_then(|_| self.masks.get(&layer.id))
                .map(|mask| &mask.view);
            match &layer.kind {
                LayerKind::Fill {
                    source: FillSource::Uniform(values),
                    ..
                } => {
                    let Some(value) = values.get(&self.channel) else {
                        continue;
                    };
                    let step = BlendStep {
                        mode: blend.mode,
                        opacity: blend.opacity,
                        source: Source::Value(*value),
                        mask,
                    };
                    self.blend(encoder, pool, pair, &step);
                }
                LayerKind::Fill {
                    source: FillSource::Procedural(_),
                    ..
                } => {
                    // Absent for channels the procedural does not produce.
                    let Some(content) =
                        self.contents.view(ContentId::Layer(layer.id, self.channel))
                    else {
                        continue;
                    };
                    let step = BlendStep {
                        mode: blend.mode,
                        opacity: blend.opacity,
                        source: Source::Texture(content),
                        mask,
                    };
                    self.blend(encoder, pool, pair, &step);
                }
                LayerKind::Folder { children } if blend.mode != BlendMode::Disable => {
                    // The children compose onto a copy of the composition below.
                    let slot = 2 * (depth + 1);
                    let mut inner = (
                        scratch.get(self.ctx, pool, self.size, self.format, slot),
                        scratch.get(self.ctx, pool, self.size, self.format, slot + 1),
                    );
                    if let (Some(from), Some(to)) = (pool.peek(pair.0), pool.peek(inner.0)) {
                        self.compositor.copy(encoder, from, to, &self.region);
                    }
                    self.apply(encoder, pool, scratch, children, &mut inner, depth + 1);
                    let inner_view = view(pool, inner.0);
                    let step = BlendStep {
                        mode: BlendMode::Normal,
                        opacity: blend.opacity,
                        source: Source::Texture(&inner_view),
                        mask,
                    };
                    self.blend(encoder, pool, pair, &step);
                }
                // Paint and image content, instances: later milestones.
                _ => {}
            }
        }
    }

    fn blend(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pool: &TexturePool,
        pair: &mut (TextureId, TextureId),
        step: &BlendStep<'_>,
    ) {
        let (read, write) = (view(pool, pair.0), view(pool, pair.1));
        self.compositor.blend(
            self.ctx,
            encoder,
            self.kind,
            Ping {
                read: &read,
                write: &write,
            },
            step,
            &self.region,
        );
        *pair = (pair.1, pair.0);
    }
}

impl GpuDocument {
    /// An empty reflection.
    pub fn new() -> Self {
        Self::default()
    }

    /// The final textures of texture set `id`.
    pub fn texture_set(&self, id: TextureSetId) -> Option<&GpuTextureSet> {
        self.sets.iter().find(|set| set.id == id)
    }

    /// Every texture set, in project order.
    pub fn texture_sets(&self) -> &[GpuTextureSet] {
        &self.sets
    }

    /// The evaluated mask of layer `id` (an `R16Float` texture with mips), if it has one.
    pub fn mask(&self, id: LayerId) -> Option<&wgpu::TextureView> {
        self.masks.get(&id).map(|mask| &mask.view)
    }

    /// Surface map of texture set `set` (an `R32Uint` texture in the pool), built when
    /// the set has procedural content.
    pub fn surface_map(&self, set: TextureSetId) -> Option<TextureId> {
        self.contents.surface_map(set)
    }

    /// Memory of the procedural contents and surface maps, in bytes.
    pub fn content_memory(&self) -> u64 {
        self.contents.memory()
    }

    /// Memory of the mask textures, in bytes.
    pub fn mask_memory(&self) -> u64 {
        self.masks
            .values()
            .map(|mask| texture_bytes(&channel_descriptor(ChannelFormat::R16Float, mask.size)))
            .sum()
    }

    /// Releases every texture (the document was closed or replaced).
    pub fn clear(&mut self, pool: &mut TexturePool) {
        for set in self.sets.drain(..) {
            for channel in set.channels {
                release_channel(pool, channel);
            }
        }
        for (_, mask) in self.masks.drain() {
            pool.release(mask.texture);
        }
        self.contents.clear(pool);
        self.scratch.release(pool);
    }

    /// Brings the GPU textures in line with `project`: allocates, reallocates and
    /// releases what changed, evaluates the masks that changed, then evaluates the layer
    /// stacks from the first changed layer of each channel. Returns the texture sets whose
    /// channel or mask textures were replaced, whose bind groups must be rebuilt.
    pub fn sync(
        &mut self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        compositor: &Compositor,
        project: &Project,
    ) -> Vec<TextureSetId> {
        let mut changed = Vec::new();
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("compositor"),
            });
        // Texture sets of the project, in its order; the others are released.
        let mut previous = std::mem::take(&mut self.sets);
        let mut masked = HashSet::new();
        let mut used = HashSet::new();
        for set in &project.texture_sets {
            let size = set.resolution.pixels();
            self.contents.sync_set(
                ctx,
                pool,
                compositor,
                &mut encoder,
                &project.mesh,
                set,
                &mut used,
            );
            if self.sync_masks(ctx, pool, compositor, &mut encoder, set, &mut masked) {
                changed.push(set.id);
            }
            let mut old = previous
                .iter()
                .position(|gpu| gpu.id == set.id)
                .map(|index| previous.remove(index).channels)
                .unwrap_or_default();
            let mut set_changed = false;
            let mut channels = Vec::with_capacity(set.channels.len());
            for def in &set.channels {
                let reusable = old
                    .iter()
                    .position(|texture| {
                        texture.def.channel == def.channel
                            && texture.def.format == def.format
                            && texture.size == size
                    })
                    .map(|index| old.remove(index));
                let mut texture = match reusable {
                    Some(mut texture) => {
                        if texture.def.default != def.default {
                            // Everything starts from the default value: evaluate again.
                            texture.def = def.clone();
                            texture.evaluated = None;
                            if let Some(cache) = texture.cache.take() {
                                pool.release(cache.texture);
                            }
                        }
                        texture
                    }
                    None => {
                        let id = pool.allocate(
                            ctx,
                            &channel_descriptor(def.format, size),
                            Residency::Pinned,
                        );
                        let gpu = pool.peek(id).expect("a texture that was just allocated");
                        let view = gpu.create_view(&wgpu::TextureViewDescriptor::default());
                        set_changed = true;
                        ChannelTexture {
                            def: def.clone(),
                            size,
                            texture: id,
                            view,
                            evaluated: None,
                            cache: None,
                        }
                    }
                };
                self.evaluate(
                    ctx,
                    pool,
                    compositor,
                    &mut encoder,
                    &mut texture,
                    &set.stack.layers,
                );
                channels.push(texture);
            }
            // Channels removed, or replaced at another size or format.
            for texture in old {
                release_channel(pool, texture);
                set_changed = true;
            }
            if set_changed && !changed.contains(&set.id) {
                changed.push(set.id);
            }
            self.sets.push(GpuTextureSet {
                id: set.id,
                channels,
            });
        }
        for set in previous {
            for texture in set.channels {
                release_channel(pool, texture);
            }
        }
        // Masks of layers that were deleted or lost their mask: their views must leave
        // the bind groups too.
        self.masks.retain(|id, mask| {
            let keep = masked.contains(id);
            if !keep {
                pool.release(mask.texture);
                if !changed.contains(&mask.set) {
                    changed.push(mask.set);
                }
            }
            keep
        });
        let sets: Vec<TextureSetId> = project.texture_sets.iter().map(|set| set.id).collect();
        self.contents.release_unused(pool, &used, &sets);
        ctx.queue.submit([encoder.finish()]);
        changed
    }

    /// Evaluates the masks of the layers of `set` that changed since their last
    /// evaluation. Adds the masked layers to `masked`. Returns whether a mask texture was
    /// allocated (its view is new).
    fn sync_masks(
        &mut self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        compositor: &Compositor,
        encoder: &mut wgpu::CommandEncoder,
        set: &patine_core::project::TextureSet,
        masked: &mut HashSet<LayerId>,
    ) -> bool {
        let size = set.resolution.pixels();
        let mut stacks = Vec::new();
        for layer in &set.stack.layers {
            layer.visit(&mut |layer| {
                if let Some(mask) = &layer.mask {
                    stacks.push((layer.id, mask));
                }
            });
        }
        let mut allocated = false;
        for (id, stack) in stacks {
            masked.insert(id);
            let current = self.masks.remove(&id);
            let alive = |pool: &mut TexturePool, mask: &MaskTexture| {
                mask.size == size && pool.get(mask.texture).is_some()
            };
            let mut mask = match current {
                Some(mask) if alive(pool, &mask) => {
                    if mask.stack == *stack {
                        self.masks.insert(id, mask);
                        continue;
                    }
                    mask
                }
                other => {
                    if let Some(old) = other {
                        pool.release(old.texture);
                    }
                    let texture = pool.allocate(
                        ctx,
                        &channel_descriptor(ChannelFormat::R16Float, size),
                        Residency::Discardable,
                    );
                    let view = pool
                        .peek(texture)
                        .expect("a texture that was just allocated")
                        .create_view(&wgpu::TextureViewDescriptor::default());
                    allocated = true;
                    MaskTexture {
                        set: set.id,
                        stack: stack.clone(),
                        size,
                        texture,
                        view,
                    }
                }
            };
            mask.stack = stack.clone();
            self.evaluate_mask(ctx, pool, compositor, encoder, id, &mask);
            self.masks.insert(id, mask);
        }
        allocated
    }

    /// Evaluates `mask.stack`, the mask of layer `layer`, into `mask.texture`, mips
    /// included.
    fn evaluate_mask(
        &mut self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        compositor: &Compositor,
        encoder: &mut wgpu::CommandEncoder,
        layer: LayerId,
        mask: &MaskTexture,
    ) {
        let target = pool
            .peek(mask.texture)
            .expect("mask textures are alive while evaluated");
        let initial = [mask.stack.initial_value(); 4];
        if mask.stack.effects.is_empty() {
            clear_levels(encoder, target, initial, 0..target.mip_level_count());
            return;
        }
        let format = wgpu::TextureFormat::R16Float;
        let mut pair = (
            self.scratch.get(ctx, pool, mask.size, format, 0),
            self.scratch.get(ctx, pool, mask.size, format, 1),
        );
        let region = Region::full(mask.size);
        clear_levels(
            encoder,
            pool.peek(pair.0).expect("allocated above"),
            initial,
            0..1,
        );
        for entry in &mask.stack.effects {
            let (read, write) = (view(pool, pair.0), view(pool, pair.1));
            let ping = Ping {
                read: &read,
                write: &write,
            };
            let source = self.contents.view(ContentId::Mask(layer, entry.id));
            if compositor.mask_effect(ctx, encoder, ping, entry, source, &region) {
                pair = (pair.1, pair.0);
            }
        }
        let (Some(result), Some(target)) = (pool.peek(pair.0), pool.peek(mask.texture)) else {
            return;
        };
        compositor.copy(encoder, result, target, &region);
        compositor.build_mips(ctx, encoder, ChannelKind::Scalar, target);
    }

    /// Evaluates the composition of one channel if its layers changed since the last
    /// evaluation.
    fn evaluate(
        &mut self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        compositor: &Compositor,
        encoder: &mut wgpu::CommandEncoder,
        texture: &mut ChannelTexture,
        layers: &[Layer],
    ) {
        let channel = texture.def.channel;
        let start = match &texture.evaluated {
            None => 0,
            Some(old) => match first_difference(old, layers, channel) {
                None => return,
                Some(index) => index,
            },
        };
        let target = pool
            .peek(texture.texture)
            .expect("channel textures are pinned");
        let levels = target.mip_level_count();
        texture.evaluated = Some(layers.to_vec());
        if !layers.iter().any(|layer| affects(layer, channel)) {
            clear_levels(encoder, target, texture.def.default, 0..levels);
            if let Some(cache) = texture.cache.take() {
                pool.release(cache.texture);
            }
            return;
        }

        let kind = ChannelKind::of(channel);
        let evaluation = Evaluation {
            ctx,
            compositor,
            masks: &self.masks,
            contents: &self.contents,
            channel,
            kind,
            size: texture.size,
            format: gpu_format(texture.def.format),
            region: Region::full(texture.size),
        };
        let scratch = &mut self.scratch;
        let mut pair = (
            scratch.get(ctx, pool, texture.size, evaluation.format, 0),
            scratch.get(ctx, pool, texture.size, evaluation.format, 1),
        );
        // The lower cache serves when it ends at or below the first change.
        let cached = texture
            .cache
            .as_ref()
            .filter(|cache| cache.start <= start && pool.get(cache.texture).is_some())
            .map(|cache| (cache.start, cache.texture));
        let resume = match cached {
            Some((cache_start, cache)) => {
                if let (Some(from), Some(to)) = (pool.peek(cache), pool.peek(pair.0)) {
                    compositor.copy(encoder, from, to, &evaluation.region);
                }
                cache_start
            }
            None => {
                let first = pool.peek(pair.0).expect("allocated above");
                clear_levels(encoder, first, texture.def.default, 0..1);
                if start == 0 {
                    // Layers below a lower cache changed: its content is stale.
                    if let Some(cache) = texture.cache.take() {
                        pool.release(cache.texture);
                    }
                } else {
                    // Compose what is below the change, and keep it.
                    evaluation.apply(encoder, pool, scratch, &layers[..start], &mut pair, 0);
                    let cache = match texture.cache.take() {
                        Some(cache) if pool.get(cache.texture).is_some() => cache.texture,
                        Some(cache) => {
                            pool.release(cache.texture);
                            pool.allocate(
                                ctx,
                                &scratch_descriptor(evaluation.format, texture.size),
                                Residency::Discardable,
                            )
                        }
                        None => pool.allocate(
                            ctx,
                            &scratch_descriptor(evaluation.format, texture.size),
                            Residency::Discardable,
                        ),
                    };
                    if let (Some(from), Some(to)) = (pool.peek(pair.0), pool.peek(cache)) {
                        compositor.copy(encoder, from, to, &evaluation.region);
                    }
                    texture.cache = Some(LowerCache {
                        start,
                        texture: cache,
                    });
                }
                start
            }
        };
        evaluation.apply(encoder, pool, scratch, &layers[resume..], &mut pair, 0);
        let (Some(result), Some(target)) = (pool.peek(pair.0), pool.peek(texture.texture)) else {
            return;
        };
        compositor.copy(encoder, result, target, &evaluation.region);
        compositor.build_mips(ctx, encoder, kind, target);
    }

    /// Memory of the final textures, in bytes.
    pub fn memory(&self) -> u64 {
        self.sets
            .iter()
            .flat_map(|set| &set.channels)
            .map(|texture| texture_bytes(&channel_descriptor(texture.def.format, texture.size)))
            .sum()
    }
}

fn release_channel(pool: &mut TexturePool, texture: ChannelTexture) {
    pool.release(texture.texture);
    if let Some(cache) = texture.cache {
        pool.release(cache.texture);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use patine_core::ids::LayerId;
    use patine_core::layer::{ChannelBlend, Projection};

    fn fill(id: u64, values: &[(Channel, f32)]) -> Layer {
        let channels: Vec<Channel> = values.iter().map(|(c, _)| *c).collect();
        Layer::new(
            LayerId(id),
            "fill",
            LayerKind::Fill {
                source: FillSource::Uniform(values.iter().map(|(c, v)| (*c, [*v; 4])).collect()),
                projection: Projection::default(),
            },
            &channels,
        )
    }

    #[test]
    fn differences_are_found_per_channel() {
        let a = fill(1, &[(Channel::BaseColor, 0.2), (Channel::Roughness, 0.5)]);
        let b = fill(2, &[(Channel::Roughness, 0.7)]);
        let old = vec![a.clone(), b.clone()];
        assert_eq!(first_difference(&old, &old, Channel::BaseColor), None);

        // Changing the roughness of the second layer leaves the base color alone.
        let mut edited = b.clone();
        edited.kind = fill(2, &[(Channel::Roughness, 0.9)]).kind;
        let new = vec![a.clone(), edited];
        assert_eq!(first_difference(&old, &new, Channel::BaseColor), None);
        assert_eq!(first_difference(&old, &new, Channel::Roughness), Some(1));

        // A blend change, a hidden layer, an inserted layer.
        let mut faded = a.clone();
        faded.channels.insert(
            Channel::BaseColor,
            ChannelBlend {
                mode: BlendMode::Multiply,
                opacity: 0.5,
            },
        );
        assert_eq!(
            first_difference(&old, &[faded, b.clone()], Channel::BaseColor),
            Some(0)
        );
        let mut hidden = b.clone();
        hidden.visible = false;
        assert_eq!(
            first_difference(&old, &[a.clone(), hidden], Channel::Roughness),
            Some(1)
        );
        assert_eq!(
            first_difference(&old, &[a.clone(), b.clone(), a], Channel::BaseColor),
            Some(2)
        );
    }
}
