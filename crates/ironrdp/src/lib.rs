#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(rustfmt, rustfmt_skip)]

#[cfg(test)]
use {
    anyhow as _, async_trait as _, image as _, ironrdp_blocking as _, ironrdp_cliprdr_native as _, opus2 as _,
    pico_args as _, rand as _, sspi as _, tokio_rustls as _, tracing as _, tracing_subscriber as _, x509_cert as _,
};

#[cfg(feature = "acceptor")]
#[doc(inline)]
pub use ironrdp_acceptor as acceptor;

#[cfg(feature = "cliprdr")]
#[doc(inline)]
pub use ironrdp_cliprdr as cliprdr;

#[cfg(feature = "connector")]
#[doc(inline)]
pub use ironrdp_connector as connector;

#[cfg(feature = "core")]
#[doc(inline)]
pub use ironrdp_core as core;

#[cfg(feature = "displaycontrol")]
#[doc(inline)]
pub use ironrdp_displaycontrol as displaycontrol;

#[cfg(feature = "dvc")]
#[doc(inline)]
pub use ironrdp_dvc as dvc;

#[cfg(feature = "graphics")]
#[doc(inline)]
pub use ironrdp_graphics as graphics;

#[cfg(feature = "input")]
#[doc(inline)]
pub use ironrdp_input as input;

#[cfg(feature = "pdu")]
#[doc(inline)]
pub use ironrdp_pdu as pdu;

#[cfg(feature = "rdpdr")]
#[doc(inline)]
pub use ironrdp_rdpdr as rdpdr;

#[cfg(feature = "rdpsnd")]
#[doc(inline)]
pub use ironrdp_rdpsnd as rdpsnd;

#[cfg(feature = "server")]
#[doc(inline)]
pub use ironrdp_server as server;

#[cfg(feature = "session")]
#[doc(inline)]
pub use ironrdp_session as session;

#[cfg(feature = "svc")]
#[doc(inline)]
pub use ironrdp_svc as svc;
