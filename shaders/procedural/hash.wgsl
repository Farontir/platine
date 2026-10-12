// Integer hashing of the procedural library. Pure integer arithmetic, bit for bit the same
// as `patine_procedural::reference::hash` on the CPU: noises built on it agree to float
// rounding only.

// PCG hash (O'Neill's permuted congruential generator, one round).
fn pcg(v: u32) -> u32 {
    let state = v * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

// Hash of a lattice cell and a seed.
fn hash_cell(cell: vec2<i32>, seed: u32) -> u32 {
    return pcg(pcg(bitcast<u32>(cell.x) + seed) + bitcast<u32>(cell.y));
}

// A number in [0, 1) from the 24 high bits of a hash (exact in f32).
fn unit_float(h: u32) -> f32 {
    return f32(h >> 8u) / 16777216.0;
}

// Two numbers in [0, 1) from a hash.
fn unit_float2(h: u32) -> vec2<f32> {
    return vec2<f32>(unit_float(h), unit_float(pcg(h)));
}

// `cell` wrapped into [0, period) on each axis (period > 0).
fn wrap_cell(cell: vec2<i32>, period: vec2<i32>) -> vec2<i32> {
    return ((cell % period) + period) % period;
}
