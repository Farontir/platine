//! Mirror of the procedural materials: `shaders/procedural/material_main.wgsl` and
//! `shaders/procedural/materials/*`.

use glam::{Vec3, Vec4Swizzles as _};
use patine_core::mask::{ParamValue, Params};

use super::noise::{fractal, worley};
use super::pattern::{pattern_fibers, soft_step};
use super::projection::{Projected, noise_period, project};
use super::surface::{Surface, normalize_or};
use super::{Vec2, fract};
use crate::evaluate::EvaluationUniforms;

/// A material at one point.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MaterialSample {
    /// Linear base color.
    pub base_color: Vec3,
    /// Metallic.
    pub metallic: f32,
    /// Roughness.
    pub roughness: f32,
    /// Height.
    pub height: f32,
}

const RELIEF: f32 = 0.02;

fn float(params: &Params, name: &str) -> f32 {
    match params.get(name) {
        Some(ParamValue::Float(v)) => *v,
        _ => 0.0,
    }
}

fn int(params: &Params, name: &str) -> i32 {
    match params.get(name) {
        Some(ParamValue::Int(v)) => *v,
        _ => 0,
    }
}

fn color(params: &Params, name: &str) -> Vec3 {
    match params.get(name) {
        Some(ParamValue::Color(c)) => Vec3::new(c[0], c[1], c[2]),
        _ => Vec3::ZERO,
    }
}

/// WGSL's `mix` of vectors.
fn mix3(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a * (1.0 - t) + b * t
}

/// One material: its sample and its height at pattern-space `q`.
struct Material<'a> {
    id: &'a str,
    params: &'a Params,
    e: &'a EvaluationUniforms,
}

impl Material<'_> {
    fn seed(&self) -> u32 {
        int(self.params, "seed").max(0) as u32
    }

    fn sample(&self, q: Vec2) -> MaterialSample {
        let p = self.params;
        let relief = float(p, "relief");
        let roughness = float(p, "roughness");
        match self.id {
            "material.steel_brushed" => {
                let b = self.brush(q) - 0.5;
                let brushing = float(p, "brushing");
                MaterialSample {
                    base_color: color(p, "color") * (1.0 + 0.3 * brushing * b),
                    metallic: 1.0,
                    roughness: (roughness + 0.25 * brushing * b).clamp(0.02, 1.0),
                    height: b * relief * 0.2,
                }
            }
            "material.copper" => {
                let n = self.tint(q) - 0.5;
                let o = self.oxide(q);
                let variation = float(p, "variation");
                let metal = color(p, "color") * (1.0 + variation * n);
                MaterialSample {
                    base_color: mix3(metal, Vec3::new(0.18, 0.1, 0.07), o),
                    metallic: 1.0 - o,
                    roughness: super::mix(roughness + 0.3 * variation * n, 0.8, o).clamp(0.02, 1.0),
                    height: n * relief * 0.2 - o * relief * 0.1,
                }
            }
            "material.plastic_grained" => {
                let g = self.bumps(q) - 0.5;
                let grain = float(p, "grain");
                MaterialSample {
                    base_color: color(p, "color") * (1.0 + 0.15 * grain * g),
                    metallic: 0.0,
                    roughness: (roughness - 0.2 * grain * g).clamp(0.02, 1.0),
                    height: g * grain * relief * 0.1,
                }
            }
            "material.wood" => {
                let r = self.ring(q);
                let f = self.fiber(q) - 0.5;
                MaterialSample {
                    base_color: mix3(
                        color(p, "light"),
                        color(p, "dark"),
                        (0.8 * r + 0.4 * f).clamp(0.0, 1.0),
                    ),
                    metallic: 0.0,
                    roughness: (roughness + 0.1 * r).clamp(0.02, 1.0),
                    height: (f * 0.3 - r * 0.1) * relief,
                }
            }
            "material.concrete" => {
                let n = self.grain(q) - 0.5;
                let pore = self.pore(q);
                MaterialSample {
                    base_color: color(p, "color")
                        * (1.0 + 0.5 * n)
                        * (1.0 - 0.4 * self.stain(q))
                        * (1.0 - 0.5 * pore),
                    metallic: 0.0,
                    roughness: (roughness + 0.1 * n).clamp(0.02, 1.0),
                    height: (n * 0.5 - pore * 0.5) * relief,
                }
            }
            _ => MaterialSample::default(),
        }
    }

    fn height(&self, q: Vec2) -> f32 {
        let p = self.params;
        let relief = float(p, "relief");
        match self.id {
            "material.steel_brushed" => (self.brush(q) - 0.5) * relief * 0.2,
            "material.copper" => (self.tint(q) - 0.5) * relief * 0.2 - self.oxide(q) * relief * 0.1,
            "material.plastic_grained" => (self.bumps(q) - 0.5) * float(p, "grain") * relief * 0.1,
            "material.wood" => ((self.fiber(q) - 0.5) * 0.3 - self.ring(q) * 0.1) * relief,
            "material.concrete" => ((self.grain(q) - 0.5) * 0.5 - self.pore(q) * 0.5) * relief,
            _ => 0.0,
        }
    }

    fn scale(&self) -> i32 {
        int(self.params, "scale")
    }

    fn brush(&self, q: Vec2) -> f32 {
        pattern_fibers(q, self.scale(), 16, self.seed())
    }

    fn tint(&self, q: Vec2) -> f32 {
        let scale = self.scale();
        fractal(
            q * scale as f32,
            noise_period(self.e, scale),
            self.seed(),
            5,
            0.5,
            0,
        )
    }

    fn oxide(&self, q: Vec2) -> f32 {
        let scale = self.scale() * 2;
        let n = fractal(
            q * scale as f32,
            noise_period(self.e, scale),
            self.seed().wrapping_add(7),
            4,
            0.55,
            0,
        );
        let oxidation = float(self.params, "oxidation");
        soft_step(1.0 - oxidation, 1.15 - oxidation, 1.0 - n)
    }

    fn bumps(&self, q: Vec2) -> f32 {
        let scale = self.scale();
        let f = worley(q * scale as f32, noise_period(self.e, scale), self.seed());
        1.0 - soft_step(0.0, 0.7, f.x)
    }

    fn ring(&self, q: Vec2) -> f32 {
        let warp = fractal(q * 3.0, noise_period(self.e, 3), self.seed(), 4, 0.5, 0) - 0.5;
        let r = fract(
            q.y * int(self.params, "rings") as f32 + float(self.params, "distortion") * 2.0 * warp,
        );
        soft_step(0.55, 0.8, r) * (1.0 - soft_step(0.85, 1.0, r))
    }

    fn fiber(&self, q: Vec2) -> f32 {
        pattern_fibers(q, 2, 24, self.seed().wrapping_add(3))
    }

    fn grain(&self, q: Vec2) -> f32 {
        let scale = self.scale();
        fractal(
            q * scale as f32,
            noise_period(self.e, scale),
            self.seed(),
            6,
            0.55,
            0,
        )
    }

    fn pore(&self, q: Vec2) -> f32 {
        let scale = self.scale() * 6;
        let f = worley(
            q * scale as f32,
            noise_period(self.e, scale),
            self.seed().wrapping_add(11),
        );
        (1.0 - soft_step(0.05, 0.2, f.x)) * float(self.params, "pores")
    }

    fn stain(&self, q: Vec2) -> f32 {
        fractal(
            q * 2.0,
            noise_period(self.e, 2),
            self.seed().wrapping_add(5),
            4,
            0.5,
            0,
        ) * float(self.params, "stains")
    }

    fn blended(&self, p: &Projected) -> MaterialSample {
        let weighted = |m: MaterialSample, w: f32| MaterialSample {
            base_color: m.base_color * w,
            metallic: m.metallic * w,
            roughness: m.roughness * w,
            height: m.height * w,
        };
        let added = |a: MaterialSample, b: MaterialSample| MaterialSample {
            base_color: a.base_color + b.base_color,
            metallic: a.metallic + b.metallic,
            roughness: a.roughness + b.roughness,
            height: a.height + b.height,
        };
        let mut m = weighted(self.sample(p.q[0]), p.w.x);
        if p.w.y > 0.0 {
            m = added(m, weighted(self.sample(p.q[1]), p.w.y));
        }
        if p.w.z > 0.0 {
            m = added(m, weighted(self.sample(p.q[2]), p.w.z));
        }
        m
    }

    fn blended_height(&self, p: &Projected) -> f32 {
        let mut h = p.w.x * self.height(p.q[0]);
        if p.w.y > 0.0 {
            h += p.w.y * self.height(p.q[1]);
        }
        if p.w.z > 0.0 {
            h += p.w.z * self.height(p.q[2]);
        }
        h
    }

    fn height_near(&self, s: &Surface, du: Vec2, dp: Vec3) -> f32 {
        let moved = Surface {
            uv: s.uv + du,
            position: s.position + dp,
            ..*s
        };
        self.blended_height(&project(self.e, &moved))
    }

    fn normal(&self, s: &Surface, h0: f32) -> Vec3 {
        let e = 0.5 / self.e.size;
        let g = if self.e.kind == 0 || self.e.flat == 1 {
            Vec2::new(
                self.height_near(s, Vec2::new(e, 0.0), Vec3::ZERO) - h0,
                self.height_near(s, Vec2::new(0.0, e), Vec3::ZERO) - h0,
            ) / e
        } else {
            let n = s.normal;
            let tangent = s.tangent.xyz();
            let t = normalize_or(tangent - n * n.dot(tangent), Vec3::X);
            let b = n.cross(t) * s.tangent.w;
            let step = e * self.e.center[3];
            Vec2::new(
                self.height_near(s, Vec2::ZERO, t * step) - h0,
                self.height_near(s, Vec2::ZERO, b * step) - h0,
            ) / e
        };
        (-g * RELIEF).extend(1.0).normalize()
    }
}

/// The outputs of the material `id` with resolved `params` on surface `s`, as the pass
/// writes them (RGBA), in the order of [`crate::def::Output::index`]: base color,
/// metallic, roughness, normal, height.
pub fn evaluate_material(
    id: &str,
    params: &Params,
    e: &EvaluationUniforms,
    s: &Surface,
) -> [[f32; 4]; 5] {
    let material = Material { id, params, e };
    let m = material.blended(&project(e, s));
    let n = material.normal(s, m.height);
    [
        [m.base_color.x, m.base_color.y, m.base_color.z, 1.0],
        [m.metallic, 0.0, 0.0, 0.0],
        [m.roughness, 0.0, 0.0, 0.0],
        [n.x, n.y, n.z, 0.0],
        [m.height, 0.0, 0.0, 0.0],
    ]
}
