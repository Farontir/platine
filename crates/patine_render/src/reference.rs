//! CPU reference implementations of the viewport's GPU algorithms. Slow and readable;
//! the GPU versions are tested against them.
//!
//! Cube faces follow the convention shared by Vulkan, D3D and Metal: faces +X, -X, +Y,
//! -Y, +Z, -Z, texture rows going down.

use std::f32::consts::{PI, TAU};

use glam::{Mat3, Vec2, Vec3};

use crate::environment::EquirectImage;
use crate::settings::ToneMapper;

/// Direction of the cube face `face` at face coordinates `s`, `t` in [-1, 1] (t down).
pub fn cube_direction(face: u32, s: f32, t: f32) -> Vec3 {
    let d = match face {
        0 => Vec3::new(1.0, -t, -s),
        1 => Vec3::new(-1.0, -t, s),
        2 => Vec3::new(s, 1.0, t),
        3 => Vec3::new(s, -1.0, -t),
        4 => Vec3::new(s, -t, 1.0),
        _ => Vec3::new(-s, -t, -1.0),
    };
    d.normalize()
}

/// Direction through the center of texel (`x`, `y`) of a `size`² face.
pub fn cube_texel_direction(face: u32, x: u32, y: u32, size: u32) -> Vec3 {
    let s = (x as f32 + 0.5) / size as f32 * 2.0 - 1.0;
    let t = (y as f32 + 0.5) / size as f32 * 2.0 - 1.0;
    cube_direction(face, s, t)
}

/// Solid angle of texel (`x`, `y`) of a `size`² face.
pub fn cube_texel_solid_angle(x: u32, y: u32, size: u32) -> f32 {
    // Area element of the unit cube face projected on the sphere: integrate
    // atan(s·t / sqrt(s² + t² + 1)) over the texel corners.
    let corner = |s: f32, t: f32| (s * t).atan2((s * s + t * t + 1.0).sqrt());
    let step = 2.0 / size as f32;
    let (s0, t0) = (x as f32 * step - 1.0, y as f32 * step - 1.0);
    let (s1, t1) = (s0 + step, t0 + step);
    corner(s1, t1) - corner(s0, t1) - corner(s1, t0) + corner(s0, t0)
}

/// A cubemap level: six square faces of RGB radiance, rows going down.
#[derive(Clone, Debug, PartialEq)]
pub struct CubeImage {
    /// Width and height of a face.
    pub size: u32,
    /// Texels of each face, row-major.
    pub faces: [Vec<Vec3>; 6],
}

impl CubeImage {
    /// Texel of `face` at (`x`, `y`).
    pub fn texel(&self, face: u32, x: u32, y: u32) -> Vec3 {
        self.faces[face as usize][(y * self.size + x) as usize]
    }

    /// Builds a cube by evaluating `radiance` at every texel center.
    pub fn from_fn(size: u32, radiance: impl Fn(Vec3) -> Vec3) -> Self {
        Self {
            size,
            faces: std::array::from_fn(|face| {
                (0..size * size)
                    .map(|i| radiance(cube_texel_direction(face as u32, i % size, i / size, size)))
                    .collect()
            }),
        }
    }
}

/// Converts an equirectangular image to a cube of `size`² faces (bilinear sampling).
pub fn equirect_to_cube(image: &EquirectImage, size: u32) -> CubeImage {
    CubeImage::from_fn(size, |d| image.sample(d))
}

/// Next mip level: 2 × 2 box filter inside each face.
pub fn downsample_cube(cube: &CubeImage) -> CubeImage {
    let size = (cube.size / 2).max(1);
    let faces = std::array::from_fn(|face| {
        (0..size * size)
            .map(|i| {
                let (x, y) = (i % size * 2, i / size * 2);
                let at = |dx: u32, dy: u32| {
                    cube.texel(
                        face as u32,
                        (x + dx).min(cube.size - 1),
                        (y + dy).min(cube.size - 1),
                    )
                };
                (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) * 0.25
            })
            .collect()
    });
    CubeImage { size, faces }
}

/// GGX normal distribution, `alpha` = roughness².
pub fn ggx_distribution(n_dot_h: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    a2 / (PI * d * d)
}

/// Height-correlated Smith visibility term V = G / (4 N·L N·V).
pub fn smith_visibility(n_dot_v: f32, n_dot_l: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let gv = n_dot_l * (n_dot_v * n_dot_v * (1.0 - a2) + a2).sqrt();
    let gl = n_dot_v * (n_dot_l * n_dot_l * (1.0 - a2) + a2).sqrt();
    0.5 / (gv + gl).max(1e-7)
}

/// Schlick's Fresnel approximation.
pub fn fresnel_schlick(f0: Vec3, v_dot_h: f32) -> Vec3 {
    f0 + (Vec3::ONE - f0) * (1.0 - v_dot_h).clamp(0.0, 1.0).powi(5)
}

/// `i`-th point of the `n`-point Hammersley set.
pub fn hammersley(i: u32, n: u32) -> Vec2 {
    Vec2::new(
        i as f32 / n as f32,
        i.reverse_bits() as f32 * 2.328_306_4e-10,
    )
}

/// Half vector sampled proportionally to D(h)·(n·h) around `n`.
pub fn importance_sample_ggx(xi: Vec2, n: Vec3, alpha: f32) -> Vec3 {
    let a2 = alpha * alpha;
    let phi = TAU * xi.x;
    let cos_theta = ((1.0 - xi.y) / (1.0 + (a2 - 1.0) * xi.y)).sqrt();
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    let local = Vec3::new(sin_theta * phi.cos(), sin_theta * phi.sin(), cos_theta);
    let up = if n.z.abs() < 0.999 { Vec3::Z } else { Vec3::X };
    let tangent = up.cross(n).normalize();
    let bitangent = n.cross(tangent);
    (tangent * local.x + bitangent * local.y + n * local.z).normalize()
}

/// Split-sum BRDF integral (scale A and bias B of F0) for `n_dot_v` and perceptual
/// `roughness`, with `samples` Hammersley samples. Same algorithm as `brdf_lut.wgsl`.
pub fn brdf_lut(n_dot_v: f32, roughness: f32, samples: u32) -> Vec2 {
    let alpha = roughness * roughness;
    let v = Vec3::new((1.0 - n_dot_v * n_dot_v).max(0.0).sqrt(), 0.0, n_dot_v);
    let mut result = Vec2::ZERO;
    for i in 0..samples {
        let h = importance_sample_ggx(hammersley(i, samples), Vec3::Z, alpha);
        let l = 2.0 * v.dot(h) * h - v;
        let (n_dot_l, n_dot_h, v_dot_h) = (l.z, h.z.max(0.0), v.dot(h).max(0.0));
        if n_dot_l > 0.0 {
            let visibility = smith_visibility(n_dot_v, n_dot_l, alpha);
            // f·cosθ / pdf with pdf(l) = D·(n·h) / (4 v·h).
            let weight = visibility * 4.0 * v_dot_h * n_dot_l / n_dot_h.max(1e-7);
            let fc = (1.0 - v_dot_h).powi(5);
            result += Vec2::new((1.0 - fc) * weight, fc * weight);
        }
    }
    result / samples as f32
}

/// Exact GGX-prefiltered radiance of `cube` around `n` (n = v = r) for perceptual
/// `roughness`: every source texel weighted by max(n·l, 0) · D(h) · dω, the limit of the
/// importance-sampled estimator used on the GPU.
pub fn prefilter_brute_force(cube: &CubeImage, n: Vec3, roughness: f32) -> Vec3 {
    let alpha = (roughness * roughness).max(1e-4);
    let mut sum = Vec3::ZERO;
    let mut weight_sum = 0.0;
    for face in 0..6 {
        for y in 0..cube.size {
            for x in 0..cube.size {
                let l = cube_texel_direction(face, x, y, cube.size);
                let n_dot_l = n.dot(l);
                if n_dot_l <= 0.0 {
                    continue;
                }
                let h = (n + l).normalize();
                let weight = n_dot_l
                    * ggx_distribution(n.dot(h), alpha)
                    * cube_texel_solid_angle(x, y, cube.size);
                sum += cube.texel(face, x, y) * weight;
                weight_sum += weight;
            }
        }
    }
    sum / weight_sum.max(1e-12)
}

/// Linear to sRGB transfer function, input clamped to [0, 1].
pub fn srgb_encode(linear: Vec3) -> Vec3 {
    linear.clamp(Vec3::ZERO, Vec3::ONE).map(|c| {
        if c <= 0.003_130_8 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
    })
}

/// Display operator `operator` on linear radiance, returning linear display values in
/// [0, 1]. Same formulas as `shaders/render/tonemap_ops.wgsl`.
pub fn tone_map(operator: ToneMapper, color: Vec3) -> Vec3 {
    match operator {
        ToneMapper::AgX => agx(color),
        ToneMapper::Aces => aces(color),
        ToneMapper::PbrNeutral => pbr_neutral(color),
        ToneMapper::Linear => color.clamp(Vec3::ZERO, Vec3::ONE),
    }
}

fn agx(color: Vec3) -> Vec3 {
    let inset = Mat3::from_cols_array(&[
        0.842_479_06,
        0.042_328_24,
        0.042_375_655,
        0.078_433_6,
        0.878_468_6,
        0.078_433_6,
        0.079_223_745,
        0.079_166_13,
        0.879_143,
    ]);
    let outset = Mat3::from_cols_array(&[
        1.196_879,
        -0.052_896_85,
        -0.052_971_635,
        -0.098_020_88,
        1.151_903_1,
        -0.098_043_45,
        -0.099_029_74,
        -0.098_961_18,
        1.151_073_7,
    ]);
    let (min_ev, max_ev) = (-12.473_93_f32, 4.026_069_f32);
    let v = inset * color.max(Vec3::splat(1e-10));
    let v = (v
        .map(f32::log2)
        .clamp(Vec3::splat(min_ev), Vec3::splat(max_ev))
        - min_ev)
        / (max_ev - min_ev);
    let v2 = v * v;
    let v4 = v2 * v2;
    let v =
        15.5 * v4 * v2 - 40.14 * v4 * v + 31.96 * v4 - 6.868 * v2 * v + 0.4298 * v2 + 0.1191 * v
            - Vec3::splat(0.00232);
    (outset * v)
        .clamp(Vec3::ZERO, Vec3::ONE)
        .map(|c| c.powf(2.2))
}

fn aces(color: Vec3) -> Vec3 {
    let input = Mat3::from_cols_array(&[
        0.597_19, 0.076, 0.028_4, 0.354_58, 0.908_34, 0.133_83, 0.048_23, 0.015_66, 0.837_77,
    ]);
    let output = Mat3::from_cols_array(&[
        1.604_75, -0.102_08, -0.003_27, -0.531_08, 1.108_13, -0.072_76, -0.073_67, -0.006_05,
        1.076_02,
    ]);
    let v = input * color;
    let a = v * (v + 0.024_578_6) - Vec3::splat(0.000_090_537);
    let b = v * (0.983_729 * v + 0.432_951) + Vec3::splat(0.238_081);
    (output * (a / b)).clamp(Vec3::ZERO, Vec3::ONE)
}

fn pbr_neutral(input: Vec3) -> Vec3 {
    let start_compression = 0.8 - 0.04;
    let desaturation = 0.15;
    let mut color = input.max(Vec3::ZERO);
    let x = color.min_element();
    let offset = if x < 0.08 { x - 6.25 * x * x } else { 0.04 };
    color -= Vec3::splat(offset);
    let peak = color.max_element();
    if peak < start_compression {
        return color.clamp(Vec3::ZERO, Vec3::ONE);
    }
    let d = 1.0 - start_compression;
    let new_peak = 1.0 - d * d / (peak + d - start_compression);
    color *= new_peak / peak;
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    color
        .lerp(Vec3::splat(new_peak), g)
        .clamp(Vec3::ZERO, Vec3::ONE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_mappers_are_monotonic_and_bounded() {
        for operator in ToneMapper::ALL {
            let mut previous = -1.0;
            for i in 0..200 {
                let x = (i as f32 * 0.1 - 12.0).exp2();
                let y = tone_map(operator, Vec3::splat(x)).x;
                assert!((0.0..=1.0).contains(&y), "{operator:?}({x}) = {y}");
                assert!(y >= previous - 1e-6, "{operator:?} decreases at {x}");
                previous = y;
            }
        }
        // Black stays black, very bright saturates.
        for operator in ToneMapper::ALL {
            assert!(
                tone_map(operator, Vec3::ZERO).max_element() < 0.01,
                "{operator:?}"
            );
            assert!(
                tone_map(operator, Vec3::splat(1e4)).min_element() > 0.95,
                "{operator:?}"
            );
        }
        // PBR Neutral leaves dark-to-mid colors almost unchanged (its purpose).
        let mid = Vec3::new(0.5, 0.3, 0.2);
        assert!((tone_map(ToneMapper::PbrNeutral, mid) - (mid - 0.04)).length() < 1e-5);
    }

    #[test]
    fn srgb_encoding_known_values() {
        assert!((srgb_encode(Vec3::splat(0.5)).x - 0.735_357).abs() < 1e-5);
        assert_eq!(srgb_encode(Vec3::ZERO), Vec3::ZERO);
        assert!((srgb_encode(Vec3::ONE).x - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cube_faces_point_along_their_axis() {
        let axes = [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ];
        for (face, axis) in axes.iter().enumerate() {
            assert!((cube_direction(face as u32, 0.0, 0.0) - *axis).length() < 1e-6);
        }
        // +Z face: s goes towards +X, t (down) towards -Y.
        assert!(cube_direction(4, 1.0, 0.0).x > 0.0);
        assert!(cube_direction(4, 0.0, 1.0).y < 0.0);
    }

    #[test]
    fn cube_solid_angles_cover_the_sphere() {
        let size = 16;
        let total: f32 = (0..size * size)
            .map(|i| cube_texel_solid_angle(i % size, i / size, size))
            .sum::<f32>()
            * 6.0;
        assert!((total - 4.0 * PI).abs() < 1e-4, "{total}");
    }

    #[test]
    fn ggx_distribution_is_normalized() {
        // ∫ D(h)(n·h) dω = 1 over the hemisphere.
        for alpha in [0.1, 0.5, 1.0] {
            let steps = 4000;
            let integral: f32 = (0..steps)
                .map(|i| {
                    let theta = (i as f32 + 0.5) / steps as f32 * PI / 2.0;
                    let c = theta.cos();
                    ggx_distribution(c, alpha) * c * theta.sin() * TAU * (PI / 2.0 / steps as f32)
                })
                .sum();
            assert!((integral - 1.0).abs() < 1e-2, "alpha {alpha}: {integral}");
        }
    }

    #[test]
    fn brdf_lut_known_values() {
        // At grazing-free normal view, smooth surfaces reflect almost all energy: A + B ≈ 1.
        let smooth = brdf_lut(1.0, 0.05, 1024);
        assert!((smooth.x + smooth.y - 1.0).abs() < 0.02, "{smooth}");
        // Rough surfaces lose energy (single scattering): A + B < 1.
        let rough = brdf_lut(0.5, 1.0, 1024);
        assert!(
            rough.x + rough.y < 0.9 && rough.x > 0.0 && rough.y > 0.0,
            "{rough}"
        );
    }

    #[test]
    fn prefiltering_a_uniform_cube_keeps_it_uniform() {
        let cube = CubeImage::from_fn(8, |_| Vec3::splat(0.7));
        for roughness in [0.2, 0.6, 1.0] {
            let value =
                prefilter_brute_force(&cube, Vec3::new(0.3, -0.4, 0.8).normalize(), roughness);
            assert!((value - Vec3::splat(0.7)).length() < 1e-4);
        }
    }

    #[test]
    fn downsampling_averages_texels() {
        let cube = CubeImage::from_fn(4, |d| d.abs());
        let half = downsample_cube(&cube);
        assert_eq!(half.size, 2);
        let expected =
            (cube.texel(0, 0, 0) + cube.texel(0, 1, 0) + cube.texel(0, 0, 1) + cube.texel(0, 1, 1))
                * 0.25;
        assert!((half.texel(0, 0, 0) - expected).length() < 1e-6);
    }
}
