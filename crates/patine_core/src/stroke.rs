//! Recorded brush strokes.
//!
//! A paint layer stores its strokes, not pixels: replaying them at any resolution
//! rebuilds the layer (see the resolution independence section of `docs/PLAN.md`).
//! Brush and pressure details are completed in M8 and M9, through format migrations.

use serde::{Deserialize, Serialize};

use crate::layer::FillSource;
use crate::resource::ResourceHash;

/// Every stroke of a paint layer or a paint mask effect, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StrokeLog {
    /// Strokes in painting order.
    pub strokes: Vec<Stroke>,
}

impl StrokeLog {
    /// Approximate heap memory, for history budgets.
    pub fn heap_size(&self) -> usize {
        self.strokes
            .iter()
            .map(|s| std::mem::size_of::<Stroke>() + s.dabs.len() * std::mem::size_of::<Dab>())
            .sum()
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
    /// Symmetry used.
    pub symmetry: Symmetry,
    /// Seed of the jitters, so that replay is deterministic.
    pub seed: u64,
    /// Dabs in order.
    pub dabs: Vec<Dab>,
}
