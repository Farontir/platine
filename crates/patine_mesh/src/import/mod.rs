//! Mesh file import: glTF (`.gltf`, `.glb`) through `gltf`, FBX and OBJ through ufbx.

mod gltf_import;
mod ufbx_import;

use std::path::Path;
use std::time::{Duration, Instant};

use glam::{DMat3, DMat4, DVec3, Vec3};
use patine_core::mesh::MeshAsset;

use crate::raw::{NormalSource, RawMesh, RawObject};
use crate::{BuildOptions, BuildReport, MeshError, assemble};

/// Supported mesh formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshFormat {
    /// glTF 2.0, text or binary.
    Gltf,
    /// Autodesk FBX, binary or ASCII.
    Fbx,
    /// Wavefront OBJ (with its `.mtl`).
    Obj,
}

impl MeshFormat {
    /// File extensions handled by the importer, lowercase.
    pub const EXTENSIONS: [&'static str; 4] = ["glb", "gltf", "fbx", "obj"];

    /// Format of `path`, from its extension.
    pub fn from_path(path: &Path) -> Result<Self, MeshError> {
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "glb" | "gltf" => Ok(MeshFormat::Gltf),
            "fbx" => Ok(MeshFormat::Fbx),
            "obj" => Ok(MeshFormat::Obj),
            _ => Err(MeshError::UnsupportedFormat { extension }),
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            MeshFormat::Gltf => "glTF",
            MeshFormat::Fbx => "FBX",
            MeshFormat::Obj => "OBJ",
        }
    }
}

/// Result of a successful import.
#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    /// The mesh.
    pub mesh: MeshAsset,
    /// What happened.
    pub report: ImportReport,
}

/// Report of an import, logged in the console.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportReport {
    /// Source format.
    pub format: MeshFormat,
    /// Time spent reading the file.
    pub read_time: Duration,
    /// Notes from the reader (skipped primitives, unsupported features).
    pub warnings: Vec<String>,
    /// Report of the mesh assembly.
    pub build: BuildReport,
}

/// Imports the mesh at `path`.
pub fn import(path: &Path, options: BuildOptions) -> Result<Imported, MeshError> {
    let format = MeshFormat::from_path(path)?;
    std::fs::metadata(path).map_err(|source| MeshError::Io {
        path: path.to_owned(),
        source,
    })?;
    let start = Instant::now();
    let mut warnings = Vec::new();
    let raw = match format {
        MeshFormat::Gltf => gltf_import::read(path, &mut warnings)?,
        MeshFormat::Fbx | MeshFormat::Obj => ufbx_import::read(path, format, &mut warnings)?,
    };
    let read_time = start.elapsed();
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let (mesh, build) = assemble(&name, raw, options)?;
    Ok(Imported {
        mesh,
        report: ImportReport {
            format,
            read_time,
            warnings,
            build,
        },
    })
}

/// Collects triangles from the readers into a [`RawMesh`].
///
/// UV and color arrays are kept for every triangle while reading (zeros or white where
/// an object has none) and dropped at the end if no object had any.
pub(crate) struct RawCollector {
    raw: RawMesh,
    any_uvs: bool,
    any_colors: bool,
    polygon: u32,
}

/// Vertex data of one source object, indexed by corner.
pub(crate) struct ObjectData<'a> {
    pub name: String,
    pub transform: DMat4,
    pub material_of_triangle: &'a dyn Fn(usize) -> u32,
    pub normals: Option<&'a dyn Fn(usize) -> [f32; 3]>,
    pub normal_source_if_missing: NormalSource,
    pub uvs: Option<&'a dyn Fn(usize) -> [f32; 2]>,
    pub colors: Option<&'a dyn Fn(usize) -> [f32; 4]>,
}

impl RawCollector {
    pub fn new() -> Self {
        Self {
            raw: RawMesh {
                uvs: Some(Vec::new()),
                colors: Some(Vec::new()),
                ..RawMesh::default()
            },
            any_uvs: false,
            any_colors: false,
            polygon: 0,
        }
    }

    /// Registers a material and returns its index.
    pub fn add_material(&mut self, name: String) -> u32 {
        self.raw.materials.push(name);
        (self.raw.materials.len() - 1) as u32
    }

    /// Adds an object whose triangles are `triangles` (corner indices into the
    /// accessors of `data`), with `polygon_of_triangle` grouping triangles into source
    /// polygons (consecutive triangles with the same value form one polygon).
    pub fn add_object(
        &mut self,
        data: &ObjectData<'_>,
        triangles: &[[usize; 3]],
        polygon_of_triangle: &dyn Fn(usize) -> u32,
        position: &dyn Fn(usize) -> DVec3,
    ) {
        let object = self.raw.objects.len() as u32;
        self.raw.objects.push(RawObject {
            name: data.name.clone(),
            normals: if data.normals.is_some() {
                NormalSource::Provided
            } else {
                data.normal_source_if_missing
            },
            has_uvs: data.uvs.is_some(),
        });
        self.any_uvs |= data.uvs.is_some();
        self.any_colors |= data.colors.is_some();

        let linear = DMat3::from_mat4(data.transform);
        let flip = linear.determinant() < 0.0;
        let normal_matrix = linear.inverse().transpose();
        let mut last_polygon = None;
        for (index, corners) in triangles.iter().enumerate() {
            // Mirroring transforms turn the triangle inside out: restore the winding.
            let corners = if flip {
                [corners[0], corners[2], corners[1]]
            } else {
                *corners
            };
            let source_polygon = polygon_of_triangle(index);
            if last_polygon != Some(source_polygon) {
                last_polygon = Some(source_polygon);
                self.polygon += 1;
            }
            let p = corners.map(|c| {
                data.transform
                    .transform_point3(position(c))
                    .as_vec3()
                    .to_array()
            });
            let n = corners.map(|c| match data.normals {
                Some(normal) => (normal_matrix * DVec3::from(Vec3::from(normal(c))))
                    .as_vec3()
                    .to_array(),
                None => [0.0; 3],
            });
            let uv = corners.map(|c| data.uvs.map_or([0.0; 2], |uv| uv(c)));
            let color = corners.map(|c| data.colors.map_or([1.0; 4], |color| color(c)));
            self.raw.push_triangle(
                p,
                n,
                uv,
                color,
                self.polygon - 1,
                object,
                (data.material_of_triangle)(index),
            );
        }
    }

    pub fn finish(mut self) -> RawMesh {
        if !self.any_uvs {
            self.raw.uvs = None;
        }
        if !self.any_colors {
            self.raw.colors = None;
        }
        self.raw
    }
}

/// Converts an sRGB-encoded color component to linear.
pub(crate) fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_from_extensions() {
        assert_eq!(
            MeshFormat::from_path(Path::new("a/b.GLB")).unwrap(),
            MeshFormat::Gltf
        );
        assert_eq!(
            MeshFormat::from_path(Path::new("b.gltf")).unwrap(),
            MeshFormat::Gltf
        );
        assert_eq!(
            MeshFormat::from_path(Path::new("c.Fbx")).unwrap(),
            MeshFormat::Fbx
        );
        assert_eq!(
            MeshFormat::from_path(Path::new("d.obj")).unwrap(),
            MeshFormat::Obj
        );
        assert!(matches!(
            MeshFormat::from_path(Path::new("e.blend")),
            Err(MeshError::UnsupportedFormat { extension }) if extension == "blend"
        ));
    }

    #[test]
    fn missing_file_is_an_io_error() {
        assert!(matches!(
            import(Path::new("does/not/exist.glb"), BuildOptions::default()),
            Err(MeshError::Io { .. })
        ));
    }

    #[test]
    fn srgb_conversion_endpoints() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < 1e-6);
        assert!((srgb_to_linear(0.5) - 0.214).abs() < 1e-3);
    }
}
