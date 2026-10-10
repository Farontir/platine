//! Loading of the WGSL shaders stored in the repository's `shaders/` directory.
//!
//! Shaders are named by their path relative to `shaders/`, without extension
//! (`"gpu/fill"` is `shaders/gpu/fill.wgsl`). They are embedded in the binary at build
//! time; with the `hot-reload` feature they are read from disk on every load instead.
//!
//! WGSL has no include mechanism. A line `// #import render/brdf` is replaced by the
//! source of `shaders/render/brdf.wgsl`; each file is included at most once per composed
//! shader and import cycles are refused. The directive is a comment, so library files
//! remain valid WGSL on their own.

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

/// Prefix of an import line.
const IMPORT: &str = "// #import ";

/// Returns the WGSL source of the shader `name`, with its imports resolved.
pub fn load(name: &str) -> Result<Cow<'static, str>, GpuError> {
    let source = load_raw(name)?;
    if !source
        .lines()
        .any(|line| line.trim_start().starts_with(IMPORT))
    {
        return Ok(source);
    }
    compose(name, &load_raw).map(Cow::Owned)
}

/// Resolves the imports of `name`, reading files through `read`.
fn compose(
    name: &str,
    read: &dyn Fn(&str) -> Result<Cow<'static, str>, GpuError>,
) -> Result<String, GpuError> {
    fn visit(
        name: &str,
        read: &dyn Fn(&str) -> Result<Cow<'static, str>, GpuError>,
        out: &mut String,
        included: &mut Vec<String>,
        stack: &mut Vec<String>,
    ) -> Result<(), GpuError> {
        if stack.iter().any(|n| n == name) {
            stack.push(name.to_owned());
            return Err(GpuError::ShaderImportCycle(stack.join(" -> ")));
        }
        if included.iter().any(|n| n == name) {
            return Ok(());
        }
        included.push(name.to_owned());
        stack.push(name.to_owned());
        for line in read(name)?.lines() {
            match line.trim_start().strip_prefix(IMPORT) {
                Some(import) => visit(import.trim(), read, out, included, stack)?,
                None => {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        stack.pop();
        Ok(())
    }
    let mut out = String::new();
    visit(name, read, &mut out, &mut Vec::new(), &mut Vec::new())?;
    Ok(out)
}

/// Returns the source of the file `name`, without resolving imports.
fn load_raw(name: &str) -> Result<Cow<'static, str>, GpuError> {
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

    fn library(name: &str) -> Result<Cow<'static, str>, GpuError> {
        let source = match name {
            "main" => "// #import lib/a\n// #import lib/b\nfn main_fn() {}",
            "lib/a" => "// #import lib/common\nfn a() {}",
            // Indented directives are recognized too.
            "lib/b" => "  // #import lib/common\nfn b() {}",
            "lib/common" => "fn common() {}",
            "cycle/x" => "// #import cycle/y\nfn x() {}",
            "cycle/y" => "// #import cycle/x\nfn y() {}",
            _ => return Err(GpuError::ShaderNotFound(name.to_owned())),
        };
        Ok(Cow::Borrowed(source))
    }

    #[test]
    fn imports_are_resolved_once_in_dependency_order() {
        let composed = compose("main", &library).unwrap();
        assert_eq!(composed.matches("fn common()").count(), 1);
        let order: Vec<usize> = ["fn common", "fn a", "fn b", "fn main_fn"]
            .iter()
            .map(|f| composed.find(f).unwrap())
            .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{composed}");
        assert!(!composed.contains("#import"));
    }

    #[test]
    fn import_cycles_are_refused() {
        match compose("cycle/x", &library) {
            Err(GpuError::ShaderImportCycle(chain)) => {
                assert_eq!(chain, "cycle/x -> cycle/y -> cycle/x")
            }
            other => panic!("expected a cycle error, got {other:?}"),
        }
    }

    #[test]
    fn missing_import_is_reported() {
        let read = |name: &str| match name {
            "main" => Ok(Cow::Borrowed("// #import nowhere")),
            _ => Err(GpuError::ShaderNotFound(name.to_owned())),
        };
        assert!(
            matches!(compose("main", &read), Err(GpuError::ShaderNotFound(n)) if n == "nowhere")
        );
    }
}
