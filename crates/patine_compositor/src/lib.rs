//! GPU evaluation of the layer stack, with a CPU reference implementation of every blend
//! mode (M5).
//!
//! [`gpu_document::GpuDocument`] keeps the final texture of every channel in step with
//! the document.

pub mod compositor;
pub mod gpu_document;
pub mod reference;
