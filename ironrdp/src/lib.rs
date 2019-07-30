#[macro_use]
mod utils;

pub mod gcc;

mod fast_path;
mod mcs;
mod nego;
mod per;
mod rdp;
mod tpdu;

pub use crate::{
    fast_path::{parse_fast_path_header, FastPath, FastPathError},
    mcs::{ConnectInitial, ConnectResponse, McsError, McsPdu, SendDataContext},
    nego::*,
    rdp::{
        CapabilitySet, ClientConfirmActive, ClientInfoPdu, ClientLicensePdu, ControlAction,
        DemandActive, ServerDemandActive, ShareControlHeader, ShareControlPdu, ShareDataHeader,
        ShareDataPdu,
    },
    tpdu::*,
};

pub trait PduParsing {
    type Error;

    fn from_buffer(stream: impl std::io::Read) -> Result<Self, Self::Error>
    where
        Self: std::marker::Sized;
    fn to_buffer(&self, stream: impl std::io::Write) -> Result<(), Self::Error>;
    fn buffer_length(&self) -> usize;
}
