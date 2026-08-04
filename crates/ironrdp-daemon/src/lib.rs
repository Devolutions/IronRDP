#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

//! Reusable support for persistent RDP sessions hosted over local RPC.

pub mod daemon;
pub mod logbuf;
pub mod now;
pub mod operations;

mod ipc {
    pub(crate) use ironrdp_rpc::ipc::*;
}

#[cfg(test)]
mod transport {
    pub(crate) use ironrdp_rpc::transport::*;
}
