//! Loading of the WGSL shaders stored in the repository's `shaders/` directory.
//!
//! Shaders are named by their path relative to `shaders/`, without extension
//! (`"gpu/fill"` is `shaders/gpu/fill.wgsl`). They are embedded in the binary at build
//! time; with the `hot-reload` feature they are read from disk on every load instead.
//! This is the single entry point for shader sources, so a future `#import`
//! preprocessor only needs to be added here.

use std::borrow::Cow;
use std::path::PathBuf;

use crate::GpuError;

include!(concat!(env!("OUT_DIR"), "/embedded_shaders.rs"));

/// Returns the `shaders/` directory of the source tree.
///
/// Only meaningful on a machine that has the sources (development, tests, hot reload).
pub fn source_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../shaders")
}

/// Names of every shader known at build time, sorted.
pub fn names() -> impl Iterator<Item = &'static str> {
    EMBEDDED_SHADERS.iter().map(|(name, _)| *name)
}

/// Returns the WGSL source of the shader `name`.
pub fn load(name: &str) -> Result<Cow<'static, str>, GpuError> {
    if cfg!(feature = "hot-reload") {
        load_from_disk(name).map(Cow::Owned)
    } else {
        EMBEDDED_SHADERS
            .iter()
            .find(|(embedded, _)| *embedded == name)
            .map(|(_, source)| Cow::Borrowed(*source))
            .ok_or_else(|| GpuError::ShaderNotFound(name.to_owned()))
    }
}

fn load_from_disk(name: &str) -> Result<String, GpuError> {
    let path = source_dir().join(format!("{name}.wgsl"));
    std::fs::read_to_string(&path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            GpuError::ShaderNotFound(name.to_owned())
        } else {
            GpuError::ShaderIo {
                name: name.to_owned(),
                source,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_shaders_include_fill() {
        assert!(names().any(|name| name == "gpu/fill"));
        assert!(load("gpu/fill").unwrap().contains("textureStore"));
    }

    #[test]
    fn unknown_shader_is_an_error() {
        assert!(matches!(
            load("does/not/exist"),
            Err(GpuError::ShaderNotFound(_))
        ));
    }
}
