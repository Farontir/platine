//! Viewport display settings.
//!
//! These are view state, except the environment (HDRI, rotation, exposure), which belongs
//! to the document and moves into the project with the commands of M3.

/// Display operator applied after exposure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToneMapper {
    /// AgX, base look.
    #[default]
    AgX,
    /// ACES, Hill's fit of the RRT and ODT.
    Aces,
    /// Khronos PBR Neutral.
    PbrNeutral,
    /// Clamp only.
    Linear,
}

impl ToneMapper {
    /// Every operator, in display order.
    pub const ALL: [ToneMapper; 4] = [
        ToneMapper::AgX,
        ToneMapper::Aces,
        ToneMapper::PbrNeutral,
        ToneMapper::Linear,
    ];

    /// Index passed to the shaders.
    pub fn index(self) -> u32 {
        match self {
            ToneMapper::AgX => 0,
            ToneMapper::Aces => 1,
            ToneMapper::PbrNeutral => 2,
            ToneMapper::Linear => 3,
        }
    }
}

/// Background behind the mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Background {
    /// The environment, blurred by `blur` (0 sharp, 1 fully blurred).
    Environment {
        /// Blur amount in [0, 1].
        blur: f32,
    },
    /// A solid linear color.
    Solid([f32; 3]),
}

/// Constant material shown until texture sets have layers (M4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConstantMaterial {
    /// Linear base color.
    pub base_color: [f32; 3],
    /// Metallic, 0 to 1.
    pub metallic: f32,
    /// Perceptual roughness, 0 to 1.
    pub roughness: f32,
}

impl Default for ConstantMaterial {
    fn default() -> Self {
        Self {
            base_color: [0.6, 0.6, 0.6],
            metallic: 0.0,
            roughness: 0.5,
        }
    }
}

/// What the mesh shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DisplayMode {
    /// The lit material.
    #[default]
    Lit,
    /// The inspected channel of each material, unlit (see
    /// [`crate::material::MaterialTextures::inspect`]).
    Channel,
    /// A checker on the UVs.
    UvChecker,
    /// The mask of the selected layer, in grayscale: the inspected texture of each
    /// material is that mask ([`InspectKind::Scalar`](crate::material::InspectKind)),
    /// shown like an inspected channel.
    Mask,
}

impl DisplayMode {
    /// Index passed to the shaders.
    pub fn index(self) -> u32 {
        match self {
            DisplayMode::Lit => 0,
            DisplayMode::Channel => 1,
            DisplayMode::UvChecker => 2,
            // The shaders show the inspected texture, here the mask.
            DisplayMode::Mask => 1,
        }
    }
}

/// Everything the viewport needs besides the camera and the mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewSettings {
    /// Exposure in EV (0 = unchanged).
    pub exposure_ev: f32,
    /// Display operator.
    pub tone_mapper: ToneMapper,
    /// Rotation of the environment around the vertical axis, radians.
    pub environment_rotation: f32,
    /// Background.
    pub background: Background,
    /// Shadows from the dominant light of the environment.
    pub shadows: bool,
    /// Polygon edges over the shaded mesh.
    pub wireframe: bool,
    /// Material of the mesh without document materials.
    pub material: ConstantMaterial,
    /// What the mesh shows.
    pub display: DisplayMode,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            exposure_ev: 0.0,
            tone_mapper: ToneMapper::AgX,
            environment_rotation: 0.0,
            background: Background::Environment { blur: 0.6 },
            shadows: true,
            wireframe: false,
            material: ConstantMaterial::default(),
            display: DisplayMode::Lit,
        }
    }
}
