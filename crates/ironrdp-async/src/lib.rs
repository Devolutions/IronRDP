#![doc = include_str!("../README.md")]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

#[macro_use]
extern crate tracing;

pub use bytes;

mod connector;
mod framed;
mod session;

use core::future::Future;
use core::pin::Pin;

use ironrdp_connector::sspi::generator::NetworkRequest;
use ironrdp_connector::ConnectorResult;

pub use self::connector::*;
pub use self::framed::*;
// pub use self::session::*;

pub trait AsyncNetworkClient {
    fn send<'a>(
        &'a mut self,
        network_request: &'a NetworkRequest,
    ) -> Pin<Box<dyn Future<Output = ConnectorResult<Vec<u8>>> + 'a>>;
}
