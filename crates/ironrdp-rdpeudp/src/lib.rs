#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod connection;
pub mod error;
pub mod pdu;
pub mod seq;
pub mod time;

pub(crate) mod congestion;
pub(crate) mod loss;
pub(crate) mod recv_window;
pub(crate) mod reliability;
pub(crate) mod rtt;
pub(crate) mod send_window;
pub(crate) mod timer;

pub use self::connection::{ConnectionConfig, Event, RdpeudpConnection, Side, Transmit};
pub use self::error::{RdpeudpError, RdpeudpErrorExt, RdpeudpErrorKind, RdpeudpResult, SendError};
pub use self::time::MonotonicInstant;
