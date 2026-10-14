// One effect of a mask stack, texel by texel:
// mask = clamp(mix(m, blend(m, e), opacity), 0, 1), where m is the mask below and e the
// effect's value: a constant (uniform fill), levels(m), 1 − m (inversion), a texture
// (procedural fill), or a paint content (premultiplied value and coverage c: the value is
// divided by c and the opacity scaled by c).
// Must match `reference::mask_effect` and `MaskUniforms` in
// crates/patine_compositor/src/compositor.rs.

// #import compositor/blend_ops
// #import render/fullscreen

struct MaskStep {
    // x: blend mode index, y: opacity, z: operation (0 fill, 1 levels, 2 invert, 3
    // texture, 4 paint), w: fill value.
    params: vec4<f32>,
    // in_low, in_high, gamma, out_low.
    levels: vec4<f32>,
    // x: out_high.
    levels_out: vec4<f32>,
}

@group(0) @binding(0) var<uniform> mask_step: MaskStep;
@group(0) @binding(1) var below: texture_2d<f32>;
// Operations 3 and 4: the procedural fill or the paint content, at the size of the mask.
@group(0) @binding(2) var source: texture_2d<f32>;

fn levels(m: f32) -> f32 {
    let p = mask_step.levels;
    let range = max(p.y - p.x, 1e-4);
    let t = clamp((m - p.x) / range, 0.0, 1.0);
    // pow(0, y) goes through log2(0): give 0 explicitly, as the CPU does.
    let x = select(pow(t, 1.0 / max(p.z, 1e-4)), 0.0, t <= 0.0);
    return p.w + x * (mask_step.levels_out.x - p.w);
}

@fragment
fn fs_main(in: FullscreenOutput) -> @location(0) vec4<f32> {
    let texel = vec2<u32>(in.clip.xy);
    let m = textureLoad(below, texel, 0).r;
    let operation = u32(mask_step.params.z);
    var e = mask_step.params.w;
    var opacity = mask_step.params.y;
    if operation == 1u {
        e = levels(m);
    } else if operation == 2u {
        e = 1.0 - m;
    } else if operation == 3u {
        e = textureLoad(source, texel, 0).r;
    } else if operation == 4u {
        let paint = textureLoad(source, texel, 0);
        let c = paint.g;
        e = paint.r * select(0.0, 1.0 / c, c > 1e-4);
        opacity = opacity * c;
    }
    let blended = apply_blend(u32(mask_step.params.x), 1u, vec4<f32>(m, 0.0, 0.0, 0.0), vec4<f32>(e, 0.0, 0.0, 0.0), opacity);
    return vec4<f32>(clamp(blended.r, 0.0, 1.0), 0.0, 0.0, 0.0);
}
