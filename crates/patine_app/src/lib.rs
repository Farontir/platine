//! Patine desktop application: window, egui interface, actions, translations and logging.
//!
//! The binary (`main.rs`) only wires these modules together.

pub mod actions;
pub mod app;
pub mod autosave;
pub mod brush_presets;
pub mod cli;
pub mod curve_editor;
pub mod display_panel;
pub mod document;
pub mod environment_sync;
pub mod history_panel;
pub mod i18n;
pub mod layers_panel;
pub mod library_panel;
pub mod logging;
pub mod masks;
pub mod new_project;
pub mod paint_tool;
pub mod procedural_ui;
pub mod profiling;
pub mod project_io;
pub mod properties_panel;
pub mod tablet;
pub mod texture_sets_panel;
pub mod ui;
pub mod view_2d;
pub mod viewport_panel;
