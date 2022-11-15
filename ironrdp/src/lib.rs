pub mod codecs;
pub mod gcc;
pub mod input;
pub mod mcs;
pub mod nego;
pub mod rdp;

mod basic_output;
mod ber;
mod per;
mod preconnection;
mod utils;
mod x224;

pub use crate::basic_output::{bitmap, fast_path, surface_commands};
pub use crate::mcs::{ConnectInitial, ConnectResponse, McsError, McsPdu, SendDataContext};
pub use crate::nego::*;
pub use crate::preconnection::{PreconnectionPdu, PreconnectionPduError};
pub use crate::rdp::vc::dvc;
pub use crate::rdp::{
    CapabilitySet, ClientConfirmActive, ClientInfoPdu, ControlAction, DemandActive, ServerDemandActive,
    ShareControlHeader, ShareControlPdu, ShareDataHeader, ShareDataPdu, VirtualChannel,
};
pub use crate::utils::Rectangle;
pub use crate::x224::*;

pub trait PduParsing {
    type Error; // FIXME: this bound type should probably be removed for the sake of simplicity

    fn from_buffer(stream: impl std::io::Read) -> Result<Self, Self::Error>
    where
        Self: Sized;
    fn to_buffer(&self, stream: impl std::io::Write) -> Result<(), Self::Error>;
    fn buffer_length(&self) -> usize;
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

pub enum RdpPdu {
    X224(x224::Data),
    FastPath(fast_path::FastPathHeader),
}

impl PduParsing for RdpPdu {
    type Error = RdpError;

    fn from_buffer(mut stream: impl std::io::Read) -> Result<Self, Self::Error> {
        use bit_field::BitField;
        use byteorder::ReadBytesExt;
        use num_traits::FromPrimitive;

        let header = stream.read_u8()?;
        let action = header.get_bits(0..2);
        let action = Action::from_u8(action).ok_or(RdpError::InvalidActionCode(action))?;

        match action {
            Action::X224 => Ok(Self::X224(x224::Data::from_buffer_with_version(&mut stream, header)?)),
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

#[derive(Debug, failure::Fail)]
pub enum RdpError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] std::io::Error),
    #[fail(display = "X224 error: {}", _0)]
    X224Error(#[fail(cause)] nego::NegotiationError),
    #[fail(display = "Surface Commands error: {}", _0)]
    FastPathError(#[fail(cause)] fast_path::FastPathError),
    #[fail(display = "Received invalid action code: {}", _0)]
    InvalidActionCode(u8),
}

impl_from_error!(std::io::Error, RdpError, RdpError::IOError);
impl_from_error!(nego::NegotiationError, RdpError, RdpError::X224Error);
impl_from_error!(fast_path::FastPathError, RdpError, RdpError::FastPathError);

#[derive(Debug, Copy, Clone, PartialEq, Eq, num_derive::FromPrimitive, num_derive::ToPrimitive)]
pub enum Action {
    FastPath = 0x0,
    X224 = 0x3,
}
