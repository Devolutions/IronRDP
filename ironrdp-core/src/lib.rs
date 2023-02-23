#[macro_use]
mod macros;

pub mod codecs;
pub mod gcc;
pub mod geometry;
pub mod input;
pub mod mcs;
pub mod rdp;
pub mod utils;

pub(crate) mod basic_output;
pub(crate) mod ber;
pub(crate) mod crypto;
pub(crate) mod per;
pub(crate) mod preconnection;
pub(crate) mod x224;

mod connection_initiation;

use thiserror::Error;

pub use crate::basic_output::{bitmap, fast_path, surface_commands};
pub use crate::connection_initiation::*;
pub use crate::mcs::{ConnectInitial, ConnectResponse, McsError, McsPdu, SendDataContext};
pub use crate::preconnection::{PreconnectionPdu, PreconnectionPduError};
pub use crate::rdp::vc::dvc;
pub use crate::rdp::{
    CapabilitySet, ClientConfirmActive, ClientInfoPdu, ControlAction, DemandActive, ServerDemandActive,
    ShareControlHeader, ShareControlPdu, ShareDataHeader, ShareDataPdu, VirtualChannel,
};
pub use crate::x224::*;

pub trait PduParsing {
    type Error; // FIXME: this bound type should probably be removed for the sake of simplicity

    fn from_buffer(stream: impl std::io::Read) -> Result<Self, Self::Error>
    where
        Self: Sized;
    fn to_buffer(&self, stream: impl std::io::Write) -> Result<(), Self::Error>;
    fn buffer_length(&self) -> usize;
}

/// Blanket implementation for references to types implementing PduParsing. Only encoding is supported.
///
/// This helps removing a few copies.
impl<T: PduParsing> PduParsing for &T {
    type Error = T::Error;

    fn from_buffer(_: impl std::io::Read) -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        panic!("Can’t return a reference to a local value")
    }

    fn to_buffer(&self, stream: impl std::io::Write) -> Result<(), Self::Error> {
        T::to_buffer(self, stream)
    }

    fn buffer_length(&self) -> usize {
        T::buffer_length(self)
    }
}

pub trait PduBufferParsing<'a>: Sized {
    type Error; // FIXME: this bound type should probably be removed for the sake of simplicity

    fn from_buffer(mut buffer: &'a [u8]) -> Result<Self, Self::Error> {
        Self::from_buffer_consume(&mut buffer)
    }
    fn from_buffer_consume(buffer: &mut &'a [u8]) -> Result<Self, Self::Error>;
    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error>;
    fn buffer_length(&self) -> usize;
}

pub enum PduHeader {
    X224(x224::DataHeader),
    FastPath(fast_path::FastPathHeader),
}

impl PduParsing for PduHeader {
    type Error = RdpError;

    fn from_buffer(mut stream: impl std::io::Read) -> Result<Self, Self::Error> {
        use bit_field::BitField as _;
        use byteorder::ReadBytesExt as _;
        use num_traits::FromPrimitive as _;

        let header = stream.read_u8()?;
        let action = header.get_bits(0..2);
        let action = Action::from_u8(action).ok_or(RdpError::InvalidActionCode(action))?;

        match action {
            Action::X224 => Ok(Self::X224(x224::DataHeader::from_buffer_with_version(
                &mut stream,
                header,
            )?)),
            Action::FastPath => Ok(Self::FastPath(fast_path::FastPathHeader::from_buffer_with_header(
                &mut stream,
                header,
            )?)),
        }
    }

    fn to_buffer(&self, stream: impl std::io::Write) -> Result<(), Self::Error> {
        match self {
            Self::X224(x224) => x224.to_buffer(stream).map_err(RdpError::from),
            Self::FastPath(fast_path) => fast_path.to_buffer(stream).map_err(RdpError::from),
        }
    }

    fn buffer_length(&self) -> usize {
        match self {
            Self::X224(x224) => x224.buffer_length(),
            Self::FastPath(fast_path) => fast_path.buffer_length(),
        }
    }
}

#[derive(Debug, Error)]
pub enum RdpError {
    #[error("IO error")]
    IOError(#[from] std::io::Error),
    #[error("X224 error")]
    X224Error(#[from] connection_initiation::NegotiationError),
    #[error("Surface Commands error")]
    FastPathError(#[from] fast_path::FastPathError),
    #[error("Received invalid action code: {0}")]
    InvalidActionCode(u8),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub enum Action {
    FastPath = 0x0,
    X224 = 0x3,
}
