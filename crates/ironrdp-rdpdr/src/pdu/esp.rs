//! PDUs for [\[MS-RDPESP\]: Remote Desktop Protocol: Serial and Parallel Port Virtual Channel Extension]
//!
//! MS-RDPESP defines no messages of its own. Port devices reuse the MS-RDPEFS
//! create, close, read, write and device-control requests, and the device-control
//! buffers are the native serial (`ntddser.h`) and parallel (`ntddpar.h`)
//! IOCTL structures. Clients that forward IOCTLs to a local driver can pass the
//! buffers through unchanged; the typed structures here describe the serial
//! buffers for clients that interpret or emulate them.
//!
//! [\[MS-RDPESP\]: Remote Desktop Protocol: Serial and Parallel Port Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesp/

use core::fmt;

use bitflags::bitflags;
use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, ensure_size,
};
use ironrdp_pdu::PduError;

use super::RdpdrPdu;
use super::efs::{
    DecodedDeviceControlRequest, DeviceCloseRequest, DeviceCloseResponse, DeviceControlRequest, DeviceControlResponse,
    DeviceCreateRequest, DeviceCreateResponse, DeviceFlushBuffersRequest, DeviceFlushBuffersResponse, DeviceIoRequest,
    DeviceIoResponse, DeviceReadRequest, DeviceReadResponse, DeviceWriteRequest, DeviceWriteResponse, Information,
    IoCtlCode, MajorFunction, NtStatus,
};

/// A serial or parallel port I/O control code.
///
/// [MS-RDPESP 2.2.2.6] lists the codes a port client can receive. Unknown codes
/// are retained, because the protocol treats IOCTLs as opaque and a client may
/// forward them to its local driver.
///
/// [MS-RDPESP 2.2.2.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesp/
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PortIoCtlCode(pub u32);

impl PortIoCtlCode {
    pub const IOCTL_SERIAL_SET_BAUD_RATE: Self = Self(0x001B_0004);
    pub const IOCTL_SERIAL_SET_QUEUE_SIZE: Self = Self(0x001B_0008);
    pub const IOCTL_SERIAL_SET_LINE_CONTROL: Self = Self(0x001B_000C);
    pub const IOCTL_SERIAL_SET_BREAK_ON: Self = Self(0x001B_0010);
    pub const IOCTL_SERIAL_SET_BREAK_OFF: Self = Self(0x001B_0014);
    pub const IOCTL_SERIAL_IMMEDIATE_CHAR: Self = Self(0x001B_0018);
    pub const IOCTL_SERIAL_SET_TIMEOUTS: Self = Self(0x001B_001C);
    pub const IOCTL_SERIAL_GET_TIMEOUTS: Self = Self(0x001B_0020);
    pub const IOCTL_SERIAL_SET_DTR: Self = Self(0x001B_0024);
    pub const IOCTL_SERIAL_CLR_DTR: Self = Self(0x001B_0028);
    pub const IOCTL_SERIAL_RESET_DEVICE: Self = Self(0x001B_002C);
    pub const IOCTL_SERIAL_SET_RTS: Self = Self(0x001B_0030);
    pub const IOCTL_SERIAL_CLR_RTS: Self = Self(0x001B_0034);
    pub const IOCTL_SERIAL_SET_XOFF: Self = Self(0x001B_0038);
    pub const IOCTL_SERIAL_SET_XON: Self = Self(0x001B_003C);
    pub const IOCTL_SERIAL_GET_WAIT_MASK: Self = Self(0x001B_0040);
    pub const IOCTL_SERIAL_SET_WAIT_MASK: Self = Self(0x001B_0044);
    pub const IOCTL_SERIAL_WAIT_ON_MASK: Self = Self(0x001B_0048);
    pub const IOCTL_SERIAL_PURGE: Self = Self(0x001B_004C);
    pub const IOCTL_SERIAL_GET_BAUD_RATE: Self = Self(0x001B_0050);
    pub const IOCTL_SERIAL_GET_LINE_CONTROL: Self = Self(0x001B_0054);
    pub const IOCTL_SERIAL_SET_CHARS: Self = Self(0x001B_0058);
    pub const IOCTL_SERIAL_GET_CHARS: Self = Self(0x001B_005C);
    pub const IOCTL_SERIAL_GET_HANDFLOW: Self = Self(0x001B_0060);
    pub const IOCTL_SERIAL_SET_HANDFLOW: Self = Self(0x001B_0064);
    pub const IOCTL_SERIAL_GET_MODEMSTATUS: Self = Self(0x001B_0068);
    pub const IOCTL_SERIAL_GET_COMMSTATUS: Self = Self(0x001B_006C);
    pub const IOCTL_SERIAL_XOFF_COUNTER: Self = Self(0x001B_0070);
    pub const IOCTL_SERIAL_GET_PROPERTIES: Self = Self(0x001B_0074);
    pub const IOCTL_SERIAL_GET_DTRRTS: Self = Self(0x001B_0078);
    pub const IOCTL_SERIAL_LSRMST_INSERT: Self = Self(0x001B_007C);
    pub const IOCTL_SERIAL_CONFIG_SIZE: Self = Self(0x001B_0080);
    pub const IOCTL_SERIAL_GET_STATS: Self = Self(0x001B_008C);
    pub const IOCTL_SERIAL_CLEAR_STATS: Self = Self(0x001B_0090);
    pub const IOCTL_SERIAL_GET_MODEM_CONTROL: Self = Self(0x001B_0094);
    pub const IOCTL_SERIAL_SET_MODEM_CONTROL: Self = Self(0x001B_0098);
    pub const IOCTL_SERIAL_SET_FIFO_CONTROL: Self = Self(0x001B_009C);
    pub const IOCTL_PAR_QUERY_INFORMATION: Self = Self(0x0016_0004);
    pub const IOCTL_PAR_SET_INFORMATION: Self = Self(0x0016_0008);
    pub const IOCTL_PAR_QUERY_DEVICE_ID: Self = Self(0x0016_000C);
    pub const IOCTL_PAR_QUERY_DEVICE_ID_SIZE: Self = Self(0x0016_0010);
    pub const IOCTL_IEEE1284_GET_MODE: Self = Self(0x0016_0014);
    pub const IOCTL_IEEE1284_NEGOTIATE: Self = Self(0x0016_0018);
    pub const IOCTL_PAR_SET_WRITE_ADDRESS: Self = Self(0x0016_001C);
    pub const IOCTL_PAR_SET_READ_ADDRESS: Self = Self(0x0016_0020);
    pub const IOCTL_PAR_GET_DEVICE_CAPS: Self = Self(0x0016_0024);
    pub const IOCTL_PAR_GET_DEFAULT_MODES: Self = Self(0x0016_0028);
    pub const IOCTL_PAR_QUERY_RAW_DEVICE_ID: Self = Self(0x0016_0030);
    pub const IOCTL_PAR_IS_PORT_FREE: Self = Self(0x0016_0054);

    /// Returns the `ntddser.h` / `ntddpar.h` name of a code listed by MS-RDPESP.
    pub fn name(self) -> Option<&'static str> {
        let name = match self {
            Self::IOCTL_SERIAL_SET_BAUD_RATE => "IOCTL_SERIAL_SET_BAUD_RATE",
            Self::IOCTL_SERIAL_SET_QUEUE_SIZE => "IOCTL_SERIAL_SET_QUEUE_SIZE",
            Self::IOCTL_SERIAL_SET_LINE_CONTROL => "IOCTL_SERIAL_SET_LINE_CONTROL",
            Self::IOCTL_SERIAL_SET_BREAK_ON => "IOCTL_SERIAL_SET_BREAK_ON",
            Self::IOCTL_SERIAL_SET_BREAK_OFF => "IOCTL_SERIAL_SET_BREAK_OFF",
            Self::IOCTL_SERIAL_IMMEDIATE_CHAR => "IOCTL_SERIAL_IMMEDIATE_CHAR",
            Self::IOCTL_SERIAL_SET_TIMEOUTS => "IOCTL_SERIAL_SET_TIMEOUTS",
            Self::IOCTL_SERIAL_GET_TIMEOUTS => "IOCTL_SERIAL_GET_TIMEOUTS",
            Self::IOCTL_SERIAL_SET_DTR => "IOCTL_SERIAL_SET_DTR",
            Self::IOCTL_SERIAL_CLR_DTR => "IOCTL_SERIAL_CLR_DTR",
            Self::IOCTL_SERIAL_RESET_DEVICE => "IOCTL_SERIAL_RESET_DEVICE",
            Self::IOCTL_SERIAL_SET_RTS => "IOCTL_SERIAL_SET_RTS",
            Self::IOCTL_SERIAL_CLR_RTS => "IOCTL_SERIAL_CLR_RTS",
            Self::IOCTL_SERIAL_SET_XOFF => "IOCTL_SERIAL_SET_XOFF",
            Self::IOCTL_SERIAL_SET_XON => "IOCTL_SERIAL_SET_XON",
            Self::IOCTL_SERIAL_GET_WAIT_MASK => "IOCTL_SERIAL_GET_WAIT_MASK",
            Self::IOCTL_SERIAL_SET_WAIT_MASK => "IOCTL_SERIAL_SET_WAIT_MASK",
            Self::IOCTL_SERIAL_WAIT_ON_MASK => "IOCTL_SERIAL_WAIT_ON_MASK",
            Self::IOCTL_SERIAL_PURGE => "IOCTL_SERIAL_PURGE",
            Self::IOCTL_SERIAL_GET_BAUD_RATE => "IOCTL_SERIAL_GET_BAUD_RATE",
            Self::IOCTL_SERIAL_GET_LINE_CONTROL => "IOCTL_SERIAL_GET_LINE_CONTROL",
            Self::IOCTL_SERIAL_SET_CHARS => "IOCTL_SERIAL_SET_CHARS",
            Self::IOCTL_SERIAL_GET_CHARS => "IOCTL_SERIAL_GET_CHARS",
            Self::IOCTL_SERIAL_GET_HANDFLOW => "IOCTL_SERIAL_GET_HANDFLOW",
            Self::IOCTL_SERIAL_SET_HANDFLOW => "IOCTL_SERIAL_SET_HANDFLOW",
            Self::IOCTL_SERIAL_GET_MODEMSTATUS => "IOCTL_SERIAL_GET_MODEMSTATUS",
            Self::IOCTL_SERIAL_GET_COMMSTATUS => "IOCTL_SERIAL_GET_COMMSTATUS",
            Self::IOCTL_SERIAL_XOFF_COUNTER => "IOCTL_SERIAL_XOFF_COUNTER",
            Self::IOCTL_SERIAL_GET_PROPERTIES => "IOCTL_SERIAL_GET_PROPERTIES",
            Self::IOCTL_SERIAL_GET_DTRRTS => "IOCTL_SERIAL_GET_DTRRTS",
            Self::IOCTL_SERIAL_LSRMST_INSERT => "IOCTL_SERIAL_LSRMST_INSERT",
            Self::IOCTL_SERIAL_CONFIG_SIZE => "IOCTL_SERIAL_CONFIG_SIZE",
            Self::IOCTL_SERIAL_GET_STATS => "IOCTL_SERIAL_GET_STATS",
            Self::IOCTL_SERIAL_CLEAR_STATS => "IOCTL_SERIAL_CLEAR_STATS",
            Self::IOCTL_SERIAL_GET_MODEM_CONTROL => "IOCTL_SERIAL_GET_MODEM_CONTROL",
            Self::IOCTL_SERIAL_SET_MODEM_CONTROL => "IOCTL_SERIAL_SET_MODEM_CONTROL",
            Self::IOCTL_SERIAL_SET_FIFO_CONTROL => "IOCTL_SERIAL_SET_FIFO_CONTROL",
            Self::IOCTL_PAR_QUERY_INFORMATION => "IOCTL_PAR_QUERY_INFORMATION",
            Self::IOCTL_PAR_SET_INFORMATION => "IOCTL_PAR_SET_INFORMATION",
            Self::IOCTL_PAR_QUERY_DEVICE_ID => "IOCTL_PAR_QUERY_DEVICE_ID",
            Self::IOCTL_PAR_QUERY_DEVICE_ID_SIZE => "IOCTL_PAR_QUERY_DEVICE_ID_SIZE",
            Self::IOCTL_IEEE1284_GET_MODE => "IOCTL_IEEE1284_GET_MODE",
            Self::IOCTL_IEEE1284_NEGOTIATE => "IOCTL_IEEE1284_NEGOTIATE",
            Self::IOCTL_PAR_SET_WRITE_ADDRESS => "IOCTL_PAR_SET_WRITE_ADDRESS",
            Self::IOCTL_PAR_SET_READ_ADDRESS => "IOCTL_PAR_SET_READ_ADDRESS",
            Self::IOCTL_PAR_GET_DEVICE_CAPS => "IOCTL_PAR_GET_DEVICE_CAPS",
            Self::IOCTL_PAR_GET_DEFAULT_MODES => "IOCTL_PAR_GET_DEFAULT_MODES",
            Self::IOCTL_PAR_QUERY_RAW_DEVICE_ID => "IOCTL_PAR_QUERY_RAW_DEVICE_ID",
            Self::IOCTL_PAR_IS_PORT_FREE => "IOCTL_PAR_IS_PORT_FREE",
            _ => return None,
        };
        Some(name)
    }

    /// Checks the buffer sizes and values the serial driver requires before the request
    /// reaches the device, as mstsc does.
    ///
    /// Too-short buffers fail with `STATUS_BUFFER_TOO_SMALL`. Timeouts that set the read
    /// interval, multiplier and constant all to `MAXULONG` fail with `STATUS_INVALID_PARAMETER`.
    /// Other codes are not checked.
    pub fn check_buffers(self, input: &[u8], output_buffer_length: u32) -> Result<(), NtStatus> {
        let required_input = match self {
            Self::IOCTL_SERIAL_SET_BAUD_RATE | Self::IOCTL_SERIAL_PURGE => size_of::<u32>(),
            Self::IOCTL_SERIAL_SET_QUEUE_SIZE => SerialQueueSize::SIZE,
            Self::IOCTL_SERIAL_SET_TIMEOUTS => SerialTimeouts::SIZE,
            _ => 0,
        };
        let required_output: u32 = match self {
            Self::IOCTL_SERIAL_WAIT_ON_MASK => 4,    // SERIAL_EV_* mask
            Self::IOCTL_SERIAL_GET_COMMSTATUS => 20, // SERIAL_STATUS with alignment padding
            _ => 0,
        };
        if input.len() < required_input || output_buffer_length < required_output {
            return Err(NtStatus::BUFFER_TOO_SMALL);
        }
        if self == Self::IOCTL_SERIAL_SET_TIMEOUTS {
            let timeouts =
                SerialTimeouts::decode(&mut ReadCursor::new(input)).map_err(|_| NtStatus::BUFFER_TOO_SMALL)?;
            if timeouts.read_interval_timeout == u32::MAX
                && timeouts.read_total_timeout_multiplier == u32::MAX
                && timeouts.read_total_timeout_constant == u32::MAX
            {
                return Err(NtStatus::INVALID_PARAMETER);
            }
        }
        Ok(())
    }

    /// Whether the code uses the serial device type (`FILE_DEVICE_SERIAL_PORT`).
    pub fn is_serial(self) -> bool {
        self.0 >> 16 == 0x001B
    }

    /// Whether the code uses the parallel device type (`FILE_DEVICE_PARALLEL_PORT`).
    pub fn is_parallel(self) -> bool {
        self.0 >> 16 == 0x0016
    }
}

impl fmt::Debug for PortIoCtlCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => write!(f, "PortIoCtlCode({:#010X})", self.0),
        }
    }
}

impl TryFrom<u32> for PortIoCtlCode {
    type Error = PduError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Ok(Self(value))
    }
}

impl From<PortIoCtlCode> for u32 {
    fn from(value: PortIoCtlCode) -> Self {
        value.0
    }
}

impl IoCtlCode for PortIoCtlCode {}

/// A server I/O request addressed to a serial or parallel port device.
///
/// [MS-RDPESP 2.2.2] describes create, close, read, write and device control.
/// Flush is also accepted: serial applications use it to wait until their writes
/// have been transmitted.
#[derive(Debug, PartialEq, Clone)]
pub enum PortIoRequest {
    /// DR_PORT_CREATE_REQ. The `Path` MUST be empty.
    Create(DeviceCreateRequest),
    /// DR_PORT_CLOSE_REQ. The client MUST cancel pending I/O on the file before completing it.
    Close(DeviceCloseRequest),
    /// DR_PORT_READ_REQ. `Offset` MUST be ignored, and partial reads are allowed.
    Read(DeviceReadRequest),
    /// DR_PORT_WRITE_REQ. `Offset` is ignored.
    Write(DeviceWriteRequest),
    FlushBuffers(DeviceFlushBuffersRequest),
    /// DR_PORT_CONTROL_REQ with its exact input buffer.
    DeviceControl(DecodedDeviceControlRequest<PortIoCtlCode>),
}

impl PortIoRequest {
    const CLOSE_PADDING_SIZE: usize = 32;

    /// Decodes a port request, or returns `Ok(Err(rejection))` for a request that must be
    /// completed with a failure status instead of reaching the backend:
    ///
    /// - a major function that ports do not implement (`STATUS_NOT_SUPPORTED`), whose body
    ///   is discarded because only the request kind defines its layout;
    /// - a device control whose InputBufferLength exceeds the PDU (`STATUS_INVALID_PARAMETER`).
    pub fn decode_or_unsupported(
        dev_io_req: DeviceIoRequest,
        src: &mut ReadCursor<'_>,
    ) -> DecodeResult<Result<Self, PortIoRejection>> {
        let req = match dev_io_req.major_function {
            MajorFunction::Create => Self::Create(DeviceCreateRequest::decode(dev_io_req, src)?),
            MajorFunction::Close => {
                ensure_size!(ctx: "DR_PORT_CLOSE_REQ", in: src, size: Self::CLOSE_PADDING_SIZE);
                src.advance(Self::CLOSE_PADDING_SIZE);
                Self::Close(DeviceCloseRequest::decode(dev_io_req))
            }
            MajorFunction::Read => Self::Read(DeviceReadRequest::decode(dev_io_req, src)?),
            MajorFunction::Write => Self::Write(DeviceWriteRequest::decode(dev_io_req, src)?),
            MajorFunction::FlushBuffers => {
                // DR_FLUSH_REQ carries 32 bytes of padding that MUST be ignored. Older servers
                // omit it, so it is not required here.
                src.advance(src.len());
                Self::FlushBuffers(DeviceFlushBuffersRequest::decode(dev_io_req))
            }
            MajorFunction::DeviceControl => {
                let request = DeviceControlRequest::<PortIoCtlCode>::decode(dev_io_req, src)?;
                let input_buffer_length = usize::try_from(request.input_buffer_length).unwrap_or(usize::MAX);
                if src.len() < input_buffer_length {
                    src.advance(src.len());
                    return Ok(Err(PortIoRejection {
                        device_io_request: request.header,
                        io_status: NtStatus::INVALID_PARAMETER,
                    }));
                }
                let input_buffer = src.read_slice(input_buffer_length).to_vec();
                Self::DeviceControl(DecodedDeviceControlRequest { request, input_buffer })
            }
            _ => {
                src.advance(src.len());
                return Ok(Err(PortIoRejection {
                    device_io_request: dev_io_req,
                    io_status: NtStatus::NOT_SUPPORTED,
                }));
            }
        };

        Ok(Ok(req))
    }

    /// Returns the common I/O request header.
    pub fn device_io_request(&self) -> &DeviceIoRequest {
        match self {
            Self::Create(req) => &req.device_io_request,
            Self::Close(req) => &req.device_io_request,
            Self::Read(req) => &req.device_io_request,
            Self::Write(req) => &req.device_io_request,
            Self::FlushBuffers(req) => &req.device_io_request,
            Self::DeviceControl(req) => &req.request.header,
        }
    }

    /// Builds the completion for a request that failed before reaching the device.
    ///
    /// The completion has the layout the server expects for the request kind, with no data.
    pub fn failure_response(&self, io_status: NtStatus) -> RdpdrPdu {
        let reply = DeviceIoResponse::new(self.device_io_request().clone(), io_status);
        match self {
            Self::Create(_) => RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
                device_io_reply: reply,
                file_id: 0,
                information: Information::empty(),
            }),
            Self::Close(_) => RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
                device_io_response: reply,
            }),
            Self::Read(_) => RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
                device_io_reply: reply,
                read_data: Vec::new(),
            }),
            Self::Write(_) => RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
                device_io_reply: reply,
                length: 0,
            }),
            Self::FlushBuffers(_) => RdpdrPdu::DeviceFlushBuffersResponse(DeviceFlushBuffersResponse {
                device_io_response: reply,
            }),
            Self::DeviceControl(_) => RdpdrPdu::DeviceControlResponse(DeviceControlResponse {
                device_io_reply: reply,
                output_buffer: None,
            }),
        }
    }
}

/// A port request completed by the channel before it reaches the backend.
#[derive(Debug, PartialEq, Clone)]
pub struct PortIoRejection {
    pub device_io_request: DeviceIoRequest,
    pub io_status: NtStatus,
}

bitflags! {
    /// Serial event mask used by IOCTL_SERIAL_SET_WAIT_MASK, IOCTL_SERIAL_GET_WAIT_MASK
    /// and IOCTL_SERIAL_WAIT_ON_MASK (`SERIAL_EV_*`).
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub struct SerialEventMask: u32 {
        const RXCHAR = 0x0001;
        const RXFLAG = 0x0002;
        const TXEMPTY = 0x0004;
        const CTS = 0x0008;
        const DSR = 0x0010;
        const RLSD = 0x0020;
        const BREAK = 0x0040;
        const ERR = 0x0080;
        const RING = 0x0100;
        const PERR = 0x0200;
        const RX80FULL = 0x0400;
        const EVENT1 = 0x0800;
        const EVENT2 = 0x1000;
    }
}

bitflags! {
    /// IOCTL_SERIAL_PURGE input (`SERIAL_PURGE_*`).
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub struct SerialPurgeMask: u32 {
        const TXABORT = 0x0000_0001;
        const RXABORT = 0x0000_0002;
        const TXCLEAR = 0x0000_0004;
        const RXCLEAR = 0x0000_0008;
    }
}

bitflags! {
    /// IOCTL_SERIAL_GET_MODEMSTATUS output, the modem status register (`SERIAL_MSR_*`).
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub struct SerialModemStatus: u32 {
        const DCTS = 0x01;
        const DDSR = 0x02;
        const TERI = 0x04;
        const DDCD = 0x08;
        const CTS = 0x10;
        const DSR = 0x20;
        const RI = 0x40;
        const DCD = 0x80;
    }
}

bitflags! {
    /// IOCTL_SERIAL_GET_DTRRTS output.
    #[derive(Debug, PartialEq, Eq, Clone, Copy)]
    pub struct SerialDtrRts: u32 {
        const DTR_STATE = 0x01;
        const RTS_STATE = 0x02;
    }
}

/// Implements [`Encode`] and [`Decode`] for a fixed-size serial buffer of consecutive fields.
macro_rules! fixed_serial_buffer {
    (
        $(#[$meta:meta])*
        $name:ident, $pdu_name:literal, padding: $padding:literal {
            $($(#[$field_meta:meta])* $field:ident: $ty:ident => $read:ident / $write:ident),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, PartialEq, Eq, Clone, Copy, Default)]
        pub struct $name {
            $($(#[$field_meta])* pub $field: $ty),+
        }

        impl $name {
            const NAME: &'static str = $pdu_name;
            const FIXED_PART_SIZE: usize = 0 $(+ size_of::<$ty>())+;
            /// Size the Windows driver returns, including trailing alignment padding.
            pub const SIZE: usize = Self::FIXED_PART_SIZE + $padding;
        }

        impl Encode for $name {
            fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
                ensure_size!(in: dst, size: Self::SIZE);
                $(dst.$write(self.$field);)+
                dst.write_array([0u8; $padding]);
                Ok(())
            }

            fn name(&self) -> &'static str {
                Self::NAME
            }

            fn size(&self) -> usize {
                Self::SIZE
            }
        }

        impl<'de> Decode<'de> for $name {
            /// Trailing alignment padding is optional and ignored when present.
            fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
                ensure_fixed_part_size!(in: src);
                let value = Self {
                    $($field: src.$read()),+
                };
                src.advance(src.len().min($padding));
                Ok(value)
            }
        }
    };
}

fixed_serial_buffer! {
    /// `SERIAL_BAUD_RATE`, used by IOCTL_SERIAL_SET_BAUD_RATE and IOCTL_SERIAL_GET_BAUD_RATE.
    SerialBaudRate, "SERIAL_BAUD_RATE", padding: 0 {
        baud_rate: u32 => read_u32 / write_u32,
    }
}

fixed_serial_buffer! {
    /// `SERIAL_LINE_CONTROL`, used by IOCTL_SERIAL_SET_LINE_CONTROL and IOCTL_SERIAL_GET_LINE_CONTROL.
    SerialLineControl, "SERIAL_LINE_CONTROL", padding: 0 {
        /// `STOP_BIT_1` (0), `STOP_BITS_1_5` (1) or `STOP_BITS_2` (2).
        stop_bits: u8 => read_u8 / write_u8,
        /// `NO_PARITY` (0), `ODD_PARITY` (1), `EVEN_PARITY` (2), `MARK_PARITY` (3) or `SPACE_PARITY` (4).
        parity: u8 => read_u8 / write_u8,
        /// Data bits per character, 5 to 8.
        word_length: u8 => read_u8 / write_u8,
    }
}

fixed_serial_buffer! {
    /// `SERIAL_TIMEOUTS`, used by IOCTL_SERIAL_SET_TIMEOUTS and IOCTL_SERIAL_GET_TIMEOUTS.
    ///
    /// These timeouts control how the client's driver completes reads and writes.
    SerialTimeouts, "SERIAL_TIMEOUTS", padding: 0 {
        read_interval_timeout: u32 => read_u32 / write_u32,
        read_total_timeout_multiplier: u32 => read_u32 / write_u32,
        read_total_timeout_constant: u32 => read_u32 / write_u32,
        write_total_timeout_multiplier: u32 => read_u32 / write_u32,
        write_total_timeout_constant: u32 => read_u32 / write_u32,
    }
}

fixed_serial_buffer! {
    /// `SERIAL_CHARS`, used by IOCTL_SERIAL_SET_CHARS and IOCTL_SERIAL_GET_CHARS.
    SerialChars, "SERIAL_CHARS", padding: 0 {
        eof_char: u8 => read_u8 / write_u8,
        error_char: u8 => read_u8 / write_u8,
        break_char: u8 => read_u8 / write_u8,
        event_char: u8 => read_u8 / write_u8,
        xon_char: u8 => read_u8 / write_u8,
        xoff_char: u8 => read_u8 / write_u8,
    }
}

fixed_serial_buffer! {
    /// `SERIAL_HANDFLOW`, used by IOCTL_SERIAL_SET_HANDFLOW and IOCTL_SERIAL_GET_HANDFLOW.
    SerialHandflow, "SERIAL_HANDFLOW", padding: 0 {
        /// `SERIAL_DTR_*`, `SERIAL_CTS_HANDSHAKE`, `SERIAL_DSR_*` and `SERIAL_ERROR_ABORT` bits.
        control_handshake: u32 => read_u32 / write_u32,
        /// `SERIAL_AUTO_*`, `SERIAL_RTS_*` and related flow replacement bits.
        flow_replace: u32 => read_u32 / write_u32,
        xon_limit: i32 => read_i32 / write_i32,
        xoff_limit: i32 => read_i32 / write_i32,
    }
}

fixed_serial_buffer! {
    /// `SERIAL_QUEUE_SIZE`, used by IOCTL_SERIAL_SET_QUEUE_SIZE.
    SerialQueueSize, "SERIAL_QUEUE_SIZE", padding: 0 {
        in_size: u32 => read_u32 / write_u32,
        out_size: u32 => read_u32 / write_u32,
    }
}

fixed_serial_buffer! {
    /// `SERIAL_STATUS`, returned by IOCTL_SERIAL_GET_COMMSTATUS.
    SerialStatus, "SERIAL_STATUS", padding: 2 {
        /// `SERIAL_ERROR_*` bits.
        errors: u32 => read_u32 / write_u32,
        /// `SERIAL_TX_WAITING_*` bits.
        hold_reasons: u32 => read_u32 / write_u32,
        amount_in_in_queue: u32 => read_u32 / write_u32,
        amount_in_out_queue: u32 => read_u32 / write_u32,
        eof_received: u8 => read_u8 / write_u8,
        wait_for_immediate: u8 => read_u8 / write_u8,
    }
}

fixed_serial_buffer! {
    /// `SERIAL_COMMPROP`, returned by IOCTL_SERIAL_GET_PROPERTIES.
    SerialCommProp, "SERIAL_COMMPROP", padding: 2 {
        packet_length: u16 => read_u16 / write_u16,
        packet_version: u16 => read_u16 / write_u16,
        service_mask: u32 => read_u32 / write_u32,
        reserved1: u32 => read_u32 / write_u32,
        max_tx_queue: u32 => read_u32 / write_u32,
        max_rx_queue: u32 => read_u32 / write_u32,
        max_baud: u32 => read_u32 / write_u32,
        prov_sub_type: u32 => read_u32 / write_u32,
        prov_capabilities: u32 => read_u32 / write_u32,
        settable_params: u32 => read_u32 / write_u32,
        settable_baud: u32 => read_u32 / write_u32,
        settable_data: u16 => read_u16 / write_u16,
        settable_stop_parity: u16 => read_u16 / write_u16,
        current_tx_queue: u32 => read_u32 / write_u32,
        current_rx_queue: u32 => read_u32 / write_u32,
        prov_spec1: u32 => read_u32 / write_u32,
        prov_spec2: u32 => read_u32 / write_u32,
        prov_char: u16 => read_u16 / write_u16,
    }
}

fixed_serial_buffer! {
    /// `SERIALPERF_STATS`, returned by IOCTL_SERIAL_GET_STATS.
    SerialPerfStats, "SERIALPERF_STATS", padding: 0 {
        received_count: u32 => read_u32 / write_u32,
        transmitted_count: u32 => read_u32 / write_u32,
        frame_error_count: u32 => read_u32 / write_u32,
        serial_overrun_error_count: u32 => read_u32 / write_u32,
        buffer_overrun_error_count: u32 => read_u32 / write_u32,
        parity_error_count: u32 => read_u32 / write_u32,
    }
}

fixed_serial_buffer! {
    /// `SERIAL_XOFF_COUNTER`, the input of IOCTL_SERIAL_XOFF_COUNTER.
    SerialXoffCounter, "SERIAL_XOFF_COUNTER", padding: 3 {
        timeout: u32 => read_u32 / write_u32,
        counter: i32 => read_i32 / write_i32,
        xoff_char: u8 => read_u8 / write_u8,
    }
}

/// The structured input of a serial device-control request.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SerialIoCtlInput {
    SetBaudRate(SerialBaudRate),
    SetQueueSize(SerialQueueSize),
    SetLineControl(SerialLineControl),
    ImmediateChar(u8),
    SetTimeouts(SerialTimeouts),
    SetWaitMask(SerialEventMask),
    Purge(SerialPurgeMask),
    SetChars(SerialChars),
    SetHandflow(SerialHandflow),
    XoffCounter(SerialXoffCounter),
    /// `SERIAL_LSRMST_*` escape character, or zero to disable insertion.
    LsrMstInsert(u8),
    SetModemControl(u32),
    SetFifoControl(u32),
}

impl SerialIoCtlInput {
    /// Decodes the input buffer of a serial IOCTL that takes one.
    ///
    /// Returns `Ok(None)` for codes without structured input, including parallel and unknown codes.
    pub fn decode(code: PortIoCtlCode, input: &[u8]) -> DecodeResult<Option<Self>> {
        let src = &mut ReadCursor::new(input);
        let input = match code {
            PortIoCtlCode::IOCTL_SERIAL_SET_BAUD_RATE => Self::SetBaudRate(SerialBaudRate::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_SET_QUEUE_SIZE => Self::SetQueueSize(SerialQueueSize::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_SET_LINE_CONTROL => Self::SetLineControl(SerialLineControl::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_IMMEDIATE_CHAR => Self::ImmediateChar(read_u8(src)?),
            PortIoCtlCode::IOCTL_SERIAL_SET_TIMEOUTS => Self::SetTimeouts(SerialTimeouts::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_SET_WAIT_MASK => {
                Self::SetWaitMask(SerialEventMask::from_bits_retain(read_u32(src)?))
            }
            PortIoCtlCode::IOCTL_SERIAL_PURGE => Self::Purge(SerialPurgeMask::from_bits_retain(read_u32(src)?)),
            PortIoCtlCode::IOCTL_SERIAL_SET_CHARS => Self::SetChars(SerialChars::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_SET_HANDFLOW => Self::SetHandflow(SerialHandflow::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_XOFF_COUNTER => Self::XoffCounter(SerialXoffCounter::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_LSRMST_INSERT => Self::LsrMstInsert(read_u8(src)?),
            PortIoCtlCode::IOCTL_SERIAL_SET_MODEM_CONTROL => Self::SetModemControl(read_u32(src)?),
            PortIoCtlCode::IOCTL_SERIAL_SET_FIFO_CONTROL => Self::SetFifoControl(read_u32(src)?),
            _ => return Ok(None),
        };

        Ok(Some(input))
    }

    /// Returns the IOCTL that carries this input.
    pub fn io_control_code(&self) -> PortIoCtlCode {
        match self {
            Self::SetBaudRate(_) => PortIoCtlCode::IOCTL_SERIAL_SET_BAUD_RATE,
            Self::SetQueueSize(_) => PortIoCtlCode::IOCTL_SERIAL_SET_QUEUE_SIZE,
            Self::SetLineControl(_) => PortIoCtlCode::IOCTL_SERIAL_SET_LINE_CONTROL,
            Self::ImmediateChar(_) => PortIoCtlCode::IOCTL_SERIAL_IMMEDIATE_CHAR,
            Self::SetTimeouts(_) => PortIoCtlCode::IOCTL_SERIAL_SET_TIMEOUTS,
            Self::SetWaitMask(_) => PortIoCtlCode::IOCTL_SERIAL_SET_WAIT_MASK,
            Self::Purge(_) => PortIoCtlCode::IOCTL_SERIAL_PURGE,
            Self::SetChars(_) => PortIoCtlCode::IOCTL_SERIAL_SET_CHARS,
            Self::SetHandflow(_) => PortIoCtlCode::IOCTL_SERIAL_SET_HANDFLOW,
            Self::XoffCounter(_) => PortIoCtlCode::IOCTL_SERIAL_XOFF_COUNTER,
            Self::LsrMstInsert(_) => PortIoCtlCode::IOCTL_SERIAL_LSRMST_INSERT,
            Self::SetModemControl(_) => PortIoCtlCode::IOCTL_SERIAL_SET_MODEM_CONTROL,
            Self::SetFifoControl(_) => PortIoCtlCode::IOCTL_SERIAL_SET_FIFO_CONTROL,
        }
    }
}

impl Encode for SerialIoCtlInput {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::SetBaudRate(value) => value.encode(dst),
            Self::SetQueueSize(value) => value.encode(dst),
            Self::SetLineControl(value) => value.encode(dst),
            Self::SetTimeouts(value) => value.encode(dst),
            Self::SetChars(value) => value.encode(dst),
            Self::SetHandflow(value) => value.encode(dst),
            Self::XoffCounter(value) => value.encode(dst),
            Self::ImmediateChar(value) | Self::LsrMstInsert(value) => {
                dst.write_u8(*value);
                Ok(())
            }
            Self::SetWaitMask(value) => {
                dst.write_u32(value.bits());
                Ok(())
            }
            Self::Purge(value) => {
                dst.write_u32(value.bits());
                Ok(())
            }
            Self::SetModemControl(value) | Self::SetFifoControl(value) => {
                dst.write_u32(*value);
                Ok(())
            }
        }
    }

    fn name(&self) -> &'static str {
        "SerialIoCtlInput"
    }

    fn size(&self) -> usize {
        match self {
            Self::SetBaudRate(value) => value.size(),
            Self::SetQueueSize(value) => value.size(),
            Self::SetLineControl(value) => value.size(),
            Self::SetTimeouts(value) => value.size(),
            Self::SetChars(value) => value.size(),
            Self::SetHandflow(value) => value.size(),
            Self::XoffCounter(value) => value.size(),
            Self::ImmediateChar(_) | Self::LsrMstInsert(_) => size_of::<u8>(),
            Self::SetWaitMask(_) | Self::Purge(_) | Self::SetModemControl(_) | Self::SetFifoControl(_) => {
                size_of::<u32>()
            }
        }
    }
}

/// The structured output of a serial device-control request.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SerialIoCtlOutput {
    BaudRate(SerialBaudRate),
    LineControl(SerialLineControl),
    Timeouts(SerialTimeouts),
    Chars(SerialChars),
    Handflow(SerialHandflow),
    WaitMask(SerialEventMask),
    /// The events that completed IOCTL_SERIAL_WAIT_ON_MASK.
    WaitOnMask(SerialEventMask),
    ModemStatus(SerialModemStatus),
    DtrRts(SerialDtrRts),
    CommStatus(SerialStatus),
    Properties(SerialCommProp),
    Stats(SerialPerfStats),
    ConfigSize(u32),
    ModemControl(u32),
}

impl SerialIoCtlOutput {
    /// Decodes the output buffer returned for a serial IOCTL that produces one.
    ///
    /// Returns `Ok(None)` for codes without structured output, including parallel and unknown codes.
    pub fn decode(code: PortIoCtlCode, output: &[u8]) -> DecodeResult<Option<Self>> {
        let src = &mut ReadCursor::new(output);
        let output = match code {
            PortIoCtlCode::IOCTL_SERIAL_GET_BAUD_RATE => Self::BaudRate(SerialBaudRate::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_GET_LINE_CONTROL => Self::LineControl(SerialLineControl::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_GET_TIMEOUTS => Self::Timeouts(SerialTimeouts::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_GET_CHARS => Self::Chars(SerialChars::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_GET_HANDFLOW => Self::Handflow(SerialHandflow::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_GET_WAIT_MASK => {
                Self::WaitMask(SerialEventMask::from_bits_retain(read_u32(src)?))
            }
            PortIoCtlCode::IOCTL_SERIAL_WAIT_ON_MASK => {
                Self::WaitOnMask(SerialEventMask::from_bits_retain(read_u32(src)?))
            }
            PortIoCtlCode::IOCTL_SERIAL_GET_MODEMSTATUS => {
                Self::ModemStatus(SerialModemStatus::from_bits_retain(read_u32(src)?))
            }
            PortIoCtlCode::IOCTL_SERIAL_GET_DTRRTS => Self::DtrRts(SerialDtrRts::from_bits_retain(read_u32(src)?)),
            PortIoCtlCode::IOCTL_SERIAL_GET_COMMSTATUS => Self::CommStatus(SerialStatus::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_GET_PROPERTIES => Self::Properties(SerialCommProp::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_GET_STATS => Self::Stats(SerialPerfStats::decode(src)?),
            PortIoCtlCode::IOCTL_SERIAL_CONFIG_SIZE => Self::ConfigSize(read_u32(src)?),
            PortIoCtlCode::IOCTL_SERIAL_GET_MODEM_CONTROL => Self::ModemControl(read_u32(src)?),
            _ => return Ok(None),
        };

        Ok(Some(output))
    }
}

impl Encode for SerialIoCtlOutput {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::BaudRate(value) => value.encode(dst),
            Self::LineControl(value) => value.encode(dst),
            Self::Timeouts(value) => value.encode(dst),
            Self::Chars(value) => value.encode(dst),
            Self::Handflow(value) => value.encode(dst),
            Self::CommStatus(value) => value.encode(dst),
            Self::Properties(value) => value.encode(dst),
            Self::Stats(value) => value.encode(dst),
            Self::WaitMask(value) | Self::WaitOnMask(value) => {
                dst.write_u32(value.bits());
                Ok(())
            }
            Self::ModemStatus(value) => {
                dst.write_u32(value.bits());
                Ok(())
            }
            Self::DtrRts(value) => {
                dst.write_u32(value.bits());
                Ok(())
            }
            Self::ConfigSize(value) | Self::ModemControl(value) => {
                dst.write_u32(*value);
                Ok(())
            }
        }
    }

    fn name(&self) -> &'static str {
        "SerialIoCtlOutput"
    }

    fn size(&self) -> usize {
        match self {
            Self::BaudRate(value) => value.size(),
            Self::LineControl(value) => value.size(),
            Self::Timeouts(value) => value.size(),
            Self::Chars(value) => value.size(),
            Self::Handflow(value) => value.size(),
            Self::CommStatus(value) => value.size(),
            Self::Properties(value) => value.size(),
            Self::Stats(value) => value.size(),
            Self::WaitMask(_)
            | Self::WaitOnMask(_)
            | Self::ModemStatus(_)
            | Self::DtrRts(_)
            | Self::ConfigSize(_)
            | Self::ModemControl(_) => size_of::<u32>(),
        }
    }
}

fn read_u8(src: &mut ReadCursor<'_>) -> DecodeResult<u8> {
    ensure_size!(in: src, size: size_of::<u8>());
    Ok(src.read_u8())
}

fn read_u32(src: &mut ReadCursor<'_>) -> DecodeResult<u32> {
    ensure_size!(in: src, size: size_of::<u32>());
    Ok(src.read_u32())
}
