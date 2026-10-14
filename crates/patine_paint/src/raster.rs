//! The GPU side of painting: the depth seen by a stroke's camera, [`Painter::rasterize_dabs`]
//! (the one function that rasterizes dabs, for live painting and replay alike), and the
//! composition of a stroke onto paint contents.
//!
//! A stroke accumulates its dabs in a stroke buffer (`R32Float`, the size of the texture
//! set's textures). A paint content is then, on every tile the stroke touched, its tile
//! from before the stroke with the stroke composed over it (see
//! [`crate::reference::deposit`] and [`crate::reference::erase`]):
//! - live, the tiles from before the stroke are saved when first touched
//!   ([`TileSnapshot`]), and every batch of dabs recomposes its tiles from them; the
//!   saved tiles then serve to undo the stroke;
//! - a replay rasterizes every dab at once, then composes once onto the content, which
//!   still holds the tiles from before the stroke.
//!
//! Both give the same texels: accumulation is in f32 and in dab order, and a dab adds
//! exactly nothing outside the tiles it touches.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};

use bytemuck::{Pod, Zeroable};
use patine_core::stroke::{Dab, Stroke, StrokeSpace, Tool};
use patine_gpu::pool::{Residency, TextureId, TexturePool};
use patine_gpu::{GpuContext, GpuError, wgpu};
use patine_procedural::surface::MeshBuffers;
use wgpu::util::DeviceExt as _;

use crate::alphas::AlphaImage;
use crate::dynamics::gpu_dabs;
use crate::projection::{GpuDab, SetGeometry, StrokeProjection, TILE, stroke_projections};
use crate::reference::ContentLayout;
use crate::{DEPTH_BIAS, DILATION, GRAZING};

/// Format of the stroke buffer.
pub const STROKE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// Format of the depth image of a stroke.
pub const DEPTH_IMAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// Side of the pages holding the tiles saved before a stroke.
pub const PAGE: u32 = 1024;

/// Uniforms of `shaders/paint/depth.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct DepthUniforms {
    view_proj: [[f32; 4]; 4],
    depth_axis: [f32; 4],
}

/// Uniforms of `shaders/paint/dabs.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct DabUniforms {
    viewport: [f32; 2],
    size: u32,
    dab_count: u32,
    copy_count: u32,
    extent: f32,
    has_alpha: u32,
    dilation: f32,
    depth_bias: f32,
    uv: u32,
    grazing: [f32; 2],
    bounds: [f32; 4],
}

// WGSL rounds the uniform structure to 16 bytes: 64 bytes with the bounds.
const _: () = assert!(std::mem::size_of::<DabUniforms>() == 64);

/// The camera of one symmetry copy. Must match `Copy` in `shaders/paint/dabs.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct GpuCopy {
    view_proj: [[f32; 4]; 4],
    depth_axis: [f32; 4],
    forward: [f32; 4],
}

/// Uniforms of `shaders/paint/stroke_composite.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct CompositeUniforms {
    value: [f32; 4],
    opacity: f32,
    eraser: u32,
    scalar: u32,
    textured: u32,
}

/// Format of a paint content texture of `layout`.
pub fn content_format(layout: ContentLayout) -> wgpu::TextureFormat {
    match layout {
        ContentLayout::Vector => wgpu::TextureFormat::Rgba16Float,
        ContentLayout::Scalar => wgpu::TextureFormat::Rg16Float,
    }
}

/// Usage of paint content textures: drawn into, read by the compositor, copied for undo.
pub const CONTENT_USAGE: wgpu::TextureUsages = wgpu::TextureUsages::RENDER_ATTACHMENT
    .union(wgpu::TextureUsages::TEXTURE_BINDING)
    .union(wgpu::TextureUsages::COPY_SRC)
    .union(wgpu::TextureUsages::COPY_DST);

/// Descriptor of a `size`² paint content texture of `layout`, without mips.
pub fn content_descriptor(layout: ContentLayout, size: u32) -> wgpu::TextureDescriptor<'static> {
    square(
        Some("paint content"),
        size,
        content_format(layout),
        CONTENT_USAGE,
    )
}

fn square(
    label: Option<&'static str>,
    size: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label,
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    }
}

/// What a stroke paints on: a texture set's mesh, surface map and size.
#[derive(Clone, Copy, Debug)]
pub struct SetSurface<'a> {
    /// The mesh in storage buffers.
    pub mesh: &'a MeshBuffers,
    /// Material of the texture set.
    pub material: u32,
    /// Surface map of the texture set, at `size`.
    pub surface_map: &'a wgpu::TextureView,
    /// The set's triangles on the CPU.
    pub geometry: &'a SetGeometry,
    /// Side of the set's textures.
    pub size: u32,
}

/// One paint content a stroke composes onto: its texture, layout and painted value.
#[derive(Clone, Copy, Debug)]
pub struct Content<'a> {
    /// The content texture ([`content_descriptor`]).
    pub texture: &'a wgpu::Texture,
    /// Its layout.
    pub layout: ContentLayout,
    /// Value deposited (ignored by the eraser): rgb or xyz, or the scalar first.
    pub value: [f32; 4],
    /// Values deposited texel by texel instead of `value` (a procedural material
    /// evaluated on the set, at the content's size): rgb or xyz, or the scalar in red.
    pub values: Option<&'a wgpu::TextureView>,
}

/// The GPU state of one stroke being rasterized: its camera's depth image, its alpha and
/// its stroke buffer (pinned in the pool until [`Painter::end_stroke`]).
#[derive(Debug)]
pub struct StrokeTarget {
    /// The stroke's cameras: one per symmetry copy, the stroke's own first (one camera
    /// for a stroke painted in UV space).
    pub projections: Vec<StrokeProjection>,
    /// Where the stroke was painted.
    pub space: StrokeSpace,
    /// Opacity of the stroke.
    pub opacity: f32,
    /// Whether the stroke erases.
    pub eraser: bool,
    /// The stroke buffer.
    pub buffer: TextureId,
    depth: wgpu::Texture,
    alpha: Option<wgpu::Texture>,
    size: u32,
}

impl StrokeTarget {
    /// The depth images of the stroke's cameras ([`DEPTH_IMAGE_FORMAT`]), one array
    /// layer per copy (a 1×1 layer for a stroke painted in UV space).
    pub fn depth_texture(&self) -> &wgpu::Texture {
        &self.depth
    }
}

/// Pipelines of the paint passes.
#[derive(Debug)]
pub struct Painter {
    depth: wgpu::RenderPipeline,
    depth_layout: wgpu::BindGroupLayout,
    dabs: wgpu::ComputePipeline,
    dabs_layout: wgpu::BindGroupLayout,
    surface_layout: wgpu::BindGroupLayout,
    /// Deposit and erase pipelines, for the vector and scalar layouts.
    deposit: [wgpu::RenderPipeline; 2],
    erase: [wgpu::RenderPipeline; 2],
    composite_layout: wgpu::BindGroupLayout,
    /// 1×1 alpha bound when a stroke has none.
    no_alpha: wgpu::TextureView,
    /// 1×1 values bound when a content takes a constant.
    no_values: wgpu::TextureView,
    passes: AtomicU64,
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn buffer(
    ctx: &GpuContext,
    label: &str,
    contents: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    // Storage bindings cannot be empty.
    let mut bytes = contents.to_vec();
    if bytes.len() < 16 {
        bytes.resize(16, 0);
    }
    ctx.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: &bytes,
            usage,
        })
}

/// A render pass on `view` that clears it to `value`.
fn clear(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, value: f64) {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("paint clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: value,
                    g: 0.0,
                    b: 0.0,
                    a: 0.0,
                }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

/// Texel rectangle of `tile` in a `size`² texture: origin and extent.
fn tile_rect(tile: [u32; 2], size: u32) -> ([u32; 2], [u32; 2]) {
    let origin = [tile[0] * TILE, tile[1] * TILE];
    (
        origin,
        [TILE.min(size - origin[0]), TILE.min(size - origin[1])],
    )
}

/// Copies a `extent` rectangle from `from` at `from_origin` to `to` at `to_origin`.
fn copy_rect(
    encoder: &mut wgpu::CommandEncoder,
    from: &wgpu::Texture,
    from_origin: [u32; 2],
    to: &wgpu::Texture,
    to_origin: [u32; 2],
    extent: [u32; 2],
) {
    fn at(texture: &wgpu::Texture, origin: [u32; 2]) -> wgpu::TexelCopyTextureInfo<'_> {
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: origin[0],
                y: origin[1],
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        }
    }
    encoder.copy_texture_to_texture(
        at(from, from_origin),
        at(to, to_origin),
        wgpu::Extent3d {
            width: extent[0],
            height: extent[1],
            depth_or_array_layers: 1,
        },
    );
}

impl Painter {
    /// Creates the pipelines.
    pub fn new(ctx: &GpuContext) -> Result<Self, GpuError> {
        let device = &ctx.device;
        let vertex = wgpu::ShaderStages::VERTEX;
        let compute = wgpu::ShaderStages::COMPUTE;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let unfilterable = wgpu::TextureSampleType::Float { filterable: false };

        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("paint depth"),
            entries: &[
                uniform_entry(0, vertex),
                storage_entry(1, vertex),
                storage_entry(2, vertex),
            ],
        });
        let module = ctx.create_shader_module("paint/depth")?;
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("paint depth"),
            bind_group_layouts: &[Some(&depth_layout)],
            immediate_size: 0,
        });
        let depth = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("paint depth"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(true),
                // Reverse-Z: nearer is greater.
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: DEPTH_IMAGE_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let dabs_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("paint dabs"),
            entries: &[
                uniform_entry(0, compute),
                storage_entry(1, compute),
                storage_entry(2, compute),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: compute,
                    ty: wgpu::BindingType::Texture {
                        sample_type: unfilterable,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                texture_entry(4, compute, unfilterable),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: compute,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::ReadWrite,
                        format: STROKE_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                storage_entry(6, compute),
            ],
        });
        let surface_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("paint surface"),
            entries: &[
                storage_entry(0, compute),
                storage_entry(1, compute),
                texture_entry(2, compute, wgpu::TextureSampleType::Uint),
            ],
        });
        let module = ctx.create_shader_module("paint/dabs")?;
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("paint dabs"),
            bind_group_layouts: &[Some(&dabs_layout), Some(&surface_layout)],
            immediate_size: 0,
        });
        let dabs = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("paint dabs"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("cs_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("paint composite"),
            entries: &[
                uniform_entry(0, fragment),
                texture_entry(1, fragment, unfilterable),
                texture_entry(2, fragment, unfilterable),
            ],
        });
        let module = ctx.create_shader_module("paint/stroke_composite")?;
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("paint composite"),
            bind_group_layouts: &[Some(&composite_layout)],
            immediate_size: 0,
        });
        // Deposit: dst = src + dst × (1 − src.a), on every component. Erase: dst × (1 −
        // src.a) only, as the coverage of a vector content is its alpha component.
        let blend = |src_factor| wgpu::BlendComponent {
            src_factor,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let composite = |format, over: wgpu::BlendComponent| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("paint composite"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_fullscreen"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: over,
                            alpha: over,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let formats = [ContentLayout::Vector, ContentLayout::Scalar].map(content_format);
        let deposit = formats.map(|format| composite(format, blend(wgpu::BlendFactor::One)));
        let erase = formats.map(|format| composite(format, blend(wgpu::BlendFactor::Zero)));

        let no_alpha = device
            .create_texture(&square(
                Some("no alpha"),
                1,
                wgpu::TextureFormat::R8Unorm,
                wgpu::TextureUsages::TEXTURE_BINDING,
            ))
            .create_view(&Default::default());
        let no_values = device
            .create_texture(&square(
                Some("no values"),
                1,
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureUsages::TEXTURE_BINDING,
            ))
            .create_view(&Default::default());
        Ok(Self {
            depth,
            depth_layout,
            dabs,
            dabs_layout,
            surface_layout,
            deposit,
            erase,
            composite_layout,
            no_alpha,
            no_values,
            passes: AtomicU64::new(0),
        })
    }

    /// Number of dab passes encoded so far ([`Painter::rasterize_dabs`] calls that did
    /// work), for tests and measurements.
    pub fn passes(&self) -> u64 {
        self.passes.load(Ordering::Relaxed)
    }

    /// Starts rasterizing `stroke` into the texture set `set`: renders the depth seen by
    /// the camera of each symmetry copy, uploads its alpha and clears a stroke buffer.
    pub fn begin_stroke(
        &self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        encoder: &mut wgpu::CommandEncoder,
        stroke: &Stroke,
        set: &SetSurface<'_>,
        alpha: Option<&AlphaImage>,
    ) -> StrokeTarget {
        let device = &ctx.device;
        let projections = stroke_projections(stroke);
        // A stroke painted in UV space sees no depth: one 1×1 layer.
        let (width, height, layers) = match stroke.space {
            StrokeSpace::View => {
                let [w, h] = stroke.camera.viewport.map(|v| v.max(1));
                (w, h, projections.len() as u32)
            }
            StrokeSpace::Uv => (1, 1, 1),
        };
        let size2 = |format, usage, layers| wgpu::TextureDescriptor {
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: layers,
            },
            ..square(Some("stroke depth"), 1, format, usage)
        };
        let depth = device.create_texture(&size2(
            DEPTH_IMAGE_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            layers,
        ));
        let depth_test = device.create_texture(&size2(
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
            1,
        ));
        let rendered = match stroke.space {
            StrokeSpace::View => projections.as_slice(),
            StrokeSpace::Uv => &[],
        };
        if rendered.is_empty() {
            // Cleared to "nothing hides anything".
            let view = depth.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                array_layer_count: Some(1),
                ..Default::default()
            });
            clear(encoder, &view, f64::from(f32::MAX));
        }
        for (layer, projection) in rendered.iter().enumerate() {
            let uniforms = DepthUniforms {
                view_proj: projection.view_proj.to_cols_array_2d(),
                depth_axis: projection.depth_axis.to_array(),
            };
            let uniforms = buffer(
                ctx,
                "paint depth",
                bytemuck::bytes_of(&uniforms),
                wgpu::BufferUsages::UNIFORM,
            );
            let view = depth.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer as u32,
                array_layer_count: Some(1),
                ..Default::default()
            });
            let test_view = depth_test.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("paint depth"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(f32::MAX),
                            g: 0.0,
                            b: 0.0,
                            a: 0.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &test_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.depth);
            for (indices, triangles) in &set.mesh.materials {
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("paint depth"),
                    layout: &self.depth_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: uniforms.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: set.mesh.vertices.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: indices.as_entire_binding(),
                        },
                    ],
                });
                pass.set_bind_group(0, &bind_group, &[]);
                pass.draw(0..triangles * 3, 0..1);
            }
        }

        let alpha = alpha.map(|alpha| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                size: wgpu::Extent3d {
                    width: alpha.width,
                    height: alpha.height,
                    depth_or_array_layers: 1,
                },
                ..square(
                    Some("stroke alpha"),
                    1,
                    wgpu::TextureFormat::R8Unorm,
                    wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                )
            });
            ctx.queue.write_texture(
                texture.as_image_copy(),
                &alpha.pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(alpha.width),
                    rows_per_image: None,
                },
                texture.size(),
            );
            texture
        });

        let buffer = pool.allocate(
            ctx,
            &square(
                Some("stroke buffer"),
                set.size,
                STROKE_FORMAT,
                wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
            ),
            Residency::Pinned,
        );
        let view = pool
            .peek(buffer)
            .expect("a texture that was just allocated")
            .create_view(&Default::default());
        clear(encoder, &view, 0.0);
        StrokeTarget {
            projections,
            space: stroke.space,
            opacity: stroke.brush.opacity.clamp(0.0, 1.0),
            eraser: stroke.tool == Tool::Eraser,
            buffer,
            depth,
            alpha,
            size: set.size,
        }
    }

    /// Rasterizes `dabs` into the stroke buffer of `target`, on `tiles` (from
    /// [`SetGeometry::touched_tiles`] for these dabs). The one function that rasterizes
    /// dabs: live painting calls it on each new batch, replay on every dab at once.
    #[allow(clippy::too_many_arguments)]
    pub fn rasterize_dabs(
        &self,
        ctx: &GpuContext,
        pool: &TexturePool,
        encoder: &mut wgpu::CommandEncoder,
        target: &StrokeTarget,
        set: &SetSurface<'_>,
        dabs: &[GpuDab],
        tiles: &[[u32; 2]],
    ) {
        if dabs.is_empty() || tiles.is_empty() {
            return;
        }
        let Some(stroke) = pool.peek(target.buffer) else {
            return;
        };
        let Some((indices, _)) = set.mesh.materials.get(set.material as usize) else {
            return;
        };
        let device = &ctx.device;
        let copies: Vec<GpuCopy> = target
            .projections
            .iter()
            .map(|projection| GpuCopy {
                view_proj: projection.view_proj.to_cols_array_2d(),
                depth_axis: projection.depth_axis.to_array(),
                forward: projection.forward.extend(0.0).to_array(),
            })
            .collect();
        let uniforms = DabUniforms {
            viewport: target.projections[0].viewport.to_array(),
            size: target.size,
            dab_count: dabs.len() as u32,
            copy_count: copies.len() as u32,
            extent: set.mesh.extent,
            has_alpha: u32::from(target.alpha.is_some()),
            dilation: DILATION,
            depth_bias: DEPTH_BIAS,
            uv: u32::from(target.space == StrokeSpace::Uv),
            grazing: GRAZING,
            bounds: crate::projection::dab_bounds(dabs),
        };
        let storage = wgpu::BufferUsages::STORAGE;
        let uniforms = buffer(
            ctx,
            "paint dabs",
            bytemuck::bytes_of(&uniforms),
            wgpu::BufferUsages::UNIFORM,
        );
        let dab_buffer = buffer(ctx, "paint dabs", bytemuck::cast_slice(dabs), storage);
        let tile_buffer = buffer(ctx, "paint tiles", bytemuck::cast_slice(tiles), storage);
        let copy_buffer = buffer(ctx, "paint copies", bytemuck::cast_slice(&copies), storage);
        let depth = target.depth.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let alpha = target
            .alpha
            .as_ref()
            .map(|alpha| alpha.create_view(&Default::default()));
        let stroke = stroke.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("paint dabs"),
            layout: &self.dabs_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: dab_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: tile_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&depth),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(
                        alpha.as_ref().unwrap_or(&self.no_alpha),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&stroke),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: copy_buffer.as_entire_binding(),
                },
            ],
        });
        let surface = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("paint surface"),
            layout: &self.surface_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: set.mesh.vertices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: indices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(set.surface_map),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("paint dabs"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.dabs);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_bind_group(1, &surface, &[]);
        let groups = TILE / 8;
        pass.dispatch_workgroups(groups, groups, tiles.len() as u32);
        self.passes.fetch_add(1, Ordering::Relaxed);
    }

    /// Composes the stroke of `target` onto `content` on `tiles`, over what the content
    /// holds there.
    pub fn composite(
        &self,
        ctx: &GpuContext,
        pool: &TexturePool,
        encoder: &mut wgpu::CommandEncoder,
        target: &StrokeTarget,
        content: &Content<'_>,
        tiles: &[[u32; 2]],
    ) {
        if tiles.is_empty() {
            return;
        }
        let Some(stroke) = pool.peek(target.buffer) else {
            return;
        };
        let device = &ctx.device;
        let uniforms = CompositeUniforms {
            value: content.value,
            opacity: target.opacity,
            eraser: u32::from(target.eraser),
            scalar: u32::from(content.layout == ContentLayout::Scalar),
            textured: u32::from(content.values.is_some()),
        };
        let uniforms = buffer(
            ctx,
            "paint composite",
            bytemuck::bytes_of(&uniforms),
            wgpu::BufferUsages::UNIFORM,
        );
        let stroke = stroke.create_view(&Default::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("paint composite"),
            layout: &self.composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&stroke),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(
                        content.values.unwrap_or(&self.no_values),
                    ),
                },
            ],
        });
        let view = content.texture.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("paint composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
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
        let pipelines = if target.eraser {
            &self.erase
        } else {
            &self.deposit
        };
        pass.set_pipeline(match content.layout {
            ContentLayout::Vector => &pipelines[0],
            ContentLayout::Scalar => &pipelines[1],
        });
        pass.set_bind_group(0, &bind_group, &[]);
        let size = content.texture.width();
        for tile in tiles {
            let (origin, extent) = tile_rect(*tile, size);
            pass.set_scissor_rect(origin[0], origin[1], extent[0], extent[1]);
            pass.draw(0..3, 0..1);
        }
    }

    /// Ends a stroke: releases its stroke buffer.
    pub fn end_stroke(&self, pool: &mut TexturePool, target: StrokeTarget) {
        pool.release(target.buffer);
    }

    /// Replays `stroke` onto `contents` at once: every dab, then one composition onto
    /// the contents as they are. Returns the tiles touched.
    #[allow(clippy::too_many_arguments)]
    pub fn replay_stroke(
        &self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        encoder: &mut wgpu::CommandEncoder,
        stroke: &Stroke,
        set: &SetSurface<'_>,
        alpha: Option<&AlphaImage>,
        contents: &[Content<'_>],
    ) -> Vec<[u32; 2]> {
        let target = self.begin_stroke(ctx, pool, encoder, stroke, set, alpha);
        let dabs = gpu_dabs(stroke, 0..stroke.dabs.len());
        let tiles = set.geometry.stroke_tiles(stroke, &dabs, set.size);
        self.rasterize_dabs(ctx, pool, encoder, &target, set, &dabs, &tiles);
        for content in contents {
            self.composite(ctx, pool, encoder, &target, content, &tiles);
        }
        self.end_stroke(pool, target);
        tiles
    }
}

/// Tiles of one content saved before a stroke, in pages of the content's format.
#[derive(Debug)]
pub struct TileSnapshot {
    format: wgpu::TextureFormat,
    pages: Vec<TextureId>,
    /// Page and slot of each saved tile.
    slots: BTreeMap<[u32; 2], (usize, u32)>,
}

impl TileSnapshot {
    /// No tile yet, for contents of `format`.
    pub fn new(format: wgpu::TextureFormat) -> Self {
        Self {
            format,
            pages: Vec::new(),
            slots: BTreeMap::new(),
        }
    }

    /// The saved tiles.
    pub fn tiles(&self) -> impl Iterator<Item = [u32; 2]> + '_ {
        self.slots.keys().copied()
    }

    /// Saves the tiles of `tiles` not saved yet from `content`. New pages are pinned.
    pub fn save(
        &mut self,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        encoder: &mut wgpu::CommandEncoder,
        content: &wgpu::Texture,
        tiles: &[[u32; 2]],
    ) {
        let per_row = PAGE / TILE;
        let per_page = per_row * per_row;
        let size = content.width();
        for tile in tiles {
            if self.slots.contains_key(tile) {
                continue;
            }
            let index = self.slots.len() as u32;
            let (page, slot) = ((index / per_page) as usize, index % per_page);
            if page == self.pages.len() {
                self.pages.push(pool.allocate(
                    ctx,
                    &square(
                        Some("paint tiles"),
                        PAGE,
                        self.format,
                        wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                    ),
                    Residency::Pinned,
                ));
            }
            let Some(page_texture) = pool.get(self.pages[page]) else {
                continue;
            };
            let (origin, extent) = tile_rect(*tile, size);
            let at = [(slot % per_row) * TILE, (slot / per_row) * TILE];
            copy_rect(encoder, content, origin, page_texture, at, extent);
            self.slots.insert(*tile, (page, slot));
        }
    }

    /// Whether every page is still in the pool.
    pub fn alive(&self, pool: &TexturePool) -> bool {
        self.pages.iter().all(|page| pool.peek(*page).is_some())
    }

    /// Copies the saved tiles among `tiles` (all of them for `None`) back into
    /// `content`. Returns `false`, copying nothing, if a page was evicted.
    pub fn restore(
        &self,
        pool: &mut TexturePool,
        encoder: &mut wgpu::CommandEncoder,
        content: &wgpu::Texture,
        tiles: Option<&[[u32; 2]]>,
    ) -> bool {
        if !self.alive(pool) {
            return false;
        }
        let per_row = PAGE / TILE;
        let size = content.width();
        let mut copy = |tile: &[u32; 2], (page, slot): (usize, u32)| {
            if let Some(page) = pool.get(self.pages[page]) {
                let (origin, extent) = tile_rect(*tile, size);
                let at = [(slot % per_row) * TILE, (slot / per_row) * TILE];
                copy_rect(encoder, page, at, content, origin, extent);
            }
        };
        match tiles {
            None => {
                for (tile, slot) in &self.slots {
                    copy(tile, *slot);
                }
            }
            Some(tiles) => {
                for tile in tiles {
                    if let Some(slot) = self.slots.get(tile) {
                        copy(tile, *slot);
                    }
                }
            }
        }
        true
    }

    /// Makes the pages pinned (while the stroke is painted) or discardable (kept for
    /// undo while memory allows).
    pub fn set_residency(&self, pool: &mut TexturePool, residency: Residency) {
        for page in &self.pages {
            pool.set_residency(*page, residency);
        }
    }

    /// Releases the pages.
    pub fn release(self, pool: &mut TexturePool) {
        for page in self.pages {
            pool.release(page);
        }
    }
}

/// A stroke painted live, batch by batch, onto one or more contents.
#[derive(Debug)]
pub struct LiveStroke {
    stroke: Stroke,
    target: StrokeTarget,
    /// Tiles from before the stroke, per content (in the order of the contents given).
    before: Vec<TileSnapshot>,
    tiles: BTreeSet<[u32; 2]>,
}

/// What a finished live stroke leaves: the stroke itself, the tiles it touched, and the
/// tiles from before it (discardable), per content.
#[derive(Debug)]
pub struct FinishedStroke {
    /// The stroke, with every dab painted.
    pub stroke: Stroke,
    /// Tiles touched.
    pub tiles: Vec<[u32; 2]>,
    /// Tiles from before the stroke, per content.
    pub before: Vec<TileSnapshot>,
}

impl LiveStroke {
    /// Starts painting `stroke` (its dabs are ignored: they come with
    /// [`LiveStroke::add`]) onto contents of the given layouts.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        painter: &Painter,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        encoder: &mut wgpu::CommandEncoder,
        stroke: Stroke,
        set: &SetSurface<'_>,
        alpha: Option<&AlphaImage>,
        layouts: &[ContentLayout],
    ) -> Self {
        let target = painter.begin_stroke(ctx, pool, encoder, &stroke, set, alpha);
        Self {
            stroke: Stroke {
                dabs: Vec::new(),
                ..stroke
            },
            target,
            before: layouts
                .iter()
                .map(|layout| TileSnapshot::new(content_format(*layout)))
                .collect(),
            tiles: BTreeSet::new(),
        }
    }

    /// The stroke so far.
    pub fn stroke(&self) -> &Stroke {
        &self.stroke
    }

    /// Paints `dabs` onto `contents` (the same contents, in the same order, at every
    /// call). Returns the tiles that changed.
    #[allow(clippy::too_many_arguments)]
    pub fn add(
        &mut self,
        painter: &Painter,
        ctx: &GpuContext,
        pool: &mut TexturePool,
        encoder: &mut wgpu::CommandEncoder,
        set: &SetSurface<'_>,
        contents: &[Content<'_>],
        dabs: &[Dab],
    ) -> Vec<[u32; 2]> {
        let start = self.stroke.dabs.len();
        self.stroke.dabs.extend_from_slice(dabs);
        let gpu = gpu_dabs(&self.stroke, start..self.stroke.dabs.len());
        let tiles = set.geometry.stroke_tiles(&self.stroke, &gpu, set.size);
        if tiles.is_empty() {
            return tiles;
        }
        for (before, content) in self.before.iter_mut().zip(contents) {
            before.save(ctx, pool, encoder, content.texture, &tiles);
        }
        painter.rasterize_dabs(ctx, pool, encoder, &self.target, set, &gpu, &tiles);
        for (before, content) in self.before.iter().zip(contents) {
            before.restore(pool, encoder, content.texture, Some(&tiles));
            painter.composite(ctx, pool, encoder, &self.target, content, &tiles);
        }
        self.tiles.extend(tiles.iter().copied());
        tiles
    }

    /// Drops the stroke without restoring anything: its buffer and saved tiles are
    /// released (the contents are being released too).
    pub fn abandon(self, pool: &mut TexturePool) {
        pool.release(self.target.buffer);
        for before in self.before {
            before.release(pool);
        }
    }

    /// Ends the stroke: its buffer is released, and the tiles from before it become
    /// discardable caches for undo.
    pub fn finish(self, painter: &Painter, pool: &mut TexturePool) -> FinishedStroke {
        painter.end_stroke(pool, self.target);
        for before in &self.before {
            before.set_residency(pool, Residency::Discardable);
        }
        FinishedStroke {
            stroke: self.stroke,
            tiles: self.tiles.into_iter().collect(),
            before: self.before,
        }
    }
}
