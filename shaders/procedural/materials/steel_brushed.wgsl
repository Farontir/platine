// Brushed steel (`material.steel_brushed`): fine fibers along the brushing direction
// lighten the color and change the roughness. `Params` must match its definition in
// crates/patine_procedural/src/library.rs.

// #import procedural/noise
// #import procedural/pattern
// #import procedural/material_main

struct Params {
    color: vec4<f32>,
    roughness: f32,
    scale: i32,
    brushing: f32,
    relief: f32,
    seed: u32,
}

@group(0) @binding(1) var<uniform> params: Params;

fn brush(q: vec2<f32>) -> f32 {
    return pattern_fibers(q, params.scale, 16, params.seed);
}

fn material_height(q: vec2<f32>) -> f32 {
    return (brush(q) - 0.5) * params.relief * 0.2;
}

fn material(q: vec2<f32>) -> MaterialSample {
    let b = brush(q) - 0.5;
    var m: MaterialSample;
    m.base_color = params.color.rgb * (1.0 + 0.3 * params.brushing * b);
    m.metallic = 1.0;
    m.roughness = clamp(params.roughness + 0.25 * params.brushing * b, 0.02, 1.0);
    m.height = b * params.relief * 0.2;
    return m;
}
