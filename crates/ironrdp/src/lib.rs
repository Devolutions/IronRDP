#![doc = include_str!("../../../README.md")]
#![doc(
    html_logo_url = "https://webdevolutions.blob.core.windows.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg"
)]

#[cfg(test)]
use {
    anyhow as _, async_trait as _, bmp as _, ironrdp_blocking as _, ironrdp_cliprdr_native as _, pico_args as _,
    rand as _, tokio_rustls as _, tracing as _, tracing_subscriber as _, x509_cert as _,
};

#[cfg(feature = "acceptor")]
pub use ironrdp_acceptor as acceptor;
#[cfg(feature = "cliprdr")]
pub use ironrdp_cliprdr as cliprdr;
#[cfg(feature = "connector")]
pub use ironrdp_connector as connector;
#[cfg(feature = "core")]
pub use ironrdp_core as core;
#[cfg(feature = "displaycontrol")]
pub use ironrdp_displaycontrol as displaycontrol;
#[cfg(feature = "dvc")]
pub use ironrdp_dvc as dvc;
#[cfg(feature = "graphics")]
pub use ironrdp_graphics as graphics;
#[cfg(feature = "input")]
pub use ironrdp_input as input;
#[cfg(feature = "pdu")]
pub use ironrdp_pdu as pdu;
#[cfg(feature = "rdpdr")]
pub use ironrdp_rdpdr as rdpdr;
#[cfg(feature = "rdpsnd")]
pub use ironrdp_rdpsnd as rdpsnd;
#[cfg(feature = "server")]
pub use ironrdp_server as server;
#[cfg(feature = "session")]
pub use ironrdp_session as session;
#[cfg(feature = "svc")]
pub use ironrdp_svc as svc;
