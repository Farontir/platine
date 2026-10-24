//! Triangle picking: a pass rendering the triangle index + 1 (`R32Uint`, 0 = background)
//! and the depth (`R32Float`) of every pixel.
//!
//! With `Features::PRIMITIVE_INDEX` the indexed mesh is drawn and the fragment shader
//! reads `@builtin(primitive_index)`. Without it, de-indexed positions with a flat
//! triangle id are drawn instead ([`crate::mesh_gpu::FlatPickBuffers`]).

use std::sync::mpsc::{Receiver, TryRecvError};

use glam::{Mat4, Vec2, Vec3};
use patine_gpu::{GpuContext, GpuError, wgpu};

use crate::mesh_gpu::{GpuMesh, VERTEX_LAYOUTS};
use crate::viewport::DEPTH_FORMAT;

const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
const DEPTH_VALUE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// What lies under a pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickResult {
    /// Picked pixel.
    pub pixel: [u32; 2],
    /// Triangle of the mesh, or `None` over the background.
    pub triangle: Option<u32>,
    /// World position of the surface point, if a triangle was hit.
    pub position: Option<Vec3>,
}

/// Picking buffer of a whole image (tests and tools).
#[derive(Clone, Debug, PartialEq)]
pub struct PickImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Triangle index + 1 per pixel, 0 for the background.
    pub ids: Vec<u32>,
    /// Reverse-Z depth per pixel (0 for the background).
    pub depths: Vec<f32>,
}

struct Targets {
    width: u32,
    height: u32,
    ids: wgpu::Texture,
    ids_view: wgpu::TextureView,
    depth_values: wgpu::Texture,
    depth_values_view: wgpu::TextureView,
    depth: wgpu::TextureView,
}

struct Pending {
    receiver: Receiver<Result<(), wgpu::BufferAsyncError>>,
    staging: wgpu::Buffer,
    pixel: [u32; 2],
    size: [u32; 2],
    inverse_view_proj: Mat4,
}

/// Owner of the picking pipelines and targets.
pub(crate) struct Picker {
    indexed: Option<wgpu::RenderPipeline>,
    flat: wgpu::RenderPipeline,
    force_flat: bool,
    targets: Option<Targets>,
    pending: Option<Pending>,
}

fn pipeline(
    ctx: &GpuContext,
    shader: &str,
    layout: &wgpu::PipelineLayout,
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
) -> Result<wgpu::RenderPipeline, GpuError> {
    let module = ctx.create_shader_module(shader)?;
    let target = |format| {
        Some(wgpu::ColorTargetState {
            format,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })
    };
    Ok(ctx
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(shader),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                // Same culling as the shaded pass: pick what is visible.
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[target(ID_FORMAT), target(DEPTH_VALUE_FORMAT)],
            }),
            multiview_mask: None,
            cache: None,
        }))
}

/// Vertex layouts of the fallback: position at location 0, triangle id at location 1.
const FLAT_LAYOUTS: [Option<wgpu::VertexBufferLayout<'static>>; 2] = [
    Some(wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        }],
    }),
    Some(wgpu::VertexBufferLayout {
        array_stride: 4,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Uint32,
            offset: 0,
            shader_location: 1,
        }],
    }),
];

impl Picker {
    pub(crate) fn new(
        ctx: &GpuContext,
        frame_layout: &wgpu::BindGroupLayout,
        force_flat: bool,
    ) -> Result<Self, GpuError> {
        let layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("picking"),
                bind_group_layouts: &[Some(frame_layout)],
                immediate_size: 0,
            });
        let indexed = if ctx.capabilities().primitive_index && !force_flat {
            // A driver may report the feature but reject the pipeline: fall back to the
            // flat path instead of failing. (D3D12 rejected it while the fragment stage
            // read @builtin(position), see picking_indexed.wgsl.)
            let validation = ctx.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let internal = ctx.device.push_error_scope(wgpu::ErrorFilter::Internal);
            let result = pipeline(
                ctx,
                "render/picking_indexed",
                &layout,
                &[Some(VERTEX_LAYOUTS[0].clone())],
            );
            // Scopes pop in reverse order.
            let error = pollster::block_on(internal.pop()).or(pollster::block_on(validation.pop()));
            match (result, error) {
                (Ok(pipeline), None) => Some(pipeline),
                (Ok(_), Some(error)) => {
                    tracing::warn!(%error, "primitive_index picking unavailable, using the fallback");
                    None
                }
                (Err(error), _) => {
                    tracing::warn!(%error, "primitive_index picking unavailable, using the fallback");
                    None
                }
            }
        } else {
            None
        };
        let flat = pipeline(ctx, "render/picking_flat", &layout, &FLAT_LAYOUTS)?;
        Ok(Self {
            indexed,
            flat,
            force_flat,
            targets: None,
            pending: None,
        })
    }

    /// Whether a request waits for the GPU.
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Whether meshes need [`crate::mesh_gpu::FlatPickBuffers`].
    pub(crate) fn needs_flat_buffers(&self) -> bool {
        self.indexed.is_none() || self.force_flat
    }

    fn ensure_targets(&mut self, ctx: &GpuContext, width: u32, height: u32) {
        if self
            .targets
            .as_ref()
            .is_some_and(|t| t.width == width && t.height == height)
        {
            return;
        }
        let texture = |label, format, usage| {
            ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let color = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let ids = texture("pick ids", ID_FORMAT, color);
        let depth_values = texture("pick depth values", DEPTH_VALUE_FORMAT, color);
        let depth = texture(
            "pick depth",
            DEPTH_FORMAT,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        self.targets = Some(Targets {
            width,
            height,
            ids_view: ids.create_view(&Default::default()),
            ids,
            depth_values_view: depth_values.create_view(&Default::default()),
            depth_values,
            depth: depth.create_view(&Default::default()),
        });
    }

    /// Encodes the picking pass, restricted to `pixel` if given.
    fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        frame_bind_group: &wgpu::BindGroup,
        mesh: &GpuMesh,
        pixel: Option<[u32; 2]>,
    ) {
        let targets = self
            .targets
            .as_ref()
            .expect("targets are allocated before encoding");
        let attachment = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("picking"),
            color_attachments: &[
                attachment(&targets.ids_view),
                attachment(&targets.depth_values_view),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &targets.depth,
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
        if let Some([x, y]) = pixel {
            pass.set_scissor_rect(x, y, 1, 1);
        }
        pass.set_bind_group(0, frame_bind_group, &[]);
        match (&self.indexed, &mesh.flat_pick) {
            (Some(indexed), _) if !self.force_flat => {
                pass.set_pipeline(indexed);
                pass.set_vertex_buffer(0, mesh.positions.slice(..));
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
            (_, Some(flat)) => {
                pass.set_pipeline(&self.flat);
                pass.set_vertex_buffer(0, flat.positions.slice(..));
                pass.set_vertex_buffer(1, flat.ids.slice(..));
                pass.draw(0..flat.vertex_count, 0..1);
            }
            (_, None) => {
                tracing::warn!("picking needs flat buffers on this device; mesh has none");
            }
        }
    }

    /// Renders the picking buffer of a whole image and reads it back (blocking).
    pub(crate) fn pick_image(
        &mut self,
        ctx: &GpuContext,
        frame_bind_group: &wgpu::BindGroup,
        mesh: &GpuMesh,
        width: u32,
        height: u32,
    ) -> Result<PickImage, GpuError> {
        self.ensure_targets(ctx, width, height);
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        self.encode(&mut encoder, frame_bind_group, mesh, None);
        ctx.queue.submit([encoder.finish()]);
        let targets = self.targets.as_ref().expect("allocated above");
        let ids = patine_gpu::readback::read_texture(ctx, &targets.ids)?;
        let depths = patine_gpu::readback::read_texture(ctx, &targets.depth_values)?;
        Ok(PickImage {
            width,
            height,
            ids: bytemuck::pod_collect_to_vec(&ids),
            depths: bytemuck::pod_collect_to_vec(&depths),
        })
    }

    /// Starts reading the pixel `pixel`; the result comes from [`Picker::poll`]. Returns
    /// `false` (and does nothing) while a previous request is in flight or outside the
    /// image.
    pub(crate) fn request(
        &mut self,
        ctx: &GpuContext,
        frame_bind_group: &wgpu::BindGroup,
        mesh: &GpuMesh,
        size: [u32; 2],
        pixel: [u32; 2],
        view_proj: Mat4,
    ) -> bool {
        if self.pending.is_some() || pixel[0] >= size[0] || pixel[1] >= size[1] {
            return false;
        }
        self.ensure_targets(ctx, size[0], size[1]);
        let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pick staging"),
            // One texel of each target, each at a 256-byte aligned offset.
            size: 2 * u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = ctx.device.create_command_encoder(&Default::default());
        self.encode(&mut encoder, frame_bind_group, mesh, Some(pixel));
        let targets = self.targets.as_ref().expect("allocated above");
        for (index, texture) in [&targets.ids, &targets.depth_values]
            .into_iter()
            .enumerate()
        {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: pixel[0],
                        y: pixel[1],
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: index as u64 * u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                        bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                        rows_per_image: Some(1),
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
        ctx.queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        staging.map_async(wgpu::MapMode::Read, .., move |result| {
            // The picker may have been dropped; nothing to report then.
            let _ = sender.send(result);
        });
        self.pending = Some(Pending {
            receiver,
            staging,
            pixel,
            size,
            inverse_view_proj: view_proj.inverse(),
        });
        true
    }

    /// Returns the result of the request in flight once the GPU has finished it.
    pub(crate) fn poll(&mut self, ctx: &GpuContext) -> Option<PickResult> {
        let pending = self.pending.as_ref()?;
        // Non-blocking: drives callbacks of finished work.
        let _ = ctx.device.poll(wgpu::PollType::Poll);
        match pending.receiver.try_recv() {
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) | Ok(Err(_)) => {
                self.pending = None;
                None
            }
            Ok(Ok(())) => {
                let pending = self.pending.take()?;
                let (id, depth) = {
                    let mapped = pending.staging.get_mapped_range(..).ok()?;
                    let offset = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
                    let id = u32::from_le_bytes(mapped[0..4].try_into().ok()?);
                    let depth = f32::from_le_bytes(mapped[offset..offset + 4].try_into().ok()?);
                    (id, depth)
                };
                pending.staging.unmap();
                Some(resolve(
                    pending.pixel,
                    pending.size,
                    id,
                    depth,
                    pending.inverse_view_proj,
                ))
            }
        }
    }
}

/// World position and triangle of a picked pixel.
pub fn resolve(
    pixel: [u32; 2],
    size: [u32; 2],
    id: u32,
    depth: f32,
    inverse_view_proj: Mat4,
) -> PickResult {
    let triangle = id.checked_sub(1);
    let position = triangle.map(|_| {
        let ndc = Vec2::new(
            (pixel[0] as f32 + 0.5) / size[0] as f32 * 2.0 - 1.0,
            1.0 - (pixel[1] as f32 + 0.5) / size[1] as f32 * 2.0,
        );
        inverse_view_proj.project_point3(ndc.extend(depth))
    });
    PickResult {
        pixel,
        triangle,
        position,
    }
}
