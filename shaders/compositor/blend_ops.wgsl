// Blend modes of the compositor. Must match crates/patine_compositor/src/reference.rs:
// mode indices are `reference::mode_index`, channel kinds `ChannelKind::index`
// (0 color, 1 scalar, 2 normal).

const EPSILON: f32 = 1e-4;

fn hard_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        return b * 2.0 * s;
    }
    let t = 2.0 * s - 1.0;
    return b + t - b * t;
}

fn soft_light(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        return b - (1.0 - 2.0 * s) * b * (1.0 - b);
    }
    var d: f32;
    if b <= 0.25 {
        d = ((16.0 * b - 12.0) * b + 4.0) * b;
    } else {
        d = sqrt(max(b, 0.0));
    }
    return b + (2.0 * s - 1.0) * (d - b);
}

fn color_dodge(b: f32, s: f32) -> f32 {
    if b <= 0.0 {
        return 0.0;
    }
    if s >= 1.0 - EPSILON {
        return 1.0;
    }
    return min(b / (1.0 - s), 1.0);
}

fn color_burn(b: f32, s: f32) -> f32 {
    if b >= 1.0 {
        return 1.0;
    }
    if s <= EPSILON {
        return 0.0;
    }
    return 1.0 - min((1.0 - b) / s, 1.0);
}

fn divide(b: f32, s: f32) -> f32 {
    if s <= EPSILON {
        return select(0.0, 1.0, b > 0.0);
    }
    return b / s;
}

fn separable(mode: u32, b: f32, s: f32) -> f32 {
    switch mode {
        case 3u: { return b * s; }
        case 4u: { return divide(b, s); }
        case 5u: { return b + s - b * s; }
        case 6u: { return hard_light(s, b); }
        case 7u: { return soft_light(b, s); }
        case 8u: { return hard_light(b, s); }
        case 9u: { return b + s; }
        case 10u: { return b + s - 1.0; }
        case 11u: { return b - s; }
        case 12u: { return abs(b - s); }
        case 13u: { return b + s - 2.0 * b * s; }
        case 14u: { return min(b, s); }
        case 15u: { return max(b, s); }
        case 16u: { return color_dodge(b, s); }
        case 17u: { return color_burn(b, s); }
        default: { return s; }
    }
}

// Non-separable modes of the W3C Compositing and Blending specification.
fn lum(c: vec3<f32>) -> f32 {
    return 0.3 * c.r + 0.59 * c.g + 0.11 * c.b;
}

fn clip_color(c: vec3<f32>) -> vec3<f32> {
    let l = lum(c);
    let n = min(min(c.r, c.g), c.b);
    let x = max(max(c.r, c.g), c.b);
    var out = c;
    // The spec divides by l - n and x - l, zero for a gray: nothing to clip then.
    if n < 0.0 && l - n > EPSILON {
        out = l + (out - l) * l / (l - n);
    }
    if x > 1.0 && x - l > EPSILON {
        out = l + (out - l) * (1.0 - l) / (x - l);
    }
    return out;
}

fn set_lum(c: vec3<f32>, l: f32) -> vec3<f32> {
    return clip_color(c + (l - lum(c)));
}

fn sat(c: vec3<f32>) -> f32 {
    return max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b);
}

fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let hi = max(max(c.r, c.g), c.b);
    let lo = min(min(c.r, c.g), c.b);
    if hi > lo {
        return (c - lo) * s / (hi - lo);
    }
    return vec3<f32>(0.0);
}

fn blend_rgb(mode: u32, b: vec3<f32>, s: vec3<f32>) -> vec3<f32> {
    switch mode {
        case 18u: { return set_lum(set_sat(s, sat(b)), lum(b)); }
        case 19u: { return set_lum(set_sat(b, sat(s)), lum(b)); }
        case 20u: { return set_lum(s, lum(b)); }
        case 21u: { return set_lum(b, lum(s)); }
        default: {
            return vec3<f32>(separable(mode, b.r, s.r), separable(mode, b.g, s.g), separable(mode, b.b, s.b));
        }
    }
}

fn safe_normalize(v: vec3<f32>) -> vec3<f32> {
    let length_squared = dot(v, v);
    if length_squared > 0.0 {
        return v / sqrt(length_squared);
    }
    return vec3<f32>(0.0, 0.0, 1.0);
}

// Reoriented normal mapping: `detail` applied on top of `base`.
fn reoriented_normal(base: vec3<f32>, detail: vec3<f32>) -> vec3<f32> {
    let t = base + vec3<f32>(0.0, 0.0, 1.0);
    let u = detail * vec3<f32>(-1.0, -1.0, 1.0);
    return safe_normalize(t * dot(t, u) / t.z - u);
}

fn udn_normal(base: vec3<f32>, detail: vec3<f32>) -> vec3<f32> {
    return safe_normalize(vec3<f32>(base.xy + detail.xy, base.z));
}

// mix(dst, blend(dst, src), weight), renormalized for normals; see reference::apply.
fn apply_blend(mode: u32, kind: u32, dst: vec4<f32>, src: vec4<f32>, weight: f32) -> vec4<f32> {
    if mode == 22u {
        if kind == 1u {
            return vec4<f32>(dst.r, 0.0, 0.0, 0.0);
        }
        return dst;
    }
    let w = clamp(weight, 0.0, 1.0);
    if kind == 2u {
        var blended = src.xyz;
        if mode == 23u {
            blended = reoriented_normal(dst.xyz, src.xyz);
        } else if mode == 24u {
            blended = udn_normal(dst.xyz, src.xyz);
        }
        return vec4<f32>(safe_normalize(mix(dst.xyz, blended, w)), dst.w);
    }
    if kind == 1u {
        let c = blend_rgb(mode, vec3<f32>(dst.r), vec3<f32>(src.r));
        return vec4<f32>(mix(dst.r, c.r, w), 0.0, 0.0, 0.0);
    }
    let c = blend_rgb(mode, dst.rgb, src.rgb);
    return vec4<f32>(mix(dst.rgb, c, w), mix(dst.a, src.a, w));
}
