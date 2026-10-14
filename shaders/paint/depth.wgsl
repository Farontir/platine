// Depth seen from a stroke's camera: for every viewport pixel, the distance of the
// nearest surface along the view direction (`StrokeProjection::depth`), written to an
// R32Float target cleared to f32::MAX. Every triangle of the mesh is drawn, both faces,
// with a reverse-Z depth test. The draw is not indexed: vertex `i` is corner `i % 3` of
// triangle `i / 3`. Must match `DepthUniforms` in crates/patine_paint/src/raster.rs.

// #import procedural/mesh

struct DepthUniforms {
    view_proj: mat4x4<f32>,
    // `dot(depth_axis, clip)` is the distance from the camera.
    depth_axis: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: DepthUniforms;
@group(0) @binding(1) var<storage, read> vertices: array<Vertex>;
@group(0) @binding(2) var<storage, read> indices: array<u32>;

struct DepthOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) depth: f32,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> DepthOutput {
    let position = vertices[indices[index]].position.xyz;
    let clip = u.view_proj * vec4<f32>(position, 1.0);
    var out: DepthOutput;
    out.clip = clip;
    out.depth = dot(u.depth_axis, clip);
    return out;
}

@fragment
fn fs_main(in: DepthOutput) -> @location(0) f32 {
    return in.depth;
}
