//! Project and texture sets.
//!
//! M1 subset of the document model: M3 adds the layer stacks, commands and
//! serialization. Creating a project replaces the whole document; it is not a command.

use std::path::PathBuf;

use crate::mesh::MeshAsset;

/// Stable identifier of a texture set, never reused within a project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureSetId(pub u32);

/// Square texture resolution: a power of two from 512 to 8192.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Resolution(u32);

impl Resolution {
    /// Every supported resolution, from smallest to largest.
    pub const ALL: [Resolution; 5] = [
        Resolution(512),
        Resolution(1024),
        Resolution(2048),
        Resolution(4096),
        Resolution(8192),
    ];

    /// Default working resolution.
    pub const DEFAULT: Resolution = Resolution(2048);

    /// The resolution `pixels`, if supported.
    pub fn new(pixels: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|resolution| resolution.0 == pixels)
    }

    /// Width and height in pixels.
    pub fn pixels(self) -> u32 {
        self.0
    }
}

impl Default for Resolution {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Orientation of the green channel of exported normal maps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NormalConvention {
    /// Y+ (Blender, Unity, Godot, glTF).
    #[default]
    OpenGl,
    /// Y− (Unreal).
    DirectX,
}

/// Textures painted for one material of the mesh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureSet {
    /// Stable identifier.
    pub id: TextureSetId,
    /// Name shown in the interface, initially the material name.
    pub name: String,
    /// Index into [`MeshAsset::materials`].
    pub material_index: u32,
    /// Working resolution.
    pub resolution: Resolution,
}

/// A texturing project.
#[derive(Clone, Debug, PartialEq)]
pub struct Project {
    /// The textured mesh.
    pub mesh: MeshAsset,
    /// File the mesh was imported from.
    pub mesh_source: PathBuf,
    /// One texture set per material of the mesh.
    pub texture_sets: Vec<TextureSet>,
    /// Normal map convention of the project.
    pub normal_convention: NormalConvention,
}

/// Settings chosen in the New Project dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProjectSettings {
    /// Resolution of every texture set.
    pub resolution: Resolution,
    /// Normal map convention.
    pub normal_convention: NormalConvention,
}

impl Project {
    /// Creates a project with one texture set per material of `mesh`.
    pub fn new(mesh: MeshAsset, mesh_source: PathBuf, settings: ProjectSettings) -> Self {
        let texture_sets = mesh
            .materials
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let index = u32::try_from(index).expect("material count fits in u32");
                TextureSet {
                    id: TextureSetId(index),
                    name: name.clone(),
                    material_index: index,
                    resolution: settings.resolution,
                }
            })
            .collect();
        Self {
            mesh,
            mesh_source,
            texture_sets,
            normal_convention: settings.normal_convention,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolutions_are_powers_of_two() {
        assert!(Resolution::ALL.iter().all(|r| r.pixels().is_power_of_two()));
        assert_eq!(Resolution::new(4096).map(Resolution::pixels), Some(4096));
        assert_eq!(Resolution::new(3000), None);
        assert_eq!(Resolution::new(16384), None);
        assert_eq!(Resolution::default().pixels(), 2048);
    }

    #[test]
    fn one_texture_set_per_material() {
        let mesh = MeshAsset {
            materials: vec!["body".into(), "glass".into()],
            ..MeshAsset::default()
        };
        let settings = ProjectSettings {
            resolution: Resolution::new(1024).unwrap(),
            normal_convention: NormalConvention::DirectX,
        };
        let project = Project::new(mesh, "mesh.fbx".into(), settings);
        assert_eq!(project.texture_sets.len(), 2);
        assert_eq!(project.texture_sets[1].name, "glass");
        assert_eq!(project.texture_sets[1].material_index, 1);
        assert_eq!(project.texture_sets[1].id, TextureSetId(1));
        assert!(
            project
                .texture_sets
                .iter()
                .all(|t| t.resolution.pixels() == 1024)
        );
        assert_eq!(project.normal_convention, NormalConvention::DirectX);
    }
}
