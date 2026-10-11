//! The viewport renderer: draws the environment and the mesh off screen and produces an
//! sRGB-encoded `Rgba8Unorm` image, the format egui displays as is.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use patine_core::mesh::BoundingBox;
use patine_gpu::{GpuContext, GpuError, wgpu};

use crate::camera::{OrbitCamera, look_at_rh, orthographic_rh};
use crate::environment::{EquirectImage, studio_sky};
use crate::environment_gpu::{BrdfLut, EnvironmentSizes, GpuEnvironment, linear_sampler};
use crate::material::{GpuMaterial, MaterialResources, MaterialTextures};
use crate::mesh_gpu::{GpuMesh, VERTEX_LAYOUTS};
use crate::picking::{PickImage, PickResult, Picker};
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
    light_view_proj: [[f32; 4]; 4],
}

/// Shadow projection of the dominant light, fitted to the mesh bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightProjection {
    /// World to shadow clip space (standard depth: 0 near, 1 far).
    pub view_proj: Mat4,
    /// World size of a shadow map texel.
    pub texel_world: f32,
}

impl LightProjection {
    /// Orthographic projection along `direction` (towards the light) enclosing the
    /// bounding sphere of `bounds`, for a `size`² shadow map.
    pub fn new(direction: Vec3, bounds: &BoundingBox, size: u32) -> Self {
        let min = Vec3::from(bounds.min);
        let max = Vec3::from(bounds.max);
        let center = (min + max) * 0.5;
        let radius = ((max - min).length() * 0.5).max(1e-3);
        let direction = direction.try_normalize().unwrap_or(Vec3::Y);
        let up = if direction.y.abs() > 0.99 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let eye = center + direction * (2.0 * radius);
        let view = look_at_rh(eye, center, up);
        let projection = orthographic_rh(-radius, radius, -radius, radius, 0.0, 4.0 * radius);
        Self {
            view_proj: projection * view,
            texel_world: 2.0 * radius / size as f32,
        }
    }
}

impl FrameUniforms {
    fn new(
        camera: &OrbitCamera,
        settings: &ViewSettings,
        light: LightProjection,
        width: u32,
        height: u32,
    ) -> Self {
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
                light.texel_world,
            ],
            light_view_proj: light.view_proj.to_cols_array_2d(),
        }
    }
}

/// Environment uniforms. Must match `Environment` in `shaders/render/environment.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct EnvironmentUniforms {
    sh: [[f32; 4]; 9],
    residual: [[f32; 4]; 9],
    light_direction: [f32; 4],
    light_color: [f32; 4],
    mips: [f32; 4],
}

/// Direction towards the dominant light of `environment` rotated by `rotation`, like
/// `Sh9::project(_, rotation)`: the light turns with the environment.
fn light_direction(environment: &GpuEnvironment, rotation: f32) -> Vec3 {
    let d = environment.light.direction;
    let (sin, cos) = rotation.sin_cos();
    Vec3::new(cos * d.x + sin * d.z, d.y, -sin * d.x + cos * d.z)
}

impl EnvironmentUniforms {
    fn new(environment: &GpuEnvironment, rotation: f32) -> Self {
        let pack = |c: Vec3| c.extend(0.0).to_array();
        let sh = environment.sh.diffuse_coefficients().map(pack);
        let residual = environment.residual.diffuse_coefficients().map(pack);
        let direction = light_direction(environment, rotation);
        Self {
            sh,
            residual,
            light_direction: pack(direction),
            light_color: pack(environment.light.color),
            mips: [
                environment.sizes.specular_mips as f32,
                environment.cube.mip_level_count() as f32,
                0.0,
                0.0,
            ],
        }
    }
}

/// Resolutions of the viewport's precomputed resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewportOptions {
    /// Size of the BRDF lookup table.
    pub lut_size: u32,
    /// Samples per texel of the lookup table.
    pub lut_samples: u32,
    /// Environment processing sizes.
    pub environment: EnvironmentSizes,
    /// Width of the default procedural sky.
    pub sky_width: u32,
    /// Size of the shadow map.
    pub shadow_map_size: u32,
    /// Use the picking fallback even when PRIMITIVE_INDEX is available (tests).
    pub force_flat_picking: bool,
}

impl Default for ViewportOptions {
    fn default() -> Self {
        Self {
            lut_size: 256,
            lut_samples: 512,
            environment: EnvironmentSizes::default(),
            sky_width: 1024,
            shadow_map_size: 2048,
            force_flat_picking: false,
        }
    }
}

impl ViewportOptions {
    /// Small resources for tests on software adapters.
    pub fn small() -> Self {
        Self {
            lut_size: 32,
            lut_samples: 256,
            environment: EnvironmentSizes {
                cube: 32,
                specular: 16,
                specular_mips: 5,
                samples: 256,
            },
            sky_width: 128,
            shadow_map_size: 256,
            force_flat_picking: false,
        }
    }
}

/// Resources bound in the environment bind group.
struct EnvironmentResources<'a> {
    environment: &'a GpuEnvironment,
    lut: &'a BrdfLut,
    sampler: &'a wgpu::Sampler,
    shadow_map: &'a wgpu::TextureView,
    shadow_sampler: &'a wgpu::Sampler,
}

/// Size-dependent render targets.
struct Targets {
    width: u32,
    height: u32,
    msaa_color: wgpu::TextureView,
    depth: wgpu::TextureView,
    output: wgpu::Texture,
    output_view: wgpu::TextureView,
    resolved_texture: wgpu::Texture,
    resolved: wgpu::TextureView,
    display_bind_group: wgpu::BindGroup,
}

/// What to draw.
pub struct Scene<'a> {
    /// The mesh, if a project is open.
    pub mesh: Option<&'a GpuMesh>,
    /// Material of each texture set, by material index; `None` (or a missing entry) uses
    /// the constant material of the settings.
    pub materials: Option<&'a [GpuMaterial]>,
    /// The camera.
    pub camera: &'a OrbitCamera,
    /// Display settings.
    pub settings: &'a ViewSettings,
}

/// Renders scenes to an off-screen image.
pub struct Viewport {
    options: ViewportOptions,
    frame_buffer: wgpu::Buffer,
    frame_bind_group: wgpu::BindGroup,
    environment_layout: wgpu::BindGroupLayout,
    environment_buffer: wgpu::Buffer,
    environment_bind_group: wgpu::BindGroup,
    environment: GpuEnvironment,
    lut: BrdfLut,
    sampler: wgpu::Sampler,
    shadow_map: wgpu::TextureView,
    shadow_sampler: wgpu::Sampler,
    shadow_pipeline: wgpu::RenderPipeline,
    display_layout: wgpu::BindGroupLayout,
    background_pipeline: wgpu::RenderPipeline,
    mesh_pipeline: wgpu::RenderPipeline,
    wire_pipeline: wgpu::RenderPipeline,
    display_pipeline: wgpu::RenderPipeline,
    targets: Option<Targets>,
    picker: Picker,
    materials: MaterialResources,
    constant_material: GpuMaterial,
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

fn uniform_entry(binding: u32, size: usize) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size as u64),
        },
        count: None,
    }
}

fn fragment_texture_entry(
    binding: u32,
    dimension: wgpu::TextureViewDimension,
    filterable: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}

fn scene_depth(write: bool, compare: wgpu::CompareFunction) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        depth_compare: Some(compare),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

/// Description of a render pipeline, filled with the viewport's shared state.
struct PipelineDesc<'a> {
    label: &'a str,
    shader: &'a str,
    vertex_entry: &'a str,
    layout: &'a wgpu::PipelineLayout,
    buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
    primitive: wgpu::PrimitiveState,
    depth: Option<wgpu::DepthStencilState>,
    samples: u32,
    format: wgpu::TextureFormat,
}

fn render_pipeline(
    ctx: &GpuContext,
    desc: PipelineDesc<'_>,
) -> Result<wgpu::RenderPipeline, GpuError> {
    let module = ctx.create_shader_module(desc.shader)?;
    Ok(ctx
        .device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(desc.label),
            layout: Some(desc.layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some(desc.vertex_entry),
                compilation_options: Default::default(),
                buffers: desc.buffers,
            },
            primitive: desc.primitive,
            depth_stencil: desc.depth,
            multisample: wgpu::MultisampleState {
                count: desc.samples,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: desc.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        }))
}

impl Viewport {
    /// Creates the pipelines and precomputed resources at their default resolutions.
    pub fn new(ctx: &GpuContext) -> Result<Self, GpuError> {
        Self::with_options(ctx, ViewportOptions::default())
    }

    /// Creates the pipelines and precomputed resources.
    pub fn with_options(ctx: &GpuContext, options: ViewportOptions) -> Result<Self, GpuError> {
        let device = &ctx.device;
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport frame"),
            entries: &[uniform_entry(0, size_of::<FrameUniforms>())],
        });
        let environment_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("viewport environment"),
                entries: &[
                    uniform_entry(0, size_of::<EnvironmentUniforms>()),
                    fragment_texture_entry(1, wgpu::TextureViewDimension::Cube, true),
                    fragment_texture_entry(2, wgpu::TextureViewDimension::Cube, true),
                    fragment_texture_entry(3, wgpu::TextureViewDimension::D2, true),
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 6,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                        count: None,
                    },
                ],
            });
        let display_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport display"),
            entries: &[
                uniform_entry(0, size_of::<FrameUniforms>()),
                fragment_texture_entry(1, wgpu::TextureViewDimension::D2, false),
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
        let environment_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewport environment"),
            size: size_of::<EnvironmentUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let lut = BrdfLut::new(ctx, options.lut_size, options.lut_samples)?;
        let sampler = linear_sampler(ctx);
        let shadow_map = texture(
            ctx,
            "viewport shadow map",
            (options.shadow_map_size, options.shadow_map_size),
            DEPTH_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        )
        .create_view(&Default::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("viewport shadow"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let environment =
            GpuEnvironment::new(ctx, &studio_sky(options.sky_width), options.environment)?;
        let environment_bind_group = Self::environment_bind_group(
            ctx,
            &environment_layout,
            &environment_buffer,
            EnvironmentResources {
                environment: &environment,
                lut: &lut,
                sampler: &sampler,
                shadow_map: &shadow_map,
                shadow_sampler: &shadow_sampler,
            },
        );

        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport scene"),
            bind_group_layouts: &[Some(&frame_layout), Some(&environment_layout)],
            immediate_size: 0,
        });
        let materials = MaterialResources::new(ctx);
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport mesh"),
            bind_group_layouts: &[
                Some(&frame_layout),
                Some(&environment_layout),
                Some(&materials.layout),
            ],
            immediate_size: 0,
        });
        let display_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("viewport display"),
                bind_group_layouts: &[Some(&display_layout)],
                immediate_size: 0,
            });
        let mesh_buffers: Vec<_> = VERTEX_LAYOUTS.iter().cloned().map(Some).collect();
        let reverse_z = wgpu::CompareFunction::GreaterEqual;

        let background_pipeline = render_pipeline(
            ctx,
            PipelineDesc {
                label: "viewport background",
                shader: "render/background",
                vertex_entry: "vs_fullscreen",
                layout: &scene_layout,
                buffers: &[],
                primitive: wgpu::PrimitiveState::default(),
                depth: Some(scene_depth(false, wgpu::CompareFunction::Always)),
                samples: SAMPLES,
                format: HDR_FORMAT,
            },
        )?;
        let mesh_pipeline = render_pipeline(
            ctx,
            PipelineDesc {
                label: "viewport mesh",
                shader: "render/pbr",
                vertex_entry: "vs_main",
                layout: &mesh_layout,
                buffers: &mesh_buffers,
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth: Some(scene_depth(true, reverse_z)),
                samples: SAMPLES,
                format: HDR_FORMAT,
            },
        )?;
        let wire_pipeline = render_pipeline(
            ctx,
            PipelineDesc {
                label: "viewport wireframe",
                shader: "render/wireframe",
                vertex_entry: "vs_main",
                layout: &scene_layout,
                buffers: &[Some(VERTEX_LAYOUTS[0].clone())],
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::LineList,
                    ..Default::default()
                },
                depth: Some(scene_depth(false, reverse_z)),
                samples: SAMPLES,
                format: HDR_FORMAT,
            },
        )?;
        let picker = Picker::new(ctx, &frame_layout, options.force_flat_picking)?;
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewport shadow"),
            bind_group_layouts: &[Some(&frame_layout)],
            immediate_size: 0,
        });
        let shadow_module = ctx.create_shader_module("render/shadow")?;
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewport shadow"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shadow_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(VERTEX_LAYOUTS[0].clone())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                // Slope-scaled bias against acne on surfaces grazing the light.
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let display_pipeline = render_pipeline(
            ctx,
            PipelineDesc {
                label: "viewport display",
                shader: "render/tonemap",
                vertex_entry: "vs_fullscreen",
                layout: &display_pipeline_layout,
                buffers: &[],
                primitive: wgpu::PrimitiveState::default(),
                depth: None,
                samples: 1,
                format: OUTPUT_FORMAT,
            },
        )?;

        Ok(Self {
            options,
            frame_buffer,
            frame_bind_group,
            environment_layout,
            environment_buffer,
            environment_bind_group,
            environment,
            lut,
            sampler,
            shadow_map,
            shadow_sampler,
            shadow_pipeline,
            display_layout,
            background_pipeline,
            mesh_pipeline,
            wire_pipeline,
            display_pipeline,
            targets: None,
            picker,
            constant_material: materials.constant(ctx),
            materials,
        })
    }

    /// Binds the channel textures of a texture set for the mesh shader; missing slots use
    /// neutral values.
    pub fn material(&self, ctx: &GpuContext, textures: &MaterialTextures<'_>) -> GpuMaterial {
        self.materials.document(ctx, textures)
    }

    fn environment_bind_group(
        ctx: &GpuContext,
        layout: &wgpu::BindGroupLayout,
        buffer: &wgpu::Buffer,
        resources: EnvironmentResources<'_>,
    ) -> wgpu::BindGroup {
        let EnvironmentResources {
            environment,
            lut,
            sampler,
            shadow_map,
            shadow_sampler,
        } = resources;
        ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewport environment"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&environment.specular_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&environment.cube_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&lut.view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(shadow_map),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(shadow_sampler),
                },
            ],
        })
    }

    /// Processes `image` and uses it as the environment.
    pub fn set_environment(
        &mut self,
        ctx: &GpuContext,
        image: &EquirectImage,
    ) -> Result<(), GpuError> {
        self.environment = GpuEnvironment::new(ctx, image, self.options.environment)?;
        self.environment_bind_group = Self::environment_bind_group(
            ctx,
            &self.environment_layout,
            &self.environment_buffer,
            EnvironmentResources {
                environment: &self.environment,
                lut: &self.lut,
                sampler: &self.sampler,
                shadow_map: &self.shadow_map,
                shadow_sampler: &self.shadow_sampler,
            },
        );
        Ok(())
    }

    /// The current environment.
    pub fn environment(&self) -> &GpuEnvironment {
        &self.environment
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
            attachment | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
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
            output_view: view(&output),
            output,
            resolved_texture: resolved,
            resolved: resolved_view,
            display_bind_group,
        });
        true
    }

    /// Writes the frame uniforms of `scene` for a `width` × `height` image.
    fn write_frame(&self, ctx: &GpuContext, scene: &Scene<'_>, width: u32, height: u32) {
        self.constant_material
            .set_constant(ctx, &scene.settings.material);
        let rotation = scene.settings.environment_rotation;
        let light = match scene.mesh {
            Some(mesh) => LightProjection::new(
                light_direction(&self.environment, rotation),
                &mesh.bounds,
                self.options.shadow_map_size,
            ),
            None => LightProjection {
                view_proj: Mat4::IDENTITY,
                texel_world: 0.0,
            },
        };
        let frame = FrameUniforms::new(scene.camera, scene.settings, light, width, height);
        ctx.queue
            .write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&frame));
    }

    /// Renders the picking buffer of `scene` at `width` × `height` and reads it back.
    /// Blocks; meant for tests and tools. Returns `None` without a mesh.
    pub fn pick_image(
        &mut self,
        ctx: &GpuContext,
        scene: &Scene<'_>,
        width: u32,
        height: u32,
    ) -> Result<Option<PickImage>, GpuError> {
        let Some(mesh) = scene.mesh else {
            return Ok(None);
        };
        self.write_frame(ctx, scene, width, height);
        self.picker
            .pick_image(ctx, &self.frame_bind_group, mesh, width, height)
            .map(Some)
    }

    /// Starts picking `pixel` of a `width` × `height` image of `scene`; the result comes
    /// from [`Viewport::poll_pick`], usually at the next frame. Returns `false` when the
    /// request is not sent (previous request in flight, no mesh, pixel outside).
    pub fn request_pick(
        &mut self,
        ctx: &GpuContext,
        scene: &Scene<'_>,
        width: u32,
        height: u32,
        pixel: [u32; 2],
    ) -> bool {
        let Some(mesh) = scene.mesh else {
            return false;
        };
        self.write_frame(ctx, scene, width, height);
        let view_proj = scene.camera.view_projection(width as f32 / height as f32);
        self.picker.request(
            ctx,
            &self.frame_bind_group,
            mesh,
            [width, height],
            pixel,
            view_proj,
        )
    }

    /// Result of the last [`Viewport::request_pick`], once the GPU has finished it.
    pub fn poll_pick(&mut self, ctx: &GpuContext) -> Option<PickResult> {
        self.picker.poll(ctx)
    }

    /// Whether a pick request is waiting for the GPU.
    pub fn pick_pending(&self) -> bool {
        self.picker.is_pending()
    }

    /// Whether meshes must be uploaded with [`GpuMesh::with_flat_picking`] for picking.
    pub fn needs_flat_pick_buffers(&self) -> bool {
        self.picker.needs_flat_buffers()
    }

    /// Renders `scene` at `width` × `height` (clamped to at least 1) and submits the work.
    /// Returns `true` when the output texture was reallocated.
    pub fn render(&mut self, ctx: &GpuContext, scene: &Scene<'_>, width: u32, height: u32) -> bool {
        puffin::profile_function!();
        let (width, height) = (width.max(1), height.max(1));
        let reallocated = self.ensure_targets(ctx, width, height);
        let shadows = scene.settings.shadows && scene.mesh.is_some();
        self.write_frame(ctx, scene, width, height);
        let environment =
            EnvironmentUniforms::new(&self.environment, scene.settings.environment_rotation);
        ctx.queue.write_buffer(
            &self.environment_buffer,
            0,
            bytemuck::bytes_of(&environment),
        );
        let targets = self
            .targets
            .as_ref()
            .expect("ensure_targets always leaves targets allocated");

        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("viewport"),
            });
        if shadows && let Some(mesh) = scene.mesh {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport shadow"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_map,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            pass.set_vertex_buffer(0, mesh.positions.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewport scene"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa_color,
                    depth_slice: None,
                    resolve_target: Some(&targets.resolved),
                    ops: wgpu::Operations {
                        // The background covers every pixel.
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
            pass.set_bind_group(0, &self.frame_bind_group, &[]);
            pass.set_bind_group(1, &self.environment_bind_group, &[]);
            pass.set_pipeline(&self.background_pipeline);
            pass.draw(0..3, 0..1);
            if let Some(mesh) = scene.mesh {
                pass.set_pipeline(&self.mesh_pipeline);
                mesh.bind_vertices(&mut pass);
                pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
                // One draw per texture set, with its material.
                for submesh in &mesh.submeshes {
                    let material = scene
                        .materials
                        .and_then(|materials| materials.get(submesh.material as usize))
                        .unwrap_or(&self.constant_material);
                    pass.set_bind_group(2, &material.bind_group, &[]);
                    pass.draw_indexed(submesh.indices.clone(), 0, 0..1);
                }
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

    /// The last scene radiance before exposure and tone mapping (`Rgba16Float`, linear).
    pub fn hdr_output(&self) -> Option<&wgpu::Texture> {
        self.targets.as_ref().map(|t| &t.resolved_texture)
    }
}
