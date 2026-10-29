//! PNP Device I/O Subprotocol messages carried on `FileRedirectorChannel` ([MS-RDPEPNP] section 2.2.2).
//!
//! Server requests share the `SERVER_IO_HEADER` and decode as one [`ServerIoPdu`].
//! Client replies carry only a request ID, so each reply type decodes on its own.
//!
//! [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/

use alloc::vec::Vec;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_size, invalid_field_err,
    unsupported_value_err,
};
use ironrdp_dvc::DvcEncode;

use super::{Guid, HResult};

/// `READ_REQUEST` function ID.
pub const READ_REQUEST: u32 = 0x0000_0000;
/// `WRITE_REQUEST` function ID.
pub const WRITE_REQUEST: u32 = 0x0000_0001;
/// `IOCONTROL_REQUEST` function ID.
pub const IOCONTROL_REQUEST: u32 = 0x0000_0002;
/// `CREATE_FILE_REQUEST` function ID.
pub const CREATE_FILE_REQUEST: u32 = 0x0000_0004;
/// `CAPABILITIES_REQUEST` function ID.
pub const CAPABILITIES_REQUEST: u32 = 0x0000_0005;
/// `SPECIFIC_IOCANCEL_REQUEST` function ID.
pub const SPECIFIC_IOCANCEL_REQUEST: u32 = 0x0000_0006;

/// `RESPONSE_PACKET` packet type of a reply to a server request.
pub const RESPONSE_PACKET: u8 = 0x00;
/// `CUSTOM_EVENT_PACKET` packet type of a client custom event.
pub const CUSTOM_EVENT_PACKET: u8 = 0x01;

/// Capabilities version without custom event redirection.
pub const VERSION_WITHOUT_CUSTOM_EVENTS: u16 = 0x0004;
/// Capabilities version with custom event redirection.
pub const VERSION_WITH_CUSTOM_EVENTS: u16 = 0x0006;

const SERVER_HEADER_SIZE: usize = 4 /* RequestId, UnusedBits */ + 4 /* FunctionId */;
const CLIENT_HEADER_SIZE: usize = 4 /* RequestId, PacketType */;
const REQUEST_ID_MASK: u32 = 0x00FF_FFFF;

/// 24-bit request ID of the `SERVER_IO_HEADER` and `CLIENT_IO_HEADER` ([MS-RDPEPNP] section 2.2.2.1).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(u32);

impl RequestId {
    pub const MAX: u32 = REQUEST_ID_MASK;

    /// Returns `None` when `value` does not fit in 24 bits.
    pub const fn new(value: u32) -> Option<Self> {
        if value <= Self::MAX { Some(Self(value)) } else { None }
    }

    pub const fn get(self) -> u32 {
        self.0
    }

    fn from_low_bits(value: u32) -> Self {
        Self(value & REQUEST_ID_MASK)
    }
}

/// A server-to-client request of the PNP Device I/O Subprotocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerIoPdu {
    /// Server Capabilities Request message (section 2.2.2.2.1).
    Capabilities(CapabilitiesRequest),
    /// CreateFile Request message (section 2.2.2.3.1).
    CreateFile(CreateFileRequest),
    /// Read Request message (section 2.2.2.3.3).
    Read(ReadRequest),
    /// Write Request message (section 2.2.2.3.5).
    Write(WriteRequest),
    /// IOControl Request message (section 2.2.2.3.7).
    IoControl(IoControlRequest),
    /// Specific IoCancel Request message (section 2.2.2.3.9).
    SpecificIoCancel(IoCancelRequest),
}

impl ServerIoPdu {
    const NAME: &'static str = "SERVER_IO_PDU";

    pub fn request_id(&self) -> RequestId {
        match self {
            Self::Capabilities(pdu) => pdu.request_id,
            Self::CreateFile(pdu) => pdu.request_id,
            Self::Read(pdu) => pdu.request_id,
            Self::Write(pdu) => pdu.request_id,
            Self::IoControl(pdu) => pdu.request_id,
            Self::SpecificIoCancel(pdu) => pdu.request_id,
        }
    }

    fn function_id(&self) -> u32 {
        match self {
            Self::Capabilities(_) => CAPABILITIES_REQUEST,
            Self::CreateFile(_) => CREATE_FILE_REQUEST,
            Self::Read(_) => READ_REQUEST,
            Self::Write(_) => WRITE_REQUEST,
            Self::IoControl(_) => IOCONTROL_REQUEST,
            Self::SpecificIoCancel(_) => SPECIFIC_IOCANCEL_REQUEST,
        }
    }

    fn payload_size(&self) -> usize {
        match self {
            Self::Capabilities(_) => {
                2 /* Version */
            }
            Self::CreateFile(_) => {
                4 /* DeviceId */ + 4 /* dwDesiredAccess */ + 4 /* dwShareMode */
                    + 4 /* dwCreationDisposition */ + 4 /* dwFlagsAndAttributes */
            }
            Self::Read(_) => {
                4 /* cbBytesToRead */ + 4 /* OffsetHigh */ + 4 /* OffsetLow */
            }
            Self::Write(pdu) => {
                4 /* cbWrite */ + 4 /* OffsetHigh */ + 4 /* OffsetLow */ + pdu.data.len() + 1 /* UnusedByte */
            }
            Self::IoControl(pdu) => {
                4 /* IoCode */ + 4 /* cbIn */ + 4 /* cbOut */ + pdu.input.len() + pdu.output.len() + 1 /* UnusedByte */
            }
            Self::SpecificIoCancel(_) => {
                1 /* UnusedBits */ + 3 /* idToCancel */
            }
        }
    }
}

impl Encode for ServerIoPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.request_id().get());
        dst.write_u32(self.function_id());
        match self {
            Self::Capabilities(pdu) => dst.write_u16(pdu.version),
            Self::CreateFile(pdu) => {
                dst.write_u32(pdu.device_id);
                dst.write_u32(pdu.desired_access);
                dst.write_u32(pdu.share_mode);
                dst.write_u32(pdu.creation_disposition);
                dst.write_u32(pdu.flags_and_attributes);
            }
            Self::Read(pdu) => {
                dst.write_u32(pdu.bytes_to_read);
                write_offset(dst, pdu.offset);
            }
            Self::Write(pdu) => {
                dst.write_u32(cast_length!("cbWrite", pdu.data.len())?);
                write_offset(dst, pdu.offset);
                dst.write_slice(&pdu.data);
                dst.write_u8(0); // UnusedByte
            }
            Self::IoControl(pdu) => {
                dst.write_u32(pdu.io_code);
                dst.write_u32(cast_length!("cbIn", pdu.input.len())?);
                dst.write_u32(pdu.output_size);
                dst.write_slice(&pdu.input);
                dst.write_slice(&pdu.output);
                dst.write_u8(0); // UnusedByte
            }
            Self::SpecificIoCancel(pdu) => dst.write_u32(pdu.id_to_cancel.get() << 8),
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        SERVER_HEADER_SIZE + self.payload_size()
    }
}

impl<'de> Decode<'de> for ServerIoPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: SERVER_HEADER_SIZE);
        // UnusedBits occupies the high byte and is ignored.
        let request_id = RequestId::from_low_bits(src.read_u32());
        let function_id = src.read_u32();

        match function_id {
            CAPABILITIES_REQUEST => {
                ensure_size!(in: src, size: 2);
                Ok(Self::Capabilities(CapabilitiesRequest {
                    request_id,
                    version: src.read_u16(),
                }))
            }
            CREATE_FILE_REQUEST => {
                ensure_size!(in: src, size: 20);
                Ok(Self::CreateFile(CreateFileRequest {
                    request_id,
                    device_id: src.read_u32(),
                    desired_access: src.read_u32(),
                    share_mode: src.read_u32(),
                    creation_disposition: src.read_u32(),
                    flags_and_attributes: src.read_u32(),
                }))
            }
            READ_REQUEST => {
                ensure_size!(in: src, size: 12);
                Ok(Self::Read(ReadRequest {
                    request_id,
                    bytes_to_read: src.read_u32(),
                    offset: read_offset(src),
                }))
            }
            WRITE_REQUEST => {
                ensure_size!(in: src, size: 12);
                let length = read_length(src, "cbWrite")?;
                let offset = read_offset(src);
                ensure_size!(in: src, size: length);
                let data = src.read_slice(length).to_vec();
                skip_unused_byte(src);
                Ok(Self::Write(WriteRequest {
                    request_id,
                    offset,
                    data,
                }))
            }
            IOCONTROL_REQUEST => {
                ensure_size!(in: src, size: 12);
                let io_code = src.read_u32();
                let input_length = read_length(src, "cbIn")?;
                let output_size = src.read_u32();
                ensure_size!(in: src, size: input_length);
                let input = src.read_slice(input_length).to_vec();
                // DataOut is optional and its size is whatever precedes the trailing UnusedByte
                // ([MS-RDPEPNP] section 3.2.5.2.2.7).
                let output = match src.len() {
                    0 => Vec::new(),
                    remaining => {
                        let output = src.read_slice(remaining - 1).to_vec();
                        skip_unused_byte(src);
                        output
                    }
                };
                Ok(Self::IoControl(IoControlRequest {
                    request_id,
                    io_code,
                    output_size,
                    input,
                    output,
                }))
            }
            SPECIFIC_IOCANCEL_REQUEST => {
                ensure_size!(in: src, size: 4);
                // UNTESTED: UnusedBits precedes the 24-bit idToCancel on the wire, as the normative
                // diagram of [MS-RDPEPNP] section 2.2.2.3.9 lays it out; the only sample cancels ID 0,
                // which cannot tell this layout from the SERVER_IO_HEADER one. A wrong layout makes
                // cancellations miss their request, which still completes normally. Check against a
                // Windows capture with a nonzero idToCancel before changing it.
                let id_to_cancel = RequestId::from_low_bits(src.read_u32() >> 8);
                Ok(Self::SpecificIoCancel(IoCancelRequest {
                    request_id,
                    id_to_cancel,
                }))
            }
            _ => Err(unsupported_value_err!(
                "FunctionId",
                alloc::format!("{function_id:#010x}")
            )),
        }
    }
}

/// Server Capabilities Request message ([MS-RDPEPNP] section 2.2.2.2.1).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilitiesRequest {
    pub request_id: RequestId,
    pub version: u16,
}

/// CreateFile Request message ([MS-RDPEPNP] section 2.2.2.3.1).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateFileRequest {
    pub request_id: RequestId,
    /// Client device ID from the Client Device Addition message.
    pub device_id: u32,
    pub desired_access: u32,
    pub share_mode: u32,
    pub creation_disposition: u32,
    pub flags_and_attributes: u32,
}

/// Read Request message ([MS-RDPEPNP] section 2.2.2.3.3).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadRequest {
    pub request_id: RequestId,
    pub bytes_to_read: u32,
    /// `OffsetHigh` and `OffsetLow` combined.
    pub offset: u64,
}

/// Write Request message ([MS-RDPEPNP] section 2.2.2.3.5).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRequest {
    pub request_id: RequestId,
    /// `OffsetHigh` and `OffsetLow` combined.
    pub offset: u64,
    pub data: Vec<u8>,
}

/// IOControl Request message ([MS-RDPEPNP] section 2.2.2.3.7).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoControlRequest {
    pub request_id: RequestId,
    pub io_code: u32,
    /// `cbOut`, the largest output the reply may return.
    pub output_size: u32,
    /// `DataIn`.
    pub input: Vec<u8>,
    /// `DataOut`, the optional initial content of the output buffer.
    pub output: Vec<u8>,
}

/// Specific IoCancel Request message ([MS-RDPEPNP] section 2.2.2.3.9).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoCancelRequest {
    pub request_id: RequestId,
    pub id_to_cancel: RequestId,
}

/// Client Capabilities Reply message ([MS-RDPEPNP] section 2.2.2.2.2).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilitiesReply {
    pub request_id: RequestId,
    pub version: u16,
}

impl CapabilitiesReply {
    const NAME: &'static str = "CLIENT_CAPABILITIES_REPLY";
}

impl Encode for CapabilitiesReply {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        write_client_header(dst, self.request_id, RESPONSE_PACKET);
        dst.write_u16(self.version);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        CLIENT_HEADER_SIZE + 2 /* Version */
    }
}

impl<'de> Decode<'de> for CapabilitiesReply {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: CLIENT_HEADER_SIZE + 2);
        let request_id = read_client_header(src, RESPONSE_PACKET)?;
        Ok(Self {
            request_id,
            version: src.read_u16(),
        })
    }
}

impl DvcEncode for CapabilitiesReply {}

/// CreateFile Reply message ([MS-RDPEPNP] section 2.2.2.3.2).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateFileReply {
    pub request_id: RequestId,
    pub result: HResult,
}

impl CreateFileReply {
    const NAME: &'static str = "CREATE_FILE_REPLY";
}

impl Encode for CreateFileReply {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        write_client_header(dst, self.request_id, RESPONSE_PACKET);
        dst.write_u32(self.result.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        CLIENT_HEADER_SIZE + 4 /* Result */
    }
}

impl<'de> Decode<'de> for CreateFileReply {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: CLIENT_HEADER_SIZE + 4);
        let request_id = read_client_header(src, RESPONSE_PACKET)?;
        Ok(Self {
            request_id,
            result: HResult(src.read_u32()),
        })
    }
}

impl DvcEncode for CreateFileReply {}

/// Read Reply message ([MS-RDPEPNP] section 2.2.2.3.4).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadReply {
    pub request_id: RequestId,
    pub result: HResult,
    pub data: Vec<u8>,
}

impl ReadReply {
    const NAME: &'static str = "READ_REPLY";
}

impl Encode for ReadReply {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        write_client_header(dst, self.request_id, RESPONSE_PACKET);
        dst.write_u32(self.result.0);
        dst.write_u32(cast_length!("cbBytesRead", self.data.len())?);
        dst.write_slice(&self.data);
        dst.write_u8(0); // UnusedByte
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        CLIENT_HEADER_SIZE + 4 /* Result */ + 4 /* cbBytesRead */ + self.data.len() + 1 /* UnusedByte */
    }
}

impl<'de> Decode<'de> for ReadReply {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: CLIENT_HEADER_SIZE + 8);
        let request_id = read_client_header(src, RESPONSE_PACKET)?;
        let result = HResult(src.read_u32());
        let data = read_data(src, "cbBytesRead")?;
        Ok(Self {
            request_id,
            result,
            data,
        })
    }
}

impl DvcEncode for ReadReply {}

/// Write Reply message ([MS-RDPEPNP] section 2.2.2.3.6).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteReply {
    pub request_id: RequestId,
    pub result: HResult,
    pub bytes_written: u32,
}

impl WriteReply {
    const NAME: &'static str = "WRITE_REPLY";
}

impl Encode for WriteReply {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        write_client_header(dst, self.request_id, RESPONSE_PACKET);
        dst.write_u32(self.result.0);
        dst.write_u32(self.bytes_written);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        CLIENT_HEADER_SIZE + 4 /* Result */ + 4 /* cbBytesWritten */
    }
}

impl<'de> Decode<'de> for WriteReply {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: CLIENT_HEADER_SIZE + 8);
        let request_id = read_client_header(src, RESPONSE_PACKET)?;
        Ok(Self {
            request_id,
            result: HResult(src.read_u32()),
            bytes_written: src.read_u32(),
        })
    }
}

impl DvcEncode for WriteReply {}

/// IOControl Reply message ([MS-RDPEPNP] section 2.2.2.3.8).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoControlReply {
    pub request_id: RequestId,
    pub result: HResult,
    pub data: Vec<u8>,
}

impl IoControlReply {
    const NAME: &'static str = "IOCONTROL_REPLY";
}

impl Encode for IoControlReply {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        write_client_header(dst, self.request_id, RESPONSE_PACKET);
        dst.write_u32(self.result.0);
        dst.write_u32(cast_length!("cbBytesReadReturned", self.data.len())?);
        dst.write_slice(&self.data);
        dst.write_u8(0); // UnusedByte
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        CLIENT_HEADER_SIZE + 4 /* Result */ + 4 /* cbBytesReadReturned */ + self.data.len() + 1 /* UnusedByte */
    }
}

impl<'de> Decode<'de> for IoControlReply {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: CLIENT_HEADER_SIZE + 8);
        let request_id = read_client_header(src, RESPONSE_PACKET)?;
        let result = HResult(src.read_u32());
        let data = read_data(src, "cbBytesReadReturned")?;
        Ok(Self {
            request_id,
            result,
            data,
        })
    }
}

impl DvcEncode for IoControlReply {}

/// Client Device Custom Event message ([MS-RDPEPNP] section 2.2.2.3.10).
///
/// [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomEvent {
    pub event_guid: Guid,
    pub data: Vec<u8>,
}

impl CustomEvent {
    const NAME: &'static str = "CLIENT_DEVICE_CUSTOM_EVENT";
}

impl Encode for CustomEvent {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        // The RequestId of a custom event is unused and set to zero.
        write_client_header(dst, RequestId(0), CUSTOM_EVENT_PACKET);
        self.event_guid.encode(dst)?;
        dst.write_u32(cast_length!("cbData", self.data.len())?);
        dst.write_slice(&self.data);
        dst.write_u8(0); // UnusedByte
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        CLIENT_HEADER_SIZE + Guid::FIXED_PART_SIZE + 4 /* cbData */ + self.data.len() + 1 /* UnusedByte */
    }
}

impl<'de> Decode<'de> for CustomEvent {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: CLIENT_HEADER_SIZE + Guid::FIXED_PART_SIZE + 4);
        read_client_header(src, CUSTOM_EVENT_PACKET)?;
        let event_guid = Guid::decode(src)?;
        let data = read_data(src, "cbData")?;
        Ok(Self { event_guid, data })
    }
}

impl DvcEncode for CustomEvent {}

fn write_client_header(dst: &mut WriteCursor<'_>, request_id: RequestId, packet_type: u8) {
    dst.write_u32(request_id.get() | (u32::from(packet_type) << 24));
}

fn read_client_header(src: &mut ReadCursor<'_>, expected_packet_type: u8) -> DecodeResult<RequestId> {
    let header = src.read_u32();
    let packet_type = header.to_le_bytes()[3];
    if packet_type != expected_packet_type {
        return Err(unsupported_value_err!(
            "PacketType",
            alloc::format!("{packet_type:#04x}")
        ));
    }
    Ok(RequestId::from_low_bits(header))
}

fn write_offset(dst: &mut WriteCursor<'_>, offset: u64) {
    let high = u32::try_from(offset >> 32).expect("upper half of a u64 fits in u32");
    let low = u32::try_from(offset & 0xFFFF_FFFF).expect("masked lower half fits in u32");
    dst.write_u32(high); // OffsetHigh
    dst.write_u32(low); // OffsetLow
}

fn read_offset(src: &mut ReadCursor<'_>) -> u64 {
    let high = src.read_u32();
    let low = src.read_u32();
    (u64::from(high) << 32) | u64::from(low)
}

fn read_length(src: &mut ReadCursor<'_>, field: &'static str) -> DecodeResult<usize> {
    usize::try_from(src.read_u32()).map_err(|_| invalid_field_err!(field, "length overflows usize"))
}

/// Reads a length-prefixed data field followed by its optional `UnusedByte`.
fn read_data(src: &mut ReadCursor<'_>, field: &'static str) -> DecodeResult<Vec<u8>> {
    let length = read_length(src, field)?;
    ensure_size!(in: src, size: length);
    let data = src.read_slice(length).to_vec();
    skip_unused_byte(src);
    Ok(data)
}

/// Skips the trailing `UnusedByte`, which receivers must ignore, when the sender included it.
fn skip_unused_byte(src: &mut ReadCursor<'_>) {
    if !src.is_empty() {
        src.advance(1);
    }
}
