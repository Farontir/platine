// Wood (`material.wood`): growth rings across v, distorted by an fBm, with fibers along
// v. `Params` must match its definition in crates/patine_procedural/src/library.rs.

// #import procedural/noise
// #import procedural/pattern
// #import procedural/material_main

struct Params {
    light: vec4<f32>,
    dark: vec4<f32>,
    roughness: f32,
    rings: i32,
    distortion: f32,
    relief: f32,
    seed: u32,
}

@group(0) @binding(1) var<uniform> params: Params;

// Late wood: 1 in the dark part of each ring.
fn ring(q: vec2<f32>) -> f32 {
    let warp = fractal(q * 3.0, noise_period(3), params.seed, 4, 0.5, 0u) - 0.5;
    let r = fract(q.y * f32(params.rings) + params.distortion * 2.0 * warp);
    return soft_step(0.55, 0.8, r) * (1.0 - soft_step(0.85, 1.0, r));
}

fn fiber(q: vec2<f32>) -> f32 {
    return pattern_fibers(q, 2, 24, params.seed + 3u);
}

fn material_height(q: vec2<f32>) -> f32 {
    return ((fiber(q) - 0.5) * 0.3 - ring(q) * 0.1) * params.relief;
}

fn material(q: vec2<f32>) -> MaterialSample {
    let r = ring(q);
    let f = fiber(q) - 0.5;
    var m: MaterialSample;
    m.base_color = mix(params.light.rgb, params.dark.rgb, clamp(0.8 * r + 0.4 * f, 0.0, 1.0));
    m.metallic = 0.0;
    m.roughness = clamp(params.roughness + 0.1 * r, 0.02, 1.0);
    m.height = (f * 0.3 - r * 0.1) * params.relief;
    return m;
}
