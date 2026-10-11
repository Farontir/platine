//! GPU passes of the compositor: blending one layer onto the composition, copying tiles,
//! and building mips. Each blend is a fullscreen render pass that reads the current
//! composition (texture A) and writes the next one (texture B); blending is texel-local,
//! so a pass restricted to some tiles (scissor rectangles) only needs those tiles of A.

use bytemuck::{Pod, Zeroable};
use patine_core::layer::BlendMode;
use patine_gpu::{GpuContext, GpuError, wgpu};
use wgpu::util::DeviceExt as _;

use crate::reference::{ChannelKind, mode_index};

/// Side of a tile, in texels.
pub const TILE: u32 = 128;

/// Tiles of a square texture to recompute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Region {
    /// Texture side, in texels.
    size: u32,
    /// Tiles per row.
    columns: u32,
    /// Dirty flag of each tile, row by row.
    tiles: Vec<bool>,
}

impl Region {
    /// No tile of a `size`² texture.
    pub fn empty(size: u32) -> Self {
        let columns = size.div_ceil(TILE).max(1);
        Self {
            size,
            columns,
            tiles: vec![false; (columns * columns) as usize],
        }
    }

    /// Every tile of a `size`² texture.
    pub fn full(size: u32) -> Self {
        let mut region = Self::empty(size);
        region.tiles.fill(true);
        region
    }

    /// Marks the tile at column `x`, row `y`.
    pub fn add_tile(&mut self, x: u32, y: u32) {
        if x < self.columns && y < self.columns {
            self.tiles[(y * self.columns + x) as usize] = true;
        }
    }

    /// Whether no tile is marked.
    pub fn is_empty(&self) -> bool {
        !self.tiles.contains(&true)
    }

    /// Whether every tile is marked.
    pub fn is_full(&self) -> bool {
        !self.tiles.contains(&false)
    }

    /// Texture side.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// The marked tiles as rectangles `[x, y, width, height]` in texels: runs of adjacent
    /// tiles of each row.
    pub fn rects(&self) -> Vec<[u32; 4]> {
        let mut rects = Vec::new();
        for row in 0..self.columns {
            let mut column = 0;
            while column < self.columns {
                if !self.tiles[(row * self.columns + column) as usize] {
                    column += 1;
                    continue;
                }
                let start = column;
                while column < self.columns && self.tiles[(row * self.columns + column) as usize] {
                    column += 1;
                }
                let x = start * TILE;
                let y = row * TILE;
                rects.push([
                    x,
                    y,
                    (column * TILE).min(self.size) - x,
                    ((row + 1) * TILE).min(self.size) - y,
                ]);
            }
        }
        rects
    }
}

/// Where a blend pass takes its source from.
#[derive(Clone, Copy, Debug)]
pub enum Source<'a> {
    /// A constant (uniform fill).
    Value([f32; 4]),
    /// A texture of the target size (folder result).
    Texture(&'a wgpu::TextureView),
}

/// One layer to blend onto the composition.
#[derive(Clone, Copy, Debug)]
pub struct BlendStep<'a> {
    /// Blend mode.
    pub mode: BlendMode,
    /// Opacity, 0 to 1.
    pub opacity: f32,
    /// Content.
    pub source: Source<'a>,
}

/// The composition read by a pass and the texture it writes: single-level views of
/// textures of the region's size, in the format of the channel kind.
#[derive(Clone, Copy, Debug)]
pub struct Ping<'a> {
    /// Current composition.
    pub read: &'a wgpu::TextureView,
    /// Next composition.
    pub write: &'a wgpu::TextureView,
}

/// Uniforms of a blend pass. Must match `Step` in `shaders/compositor/blend.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct BlendUniforms {
    src: [f32; 4],
    params: [f32; 4],
}

/// GPU format holding channels of `kind`.
pub fn format_of(kind: ChannelKind) -> wgpu::TextureFormat {
    match kind {
        ChannelKind::Scalar => wgpu::TextureFormat::R16Float,
        ChannelKind::Color | ChannelKind::Normal => wgpu::TextureFormat::Rgba16Float,
    }
}

struct Pipelines {
    blend: wgpu::RenderPipeline,
    downsample: wgpu::RenderPipeline,
}

/// The compositor's pipelines and fixed resources.
pub struct Compositor {
    blend_layout: wgpu::BindGroupLayout,
    downsample_layout: wgpu::BindGroupLayout,
    rgba: Pipelines,
    scalar: Pipelines,
    /// Mask of the layers without mask (until M6): 1×1 white.
    white: wgpu::TextureView,
    /// Bound as the source texture of passes with a constant source.
    unused: wgpu::TextureView,
    passes: std::cell::Cell<u64>,
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn uniform_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn pixel(ctx: &GpuContext, format: wgpu::TextureFormat, value: f32) -> wgpu::TextureView {
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("compositor pixel"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let bytes: Vec<u8> = [value; 4]
        .iter()
        .flat_map(|v| half::f16::from_f32(*v).to_le_bytes())
        .collect();
    ctx.queue.write_texture(
        texture.as_image_copy(),
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: None,
            rows_per_image: None,
        },
        texture.size(),
    );
    texture.create_view(&Default::default())
}

impl Compositor {
    /// Creates the pipelines.
    pub fn new(ctx: &GpuContext) -> Result<Self, GpuError> {
        let device = &ctx.device;
        let blend_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compositor blend"),
            entries: &[
                uniform_entry(),
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
            ],
        });
        let downsample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compositor downsample"),
            entries: &[uniform_entry(), texture_entry(1)],
        });
        let blend_module = ctx.create_shader_module("compositor/blend")?;
        let downsample_module = ctx.create_shader_module("compositor/downsample")?;
        let pipeline = |module: &wgpu::ShaderModule, layout: &wgpu::BindGroupLayout, format| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("compositor"),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("compositor"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipelines = |format| Pipelines {
            blend: pipeline(&blend_module, &blend_layout, format),
            downsample: pipeline(&downsample_module, &downsample_layout, format),
        };
        Ok(Self {
            rgba: pipelines(wgpu::TextureFormat::Rgba16Float),
            scalar: pipelines(wgpu::TextureFormat::R16Float),
            white: pixel(ctx, wgpu::TextureFormat::R16Float, 1.0),
            unused: pixel(ctx, wgpu::TextureFormat::R16Float, 0.0),
            blend_layout,
            downsample_layout,
            passes: std::cell::Cell::new(0),
        })
    }

    /// Number of blend passes encoded since creation (tests of the lower cache).
    pub fn blend_passes(&self) -> u64 {
        self.passes.get()
    }

    fn pipelines(&self, kind: ChannelKind) -> &Pipelines {
        match kind {
            ChannelKind::Scalar => &self.scalar,
            ChannelKind::Color | ChannelKind::Normal => &self.rgba,
        }
    }

    /// Encodes `ping.write = blend(ping.read, step)` on the tiles of `region`.
    pub fn blend(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        kind: ChannelKind,
        ping: Ping<'_>,
        step: &BlendStep<'_>,
        region: &Region,
    ) {
        if region.is_empty() {
            return;
        }
        self.passes.set(self.passes.get() + 1);
        let (src, from_texture, texture) = match step.source {
            Source::Value(value) => (value, 0.0, &self.unused),
            Source::Texture(view) => ([0.0; 4], 1.0, view),
        };
        let uniforms = BlendUniforms {
            src,
            params: [
                mode_index(step.mode) as f32,
                kind.index() as f32,
                step.opacity,
                from_texture,
            ],
        };
        let buffer = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("compositor blend"),
                contents: bytemuck::bytes_of(&uniforms),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compositor blend"),
            layout: &self.blend_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(ping.read),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(texture),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.white),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compositor blend"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: ping.write,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipelines(kind).blend);
        pass.set_bind_group(0, &bind_group, &[]);
        for [x, y, width, height] in region.rects() {
            pass.set_scissor_rect(x, y, width, height);
            pass.draw(0..3, 0..1);
        }
    }

    /// Copies the tiles of `region` from mip 0 of `source` to mip 0 of `destination`
    /// (same size and format).
    pub fn copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        destination: &wgpu::Texture,
        region: &Region,
    ) {
        for [x, y, width, height] in region.rects() {
            let origin = wgpu::Origin3d { x, y, z: 0 };
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: source,
                    mip_level: 0,
                    origin,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: destination,
                    mip_level: 0,
                    origin,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    /// Rebuilds mips 1 and up of `texture` from mip 0 (2 × 2 averages, renormalized for
    /// normals).
    pub fn build_mips(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        kind: ChannelKind,
        texture: &wgpu::Texture,
    ) {
        let buffer = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("compositor downsample"),
                contents: bytemuck::cast_slice(&[kind.index() as f32, 0.0, 0.0, 0.0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let level_view = |level| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("compositor mip"),
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            })
        };
        for level in 1..texture.mip_level_count() {
            let source = level_view(level - 1);
            let target = level_view(level);
            let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("compositor downsample"),
                layout: &self.downsample_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&source),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("compositor downsample"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipelines(kind).downsample);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_merge_tiles_into_row_runs() {
        let mut region = Region::empty(300);
        assert!(region.is_empty());
        region.add_tile(0, 0);
        region.add_tile(1, 0);
        region.add_tile(2, 2);
        region.add_tile(9, 9); // Outside: ignored.
        assert_eq!(region.rects(), vec![[0, 0, 256, 128], [256, 256, 44, 44]]);
        assert!(Region::full(256).is_full());
        assert_eq!(
            Region::full(256).rects(),
            vec![[0, 0, 256, 128], [0, 128, 256, 128]]
        );
    }
}
