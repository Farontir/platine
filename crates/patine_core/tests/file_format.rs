//! Project files: round trips, the versioned fixtures and damaged files.

mod common;

use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::Arc;

use common::ops::{command, op};
use common::{rich_project, rich_project_v2};
use patine_core::file::{self, FORMAT_VERSION, FileError};
use patine_core::layer::{FillSource, LayerKind};
use patine_core::project::{Project, ValidationError};
use patine_core::resource::ResourceKind;
use proptest::prelude::*;
use zip::write::SimpleFileOptions;

/// Writes then reads `project` in memory.
fn round_trip(project: &Project) -> Project {
    let bytes = file::write(project, Cursor::new(Vec::new()), Some(b"png"))
        .expect("write")
        .into_inner();
    file::read(Cursor::new(bytes)).expect("read")
}

/// The document as postcard bytes: equal bytes mean every float is equal to the bit,
/// which `PartialEq` does not check (it equates 0.0 and -0.0).
fn bits(project: &Project) -> Vec<u8> {
    postcard::to_allocvec(project).expect("the document serializes")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("patine-file-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir.join(name)
}

/// A zip archive containing `entries`.
fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(*name, SimpleFileOptions::default())
            .expect("in-memory zip entry");
        zip.write_all(bytes).expect("in-memory write");
    }
    zip.finish().expect("in-memory zip").into_inner()
}

/// The entries of a project archive, with `edit` applied to them.
fn edited(project: &Project, edit: impl FnOnce(&mut Vec<(String, Vec<u8>)>)) -> Vec<u8> {
    let bytes = file::write(project, Cursor::new(Vec::new()), None)
        .expect("write")
        .into_inner();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).expect("a project is a zip");
    let mut entries = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).expect("entry");
        let mut data = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut data).expect("entry data");
        let name = entry.name().expect("entry name").into_owned();
        entries.push((name, data));
    }
    edit(&mut entries);
    let refs: Vec<(&str, &[u8])> = entries
        .iter()
        .map(|(name, data)| (name.as_str(), data.as_slice()))
        .collect();
    archive(&refs)
}

#[test]
fn every_kind_of_node_round_trips() {
    let project = rich_project();
    let read = round_trip(&project);
    assert_eq!(read, project);
    assert_eq!(bits(&read), bits(&project));
    // Shared data comes back too.
    let LayerKind::Folder { children } = &read.texture_sets[0].stack.layers[0].kind else {
        panic!("the first layer is a folder");
    };
    let LayerKind::Paint { strokes } = &children[0].kind else {
        panic!("the folder holds the painted layer");
    };
    assert_eq!(strokes.strokes[0].dabs.len(), 50);
    assert_eq!(read.mesh.triangle_count(), 2);
}

#[test]
fn the_manifest_is_readable_text_and_holds_no_pixels() {
    let bytes = file::write(&rich_project(), Cursor::new(Vec::new()), None)
        .unwrap()
        .into_inner();
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut manifest = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("manifest.ron").unwrap(), &mut manifest)
        .unwrap();
    assert!(manifest.contains(&format!("format_version: {FORMAT_VERSION}")));
    assert!(manifest.contains("iron_rust"));
    assert!(!manifest.contains("dabs"), "strokes are stored apart");
    assert!(!manifest.contains("positions"), "the mesh is stored apart");
    let names: Vec<String> = zip
        .file_names()
        .map(|name| name.expect("entry name").into_owned())
        .collect();
    for expected in ["meshes/mesh.bin", "strokes/2.bin", "strokes/6.bin"] {
        assert!(
            names.iter().any(|n| n == expected),
            "{expected} in {names:?}"
        );
    }
    assert_eq!(
        names.iter().filter(|n| n.starts_with("resources/")).count(),
        2
    );
}

/// Fixture of format version 1 (written by M3 to M8), kept forever: newer builds must
/// keep reading it, and its strokes and texture sets come out migrated to the current
/// types exactly as [`rich_project`] builds them.
#[test]
fn version_1_fixture_still_loads() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/projects/v1_rich.patine");
    let read = file::load(&path).expect("the v1 fixture loads");
    assert_eq!(read, rich_project());
}

/// Fixture of format version 2. `PATINE_UPDATE_FIXTURES=1` writes it again, only while
/// version 2 is current.
#[test]
fn version_2_fixture_still_loads() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/projects/v2_rich.patine");
    if std::env::var_os("PATINE_UPDATE_FIXTURES").is_some() {
        assert_eq!(FORMAT_VERSION, 2, "old fixtures are never rewritten");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        file::save(&rich_project_v2(), &path, None).unwrap();
    }
    let read = file::load(&path).expect("the v2 fixture loads");
    assert_eq!(read, rich_project_v2());
    assert_ne!(read, rich_project(), "version 2 adds to the rich project");
}

#[test]
fn save_replaces_the_file_atomically() {
    let path = scratch("atomic.patine");
    let mut project = rich_project();
    file::save(&project, &path, Some(b"thumb")).unwrap();
    project.texture_sets[0].name = "second".into();
    file::save(&project, &path, None).unwrap();
    assert_eq!(file::load(&path).unwrap(), project);
    assert_eq!(file::load_thumbnail(&path).unwrap(), None);
    let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    // A save that cannot complete leaves no file behind.
    let missing_dir = scratch("missing").join("dir").join("a.patine");
    assert!(matches!(
        file::save(&project, &missing_dir, None),
        Err(FileError::Io(_))
    ));
    assert!(!missing_dir.exists());
}

#[test]
fn damaged_files_are_refused_with_a_reason() {
    let project = rich_project();
    let valid = file::write(&project, Cursor::new(Vec::new()), None)
        .unwrap()
        .into_inner();

    let not_zip = file::read(Cursor::new(b"glTF binary".to_vec()));
    assert!(matches!(not_zip, Err(FileError::Archive(_))), "{not_zip:?}");

    let truncated = file::read(Cursor::new(valid[..valid.len() / 2].to_vec()));
    assert!(
        matches!(truncated, Err(FileError::Archive(_))),
        "{truncated:?}"
    );

    let other_zip = file::read(Cursor::new(archive(&[("readme.txt", b"hello")])));
    assert!(
        matches!(other_zip, Err(FileError::NotAProject)),
        "{other_zip:?}"
    );

    let no_mesh = edited(&project, |entries| {
        entries.retain(|(n, _)| n != "meshes/mesh.bin")
    });
    assert!(matches!(
        file::read(Cursor::new(no_mesh)),
        Err(FileError::MissingEntry(name)) if name == "meshes/mesh.bin"
    ));

    let bad_strokes = edited(&project, |entries| {
        for (name, data) in entries.iter_mut() {
            if name == "strokes/2.bin" {
                data.truncate(5);
            }
        }
    });
    assert!(matches!(
        file::read(Cursor::new(bad_strokes)),
        Err(FileError::Entry { entry, .. }) if entry == "strokes/2.bin"
    ));

    let tampered = edited(&project, |entries| {
        for (name, data) in entries.iter_mut() {
            if name.starts_with("resources/") {
                data.push(0);
            }
        }
    });
    assert!(matches!(
        file::read(Cursor::new(tampered)),
        Err(FileError::Invalid(ValidationError::CorruptedResource(_)))
    ));

    let newer = edited(&project, |entries| {
        for (name, data) in entries.iter_mut() {
            if name == "manifest.ron" {
                let text = String::from_utf8(data.clone()).unwrap();
                *data = text
                    .replace(
                        &format!("format_version: {FORMAT_VERSION}"),
                        "format_version: 99",
                    )
                    .into_bytes();
            }
        }
    });
    assert!(matches!(
        file::read(Cursor::new(newer)),
        Err(FileError::NewerVersion { found: 99, .. })
    ));

    let broken_manifest = edited(&project, |entries| {
        for (name, data) in entries.iter_mut() {
            if name == "manifest.ron" {
                data.truncate(data.len() / 2);
            }
        }
    });
    assert!(matches!(
        file::read(Cursor::new(broken_manifest)),
        Err(FileError::Manifest(_))
    ));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Any document reachable through commands, with arbitrary floats in a fill and in
    /// the environment, saves and reads back identical, to the bit.
    #[test]
    fn saved_documents_read_back_identical(
        ops in proptest::collection::vec(op(), 0..60),
        values in proptest::collection::vec(any::<f32>().prop_filter("finite", |v| v.is_finite()), 4),
        rotation in any::<f32>().prop_filter("finite", |v| v.is_finite()),
        bytes in proptest::collection::vec(any::<u8>(), 0..64),
    ) {
        let mut project = rich_project();
        let mut history = patine_core::history::History::default();
        for op in &ops {
            if let Some(command) = command(&project, op) {
                let _ = history.push(&mut project, command);
            }
        }
        project.environment.rotation = rotation;
        let set = &mut project.texture_sets[0];
        set.stack.visit_mut(&mut |layer| {
            if let LayerKind::Fill { source: FillSource::Uniform(map), .. } = &mut layer.kind {
                for value in map.values_mut() {
                    value.copy_from_slice(&values);
                }
            }
        });
        let (hash, resource) = patine_core::resource::Resource::new(
            "extra.bin",
            ResourceKind::Image,
            Arc::from(bytes.as_slice()),
        );
        project.resources.insert(hash, resource);
        prop_assert_eq!(project.validate(), Ok(()));

        let read = round_trip(&project);
        prop_assert_eq!(&read, &project);
        prop_assert_eq!(bits(&read), bits(&project));
    }
}
