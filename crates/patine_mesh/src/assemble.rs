//! Turns a [`RawMesh`] into a validated, indexed [`MeshAsset`].
//!
//! Steps: UV check, degenerate removal, normals, mikktspace tangents, vertex welding,
//! sorting by material.

use std::time::{Duration, Instant};

use bevy_mikktspace::{Geometry, TangentSpace};
use glam::Vec3;
use patine_core::mesh::{MeshAsset, Submesh};

use crate::MeshError;
use crate::hash::{FxHashMap, float_key, position_key};
use crate::raw::{NormalSource, RawMesh};

/// Options of [`assemble`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildOptions {
    /// Refuse meshes with objects lacking UVs (true for project meshes, false for bake
    /// sources).
    pub require_uvs: bool,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self { require_uvs: true }
    }
}

/// What happened while building a mesh.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildReport {
    /// Triangles read from the source.
    pub source_triangles: usize,
    /// Triangles removed because their area is zero.
    pub degenerate_removed: usize,
    /// Provided normals that were zero or invalid and got replaced by the face normal.
    pub invalid_normals_replaced: usize,
    /// Vertices whose UVs lie outside the unit square (UDIM tiles are not supported).
    pub uvs_outside_unit_square: usize,
    /// Materials without any triangle, dropped.
    pub unused_materials: Vec<String>,
    /// Duration of each step.
    pub timings: Vec<(&'static str, Duration)>,
}

/// Builds the final mesh named `name` from `raw`.
pub fn assemble(
    name: &str,
    mut raw: RawMesh,
    options: BuildOptions,
) -> Result<(MeshAsset, BuildReport), MeshError> {
    debug_assert_eq!(raw.positions.len(), raw.triangle_count() * 3);
    debug_assert_eq!(raw.normals.len(), raw.positions.len());
    let mut report = BuildReport {
        source_triangles: raw.triangle_count(),
        ..BuildReport::default()
    };
    if raw.triangle_count() == 0 {
        return Err(MeshError::NoGeometry);
    }

    check_uvs(&mut raw, options)?;

    let start = Instant::now();
    report.degenerate_removed = remove_degenerate(&mut raw);
    if raw.triangle_count() == 0 {
        return Err(MeshError::NoGeometry);
    }
    report.timings.push(("cleanup", start.elapsed()));

    let start = Instant::now();
    report.invalid_normals_replaced = compute_normals(&mut raw);
    report.timings.push(("normals", start.elapsed()));

    let start = Instant::now();
    let tangents = match &raw.uvs {
        Some(uvs) => Some(generate_tangents(&raw.positions, &raw.normals, uvs)?),
        None => None,
    };
    report.timings.push(("tangents", start.elapsed()));

    let start = Instant::now();
    let mesh = weld_and_sort(name, raw, tangents, &mut report)?;
    report.timings.push(("weld", start.elapsed()));

    mesh.validate()?;
    Ok((mesh, report))
}

/// Applies the UV policy: refuse, or drop partial UVs for bake sources.
fn check_uvs(raw: &mut RawMesh, options: BuildOptions) -> Result<(), MeshError> {
    let mut used = vec![false; raw.objects.len()];
    for &object in &raw.triangle_object {
        used[object as usize] = true;
    }
    let missing: Vec<String> = raw
        .objects
        .iter()
        .zip(&used)
        .filter(|(object, used)| **used && (raw.uvs.is_none() || !object.has_uvs))
        .map(|(object, _)| object.name.clone())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    if options.require_uvs {
        return Err(MeshError::MissingUvs { objects: missing });
    }
    raw.uvs = None;
    Ok(())
}

fn corner_positions(raw: &RawMesh, triangle: usize) -> [Vec3; 3] {
    std::array::from_fn(|corner| Vec3::from(raw.positions[triangle * 3 + corner]))
}

fn face_cross(p: [Vec3; 3]) -> Vec3 {
    (p[1] - p[0]).cross(p[2] - p[0])
}

/// Removes zero-area triangles. Returns how many were removed.
fn remove_degenerate(raw: &mut RawMesh) -> usize {
    let diagonal = patine_core::mesh::BoundingBox::from_points(&raw.positions)
        .map_or(0.0, |bounds| bounds.diagonal());
    // Twice the area below which a triangle is noise at f32 precision.
    let threshold = 1e-12 * diagonal * diagonal;
    let keep: Vec<bool> = (0..raw.triangle_count())
        .map(|triangle| {
            let cross = face_cross(corner_positions(raw, triangle));
            cross.is_finite() && cross.length() > threshold
        })
        .collect();
    let removed = keep.iter().filter(|keep| !**keep).count();
    if removed == 0 {
        return 0;
    }

    fn retain_corners<T: Copy>(values: &mut Vec<T>, keep: &[bool]) {
        let mut index = 0;
        values.retain(|_| {
            let kept = keep[index / 3];
            index += 1;
            kept
        });
    }
    fn retain_triangles<T: Copy>(values: &mut Vec<T>, keep: &[bool]) {
        let mut index = 0;
        values.retain(|_| {
            let kept = keep[index];
            index += 1;
            kept
        });
    }
    retain_corners(&mut raw.positions, &keep);
    retain_corners(&mut raw.normals, &keep);
    if let Some(uvs) = &mut raw.uvs {
        retain_corners(uvs, &keep);
    }
    if let Some(colors) = &mut raw.colors {
        retain_corners(colors, &keep);
    }
    retain_triangles(&mut raw.triangle_polygon, &keep);
    retain_triangles(&mut raw.triangle_object, &keep);
    retain_triangles(&mut raw.triangle_material, &keep);
    removed
}

/// Fills the normals of every corner according to its object's [`NormalSource`].
/// Returns the number of provided normals replaced because they were invalid.
fn compute_normals(raw: &mut RawMesh) -> usize {
    let triangles = raw.triangle_count();
    let face_normals: Vec<Vec3> = (0..triangles)
        .map(|triangle| face_cross(corner_positions(raw, triangle)).normalize_or_zero())
        .collect();

    // Angle-weighted sums per (object, position), for smooth objects only.
    let mut sums: FxHashMap<(u32, [u32; 3]), Vec3> = FxHashMap::default();
    let source = |raw: &RawMesh, triangle: usize| {
        raw.objects[raw.triangle_object[triangle] as usize].normals
    };
    let corner_angle = |p: [Vec3; 3], corner: usize| {
        let a = p[(corner + 1) % 3] - p[corner];
        let b = p[(corner + 2) % 3] - p[corner];
        a.angle_between(b)
    };
    for (triangle, &face) in face_normals.iter().enumerate() {
        if source(raw, triangle) != NormalSource::Smooth {
            continue;
        }
        let p = corner_positions(raw, triangle);
        let object = raw.triangle_object[triangle];
        for corner in 0..3 {
            let key = (object, position_key(raw.positions[triangle * 3 + corner]));
            let weight = corner_angle(p, corner);
            *sums.entry(key).or_insert(Vec3::ZERO) += face * weight;
        }
    }

    let mut replaced = 0;
    for (triangle, &face) in face_normals.iter().enumerate() {
        let object = raw.triangle_object[triangle];
        for corner in 0..3 {
            let index = triangle * 3 + corner;
            let normal = match source(raw, triangle) {
                NormalSource::Flat => face,
                NormalSource::Smooth => {
                    let key = (object, position_key(raw.positions[index]));
                    sums.get(&key)
                        .copied()
                        .unwrap_or(face)
                        .try_normalize()
                        .unwrap_or(face)
                }
                NormalSource::Provided => match Vec3::from(raw.normals[index]).try_normalize() {
                    Some(normal) => normal,
                    None => {
                        replaced += 1;
                        face
                    }
                },
            };
            raw.normals[index] = normal.to_array();
        }
    }
    replaced
}

/// Corner-based geometry handed to mikktspace.
struct MikkGeometry<'a> {
    positions: &'a [[f32; 3]],
    normals: &'a [[f32; 3]],
    uvs: &'a [[f32; 2]],
    tangents: Vec<[f32; 4]>,
}

impl Geometry for MikkGeometry<'_> {
    fn num_faces(&self) -> usize {
        self.positions.len() / 3
    }

    fn num_vertices_of_face(&self, _face: usize) -> usize {
        3
    }

    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        self.positions[face * 3 + vert]
    }

    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        self.normals[face * 3 + vert]
    }

    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        self.uvs[face * 3 + vert]
    }

    fn set_tangent(&mut self, tangent_space: Option<TangentSpace>, face: usize, vert: usize) {
        // mikktspace gives every corner of a valid face a tangent space; the fallback
        // only covers faces it ignores.
        self.tangents[face * 3 + vert] = tangent_space
            .map(|space| space.tangent_encoded())
            .unwrap_or([1.0, 0.0, 0.0, 1.0]);
    }
}

/// mikktspace tangents of every corner.
pub(crate) fn generate_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
) -> Result<Vec<[f32; 4]>, MeshError> {
    let mut geometry = MikkGeometry {
        positions,
        normals,
        uvs,
        tangents: vec![[1.0, 0.0, 0.0, 1.0]; positions.len()],
    };
    bevy_mikktspace::generate_tangents(&mut geometry)
        .map_err(|error| MeshError::Tangents(format!("{error:?}")))?;
    Ok(geometry.tangents)
}

/// Welds identical corners into vertices and sorts triangles by material.
fn weld_and_sort(
    name: &str,
    raw: RawMesh,
    tangents: Option<Vec<[f32; 4]>>,
    report: &mut BuildReport,
) -> Result<MeshAsset, MeshError> {
    let corners = raw.positions.len();
    let mut map: FxHashMap<[u32; 16], u32> = FxHashMap::default();
    map.reserve(corners / 2);
    let mut corner_vertex = Vec::with_capacity(corners);
    let mut first_corner = Vec::new();
    for corner in 0..corners {
        let mut key = [0u32; 16];
        key[0..3].copy_from_slice(&raw.positions[corner].map(float_key));
        key[3..6].copy_from_slice(&raw.normals[corner].map(float_key));
        if let Some(uvs) = &raw.uvs {
            key[6..8].copy_from_slice(&uvs[corner].map(float_key));
        }
        if let Some(tangents) = &tangents {
            key[8..12].copy_from_slice(&tangents[corner].map(float_key));
        }
        if let Some(colors) = &raw.colors {
            key[12..16].copy_from_slice(&colors[corner].map(float_key));
        }
        let next = first_corner.len();
        let vertex = *map.entry(key).or_insert_with(|| {
            first_corner.push(corner);
            // Checked right after the loop.
            next as u32
        });
        corner_vertex.push(vertex);
    }
    if first_corner.len() > u32::MAX as usize {
        return Err(MeshError::TooManyVertices(first_corner.len()));
    }

    let gather = |values: &[_]| first_corner.iter().map(|&c| values[c]).collect::<Vec<_>>();
    let positions: Vec<[f32; 3]> = gather(&raw.positions);
    let normals: Vec<[f32; 3]> = gather(&raw.normals);
    let uv0: Option<Vec<[f32; 2]>> = raw
        .uvs
        .as_deref()
        .map(|uvs| first_corner.iter().map(|&c| uvs[c]).collect());
    let tangents: Option<Vec<[f32; 4]>> = tangents
        .as_deref()
        .map(|tangents| first_corner.iter().map(|&c| tangents[c]).collect());
    let colors: Option<Vec<[f32; 4]>> = raw
        .colors
        .as_deref()
        .map(|colors| first_corner.iter().map(|&c| colors[c]).collect());
    if let Some(uv0) = &uv0 {
        report.uvs_outside_unit_square = uv0
            .iter()
            .filter(|uv| uv.iter().any(|c| !(0.0..=1.0).contains(c)))
            .count();
    }

    // Stable sort by material, then drop unused materials.
    let mut order: Vec<usize> = (0..raw.triangle_count()).collect();
    order.sort_by_key(|&triangle| raw.triangle_material[triangle]);
    let mut material_remap = vec![u32::MAX; raw.materials.len()];
    let mut materials = Vec::new();
    let mut submeshes: Vec<Submesh> = Vec::new();
    for (new_index, &triangle) in order.iter().enumerate() {
        let old = raw.triangle_material[triangle] as usize;
        if material_remap[old] == u32::MAX {
            material_remap[old] = materials.len() as u32;
            materials.push(raw.materials[old].clone());
            submeshes.push(Submesh {
                material: material_remap[old],
                triangles: new_index as u32..new_index as u32,
            });
        }
        if let Some(last) = submeshes.last_mut() {
            last.triangles.end = new_index as u32 + 1;
        }
    }
    report.unused_materials = raw
        .materials
        .iter()
        .zip(&material_remap)
        .filter(|(_, remap)| **remap == u32::MAX)
        .map(|(name, _)| name.clone())
        .collect();

    let triangles = order
        .iter()
        .map(|&t| std::array::from_fn(|corner| corner_vertex[t * 3 + corner]))
        .collect();
    Ok(MeshAsset {
        name: name.to_owned(),
        positions,
        normals,
        uv0,
        tangents,
        colors,
        triangles,
        triangle_polygon: order.iter().map(|&t| raw.triangle_polygon[t]).collect(),
        triangle_object: order.iter().map(|&t| raw.triangle_object[t]).collect(),
        objects: raw.objects.into_iter().map(|object| object.name).collect(),
        materials,
        submeshes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::RawObject;

    const WHITE: [[f32; 4]; 3] = [[1.0; 4]; 3];

    fn object(normals: NormalSource) -> RawObject {
        RawObject {
            name: "object".into(),
            normals,
            has_uvs: true,
        }
    }

    /// Unit quad in the XY plane, UVs equal to XY, as two triangles sharing an edge.
    fn quad(normals: NormalSource) -> RawMesh {
        let mut raw = RawMesh {
            uvs: Some(Vec::new()),
            objects: vec![object(normals)],
            materials: vec!["default".into()],
            ..RawMesh::default()
        };
        let p = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let uv = |i: usize| [p[i][0], p[i][1]];
        let n = [[0.0, 0.0, 1.0]; 3];
        for (polygon, [a, b, c]) in [[0, 1, 2], [0, 2, 3]].into_iter().enumerate() {
            raw.push_triangle(
                [p[a], p[b], p[c]],
                n,
                [uv(a), uv(b), uv(c)],
                WHITE,
                polygon as u32,
                0,
                0,
            );
        }
        raw
    }

    #[test]
    fn quad_welds_to_four_vertices_with_plus_x_tangents() {
        let (mesh, report) = assemble(
            "quad",
            quad(NormalSource::Provided),
            BuildOptions::default(),
        )
        .unwrap();
        assert_eq!(mesh.vertex_count(), 4);
        assert_eq!(mesh.triangle_count(), 2);
        assert_eq!(report.degenerate_removed, 0);
        for tangent in mesh.tangents.as_ref().unwrap() {
            assert!((tangent[0] - 1.0).abs() < 1e-6, "{tangent:?}");
            assert!(tangent[1].abs() < 1e-6 && tangent[2].abs() < 1e-6);
            // U along +X, V along +Y, normal +Z: right-handed, sign +1.
            assert_eq!(tangent[3], 1.0);
        }
    }

    #[test]
    fn flat_and_smooth_normals_on_a_plane_point_up() {
        for source in [NormalSource::Flat, NormalSource::Smooth] {
            let mut raw = quad(source);
            raw.normals.fill([0.0; 3]);
            let (mesh, _) = assemble("quad", raw, BuildOptions::default()).unwrap();
            for normal in &mesh.normals {
                assert!(
                    (Vec3::from(*normal) - Vec3::Z).length() < 1e-6,
                    "{source:?}"
                );
            }
        }
    }

    #[test]
    fn smooth_normals_average_across_a_fold() {
        // Two triangles folded at 90° along the X axis: the shared edge gets the bisector.
        let mut raw = RawMesh {
            uvs: Some(Vec::new()),
            objects: vec![object(NormalSource::Smooth)],
            materials: vec!["default".into()],
            ..RawMesh::default()
        };
        let zero = [[0.0; 3]; 3];
        let uvs = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]];
        // Floor (normal +Y) and wall (normal -Z, from its winding), sharing the edge
        // (0,0,0)-(1,0,0).
        raw.push_triangle(
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]],
            zero,
            uvs,
            WHITE,
            0,
            0,
            0,
        );
        raw.push_triangle(
            [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
            zero,
            uvs,
            WHITE,
            1,
            0,
            0,
        );
        let (mesh, _) = assemble("fold", raw, BuildOptions::default()).unwrap();
        let expected = Vec3::new(0.0, 1.0, -1.0).normalize();
        let on_edge = mesh
            .positions
            .iter()
            .zip(&mesh.normals)
            .filter(|(p, _)| p[1] == 0.0 && p[2] == 0.0)
            .count();
        assert!(on_edge >= 2);
        for (position, normal) in mesh.positions.iter().zip(&mesh.normals) {
            if position[1] == 0.0 && position[2] == 0.0 {
                assert!(
                    (Vec3::from(*normal) - expected).length() < 1e-5,
                    "{normal:?}"
                );
            }
        }
    }

    #[test]
    fn degenerate_triangles_are_removed() {
        let mut raw = quad(NormalSource::Provided);
        let p = [0.5, 0.5, 0.0];
        raw.push_triangle(
            [p, p, p],
            [[0.0, 0.0, 1.0]; 3],
            [[0.5; 2]; 3],
            WHITE,
            2,
            0,
            0,
        );
        let (mesh, report) = assemble("quad", raw, BuildOptions::default()).unwrap();
        assert_eq!(report.degenerate_removed, 1);
        assert_eq!(mesh.triangle_count(), 2);
    }

    #[test]
    fn only_degenerate_triangles_is_no_geometry() {
        let mut raw = quad(NormalSource::Provided);
        raw.positions.fill([0.0; 3]);
        assert!(matches!(
            assemble("quad", raw, BuildOptions::default()),
            Err(MeshError::NoGeometry)
        ));
    }

    #[test]
    fn missing_uvs_are_refused_for_projects_and_dropped_for_bake_sources() {
        let mut raw = quad(NormalSource::Provided);
        raw.objects[0].has_uvs = false;
        match assemble("quad", raw.clone(), BuildOptions::default()) {
            Err(MeshError::MissingUvs { objects }) => assert_eq!(objects, ["object"]),
            other => panic!("expected MissingUvs, got {other:?}"),
        }
        let (mesh, _) = assemble("quad", raw, BuildOptions { require_uvs: false }).unwrap();
        assert!(mesh.uv0.is_none() && mesh.tangents.is_none());
    }

    #[test]
    fn triangles_are_sorted_by_material_and_unused_materials_dropped() {
        let mut raw = quad(NormalSource::Provided);
        raw.materials = vec!["unused".into(), "b".into(), "a".into()];
        raw.triangle_material = vec![2, 1];
        let (mesh, report) = assemble("quad", raw, BuildOptions::default()).unwrap();
        assert_eq!(mesh.materials, ["b", "a"]);
        assert_eq!(mesh.submeshes.len(), 2);
        assert_eq!(mesh.submeshes[0].material, 0);
        assert_eq!(mesh.submeshes[1].triangles, 1..2);
        // The triangle of material "b" (source polygon 1) now comes first.
        assert_eq!(mesh.triangle_polygon, [1, 0]);
        assert_eq!(report.unused_materials, ["unused"]);
    }

    #[test]
    fn invalid_provided_normals_are_replaced() {
        let mut raw = quad(NormalSource::Provided);
        raw.normals[0] = [0.0; 3];
        raw.normals[1] = [f32::NAN, 0.0, 0.0];
        let (mesh, report) = assemble("quad", raw, BuildOptions::default()).unwrap();
        assert_eq!(report.invalid_normals_replaced, 2);
        assert!(
            mesh.normals
                .iter()
                .all(|n| (Vec3::from(*n) - Vec3::Z).length() < 1e-6)
        );
    }
}
