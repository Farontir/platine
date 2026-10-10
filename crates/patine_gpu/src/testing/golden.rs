//! Golden-image comparison.
//!
//! References live in `tests/golden/<crate>/<name>.png` (RGBA, 8 bits per channel).
//! On failure, the rendered image and an amplified difference are written to
//! `target/golden-failures/<crate>/`. `PATINE_UPDATE_GOLDEN=1` rewrites the references.
//! A missing reference is a failure: references are only created on purpose.

use std::path::PathBuf;

use image::{Rgba, RgbaImage};

use super::{adapter_choice, workspace_root};
use crate::AdapterChoice;

/// Environment variable that rewrites the references instead of comparing.
pub const UPDATE_ENV: &str = "PATINE_UPDATE_GOLDEN";

/// How far an image may stray from its reference.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    /// Largest accepted difference on any channel of a pixel (0 to 255).
    pub max_channel_delta: u8,
    /// Largest accepted fraction of pixels exceeding `max_channel_delta` (0 to 1).
    pub max_outlier_ratio: f64,
}

impl Tolerance {
    /// Bit-exact comparison, for CPU-generated images.
    pub const EXACT: Self = Self {
        max_channel_delta: 0,
        max_outlier_ratio: 0.0,
    };

    /// Tolerance for GPU renders on the current test adapter.
    ///
    /// References are produced on the software adapter; a real GPU gets more slack.
    pub fn gpu_render() -> Self {
        match adapter_choice() {
            AdapterChoice::Software => Self {
                max_channel_delta: 2,
                max_outlier_ratio: 0.001,
            },
            AdapterChoice::Hardware => Self {
                max_channel_delta: 8,
                max_outlier_ratio: 0.01,
            },
        }
    }
}

/// Result of comparing an image with its reference.
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    /// Largest channel difference found.
    pub max_delta: u8,
    /// Number of pixels whose difference exceeds the tolerance.
    pub outliers: u64,
    /// Total number of pixels.
    pub pixels: u64,
}

impl Comparison {
    /// Whether the comparison satisfies `tolerance`.
    pub fn passes(&self, tolerance: Tolerance) -> bool {
        self.pixels > 0
            && (self.outliers as f64 / self.pixels as f64) <= tolerance.max_outlier_ratio
    }
}

/// Compares two images of the same size. Returns `None` if the sizes differ.
pub fn compare(
    actual: &RgbaImage,
    expected: &RgbaImage,
    tolerance: Tolerance,
) -> Option<Comparison> {
    if actual.dimensions() != expected.dimensions() {
        return None;
    }
    let mut max_delta = 0;
    let mut outliers = 0;
    for (a, e) in actual.pixels().zip(expected.pixels()) {
        let delta = pixel_delta(a, e);
        max_delta = max_delta.max(delta);
        if delta > tolerance.max_channel_delta {
            outliers += 1;
        }
    }
    Some(Comparison {
        max_delta,
        outliers,
        pixels: u64::from(actual.width()) * u64::from(actual.height()),
    })
}

fn pixel_delta(a: &Rgba<u8>, b: &Rgba<u8>) -> u8 {
    a.0.iter()
        .zip(b.0.iter())
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

/// Difference image, amplified ×8 so small errors are visible, fully opaque.
pub fn diff_image(actual: &RgbaImage, expected: &RgbaImage) -> RgbaImage {
    RgbaImage::from_fn(actual.width(), actual.height(), |x, y| {
        let a = actual.get_pixel(x, y);
        let e = expected.get_pixel(x, y);
        let channel = |i: usize| a[i].abs_diff(e[i]).saturating_mul(8);
        Rgba([channel(0), channel(1), channel(2), 255])
    })
}

/// Path of the reference `name` of crate `crate_name`.
pub fn reference_path(crate_name: &str, name: &str) -> PathBuf {
    workspace_root()
        .join("tests/golden")
        .join(crate_name)
        .join(format!("{name}.png"))
}

fn failure_dir(crate_name: &str) -> PathBuf {
    workspace_root()
        .join("target/golden-failures")
        .join(crate_name)
}

/// Compares `actual` with the reference `name` of crate `crate_name`, or rewrites the
/// reference when `PATINE_UPDATE_GOLDEN=1`. Prefer the [`assert_golden!`](crate::assert_golden)
/// macro, which fills in the crate name.
///
/// # Panics
/// If the reference is missing, has another size, or differs beyond `tolerance`.
pub fn assert_golden(crate_name: &str, name: &str, actual: &RgbaImage, tolerance: Tolerance) {
    let reference = reference_path(crate_name, name);
    if std::env::var(UPDATE_ENV).is_ok_and(|value| value == "1") {
        let dir = reference
            .parent()
            .expect("reference path has a parent directory");
        std::fs::create_dir_all(dir).expect("golden directory is writable");
        actual.save(&reference).expect("golden image is writable");
        eprintln!("golden image updated: {}", reference.display());
        return;
    }

    let expected = match image::open(&reference) {
        Ok(image) => image.to_rgba8(),
        Err(error) => panic!(
            "missing golden image {} ({error}). Run with {UPDATE_ENV}=1 to create it.",
            reference.display()
        ),
    };

    let comparison = compare(actual, &expected, tolerance);
    if comparison.as_ref().is_some_and(|c| c.passes(tolerance)) {
        return;
    }

    let dir = failure_dir(crate_name);
    std::fs::create_dir_all(&dir).expect("failure directory is writable");
    let actual_path = dir.join(format!("{name}.actual.png"));
    actual
        .save(&actual_path)
        .expect("failure image is writable");
    match comparison {
        None => panic!(
            "golden image {name}: size {:?} differs from the reference {:?}. Actual image: {}",
            actual.dimensions(),
            expected.dimensions(),
            actual_path.display()
        ),
        Some(comparison) => {
            let diff_path = dir.join(format!("{name}.diff.png"));
            diff_image(actual, &expected)
                .save(&diff_path)
                .expect("failure image is writable");
            panic!(
                "golden image {name} differs: {} of {} pixels beyond ±{} (max delta {}), \
                 allowed ratio {}. Actual: {} Diff: {}",
                comparison.outliers,
                comparison.pixels,
                tolerance.max_channel_delta,
                comparison.max_delta,
                tolerance.max_outlier_ratio,
                actual_path.display(),
                diff_path.display()
            );
        }
    }
}

/// Compares an image with the golden reference `name` of the calling crate.
///
/// ```ignore
/// assert_golden!("fill_solid", &image, Tolerance::gpu_render());
/// ```
#[macro_export]
macro_rules! assert_golden {
    ($name:expr, $image:expr, $tolerance:expr) => {
        $crate::testing::golden::assert_golden(env!("CARGO_PKG_NAME"), $name, $image, $tolerance)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient() -> RgbaImage {
        RgbaImage::from_fn(16, 8, |x, y| {
            Rgba([(x * 16) as u8, (y * 32) as u8, 128, 255])
        })
    }

    #[test]
    fn identical_images_pass_exactly() {
        let image = gradient();
        let comparison = compare(&image, &image, Tolerance::EXACT).unwrap();
        assert_eq!(comparison.max_delta, 0);
        assert!(comparison.passes(Tolerance::EXACT));
    }

    #[test]
    fn small_difference_within_tolerance() {
        let expected = gradient();
        let mut actual = expected.clone();
        actual.get_pixel_mut(3, 3).0[0] += 2;
        let tolerance = Tolerance {
            max_channel_delta: 2,
            max_outlier_ratio: 0.0,
        };
        assert!(
            compare(&actual, &expected, tolerance)
                .unwrap()
                .passes(tolerance)
        );
        assert!(
            !compare(&actual, &expected, Tolerance::EXACT)
                .unwrap()
                .passes(Tolerance::EXACT)
        );
    }

    #[test]
    fn outlier_ratio_is_enforced() {
        let expected = gradient();
        let mut actual = expected.clone();
        actual.get_pixel_mut(0, 0).0[1] = 255;
        // One pixel out of 128 is beyond the delta.
        let loose = Tolerance {
            max_channel_delta: 0,
            max_outlier_ratio: 1.0 / 128.0,
        };
        let strict = Tolerance {
            max_channel_delta: 0,
            max_outlier_ratio: 1.0 / 129.0,
        };
        let comparison = compare(&actual, &expected, loose).unwrap();
        assert_eq!(comparison.outliers, 1);
        assert!(comparison.passes(loose));
        assert!(!comparison.passes(strict));
    }

    #[test]
    fn different_sizes_do_not_compare() {
        let other = RgbaImage::new(8, 8);
        assert!(compare(&gradient(), &other, Tolerance::EXACT).is_none());
    }

    #[test]
    fn cpu_gradient_matches_its_reference() {
        assert_golden(
            env!("CARGO_PKG_NAME"),
            "cpu_gradient",
            &gradient(),
            Tolerance::EXACT,
        );
    }
}
