#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

use core::future::Future;

pub use bytes;

mod connector;
mod framed;
mod session;
mod vmconnector;

use ironrdp_connector::sspi::generator::NetworkRequest;
use ironrdp_connector::ConnectorResult;

pub use self::connector::*;
pub use self::framed::*;
pub use self::vmconnector::*;

pub trait NetworkClient {
    fn send(&mut self, network_request: &NetworkRequest) -> impl Future<Output = ConnectorResult<Vec<u8>>>;
}
