//! Patine desktop application: window, egui interface, actions, translations and logging.
//!
//! The binary (`main.rs`) only wires these modules together.

pub mod actions;
pub mod app;
pub mod autosave;
pub mod cli;
pub mod display_panel;
pub mod document;
pub mod environment_sync;
pub mod history_panel;
pub mod i18n;
pub mod logging;
pub mod new_project;
pub mod profiling;
pub mod project_io;
pub mod ui;
pub mod viewport_panel;
