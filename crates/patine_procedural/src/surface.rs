//! The mesh as the procedural passes read it, and the *surface map* of a texture set: for
//! every texel, the triangle whose surface the texel shows.
//!
//! The map is built in two steps. A render pass rasterizes the set's triangles in UV space
//! (a texel is covered when its center is inside a triangle). Jump flooding then gives
//! every uncovered texel the triangle of its nearest covered texel. The procedural shaders
//! take the point of that triangle closest to the texel center (see
//! [`crate::reference::surface`]), so that texels around the UV islands extend the
//! surface of their nearest triangle.

use bytemuck::{Pod, Zeroable};
use patine_core::mesh::MeshAsset;
use patine_gpu::{GpuContext, GpuError, wgpu};
use wgpu::util::DeviceExt as _;

/// One vertex in the storage buffer. Must match `Vertex` in
/// `shaders/procedural/mesh.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuVertex {
    /// xyz: position.
    pub position: [f32; 4],
    /// xyz: unit normal.
    pub normal: [f32; 4],
    /// xyz: tangent, w: bitangent sign.
    pub tangent: [f32; 4],
    /// xy: UV.
    pub uv: [f32; 4],
}

/// The vertices of `mesh` as the shaders read them (missing UVs and tangents are zero
/// and +x).
pub fn gpu_vertices(mesh: &MeshAsset) -> Vec<GpuVertex> {
    (0..mesh.positions.len())
        .map(|i| {
            let p = mesh.positions[i];
            let n = mesh.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]);
            let t = mesh
                .tangents
                .as_ref()
                .and_then(|t| t.get(i))
                .copied()
                .unwrap_or([1.0, 0.0, 0.0, 1.0]);
            let uv = mesh
                .uv0
                .as_ref()
                .and_then(|uv| uv.get(i))
                .copied()
                .unwrap_or([0.0, 0.0]);
            GpuVertex {
                position: [p[0], p[1], p[2], 1.0],
                normal: [n[0], n[1], n[2], 0.0],
                tangent: t,
                uv: [uv[0], uv[1], 0.0, 0.0],
            }
        })
        .collect()
}

/// The triangles of material `material`, in mesh order: triangle `i` of the surface map
/// is entry `i` here.
pub fn material_triangles(mesh: &MeshAsset, material: u32) -> Vec<[u32; 3]> {
    mesh.submeshes
        .iter()
        .filter(|submesh| submesh.material == material)
        .flat_map(|submesh| {
            mesh.triangles[submesh.triangles.start as usize..submesh.triangles.end as usize]
                .iter()
                .copied()
        })
        .collect()
}

/// Center of the mesh bounds and their largest extent (1 for an empty or flat mesh).
pub fn bounds_frame(mesh: &MeshAsset) -> ([f32; 3], f32) {
    match mesh.bounds() {
        Some(bounds) => {
            let center = std::array::from_fn(|i| 0.5 * (bounds.min[i] + bounds.max[i]));
            let extent = (0..3)
                .map(|i| bounds.max[i] - bounds.min[i])
                .fold(0.0f32, f32::max);
            (center, if extent > 0.0 { extent } else { 1.0 })
        }
        None => ([0.0; 3], 1.0),
    }
}

/// The mesh in storage buffers: every vertex, and the triangles of each material.
#[derive(Debug)]
pub struct MeshBuffers {
    /// Every vertex of the mesh.
    pub vertices: wgpu::Buffer,
    /// Vertex indices of each material's triangles, by material index.
    pub materials: Vec<(wgpu::Buffer, u32)>,
    /// Center of the mesh bounds.
    pub center: [f32; 3],
    /// Largest extent of the mesh bounds.
    pub extent: f32,
}

/// A storage buffer of `contents`, at least 16 bytes (storage bindings cannot be empty).
fn storage(ctx: &GpuContext, label: &str, contents: &[u8]) -> wgpu::Buffer {
    let mut bytes = contents.to_vec();
    if bytes.len() < 16 {
        bytes.resize(16, 0);
    }
    ctx.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: &bytes,
            usage: wgpu::BufferUsages::STORAGE,
        })
}

impl MeshBuffers {
    /// Uploads `mesh`.
    pub fn new(ctx: &GpuContext, mesh: &MeshAsset) -> Self {
        let vertices = storage(
            ctx,
            "procedural vertices",
            bytemuck::cast_slice(&gpu_vertices(mesh)),
        );
        let materials = (0..mesh.materials.len() as u32)
            .map(|material| {
                let triangles = material_triangles(mesh, material);
                let indices: Vec<u32> = triangles.iter().flatten().copied().collect();
                (
                    storage(ctx, "procedural indices", bytemuck::cast_slice(&indices)),
                    triangles.len() as u32,
                )
            })
            .collect();
        let (center, extent) = bounds_frame(mesh);
        Self {
            vertices,
            materials,
            center,
            extent,
        }
    }
}

/// Uniforms of `shaders/procedural/surface_jfa.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct JfaUniforms {
    mode: u32,
    step: i32,
    size: u32,
    padding: u32,
}

/// Format of the surface map.
pub const SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

/// Usage of a surface map.
pub const SURFACE_USAGE: wgpu::TextureUsages = wgpu::TextureUsages::TEXTURE_BINDING
    .union(wgpu::TextureUsages::RENDER_ATTACHMENT)
    .union(wgpu::TextureUsages::COPY_SRC);

/// Descriptor of a `size` × `size` surface map.
pub fn surface_descriptor(size: u32) -> wgpu::TextureDescriptor<'static> {
    wgpu::TextureDescriptor {
        label: Some("surface map"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SURFACE_FORMAT,
        usage: SURFACE_USAGE,
        view_formats: &[],
    }
}

/// A render pass clearing `target` to 0.
fn clear_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("surface map"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

/// Pipelines building surface maps.
#[derive(Debug)]
pub struct SurfaceBuilder {
    raster: wgpu::RenderPipeline,
    raster_layout: wgpu::BindGroupLayout,
    jfa: wgpu::RenderPipeline,
    jfa_layout: wgpu::BindGroupLayout,
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

fn uint_texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

impl SurfaceBuilder {
    /// Creates the pipelines.
    pub fn new(ctx: &GpuContext) -> Result<Self, GpuError> {
        let device = &ctx.device;
        let raster_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surface raster"),
            entries: &[
                storage_entry(0, wgpu::ShaderStages::VERTEX),
                storage_entry(1, wgpu::ShaderStages::VERTEX),
            ],
        });
        let jfa_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("surface jfa"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                uint_texture_entry(1),
                uint_texture_entry(2),
            ],
        });
        let pipeline = |name: &str, layout: &wgpu::BindGroupLayout, vertex: &str| {
            let module = ctx.create_shader_module(name)?;
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(name),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            Ok::<_, GpuError>(
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(name),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some(vertex),
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
                            format: SURFACE_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                }),
            )
        };
        Ok(Self {
            raster: pipeline("procedural/surface_raster", &raster_layout, "vs_main")?,
            jfa: pipeline("procedural/surface_jfa", &jfa_layout, "vs_fullscreen")?,
            raster_layout,
            jfa_layout,
        })
    }

    /// Encodes the surface map of material `material` of `mesh` into `target` (a square
    /// texture of [`surface_descriptor`]). Working textures live until the commands run.
    pub fn build(
        &self,
        ctx: &GpuContext,
        encoder: &mut wgpu::CommandEncoder,
        mesh: &MeshBuffers,
        material: u32,
        target: &wgpu::Texture,
    ) {
        let size = target.width();
        let device = &ctx.device;
        let work = |label| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                ..surface_descriptor(size)
            })
        };
        let raster = work("surface raster");
        let ping = [work("surface jfa a"), work("surface jfa b")];
        let view = |texture: &wgpu::Texture| texture.create_view(&Default::default());
        let pass = clear_pass;

        // Triangles in UV space.
        if let Some((indices, triangles)) = mesh.materials.get(material as usize) {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("surface raster"),
                layout: &self.raster_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: mesh.vertices.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: indices.as_entire_binding(),
                    },
                ],
            });
            let target = view(&raster);
            let mut pass = pass(encoder, &target);
            pass.set_pipeline(&self.raster);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..triangles * 3, 0..1);
        } else {
            pass(encoder, &view(&raster));
        }

        // Jump flooding: seeds, steps of size/2 down to 1, one more step of 1, resolve.
        let mut steps = Vec::new();
        let mut step = (size / 2).max(1) as i32;
        while step >= 1 {
            steps.push(step);
            step /= 2;
        }
        steps.push(1);
        let jfa = |encoder: &mut wgpu::CommandEncoder,
                   mode: u32,
                   step: i32,
                   source: &wgpu::Texture,
                   seeds: &wgpu::Texture,
                   target: &wgpu::Texture| {
            let uniforms = JfaUniforms {
                mode,
                step,
                size,
                padding: 0,
            };
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("surface jfa"),
                contents: bytemuck::bytes_of(&uniforms),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let (source, seeds) = (view(source), view(seeds));
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("surface jfa"),
                layout: &self.jfa_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&seeds),
                    },
                ],
            });
            let target = view(target);
            let mut pass = pass(encoder, &target);
            pass.set_pipeline(&self.jfa);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        };
        jfa(encoder, 0, 0, &raster, &raster, &ping[0]);
        let mut current = 0;
        for step in steps {
            jfa(
                encoder,
                1,
                step,
                &ping[current],
                &raster,
                &ping[1 - current],
            );
            current = 1 - current;
        }
        jfa(encoder, 2, 0, &raster, &ping[current], target);
    }
}
