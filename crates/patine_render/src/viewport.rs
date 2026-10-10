//! The viewport renderer: draws the mesh off screen and produces an sRGB-encoded
//! `Rgba8Unorm` image, the format egui displays as is.

use bytemuck::{Pod, Zeroable};
use patine_gpu::{GpuContext, GpuError, wgpu};

use crate::camera::OrbitCamera;
use crate::mesh_gpu::{GpuMesh, VERTEX_LAYOUTS};
use crate::settings::{Background, ViewSettings};

/// Linear HDR format of the scene.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Depth format (reverse-Z).
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Output format, sRGB-encoded values in a non-sRGB texture (what egui expects).
pub const OUTPUT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// MSAA sample count of the scene pass.
pub const SAMPLES: u32 = 4;

/// Uniforms shared by every pass. Must match `Frame` in `shaders/render/frame.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct FrameUniforms {
    view_proj: [[f32; 4]; 4],
    inv_view_proj: [[f32; 4]; 4],
    camera_position: [f32; 4],
    base_color: [f32; 4],
    material: [f32; 4],
    wire_color: [f32; 4],
    viewport: [f32; 4],
    background: [f32; 4],
    environment: [f32; 4],
}

impl FrameUniforms {
    fn new(camera: &OrbitCamera, settings: &ViewSettings, width: u32, height: u32) -> Self {
        let aspect = width as f32 / height as f32;
        let view_proj = camera.view_projection(aspect);
        let material = &settings.material;
        let (background, blur) = match settings.background {
            Background::Solid(color) => ([color[0], color[1], color[2], 0.0], 0.0),
            Background::Environment { blur } => ([0.0, 0.0, 0.0, 1.0], blur),
        };
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            camera_position: camera.eye().extend(1.0).to_array(),
            base_color: [
                material.base_color[0],
                material.base_color[1],
                material.base_color[2],
                1.0,
            ],
            material: [
                material.metallic,
                material.roughness,
                settings.exposure_ev.exp2(),
                settings.tone_mapper.index() as f32,
            ],
            wire_color: [0.02, 0.02, 0.02, 1.0],
            viewport: [
                width as f32,
                height as f32,
                1.0 / width as f32,
                1.0 / height as f32,
            ],
            background,
            environment: [
                settings.environment_rotation,
                blur,
                if settings.shadows { 1.0 } else { 0.0 },
                0.0,
            ],
        }
    }
}

/// Size-dependent render targets.
struct Targets {
    width: u32,
    height: u32,
    msaa_color: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolved: wgpu::TextureView,
    output: wgpu::Texture,
    output_view: wgpu::TextureView,
    display_bind_group: wgpu::BindGroup,
}

/// What to draw.
pub struct Scene<'a> {
    /// The mesh, if a project is open.
    pub mesh: Option<&'a GpuMesh>,
    /// The camera.
    pub camera: &'a OrbitCamera,
    /// Display settings.
    pub settings: &'a ViewSettings,
}

/// Renders scenes to an off-screen image.
pub struct Viewport {
    frame_buffer: wgpu::Buffer,
    frame_layout: wgpu::BindGroupLayout,
    frame_bind_group: wgpu::BindGroup,
    display_layout: wgpu::BindGroupLayout,
    mesh_pipeline: wgpu::RenderPipeline,
    wire_pipeline: wgpu::RenderPipeline,
    display_pipeline: wgpu::RenderPipeline,
    targets: Option<Targets>,
}

fn texture(
    ctx: &GpuContext,
    label: &str,
    (width, height): (u32, u32),
    format: wgpu::TextureFormat,
    samples: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size_of::<FrameUniforms>() as u64),
        },
        count: None,
    }
}

fn scene_depth(write: bool) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        // Reverse-Z: closer is larger.
        depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

impl Viewport {
    /// Creates the pipelines.
    pub fn new(ctx: &GpuContext) -> Result<Self, GpuError> {
        let device = &ctx.device;
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport frame"),
            entries: &[uniform_entry(0)],
        });
        let display_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport display"),
            entries: &[
                uniform_entry(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport frame"),
            size: size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            }],
        });

        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport scene"),
            bind_group_layouts: &[Some(&frame_layout)],
            immediate_size: 0,
        });
        let msaa = wgpu::MultisampleState {
            count: SAMPLES,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };
        let hdr_target = [Some(wgpu::ColorTargetState {
            format: HDR_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let mesh_shader = ctx.create_shader_module("render/mesh_lit")?;
        let vertex_layouts: Vec<_> = VERTEX_LAYOUTS.iter().cloned().map(Some).collect();
        let mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewport mesh"),
            layout: Some(&scene_layout),
            vertex: wgpu::VertexState {
                module: &mesh_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &vertex_layouts,
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(scene_depth(true)),
            multisample: msaa,
            fragment: Some(wgpu::FragmentState {
                module: &mesh_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &hdr_target,
            }),
            multiview_mask: None,
            cache: None,
        });

        let wire_shader = ctx.create_shader_module("render/wireframe")?;
        let wire_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewport wireframe"),
            layout: Some(&scene_layout),
            vertex: wgpu::VertexState {
                module: &wire_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(VERTEX_LAYOUTS[0].clone())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(scene_depth(false)),
            multisample: msaa,
            fragment: Some(wgpu::FragmentState {
                module: &wire_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &hdr_target,
            }),
            multiview_mask: None,
            cache: None,
        });

        let display_shader = ctx.create_shader_module("render/tonemap")?;
        let display_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("viewport display"),
                bind_group_layouts: &[Some(&display_layout)],
                immediate_size: 0,
            });
        let display_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewport display"),
            layout: Some(&display_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &display_shader,
                entry_point: Some("vs_fullscreen"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &display_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: OUTPUT_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        Ok(Self {
            frame_buffer,
            frame_layout,
            frame_bind_group,
            display_layout,
            mesh_pipeline,
            wire_pipeline,
            display_pipeline,
            targets: None,
        })
    }

    /// (Re)allocates the targets for `width` × `height`. Returns whether they changed.
    fn ensure_targets(&mut self, ctx: &GpuContext, width: u32, height: u32) -> bool {
        if self
            .targets
            .as_ref()
            .is_some_and(|t| t.width == width && t.height == height)
        {
            return false;
        }
        let size = (width, height);
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let view = |texture: &wgpu::Texture| texture.create_view(&Default::default());
        let msaa_color = texture(
            ctx,
            "viewport msaa color",
            size,
            HDR_FORMAT,
            SAMPLES,
            attachment,
        );
        let depth = texture(
            ctx,
            "viewport depth",
            size,
            DEPTH_FORMAT,
            SAMPLES,
            attachment,
        );
        let resolved = texture(
            ctx,
            "viewport resolved",
            size,
            HDR_FORMAT,
            1,
            attachment | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let output = texture(
            ctx,
            "viewport output",
            size,
            OUTPUT_FORMAT,
            1,
            attachment | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let resolved_view = view(&resolved);
        let display_bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport display"),
            layout: &self.display_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.frame_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&resolved_view),
                },
            ],
        });
        self.targets = Some(Targets {
            width,
            height,
            msaa_color: view(&msaa_color),
            depth: view(&depth),
            resolved: resolved_view,
            output_view: view(&output),
            output,
            display_bind_group,
        });
        true
    }

    /// Renders `scene` at `width` × `height` (clamped to at least 1) and submits the work.
    /// Returns `true` when the output texture was reallocated.
    pub fn render(&mut self, ctx: &GpuContext, scene: &Scene<'_>, width: u32, height: u32) -> bool {
        puffin::profile_function!();
        let (width, height) = (width.max(1), height.max(1));
        let reallocated = self.ensure_targets(ctx, width, height);
        let uniforms = FrameUniforms::new(scene.camera, scene.settings, width, height);
        ctx.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniforms));
        let targets = self
            .targets
            .as_ref()
            .expect("ensure_targets always leaves targets allocated");

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("viewport"),
            });
        {
            let background = match scene.settings.background {
                Background::Solid(c) => wgpu::Color {
                    r: c[0].into(),
                    g: c[1].into(),
                    b: c[2].into(),
                    a: 1.0,
                },
                Background::Environment { .. } => wgpu::Color {
                    r: 0.05,
                    g: 0.05,
                    b: 0.05,
                    a: 1.0,
                },
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa_color,
                    depth_slice: None,
                    resolve_target: Some(&targets.resolved),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(background),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    // Reverse-Z: the far plane is 0.
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
            if let Some(mesh) = scene.mesh {
                pass.set_bind_group(0, &self.frame_bind_group, &[]);
                pass.set_pipeline(&self.mesh_pipeline);
                mesh.bind_vertices(&mut pass);
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                if scene.settings.wireframe {
                    pass.set_pipeline(&self.wire_pipeline);
                    pass.set_vertex_buffer(0, mesh.positions.slice(..));
                    pass.set_index_buffer(mesh.wire_indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..mesh.wire_index_count, 0, 0..1);
                }
            }
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport display"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.output_view,
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
            pass.set_pipeline(&self.display_pipeline);
            pass.set_bind_group(0, &targets.display_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        ctx.queue.submit([encoder.finish()]);
        reallocated
    }

    /// The last rendered image, `Rgba8Unorm` with sRGB-encoded values.
    pub fn output(&self) -> Option<(&wgpu::Texture, &wgpu::TextureView)> {
        self.targets.as_ref().map(|t| (&t.output, &t.output_view))
    }

    /// The bind group layout of the frame uniforms, for passes added by other modules.
    pub fn frame_layout(&self) -> &wgpu::BindGroupLayout {
        &self.frame_layout
    }
}
