//! Client dynamic channel processors for [MS-RDPEPNP].
//!
//! [`PnpInfoClient`] runs the `PNPDR` channel and announces the redirected devices.
//! [`FileRedirectorListener`] opens one [`FileRedirectorClient`] for each `FileRedirectorChannel`
//! instance and forwards its I/O requests to a [`PnpBackend`].
//!
//! [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/

use alloc::collections::BTreeSet;
use alloc::sync::Arc;

mod info;
mod io;

pub use info::PnpInfoClient;
pub use io::{
    CustomEventSender, FileRedirectorClient, FileRedirectorListener, IoControlCompletion, IoUplink, PnpBackend,
    PnpFile, ReadCompletion, WriteCompletion,
};

/// Client device IDs announced to the server on the active `PNPDR` channel.
type AnnouncedDevices = Arc<spin::Mutex<BTreeSet<u32>>>;
