//! Mirror of the gray procedurals (`shaders/procedural/noises/*` and `patterns/*`).

use patine_core::mask::{ParamValue, Params};

use super::Vec2;
use super::noise::{fractal, perlin_noise, simplex_noise, value_noise, warp_noise, worley_noise};
use super::pattern::{
    pattern_bricks, pattern_checker, pattern_circles, pattern_fibers, pattern_gradient,
    pattern_scratches, pattern_stripes, pattern_tiles,
};
use super::projection::{noise_period, project};
use super::surface::Surface;
use crate::def::{ParamKind, ProceduralDef};
use crate::evaluate::EvaluationUniforms;

/// Reads resolved parameters as the shader does.
struct Values<'a> {
    def: &'a ProceduralDef,
    params: &'a Params,
}

impl Values<'_> {
    fn float(&self, name: &str) -> f32 {
        match self.params.get(name) {
            Some(ParamValue::Float(v)) => *v,
            _ => 0.0,
        }
    }

    fn int(&self, name: &str) -> i32 {
        match self.params.get(name) {
            Some(ParamValue::Int(v)) => *v,
            _ => 0,
        }
    }

    fn uint(&self, name: &str) -> u32 {
        match (self.params.get(name), self.def.param(name).map(|p| &p.kind)) {
            (Some(ParamValue::Int(v)), _) => (*v).max(0) as u32,
            (Some(ParamValue::Bool(v)), _) => u32::from(*v),
            (Some(ParamValue::Enum(option)), Some(ParamKind::Enum { options, .. })) => {
                options.iter().position(|o| o == option).unwrap_or(0) as u32
            }
            _ => 0,
        }
    }
}

/// Value of the gray procedural `def` (with resolved `params`) at pattern-space `q`.
pub fn gray(def: &ProceduralDef, params: &Params, e: &EvaluationUniforms, q: Vec2) -> f32 {
    let v = Values { def, params };
    let seed = v.uint("seed");
    let scale = v.int("scale");
    let cells = || q * scale as f32;
    let period = || noise_period(e, scale);
    match def.id.as_str() {
        "noise.value" => value_noise(cells(), period(), seed),
        "noise.perlin" => perlin_noise(cells(), period(), seed),
        "noise.simplex" => simplex_noise(cells(), period(), seed),
        "noise.worley" => worley_noise(cells(), period(), seed, v.uint("mode")),
        "noise.fbm" => fractal(
            cells(),
            period(),
            seed,
            v.int("octaves"),
            v.float("gain"),
            0,
        ),
        "noise.ridged" => fractal(
            cells(),
            period(),
            seed,
            v.int("octaves"),
            v.float("gain"),
            1,
        ),
        "noise.turbulence" => fractal(
            cells(),
            period(),
            seed,
            v.int("octaves"),
            v.float("gain"),
            2,
        ),
        "noise.warp" => warp_noise(
            cells(),
            period(),
            seed,
            v.int("octaves"),
            v.float("gain"),
            v.float("strength"),
        ),
        "pattern.gradient" => pattern_gradient(q, v.uint("mode")),
        "pattern.checker" => (v.uint("invert") as f32 - pattern_checker(q)).abs(),
        "pattern.tiles" => pattern_tiles(
            q,
            v.int("count"),
            v.float("gap"),
            v.float("bevel"),
            v.float("variation"),
            seed,
        ),
        "pattern.bricks" => pattern_bricks(
            q,
            v.int("count"),
            v.float("gap"),
            v.float("bevel"),
            v.float("variation"),
            seed,
        ),
        "pattern.stripes" => pattern_stripes(
            q,
            v.int("count"),
            v.float("width"),
            v.float("softness"),
            v.uint("axis"),
        ),
        "pattern.circles" => {
            pattern_circles(q, v.int("count"), v.float("radius"), v.float("softness"))
        }
        "pattern.scratches" => {
            pattern_scratches(q, v.int("count"), v.float("length"), v.float("width"), seed)
        }
        "pattern.fibers" => pattern_fibers(q, v.int("count"), v.int("stretch"), seed),
        _ => 0.0,
    }
}

/// Value of the gray procedural on surface `s`: blended over the planes of its projection.
pub fn evaluate_gray(
    def: &ProceduralDef,
    params: &Params,
    e: &EvaluationUniforms,
    s: &Surface,
) -> f32 {
    let p = project(e, s);
    let mut value = p.w.x * gray(def, params, e, p.q[0]);
    if p.w.y > 0.0 {
        value += p.w.y * gray(def, params, e, p.q[1]);
    }
    if p.w.z > 0.0 {
        value += p.w.z * gray(def, params, e, p.q[2]);
    }
    value
}
