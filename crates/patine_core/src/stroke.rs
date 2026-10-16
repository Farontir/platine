//! Recorded brush strokes.
//!
//! A paint layer stores its strokes, not pixels: replaying them at any resolution
//! rebuilds the layer (see the resolution independence section of `docs/PLAN.md`).
//! Brush and pressure details are completed in M9, through format migrations.
//!
//! Strokes are shared (`Arc`): adding one to a log copies pointers, not dabs, and the
//! pointer identifies a stroke for the GPU caches (tiles saved for undo). An `Arc<T>` is
//! serialized exactly as `T`, so the file format does not see it.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::channel::Channel;
use crate::layer::{FillSource, Projection, ProjectionKind};
use crate::resource::ResourceHash;

/// Every stroke of a paint layer or a paint mask effect, oldest first.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StrokeLog {
    /// Strokes in painting order.
    pub strokes: Vec<Arc<Stroke>>,
}

impl StrokeLog {
    /// Approximate heap memory, for history budgets.
    pub fn heap_size(&self) -> usize {
        self.strokes.iter().map(|stroke| stroke.heap_size()).sum()
    }

    /// SHA-256 of the log's postcard bytes: equal logs have equal fingerprints, and any
    /// change to a stroke (a dab, the brush, the camera...) changes it. Raster caches
    /// use it to recognise the strokes they were made from.
    pub fn fingerprint(&self) -> ResourceHash {
        let bytes = postcard::to_allocvec(self).expect("a stroke log always serializes");
        ResourceHash::of(&bytes)
    }
}

impl PartialEq for StrokeLog {
    /// Equal strokes, compared by pointer first: logs sharing their strokes compare
    /// without reading thousands of dabs.
    fn eq(&self, other: &Self) -> bool {
        self.strokes.len() == other.strokes.len()
            && self
                .strokes
                .iter()
                .zip(&other.strokes)
                .all(|(a, b)| Arc::ptr_eq(a, b) || a == b)
    }
}

/// Painting tool of a stroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tool {
    /// Adds material.
    Brush,
    /// Reduces the layer's coverage, channel by channel.
    Eraser,
    /// Reveals an image or material fixed in screen space.
    Projection,
    /// Copies the composition from an offset.
    Clone,
    /// Drags the composition along the stroke.
    Smudge,
}

/// Unit of the brush size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SizeSpace {
    /// Pixels of the viewport.
    Screen,
    /// Object-space units.
    Object,
}

/// A response curve: control points `[input, output]` sorted by input, linear between
/// them, constant beyond the first and the last.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Curve(pub Vec<[f32; 2]>);

impl Curve {
    /// Output equals input.
    pub fn identity() -> Self {
        Self(vec![[0.0, 0.0], [1.0, 1.0]])
    }

    /// Output `value` whatever the input.
    pub fn constant(value: f32) -> Self {
        Self(vec![[0.0, value], [1.0, value]])
    }

    /// The output at `x` (1 for a curve without points).
    pub fn evaluate(&self, x: f32) -> f32 {
        let points = &self.0;
        let (Some(first), Some(last)) = (points.first(), points.last()) else {
            return 1.0;
        };
        if x <= first[0] {
            return first[1];
        }
        if x >= last[0] {
            return last[1];
        }
        for pair in points.windows(2) {
            let ([x0, y0], [x1, y1]) = (pair[0], pair[1]);
            if x <= x1 {
                let t = if x1 > x0 { (x - x0) / (x1 - x0) } else { 1.0 };
                return y0 + (y1 - y0) * t;
            }
        }
        last[1]
    }
}

/// What the pen pressure changes: factors of the size, the flow and the opacity of each
/// dab, by pressure (0 to 1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PressureCurves {
    /// Factor of the size.
    pub size: Curve,
    /// Factor of the flow.
    pub flow: Curve,
    /// Factor of the opacity.
    pub opacity: Curve,
}

impl Default for PressureCurves {
    /// The pressure sets the size; flow and opacity stay full.
    fn default() -> Self {
        Self {
            size: Curve::identity(),
            flow: Curve::constant(1.0),
            opacity: Curve::constant(1.0),
        }
    }
}

impl PressureCurves {
    /// The pressure changes nothing.
    pub fn none() -> Self {
        Self {
            size: Curve::constant(1.0),
            flow: Curve::constant(1.0),
            opacity: Curve::constant(1.0),
        }
    }
}

/// Where a stroke was painted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StrokeSpace {
    /// In the 3D view: dabs are viewport pixels of the stroke's camera.
    #[default]
    View,
    /// In the 2D view: the stroke's camera maps UVs `(u, v, 0, 1)` to the view's pixels.
    Uv,
}

/// Brush settings at the time of the stroke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushSnapshot {
    /// Diameter, in `size_space` units.
    pub size: f32,
    /// Unit of `size`.
    pub size_space: SizeSpace,
    /// Paint deposited per dab, 0 to 1.
    pub flow: f32,
    /// Maximum opacity of the stroke, 0 to 1.
    pub opacity: f32,
    /// Edge hardness, 0 to 1.
    pub hardness: f32,
    /// Distance between dabs, as a fraction of the size.
    pub spacing: f32,
    /// Dab rotation, radians.
    pub angle: f32,
    /// Random size variation, 0 to 1.
    pub size_jitter: f32,
    /// Random rotation, radians.
    pub angle_jitter: f32,
    /// Random flow variation, 0 to 1.
    pub flow_jitter: f32,
    /// Random offset perpendicular to the stroke, as a fraction of the size.
    pub scatter: f32,
    /// Rotate dabs along the stroke direction.
    pub follow_path: bool,
    /// Grayscale alpha image; `None` for a round dab.
    pub alpha: Option<ResourceHash>,
    /// Painted material: one value per channel, or a procedural material.
    pub material: FillSource,
    /// What the pen pressure changes.
    pub pressure: PressureCurves,
    /// The pen tilt sets the dab angle (added to `angle`).
    pub tilt_angle: bool,
    /// Projection of a procedural material.
    pub projection: Projection,
}

impl BrushSnapshot {
    /// Projection of a procedural material by default: triplanar, which needs no UVs and
    /// shows no seams.
    pub fn default_projection() -> Projection {
        Projection {
            kind: ProjectionKind::Triplanar { blend: 0.5 },
            ..Projection::default()
        }
    }
}

/// Camera of the viewport when the stroke was painted.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokeCamera {
    /// View-projection matrix, column major.
    pub view_proj: [[f32; 4]; 4],
    /// Viewport size in pixels.
    pub viewport: [u32; 2],
}

/// Symmetry of a stroke, in object space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symmetry {
    /// Mirror across the X, Y and Z planes.
    pub mirror: [bool; 3],
    /// Copies around the Y axis; 0 or 1 for none.
    pub radial: u32,
}

/// One dab, in screen space.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Dab {
    /// Center in viewport pixels.
    pub position: [f32; 2],
    /// Pen pressure, 0 to 1 (1 for a mouse).
    pub pressure: f32,
    /// Pen tilt, X and Y, radians.
    pub tilt: [f32; 2],
    /// Seconds since the start of the stroke.
    pub time: f32,
}

/// One stroke: everything needed to replay it exactly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    /// Brush at the time of the stroke.
    pub brush: BrushSnapshot,
    /// Tool used.
    pub tool: Tool,
    /// Camera used.
    pub camera: StrokeCamera,
    /// Where the stroke was painted (what its camera projects).
    pub space: StrokeSpace,
    /// Symmetry used.
    pub symmetry: Symmetry,
    /// Seed of the jitters, so that replay is deterministic.
    pub seed: u64,
    /// Dabs in order.
    pub dabs: Vec<Dab>,
}

impl Stroke {
    /// Approximate heap memory, for history budgets.
    pub fn heap_size(&self) -> usize {
        std::mem::size_of::<Stroke>() + self.dabs.len() * std::mem::size_of::<Dab>()
    }

    /// Channels the stroke paints or erases: those of its uniform material, in channel
    /// order. Other materials paint nothing until their milestone.
    pub fn channels(&self) -> Vec<Channel> {
        match &self.brush.material {
            FillSource::Uniform(values) => values.keys().copied().collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_interpolate_and_clamp() {
        let curve = Curve(vec![[0.2, 0.0], [0.6, 1.0], [1.0, 0.5]]);
        assert_eq!(curve.evaluate(0.0), 0.0);
        assert!((curve.evaluate(0.4) - 0.5).abs() < 1e-6);
        assert!((curve.evaluate(0.8) - 0.75).abs() < 1e-6);
        assert_eq!(curve.evaluate(2.0), 0.5);
        assert_eq!(Curve::identity().evaluate(0.3), 0.3);
        assert_eq!(Curve::constant(0.7).evaluate(0.1), 0.7);
        assert_eq!(Curve(Vec::new()).evaluate(0.5), 1.0);
        // A vertical step takes the upper point.
        assert_eq!(Curve(vec![[0.5, 0.0], [0.5, 1.0]]).evaluate(0.5), 0.0);
        let none = PressureCurves::none();
        for pressure in [0.0, 0.5, 1.0] {
            assert_eq!(none.size.evaluate(pressure), 1.0);
            assert_eq!(none.flow.evaluate(pressure), 1.0);
            assert_eq!(none.opacity.evaluate(pressure), 1.0);
        }
        assert_eq!(PressureCurves::default().size.evaluate(0.25), 0.25);
    }
}
