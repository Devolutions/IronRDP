//! Client-side EGFX implementation
//!
//! This module provides client-side support for the Graphics Pipeline Extension
//! ([MS-RDPEGFX]), including H.264 AVC420 decode and surface management.
//!
//! # Protocol Compliance
//!
//! This implementation follows MS-RDPEGFX client requirements:
//!
//! - **Capability Negotiation**: Advertises V8 through V10.7 ([2.2.3])
//! - **Surface Management**: Tracks server-created surfaces ([3.3.1.6])
//! - **Frame Acknowledgment**: Sends `FrameAcknowledge` after `EndFrame` ([3.3.5.12])
//! - **Codec Dispatch**: Routes `WireToSurface1` by `codec_id` ([3.3.5.2])
//!
//! # Architecture
//!
//! ```text
//! Server                                  Client
//!    |                                       |
//!    |--- CapabilitiesConfirm -------------->|
//!    |--- ResetGraphics -------------------->|
//!    |--- CreateSurface -------------------->|
//!    |--- MapSurfaceToOutput --------------->|
//!    |                                       |
//!    |  (For each frame:)                    |
//!    |--- StartFrame ----------------------->|
//!    |--- WireToSurface1 (H.264) ----------->|  -> H264Decoder::decode()
//!    |--- EndFrame ------------------------->|  -> FrameAcknowledge
//!    |                                       |
//!    |<---------- FrameAcknowledge ----------|
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use ironrdp_egfx::client::{GraphicsPipelineClient, GraphicsPipelineHandler, BitmapUpdate};
//! use ironrdp_egfx::decode::H264Decoder;
//!
//! struct MyHandler;
//!
//! impl GraphicsPipelineHandler for MyHandler {
//!     fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
//!         // Render decoded bitmap to screen
//!     }
//! }
//!
//! let client = GraphicsPipelineClient::new(Box::new(MyHandler), None);
//! ```
//!
//! [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
//! [2.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/b5e09f90-6dde-47ca-8ec1-7dcdd5dc70b0
//! [3.3.1.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/83cb08ff-c97f-4d08-b834-7aa69cdea6c5
//! [3.3.5.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/90aba3e3-d4a8-4af1-b1bb-a94e2313bbf0
//! [3.3.5.12]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/e3c80bff-3e4e-4e65-b7c2-c2cd6b1fb4f5

use std::collections::BTreeMap;

use ironrdp_core::{impl_as_any, Decode as _, ReadCursor};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_graphics::zgfx;
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp_pdu::{decode_cursor, decode_err, pdu_other_err, PduResult};
use tracing::{debug, trace, warn};

use crate::decode::H264Decoder;
use crate::pdu::{
    Avc420BitmapStream, CapabilitiesAdvertisePdu, CapabilitiesV107Flags, CapabilitiesV81Flags, CapabilitiesV8Flags,
    CapabilitySet, Codec1Type, FrameAcknowledgePdu, GfxPdu, PixelFormat, QueueDepth,
};
use crate::CHANNEL_NAME;

/// Max capacity to keep for decompressed buffer when cleared.
const MAX_DECOMPRESSED_BUFFER_CAPACITY: usize = 16384; // 16 KiB

// ============================================================================
// Surface Management
// ============================================================================

/// Client-side surface state
///
/// Per [MS-RDPEGFX 3.3.1.6], the client maintains an "Offscreen Surfaces
/// ADM element" tracking surfaces created by the server.
///
/// [MS-RDPEGFX 3.3.1.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/83cb08ff-c97f-4d08-b834-7aa69cdea6c5
#[derive(Debug, Clone)]
pub struct Surface {
    /// Surface identifier (assigned by server)
    pub id: u16,
    /// Surface width in pixels
    pub width: u16,
    /// Surface height in pixels
    pub height: u16,
    /// Pixel format
    pub pixel_format: PixelFormat,
    /// Whether this surface is mapped to an output
    pub is_mapped: bool,
    /// Output X origin (if mapped)
    pub output_origin_x: u32,
    /// Output Y origin (if mapped)
    pub output_origin_y: u32,
}

// ============================================================================
// Codec Capabilities
// ============================================================================

/// Codec capabilities determined from negotiated capability set
#[derive(Debug, Clone, Default)]
pub struct CodecCapabilities {
    /// AVC420 (H.264 4:2:0) is available
    pub avc420: bool,
    /// AVC444 (H.264 4:4:4) is available
    pub avc444: bool,
    /// Small cache mode
    pub small_cache: bool,
    /// Thin client mode
    pub thin_client: bool,
}

impl CodecCapabilities {
    fn from_capability_set(cap: &CapabilitySet) -> Self {
        // Mirrors the server-side extraction logic
        match cap {
            CapabilitySet::V8 { flags } => Self {
                avc420: false,
                avc444: false,
                small_cache: flags.contains(CapabilitiesV8Flags::SMALL_CACHE),
                thin_client: flags.contains(CapabilitiesV8Flags::THIN_CLIENT),
            },
            CapabilitySet::V8_1 { flags } => Self {
                avc420: flags.contains(CapabilitiesV81Flags::AVC420_ENABLED),
                avc444: false,
                small_cache: flags.contains(CapabilitiesV81Flags::SMALL_CACHE),
                thin_client: flags.contains(CapabilitiesV81Flags::THIN_CLIENT),
            },
            CapabilitySet::V10 { flags } | CapabilitySet::V10_2 { flags } => Self {
                avc420: !flags.contains(crate::pdu::CapabilitiesV10Flags::AVC_DISABLED),
                avc444: !flags.contains(crate::pdu::CapabilitiesV10Flags::AVC_DISABLED),
                small_cache: flags.contains(crate::pdu::CapabilitiesV10Flags::SMALL_CACHE),
                thin_client: false,
            },
            CapabilitySet::V10_1 => Self {
                avc420: true,
                avc444: true,
                small_cache: false,
                thin_client: false,
            },
            CapabilitySet::V10_3 { flags } => Self {
                avc420: !flags.contains(crate::pdu::CapabilitiesV103Flags::AVC_DISABLED),
                avc444: !flags.contains(crate::pdu::CapabilitiesV103Flags::AVC_DISABLED),
                small_cache: false,
                thin_client: flags.contains(crate::pdu::CapabilitiesV103Flags::AVC_THIN_CLIENT),
            },
            CapabilitySet::V10_4 { flags }
            | CapabilitySet::V10_5 { flags }
            | CapabilitySet::V10_6 { flags }
            | CapabilitySet::V10_6Err { flags } => Self {
                avc420: !flags.contains(crate::pdu::CapabilitiesV104Flags::AVC_DISABLED),
                avc444: !flags.contains(crate::pdu::CapabilitiesV104Flags::AVC_DISABLED),
                small_cache: flags.contains(crate::pdu::CapabilitiesV104Flags::SMALL_CACHE),
                thin_client: flags.contains(crate::pdu::CapabilitiesV104Flags::AVC_THIN_CLIENT),
            },
            CapabilitySet::V10_7 { flags } => Self {
                avc420: !flags.contains(CapabilitiesV107Flags::AVC_DISABLED),
                avc444: !flags.contains(CapabilitiesV107Flags::AVC_DISABLED),
                small_cache: flags.contains(CapabilitiesV107Flags::SMALL_CACHE),
                thin_client: flags.contains(CapabilitiesV107Flags::AVC_THIN_CLIENT),
            },
            CapabilitySet::Unknown(_) => Self::default(),
        }
    }
}

// ============================================================================
// Bitmap Update
// ============================================================================

/// Decoded bitmap data for a surface region
///
/// Delivered to [`GraphicsPipelineHandler::on_bitmap_updated`] when
/// a `WireToSurface1` PDU is processed with decoded pixel data.
#[derive(Debug)]
pub struct BitmapUpdate {
    /// Surface this update applies to
    pub surface_id: u16,
    /// Destination rectangle within the surface
    pub destination_rectangle: InclusiveRectangle,
    /// Codec that produced this update
    pub codec_id: Codec1Type,
    /// RGBA pixel data (4 bytes per pixel), row-major
    ///
    /// Dimensions match `width * height * 4` bytes.
    /// May be empty if decode was skipped (no decoder configured).
    pub data: Vec<u8>,
    /// Width of the decoded data in pixels
    pub width: u16,
    /// Height of the decoded data in pixels
    pub height: u16,
}

// ============================================================================
// Handler Trait
// ============================================================================

/// Handler trait for client-side EGFX events
///
/// Implement this trait to receive decoded bitmap data and surface
/// lifecycle notifications from the EGFX pipeline.
pub trait GraphicsPipelineHandler: Send {
    /// Returns the capability sets to advertise to the server
    ///
    /// The default advertises V10.7 (AVC420+AVC444), V8.1 (AVC420 only),
    /// and V8 (no AVC) as fallback.
    fn capabilities(&self) -> Vec<CapabilitySet> {
        vec![
            CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::SMALL_CACHE,
            },
            CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::AVC420_ENABLED | CapabilitiesV81Flags::SMALL_CACHE,
            },
            CapabilitySet::V8 {
                flags: CapabilitiesV8Flags::SMALL_CACHE,
            },
        ]
    }

    /// Called when the server confirms negotiated capabilities
    fn on_capabilities_confirmed(&mut self, _caps: &CapabilitySet) {}

    /// Called when the server resets the graphics output buffer
    fn on_reset_graphics(&mut self, _width: u32, _height: u32) {}

    /// Called when a surface is created by the server
    fn on_surface_created(&mut self, _surface: &Surface) {}

    /// Called when a surface is deleted by the server
    fn on_surface_deleted(&mut self, _surface_id: u16) {}

    /// Called when a surface is mapped to an output position
    fn on_surface_mapped(&mut self, _surface_id: u16, _origin_x: u32, _origin_y: u32) {}

    /// Called when decoded bitmap data is available for a surface
    ///
    /// This is the primary output path. The `update` contains the
    /// surface ID, destination rectangle, and RGBA pixel data.
    fn on_bitmap_updated(&mut self, _update: &BitmapUpdate) {}

    /// Called when a logical frame is complete
    ///
    /// All bitmap updates between the corresponding `StartFrame`
    /// and this notification belong to the same logical frame.
    fn on_frame_complete(&mut self, _frame_id: u32) {}

    /// Called when the EGFX channel is closed
    fn on_close(&mut self) {}

    /// Called for PDUs that have no specific handler
    ///
    /// Includes `SolidFill`, `SurfaceToSurface`, `SurfaceToCache`,
    /// `CacheToSurface`, `EvictCacheEntry`, and other PDUs not
    /// directly handled by the client core.
    fn on_unhandled_pdu(&mut self, _pdu: &GfxPdu) {}
}

// ============================================================================
// Client State Machine
// ============================================================================

/// Client state machine states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientState {
    /// Waiting for server `CapabilitiesConfirm`
    WaitingForConfirm,
    /// Channel is active, processing frames
    Active,
    /// Channel has been closed
    Closed,
}

// ============================================================================
// Graphics Pipeline Client
// ============================================================================

/// Client for the Graphics Pipeline Virtual Channel (EGFX)
///
/// This client handles capability negotiation, surface tracking,
/// H.264 AVC420 decode, and frame acknowledgment per [MS-RDPEGFX].
///
/// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
pub struct GraphicsPipelineClient {
    handler: Box<dyn GraphicsPipelineHandler>,
    h264_decoder: Option<Box<dyn H264Decoder>>,

    decompressor: zgfx::Decompressor,
    decompressed_buffer: Vec<u8>,

    state: ClientState,
    negotiated_caps: Option<CapabilitySet>,
    codec_caps: CodecCapabilities,

    surfaces: BTreeMap<u16, Surface>,
    current_frame_id: Option<u32>,
    frames_queued: u32,
    total_frames_decoded: u32,
}

impl GraphicsPipelineClient {
    /// Create a new `GraphicsPipelineClient`
    ///
    /// If `h264_decoder` is `None`, AVC420 frames are logged and skipped.
    pub fn new(handler: Box<dyn GraphicsPipelineHandler>, h264_decoder: Option<Box<dyn H264Decoder>>) -> Self {
        Self {
            handler,
            h264_decoder,
            decompressor: zgfx::Decompressor::new(),
            decompressed_buffer: Vec::new(),
            state: ClientState::WaitingForConfirm,
            negotiated_caps: None,
            codec_caps: CodecCapabilities::default(),
            surfaces: BTreeMap::new(),
            current_frame_id: None,
            frames_queued: 0,
            total_frames_decoded: 0,
        }
    }

    // ========================================================================
    // State Queries
    // ========================================================================

    /// Check if the client has completed capability negotiation
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.state == ClientState::Active
    }

    /// Get the negotiated capability set
    #[must_use]
    pub fn negotiated_capabilities(&self) -> Option<&CapabilitySet> {
        self.negotiated_caps.as_ref()
    }

    /// Get codec capabilities determined from negotiation
    #[must_use]
    pub fn codec_capabilities(&self) -> &CodecCapabilities {
        &self.codec_caps
    }

    /// Get a surface by ID
    #[must_use]
    pub fn get_surface(&self, surface_id: u16) -> Option<&Surface> {
        self.surfaces.get(&surface_id)
    }

    /// Get the total number of frames decoded
    #[must_use]
    pub fn total_frames_decoded(&self) -> u32 {
        self.total_frames_decoded
    }

    // ========================================================================
    // PDU Handlers
    // ========================================================================

    fn handle_pdu(&mut self, pdu: GfxPdu) -> PduResult<Vec<DvcMessage>> {
        match pdu {
            GfxPdu::CapabilitiesConfirm(confirm) => {
                self.handle_capabilities_confirm(confirm.0);
                Ok(vec![])
            }
            GfxPdu::ResetGraphics(reset) => {
                self.handle_reset_graphics(reset.width, reset.height);
                Ok(vec![])
            }
            GfxPdu::CreateSurface(create) => {
                self.handle_create_surface(create.surface_id, create.width, create.height, create.pixel_format);
                Ok(vec![])
            }
            GfxPdu::DeleteSurface(delete) => {
                self.handle_delete_surface(delete.surface_id);
                Ok(vec![])
            }
            GfxPdu::MapSurfaceToOutput(map) => {
                self.handle_map_surface(map.surface_id, map.output_origin_x, map.output_origin_y);
                Ok(vec![])
            }
            GfxPdu::StartFrame(start) => {
                self.current_frame_id = Some(start.frame_id);
                self.frames_queued = self.frames_queued.saturating_add(1);
                trace!(frame_id = start.frame_id, "StartFrame");
                Ok(vec![])
            }
            GfxPdu::WireToSurface1(wire) => {
                self.handle_wire_to_surface1(wire)?;
                Ok(vec![])
            }
            GfxPdu::EndFrame(end) => self.handle_end_frame(end.frame_id),
            // Forward unhandled PDUs to application
            other => {
                self.handler.on_unhandled_pdu(&other);
                Ok(vec![])
            }
        }
    }

    fn handle_capabilities_confirm(&mut self, cap: CapabilitySet) {
        self.codec_caps = CodecCapabilities::from_capability_set(&cap);
        self.negotiated_caps = Some(cap.clone());
        self.state = ClientState::Active;

        debug!(
            avc420 = self.codec_caps.avc420,
            avc444 = self.codec_caps.avc444,
            "EGFX capabilities confirmed"
        );

        self.handler.on_capabilities_confirmed(&cap);
    }

    fn handle_reset_graphics(&mut self, width: u32, height: u32) {
        // Per spec, ResetGraphics implicitly destroys all surfaces
        self.surfaces.clear();

        // Reset decoder state for new stream
        if let Some(ref mut decoder) = self.h264_decoder {
            decoder.reset();
        }

        debug!(width, height, "Graphics reset");
        self.handler.on_reset_graphics(width, height);
    }

    fn handle_create_surface(&mut self, surface_id: u16, width: u16, height: u16, pixel_format: PixelFormat) {
        let surface = Surface {
            id: surface_id,
            width,
            height,
            pixel_format,
            is_mapped: false,
            output_origin_x: 0,
            output_origin_y: 0,
        };

        debug!(surface_id, width, height, ?pixel_format, "Surface created");
        self.handler.on_surface_created(&surface);
        self.surfaces.insert(surface_id, surface);
    }

    fn handle_delete_surface(&mut self, surface_id: u16) {
        if self.surfaces.remove(&surface_id).is_some() {
            debug!(surface_id, "Surface deleted");
            self.handler.on_surface_deleted(surface_id);
        } else {
            warn!(surface_id, "DeleteSurface for unknown surface");
        }
    }

    fn handle_map_surface(&mut self, surface_id: u16, origin_x: u32, origin_y: u32) {
        if let Some(surface) = self.surfaces.get_mut(&surface_id) {
            surface.is_mapped = true;
            surface.output_origin_x = origin_x;
            surface.output_origin_y = origin_y;
            debug!(surface_id, origin_x, origin_y, "Surface mapped to output");
            self.handler.on_surface_mapped(surface_id, origin_x, origin_y);
        } else {
            warn!(surface_id, "MapSurfaceToOutput for unknown surface");
        }
    }

    fn handle_wire_to_surface1(&mut self, pdu: crate::pdu::WireToSurface1Pdu) -> PduResult<()> {
        let surface = self
            .surfaces
            .get(&pdu.surface_id)
            .ok_or_else(|| pdu_other_err!("unknown surface in WireToSurface1"))?;

        // Validate destination rectangle against surface bounds
        if pdu.destination_rectangle.right >= surface.width || pdu.destination_rectangle.bottom >= surface.height {
            warn!(
                surface_id = pdu.surface_id,
                rect_right = pdu.destination_rectangle.right,
                rect_bottom = pdu.destination_rectangle.bottom,
                surface_width = surface.width,
                surface_height = surface.height,
                "WireToSurface1 destination rectangle exceeds surface bounds"
            );
        }

        match pdu.codec_id {
            Codec1Type::Avc420 => {
                self.decode_avc420(pdu.surface_id, &pdu.destination_rectangle, &pdu.bitmap_data)?;
            }
            Codec1Type::Avc444 | Codec1Type::Avc444v2 => {
                debug!("AVC444 codec not yet implemented, forwarding to handler");
                self.handler.on_unhandled_pdu(&GfxPdu::WireToSurface1(pdu));
            }
            Codec1Type::Uncompressed => {
                self.handle_uncompressed(pdu);
            }
            _ => {
                trace!(codec_id = ?pdu.codec_id, "Forwarding unsupported codec to handler");
                self.handler.on_unhandled_pdu(&GfxPdu::WireToSurface1(pdu));
            }
        }

        Ok(())
    }

    fn decode_avc420(&mut self, surface_id: u16, dest_rect: &InclusiveRectangle, bitmap_data: &[u8]) -> PduResult<()> {
        let mut cursor = ReadCursor::new(bitmap_data);
        let stream = Avc420BitmapStream::decode(&mut cursor).map_err(|e| decode_err!(e))?;

        let Some(ref mut decoder) = self.h264_decoder else {
            debug!("No H.264 decoder configured, skipping AVC420 frame");
            return Ok(());
        };

        let frame = decoder
            .decode(stream.data)
            .map_err(|e| pdu_other_err!("H.264 decode", source: e))?;

        let dest_width = dest_rect.width();
        let dest_height = dest_rect.height();

        let cropped_data = crop_decoded_frame(&frame.data, frame.width, frame.height, dest_width, dest_height);

        let update = BitmapUpdate {
            surface_id,
            destination_rectangle: dest_rect.clone(),
            codec_id: Codec1Type::Avc420,
            data: cropped_data,
            width: dest_width,
            height: dest_height,
        };

        self.handler.on_bitmap_updated(&update);
        Ok(())
    }

    fn handle_uncompressed(&mut self, pdu: crate::pdu::WireToSurface1Pdu) {
        let dest_width = pdu.destination_rectangle.width();
        let dest_height = pdu.destination_rectangle.height();

        let update = BitmapUpdate {
            surface_id: pdu.surface_id,
            destination_rectangle: pdu.destination_rectangle,
            codec_id: Codec1Type::Uncompressed,
            data: pdu.bitmap_data,
            width: dest_width,
            height: dest_height,
        };

        self.handler.on_bitmap_updated(&update);
    }

    #[expect(clippy::as_conversions, reason = "Box<GfxPdu> to Box<dyn DvcEncode> coercion")]
    fn handle_end_frame(&mut self, frame_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.total_frames_decoded = self.total_frames_decoded.wrapping_add(1);
        self.current_frame_id = None;
        self.frames_queued = self.frames_queued.saturating_sub(1);

        self.handler.on_frame_complete(frame_id);

        // Per [3.3.5.12]: client MUST send FrameAcknowledge after EndFrame
        let ack = GfxPdu::FrameAcknowledge(FrameAcknowledgePdu {
            queue_depth: QueueDepth::from_u32(self.frames_queued),
            frame_id,
            total_frames_decoded: self.total_frames_decoded,
        });

        trace!(frame_id, "Sending FrameAcknowledge");
        Ok(vec![Box::new(ack) as DvcMessage])
    }
}

impl_as_any!(GraphicsPipelineClient);

impl DvcProcessor for GraphicsPipelineClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        let pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu(self.handler.capabilities()));

        #[expect(clippy::as_conversions, reason = "Box<GfxPdu> to Box<dyn DvcEncode> coercion")]
        Ok(vec![Box::new(pdu) as DvcMessage])
    }

    fn close(&mut self, _channel_id: u32) {
        self.state = ClientState::Closed;
        self.handler.on_close();
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        // ZGFX decompress
        self.decompressed_buffer.clear();
        self.decompressed_buffer.shrink_to(MAX_DECOMPRESSED_BUFFER_CAPACITY);
        self.decompressor
            .decompress(payload, &mut self.decompressed_buffer)
            .map_err(|e| decode_err!(e))?;

        // Decode all PDUs first (cursor borrows decompressed_buffer)
        let mut pdus = Vec::new();
        {
            let mut cursor = ReadCursor::new(self.decompressed_buffer.as_slice());
            while !cursor.is_empty() {
                let pdu: GfxPdu = decode_cursor(&mut cursor).map_err(|e| decode_err!(e))?;
                pdus.push(pdu);
            }
        }

        // Process decoded PDUs
        let mut responses: Vec<DvcMessage> = Vec::new();
        for pdu in pdus {
            let pdu_responses = self.handle_pdu(pdu)?;
            responses.extend(pdu_responses);
        }

        Ok(responses)
    }
}

impl DvcClientProcessor for GraphicsPipelineClient {}

// ============================================================================
// Frame Cropping
// ============================================================================

/// Crop a decoded RGBA frame to target dimensions
///
/// H.264 frames are macroblock-aligned (16x16), so decoded frames
/// may be larger than the destination rectangle. This function
/// extracts the top-left region matching the target size.
fn crop_decoded_frame(
    data: &[u8],
    decoded_width: u32,
    decoded_height: u32,
    target_width: u16,
    target_height: u16,
) -> Vec<u8> {
    let tw = u32::from(target_width);
    let th = u32::from(target_height);

    if decoded_width == 0 || decoded_height == 0 || tw == 0 || th == 0 {
        return Vec::new();
    }

    // If dimensions match, return as-is
    if decoded_width == tw && decoded_height == th {
        return data.to_vec();
    }

    let src_stride = decoded_width.saturating_mul(4);
    let dst_stride = tw.saturating_mul(4);
    let rows = th.min(decoded_height);

    #[expect(clippy::as_conversions, reason = "product of u32 values bounded by frame dimensions")]
    let mut cropped = Vec::with_capacity((dst_stride as usize).saturating_mul(rows as usize));

    for row in 0..rows {
        #[expect(clippy::as_conversions, reason = "row * src_stride bounded by frame size")]
        let src_start = (row.saturating_mul(src_stride)) as usize;
        #[expect(clippy::as_conversions, reason = "bounded by frame dimensions")]
        let copy_len = dst_stride.min(src_stride) as usize;
        let src_end = src_start.saturating_add(copy_len);
        if src_end <= data.len() {
            cropped.extend_from_slice(&data[src_start..src_end]);
        }
    }

    cropped
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test handler that records calls
    struct TestHandler {
        caps_confirmed: bool,
        surfaces_created: Vec<u16>,
        surfaces_deleted: Vec<u16>,
        surfaces_mapped: Vec<(u16, u32, u32)>,
        bitmaps_received: Vec<(u16, Codec1Type)>,
        frames_completed: Vec<u32>,
        reset_count: u32,
        closed: bool,
        unhandled: Vec<u16>, // codec_id values for unhandled WireToSurface1
    }

    impl TestHandler {
        fn new() -> Self {
            Self {
                caps_confirmed: false,
                surfaces_created: Vec::new(),
                surfaces_deleted: Vec::new(),
                surfaces_mapped: Vec::new(),
                bitmaps_received: Vec::new(),
                frames_completed: Vec::new(),
                reset_count: 0,
                closed: false,
                unhandled: Vec::new(),
            }
        }
    }

    impl GraphicsPipelineHandler for TestHandler {
        fn on_capabilities_confirmed(&mut self, _caps: &CapabilitySet) {
            self.caps_confirmed = true;
        }

        fn on_reset_graphics(&mut self, _width: u32, _height: u32) {
            self.reset_count += 1;
        }

        fn on_surface_created(&mut self, surface: &Surface) {
            self.surfaces_created.push(surface.id);
        }

        fn on_surface_deleted(&mut self, surface_id: u16) {
            self.surfaces_deleted.push(surface_id);
        }

        fn on_surface_mapped(&mut self, surface_id: u16, origin_x: u32, origin_y: u32) {
            self.surfaces_mapped.push((surface_id, origin_x, origin_y));
        }

        fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
            self.bitmaps_received.push((update.surface_id, update.codec_id));
        }

        fn on_frame_complete(&mut self, frame_id: u32) {
            self.frames_completed.push(frame_id);
        }

        fn on_close(&mut self) {
            self.closed = true;
        }

        fn on_unhandled_pdu(&mut self, pdu: &GfxPdu) {
            if let GfxPdu::WireToSurface1(w) = pdu {
                self.unhandled.push(w.codec_id.into());
            }
        }
    }

    /// Mock decoder that returns a solid-color frame
    struct MockH264Decoder;

    impl H264Decoder for MockH264Decoder {
        fn decode(&mut self, _data: &[u8]) -> crate::decode::DecoderResult<crate::decode::DecodedFrame> {
            // Return a 16x16 solid red frame (macroblock-aligned minimum)
            let width = 16u32;
            let height = 16u32;
            let mut data = vec![0u8; 16 * 16 * 4];
            for pixel in data.chunks_exact_mut(4) {
                pixel[0] = 255; // R
                pixel[3] = 255; // A
            }
            Ok(crate::decode::DecodedFrame { data, width, height })
        }
    }

    #[test]
    fn test_client_sends_capabilities_on_start() {
        let handler = TestHandler::new();
        let mut client = GraphicsPipelineClient::new(Box::new(handler), None);
        let messages = client.start(0).expect("start should succeed");
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn test_client_handles_create_surface() {
        let handler = TestHandler::new();
        let mut client = GraphicsPipelineClient::new(Box::new(handler), None);

        let pdu = GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
            surface_id: 1,
            width: 1920,
            height: 1080,
            pixel_format: PixelFormat::XRgb,
        });

        let _ = client.handle_pdu(pdu).expect("should succeed");
        assert!(client.get_surface(1).is_some());
        assert_eq!(client.get_surface(1).expect("surface exists").width, 1920);
    }

    #[test]
    fn test_client_handles_delete_surface() {
        let handler = TestHandler::new();
        let mut client = GraphicsPipelineClient::new(Box::new(handler), None);

        // Create then delete
        let _ = client
            .handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
                surface_id: 5,
                width: 100,
                height: 100,
                pixel_format: PixelFormat::XRgb,
            }))
            .expect("create should succeed");

        assert!(client.get_surface(5).is_some());

        let _ = client
            .handle_pdu(GfxPdu::DeleteSurface(crate::pdu::DeleteSurfacePdu { surface_id: 5 }))
            .expect("delete should succeed");

        assert!(client.get_surface(5).is_none());
    }

    #[test]
    fn test_client_handles_reset_graphics() {
        let handler = TestHandler::new();
        let mut client = GraphicsPipelineClient::new(Box::new(handler), None);

        // Create two surfaces
        let _ = client.handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
            surface_id: 1,
            width: 100,
            height: 100,
            pixel_format: PixelFormat::XRgb,
        }));
        let _ = client.handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
            surface_id: 2,
            width: 200,
            height: 200,
            pixel_format: PixelFormat::XRgb,
        }));

        assert_eq!(client.surfaces.len(), 2);

        // ResetGraphics should clear all
        let _ = client.handle_pdu(GfxPdu::ResetGraphics(crate::pdu::ResetGraphicsPdu {
            width: 1920,
            height: 1080,
            monitors: vec![],
        }));

        assert!(client.surfaces.is_empty());
    }

    #[test]
    fn test_client_sends_frame_ack() {
        let handler = TestHandler::new();
        let mut client = GraphicsPipelineClient::new(Box::new(handler), None);

        let responses = client
            .handle_pdu(GfxPdu::EndFrame(crate::pdu::EndFramePdu { frame_id: 42 }))
            .expect("end frame should succeed");

        // Should produce exactly one FrameAcknowledge response
        assert_eq!(responses.len(), 1);
        assert_eq!(client.total_frames_decoded(), 1);
    }

    #[test]
    fn test_client_dispatches_avc420() {
        let handler = TestHandler::new();
        let mut client = GraphicsPipelineClient::new(Box::new(handler), Some(Box::new(MockH264Decoder)));

        // Create a surface first
        let _ = client.handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
            surface_id: 1,
            width: 16,
            height: 16,
            pixel_format: PixelFormat::XRgb,
        }));

        // Build a minimal AVC420 bitmap stream:
        // nRect=1 (4 bytes) + rectangle (8 bytes) + quant_qual (2 bytes) + h264 data
        let mut bitmap_data = Vec::new();
        bitmap_data.extend_from_slice(&1u32.to_le_bytes()); // nRect = 1
                                                            // InclusiveRectangle: left=0, top=0, right=15, bottom=15
        bitmap_data.extend_from_slice(&0u16.to_le_bytes()); // left
        bitmap_data.extend_from_slice(&0u16.to_le_bytes()); // top
        bitmap_data.extend_from_slice(&15u16.to_le_bytes()); // right
        bitmap_data.extend_from_slice(&15u16.to_le_bytes()); // bottom
                                                             // QuantQuality: qp=22 (bits 0..6), progressive=false (bit 7), quality=100
        bitmap_data.push(22); // qp=22, progressive=0
        bitmap_data.push(100); // quality
                               // Fake H.264 data (decoder is mocked)
        bitmap_data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x67]);

        let pdu = GfxPdu::WireToSurface1(crate::pdu::WireToSurface1Pdu {
            surface_id: 1,
            codec_id: Codec1Type::Avc420,
            pixel_format: PixelFormat::XRgb,
            destination_rectangle: InclusiveRectangle {
                left: 0,
                top: 0,
                right: 15,
                bottom: 15,
            },
            bitmap_data,
        });

        let _ = client.handle_pdu(pdu).expect("AVC420 dispatch should succeed");
    }

    #[test]
    fn test_client_handles_uncompressed() {
        let handler = TestHandler::new();
        let mut client = GraphicsPipelineClient::new(Box::new(handler), None);

        // Create a surface
        let _ = client.handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
            surface_id: 1,
            width: 4,
            height: 4,
            pixel_format: PixelFormat::XRgb,
        }));

        // 4x4 uncompressed XRGB data (4 bytes per pixel)
        let raw_data = vec![0u8; 4 * 4 * 4];

        let pdu = GfxPdu::WireToSurface1(crate::pdu::WireToSurface1Pdu {
            surface_id: 1,
            codec_id: Codec1Type::Uncompressed,
            pixel_format: PixelFormat::XRgb,
            destination_rectangle: InclusiveRectangle {
                left: 0,
                top: 0,
                right: 3,
                bottom: 3,
            },
            bitmap_data: raw_data,
        });

        let _ = client.handle_pdu(pdu).expect("uncompressed should succeed");
    }

    #[test]
    fn test_client_skips_decode_without_decoder() {
        let handler = TestHandler::new();
        // No decoder provided
        let mut client = GraphicsPipelineClient::new(Box::new(handler), None);

        // Create surface
        let _ = client.handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
            surface_id: 1,
            width: 16,
            height: 16,
            pixel_format: PixelFormat::XRgb,
        }));

        // AVC420 data (minimal)
        let mut bitmap_data = Vec::new();
        bitmap_data.extend_from_slice(&1u32.to_le_bytes());
        bitmap_data.extend_from_slice(&0u16.to_le_bytes());
        bitmap_data.extend_from_slice(&0u16.to_le_bytes());
        bitmap_data.extend_from_slice(&15u16.to_le_bytes());
        bitmap_data.extend_from_slice(&15u16.to_le_bytes());
        bitmap_data.push(22);
        bitmap_data.push(100);
        bitmap_data.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x67]);

        let pdu = GfxPdu::WireToSurface1(crate::pdu::WireToSurface1Pdu {
            surface_id: 1,
            codec_id: Codec1Type::Avc420,
            pixel_format: PixelFormat::XRgb,
            destination_rectangle: InclusiveRectangle {
                left: 0,
                top: 0,
                right: 15,
                bottom: 15,
            },
            bitmap_data,
        });

        // Should succeed without panicking
        let _ = client.handle_pdu(pdu).expect("should succeed without decoder");
    }

    #[test]
    fn test_crop_decoded_frame_identity() {
        let data = vec![0xFFu8; 4 * 4 * 4]; // 4x4 RGBA
        let cropped = crop_decoded_frame(&data, 4, 4, 4, 4);
        assert_eq!(cropped.len(), data.len());
    }

    #[test]
    fn test_crop_decoded_frame_macroblock() {
        // H.264 encodes 1920x1080 as 1920x1088 (1080 rounded up to 16-pixel macroblock boundary)
        let decoded_w = 1920u32;
        let decoded_h = 1088u32;
        let data = vec![0xAAu8; 1920 * 1088 * 4];

        let cropped = crop_decoded_frame(&data, decoded_w, decoded_h, 1920, 1080);
        assert_eq!(cropped.len(), 1920 * 1080 * 4);
    }

    #[test]
    fn test_client_state_transitions() {
        let handler = TestHandler::new();
        let mut client = GraphicsPipelineClient::new(Box::new(handler), None);

        assert_eq!(client.state, ClientState::WaitingForConfirm);
        assert!(!client.is_active());

        // Confirm capabilities
        let _ = client.handle_pdu(GfxPdu::CapabilitiesConfirm(crate::pdu::CapabilitiesConfirmPdu(
            CapabilitySet::V8 {
                flags: CapabilitiesV8Flags::empty(),
            },
        )));

        assert_eq!(client.state, ClientState::Active);
        assert!(client.is_active());

        // Close
        client.close(0);
        assert_eq!(client.state, ClientState::Closed);
        assert!(!client.is_active());
    }

    #[test]
    fn test_frame_ordering() {
        let handler = TestHandler::new();
        let mut client = GraphicsPipelineClient::new(Box::new(handler), None);

        // Create surface
        let _ = client.handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
            surface_id: 1,
            width: 4,
            height: 4,
            pixel_format: PixelFormat::XRgb,
        }));

        // StartFrame
        let _ = client.handle_pdu(GfxPdu::StartFrame(crate::pdu::StartFramePdu {
            timestamp: crate::pdu::Timestamp {
                milliseconds: 0,
                seconds: 0,
                minutes: 0,
                hours: 0,
            },
            frame_id: 1,
        }));

        // WireToSurface1 (uncompressed)
        let _ = client.handle_pdu(GfxPdu::WireToSurface1(crate::pdu::WireToSurface1Pdu {
            surface_id: 1,
            codec_id: Codec1Type::Uncompressed,
            pixel_format: PixelFormat::XRgb,
            destination_rectangle: InclusiveRectangle {
                left: 0,
                top: 0,
                right: 3,
                bottom: 3,
            },
            bitmap_data: vec![0u8; 4 * 4 * 4],
        }));

        // EndFrame should produce FrameAcknowledge
        let responses = client
            .handle_pdu(GfxPdu::EndFrame(crate::pdu::EndFramePdu { frame_id: 1 }))
            .expect("end frame");

        assert_eq!(responses.len(), 1);
        assert_eq!(client.total_frames_decoded(), 1);
    }
}
