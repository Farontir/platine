// One blend pass of the compositor: target = mix(dst, blend(dst, src), mask × opacity),
// texel by texel. The source is a constant (uniform fill), a texture (folder result,
// procedural content), or a paint content: premultiplied value with its coverage c
// (alpha for colors and normals, green for scalars), divided by c and blended with
// weight × c. Textures smaller than the target (1×1 masks) are clamped.
// Must match `BlendUniforms` in crates/patine_compositor/src/compositor.rs.

// #import compositor/blend_ops
// #import render/fullscreen

struct Step {
    // Constant source value.
    src: vec4<f32>,
    // x: mode index, y: channel kind, z: opacity, w: source (0 constant, 1 texture,
    // 2 paint content).
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
    let kind = u32(blend_step.params.y);
    var src = blend_step.src;
    let mask = load(mask_texture, pixel).r;
    var weight = blend_step.params.z * mask;
    if blend_step.params.w > 0.5 {
        src = load(src_texture, pixel);
    }
    if blend_step.params.w > 1.5 {
        // Must match `patine_paint::reference::unpremultiply`.
        let c = select(src.a, src.g, kind == 1u);
        let inverse = select(0.0, 1.0 / c, c > 1e-4);
        if kind == 1u {
            src = vec4<f32>(src.r * inverse, 0.0, 0.0, 0.0);
        } else {
            src = vec4<f32>(src.rgb * inverse, 1.0);
        }
        weight = weight * c;
    }
    return apply_blend(u32(blend_step.params.x), kind, dst, src, weight);
}
