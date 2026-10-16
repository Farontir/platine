//! GPU evaluation of the layer stack, with a CPU reference implementation of every blend
//! mode (M5).
//!
//! [`gpu_document::GpuDocument`] keeps the final texture of every channel in step with
//! the document; [`content`] and [`paint`] hold the procedural and painted contents;
//! [`offline`] evaluates a texture set at another resolution (M10).

pub mod compositor;
pub mod content;
pub mod gpu_document;
pub mod offline;
pub mod paint;
pub mod reference;
