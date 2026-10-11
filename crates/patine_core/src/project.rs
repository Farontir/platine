//! Projects and texture sets.
//!
//! Creating, opening or recovering a project replaces the whole document; it is not a
//! command. Every later change goes through [`crate::command::Command`].

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::bake::BakedMaps;
use crate::channel::{ChannelDef, default_channels};
use crate::environment::{EnvironmentSettings, EnvironmentSource};
use crate::export::ExportPreset;
use crate::layer::{FillSource, Layer, LayerKind, LayerStack};
use crate::mask::{Effect, MaskEffect, ParamValue, Params};
use crate::mesh::{MeshAsset, MeshValidationError};
use crate::resource::{Resource, ResourceHash};

/// Stable identifier of a texture set, never reused within a project.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TextureSetId(pub u32);

/// Square texture resolution: a power of two from 512 to 8192.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
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

/// A resolution that is not a supported power of two.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("unsupported resolution {0}")]
pub struct UnsupportedResolution(pub u32);

impl TryFrom<u32> for Resolution {
    type Error = UnsupportedResolution;

    fn try_from(pixels: u32) -> Result<Self, Self::Error> {
        Self::new(pixels).ok_or(UnsupportedResolution(pixels))
    }
}

impl From<Resolution> for u32 {
    fn from(resolution: Resolution) -> u32 {
        resolution.0
    }
}

/// Orientation of the green channel of exported normal maps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NormalConvention {
    /// Y+ (Blender, Unity, Godot, glTF).
    #[default]
    OpenGl,
    /// Y− (Unreal).
    DirectX,
}

/// Textures painted for one material of the mesh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextureSet {
    /// Stable identifier.
    pub id: TextureSetId,
    /// Name shown in the interface, initially the material name.
    pub name: String,
    /// Index into [`MeshAsset::materials`].
    pub material_index: u32,
    /// Working resolution.
    pub resolution: Resolution,
    /// Active channels.
    pub channels: Vec<ChannelDef>,
    /// Maps baked from the high-poly mesh.
    pub baked_maps: BakedMaps,
    /// Layers.
    pub stack: LayerStack,
}

/// A texturing project.
///
/// Heavy data is shared (`Arc`): cloning a project is cheap, which lets saving work on a
/// snapshot in the background. The mesh, stroke logs and resource bytes are stored next
/// to the manifest in project files, not inside it (they are skipped by serde).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    /// The textured mesh.
    #[serde(skip)]
    pub mesh: Arc<MeshAsset>,
    /// File the mesh was imported from.
    pub mesh_source: PathBuf,
    /// One texture set per material of the mesh.
    pub texture_sets: Vec<TextureSet>,
    /// Lighting of the viewport.
    pub environment: EnvironmentSettings,
    /// Export presets of the project.
    pub export_presets: Vec<ExportPreset>,
    /// Files imported by the user, by hash.
    pub resources: BTreeMap<ResourceHash, Resource>,
    /// Normal map convention of the project.
    pub normal_convention: NormalConvention,
    /// Next free layer, effect or anchor identifier.
    pub next_id: u64,
}

/// Settings chosen in the New Project dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProjectSettings {
    /// Resolution of every texture set.
    pub resolution: Resolution,
    /// Normal map convention.
    pub normal_convention: NormalConvention,
}

/// A broken invariant of a [`Project`] (corrupted or hand-edited file).
#[derive(Debug, Error, PartialEq)]
pub enum ValidationError {
    /// The mesh is invalid.
    #[error("invalid mesh: {0}")]
    Mesh(#[from] MeshValidationError),
    /// A texture set's identifier is not its index.
    #[error("texture set {index} has identifier {id}")]
    TextureSetId {
        /// Index in the list.
        index: usize,
        /// Identifier found.
        id: u32,
    },
    /// A texture set references a material the mesh does not have.
    #[error("texture set {0} uses a material the mesh does not have")]
    Material(u32),
    /// Two texture sets use the same material.
    #[error("two texture sets use material {0}")]
    DuplicateMaterial(u32),
    /// A channel is listed twice in a texture set.
    #[error("texture set {0} lists a channel twice")]
    DuplicateChannel(u32),
    /// Two nodes share an identifier.
    #[error("identifier {0} is used twice")]
    DuplicateId(u64),
    /// An identifier is not below `next_id`.
    #[error("identifier {id} is not below next_id {next_id}")]
    IdNotAllocated {
        /// Identifier found.
        id: u64,
        /// The project's counter.
        next_id: u64,
    },
    /// An instance layer points to a missing layer or to another instance.
    #[error("instance layer {0} has an invalid source")]
    InstanceSource(u64),
    /// A mask reads an anchor that does not exist.
    #[error("missing anchor {0}")]
    MissingAnchor(u64),
    /// A resource is referenced but not stored.
    #[error("missing resource {0}")]
    MissingResource(ResourceHash),
    /// A resource's bytes do not match its hash.
    #[error("resource {0} is corrupted")]
    CorruptedResource(ResourceHash),
}

impl Project {
    /// Creates a project with one texture set per material of `mesh`, default channels,
    /// no layers and the default environment.
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
                    channels: default_channels(),
                    baked_maps: BakedMaps::default(),
                    stack: LayerStack::default(),
                }
            })
            .collect();
        Self {
            mesh: Arc::new(mesh),
            mesh_source,
            texture_sets,
            environment: EnvironmentSettings::default(),
            export_presets: Vec::new(),
            resources: BTreeMap::new(),
            normal_convention: settings.normal_convention,
            next_id: 1,
        }
    }

    /// The texture set `id`.
    pub fn texture_set(&self, id: TextureSetId) -> Option<&TextureSet> {
        self.texture_sets
            .get(id.0 as usize)
            .filter(|set| set.id == id)
    }

    /// The texture set `id`, mutably.
    pub fn texture_set_mut(&mut self, id: TextureSetId) -> Option<&mut TextureSet> {
        self.texture_sets
            .get_mut(id.0 as usize)
            .filter(|set| set.id == id)
    }

    /// Calls `f` on every layer of every texture set, with its texture set.
    pub fn visit_layers<'a>(&'a self, f: &mut impl FnMut(TextureSetId, &'a Layer)) {
        for set in &self.texture_sets {
            set.stack.visit(&mut |layer| f(set.id, layer));
        }
    }

    /// The texture set containing the layer `id`.
    pub fn layer_texture_set(&self, id: crate::ids::LayerId) -> Option<TextureSetId> {
        self.texture_sets
            .iter()
            .find(|set| set.stack.find(id).is_some())
            .map(|set| set.id)
    }

    /// Calls `f` on every resource hash the document references.
    pub fn visit_resource_refs(&self, f: &mut impl FnMut(ResourceHash)) {
        fn fill(source: &FillSource, f: &mut impl FnMut(ResourceHash)) {
            match source {
                FillSource::Image { resource } => f(*resource),
                FillSource::Procedural(procedural) => params(&procedural.params, f),
                FillSource::Uniform(_) => {}
            }
        }
        fn params(params: &Params, f: &mut impl FnMut(ResourceHash)) {
            for value in params.values() {
                if let ParamValue::Texture(Some(hash)) = value {
                    f(*hash);
                }
            }
        }
        if let EnvironmentSource::Resource(hash) = &self.environment.source {
            f(*hash);
        }
        self.visit_layers(&mut |_, layer| {
            match &layer.kind {
                LayerKind::Fill { source, .. } => fill(source, f),
                LayerKind::Paint { strokes } => {
                    for stroke in &strokes.strokes {
                        if let Some(alpha) = stroke.brush.alpha {
                            f(alpha);
                        }
                        fill(&stroke.brush.material, f);
                    }
                }
                LayerKind::Folder { .. } | LayerKind::Instance { .. } => {}
            }
            for entry in &layer.effects {
                if let Effect::Filter { params: p, .. } = &entry.effect {
                    params(p, f);
                }
            }
            for entry in layer.mask.iter().flat_map(|mask| &mask.effects) {
                match &entry.effect {
                    MaskEffect::Fill { source } => fill(source, f),
                    MaskEffect::Generator { params: p, .. }
                    | MaskEffect::Filter { params: p, .. } => params(p, f),
                    MaskEffect::Paint { strokes } => {
                        for stroke in &strokes.strokes {
                            if let Some(alpha) = stroke.brush.alpha {
                                f(alpha);
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    /// Checks every invariant of the document. Run on files read from disk.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.mesh.validate()?;
        let mut materials = HashSet::new();
        for (index, set) in self.texture_sets.iter().enumerate() {
            if set.id.0 as usize != index {
                return Err(ValidationError::TextureSetId {
                    index,
                    id: set.id.0,
                });
            }
            if set.material_index as usize >= self.mesh.materials.len() {
                return Err(ValidationError::Material(set.id.0));
            }
            if !materials.insert(set.material_index) {
                return Err(ValidationError::DuplicateMaterial(set.material_index));
            }
            let channels: HashSet<_> = set.channels.iter().map(|def| def.channel).collect();
            if channels.len() != set.channels.len() {
                return Err(ValidationError::DuplicateChannel(set.id.0));
            }
        }

        // Every node identifier is unique and allocated.
        let mut ids = HashSet::new();
        let mut layers = HashSet::new();
        let mut anchors = HashSet::new();
        let mut result = Ok(());
        let mut check = |id: u64| {
            if result.is_err() {
                return;
            }
            if id >= self.next_id {
                result = Err(ValidationError::IdNotAllocated {
                    id,
                    next_id: self.next_id,
                });
            } else if !ids.insert(id) {
                result = Err(ValidationError::DuplicateId(id));
            }
        };
        self.visit_layers(&mut |_, layer| {
            check(layer.id.0);
            layers.insert(layer.id);
            if let Some(anchor) = &layer.anchor {
                check(anchor.id.0);
                anchors.insert(anchor.id);
            }
            for entry in &layer.effects {
                check(entry.id.0);
            }
            for entry in layer.mask.iter().flat_map(|mask| &mask.effects) {
                check(entry.id.0);
            }
        });
        result?;

        // References between nodes.
        let mut result = Ok(());
        self.visit_layers(&mut |_, layer| {
            if result.is_err() {
                return;
            }
            if let LayerKind::Instance { source } = &layer.kind {
                let valid = *source != layer.id
                    && self.texture_sets.iter().any(|set| {
                        set.stack
                            .find(*source)
                            .is_some_and(|s| !matches!(s.kind, LayerKind::Instance { .. }))
                    });
                if !valid {
                    result = Err(ValidationError::InstanceSource(layer.id.0));
                }
            }
            for entry in layer.mask.iter().flat_map(|mask| &mask.effects) {
                if let MaskEffect::AnchorRef { anchor, .. } = &entry.effect
                    && !anchors.contains(anchor)
                {
                    result = Err(ValidationError::MissingAnchor(anchor.0));
                }
            }
        });
        result?;

        let mut missing = None;
        self.visit_resource_refs(&mut |hash| {
            if missing.is_none() && !self.resources.contains_key(&hash) {
                missing = Some(hash);
            }
        });
        if let Some(hash) = missing {
            return Err(ValidationError::MissingResource(hash));
        }
        for (hash, resource) in &self.resources {
            if ResourceHash::of(&resource.bytes) != *hash {
                return Err(ValidationError::CorruptedResource(*hash));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::Channel;
    use crate::ids::{AnchorId, LayerId};
    use crate::layer::{Anchor, Projection};
    use crate::mask::{MaskEffectEntry, MaskInitial, MaskStack};
    use crate::resource::ResourceKind;

    pub(crate) fn two_materials() -> MeshAsset {
        MeshAsset {
            materials: vec!["body".into(), "glass".into()],
            ..MeshAsset::default()
        }
    }

    fn fill(id: u64) -> Layer {
        Layer::new(
            LayerId(id),
            format!("fill {id}"),
            LayerKind::Fill {
                source: FillSource::Uniform(Default::default()),
                projection: Projection::default(),
            },
            &[Channel::BaseColor],
        )
    }

    #[test]
    fn resolutions_are_powers_of_two() {
        assert!(Resolution::ALL.iter().all(|r| r.pixels().is_power_of_two()));
        assert_eq!(Resolution::new(4096).map(Resolution::pixels), Some(4096));
        assert_eq!(Resolution::new(3000), None);
        assert_eq!(Resolution::new(16384), None);
        assert_eq!(Resolution::default().pixels(), 2048);
        assert_eq!(Resolution::try_from(1000), Err(UnsupportedResolution(1000)));
    }

    #[test]
    fn one_texture_set_per_material() {
        let settings = ProjectSettings {
            resolution: Resolution::new(1024).unwrap(),
            normal_convention: NormalConvention::DirectX,
        };
        let project = Project::new(two_materials(), "mesh.fbx".into(), settings);
        assert_eq!(project.texture_sets.len(), 2);
        assert_eq!(project.texture_sets[1].name, "glass");
        assert_eq!(project.texture_sets[1].material_index, 1);
        assert_eq!(project.texture_sets[1].id, TextureSetId(1));
        assert!(
            project
                .texture_sets
                .iter()
                .all(|t| t.resolution.pixels() == 1024 && t.channels.len() == 5)
        );
        assert_eq!(project.normal_convention, NormalConvention::DirectX);
        assert_eq!(project.validate(), Ok(()));
    }

    #[test]
    fn validation_catches_broken_references() {
        let base = Project::new(two_materials(), "m.glb".into(), ProjectSettings::default());

        let mut project = base.clone();
        project.texture_sets[0].stack.layers = vec![fill(1), fill(1)];
        project.next_id = 2;
        assert_eq!(project.validate(), Err(ValidationError::DuplicateId(1)));

        project.texture_sets[0].stack.layers = vec![fill(5)];
        assert_eq!(
            project.validate(),
            Err(ValidationError::IdNotAllocated { id: 5, next_id: 2 })
        );

        let mut project = base.clone();
        let mut instance = fill(2);
        instance.kind = LayerKind::Instance { source: LayerId(9) };
        project.texture_sets[1].stack.layers = vec![instance.clone()];
        project.next_id = 10;
        assert_eq!(project.validate(), Err(ValidationError::InstanceSource(2)));
        // An instance of a layer of another texture set is valid.
        project.texture_sets[0].stack.layers = vec![fill(9)];
        assert_eq!(project.validate(), Ok(()));

        let mut project = base.clone();
        let mut masked = fill(1);
        masked.mask = Some(MaskStack {
            initial: MaskInitial::Black,
            effects: vec![MaskEffectEntry {
                id: crate::ids::EffectId(2),
                visible: true,
                blend: crate::layer::BlendMode::Normal,
                opacity: 1.0,
                effect: MaskEffect::AnchorRef {
                    anchor: AnchorId(3),
                    channel: Channel::Height,
                },
            }],
        });
        project.texture_sets[0].stack.layers = vec![masked.clone()];
        project.next_id = 4;
        assert_eq!(project.validate(), Err(ValidationError::MissingAnchor(3)));
        let mut anchored = fill(0);
        anchored.anchor = Some(Anchor {
            id: AnchorId(3),
            name: "height".into(),
            channels: vec![Channel::Height],
        });
        project.texture_sets[0].stack.layers = vec![anchored, masked];
        assert_eq!(project.validate(), Ok(()));

        let mut project = base.clone();
        let bytes: Arc<[u8]> = Arc::from(&b"hdr"[..]);
        let (hash, resource) = Resource::new("sky.hdr", ResourceKind::Environment, bytes);
        project.environment.source = EnvironmentSource::Resource(hash);
        assert_eq!(
            project.validate(),
            Err(ValidationError::MissingResource(hash))
        );
        project.resources.insert(hash, resource.clone());
        assert_eq!(project.validate(), Ok(()));
        project.resources.get_mut(&hash).unwrap().bytes = Arc::from(&b"xxx"[..]);
        assert_eq!(
            project.validate(),
            Err(ValidationError::CorruptedResource(hash))
        );

        let mut project = base;
        project.texture_sets[1].material_index = 0;
        assert_eq!(
            project.validate(),
            Err(ValidationError::DuplicateMaterial(0))
        );
    }
}
