// Copper (`material.copper`): a metal whose tint and roughness vary with an fBm, with
// optional oxidation (dark, rough, dielectric) in the low areas of a second fBm.
// `Params` must match its definition in crates/patine_procedural/src/library.rs.

// #import procedural/noise
// #import procedural/pattern
// #import procedural/material_main

struct Params {
    color: vec4<f32>,
    roughness: f32,
    scale: i32,
    variation: f32,
    oxidation: f32,
    relief: f32,
    seed: u32,
}

@group(0) @binding(1) var<uniform> params: Params;

fn tint(q: vec2<f32>) -> f32 {
    return fractal(q * f32(params.scale), noise_period(params.scale), params.seed, 5, 0.5, 0u);
}

// Oxidized amount, 0 to 1.
fn oxide(q: vec2<f32>) -> f32 {
    let scale = params.scale * 2;
    let n = fractal(q * f32(scale), noise_period(scale), params.seed + 7u, 4, 0.55, 0u);
    return soft_step(1.0 - params.oxidation, 1.15 - params.oxidation, 1.0 - n);
}

fn material_height(q: vec2<f32>) -> f32 {
    return (tint(q) - 0.5) * params.relief * 0.2 - oxide(q) * params.relief * 0.1;
}

fn material(q: vec2<f32>) -> MaterialSample {
    let n = tint(q) - 0.5;
    let o = oxide(q);
    var m: MaterialSample;
    let metal = params.color.rgb * (1.0 + params.variation * n);
    m.base_color = mix(metal, vec3<f32>(0.18, 0.1, 0.07), o);
    m.metallic = 1.0 - o;
    m.roughness = clamp(mix(params.roughness + 0.3 * params.variation * n, 0.8, o), 0.02, 1.0);
    m.height = n * params.relief * 0.2 - o * params.relief * 0.1;
    return m;
}
