// Parameters of the `test.probe` procedural of the patine_procedural tests: the layout
// test checks this structure against the layout computed from the definition.

struct Params {
    scale: f32,
    seed: u32,
    tint: vec4<f32>,
    invert: u32,
    mode: u32,
    octaves: i32,
}

@group(0) @binding(0) var<uniform> params: Params;

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return params.tint * params.scale;
}
