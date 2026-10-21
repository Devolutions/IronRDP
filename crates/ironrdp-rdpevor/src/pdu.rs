//! Video Optimized Remoting Virtual Channel Extension PDUs defined by [MS-RDPEVOR].
//!
//! [MS-RDPEVOR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size, invalid_field_err,
};
use ironrdp_dvc::DvcEncode;

const HEADER_SIZE: usize = 4 /* cbSize */ + 4 /* PacketType */;
const PRESENTATION_REQUEST_BODY_SIZE: usize = 1 // PresentationId
    + 1 // Version
    + 1 // Command
    + 1 // FrameRate
    + 2 // AverageBitrateKbps
    + 2 // Reserved
    + 4 // SourceWidth
    + 4 // SourceHeight
    + 4 // ScaledWidth
    + 4 // ScaledHeight
    + 8 // hnsTimestampOffset
    + 8 // GeometryMappingId
    + 16 // VideoSubtypeId
    + 4; // cbExtra
const PRESENTATION_RESPONSE_BODY_SIZE: usize = 1 /* PresentationId */ + 1 /* ResponseFlags */ + 2 /* ResultFlags */;
const NOTIFICATION_BODY_SIZE: usize = 1 /* PresentationId */ + 1 /* NotificationType */ + 2 /* Reserved */ + 4 /* cbData */;
const FRAME_RATE_OVERRIDE_SIZE: usize = 4 /* Flags */ + 4 /* DesiredFrameRate */ + 4 /* Reserved1 */ + 4 /* Reserved2 */;
const VIDEO_DATA_BODY_SIZE: usize = 1 // PresentationId
    + 1 // Version
    + 1 // Flags
    + 1 // Reserved
    + 8 // hnsTimestamp
    + 8 // hnsDuration
    + 2 // CurrentPacketIndex
    + 2 // PacketsInSample
    + 4 // SampleNumber
    + 4; // cbSample

const PACKET_TYPE_PRESENTATION_REQUEST: u32 = 1;
const PACKET_TYPE_PRESENTATION_RESPONSE: u32 = 2;
const PACKET_TYPE_CLIENT_NOTIFICATION: u32 = 3;
const PACKET_TYPE_VIDEO_DATA: u32 = 4;

const NOTIFICATION_NETWORK_ERROR: u8 = 0x01;
const NOTIFICATION_FRAME_RATE_OVERRIDE: u8 = 0x02;

/// `MFVideoFormat_H264` ({34363248-0000-0010-8000-00AA00389B71}) in its wire layout.
pub const MF_VIDEO_FORMAT_H264: [u8; 16] = [
    0x48, 0x32, 0x36, 0x34, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

/// The **Command** field of [`PresentationRequest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentationCommand(pub u8);

impl PresentationCommand {
    pub const START: Self = Self(0x01);
    pub const STOP: Self = Self(0x02);
}

/// The [2.2.1.2] TSMM_PRESENTATION_REQUEST message, sent by the server on the control channel.
///
/// [2.2.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresentationRequest {
    pub presentation_id: u8,
    pub version: u8,
    pub command: PresentationCommand,
    /// Reserved; ignored by the client.
    pub frame_rate: u8,
    /// Reserved; ignored by the client.
    pub average_bitrate_kbps: u16,
    /// Displayed width, after scaling the stream back to its original resolution.
    pub source_width: u32,
    pub source_height: u32,
    /// Encoded width of the stream.
    pub scaled_width: u32,
    pub scaled_height: u32,
    /// Server time when the presentation started, in 100-ns units.
    pub timestamp_offset: u64,
    /// MS-RDPEGT mapping that positions the video.
    pub geometry_mapping_id: u64,
    pub video_subtype: [u8; 16],
    /// For H.264, the sequence header (SPS and PPS).
    pub extra_data: Vec<u8>,
}

impl Encode for PresentationRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        encode_header(dst, self.size(), PACKET_TYPE_PRESENTATION_REQUEST)?;
        dst.write_u8(self.presentation_id);
        dst.write_u8(self.version);
        dst.write_u8(self.command.0);
        dst.write_u8(self.frame_rate);
        dst.write_u16(self.average_bitrate_kbps);
        dst.write_u16(0);
        dst.write_u32(self.source_width);
        dst.write_u32(self.source_height);
        dst.write_u32(self.scaled_width);
        dst.write_u32(self.scaled_height);
        dst.write_u64(self.timestamp_offset);
        dst.write_u64(self.geometry_mapping_id);
        dst.write_array(self.video_subtype);
        dst.write_u32(
            u32::try_from(self.extra_data.len())
                .map_err(|_| invalid_field_err!("cbExtra", "extra data is too large"))?,
        );
        dst.write_slice(&self.extra_data);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TSMM_PRESENTATION_REQUEST"
    }

    fn size(&self) -> usize {
        HEADER_SIZE + PRESENTATION_REQUEST_BODY_SIZE + self.extra_data.len()
    }
}

impl<'de> Decode<'de> for PresentationRequest {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let mut src = decode_header(src, PACKET_TYPE_PRESENTATION_REQUEST)?;
        ensure_size!(in: src, size: PRESENTATION_REQUEST_BODY_SIZE);
        let presentation_id = src.read_u8();
        let version = src.read_u8();
        let command = PresentationCommand(src.read_u8());
        let frame_rate = src.read_u8();
        let average_bitrate_kbps = src.read_u16();
        let _reserved = src.read_u16();
        let source_width = src.read_u32();
        let source_height = src.read_u32();
        let scaled_width = src.read_u32();
        let scaled_height = src.read_u32();
        let timestamp_offset = src.read_u64();
        let geometry_mapping_id = src.read_u64();
        let video_subtype = src.read_array();
        let extra_len = usize::try_from(src.read_u32())
            .map_err(|_| invalid_field_err!("cbExtra", "extra data length overflows", in: src))?;
        // Only the header fields of a Stop request are valid (2.2.1.2), so its cbExtra is ignored.
        let extra_data = if command == PresentationCommand::STOP {
            Vec::new()
        } else {
            ensure_size!(in: src, size: extra_len);
            src.read_slice(extra_len).to_vec()
        };
        Ok(Self {
            presentation_id,
            version,
            command,
            frame_rate,
            average_bitrate_kbps,
            source_width,
            source_height,
            scaled_width,
            scaled_height,
            timestamp_offset,
            geometry_mapping_id,
            video_subtype,
            extra_data,
        })
    }
}

impl DvcEncode for PresentationRequest {}

/// The [2.2.1.3] TSMM_PRESENTATION_RESPONSE message, sent by the client on the control channel.
///
/// [2.2.1.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentationResponse {
    pub presentation_id: u8,
}

impl Encode for PresentationResponse {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        encode_header(dst, self.size(), PACKET_TYPE_PRESENTATION_RESPONSE)?;
        dst.write_u8(self.presentation_id);
        dst.write_u8(0);
        dst.write_u16(0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TSMM_PRESENTATION_RESPONSE"
    }

    fn size(&self) -> usize {
        HEADER_SIZE + PRESENTATION_RESPONSE_BODY_SIZE
    }
}

impl<'de> Decode<'de> for PresentationResponse {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let mut src = decode_header(src, PACKET_TYPE_PRESENTATION_RESPONSE)?;
        ensure_size!(in: src, size: PRESENTATION_RESPONSE_BODY_SIZE);
        let presentation_id = src.read_u8();
        let _response_flags = src.read_u8();
        let _result_flags = src.read_u16();
        Ok(Self { presentation_id })
    }
}

impl DvcEncode for PresentationResponse {}

/// Notification carried by a [`ClientNotification`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notification {
    /// Missing or out-of-order packets; asks the server for a key frame.
    NetworkError,
    /// The [2.2.1.5] TSMM_CLIENT_NOTIFICATION_FRAMERATE_OVERRIDE structure.
    ///
    /// [2.2.1.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/
    FrameRateOverride { flags: u32, desired_frame_rate: u32 },
}

impl Notification {
    pub const UNRESTRICTED_FRAME_RATE: u32 = 0x1;
    pub const OVERRIDE_FRAME_RATE: u32 = 0x2;
}

/// The [2.2.1.4] TSMM_CLIENT_NOTIFICATION message, sent by the client on the control channel.
///
/// [2.2.1.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientNotification {
    pub presentation_id: u8,
    pub notification: Notification,
}

impl ClientNotification {
    fn data_size(&self) -> usize {
        match self.notification {
            Notification::NetworkError => 0,
            Notification::FrameRateOverride { .. } => FRAME_RATE_OVERRIDE_SIZE,
        }
    }
}

impl Encode for ClientNotification {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        encode_header(dst, self.size(), PACKET_TYPE_CLIENT_NOTIFICATION)?;
        dst.write_u8(self.presentation_id);
        match self.notification {
            Notification::NetworkError => {
                dst.write_u8(NOTIFICATION_NETWORK_ERROR);
                dst.write_u16(0);
                dst.write_u32(0);
            }
            Notification::FrameRateOverride {
                flags,
                desired_frame_rate,
            } => {
                dst.write_u8(NOTIFICATION_FRAME_RATE_OVERRIDE);
                dst.write_u16(0);
                dst.write_u32(4 * 4);
                dst.write_u32(flags);
                dst.write_u32(desired_frame_rate);
                dst.write_u32(0);
                dst.write_u32(0);
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TSMM_CLIENT_NOTIFICATION"
    }

    fn size(&self) -> usize {
        HEADER_SIZE + NOTIFICATION_BODY_SIZE + self.data_size()
    }
}

impl<'de> Decode<'de> for ClientNotification {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let mut src = decode_header(src, PACKET_TYPE_CLIENT_NOTIFICATION)?;
        ensure_size!(in: src, size: NOTIFICATION_BODY_SIZE);
        let presentation_id = src.read_u8();
        let notification_type = src.read_u8();
        let _reserved = src.read_u16();
        let data_len = usize::try_from(src.read_u32())
            .map_err(|_| invalid_field_err!("cbData", "notification data length overflows", in: src))?;
        ensure_size!(in: src, size: data_len);
        let mut data = ReadCursor::new(src.read_slice(data_len));
        let notification = match notification_type {
            NOTIFICATION_NETWORK_ERROR => Notification::NetworkError,
            NOTIFICATION_FRAME_RATE_OVERRIDE => {
                ensure_size!(in: data, size: FRAME_RATE_OVERRIDE_SIZE);
                let flags = data.read_u32();
                let desired_frame_rate = data.read_u32();
                Notification::FrameRateOverride {
                    flags,
                    desired_frame_rate,
                }
            }
            _ => {
                return Err(invalid_field_err!("NotificationType", "unknown notification type", in: src));
            }
        };
        Ok(Self {
            presentation_id,
            notification,
        })
    }
}

impl DvcEncode for ClientNotification {}

/// The [2.2.1.6] TSMM_VIDEO_DATA message, sent by the server on the data channel.
///
/// The sample borrows the received payload.
///
/// [2.2.1.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoData<'a> {
    pub presentation_id: u8,
    pub version: u8,
    pub flags: u8,
    /// Time since the presentation started, in 100-ns units, valid with [`Self::HAS_TIMESTAMPS`].
    pub timestamp: u64,
    /// Time since the previous sample, in 100-ns units.
    pub duration: u64,
    /// One-based index of this packet within its sample.
    pub current_packet_index: u16,
    pub packets_in_sample: u16,
    /// One-based sample number.
    pub sample_number: u32,
    pub sample: &'a [u8],
}

impl VideoData<'_> {
    pub const HAS_TIMESTAMPS: u8 = 0x01;
    pub const KEYFRAME: u8 = 0x02;
    pub const NEW_FRAMERATE: u8 = 0x04;
}

impl Encode for VideoData<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        encode_header(dst, self.size(), PACKET_TYPE_VIDEO_DATA)?;
        dst.write_u8(self.presentation_id);
        dst.write_u8(self.version);
        dst.write_u8(self.flags);
        dst.write_u8(0);
        dst.write_u64(self.timestamp);
        dst.write_u64(self.duration);
        dst.write_u16(self.current_packet_index);
        dst.write_u16(self.packets_in_sample);
        dst.write_u32(self.sample_number);
        dst.write_u32(
            u32::try_from(self.sample.len()).map_err(|_| invalid_field_err!("cbSample", "sample is too large"))?,
        );
        dst.write_slice(self.sample);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TSMM_VIDEO_DATA"
    }

    fn size(&self) -> usize {
        HEADER_SIZE + VIDEO_DATA_BODY_SIZE + self.sample.len()
    }
}

impl<'de> Decode<'de> for VideoData<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let mut src = decode_header(src, PACKET_TYPE_VIDEO_DATA)?;
        ensure_size!(in: src, size: VIDEO_DATA_BODY_SIZE);
        let presentation_id = src.read_u8();
        let version = src.read_u8();
        let flags = src.read_u8();
        let _reserved = src.read_u8();
        let timestamp = src.read_u64();
        let duration = src.read_u64();
        let current_packet_index = src.read_u16();
        let packets_in_sample = src.read_u16();
        let sample_number = src.read_u32();
        let sample_len = usize::try_from(src.read_u32())
            .map_err(|_| invalid_field_err!("cbSample", "sample length overflows", in: src))?;
        ensure_size!(in: src, size: sample_len);
        if current_packet_index == 0 || packets_in_sample < current_packet_index {
            return Err(invalid_field_err!(
                "CurrentPacketIndex",
                "packet index is outside the sample",
                in: src
            ));
        }
        let sample = src.read_slice(sample_len);
        Ok(Self {
            presentation_id,
            version,
            flags,
            timestamp,
            duration,
            current_packet_index,
            packets_in_sample,
            sample_number,
            sample,
        })
    }
}

impl DvcEncode for VideoData<'_> {}

fn encode_header(dst: &mut WriteCursor<'_>, size: usize, packet_type: u32) -> EncodeResult<()> {
    dst.write_u32(u32::try_from(size).map_err(|_| invalid_field_err!("cbSize", "message is too large"))?);
    dst.write_u32(packet_type);
    Ok(())
}

/// Reads the [2.2.1.1] TSMM_VIDEO_PACKET_HEADER and returns the message body.
///
/// The body is bounded by **cbSize**. Bytes past it, such as the reserved trailing byte the
/// specification examples show, are ignored.
///
/// [2.2.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/
fn decode_header<'de>(src: &mut ReadCursor<'de>, expected_type: u32) -> DecodeResult<ReadCursor<'de>> {
    ensure_size!(in: src, size: HEADER_SIZE);
    let size =
        usize::try_from(src.read_u32()).map_err(|_| invalid_field_err!("cbSize", "length overflows", in: src))?;
    let packet_type = src.read_u32();
    if size < HEADER_SIZE || size - HEADER_SIZE > src.len() {
        return Err(invalid_field_err!(
            "cbSize",
            "message length does not match the received data",
            in: src
        ));
    }
    if packet_type != expected_type {
        return Err(invalid_field_err!("PacketType", "unexpected packet type", in: src));
    }
    Ok(ReadCursor::new(src.read_slice(size - HEADER_SIZE)))
}
