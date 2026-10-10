//! puffin profiler, switchable at runtime.
//!
//! When enabled, scopes are recorded and served by `puffin_http` on
//! [`puffin_http::DEFAULT_PORT`]; connect `puffin_viewer` to inspect them.

/// Environment variable enabling the profiler at startup when set to `1`.
pub const PROFILE_ENV: &str = "PATINE_PROFILE";

/// Profiler state owned by the application.
#[derive(Default)]
pub struct Profiler {
    server: Option<puffin_http::Server>,
    enabled: bool,
}

impl Profiler {
    /// Creates the profiler, enabled if `PATINE_PROFILE=1`.
    pub fn from_env() -> Self {
        let mut profiler = Self::default();
        if std::env::var(PROFILE_ENV).is_ok_and(|value| value == "1") {
            profiler.set_enabled(true);
        }
        profiler
    }

    /// Whether scopes are currently recorded.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Enables or disables recording. The HTTP server starts on first activation.
    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled && self.server.is_none() {
            let address = format!("127.0.0.1:{}", puffin_http::DEFAULT_PORT);
            match puffin_http::Server::new(&address) {
                Ok(server) => {
                    tracing::info!(%address, "profiler server started, connect puffin_viewer");
                    self.server = Some(server);
                }
                Err(error) => {
                    tracing::error!(%address, %error, "failed to start the profiler server");
                    return;
                }
            }
        }
        self.enabled = enabled;
        puffin::set_scopes_on(enabled);
        tracing::info!(enabled, "profiler toggled");
    }

    /// Marks the end of a frame. Call once per rendered frame.
    pub fn new_frame(&self) {
        if self.enabled {
            puffin::GlobalProfiler::lock().new_frame();
        }
    }
}
