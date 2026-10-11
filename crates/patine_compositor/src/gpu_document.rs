//! GPU reflection of the document: the final texture of every channel of every texture
//! set, kept in step with the project by [`GpuDocument::sync`].
//!
//! The GPU is only a cache: everything here is derived from the project. Until layer
//! stacks are evaluated (M5), a channel's texture holds its default value, written by
//! clearing every mip level.

use patine_core::channel::{Channel, ChannelDef, ChannelFormat};
use patine_core::project::{Project, TextureSetId};
use patine_gpu::pool::{Residency, TextureId, TexturePool, full_mip_count, texture_bytes};
use patine_gpu::{GpuContext, wgpu};

/// GPU format of a channel format.
pub fn gpu_format(format: ChannelFormat) -> wgpu::TextureFormat {
    match format {
        ChannelFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        ChannelFormat::R16Float => wgpu::TextureFormat::R16Float,
    }
}

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
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
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

/// The final textures of every texture set.
#[derive(Debug, Default)]
pub struct GpuDocument {
    sets: Vec<GpuTextureSet>,
}

/// Clears every mip level of `texture` to `value` (scalar formats use the first
/// component).
fn clear(ctx: &GpuContext, texture: &wgpu::Texture, value: [f32; 4]) {
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("channel clear"),
        });
    for level in 0..texture.mip_level_count() {
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("channel clear"),
            base_mip_level: level,
            mip_level_count: Some(1),
            ..Default::default()
        });
        let [r, g, b, a] = value.map(f64::from);
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
    ctx.queue.submit([encoder.finish()]);
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

    /// Releases every texture (the document was closed or replaced).
    pub fn clear(&mut self, pool: &mut TexturePool) {
        for set in self.sets.drain(..) {
            for channel in set.channels {
                pool.release(channel.texture);
            }
        }
    }

    /// Brings the GPU textures in line with `project`: allocates, reallocates, releases
    /// and clears what changed. Returns the texture sets whose textures changed, whose
    /// bind groups must be rebuilt.
    pub fn sync(
        &mut self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        project: &Project,
    ) -> Vec<TextureSetId> {
        let mut changed = Vec::new();
        // Texture sets of the project, in its order; the others are released.
        let mut previous = std::mem::take(&mut self.sets);
        for set in &project.texture_sets {
            let size = set.resolution.pixels();
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
                let texture = match reusable {
                    Some(mut texture) => {
                        if texture.def.default != def.default {
                            if let Some(gpu) = pool.peek(texture.texture) {
                                clear(ctx, gpu, def.default);
                            }
                            texture.def = def.clone();
                            set_changed = true;
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
                        clear(ctx, gpu, def.default);
                        let view = gpu.create_view(&wgpu::TextureViewDescriptor::default());
                        set_changed = true;
                        ChannelTexture {
                            def: def.clone(),
                            size,
                            texture: id,
                            view,
                        }
                    }
                };
                channels.push(texture);
            }
            // Channels removed, or replaced at another size or format.
            for texture in old {
                pool.release(texture.texture);
                set_changed = true;
            }
            if set_changed {
                changed.push(set.id);
            }
            self.sets.push(GpuTextureSet {
                id: set.id,
                channels,
            });
        }
        for set in previous {
            for texture in set.channels {
                pool.release(texture.texture);
            }
        }
        changed
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
