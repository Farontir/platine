// Evaluates one function of the procedural library at a list of points, for the tests of
// patine_procedural (GPU against the CPU reference, tileability).
// Must match `Probe` in crates/patine_procedural/tests/library.rs.

// #import procedural/hash
// #import procedural/noise
// #import procedural/pattern

struct Probe {
    // Function index (see `Function` in the test).
    function: u32,
    seed: u32,
    // Integer parameters: counts, modes, octaves.
    a: i32,
    b: i32,
    period: vec2<i32>,
    // Real parameters.
    x: f32,
    y: f32,
    z: f32,
    count: u32,
}

@group(0) @binding(0) var<uniform> probe: Probe;
@group(0) @binding(1) var<storage, read> points: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read_write> values: array<f32>;

fn evaluate(p: vec2<f32>) -> f32 {
    let period = probe.period;
    let seed = probe.seed;
    switch probe.function {
        case 0u: { return value_noise(p, period, seed); }
        case 1u: { return perlin_noise(p, period, seed); }
        case 2u: { return simplex_noise(p, period, seed); }
        case 3u: { return worley_noise(p, period, seed, u32(probe.a)); }
        case 4u: { return fractal(p, period, seed, probe.a, probe.x, u32(probe.b)); }
        case 5u: { return warp_noise(p, period, seed, probe.a, probe.x, probe.y); }
        case 6u: { return pattern_gradient(p, u32(probe.a)); }
        case 7u: { return pattern_checker(p); }
        case 8u: { return pattern_tiles(p, probe.a, probe.x, probe.y, probe.z, seed); }
        case 9u: { return pattern_bricks(p, probe.a, probe.x, probe.y, probe.z, seed); }
        case 10u: { return pattern_stripes(p, probe.a, probe.x, probe.y, u32(probe.b)); }
        case 11u: { return pattern_circles(p, probe.a, probe.x, probe.y); }
        case 12u: { return pattern_scratches(p, probe.a, probe.x, probe.y, seed); }
        default: { return pattern_fibers(p, probe.a, probe.b, seed); }
    }
}

@compute @workgroup_size(64)
fn cs_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x < probe.count {
        values[id.x] = evaluate(points[id.x]);
    }
}
