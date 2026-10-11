// One blend pass of the compositor: target = mix(dst, blend(dst, src), mask × opacity),
// texel by texel. The source is a constant (uniform fill) or a texture (folder result,
// later layer content). Textures smaller than the target (1×1 masks) are clamped.
// Must match `BlendUniforms` in crates/patine_compositor/src/compositor.rs.

// #import compositor/blend_ops
// #import render/fullscreen

struct Step {
    // Constant source value.
    src: vec4<f32>,
    // x: mode index, y: channel kind, z: opacity, w: 1 to read the source texture.
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> blend_step: Step;
@group(0) @binding(1) var dst_texture: texture_2d<f32>;
@group(0) @binding(2) var src_texture: texture_2d<f32>;
@group(0) @binding(3) var mask_texture: texture_2d<f32>;

fn load(t: texture_2d<f32>, pixel: vec2<u32>) -> vec4<f32> {
    let size = textureDimensions(t);
    return textureLoad(t, min(pixel, size - vec2<u32>(1u)), 0);
}

@fragment
fn fs_main(in: FullscreenOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(in.clip.xy);
    let dst = load(dst_texture, pixel);
    var src = blend_step.src;
    if blend_step.params.w > 0.5 {
        src = load(src_texture, pixel);
    }
    let mask = load(mask_texture, pixel).r;
    return apply_blend(u32(blend_step.params.x), u32(blend_step.params.y), dst, src, blend_step.params.z * mask);
}
