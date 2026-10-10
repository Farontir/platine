//! Environment maps on the CPU: equirectangular images, the procedural studio sky,
//! HDR/EXR loading, spherical harmonics and dominant light extraction.
//!
//! Direction convention: Y up. An equirectangular image maps longitude to `u` (0 at -Z,
//! growing towards +X) and colatitude to `v` (0 at +Y, the top row).

use std::f32::consts::{PI, TAU};
use std::path::Path;

use glam::{DVec3, Vec2, Vec3};
use thiserror::Error;

/// Linear RGB radiance in an equirectangular layout, rows from the top (+Y).
#[derive(Clone, Debug, PartialEq)]
pub struct EquirectImage {
    /// Width in texels (longitude).
    pub width: u32,
    /// Height in texels (colatitude).
    pub height: u32,
    /// Row-major radiance values.
    pub texels: Vec<[f32; 3]>,
}

/// Why an environment file could not be loaded.
#[derive(Debug, Error)]
pub enum EnvironmentError {
    /// The file could not be read or decoded.
    #[error("cannot load the environment image: {0}")]
    Image(#[from] image::ImageError),
    /// The image is empty.
    #[error("the environment image is empty")]
    Empty,
}

/// Equirectangular coordinates (u, v in [0, 1]) of a unit direction.
pub fn direction_to_equirect(direction: Vec3) -> Vec2 {
    let u = direction.x.atan2(-direction.z) / TAU + 0.5;
    let v = direction.y.clamp(-1.0, 1.0).acos() / PI;
    Vec2::new(u, v)
}

/// Unit direction of equirectangular coordinates.
pub fn equirect_to_direction(uv: Vec2) -> Vec3 {
    let phi = (uv.x - 0.5) * TAU;
    let theta = uv.y * PI;
    Vec3::new(
        theta.sin() * phi.sin(),
        theta.cos(),
        -theta.sin() * phi.cos(),
    )
}

impl EquirectImage {
    /// Builds an image by evaluating `radiance` at the center of every texel.
    pub fn from_fn(width: u32, height: u32, radiance: impl Fn(Vec3) -> [f32; 3]) -> Self {
        let texels = (0..height)
            .flat_map(|y| (0..width).map(move |x| (x, y)))
            .map(|(x, y)| {
                let uv = Vec2::new(
                    (x as f32 + 0.5) / width as f32,
                    (y as f32 + 0.5) / height as f32,
                );
                radiance(equirect_to_direction(uv))
            })
            .collect();
        Self {
            width,
            height,
            texels,
        }
    }

    /// Uniform radiance `value` (the "white furnace" when 1).
    pub fn uniform(value: f32) -> Self {
        // Fine enough for an exact projection of the order-2 bands.
        Self::from_fn(64, 32, |_| [value; 3])
    }

    /// Loads an `.hdr` or `.exr` file (any format the `image` crate decodes).
    pub fn load(path: &Path) -> Result<Self, EnvironmentError> {
        let image = image::open(path)?.into_rgb32f();
        let (width, height) = image.dimensions();
        if width == 0 || height == 0 {
            return Err(EnvironmentError::Empty);
        }
        let texels = image.pixels().map(|p| p.0).collect();
        Ok(Self {
            width,
            height,
            texels,
        })
    }

    fn texel(&self, x: i64, y: i64) -> Vec3 {
        // Longitude wraps; colatitude clamps at the poles.
        let x = x.rem_euclid(i64::from(self.width)) as usize;
        let y = y.clamp(0, i64::from(self.height) - 1) as usize;
        Vec3::from(self.texels[y * self.width as usize + x])
    }

    /// Bilinear sample in a direction. Texel centers sit at half-integer coordinates.
    pub fn sample(&self, direction: Vec3) -> Vec3 {
        let uv = direction_to_equirect(direction.normalize());
        let x = uv.x * self.width as f32 - 0.5;
        let y = uv.y * self.height as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let top = self.texel(x0, y0).lerp(self.texel(x0 + 1, y0), fx);
        let bottom = self.texel(x0, y0 + 1).lerp(self.texel(x0 + 1, y0 + 1), fx);
        top.lerp(bottom, fy)
    }

    /// Solid angle of a texel of row `y`.
    fn texel_solid_angle(&self, y: u32) -> f32 {
        let theta0 = y as f32 / self.height as f32 * PI;
        let theta1 = (y + 1) as f32 / self.height as f32 * PI;
        (theta0.cos() - theta1.cos()) * TAU / self.width as f32
    }
}

/// The procedural studio sky: a soft gradient dome, a darker floor, a warm key light high
/// on one side and a cool fill on the other. Deterministic; used by default and in tests.
pub fn studio_sky(width: u32) -> EquirectImage {
    let key = Vec3::new(0.55, 0.65, 0.52).normalize();
    let fill = Vec3::new(-0.7, 0.35, -0.6).normalize();
    EquirectImage::from_fn(width, (width / 2).max(1), |d| {
        let up = d.y.max(0.0);
        let horizon = Vec3::new(0.95, 0.92, 0.88);
        let zenith = Vec3::new(0.55, 0.62, 0.75);
        let floor = Vec3::new(0.16, 0.15, 0.14);
        let mut radiance = if d.y >= 0.0 {
            horizon.lerp(zenith, up.powf(0.6))
        } else {
            floor.lerp(horizon * 0.5, (1.0 + d.y).powi(8))
        };
        // Soft-edged area lights (smoothstep on the angular distance).
        let lobe = |axis: Vec3, cos_inner: f32, cos_outer: f32| {
            let c = d.dot(axis);
            let t = ((c - cos_outer) / (cos_inner - cos_outer)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        // Strong enough to dominate the dome, like the sun of an outdoor HDRI, so the
        // extracted shadow-casting light comes from it.
        radiance += Vec3::new(1.0, 0.92, 0.82) * 120.0 * lobe(key, 0.995, 0.985);
        radiance += Vec3::new(0.75, 0.85, 1.0) * 3.0 * lobe(fill, 0.97, 0.9);
        // Overall level: a mid-gray diffuse surface facing the key light lands around the
        // middle of the display range at 0 EV.
        (radiance * 0.4).to_array()
    })
}

/// Real spherical harmonics basis up to order 2 (9 functions) at a unit direction.
pub fn sh_basis(d: Vec3) -> [f32; 9] {
    [
        0.282_095,
        0.488_603 * d.y,
        0.488_603 * d.z,
        0.488_603 * d.x,
        1.092_548 * d.x * d.y,
        1.092_548 * d.y * d.z,
        0.315_392 * (3.0 * d.z * d.z - 1.0),
        1.092_548 * d.x * d.z,
        0.546_274 * (d.x * d.x - d.y * d.y),
    ]
}

/// Cosine-lobe convolution of each band divided by π: multiplying the radiance
/// coefficients by these gives coefficients of irradiance / π, i.e. the outgoing
/// radiance of a white Lambertian surface.
const BAND_FACTORS: [f32; 9] = [
    1.0,
    2.0 / 3.0,
    2.0 / 3.0,
    2.0 / 3.0,
    0.25,
    0.25,
    0.25,
    0.25,
    0.25,
];

/// Order-2 spherical harmonics of RGB radiance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sh9(pub [Vec3; 9]);

impl Sh9 {
    /// Projects an equirectangular image (exact texel solid angles), rotated by
    /// `rotation` radians around Y.
    pub fn project(image: &EquirectImage, rotation: f32) -> Self {
        // Accumulated in f64: an 8K image sums 32 million terms.
        let mut coefficients = [DVec3::ZERO; 9];
        let (sin, cos) = rotation.sin_cos();
        for y in 0..image.height {
            let solid_angle = f64::from(image.texel_solid_angle(y));
            for x in 0..image.width {
                let uv = Vec2::new(
                    (x as f32 + 0.5) / image.width as f32,
                    (y as f32 + 0.5) / image.height as f32,
                );
                let d = equirect_to_direction(uv);
                // The environment is rotated, so a texel at `d` lights direction R(d).
                let d = Vec3::new(cos * d.x + sin * d.z, d.y, -sin * d.x + cos * d.z);
                let radiance = Vec3::from(image.texels[(y * image.width + x) as usize]).as_dvec3();
                for (c, basis) in coefficients.iter_mut().zip(sh_basis(d)) {
                    *c += radiance * (f64::from(basis) * solid_angle);
                }
            }
        }
        Self(coefficients.map(|c| c.as_vec3()))
    }

    /// Radiance in direction `d`.
    pub fn radiance(&self, d: Vec3) -> Vec3 {
        self.0.iter().zip(sh_basis(d)).map(|(c, b)| *c * b).sum()
    }

    /// Outgoing radiance of a white Lambertian surface of normal `n` (irradiance / π).
    pub fn diffuse(&self, n: Vec3) -> Vec3 {
        self.0
            .iter()
            .zip(sh_basis(n))
            .zip(BAND_FACTORS)
            .map(|((c, b), f)| *c * b * f)
            .sum()
    }

    /// Coefficients pre-multiplied by the cosine convolution, as the shaders use them:
    /// `diffuse(n) = Σ c_i · basis_i(n)`.
    pub fn diffuse_coefficients(&self) -> [Vec3; 9] {
        std::array::from_fn(|i| self.0[i] * BAND_FACTORS[i])
    }
}

/// A directional light extracted from an environment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionalLight {
    /// Unit direction towards the light.
    pub direction: Vec3,
    /// Irradiance on a surface facing the light, divided by π (white Lambertian radiance
    /// at normal incidence).
    pub color: Vec3,
}

/// Extracts the dominant directional light of `sh` (Sloan, "Stupid Spherical Harmonics
/// Tricks") and returns it with the residual radiance, so that
/// `residual + light ≈ sh` and the light can be shadowed separately.
pub fn extract_dominant_light(sh: &Sh9) -> (DirectionalLight, Sh9) {
    let luma = |c: Vec3| c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
    // Band 1 of the luminance points towards the light: (Y11, Y1-1, Y10) ∝ (x, y, z).
    let axis = Vec3::new(luma(sh.0[3]), luma(sh.0[1]), luma(sh.0[2]));
    let Some(direction) = axis.try_normalize() else {
        return (
            DirectionalLight {
                direction: Vec3::Y,
                color: Vec3::ZERO,
            },
            *sh,
        );
    };
    // Least-squares intensity of a delta light: its radiance coefficients are I·Y(d).
    let basis = sh_basis(direction);
    let norm: f32 = basis.iter().map(|b| b * b).sum();
    let intensity: Vec3 = sh.0.iter().zip(basis).map(|(c, b)| *c * b).sum::<Vec3>() / norm;
    let intensity = intensity.max(Vec3::ZERO);
    let residual = Sh9(std::array::from_fn(|i| sh.0[i] - intensity * basis[i]));
    // A delta light of radiance coefficients I·Y(d) gives irradiance I·max(n·d, 0); its
    // white Lambertian radiance is I / π.
    (
        DirectionalLight {
            direction,
            color: intensity / PI,
        },
        residual,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equirect_mapping_round_trips() {
        for uv in [
            Vec2::new(0.1, 0.2),
            Vec2::new(0.5, 0.5),
            Vec2::new(0.9, 0.8),
        ] {
            let back = direction_to_equirect(equirect_to_direction(uv));
            assert!((back - uv).length() < 1e-5, "{uv} -> {back}");
        }
        // u = 0.5 looks towards -Z, the top row is +Y.
        assert!((equirect_to_direction(Vec2::new(0.5, 0.5)) - Vec3::NEG_Z).length() < 1e-6);
        assert!((equirect_to_direction(Vec2::new(0.3, 0.0)) - Vec3::Y).length() < 1e-6);
    }

    #[test]
    fn uniform_environment_projects_to_a_constant() {
        // Texel-center quadrature leaves spurious band-2 energy decreasing as 1/height²
        // (0.2% at 64 × 32, 0.015% at 256 × 128); band 0 is exact (exact solid angles).
        let sh = Sh9::project(&EquirectImage::from_fn(256, 128, |_| [2.0; 3]), 0.0);
        let tolerance = 1e-3 * 2.0 * 3f32.sqrt();
        for d in [Vec3::X, Vec3::NEG_Y, Vec3::new(1.0, 1.0, -1.0).normalize()] {
            let error = (sh.radiance(d) - Vec3::splat(2.0)).length();
            assert!(error < tolerance, "radiance error {error} at {d}");
            // A white Lambertian surface under uniform radiance L reflects L.
            assert!((sh.diffuse(d) - Vec3::splat(2.0)).length() < tolerance);
        }
        assert!((sh.0[0].x - 2.0 * (4.0 * PI).sqrt()).abs() < 1e-4);
    }

    #[test]
    fn rotation_turns_the_environment() {
        // Bright towards -Z only.
        let image = EquirectImage::from_fn(64, 32, |d| [d.z.min(0.0).abs(); 3]);
        let sh = Sh9::project(&image, 0.0);
        assert!(sh.diffuse(Vec3::NEG_Z).x > sh.diffuse(Vec3::X).x);
        // A quarter turn brings it to -X (counter-clockwise seen from above).
        let turned = Sh9::project(&image, std::f32::consts::FRAC_PI_2);
        assert!((turned.diffuse(Vec3::NEG_X) - sh.diffuse(Vec3::NEG_Z)).length() < 1e-2);
    }

    #[test]
    fn extraction_finds_a_small_light_and_preserves_the_sum() {
        let light = Vec3::new(0.3, 0.8, -0.5).normalize();
        let image = EquirectImage::from_fn(256, 128, |d| {
            let spot = if d.dot(light) > 0.995 { 200.0 } else { 0.0 };
            [0.1 + spot; 3]
        });
        let sh = Sh9::project(&image, 0.0);
        let (extracted, residual) = extract_dominant_light(&sh);
        assert!(
            extracted.direction.dot(light) > 0.999,
            "{}",
            extracted.direction
        );
        assert!(extracted.color.x > 0.0);
        // Residual + light reconstructs the original coefficients exactly.
        let basis = sh_basis(extracted.direction);
        for ((residual, original), basis) in residual.0.iter().zip(&sh.0).zip(basis) {
            let rebuilt = *residual + extracted.color * PI * basis;
            assert!((rebuilt - *original).length() < 1e-4);
        }
    }

    #[test]
    fn studio_sky_is_brightest_at_its_key_light() {
        let sky = studio_sky(256);
        let key = Vec3::new(0.55, 0.65, 0.52).normalize();
        assert!(sky.sample(key).x > 30.0);
        assert!(sky.sample(Vec3::NEG_Y).x < 0.3);
        let (light, _) = extract_dominant_light(&Sh9::project(&sky, 0.0));
        assert!(light.direction.dot(key) > 0.95, "{}", light.direction);
    }

    #[test]
    fn hdr_files_load() {
        let dir = std::env::temp_dir().join("patine_env_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tiny.hdr");
        let pixels: Vec<image::Rgb<f32>> = (0..8)
            .map(|i| image::Rgb([i as f32 * 2.0, 0.5, 100.0]))
            .collect();
        let file = std::fs::File::create(&path).unwrap();
        image::codecs::hdr::HdrEncoder::new(file)
            .encode(&pixels, 4, 2)
            .unwrap();
        let loaded = EquirectImage::load(&path).unwrap();
        assert_eq!((loaded.width, loaded.height), (4, 2));
        // Radiance beyond the f16 range of 8-bit formats survives (RGBE: 1% precision).
        assert!((loaded.texels[0][2] - 100.0).abs() < 1.0);
        assert!((loaded.texels[7][0] - 14.0).abs() < 0.2);
    }
}
