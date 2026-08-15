#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod error;
pub mod pdu;
pub mod tunnel;

pub use self::error::{RdpemtError, RdpemtErrorExt, RdpemtErrorKind, RdpemtResult};
pub use self::pdu::{
    SECURITY_COOKIE_LEN, SubHeaderType, TunnelAction, TunnelCreateRequest, TunnelCreateResponse, TunnelData,
    TunnelHeader, TunnelPdu, TunnelSubHeader,
};
pub use self::tunnel::{RdpemtTunnel, Side, TunnelConfig, TunnelEvent};
