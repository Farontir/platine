//! Format version 1 (M3 to M8), frozen: the types that version 2 changed, as version 1
//! wrote them, and their upgrade to the current types.
//!
//! Version 2 added `TextureSet::height_force`, `BrushSnapshot::{pressure, tilt_angle,
//! projection}` and `Stroke::space`. The upgrade gives the values that keep a version 1
//! project as it was: strokes ignore the pen pressure (they were painted before pressure
//! curves existed), height has its default force.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;

use crate::bake::BakedMaps;
use crate::channel::ChannelDef;
use crate::environment::EnvironmentSettings;
use crate::export::ExportPreset;
use crate::layer::{FillSource, LayerStack};
use crate::mesh::MeshAsset;
use crate::project::{self, NormalConvention, Resolution, TextureSetId};
use crate::resource::{Resource, ResourceHash};
use crate::stroke::{
    self, Dab, PressureCurves, SizeSpace, StrokeCamera, StrokeSpace, Symmetry, Tool,
};

/// Manifest of version 1.
#[derive(Deserialize)]
#[serde(rename = "Manifest")]
pub(super) struct Manifest {
    #[allow(dead_code)] // Checked by the caller through the header.
    format_version: u32,
    #[allow(dead_code)] // Kept for diagnostics.
    app_version: String,
    project: Project,
}

/// Project of version 1.
#[derive(Deserialize)]
struct Project {
    mesh_source: PathBuf,
    texture_sets: Vec<TextureSet>,
    environment: EnvironmentSettings,
    export_presets: Vec<ExportPreset>,
    resources: BTreeMap<ResourceHash, Resource>,
    normal_convention: NormalConvention,
    next_id: u64,
}

/// Texture set of version 1: no height force.
#[derive(Deserialize)]
struct TextureSet {
    id: TextureSetId,
    name: String,
    material_index: u32,
    resolution: Resolution,
    channels: Vec<ChannelDef>,
    baked_maps: BakedMaps,
    stack: LayerStack,
}

impl Manifest {
    /// The current project (its mesh, strokes and resource bytes are read next).
    pub(super) fn upgrade(self) -> project::Project {
        let p = self.project;
        project::Project {
            mesh: Arc::new(MeshAsset::default()),
            mesh_source: p.mesh_source,
            texture_sets: p
                .texture_sets
                .into_iter()
                .map(|set| project::TextureSet {
                    id: set.id,
                    name: set.name,
                    material_index: set.material_index,
                    resolution: set.resolution,
                    channels: set.channels,
                    height_force: project::DEFAULT_HEIGHT_FORCE,
                    baked_maps: set.baked_maps,
                    stack: set.stack,
                })
                .collect(),
            environment: p.environment,
            export_presets: p.export_presets,
            resources: p.resources,
            normal_convention: p.normal_convention,
            next_id: p.next_id,
        }
    }
}

/// Stroke log of version 1.
#[derive(Deserialize)]
pub(super) struct StrokeLog {
    strokes: Vec<Stroke>,
}

/// Stroke of version 1: always painted in the 3D view.
#[derive(Deserialize)]
struct Stroke {
    brush: BrushSnapshot,
    tool: Tool,
    camera: StrokeCamera,
    symmetry: Symmetry,
    seed: u64,
    dabs: Vec<Dab>,
}

/// Brush of version 1: no pressure curves, tilt or projection.
#[derive(Deserialize)]
struct BrushSnapshot {
    size: f32,
    size_space: SizeSpace,
    flow: f32,
    opacity: f32,
    hardness: f32,
    spacing: f32,
    angle: f32,
    size_jitter: f32,
    angle_jitter: f32,
    flow_jitter: f32,
    scatter: f32,
    follow_path: bool,
    alpha: Option<ResourceHash>,
    material: FillSource,
}

impl StrokeLog {
    /// The current stroke log: the same strokes, ignoring the pen pressure as version 1
    /// did.
    pub(super) fn upgrade(self) -> stroke::StrokeLog {
        stroke::StrokeLog {
            strokes: self
                .strokes
                .into_iter()
                .map(|s| {
                    let b = s.brush;
                    Arc::new(stroke::Stroke {
                        brush: stroke::BrushSnapshot {
                            size: b.size,
                            size_space: b.size_space,
                            flow: b.flow,
                            opacity: b.opacity,
                            hardness: b.hardness,
                            spacing: b.spacing,
                            angle: b.angle,
                            size_jitter: b.size_jitter,
                            angle_jitter: b.angle_jitter,
                            flow_jitter: b.flow_jitter,
                            scatter: b.scatter,
                            follow_path: b.follow_path,
                            alpha: b.alpha,
                            material: b.material,
                            pressure: PressureCurves::none(),
                            tilt_angle: false,
                            projection: stroke::BrushSnapshot::default_projection(),
                        },
                        tool: s.tool,
                        camera: s.camera,
                        space: StrokeSpace::View,
                        symmetry: s.symmetry,
                        seed: s.seed,
                        dabs: s.dabs,
                    })
                })
                .collect(),
        }
    }
}
