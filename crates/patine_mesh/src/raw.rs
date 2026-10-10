//! Unindexed triangle soup produced by the importers and the test-mesh generators.

/// How the normals of an object are obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalSource {
    /// The file provides normals in [`RawMesh::normals`].
    Provided,
    /// Missing: compute one normal per triangle (glTF rule).
    Flat,
    /// Missing: compute angle-weighted normals shared by corners at the same position.
    Smooth,
}

/// A source object (node) of the mesh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawObject {
    /// Object name.
    pub name: String,
    /// Origin of its normals.
    pub normals: NormalSource,
    /// Whether the file provides UVs for it. If not, its corners hold `[0, 0]`.
    pub has_uvs: bool,
}

/// Triangle corners: arrays indexed by `triangle * 3 + corner`, all of the same length.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RawMesh {
    /// Corner positions, already transformed to the mesh space.
    pub positions: Vec<[f32; 3]>,
    /// Corner normals; ignored for objects whose normals are not [`NormalSource::Provided`].
    pub normals: Vec<[f32; 3]>,
    /// Corner UVs, or `None` if no object has UVs.
    pub uvs: Option<Vec<[f32; 2]>>,
    /// Corner colors, or `None` if no object has vertex colors.
    pub colors: Option<Vec<[f32; 4]>>,
    /// Source polygon of each triangle.
    pub triangle_polygon: Vec<u32>,
    /// Source object of each triangle, index into `objects`.
    pub triangle_object: Vec<u32>,
    /// Material of each triangle, index into `materials`.
    pub triangle_material: Vec<u32>,
    /// Source objects.
    pub objects: Vec<RawObject>,
    /// Material names.
    pub materials: Vec<String>,
}

impl RawMesh {
    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.triangle_material.len()
    }

    /// Appends a triangle. Arrays that are `None` must stay `None` (`uv`/`color` ignored).
    #[allow(clippy::too_many_arguments)]
    pub fn push_triangle(
        &mut self,
        positions: [[f32; 3]; 3],
        normals: [[f32; 3]; 3],
        uvs: [[f32; 2]; 3],
        colors: [[f32; 4]; 3],
        polygon: u32,
        object: u32,
        material: u32,
    ) {
        self.positions.extend(positions);
        self.normals.extend(normals);
        if let Some(all) = &mut self.uvs {
            all.extend(uvs);
        }
        if let Some(all) = &mut self.colors {
            all.extend(colors);
        }
        self.triangle_polygon.push(polygon);
        self.triangle_object.push(object);
        self.triangle_material.push(material);
    }
}
