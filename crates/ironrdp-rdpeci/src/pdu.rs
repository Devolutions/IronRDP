//! Core Input Virtual Channel Extension PDUs defined by [MS-RDPECI].
//!
//! [MS-RDPECI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeci/

use alloc::vec::Vec;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, decode, ensure_size, invalid_field_err,
};
use ironrdp_dvc::DvcEncode;
use ironrdp_pdu::input::fast_path::FastPathInputEvent;
use tracing::warn;

const SIGNATURE: u8 = 0x03;

const PDU_TYPE_CS_INIT_REQUEST: u8 = 0x01;
const PDU_TYPE_SC_INIT_RESPONSE: u8 = 0x02;
const PDU_TYPE_CS_KEYBOARD_AND_MOUSE_INPUT: u8 = 0x03;

const HEADER_SIZE: usize = 1 /* signature */ + 1 /* pduType */ + 1 /* eventCount */ + 1 /* padding */;
const INIT_REQUEST_BODY_SIZE: usize = 2 /* protocolVersionMin */ + 2 /* protocolVersionMax */ + 8 /* reserved */;
const INIT_RESPONSE_BODY_SIZE: usize =
    2 /* selectedProtocolVersion */ + 2 /* protocolVersionMax */ + 8 /* reserved */;

/// Core input protocol version from [MS-RDPECI] section 2.2.3.1 "RDP_CORE_INPUT_CS_INIT_REQUEST_PDU".
///
/// Versions other than [`ProtocolVersion::V1_0`] decode as-is so that an endpoint can tell an unsupported version apart from a malformed PDU.
///
/// [MS-RDPECI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeci/
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion(pub u16);

impl ProtocolVersion {
    /// `RDP_CORE_INPUT_PROTOCOL_VERSION_100`.
    pub const V1_0: Self = Self(0x0100);
}

/// RDP_CORE_INPUT_CS_INIT_REQUEST_PDU from [MS-RDPECI] section 2.2.3.1 "RDP_CORE_INPUT_CS_INIT_REQUEST_PDU".
///
/// [MS-RDPECI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeci/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitRequestPdu {
    pub protocol_version_min: ProtocolVersion,
    pub protocol_version_max: ProtocolVersion,
}

impl InitRequestPdu {
    fn encode_body(&self, dst: &mut WriteCursor<'_>) {
        dst.write_u16(self.protocol_version_min.0);
        dst.write_u16(self.protocol_version_max.0);
        dst.write_u64(0); // reserved
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: INIT_REQUEST_BODY_SIZE);
        let protocol_version_min = ProtocolVersion(src.read_u16());
        let protocol_version_max = ProtocolVersion(src.read_u16());
        // The reserved field SHOULD be zero, so a nonzero value is accepted.
        let _reserved = src.read_u64();
        Ok(Self {
            protocol_version_min,
            protocol_version_max,
        })
    }
}

/// RDP_CORE_INPUT_SC_INIT_RESPONSE_PDU from [MS-RDPECI] section 2.2.3.2 "RDP_CORE_INPUT_SC_INIT_RESPONSE_PDU".
///
/// [MS-RDPECI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeci/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitResponsePdu {
    pub selected_protocol_version: ProtocolVersion,
    pub protocol_version_max: ProtocolVersion,
}

impl InitResponsePdu {
    fn encode_body(&self, dst: &mut WriteCursor<'_>) {
        dst.write_u16(self.selected_protocol_version.0);
        dst.write_u16(self.protocol_version_max.0);
        dst.write_u64(0); // reserved
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: INIT_RESPONSE_BODY_SIZE);
        let selected_protocol_version = ProtocolVersion(src.read_u16());
        let protocol_version_max = ProtocolVersion(src.read_u16());
        // The reserved field SHOULD be zero, so a nonzero value is accepted.
        let _reserved = src.read_u64();
        Ok(Self {
            selected_protocol_version,
            protocol_version_max,
        })
    }
}

/// RDP_CORE_INPUT_CS_KEYBOARD_AND_MOUSE_INPUT_PDU from [MS-RDPECI] section 2.2.3.3 "RDP_CORE_INPUT_CS_KEYBOARD_AND_MOUSE_INPUT_PDU".
///
/// Each RDP_CORE_INPUT_EVENT_CONTAINER from section 2.2.2.8 has the layout of a fast-path input event, so the events are [`FastPathInputEvent`] values.
///
/// [MS-RDPECI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeci/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputPdu {
    /// INVARIANT: `events.len() <= InputPdu::MAX_EVENTS`.
    events: Vec<FastPathInputEvent>,
}

impl InputPdu {
    /// Largest event count the one-byte `eventCount` header field can carry.
    pub const MAX_EVENTS: usize = 255;

    /// Creates an input PDU, failing when the events do not fit in one PDU.
    pub fn new(events: Vec<FastPathInputEvent>) -> EncodeResult<Self> {
        if Self::MAX_EVENTS < events.len() {
            return Err(invalid_field_err!("eventCount", "too many input events for one PDU"));
        }
        Ok(Self { events })
    }

    pub fn into_events(self) -> Vec<FastPathInputEvent> {
        self.events
    }

    fn event_count(&self) -> u8 {
        u8::try_from(self.events.len()).expect("INVARIANT: events.len() <= InputPdu::MAX_EVENTS")
    }

    fn body_size(&self) -> usize {
        self.events.iter().map(Encode::size).sum()
    }

    fn encode_body(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        for event in &self.events {
            event.encode(dst)?;
        }
        Ok(())
    }

    fn decode_body(src: &mut ReadCursor<'_>, event_count: u8) -> DecodeResult<Self> {
        let events = core::iter::repeat_with(|| FastPathInputEvent::decode(src))
            .take(usize::from(event_count))
            .collect::<DecodeResult<Vec<_>>>()?;
        Ok(Self { events })
    }
}

/// A core input PDU with its RDP_CORE_INPUT_HEADER from [MS-RDPECI] section 2.2.2.1 "RDP_CORE_INPUT_HEADER".
///
/// [MS-RDPECI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeci/
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreInputPdu {
    InitRequest(InitRequestPdu),
    InitResponse(InitResponsePdu),
    Input(InputPdu),
}

impl CoreInputPdu {
    const NAME: &'static str = "RDP_CORE_INPUT_PDU";

    fn pdu_type(&self) -> u8 {
        match self {
            Self::InitRequest(_) => PDU_TYPE_CS_INIT_REQUEST,
            Self::InitResponse(_) => PDU_TYPE_SC_INIT_RESPONSE,
            Self::Input(_) => PDU_TYPE_CS_KEYBOARD_AND_MOUSE_INPUT,
        }
    }

    fn body_size(&self) -> usize {
        match self {
            Self::InitRequest(_) => INIT_REQUEST_BODY_SIZE,
            Self::InitResponse(_) => INIT_RESPONSE_BODY_SIZE,
            Self::Input(pdu) => pdu.body_size(),
        }
    }
}

impl Encode for CoreInputPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        // The eventCount field MUST be zero for every PDU type except the input PDU.
        let event_count = match self {
            Self::Input(pdu) => pdu.event_count(),
            Self::InitRequest(_) | Self::InitResponse(_) => 0,
        };
        dst.write_u8(SIGNATURE);
        dst.write_u8(self.pdu_type());
        dst.write_u8(event_count);
        dst.write_u8(0); // padding

        match self {
            Self::InitRequest(pdu) => pdu.encode_body(dst),
            Self::InitResponse(pdu) => pdu.encode_body(dst),
            Self::Input(pdu) => pdu.encode_body(dst)?,
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        HEADER_SIZE + self.body_size()
    }
}

impl<'de> Decode<'de> for CoreInputPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: HEADER_SIZE);
        let signature = src.read_u8();
        if signature != SIGNATURE {
            return Err(invalid_field_err!("signature", "core input signature must be 0x03", in: src));
        }
        let pdu_type = src.read_u8();
        let event_count = src.read_u8();
        // The padding field SHOULD be zero, so a nonzero value is accepted.
        let _padding = src.read_u8();

        // The eventCount field is meaningful only for the input PDU and is ignored for the others.
        match pdu_type {
            PDU_TYPE_CS_INIT_REQUEST => InitRequestPdu::decode_body(src).map(Self::InitRequest),
            PDU_TYPE_SC_INIT_RESPONSE => InitResponsePdu::decode_body(src).map(Self::InitResponse),
            PDU_TYPE_CS_KEYBOARD_AND_MOUSE_INPUT => InputPdu::decode_body(src, event_count).map(Self::Input),
            _ => Err(invalid_field_err!("pduType", "unknown core input PDU type", in: src)),
        }
    }
}

impl DvcEncode for CoreInputPdu {}

/// Decodes a received core input message, returning `None` for a malformed one.
///
/// Malformed messages are logged and ignored, per [MS-RDPECI] section 3.1.5.1 "Processing a Core Input Message".
///
/// [MS-RDPECI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeci/
pub(crate) fn decode_received(payload: &[u8]) -> Option<CoreInputPdu> {
    decode(payload)
        .inspect_err(|error| warn!(%error, "Ignoring malformed core input PDU"))
        .ok()
}
