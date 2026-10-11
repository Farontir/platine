// 2D view: a channel texture in UV space, a neutral checker around the 0–1 square, the
// square's border, and the UV wireframe of a submesh.
// Must match `UvUniforms` in crates/patine_render/src/uv_view.rs.

// #import render/display
// #import render/fullscreen

struct Uv {
    // xy: UV at the center of the view; z: pixels per UV unit.
    transform: vec4<f32>,
    // xy: size in pixels.
    viewport: vec4<f32>,
    // x: channel kind (0 color, 1 scalar, 2 normal); y: 1 when a texture is shown.
    display: vec4<f32>,
    // rgb: wireframe color (display-encoded).
    wire_color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uv_view: Uv;
@group(0) @binding(1) var channel_texture: texture_2d<f32>;
@group(0) @binding(2) var channel_sampler: sampler;

// UV of a pixel position (pixel centers at .5), v growing downwards like texture rows.
fn pixel_to_uv(pixel: vec2<f32>) -> vec2<f32> {
    return uv_view.transform.xy + (pixel - 0.5 * uv_view.viewport.xy) / uv_view.transform.z;
}

@fragment
fn fs_background(in: FullscreenOutput) -> @location(0) vec4<f32> {
    let pixel = in.clip.xy;
    let uv = pixel_to_uv(pixel);
    let scale = uv_view.transform.z;
    // Distance to the border of the 0–1 square, in pixels.
    let outside = max(max(-uv, uv - vec2<f32>(1.0)), vec2<f32>(0.0));
    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
    let to_edge = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y)) * scale;
    // Sampled in uniform control flow, used inside the square only.
    let value = textureSample(channel_texture, channel_sampler, uv);
    var color: vec3<f32>;
    if inside {
        if uv_view.display.y > 0.5 {
            color = display_encode(value, uv_view.display.x);
        } else {
            color = vec3<f32>(0.5);
        }
        if to_edge < 1.0 {
            color = vec3<f32>(0.85);
        }
    } else {
        // Checker of 16-pixel squares, aligned with the UV origin.
        let cell = floor((pixel - 0.5 * uv_view.viewport.xy) / 16.0 + uv_view.transform.xy * scale / 16.0);
        let even = (i32(cell.x) + i32(cell.y)) % 2 == 0;
        color = select(vec3<f32>(0.14), vec3<f32>(0.18), even);
        if length(outside) * scale < 1.0 {
            color = vec3<f32>(0.85);
        }
    }
    return vec4<f32>(color, 1.0);
}

@vertex
fn vs_wire(@location(0) uv: vec2<f32>) -> @builtin(position) vec4<f32> {
    let pixel = (uv - uv_view.transform.xy) * uv_view.transform.z + 0.5 * uv_view.viewport.xy;
    let ndc = pixel / uv_view.viewport.xy * 2.0 - 1.0;
    return vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
}

@fragment
fn fs_wire() -> @location(0) vec4<f32> {
    return vec4<f32>(uv_view.wire_color.rgb, 1.0);
}
