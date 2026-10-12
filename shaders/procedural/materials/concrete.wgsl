// Concrete (`material.concrete`): an fBm surface with Worley pores and large stains.
// `Params` must match its definition in crates/patine_procedural/src/library.rs.

// #import procedural/noise
// #import procedural/pattern
// #import procedural/material_main

struct Params {
    color: vec4<f32>,
    roughness: f32,
    scale: i32,
    pores: f32,
    stains: f32,
    relief: f32,
    seed: u32,
}

@group(0) @binding(1) var<uniform> params: Params;

fn grain(q: vec2<f32>) -> f32 {
    return fractal(q * f32(params.scale), noise_period(params.scale), params.seed, 6, 0.55, 0u);
}

// Pores: 1 inside a pore.
fn pore(q: vec2<f32>) -> f32 {
    let scale = params.scale * 6;
    let f = worley(q * f32(scale), noise_period(scale), params.seed + 11u);
    return (1.0 - soft_step(0.05, 0.2, f.x)) * params.pores;
}

fn stain(q: vec2<f32>) -> f32 {
    return fractal(q * 2.0, noise_period(2), params.seed + 5u, 4, 0.5, 0u) * params.stains;
}

fn material_height(q: vec2<f32>) -> f32 {
    return ((grain(q) - 0.5) * 0.5 - pore(q) * 0.5) * params.relief;
}

fn material(q: vec2<f32>) -> MaterialSample {
    let n = grain(q) - 0.5;
    let p = pore(q);
    var m: MaterialSample;
    m.base_color = params.color.rgb * (1.0 + 0.5 * n) * (1.0 - 0.4 * stain(q)) * (1.0 - 0.5 * p);
    m.metallic = 0.0;
    m.roughness = clamp(params.roughness + 0.1 * n, 0.02, 1.0);
    m.height = (n * 0.5 - p * 0.5) * params.relief;
    return m;
}
