//! Export presets: which files to write and what each of their channels contains (M17).

use serde::{Deserialize, Serialize};

use crate::bake::BakedMapKind;
use crate::channel::Channel;
use crate::project::NormalConvention;

/// File format of an export output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(missing_docs)] // Image formats, named after their file extension.
pub enum ImageFormat {
    Png,
    Tga,
    Tiff,
    Jpeg,
    Exr,
}

/// Color space of an export output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorSpace {
    /// sRGB transfer function (colors).
    Srgb,
    /// Linear values (data).
    Linear,
}

/// Component read from a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Component {
    /// Red.
    R,
    /// Green.
    G,
    /// Blue.
    B,
    /// Alpha.
    A,
    /// Luminance, or the value of a scalar channel.
    Gray,
}

/// Axis of the final normal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum Axis {
    X,
    Y,
    Z,
}

/// What one component of an output file contains.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum OutputChannel {
    /// Nothing (the component is not written).
    None,
    /// A component of a project channel.
    Channel(Channel, Component),
    /// One minus a component (smoothness from roughness).
    Inverted(Channel, Component),
    /// The final normal (normal channel, height and baked normal combined).
    Normal(Axis),
    /// Height normalized to the 0–1 range.
    NormalizedHeight,
    /// A component of a baked map.
    Baked(BakedMapKind, Component),
    /// A constant value.
    Constant(f32),
}

/// One exported file per texture set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportOutput {
    /// File name pattern (`$mesh`, `$textureSet`, `$channel`).
    pub file: String,
    /// Format.
    pub format: ImageFormat,
    /// Bits per component: 8, 16 or 32 (EXR).
    pub bit_depth: u8,
    /// Color space.
    pub color_space: ColorSpace,
    /// Red component.
    pub r: OutputChannel,
    /// Green component.
    pub g: OutputChannel,
    /// Blue component.
    pub b: OutputChannel,
    /// Alpha component.
    pub a: OutputChannel,
}

/// A named set of outputs, for a target engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportPreset {
    /// Name shown in the interface.
    pub name: String,
    /// Normal map convention of the target.
    pub normal_convention: NormalConvention,
    /// Files written per texture set.
    pub outputs: Vec<ExportOutput>,
}
