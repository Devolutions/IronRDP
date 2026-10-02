#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

pub use bytes;

#[cfg(feature = "rustcrypto")]
mod connector;
mod framed;
mod session;

#[cfg(feature = "rustcrypto")]
use ironrdp_connector::ConnectorResult;
#[cfg(feature = "rustcrypto")]
use ironrdp_connector::sspi::generator::NetworkRequest;

#[cfg(feature = "rustcrypto")]
pub use self::connector::*;
pub use self::framed::*;
// pub use self::session::*;

#[cfg(feature = "rustcrypto")]
pub trait NetworkClient {
    fn send(&mut self, network_request: &NetworkRequest) -> impl Future<Output = ConnectorResult<Vec<u8>>>;
}
