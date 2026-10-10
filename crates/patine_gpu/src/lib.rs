//! GPU foundation of Patine: wgpu context, shader loading and GPU test tooling.
//!
//! Every GPU crate builds on [`GpuContext`] and loads its WGSL through [`shaders::load`].
//! Tests use the `testing` feature, which provides the [`gpu_test!`] macro and the
//! golden-image comparator.

mod context;
mod error;
pub mod fill;
pub mod readback;
pub mod shaders;
// T3: #[cfg(feature = "testing")]
// T3: pub mod testing;

pub use context::{AdapterChoice, GpuCapabilities, GpuContext};
pub use error::GpuError;
pub use wgpu;
