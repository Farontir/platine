//! Documents shared by the integration tests.

#![allow(dead_code, unused_imports)] // Each test file uses a subset.

pub mod ops;
mod rich;

pub use rich::rich_project;

use patine_core::channel::Channel;
use patine_core::ids::{IdSequence, LayerId};
use patine_core::layer::{FillSource, Layer, LayerKind, Projection};
use patine_core::mesh::{MeshAsset, Submesh};
use patine_core::project::{Project, ProjectSettings};

/// A valid two-triangle mesh with two materials.
pub fn mesh() -> MeshAsset {
    MeshAsset {
        name: "quad".into(),
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        uv0: Some(vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]),
        tangents: Some(vec![[1.0, 0.0, 0.0, 1.0]; 4]),
        colors: None,
        triangles: vec![[0, 1, 2], [0, 2, 3]],
        triangle_polygon: vec![0, 1],
        triangle_object: vec![0, 0],
        objects: vec!["quad".into()],
        materials: vec!["paint".into(), "metal".into()],
        submeshes: vec![
            Submesh {
                material: 0,
                triangles: 0..1,
            },
            Submesh {
                material: 1,
                triangles: 1..2,
            },
        ],
    }
}

/// A new project on [`mesh`].
pub fn project() -> Project {
    let project = Project::new(mesh(), "quad.glb".into(), ProjectSettings::default());
    project.validate().expect("the test project is valid");
    project
}

/// A uniform fill layer affecting the base color.
pub fn fill(id: LayerId, name: &str) -> Layer {
    Layer::new(
        id,
        name,
        LayerKind::Fill {
            source: FillSource::Uniform([(Channel::BaseColor, [0.8, 0.1, 0.1, 1.0])].into()),
            projection: Projection::default(),
        },
        &[Channel::BaseColor, Channel::Roughness],
    )
}

/// An empty folder.
pub fn folder(id: LayerId, name: &str) -> Layer {
    Layer::new(
        id,
        name,
        LayerKind::Folder {
            children: Vec::new(),
        },
        &[],
    )
}

/// Identifiers starting at the project's next free one.
pub fn ids(project: &Project) -> IdSequence {
    IdSequence::starting_at(project.next_id)
}

/// A brush stroke of `dabs` dabs painting the base color and the roughness (or erasing
/// them), its values drawn from `seed`.
pub fn stroke(seed: u64, eraser: bool, dabs: usize) -> std::sync::Arc<patine_core::stroke::Stroke> {
    use patine_core::stroke::{
        BrushSnapshot, Dab, SizeSpace, Stroke, StrokeCamera, Symmetry, Tool,
    };
    let value = (seed % 97) as f32 / 97.0;
    std::sync::Arc::new(Stroke {
        brush: BrushSnapshot {
            size: 20.0 + value * 30.0,
            size_space: SizeSpace::Screen,
            flow: 0.5,
            opacity: 1.0 - value * 0.5,
            hardness: value,
            spacing: 0.1,
            angle: 0.0,
            size_jitter: 0.0,
            angle_jitter: 0.0,
            flow_jitter: 0.0,
            scatter: 0.0,
            follow_path: false,
            alpha: None,
            material: FillSource::Uniform(
                [
                    (Channel::BaseColor, [value, 0.2, 0.1, 1.0]),
                    (Channel::Roughness, [1.0 - value; 4]),
                ]
                .into(),
            ),
        },
        tool: if eraser { Tool::Eraser } else { Tool::Brush },
        camera: StrokeCamera {
            view_proj: identity(),
            viewport: [640, 480],
        },
        symmetry: Symmetry::default(),
        seed,
        dabs: (0..dabs)
            .map(|i| Dab {
                position: [10.0 + i as f32 * 3.0, 200.0 + value * 50.0],
                pressure: 1.0,
                tilt: [0.0, 0.0],
                time: i as f32 / 60.0,
            })
            .collect(),
    })
}

fn identity() -> [[f32; 4]; 4] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]
}
