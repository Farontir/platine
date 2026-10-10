//! Embeds every `.wgsl` file of the repository's `shaders/` directory into the binary.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn collect(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) {
    let entries = std::fs::read_dir(dir).expect("shaders directory must be readable");
    for entry in entries {
        let path = entry
            .expect("shaders directory entry must be readable")
            .path();
        if path.is_dir() {
            collect(&path, root, out);
        } else if path.extension().is_some_and(|ext| ext == "wgsl") {
            let name = path
                .strip_prefix(root)
                .expect("shader path is inside the shaders root")
                .with_extension("")
                .to_string_lossy()
                .replace('\\', "/");
            out.push((name, path));
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("cargo always sets CARGO_MANIFEST_DIR"),
    );
    let root = manifest_dir
        .join("../../shaders")
        .canonicalize()
        .expect("the shaders/ directory exists at the workspace root");
    println!("cargo:rerun-if-changed={}", root.display());

    let mut shaders = Vec::new();
    collect(&root, &root, &mut shaders);
    shaders.sort();

    let mut code = String::from("/// Shaders embedded at build time: (name, WGSL source).\n");
    code.push_str("pub(crate) static EMBEDDED_SHADERS: &[(&str, &str)] = &[\n");
    for (name, path) in &shaders {
        writeln!(code, "    ({name:?}, include_str!({:?})),", path.display())
            .expect("writing to a String cannot fail");
    }
    code.push_str("];\n");

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo always sets OUT_DIR"));
    std::fs::write(out.join("embedded_shaders.rs"), code).expect("OUT_DIR must be writable");
}
