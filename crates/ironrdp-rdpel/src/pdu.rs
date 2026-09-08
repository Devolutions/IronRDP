//! Location Virtual Channel Extension PDUs defined by [MS-RDPEL].
//!
//! [MS-RDPEL]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpel/

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_size, invalid_field_err,
};
use ironrdp_dvc::DvcEncode;

const HEADER_SIZE: usize = 2 /* PduType */ + 4 /* PduLength */;
const MAX_SIGNED_INTEGER: u32 = 0x1FFF_FFFF;
const MAX_FLOAT_MANTISSA: u32 = 0x03FF_FFFF;
const MAX_FLOAT_EXPONENT: u8 = 7;
const MAX_COORDINATE_EXPONENT: u8 = 5;

const PDU_TYPE_SERVER_READY: u16 = 0x0001;
const PDU_TYPE_CLIENT_READY: u16 = 0x0002;
const PDU_TYPE_BASE_LOCATION3D: u16 = 0x0003;
const PDU_TYPE_LOCATION2D_DELTA: u16 = 0x0004;
const PDU_TYPE_LOCATION3D_DELTA: u16 = 0x0005;

/// Protocol versions from [MS-RDPEL] sections 2.2.2.1 and 2.2.2.2.
///
/// [MS-RDPEL]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpel/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ProtocolVersion {
    V1 = 0x0001_0000,
    V2 = 0x0002_0000,
}

/// Location source values from [MS-RDPEL] section 2.2.2.3.
///
/// [MS-RDPEL]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpel/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LocationSource {
    Ip = 0x00,
    Wifi = 0x01,
    Cellular = 0x02,
    Gnss = 0x03,
}

impl TryFrom<u8> for LocationSource {
    type Error = ironrdp_core::DecodeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(Self::Ip),
            0x01 => Ok(Self::Wifi),
            0x02 => Ok(Self::Cellular),
            0x03 => Ok(Self::Gnss),
            _ => Err(invalid_field_err!("source", "unknown location source")),
        }
    }
}

impl From<LocationSource> for u8 {
    #[expect(
        clippy::as_conversions,
        reason = "repr(u8) guarantees the discriminant layout and enum-to-primitive casts require as"
    )]
    fn from(value: LocationSource) -> Self {
        value as u8
    }
}

impl TryFrom<u32> for ProtocolVersion {
    type Error = ironrdp_core::DecodeError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x0001_0000 => Ok(Self::V1),
            0x0002_0000 => Ok(Self::V2),
            _ => Err(invalid_field_err!(
                "protocolVersion",
                "unknown location protocol version"
            )),
        }
    }
}

impl From<ProtocolVersion> for u32 {
    #[expect(
        clippy::as_conversions,
        reason = "repr(u32) guarantees the discriminant layout and enum-to-primitive casts require as"
    )]
    fn from(value: ProtocolVersion) -> Self {
        value as u32
    }
}

/// Variable-width signed integer from [MS-RDPEL] section 2.2.1.1.
///
/// [MS-RDPEL]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpel/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FourByteSignedInteger(i32);

impl FourByteSignedInteger {
    pub const MIN: i32 = -0x1FFF_FFFF;
    pub const MAX: i32 = 0x1FFF_FFFF;

    pub fn new(value: i32) -> EncodeResult<Self> {
        if !(Self::MIN..=Self::MAX).contains(&value) {
            return Err(invalid_field_err!(
                "value",
                "FOUR_BYTE_SIGNED_INTEGER exceeds its 29-bit magnitude"
            ));
        }

        Ok(Self(value))
    }

    pub fn value(self) -> i32 {
        self.0
    }
}

impl Encode for FourByteSignedInteger {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let magnitude = self.0.unsigned_abs();
        let byte_count = encoded_integer_byte_count(magnitude);
        ensure_size!(in: dst, size: byte_count);

        let mut first = u8::try_from((byte_count - 1) << 6).expect("encoded integer uses at most four bytes");
        if self.0 < 0 {
            first |= 0x20;
        }
        first |= u8::try_from((magnitude >> ((byte_count - 1) * 8)) & 0x1F)
            .expect("first integer magnitude fragment uses five bits");
        dst.write_u8(first);
        write_magnitude_tail(dst, magnitude, byte_count);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "FOUR_BYTE_SIGNED_INTEGER"
    }

    fn size(&self) -> usize {
        encoded_integer_byte_count(self.0.unsigned_abs())
    }
}

impl<'de> Decode<'de> for FourByteSignedInteger {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        let first = src.read_u8();
        let byte_count = usize::from((first >> 6) + 1);
        ensure_size!(in: src, size: byte_count - 1);

        let magnitude = read_magnitude_tail(src, u32::from(first & 0x1F), byte_count);
        if magnitude > MAX_SIGNED_INTEGER {
            return Err(invalid_field_err!(
                "value",
                "FOUR_BYTE_SIGNED_INTEGER magnitude is too large"
            ));
        }

        let value = i32::try_from(magnitude).expect("29-bit magnitude always fits in i32");
        Ok(Self(if first & 0x20 == 0 { value } else { -value }))
    }
}

/// Variable-width decimal value from [MS-RDPEL] section 2.2.1.2.
///
/// [MS-RDPEL]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpel/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FourByteFloat {
    magnitude: u32,
    exponent: u8,
    negative: bool,
}

impl FourByteFloat {
    pub fn new(value: f64) -> EncodeResult<Self> {
        Self::with_max_exponent(value, MAX_FLOAT_EXPONENT)
    }

    pub(crate) fn coordinate(value: f64) -> EncodeResult<Self> {
        Self::with_max_exponent(value, MAX_COORDINATE_EXPONENT)
    }

    pub(crate) fn from_coordinate_units(value: i32) -> Self {
        let mut magnitude = value.unsigned_abs();
        let mut exponent = MAX_COORDINATE_EXPONENT;
        while exponent != 0 && magnitude.is_multiple_of(10) {
            magnitude /= 10;
            exponent -= 1;
        }
        Self {
            magnitude,
            exponent,
            negative: value < 0,
        }
    }

    pub(crate) fn coordinate_units(self) -> i32 {
        let scale = 10i32.pow(u32::from(MAX_COORDINATE_EXPONENT - self.exponent));
        let magnitude = i32::try_from(self.magnitude).expect("26-bit coordinate magnitude fits in i32") * scale;
        if self.negative { -magnitude } else { magnitude }
    }

    fn with_max_exponent(value: f64, max_exponent: u8) -> EncodeResult<Self> {
        if !value.is_finite() {
            return Err(invalid_field_err!("value", "FOUR_BYTE_FLOAT must be finite"));
        }

        let mut scaled = value.abs();
        let mut exponent = 0;
        loop {
            #[expect(
                clippy::float_cmp,
                reason = "the wire exponent increases until the scaled value is exactly integral"
            )]
            let is_integral = scaled == scaled.floor();
            if is_integral || exponent == max_exponent || scaled * 10.0 > f64::from(MAX_FLOAT_MANTISSA) {
                break;
            }
            scaled *= 10.0;
            exponent += 1;
        }

        if scaled > f64::from(MAX_FLOAT_MANTISSA) {
            return Err(invalid_field_err!("value", "FOUR_BYTE_FLOAT magnitude is too large"));
        }

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::as_conversions,
            reason = "the finite non-negative value is bounded to 26 bits immediately above"
        )]
        let magnitude = scaled as u32;
        Ok(Self {
            magnitude,
            exponent,
            negative: value < 0.0,
        })
    }

    pub fn value(self) -> f64 {
        let value = f64::from(self.magnitude) / 10f64.powi(i32::from(self.exponent));
        if self.negative { -value } else { value }
    }
}

impl Encode for FourByteFloat {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let byte_count = encoded_float_byte_count(self.magnitude);
        ensure_size!(in: dst, size: byte_count);

        let mut first = u8::try_from((byte_count - 1) << 6).expect("encoded float uses at most four bytes");
        if self.negative {
            first |= 0x20;
        }
        first |= self.exponent << 2;
        first |= u8::try_from((self.magnitude >> ((byte_count - 1) * 8)) & 0x03)
            .expect("first float magnitude fragment uses two bits");
        dst.write_u8(first);
        write_magnitude_tail(dst, self.magnitude, byte_count);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "FOUR_BYTE_FLOAT"
    }

    fn size(&self) -> usize {
        encoded_float_byte_count(self.magnitude)
    }
}

impl<'de> Decode<'de> for FourByteFloat {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        let first = src.read_u8();
        let byte_count = usize::from((first >> 6) + 1);
        ensure_size!(in: src, size: byte_count - 1);

        let magnitude = read_magnitude_tail(src, u32::from(first & 0x03), byte_count);
        if magnitude > MAX_FLOAT_MANTISSA {
            return Err(invalid_field_err!("value", "FOUR_BYTE_FLOAT magnitude is too large"));
        }

        Ok(Self {
            magnitude,
            exponent: (first >> 2) & 0x07,
            negative: first & 0x20 != 0,
        })
    }
}

/// Ready PDU body shared by the server and client ready messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadyPdu {
    pub protocol_version: ProtocolVersion,
    pub flags: Option<u32>,
}

impl ReadyPdu {
    pub fn v1() -> Self {
        Self {
            protocol_version: ProtocolVersion::V1,
            flags: Some(0),
        }
    }

    fn body_size(&self) -> usize {
        4 /* ProtocolVersion */ + self.flags.map_or(0, |_| 4 /* Flags */)
    }

    fn encode_body(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if self.flags.is_some_and(|flags| flags != 0) {
            return Err(invalid_field_err!("flags", "location ready flags must be zero"));
        }
        ensure_size!(in: dst, size: self.body_size());
        dst.write_u32(self.protocol_version.into());
        if let Some(flags) = self.flags {
            dst.write_u32(flags);
        }
        Ok(())
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4);
        let protocol_version = ProtocolVersion::try_from(src.read_u32())?;
        let flags = match src.len() {
            0 => None,
            4 => {
                let flags = src.read_u32();
                if flags != 0 {
                    return Err(invalid_field_err!("flags", "location ready flags must be zero"));
                }
                Some(flags)
            }
            _ => {
                return Err(invalid_field_err!(
                    "flags",
                    "ready PDU has an invalid optional flags field"
                ));
            }
        };
        Ok(Self {
            protocol_version,
            flags,
        })
    }
}

/// Absolute location and optional version 2 attributes from [MS-RDPEL] section 2.2.2.3.
///
/// [MS-RDPEL]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpel/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseLocation3dPdu {
    pub latitude: FourByteFloat,
    pub longitude: FourByteFloat,
    pub altitude: FourByteSignedInteger,
    pub speed: Option<FourByteFloat>,
    pub heading: Option<FourByteFloat>,
    pub horizontal_accuracy: Option<FourByteFloat>,
    pub source: Option<LocationSource>,
}

impl BaseLocation3dPdu {
    pub fn coordinates(latitude: f64, longitude: f64, altitude: i32) -> EncodeResult<Self> {
        Ok(Self {
            latitude: FourByteFloat::coordinate(latitude)?,
            longitude: FourByteFloat::coordinate(longitude)?,
            altitude: FourByteSignedInteger::new(altitude)?,
            speed: None,
            heading: None,
            horizontal_accuracy: None,
            source: None,
        })
    }

    fn body_size(&self) -> usize {
        self.latitude.size()
            + self.longitude.size()
            + self.altitude.size()
            + self.speed.map_or(0, |value| value.size())
            + self.heading.map_or(0, |value| value.size())
            + self.horizontal_accuracy.map_or(0, |value| value.size())
            + usize::from(self.source.is_some())
    }

    fn validate_optional_fields(&self) -> EncodeResult<()> {
        if self.speed.is_some() != self.heading.is_some()
            || self.heading.is_some() != self.horizontal_accuracy.is_some()
            || self.horizontal_accuracy.is_some() != self.source.is_some()
        {
            return Err(invalid_field_err!(
                "optionalFields",
                "base location version 2 optional fields must all be present or absent"
            ));
        }
        Ok(())
    }

    fn encode_body(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        self.validate_optional_fields()?;
        self.latitude.encode(dst)?;
        self.longitude.encode(dst)?;
        self.altitude.encode(dst)?;
        if let Some(value) = self.speed {
            value.encode(dst)?;
        }
        if let Some(value) = self.heading {
            value.encode(dst)?;
        }
        if let Some(value) = self.horizontal_accuracy {
            value.encode(dst)?;
        }
        if let Some(value) = self.source {
            ensure_size!(in: dst, size: 1);
            dst.write_u8(value.into());
        }
        Ok(())
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let latitude = FourByteFloat::decode(src)?;
        let longitude = FourByteFloat::decode(src)?;
        let altitude = FourByteSignedInteger::decode(src)?;
        let (speed, heading, horizontal_accuracy, source) = if src.is_empty() {
            (None, None, None, None)
        } else {
            let speed = Some(FourByteFloat::decode(src)?);
            let heading = Some(FourByteFloat::decode(src)?);
            let horizontal_accuracy = Some(FourByteFloat::decode(src)?);
            ensure_size!(in: src, size: 1);
            let source = Some(LocationSource::try_from(src.read_u8())?);
            (speed, heading, horizontal_accuracy, source)
        };
        Ok(Self {
            latitude,
            longitude,
            altitude,
            speed,
            heading,
            horizontal_accuracy,
            source,
        })
    }
}

/// Latitude and longitude delta update from [MS-RDPEL] section 2.2.2.4.
///
/// [MS-RDPEL]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpel/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location2dDeltaPdu {
    pub latitude_delta: FourByteFloat,
    pub longitude_delta: FourByteFloat,
    pub speed_delta: Option<FourByteFloat>,
    pub heading_delta: Option<FourByteFloat>,
}

impl Location2dDeltaPdu {
    pub fn coordinates(latitude_delta: f64, longitude_delta: f64) -> EncodeResult<Self> {
        Ok(Self {
            latitude_delta: FourByteFloat::coordinate(latitude_delta)?,
            longitude_delta: FourByteFloat::coordinate(longitude_delta)?,
            speed_delta: None,
            heading_delta: None,
        })
    }

    fn body_size(&self) -> usize {
        self.latitude_delta.size()
            + self.longitude_delta.size()
            + self.speed_delta.map_or(0, |value| value.size())
            + self.heading_delta.map_or(0, |value| value.size())
    }

    fn encode_body(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if self.heading_delta.is_some() != self.speed_delta.is_some() {
            return Err(invalid_field_err!(
                "optionalFields",
                "speed and heading deltas must both be present or absent"
            ));
        }
        self.latitude_delta.encode(dst)?;
        self.longitude_delta.encode(dst)?;
        if let Some(value) = self.speed_delta {
            value.encode(dst)?;
        }
        if let Some(value) = self.heading_delta {
            value.encode(dst)?;
        }
        Ok(())
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let latitude_delta = FourByteFloat::decode(src)?;
        let longitude_delta = FourByteFloat::decode(src)?;
        let (speed_delta, heading_delta) = if src.is_empty() {
            (None, None)
        } else {
            (Some(FourByteFloat::decode(src)?), Some(FourByteFloat::decode(src)?))
        };
        Ok(Self {
            latitude_delta,
            longitude_delta,
            speed_delta,
            heading_delta,
        })
    }
}

/// Latitude, longitude, and altitude delta update from [MS-RDPEL] section 2.2.2.5.
///
/// [MS-RDPEL]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpel/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location3dDeltaPdu {
    pub latitude_delta: FourByteFloat,
    pub longitude_delta: FourByteFloat,
    pub altitude_delta: FourByteSignedInteger,
    pub speed_delta: Option<FourByteFloat>,
    pub heading_delta: Option<FourByteFloat>,
}

impl Location3dDeltaPdu {
    pub fn coordinates(latitude_delta: f64, longitude_delta: f64, altitude_delta: i32) -> EncodeResult<Self> {
        Ok(Self {
            latitude_delta: FourByteFloat::coordinate(latitude_delta)?,
            longitude_delta: FourByteFloat::coordinate(longitude_delta)?,
            altitude_delta: FourByteSignedInteger::new(altitude_delta)?,
            speed_delta: None,
            heading_delta: None,
        })
    }

    fn body_size(&self) -> usize {
        self.latitude_delta.size()
            + self.longitude_delta.size()
            + self.altitude_delta.size()
            + self.speed_delta.map_or(0, |value| value.size())
            + self.heading_delta.map_or(0, |value| value.size())
    }

    fn encode_body(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if self.heading_delta.is_some() != self.speed_delta.is_some() {
            return Err(invalid_field_err!(
                "optionalFields",
                "speed and heading deltas must both be present or absent"
            ));
        }
        self.latitude_delta.encode(dst)?;
        self.longitude_delta.encode(dst)?;
        self.altitude_delta.encode(dst)?;
        if let Some(value) = self.speed_delta {
            value.encode(dst)?;
        }
        if let Some(value) = self.heading_delta {
            value.encode(dst)?;
        }
        Ok(())
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let latitude_delta = FourByteFloat::decode(src)?;
        let longitude_delta = FourByteFloat::decode(src)?;
        let altitude_delta = FourByteSignedInteger::decode(src)?;
        let (speed_delta, heading_delta) = if src.is_empty() {
            (None, None)
        } else {
            (Some(FourByteFloat::decode(src)?), Some(FourByteFloat::decode(src)?))
        };
        Ok(Self {
            latitude_delta,
            longitude_delta,
            altitude_delta,
            speed_delta,
            heading_delta,
        })
    }
}

/// A complete location virtual channel PDU with the header from [MS-RDPEL] section 2.2.1.3.
///
/// [MS-RDPEL]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpel/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationPdu {
    ServerReady(ReadyPdu),
    ClientReady(ReadyPdu),
    BaseLocation3d(BaseLocation3dPdu),
    Location2dDelta(Location2dDeltaPdu),
    Location3dDelta(Location3dDeltaPdu),
}

impl LocationPdu {
    const NAME: &'static str = "RDPLOCATION_PDU";

    fn pdu_type(self) -> u16 {
        match self {
            Self::ServerReady(_) => PDU_TYPE_SERVER_READY,
            Self::ClientReady(_) => PDU_TYPE_CLIENT_READY,
            Self::BaseLocation3d(_) => PDU_TYPE_BASE_LOCATION3D,
            Self::Location2dDelta(_) => PDU_TYPE_LOCATION2D_DELTA,
            Self::Location3dDelta(_) => PDU_TYPE_LOCATION3D_DELTA,
        }
    }

    fn body_size(self) -> usize {
        match self {
            Self::ServerReady(pdu) | Self::ClientReady(pdu) => pdu.body_size(),
            Self::BaseLocation3d(pdu) => pdu.body_size(),
            Self::Location2dDelta(pdu) => pdu.body_size(),
            Self::Location3dDelta(pdu) => pdu.body_size(),
        }
    }
}

impl Encode for LocationPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u16(self.pdu_type());
        dst.write_u32(cast_length!("pduLength", self.size())?);
        match self {
            Self::ServerReady(pdu) | Self::ClientReady(pdu) => pdu.encode_body(dst),
            Self::BaseLocation3d(pdu) => pdu.encode_body(dst),
            Self::Location2dDelta(pdu) => pdu.encode_body(dst),
            Self::Location3dDelta(pdu) => pdu.encode_body(dst),
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        HEADER_SIZE + self.body_size()
    }
}

impl<'de> Decode<'de> for LocationPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: HEADER_SIZE);
        let pdu_type = src.read_u16();
        let pdu_length = usize::try_from(src.read_u32()).unwrap_or(usize::MAX);
        if pdu_length != HEADER_SIZE + src.len() {
            return Err(invalid_field_err!(
                "pduLength",
                "location PDU length does not match its payload"
            ));
        }

        let pdu = match pdu_type {
            PDU_TYPE_SERVER_READY => Self::ServerReady(ReadyPdu::decode_body(src)?),
            PDU_TYPE_CLIENT_READY => Self::ClientReady(ReadyPdu::decode_body(src)?),
            PDU_TYPE_BASE_LOCATION3D => Self::BaseLocation3d(BaseLocation3dPdu::decode_body(src)?),
            PDU_TYPE_LOCATION2D_DELTA => Self::Location2dDelta(Location2dDeltaPdu::decode_body(src)?),
            PDU_TYPE_LOCATION3D_DELTA => Self::Location3dDelta(Location3dDeltaPdu::decode_body(src)?),
            _ => return Err(invalid_field_err!("pduType", "unknown location PDU type")),
        };

        if !src.is_empty() {
            return Err(invalid_field_err!(
                "pduLength",
                "trailing bytes after location PDU body"
            ));
        }
        Ok(pdu)
    }
}

impl DvcEncode for LocationPdu {}

fn encoded_integer_byte_count(magnitude: u32) -> usize {
    match magnitude {
        0..=0x1F => 1,
        0x20..=0x1FFF => 2,
        0x2000..=0x1F_FFFF => 3,
        _ => 4,
    }
}

fn encoded_float_byte_count(magnitude: u32) -> usize {
    match magnitude {
        0..=0x03 => 1,
        0x04..=0x03FF => 2,
        0x0400..=0x03_FFFF => 3,
        _ => 4,
    }
}

fn write_magnitude_tail(dst: &mut WriteCursor<'_>, magnitude: u32, byte_count: usize) {
    let bytes = magnitude.to_be_bytes();
    dst.write_slice(&bytes[4 - (byte_count - 1)..]);
}

fn read_magnitude_tail(src: &mut ReadCursor<'_>, first_magnitude: u32, byte_count: usize) -> u32 {
    let mut magnitude = first_magnitude;
    for _ in 1..byte_count {
        magnitude = (magnitude << 8) | u32::from(src.read_u8());
    }
    magnitude
}
