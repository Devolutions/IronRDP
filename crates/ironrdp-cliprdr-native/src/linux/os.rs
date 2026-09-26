//! The OS clipboard as seen by the worker: a trait so the state machine can be
//! tested without a display, and its `arboard` implementation.

use std::borrow::Cow;

use tracing::trace;

use super::image::Rgba;

/// One logical clipboard item. Text wins over an image when both are offered,
/// which is what a paste into a terminal or an editor expects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Content {
    Text(String),
    Image(Rgba),
}

pub(super) trait OsClipboard {
    /// The current clipboard content, or `None` when it is empty or holds
    /// nothing this backend can carry.
    fn read(&mut self) -> Option<Content>;

    /// Replaces the clipboard content.
    fn write(&mut self, content: &Content) -> Result<(), String>;
}

/// The real clipboard through `arboard`: Wayland data-control when the
/// compositor offers it, the X11 clipboard (through XWayland on Wayland
/// desktops) otherwise.
pub(super) struct ArboardClipboard {
    inner: arboard::Clipboard,
}

impl ArboardClipboard {
    pub(super) fn open() -> Result<Self, arboard::Error> {
        Ok(Self {
            inner: arboard::Clipboard::new()?,
        })
    }
}

impl OsClipboard for ArboardClipboard {
    fn read(&mut self) -> Option<Content> {
        match self.inner.get_text() {
            Ok(text) if !text.is_empty() => return Some(Content::Text(text)),
            Ok(_) | Err(arboard::Error::ContentNotAvailable) => {}
            Err(error) => {
                trace!(%error, "Could not read text from the OS clipboard");
                return None;
            }
        }

        match self.inner.get_image() {
            Ok(image) => {
                let width = u32::try_from(image.width).ok()?;
                let height = u32::try_from(image.height).ok()?;
                Some(Content::Image(Rgba {
                    width,
                    height,
                    bytes: image.bytes.into_owned(),
                }))
            }
            Err(arboard::Error::ContentNotAvailable) => None,
            Err(error) => {
                trace!(%error, "Could not read an image from the OS clipboard");
                None
            }
        }
    }

    fn write(&mut self, content: &Content) -> Result<(), String> {
        let result = match content {
            Content::Text(text) => self.inner.set_text(text.as_str()),
            Content::Image(image) => {
                let width = usize::try_from(image.width).map_err(|error| error.to_string())?;
                let height = usize::try_from(image.height).map_err(|error| error.to_string())?;
                self.inner.set_image(arboard::ImageData {
                    width,
                    height,
                    bytes: Cow::Borrowed(&image.bytes),
                })
            }
        };
        result.map_err(|error| error.to_string())
    }
}
