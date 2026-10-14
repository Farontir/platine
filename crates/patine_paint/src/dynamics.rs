//! The dabs of a stroke as the shaders read them: pen pressure through the brush's
//! curves, tilt, jitters, scatter and the direction of the path.
//!
//! The dab `i` only depends on the stroke's seed, on `i` and on the dabs `0..=i`: live
//! painting, which knows the dabs batch by batch, and replay, which knows them all, give
//! the same dabs. Random values come from an integer hash of `(seed, i, k)`.

use std::ops::Range;

use glam::Vec2;
use patine_core::stroke::Stroke;

use crate::projection::GpuDab;

/// A hash of `(seed, index, k)` mapped to [0, 1): the random values of a dab.
pub fn random(seed: u64, index: u32, k: u32) -> f32 {
    // PCG output steps over a mix of the inputs.
    let mut v = (seed as u32)
        ^ ((seed >> 32) as u32).wrapping_mul(0x9E37_79B9)
        ^ index.wrapping_mul(0x85EB_CA6B)
        ^ k.wrapping_mul(0xC2B2_AE35);
    for _ in 0..2 {
        v = v.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
        let word = ((v >> ((v >> 28) + 4)) ^ v).wrapping_mul(277_803_737);
        v = (word >> 22) ^ word;
    }
    (v >> 8) as f32 / (1u32 << 24) as f32
}

/// A random value in [-1, 1).
fn signed(seed: u64, index: u32, k: u32) -> f32 {
    random(seed, index, k) * 2.0 - 1.0
}

/// Unit direction of the path at dab `i`: from the previous dab, or along +x for the
/// first one and when the pointer did not move.
fn direction(stroke: &Stroke, i: usize) -> Vec2 {
    if i == 0 {
        return Vec2::X;
    }
    let d = Vec2::from(stroke.dabs[i].position) - Vec2::from(stroke.dabs[i - 1].position);
    if d.length() > 1e-6 {
        d.normalize()
    } else {
        Vec2::X
    }
}

/// The dabs `range` of `stroke` as the shaders read them.
pub fn gpu_dabs(stroke: &Stroke, range: Range<usize>) -> Vec<GpuDab> {
    let brush = &stroke.brush;
    range
        .map(|i| {
            let dab = &stroke.dabs[i];
            let index = i as u32;
            let seed = stroke.seed;
            let pressure = dab.pressure.clamp(0.0, 1.0);

            let size = brush.size
                * brush.pressure.size.evaluate(pressure).max(0.0)
                * (1.0 + brush.size_jitter.clamp(0.0, 1.0) * signed(seed, index, 0)).max(0.0);
            let flow = brush.flow.clamp(0.0, 1.0)
                * brush.pressure.flow.evaluate(pressure).clamp(0.0, 1.0)
                * (1.0 - brush.flow_jitter.clamp(0.0, 1.0) * random(seed, index, 1));
            let opacity = brush.pressure.opacity.evaluate(pressure).clamp(0.0, 1.0);

            let path = direction(stroke, i);
            let mut angle = brush.angle + brush.angle_jitter * signed(seed, index, 2);
            if brush.follow_path {
                angle += path.y.atan2(path.x);
            }
            if brush.tilt_angle && (dab.tilt[0] != 0.0 || dab.tilt[1] != 0.0) {
                angle += dab.tilt[1].atan2(dab.tilt[0]);
            }
            // Scatter moves the dab across the path.
            let across = Vec2::new(-path.y, path.x);
            let center = Vec2::from(dab.position)
                + across * (brush.scatter * brush.size * signed(seed, index, 3));

            GpuDab {
                center: center.to_array(),
                radius: (size * 0.5).max(0.5),
                flow,
                rotation: [angle.cos(), angle.sin()],
                hardness: brush.hardness.clamp(0.0, 1.0),
                opacity,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use patine_core::layer::FillSource;
    use patine_core::stroke::{
        BrushSnapshot, Curve, Dab, PressureCurves, SizeSpace, StrokeCamera, StrokeSpace, Symmetry,
        Tool,
    };
    use proptest::prelude::*;

    use super::*;

    fn stroke(seed: u64, jitter: f32, points: usize) -> Stroke {
        Stroke {
            brush: BrushSnapshot {
                size: 40.0,
                size_space: SizeSpace::Screen,
                flow: 0.8,
                opacity: 1.0,
                hardness: 0.5,
                spacing: 0.1,
                angle: 0.3,
                size_jitter: jitter,
                angle_jitter: jitter,
                flow_jitter: jitter,
                scatter: jitter,
                follow_path: true,
                alpha: None,
                material: FillSource::Uniform(Default::default()),
                pressure: PressureCurves {
                    size: Curve::identity(),
                    flow: Curve(vec![[0.0, 0.5], [1.0, 1.0]]),
                    opacity: Curve(vec![[0.0, 0.2], [1.0, 1.0]]),
                },
                tilt_angle: true,
                projection: BrushSnapshot::default_projection(),
            },
            tool: Tool::Brush,
            camera: StrokeCamera {
                view_proj: [[0.0; 4]; 4],
                viewport: [100, 100],
            },
            space: StrokeSpace::View,
            symmetry: Symmetry::default(),
            seed,
            dabs: (0..points)
                .map(|i| Dab {
                    position: [i as f32 * 4.0, (i as f32 * 0.3).sin() * 20.0],
                    pressure: (i % 7) as f32 / 6.0,
                    tilt: [0.2, -0.1 * (i % 3) as f32],
                    time: i as f32,
                })
                .collect(),
        }
    }

    #[test]
    fn pressure_curves_shape_the_dabs() {
        let mut s = stroke(1, 0.0, 8);
        s.brush.follow_path = false;
        s.brush.tilt_angle = false;
        let dabs = gpu_dabs(&s, 0..8);
        // Pressure 0 (dab 0): size curve gives 0, the radius stays half a pixel.
        assert_eq!(dabs[0].radius, 0.5);
        // Pressure 1 (dab 6): full size, flow and opacity.
        assert!((dabs[6].radius - 20.0).abs() < 1e-5);
        assert!((dabs[6].flow - 0.8).abs() < 1e-6);
        assert!((dabs[6].opacity - 1.0).abs() < 1e-6);
        // Pressure 0.5 (dab 3): half size, flow factor 0.75, opacity 0.6.
        assert!((dabs[3].radius - 10.0).abs() < 1e-4);
        assert!((dabs[3].flow - 0.6).abs() < 1e-5);
        assert!((dabs[3].opacity - 0.6).abs() < 1e-5);
        // Without jitter, the brush angle.
        assert!((dabs[3].rotation[0] - 0.3f32.cos()).abs() < 1e-6);
        assert_eq!(dabs[3].center, s.dabs[3].position);
    }

    #[test]
    fn seeds_change_the_jitters_only() {
        let a = gpu_dabs(&stroke(1, 0.8, 30), 0..30);
        let b = gpu_dabs(&stroke(2, 0.8, 30), 0..30);
        assert_ne!(a, b);
        assert_eq!(
            a,
            gpu_dabs(&stroke(1, 0.8, 30), 0..30),
            "same seed, same dabs"
        );
        // Without jitter, the seed does not matter.
        assert_eq!(
            gpu_dabs(&stroke(1, 0.0, 30), 0..30),
            gpu_dabs(&stroke(2, 0.0, 30), 0..30)
        );
        let values: Vec<f32> = (0..1000).map(|i| random(7, i, 0)).collect();
        assert!(values.iter().all(|v| (0.0..1.0).contains(v)));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!((mean - 0.5).abs() < 0.05, "{mean}");
    }

    proptest! {
        /// Dabs computed batch by batch, as live painting does, equal the dabs computed
        /// at once, as replay does.
        #[test]
        fn batches_change_nothing(
            seed in any::<u64>(),
            cuts in proptest::collection::vec(1usize..60, 0..8),
        ) {
            let s = stroke(seed, 0.7, 60);
            let whole = gpu_dabs(&s, 0..60);
            let mut cuts = cuts;
            cuts.sort_unstable();
            cuts.dedup();
            let mut batched = Vec::new();
            let mut start = 0;
            for end in cuts.into_iter().chain([60]) {
                // Live painting only knows the dabs so far.
                let mut partial = s.clone();
                partial.dabs.truncate(end);
                batched.extend(gpu_dabs(&partial, start..end));
                start = end;
            }
            prop_assert_eq!(batched, whole);
        }
    }
}
