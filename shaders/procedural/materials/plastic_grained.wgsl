// Grained plastic (`material.plastic_grained`): small Worley bumps over a colored
// dielectric, rougher in the hollows. `Params` must match its definition in
// crates/patine_procedural/src/library.rs.

// #import procedural/noise
// #import procedural/pattern
// #import procedural/material_main

struct Params {
    color: vec4<f32>,
    roughness: f32,
    scale: i32,
    grain: f32,
    relief: f32,
    seed: u32,
}

@group(0) @binding(1) var<uniform> params: Params;

// Bumps: 1 on the grains, 0 between them.
fn bumps(q: vec2<f32>) -> f32 {
    let f = worley(q * f32(params.scale), noise_period(params.scale), params.seed);
    return 1.0 - soft_step(0.0, 0.7, f.x);
}

fn material_height(q: vec2<f32>) -> f32 {
    return (bumps(q) - 0.5) * params.grain * params.relief * 0.1;
}

fn material(q: vec2<f32>) -> MaterialSample {
    let g = bumps(q) - 0.5;
    var m: MaterialSample;
    m.base_color = params.color.rgb * (1.0 + 0.15 * params.grain * g);
    m.metallic = 0.0;
    m.roughness = clamp(params.roughness - 0.2 * params.grain * g, 0.02, 1.0);
    m.height = g * params.grain * params.relief * 0.1;
    return m;
}
