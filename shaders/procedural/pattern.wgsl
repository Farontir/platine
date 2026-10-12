// Tileable patterns of the procedural library. Each pattern repeats with period 1 in `p`
// (one repetition per projection unit) and returns a value in [0, 1]. Must match
// `patine_procedural::reference::pattern`.

// #import procedural/hash
// #import procedural/noise

// Hermite step, as WGSL's smoothstep but defined for e1 == e0 (a sharp step).
fn soft_step(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / max(e1 - e0, 1e-4), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Gradient: 0 along x, 1 along y, 2 triangle along x (0 at the edges, 1 in the middle),
// 3 radial (1 at the center, 0 from the edges on).
fn pattern_gradient(p: vec2<f32>, mode: u32) -> f32 {
    let f = fract(p);
    switch mode {
        case 0u: { return f.x; }
        case 1u: { return f.y; }
        case 2u: { return 1.0 - abs(2.0 * f.x - 1.0); }
        default: { return clamp(1.0 - 2.0 * length(f - 0.5), 0.0, 1.0); }
    }
}

// Checker of 2 × 2 squares per unit.
fn pattern_checker(p: vec2<f32>) -> f32 {
    let c = vec2<i32>(floor(p * 2.0));
    return f32((((c.x + c.y) % 2) + 2) % 2);
}

// Shading of a cell: 1 inside, falling to 0 over `bevel` at `gap / 2` from the border
// (`d` is the distance to the border in cell heights), darkened at random by `variation`.
fn cell_shade(d: f32, gap: f32, bevel: f32, variation: f32, h: u32) -> f32 {
    let edge = soft_step(gap * 0.5, gap * 0.5 + bevel, d);
    return edge * (1.0 - variation * unit_float(h));
}

// Square tiles, `count` per unit.
fn pattern_tiles(p: vec2<f32>, count: i32, gap: f32, bevel: f32, variation: f32, seed: u32) -> f32 {
    let q = p * f32(count);
    let base = floor(q);
    let f = q - base;
    let d = min(min(f.x, 1.0 - f.x), min(f.y, 1.0 - f.y));
    let h = hash_cell(wrap_cell(vec2<i32>(base), vec2<i32>(count, count)), seed);
    return cell_shade(d, gap, bevel, variation, h);
}

// Running bond bricks: `count` bricks per row, 2 × `count` rows per unit, every other row
// shifted by half a brick; a brick is twice as wide as it is high.
fn pattern_bricks(p: vec2<f32>, count: i32, gap: f32, bevel: f32, variation: f32, seed: u32) -> f32 {
    let rows = 2 * count;
    var q = p * vec2<f32>(f32(count), f32(rows));
    let row = i32(floor(q.y));
    q.x += 0.5 * f32(((row % 2) + 2) % 2);
    let base = floor(q);
    let f = q - base;
    let d = min(2.0 * min(f.x, 1.0 - f.x), min(f.y, 1.0 - f.y));
    let h = hash_cell(wrap_cell(vec2<i32>(base), vec2<i32>(count, rows)), seed);
    return cell_shade(d, gap, bevel, variation, h);
}

// `count` stripes per unit across `axis` (0: x, 1: y), `width` their relative width.
fn pattern_stripes(p: vec2<f32>, count: i32, width: f32, softness: f32, axis: u32) -> f32 {
    let x = select(p.x, p.y, axis == 1u) * f32(count);
    let d = abs(fract(x) - 0.5) * 2.0;
    return 1.0 - soft_step(width, width + softness, d);
}

// Dots of relative radius `radius`, `count` × `count` per unit.
fn pattern_circles(p: vec2<f32>, count: i32, radius: f32, softness: f32) -> f32 {
    let f = fract(p * f32(count)) - 0.5;
    let d = length(f) * 2.0;
    return 1.0 - soft_step(radius, radius + softness, d);
}

// Random thin scratches: one segment per cell (`count` × `count` cells per unit) of
// random direction and of length up to `size` cells, `width` cells wide.
fn pattern_scratches(p: vec2<f32>, count: i32, size: f32, width: f32, seed: u32) -> f32 {
    let q = p * f32(count);
    let base = floor(q);
    let cell = vec2<i32>(base);
    let f = q - base;
    let period = vec2<i32>(count, count);
    var value = 0.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let offset = vec2<i32>(i, j);
            let h = hash_cell(wrap_cell(cell + offset, period), seed);
            let center = vec2<f32>(offset) + unit_float2(h);
            let r = unit_float2(pcg(h + 1u)) * 2.0 - 1.0;
            let direction = r / max(length(r), 1e-4);
            let half_length = 0.5 * size * (0.5 + 0.5 * unit_float(pcg(h + 2u)));
            let t = clamp(dot(f - center, direction), -half_length, half_length);
            let d = length(f - (center + direction * t));
            value = max(value, 1.0 - soft_step(0.0, width, d));
        }
    }
    return value;
}

// Fibers along y: Perlin noise `stretch` times finer across x than along y, two octaves.
fn pattern_fibers(p: vec2<f32>, count: i32, stretch: i32, seed: u32) -> f32 {
    let scale = vec2<f32>(f32(count * stretch), f32(count));
    let period = vec2<i32>(count * stretch, count);
    return fractal(p * scale, period, seed, 2, 0.5, 0u);
}
