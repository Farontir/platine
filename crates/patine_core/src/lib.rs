//! Document model of Patine: projects, texture sets, layer stacks, commands and undo.
//!
//! This crate is pure CPU data and must never depend on `wgpu` or `egui`:
//! the whole document model is testable without a GPU.

pub mod mesh;
pub mod project;
