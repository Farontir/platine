//! Patine desktop application.
//!
//! Usage: `patine_app [mesh]`. With a mesh file, a project is created from it at startup.

use std::path::PathBuf;

use patine_app::{app, logging, profiling::Profiler};

fn main() -> anyhow::Result<()> {
    let logs = logging::init();
    let profiler = Profiler::from_env();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Patine starting");
    let mesh = std::env::args_os().nth(1).map(PathBuf::from);
    app::run(logs, profiler, mesh.as_deref())
}
