//! Mesh data uploaded to the GPU.

use patine_core::mesh::{BoundingBox, MeshAsset};
use patine_gpu::{GpuContext, wgpu};
use wgpu::util::DeviceExt as _;

/// Ranges of one submesh (one material, one texture set) in the index buffers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuSubmesh {
    /// Material index, i.e. texture set.
    pub material: u32,
    /// Range in the triangle index buffer, in indices.
    pub indices: std::ops::Range<u32>,
    /// Range in the UV wireframe index buffer, in indices: the polygon edges of this
    /// submesh in UV space (both sides of seams).
    pub uv_wire: std::ops::Range<u32>,
}

/// Vertex buffers of a mesh: one buffer per attribute, in shader locations 0 to 3.
#[derive(Debug)]
pub struct GpuMesh {
    /// Positions, `Float32x3`, location 0.
    pub positions: wgpu::Buffer,
    /// Normals, `Float32x3`, location 1.
    pub normals: wgpu::Buffer,
    /// mikktspace tangents with sign, `Float32x4`, location 2 (zeros without UVs).
    pub tangents: wgpu::Buffer,
    /// UV0, `Float32x2`, location 3 (zeros without UVs).
    pub uvs: wgpu::Buffer,
    /// Triangle indices.
    pub indices: wgpu::Buffer,
    /// Number of indices (3 per triangle).
    pub index_count: u32,
    /// Line-list indices of the polygon edges.
    pub wire_indices: wgpu::Buffer,
    /// Number of wire indices.
    pub wire_index_count: u32,
    /// Line-list indices of the UV polygon edges, submesh by submesh.
    pub uv_wire_indices: wgpu::Buffer,
    /// Ranges of each submesh, in order.
    pub submeshes: Vec<GpuSubmesh>,
    /// Bounds of the mesh.
    pub bounds: BoundingBox,
    /// De-indexed picking buffers, only on devices without `PRIMITIVE_INDEX`.
    pub flat_pick: Option<FlatPickBuffers>,
}

/// De-indexed positions and per-vertex triangle ids for picking without
/// `@builtin(primitive_index)`.
#[derive(Debug)]
pub struct FlatPickBuffers {
    /// Three positions per triangle.
    pub positions: wgpu::Buffer,
    /// Triangle index of each vertex.
    pub ids: wgpu::Buffer,
    /// Number of vertices (3 per triangle).
    pub vertex_count: u32,
}

impl FlatPickBuffers {
    fn new(ctx: &GpuContext, mesh: &MeshAsset) -> Self {
        let positions: Vec<[f32; 3]> = mesh
            .triangles
            .iter()
            .flat_map(|t| t.map(|v| mesh.positions[v as usize]))
            .collect();
        let ids: Vec<u32> = (0..mesh.triangles.len() as u32)
            .flat_map(|t| [t; 3])
            .collect();
        let vertex = wgpu::BufferUsages::VERTEX;
        Self {
            positions: buffer(
                ctx,
                "pick positions",
                bytemuck::cast_slice(&positions),
                vertex,
            ),
            ids: buffer(ctx, "pick ids", bytemuck::cast_slice(&ids), vertex),
            vertex_count: positions.len() as u32,
        }
    }
}

/// Vertex buffer layouts matching [`GpuMesh`], for the mesh pipelines.
pub const VERTEX_LAYOUTS: [wgpu::VertexBufferLayout<'static>; 4] = [
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        }],
    },
    wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 1,
        }],
    },
    wgpu::VertexBufferLayout {
        array_stride: 16,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x4,
            offset: 0,
            shader_location: 2,
        }],
    },
    wgpu::VertexBufferLayout {
        array_stride: 8,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 0,
            shader_location: 3,
        }],
    },
];

fn buffer(
    ctx: &GpuContext,
    label: &str,
    contents: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    ctx.device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            // A zero-sized buffer cannot be bound; empty meshes are not uploaded anyway.
            usage,
        })
}

impl GpuMesh {
    /// Uploads `mesh`. Returns `None` for a mesh without triangles. Picking buffers are
    /// added when the device lacks `PRIMITIVE_INDEX`.
    pub fn new(ctx: &GpuContext, mesh: &MeshAsset) -> Option<Self> {
        Self::upload(ctx, mesh, !ctx.capabilities().primitive_index)
    }

    /// Uploads `mesh` with the picking buffers of the `PRIMITIVE_INDEX` fallback, to
    /// exercise that path on any device.
    pub fn with_flat_picking(ctx: &GpuContext, mesh: &MeshAsset) -> Option<Self> {
        Self::upload(ctx, mesh, true)
    }

    fn upload(ctx: &GpuContext, mesh: &MeshAsset, flat_picking: bool) -> Option<Self> {
        let bounds = mesh.bounds()?;
        if mesh.triangles.is_empty() {
            return None;
        }
        let vertices = mesh.vertex_count();
        let tangents = mesh
            .tangents
            .clone()
            .unwrap_or_else(|| vec![[0.0; 4]; vertices]);
        let uvs = mesh.uv0.clone().unwrap_or_else(|| vec![[0.0; 2]; vertices]);
        let mut uv_edges = Vec::new();
        let submeshes = mesh
            .submeshes
            .iter()
            .map(|submesh| {
                let start = (uv_edges.len() * 2) as u32;
                uv_edges.extend(patine_mesh::topology::uv_polygon_edges(
                    mesh,
                    submesh.triangles.clone(),
                ));
                GpuSubmesh {
                    material: submesh.material,
                    indices: submesh.triangles.start * 3..submesh.triangles.end * 3,
                    uv_wire: start..(uv_edges.len() * 2) as u32,
                }
            })
            .collect();
        let mut edges = patine_mesh::topology::polygon_edges(mesh);
        let wire_index_count = (edges.len() * 2) as u32;
        // At least one line so the buffers are never empty.
        for lines in [&mut edges, &mut uv_edges] {
            if lines.is_empty() {
                lines.push([0, 0]);
            }
        }
        let vertex = wgpu::BufferUsages::VERTEX;
        let index = wgpu::BufferUsages::INDEX;
        Some(Self {
            positions: buffer(
                ctx,
                "mesh positions",
                bytemuck::cast_slice(&mesh.positions),
                vertex,
            ),
            normals: buffer(
                ctx,
                "mesh normals",
                bytemuck::cast_slice(&mesh.normals),
                vertex,
            ),
            tangents: buffer(
                ctx,
                "mesh tangents",
                bytemuck::cast_slice(&tangents),
                vertex,
            ),
            uvs: buffer(ctx, "mesh uvs", bytemuck::cast_slice(&uvs), vertex),
            indices: buffer(
                ctx,
                "mesh indices",
                bytemuck::cast_slice(&mesh.triangles),
                index,
            ),
            index_count: (mesh.triangles.len() * 3) as u32,
            wire_indices: buffer(
                ctx,
                "mesh wire indices",
                bytemuck::cast_slice(&edges),
                index,
            ),
            wire_index_count,
            uv_wire_indices: buffer(
                ctx,
                "mesh uv wire indices",
                bytemuck::cast_slice(&uv_edges),
                index,
            ),
            submeshes,
            bounds,
            flat_pick: flat_picking.then(|| FlatPickBuffers::new(ctx, mesh)),
        })
    }

    /// Binds the four vertex buffers to slots 0 to 3.
    pub fn bind_vertices(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(0, self.positions.slice(..));
        pass.set_vertex_buffer(1, self.normals.slice(..));
        pass.set_vertex_buffer(2, self.tangents.slice(..));
        pass.set_vertex_buffer(3, self.uvs.slice(..));
    }
}
