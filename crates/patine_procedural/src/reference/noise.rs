//! Mirror of `shaders/procedural/noise.wgsl`: tileable noises in [0, 1].

use super::hash::{hash_cell, unit_float, unit_float2, wrap_cell};
use super::{IVec2, Vec2, floor2, mix, to_ivec};

/// Quintic fade curve.
pub fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// One of 8 unit gradients, from a hash.
pub fn gradient(h: u32) -> Vec2 {
    // The f32 nearest to the WGSL literal 0.70710678.
    let d = std::f32::consts::FRAC_1_SQRT_2;
    match h & 7 {
        0 => Vec2::new(1.0, 0.0),
        1 => Vec2::new(-1.0, 0.0),
        2 => Vec2::new(0.0, 1.0),
        3 => Vec2::new(0.0, -1.0),
        4 => Vec2::new(d, d),
        5 => Vec2::new(-d, d),
        6 => Vec2::new(d, -d),
        _ => Vec2::new(-d, -d),
    }
}

/// `a · b` in the order WGSL's `dot` computes it.
fn dot(a: Vec2, b: Vec2) -> f32 {
    a.x * b.x + a.y * b.y
}

/// WGSL's `length`.
fn length(v: Vec2) -> f32 {
    dot(v, v).sqrt()
}

/// Value noise.
pub fn value_noise(p: Vec2, period: IVec2, seed: u32) -> f32 {
    let base = floor2(p);
    let cell = to_ivec(base);
    let f = p - base;
    let at = |offset: IVec2| unit_float(hash_cell(wrap_cell(cell + offset, period), seed));
    let a = at(IVec2::new(0, 0));
    let b = at(IVec2::new(1, 0));
    let c = at(IVec2::new(0, 1));
    let d = at(IVec2::new(1, 1));
    let u = fade(f.x);
    let v = fade(f.y);
    mix(mix(a, b, u), mix(c, d, u), v)
}

/// Gradient (Perlin) noise.
pub fn perlin_noise(p: Vec2, period: IVec2, seed: u32) -> f32 {
    let base = floor2(p);
    let cell = to_ivec(base);
    let f = p - base;
    let at = |offset: IVec2| {
        let g = gradient(hash_cell(wrap_cell(cell + offset, period), seed));
        dot(g, f - Vec2::new(offset.x as f32, offset.y as f32))
    };
    let a = at(IVec2::new(0, 0));
    let b = at(IVec2::new(1, 0));
    let c = at(IVec2::new(0, 1));
    let d = at(IVec2::new(1, 1));
    let u = fade(f.x);
    let v = fade(f.y);
    0.5 + mix(mix(a, b, u), mix(c, d, u), v) * std::f32::consts::FRAC_1_SQRT_2
}

fn simplex_corner(vertex: Vec2, x: Vec2, period: Vec2, seed: u32) -> f32 {
    let w = vertex - floor2(vertex / period) * period;
    let lattice = to_ivec(floor2(Vec2::new(w.x + w.y * 0.5, w.y) + 0.5));
    let t = (0.8 - dot(x, x)).max(0.0);
    let t2 = t * t;
    t2 * t2 * dot(gradient(hash_cell(lattice, seed)), x)
}

/// Periodic simplex noise (the domain is doubled so that every period is even).
pub fn simplex_noise(p_in: Vec2, period_in: IVec2, seed: u32) -> f32 {
    let p = p_in * 2.0;
    let period = Vec2::new((period_in.x * 2) as f32, (period_in.y * 2) as f32);
    let uv = Vec2::new(p.x + p.y * 0.5, p.y);
    let i0 = floor2(uv);
    let f0 = uv - i0;
    let o1 = if f0.x > f0.y {
        Vec2::new(1.0, 0.0)
    } else {
        Vec2::new(0.0, 1.0)
    };
    let i1 = i0 + o1;
    let i2 = i0 + Vec2::new(1.0, 1.0);
    let v0 = Vec2::new(i0.x - i0.y * 0.5, i0.y);
    let v1 = Vec2::new(i1.x - i1.y * 0.5, i1.y);
    let v2 = Vec2::new(i2.x - i2.y * 0.5, i2.y);
    let n = simplex_corner(v0, p - v0, period, seed)
        + simplex_corner(v1, p - v1, period, seed)
        + simplex_corner(v2, p - v2, period, seed);
    (0.5 + n * 5.0).clamp(0.0, 1.0)
}

/// Distances to the nearest and second nearest feature points.
pub fn worley(p: Vec2, period: IVec2, seed: u32) -> Vec2 {
    let base = floor2(p);
    let cell = to_ivec(base);
    let f = p - base;
    let mut f1 = 8.0f32;
    let mut f2 = 8.0f32;
    for j in -1..=1 {
        for i in -1..=1 {
            let offset = IVec2::new(i, j);
            let point = Vec2::new(i as f32, j as f32)
                + unit_float2(hash_cell(wrap_cell(cell + offset, period), seed));
            let d = length(point - f);
            if d < f1 {
                f2 = f1;
                f1 = d;
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    Vec2::new(f1, f2)
}

/// Worley noise: mode 0 is F1, 1 is F2 (scaled), 2 is F2 − F1.
pub fn worley_noise(p: Vec2, period: IVec2, seed: u32, mode: u32) -> f32 {
    let f = worley(p, period, seed);
    match mode {
        0 => f.x.clamp(0.0, 1.0),
        1 => (f.y * 0.75).clamp(0.0, 1.0),
        _ => (f.y - f.x).clamp(0.0, 1.0),
    }
}

/// Seed of octave `octave`.
pub fn octave_seed(seed: u32, octave: i32) -> u32 {
    seed.wrapping_add((octave as u32).wrapping_mul(1013))
}

/// Fractal sum of Perlin octaves: `kind` 0 fBm, 1 ridged, 2 turbulence.
pub fn fractal(p: Vec2, period: IVec2, seed: u32, octaves: i32, gain: f32, kind: u32) -> f32 {
    let mut sum = 0.0f32;
    let mut norm = 0.0f32;
    let mut amplitude = 1.0f32;
    let mut q = p;
    let mut per = period;
    for octave in 0..octaves {
        let n = perlin_noise(q, per, octave_seed(seed, octave));
        let value = match kind {
            1 => {
                let r = 1.0 - (2.0 * n - 1.0).abs();
                r * r
            }
            2 => (2.0 * n - 1.0).abs(),
            _ => n,
        };
        sum += amplitude * value;
        norm += amplitude;
        amplitude *= gain;
        q *= 2.0;
        per *= 2;
    }
    sum / norm
}

/// fBm of `p` displaced by two other fBm.
pub fn warp_noise(
    p: Vec2,
    period: IVec2,
    seed: u32,
    octaves: i32,
    gain: f32,
    strength: f32,
) -> f32 {
    let wx = fractal(
        p + Vec2::new(5.2, 1.3),
        period,
        seed.wrapping_add(17),
        octaves,
        gain,
        0,
    );
    let wy = fractal(
        p + Vec2::new(1.7, 9.2),
        period,
        seed.wrapping_add(31),
        octaves,
        gain,
        0,
    );
    let q = p + (Vec2::new(wx, wy) - 0.5) * (2.0 * strength);
    fractal(q, period, seed, octaves, gain, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Samples of `noise` over one period, as (min, max, mean).
    fn range(noise: impl Fn(Vec2) -> f32) -> (f32, f32, f32) {
        let mut values = Vec::new();
        for j in 0..64 {
            for i in 0..64 {
                values.push(noise(Vec2::new(i as f32, j as f32) / 64.0 * 4.0 + 0.013));
            }
        }
        let min = values.iter().copied().fold(f32::MAX, f32::min);
        let max = values.iter().copied().fold(f32::MIN, f32::max);
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        (min, max, mean)
    }

    #[test]
    fn noises_cover_the_unit_range_around_the_middle() {
        let period = IVec2::new(4, 4);
        type Noise = Box<dyn Fn(Vec2) -> f32>;
        let noises: [(&str, Noise); 6] = [
            ("value", Box::new(move |p| value_noise(p, period, 3))),
            ("perlin", Box::new(move |p| perlin_noise(p, period, 3))),
            ("simplex", Box::new(move |p| simplex_noise(p, period, 3))),
            ("worley", Box::new(move |p| worley_noise(p, period, 3, 0))),
            ("fbm", Box::new(move |p| fractal(p, period, 3, 5, 0.5, 0))),
            (
                "warp",
                Box::new(move |p| warp_noise(p, period, 3, 4, 0.5, 0.5)),
            ),
        ];
        for (name, noise) in noises {
            let (min, max, mean) = range(noise);
            assert!(min >= 0.0 && max <= 1.0, "{name}: {min}..{max}");
            assert!(max - min > 0.35, "{name} is too flat: {min}..{max}");
            assert!((0.2..0.8).contains(&mean), "{name}: mean {mean}");
        }
    }

    #[test]
    fn noises_repeat_with_their_period() {
        let period = IVec2::new(3, 5);
        for p in [
            Vec2::new(0.3, 0.7),
            Vec2::new(2.9, 4.1),
            Vec2::new(-1.2, 0.5),
        ] {
            let shifted = p + Vec2::new(3.0, -5.0);
            assert!((value_noise(p, period, 1) - value_noise(shifted, period, 1)).abs() < 1e-5);
            assert!((perlin_noise(p, period, 1) - perlin_noise(shifted, period, 1)).abs() < 1e-5);
            assert!((simplex_noise(p, period, 1) - simplex_noise(shifted, period, 1)).abs() < 1e-5);
            assert!(
                (worley_noise(p, period, 1, 2) - worley_noise(shifted, period, 1, 2)).abs() < 1e-5
            );
        }
    }
}
