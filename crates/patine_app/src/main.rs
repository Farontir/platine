//! Patine desktop application.
//!
//! Usage: `patine_app [mesh] [options]`; see `--help`.

use patine_app::cli::{StartupOptions, USAGE};
use patine_app::{app, logging, profiling::Profiler};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let options =
        StartupOptions::parse(args).map_err(|error| anyhow::anyhow!("{error}\n{USAGE}"))?;
    let logs = logging::init();
    let profiler = Profiler::from_env();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Patine starting");
    app::run(logs, profiler, options)
}
