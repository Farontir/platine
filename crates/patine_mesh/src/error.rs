use std::path::PathBuf;

use patine_core::mesh::MeshValidationError;
use thiserror::Error;

/// Why a mesh could not be imported.
///
/// Every variant has a user-facing translation; see [`MeshError::translation_key`].
#[derive(Debug, Error)]
pub enum MeshError {
    /// The file could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// File path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The file extension is not a supported mesh format.
    #[error("unsupported mesh format `{extension}`")]
    UnsupportedFormat {
        /// Extension found, possibly empty.
        extension: String,
    },
    /// The file is malformed.
    #[error("invalid {format} file: {message}")]
    Parse {
        /// Format name.
        format: &'static str,
        /// Parser message.
        message: String,
    },
    /// The file contains no triangle.
    #[error("the file contains no triangle geometry")]
    NoGeometry,
    /// Some objects have no UVs, which a project mesh requires.
    #[error("objects without UV coordinates: {}", objects.join(", "))]
    MissingUvs {
        /// Names of the objects without UVs.
        objects: Vec<String>,
    },
    /// The mesh has more vertices than 32-bit indices can address.
    #[error("the mesh has too many vertices ({0})")]
    TooManyVertices(usize),
    /// Tangent generation failed.
    #[error("tangent generation failed: {0}")]
    Tangents(String),
    /// The importer produced an inconsistent mesh. This is a bug.
    #[error("internal error, invalid mesh: {0}")]
    Invalid(#[from] MeshValidationError),
}

impl MeshError {
    /// Translation key of the user-facing message.
    ///
    /// The message receives the arguments `path`, `extension`, `format`, `message`,
    /// `objects` and `count` when they apply.
    pub fn translation_key(&self) -> &'static str {
        match self {
            MeshError::Io { .. } => "error-mesh-io",
            MeshError::UnsupportedFormat { .. } => "error-mesh-unsupported-format",
            MeshError::Parse { .. } => "error-mesh-parse",
            MeshError::NoGeometry => "error-mesh-no-geometry",
            MeshError::MissingUvs { .. } => "error-mesh-missing-uvs",
            MeshError::TooManyVertices(_) => "error-mesh-too-many-vertices",
            MeshError::Tangents(_) => "error-mesh-tangents",
            MeshError::Invalid(_) => "error-mesh-internal",
        }
    }

    /// Every translation key, for tests.
    pub const TRANSLATION_KEYS: [&'static str; 8] = [
        "error-mesh-io",
        "error-mesh-unsupported-format",
        "error-mesh-parse",
        "error-mesh-no-geometry",
        "error-mesh-missing-uvs",
        "error-mesh-too-many-vertices",
        "error-mesh-tangents",
        "error-mesh-internal",
    ];
}
