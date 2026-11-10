//! The clipboard interface used by the protocol worker.

use std::sync::mpsc::Sender;

use super::worker::Command;
use super::x11::X11Clipboard;
use crate::data_control::{Content, DataControl};

pub const TEXT: &str = "text/plain;charset=utf-8";
pub const PNG: &str = "image/png";

type CompletePaste = Box<dyn FnOnce(Option<Vec<u8>>) + Send>;

/// A delayed OS paste. Every completion, including failure, closes its waiter.
pub struct Transfer {
    mime: String,
    generation: u64,
    complete: Option<CompletePaste>,
}

impl core::fmt::Debug for Transfer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Transfer")
            .field("mime", &self.mime)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl Transfer {
    pub fn new(mime: String, generation: u64, complete: impl FnOnce(Option<Vec<u8>>) + Send + 'static) -> Self {
        Self {
            mime,
            generation,
            complete: Some(Box::new(complete)),
        }
    }

    pub fn mime_type(&self) -> &str {
        &self.mime
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn finish(mut self, data: Option<Vec<u8>>) {
        if let Some(complete) = self.complete.take() {
            complete(data);
        }
    }
}

impl Drop for Transfer {
    fn drop(&mut self) {
        if let Some(complete) = self.complete.take() {
            complete(None);
        }
    }
}

pub trait OsClipboard {
    fn is_owner(&self) -> bool;
    fn mime_types(&self) -> Vec<String>;
    fn read(&mut self, mime: &str) -> Result<Option<Vec<u8>>, String>;
    fn offer(&mut self, mimes: &[String], generation: u64) -> Result<(), String>;
    fn clear(&mut self) -> Result<(), String>;
}

pub enum NativeClipboard {
    Wayland {
        clipboard: DataControl,
        events: Sender<Command>,
    },
    X11(X11Clipboard),
}

impl NativeClipboard {
    pub fn open(events: Sender<Command>) -> Result<Self, String> {
        match DataControl::connect() {
            Ok(clipboard) => {
                let changes = events.clone();
                clipboard.on_change(move |_| {
                    let _ = changes.send(Command::LocalChanged);
                });
                Ok(Self::Wayland { clipboard, events })
            }
            Err(wayland_error) => {
                tracing::debug!(%wayland_error, "Wayland data-control unavailable; trying X11");
                X11Clipboard::open(events)
                    .map(Self::X11)
                    .map_err(|x11_error| format!("Wayland: {wayland_error}; X11: {x11_error}"))
            }
        }
    }
}

impl OsClipboard for NativeClipboard {
    fn is_owner(&self) -> bool {
        match self {
            Self::Wayland { clipboard, .. } => clipboard.owns_selection(),
            Self::X11(clipboard) => clipboard.is_owner(),
        }
    }

    fn mime_types(&self) -> Vec<String> {
        match self {
            Self::Wayland { clipboard, .. } => clipboard.selection_mime_types(),
            Self::X11(clipboard) => clipboard.mime_types(),
        }
    }

    fn read(&mut self, mime: &str) -> Result<Option<Vec<u8>>, String> {
        match self {
            // Fails with `SelectionChanged` if another copy lands during the read.
            Self::Wayland { clipboard, .. } => clipboard.read(mime).map_err(|error| error.to_string()),
            Self::X11(clipboard) => clipboard.read(mime),
        }
    }

    fn offer(&mut self, mimes: &[String], generation: u64) -> Result<(), String> {
        match self {
            Self::Wayland { clipboard, events } => {
                let events = events.clone();
                clipboard.on_transfer(move |request| {
                    let transfer = Transfer::new(request.mime_type().to_owned(), generation, move |data| {
                        let result = match data {
                            Some(data) => request.complete(data),
                            None => request.fail(),
                        };
                        if let Err(error) = result {
                            tracing::debug!(%error, "Could not finish a Wayland paste");
                        }
                    });
                    let _ = events.send(Command::Paste(transfer));
                });
                let content = mimes
                    .iter()
                    .fold(Content::new(), |content, mime| content.advertise(mime));
                clipboard.set_selection(content).map_err(|error| error.to_string())?;
                clipboard.synchronize().map_err(|error| error.to_string())?;
                if !clipboard.owns_selection() {
                    return Err("clipboard ownership was not acquired".into());
                }
                Ok(())
            }
            Self::X11(clipboard) => clipboard.offer(mimes, generation),
        }
    }

    fn clear(&mut self) -> Result<(), String> {
        match self {
            Self::Wayland { clipboard, .. } => clipboard.clear_selection().map_err(|error| error.to_string()),
            Self::X11(clipboard) => clipboard.clear(),
        }
    }
}
