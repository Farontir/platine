// Rasterizes the triangles of a texture set in UV space: each covered texel (its center
// inside a triangle) receives the triangle's index + 1; the target is cleared to 0. The
// draw is not indexed: vertex `i` is corner `i % 3` of triangle `i / 3`.

// #import procedural/mesh

@group(0) @binding(0) var<storage, read> vertices: array<Vertex>;
@group(0) @binding(1) var<storage, read> indices: array<u32>;

struct RasterOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) triangle: u32,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> RasterOutput {
    let triangle = index / 3u;
    let uv = vertices[indices[index]].uv.xy;
    var out: RasterOutput;
    // Texture row 0 is v = 0.
    out.clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.triangle = triangle + 1u;
    return out;
}

@fragment
fn fs_main(in: RasterOutput) -> @location(0) u32 {
    return in.triangle;
}
