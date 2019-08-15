pub mod gcc;
pub mod mcs;
pub mod nego;
pub mod rdp;

mod fast_path;
mod per;
mod x224;
mod utils;

pub use crate::{
    fast_path::{parse_fast_path_header, FastPath, FastPathError},
    mcs::{ConnectInitial, ConnectResponse, McsError, McsPdu, SendDataContext},
    nego::*,
    rdp::{
        CapabilitySet, ClientConfirmActive, ClientInfoPdu, ControlAction, DemandActive,
        ServerDemandActive, ServerLicensePdu, ShareControlHeader, ShareControlPdu, ShareDataHeader,
        ShareDataPdu, VirtualChannel,
    },
    x224::*,
};

pub trait PduParsing {
    type Error;

    fn from_buffer(stream: impl std::io::Read) -> Result<Self, Self::Error>
    where
        Self: std::marker::Sized;
    fn to_buffer(&self, stream: impl std::io::Write) -> Result<(), Self::Error>;
    fn buffer_length(&self) -> usize;
}
