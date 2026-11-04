//! Screen capture redirection channel.
//!
//! The client uploads a screenshot taken on the physical device, and the Cloud PC saves it and reports where.
//! See [`PROTOCOL.md`] for the layouts and the client behavior.
//!
//! [`PROTOCOL.md`]: https://github.com/Devolutions/IronRDP/blob/master/crates/ironrdp-w365/PROTOCOL.md#screen-capture-redirection

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, decode, ensure_size, impl_as_any,
    invalid_field_err,
};
use ironrdp_dvc::{DvcClientProcessor, DvcEncode, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, encode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, warn};

use crate::PROTOCOL_VERSION_1;

/// Dynamic virtual channel name.
pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::ScreenCaptureRedirection";

const HEADER_SIZE: usize = 2 /* OperationType */ + 2 /* Flags */ + 4 /* PayloadLength */;
const READY_SIZE: usize = 4 /* ProtocolVersion */ + 4 /* ProtocolFlags */;
const CAPTURE_ID_SIZE: usize = 2 /* CaptureId */;
const SAVE_RESULT_FIXED_SIZE: usize =
    2 /* CaptureId */ + 4 /* HResult */ + 4 /* FileNameLength */ + 4 /* FilePathLength */;

const PDU_TYPE_SERVER_READY: u16 = 0x0001;
const PDU_TYPE_CLIENT_READY: u16 = 0x0002;
const PDU_TYPE_SCREEN_CAPTURE: u16 = 0x0003;
const PDU_TYPE_SAVE_RESULT: u16 = 0x0004;

/// Largest capture ID; the next one wraps to zero.
const MAX_CAPTURE_ID: u16 = 0x7FFF;

/// Encoding of an uploaded image, carried in the screen capture PDU's header flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageType(pub u16);

impl ImageType {
    pub const PNG: Self = Self(0x0001);
    pub const JPEG: Self = Self(0x0002);
}

/// Where the Cloud PC saved an uploaded screenshot, or why it could not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveResult {
    pub capture_id: u16,
    /// HRESULT of the save; negative on failure.
    pub hresult: u32,
    pub file_name: String,
    pub file_path: String,
}

/// Screen capture redirection PDU.
///
/// Every PDU starts with a 16-bit operation type, 16-bit flags, and a 32-bit length of the payload that follows the header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenCapturePdu {
    /// Server to client: the server's protocol version and reserved flags.
    ServerReady { version: u32, flags: u32 },
    /// Client to server: the client's protocol version and reserved flags.
    ClientReady { version: u32, flags: u32 },
    /// Client to server: an encoded screenshot.
    ScreenCapture {
        image_type: ImageType,
        capture_id: u16,
        image: Vec<u8>,
    },
    /// Server to client: the outcome of saving a screenshot.
    SaveResult(SaveResult),
}

impl ScreenCapturePdu {
    const NAME: &'static str = "SCREEN_CAPTURE_PDU";

    /// Creates the client-ready PDU Windows App sends.
    pub fn client_ready() -> Self {
        Self::ClientReady {
            version: PROTOCOL_VERSION_1,
            flags: 0,
        }
    }

    fn pdu_type(&self) -> u16 {
        match self {
            Self::ServerReady { .. } => PDU_TYPE_SERVER_READY,
            Self::ClientReady { .. } => PDU_TYPE_CLIENT_READY,
            Self::ScreenCapture { .. } => PDU_TYPE_SCREEN_CAPTURE,
            Self::SaveResult(_) => PDU_TYPE_SAVE_RESULT,
        }
    }

    fn header_flags(&self) -> u16 {
        match self {
            Self::ScreenCapture { image_type, .. } => image_type.0,
            Self::ServerReady { .. } | Self::ClientReady { .. } | Self::SaveResult(_) => 0,
        }
    }

    fn payload_size(&self) -> usize {
        match self {
            Self::ServerReady { .. } | Self::ClientReady { .. } => READY_SIZE,
            Self::ScreenCapture { image, .. } => CAPTURE_ID_SIZE + image.len(),
            Self::SaveResult(result) => {
                SAVE_RESULT_FIXED_SIZE + utf16_size(&result.file_name) + utf16_size(&result.file_path)
            }
        }
    }
}

impl Encode for ScreenCapturePdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u16(self.pdu_type());
        dst.write_u16(self.header_flags());
        dst.write_u32(cast_length!("payloadLength", self.payload_size())?);
        match self {
            Self::ServerReady { version, flags } | Self::ClientReady { version, flags } => {
                dst.write_u32(*version);
                dst.write_u32(*flags);
            }
            Self::ScreenCapture { capture_id, image, .. } => {
                dst.write_u16(*capture_id);
                dst.write_slice(image);
            }
            Self::SaveResult(result) => {
                dst.write_u16(result.capture_id);
                dst.write_u32(result.hresult);
                dst.write_u32(cast_length!("fileNameLength", utf16_size(&result.file_name))?);
                dst.write_u32(cast_length!("filePathLength", utf16_size(&result.file_path))?);
                for unit in result.file_name.encode_utf16().chain(result.file_path.encode_utf16()) {
                    dst.write_u16(unit);
                }
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        HEADER_SIZE + self.payload_size()
    }
}

impl<'de> Decode<'de> for ScreenCapturePdu {
    /// Decodes one PDU, which must fill the payload exactly, as Windows App requires.
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: HEADER_SIZE);
        let pdu_type = src.read_u16();
        let flags = src.read_u16();
        let payload_length = usize::try_from(src.read_u32()).unwrap_or(usize::MAX);
        if payload_length != src.len() {
            return Err(invalid_field_err!(
                "payloadLength",
                "screen capture PDU length does not match its payload",
                in: src
            ));
        }
        let mut payload = ReadCursor::new(src.read_slice(payload_length));

        match pdu_type {
            PDU_TYPE_SERVER_READY | PDU_TYPE_CLIENT_READY => {
                if !(flags == 0 && payload_length == READY_SIZE) {
                    return Err(invalid_field_err!("flags", "malformed screen capture ready PDU", in: src));
                }
                let version = payload.read_u32();
                let flags = payload.read_u32();
                Ok(if pdu_type == PDU_TYPE_SERVER_READY {
                    Self::ServerReady { version, flags }
                } else {
                    Self::ClientReady { version, flags }
                })
            }
            PDU_TYPE_SCREEN_CAPTURE => {
                ensure_size!(in: payload, size: CAPTURE_ID_SIZE);
                let capture_id = payload.read_u16();
                Ok(Self::ScreenCapture {
                    image_type: ImageType(flags),
                    capture_id,
                    image: payload.read_remaining().to_vec(),
                })
            }
            PDU_TYPE_SAVE_RESULT => {
                if flags != 0 {
                    return Err(invalid_field_err!("flags", "save result PDU flags are not zero", in: src));
                }
                ensure_size!(in: payload, size: SAVE_RESULT_FIXED_SIZE);
                let capture_id = payload.read_u16();
                if MAX_CAPTURE_ID < capture_id {
                    return Err(invalid_field_err!("captureId", "capture ID exceeds 0x7FFF", in: payload));
                }
                let hresult = payload.read_u32();
                let file_name_size = usize::try_from(payload.read_u32()).unwrap_or(usize::MAX);
                let file_path_size = usize::try_from(payload.read_u32()).unwrap_or(usize::MAX);
                if file_name_size.checked_add(file_path_size) != Some(payload.len()) {
                    return Err(invalid_field_err!(
                        "fileNameLength",
                        "save result string lengths do not match its payload",
                        in: payload
                    ));
                }
                let file_name = decode_utf16(payload.read_slice(file_name_size))
                    .ok_or_else(|| invalid_field_err!("fileName", "file name is not valid UTF-16", in: payload))?;
                let file_path = decode_utf16(payload.read_slice(file_path_size))
                    .ok_or_else(|| invalid_field_err!("filePath", "file path is not valid UTF-16", in: payload))?;
                Ok(Self::SaveResult(SaveResult {
                    capture_id,
                    hresult,
                    file_name,
                    file_path,
                }))
            }
            _ => Err(invalid_field_err!("operationType", "unknown screen capture PDU type", in: src)),
        }
    }
}

impl DvcEncode for ScreenCapturePdu {}

fn utf16_size(text: &str) -> usize {
    text.encode_utf16().count() * 2
}

/// Decodes UTF-16LE text, dropping trailing NUL characters a sender may include as a terminator.
fn decode_utf16(bytes: &[u8]) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units = bytes.chunks_exact(2).map(|unit| u16::from_le_bytes([unit[0], unit[1]]));
    let mut text = char::decode_utf16(units).collect::<Result<String, _>>().ok()?;
    let trimmed_len = text.trim_end_matches('\0').len();
    text.truncate(trimmed_len);
    Some(text)
}

/// Host behavior for the screen capture channel.
///
/// The processor calls these methods from the session loop, so they must return promptly.
pub trait ScreenCaptureHandler: Send {
    /// The server accepted the protocol version, so the client can upload screenshots.
    fn uploads_ready(&mut self) {}

    /// The Cloud PC reported the outcome of saving an uploaded screenshot.
    fn save_completed(&mut self, result: SaveResult);
}

/// Client processor for `Microsoft::Windows::RDS::ScreenCaptureRedirection`.
pub struct ScreenCaptureClient {
    handler: Box<dyn ScreenCaptureHandler>,
    channel_id: Option<u32>,
    negotiated: bool,
    /// INVARIANT: `next_capture_id <= MAX_CAPTURE_ID`.
    next_capture_id: u16,
}

impl core::fmt::Debug for ScreenCaptureClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ScreenCaptureClient")
            .field("channel_id", &self.channel_id)
            .field("negotiated", &self.negotiated)
            .field("next_capture_id", &self.next_capture_id)
            .finish_non_exhaustive()
    }
}

impl ScreenCaptureClient {
    pub fn new(handler: Box<dyn ScreenCaptureHandler>) -> Self {
        Self {
            handler,
            channel_id: None,
            negotiated: false,
            next_capture_id: 0,
        }
    }

    /// Returns whether the server accepted the protocol version, so that uploads are possible.
    pub fn ready(&self) -> bool {
        self.negotiated && self.channel_id.is_some()
    }

    /// Encodes a screenshot upload and returns the capture ID the save result will carry.
    ///
    /// Like Windows App, this accepts only non-empty PNG or JPEG images once the version negotiation has completed.
    pub fn upload(&mut self, image_type: ImageType, image: Vec<u8>) -> PduResult<(u16, Vec<SvcMessage>)> {
        const CONTEXT: &str = "ScreenCaptureClient::upload";

        let channel_id = match self.channel_id {
            Some(channel_id) if self.negotiated => channel_id,
            _ => return Err(pdu_other_err!(CONTEXT, "screen capture channel is not ready")),
        };
        if !(image_type == ImageType::PNG || image_type == ImageType::JPEG) {
            return Err(pdu_other_err!(CONTEXT, "screen capture image type is not PNG or JPEG"));
        }
        if image.is_empty() {
            return Err(pdu_other_err!(CONTEXT, "screen capture image is empty"));
        }

        let capture_id = self.next_capture_id;
        let pdu = ScreenCapturePdu::ScreenCapture {
            image_type,
            capture_id,
            image,
        };
        let messages = encode_dvc_messages(channel_id, vec![Box::new(pdu)], ChannelFlags::empty())
            .map_err(|error| encode_err!(error))?;
        // Upholds the next_capture_id invariant by wrapping after MAX_CAPTURE_ID.
        self.next_capture_id = if capture_id < MAX_CAPTURE_ID { capture_id + 1 } else { 0 };
        Ok((capture_id, messages))
    }
}

impl_as_any!(ScreenCaptureClient);

impl DvcProcessor for ScreenCaptureClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.channel_id = Some(channel_id);
        self.negotiated = false;
        debug!(channel_id, "Screen capture channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu: ScreenCapturePdu = match decode(payload) {
            Ok(pdu) => pdu,
            Err(error) => {
                warn!(%error, "Ignoring malformed screen capture PDU");
                return Ok(Vec::new());
            }
        };

        match pdu {
            ScreenCapturePdu::ServerReady { version, .. } if version == PROTOCOL_VERSION_1 => {
                debug!(version, "Screen capture version negotiated");
                self.negotiated = true;
                self.handler.uploads_ready();
                Ok(vec![Box::new(ScreenCapturePdu::client_ready())])
            }
            ScreenCapturePdu::ServerReady { version, .. } => {
                warn!(version, "Server announced an unsupported screen capture version");
                Ok(Vec::new())
            }
            ScreenCapturePdu::SaveResult(result) => {
                debug!(
                    capture_id = result.capture_id,
                    hresult = result.hresult,
                    "Cloud PC saved a screen capture"
                );
                self.handler.save_completed(result);
                Ok(Vec::new())
            }
            ScreenCapturePdu::ClientReady { .. } | ScreenCapturePdu::ScreenCapture { .. } => {
                warn!("Ignoring a client-originated screen capture PDU received from the server");
                Ok(Vec::new())
            }
        }
    }

    fn close(&mut self, _channel_id: u32) {
        self.channel_id = None;
        self.negotiated = false;
        debug!("Screen capture channel closed");
    }
}

impl DvcClientProcessor for ScreenCaptureClient {}
