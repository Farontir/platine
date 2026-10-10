//! Logging: `tracing` events go to the terminal and to a ring buffer read by the Console panel.

use std::collections::VecDeque;
use std::fmt::Write as _;
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

/// Installs the global subscriber: `RUST_LOG` filter (default: `info` for Patine,
/// `warn` elsewhere), terminal output, and the Console buffer it returns.
pub fn init() -> LogBuffer {
    let buffer = LogBuffer::default();
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,patine_app=info,patine_gpu=info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(ConsoleLayer {
            buffer: buffer.clone(),
        })
        .init();
    buffer
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
}
