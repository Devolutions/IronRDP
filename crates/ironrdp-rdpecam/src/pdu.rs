//! Video Capture Virtual Channel Extension PDUs defined by [MS-RDPECAM].
//!
//! [MS-RDPECAM]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/

use alloc::string::String;
use alloc::vec::Vec;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size, invalid_field_err,
};
use ironrdp_dvc::DvcEncode;

const HEADER_SIZE: usize = 1 /* Version */ + 1 /* MessageId */;
const STREAM_DESCRIPTION_SIZE: usize =
    2 /* FrameSourceTypes */ + 1 /* StreamCategory */ + 1 /* Selected */ + 1 /* CanBeShared */;
const MEDIA_TYPE_SIZE: usize = 1 // Format
    + 4 // Width
    + 4 // Height
    + 4 // FrameRateNumerator
    + 4 // FrameRateDenominator
    + 4 // PixelAspectRatioNumerator
    + 4 // PixelAspectRatioDenominator
    + 1; // Flags
const START_STREAM_INFO_SIZE: usize = 1 /* StreamIndex */ + MEDIA_TYPE_SIZE;

const MESSAGE_SUCCESS_RESPONSE: u8 = 0x01;
const MESSAGE_ERROR_RESPONSE: u8 = 0x02;
const MESSAGE_SELECT_VERSION_REQUEST: u8 = 0x03;
const MESSAGE_SELECT_VERSION_RESPONSE: u8 = 0x04;
const MESSAGE_DEVICE_ADDED_NOTIFICATION: u8 = 0x05;
const MESSAGE_DEVICE_REMOVED_NOTIFICATION: u8 = 0x06;
const MESSAGE_ACTIVATE_DEVICE_REQUEST: u8 = 0x07;
const MESSAGE_DEACTIVATE_DEVICE_REQUEST: u8 = 0x08;
const MESSAGE_STREAM_LIST_REQUEST: u8 = 0x09;
const MESSAGE_STREAM_LIST_RESPONSE: u8 = 0x0A;
const MESSAGE_MEDIA_TYPE_LIST_REQUEST: u8 = 0x0B;
const MESSAGE_MEDIA_TYPE_LIST_RESPONSE: u8 = 0x0C;
const MESSAGE_CURRENT_MEDIA_TYPE_REQUEST: u8 = 0x0D;
const MESSAGE_CURRENT_MEDIA_TYPE_RESPONSE: u8 = 0x0E;
const MESSAGE_START_STREAMS_REQUEST: u8 = 0x0F;
const MESSAGE_STOP_STREAMS_REQUEST: u8 = 0x10;
const MESSAGE_SAMPLE_REQUEST: u8 = 0x11;
const MESSAGE_SAMPLE_RESPONSE: u8 = 0x12;
const MESSAGE_SAMPLE_ERROR_RESPONSE: u8 = 0x13;

/// Largest accepted width or height.
pub const MAX_FRAME_DIMENSION: u32 = 8192;
/// Largest accepted uncompressed frame or encoded Sample Response.
pub const MAX_SAMPLE_SIZE: usize = 64 * 1024 * 1024;
/// Largest accepted media-type list.
pub const MAX_MEDIA_TYPES: usize = 4096;
/// Largest accepted device display name, in UTF-16 code units.
pub const MAX_DEVICE_NAME_LEN: usize = 1024;
/// Largest accepted device channel name, excluding its terminator.
pub const MAX_CHANNEL_NAME_LEN: usize = 256;

/// MS-RDPECAM protocol versions supported by this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ProtocolVersion {
    V1 = 1,
}

impl TryFrom<u8> for ProtocolVersion {
    type Error = ironrdp_core::DecodeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::V1),
            _ => Err(invalid_field_err!("Version", "unsupported RDPECAM protocol version")),
        }
    }
}

impl From<ProtocolVersion> for u8 {
    fn from(value: ProtocolVersion) -> Self {
        match value {
            ProtocolVersion::V1 => 1,
        }
    }
}

/// Error codes from [2.2.3.2] Error Response.
///
/// [2.2.3.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ErrorCode {
    Unexpected = 0x0000_0001,
    InvalidMessage = 0x0000_0002,
    NotInitialized = 0x0000_0003,
    InvalidRequest = 0x0000_0004,
    InvalidStreamNumber = 0x0000_0005,
    InvalidMediaType = 0x0000_0006,
    OutOfMemory = 0x0000_0007,
}

impl TryFrom<u32> for ErrorCode {
    type Error = ironrdp_core::DecodeError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Unexpected),
            2 => Ok(Self::InvalidMessage),
            3 => Ok(Self::NotInitialized),
            4 => Ok(Self::InvalidRequest),
            5 => Ok(Self::InvalidStreamNumber),
            6 => Ok(Self::InvalidMediaType),
            7 => Ok(Self::OutOfMemory),
            _ => Err(invalid_field_err!(
                "ErrorCode",
                "unsupported RDPECAM version 1 error code"
            )),
        }
    }
}

impl From<ErrorCode> for u32 {
    fn from(value: ErrorCode) -> Self {
        match value {
            ErrorCode::Unexpected => 1,
            ErrorCode::InvalidMessage => 2,
            ErrorCode::NotInitialized => 3,
            ErrorCode::InvalidRequest => 4,
            ErrorCode::InvalidStreamNumber => 5,
            ErrorCode::InvalidMediaType => 6,
            ErrorCode::OutOfMemory => 7,
        }
    }
}

/// Format values from [2.2.3.8.1] MEDIA_TYPE_DESCRIPTION.
///
/// [2.2.3.8.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MediaFormat {
    H264 = 0x01,
    Mjpg = 0x02,
    Yuy2 = 0x03,
    Nv12 = 0x04,
    I420 = 0x05,
    Rgb24 = 0x06,
    Rgb32 = 0x07,
}

impl TryFrom<u8> for MediaFormat {
    type Error = ironrdp_core::DecodeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::H264),
            2 => Ok(Self::Mjpg),
            3 => Ok(Self::Yuy2),
            4 => Ok(Self::Nv12),
            5 => Ok(Self::I420),
            6 => Ok(Self::Rgb24),
            7 => Ok(Self::Rgb32),
            _ => Err(invalid_field_err!("Format", "unknown RDPECAM media format")),
        }
    }
}

impl From<MediaFormat> for u8 {
    fn from(value: MediaFormat) -> Self {
        match value {
            MediaFormat::H264 => 1,
            MediaFormat::Mjpg => 2,
            MediaFormat::Yuy2 => 3,
            MediaFormat::Nv12 => 4,
            MediaFormat::I420 => 5,
            MediaFormat::Rgb24 => 6,
            MediaFormat::Rgb32 => 7,
        }
    }
}

/// Description of a redirected device and its dedicated DVC listener.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDescriptor {
    pub display_name: String,
    pub channel_name: String,
}

impl DeviceDescriptor {
    pub fn new(display_name: String, channel_name: String) -> EncodeResult<Self> {
        let descriptor = Self {
            display_name,
            channel_name,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub(crate) fn validate(&self) -> EncodeResult<()> {
        validate_device_name(&self.display_name)?;
        validate_channel_name(&self.channel_name)
    }
}

/// Description of one camera stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamDescription {
    /// Frame-source advertisement bits.
    ///
    /// Unknown bits are retained for forward-compatible round trips.
    pub frame_source_types: u16,
    pub selected: bool,
    pub can_be_shared: bool,
}

impl StreamDescription {
    pub const COLOR: u16 = 0x0001;
    pub const INFRARED: u16 = 0x0002;
    pub const CUSTOM: u16 = 0x0008;

    pub fn color(selected: bool, can_be_shared: bool) -> Self {
        Self {
            frame_source_types: Self::COLOR,
            selected,
            can_be_shared,
        }
    }

    fn validate(&self) -> EncodeResult<()> {
        if self.frame_source_types == 0 {
            return Err(invalid_field_err!(
                "FrameSourceTypes",
                "at least one frame source type is required"
            ));
        }
        Ok(())
    }
}

impl Encode for StreamDescription {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        self.validate()?;
        ensure_size!(in: dst, size: STREAM_DESCRIPTION_SIZE);
        dst.write_u16(self.frame_source_types);
        dst.write_u8(0x01);
        dst.write_u8(u8::from(self.selected));
        dst.write_u8(u8::from(self.can_be_shared));
        Ok(())
    }

    fn name(&self) -> &'static str {
        "STREAM_DESCRIPTION"
    }

    fn size(&self) -> usize {
        STREAM_DESCRIPTION_SIZE
    }
}

impl<'de> Decode<'de> for StreamDescription {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: STREAM_DESCRIPTION_SIZE);
        let frame_source_types = src.read_u16();
        if frame_source_types == 0 {
            return Err(invalid_field_err!(
                "FrameSourceTypes",
                "at least one frame source type is required",
                in: src
            ));
        }
        if src.read_u8() != 0x01 {
            return Err(invalid_field_err!(
                "StreamCategory",
                "only capture streams are supported",
                in: src
            ));
        }
        let selected = decode_bool("Selected", src)?;
        let can_be_shared = decode_bool("CanBeShared", src)?;
        Ok(Self {
            frame_source_types,
            selected,
            can_be_shared,
        })
    }
}

/// Stream format properties from [2.2.3.8.1] MEDIA_TYPE_DESCRIPTION.
///
/// [2.2.3.8.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpecam/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaType {
    pub format: MediaFormat,
    pub width: u32,
    pub height: u32,
    pub frame_rate_numerator: u32,
    pub frame_rate_denominator: u32,
    pub pixel_aspect_ratio_numerator: u32,
    pub pixel_aspect_ratio_denominator: u32,
    /// Media advertisement bits.
    ///
    /// Unknown bits are retained for forward-compatible round trips.
    pub flags: u8,
}

impl MediaType {
    pub const DECODING_REQUIRED: u8 = 0x01;
    pub const BOTTOM_UP_IMAGE: u8 = 0x02;

    #[expect(
        clippy::too_many_arguments,
        reason = "arguments map directly to the wire description"
    )]
    pub fn new(
        format: MediaFormat,
        width: u32,
        height: u32,
        frame_rate_numerator: u32,
        frame_rate_denominator: u32,
        pixel_aspect_ratio_numerator: u32,
        pixel_aspect_ratio_denominator: u32,
        flags: u8,
    ) -> EncodeResult<Self> {
        let media_type = Self {
            format,
            width,
            height,
            frame_rate_numerator,
            frame_rate_denominator,
            pixel_aspect_ratio_numerator,
            pixel_aspect_ratio_denominator,
            flags,
        };
        media_type.validate_for_backend()?;
        Ok(media_type)
    }

    /// Required sample length for an uncompressed format.
    ///
    /// Compressed H.264 and MJPEG formats return `None`.
    pub fn uncompressed_sample_len(&self) -> Option<usize> {
        let width = usize::try_from(self.width).ok()?;
        let height = usize::try_from(self.height).ok()?;
        let pixels = width.checked_mul(height)?;
        match self.format {
            MediaFormat::H264 | MediaFormat::Mjpg => None,
            MediaFormat::Yuy2 => pixels.checked_mul(2),
            MediaFormat::Nv12 | MediaFormat::I420 => pixels.checked_mul(3)?.checked_div(2),
            MediaFormat::Rgb24 => pixels.checked_mul(3),
            MediaFormat::Rgb32 => pixels.checked_mul(4),
        }
    }

    pub fn validate_sample(&self, sample: &[u8]) -> bool {
        match self.uncompressed_sample_len() {
            Some(expected) => sample.len() == expected,
            None => false,
        }
    }

    pub(crate) fn validate_for_backend(&self) -> EncodeResult<()> {
        if self.width == 0 || self.height == 0 || self.width > MAX_FRAME_DIMENSION || self.height > MAX_FRAME_DIMENSION
        {
            return Err(invalid_field_err!(
                "Width/Height",
                "frame dimensions must be between 1 and 8192 pixels"
            ));
        }
        if self.frame_rate_numerator == 0 || self.frame_rate_denominator == 0 {
            return Err(invalid_field_err!(
                "FrameRate",
                "frame rate numerator and denominator must be nonzero"
            ));
        }
        if self.pixel_aspect_ratio_numerator == 0 || self.pixel_aspect_ratio_denominator == 0 {
            return Err(invalid_field_err!(
                "PixelAspectRatio",
                "pixel aspect ratio numerator and denominator must be nonzero"
            ));
        }
        if matches!(self.format, MediaFormat::Yuy2) && !self.width.is_multiple_of(2) {
            return Err(invalid_field_err!("Width", "YUY2 width must be even"));
        }
        if matches!(self.format, MediaFormat::Nv12 | MediaFormat::I420)
            && (!self.width.is_multiple_of(2) || !self.height.is_multiple_of(2))
        {
            return Err(invalid_field_err!(
                "Width/Height",
                "4:2:0 frame dimensions must be even"
            ));
        }
        if self
            .uncompressed_sample_len()
            .is_some_and(|length| length > MAX_SAMPLE_SIZE)
        {
            return Err(invalid_field_err!(
                "Width/Height",
                "uncompressed frame exceeds the sample bound"
            ));
        }
        Ok(())
    }
}

impl Encode for MediaType {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: MEDIA_TYPE_SIZE);
        dst.write_u8(self.format.into());
        dst.write_u32(self.width);
        dst.write_u32(self.height);
        dst.write_u32(self.frame_rate_numerator);
        dst.write_u32(self.frame_rate_denominator);
        dst.write_u32(self.pixel_aspect_ratio_numerator);
        dst.write_u32(self.pixel_aspect_ratio_denominator);
        dst.write_u8(self.flags);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "MEDIA_TYPE_DESCRIPTION"
    }

    fn size(&self) -> usize {
        MEDIA_TYPE_SIZE
    }
}

impl<'de> Decode<'de> for MediaType {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: MEDIA_TYPE_SIZE);
        Ok(Self {
            format: MediaFormat::try_from(src.read_u8())?,
            width: src.read_u32(),
            height: src.read_u32(),
            frame_rate_numerator: src.read_u32(),
            frame_rate_denominator: src.read_u32(),
            pixel_aspect_ratio_numerator: src.read_u32(),
            pixel_aspect_ratio_denominator: src.read_u32(),
            flags: src.read_u8(),
        })
    }
}

/// One selected stream and media type in a Start Streams Request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartStreamInfo {
    pub stream_index: u8,
    pub media_type: MediaType,
}

impl Encode for StartStreamInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: START_STREAM_INFO_SIZE);
        dst.write_u8(self.stream_index);
        self.media_type.encode(dst)
    }

    fn name(&self) -> &'static str {
        "START_STREAM_INFO"
    }

    fn size(&self) -> usize {
        START_STREAM_INFO_SIZE
    }
}

impl<'de> Decode<'de> for StartStreamInfo {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: START_STREAM_INFO_SIZE);
        Ok(Self {
            stream_index: src.read_u8(),
            media_type: MediaType::decode(src)?,
        })
    }
}

/// Messages exchanged on the RDPECAM device-enumeration channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnumerationPdu {
    SelectVersionRequest(ProtocolVersion),
    SelectVersionResponse(ProtocolVersion),
    DeviceAdded {
        version: ProtocolVersion,
        device: DeviceDescriptor,
    },
    DeviceRemoved {
        version: ProtocolVersion,
        channel_name: String,
    },
}

impl EnumerationPdu {
    fn header(&self) -> (ProtocolVersion, u8) {
        match self {
            Self::SelectVersionRequest(version) => (*version, MESSAGE_SELECT_VERSION_REQUEST),
            Self::SelectVersionResponse(version) => (*version, MESSAGE_SELECT_VERSION_RESPONSE),
            Self::DeviceAdded { version, .. } => (*version, MESSAGE_DEVICE_ADDED_NOTIFICATION),
            Self::DeviceRemoved { version, .. } => (*version, MESSAGE_DEVICE_REMOVED_NOTIFICATION),
        }
    }
}

impl Encode for EnumerationPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        let (version, message_id) = self.header();
        encode_header(dst, version, message_id);
        match self {
            Self::SelectVersionRequest(_) | Self::SelectVersionResponse(_) => {}
            Self::DeviceAdded { device, .. } => {
                validate_device_name(&device.display_name)?;
                validate_channel_name(&device.channel_name)?;
                for unit in device.display_name.encode_utf16() {
                    dst.write_u16(unit);
                }
                dst.write_u16(0);
                dst.write_slice(device.channel_name.as_bytes());
                dst.write_u8(0);
            }
            Self::DeviceRemoved { channel_name, .. } => {
                validate_channel_name(channel_name)?;
                dst.write_slice(channel_name.as_bytes());
                dst.write_u8(0);
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "RDPECAM_ENUMERATION_PDU"
    }

    fn size(&self) -> usize {
        match self {
            Self::SelectVersionRequest(_) | Self::SelectVersionResponse(_) => HEADER_SIZE,
            Self::DeviceAdded { device, .. } => {
                HEADER_SIZE + device.display_name.encode_utf16().count() * 2 + 2 + device.channel_name.len() + 1
            }
            Self::DeviceRemoved { channel_name, .. } => HEADER_SIZE + channel_name.len() + 1,
        }
    }
}

impl<'de> Decode<'de> for EnumerationPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let (version, message_id) = decode_header(src)?;
        match message_id {
            MESSAGE_SELECT_VERSION_REQUEST => {
                ensure_empty(src)?;
                Ok(Self::SelectVersionRequest(version))
            }
            MESSAGE_SELECT_VERSION_RESPONSE => {
                ensure_empty(src)?;
                Ok(Self::SelectVersionResponse(version))
            }
            MESSAGE_DEVICE_ADDED_NOTIFICATION => {
                let display_name = decode_utf16z(src)?;
                let channel_name = decode_ansiz(src)?;
                ensure_empty(src)?;
                Ok(Self::DeviceAdded {
                    version,
                    device: DeviceDescriptor {
                        display_name,
                        channel_name,
                    },
                })
            }
            MESSAGE_DEVICE_REMOVED_NOTIFICATION => {
                let channel_name = decode_ansiz(src)?;
                ensure_empty(src)?;
                Ok(Self::DeviceRemoved { version, channel_name })
            }
            _ => Err(invalid_field_err!(
                "MessageId",
                "message is not valid on the enumeration channel",
                in: src
            )),
        }
    }
}

impl DvcEncode for EnumerationPdu {}

/// Version 1 messages exchanged on a per-device RDPECAM channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePdu {
    SuccessResponse,
    ErrorResponse(ErrorCode),
    ActivateDeviceRequest,
    DeactivateDeviceRequest,
    StreamListRequest,
    StreamListResponse(Vec<StreamDescription>),
    MediaTypeListRequest(u8),
    MediaTypeListResponse(Vec<MediaType>),
    CurrentMediaTypeRequest(u8),
    CurrentMediaTypeResponse(MediaType),
    StartStreamsRequest(Vec<StartStreamInfo>),
    StopStreamsRequest,
    SampleRequest(u8),
    SampleResponse { stream_index: u8, sample: Vec<u8> },
    SampleErrorResponse { stream_index: u8, error: ErrorCode },
}

impl DevicePdu {
    fn message_id(&self) -> u8 {
        match self {
            Self::SuccessResponse => MESSAGE_SUCCESS_RESPONSE,
            Self::ErrorResponse(_) => MESSAGE_ERROR_RESPONSE,
            Self::ActivateDeviceRequest => MESSAGE_ACTIVATE_DEVICE_REQUEST,
            Self::DeactivateDeviceRequest => MESSAGE_DEACTIVATE_DEVICE_REQUEST,
            Self::StreamListRequest => MESSAGE_STREAM_LIST_REQUEST,
            Self::StreamListResponse(_) => MESSAGE_STREAM_LIST_RESPONSE,
            Self::MediaTypeListRequest(_) => MESSAGE_MEDIA_TYPE_LIST_REQUEST,
            Self::MediaTypeListResponse(_) => MESSAGE_MEDIA_TYPE_LIST_RESPONSE,
            Self::CurrentMediaTypeRequest(_) => MESSAGE_CURRENT_MEDIA_TYPE_REQUEST,
            Self::CurrentMediaTypeResponse(_) => MESSAGE_CURRENT_MEDIA_TYPE_RESPONSE,
            Self::StartStreamsRequest(_) => MESSAGE_START_STREAMS_REQUEST,
            Self::StopStreamsRequest => MESSAGE_STOP_STREAMS_REQUEST,
            Self::SampleRequest(_) => MESSAGE_SAMPLE_REQUEST,
            Self::SampleResponse { .. } => MESSAGE_SAMPLE_RESPONSE,
            Self::SampleErrorResponse { .. } => MESSAGE_SAMPLE_ERROR_RESPONSE,
        }
    }
}

impl Encode for DevicePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        encode_header(dst, ProtocolVersion::V1, self.message_id());
        match self {
            Self::SuccessResponse
            | Self::ActivateDeviceRequest
            | Self::DeactivateDeviceRequest
            | Self::StreamListRequest
            | Self::StopStreamsRequest => {}
            Self::ErrorResponse(error) => dst.write_u32((*error).into()),
            Self::StreamListResponse(streams) => {
                validate_u8_count("StreamDescriptions", streams.len())?;
                for stream in streams {
                    stream.encode(dst)?;
                }
            }
            Self::MediaTypeListRequest(stream_index)
            | Self::CurrentMediaTypeRequest(stream_index)
            | Self::SampleRequest(stream_index) => dst.write_u8(*stream_index),
            Self::MediaTypeListResponse(media_types) => {
                validate_media_type_count(media_types.len())?;
                for media_type in media_types {
                    media_type.encode(dst)?;
                }
            }
            Self::CurrentMediaTypeResponse(media_type) => media_type.encode(dst)?,
            Self::StartStreamsRequest(streams) => {
                validate_u8_count("StartStreamsInfo", streams.len())?;
                for stream in streams {
                    stream.encode(dst)?;
                }
            }
            Self::SampleResponse { stream_index, sample } => {
                if sample.len() > MAX_SAMPLE_SIZE {
                    return Err(invalid_field_err!(
                        "Sample",
                        "sample length is outside the supported bound"
                    ));
                }
                dst.write_u8(*stream_index);
                dst.write_slice(sample);
            }
            Self::SampleErrorResponse { stream_index, error } => {
                dst.write_u8(*stream_index);
                dst.write_u32((*error).into());
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "RDPECAM_DEVICE_PDU"
    }

    fn size(&self) -> usize {
        HEADER_SIZE
            + match self {
                Self::SuccessResponse
                | Self::ActivateDeviceRequest
                | Self::DeactivateDeviceRequest
                | Self::StreamListRequest
                | Self::StopStreamsRequest => 0,
                Self::ErrorResponse(_) => 4,
                Self::StreamListResponse(streams) => streams.len() * STREAM_DESCRIPTION_SIZE,
                Self::MediaTypeListRequest(_) | Self::CurrentMediaTypeRequest(_) | Self::SampleRequest(_) => 1,
                Self::MediaTypeListResponse(media_types) => media_types.len() * MEDIA_TYPE_SIZE,
                Self::CurrentMediaTypeResponse(_) => MEDIA_TYPE_SIZE,
                Self::StartStreamsRequest(streams) => streams.len() * START_STREAM_INFO_SIZE,
                Self::SampleResponse { sample, .. } => 1 + sample.len(),
                Self::SampleErrorResponse { .. } => 1 + 4,
            }
    }
}

impl<'de> Decode<'de> for DevicePdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let (_version, message_id) = decode_header(src)?;
        let pdu = match message_id {
            MESSAGE_SUCCESS_RESPONSE => Self::SuccessResponse,
            MESSAGE_ERROR_RESPONSE => {
                ensure_size!(in: src, size: 4);
                Self::ErrorResponse(ErrorCode::try_from(src.read_u32())?)
            }
            MESSAGE_ACTIVATE_DEVICE_REQUEST => Self::ActivateDeviceRequest,
            MESSAGE_DEACTIVATE_DEVICE_REQUEST => Self::DeactivateDeviceRequest,
            MESSAGE_STREAM_LIST_REQUEST => Self::StreamListRequest,
            MESSAGE_STREAM_LIST_RESPONSE => {
                let count = ensure_array(src, STREAM_DESCRIPTION_SIZE, usize::from(u8::MAX))?;
                let mut streams = Vec::with_capacity(count);
                while !src.is_empty() {
                    streams.push(StreamDescription::decode(src)?);
                }
                Self::StreamListResponse(streams)
            }
            MESSAGE_MEDIA_TYPE_LIST_REQUEST => {
                ensure_size!(in: src, size: 1);
                Self::MediaTypeListRequest(src.read_u8())
            }
            MESSAGE_MEDIA_TYPE_LIST_RESPONSE => {
                let count = ensure_array(src, MEDIA_TYPE_SIZE, MAX_MEDIA_TYPES)?;
                let mut media_types = Vec::with_capacity(count);
                while !src.is_empty() {
                    media_types.push(MediaType::decode(src)?);
                }
                Self::MediaTypeListResponse(media_types)
            }
            MESSAGE_CURRENT_MEDIA_TYPE_REQUEST => {
                ensure_size!(in: src, size: 1);
                Self::CurrentMediaTypeRequest(src.read_u8())
            }
            MESSAGE_CURRENT_MEDIA_TYPE_RESPONSE => Self::CurrentMediaTypeResponse(MediaType::decode(src)?),
            MESSAGE_START_STREAMS_REQUEST => {
                let count = ensure_array(src, START_STREAM_INFO_SIZE, usize::from(u8::MAX))?;
                let mut streams = Vec::with_capacity(count);
                while !src.is_empty() {
                    streams.push(StartStreamInfo::decode(src)?);
                }
                Self::StartStreamsRequest(streams)
            }
            MESSAGE_STOP_STREAMS_REQUEST => Self::StopStreamsRequest,
            MESSAGE_SAMPLE_REQUEST => {
                ensure_size!(in: src, size: 1);
                Self::SampleRequest(src.read_u8())
            }
            MESSAGE_SAMPLE_RESPONSE => {
                ensure_size!(in: src, size: 1);
                if src.len() - 1 > MAX_SAMPLE_SIZE {
                    return Err(invalid_field_err!("Sample", "sample exceeds the supported bound", in: src));
                }
                let stream_index = src.read_u8();
                let sample = src.read_remaining().to_vec();
                Self::SampleResponse { stream_index, sample }
            }
            MESSAGE_SAMPLE_ERROR_RESPONSE => {
                ensure_size!(in: src, size: 5);
                let stream_index = src.read_u8();
                let error = ErrorCode::try_from(src.read_u32())?;
                Self::SampleErrorResponse { stream_index, error }
            }
            _ => {
                return Err(invalid_field_err!(
                    "MessageId",
                    "message is not supported by RDPECAM version 1",
                    in: src
                ));
            }
        };
        ensure_empty(src)?;
        Ok(pdu)
    }
}

impl DvcEncode for DevicePdu {}

fn encode_header(dst: &mut WriteCursor<'_>, version: ProtocolVersion, message_id: u8) {
    dst.write_u8(version.into());
    dst.write_u8(message_id);
}

fn decode_header(src: &mut ReadCursor<'_>) -> DecodeResult<(ProtocolVersion, u8)> {
    ensure_size!(in: src, size: HEADER_SIZE);
    Ok((ProtocolVersion::try_from(src.read_u8())?, src.read_u8()))
}

fn decode_bool(field: &'static str, src: &mut ReadCursor<'_>) -> DecodeResult<bool> {
    match src.read_u8() {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid_field_err!(field, "boolean field must be zero or one", in: src)),
    }
}

fn decode_utf16z(src: &mut ReadCursor<'_>) -> DecodeResult<String> {
    let mut units = Vec::new();
    loop {
        ensure_size!(in: src, size: 2);
        let unit = src.read_u16();
        if unit == 0 {
            break;
        }
        if units.len() == MAX_DEVICE_NAME_LEN {
            return Err(invalid_field_err!("DeviceName", "device name exceeds the supported bound", in: src));
        }
        units.push(unit);
    }
    let value = String::from_utf16(&units)
        .map_err(|_| invalid_field_err!("DeviceName", "device name is not valid UTF-16", in: src))?;
    if value.is_empty() {
        return Err(invalid_field_err!("DeviceName", "device name must not be empty", in: src));
    }
    Ok(value)
}

fn decode_ansiz(src: &mut ReadCursor<'_>) -> DecodeResult<String> {
    let remaining = src.remaining();
    let terminator = remaining
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| invalid_field_err!("VirtualChannelName", "channel name is not terminated", in: src))?;
    if terminator == 0 || terminator > MAX_CHANNEL_NAME_LEN {
        return Err(invalid_field_err!(
            "VirtualChannelName",
            "channel name length is outside the supported bound",
            in: src
        ));
    }
    if !remaining[..terminator].is_ascii() {
        return Err(invalid_field_err!(
            "VirtualChannelName",
            "channel name must be ANSI-compatible ASCII",
            in: src
        ));
    }
    let bytes = src.read_slice(terminator);
    let value = core::str::from_utf8(bytes)
        .map_err(|_| invalid_field_err!("VirtualChannelName", "channel name is not valid ASCII", in: src))?;
    let value = String::from(value);
    let _terminator = src.read_u8();
    Ok(value)
}

fn validate_device_name(value: &str) -> EncodeResult<()> {
    let len = value.encode_utf16().count();
    if len == 0 || len > MAX_DEVICE_NAME_LEN || value.encode_utf16().any(|unit| unit == 0) {
        return Err(invalid_field_err!(
            "DeviceName",
            "device name must be nonempty, bounded, and contain no null"
        ));
    }
    Ok(())
}

pub(crate) fn validate_channel_name(value: &str) -> EncodeResult<()> {
    if value.is_empty() || value.len() > MAX_CHANNEL_NAME_LEN || !value.is_ascii() || value.as_bytes().contains(&0) {
        return Err(invalid_field_err!(
            "VirtualChannelName",
            "channel name must be nonempty bounded ASCII without null"
        ));
    }
    Ok(())
}

fn validate_u8_count(field: &'static str, count: usize) -> EncodeResult<()> {
    if !(1..=u8::MAX.into()).contains(&count) {
        return Err(invalid_field_err!(
            field,
            "array must contain between 1 and 255 entries"
        ));
    }
    Ok(())
}

fn validate_media_type_count(count: usize) -> EncodeResult<()> {
    if !(1..=MAX_MEDIA_TYPES).contains(&count) {
        return Err(invalid_field_err!(
            "MediaTypeDescriptions",
            "media-type array is empty or exceeds the supported bound"
        ));
    }
    Ok(())
}

fn ensure_array(src: &ReadCursor<'_>, item_size: usize, max_count: usize) -> DecodeResult<usize> {
    if src.is_empty() || !src.len().is_multiple_of(item_size) {
        return Err(invalid_field_err!(
            "MessageLength",
            "message body is not a nonempty array of fixed-size entries",
            in: src
        ));
    }
    let count = src.len() / item_size;
    if count > max_count {
        return Err(invalid_field_err!(
            "MessageLength",
            "message array exceeds the supported entry count",
            in: src
        ));
    }
    Ok(count)
}

fn ensure_empty(src: &ReadCursor<'_>) -> DecodeResult<()> {
    if !src.is_empty() {
        return Err(invalid_field_err!("MessageLength", "message contains trailing bytes", in: src));
    }
    Ok(())
}
