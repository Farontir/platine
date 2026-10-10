// Display pass: exposure, tone mapping, sRGB encoding into 8 bits for egui.

// #import render/frame
// #import render/color
// #import render/tonemap_ops
// #import render/fullscreen

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var hdr: texture_2d<f32>;

@fragment
fn fs_main(in: FullscreenOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.clip.xy);
    let exposed = textureLoad(hdr, pixel, 0).rgb * frame.material.z;
    let display = tone_map(exposed, u32(frame.material.w));
    return vec4<f32>(srgb_encode(display), 1.0);
}
