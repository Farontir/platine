//! glTF 2.0 reader. Buffers are loaded without decoding any image.

use std::collections::HashMap;
use std::path::Path;

use glam::{DMat4, DVec3, Mat4};
use gltf::mesh::Mode;

use super::{ObjectData, RawCollector};
use crate::MeshError;
use crate::raw::{NormalSource, RawMesh};

fn parse_error(error: impl std::fmt::Display) -> MeshError {
    MeshError::Parse {
        format: "glTF",
        message: error.to_string(),
    }
}

/// Corner indices of the triangles of a primitive, from its index list or vertex count.
fn triangle_corners(mode: Mode, indices: &[usize]) -> Option<Vec<[usize; 3]>> {
    match mode {
        Mode::Triangles => Some(indices.as_chunks::<3>().0.to_vec()),
        Mode::TriangleStrip => Some(
            (2..indices.len())
                .map(|i| {
                    // Every other triangle is reversed to keep a consistent winding.
                    if i % 2 == 0 {
                        [indices[i - 2], indices[i - 1], indices[i]]
                    } else {
                        [indices[i - 1], indices[i - 2], indices[i]]
                    }
                })
                .collect(),
        ),
        Mode::TriangleFan => Some(
            (2..indices.len())
                .map(|i| [indices[0], indices[i - 1], indices[i]])
                .collect(),
        ),
        Mode::Points | Mode::Lines | Mode::LineLoop | Mode::LineStrip => None,
    }
}

/// Reads the default scene (or the first one) of the glTF file at `path`.
pub(super) fn read(path: &Path, warnings: &mut Vec<String>) -> Result<RawMesh, MeshError> {
    let gltf = gltf::Gltf::open(path).map_err(parse_error)?;
    let buffers = gltf::import_buffers(&gltf.document, path.parent(), gltf.blob.clone())
        .map_err(parse_error)?;
    let document = &gltf.document;
    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .ok_or(MeshError::NoGeometry)?;

    let mut collector = RawCollector::new();
    let mut materials: HashMap<Option<usize>, u32> = HashMap::new();
    let mut stack: Vec<(gltf::Node<'_>, DMat4)> =
        scene.nodes().map(|node| (node, DMat4::IDENTITY)).collect();
    while let Some((node, parent)) = stack.pop() {
        let local = Mat4::from_cols_array_2d(&node.transform().matrix()).as_dmat4();
        let world = parent * local;
        stack.extend(node.children().map(|child| (child, world)));
        let Some(mesh) = node.mesh() else {
            continue;
        };
        let name = node
            .name()
            .or(mesh.name())
            .map_or_else(|| format!("node_{}", node.index()), str::to_owned);

        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| buffers.get(buffer.index()).map(|d| &d.0[..]));
            let Some(positions) = reader.read_positions() else {
                warnings.push(format!("{name}: primitive without positions skipped"));
                continue;
            };
            let positions: Vec<[f32; 3]> = positions.collect();
            let indices: Vec<usize> = match reader.read_indices() {
                Some(indices) => indices.into_u32().map(|i| i as usize).collect(),
                None => (0..positions.len()).collect(),
            };
            if let Some(&bad) = indices.iter().find(|&&i| i >= positions.len()) {
                return Err(parse_error(format!("{name}: index {bad} out of range")));
            }
            let Some(triangles) = triangle_corners(primitive.mode(), &indices) else {
                warnings.push(format!("{name}: {:?} primitive skipped", primitive.mode()));
                continue;
            };

            let material = primitive.material();
            let next = materials.len();
            let material_index = *materials.entry(material.index()).or_insert(next as u32);
            if material_index as usize == next {
                let material_name = match (material.index(), material.name()) {
                    (_, Some(name)) => name.to_owned(),
                    (Some(index), None) => format!("material_{index}"),
                    (None, None) => "default".to_owned(),
                };
                collector.add_material(material_name);
            }

            let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(Iterator::collect);
            let uvs: Option<Vec<[f32; 2]>> =
                reader.read_tex_coords(0).map(|uv| uv.into_f32().collect());
            let colors: Option<Vec<[f32; 4]>> =
                reader.read_colors(0).map(|c| c.into_rgba_f32().collect());
            for (attribute, len) in [
                ("NORMAL", normals.as_ref().map(Vec::len)),
                ("TEXCOORD_0", uvs.as_ref().map(Vec::len)),
                ("COLOR_0", colors.as_ref().map(Vec::len)),
            ] {
                if len.is_some_and(|len| len != positions.len()) {
                    return Err(parse_error(format!("{name}: {attribute} count mismatch")));
                }
            }

            let normal = |i: usize| normals.as_ref().map_or([0.0; 3], |n| n[i]);
            let uv = |i: usize| uvs.as_ref().map_or([0.0; 2], |u| u[i]);
            // glTF colors are linear already.
            let color = |i: usize| colors.as_ref().map_or([1.0; 4], |c| c[i]);
            let material_of = |_: usize| material_index;
            let data = ObjectData {
                name: name.clone(),
                transform: world,
                material_of_triangle: &material_of,
                normals: normals
                    .is_some()
                    .then_some(&normal as &dyn Fn(usize) -> [f32; 3]),
                // The glTF specification requires flat normals when NORMAL is absent.
                normal_source_if_missing: NormalSource::Flat,
                uvs: uvs.is_some().then_some(&uv as &dyn Fn(usize) -> [f32; 2]),
                colors: colors
                    .is_some()
                    .then_some(&color as &dyn Fn(usize) -> [f32; 4]),
            };
            let position = |i: usize| DVec3::from(glam::Vec3::from(positions[i]));
            // glTF has no polygons: each triangle is its own polygon.
            collector.add_object(&data, &triangles, &|t| t as u32, &position);
        }
    }
    Ok(collector.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_and_fans_keep_winding() {
        let strip = triangle_corners(Mode::TriangleStrip, &[0, 1, 2, 3]).unwrap();
        assert_eq!(strip, [[0, 1, 2], [2, 1, 3]]);
        let fan = triangle_corners(Mode::TriangleFan, &[0, 1, 2, 3]).unwrap();
        assert_eq!(fan, [[0, 1, 2], [0, 2, 3]]);
        assert!(triangle_corners(Mode::Lines, &[0, 1]).is_none());
    }
}
