#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]

//! MS-RDPEWA WebAuthn dynamic virtual channel.

/// DVC channel name per [MS-RDPEWA].
///
/// [MS-RDPEWA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpewa/
pub const CHANNEL_NAME: &str = "WebAuthN_Channel";

pub mod client;
pub mod pdu;
pub mod server;

pub use client::{
    RdpewaClient, RdpewaClientHandler, RdpewaClientListener, RdpewaHandlerError, RdpewaResponseSender, RdpewaResult,
    StubRdpewaHandler, WebAuthnDispatch, WebAuthnOperationRequest, WebAuthnOperationResponse,
};
pub use pdu::{
    Attachment, Attestation, DeviceInfo, E_ABORT, E_BUSY, E_FAIL, E_INVALIDARG, E_NOTIMPL, RdpewaRequest,
    RdpewaResponse, RpcCommand, S_OK, UserVerification, WebAuthnPara, WebAuthnRequestBody, WebAuthnResponsePayload,
    WebAuthnSubcommand,
};
pub use server::RdpewaServer;
