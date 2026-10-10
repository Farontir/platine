//! Import of the Blender-generated fixtures (tests/fixtures/meshes, see
//! tests/fixtures/generate_fixtures.py) in every format.
//!
//! M1 acceptance criteria 1 (the three formats load the fixtures) and 3 (a mesh without
//! UVs is refused with a clear error).

use std::collections::BTreeSet;
use std::path::PathBuf;

use glam::Vec3;
use patine_core::mesh::{BoundingBox, MeshAsset};
use patine_mesh::topology::MeshTopology;
use patine_mesh::{BuildOptions, MeshError, import};

/// Every file of a scene: binary and separate glTF, OBJ, FBX.
const EXTENSIONS: [&str; 4] = ["glb", "gltf", "obj", "fbx"];

fn fixture(scene: &str, extension: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/meshes")
        .join(format!("{scene}.{extension}"))
}

fn load(scene: &str, extension: &str) -> MeshAsset {
    let path = fixture(scene, extension);
    match import(&path, BuildOptions::default()) {
        Ok(imported) => imported.mesh,
        Err(error) => panic!("{}: {error}", path.display()),
    }
}

/// Loads a scene in every format.
fn load_all(scene: &str) -> Vec<(&'static str, MeshAsset)> {
    EXTENSIONS
        .iter()
        .map(|&ext| (ext, load(scene, ext)))
        .collect()
}

fn corners(mesh: &MeshAsset, triangle: usize) -> [Vec3; 3] {
    mesh.triangles[triangle].map(|v| Vec3::from(mesh.positions[v as usize]))
}

fn total_area(mesh: &MeshAsset) -> f32 {
    (0..mesh.triangle_count())
        .map(|t| {
            let p = corners(mesh, t);
            (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5
        })
        .sum()
}

fn assert_bounds(name: &str, bounds: BoundingBox, min: [f32; 3], max: [f32; 3]) {
    for axis in 0..3 {
        assert!(
            (bounds.min[axis] - min[axis]).abs() < 1e-4
                && (bounds.max[axis] - max[axis]).abs() < 1e-4,
            "{name}: bounds {bounds:?}, expected {min:?}..{max:?}"
        );
    }
}

/// Checks invariants every imported mesh must satisfy.
fn check_common(name: &str, mesh: &MeshAsset) {
    if let Err(error) = mesh.validate() {
        panic!("{name}: invalid mesh: {error}");
    }
    for normal in &mesh.normals {
        assert!(
            (Vec3::from(*normal).length() - 1.0).abs() < 1e-4,
            "{name}: normal {normal:?}"
        );
    }
    if let Some(tangents) = &mesh.tangents {
        for (tangent, normal) in tangents.iter().zip(&mesh.normals) {
            let t = Vec3::new(tangent[0], tangent[1], tangent[2]);
            assert!(
                t.dot(Vec3::from(*normal)).abs() < 1e-3,
                "{name}: tangent not orthogonal"
            );
        }
    }
    // Face winding agrees with the vertex normals.
    for t in 0..mesh.triangle_count() {
        let p = corners(mesh, t);
        let face = (p[1] - p[0]).cross(p[2] - p[0]);
        let normal: Vec3 = mesh.triangles[t]
            .iter()
            .map(|&v| Vec3::from(mesh.normals[v as usize]))
            .sum();
        assert!(
            face.dot(normal) > 0.0,
            "{name}: triangle {t} winding disagrees with its normals"
        );
    }
}

/// Every triangle of each closed part faces away from the part's center.
fn assert_outward(name: &str, mesh: &MeshAsset) {
    let topology = MeshTopology::new(mesh);
    for part in 0..topology.part_count() {
        let triangles: Vec<u32> = topology.parts.triangles(part).collect();
        let center = triangles
            .iter()
            .flat_map(|&t| corners(mesh, t as usize))
            .sum::<Vec3>()
            / (triangles.len() * 3) as f32;
        for &t in &triangles {
            let p = corners(mesh, t as usize);
            let face = (p[1] - p[0]).cross(p[2] - p[0]);
            let middle = (p[0] + p[1] + p[2]) / 3.0;
            assert!(
                face.dot(middle - center) > 0.0,
                "{name}: triangle {t} faces inward"
            );
        }
    }
}

#[test]
fn cube_with_two_materials() {
    for (format, mesh) in load_all("cube_materials") {
        let name = format!("cube_materials.{format}");
        check_common(&name, &mesh);
        assert_eq!(mesh.triangle_count(), 12, "{name}");
        let materials: BTreeSet<_> = mesh.materials.iter().map(String::as_str).collect();
        assert_eq!(materials, BTreeSet::from(["Metal", "Painted"]), "{name}");
        // Top and bottom are metal: 4 triangles.
        let metal = mesh.materials.iter().position(|m| m == "Metal").unwrap() as u32;
        assert_eq!(mesh.material_triangle_count(metal), 4, "{name}");
        let topology = MeshTopology::new(&mesh);
        assert_eq!(topology.island_count(), Some(6), "{name}");
        assert_eq!(topology.part_count(), 1, "{name}");
        assert_bounds(&name, mesh.bounds().unwrap(), [-1.0; 3], [1.0; 3]);
        assert_outward(&name, &mesh);
    }
}

#[test]
fn concave_ngon_is_triangulated_inside_its_outline() {
    for (format, mesh) in load_all("concave_ngon") {
        let name = format!("concave_ngon.{format}");
        check_common(&name, &mesh);
        assert_eq!(mesh.triangle_count(), 6, "{name}");
        // L-shaped hexagon (area 3) and quad (area 1): a triangulation leaving the outline
        // would overlap and exceed the area.
        assert!(
            (total_area(&mesh) - 4.0).abs() < 1e-4,
            "{name}: area {}",
            total_area(&mesh)
        );
        // Blender's +Z becomes +Y.
        for normal in &mesh.normals {
            assert!((Vec3::from(*normal) - Vec3::Y).length() < 1e-4, "{name}");
        }
        assert_bounds(
            &name,
            mesh.bounds().unwrap(),
            [0.0, 0.0, -2.0],
            [3.0, 0.0, 0.0],
        );
        let topology = MeshTopology::new(&mesh);
        assert_eq!(topology.island_count(), Some(1), "{name}");
        // OBJ and FBX keep the two source polygons; glTF has triangles only.
        let polygons: BTreeSet<_> = mesh.triangle_polygon.iter().collect();
        let expected = if format.starts_with("gl") { 6 } else { 2 };
        assert_eq!(polygons.len(), expected, "{name}");
    }
}

#[test]
fn objects_keep_their_transforms_including_mirroring() {
    let half_diagonal = 0.5 * 2f32.sqrt();
    for (format, mesh) in load_all("multi_object") {
        let name = format!("multi_object.{format}");
        check_common(&name, &mesh);
        assert_eq!(mesh.triangle_count(), 24, "{name}");
        assert_eq!(mesh.materials, ["Shared"], "{name}");
        let objects: BTreeSet<_> = mesh.objects.iter().map(String::as_str).collect();
        assert_eq!(objects, BTreeSet::from(["Left", "Right"]), "{name}");
        let topology = MeshTopology::new(&mesh);
        assert_eq!(topology.part_count(), 2, "{name}");
        assert_eq!(topology.island_count(), Some(12), "{name}");
        assert_bounds(
            &name,
            mesh.bounds().unwrap(),
            [-4.0, -1.0, -1.0],
            [3.0 + half_diagonal, 1.0, 1.0],
        );
        // The left cube has a negative scale: its winding must still face outward.
        assert_outward(&name, &mesh);
    }
}

#[test]
fn mesh_without_uvs_is_refused_with_its_object_name() {
    for format in EXTENSIONS {
        let path = fixture("no_uv", format);
        match import(&path, BuildOptions::default()) {
            Err(MeshError::MissingUvs { objects }) => assert_eq!(objects, ["Bare"], "{format}"),
            other => panic!("no_uv.{format}: expected MissingUvs, got {other:?}"),
        }
        // Bake sources may lack UVs.
        let imported = import(&path, BuildOptions { require_uvs: false }).unwrap();
        assert_eq!(imported.mesh.triangle_count(), 12, "{format}");
        assert!(imported.mesh.uv0.is_none());
    }
}

#[test]
fn vertex_colors_are_imported() {
    let expected = [
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
        [1.0; 4],
    ];
    for (format, mesh) in load_all("vertex_colors") {
        let name = format!("vertex_colors.{format}");
        check_common(&name, &mesh);
        let colors = mesh
            .colors
            .as_ref()
            .unwrap_or_else(|| panic!("{name}: no colors"));
        for color in &expected {
            assert!(
                colors
                    .iter()
                    .any(|c| c.iter().zip(color).all(|(a, b)| (a - b).abs() < 1e-3)),
                "{name}: missing color {color:?} in {colors:?}"
            );
        }
    }
}

#[test]
fn formats_agree_on_every_scene() {
    for scene in [
        "cube_materials",
        "concave_ngon",
        "multi_object",
        "vertex_colors",
    ] {
        let meshes = load_all(scene);
        let (_, reference) = &meshes[0];
        let reference_bounds = reference.bounds().unwrap();
        for (format, mesh) in &meshes[1..] {
            assert_eq!(
                mesh.triangle_count(),
                reference.triangle_count(),
                "{scene}.{format}"
            );
            assert_eq!(
                mesh.materials.len(),
                reference.materials.len(),
                "{scene}.{format}"
            );
            let bounds = mesh.bounds().unwrap();
            assert_bounds(
                &format!("{scene}.{format}"),
                bounds,
                reference_bounds.min,
                reference_bounds.max,
            );
            assert!(
                (total_area(mesh) - total_area(reference)).abs() < 1e-3,
                "{scene}.{format}"
            );
        }
    }
}
