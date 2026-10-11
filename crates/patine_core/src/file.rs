//! Project files (`.patine`).
//!
//! A project file is a zip archive:
//!
//! | Entry | Contents |
//! | --- | --- |
//! | `manifest.ron` | format version, application version, document without pixels |
//! | `meshes/mesh.bin` | the processed [`MeshAsset`], postcard + zstd |
//! | `strokes/<id>.bin` | the [`StrokeLog`] of each paint layer or mask effect, postcard + zstd |
//! | `resources/<sha256>` | files imported by the user |
//! | `thumb.png` | optional thumbnail |
//!
//! Entries are stored without zip compression: binaries are already compressed, and the
//! manifest stays readable with any unzip tool.
//!
//! # Format versions
//!
//! [`FORMAT_VERSION`] is written in the manifest. Reading a file first reads that number
//! alone, then the manifest of that version, which is upgraded step by step to the
//! current one. Any change to the serialized types must increment [`FORMAT_VERSION`],
//! freeze the previous types in a `vN` module with their upgrade function, and add a
//! fixture `tests/fixtures/projects/vN_*.patine` that the tests keep loading.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::layer::LayerKind;
use crate::mask::MaskEffect;
use crate::mesh::MeshAsset;
use crate::project::{Project, ValidationError};
use crate::stroke::StrokeLog;

/// Version of the format written by this build.
pub const FORMAT_VERSION: u32 = 1;

/// File extension of projects, without the dot.
pub const EXTENSION: &str = "patine";

const MANIFEST: &str = "manifest.ron";
const MESH: &str = "meshes/mesh.bin";
const THUMBNAIL: &str = "thumb.png";
/// zstd level: fast enough for interactive saves, about as small as level 9 on meshes.
const ZSTD_LEVEL: i32 = 3;

/// Why a project file cannot be written or read.
#[derive(Debug, Error)]
pub enum FileError {
    /// The file system refused.
    #[error("{0}")]
    Io(#[from] io::Error),
    /// The file is not a zip archive, or the archive is damaged.
    #[error("damaged archive: {0}")]
    Archive(String),
    /// The archive has no manifest: it is not a project.
    #[error("not a project file")]
    NotAProject,
    /// An entry the manifest needs is missing.
    #[error("missing entry `{0}`")]
    MissingEntry(String),
    /// The manifest cannot be parsed.
    #[error("invalid manifest: {0}")]
    Manifest(String),
    /// The file was written by a newer version of the application.
    #[error("format version {found} is newer than this application (version {supported})")]
    NewerVersion {
        /// Version in the file.
        found: u32,
        /// Newest version this build reads.
        supported: u32,
    },
    /// The format version is not one this build knows.
    #[error("unsupported format version {0}")]
    UnsupportedVersion(u32),
    /// A binary entry cannot be decoded.
    #[error("invalid entry `{entry}`: {message}")]
    Entry {
        /// Entry name.
        entry: String,
        /// Decoder message.
        message: String,
    },
    /// The document breaks an invariant.
    #[error("invalid document: {0}")]
    Invalid(#[from] ValidationError),
    /// The document cannot be serialized (a path that is not valid Unicode, for example).
    #[error("cannot serialize the document: {0}")]
    Serialize(String),
}

impl From<zip::result::ZipError> for FileError {
    fn from(error: zip::result::ZipError) -> Self {
        match error {
            zip::result::ZipError::Io(error) => FileError::Io(error),
            other => FileError::Archive(other.to_string()),
        }
    }
}

/// Manifest of the current version, as written.
#[derive(Serialize)]
#[serde(rename = "Manifest")]
struct ManifestRef<'a> {
    format_version: u32,
    app_version: &'a str,
    project: &'a Project,
}

/// Manifest of the current version, as read.
#[derive(Deserialize)]
#[serde(rename = "Manifest")]
struct Manifest {
    format_version: u32,
    #[allow(dead_code)] // Kept for diagnostics of files written by other versions.
    app_version: String,
    project: Project,
}

/// Just the version number, read before the rest of the manifest.
#[derive(Deserialize)]
#[serde(rename = "Manifest")]
struct Header {
    format_version: u32,
}

fn stroke_entry(id: u64) -> String {
    format!("strokes/{id}.bin")
}

fn resource_entry(hash: &crate::resource::ResourceHash) -> String {
    format!("resources/{hash}")
}

/// Every stroke log of the project, by the identifier of its layer or mask effect.
fn stroke_logs(project: &Project) -> Vec<(u64, Arc<StrokeLog>)> {
    let mut logs = Vec::new();
    project.visit_layers(&mut |_, layer| {
        if let LayerKind::Paint { strokes } = &layer.kind {
            logs.push((layer.id.0, strokes.clone()));
        }
        for entry in layer.mask.iter().flat_map(|mask| &mask.effects) {
            if let MaskEffect::Paint { strokes } = &entry.effect {
                logs.push((entry.id.0, strokes.clone()));
            }
        }
    });
    logs
}

/// Calls `f` with the identifier and the stroke log slot of every paint node.
fn attach_strokes(
    project: &mut Project,
    f: &mut impl FnMut(u64, &mut Arc<StrokeLog>) -> Result<(), FileError>,
) -> Result<(), FileError> {
    let mut result = Ok(());
    for set in &mut project.texture_sets {
        set.stack.visit_mut(&mut |layer| {
            if result.is_err() {
                return;
            }
            if let LayerKind::Paint { strokes } = &mut layer.kind {
                result = f(layer.id.0, strokes);
            }
            for entry in layer.mask.iter_mut().flat_map(|mask| &mut mask.effects) {
                if let MaskEffect::Paint { strokes } = &mut entry.effect
                    && result.is_ok()
                {
                    result = f(entry.id.0, strokes);
                }
            }
        });
    }
    result
}

fn encode<T: Serialize>(value: &T, entry: &str) -> Result<Vec<u8>, FileError> {
    let bytes = postcard::to_allocvec(value).map_err(|error| FileError::Entry {
        entry: entry.to_owned(),
        message: error.to_string(),
    })?;
    Ok(zstd::encode_all(bytes.as_slice(), ZSTD_LEVEL)?)
}

fn decode<T: DeserializeOwned>(bytes: &[u8], entry: &str) -> Result<T, FileError> {
    let invalid = |message: String| FileError::Entry {
        entry: entry.to_owned(),
        message,
    };
    let raw = zstd::decode_all(bytes).map_err(|error| invalid(error.to_string()))?;
    postcard::from_bytes(&raw).map_err(|error| invalid(error.to_string()))
}

/// Writes `project` as a project file into `writer`, with an optional PNG thumbnail.
pub fn write<W: Write + Seek>(
    project: &Project,
    writer: W,
    thumbnail: Option<&[u8]>,
) -> Result<W, FileError> {
    let manifest = ron::ser::to_string_pretty(
        &ManifestRef {
            format_version: FORMAT_VERSION,
            app_version: env!("CARGO_PKG_VERSION"),
            project,
        },
        ron::ser::PrettyConfig::default(),
    )
    .map_err(|error| FileError::Serialize(error.to_string()))?;

    let mut zip = ZipWriter::new(writer);
    let mut put = |name: &str, bytes: &[u8]| -> Result<(), FileError> {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(bytes.len() as u64 >= u64::from(u32::MAX));
        zip.start_file(name, options)?;
        zip.write_all(bytes)?;
        Ok(())
    };
    put(MANIFEST, manifest.as_bytes())?;
    put(MESH, &encode(project.mesh.as_ref(), MESH)?)?;
    for (id, strokes) in stroke_logs(project) {
        let entry = stroke_entry(id);
        put(&entry, &encode(strokes.as_ref(), &entry)?)?;
    }
    for (hash, resource) in &project.resources {
        put(&resource_entry(hash), &resource.bytes)?;
    }
    if let Some(png) = thumbnail {
        put(THUMBNAIL, png)?;
    }
    Ok(zip.finish()?)
}

/// Saves `project` to `path`. The file is written next to its destination, then renamed
/// over it: a failed save never damages the previous file.
pub fn save(project: &Project, path: &Path, thumbnail: Option<&[u8]>) -> Result<(), FileError> {
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
    let mut temporary_name = file_name.to_os_string();
    temporary_name.push(format!(".tmp{}", std::process::id()));
    let temporary = path.with_file_name(temporary_name);
    let result = (|| {
        let file = File::create(&temporary)?;
        let mut writer = write(project, BufWriter::new(file), thumbnail)?;
        writer.flush()?;
        let file = writer.into_inner().map_err(|error| error.into_error())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn read_entry<R: Read + Seek>(zip: &mut ZipArchive<R>, name: &str) -> Result<Vec<u8>, FileError> {
    let mut entry = match zip.by_name(name) {
        Ok(entry) => entry,
        Err(zip::result::ZipError::FileNotFound) => {
            return Err(FileError::MissingEntry(name.to_owned()));
        }
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Parses a manifest of any supported version into the current document.
fn parse_manifest(text: &str) -> Result<Project, FileError> {
    let manifest_error = |error: ron::error::SpannedError| FileError::Manifest(error.to_string());
    let header: Header = ron::from_str(text).map_err(manifest_error)?;
    match header.format_version {
        FORMAT_VERSION => {
            let manifest: Manifest = ron::from_str(text).map_err(manifest_error)?;
            debug_assert_eq!(manifest.format_version, FORMAT_VERSION);
            Ok(manifest.project)
        }
        // Older versions go here, e.g. `1 => Ok(v1::parse(text)?.upgrade()),` once the
        // format changes.
        found if found > FORMAT_VERSION => Err(FileError::NewerVersion {
            found,
            supported: FORMAT_VERSION,
        }),
        found => Err(FileError::UnsupportedVersion(found)),
    }
}

/// Reads a project file from `reader`, and validates the document.
pub fn read<R: Read + Seek>(reader: R) -> Result<Project, FileError> {
    let mut zip = ZipArchive::new(reader)?;
    let manifest = match read_entry(&mut zip, MANIFEST) {
        Err(FileError::MissingEntry(_)) => return Err(FileError::NotAProject),
        other => other?,
    };
    let text = String::from_utf8(manifest)
        .map_err(|_| FileError::Manifest("not valid UTF-8".to_owned()))?;
    let mut project = parse_manifest(&text)?;

    let mesh: MeshAsset = decode(&read_entry(&mut zip, MESH)?, MESH)?;
    project.mesh = Arc::new(mesh);
    attach_strokes(&mut project, &mut |id, slot| {
        let entry = stroke_entry(id);
        let log: StrokeLog = decode(&read_entry(&mut zip, &entry)?, &entry)?;
        *slot = Arc::new(log);
        Ok(())
    })?;
    for (hash, resource) in &mut project.resources {
        resource.bytes = Arc::from(read_entry(&mut zip, &resource_entry(hash))?);
    }
    project.validate()?;
    Ok(project)
}

/// Opens the project file at `path`.
pub fn load(path: &Path) -> Result<Project, FileError> {
    read(BufReader::new(File::open(path)?))
}

/// The thumbnail of the project file at `path`, if it has one.
pub fn load_thumbnail(path: &Path) -> Result<Option<Vec<u8>>, FileError> {
    let mut zip = ZipArchive::new(BufReader::new(File::open(path)?))?;
    match read_entry(&mut zip, THUMBNAIL) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(FileError::MissingEntry(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// `path` with the project extension, unless it already has it.
pub fn with_extension(path: impl Into<PathBuf>) -> PathBuf {
    let path = path.into();
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(EXTENSION))
    {
        path
    } else {
        let mut name = path.clone().into_os_string();
        name.push(".");
        name.push(EXTENSION);
        PathBuf::from(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_is_added_once() {
        assert_eq!(with_extension("a/b"), PathBuf::from("a/b.patine"));
        assert_eq!(with_extension("a/b.PATINE"), PathBuf::from("a/b.PATINE"));
        assert_eq!(with_extension("a/b.glb"), PathBuf::from("a/b.glb.patine"));
    }

    #[test]
    fn versions_are_checked_before_the_document() {
        let newer = "(format_version: 7, app_version: \"9.0\", project: ())";
        assert!(matches!(
            parse_manifest(newer),
            Err(FileError::NewerVersion {
                found: 7,
                supported: FORMAT_VERSION
            })
        ));
        let older = "(format_version: 0)";
        assert!(matches!(
            parse_manifest(older),
            Err(FileError::UnsupportedVersion(0))
        ));
        assert!(matches!(
            parse_manifest("not ron"),
            Err(FileError::Manifest(_))
        ));
    }
}
