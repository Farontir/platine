//! Masks, layer effects and procedural parameters.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::channel::Channel;
use crate::ids::{AnchorId, EffectId};
use crate::layer::{BlendMode, FillSource};
use crate::resource::ResourceHash;
use crate::stroke::StrokeLog;

/// Value of a procedural parameter. The definitions (`ParamDef`) arrive in M7.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ParamValue {
    /// Real number.
    Float(f32),
    /// Integer.
    Int(i32),
    /// Switch.
    Bool(bool),
    /// Linear RGBA color.
    Color([f32; 4]),
    /// Selected option of an enumeration.
    Enum(String),
    /// Curve control points `[x, y]`, sorted by x.
    Curve(Vec<[f32; 2]>),
    /// Gradient stops.
    Gradient(Vec<GradientStop>),
    /// Image resource, or none.
    Texture(Option<ResourceHash>),
}

/// A color stop of a gradient.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    /// Position, 0 to 1.
    pub position: f32,
    /// Linear RGBA color.
    pub color: [f32; 4],
}

/// Parameters by name, in a stable order.
pub type Params = BTreeMap<String, ParamValue>;

/// Mask generators, driven by the baked maps (M13).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GeneratorKind {
    /// Wear uncovering the edges (curvature, noise).
    EdgeWear,
    /// Dirt in the cavities (AO, noise).
    Dirt,
    /// Deposit on upward faces (world normal, noise).
    Dust,
    /// Bottom-to-top gradient (position).
    PositionGradient,
    /// Light painted in the texture (world normal, direction).
    Light,
    /// Thin areas (thickness).
    Thickness,
    /// Vertical drips under edges.
    Leaks,
    /// Fully configurable combination.
    MaskBuilder,
}

/// Filters, usable as layer effects and mask effects (M15).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FilterKind {
    /// Gaussian blur.
    GaussianBlur,
    /// Blur along a direction.
    DirectionalBlur,
    /// Blur along the slope of a map.
    SlopeBlur,
    /// Sharpening.
    Sharpen,
    /// Hue, saturation, lightness.
    Hsl,
    /// Tone curves.
    Curves,
    /// Inversion.
    Invert,
    /// Posterization.
    Posterize,
    /// Distortion by a noise.
    NoiseWarp,
    /// Clamp to a range.
    Clamp,
    /// Height to normal conversion.
    HeightToNormal,
    /// Selection of an ID map color.
    ColorToMask,
}

/// Levels adjustment.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LevelsParams {
    /// Input black point.
    pub in_low: f32,
    /// Input white point.
    pub in_high: f32,
    /// Gamma (1 = linear).
    pub gamma: f32,
    /// Output black point.
    pub out_low: f32,
    /// Output white point.
    pub out_high: f32,
}

impl Default for LevelsParams {
    fn default() -> Self {
        Self {
            in_low: 0.0,
            in_high: 1.0,
            gamma: 1.0,
            out_low: 0.0,
            out_high: 1.0,
        }
    }
}

/// Faces filled by a polygon fill, as triangle indices of the project mesh (sorted).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaceSelection {
    /// Selected triangles.
    pub triangles: Vec<u32>,
}

/// One operation of a mask stack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MaskEffect {
    /// Painted strokes; stored next to the manifest, not inside it.
    Paint {
        /// The strokes.
        #[serde(skip)]
        strokes: Arc<StrokeLog>,
    },
    /// Uniform value, image or procedural.
    Fill {
        /// Content.
        source: FillSource,
    },
    /// Generator reading the baked maps.
    Generator {
        /// Generator.
        kind: GeneratorKind,
        /// Its parameters.
        params: Params,
    },
    /// Filter of the mask below.
    Filter {
        /// Filter.
        kind: FilterKind,
        /// Its parameters.
        params: Params,
    },
    /// Levels of the mask below.
    Levels(LevelsParams),
    /// Channel of an anchor point captured lower in the stack.
    AnchorRef {
        /// The anchor.
        anchor: AnchorId,
        /// Channel read from it.
        channel: Channel,
    },
    /// Faces selected on the mesh.
    PolygonFill {
        /// The selection.
        selection: FaceSelection,
    },
}

/// A mask effect with its blending.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskEffectEntry {
    /// Stable identifier.
    pub id: EffectId,
    /// Whether the effect is evaluated.
    pub visible: bool,
    /// How it combines with the effects below.
    pub blend: BlendMode,
    /// Opacity, 0 to 1.
    pub opacity: f32,
    /// The operation.
    pub effect: MaskEffect,
}

/// Value of a mask before its effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MaskInitial {
    /// Everything hidden.
    Black,
    /// Everything visible.
    White,
}

/// A layer mask: a stack of effects reduced to one grayscale texture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskStack {
    /// Starting value.
    pub initial: MaskInitial,
    /// Effects, bottom to top.
    pub effects: Vec<MaskEffectEntry>,
}

/// An operation applied to a layer's content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Effect {
    /// Filter.
    Filter {
        /// Filter.
        kind: FilterKind,
        /// Its parameters.
        params: Params,
    },
    /// Levels.
    Levels(LevelsParams),
}

/// A layer effect with its visibility.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectEntry {
    /// Stable identifier.
    pub id: EffectId,
    /// Whether the effect is evaluated.
    pub visible: bool,
    /// The operation.
    pub effect: Effect,
}

/// Key under which a uniform mask fill stores its gray value, in the first component.
/// Readers take the first entry whatever its key ([`MaskEffect::fill_value`]).
pub const MASK_VALUE_CHANNEL: Channel = Channel::Opacity;

impl MaskStack {
    /// A mask without effects.
    pub fn new(initial: MaskInitial) -> Self {
        Self {
            initial,
            effects: Vec::new(),
        }
    }

    /// The value the mask starts from: 0 (black) or 1 (white).
    pub fn initial_value(&self) -> f32 {
        match self.initial {
            MaskInitial::Black => 0.0,
            MaskInitial::White => 1.0,
        }
    }

    /// Index of the effect `id`.
    pub fn index_of(&self, id: EffectId) -> Option<usize> {
        self.effects.iter().position(|entry| entry.id == id)
    }
}

impl MaskEffect {
    /// A uniform fill of value `value` (0 to 1).
    pub fn uniform(value: f32) -> Self {
        Self::Fill {
            source: FillSource::Uniform([(MASK_VALUE_CHANNEL, [value; 4])].into()),
        }
    }

    /// The inversion filter (`1 − m`).
    pub fn invert() -> Self {
        Self::Filter {
            kind: FilterKind::Invert,
            params: Params::new(),
        }
    }

    /// The gray value of a uniform fill: the first component of its first entry (0 when
    /// the fill has none). `None` for other effects and other sources.
    pub fn fill_value(&self) -> Option<f32> {
        match self {
            Self::Fill {
                source: FillSource::Uniform(values),
            } => Some(values.values().next().map_or(0.0, |value| value[0])),
            _ => None,
        }
    }

    /// Short identifier of the operation, for history labels: `fill`, `levels`,
    /// `invert` or `other`.
    pub fn kind_id(&self) -> &'static str {
        match self {
            Self::Fill { .. } => "fill",
            Self::Levels(_) => "levels",
            Self::Filter {
                kind: FilterKind::Invert,
                ..
            } => "invert",
            _ => "other",
        }
    }
}

impl MaskEffectEntry {
    /// A visible effect in Normal mode at full opacity.
    pub fn new(id: EffectId, effect: MaskEffect) -> Self {
        Self {
            id,
            visible: true,
            blend: BlendMode::Normal,
            opacity: 1.0,
            effect,
        }
    }
}
