//! The 2D view: a channel texture of a texture set in UV space, with the UV wireframe of
//! its submesh, rendered off-screen like the viewport (`Rgba8Unorm` holding display-encoded
//! values, what egui expects).
//!
//! The wireframe draws the polygon edges of the submesh with the UVs as positions. Mesh
//! vertices are split at UV seams, so every vertex has a single UV.

use bytemuck::{Pod, Zeroable};
use patine_gpu::{GpuContext, GpuError, wgpu};

use crate::material::InspectKind;
use crate::mesh_gpu::GpuMesh;
use crate::viewport::OUTPUT_FORMAT;

/// Wireframe color of the 2D view, display-encoded.
pub const WIRE_COLOR: [f32; 3] = [1.0, 0.62, 0.25];

/// Mapping between UV space and the pixels of the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvTransform {
    /// UV at the center of the view.
    pub center: [f32; 2],
    /// Pixels per UV unit.
    pub zoom: f32,
}

impl Default for UvTransform {
    fn default() -> Self {
        Self {
            center: [0.5, 0.5],
            zoom: 256.0,
        }
    }
}

impl UvTransform {
    /// Shows the whole 0–1 square in a `width` × `height` view, with a margin.
    pub fn framing(width: u32, height: u32) -> Self {
        Self {
            center: [0.5, 0.5],
            zoom: 0.9 * width.min(height).max(1) as f32,
        }
    }

    /// UV under the pixel position `pixel` of a `size` view.
    pub fn uv_at(&self, pixel: [f32; 2], size: [u32; 2]) -> [f32; 2] {
        [
            self.center[0] + (pixel[0] - 0.5 * size[0] as f32) / self.zoom,
            self.center[1] + (pixel[1] - 0.5 * size[1] as f32) / self.zoom,
        ]
    }

    /// Pixel position of `uv` in a `size` view.
    pub fn pixel_of(&self, uv: [f32; 2], size: [u32; 2]) -> [f32; 2] {
        [
            (uv[0] - self.center[0]) * self.zoom + 0.5 * size[0] as f32,
            (uv[1] - self.center[1]) * self.zoom + 0.5 * size[1] as f32,
        ]
    }

    /// Moves the content by `delta` pixels.
    pub fn pan(&mut self, delta: [f32; 2]) {
        self.center[0] -= delta[0] / self.zoom;
        self.center[1] -= delta[1] / self.zoom;
    }

    /// Zooms by `factor`, keeping the UV under `pixel` in place.
    pub fn zoom_at(&mut self, factor: f32, pixel: [f32; 2], size: [u32; 2]) {
        let anchor = self.uv_at(pixel, size);
        self.zoom = (self.zoom * factor).clamp(8.0, 65_536.0);
        let moved = self.pixel_of(anchor, size);
        self.pan([pixel[0] - moved[0], pixel[1] - moved[1]]);
    }
}

/// What the 2D view shows.
#[derive(Clone, Copy, Debug)]
pub struct UvScene<'a> {
    /// The mesh.
    pub mesh: &'a GpuMesh,
    /// The texture set (material index) whose UVs are shown.
    pub material: u32,
    /// Channel texture shown in the 0–1 square, and how; neutral gray without.
    pub texture: Option<(&'a wgpu::TextureView, InspectKind)>,
    /// Mapping between UVs and pixels.
    pub transform: UvTransform,
    /// Whether the UV wireframe is drawn.
    pub wireframe: bool,
}

/// Uniforms of the 2D view. Must match `Uv` in `shaders/render/uv_view.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct UvUniforms {
    transform: [f32; 4],
    viewport: [f32; 4],
    display: [f32; 4],
    wire_color: [f32; 4],
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Renders the 2D view off-screen.
pub struct UvView {
    layout: wgpu::BindGroupLayout,
    buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    background: wgpu::RenderPipeline,
    wire: wgpu::RenderPipeline,
    /// Shown when no channel texture is given.
    blank: wgpu::TextureView,
    target: Option<Target>,
}

fn pipeline(
    ctx: &GpuContext,
    module: &wgpu::ShaderModule,
    layout: &wgpu::PipelineLayout,
    (vertex, fragment): (&str, &str),
    buffers: &[Option<wgpu::VertexBufferLayout<'_>>],
    topology: wgpu::PrimitiveTopology,
) -> wgpu::RenderPipeline {
    ctx.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(fragment),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some(vertex),
                compilation_options: Default::default(),
                buffers,
            },
            primitive: wgpu::PrimitiveState {
                topology,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OUTPUT_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
}

impl UvView {
    /// Creates the pipelines.
    pub fn new(ctx: &GpuContext) -> Result<Self, GpuError> {
        let device = &ctx.device;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("uv view"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<UvUniforms>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("uv view"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = ctx.create_shader_module("render/uv_view")?;
        let background = pipeline(
            ctx,
            &module,
            &pipeline_layout,
            ("vs_fullscreen", "fs_background"),
            &[],
            wgpu::PrimitiveTopology::TriangleList,
        );
        let uv_layout = wgpu::VertexBufferLayout {
            array_stride: 8,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 0,
                shader_location: 0,
            }],
        };
        let wire = pipeline(
            ctx,
            &module,
            &pipeline_layout,
            ("vs_wire", "fs_wire"),
            &[Some(uv_layout)],
            wgpu::PrimitiveTopology::LineList,
        );
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uv view"),
            size: size_of::<UvUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("uv view"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let blank = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("uv view blank"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Ok(Self {
            layout,
            buffer,
            sampler,
            background,
            wire,
            blank,
            target: None,
        })
    }

    /// Renders `scene` at `width` × `height` and submits the work. Returns `true` when the
    /// output texture was reallocated.
    pub fn render(
        &mut self,
        ctx: &GpuContext,
        scene: &UvScene<'_>,
        width: u32,
        height: u32,
    ) -> bool {
        let (width, height) = (width.max(1), height.max(1));
        let reallocated = self
            .target
            .as_ref()
            .is_none_or(|t| t.texture.width() != width || t.texture.height() != height);
        if reallocated {
            let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("uv view output"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: OUTPUT_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            self.target = Some(Target { texture, view });
        }
        let target = self.target.as_ref().expect("allocated above");

        let (texture, kind, shown) = match scene.texture {
            Some((view, kind)) => (view, kind, 1.0),
            None => (&self.blank, InspectKind::Color, 0.0),
        };
        let kind = match kind {
            InspectKind::Color => 0.0,
            InspectKind::Scalar => 1.0,
            InspectKind::Normal => 2.0,
        };
        let t = scene.transform;
        let uniforms = UvUniforms {
            transform: [t.center[0], t.center[1], t.zoom, 0.0],
            viewport: [width as f32, height as f32, 0.0, 0.0],
            display: [kind, shown, 0.0, 0.0],
            wire_color: [WIRE_COLOR[0], WIRE_COLOR[1], WIRE_COLOR[2], 1.0],
        };
        ctx.queue
            .write_buffer(&self.buffer, 0, bytemuck::bytes_of(&uniforms));
        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("uv view"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(texture),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("uv view"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("uv view"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
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
            pass.set_bind_group(0, &bind_group, &[]);
            pass.set_pipeline(&self.background);
            pass.draw(0..3, 0..1);
            let submesh = scene
                .mesh
                .submeshes
                .iter()
                .find(|submesh| submesh.material == scene.material);
            if scene.wireframe
                && let Some(submesh) = submesh
                && !submesh.uv_wire.is_empty()
            {
                pass.set_pipeline(&self.wire);
                pass.set_vertex_buffer(0, scene.mesh.uvs.slice(..));
                pass.set_index_buffer(
                    scene.mesh.uv_wire_indices.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(submesh.uv_wire.clone(), 0, 0..1);
            }
        }
        ctx.queue.submit([encoder.finish()]);
        reallocated
    }

    /// The last rendered image, `Rgba8Unorm` with display-encoded values.
    pub fn output(&self) -> Option<(&wgpu::Texture, &wgpu::TextureView)> {
        self.target.as_ref().map(|t| (&t.texture, &t.view))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_round_trips_and_zooms_around_the_cursor() {
        let size = [400, 300];
        let mut t = UvTransform::framing(size[0], size[1]);
        assert_eq!(t.uv_at([200.0, 150.0], size), [0.5, 0.5]);
        let corner = t.pixel_of([0.0, 0.0], size);
        assert!(
            corner[0] > 0.0 && corner[1] > 0.0,
            "the square fits: {corner:?}"
        );
        let uv = t.uv_at([123.0, 45.0], size);
        let back = t.pixel_of(uv, size);
        assert!((back[0] - 123.0).abs() < 1e-3 && (back[1] - 45.0).abs() < 1e-3);
        t.zoom_at(2.5, [123.0, 45.0], size);
        let still = t.uv_at([123.0, 45.0], size);
        assert!((still[0] - uv[0]).abs() < 1e-5 && (still[1] - uv[1]).abs() < 1e-5);
        t.pan([10.0, -20.0]);
        let moved = t.uv_at([133.0, 25.0], size);
        assert!((moved[0] - uv[0]).abs() < 1e-5 && (moved[1] - uv[1]).abs() < 1e-5);
    }
}
