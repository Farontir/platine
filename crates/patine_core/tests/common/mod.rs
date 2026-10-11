//! Documents shared by the integration tests.

#![allow(dead_code)] // Each test file uses a subset.

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
