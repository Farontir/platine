//! Document model of Patine: projects, texture sets, layer stacks, commands and undo.
//!
//! This crate is pure CPU data and must never depend on `wgpu` or `egui`:
//! the whole document model is testable without a GPU.

pub mod bake;
pub mod channel;
pub mod environment;
pub mod export;
pub mod ids;
pub mod layer;
pub mod mask;
pub mod mesh;
pub mod project;
pub mod resource;
pub mod stroke;
