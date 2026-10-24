//! Patine desktop application.
//!
//! Usage: `patine_app [mesh] [options]`; see `--help`.
//!
//! With the `windows-gui` feature (installed builds), Windows starts it without a
//! console: the logs stay in the Console panel, and a fatal error shows in a dialog.

#![cfg_attr(feature = "windows-gui", windows_subsystem = "windows")]

use std::panic::AssertUnwindSafe;
use std::path::Path;

use fluent_bundle::FluentArgs;
use patine_app::cli::{StartupOptions, USAGE};
use patine_app::i18n::{I18n, Language};
use patine_app::{app, logging, profiling::Profiler};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let log_file = logging::log_file();
    // A panic of the main thread ends the application like an error: it gets the same
    // dialog, the panic hook having logged its details.
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| run(args, log_file.as_deref())))
        .unwrap_or_else(|payload| {
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_default();
            Err(anyhow::anyhow!("panic: {message}"))
        });
    if let Err(error) = &result
        && cfg!(feature = "windows-gui")
    {
        report(error, log_file.as_deref());
    }
    result
}

fn run(args: Vec<String>, log_file: Option<&Path>) -> anyhow::Result<()> {
    let options =
        StartupOptions::parse(args).map_err(|error| anyhow::anyhow!("{error}\n{USAGE}"))?;
    let logs = logging::init(log_file);
    logging::log_panics();
    let profiler = Profiler::from_env();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        "Patine starting"
    );
    app::run(logs, profiler, options)
}

/// Shows a fatal error in a dialog: without a console, it would go unseen.
fn report(error: &anyhow::Error, log_file: Option<&Path>) {
    let i18n = I18n::new(Language::system());
    let mut description = format!("{}\n\n{error:#}", i18n.tr("app-fatal-error"));
    if let Some(path) = log_file {
        let mut args = FluentArgs::new();
        args.set("path", path.display().to_string());
        description += &format!("\n\n{}", i18n.tr_args("app-log-file", &args));
    }
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title(i18n.tr("app-title"))
        .set_description(description)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}
