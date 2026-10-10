//! Test tooling shared by every GPU crate (feature `testing`).
//!
//! - [`gpu_test!`](crate::gpu_test) declares a test that receives a headless [`GpuContext`].
//! - [`golden`] compares rendered images with the references in `tests/golden/`.
//!
//! Tests run on the software adapter by default (WARP on Windows, lavapipe on Linux), so
//! the golden images are the same on a developer machine and in CI. Set
//! `PATINE_TEST_ADAPTER=hardware` to run them on the real GPU, with wider tolerances.

use std::path::PathBuf;

use crate::{AdapterChoice, GpuContext};

pub mod golden;

/// Environment variable selecting the test adapter: `software` (default) or `hardware`.
pub const ADAPTER_ENV: &str = "PATINE_TEST_ADAPTER";

/// Adapter used by GPU tests, read from [`ADAPTER_ENV`].
///
/// # Panics
/// If the variable holds an unknown value: a typo must not silently change the adapter.
pub fn adapter_choice() -> AdapterChoice {
    match std::env::var(ADAPTER_ENV).as_deref() {
        Err(_) | Ok("") | Ok("software") => AdapterChoice::Software,
        Ok("hardware") => AdapterChoice::Hardware,
        Ok(other) => panic!("{ADAPTER_ENV}={other:?}: expected `software` or `hardware`"),
    }
}

/// Creates the GPU context of a test.
///
/// # Panics
/// If no adapter is available. GPU tests never skip silently.
pub fn context() -> GpuContext {
    let choice = adapter_choice();
    GpuContext::new_headless(choice).unwrap_or_else(|error| {
        panic!(
            "GPU test needs a {choice}: {error}\n\
             On Linux, install lavapipe (mesa-vulkan-drivers). \
             Set {ADAPTER_ENV}=hardware to use the real GPU."
        )
    })
}

/// Root of the source tree (the directory holding the workspace `Cargo.toml`).
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("patine_gpu lives in <root>/crates/patine_gpu")
        .to_path_buf()
}

/// Declares a `#[test]` that receives a headless [`GpuContext`] on the test adapter.
///
/// ```ignore
/// gpu_test!(fn fills_a_texture(ctx) {
///     let texture = ctx.device.create_texture(/* ... */);
/// });
/// ```
#[macro_export]
macro_rules! gpu_test {
    ($(#[$meta:meta])* fn $name:ident($ctx:ident) $body:block) => {
        $(#[$meta])*
        #[test]
        fn $name() {
            let $ctx = $crate::testing::context();
            $body
        }
    };
}
