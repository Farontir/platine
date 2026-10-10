//! GPU processing of an environment: cubemap with mips, GGX-prefiltered specular cube and
//! the split-sum BRDF lookup table. Every pass has a CPU reference in
//! [`crate::reference`]; diffuse lighting uses spherical harmonics computed on the CPU.
//!
//! Long passes are split into separate submissions (one per mip level and per band of
//! rows) so that no dispatch approaches the Windows driver timeout.

use patine_gpu::{GpuContext, GpuError, wgpu};
use wgpu::util::DeviceExt as _;

use crate::environment::{DirectionalLight, EquirectImage, Sh9, extract_dominant_light};

/// Format of the cubemaps.
pub const CUBE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Resolutions and sample counts of the processed environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvironmentSizes {
    /// Face size of the source cube (mip 0); mips go down to 1.
    pub cube: u32,
    /// Face size of mip 0 of the prefiltered specular cube.
    pub specular: u32,
    /// Number of specular mips, mapping roughness 0 to 1 linearly.
    pub specular_mips: u32,
    /// GGX samples per specular texel.
    pub samples: u32,
}

impl Default for EnvironmentSizes {
    fn default() -> Self {
        Self {
            cube: 512,
            specular: 256,
            specular_mips: 6,
            samples: 512,
        }
    }
}

/// Upper bound on GGX samples per submission (rows of a level are split to stay below).
const SAMPLES_PER_SUBMISSION: u64 = 1 << 26;

/// An environment ready for shading.
pub struct GpuEnvironment {
    /// Source radiance cube with a full mip chain (background and mirror reflections).
    pub cube: wgpu::Texture,
    /// Cube view of all mips of [`GpuEnvironment::cube`].
    pub cube_view: wgpu::TextureView,
    /// GGX-prefiltered radiance, mip `m` for roughness `m / (mips - 1)`.
    pub specular: wgpu::Texture,
    /// Cube view of all mips of [`GpuEnvironment::specular`].
    pub specular_view: wgpu::TextureView,
    /// Diffuse spherical harmonics of the whole environment (unrotated).
    pub sh: Sh9,
    /// Dominant light, unrotated, and the diffuse harmonics without it.
    pub light: DirectionalLight,
    /// [`GpuEnvironment::sh`] minus the dominant light.
    pub residual: Sh9,
    /// The sizes used.
    pub sizes: EnvironmentSizes,
}

fn mip_count(size: u32) -> u32 {
    32 - size.max(1).leading_zeros()
}

fn cube_texture(ctx: &GpuContext, label: &str, size: u32, mips: u32) -> wgpu::Texture {
    ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 6,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: CUBE_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn level_view(texture: &wgpu::Texture, mip: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("cube level"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        base_mip_level: mip,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

fn full_cube_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("cube"),
        dimension: Some(wgpu::TextureViewDimension::Cube),
        ..Default::default()
    })
}

fn storage_entry(
    binding: u32,
    dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: CUBE_FORMAT,
            view_dimension: dimension,
        },
        count: None,
    }
}

fn texture_entry(
    binding: u32,
    dimension: wgpu::TextureViewDimension,
    filterable: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// A compute pipeline with its bind group layout.
struct Compute {
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::ComputePipeline,
}

impl Compute {
    fn new(
        ctx: &GpuContext,
        shader: &str,
        entries: &[wgpu::BindGroupLayoutEntry],
    ) -> Result<Self, GpuError> {
        let module = ctx.create_shader_module(shader)?;
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(shader),
                entries,
            });
        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(shader),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
        let pipeline = ctx
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(shader),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
        Ok(Self { layout, pipeline })
    }

    /// Records and submits one dispatch.
    fn run(&self, ctx: &GpuContext, resources: &[wgpu::BindingResource<'_>], groups: [u32; 3]) {
        let entries: Vec<_> = resources
            .iter()
            .enumerate()
            .map(|(binding, resource)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: resource.clone(),
            })
            .collect();
        let bind_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &entries,
        });
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(groups[0], groups[1], groups[2]);
        }
        ctx.queue.submit([encoder.finish()]);
    }
}

fn groups(size: u32) -> u32 {
    size.div_ceil(8)
}

impl GpuEnvironment {
    /// Processes `image` on the GPU.
    pub fn new(
        ctx: &GpuContext,
        image: &EquirectImage,
        sizes: EnvironmentSizes,
    ) -> Result<Self, GpuError> {
        puffin::profile_function!();
        let device = &ctx.device;
        let d2 = wgpu::TextureViewDimension::D2;
        let d2_array = wgpu::TextureViewDimension::D2Array;

        // Source radiance in f32: HDRI suns exceed the range of f16.
        let source = device.create_texture_with_data(
            &ctx.queue,
            &wgpu::TextureDescriptor {
                label: Some("environment equirect"),
                size: wgpu::Extent3d {
                    width: image.width,
                    height: image.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytemuck::cast_slice(
                &image
                    .texels
                    .iter()
                    .map(|t| [t[0], t[1], t[2], 1.0])
                    .collect::<Vec<[f32; 4]>>(),
            ),
        );
        let source_view = source.create_view(&Default::default());

        let cube_mips = mip_count(sizes.cube);
        let cube = cube_texture(ctx, "environment cube", sizes.cube, cube_mips);
        let to_cube = Compute::new(
            ctx,
            "render/equirect_to_cube",
            &[texture_entry(0, d2, false), storage_entry(1, d2_array)],
        )?;
        let level0 = level_view(&cube, 0);
        to_cube.run(
            ctx,
            &[
                wgpu::BindingResource::TextureView(&source_view),
                wgpu::BindingResource::TextureView(&level0),
            ],
            [groups(sizes.cube), groups(sizes.cube), 6],
        );

        let downsample = Compute::new(
            ctx,
            "render/downsample_cube",
            &[
                texture_entry(0, d2_array, false),
                storage_entry(1, d2_array),
            ],
        )?;
        for mip in 1..cube_mips {
            let input = level_view(&cube, mip - 1);
            let output = level_view(&cube, mip);
            let size = (sizes.cube >> mip).max(1);
            downsample.run(
                ctx,
                &[
                    wgpu::BindingResource::TextureView(&input),
                    wgpu::BindingResource::TextureView(&output),
                ],
                [groups(size), groups(size), 6],
            );
        }
        let cube_view = full_cube_view(&cube);

        let specular = cube_texture(
            ctx,
            "environment specular",
            sizes.specular,
            sizes.specular_mips,
        );
        let prefilter = Compute::new(
            ctx,
            "render/prefilter",
            &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                storage_entry(2, d2_array),
                uniform_entry(3),
            ],
        )?;
        let sampler = linear_sampler(ctx);
        for mip in 0..sizes.specular_mips {
            let size = (sizes.specular >> mip).max(1);
            let roughness = if sizes.specular_mips > 1 {
                mip as f32 / (sizes.specular_mips - 1) as f32
            } else {
                0.0
            };
            let output = level_view(&specular, mip);
            let samples_per_row = u64::from(size) * 6 * u64::from(sizes.samples.max(1));
            let rows_per_submission = (SAMPLES_PER_SUBMISSION / samples_per_row)
                .max(8)
                .min(u64::from(size)) as u32;
            let mut row = 0;
            while row < size {
                let rows = rows_per_submission.min(size - row);
                let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("prefilter params"),
                    contents: bytemuck::cast_slice(&[
                        roughness.to_bits(),
                        sizes.samples,
                        (sizes.cube as f32).to_bits(),
                        row,
                    ]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                prefilter.run(
                    ctx,
                    &[
                        wgpu::BindingResource::TextureView(&cube_view),
                        wgpu::BindingResource::Sampler(&sampler),
                        wgpu::BindingResource::TextureView(&output),
                        params.as_entire_binding(),
                    ],
                    [groups(size), groups(rows), 6],
                );
                row += rows;
            }
        }
        let specular_view = full_cube_view(&specular);

        let sh = Sh9::project(image, 0.0);
        let (light, residual) = extract_dominant_light(&sh);
        Ok(Self {
            cube,
            cube_view,
            specular,
            specular_view,
            sh,
            light,
            residual,
            sizes,
        })
    }
}

/// Trilinear, clamp-to-edge sampler used for environment lookups.
pub fn linear_sampler(ctx: &GpuContext) -> wgpu::Sampler {
    ctx.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("environment"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    })
}

/// The split-sum BRDF lookup table (x = n·v, y = roughness, rg = scale and bias of F0).
pub struct BrdfLut {
    /// `Rgba16Float` texture (r = A, g = B).
    pub texture: wgpu::Texture,
    /// View of the texture.
    pub view: wgpu::TextureView,
}

impl BrdfLut {
    /// Integrates the table on the GPU.
    pub fn new(ctx: &GpuContext, size: u32, samples: u32) -> Result<Self, GpuError> {
        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("brdf lut"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CUBE_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let compute = Compute::new(
            ctx,
            "render/brdf_lut",
            &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: CUBE_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                uniform_entry(1),
            ],
        )?;
        let samples_per_row = u64::from(size) * u64::from(samples.max(1));
        let rows_per_submission = (SAMPLES_PER_SUBMISSION / samples_per_row)
            .max(8)
            .min(u64::from(size)) as u32;
        let mut row = 0;
        while row < size {
            let rows = rows_per_submission.min(size - row);
            let params = ctx
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("brdf lut params"),
                    // Padded to 16 bytes, the minimum uniform binding size.
                    contents: bytemuck::cast_slice(&[samples, row, 0, 0]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            compute.run(
                ctx,
                &[
                    wgpu::BindingResource::TextureView(&view),
                    params.as_entire_binding(),
                ],
                [groups(size), groups(rows), 1],
            );
            row += rows;
        }
        Ok(Self { texture, view })
    }
}
