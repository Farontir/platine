// Picking pass without primitive_index: de-indexed positions with a flat per-vertex
// triangle id. Writes triangle index + 1 (0 = background) and depth.

// #import render/frame

@group(0) @binding(0) var<uniform> frame: Frame;

struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) id: u32,
}

struct PickOutput {
    @location(0) id: u32,
    @location(1) depth: f32,
}

@vertex
fn vs_main(@location(0) position: vec3<f32>, @location(1) triangle: u32) -> VertexOutput {
    var out: VertexOutput;
    out.clip = frame.view_proj * vec4<f32>(position, 1.0);
    out.id = triangle + 1u;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> PickOutput {
    return PickOutput(in.id, in.clip.z);
}
