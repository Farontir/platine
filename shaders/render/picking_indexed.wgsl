enable primitive_index;

// Picking pass on the indexed mesh: triangle index + 1 (0 = background) and depth.
// Needs Features::PRIMITIVE_INDEX; picking_flat.wgsl is the fallback.

// #import render/frame

@group(0) @binding(0) var<uniform> frame: Frame;

// The fragment stage does not read @builtin(position): the HLSL backend puts
// SV_PrimitiveID first in the pixel shader inputs, and with SV_Position after it the
// inputs no longer line up with the vertex outputs, which D3D12 rejects (E_INVALIDARG
// when creating the pipeline). The depth comes from clip z and w instead,
// perspective-correct, which is the depth the rasterizer computes.
struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) clip_zw: vec2<f32>,
}

struct PickOutput {
    @location(0) id: u32,
    @location(1) depth: f32,
}

@vertex
fn vs_main(@location(0) position: vec3<f32>) -> VertexOutput {
    let clip = frame.view_proj * vec4<f32>(position, 1.0);
    return VertexOutput(clip, clip.zw);
}

@fragment
fn fs_main(@location(0) clip_zw: vec2<f32>, @builtin(primitive_index) triangle: u32) -> PickOutput {
    return PickOutput(triangle + 1u, clip_zw.x / clip_zw.y);
}
