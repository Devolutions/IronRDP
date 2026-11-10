//! A Wayland clipboard client built on the data-control protocols.
//!
//! Data-control lets a client that has no window read and set the clipboard.
//! This module speaks both [`ext-data-control-v1`] and
//! [`wlr-data-control-unstable-v1`] and picks whichever the compositor offers.
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
//! CLIPRDR, where the [2.2.5.2] Format Data Response PDU follows the paste.
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
//! A paste that waits more than five seconds for its data is closed with no
//! data, and an answer that comes later is kept for the next paste. A reader
//! that takes nothing for five seconds is dropped too, and at most 16 pastes
//! are served at once, so a stuck program cannot pile up threads.
//!
//! A selection set here also advertises one private type, so the client can tell it from another
//! client's copy. Other clients see that type among the types they are offered.
//!
//! GNOME's Mutter offers no data-control protocol, so [`DataControl::connect`]
//! returns [`Error::Unsupported`] there.
//!
//! [`ext-data-control-v1`]: https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/ext-data-control/ext-data-control-v1.xml
//! [`wlr-data-control-unstable-v1`]: https://gitlab.freedesktop.org/wlroots/wlr-protocols/-/blob/master/unstable/wlr-data-control-unstable-v1.xml
//! [2.2.5.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeclip/28c193b8-4cec-413e-a07b-9235e5e15f6b

mod client;
mod dispatch;
mod error;
mod mime;
mod options;
#[cfg(feature = "__test")]
pub mod state;
#[cfg(not(feature = "__test"))]
pub(crate) mod state;
mod worker;

pub use self::client::{Content, DataControl, TransferRequest};
pub use self::error::{Error, Result};
pub use self::mime::find_mime_match;
pub use self::options::{Options, Preference, Protocol};
