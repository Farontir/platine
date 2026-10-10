//! Patine desktop application.

use patine_app::{app, logging, profiling::Profiler};

fn main() -> anyhow::Result<()> {
    let logs = logging::init();
    let profiler = Profiler::from_env();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Patine starting");
    app::run(logs, profiler)
}
