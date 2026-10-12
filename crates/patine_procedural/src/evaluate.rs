//! Evaluation of a procedural into a texture: one fullscreen pass of its shader, which
//! reads the surface of each texel (see [`crate::surface`]), projects it and evaluates
//! the procedural there.

use std::cell::RefCell;
use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use patine_core::layer::{Projection, ProjectionKind};
use patine_core::mask::Params;
use patine_gpu::{GpuContext, GpuError, wgpu};
use wgpu::util::DeviceExt as _;

use crate::def::{DefError, Output, ProceduralDef};
use crate::surface::{MeshBuffers, surface_descriptor};

/// Uniforms shared by every procedural pass. Must match `Evaluation` in
/// `shaders/procedural/evaluate.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct EvaluationUniforms {
    /// 0 UV, 1 triplanar, 2 planar, 3 spherical, 4 cylindrical.
    pub kind: u32,
    /// Triplanar blend.
    pub blend: f32,
    /// 1: the surface is the unit square.
    pub flat: u32,
    /// Padding.
    pub padding3: u32,
    /// Projection scale.
    pub scale: [f32; 2],
    /// Projection offset.
    pub offset: [f32; 2],
    /// cos and sin of the projection rotation.
    pub rotation: [f32; 2],
    /// Integer repetitions when the projection tiles, (0, 0) otherwise.
    pub tile: [i32; 2],
    /// xyz: center of the mesh bounds, w: their largest extent.
    pub center: [f32; 4],
    /// Side of the target, in texels.
    pub size: f32,
    /// Padding to 16 bytes.
    pub padding: [f32; 3],
}

impl EvaluationUniforms {
    /// The uniforms of `projection` for a `size` × `size` target, on a mesh whose bounds
    /// have `center` and largest extent `extent` (`flat`: on the unit square instead).
    pub fn new(
        projection: &Projection,
        size: u32,
        center: [f32; 3],
        extent: f32,
        flat: bool,
    ) -> Self {
        let (kind, blend) = match projection.kind {
            ProjectionKind::Uv => (0, 0.0),
            ProjectionKind::Triplanar { blend } => (1, blend.clamp(0.0, 1.0)),
            ProjectionKind::Planar => (2, 0.0),
            ProjectionKind::Spherical => (3, 0.0),
            ProjectionKind::Cylindrical => (4, 0.0),
        };
        // The projections that wrap around (UV, longitude) tile with whole repetitions.
        let integral = |v: f32| v >= 1.0 && v.fract() == 0.0;
        let tiles = matches!(kind, 0 | 3 | 4)
            && projection.rotation == 0.0
            && projection.scale.iter().all(|s| integral(*s));
        Self {
            kind,
            blend,
            flat: u32::from(flat),
            padding3: 0,
            scale: projection.scale,
            offset: projection.offset,
            rotation: [projection.rotation.cos(), projection.rotation.sin()],
            tile: if tiles {
                [projection.scale[0] as i32, projection.scale[1] as i32]
            } else {
                [0, 0]
            },
            center: [center[0], center[1], center[2], extent],
            size: size as f32,
            padding: [0.0; 3],
        }
    }
}

/// Format of the texture of `output`: `Rgba16Float` for colors and normals,
/// `R16Float` for scalars and gray values (the formats of the channel textures). With
/// these, the five outputs of a material take 22 bytes per texel, within the 32 bytes a
/// render pass may write.
pub fn output_format(output: Output) -> wgpu::TextureFormat {
    use patine_core::channel::Channel;
    match output {
        Output::Channel(Channel::BaseColor | Channel::Emissive | Channel::Normal) => {
            wgpu::TextureFormat::Rgba16Float
        }
        _ => wgpu::TextureFormat::R16Float,
    }
}

/// The formats of every output of `def`, in order.
pub fn output_formats(def: &ProceduralDef) -> Vec<wgpu::TextureFormat> {
    def.outputs
        .iter()
        .map(|output| output_format(*output))
        .collect()
}

/// What the surface of a pass is.
#[derive(Clone, Copy, Debug)]
pub enum SurfaceInput<'a> {
    /// A texture set of a mesh: its buffers, material index and surface map.
    Mesh {
        /// The mesh.
        mesh: &'a MeshBuffers,
        /// Material (texture set) index.
        material: u32,
        /// Surface map of the texture set, at the size of the target.
        map: &'a wgpu::TextureView,
    },
    /// The unit square (thumbnails, tests).
    Flat,
}

/// Errors of an evaluation.
#[derive(Debug, thiserror::Error)]
pub enum EvaluateError {
    /// The definition cannot be packed.
    #[error(transparent)]
    Def(#[from] DefError),
    /// The shader does not compile.
    #[error(transparent)]
    Gpu(#[from] GpuError),
    /// The pass needs one target per output of the procedural.
    #[error("{expected} targets needed, {given} given")]
    Targets {
        /// Outputs of the procedural.
        expected: usize,
        /// Targets given.
        given: usize,
    },
}

/// Pipelines of the procedurals, created on first use for each shader and target format.
#[derive(Debug)]
pub struct Procedurals {
    uniforms_layout: wgpu::BindGroupLayout,
    surface_layout: wgpu::BindGroupLayout,
    layout: wgpu::PipelineLayout,
    pipelines: RefCell<HashMap<(String, Vec<wgpu::TextureFormat>), wgpu::RenderPipeline>>,
    /// Bound as the surface of flat passes.
    empty_buffer: wgpu::Buffer,
    empty_map: wgpu::TextureView,
    passes: std::cell::Cell<u64>,
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl Procedurals {
    /// Creates the shared layouts; pipelines come with their first use.
    pub fn new(ctx: &GpuContext) -> Self {
        let device = &ctx.device;
        let uniforms_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("procedural uniforms"),
            entries: &[uniform_entry(0), uniform_entry(1)],
        });
        let surface_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("procedural surface"),
            entries: &[
                storage_entry(0),
                storage_entry(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("procedural"),
            bind_group_layouts: &[Some(&uniforms_layout), Some(&surface_layout)],
            immediate_size: 0,
        });
        let empty_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("procedural empty"),
            size: 64,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let empty_map = device
            .create_texture(&surface_descriptor(1))
            .create_view(&Default::default());
        Self {
            uniforms_layout,
            surface_layout,
            layout,
            pipelines: RefCell::new(HashMap::new()),
            empty_buffer,
            empty_map,
            passes: std::cell::Cell::new(0),
        }
    }

    /// Number of procedural passes encoded since creation (tests of the content cache).
    pub fn passes(&self) -> u64 {
        self.passes.get()
    }

    fn pipeline(
        &self,
        ctx: &GpuContext,
        shader: &str,
        formats: &[wgpu::TextureFormat],
    ) -> Result<wgpu::RenderPipeline, GpuError> {
        let key = (shader.to_owned(), formats.to_vec());
        if let Some(pipeline) = self.pipelines.borrow().get(&key) {
            return Ok(pipeline.clone());
        }
        let module = ctx.create_shader_module(shader)?;
        let targets: Vec<Option<wgpu::ColorTargetState>> = formats
            .iter()
            .map(|format| {
                Some(wgpu::ColorTargetState {
                    format: *format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })
            })
            .collect();
        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(shader),
                layout: Some(&self.layout),
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
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            });
        self.pipelines.borrow_mut().insert(key, pipeline.clone());
        Ok(pipeline)
    }

    /// Encodes `def` with `params` (resolved values) for `uniforms` on `surface`, in one
    /// pass writing every output of the procedural: `targets` holds, in the order of
    /// `def.outputs`, mip 0 of square textures of the same size with their format.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        def: &ProceduralDef,
        params: &Params,
        uniforms: &EvaluationUniforms,
        surface: SurfaceInput<'_>,
        targets: &[(&wgpu::TextureView, wgpu::TextureFormat)],
    ) -> Result<(), EvaluateError> {
        if targets.len() != def.outputs.len() {
            return Err(EvaluateError::Targets {
                expected: def.outputs.len(),
                given: targets.len(),
            });
        }
        let formats: Vec<wgpu::TextureFormat> = targets.iter().map(|(_, f)| *f).collect();
        let pipeline = self.pipeline(ctx, &def.shader, &formats)?;
        let device = &ctx.device;
        let evaluation = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("procedural evaluation"),
            contents: bytemuck::bytes_of(uniforms),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("procedural params"),
            contents: &def.pack(params)?,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let uniforms_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("procedural uniforms"),
            layout: &self.uniforms_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: evaluation.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        let (vertices, indices, map) = match surface {
            SurfaceInput::Mesh {
                mesh,
                material,
                map,
            } => match mesh.materials.get(material as usize) {
                Some((indices, _)) => (&mesh.vertices, indices, map),
                None => (&self.empty_buffer, &self.empty_buffer, &self.empty_map),
            },
            SurfaceInput::Flat => (&self.empty_buffer, &self.empty_buffer, &self.empty_map),
        };
        let surface_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("procedural surface"),
            layout: &self.surface_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: vertices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: indices.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(map),
                },
            ],
        });
        self.passes.set(self.passes.get() + 1);
        let attachments: Vec<Option<wgpu::RenderPassColorAttachment>> = targets
            .iter()
            .map(|(view, _)| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })
            })
            .collect();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("procedural"),
            color_attachments: &attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &uniforms_group, &[]);
        pass.set_bind_group(1, &surface_group, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_procedural_fits_in_one_pass() {
        for def in crate::library::all() {
            let bytes: u32 = output_formats(def)
                .iter()
                .map(|format| format.target_pixel_byte_cost().expect("color formats"))
                .sum();
            // The default limit of wgpu (max_color_attachment_bytes_per_sample).
            assert!(bytes <= 32, "{}: {bytes} bytes per texel", def.id);
        }
    }
}
