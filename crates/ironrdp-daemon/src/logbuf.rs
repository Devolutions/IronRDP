//! The RDP session log ring buffer and its [`tracing`] layer.
//!
//! The logs emitted while driving the RDP engine are captured into a small, queryable
//! [`LogBuffer`] ring (read via `Request::QueryLogs`) instead of the terminal. The capture is
//! installed as a thread-local subscriber for the session thread only (see [`session_dispatch`] and
//! [`tracing::dispatcher::with_default`]), so it never becomes the global subscriber. It defaults
//! to `DEBUG`, which is useful when inspecting a session, and a per-`Connect` directive can refine
//! the filter (e.g. `ironrdp_connector=trace`) to troubleshoot IronRDP itself.
//!
//! The daemon's *own* operational logging is a separate concern; see
//! [`crate::daemon`]'s global subscriber setup.

use core::fmt::Write as _;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::{Dispatch, Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

/// Default ring-buffer capacity, in lines.
const DEFAULT_CAPACITY: usize = 100;

/// A bounded ring buffer of formatted log lines.
pub(crate) struct LogBuffer {
    inner: Mutex<Inner>,
}

struct Inner {
    capacity: usize,
    lines: VecDeque<String>,
}

impl LogBuffer {
    pub(crate) fn new() -> Arc<Self> {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    pub(crate) fn with_capacity(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                capacity: capacity.max(1),
                lines: VecDeque::new(),
            }),
        })
    }

    fn push(&self, line: String) {
        let mut inner = self.inner.lock().expect("log buffer poisoned");

        if inner.capacity <= inner.lines.len() {
            inner.lines.pop_front();
        }
        inner.lines.push_back(line);
    }

    /// Returns retained lines, optionally filtered to those containing `substring`.
    pub(crate) fn query(&self, substring: Option<&str>) -> Vec<String> {
        let inner = self.inner.lock().expect("log buffer poisoned");
        inner
            .lines
            .iter()
            .filter(|line| substring.is_none_or(|needle| line.contains(needle)))
            .cloned()
            .collect()
    }
}

/// Builds a session-scoped [`Dispatch`] that routes the RDP session's logs into `buffer`.
///
/// The session runs on its own thread; wrapping its execution in
/// [`tracing::dispatcher::with_default`] keeps the engine's events out of the daemon's terminal and
/// in the ring buffer instead. The default level is `DEBUG`; `directive` (carried by
/// `Request::Connect`) refines it per-session — a bare level sets the global session level, while a
/// targeted directive (e.g. `ironrdp_connector=trace`) layers on top of the `DEBUG` default.
pub(crate) fn session_dispatch(buffer: Arc<LogBuffer>, directive: Option<&str>) -> Dispatch {
    use tracing::level_filters::LevelFilter;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::DEBUG.into())
        .parse_lossy(directive.unwrap_or(""));

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(LogLayer::new(buffer));

    Dispatch::new(subscriber)
}

/// A tracing [`Layer`] that formats each event into a single line and pushes it to a [`LogBuffer`].
struct LogLayer {
    buffer: Arc<LogBuffer>,
}

impl LogLayer {
    fn new(buffer: Arc<LogBuffer>) -> Self {
        Self { buffer }
    }
}

impl<S: Subscriber> Layer<S> for LogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();

        let mut visitor = LogVisitor {
            message: None,
            fields: String::new(),
        };
        event.record(&mut visitor);

        let mut line = String::new();
        let _ = write!(line, "{:>5} {}", meta.level(), meta.target());
        if let Some(message) = &visitor.message {
            let _ = write!(line, " {message}");
        }
        line.push_str(&visitor.fields);

        self.buffer.push(line);
    }
}

/// Collects an event's message and structured fields into strings.
struct LogVisitor {
    message: Option<String>,
    fields: String,
}

impl Visit for LogVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn core::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        } else {
            let _ = write!(self.fields, " {}={:?}", field.name(), value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_owned());
        } else {
            let _ = write!(self.fields, " {}={}", field.name(), value);
        }
    }
}
