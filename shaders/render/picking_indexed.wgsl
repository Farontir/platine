enable primitive_index;

// Picking pass on the indexed mesh: triangle index + 1 (0 = background) and depth.
// Needs Features::PRIMITIVE_INDEX; picking_flat.wgsl is the fallback.

// #import render/frame

@group(0) @binding(0) var<uniform> frame: Frame;

struct PickOutput {
    @location(0) id: u32,
    @location(1) depth: f32,
}

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return frame.view_proj * vec4<f32>(position, 1.0);
}

@fragment
fn fs_main(@builtin(position) clip: vec4<f32>, @builtin(primitive_index) triangle: u32) -> PickOutput {
    return PickOutput(triangle + 1u, clip.z);
}
