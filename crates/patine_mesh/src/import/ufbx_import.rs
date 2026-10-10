//! FBX and OBJ reader, through ufbx.
//!
//! Scenes are converted to a right-handed, Y-up space in meters. Faces are triangulated
//! by ufbx, which handles concave polygons. Vertex colors of FBX and OBJ files are
//! assumed sRGB-encoded (the default of common exporters) and converted to linear.

use std::collections::HashMap;
use std::path::Path;

use glam::{DMat4, DVec3, DVec4};

use super::{MeshFormat, ObjectData, RawCollector, srgb_to_linear};
use crate::MeshError;
use crate::raw::{NormalSource, RawMesh};

fn to_dmat4(m: &ufbx::Matrix) -> DMat4 {
    DMat4::from_cols(
        DVec4::new(m.m00, m.m10, m.m20, 0.0),
        DVec4::new(m.m01, m.m11, m.m21, 0.0),
        DVec4::new(m.m02, m.m12, m.m22, 0.0),
        DVec4::new(m.m03, m.m13, m.m23, 1.0),
    )
}

/// Reads the FBX or OBJ file at `path`.
pub(super) fn read(
    path: &Path,
    format: MeshFormat,
    warnings: &mut Vec<String>,
) -> Result<RawMesh, MeshError> {
    let parse_error = |message: String| MeshError::Parse {
        format: format.name(),
        message,
    };
    let filename = path
        .to_str()
        .ok_or_else(|| parse_error("the file path is not valid Unicode".to_owned()))?;
    let options = ufbx::LoadOpts {
        target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
        // FBX files declare their axes and unit; OBJ files do not and are kept as
        // authored (Y up, meters, the convention of common exporters).
        target_unit_meters: 1.0,
        ignore_animation: true,
        ignore_embedded: true,
        load_external_files: false,
        ..Default::default()
    };
    let scene = ufbx::load_file(filename, options)
        .map_err(|error| parse_error(format!("{} ({})", &*error.description, error.info())))?;

    let mut collector = RawCollector::new();
    // Materials are shared between meshes: key them by their element id.
    let mut materials: HashMap<Option<u32>, u32> = HashMap::new();
    let mut triangle_indices = Vec::new();

    for node in &scene.nodes {
        let Some(mesh) = &node.mesh else {
            continue;
        };
        let name = if node.element.name.is_empty() {
            mesh.element.name.to_string()
        } else {
            node.element.name.to_string()
        };

        // Global material of each mesh-local material slot.
        let mut slot_material = Vec::with_capacity(mesh.materials.len().max(1));
        for material in &mesh.materials {
            let key = Some(material.element.element_id);
            let next = materials.len() as u32;
            let index = *materials.entry(key).or_insert(next);
            if index == next {
                collector.add_material(material.element.name.to_string());
            }
            slot_material.push(index);
        }
        if slot_material.is_empty() {
            let next = materials.len() as u32;
            let index = *materials.entry(None).or_insert(next);
            if index == next {
                collector.add_material("default".to_owned());
            }
            slot_material.push(index);
        }

        let mut triangles: Vec<[usize; 3]> = Vec::with_capacity(mesh.num_triangles);
        let mut triangle_face: Vec<u32> = Vec::with_capacity(mesh.num_triangles);
        let mut skipped = 0;
        for (face_index, face) in mesh.faces.iter().enumerate() {
            if face.num_indices < 3 {
                skipped += 1;
                continue;
            }
            let count = ufbx::triangulate_face_vec(&mut triangle_indices, mesh, *face);
            for corners in triangle_indices
                .as_chunks::<3>()
                .0
                .iter()
                .take(count as usize)
            {
                triangles.push([
                    corners[0] as usize,
                    corners[1] as usize,
                    corners[2] as usize,
                ]);
                triangle_face.push(face_index as u32);
            }
        }
        if skipped > 0 {
            warnings.push(format!("{name}: {skipped} point or line faces skipped"));
        }

        let material_of = |triangle: usize| {
            let face = triangle_face[triangle] as usize;
            let slot = mesh.face_material.get(face).copied().unwrap_or(0) as usize;
            slot_material.get(slot).copied().unwrap_or(slot_material[0])
        };
        let normal = |i: usize| {
            let n = mesh.vertex_normal[i];
            [n.x as f32, n.y as f32, n.z as f32]
        };
        let uv = |i: usize| {
            let t = mesh.vertex_uv[i];
            [t.x as f32, t.y as f32]
        };
        let color = |i: usize| {
            let c = mesh.vertex_color[i];
            [
                srgb_to_linear(c.x as f32),
                srgb_to_linear(c.y as f32),
                srgb_to_linear(c.z as f32),
                c.w as f32,
            ]
        };
        let data = ObjectData {
            name,
            transform: to_dmat4(&node.geometry_to_world),
            material_of_triangle: &material_of,
            normals: mesh
                .vertex_normal
                .exists
                .then_some(&normal as &dyn Fn(usize) -> [f32; 3]),
            normal_source_if_missing: NormalSource::Smooth,
            uvs: mesh
                .vertex_uv
                .exists
                .then_some(&uv as &dyn Fn(usize) -> [f32; 2]),
            colors: mesh
                .vertex_color
                .exists
                .then_some(&color as &dyn Fn(usize) -> [f32; 4]),
        };
        let position = |i: usize| {
            let p = mesh.vertex_position[i];
            DVec3::new(p.x, p.y, p.z)
        };
        collector.add_object(&data, &triangles, &|t| triangle_face[t], &position);
    }
    Ok(collector.finish())
}
