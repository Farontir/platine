//! Mirror of `shaders/procedural/surface.wgsl`: the surface shown by a texel.

use glam::{Vec3, Vec4};

use super::Vec2;
use crate::surface::GpuVertex;

/// Surface at a texel.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Surface {
    /// Position.
    pub position: Vec3,
    /// Unit normal.
    pub normal: Vec3,
    /// xyz: unit tangent, w: bitangent sign.
    pub tangent: Vec4,
    /// UV.
    pub uv: Vec2,
}

fn dot(a: Vec2, b: Vec2) -> f32 {
    a.x * b.x + a.y * b.y
}

/// Barycentric coordinates of the point of triangle (a, b, c) closest to `p`.
pub fn closest_barycentric(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> Vec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return Vec3::new(1.0, 0.0, 0.0);
    }
    let bp = p - b;
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return Vec3::new(0.0, 1.0, 0.0);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return Vec3::new(1.0 - v, v, 0.0);
    }
    let cp = p - c;
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return Vec3::new(0.0, 0.0, 1.0);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return Vec3::new(1.0 - w, 0.0, w);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return Vec3::new(0.0, 1.0 - w, w);
    }
    let sum = va + vb + vc;
    if sum <= 1e-20 {
        return Vec3::new(1.0, 0.0, 0.0);
    }
    let v = vb / sum;
    let w = vc / sum;
    Vec3::new(1.0 - v - w, v, w)
}

/// WGSL's `length(v) > 1e-12 ? v / length(v) : fallback`.
pub fn normalize_or(v: Vec3, fallback: Vec3) -> Vec3 {
    let l = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    if l > 1e-12 { v / l } else { fallback }
}

/// The surface at `texel` of a `size` × `size` map whose entry there is `entry` (the
/// triangle index + 1 in `indices`, 0 for none).
pub fn surface_at(
    vertices: &[GpuVertex],
    indices: &[[u32; 3]],
    entry: u32,
    texel: [u32; 2],
    size: f32,
) -> Surface {
    if entry == 0 {
        return Surface {
            tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
            ..Surface::default()
        };
    }
    let [ia, ib, ic] = indices[(entry - 1) as usize];
    let (a, b, c) = (
        vertices[ia as usize],
        vertices[ib as usize],
        vertices[ic as usize],
    );
    let uv = |v: &GpuVertex| Vec2::new(v.uv[0], v.uv[1]);
    let xyz = |v: [f32; 4]| Vec3::new(v[0], v[1], v[2]);
    let p = (Vec2::new(texel[0] as f32, texel[1] as f32) + 0.5) / size;
    let w = closest_barycentric(p, uv(&a), uv(&b), uv(&c));
    let mix3 = |x: Vec3, y: Vec3, z: Vec3| x * w.x + y * w.y + z * w.z;
    let tangent = normalize_or(
        mix3(xyz(a.tangent), xyz(b.tangent), xyz(c.tangent)),
        Vec3::X,
    );
    Surface {
        position: mix3(xyz(a.position), xyz(b.position), xyz(c.position)),
        normal: normalize_or(mix3(xyz(a.normal), xyz(b.normal), xyz(c.normal)), Vec3::Z),
        tangent: tangent.extend(if a.tangent[3] >= 0.0 { 1.0 } else { -1.0 }),
        uv: uv(&a) * w.x + uv(&b) * w.y + uv(&c) * w.z,
    }
}

/// The surface of the unit square at `texel` (thumbnails).
pub fn flat_surface(texel: [u32; 2], size: f32) -> Surface {
    let uv = (Vec2::new(texel[0] as f32, texel[1] as f32) + 0.5) / size;
    Surface {
        position: uv.extend(0.0),
        normal: Vec3::Z,
        tangent: Vec4::new(1.0, 0.0, 0.0, 1.0),
        uv,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closest_points_are_inside_or_on_the_border() {
        let (a, b, c) = (Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0));
        // Inside: the point itself.
        let w = closest_barycentric(Vec2::new(0.25, 0.25), a, b, c);
        assert!((w - Vec3::new(0.5, 0.25, 0.25)).abs().max_element() < 1e-6);
        // Beyond an edge, beyond a corner.
        assert_eq!(
            closest_barycentric(Vec2::new(0.5, -1.0), a, b, c),
            Vec3::new(0.5, 0.5, 0.0)
        );
        assert_eq!(
            closest_barycentric(Vec2::new(2.0, -1.0), a, b, c),
            Vec3::new(0.0, 1.0, 0.0)
        );
        let w = closest_barycentric(Vec2::new(1.0, 1.0), a, b, c);
        assert!((w - Vec3::new(0.0, 0.5, 0.5)).abs().max_element() < 1e-6);
        // A degenerate triangle gives a corner.
        assert_eq!(
            closest_barycentric(Vec2::new(0.3, 0.3), a, a, a),
            Vec3::new(1.0, 0.0, 0.0)
        );
    }
}
