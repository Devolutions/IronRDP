/// Native CLIPRDR backend implementations. Currently only Windows is supported.

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use crate::windows::{WinClipboard, WinCliprdrError, WinCliprdrResult};
