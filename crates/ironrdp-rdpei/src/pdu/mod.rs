//! MS-RDPEI PDU definitions.
//!
//! Structures follow [MS-RDPEI] sections 2.2.2 and 2.2.3.
//!
//! [MS-RDPEI]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpei/deb1ca39-344a-467a-b9d4-dfe196b46c9d

// Wire codecs intentionally pack integers with truncating casts after range checks.
#![allow(clippy::as_conversions)]
#![allow(clippy::cast_possible_truncation)]

mod varint;

use bitflags::bitflags;
use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_int, cast_length, ensure_fixed_part_size,
    ensure_size, invalid_field_err,
};
use ironrdp_dvc::DvcEncode;

pub use self::varint::{EightByteUnsigned, FourByteSigned, FourByteUnsigned, TwoByteSigned, TwoByteUnsigned};

/// Fixed size of [`RdpInputHeader`].
pub const RDP_INPUT_HEADER_SIZE: usize = 6;

/// EVENTID values from [MS-RDPEI] 2.2.2.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum RdpInputEventId {
    ScReady = 0x0001,
    CsReady = 0x0002,
    Touch = 0x0003,
    SuspendInput = 0x0004,
    ResumeInput = 0x0005,
    DismissHoveringTouchContact = 0x0006,
    Pen = 0x0008,
}

impl RdpInputEventId {
    pub fn from_u16(value: u16) -> DecodeResult<Self> {
        match value {
            0x0001 => Ok(Self::ScReady),
            0x0002 => Ok(Self::CsReady),
            0x0003 => Ok(Self::Touch),
            0x0004 => Ok(Self::SuspendInput),
            0x0005 => Ok(Self::ResumeInput),
            0x0006 => Ok(Self::DismissHoveringTouchContact),
            0x0008 => Ok(Self::Pen),
            _ => Err(invalid_field_err!("eventId", "unknown RDPINPUT event id")),
        }
    }

    pub fn as_u16(self) -> u16 {
        match self {
            Self::ScReady => 0x0001,
            Self::CsReady => 0x0002,
            Self::Touch => 0x0003,
            Self::SuspendInput => 0x0004,
            Self::ResumeInput => 0x0005,
            Self::DismissHoveringTouchContact => 0x0006,
            Self::Pen => 0x0008,
        }
    }
}

/// Protocol versions advertised in SC_READY / CS_READY.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum RdpInputProtocolVersion {
    V100 = 0x0001_0000,
    V101 = 0x0001_0001,
    V200 = 0x0002_0000,
    V300 = 0x0003_0000,
}

impl RdpInputProtocolVersion {
    pub fn from_u32(value: u32) -> DecodeResult<Self> {
        match value {
            0x0001_0000 => Ok(Self::V100),
            0x0001_0001 => Ok(Self::V101),
            0x0002_0000 => Ok(Self::V200),
            0x0003_0000 => Ok(Self::V300),
            _ => Err(invalid_field_err!(
                "protocolVersion",
                "unknown RDPINPUT protocol version"
            )),
        }
    }

    pub fn as_u32(self) -> u32 {
        self as u32
    }

    pub fn supports_pen(self) -> bool {
        self >= Self::V200
    }
}

bitflags! {
    /// Optional features in SC_READY (V300).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct ScReadyFeatures: u32 {
        const MULTIPEN_INJECTION_SUPPORTED = 0x0000_0001;
    }
}

bitflags! {
    /// CS_READY flags ([MS-RDPEI] 2.2.3.2).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct CsReadyFlags: u32 {
        const SHOW_TOUCH_VISUALS = 0x0000_0001;
        const DISABLE_TIMESTAMP_INJECTION = 0x0000_0002;
        const ENABLE_MULTIPEN_INJECTION = 0x0000_0004;
    }
}

bitflags! {
    /// Touch contact flags ([MS-RDPEI] 2.2.3.3.1.1 `contactFlags`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct TouchContactFlags: u32 {
        const DOWN = 0x0001;
        const UPDATE = 0x0002;
        const UP = 0x0004;
        const INRANGE = 0x0008;
        const INCONTACT = 0x0010;
        const CANCELED = 0x0020;
    }
}

impl TouchContactFlags {
    /// Returns true when the flag combination is one of the eight legal sets from MS-RDPEI 2.2.3.3.1.1.
    #[must_use]
    pub fn is_legal(self) -> bool {
        let legal = [
            Self::UP.bits(),
            (Self::UP | Self::CANCELED).bits(),
            Self::UPDATE.bits(),
            (Self::UPDATE | Self::CANCELED).bits(),
            (Self::DOWN | Self::INRANGE | Self::INCONTACT).bits(),
            (Self::UPDATE | Self::INRANGE | Self::INCONTACT).bits(),
            (Self::UP | Self::INRANGE).bits(),
            (Self::UPDATE | Self::INRANGE).bits(),
        ];
        legal.contains(&self.bits())
    }
}

bitflags! {
    /// Optional fields present in RDPINPUT_TOUCH_CONTACT ([MS-RDPEI] 2.2.3.3.1.1).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct TouchContactDataFlags: u16 {
        const CONTACTRECT_PRESENT = 0x0001;
        const ORIENTATION_PRESENT = 0x0002;
        const PRESSURE_PRESENT = 0x0004;
    }
}

bitflags! {
    /// Pen contact state flags ([MS-RDPEI] 2.2.3.7.1.1 `contactFlags`).
    ///
    /// Same legal combinations as [`TouchContactFlags`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct PenContactFlags: u32 {
        const DOWN = 0x0001;
        const UPDATE = 0x0002;
        const UP = 0x0004;
        const INRANGE = 0x0008;
        const INCONTACT = 0x0010;
        const CANCELED = 0x0020;
    }
}

impl PenContactFlags {
    /// Returns true when the flag combination is one of the eight legal sets from MS-RDPEI 2.2.3.7.1.1.
    #[must_use]
    pub fn is_legal(self) -> bool {
        TouchContactFlags::from_bits_truncate(self.bits()).is_legal()
    }
}

bitflags! {
    /// Pen button / inversion flags ([MS-RDPEI] 2.2.3.7.1.1 `penFlags`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct PenFlags: u32 {
        const BARREL_PRESSED = 0x0001;
        const ERASER_PRESSED = 0x0002;
        const INVERTED = 0x0004;
    }
}

bitflags! {
    /// Optional fields present in RDPINPUT_PEN_CONTACT ([MS-RDPEI] 2.2.3.7.1.1).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct PenContactDataFlags: u16 {
        const PENFLAGS_PRESENT = 0x0001;
        const PRESSURE_PRESENT = 0x0002;
        const ROTATION_PRESENT = 0x0004;
        const TILTX_PRESENT = 0x0008;
        const TILTY_PRESENT = 0x0010;
    }
}

/// RDPINPUT_HEADER ([MS-RDPEI] 2.2.2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RdpInputHeader {
    pub event_id: RdpInputEventId,
    /// Total PDU length in bytes, including this header.
    pub pdu_length: u32,
}

impl RdpInputHeader {
    const NAME: &'static str = "RDPINPUT_HEADER";
    const FIXED_PART_SIZE: usize = RDP_INPUT_HEADER_SIZE;
}

impl Encode for RdpInputHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u16(self.event_id.as_u16());
        dst.write_u32(self.pdu_length);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for RdpInputHeader {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let event_id = RdpInputEventId::from_u16(src.read_u16())?;
        let pdu_length = src.read_u32();
        if pdu_length < Self::FIXED_PART_SIZE as u32 {
            return Err(invalid_field_err!("pduLength", "less than header size"));
        }
        Ok(Self { event_id, pdu_length })
    }
}

/// RDPINPUT_SC_READY_PDU ([MS-RDPEI] 2.2.3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScReadyPdu {
    pub protocol_version: RdpInputProtocolVersion,
    pub supported_features: Option<ScReadyFeatures>,
}

impl ScReadyPdu {
    const NAME: &'static str = "RDPINPUT_SC_READY_PDU";

    pub fn new(protocol_version: RdpInputProtocolVersion) -> Self {
        Self {
            protocol_version,
            supported_features: None,
        }
    }

    #[must_use]
    pub fn with_features(mut self, features: ScReadyFeatures) -> Self {
        self.supported_features = Some(features);
        self
    }

    fn payload_size(&self) -> usize {
        4 /* protocolVersion */ + if self.supported_features.is_some() { 4 } else { 0 }
    }
}

impl Encode for ScReadyPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let pdu_length = cast_int!("pduLength", RDP_INPUT_HEADER_SIZE + self.payload_size())?;
        RdpInputHeader {
            event_id: RdpInputEventId::ScReady,
            pdu_length,
        }
        .encode(dst)?;
        dst.write_u32(self.protocol_version.as_u32());
        if let Some(features) = self.supported_features {
            dst.write_u32(features.bits());
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        RDP_INPUT_HEADER_SIZE + self.payload_size()
    }
}

impl<'de> Decode<'de> for ScReadyPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let header = RdpInputHeader::decode(src)?;
        if header.event_id != RdpInputEventId::ScReady {
            return Err(invalid_field_err!("eventId", "expected EVENTID_SC_READY"));
        }
        ensure_size!(in: src, size: 4);
        let protocol_version = RdpInputProtocolVersion::from_u32(src.read_u32())?;
        let remaining = header.pdu_length as usize - RDP_INPUT_HEADER_SIZE - 4;
        let supported_features = if remaining >= 4 {
            ensure_size!(in: src, size: 4);
            Some(ScReadyFeatures::from_bits_truncate(src.read_u32()))
        } else {
            None
        };
        Ok(Self {
            protocol_version,
            supported_features,
        })
    }
}

/// RDPINPUT_CS_READY_PDU ([MS-RDPEI] 2.2.3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsReadyPdu {
    pub flags: CsReadyFlags,
    pub protocol_version: RdpInputProtocolVersion,
    pub max_touch_contacts: u16,
}

impl CsReadyPdu {
    const NAME: &'static str = "RDPINPUT_CS_READY_PDU";
    const PAYLOAD_SIZE: usize = 4 /* flags */ + 4 /* protocolVersion */ + 2 /* maxTouchContacts */;

    pub fn new(flags: CsReadyFlags, protocol_version: RdpInputProtocolVersion, max_touch_contacts: u16) -> Self {
        Self {
            flags,
            protocol_version,
            max_touch_contacts,
        }
    }
}

impl Encode for CsReadyPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let pdu_length = cast_int!("pduLength", RDP_INPUT_HEADER_SIZE + Self::PAYLOAD_SIZE)?;
        RdpInputHeader {
            event_id: RdpInputEventId::CsReady,
            pdu_length,
        }
        .encode(dst)?;
        dst.write_u32(self.flags.bits());
        dst.write_u32(self.protocol_version.as_u32());
        dst.write_u16(self.max_touch_contacts);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        RDP_INPUT_HEADER_SIZE + Self::PAYLOAD_SIZE
    }
}

impl<'de> Decode<'de> for CsReadyPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let header = RdpInputHeader::decode(src)?;
        if header.event_id != RdpInputEventId::CsReady {
            return Err(invalid_field_err!("eventId", "expected EVENTID_CS_READY"));
        }
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);
        let flags = CsReadyFlags::from_bits_truncate(src.read_u32());
        let protocol_version = RdpInputProtocolVersion::from_u32(src.read_u32())?;
        let max_touch_contacts = src.read_u16();
        Ok(Self {
            flags,
            protocol_version,
            max_touch_contacts,
        })
    }
}

/// Optional geometry/pressure fields for a touch contact.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TouchContactFields {
    pub contact_rect_left: Option<i16>,
    pub contact_rect_top: Option<i16>,
    pub contact_rect_right: Option<i16>,
    pub contact_rect_bottom: Option<i16>,
    pub orientation: Option<u32>,
    pub pressure: Option<u32>,
}

/// RDPINPUT_CONTACT_DATA ([MS-RDPEI] 2.2.3.3.1.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchContact {
    pub contact_id: u8,
    pub fields_present: TouchContactDataFlags,
    pub x: i32,
    pub y: i32,
    pub contact_flags: TouchContactFlags,
    pub fields: TouchContactFields,
}

impl TouchContact {
    const NAME: &'static str = "RDPINPUT_CONTACT_DATA";

    #[must_use]
    pub fn new(contact_id: u8, x: i32, y: i32, contact_flags: TouchContactFlags) -> Self {
        Self {
            contact_id,
            fields_present: TouchContactDataFlags::empty(),
            x,
            y,
            contact_flags,
            fields: TouchContactFields::default(),
        }
    }

    #[must_use]
    pub fn with_contact_rect(mut self, left: i16, top: i16, right: i16, bottom: i16) -> Self {
        self.fields_present.insert(TouchContactDataFlags::CONTACTRECT_PRESENT);
        self.fields.contact_rect_left = Some(left);
        self.fields.contact_rect_top = Some(top);
        self.fields.contact_rect_right = Some(right);
        self.fields.contact_rect_bottom = Some(bottom);
        self
    }

    #[must_use]
    pub fn with_orientation(mut self, orientation: u32) -> Self {
        self.fields_present.insert(TouchContactDataFlags::ORIENTATION_PRESENT);
        self.fields.orientation = Some(orientation);
        self
    }

    #[must_use]
    pub fn with_pressure(mut self, pressure: u32) -> Self {
        self.fields_present.insert(TouchContactDataFlags::PRESSURE_PRESENT);
        self.fields.pressure = Some(pressure);
        self
    }

    fn payload_size(&self) -> usize {
        1 /* contactId */
            + TwoByteUnsigned::encoded_size(self.fields_present.bits())
            + FourByteSigned::encoded_size(self.x)
            + FourByteSigned::encoded_size(self.y)
            + FourByteUnsigned::encoded_size(self.contact_flags.bits())
            + if self.fields_present.contains(TouchContactDataFlags::CONTACTRECT_PRESENT) {
                TwoByteSigned::encoded_size(self.fields.contact_rect_left.unwrap_or(0))
                    + TwoByteSigned::encoded_size(self.fields.contact_rect_top.unwrap_or(0))
                    + TwoByteSigned::encoded_size(self.fields.contact_rect_right.unwrap_or(0))
                    + TwoByteSigned::encoded_size(self.fields.contact_rect_bottom.unwrap_or(0))
            } else {
                0
            }
            + if self.fields_present.contains(TouchContactDataFlags::ORIENTATION_PRESENT) {
                FourByteUnsigned::encoded_size(self.fields.orientation.unwrap_or(0))
            } else {
                0
            }
            + if self.fields_present.contains(TouchContactDataFlags::PRESSURE_PRESENT) {
                FourByteUnsigned::encoded_size(self.fields.pressure.unwrap_or(0))
            } else {
                0
            }
    }
}

impl Encode for TouchContact {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u8(self.contact_id);
        TwoByteUnsigned::new(self.fields_present.bits())
            .map_err(|e| ironrdp_core::other_err!("fieldsPresent", source: e))?
            .encode(dst)?;
        FourByteSigned::new(self.x)
            .map_err(|e| ironrdp_core::other_err!("x", source: e))?
            .encode(dst)?;
        FourByteSigned::new(self.y)
            .map_err(|e| ironrdp_core::other_err!("y", source: e))?
            .encode(dst)?;
        FourByteUnsigned::new(self.contact_flags.bits())
            .map_err(|e| ironrdp_core::other_err!("contactFlags", source: e))?
            .encode(dst)?;

        if self.fields_present.contains(TouchContactDataFlags::CONTACTRECT_PRESENT) {
            let left = self
                .fields
                .contact_rect_left
                .ok_or_else(|| invalid_field_err!("contactRectLeft", "missing"))?;
            let top = self
                .fields
                .contact_rect_top
                .ok_or_else(|| invalid_field_err!("contactRectTop", "missing"))?;
            let right = self
                .fields
                .contact_rect_right
                .ok_or_else(|| invalid_field_err!("contactRectRight", "missing"))?;
            let bottom = self
                .fields
                .contact_rect_bottom
                .ok_or_else(|| invalid_field_err!("contactRectBottom", "missing"))?;
            TwoByteSigned::new(left)
                .map_err(|e| ironrdp_core::other_err!("contactRectLeft", source: e))?
                .encode(dst)?;
            TwoByteSigned::new(top)
                .map_err(|e| ironrdp_core::other_err!("contactRectTop", source: e))?
                .encode(dst)?;
            TwoByteSigned::new(right)
                .map_err(|e| ironrdp_core::other_err!("contactRectRight", source: e))?
                .encode(dst)?;
            TwoByteSigned::new(bottom)
                .map_err(|e| ironrdp_core::other_err!("contactRectBottom", source: e))?
                .encode(dst)?;
        }
        if self.fields_present.contains(TouchContactDataFlags::ORIENTATION_PRESENT) {
            let orientation = self
                .fields
                .orientation
                .ok_or_else(|| invalid_field_err!("orientation", "missing"))?;
            FourByteUnsigned::new(orientation)
                .map_err(|e| ironrdp_core::other_err!("orientation", source: e))?
                .encode(dst)?;
        }
        if self.fields_present.contains(TouchContactDataFlags::PRESSURE_PRESENT) {
            let pressure = self
                .fields
                .pressure
                .ok_or_else(|| invalid_field_err!("pressure", "missing"))?;
            FourByteUnsigned::new(pressure)
                .map_err(|e| ironrdp_core::other_err!("pressure", source: e))?
                .encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.payload_size()
    }
}

impl<'de> Decode<'de> for TouchContact {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        let contact_id = src.read_u8();
        let fields_present = TouchContactDataFlags::from_bits_truncate(TwoByteUnsigned::decode(src)?.get());
        let x = FourByteSigned::decode(src)?.get();
        let y = FourByteSigned::decode(src)?.get();
        let contact_flags = TouchContactFlags::from_bits_truncate(FourByteUnsigned::decode(src)?.get());

        let mut fields = TouchContactFields::default();
        if fields_present.contains(TouchContactDataFlags::CONTACTRECT_PRESENT) {
            fields.contact_rect_left = Some(TwoByteSigned::decode(src)?.get());
            fields.contact_rect_top = Some(TwoByteSigned::decode(src)?.get());
            fields.contact_rect_right = Some(TwoByteSigned::decode(src)?.get());
            fields.contact_rect_bottom = Some(TwoByteSigned::decode(src)?.get());
        }
        if fields_present.contains(TouchContactDataFlags::ORIENTATION_PRESENT) {
            fields.orientation = Some(FourByteUnsigned::decode(src)?.get());
        }
        if fields_present.contains(TouchContactDataFlags::PRESSURE_PRESENT) {
            fields.pressure = Some(FourByteUnsigned::decode(src)?.get());
        }

        Ok(Self {
            contact_id,
            fields_present,
            x,
            y,
            contact_flags,
            fields,
        })
    }
}

/// RDPINPUT_TOUCH_FRAME ([MS-RDPEI] 2.2.3.3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchFrame {
    /// Microseconds since the previous frame (0 for the first frame of a transaction).
    pub frame_offset: u64,
    pub contacts: Vec<TouchContact>,
}

impl TouchFrame {
    const NAME: &'static str = "RDPINPUT_TOUCH_FRAME";

    pub fn new(frame_offset: u64, contacts: Vec<TouchContact>) -> Self {
        Self { frame_offset, contacts }
    }

    fn payload_size(&self) -> usize {
        let contact_count = self.contacts.len() as u16;
        TwoByteUnsigned::encoded_size(contact_count)
            + EightByteUnsigned::encoded_size(self.frame_offset)
            + self.contacts.iter().map(Encode::size).sum::<usize>()
    }
}

impl Encode for TouchFrame {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let contact_count: u16 = cast_length!("contactCount", self.contacts.len())?;
        TwoByteUnsigned::new(contact_count)
            .map_err(|e| ironrdp_core::other_err!("contactCount", source: e))?
            .encode(dst)?;
        EightByteUnsigned::new(self.frame_offset)
            .map_err(|e| ironrdp_core::other_err!("frameOffset", source: e))?
            .encode(dst)?;
        for contact in &self.contacts {
            contact.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.payload_size()
    }
}

impl<'de> Decode<'de> for TouchFrame {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let contact_count = TwoByteUnsigned::decode(src)?.get();
        let frame_offset = EightByteUnsigned::decode(src)?.get();
        let mut contacts = Vec::with_capacity(contact_count as usize);
        for _ in 0..contact_count {
            contacts.push(TouchContact::decode(src)?);
        }
        Ok(Self { frame_offset, contacts })
    }
}

/// RDPINPUT_TOUCH_EVENT_PDU ([MS-RDPEI] 2.2.3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchEventPdu {
    /// Milliseconds elapsed for the oldest frame in this PDU.
    pub encode_time: u32,
    pub frames: Vec<TouchFrame>,
}

impl TouchEventPdu {
    const NAME: &'static str = "RDPINPUT_TOUCH_EVENT_PDU";

    pub fn new(encode_time: u32, frames: Vec<TouchFrame>) -> Self {
        Self { encode_time, frames }
    }

    fn payload_size(&self) -> usize {
        let frame_count = self.frames.len() as u16;
        FourByteUnsigned::encoded_size(self.encode_time)
            + TwoByteUnsigned::encoded_size(frame_count)
            + self.frames.iter().map(Encode::size).sum::<usize>()
    }
}

impl Encode for TouchEventPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let pdu_length = cast_int!("pduLength", RDP_INPUT_HEADER_SIZE + self.payload_size())?;
        RdpInputHeader {
            event_id: RdpInputEventId::Touch,
            pdu_length,
        }
        .encode(dst)?;
        FourByteUnsigned::new(self.encode_time)
            .map_err(|e| ironrdp_core::other_err!("encodeTime", source: e))?
            .encode(dst)?;
        let frame_count: u16 = cast_length!("frameCount", self.frames.len())?;
        TwoByteUnsigned::new(frame_count)
            .map_err(|e| ironrdp_core::other_err!("frameCount", source: e))?
            .encode(dst)?;
        for frame in &self.frames {
            frame.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        RDP_INPUT_HEADER_SIZE + self.payload_size()
    }
}

impl<'de> Decode<'de> for TouchEventPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let header = RdpInputHeader::decode(src)?;
        if header.event_id != RdpInputEventId::Touch {
            return Err(invalid_field_err!("eventId", "expected EVENTID_TOUCH"));
        }
        let encode_time = FourByteUnsigned::decode(src)?.get();
        let frame_count = TwoByteUnsigned::decode(src)?.get();
        let mut frames = Vec::with_capacity(frame_count as usize);
        for _ in 0..frame_count {
            frames.push(TouchFrame::decode(src)?);
        }
        Ok(Self { encode_time, frames })
    }
}

/// Header-only PDUs (SUSPEND / RESUME).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderOnlyPdu {
    pub event_id: RdpInputEventId,
}

impl HeaderOnlyPdu {
    const NAME: &'static str = "RDPINPUT_HEADER_ONLY_PDU";

    pub fn suspend() -> Self {
        Self {
            event_id: RdpInputEventId::SuspendInput,
        }
    }

    pub fn resume() -> Self {
        Self {
            event_id: RdpInputEventId::ResumeInput,
        }
    }
}

impl Encode for HeaderOnlyPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        RdpInputHeader {
            event_id: self.event_id,
            pdu_length: RDP_INPUT_HEADER_SIZE as u32,
        }
        .encode(dst)
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        RDP_INPUT_HEADER_SIZE
    }
}

impl<'de> Decode<'de> for HeaderOnlyPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let header = RdpInputHeader::decode(src)?;
        match header.event_id {
            RdpInputEventId::SuspendInput | RdpInputEventId::ResumeInput => Ok(Self {
                event_id: header.event_id,
            }),
            _ => Err(invalid_field_err!(
                "eventId",
                "expected EVENTID_SUSPEND_INPUT or EVENTID_RESUME_INPUT"
            )),
        }
    }
}

/// RDPINPUT_DISMISS_HOVERING_TOUCH_CONTACT_PDU ([MS-RDPEI] 2.2.3.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DismissHoveringTouchContactPdu {
    pub contact_id: u8,
}

impl DismissHoveringTouchContactPdu {
    const NAME: &'static str = "RDPINPUT_DISMISS_HOVERING_TOUCH_CONTACT_PDU";
    const PAYLOAD_SIZE: usize = 1;

    pub fn new(contact_id: u8) -> Self {
        Self { contact_id }
    }
}

impl Encode for DismissHoveringTouchContactPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let pdu_length = cast_int!("pduLength", RDP_INPUT_HEADER_SIZE + Self::PAYLOAD_SIZE)?;
        RdpInputHeader {
            event_id: RdpInputEventId::DismissHoveringTouchContact,
            pdu_length,
        }
        .encode(dst)?;
        dst.write_u8(self.contact_id);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        RDP_INPUT_HEADER_SIZE + Self::PAYLOAD_SIZE
    }
}

impl<'de> Decode<'de> for DismissHoveringTouchContactPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let header = RdpInputHeader::decode(src)?;
        if header.event_id != RdpInputEventId::DismissHoveringTouchContact {
            return Err(invalid_field_err!(
                "eventId",
                "expected EVENTID_DISMISS_HOVERING_TOUCH_CONTACT"
            ));
        }
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);
        Ok(Self {
            contact_id: src.read_u8(),
        })
    }
}

/// Optional fields for a pen contact ([MS-RDPEI] 2.2.3.7.1.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PenContactFields {
    pub pen_flags: Option<PenFlags>,
    pub pressure: Option<u32>,
    pub rotation: Option<u16>,
    pub tilt_x: Option<i16>,
    pub tilt_y: Option<i16>,
}

/// RDPINPUT_PEN_CONTACT ([MS-RDPEI] 2.2.3.7.1.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PenContact {
    pub device_id: u8,
    pub fields_present: PenContactDataFlags,
    pub x: i32,
    pub y: i32,
    pub contact_flags: PenContactFlags,
    pub fields: PenContactFields,
}

impl PenContact {
    const NAME: &'static str = "RDPINPUT_PEN_CONTACT";

    #[must_use]
    pub fn new(device_id: u8, x: i32, y: i32, contact_flags: PenContactFlags) -> Self {
        Self {
            device_id,
            fields_present: PenContactDataFlags::empty(),
            x,
            y,
            contact_flags,
            fields: PenContactFields::default(),
        }
    }

    #[must_use]
    pub fn with_pen_flags(mut self, pen_flags: PenFlags) -> Self {
        self.fields_present.insert(PenContactDataFlags::PENFLAGS_PRESENT);
        self.fields.pen_flags = Some(pen_flags);
        self
    }

    #[must_use]
    pub fn with_pressure(mut self, pressure: u32) -> Self {
        self.fields_present.insert(PenContactDataFlags::PRESSURE_PRESENT);
        self.fields.pressure = Some(pressure);
        self
    }

    #[must_use]
    pub fn with_rotation(mut self, rotation: u16) -> Self {
        self.fields_present.insert(PenContactDataFlags::ROTATION_PRESENT);
        self.fields.rotation = Some(rotation);
        self
    }

    #[must_use]
    pub fn with_tilt(mut self, tilt_x: i16, tilt_y: i16) -> Self {
        self.fields_present
            .insert(PenContactDataFlags::TILTX_PRESENT | PenContactDataFlags::TILTY_PRESENT);
        self.fields.tilt_x = Some(tilt_x);
        self.fields.tilt_y = Some(tilt_y);
        self
    }

    fn payload_size(&self) -> usize {
        1 /* deviceId */
            + TwoByteUnsigned::encoded_size(self.fields_present.bits())
            + FourByteSigned::encoded_size(self.x)
            + FourByteSigned::encoded_size(self.y)
            + FourByteUnsigned::encoded_size(self.contact_flags.bits())
            + if self.fields_present.contains(PenContactDataFlags::PENFLAGS_PRESENT) {
                FourByteUnsigned::encoded_size(self.fields.pen_flags.unwrap_or(PenFlags::empty()).bits())
            } else {
                0
            }
            + if self.fields_present.contains(PenContactDataFlags::PRESSURE_PRESENT) {
                FourByteUnsigned::encoded_size(self.fields.pressure.unwrap_or(0))
            } else {
                0
            }
            + if self.fields_present.contains(PenContactDataFlags::ROTATION_PRESENT) {
                TwoByteUnsigned::encoded_size(self.fields.rotation.unwrap_or(0))
            } else {
                0
            }
            + if self.fields_present.contains(PenContactDataFlags::TILTX_PRESENT) {
                TwoByteSigned::encoded_size(self.fields.tilt_x.unwrap_or(0))
            } else {
                0
            }
            + if self.fields_present.contains(PenContactDataFlags::TILTY_PRESENT) {
                TwoByteSigned::encoded_size(self.fields.tilt_y.unwrap_or(0))
            } else {
                0
            }
    }
}

impl Encode for PenContact {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u8(self.device_id);
        TwoByteUnsigned::new(self.fields_present.bits())
            .map_err(|e| ironrdp_core::other_err!("fieldsPresent", source: e))?
            .encode(dst)?;
        FourByteSigned::new(self.x)
            .map_err(|e| ironrdp_core::other_err!("x", source: e))?
            .encode(dst)?;
        FourByteSigned::new(self.y)
            .map_err(|e| ironrdp_core::other_err!("y", source: e))?
            .encode(dst)?;
        FourByteUnsigned::new(self.contact_flags.bits())
            .map_err(|e| ironrdp_core::other_err!("contactFlags", source: e))?
            .encode(dst)?;

        if self.fields_present.contains(PenContactDataFlags::PENFLAGS_PRESENT) {
            let pen_flags = self
                .fields
                .pen_flags
                .ok_or_else(|| invalid_field_err!("penFlags", "missing"))?;
            FourByteUnsigned::new(pen_flags.bits())
                .map_err(|e| ironrdp_core::other_err!("penFlags", source: e))?
                .encode(dst)?;
        }
        if self.fields_present.contains(PenContactDataFlags::PRESSURE_PRESENT) {
            let pressure = self
                .fields
                .pressure
                .ok_or_else(|| invalid_field_err!("pressure", "missing"))?;
            FourByteUnsigned::new(pressure)
                .map_err(|e| ironrdp_core::other_err!("pressure", source: e))?
                .encode(dst)?;
        }
        if self.fields_present.contains(PenContactDataFlags::ROTATION_PRESENT) {
            let rotation = self
                .fields
                .rotation
                .ok_or_else(|| invalid_field_err!("rotation", "missing"))?;
            TwoByteUnsigned::new(rotation)
                .map_err(|e| ironrdp_core::other_err!("rotation", source: e))?
                .encode(dst)?;
        }
        if self.fields_present.contains(PenContactDataFlags::TILTX_PRESENT) {
            let tilt_x = self
                .fields
                .tilt_x
                .ok_or_else(|| invalid_field_err!("tiltX", "missing"))?;
            TwoByteSigned::new(tilt_x)
                .map_err(|e| ironrdp_core::other_err!("tiltX", source: e))?
                .encode(dst)?;
        }
        if self.fields_present.contains(PenContactDataFlags::TILTY_PRESENT) {
            let tilt_y = self
                .fields
                .tilt_y
                .ok_or_else(|| invalid_field_err!("tiltY", "missing"))?;
            TwoByteSigned::new(tilt_y)
                .map_err(|e| ironrdp_core::other_err!("tiltY", source: e))?
                .encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.payload_size()
    }
}

impl<'de> Decode<'de> for PenContact {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        let device_id = src.read_u8();
        let fields_present = PenContactDataFlags::from_bits_truncate(TwoByteUnsigned::decode(src)?.get());
        let x = FourByteSigned::decode(src)?.get();
        let y = FourByteSigned::decode(src)?.get();
        let contact_flags = PenContactFlags::from_bits_truncate(FourByteUnsigned::decode(src)?.get());

        let mut fields = PenContactFields::default();
        if fields_present.contains(PenContactDataFlags::PENFLAGS_PRESENT) {
            fields.pen_flags = Some(PenFlags::from_bits_truncate(FourByteUnsigned::decode(src)?.get()));
        }
        if fields_present.contains(PenContactDataFlags::PRESSURE_PRESENT) {
            fields.pressure = Some(FourByteUnsigned::decode(src)?.get());
        }
        if fields_present.contains(PenContactDataFlags::ROTATION_PRESENT) {
            fields.rotation = Some(TwoByteUnsigned::decode(src)?.get());
        }
        if fields_present.contains(PenContactDataFlags::TILTX_PRESENT) {
            fields.tilt_x = Some(TwoByteSigned::decode(src)?.get());
        }
        if fields_present.contains(PenContactDataFlags::TILTY_PRESENT) {
            fields.tilt_y = Some(TwoByteSigned::decode(src)?.get());
        }

        Ok(Self {
            device_id,
            fields_present,
            x,
            y,
            contact_flags,
            fields,
        })
    }
}

/// RDPINPUT_PEN_FRAME ([MS-RDPEI] 2.2.3.7.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PenFrame {
    pub frame_offset: u64,
    pub contacts: Vec<PenContact>,
}

impl PenFrame {
    const NAME: &'static str = "RDPINPUT_PEN_FRAME";

    pub fn new(frame_offset: u64, contacts: Vec<PenContact>) -> Self {
        Self { frame_offset, contacts }
    }

    fn payload_size(&self) -> usize {
        let contact_count = self.contacts.len() as u16;
        TwoByteUnsigned::encoded_size(contact_count)
            + EightByteUnsigned::encoded_size(self.frame_offset)
            + self.contacts.iter().map(Encode::size).sum::<usize>()
    }
}

impl Encode for PenFrame {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let contact_count: u16 = cast_length!("contactCount", self.contacts.len())?;
        TwoByteUnsigned::new(contact_count)
            .map_err(|e| ironrdp_core::other_err!("contactCount", source: e))?
            .encode(dst)?;
        EightByteUnsigned::new(self.frame_offset)
            .map_err(|e| ironrdp_core::other_err!("frameOffset", source: e))?
            .encode(dst)?;
        for contact in &self.contacts {
            contact.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.payload_size()
    }
}

impl<'de> Decode<'de> for PenFrame {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let contact_count = TwoByteUnsigned::decode(src)?.get();
        let frame_offset = EightByteUnsigned::decode(src)?.get();
        let mut contacts = Vec::with_capacity(contact_count as usize);
        for _ in 0..contact_count {
            contacts.push(PenContact::decode(src)?);
        }
        Ok(Self { frame_offset, contacts })
    }
}

/// RDPINPUT_PEN_EVENT_PDU ([MS-RDPEI] 2.2.3.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PenEventPdu {
    pub encode_time: u32,
    pub frames: Vec<PenFrame>,
}

impl PenEventPdu {
    const NAME: &'static str = "RDPINPUT_PEN_EVENT_PDU";

    pub fn new(encode_time: u32, frames: Vec<PenFrame>) -> Self {
        Self { encode_time, frames }
    }

    fn payload_size(&self) -> usize {
        let frame_count = self.frames.len() as u16;
        FourByteUnsigned::encoded_size(self.encode_time)
            + TwoByteUnsigned::encoded_size(frame_count)
            + self.frames.iter().map(Encode::size).sum::<usize>()
    }
}

impl Encode for PenEventPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let pdu_length = cast_int!("pduLength", RDP_INPUT_HEADER_SIZE + self.payload_size())?;
        RdpInputHeader {
            event_id: RdpInputEventId::Pen,
            pdu_length,
        }
        .encode(dst)?;
        FourByteUnsigned::new(self.encode_time)
            .map_err(|e| ironrdp_core::other_err!("encodeTime", source: e))?
            .encode(dst)?;
        let frame_count: u16 = cast_length!("frameCount", self.frames.len())?;
        TwoByteUnsigned::new(frame_count)
            .map_err(|e| ironrdp_core::other_err!("frameCount", source: e))?
            .encode(dst)?;
        for frame in &self.frames {
            frame.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        RDP_INPUT_HEADER_SIZE + self.payload_size()
    }
}

impl<'de> Decode<'de> for PenEventPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let header = RdpInputHeader::decode(src)?;
        if header.event_id != RdpInputEventId::Pen {
            return Err(invalid_field_err!("eventId", "expected EVENTID_PEN"));
        }
        let encode_time = FourByteUnsigned::decode(src)?.get();
        let frame_count = TwoByteUnsigned::decode(src)?.get();
        let mut frames = Vec::with_capacity(frame_count as usize);
        for _ in 0..frame_count {
            frames.push(PenFrame::decode(src)?);
        }
        Ok(Self { encode_time, frames })
    }
}

/// Top-level RDPEI PDU enum for channel process/encode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RdpeiPdu {
    ScReady(ScReadyPdu),
    CsReady(CsReadyPdu),
    Touch(TouchEventPdu),
    SuspendInput,
    ResumeInput,
    DismissHoveringTouchContact(DismissHoveringTouchContactPdu),
    Pen(PenEventPdu),
}

impl RdpeiPdu {
    const NAME: &'static str = "RdpeiPdu";
}

impl Encode for RdpeiPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        match self {
            Self::ScReady(pdu) => pdu.encode(dst),
            Self::CsReady(pdu) => pdu.encode(dst),
            Self::Touch(pdu) => pdu.encode(dst),
            Self::SuspendInput => HeaderOnlyPdu::suspend().encode(dst),
            Self::ResumeInput => HeaderOnlyPdu::resume().encode(dst),
            Self::DismissHoveringTouchContact(pdu) => pdu.encode(dst),
            Self::Pen(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            Self::ScReady(pdu) => pdu.size(),
            Self::CsReady(pdu) => pdu.size(),
            Self::Touch(pdu) => pdu.size(),
            Self::SuspendInput | Self::ResumeInput => RDP_INPUT_HEADER_SIZE,
            Self::DismissHoveringTouchContact(pdu) => pdu.size(),
            Self::Pen(pdu) => pdu.size(),
        }
    }
}

impl<'de> Decode<'de> for RdpeiPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: RDP_INPUT_HEADER_SIZE);
        let bytes = src.remaining();
        let header_preview = {
            let mut tmp = ReadCursor::new(bytes);
            RdpInputHeader::decode(&mut tmp)?
        };
        let pdu_len: usize = cast_length!("pduLength", header_preview.pdu_length)?;
        ensure_size!(in: src, size: pdu_len);
        let pdu_bytes = src.read_slice(pdu_len);
        let mut pdu_src = ReadCursor::new(pdu_bytes);

        match header_preview.event_id {
            RdpInputEventId::ScReady => Ok(Self::ScReady(ScReadyPdu::decode(&mut pdu_src)?)),
            RdpInputEventId::CsReady => Ok(Self::CsReady(CsReadyPdu::decode(&mut pdu_src)?)),
            RdpInputEventId::Touch => Ok(Self::Touch(TouchEventPdu::decode(&mut pdu_src)?)),
            RdpInputEventId::SuspendInput => {
                HeaderOnlyPdu::decode(&mut pdu_src)?;
                Ok(Self::SuspendInput)
            }
            RdpInputEventId::ResumeInput => {
                HeaderOnlyPdu::decode(&mut pdu_src)?;
                Ok(Self::ResumeInput)
            }
            RdpInputEventId::DismissHoveringTouchContact => Ok(Self::DismissHoveringTouchContact(
                DismissHoveringTouchContactPdu::decode(&mut pdu_src)?,
            )),
            RdpInputEventId::Pen => Ok(Self::Pen(PenEventPdu::decode(&mut pdu_src)?)),
        }
    }
}

impl DvcEncode for RdpeiPdu {}

#[cfg(test)]
mod tests {
    use ironrdp_core::{decode, encode_vec};

    use super::*;

    #[test]
    fn sc_ready_round_trip() {
        let pdu = ScReadyPdu::new(RdpInputProtocolVersion::V200);
        let encoded = encode_vec(&pdu).unwrap();
        assert_eq!(decode::<ScReadyPdu>(&encoded).unwrap(), pdu);
        assert_eq!(decode::<RdpeiPdu>(&encoded).unwrap(), RdpeiPdu::ScReady(pdu));
    }

    #[test]
    fn cs_ready_round_trip() {
        let pdu = CsReadyPdu::new(CsReadyFlags::SHOW_TOUCH_VISUALS, RdpInputProtocolVersion::V200, 10);
        let encoded = encode_vec(&pdu).unwrap();
        assert_eq!(decode::<CsReadyPdu>(&encoded).unwrap(), pdu);
    }

    #[test]
    fn touch_event_round_trip() {
        let contact = TouchContact::new(
            1,
            100,
            200,
            TouchContactFlags::DOWN | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT,
        )
        .with_pressure(512);
        let frame = TouchFrame::new(0, vec![contact]);
        let pdu = TouchEventPdu::new(16, vec![frame]);
        let encoded = encode_vec(&pdu).unwrap();
        assert_eq!(decode::<TouchEventPdu>(&encoded).unwrap(), pdu);
        assert_eq!(decode::<RdpeiPdu>(&encoded).unwrap(), RdpeiPdu::Touch(pdu));
    }

    #[test]
    fn pen_event_round_trip() {
        let contact = PenContact::new(
            0,
            50,
            -20,
            PenContactFlags::UPDATE | PenContactFlags::INRANGE | PenContactFlags::INCONTACT,
        )
        .with_pressure(1024)
        .with_tilt(10, -5);
        let frame = PenFrame::new(1000, vec![contact]);
        let pdu = PenEventPdu::new(32, vec![frame]);
        let encoded = encode_vec(&pdu).unwrap();
        assert_eq!(decode::<PenEventPdu>(&encoded).unwrap(), pdu);
    }

    #[test]
    fn suspend_resume_dismiss_round_trip() {
        let suspend = encode_vec(&HeaderOnlyPdu::suspend()).unwrap();
        assert_eq!(decode::<RdpeiPdu>(&suspend).unwrap(), RdpeiPdu::SuspendInput);

        let resume = encode_vec(&HeaderOnlyPdu::resume()).unwrap();
        assert_eq!(decode::<RdpeiPdu>(&resume).unwrap(), RdpeiPdu::ResumeInput);

        let dismiss = DismissHoveringTouchContactPdu::new(3);
        let encoded = encode_vec(&dismiss).unwrap();
        assert_eq!(
            decode::<RdpeiPdu>(&encoded).unwrap(),
            RdpeiPdu::DismissHoveringTouchContact(dismiss)
        );
    }
}
