//! From pointer positions to dabs: smoothing, then dabs at regular intervals.
//!
//! The pointer drags the brush with a string of length `smoothing` (lazy mouse): the
//! brush only moves when the pointer is farther than that, and then just enough to keep
//! the string taut. Dabs are placed along the path of the brush, one every
//! `spacing × size` pixels, carrying the remainder from one pointer event to the next.
//! The recorded dabs are the ones after smoothing, so replaying a stroke does not depend
//! on the sampler.

use glam::Vec2;
use patine_core::stroke::Dab;

/// Smallest distance between two dabs, in pixels.
pub const MIN_STEP: f32 = 0.5;

/// Settings of a [`StrokeSampler`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SamplerSettings {
    /// Brush diameter, in pixels.
    pub size: f32,
    /// Distance between dabs, as a fraction of the size.
    pub spacing: f32,
    /// Length of the lazy mouse string, in pixels; 0 follows the pointer.
    pub smoothing: f32,
}

impl SamplerSettings {
    /// Distance between dabs, in pixels.
    pub fn step(&self) -> f32 {
        (self.spacing * self.size).max(MIN_STEP)
    }
}

/// Turns the pointer events of one stroke into dabs.
#[derive(Clone, Debug)]
pub struct StrokeSampler {
    settings: SamplerSettings,
    /// Position of the brush, pressure, tilt and time of the last event; `None` before
    /// the first event.
    brush: Option<(Vec2, f32, Vec2, f32)>,
    /// Distance travelled since the last dab.
    travelled: f32,
}

impl StrokeSampler {
    /// A sampler for a new stroke.
    pub fn new(settings: SamplerSettings) -> Self {
        Self {
            settings,
            brush: None,
            travelled: 0.0,
        }
    }

    /// Adds a pointer event at `position` (pixels), with `pressure` (1 for a mouse) at
    /// `time` seconds since the start of the stroke, without tilt. Returns the new dabs,
    /// in order: the first event places a dab under the pointer.
    pub fn push(&mut self, position: [f32; 2], pressure: f32, time: f32) -> Vec<Dab> {
        self.push_pen(position, pressure, [0.0, 0.0], time)
    }

    /// Like [`StrokeSampler::push`], with the pen `tilt` (X and Y, radians),
    /// interpolated along the path like the pressure.
    pub fn push_pen(
        &mut self,
        position: [f32; 2],
        pressure: f32,
        tilt: [f32; 2],
        time: f32,
    ) -> Vec<Dab> {
        let pointer = Vec2::from(position);
        let tilt = Vec2::from(tilt);
        let dab = |p: Vec2, pressure: f32, tilt: Vec2, time: f32| Dab {
            position: p.to_array(),
            pressure,
            tilt: tilt.to_array(),
            time,
        };
        let Some((from, from_pressure, from_tilt, from_time)) = self.brush else {
            self.brush = Some((pointer, pressure, tilt, time));
            return vec![dab(pointer, pressure, tilt, time)];
        };
        // The brush follows the pointer at the end of the string.
        let offset = pointer - from;
        let distance = offset.length();
        if distance <= self.settings.smoothing {
            return Vec::new();
        }
        let to = from + offset * ((distance - self.settings.smoothing) / distance);
        self.brush = Some((to, pressure, tilt, time));

        let step = self.settings.step();
        let length = (to - from).length();
        let mut dabs = Vec::new();
        // Distance along this segment of the next dab.
        let mut along = step - self.travelled;
        while along <= length {
            let t = along / length;
            dabs.push(dab(
                from.lerp(to, t),
                from_pressure + (pressure - from_pressure) * t,
                from_tilt.lerp(tilt, t),
                from_time + (time - from_time) * t,
            ));
            along += step;
        }
        self.travelled = length - (along - step);
        dabs
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn settings(size: f32, spacing: f32, smoothing: f32) -> SamplerSettings {
        SamplerSettings {
            size,
            spacing,
            smoothing,
        }
    }

    fn positions(dabs: &[Dab]) -> Vec<Vec2> {
        dabs.iter().map(|dab| Vec2::from(dab.position)).collect()
    }

    #[test]
    fn dabs_follow_a_line_at_the_spacing() {
        let mut sampler = StrokeSampler::new(settings(40.0, 0.25, 0.0));
        let mut dabs = sampler.push([0.0, 0.0], 1.0, 0.0);
        dabs.extend(sampler.push([25.0, 0.0], 0.5, 1.0));
        dabs.extend(sampler.push([100.0, 0.0], 1.0, 2.0));
        let x: Vec<f32> = positions(&dabs).iter().map(|p| p.x).collect();
        assert_eq!(x, (0..=10).map(|i| i as f32 * 10.0).collect::<Vec<_>>());
        // Pressure and time are interpolated along each segment.
        assert!((dabs[1].pressure - 0.8).abs() < 1e-6);
        assert!((dabs[1].time - 0.4).abs() < 1e-6);
        assert!(positions(&dabs).iter().all(|p| p.y == 0.0));
    }

    #[test]
    fn tilt_is_interpolated_like_the_pressure() {
        let mut sampler = StrokeSampler::new(settings(40.0, 0.25, 0.0));
        let first = sampler.push_pen([0.0, 0.0], 0.2, [0.4, -0.2], 0.0);
        assert_eq!(first[0].tilt, [0.4, -0.2]);
        let dabs = sampler.push_pen([20.0, 0.0], 1.0, [0.0, 0.2], 1.0);
        // Dabs at 10 and 20 pixels: halfway, then the end.
        assert_eq!(dabs.len(), 2);
        assert!((dabs[0].tilt[0] - 0.2).abs() < 1e-6);
        assert!(dabs[0].tilt[1].abs() < 1e-6);
        assert!((dabs[0].pressure - 0.6).abs() < 1e-6);
        assert!((dabs[1].tilt[1] - 0.2).abs() < 1e-6);
    }

    #[test]
    fn the_string_holds_the_brush_back() {
        let mut sampler = StrokeSampler::new(settings(10.0, 0.1, 20.0));
        assert_eq!(sampler.push([0.0, 0.0], 1.0, 0.0).len(), 1);
        // Within the string: the brush does not move.
        assert!(sampler.push([15.0, 0.0], 1.0, 0.1).is_empty());
        // Beyond: it moves to 20 pixels behind the pointer.
        let dabs = sampler.push([50.0, 0.0], 1.0, 0.2);
        let last = positions(&dabs).last().copied().unwrap();
        assert!((last.x - 30.0).abs() < 1e-4, "{last}");
    }

    #[test]
    fn a_tiny_spacing_is_bounded() {
        let mut sampler = StrokeSampler::new(settings(1.0, 0.01, 0.0));
        sampler.push([0.0, 0.0], 1.0, 0.0);
        assert_eq!(sampler.push([10.0, 0.0], 1.0, 0.0).len(), 20);
    }

    proptest! {
        /// Along any path, consecutive dabs are one step apart along the brush path (so
        /// at most one step apart in a straight line), whatever the events.
        #[test]
        fn dabs_are_never_farther_apart_than_the_step(
            points in proptest::collection::vec((-500.0f32..500.0, -500.0f32..500.0), 1..40),
            size in 1.0f32..200.0,
            spacing in 0.01f32..2.0,
            smoothing in 0.0f32..50.0,
        ) {
            let settings = settings(size, spacing, smoothing);
            let mut sampler = StrokeSampler::new(settings);
            let mut dabs = Vec::new();
            for (i, (x, y)) in points.iter().enumerate() {
                dabs.extend(sampler.push([*x, *y], 1.0, i as f32));
            }
            prop_assert!(!dabs.is_empty());
            let step = settings.step();
            for pair in positions(&dabs).windows(2) {
                prop_assert!(pair[0].distance(pair[1]) <= step * (1.0 + 1e-3) + 1e-3);
            }
            // Time never goes backwards.
            for pair in dabs.windows(2) {
                prop_assert!(pair[1].time >= pair[0].time);
            }
        }

        /// The dabs of a straight drag do not depend on how its events were split.
        #[test]
        fn splitting_the_events_changes_nothing(
            cuts in proptest::collection::vec(1.0f32..300.0, 0..10),
            spacing in 0.05f32..1.0,
        ) {
            let settings = settings(30.0, spacing, 0.0);
            let mut whole = StrokeSampler::new(settings);
            let mut expected = whole.push([0.0, 0.0], 1.0, 0.0);
            expected.extend(whole.push([300.0, 0.0], 1.0, 1.0));
            let mut cuts = cuts;
            cuts.sort_by(f32::total_cmp);
            let mut split = StrokeSampler::new(settings);
            let mut got = split.push([0.0, 0.0], 1.0, 0.0);
            for x in cuts.iter().chain([300.0].iter()) {
                got.extend(split.push([*x, 0.0], 1.0, *x / 300.0));
            }
            prop_assert_eq!(got.len(), expected.len());
            for (a, b) in positions(&got).iter().zip(positions(&expected)) {
                prop_assert!(a.distance(b) < 1e-2, "{} {}", a, b);
            }
        }
    }
}
