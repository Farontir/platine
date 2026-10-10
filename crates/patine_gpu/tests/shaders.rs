//! Every WGSL file under `shaders/` must parse and validate with naga.

use std::path::{Path, PathBuf};

fn wgsl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("shaders directory is readable") {
        let path = entry.expect("directory entry is readable").path();
        if path.is_dir() {
            wgsl_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "wgsl") {
            out.push(path);
        }
    }
}

#[test]
fn all_shaders_validate_with_naga() {
    let mut files = Vec::new();
    wgsl_files(&patine_gpu::shaders::source_dir(), &mut files);
    assert!(!files.is_empty(), "no shader found under shaders/");

    let mut failures = Vec::new();
    for path in &files {
        let source = std::fs::read_to_string(path).unwrap();
        let module = match naga::front::wgsl::parse_str(&source) {
            Ok(module) => module,
            Err(error) => {
                failures.push(error.emit_to_string_with_path(&source, &path.to_string_lossy()));
                continue;
            }
        };
        // Default capabilities: what every WebGPU device supports. A shader that needs
        // more must say so here explicitly.
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        );
        if let Err(error) = validator.validate(&module) {
            failures.push(error.emit_to_string_with_path(&source, &path.to_string_lossy()));
        }
    }
    assert!(
        failures.is_empty(),
        "invalid shaders:\n{}",
        failures.join("\n")
    );
}

#[test]
fn every_shader_file_is_embedded() {
    let mut files = Vec::new();
    wgsl_files(&patine_gpu::shaders::source_dir(), &mut files);
    let embedded: Vec<_> = patine_gpu::shaders::names().collect();
    assert_eq!(files.len(), embedded.len(), "embedded: {embedded:?}");
}
