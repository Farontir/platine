//! Mirror of the projection part of `shaders/procedural/evaluate.wgsl`.

use glam::Vec3;

use super::Vec2;
use super::surface::{Surface, normalize_or};
use crate::evaluate::EvaluationUniforms;

/// Up to three pattern-space coordinates and their weights.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Projected {
    /// Coordinates of each plane.
    pub q: [Vec2; 3],
    /// Weight of each plane.
    pub w: Vec3,
}

const PI: f32 = std::f32::consts::PI;

/// Scale, rotation, then offset.
pub fn place(e: &EvaluationUniforms, x: Vec2) -> Vec2 {
    let s = Vec2::new(x.x * e.scale[0], x.y * e.scale[1]);
    let r = e.rotation;
    Vec2::new(r[0] * s.x - r[1] * s.y, r[1] * s.x + r[0] * s.y) + Vec2::from(e.offset)
}

/// Triplanar weights of a normal.
pub fn triplanar_weights(e: &EvaluationUniforms, n: Vec3) -> Vec3 {
    let w = (n.abs() - Vec3::splat(0.57 * (1.0 - e.blend))).max(Vec3::ZERO);
    w / (w.x + w.y + w.z).max(1e-6)
}

/// The projection of surface `s`.
pub fn project(e: &EvaluationUniforms, s: &Surface) -> Projected {
    let center = Vec3::new(e.center[0], e.center[1], e.center[2]);
    let extent = e.center[3];
    let pn = (s.position - center) / extent + 0.5;
    let mut out = Projected {
        w: Vec3::new(1.0, 0.0, 0.0),
        ..Projected::default()
    };
    match e.kind {
        1 => {
            out.q = [
                place(e, Vec2::new(pn.z, 1.0 - pn.y)),
                place(e, Vec2::new(pn.x, pn.z)),
                place(e, Vec2::new(pn.x, 1.0 - pn.y)),
            ];
            out.w = triplanar_weights(e, s.normal);
        }
        2 => out.q[0] = place(e, Vec2::new(pn.x, 1.0 - pn.y)),
        3 => {
            let d = normalize_or(s.position - center, Vec3::Y);
            let u = d.z.atan2(d.x) / (2.0 * PI) + 0.5;
            let v = d.y.clamp(-1.0, 1.0).acos() / PI;
            out.q[0] = place(e, Vec2::new(u, v));
        }
        4 => {
            let d = s.position - center;
            let u = d.z.atan2(d.x) / (2.0 * PI) + 0.5;
            out.q[0] = place(e, Vec2::new(u, 0.5 - d.y / extent));
        }
        _ => out.q[0] = place(e, s.uv),
    }
    out
}

/// Period of a noise of `cells` cells per projection unit.
pub fn noise_period(e: &EvaluationUniforms, cells: i32) -> glam::IVec2 {
    if e.tile[0] > 0 {
        glam::IVec2::new(e.tile[0] * cells, e.tile[1] * cells)
    } else {
        glam::IVec2::splat(1 << 20)
    }
}
