// Display pass: linear HDR (resolved) to sRGB-encoded 8-bit for egui.

// #import render/frame
// #import render/color
// #import render/fullscreen

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var hdr: texture_2d<f32>;

@fragment
fn fs_main(in: FullscreenOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.clip.xy);
    let linear = textureLoad(hdr, pixel, 0).rgb * frame.material.z;
    return vec4<f32>(srgb_encode(linear), 1.0);
}
