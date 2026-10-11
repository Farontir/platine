//! Baked maps of a texture set (M11).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::project::Resolution;

/// A map baked from the high-poly mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum BakedMapKind {
    /// Tangent-space normal.
    Normal,
    /// Object-space normal.
    WorldNormal,
    /// Ambient occlusion.
    AmbientOcclusion,
    /// Curvature, 0.5 = flat.
    Curvature,
    /// Object position normalized in the bounding box.
    Position,
    /// Thickness.
    Thickness,
    /// ID colors.
    Id,
    /// Height, low to high distance.
    Height,
}

/// Baked maps available for a texture set; pixels are stored in the project's `baked/`
/// folder, as they can be baked again.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BakedMaps {
    /// Resolution of each baked map.
    pub maps: BTreeMap<BakedMapKind, Resolution>,
}
