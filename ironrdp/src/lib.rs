#[macro_use]
mod utils;

pub mod gcc;
pub mod nego;

mod fast_path;
mod mcs;
mod per;
mod rdp;
mod x224;

pub use crate::{
    fast_path::{parse_fast_path_header, FastPath, FastPathError},
    mcs::{ConnectInitial, ConnectResponse, McsError, McsPdu, SendDataContext},
    nego::*,
    rdp::{
        CapabilitySet, ClientConfirmActive, ClientInfoPdu, ClientLicensePdu, ControlAction,
        DemandActive, ServerDemandActive, ShareControlHeader, ShareControlPdu, ShareDataHeader,
        ShareDataPdu,
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
