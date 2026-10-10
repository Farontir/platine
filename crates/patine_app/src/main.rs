//! Patine desktop application.

use patine_app::{logging, profiling::Profiler};

fn main() -> anyhow::Result<()> {
    let _logs = logging::init();
    let _profiler = Profiler::from_env();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Patine starting");
    Ok(())
}
