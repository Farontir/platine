// One effect of a mask stack, texel by texel:
// mask = clamp(mix(m, blend(m, e), opacity), 0, 1), where m is the mask below and e the
// effect's value: a constant (uniform fill), levels(m) or 1 − m (inversion).
// Must match `reference::mask_effect` and `MaskUniforms` in
// crates/patine_compositor/src/compositor.rs.

// #import compositor/blend_ops
// #import render/fullscreen

struct MaskStep {
    // x: blend mode index, y: opacity, z: operation (0 fill, 1 levels, 2 invert), w: fill value.
    params: vec4<f32>,
    // in_low, in_high, gamma, out_low.
    levels: vec4<f32>,
    // x: out_high.
    levels_out: vec4<f32>,
}

@group(0) @binding(0) var<uniform> mask_step: MaskStep;
@group(0) @binding(1) var below: texture_2d<f32>;

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
    let m = textureLoad(below, vec2<u32>(in.clip.xy), 0).r;
    let operation = u32(mask_step.params.z);
    var e = mask_step.params.w;
    if operation == 1u {
        e = levels(m);
    } else if operation == 2u {
        e = 1.0 - m;
    }
    let blended = apply_blend(u32(mask_step.params.x), 1u, vec4<f32>(m, 0.0, 0.0, 0.0), vec4<f32>(e, 0.0, 0.0, 0.0), mask_step.params.y);
    return vec4<f32>(clamp(blended.r, 0.0, 1.0), 0.0, 0.0, 0.0);
}
