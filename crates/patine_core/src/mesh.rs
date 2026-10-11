//! Triangle meshes as stored in the document.
//!
//! Conventions: object space, Y up, right-handed, meters. Colors are linear. Tangents
//! follow mikktspace: `[x, y, z, sign]`, bitangent = sign × cross(normal, tangent), and
//! interpolated vectors are not renormalized.

use std::ops::Range;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A contiguous range of triangles sharing one material.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Submesh {
    /// Index into [`MeshAsset::materials`].
    pub material: u32,
    /// Range into [`MeshAsset::triangles`].
    pub triangles: Range<u32>,
}

/// An indexed triangle mesh with everything later subsystems need.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MeshAsset {
    /// Display name, usually the file stem.
    pub name: String,
    /// Vertex positions.
    pub positions: Vec<[f32; 3]>,
    /// Unit vertex normals.
    pub normals: Vec<[f32; 3]>,
    /// First UV set. Required for a project mesh, optional for a bake source.
    pub uv0: Option<Vec<[f32; 2]>>,
    /// mikktspace tangents with the bitangent sign in `w`. Present if and only if `uv0` is.
    pub tangents: Option<Vec<[f32; 4]>>,
    /// Linear vertex colors.
    pub colors: Option<Vec<[f32; 4]>>,
    /// Triangles, sorted by material.
    pub triangles: Vec<[u32; 3]>,
    /// Source polygon of each triangle, for face selection.
    pub triangle_polygon: Vec<u32>,
    /// Source object of each triangle, index into [`MeshAsset::objects`].
    pub triangle_object: Vec<u32>,
    /// Names of the source objects.
    pub objects: Vec<String>,
    /// Material names; each one becomes a texture set.
    pub materials: Vec<String>,
    /// Material ranges of [`MeshAsset::triangles`], in order, covering every triangle.
    pub submeshes: Vec<Submesh>,
}

/// Axis-aligned bounding box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoundingBox {
    /// Minimum corner.
    pub min: [f32; 3],
    /// Maximum corner.
    pub max: [f32; 3],
}

impl BoundingBox {
    /// Smallest box containing every point, or `None` without points.
    pub fn from_points(points: &[[f32; 3]]) -> Option<Self> {
        let (first, rest) = points.split_first()?;
        let mut bounds = Self {
            min: *first,
            max: *first,
        };
        for point in rest {
            for ((min, max), value) in bounds.min.iter_mut().zip(&mut bounds.max).zip(point) {
                *min = min.min(*value);
                *max = max.max(*value);
            }
        }
        Some(bounds)
    }

    /// Size along each axis.
    pub fn size(&self) -> [f32; 3] {
        [
            self.max[0] - self.min[0],
            self.max[1] - self.min[1],
            self.max[2] - self.min[2],
        ]
    }

    /// Length of the diagonal.
    pub fn diagonal(&self) -> f32 {
        let [x, y, z] = self.size();
        (x * x + y * y + z * z).sqrt()
    }
}

/// A broken invariant of a [`MeshAsset`].
#[derive(Debug, Error, PartialEq)]
pub enum MeshValidationError {
    /// A vertex attribute does not have one value per vertex.
    #[error("attribute `{attribute}` has {actual} values for {expected} vertices")]
    AttributeLength {
        /// Attribute name.
        attribute: &'static str,
        /// Number of vertices.
        expected: usize,
        /// Number of values.
        actual: usize,
    },
    /// A per-triangle array does not have one value per triangle.
    #[error("`{array}` has {actual} values for {expected} triangles")]
    TriangleArrayLength {
        /// Array name.
        array: &'static str,
        /// Number of triangles.
        expected: usize,
        /// Number of values.
        actual: usize,
    },
    /// `uv0` and `tangents` must be both present or both absent.
    #[error("tangents must be present exactly when uv0 is")]
    TangentsWithoutUvs,
    /// A triangle references a vertex that does not exist.
    #[error("triangle {triangle} references vertex {vertex}, but there are {count} vertices")]
    VertexOutOfRange {
        /// Triangle index.
        triangle: usize,
        /// Referenced vertex.
        vertex: u32,
        /// Number of vertices.
        count: usize,
    },
    /// A per-triangle index points outside its table.
    #[error("triangle {triangle} references {table} entry {index}, but there are {count}")]
    TableIndexOutOfRange {
        /// Triangle index.
        triangle: usize,
        /// Table name.
        table: &'static str,
        /// Referenced entry.
        index: u32,
        /// Table length.
        count: usize,
    },
    /// An attribute holds NaN or infinity.
    #[error("attribute `{attribute}` has a non-finite value at vertex {vertex}")]
    NonFinite {
        /// Attribute name.
        attribute: &'static str,
        /// Vertex index.
        vertex: usize,
    },
    /// Submeshes are not contiguous, in order and covering every triangle.
    #[error("submeshes do not cover the triangles contiguously")]
    SubmeshCoverage,
}

fn check_len(
    attribute: &'static str,
    expected: usize,
    actual: usize,
) -> Result<(), MeshValidationError> {
    if expected == actual {
        Ok(())
    } else {
        Err(MeshValidationError::AttributeLength {
            attribute,
            expected,
            actual,
        })
    }
}

fn check_finite<const N: usize>(
    attribute: &'static str,
    values: &[[f32; N]],
) -> Result<(), MeshValidationError> {
    match values
        .iter()
        .position(|value| value.iter().any(|component| !component.is_finite()))
    {
        Some(vertex) => Err(MeshValidationError::NonFinite { attribute, vertex }),
        None => Ok(()),
    }
}

impl MeshAsset {
    /// Number of vertices.
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.triangles.len()
    }

    /// Bounding box of the vertices, or `None` for an empty mesh.
    pub fn bounds(&self) -> Option<BoundingBox> {
        BoundingBox::from_points(&self.positions)
    }

    /// Number of triangles using material `material`.
    pub fn material_triangle_count(&self, material: u32) -> usize {
        self.submeshes
            .iter()
            .filter(|submesh| submesh.material == material)
            .map(|submesh| submesh.triangles.len())
            .sum()
    }

    /// Checks every invariant documented on the fields.
    pub fn validate(&self) -> Result<(), MeshValidationError> {
        let vertices = self.vertex_count();
        check_len("normals", vertices, self.normals.len())?;
        if let Some(uv0) = &self.uv0 {
            check_len("uv0", vertices, uv0.len())?;
            check_finite("uv0", uv0)?;
        }
        match (&self.uv0, &self.tangents) {
            (Some(_), Some(tangents)) => {
                check_len("tangents", vertices, tangents.len())?;
                check_finite("tangents", tangents)?;
            }
            (None, None) => {}
            _ => return Err(MeshValidationError::TangentsWithoutUvs),
        }
        if let Some(colors) = &self.colors {
            check_len("colors", vertices, colors.len())?;
            check_finite("colors", colors)?;
        }
        check_finite("positions", &self.positions)?;
        check_finite("normals", &self.normals)?;

        let triangles = self.triangle_count();
        for (array, actual) in [
            ("triangle_polygon", self.triangle_polygon.len()),
            ("triangle_object", self.triangle_object.len()),
        ] {
            if actual != triangles {
                return Err(MeshValidationError::TriangleArrayLength {
                    array,
                    expected: triangles,
                    actual,
                });
            }
        }
        for (triangle, corners) in self.triangles.iter().enumerate() {
            if let Some(&vertex) = corners.iter().find(|&&v| v as usize >= vertices) {
                return Err(MeshValidationError::VertexOutOfRange {
                    triangle,
                    vertex,
                    count: vertices,
                });
            }
        }
        for (triangle, &object) in self.triangle_object.iter().enumerate() {
            if object as usize >= self.objects.len() {
                return Err(MeshValidationError::TableIndexOutOfRange {
                    triangle,
                    table: "objects",
                    index: object,
                    count: self.objects.len(),
                });
            }
        }

        let mut next = 0;
        for submesh in &self.submeshes {
            if submesh.triangles.start != next || submesh.triangles.end <= submesh.triangles.start {
                return Err(MeshValidationError::SubmeshCoverage);
            }
            if submesh.material as usize >= self.materials.len() {
                return Err(MeshValidationError::TableIndexOutOfRange {
                    triangle: next as usize,
                    table: "materials",
                    index: submesh.material,
                    count: self.materials.len(),
                });
            }
            next = submesh.triangles.end;
        }
        if next as usize != triangles {
            return Err(MeshValidationError::SubmeshCoverage);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit quad made of two triangles, one material, one object.
    fn quad() -> MeshAsset {
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
            triangle_polygon: vec![0, 0],
            triangle_object: vec![0, 0],
            objects: vec!["quad".into()],
            materials: vec!["default".into()],
            submeshes: vec![Submesh {
                material: 0,
                triangles: 0..2,
            }],
        }
    }

    #[test]
    fn valid_mesh_passes() {
        let mesh = quad();
        assert_eq!(mesh.validate(), Ok(()));
        assert_eq!(mesh.material_triangle_count(0), 2);
        let bounds = mesh.bounds().unwrap();
        assert_eq!(bounds.min, [0.0, 0.0, 0.0]);
        assert_eq!(bounds.max, [1.0, 1.0, 0.0]);
        assert!((bounds.diagonal() - 2f32.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn detects_attribute_length_mismatch() {
        let mut mesh = quad();
        mesh.normals.pop();
        assert!(matches!(
            mesh.validate(),
            Err(MeshValidationError::AttributeLength {
                attribute: "normals",
                ..
            })
        ));
    }

    #[test]
    fn detects_tangents_without_uvs() {
        let mut mesh = quad();
        mesh.uv0 = None;
        assert_eq!(
            mesh.validate(),
            Err(MeshValidationError::TangentsWithoutUvs)
        );
        mesh.tangents = None;
        assert_eq!(mesh.validate(), Ok(()));
    }

    #[test]
    fn detects_out_of_range_vertex() {
        let mut mesh = quad();
        mesh.triangles[1][2] = 4;
        assert!(matches!(
            mesh.validate(),
            Err(MeshValidationError::VertexOutOfRange {
                triangle: 1,
                vertex: 4,
                ..
            })
        ));
    }

    #[test]
    fn detects_non_finite_values() {
        let mut mesh = quad();
        mesh.positions[2][1] = f32::NAN;
        assert_eq!(
            mesh.validate(),
            Err(MeshValidationError::NonFinite {
                attribute: "positions",
                vertex: 2
            })
        );
    }

    #[test]
    fn detects_bad_submeshes() {
        let mut mesh = quad();
        mesh.submeshes[0].triangles = 0..1;
        assert_eq!(mesh.validate(), Err(MeshValidationError::SubmeshCoverage));
        mesh.submeshes = vec![
            Submesh {
                material: 0,
                triangles: 1..2,
            },
            Submesh {
                material: 0,
                triangles: 0..1,
            },
        ];
        assert_eq!(mesh.validate(), Err(MeshValidationError::SubmeshCoverage));
    }

    #[test]
    fn detects_bad_object_index() {
        let mut mesh = quad();
        mesh.triangle_object[0] = 3;
        assert!(matches!(
            mesh.validate(),
            Err(MeshValidationError::TableIndexOutOfRange {
                table: "objects",
                ..
            })
        ));
    }
}
