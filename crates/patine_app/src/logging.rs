//! Logging: `tracing` events go to the terminal, to a log file, and to a ring buffer read
//! by the Console panel.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer};

/// Number of lines kept for the Console panel.
pub const CAPACITY: usize = 2000;

/// One log line shown in the Console panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    /// Severity of the event.
    pub level: Level,
    /// Module path of the event.
    pub target: String,
    /// Message followed by the event fields (`key=value`).
    pub message: String,
}

/// Shared, bounded buffer of the most recent log lines.
#[derive(Clone, Debug, Default)]
pub struct LogBuffer {
    lines: Arc<Mutex<VecDeque<LogLine>>>,
}

impl LogBuffer {
    /// Appends a line, dropping the oldest one beyond [`CAPACITY`].
    pub fn push(&self, line: LogLine) {
        let mut lines = self.lock();
        if lines.len() == CAPACITY {
            lines.pop_front();
        }
        lines.push_back(line);
    }

    /// Locks the buffer for reading. Hold the guard only while drawing.
    pub fn lock(&self) -> MutexGuard<'_, VecDeque<LogLine>> {
        // A panic while holding the lock leaves a valid VecDeque: keep the logs.
        self.lines
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Removes every line.
    pub fn clear(&self) {
        self.lock().clear();
    }
}

/// `tracing` layer copying events into a [`LogBuffer`].
struct ConsoleLayer {
    buffer: LogBuffer,
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: String,
}

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, " {}={value}", field.name());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }
}

impl<S: Subscriber> Layer<S> for ConsoleLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let metadata = event.metadata();
        self.buffer.push(LogLine {
            level: *metadata.level(),
            target: metadata.target().to_owned(),
            message: visitor.message + &visitor.fields,
        });
    }
}

/// Where the log of this session is written: `PATINE_LOG_DIR`, or the `logs` folder of
/// the application data folder.
pub fn log_file() -> Option<PathBuf> {
    let dir = match std::env::var_os("PATINE_LOG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => crate::autosave::data_dir()?.join("logs"),
    };
    Some(dir.join(LOG_FILE))
}

/// Name of the log file of the current session.
pub const LOG_FILE: &str = "patine.log";

/// Name the log of the previous session is kept under.
pub const PREVIOUS_LOG_FILE: &str = "patine.previous.log";

/// Creates `path` for writing, after keeping the log it holds as the previous session's.
fn open_log_file(path: &Path) -> std::io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        if path.exists() {
            std::fs::rename(path, dir.join(PREVIOUS_LOG_FILE))?;
        }
    }
    File::create(path)
}

/// Installs the global subscriber: `RUST_LOG` filter (default: `info` for Patine,
/// `warn` elsewhere), terminal output, the log file `file` when given, and the Console
/// buffer it returns.
///
/// Installed builds have no terminal: the file is what a user can send when the
/// application fails before showing anything.
pub fn init(file: Option<&Path>) -> LogBuffer {
    let buffer = LogBuffer::default();
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,patine_app=info,patine_gpu=info"));
    let (writer, error) = match file.map(open_log_file) {
        Some(Ok(file)) => (Some(Mutex::new(file)), None),
        Some(Err(error)) => (None, Some(error)),
        None => (None, None),
    };
    let file_layer = writer.map(|writer| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(writer)
    });
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(file_layer)
        .with(ConsoleLayer {
            buffer: buffer.clone(),
        })
        .init();
    if let Some(path) = file {
        match error {
            None => tracing::info!(path = %path.display(), "log file"),
            Some(error) => {
                tracing::warn!(path = %path.display(), %error, "cannot write the log file");
            }
        }
    }
    buffer
}

/// Logs panics, so that they reach the log file and the Console panel: an installed
/// build has no terminal to print them to. The previous hook still runs afterwards.
pub fn log_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        tracing::error!(
            thread = thread.name().unwrap_or("unnamed"),
            "panic: {info}\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(message: &str) -> LogLine {
        LogLine {
            level: Level::INFO,
            target: "test".into(),
            message: message.into(),
        }
    }

    #[test]
    fn buffer_drops_oldest_lines() {
        let buffer = LogBuffer::default();
        for i in 0..CAPACITY + 5 {
            buffer.push(line(&i.to_string()));
        }
        let lines = buffer.lock();
        assert_eq!(lines.len(), CAPACITY);
        assert_eq!(lines.front().unwrap().message, "5");
        assert_eq!(lines.back().unwrap().message, (CAPACITY + 4).to_string());
    }

    #[test]
    fn layer_captures_message_and_fields() {
        let buffer = LogBuffer::default();
        let subscriber = tracing_subscriber::registry().with(ConsoleLayer {
            buffer: buffer.clone(),
        });
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(width = 640, "surface resized");
        });
        let lines = buffer.lock();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].level, Level::WARN);
        assert_eq!(lines[0].message, "surface resized width=640");
    }

    #[test]
    fn log_file_keeps_the_previous_session() {
        let dir = std::env::temp_dir().join(format!("patine-logs-{}", std::process::id()));
        let path = dir.join("nested").join(LOG_FILE);
        let _ = std::fs::remove_dir_all(&dir);

        let mut first = open_log_file(&path).unwrap();
        std::io::Write::write_all(&mut first, b"first session").unwrap();
        drop(first);
        let second = open_log_file(&path).unwrap();
        drop(second);

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        let previous = path.with_file_name(PREVIOUS_LOG_FILE);
        assert_eq!(std::fs::read_to_string(previous).unwrap(), "first session");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
