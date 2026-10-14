//! GPU passes of the compositor: blending one layer onto the composition, applying one
//! effect of a mask, copying tiles, and building mips. Each blend is a fullscreen render pass that reads the current
//! composition (texture A) and writes the next one (texture B); blending is texel-local,
//! so a pass restricted to some tiles (scissor rectangles) only needs those tiles of A.

use bytemuck::{Pod, Zeroable};
use patine_core::layer::{BlendMode, FillSource};
use patine_core::mask::{FilterKind, MaskEffect, MaskEffectEntry};
use patine_gpu::{GpuContext, GpuError, wgpu};
use patine_paint::raster::Painter;
use patine_procedural::evaluate::Procedurals;
use patine_procedural::surface::SurfaceBuilder;
use wgpu::util::DeviceExt as _;

use crate::reference::{ChannelKind, mode_index};

/// Side of a tile, in texels (the paint engine's tiles).
pub const TILE: u32 = patine_paint::projection::TILE;

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

    /// Marks the tiles of `other` too (a region of the same size).
    pub fn union(&mut self, other: &Region) {
        for (mine, theirs) in self.tiles.iter_mut().zip(&other.tiles) {
            *mine |= *theirs;
        }
    }

    /// The region with the tiles around each marked tile marked too: what changes when
    /// texels read their neighbors.
    pub fn dilated(&self) -> Region {
        let mut dilated = self.clone();
        let n = self.columns as i64;
        for y in 0..n {
            for x in 0..n {
                if self.tiles[(y * n + x) as usize] {
                    for (dx, dy) in [
                        (-1, 0),
                        (1, 0),
                        (0, -1),
                        (0, 1),
                        (-1, -1),
                        (1, 1),
                        (-1, 1),
                        (1, -1),
                    ] {
                        let (nx, ny) = (x + dx, y + dy);
                        if (0..n).contains(&nx) && (0..n).contains(&ny) {
                            dilated.tiles[(ny * n + nx) as usize] = true;
                        }
                    }
                }
            }
        }
        dilated
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
    /// A texture of the target size (folder result, procedural content).
    Texture(&'a wgpu::TextureView),
    /// A paint content of the target size: premultiplied value with its coverage (see
    /// `patine_paint::reference::ContentLayout`), which scales the blend weight.
    Paint(&'a wgpu::TextureView),
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
    /// The layer's mask (an `R16Float` texture of the target size); `None` for none.
    pub mask: Option<&'a wgpu::TextureView>,
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

/// Uniforms of the shading normal pass. Must match `HeightNormal` in
/// `shaders/compositor/height_normal.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct HeightNormalUniforms {
    params: [f32; 4],
}

/// Slope of the relief per unit of height and of height force: a height step of 0.1
/// over 10 % of the UV square tilts the normal by about 3 degrees at force 1.
pub const HEIGHT_SCALE: f32 = 0.05;

/// Uniforms of a mask effect pass. Must match `MaskStep` in
/// `shaders/compositor/mask_effect.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct MaskUniforms {
    params: [f32; 4],
    levels: [f32; 4],
    levels_out: [f32; 4],
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
    mask_layout: wgpu::BindGroupLayout,
    procedurals: Procedurals,
    surfaces: SurfaceBuilder,
    painter: Painter,
    rgba: Pipelines,
    scalar: Pipelines,
    /// Mask effect pass (`R16Float`).
    mask: wgpu::RenderPipeline,
    /// Shading normal pass (`Rgba16Float`).
    height_normal: wgpu::RenderPipeline,
    /// Bound as the normal channel of a set without one: 1×1.
    flat_normal: wgpu::TextureView,
    /// Mask of the layers without mask: 1×1 white.
    white: wgpu::TextureView,
    /// Bound as the source texture of passes with a constant source.
    unused: wgpu::TextureView,
    passes: std::cell::Cell<u64>,
    mask_passes: std::cell::Cell<u64>,
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
        let mask_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compositor mask"),
            entries: &[uniform_entry(), texture_entry(1), texture_entry(2)],
        });
        let blend_module = ctx.create_shader_module("compositor/blend")?;
        let downsample_module = ctx.create_shader_module("compositor/downsample")?;
        let mask_module = ctx.create_shader_module("compositor/mask_effect")?;
        let height_normal_module = ctx.create_shader_module("compositor/height_normal")?;
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
            mask: pipeline(&mask_module, &mask_layout, wgpu::TextureFormat::R16Float),
            height_normal: pipeline(
                &height_normal_module,
                &mask_layout,
                wgpu::TextureFormat::Rgba16Float,
            ),
            flat_normal: pixel(ctx, wgpu::TextureFormat::Rgba16Float, 0.0),
            white: pixel(ctx, wgpu::TextureFormat::R16Float, 1.0),
            unused: pixel(ctx, wgpu::TextureFormat::R16Float, 0.0),
            blend_layout,
            downsample_layout,
            mask_layout,
            procedurals: Procedurals::new(ctx),
            surfaces: SurfaceBuilder::new(ctx)?,
            painter: Painter::new(ctx)?,
            passes: std::cell::Cell::new(0),
            mask_passes: std::cell::Cell::new(0),
        })
    }

    /// The pipelines of the procedurals.
    pub fn procedurals(&self) -> &Procedurals {
        &self.procedurals
    }

    /// The pipelines building surface maps.
    pub fn surfaces(&self) -> &SurfaceBuilder {
        &self.surfaces
    }

    /// The pipelines of the paint passes.
    pub fn painter(&self) -> &Painter {
        &self.painter
    }

    /// Number of mask effect passes encoded since creation (tests of the mask cache).
    pub fn mask_passes(&self) -> u64 {
        self.mask_passes.get()
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
            Source::Paint(view) => ([0.0; 4], 2.0, view),
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
                    resource: wgpu::BindingResource::TextureView(step.mask.unwrap_or(&self.white)),
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

    /// Encodes `ping.write = entry(ping.read)` on the tiles of `region`, for a mask
    /// (`R16Float` textures). A procedural fill reads `source` (its evaluated content, at
    /// the mask's size), and so does a paint effect (its paint content). Returns `false` without encoding anything when the effect leaves
    /// the mask unchanged: hidden, disabled, a procedural fill without content, or an
    /// effect of a later milestone (see [`crate::reference::mask_source`]); the
    /// composition then stays in `ping.read`.
    #[allow(clippy::too_many_arguments)]
    pub fn mask_effect(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        ping: Ping<'_>,
        entry: &MaskEffectEntry,
        source: Option<&wgpu::TextureView>,
        region: &Region,
    ) -> bool {
        if !entry.visible || entry.blend == BlendMode::Disable || region.is_empty() {
            return false;
        }
        let mut uniforms = MaskUniforms {
            params: [mode_index(entry.blend) as f32, entry.opacity, 0.0, 0.0],
            levels: [0.0; 4],
            levels_out: [0.0; 4],
        };
        match &entry.effect {
            MaskEffect::Fill {
                source: FillSource::Procedural(_),
            } => {
                if source.is_none() {
                    return false;
                }
                uniforms.params[2] = 3.0;
            }
            MaskEffect::Paint { .. } => {
                if source.is_none() {
                    return false;
                }
                uniforms.params[2] = 4.0;
            }
            MaskEffect::Fill { .. } => match entry.effect.fill_value() {
                Some(value) => uniforms.params[3] = value,
                None => return false,
            },
            MaskEffect::Levels(levels) => {
                uniforms.params[2] = 1.0;
                uniforms.levels = [levels.in_low, levels.in_high, levels.gamma, levels.out_low];
                uniforms.levels_out[0] = levels.out_high;
            }
            MaskEffect::Filter {
                kind: FilterKind::Invert,
                ..
            } => uniforms.params[2] = 2.0,
            _ => return false,
        }
        self.mask_passes.set(self.mask_passes.get() + 1);
        let buffer = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("compositor mask"),
                contents: bytemuck::bytes_of(&uniforms),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compositor mask"),
            layout: &self.mask_layout,
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
                    resource: wgpu::BindingResource::TextureView(source.unwrap_or(&self.unused)),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compositor mask"),
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
        pass.set_pipeline(&self.mask);
        pass.set_bind_group(0, &bind_group, &[]);
        for [x, y, width, height] in region.rects() {
            pass.set_scissor_rect(x, y, width, height);
            pass.draw(0..3, 0..1);
        }
        true
    }

    /// Encodes the shading normal into mip 0 of `target` (`Rgba16Float`, `size`²) on the
    /// tiles of `region`: the height channel's relief (`height`, force `force`) combined
    /// with the normal channel (`normal`, flat without).
    #[allow(clippy::too_many_arguments)]
    pub fn height_normal(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        height: &wgpu::TextureView,
        normal: Option<&wgpu::TextureView>,
        target: &wgpu::TextureView,
        force: f32,
        size: u32,
        region: &Region,
    ) {
        if region.is_empty() {
            return;
        }
        let uniforms = HeightNormalUniforms {
            params: [
                force * HEIGHT_SCALE,
                size as f32,
                if normal.is_some() { 1.0 } else { 0.0 },
                0.0,
            ],
        };
        let buffer = ctx
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("compositor height normal"),
                contents: bytemuck::bytes_of(&uniforms),
                usage: wgpu::BufferUsages::UNIFORM,
            });
        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compositor height normal"),
            layout: &self.mask_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(height),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        normal.unwrap_or(&self.flat_normal),
                    ),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("compositor height normal"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        pass.set_pipeline(&self.height_normal);
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
    /// normals), over the part of each level that the tiles of `region` (in level 0)
    /// cover.
    pub fn build_mips(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        kind: ChannelKind,
        texture: &wgpu::Texture,
        region: &Region,
    ) {
        if region.is_empty() {
            return;
        }
        let rects = region.rects();
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
            let width = (texture.width() >> level).max(1);
            let height = (texture.height() >> level).max(1);
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
                        load: wgpu::LoadOp::Load,
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
            for [x, y, w, h] in &rects {
                // The texels of this level that the rectangle of level 0 covers.
                let x0 = (x >> level).min(width - 1);
                let y0 = (y >> level).min(height - 1);
                let x1 = (x + w).div_ceil(1 << level).clamp(x0 + 1, width);
                let y1 = (y + h).div_ceil(1 << level).clamp(y0 + 1, height);
                pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
                pass.draw(0..3, 0..1);
            }
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
