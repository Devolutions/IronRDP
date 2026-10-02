//! A Wayland clipboard client built on the data-control protocols.
//!
//! Data-control lets a client that has no window read and set the clipboard.
//! This module speaks both `ext-data-control-v1` and
//! `wlr-data-control-unstable-v1` and picks whichever the compositor offers.
//!
//! It has no async runtime. One thread owns the Wayland connection, and
//! [`DataControl`] is the handle to call from anywhere.
//!
//! # Reading
//!
//! ```no_run
//! use ironrdp_cliprdr_native::data_control::DataControl;
//!
//! # fn main() -> ironrdp_cliprdr_native::data_control::Result<()> {
//! let clipboard = DataControl::connect()?;
//! if let Some(bytes) = clipboard.read("text/plain;charset=utf-8")? {
//!     println!("{}", String::from_utf8_lossy(&bytes));
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Owning the clipboard, with delayed rendering
//!
//! A type advertised without data raises a [`TransferRequest`] when something
//! pastes it, so the data is produced only if it is wanted. That maps onto
//! CLIPRDR, where the Format Data Response follows the paste.
//!
//! ```no_run
//! use ironrdp_cliprdr_native::data_control::{Content, DataControl};
//!
//! # fn main() -> ironrdp_cliprdr_native::data_control::Result<()> {
//! let clipboard = DataControl::connect()?;
//! clipboard.on_transfer(|request| {
//!     let _ = request.complete("rendered on demand");
//! });
//! clipboard.set_selection(Content::new().advertise("text/plain;charset=utf-8"))?;
//! # Ok(())
//! # }
//! ```
//!
//! GNOME's Mutter offers no data-control protocol, so [`DataControl::connect`]
//! returns [`Error::Unsupported`] there.

mod client;
mod dispatch;
mod error;
mod mime;
mod options;
#[cfg(feature = "__test")]
pub mod state;
#[cfg(not(feature = "__test"))]
#[expect(
    unreachable_pub,
    reason = "the __test feature exposes this module to the shared integration tests"
)]
pub(crate) mod state;
mod worker;

#[cfg(feature = "__test")]
pub use self::client::read_pipe;
pub use self::client::{Content, DataControl, TransferRequest};
pub use self::error::{Error, Result};
pub use self::mime::find_mime_match;
pub use self::options::{Options, Preference, Protocol};
