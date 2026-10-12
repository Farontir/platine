//! CPU reference of the procedural library: the exact mirror of `shaders/procedural/`
//! (same operations, in the same order), against which the GPU is tested.
//!
//! WGSL built-ins are reproduced by their definition: `fract(x) = x − floor(x)` (not
//! Rust's `f32::fract`, which truncates), `mix(a, b, t) = a·(1 − t) + b·t`, and integer `%`
//! truncates like Rust's.

pub mod gray;
pub mod hash;
pub mod material;
pub mod noise;
pub mod pattern;
pub mod projection;
pub mod surface;

pub use glam::{IVec2, Vec2};

/// WGSL's `fract`.
pub fn fract(x: f32) -> f32 {
    x - x.floor()
}

/// WGSL's `mix`.
pub fn mix(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

/// WGSL's `floor` of a vector.
pub fn floor2(p: Vec2) -> Vec2 {
    Vec2::new(p.x.floor(), p.y.floor())
}

/// WGSL's `vec2<i32>(v)` of an integral vector.
pub fn to_ivec(v: Vec2) -> IVec2 {
    IVec2::new(v.x as i32, v.y as i32)
}
