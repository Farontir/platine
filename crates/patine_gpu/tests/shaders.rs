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

/// Shaders allowed to use capabilities beyond the WebGPU baseline. The code that creates
/// their pipelines must check the matching device feature first.
const EXTRA_CAPABILITIES: &[(&str, naga::valid::Capabilities)] = &[
    // Features::PRIMITIVE_INDEX, checked by patine_render before building the pipeline.
    (
        "render/picking_indexed",
        naga::valid::Capabilities::PRIMITIVE_INDEX,
    ),
];

#[test]
fn all_shaders_validate_with_naga() {
    let mut failures = Vec::new();
    let mut count = 0;
    for name in patine_gpu::shaders::names() {
        count += 1;
        // Imports resolved: this is the source the device compiles.
        let source = match patine_gpu::shaders::load(name) {
            Ok(source) => source,
            Err(error) => {
                failures.push(format!("{name}: {error}"));
                continue;
            }
        };
        let module = match naga::front::wgsl::parse_str(&source) {
            Ok(module) => module,
            Err(error) => {
                failures.push(error.emit_to_string_with_path(&source, name));
                continue;
            }
        };
        let extra = EXTRA_CAPABILITIES
            .iter()
            .filter(|(shader, _)| *shader == name)
            .fold(naga::valid::Capabilities::empty(), |caps, (_, extra)| {
                caps | *extra
            });
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default() | extra,
        );
        if let Err(error) = validator.validate(&module) {
            failures.push(error.emit_to_string_with_path(&source, name));
        }
    }
    assert!(count > 0, "no shader found under shaders/");
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
