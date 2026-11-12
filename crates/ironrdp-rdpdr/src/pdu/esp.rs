//! PDUs for [\[MS-RDPESP\]: Remote Desktop Protocol: Serial and Parallel Port Virtual Channel Extension]
//!
//! MS-RDPESP defines no messages of its own. Port devices reuse the MS-RDPEFS
//! create, close, read, write and device-control requests, and the device-control
//! buffers are the native serial (`ntddser.h`) and parallel (`ntddpar.h`)
//! IOCTL structures. Clients that forward IOCTLs to a local driver can pass the
//! buffers through unchanged, after [`PortIoCtlCode::check_buffers`]; the few
//! serial structures here are the ones those checks need.
//!
//! [\[MS-RDPESP\]: Remote Desktop Protocol: Serial and Parallel Port Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesp/

use core::fmt;

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

/// Defines the MS-RDPESP control codes and their `ntddser.h` / `ntddpar.h` names from one table.
macro_rules! port_ioctl_codes {
    ($($name:ident = $value:literal),+ $(,)?) => {
        $(pub const $name: Self = Self($value);)+

        /// Returns the `ntddser.h` / `ntddpar.h` name of a code listed by MS-RDPESP.
        pub fn name(self) -> Option<&'static str> {
            match self {
                $(Self::$name => Some(stringify!($name)),)+
                _ => None,
            }
        }
    };
}

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
    port_ioctl_codes! {
        IOCTL_SERIAL_SET_BAUD_RATE = 0x001B_0004,
        IOCTL_SERIAL_SET_QUEUE_SIZE = 0x001B_0008,
        IOCTL_SERIAL_SET_LINE_CONTROL = 0x001B_000C,
        IOCTL_SERIAL_SET_BREAK_ON = 0x001B_0010,
        IOCTL_SERIAL_SET_BREAK_OFF = 0x001B_0014,
        IOCTL_SERIAL_IMMEDIATE_CHAR = 0x001B_0018,
        IOCTL_SERIAL_SET_TIMEOUTS = 0x001B_001C,
        IOCTL_SERIAL_GET_TIMEOUTS = 0x001B_0020,
        IOCTL_SERIAL_SET_DTR = 0x001B_0024,
        IOCTL_SERIAL_CLR_DTR = 0x001B_0028,
        IOCTL_SERIAL_RESET_DEVICE = 0x001B_002C,
        IOCTL_SERIAL_SET_RTS = 0x001B_0030,
        IOCTL_SERIAL_CLR_RTS = 0x001B_0034,
        IOCTL_SERIAL_SET_XOFF = 0x001B_0038,
        IOCTL_SERIAL_SET_XON = 0x001B_003C,
        IOCTL_SERIAL_GET_WAIT_MASK = 0x001B_0040,
        IOCTL_SERIAL_SET_WAIT_MASK = 0x001B_0044,
        IOCTL_SERIAL_WAIT_ON_MASK = 0x001B_0048,
        IOCTL_SERIAL_PURGE = 0x001B_004C,
        IOCTL_SERIAL_GET_BAUD_RATE = 0x001B_0050,
        IOCTL_SERIAL_GET_LINE_CONTROL = 0x001B_0054,
        IOCTL_SERIAL_SET_CHARS = 0x001B_0058,
        IOCTL_SERIAL_GET_CHARS = 0x001B_005C,
        IOCTL_SERIAL_GET_HANDFLOW = 0x001B_0060,
        IOCTL_SERIAL_SET_HANDFLOW = 0x001B_0064,
        IOCTL_SERIAL_GET_MODEMSTATUS = 0x001B_0068,
        IOCTL_SERIAL_GET_COMMSTATUS = 0x001B_006C,
        IOCTL_SERIAL_XOFF_COUNTER = 0x001B_0070,
        IOCTL_SERIAL_GET_PROPERTIES = 0x001B_0074,
        IOCTL_SERIAL_GET_DTRRTS = 0x001B_0078,
        IOCTL_SERIAL_LSRMST_INSERT = 0x001B_007C,
        IOCTL_SERIAL_CONFIG_SIZE = 0x001B_0080,
        IOCTL_SERIAL_GET_STATS = 0x001B_008C,
        IOCTL_SERIAL_CLEAR_STATS = 0x001B_0090,
        IOCTL_SERIAL_GET_MODEM_CONTROL = 0x001B_0094,
        IOCTL_SERIAL_SET_MODEM_CONTROL = 0x001B_0098,
        IOCTL_SERIAL_SET_FIFO_CONTROL = 0x001B_009C,
        IOCTL_PAR_QUERY_INFORMATION = 0x0016_0004,
        IOCTL_PAR_SET_INFORMATION = 0x0016_0008,
        IOCTL_PAR_QUERY_DEVICE_ID = 0x0016_000C,
        IOCTL_PAR_QUERY_DEVICE_ID_SIZE = 0x0016_0010,
        IOCTL_IEEE1284_GET_MODE = 0x0016_0014,
        IOCTL_IEEE1284_NEGOTIATE = 0x0016_0018,
        IOCTL_PAR_SET_WRITE_ADDRESS = 0x0016_001C,
        IOCTL_PAR_SET_READ_ADDRESS = 0x0016_0020,
        IOCTL_PAR_GET_DEVICE_CAPS = 0x0016_0024,
        IOCTL_PAR_GET_DEFAULT_MODES = 0x0016_0028,
        IOCTL_PAR_QUERY_RAW_DEVICE_ID = 0x0016_0030,
        IOCTL_PAR_IS_PORT_FREE = 0x0016_0054,
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
        let required_output = match self {
            Self::IOCTL_SERIAL_WAIT_ON_MASK => size_of::<u32>(), // SERIAL_EV_* mask
            Self::IOCTL_SERIAL_GET_COMMSTATUS => SerialStatus::SIZE,
            _ => 0,
        };
        let output_buffer_length = usize::try_from(output_buffer_length).unwrap_or(usize::MAX);
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
