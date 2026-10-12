// Tileable noises of the procedural library. Every noise is periodic on its integer
// lattice: noise(p + period) = noise(p), for any integer period > 0 on each axis. Results
// are in [0, 1]. Must match `patine_procedural::reference::noise` (same operations, in
// the same order).

// #import procedural/hash

// Quintic fade curve of the lattice interpolation.
fn fade(t: f32) -> f32 {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

// One of 8 unit gradients (axes and diagonals), from a hash.
fn gradient(h: u32) -> vec2<f32> {
    let d = 0.70710678;
    switch h & 7u {
        case 0u: { return vec2<f32>(1.0, 0.0); }
        case 1u: { return vec2<f32>(-1.0, 0.0); }
        case 2u: { return vec2<f32>(0.0, 1.0); }
        case 3u: { return vec2<f32>(0.0, -1.0); }
        case 4u: { return vec2<f32>(d, d); }
        case 5u: { return vec2<f32>(-d, d); }
        case 6u: { return vec2<f32>(d, -d); }
        default: { return vec2<f32>(-d, -d); }
    }
}

// Value noise: random values at the lattice points, smoothly interpolated.
fn value_noise(p: vec2<f32>, period: vec2<i32>, seed: u32) -> f32 {
    let base = floor(p);
    let cell = vec2<i32>(base);
    let f = p - base;
    let a = unit_float(hash_cell(wrap_cell(cell, period), seed));
    let b = unit_float(hash_cell(wrap_cell(cell + vec2<i32>(1, 0), period), seed));
    let c = unit_float(hash_cell(wrap_cell(cell + vec2<i32>(0, 1), period), seed));
    let d = unit_float(hash_cell(wrap_cell(cell + vec2<i32>(1, 1), period), seed));
    let u = fade(f.x);
    let v = fade(f.y);
    return mix(mix(a, b, u), mix(c, d, u), v);
}

// Gradient (Perlin) noise.
fn perlin_noise(p: vec2<f32>, period: vec2<i32>, seed: u32) -> f32 {
    let base = floor(p);
    let cell = vec2<i32>(base);
    let f = p - base;
    let a = dot(gradient(hash_cell(wrap_cell(cell, period), seed)), f);
    let b = dot(gradient(hash_cell(wrap_cell(cell + vec2<i32>(1, 0), period), seed)), f - vec2<f32>(1.0, 0.0));
    let c = dot(gradient(hash_cell(wrap_cell(cell + vec2<i32>(0, 1), period), seed)), f - vec2<f32>(0.0, 1.0));
    let d = dot(gradient(hash_cell(wrap_cell(cell + vec2<i32>(1, 1), period), seed)), f - vec2<f32>(1.0, 1.0));
    let u = fade(f.x);
    let v = fade(f.y);
    // Unit gradients give at most sqrt(2)/2 in magnitude.
    return 0.5 + mix(mix(a, b, u), mix(c, d, u), v) * 0.70710678;
}

// Contribution of one simplex corner at Cartesian position `vertex`, offset `x` from it.
fn simplex_corner(vertex: vec2<f32>, x: vec2<f32>, period: vec2<f32>, seed: u32) -> f32 {
    // Wrapped into the period, then back to integer lattice coordinates (exact: rows of
    // odd index sit on half integers, and the period has an even number of rows).
    let w = vertex - floor(vertex / period) * period;
    let lattice = vec2<i32>(floor(vec2<f32>(w.x + w.y * 0.5, w.y) + 0.5));
    let t = max(0.8 - dot(x, x), 0.0);
    let t2 = t * t;
    return t2 * t2 * dot(gradient(hash_cell(lattice, seed)), x);
}

// Simplex noise on a sheared triangular lattice whose vertices are (i − j/2, j): it tiles
// with any integer period along x and an even one along y (after Gustavson and McEwan,
// "Tiling simplex noise and flow noise in two and three dimensions", JCGT 2022). The
// domain is doubled so that every integer period of the caller is even.
fn simplex_noise(p_in: vec2<f32>, period_in: vec2<i32>, seed: u32) -> f32 {
    let p = p_in * 2.0;
    let period = vec2<f32>(period_in * 2);
    let uv = vec2<f32>(p.x + p.y * 0.5, p.y);
    let i0 = floor(uv);
    let f0 = uv - i0;
    let o1 = select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), f0.x > f0.y);
    let i1 = i0 + o1;
    let i2 = i0 + vec2<f32>(1.0, 1.0);
    let v0 = vec2<f32>(i0.x - i0.y * 0.5, i0.y);
    let v1 = vec2<f32>(i1.x - i1.y * 0.5, i1.y);
    let v2 = vec2<f32>(i2.x - i2.y * 0.5, i2.y);
    let n = simplex_corner(v0, p - v0, period, seed)
        + simplex_corner(v1, p - v1, period, seed)
        + simplex_corner(v2, p - v2, period, seed);
    return clamp(0.5 + n * 5.0, 0.0, 1.0);
}

// Distances to the nearest and second nearest feature points (one random point per cell).
fn worley(p: vec2<f32>, period: vec2<i32>, seed: u32) -> vec2<f32> {
    let base = floor(p);
    let cell = vec2<i32>(base);
    let f = p - base;
    var f1 = 8.0;
    var f2 = 8.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let offset = vec2<i32>(i, j);
            let point = vec2<f32>(offset) + unit_float2(hash_cell(wrap_cell(cell + offset, period), seed));
            let d = length(point - f);
            if d < f1 {
                f2 = f1;
                f1 = d;
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    return vec2<f32>(f1, f2);
}

// Worley noise in [0, 1]: mode 0 is F1, 1 is F2 (scaled), 2 is F2 − F1.
fn worley_noise(p: vec2<f32>, period: vec2<i32>, seed: u32, mode: u32) -> f32 {
    let f = worley(p, period, seed);
    switch mode {
        case 0u: { return clamp(f.x, 0.0, 1.0); }
        case 1u: { return clamp(f.y * 0.75, 0.0, 1.0); }
        default: { return clamp(f.y - f.x, 0.0, 1.0); }
    }
}

// Seed of octave `octave`.
fn octave_seed(seed: u32, octave: i32) -> u32 {
    return seed + u32(octave) * 1013u;
}

// Fractal sum of Perlin octaves, each twice the frequency (and period) of the previous
// one, weighted by `gain`^octave and normalized. `kind` 0 is fBm, 1 ridged, 2 turbulence.
fn fractal(p: vec2<f32>, period: vec2<i32>, seed: u32, octaves: i32, gain: f32, kind: u32) -> f32 {
    var sum = 0.0;
    var norm = 0.0;
    var amplitude = 1.0;
    var q = p;
    var per = period;
    for (var octave = 0; octave < octaves; octave++) {
        let n = perlin_noise(q, per, octave_seed(seed, octave));
        var value = n;
        if kind == 1u {
            let r = 1.0 - abs(2.0 * n - 1.0);
            value = r * r;
        } else if kind == 2u {
            value = abs(2.0 * n - 1.0);
        }
        sum += amplitude * value;
        norm += amplitude;
        amplitude *= gain;
        q *= 2.0;
        per *= 2;
    }
    return sum / norm;
}

// fBm of `p` displaced by two other fBm: tileable, since the displacement is periodic.
fn warp_noise(p: vec2<f32>, period: vec2<i32>, seed: u32, octaves: i32, gain: f32, strength: f32) -> f32 {
    let wx = fractal(p + vec2<f32>(5.2, 1.3), period, seed + 17u, octaves, gain, 0u);
    let wy = fractal(p + vec2<f32>(1.7, 9.2), period, seed + 31u, octaves, gain, 0u);
    let q = p + (vec2<f32>(wx, wy) - 0.5) * (2.0 * strength);
    return fractal(q, period, seed, octaves, gain, 0u);
}
