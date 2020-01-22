pub mod codecs;
pub mod gcc;
pub mod mcs;
pub mod nego;
pub mod rdp;

mod ber;
mod fast_path;
mod per;
mod utils;
mod x224;

pub use crate::{
    fast_path::{parse_fast_path_header, FastPath, FastPathError},
    mcs::{ConnectInitial, ConnectResponse, McsError, McsPdu, SendDataContext},
    nego::*,
    rdp::{
        vc::dvc, CapabilitySet, ClientConfirmActive, ClientInfoPdu, ControlAction, DemandActive,
        ServerDemandActive, ShareControlHeader, ShareControlPdu, ShareDataHeader, ShareDataPdu,
        VirtualChannel,
    },
    x224::*,
};

pub trait PduParsing {
    type Error;

    fn from_buffer(stream: impl std::io::Read) -> Result<Self, Self::Error>
    where
        Self: Sized;
    fn to_buffer(&self, stream: impl std::io::Write) -> Result<(), Self::Error>;
    fn buffer_length(&self) -> usize;
}

pub trait PduBufferParsing: Sized {
    type Error;

    fn from_buffer(buffer: &[u8]) -> Result<Self, Self::Error>;
    fn to_buffer(&self, buffer: &mut [u8]) -> Result<(), Self::Error>;
    fn buffer_length(&self) -> usize;
}
