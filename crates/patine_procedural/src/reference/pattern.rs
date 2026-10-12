//! Mirror of `shaders/procedural/pattern.wgsl`: patterns repeating with period 1.

use super::hash::{hash_cell, pcg, unit_float, unit_float2, wrap_cell};
use super::noise::fractal;
use super::{IVec2, Vec2, floor2, fract, to_ivec};

fn dot(a: Vec2, b: Vec2) -> f32 {
    a.x * b.x + a.y * b.y
}

fn length(v: Vec2) -> f32 {
    dot(v, v).sqrt()
}

/// Hermite step, sharp when `e1 == e0`.
pub fn soft_step(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-4)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Gradient: 0 along x, 1 along y, 2 triangle along x, 3 radial.
pub fn pattern_gradient(p: Vec2, mode: u32) -> f32 {
    let f = Vec2::new(fract(p.x), fract(p.y));
    match mode {
        0 => f.x,
        1 => f.y,
        2 => 1.0 - (2.0 * f.x - 1.0).abs(),
        _ => (1.0 - 2.0 * length(f - 0.5)).clamp(0.0, 1.0),
    }
}

/// Checker of 2 × 2 squares per unit.
pub fn pattern_checker(p: Vec2) -> f32 {
    let c = to_ivec(floor2(p * 2.0));
    (((c.x + c.y) % 2 + 2) % 2) as f32
}

fn cell_shade(d: f32, gap: f32, bevel: f32, variation: f32, h: u32) -> f32 {
    let edge = soft_step(gap * 0.5, gap * 0.5 + bevel, d);
    edge * (1.0 - variation * unit_float(h))
}

/// Square tiles, `count` per unit.
pub fn pattern_tiles(p: Vec2, count: i32, gap: f32, bevel: f32, variation: f32, seed: u32) -> f32 {
    let q = p * count as f32;
    let base = floor2(q);
    let f = q - base;
    let d = f.x.min(1.0 - f.x).min(f.y.min(1.0 - f.y));
    let h = hash_cell(wrap_cell(to_ivec(base), IVec2::new(count, count)), seed);
    cell_shade(d, gap, bevel, variation, h)
}

/// Running bond bricks, `count` per row, 2 × `count` rows per unit.
pub fn pattern_bricks(p: Vec2, count: i32, gap: f32, bevel: f32, variation: f32, seed: u32) -> f32 {
    let rows = 2 * count;
    let mut q = p * Vec2::new(count as f32, rows as f32);
    let row = q.y.floor() as i32;
    q.x += 0.5 * ((row % 2 + 2) % 2) as f32;
    let base = floor2(q);
    let f = q - base;
    let d = (2.0 * f.x.min(1.0 - f.x)).min(f.y.min(1.0 - f.y));
    let h = hash_cell(wrap_cell(to_ivec(base), IVec2::new(count, rows)), seed);
    cell_shade(d, gap, bevel, variation, h)
}

/// `count` stripes per unit across `axis` (0: x, 1: y).
pub fn pattern_stripes(p: Vec2, count: i32, width: f32, softness: f32, axis: u32) -> f32 {
    let x = if axis == 1 { p.y } else { p.x } * count as f32;
    let d = (fract(x) - 0.5).abs() * 2.0;
    1.0 - soft_step(width, width + softness, d)
}

/// Dots, `count` × `count` per unit.
pub fn pattern_circles(p: Vec2, count: i32, radius: f32, softness: f32) -> f32 {
    let q = p * count as f32;
    let f = Vec2::new(fract(q.x), fract(q.y)) - 0.5;
    let d = length(f) * 2.0;
    1.0 - soft_step(radius, radius + softness, d)
}

/// Random thin scratches.
pub fn pattern_scratches(p: Vec2, count: i32, size: f32, width: f32, seed: u32) -> f32 {
    let q = p * count as f32;
    let base = floor2(q);
    let cell = to_ivec(base);
    let f = q - base;
    let period = IVec2::new(count, count);
    let mut value = 0.0f32;
    for j in -1..=1 {
        for i in -1..=1 {
            let offset = IVec2::new(i, j);
            let h = hash_cell(wrap_cell(cell + offset, period), seed);
            let center = Vec2::new(i as f32, j as f32) + unit_float2(h);
            let r = unit_float2(pcg(h.wrapping_add(1))) * 2.0 - 1.0;
            let direction = r / length(r).max(1e-4);
            let half_length = 0.5 * size * (0.5 + 0.5 * unit_float(pcg(h.wrapping_add(2))));
            let t = dot(f - center, direction).clamp(-half_length, half_length);
            let d = length(f - (center + direction * t));
            value = value.max(1.0 - soft_step(0.0, width, d));
        }
    }
    value
}

/// Fibers along y.
pub fn pattern_fibers(p: Vec2, count: i32, stretch: i32, seed: u32) -> f32 {
    let scale = Vec2::new((count * stretch) as f32, count as f32);
    let period = IVec2::new(count * stretch, count);
    fractal(p * scale, period, seed, 2, 0.5, 0)
}
