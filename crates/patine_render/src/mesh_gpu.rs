//! Mesh data uploaded to the GPU.

use patine_core::mesh::{BoundingBox, MeshAsset};
use patine_gpu::{GpuContext, wgpu};
use wgpu::util::DeviceExt as _;

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
    /// Bounds of the mesh.
    pub bounds: BoundingBox,
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
    /// Uploads `mesh`. Returns `None` for a mesh without triangles.
    pub fn new(ctx: &GpuContext, mesh: &MeshAsset) -> Option<Self> {
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
        let edges = patine_mesh::topology::polygon_edges(mesh);
        // At least one line so the buffer is never empty.
        let edges = if edges.is_empty() {
            vec![[0, 0]]
        } else {
            edges
        };
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
            wire_index_count: (edges.len() * 2) as u32,
            bounds,
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
