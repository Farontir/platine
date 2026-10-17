//! Patine desktop application.
//!
//! Usage: `patine_app [mesh] [options]`; see `--help`.
//!
//! With the `windows-gui` feature (installed builds), Windows starts it without a
//! console: the logs stay in the Console panel, and a fatal error shows in a dialog.

#![cfg_attr(feature = "windows-gui", windows_subsystem = "windows")]

use patine_app::cli::{StartupOptions, USAGE};
use patine_app::i18n::{I18n, Language};
use patine_app::{app, logging, profiling::Profiler};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let result = run(args);
    if let Err(error) = &result
        && cfg!(feature = "windows-gui")
    {
        report(error);
    }
    result
}

fn run(args: Vec<String>) -> anyhow::Result<()> {
    let options =
        StartupOptions::parse(args).map_err(|error| anyhow::anyhow!("{error}\n{USAGE}"))?;
    let logs = logging::init();
    let profiler = Profiler::from_env();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Patine starting");
    app::run(logs, profiler, options)
}

/// Shows a fatal error in a dialog: without a console, it would go unseen.
fn report(error: &anyhow::Error) {
    let i18n = I18n::new(Language::system());
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(i18n.tr("app-title"))
        .set_description(format!("{}\n\n{error:#}", i18n.tr("app-fatal-error")))
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
