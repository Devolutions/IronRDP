//! Protocol-independent USB transfer requests and completions.
//!
//! These are deliberately plain data structures. They describe USB operations
//! without parsing transport packets, allocating buffers, resolving endpoint
//! state, or managing request lifetimes.
//!
//! The four transfer types modelled here are defined by [USB 2.0] 5.5 "Control
//! Transfers", 5.6 "Isochronous Transfers", 5.7 "Interrupt Transfers", and 5.8
//! "Bulk Transfers".
//!
//! [USB 2.0]: https://www.usb.org/document-library/usb-20-specification

use core::fmt;

use super::{control::SetupPacket, endpoint::EndpointAddress};

/// Non-success outcome of a submitted USB operation.
///
/// This is unrelated to the payload returned by the USB standard `GET_STATUS`
/// request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UsbError {
    Cancelled,
    Stall,
    Timeout,
    Overflow,
    NoDevice,
    Error,
}

impl fmt::Display for UsbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "cancelled",
            Self::Stall => "stall",
            Self::Timeout => "timeout",
            Self::Overflow => "overflow",
            Self::NoDevice => "no-device",
            Self::Error => "error",
        })
    }
}

impl core::error::Error for UsbError {}

/// Result of a USB operation.
///
/// For an all-or-nothing operation, `Ok` carries the operation's output.
/// Operations which can fail while still producing data, such as
/// [`TransferCompletion`], report a `UsbResult<()>` status alongside their
/// payload instead.
pub type UsbResult<T> = Result<T, UsbError>;

/// One default-control-pipe transfer.
///
/// Direction and requested length are carried by `setup`. For an IN transfer,
/// `data` is empty. For an OUT transfer, it contains the data stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ControlTransferRequest<B> {
    pub setup: SetupPacket,
    pub data: B,
}

/// One bulk or interrupt transfer on a non-control endpoint.
///
/// Direction is carried by `endpoint`. For an IN endpoint, `length` is the
/// maximum requested response and `data` is empty. For an OUT endpoint,
/// `length` describes `data`.
///
/// INVARIANT: for an OUT transfer, `length` equals the `data` length.
/// Transports validate this at translation time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DataTransferRequest<B> {
    pub endpoint: EndpointAddress,
    pub length: u32,
    pub data: B,
}

/// A bulk transfer request.
///
/// Whether the active endpoint is actually bulk is device state checked by the
/// handle, not a property duplicated in this transport-independent type.
pub type BulkTransferRequest<B> = DataTransferRequest<B>;

/// An interrupt transfer request.
///
/// Whether the active endpoint is actually interrupt is device state checked
/// by the handle.
pub type InterruptTransferRequest<B> = DataTransferRequest<B>;

/// Completion of a control, bulk, or interrupt transfer.
///
/// For an IN transfer, `data` contains the received bytes. For an OUT transfer,
/// it is empty. `actual_length` is valid in both directions.
///
/// INVARIANT: for an IN transfer, `actual_length` equals `data` length; the
/// field carries independent information only for OUT transfers.
///
/// A failed transfer can still carry partial data, so `status` is reported
/// alongside the payload rather than replacing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransferCompletion<B> {
    pub status: UsbResult<()>,
    pub actual_length: u32,
    pub data: B,
}

pub type ControlCompletion<B> = TransferCompletion<B>;
pub type DataCompletion<B> = TransferCompletion<B>;

/// Host-controller frame number used for isochronous scheduling.
pub type FrameNumber = u32;

/// Result of one packet in an isochronous transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IsochronousPacketCompletion {
    pub status: UsbResult<()>,
    pub actual_length: u32,
}

/// One isochronous transfer containing one or more packets.
///
/// `start_frame == None` asks the host controller to schedule the transfer as
/// soon as possible. Packet payload slots are packed in request order. For an
/// IN endpoint `data` is empty; for an OUT endpoint it contains the packet
/// payloads in the same order. Each item in `packets` is a `u32` requested
/// packet length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IsochronousTransferRequest<B, P> {
    pub endpoint: EndpointAddress,
    pub start_frame: Option<FrameNumber>,
    pub data: B,
    pub packets: P,
}

/// Direction-independent output of an isochronous transfer.
///
/// For IN, successful packet payloads are concatenated in packet order in
/// `data`; failed packets contribute no bytes. The packet `actual_length`
/// values split the buffer. For OUT, `data` is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IsochronousTransferOutput<B, P> {
    pub start_frame: FrameNumber,
    pub actual_length: u32,
    pub data: B,
    pub packets: P,
}

pub type IsoCompletion<B, P> = UsbResult<IsochronousTransferOutput<B, P>>;
