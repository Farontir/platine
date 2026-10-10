// Fullscreen triangle generated from the vertex index; no vertex buffer.

struct FullscreenOutput {
    @builtin(position) clip: vec4<f32>,
    // 0..1, y down (texture convention).
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> FullscreenOutput {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: FullscreenOutput;
    out.clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}
