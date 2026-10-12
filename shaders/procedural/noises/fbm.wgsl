// fBm (`noise.fbm`), a gray procedural: one value per texel, blended over the planes of
// the projection. `Params` must match its definition in
// crates/patine_procedural/src/library.rs (checked by the layout test).

// #import procedural/noise
// #import procedural/evaluate

struct Params {
    scale: i32,
    octaves: i32,
    gain: f32,
    seed: u32,
}

@group(0) @binding(1) var<uniform> params: Params;

fn gray(q: vec2<f32>) -> f32 {
    return fractal(q * f32(params.scale), noise_period(params.scale), params.seed, params.octaves, params.gain, 0u);
}

@fragment
fn fs_main(in: FullscreenOutput) -> @location(0) vec4<f32> {
    let p = project(current_surface(in.clip));
    var v = p.w.x * gray(p.q0);
    if p.w.y > 0.0 {
        v += p.w.y * gray(p.q1);
    }
    if p.w.z > 0.0 {
        v += p.w.z * gray(p.q2);
    }
    return vec4<f32>(v, 0.0, 0.0, 1.0);
}
