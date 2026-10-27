//! Client-side EGFX implementation
//!
//! This module provides client-side support for the Graphics Pipeline Extension
//! ([MS-RDPEGFX]), including H.264 AVC420 and AVC444 decode, ClearCodec decode, and surface management.
//!
//! # Protocol Compliance
//!
//! This implementation follows MS-RDPEGFX client requirements:
//!
//! - **Capability Negotiation**: Advertises V8 through V10.7 ([2.2.3])
//! - **Surface Management**: Tracks server-created surfaces ([3.3.1.6])
//! - **Frame Acknowledgment**: Sends `FrameAcknowledge` after `EndFrame` ([3.3.5.12])
//! - **Codec Dispatch**: Routes `WireToSurface1` by `codec_id` ([3.3.5.2])
//! - **Persistent Bitmap Cache**: Offers stored cache entries after `CapabilitiesConfirm` and
//!   preloads the slots the server accepts ([3.3.5.16], [3.3.5.17]), through an embedder-supplied
//!   [`PersistentCacheStore`]
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
//!    |--- WireToSurface1 (codec) ----------->|  -> H264/ClearCodec decode
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
//! [3.3.5.16]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/adf5cc51-d0b1-4533-987f-16296bbe6966
//! [3.3.5.17]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/df29d040-f74b-4002-92ed-f0903b8402c3

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use ironrdp_core::{Decode as _, ReadCursor, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_graphics::clearcodec::ClearCodecDecoder;
use ironrdp_graphics::progressive::{ProgressiveDecoder, TILE_BYTES_PER_PIXEL, TILE_DIM};
use ironrdp_graphics::rdp6::BitmapStreamDecoder;
use ironrdp_graphics::rectangle_processing::Region;
use ironrdp_graphics::rfx::{self, MessageDecoder as RfxMessageDecoder, TileDecoder as RfxTileDecoder};
use ironrdp_graphics::zgfx;
use ironrdp_pdu::geometry::{ExclusiveRectangle, InclusiveRectangle};
use ironrdp_pdu::{PduResult, decode_cursor, decode_err, pdu_other_err};
use tracing::{debug, trace, warn};

use crate::CHANNEL_NAME;
use crate::avc444::{Avc444Layout, Yuv444Frame};
use crate::cache::{
    CACHE_BYTES, CACHE_SLOTS, MAX_CACHE_IMPORT_ENTRIES, MAX_CACHE_IMPORT_ENTRY_BYTES, PersistentCacheStore,
    SMALL_CACHE_BYTES, SMALL_CACHE_SLOTS,
};
use crate::compositor::{Compositor, OutputUpdate};
use crate::decode::H264Decoder;
use crate::pdu::{
    Avc420BitmapStream, Avc444BitmapStream, CacheImportOfferPdu, CacheImportReplyPdu, CacheToSurfacePdu,
    CapabilitiesAdvertisePdu, CapabilitiesV8Flags, CapabilitiesV10Flags, CapabilitiesV81Flags, CapabilitiesV103Flags,
    CapabilitiesV104Flags, CapabilitiesV107Flags, CapabilitySet, Codec1Type, DeleteEncodingContextPdu, Encoding,
    EvictCacheEntryPdu, FrameAcknowledgePdu, GfxPdu, MapSurfaceToScaledOutputPdu, MapSurfaceToScaledWindowPdu,
    MapSurfaceToWindowPdu, PixelFormat, QueueDepth, RawCapabilitySet, SolidFillPdu, SurfaceToCachePdu,
    SurfaceToSurfacePdu, WireToSurface2Pdu,
};

/// Upper bound on the region-clipping work of one RemoteFX command: rectangles scanned while
/// normalizing each frame's region, plus tiles times normalized rectangles. It bounds the work a
/// small payload can request, and a command past it is dropped. Normalization merges overlapping
/// and repeated rectangles first, so only a region of very many disjoint rectangles reaches it,
/// the same bound the RemoteFX Progressive path applies.
const MAX_RFX_CLIPPING_WORK: usize = 1 << 20;

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
#[non_exhaustive]
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
#[non_exhaustive]
pub struct CodecCapabilities {
    /// AVC420 (H.264 4:2:0) is available
    pub avc420: bool,
    /// AVC444 (H.264 4:4:4) is available
    pub avc444: bool,
    /// The bitmap cache is constrained to 16 MB and 4,096 slots instead of 100 MB and 25,600
    /// slots ([MS-RDPEGFX] 3.3.1.4)
    ///
    /// Set by `SMALL_CACHE`, by `THINCLIENT` in versions 8 and 8.1, and by version 10.3.
    ///
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
    pub small_cache: bool,
    /// Thin client mode: `THINCLIENT` in versions 8 and 8.1, `AVC_THINCLIENT` from version 10.3
    pub thin_client: bool,
}

impl CodecCapabilities {
    fn from_capability_set(cap: &CapabilitySet) -> Self {
        // Mirrors the server-side extraction logic
        match cap {
            CapabilitySet::V8 { flags } => Self {
                avc420: false,
                avc444: false,
                small_cache: flags.intersects(CapabilitiesV8Flags::SMALL_CACHE | CapabilitiesV8Flags::THIN_CLIENT),
                thin_client: flags.contains(CapabilitiesV8Flags::THIN_CLIENT),
            },
            CapabilitySet::V8_1 { flags } => Self {
                avc420: flags.contains(CapabilitiesV81Flags::AVC420_ENABLED),
                avc444: false,
                small_cache: flags.intersects(CapabilitiesV81Flags::SMALL_CACHE | CapabilitiesV81Flags::THIN_CLIENT),
                thin_client: flags.contains(CapabilitiesV81Flags::THIN_CLIENT),
            },
            CapabilitySet::V10 { flags } | CapabilitySet::V10_2 { flags } => Self {
                avc420: !flags.contains(CapabilitiesV10Flags::AVC_DISABLED),
                avc444: !flags.contains(CapabilitiesV10Flags::AVC_DISABLED),
                small_cache: flags.contains(CapabilitiesV10Flags::SMALL_CACHE),
                thin_client: false,
            },
            CapabilitySet::V10_1 => Self {
                avc420: true,
                avc444: true,
                small_cache: false,
                thin_client: false,
            },
            CapabilitySet::V10_3 { flags } => Self {
                avc420: !flags.contains(CapabilitiesV103Flags::AVC_DISABLED),
                avc444: !flags.contains(CapabilitiesV103Flags::AVC_DISABLED),
                // MS-RDPEGFX 3.3.1.4: version 10.3 always constrains the cache to 16 MB.
                small_cache: true,
                thin_client: flags.contains(CapabilitiesV103Flags::AVC_THIN_CLIENT),
            },
            CapabilitySet::V10_4 { flags }
            | CapabilitySet::V10_5 { flags }
            | CapabilitySet::V10_6 { flags }
            | CapabilitySet::V10_6Err { flags } => Self {
                avc420: !flags.contains(CapabilitiesV104Flags::AVC_DISABLED),
                avc444: !flags.contains(CapabilitiesV104Flags::AVC_DISABLED),
                small_cache: flags.contains(CapabilitiesV104Flags::SMALL_CACHE),
                thin_client: flags.contains(CapabilitiesV104Flags::AVC_THIN_CLIENT),
            },
            CapabilitySet::V10_7 { flags } => Self {
                avc420: !flags.contains(CapabilitiesV107Flags::AVC_DISABLED),
                avc444: !flags.contains(CapabilitiesV107Flags::AVC_DISABLED),
                small_cache: flags.contains(CapabilitiesV107Flags::SMALL_CACHE),
                thin_client: flags.contains(CapabilitiesV107Flags::AVC_THIN_CLIENT),
            },
        }
    }
}

// ============================================================================
// Bitmap Update
// ============================================================================

/// Decoded bitmap data for a surface region
///
/// Delivered to [`GraphicsPipelineHandler::on_bitmap_updated`] when
/// a `WireToSurface1` or `WireToSurface2` PDU is processed with decoded pixel data.
#[derive(Debug)]
#[non_exhaustive]
pub struct BitmapUpdate {
    /// Surface this update applies to
    pub surface_id: u16,
    /// Destination rectangle within the surface (exclusive `right`/`bottom`)
    pub destination_rectangle: ExclusiveRectangle,
    /// Codec associated with this update
    ///
    /// RFX Progressive tiles use [`Codec1Type::Uncompressed`] after decoding to RGBA.
    /// [`Codec1Type::Alpha`] updates carry the surface pixels after the opacity was applied:
    /// the color an earlier command painted, with the alpha this command set.
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
///
/// All methods have default no-op implementations so you only need
/// to override the ones relevant to your use case.
pub trait GraphicsPipelineHandler: Send {
    /// Returns the capability sets to advertise to the server
    ///
    /// The default advertises every version from 10.7 down to 10 with AVC444, then V8.1
    /// (AVC420) and V8 (no AVC) as fallback, all with a small cache.
    ///
    /// Sets the [`GraphicsPipelineClient`] cannot honor are filtered out at
    /// advertisement time: without an H.264 decoder, every set that implies AVC; with
    /// a decoder that does not return YUV planes ([`H264Decoder::supports_yuv420`]),
    /// every set that implies AVC444, since the server picks the most capable
    /// advertised set ([capability negotiation]) and would then send frames the client
    /// cannot paint. If nothing is left, a V8-only fallback is used.
    ///
    /// [capability negotiation]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/31c6e2b1-335b-4a75-9454-bb2309958c21
    fn capabilities(&self) -> Vec<CapabilitySet> {
        vec![
            CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::SMALL_CACHE,
            },
            CapabilitySet::V10_6 {
                flags: CapabilitiesV104Flags::SMALL_CACHE,
            },
            CapabilitySet::V10_5 {
                flags: CapabilitiesV104Flags::SMALL_CACHE,
            },
            CapabilitySet::V10_4 {
                flags: CapabilitiesV104Flags::SMALL_CACHE,
            },
            CapabilitySet::V10_3 {
                flags: CapabilitiesV103Flags::empty(),
            },
            CapabilitySet::V10_2 {
                flags: CapabilitiesV10Flags::SMALL_CACHE,
            },
            CapabilitySet::V10_1,
            CapabilitySet::V10 {
                flags: CapabilitiesV10Flags::SMALL_CACHE,
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

    /// Called when a graphics-output reset has invalid dimensions or exceeds the output allocation limit.
    fn on_reset_graphics_rejected(&mut self, _width: u32, _height: u32) {}

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

    // ========================================================================
    // Additional PDU handlers (server→client)
    // ========================================================================

    /// Called when the server fills a surface region with a solid color
    ///
    /// Per [MS-RDPEGFX 3.3.5.4].
    ///
    /// [MS-RDPEGFX 3.3.5.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/d696ab07-fd47-42f6-a601-c8b6fae26577
    fn on_solid_fill(&mut self, _pdu: &SolidFillPdu) {}

    /// Called when the server copies pixels between surfaces
    ///
    /// Per [MS-RDPEGFX 3.3.5.5].
    ///
    /// [MS-RDPEGFX 3.3.5.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/0b19d058-fff0-43e5-8671-8c4186d60529
    fn on_surface_to_surface(&mut self, _pdu: &SurfaceToSurfacePdu) {}

    /// Called when the server caches a surface region
    ///
    /// Per [MS-RDPEGFX 3.3.5.6].
    ///
    /// [MS-RDPEGFX 3.3.5.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/01108b9f-a888-4e5c-b790-42d5c5985998
    fn on_surface_to_cache(&mut self, _pdu: &SurfaceToCachePdu) {}

    /// Called when the server renders cached content to a surface
    ///
    /// Per [MS-RDPEGFX 3.3.5.7].
    ///
    /// [MS-RDPEGFX 3.3.5.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/78c00bcd-f5cb-4c33-8d6c-f4cd50facfab
    fn on_cache_to_surface(&mut self, _pdu: &CacheToSurfacePdu) {}

    /// Called when the server evicts a cache entry
    ///
    /// Per [MS-RDPEGFX 3.3.5.8].
    ///
    /// [MS-RDPEGFX 3.3.5.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/9dd32c5c-fabc-497b-81be-776fa581a4f6
    fn on_evict_cache_entry(&mut self, _pdu: &EvictCacheEntryPdu) {}

    /// Called when the server maps a surface to a RAIL window
    ///
    /// Per [MS-RDPEGFX 2.2.2.20].
    ///
    /// [MS-RDPEGFX 2.2.2.20]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/2ec1357c-ee65-4d9b-89f3-8fc49348c92a
    fn on_map_surface_to_window(&mut self, _pdu: &MapSurfaceToWindowPdu) {}

    /// Called when the server maps a surface to a scaled output
    ///
    /// Per [MS-RDPEGFX 2.2.2.22].
    ///
    /// [MS-RDPEGFX 2.2.2.22]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/3fcc3e63-e5a2-4b18-a572-26bbeb87b3aa
    fn on_map_surface_to_scaled_output(&mut self, _pdu: &MapSurfaceToScaledOutputPdu) {}

    /// Called when the server maps a surface to a scaled RAIL window
    ///
    /// Per [MS-RDPEGFX 2.2.2.23].
    ///
    /// [MS-RDPEGFX 2.2.2.23]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/22fc0ec7-38ce-4d9d-ad6d-93a0e9f3c38c
    fn on_map_surface_to_scaled_window(&mut self, _pdu: &MapSurfaceToScaledWindowPdu) {}

    /// Called when the server sends an RFX Progressive bitmap PDU
    ///
    /// Implement [`GraphicsPipelineHandler::on_bitmap_updated`] to render its decoded tiles.
    ///
    /// Per [MS-RDPEGFX 3.3.5.2].
    ///
    /// [MS-RDPEGFX 3.3.5.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/9791fc34-7644-4279-844f-7728ae9959c2
    fn on_wire_to_surface2(&mut self, _pdu: &WireToSurface2Pdu) {}

    /// Called when the server deletes a progressive encoding context
    ///
    /// Per [MS-RDPEGFX 2.2.2.3].
    ///
    /// [MS-RDPEGFX 2.2.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/bd0c64d4-07b3-47e5-9f7b-ba5c14a3a2e2
    fn on_delete_encoding_context(&mut self, _pdu: &DeleteEncodingContextPdu) {}

    /// Called when the server replies to a cache import offer
    ///
    /// Per [MS-RDPEGFX 2.2.2.17].
    ///
    /// [MS-RDPEGFX 2.2.2.17]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/0c4d88f8-50dc-465a-ab00-88a3fe0ec3c5
    fn on_cache_import_reply(&mut self, _pdu: &CacheImportReplyPdu) {}

    /// Called for PDUs that have no specific handler
    ///
    /// This is a catch-all for any GfxPdu variant not matched above.
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
    /// Ignoring in-flight graphics until the protocol reset is confirmed.
    Resetting,
    /// Channel is active, processing frames
    Active,
    /// Channel has been closed or a protocol reset has failed.
    Closed,
}

// ============================================================================
// Graphics Pipeline Client
// ============================================================================

/// Client for the Graphics Pipeline Virtual Channel (EGFX)
///
/// This client handles capability negotiation, surface tracking,
/// H.264 AVC420, AVC444 and RFX Progressive decode, and frame acknowledgment per [MS-RDPEGFX].
///
/// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
pub struct GraphicsPipelineClient {
    handler: Box<dyn GraphicsPipelineHandler>,
    h264_decoder: Option<Box<dyn H264Decoder>>,
    /// Combined AVC444 picture of each surface, kept between updates so a luma-only or
    /// chroma-only update combines with what earlier ones left.
    avc444_frames: BTreeMap<u16, Yuv444Frame>,
    /// Built on first use via [`Self::decode_clearcodec`]. `None` means no ClearCodec
    /// frame has arrived yet, not that the codec is unsupported: keeping the ~1.37 MiB
    /// V-bar and glyph cache spine (see `ClearCodecDecoder::new`) unallocated saves that
    /// much per session for the common case of a server that never sends ClearCodec.
    clearcodec_decoder: Option<ClearCodecDecoder>,
    planar_decoder: BitmapStreamDecoder,
    progressive_decoder: ProgressiveDecoder,
    rfx_decoder: RfxMessageDecoder,
    rfx_tiles: RfxTileDecoder,
    /// Lifetime decode failures, used to throttle diagnostics.
    dropped_updates: u64,

    decompressor: zgfx::Decompressor,
    decompressed_buffer: Vec<u8>,

    state: ClientState,
    negotiated_caps: Option<CapabilitySet>,
    codec_caps: CodecCapabilities,

    surfaces: BTreeMap<u16, Surface>,
    compositor: Compositor,
    current_frame_id: Option<u32>,
    total_frames_decoded: u32,
    pending_output_reset: Option<(u16, u16)>,

    persistent_cache: Option<Box<dyn PersistentCacheStore>>,
    /// Whether the persistent cache was already listed for an offer on this channel. Offering
    /// again after the session filled and evicted slots would announce stale entries.
    cache_import_attempted: bool,
    /// Keys of the last `CacheImportOffer`, in offer order, until its reply arrives.
    offered_cache_keys: Option<Vec<u64>>,
}

impl GraphicsPipelineClient {
    /// Create a new `GraphicsPipelineClient`
    ///
    /// If `h264_decoder` is `None`, AVC420 frames are logged and skipped.
    /// ClearCodec decoding is always available (no external decoder required)
    /// and its cache spine is allocated lazily, on the first ClearCodec frame,
    /// rather than up front for a codec the session may never use.
    pub fn new(handler: Box<dyn GraphicsPipelineHandler>, h264_decoder: Option<Box<dyn H264Decoder>>) -> Self {
        Self {
            handler,
            h264_decoder,
            avc444_frames: BTreeMap::new(),
            clearcodec_decoder: None,
            planar_decoder: BitmapStreamDecoder::default(),
            progressive_decoder: ProgressiveDecoder::new(),
            rfx_decoder: RfxMessageDecoder::new(),
            rfx_tiles: RfxTileDecoder::new(),
            dropped_updates: 0,
            decompressor: zgfx::Decompressor::new(),
            decompressed_buffer: Vec::new(),
            state: ClientState::WaitingForConfirm,
            negotiated_caps: None,
            codec_caps: CodecCapabilities::default(),
            surfaces: BTreeMap::new(),
            compositor: Compositor::default(),
            current_frame_id: None,
            total_frames_decoded: 0,
            pending_output_reset: None,
            persistent_cache: None,
            cache_import_attempted: false,
            offered_cache_keys: None,
        }
    }

    /// Keep the bitmap cache across sessions in `store` ([MS-RDPEGFX] [3.3.1.5]).
    ///
    /// Once the server confirms capabilities, the client offers the stored entries in a
    /// `CacheImportOffer` and loads the ones the server accepts into the slots its
    /// `CacheImportReply` assigns. When the channel closes, the bitmap cache is saved back.
    ///
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
    /// [3.3.1.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/1a2abd79-889c-4207-a1c3-c8c1e1d6f117
    #[must_use]
    pub fn with_persistent_cache(mut self, store: Box<dyn PersistentCacheStore>) -> Self {
        self.persistent_cache = Some(store);
        self
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

    /// Request an in-channel protocol reset with the negotiated capabilities.
    ///
    /// [MS-RDPEGFX 3.3.5.19] "Processing an RDPGFX_CAPS_CONFIRM_PDU message" permits this for V10.3-V10.7.
    /// Returns `None` before negotiation, while resetting, after closure, or for unsupported versions.
    /// Send the returned PDU on this graphics channel before processing more input; if sending fails, recreate the channel.
    /// Calling this method clears graphics state immediately; discarding the returned PDU stalls graphics until channel recreation.
    /// Incoming graphics are ignored until the advertised version is confirmed; changed capability flags are accepted.
    /// Any processing error while awaiting confirmation closes this client; subsequent `process()` calls fail.
    /// Recreate the channel with a new client after such an error.
    /// Surfaces and cached pixels are discarded; the last output extent remains until a post-confirmation `ResetGraphics` arrives.
    /// With a persistent cache, the bitmap cache is saved first and offered again once the reset is confirmed.
    /// A previously confirmed `V10_6Err` is echoed for compatibility with peers using that alternate V10.6 value.
    /// The ACK count restarts for Windows interoperability, despite the connection-wide wording in
    /// [MS-RDPEGFX 2.2.2.13] "RDPGFX_FRAME_ACKNOWLEDGE_PDU".
    /// This does not reset other virtual channels or guarantee a particular codec or keyframe.
    ///
    /// [MS-RDPEGFX 3.3.5.19]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/a720af02-0a47-44e2-a934-dc8534062b44
    /// [MS-RDPEGFX 2.2.2.13]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/0241e258-77ef-4a58-b426-5039ed6296ce
    #[must_use]
    pub fn request_protocol_reset(&mut self) -> Option<GfxPdu> {
        if !self.is_active() {
            return None;
        }
        let caps = self.negotiated_caps.as_ref()?;
        if !matches!(
            caps,
            CapabilitySet::V10_3 { .. }
                | CapabilitySet::V10_4 { .. }
                | CapabilitySet::V10_5 { .. }
                | CapabilitySet::V10_6 { .. }
                | CapabilitySet::V10_6Err { .. }
                | CapabilitySet::V10_7 { .. }
        ) {
            return None;
        }
        let pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(core::slice::from_ref(caps)));
        self.state = ClientState::Resetting;
        self.surfaces.clear();
        // The compositor reset below drops their budget charge with everything else.
        self.avc444_frames.clear();
        // The reset discards the bitmap cache; save it first, and let the confirmation offer it
        // again, so the server can paste from it without uploading the tiles anew.
        self.save_persistent_cache();
        self.cache_import_attempted = false;
        self.offered_cache_keys = None;
        self.compositor.reset_protocol();
        // A full channel reset restarts codec contexts, unlike the output-only ResetGraphics PDU.
        self.clearcodec_decoder = None;
        self.planar_decoder = BitmapStreamDecoder::default();
        self.progressive_decoder = ProgressiveDecoder::new();
        if let Some(decoder) = self.h264_decoder.as_mut() {
            decoder.reset();
        }
        self.current_frame_id = None;
        // Windows restarts its sent-frame count and rejects ACK counts greater than that value.
        self.total_frames_decoded = 0;
        // ZGFX history still decodes packets already in flight before confirmation.
        Some(pdu)
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

    /// Get the number of frames decoded since channel creation or the last protocol reset.
    #[must_use]
    pub fn total_frames_decoded(&self) -> u32 {
        self.total_frames_decoded
    }

    /// Get the lifetime number of surface updates dropped after a codec decoding failure.
    #[must_use]
    pub fn dropped_updates(&self) -> u64 {
        self.dropped_updates
    }

    /// Take the output-buffer regions that changed in completed frames.
    ///
    /// The client-side compositor applies every surface command (`WireToSurface1`
    /// decodes, `SolidFill`, `SurfaceToSurface`, and the bitmap cache) into
    /// persistent RGBA8888 surfaces and maps them onto the graphics output. This
    /// drains the accumulated output-space deltas, committed per `EndFrame` and
    /// ready to blit into a framebuffer. Each call empties the queue.
    #[must_use]
    pub fn drain_output(&mut self) -> Vec<OutputUpdate> {
        self.compositor.drain_output()
    }

    /// Take the RemoteApp window regions that changed in completed frames.
    ///
    /// Surfaces the server maps to a RAIL window with `MapSurfaceToWindow` or
    /// `MapSurfaceToScaledWindow` are presented in that window rather than on the
    /// graphics output, so their changes are drained here instead of by
    /// [`Self::drain_output`]. Each call empties the queue.
    #[must_use]
    pub fn drain_window_updates(&mut self) -> Vec<crate::compositor::WindowUpdate> {
        self.compositor.drain_window_updates()
    }

    /// Take the most recent graphics-output extent announced by `ResetGraphics`.
    ///
    /// The returned dimensions satisfy the protocol's output limit and the
    /// compositor's output-framebuffer allocation limit and are reported once.
    #[must_use]
    pub fn take_output_reset(&mut self) -> Option<(u16, u16)> {
        self.pending_output_reset.take()
    }

    // ========================================================================
    // PDU Handlers
    // ========================================================================

    fn process_messages(&mut self, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        self.decompressed_buffer.clear();
        self.decompressed_buffer.shrink_to(MAX_DECOMPRESSED_BUFFER_CAPACITY);
        self.decompressor
            .decompress(payload, &mut self.decompressed_buffer)
            .map_err(|e| decode_err!(e))?;

        // Decode all PDUs before dispatch while the cursor borrows the buffer.
        let mut pdus = Vec::new();
        {
            let mut cursor = ReadCursor::new(self.decompressed_buffer.as_slice());
            while !cursor.is_empty() {
                let pdu: GfxPdu = decode_cursor(&mut cursor).map_err(|e| decode_err!(e))?;
                pdus.push(pdu);
            }
        }

        let mut responses: Vec<DvcMessage> = Vec::new();
        for pdu in pdus {
            let pdu_responses = self.handle_pdu(pdu)?;
            responses.extend(pdu_responses);
        }

        Ok(responses)
    }

    fn handle_pdu(&mut self, pdu: GfxPdu) -> PduResult<Vec<DvcMessage>> {
        // MS-RDPEGFX 3.3.5.19 ignores all pre-confirmation server messages, including ResetGraphics.
        if self.state == ClientState::Resetting && !matches!(pdu, GfxPdu::CapabilitiesConfirm(_)) {
            return Ok(Vec::new());
        }
        match pdu {
            GfxPdu::CapabilitiesConfirm(confirm) => {
                self.handle_capabilities_confirm(confirm.0)?;
                Ok(self.cache_import_offer().into_iter().collect())
            }
            GfxPdu::ResetGraphics(reset) => {
                self.handle_reset_graphics(reset.width, reset.height)?;
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
                self.progressive_decoder.begin_frame();
                trace!(frame_id = start.frame_id, "StartFrame");
                Ok(vec![])
            }
            GfxPdu::WireToSurface1(wire) => {
                self.handle_wire_to_surface1(wire)?;
                Ok(vec![])
            }
            GfxPdu::WireToSurface2(pdu) => {
                trace!("WireToSurface2 (progressive codec)");
                self.handler.on_wire_to_surface2(&pdu);
                self.handle_wire_to_surface2(pdu)?;
                Ok(vec![])
            }
            GfxPdu::EndFrame(end) => self.handle_end_frame(end.frame_id),

            // Surface operations
            GfxPdu::SolidFill(pdu) => {
                trace!(surface_id = pdu.surface_id, "SolidFill");
                self.compositor
                    .solid_fill(pdu.surface_id, &pdu.fill_pixel, &pdu.rectangles);
                self.handler.on_solid_fill(&pdu);
                Ok(vec![])
            }
            GfxPdu::SurfaceToSurface(pdu) => {
                trace!(
                    src = pdu.source_surface_id,
                    dst = pdu.destination_surface_id,
                    "SurfaceToSurface"
                );
                self.compositor.surface_to_surface(
                    pdu.source_surface_id,
                    pdu.destination_surface_id,
                    &pdu.source_rectangle,
                    &pdu.destination_points,
                );
                self.handler.on_surface_to_surface(&pdu);
                Ok(vec![])
            }

            // Cache operations
            GfxPdu::SurfaceToCache(pdu) => {
                trace!(
                    surface_id = pdu.surface_id,
                    cache_slot = pdu.cache_slot,
                    "SurfaceToCache"
                );
                self.compositor
                    .surface_to_cache(pdu.surface_id, pdu.cache_slot, pdu.cache_key, &pdu.source_rectangle);
                self.handler.on_surface_to_cache(&pdu);
                Ok(vec![])
            }
            GfxPdu::CacheToSurface(pdu) => {
                trace!(
                    cache_slot = pdu.cache_slot,
                    surface_id = pdu.surface_id,
                    "CacheToSurface"
                );
                self.compositor
                    .cache_to_surface(pdu.cache_slot, pdu.surface_id, &pdu.destination_points);
                self.handler.on_cache_to_surface(&pdu);
                Ok(vec![])
            }
            GfxPdu::EvictCacheEntry(pdu) => {
                trace!(cache_slot = pdu.cache_slot, "EvictCacheEntry");
                self.compositor.evict_cache_entry(pdu.cache_slot);
                self.handler.on_evict_cache_entry(&pdu);
                Ok(vec![])
            }
            GfxPdu::CacheImportReply(pdu) => {
                trace!(imported = pdu.cache_slots.len(), "CacheImportReply");
                self.handle_cache_import_reply(&pdu);
                self.handler.on_cache_import_reply(&pdu);
                Ok(vec![])
            }

            // Surface mapping variants
            GfxPdu::MapSurfaceToWindow(pdu) => {
                trace!(
                    surface_id = pdu.surface_id,
                    window_id = pdu.window_id,
                    "MapSurfaceToWindow"
                );
                self.handle_map_surface_to_window(
                    pdu.surface_id,
                    pdu.window_id,
                    (pdu.mapped_width, pdu.mapped_height),
                    (pdu.mapped_width, pdu.mapped_height),
                );
                self.handler.on_map_surface_to_window(&pdu);
                Ok(vec![])
            }
            GfxPdu::MapSurfaceToScaledOutput(pdu) => {
                trace!(surface_id = pdu.surface_id, "MapSurfaceToScaledOutput");
                self.handle_map_surface_to_scaled_output(&pdu);
                self.handler.on_map_surface_to_scaled_output(&pdu);
                Ok(vec![])
            }
            GfxPdu::MapSurfaceToScaledWindow(pdu) => {
                trace!(surface_id = pdu.surface_id, "MapSurfaceToScaledWindow");
                self.handle_map_surface_to_window(
                    pdu.surface_id,
                    pdu.window_id,
                    (pdu.mapped_width, pdu.mapped_height),
                    (pdu.target_width, pdu.target_height),
                );
                self.handler.on_map_surface_to_scaled_window(&pdu);
                Ok(vec![])
            }

            // Progressive codec context management
            GfxPdu::DeleteEncodingContext(pdu) => {
                trace!(
                    surface_id = pdu.surface_id,
                    codec_context_id = pdu.codec_context_id,
                    "DeleteEncodingContext"
                );
                self.progressive_decoder
                    .delete_context(pdu.surface_id, pdu.codec_context_id);
                self.handler.on_delete_encoding_context(&pdu);
                Ok(vec![])
            }

            // Catch-all for any remaining PDUs
            other => {
                self.handler.on_unhandled_pdu(&other);
                Ok(vec![])
            }
        }
    }

    fn handle_capabilities_confirm(&mut self, cap: RawCapabilitySet) -> PduResult<()> {
        if self.state == ClientState::Resetting
            && Some(cap.version) != self.negotiated_caps.as_ref().map(CapabilitySet::version)
        {
            return Err(pdu_other_err!(
                "server confirmed an unadvertised graphics reset version"
            ));
        }
        // Server confirms a single capability set. If we cannot interpret it
        // (unknown version, or malformed body), we still transition to Active
        // to avoid hanging the session, but we keep `negotiated_caps` empty
        // and skip the typed callback so consumers don't observe a confirm
        // they can't reason about.
        let cap = match cap.parsed() {
            Ok(Some(typed)) => typed,
            Ok(None) => {
                warn!(
                    version = cap.version.0,
                    "Server confirmed an unknown EGFX capability version; proceeding with defaults"
                );
                self.negotiated_caps = None;
                self.codec_caps = CodecCapabilities::default();
                self.compositor.set_small_bitmap_cache(self.codec_caps.small_cache);
                self.state = ClientState::Active;
                return Ok(());
            }
            Err(e) if self.state == ClientState::Resetting => return Err(decode_err!(e)),
            Err(e) => {
                warn!(error = %e, "Failed to parse server's EGFX capabilities confirmation");
                self.negotiated_caps = None;
                self.codec_caps = CodecCapabilities::default();
                self.compositor.set_small_bitmap_cache(self.codec_caps.small_cache);
                self.state = ClientState::Active;
                return Ok(());
            }
        };

        self.codec_caps = CodecCapabilities::from_capability_set(&cap);
        self.compositor.set_small_bitmap_cache(self.codec_caps.small_cache);
        self.state = ClientState::Active;
        let cap = self.negotiated_caps.insert(cap);

        debug!(
            version = cap.version().0,
            avc420 = self.codec_caps.avc420,
            avc444 = self.codec_caps.avc444,
            small_cache = self.codec_caps.small_cache,
            thin_client = self.codec_caps.thin_client,
            "EGFX capabilities confirmed"
        );

        self.handler.on_capabilities_confirmed(cap);
        Ok(())
    }

    fn handle_reset_graphics(&mut self, width: u32, height: u32) -> PduResult<()> {
        let output_size = Compositor::materializable_output_size(width, height);

        // Per spec, ResetGraphics implicitly destroys all surfaces
        self.surfaces.clear();
        // The compositor reset below recomputes its budget without them.
        self.avc444_frames.clear();
        self.compositor.reset(width, height);
        self.pending_output_reset = output_size;

        // Reset frame tracking state so subsequent FrameAcknowledge PDUs
        // don't report stale queue depth from a previous stream.
        // Capability state (negotiated_caps, codec_caps) is NOT reset here:
        // per spec, capabilities are negotiated via CapabilitiesConfirm before
        // ResetGraphics, and a ResetGraphics does not re-negotiate capabilities.
        self.current_frame_id = None;

        // Reset decoder state for new stream
        if let Some(ref mut decoder) = self.h264_decoder {
            decoder.reset();
        }
        // The Progressive decoder is deliberately NOT reset here either. Its context lifetime
        // is driven by DeleteEncodingContext and DeleteSurface; MS-RDPEGFX 3.3.5.14 only
        // resizes the Graphics Output Buffer. Windows establishes a codec context once and
        // never re-sends SYNC + CONTEXT afterwards, so dropping it here makes every later
        // payload fail with MissingBlock("CONTEXT").
        // The ClearCodec decoder is deliberately NOT reset here. MS-RDPEGFX 3.3.5.14 only
        // resizes the Graphics Output Buffer; cache lifetime is driven by the stream instead,
        // through CLEARCODEC_FLAG_CACHE_RESET (2.2.4.1), which ClearCodecDecoder::decode
        // already honors by resetting the V-bar cursors. Dropping the decoder here would also
        // drop the glyph cache, so a legitimate post-reset GLYPH_HIT would fail unless the
        // server redundantly re-sent every glyph.

        if output_size.is_some() {
            debug!(width, height, "Graphics reset");
            self.handler.on_reset_graphics(width, height);
            Ok(())
        } else {
            self.handler.on_reset_graphics_rejected(width, height);
            Err(pdu_other_err!(
                "reset graphics output dimensions exceed compositor limits"
            ))
        }
    }

    fn handle_create_surface(&mut self, surface_id: u16, width: u16, height: u16, pixel_format: PixelFormat) {
        if width == 0 || height == 0 {
            warn!(surface_id, width, height, "Ignoring CreateSurface with zero dimensions");
            return;
        }

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
        self.release_avc444_frame(surface_id);
        self.compositor.create_surface(surface_id, width, height);
        self.handler.on_surface_created(&surface);
        self.surfaces.insert(surface_id, surface);
    }

    fn handle_delete_surface(&mut self, surface_id: u16) {
        self.progressive_decoder.delete_surface(surface_id);
        self.release_avc444_frame(surface_id);
        if self.surfaces.remove(&surface_id).is_some() {
            self.compositor.delete_surface(surface_id);
            debug!(surface_id, "Surface deleted");
            self.handler.on_surface_deleted(surface_id);
        } else {
            warn!(surface_id, "DeleteSurface for unknown surface");
        }
    }

    fn release_avc444_frame(&mut self, surface_id: u16) {
        if let Some(frame) = self.avc444_frames.remove(&surface_id) {
            self.compositor.release(frame.allocated_bytes());
        }
    }

    fn handle_map_surface(&mut self, surface_id: u16, origin_x: u32, origin_y: u32) {
        if let Some(surface) = self.surfaces.get_mut(&surface_id) {
            surface.is_mapped = true;
            surface.output_origin_x = origin_x;
            surface.output_origin_y = origin_y;
            self.compositor.map_surface(surface_id, origin_x, origin_y);
            debug!(surface_id, origin_x, origin_y, "Surface mapped to output");
            self.handler.on_surface_mapped(surface_id, origin_x, origin_y);
        } else {
            warn!(surface_id, "MapSurfaceToOutput for unknown surface");
        }
    }

    fn handle_map_surface_to_window(
        &mut self,
        surface_id: u16,
        window_id: u64,
        mapped: (u32, u32),
        target: (u32, u32),
    ) {
        if let Some(surface) = self.surfaces.get_mut(&surface_id) {
            surface.is_mapped = true;
            self.compositor
                .map_surface_to_window(surface_id, window_id, mapped, target);
            debug!(surface_id, window_id, ?mapped, ?target, "Surface mapped to window");
        } else {
            warn!(surface_id, "MapSurfaceToWindow for unknown surface");
        }
    }

    fn handle_map_surface_to_scaled_output(&mut self, pdu: &MapSurfaceToScaledOutputPdu) {
        if let Some(surface) = self.surfaces.get_mut(&pdu.surface_id) {
            surface.is_mapped = true;
            surface.output_origin_x = pdu.output_origin_x;
            surface.output_origin_y = pdu.output_origin_y;
            self.compositor.map_surface_scaled(
                pdu.surface_id,
                pdu.output_origin_x,
                pdu.output_origin_y,
                pdu.target_width,
                pdu.target_height,
            );
            debug!(
                surface_id = pdu.surface_id,
                origin_x = pdu.output_origin_x,
                origin_y = pdu.output_origin_y,
                target_width = pdu.target_width,
                target_height = pdu.target_height,
                "Surface mapped to scaled output"
            );
            self.handler
                .on_surface_mapped(pdu.surface_id, pdu.output_origin_x, pdu.output_origin_y);
        } else {
            warn!(
                surface_id = pdu.surface_id,
                "MapSurfaceToScaledOutput for unknown surface"
            );
        }
    }

    fn handle_wire_to_surface1(&mut self, pdu: crate::pdu::WireToSurface1Pdu) -> PduResult<()> {
        let surface = self
            .surfaces
            .get(&pdu.surface_id)
            .ok_or_else(|| pdu_other_err!("unknown surface in WireToSurface1"))?;

        // Validate rectangle ordering (left <= right, top <= bottom)
        let rect = &pdu.destination_rectangle;
        if rect.left > rect.right || rect.top > rect.bottom {
            warn!(
                left = rect.left,
                top = rect.top,
                right = rect.right,
                bottom = rect.bottom,
                "invalid destination rectangle ordering"
            );
            return Err(pdu_other_err!("invalid destination rectangle ordering"));
        }

        // Validate destination rectangle against surface bounds. The rectangle
        // uses exclusive `right`/`bottom`, so a full-surface update has
        // `right == surface.width` and `bottom == surface.height`, which is valid.
        if rect.right > surface.width || rect.bottom > surface.height {
            warn!(
                surface_id = pdu.surface_id,
                rect_right = rect.right,
                rect_bottom = rect.bottom,
                surface_width = surface.width,
                surface_height = surface.height,
                "WireToSurface1 destination rectangle exceeds surface bounds"
            );
        }

        let result = match pdu.codec_id {
            Codec1Type::Avc420 => self.decode_avc420(pdu.surface_id, &pdu.destination_rectangle, &pdu.bitmap_data),
            Codec1Type::ClearCodec => {
                self.decode_clearcodec(pdu.surface_id, &pdu.destination_rectangle, &pdu.bitmap_data)
            }
            Codec1Type::Planar => self.decode_planar(pdu.surface_id, &pdu.destination_rectangle, &pdu.bitmap_data),
            Codec1Type::RemoteFx => self.decode_remotefx(pdu.surface_id, &pdu.destination_rectangle, &pdu.bitmap_data),
            Codec1Type::Alpha => self.decode_alpha(pdu.surface_id, &pdu.destination_rectangle, &pdu.bitmap_data),
            Codec1Type::Avc444 | Codec1Type::Avc444v2 => {
                if !self
                    .h264_decoder
                    .as_ref()
                    .is_some_and(|decoder| decoder.supports_yuv420())
                {
                    trace!("No H.264 decoder returning YUV planes, forwarding AVC444 to handler");
                    self.handler.on_unhandled_pdu(&GfxPdu::WireToSurface1(pdu));
                    return Ok(());
                }
                self.decode_avc444(pdu.codec_id, pdu.surface_id, &pdu.bitmap_data)
            }
            Codec1Type::Uncompressed => {
                self.handle_uncompressed(pdu);
                return Ok(());
            }
        };
        if let Err(error) = result {
            self.drop_update(pdu.codec_id, pdu.surface_id, pdu.bitmap_data.len(), &error);
        }

        Ok(())
    }

    /// Decode RemoteFX Progressive bitmap data, applying each decoded tile to the
    /// persistent surface before delivering the same RGBA update to the handler.
    fn handle_wire_to_surface2(&mut self, pdu: WireToSurface2Pdu) -> PduResult<()> {
        let surface = self
            .surfaces
            .get(&pdu.surface_id)
            .ok_or_else(|| pdu_other_err!("unknown surface in WireToSurface2"))?;
        let (surface_width, surface_height) = (surface.width, surface.height);

        let tiles = match self.progressive_decoder.decode_bitmap(
            pdu.surface_id,
            pdu.codec_context_id,
            surface_width,
            surface_height,
            &pdu.bitmap_data,
        ) {
            Ok(tiles) => tiles,
            Err(error) => {
                self.drop_update(pdu.codec_id, pdu.surface_id, pdu.bitmap_data.len(), &error);
                return Ok(());
            }
        };

        for tile in tiles {
            let tile_left = tile.x_idx.saturating_mul(TILE_DIM);
            let tile_top = tile.y_idx.saturating_mul(TILE_DIM);
            let tile_width = surface_width.saturating_sub(tile_left).min(TILE_DIM);
            let tile_height = surface_height.saturating_sub(tile_top).min(TILE_DIM);
            if tile_width == 0 || tile_height == 0 {
                continue;
            }

            let tile_rectangle = ExclusiveRectangle {
                left: tile_left,
                top: tile_top,
                right: tile_left + tile_width,
                bottom: tile_top + tile_height,
            };
            let mut emit_update = |destination_rectangle: ExclusiveRectangle, data: Vec<u8>| {
                let width = destination_rectangle.right - destination_rectangle.left;
                let height = destination_rectangle.bottom - destination_rectangle.top;
                let update = BitmapUpdate {
                    surface_id: pdu.surface_id,
                    destination_rectangle,
                    codec_id: Codec1Type::Uncompressed,
                    data,
                    width,
                    height,
                };
                self.compositor
                    .apply_bitmap(update.surface_id, &update.destination_rectangle, &update.data);
                self.handler.on_bitmap_updated(&update);
            };

            if tile.update_rectangles.len() == 1 && tile.update_rectangles[0] == tile_rectangle {
                let data = if tile_width == TILE_DIM && tile_height == TILE_DIM {
                    tile.pixels
                } else {
                    crop_decoded_frame(
                        &tile.pixels,
                        u32::from(TILE_DIM),
                        u32::from(TILE_DIM),
                        tile_width,
                        tile_height,
                    )
                };
                emit_update(tile_rectangle, data);
                continue;
            }

            for destination_rectangle in tile.update_rectangles {
                let width = destination_rectangle.right - destination_rectangle.left;
                let height = destination_rectangle.bottom - destination_rectangle.top;
                let source_x = usize::from(destination_rectangle.left - tile_left);
                let source_y = usize::from(destination_rectangle.top - tile_top);
                let source_stride = usize::from(TILE_DIM) * TILE_BYTES_PER_PIXEL;
                let row_bytes = usize::from(width) * TILE_BYTES_PER_PIXEL;
                let mut data = Vec::with_capacity(row_bytes * usize::from(height));
                for row in 0..usize::from(height) {
                    let source_start = (source_y + row) * source_stride + source_x * TILE_BYTES_PER_PIXEL;
                    data.extend_from_slice(&tile.pixels[source_start..source_start + row_bytes]);
                }
                emit_update(destination_rectangle, data);
            }
        }

        Ok(())
    }

    // Decode failures do not end the session or discard protocol-persistent contexts and caches.
    fn drop_update(
        &mut self,
        codec: impl core::fmt::Debug,
        surface_id: u16,
        bytes: usize,
        error: &dyn core::fmt::Debug,
    ) {
        self.dropped_updates = self.dropped_updates.saturating_add(1);
        if self.dropped_updates == 1 || self.dropped_updates.is_multiple_of(64) {
            warn!(
                ?error,
                ?codec,
                surface_id,
                bytes,
                dropped_total = self.dropped_updates,
                "Dropping an undecodable surface update"
            );
        }
    }

    fn decode_avc420(&mut self, surface_id: u16, dest_rect: &ExclusiveRectangle, bitmap_data: &[u8]) -> PduResult<()> {
        let mut cursor = ReadCursor::new(bitmap_data);
        let stream = Avc420BitmapStream::decode(&mut cursor).map_err(|e| decode_err!(e))?;

        let Some(ref mut decoder) = self.h264_decoder else {
            debug!("No H.264 decoder configured, skipping AVC420 frame");
            return Ok(());
        };

        // A decoder returning YUV planes is converted with the transform MS-RDPEGFX
        // specifies, whatever matrix it would convert with itself.
        let frame = if decoder.supports_yuv420() {
            decoder.decode_yuv420(stream.data).map(|picture| picture.to_rgba())
        } else {
            decoder.decode(stream.data)
        }
        .map_err(|e| pdu_other_err!("H.264 decode", source: e))?;

        // MS-RDPEGFX 2.2.1.4.1: RDPGFX_RECT16 right/bottom are exclusive (one-past-end),
        // so dimensions are right-left / bottom-top despite the parsed type's name.
        let dest_width = dest_rect.right - dest_rect.left;
        let dest_height = dest_rect.bottom - dest_rect.top;

        // Decoded frame must be at least as large as the destination rectangle.
        // Larger is expected (macroblock alignment) and handled by cropping.
        // Smaller means the server sent mismatched dimensions.
        if frame.width() < u32::from(dest_width) || frame.height() < u32::from(dest_height) {
            return Err(pdu_other_err!(
                "decoded frame smaller than destination rectangle",
                source: crate::decode::DecoderError::msg(format!(
                    "frame_width={}, frame_height={}, dest_width={dest_width}, dest_height={dest_height}",
                    frame.width(),
                    frame.height(),
                )),
            ));
        }

        // MS-RDPEGFX 2.2.4.4 (RFX_AVC420_METABLOCK): `regionRects` lists the
        // sub-regions of the frame that actually changed. Each rectangle is in
        // the same coordinate space as the destination rectangle, and its pixels
        // come from the MATCHING (x, y) in the decoded frame -- source equals
        // destination. The destination rectangle on the PDU is only the bounding
        // box of these rects, not a copy source.
        //
        // Emitting one surface update per region rectangle (copying each from its
        // own coordinate) is what fixes issue #2042: previously the code ignored
        // `regionRects` and blitted a single bounding-box block copied from the
        // frame origin (0, 0), so partial updates drew pixels from the top-left
        // corner and overwrote the gaps between regions.
        let surface_bounds = self
            .surfaces
            .get(&surface_id)
            .map(|s| (s.width, s.height))
            .unwrap_or((dest_rect.right, dest_rect.bottom));

        // Build and emit one AVC420 surface update from a destination rectangle
        // and its pixel data. Shared by the no-regionRects fallback and the
        // per-region loop so the BitmapUpdate shape lives in one place (mirrors
        // the `emit_update` closure on the uncompressed path).
        let compositor = &mut self.compositor;
        let handler = &mut self.handler;
        let mut emit_avc420 = |destination_rectangle: ExclusiveRectangle, data: Vec<u8>| {
            let width = destination_rectangle.right - destination_rectangle.left;
            let height = destination_rectangle.bottom - destination_rectangle.top;
            let update = BitmapUpdate {
                surface_id,
                destination_rectangle,
                codec_id: Codec1Type::Avc420,
                data,
                width,
                height,
            };
            compositor.apply_bitmap(update.surface_id, &update.destination_rectangle, &update.data);
            handler.on_bitmap_updated(&update);
        };

        if stream.rectangles.is_empty() {
            // No region metadata: fall back to the single bounding-box update,
            // copying from the frame origin. This matches the pre-#2042 behavior
            // for streams that legitimately omit regionRects.
            let cropped_data = crop_decoded_frame(frame.data(), frame.width(), frame.height(), dest_width, dest_height);
            emit_avc420(dest_rect.clone(), cropped_data);
            return Ok(());
        }

        for region in &stream.rectangles {
            // Clip the region to the surface and the decoded frame so a
            // malformed or over-large rect cannot read out of bounds or paint
            // outside the surface.
            let frame_w = u16::try_from(frame.width()).unwrap_or(u16::MAX);
            let frame_h = u16::try_from(frame.height()).unwrap_or(u16::MAX);
            let clip_right = region.right.min(surface_bounds.0).min(frame_w);
            let clip_bottom = region.bottom.min(surface_bounds.1).min(frame_h);
            if region.left >= clip_right || region.top >= clip_bottom {
                // A region that clips away to nothing is dropped here. Log it:
                // a silent skip would leave a permanently stale screen area with
                // no trace if a server's coordinate convention ever disagreed
                // with the surface/frame bounds -- the same undebuggable class of
                // rendering defect this change fixes.
                warn!(
                    surface_id,
                    region_left = region.left,
                    region_top = region.top,
                    region_right = region.right,
                    region_bottom = region.bottom,
                    clip_right,
                    clip_bottom,
                    surface_width = surface_bounds.0,
                    surface_height = surface_bounds.1,
                    frame_width = frame_w,
                    frame_height = frame_h,
                    "AVC420 regionRect empty after clipping to surface/frame bounds -- skipped"
                );
                continue;
            }
            let clipped = ExclusiveRectangle {
                left: region.left,
                top: region.top,
                right: clip_right,
                bottom: clip_bottom,
            };

            // Copy pixels (x, y) -> (x, y): the source in the decoded frame is the
            // region's own top-left, not the frame origin.
            let data = copy_frame_region(frame.data(), frame.width(), &clipped);
            emit_avc420(clipped, data);
        }
        Ok(())
    }

    /// Decode an AVC444 or AVC444v2 update ([MS-RDPEGFX] 2.2.4.5 and 2.2.4.6).
    ///
    /// Both sub-streams go through the one H.264 decoder, in order, whatever their
    /// rectangles, so the decoder sees the whole stream. Each view then updates the
    /// surface's combined picture within its own `regionRects`, and those rectangles
    /// are painted.
    ///
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
    fn decode_avc444(&mut self, codec_id: Codec1Type, surface_id: u16, bitmap_data: &[u8]) -> PduResult<()> {
        let mut cursor = ReadCursor::new(bitmap_data);
        let stream = Avc444BitmapStream::decode(&mut cursor).map_err(|e| decode_err!(e))?;
        // The caller forwards AVC444 to the handler unless a YUV decoder is present.
        let decoder = self
            .h264_decoder
            .as_mut()
            .ok_or_else(|| pdu_other_err!("no H.264 decoder for AVC444"))?;
        let layout = if codec_id == Codec1Type::Avc444 {
            Avc444Layout::V1
        } else {
            Avc444Layout::V2
        };
        let (width, height) = self
            .surfaces
            .get(&surface_id)
            .map(|surface| (surface.width, surface.height))
            .ok_or_else(|| pdu_other_err!("unknown surface in WireToSurface1"))?;

        trace!(surface_id, ?codec_id, encoding = ?stream.encoding, "AVC444 update");
        let (main, auxiliary) = match stream.encoding {
            Encoding::LUMA => (Some(&stream.stream1), None),
            Encoding::CHROMA => (None, Some(&stream.stream1)),
            _ => (Some(&stream.stream1), stream.stream2.as_ref()),
        };
        // UNTESTED: that the decoder returns each view for its own sub-stream. Both sub-streams
        // are one H.264 stream (MS-RDPEGFX 2.2.4.5), and a decoder that delays pictures would
        // pair a main view with the previous auxiliary view; the tests use a decoder that cannot
        // delay. Check a Windows server's AVC444 output with the decoder in use when changing this.
        let main_view = main
            .map(|main| decoder.decode_yuv420(main.data))
            .transpose()
            .map_err(|e| pdu_other_err!("H.264 decode", source: e))?;
        let auxiliary_view = auxiliary
            .map(|auxiliary| decoder.decode_yuv420(auxiliary.data))
            .transpose()
            .map_err(|e| pdu_other_err!("H.264 decode", source: e))?;

        // The combined picture lives as long as the surface, so it is charged to the
        // compositor's budget, and only for a surface the compositor accepted, whose
        // dimensions it has bounded.
        let frame = match self.avc444_frames.entry(surface_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                if !self
                    .compositor
                    .reserve_for_surface(surface_id, Yuv444Frame::allocation_size(width, height))
                {
                    return Err(pdu_other_err!("AVC444 picture exceeds the compositor budget"));
                }
                entry.insert(Yuv444Frame::new(width, height))
            }
        };
        if let (Some(main), Some(view)) = (main, &main_view) {
            frame.apply_main_view(view, &main.rectangles);
        }
        if let (Some(auxiliary), Some(view)) = (auxiliary, &auxiliary_view) {
            frame.apply_auxiliary_view(layout, view, &auxiliary.rectangles);
        }

        let mut painted = BTreeSet::new();
        for rect in main
            .into_iter()
            .chain(auxiliary)
            .flat_map(|stream| stream.rectangles.iter())
        {
            let clipped = ExclusiveRectangle {
                left: rect.left,
                top: rect.top,
                right: rect.right.min(width),
                bottom: rect.bottom.min(height),
            };
            let fresh = clipped.left < clipped.right
                && clipped.top < clipped.bottom
                && painted.insert((clipped.left, clipped.top, clipped.right, clipped.bottom));
            if !fresh {
                continue;
            }
            let update = BitmapUpdate {
                surface_id,
                data: frame.to_rgba(&clipped),
                codec_id,
                width: clipped.right - clipped.left,
                height: clipped.bottom - clipped.top,
                destination_rectangle: clipped,
            };
            self.compositor
                .apply_bitmap(surface_id, &update.destination_rectangle, &update.data);
            self.handler.on_bitmap_updated(&update);
        }
        Ok(())
    }

    fn decode_clearcodec(
        &mut self,
        surface_id: u16,
        dest_rect: &ExclusiveRectangle,
        bitmap_data: &[u8],
    ) -> PduResult<()> {
        // MS-RDPEGFX 2.2.1.4.1: see decode_avc420 above for wire-format note.
        let dest_width = dest_rect.right - dest_rect.left;
        let dest_height = dest_rect.bottom - dest_rect.top;

        let bgra = self
            .clearcodec_decoder
            .get_or_insert_with(ClearCodecDecoder::new)
            .decode(bitmap_data, dest_width, dest_height)
            .map_err(|e| pdu_other_err!("ClearCodec decode", source: e))?;

        // ClearCodec outputs BGRA; convert to RGBA for the uniform BitmapUpdate format
        let rgba = convert_bgra_to_rgba(&bgra);

        let update = BitmapUpdate {
            surface_id,
            destination_rectangle: dest_rect.clone(),
            codec_id: Codec1Type::ClearCodec,
            data: rgba,
            width: dest_width,
            height: dest_height,
        };

        self.compositor.apply_bitmap(surface_id, dest_rect, &update.data);
        self.handler.on_bitmap_updated(&update);
        Ok(())
    }

    /// Decode an RDP 6.0 Planar bitmap ([MS-RDPEGFX] `RDPGFX_CODECID_PLANAR`, 0x000A).
    ///
    /// The payload is an `RDP6_BITMAP_STREAM` ([MS-RDPEGDI] 2.2.2.5.1), the same
    /// structure the fast-path bitmap route already decodes, so this reuses
    /// `ironrdp-graphics`' decoder rather than adding a second implementation.
    fn decode_planar(&mut self, surface_id: u16, dest_rect: &ExclusiveRectangle, bitmap_data: &[u8]) -> PduResult<()> {
        // MS-RDPEGFX 2.2.2.1: destRect gives both the target point and "the
        // dimensions (width and height) of the bitmap data". It is only a bounding
        // rectangle for the AVC codecs, so for Planar these are exact.
        let dest_width = dest_rect.right - dest_rect.left;
        let dest_height = dest_rect.bottom - dest_rect.top;

        let mut rgb24 = Vec::new();
        self.planar_decoder
            .decode_bitmap_stream_to_rgb24(
                bitmap_data,
                &mut rgb24,
                usize::from(dest_width),
                usize::from(dest_height),
            )
            .map_err(|e| pdu_other_err!("Planar decode", source: e))?;

        // The decoder emits RGB24 top-down row-major, which is the order surfaces
        // are stored in, so no vertical flip is needed.
        //
        // Opacity is not carried here. An `RDP6_BITMAP_STREAM` can include an alpha
        // plane (FormatHeader NA bit clear), but EGFX conveys per-pixel opacity in a
        // separate `RDPGFX_CODECID_ALPHA` (0x000C) command carrying an
        // `ALPHACODEC_BITMAP_STREAM` ([MS-RDPEGFX] 2.2.4.3), so a Planar command
        // contributes color only and the pixels are opaque.
        let rgba = convert_rgb24_to_rgba(&rgb24);

        let update = BitmapUpdate {
            surface_id,
            destination_rectangle: dest_rect.clone(),
            codec_id: Codec1Type::Planar,
            data: rgba,
            width: dest_width,
            height: dest_height,
        };

        self.compositor.apply_bitmap(surface_id, dest_rect, &update.data);
        self.handler.on_bitmap_updated(&update);
        Ok(())
    }

    /// Decode a RemoteFX (`RDPGFX_CODECID_CAVIDEO`, 0x0003) command.
    ///
    /// The payload is a RemoteFX message ([MS-RDPRFX] [2.2.2][rfx-2.2.2]). Its tile positions and
    /// region rectangles are relative to the top-left corner of `destRect`
    /// ([MS-RDPEGFX] [2.2.2.1]); only the parts of each tile inside both a region rectangle and
    /// `destRect` are painted, and only those tiles are decoded.
    ///
    /// [MS-RDPRFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdprfx/62495a4a-a495-46ea-b459-5cde04c44549
    /// [rfx-2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdprfx/edd6c368-4f39-420a-bf14-a0e83c654ab7
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
    /// [2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/fb919fce-cc97-4d2b-8cf5-a737a00ef1a6
    fn decode_remotefx(
        &mut self,
        surface_id: u16,
        dest_rect: &ExclusiveRectangle,
        bitmap_data: &[u8],
    ) -> PduResult<()> {
        let message = self
            .rfx_decoder
            .parse(bitmap_data)
            .map_err(|e| pdu_other_err!("RemoteFX decode", source: e))?;

        let Some((surface_width, surface_height)) = self.surfaces.get(&surface_id).map(|s| (s.width, s.height)) else {
            return Ok(());
        };
        // Nothing is painted outside the command destination or the surface.
        let bound_right = u32::from(dest_rect.right.min(surface_width));
        let bound_bottom = u32::from(dest_rect.bottom.min(surface_height));
        let clip = |left: u32, top: u32, width: u32, height: u32| {
            let right = left.saturating_add(width).min(bound_right);
            let bottom = top.saturating_add(height).min(bound_bottom);
            // INVARIANT: `left < right <= bound_right` and `top < bottom <= bound_bottom`, and both
            // bounds are `u16` values, so every edge fits a `u16` and `right - 1`, `bottom - 1`
            // cannot underflow.
            (left < right && top < bottom).then(|| InclusiveRectangle {
                left: u16::try_from(left).expect("left < right fits u16"),
                top: u16::try_from(top).expect("top < bottom fits u16"),
                right: u16::try_from(right - 1).expect("right is at most a u16 bound"),
                bottom: u16::try_from(bottom - 1).expect("bottom is at most a u16 bound"),
            })
        };

        let origin_x = u32::from(dest_rect.left);
        let origin_y = u32::from(dest_rect.top);

        // Each frame's region is normalized into disjoint rectangles, so a pixel covered by several
        // (or repeated) region rectangles is painted once, and each tile is intersected with the
        // normalized region. Both steps scan the region, so their cost is charged against one
        // bound, all of it before the first tile is decoded.
        let mut work = 0usize;
        let mut charge = |units: usize| {
            work = work.saturating_add(units);
            if MAX_RFX_CLIPPING_WORK < work {
                Err(pdu_other_err!("RemoteFX message needs too much region clipping work"))
            } else {
                Ok(())
            }
        };
        let mut regions = Vec::with_capacity(message.frames.len());
        for frame in &message.frames {
            let mut region = Region::new();
            let rectangles: Vec<InclusiveRectangle> = if frame.rectangles.is_empty() {
                // A region without rectangles makes the whole frame valid (MS-RDPRFX 2.2.2.3.3);
                // in EGFX the frame is the destination.
                clip(origin_x, origin_y, u32::MAX, u32::MAX).into_iter().collect()
            } else {
                frame
                    .rectangles
                    .iter()
                    .filter_map(|r| {
                        clip(
                            origin_x + u32::from(r.x),
                            origin_y + u32::from(r.y),
                            u32::from(r.width),
                            u32::from(r.height),
                        )
                    })
                    .collect()
            };
            for rectangle in rectangles {
                charge(region.rectangles.len().saturating_add(1))?;
                region.union_rectangle(rectangle);
            }
            charge(
                frame
                    .tile_set
                    .tiles
                    .len()
                    .saturating_mul(region.rectangles.len().max(1)),
            )?;
            regions.push(region);
        }

        let tile_size = u32::from(rfx::TILE_SIZE);
        for (frame, region) in message.frames.iter().zip(&regions) {
            let tile_set = &frame.tile_set;
            // A tile position listed twice in one tileset is decoded once: the last occurrence is
            // the one that would remain painted.
            let mut seen = BTreeSet::new();
            for tile in tile_set.tiles.iter().rev() {
                if !seen.insert((tile.x, tile.y)) {
                    continue;
                }
                let tile_left = origin_x + u32::from(tile.x) * tile_size;
                let tile_top = origin_y + u32::from(tile.y) * tile_size;
                let Some(tile_rect) = clip(tile_left, tile_top, tile_size, tile_size) else {
                    continue;
                };
                let visible = region.intersect_rectangle(&tile_rect).rectangles;
                if visible.is_empty() {
                    continue;
                }

                let pixels = self
                    .rfx_tiles
                    .decode(tile, &tile_set.quants, tile_set.entropy_algorithm)
                    .map_err(|e| pdu_other_err!("RemoteFX tile decode", source: e))?;

                for rectangle in visible {
                    let destination_rectangle = ExclusiveRectangle {
                        left: rectangle.left,
                        top: rectangle.top,
                        right: rectangle.right + 1,
                        bottom: rectangle.bottom + 1,
                    };
                    // INVARIANT: `destination_rectangle` lies within the 64x64 tile, so each edge is
                    // at least the tile origin and at most 64 past it: the subtractions cannot
                    // underflow and the results fit a `u16`.
                    let local_edge = |edge: u16, origin: u32| {
                        u16::try_from(u32::from(edge) - origin).expect("tile-local edge is at most 64")
                    };
                    let local = ExclusiveRectangle {
                        left: local_edge(destination_rectangle.left, tile_left),
                        top: local_edge(destination_rectangle.top, tile_top),
                        right: local_edge(destination_rectangle.right, tile_left),
                        bottom: local_edge(destination_rectangle.bottom, tile_top),
                    };
                    let update = BitmapUpdate {
                        surface_id,
                        width: destination_rectangle.right - destination_rectangle.left,
                        height: destination_rectangle.bottom - destination_rectangle.top,
                        destination_rectangle,
                        codec_id: Codec1Type::RemoteFx,
                        data: copy_frame_region(pixels, tile_size, &local),
                    };
                    self.compositor
                        .apply_bitmap(surface_id, &update.destination_rectangle, &update.data);
                    self.handler.on_bitmap_updated(&update);
                }
            }
        }

        Ok(())
    }

    /// Decode an Alpha codec (`RDPGFX_CODECID_ALPHA`, 0x000C) command into the surface alpha
    /// channel, leaving its color channels as an earlier command painted them
    /// ([MS-RDPEGFX] [3.3.5.1]).
    ///
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
    /// [3.3.5.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/3fdfcd8d-ec4f-4b7a-8e0d-0385a840ec81
    fn decode_alpha(&mut self, surface_id: u16, dest_rect: &ExclusiveRectangle, bitmap_data: &[u8]) -> PduResult<()> {
        let composited = self
            .compositor
            .apply_alpha(surface_id, dest_rect, bitmap_data)
            .map_err(|e| pdu_other_err!("Alpha decode", source: e))?;

        // The handler receives the whole clipped destination after every Alpha command,
        // a surface-sized copy for a full-surface command. Handlers compose `BitmapUpdate`s as
        // complete RGBA pixels, which an alpha-only update would break; if that cost matters,
        // change the handler contract rather than shrinking this update.
        if let Some((destination_rectangle, data)) = composited {
            let update = BitmapUpdate {
                surface_id,
                width: destination_rectangle.right - destination_rectangle.left,
                height: destination_rectangle.bottom - destination_rectangle.top,
                destination_rectangle,
                codec_id: Codec1Type::Alpha,
                data,
            };
            self.handler.on_bitmap_updated(&update);
        }
        Ok(())
    }

    /// Build the `CacheImportOffer` sent right after `CapabilitiesConfirm` ([MS-RDPEGFX] [3.3.5.16]),
    /// or `None` without a persistent cache, a confirmed capability set, or stored entries.
    ///
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
    /// [3.3.5.16]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/adf5cc51-d0b1-4533-987f-16296bbe6966
    #[expect(clippy::as_conversions, reason = "Box<GfxPdu> to Box<dyn DvcEncode> coercion")]
    fn cache_import_offer(&mut self) -> Option<DvcMessage> {
        let store = self.persistent_cache.as_mut()?;
        // The cache limits depend on the confirmed flags, so without them nothing is offered.
        self.negotiated_caps.as_ref()?;
        if self.cache_import_attempted {
            return None;
        }
        self.cache_import_attempted = true;
        let (max_slots, max_bytes) = bitmap_cache_limits(&self.codec_caps);

        let stored = match store.entries() {
            Ok(entries) => entries,
            Err(error) => {
                warn!(%error, "Failed to list the persistent bitmap cache; not offering it");
                return None;
            }
        };

        let max_entries = MAX_CACHE_IMPORT_ENTRIES.min(usize::from(max_slots));
        let mut seen = BTreeSet::new();
        let mut bytes = 0usize;
        let mut cache_entries = Vec::new();
        for entry in stored {
            if cache_entries.len() == max_entries {
                break;
            }
            let len = usize::try_from(entry.bitmap_len).unwrap_or(usize::MAX);
            if len > MAX_CACHE_IMPORT_ENTRY_BYTES {
                continue;
            }
            // The server charges each accepted entry against its model of the client's cache, so
            // an offer larger than the cache would be refused or overflow it.
            let Some(total) = bytes.checked_add(len).filter(|total| *total <= max_bytes) else {
                continue;
            };
            if len == 0 || !seen.insert(entry.cache_key) {
                continue;
            }
            bytes = total;
            cache_entries.push(entry);
        }

        if cache_entries.is_empty() {
            // No reply follows, so release what listing opened now.
            store.import_finished();
            return None;
        }

        debug!(
            entries = cache_entries.len(),
            bytes, "Offering the persistent bitmap cache"
        );
        self.offered_cache_keys = Some(cache_entries.iter().map(|entry| entry.cache_key).collect());
        let offer = GfxPdu::CacheImportOffer(CacheImportOfferPdu { cache_entries });
        Some(Box::new(offer) as DvcMessage)
    }

    /// Load the entries the server accepted into the slots it assigned them
    /// ([MS-RDPEGFX] [3.3.5.17]).
    ///
    /// The reply's Nth slot belongs to the Nth offered entry. An entry that cannot be loaded leaves
    /// its slot empty, so a later paste from it draws nothing, as for any other missing slot.
    ///
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
    /// [3.3.5.17]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/df29d040-f74b-4002-92ed-f0903b8402c3
    fn handle_cache_import_reply(&mut self, pdu: &CacheImportReplyPdu) {
        let Some(offered) = self.offered_cache_keys.take() else {
            warn!("CacheImportReply without a pending CacheImportOffer; ignoring it");
            return;
        };
        let Some(store) = self.persistent_cache.as_mut() else {
            return;
        };
        if offered.len() < pdu.cache_slots.len() {
            warn!(
                offered = offered.len(),
                imported = pdu.cache_slots.len(),
                "CacheImportReply names more entries than were offered; ignoring the excess"
            );
        }

        // An offer only goes out once capabilities are confirmed, so these are the confirmed limits.
        let (max_slots, _) = bitmap_cache_limits(&self.codec_caps);
        let mut imported = 0usize;
        let mut invalid_slots = 0usize;
        for (&cache_key, &cache_slot) in offered.iter().zip(&pdu.cache_slots) {
            // Cache slots are one-based (3.3.1.4).
            if !(1..=max_slots).contains(&cache_slot) {
                invalid_slots += 1;
                continue;
            }
            match store.load(cache_key) {
                Ok(Some(entry)) if entry.cache_key == cache_key => {
                    if self
                        .compositor
                        .import_cache_entry(cache_slot, cache_key, entry.width, entry.height, entry.data)
                    {
                        imported += 1;
                    } else {
                        trace!(cache_key, cache_slot, "Persistent cache entry rejected");
                    }
                }
                Ok(_) => trace!(cache_key, "Persistent cache entry is gone"),
                Err(error) => {
                    warn!(%error, "Failed to read the persistent bitmap cache; skipping the rest");
                    break;
                }
            }
        }

        store.import_finished();
        if invalid_slots > 0 {
            warn!(
                invalid_slots,
                max_slots, "CacheImportReply assigned cache slots outside the bitmap cache"
            );
        }
        debug!(
            offered = offered.len(),
            accepted = pdu.cache_slots.len(),
            imported,
            "Persistent bitmap cache imported"
        );
    }

    /// Save the bitmap cache to the persistent store, most recently used first.
    ///
    /// An empty cache is not saved, so a session that ends before the server cached anything
    /// keeps the entries a previous session stored.
    fn save_persistent_cache(&mut self) {
        let Some(store) = self.persistent_cache.as_mut() else {
            return;
        };

        // Nothing past what one offer can carry, larger than an offered entry may be, or past the
        // full cache's capacity, can be offered again, so it is not worth storing. As in the
        // offer, an entry that does not fit the remaining capacity is skipped so smaller ones
        // still can. A key cached in several slots is stored once, from its most recently used
        // slot.
        let mut seen = BTreeSet::new();
        let mut bytes = 0usize;
        let mut entries = Vec::new();
        for entry in self.compositor.cache_entries() {
            if entries.len() == MAX_CACHE_IMPORT_ENTRIES {
                break;
            }
            if entry.data.is_empty() || entry.data.len() > MAX_CACHE_IMPORT_ENTRY_BYTES || !seen.insert(entry.cache_key)
            {
                continue;
            }
            let Some(total) = bytes
                .checked_add(entry.data.len())
                .filter(|total| *total <= CACHE_BYTES)
            else {
                continue;
            };
            bytes = total;
            entries.push(entry);
        }
        if entries.is_empty() {
            return;
        }

        match store.save(&entries) {
            Ok(()) => debug!(entries = entries.len(), "Saved the persistent bitmap cache"),
            Err(error) => warn!(%error, "Failed to save the persistent bitmap cache"),
        }
    }

    fn handle_uncompressed(&mut self, pdu: crate::pdu::WireToSurface1Pdu) {
        // MS-RDPEGFX 2.2.1.4.1: see decode_avc420 above for wire-format note.
        let dest_width = pdu.destination_rectangle.right - pdu.destination_rectangle.left;
        let dest_height = pdu.destination_rectangle.bottom - pdu.destination_rectangle.top;

        // Convert wire-format pixels to RGBA.
        // BitmapUpdate.data is always RGBA8888 regardless of codec -- this is
        // the convention so that handlers get a uniform pixel format.
        // Uncompressed wire format is 32-bit LE (0xAARRGGBB → bytes [B, G, R, A]).
        let rgba_data = convert_uncompressed_to_rgba(&pdu.bitmap_data);

        let update = BitmapUpdate {
            surface_id: pdu.surface_id,
            destination_rectangle: pdu.destination_rectangle,
            codec_id: Codec1Type::Uncompressed,
            data: rgba_data,
            width: dest_width,
            height: dest_height,
        };

        self.compositor
            .apply_bitmap(update.surface_id, &update.destination_rectangle, &update.data);
        self.handler.on_bitmap_updated(&update);
    }

    #[expect(clippy::as_conversions, reason = "Box<GfxPdu> to Box<dyn DvcEncode> coercion")]
    fn handle_end_frame(&mut self, frame_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.total_frames_decoded = self.total_frames_decoded.wrapping_add(1);
        self.current_frame_id = None;

        self.progressive_decoder.end_frame();

        // Commit the frame's compositor deltas so `drain_output` can surface them.
        self.compositor.end_frame();

        self.handler.on_frame_complete(frame_id);

        // Per [3.3.5.12]: client MUST send FrameAcknowledge after EndFrame.
        // QueueDepth counts queued bytes, not decoded/queued frames. This client
        // does not measure queued bytes, so report Unavailable (not Suspend).
        let ack = GfxPdu::FrameAcknowledge(FrameAcknowledgePdu {
            queue_depth: QueueDepth::Unavailable,
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
        if let Some(decoder) = self.h264_decoder.as_mut()
            && !decoder.wait_until_ready()
        {
            warn!("The H.264 decoder is unavailable; advertising no AVC capability set");
            self.h264_decoder = None;
        }

        // Keep only the sets this client can honor: AVC420 needs a decoder, and AVC444
        // needs one returning the YUV planes the two views are combined from.
        let avc420 = self.h264_decoder.is_some();
        let avc444 = self
            .h264_decoder
            .as_ref()
            .is_some_and(|decoder| decoder.supports_yuv420());
        let mut caps: Vec<CapabilitySet> = self
            .handler
            .capabilities()
            .into_iter()
            .filter(|cap| {
                let codecs = CodecCapabilities::from_capability_set(cap);
                (avc420 || !codecs.avc420) && (avc444 || !codecs.avc444)
            })
            .collect();
        debug!(avc420, avc444, "Advertising EGFX capabilities");
        if caps.is_empty() {
            debug!(
                avc420,
                avc444, "No advertisable EGFX capability set; falling back to V8"
            );
            caps.push(CapabilitySet::V8 {
                flags: CapabilitiesV8Flags::SMALL_CACHE,
            });
        }

        let pdu = GfxPdu::CapabilitiesAdvertise(CapabilitiesAdvertisePdu::from_typed(&caps));

        #[expect(clippy::as_conversions, reason = "Box<GfxPdu> to Box<dyn DvcEncode> coercion")]
        Ok(vec![Box::new(pdu) as DvcMessage])
    }

    fn close(&mut self, _channel_id: u32) {
        self.save_persistent_cache();
        self.state = ClientState::Closed;
        self.handler.on_close();
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        if self.state == ClientState::Closed {
            return Err(pdu_other_err!("graphics channel is closed; recreate the channel"));
        }
        self.process_messages(payload).inspect_err(|_| {
            if self.state == ClientState::Resetting {
                self.state = ClientState::Closed;
            }
        })
    }
}

impl DvcClientProcessor for GraphicsPipelineClient {}

/// Cache slot count and byte capacity of the bitmap cache under the confirmed capabilities.
fn bitmap_cache_limits(caps: &CodecCapabilities) -> (u16, usize) {
    if caps.small_cache {
        (SMALL_CACHE_SLOTS, SMALL_CACHE_BYTES)
    } else {
        (CACHE_SLOTS, CACHE_BYTES)
    }
}

// ============================================================================
// Frame Cropping
// ============================================================================

/// Convert BGRA pixel data to RGBA8888
///
/// ClearCodec produces BGRA output per [MS-RDPEGFX 2.2.4.1]. Reorder to
/// [R, G, B, A] for the uniform `BitmapUpdate` pixel format.
/// Widen tightly-packed RGB24 to RGBA, marking every pixel opaque.
///
/// The RDP 6.0 Planar decoder emits three bytes per pixel; `BitmapUpdate` carries
/// four. See `decode_planar` for why the alpha byte is synthesized rather than
/// taken from the stream.
fn convert_rgb24_to_rgba(src: &[u8]) -> Vec<u8> {
    debug_assert!(src.len().is_multiple_of(3), "RGB24 input length not aligned to 3 bytes");
    let mut dst = Vec::with_capacity(src.len() / 3 * 4);
    for pixel in src.chunks_exact(3) {
        dst.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 0xFF]);
    }
    dst
}

fn convert_bgra_to_rgba(src: &[u8]) -> Vec<u8> {
    debug_assert!(src.len().is_multiple_of(4), "BGRA input length not aligned to 4 bytes");
    let mut dst = Vec::with_capacity(src.len());
    for pixel in src.chunks_exact(4) {
        dst.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    dst
}

/// Convert uncompressed 32bpp little-endian pixels to RGBA8888
///
/// The wire format for uncompressed graphics is 0xAARRGGBB in a 32-bit
/// little-endian word, which corresponds to bytes [B, G, R, A]. This
/// reorders to [R, G, B, 0xFF], treating all pixels as fully opaque.
fn convert_uncompressed_to_rgba(src: &[u8]) -> Vec<u8> {
    let mut dst = Vec::with_capacity(src.len());
    for pixel in src.chunks_exact(4) {
        let b = pixel[0];
        let g = pixel[1];
        let r = pixel[2];
        dst.extend_from_slice(&[r, g, b, 0xFF]);
    }
    dst
}

/// Crop a decoded RGBA frame to target dimensions
///
/// H.264 frames are macroblock-aligned (16x16), so decoded frames
/// may be larger than the destination rectangle. This function
/// extracts the top-left region matching the target size.
///
/// This is the frame-origin special case of [`copy_frame_region`] (a region
/// anchored at `(0, 0)`); it delegates the row copy there so there is a single
/// RGBA row-copy implementation, keeping only the zero-dimension guard, the
/// equal-dimensions fast path, and the truncation warning here.
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

    // Clamp the origin region to the decoded frame so `copy_frame_region` reads
    // at most `src_stride` bytes per row -- the same clamp the old inline loop
    // did with `dst_stride.min(src_stride)`.
    let right = u16::try_from(tw.min(decoded_width)).unwrap_or(u16::MAX);
    let bottom = u16::try_from(th.min(decoded_height)).unwrap_or(u16::MAX);
    let origin_region = ExclusiveRectangle {
        left: 0,
        top: 0,
        right,
        bottom,
    };
    let cropped = copy_frame_region(data, decoded_width, &origin_region);

    let dst_stride = tw.saturating_mul(4);
    let rows = th.min(decoded_height);
    #[expect(clippy::as_conversions, reason = "dst_stride * rows bounded by frame dimensions")]
    let expected_len = (dst_stride as usize).saturating_mul(rows as usize);
    if cropped.len() < expected_len {
        tracing::warn!(
            expected = expected_len,
            actual = cropped.len(),
            "Decoded frame data truncated during crop"
        );
    }

    cropped
}

/// Copy one region out of a decoded RGBA frame, taking pixels from the SAME
/// coordinates the region occupies (source `(x, y)` -> destination `(x, y)`).
///
/// This is the per-region-rectangle counterpart to [`crop_decoded_frame`], which
/// only ever copies the top-left block from the frame origin. AVC420 partial
/// updates ([MS-RDPEGFX] 2.2.4.4 `regionRects`) require copying each region from
/// its own position in the frame; see `decode_avc420` and issue #2042.
///
/// `region` must already be clipped to the frame bounds by the caller.
fn copy_frame_region(frame_data: &[u8], frame_width: u32, region: &ExclusiveRectangle) -> Vec<u8> {
    const BYTES_PER_PIXEL: usize = 4;

    let src_stride = usize::try_from(frame_width)
        .unwrap_or(usize::MAX)
        .saturating_mul(BYTES_PER_PIXEL);
    let region_width = usize::from(region.right - region.left);
    let region_height = usize::from(region.bottom - region.top);
    let row_bytes = region_width.saturating_mul(BYTES_PER_PIXEL);
    let left_off = usize::from(region.left).saturating_mul(BYTES_PER_PIXEL);

    let mut out = Vec::with_capacity(row_bytes.saturating_mul(region_height));
    for row in 0..region_height {
        let src_row = usize::from(region.top).saturating_add(row);
        let src_start = src_row.saturating_mul(src_stride).saturating_add(left_off);
        let src_end = src_start.saturating_add(row_bytes);
        if src_end <= frame_data.len() {
            out.extend_from_slice(&frame_data[src_start..src_end]);
        } else {
            // Defensive: the caller clips to frame bounds, so this should not
            // happen, but never read out of bounds if it does.
            break;
        }
    }
    out
}

/// Unit tests that require access to private fields (state, surfaces, frame tracking).
/// Integration tests exercising the public DVC API are in ironrdp-testsuite-core/tests/egfx/client.rs.
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    struct TestHandler;
    impl GraphicsPipelineHandler for TestHandler {
        fn on_capabilities_confirmed(&mut self, _caps: &CapabilitySet) {}
        fn on_reset_graphics(&mut self, _width: u32, _height: u32) {}
        fn on_surface_created(&mut self, _surface: &Surface) {}
        fn on_surface_deleted(&mut self, _surface_id: u16) {}
        fn on_surface_mapped(&mut self, _surface_id: u16, _x: u32, _y: u32) {}
        fn on_bitmap_updated(&mut self, _update: &BitmapUpdate) {}
        fn on_frame_complete(&mut self, _frame_id: u32) {}
        fn on_close(&mut self) {}
        fn on_unhandled_pdu(&mut self, _pdu: &GfxPdu) {}
    }

    /// Captures bitmap updates and unhandled PDUs so a test can tell decode from
    /// fallthrough.
    /// `(codec_id, width, height, rgba)` extracted from each update, since
    /// `BitmapUpdate` is not `Clone` and does not need to be.
    type CapturedUpdate = (Codec1Type, u16, u16, Vec<u8>);
    type SurfaceMapping = (u16, u32, u32);
    type ScaledOutputMapping = (u16, u32, u32, u32, u32);

    struct CapturingHandler {
        updates: Arc<Mutex<Vec<CapturedUpdate>>>,
        unhandled: Arc<Mutex<usize>>,
    }
    impl GraphicsPipelineHandler for CapturingHandler {
        fn on_capabilities_confirmed(&mut self, _caps: &CapabilitySet) {}
        fn on_reset_graphics(&mut self, _width: u32, _height: u32) {}
        fn on_surface_created(&mut self, _surface: &Surface) {}
        fn on_surface_deleted(&mut self, _surface_id: u16) {}
        fn on_surface_mapped(&mut self, _surface_id: u16, _x: u32, _y: u32) {}
        fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
            self.updates.lock().expect("updates lock").push((
                update.codec_id,
                update.width,
                update.height,
                update.data.clone(),
            ));
        }
        fn on_frame_complete(&mut self, _frame_id: u32) {}
        fn on_close(&mut self) {}
        fn on_unhandled_pdu(&mut self, _pdu: &GfxPdu) {
            *self.unhandled.lock().expect("unhandled lock") += 1;
        }
    }

    struct ScaledOutputHandler {
        mappings: Arc<Mutex<Vec<ScaledOutputMapping>>>,
        surface_mappings: Arc<Mutex<Vec<SurfaceMapping>>>,
    }

    impl GraphicsPipelineHandler for ScaledOutputHandler {
        fn on_capabilities_confirmed(&mut self, _caps: &CapabilitySet) {}
        fn on_reset_graphics(&mut self, _width: u32, _height: u32) {}
        fn on_surface_created(&mut self, _surface: &Surface) {}
        fn on_surface_deleted(&mut self, _surface_id: u16) {}
        fn on_surface_mapped(&mut self, surface_id: u16, x: u32, y: u32) {
            self.surface_mappings
                .lock()
                .expect("surface mappings lock")
                .push((surface_id, x, y));
        }
        fn on_bitmap_updated(&mut self, _update: &BitmapUpdate) {}
        fn on_frame_complete(&mut self, _frame_id: u32) {}
        fn on_close(&mut self) {}
        fn on_unhandled_pdu(&mut self, _pdu: &GfxPdu) {}

        fn on_map_surface_to_scaled_output(&mut self, pdu: &MapSurfaceToScaledOutputPdu) {
            self.mappings.lock().expect("mappings lock").push((
                pdu.surface_id,
                pdu.output_origin_x,
                pdu.output_origin_y,
                pdu.target_width,
                pdu.target_height,
            ));
        }
    }

    #[test]
    fn map_surface_to_scaled_output_dispatches_to_compositor_and_handler() {
        let mappings = Arc::new(Mutex::new(Vec::new()));
        let surface_mappings = Arc::new(Mutex::new(Vec::new()));
        let mut client = GraphicsPipelineClient::new(
            Box::new(ScaledOutputHandler {
                mappings: Arc::clone(&mappings),
                surface_mappings: Arc::clone(&surface_mappings),
            }),
            None,
        );
        client
            .handle_pdu(GfxPdu::ResetGraphics(crate::pdu::ResetGraphicsPdu {
                width: 12,
                height: 12,
                monitors: vec![],
            }))
            .unwrap();
        client
            .handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
                surface_id: 1,
                width: 2,
                height: 2,
                pixel_format: PixelFormat::XRgb,
            }))
            .unwrap();
        client
            .handle_pdu(GfxPdu::MapSurfaceToScaledOutput(MapSurfaceToScaledOutputPdu {
                surface_id: 1,
                output_origin_x: 3,
                output_origin_y: 4,
                target_width: 4,
                target_height: 4,
            }))
            .unwrap();

        assert!(client.drain_output().is_empty());
        client
            .handle_pdu(GfxPdu::EndFrame(crate::pdu::EndFramePdu { frame_id: 1 }))
            .unwrap();

        assert_eq!(*mappings.lock().expect("mappings lock"), vec![(1, 3, 4, 4, 4)]);
        assert_eq!(
            *surface_mappings.lock().expect("surface mappings lock"),
            vec![(1, 3, 4)]
        );
        assert!(client.surfaces[&1].is_mapped);
        assert_eq!(
            (client.surfaces[&1].output_origin_x, client.surfaces[&1].output_origin_y),
            (3, 4)
        );

        let output = client.drain_output();
        assert_eq!(output.len(), 1);
        assert_eq!(
            output[0].region,
            ExclusiveRectangle {
                left: 3,
                top: 4,
                right: 7,
                bottom: 8,
            }
        );
    }

    #[test]
    fn map_surface_to_scaled_output_ignores_unknown_surface() {
        let mut client = GraphicsPipelineClient::new(Box::new(TestHandler), None);
        client
            .handle_pdu(GfxPdu::MapSurfaceToScaledOutput(MapSurfaceToScaledOutputPdu {
                surface_id: 1,
                output_origin_x: 0,
                output_origin_y: 0,
                target_width: 2,
                target_height: 2,
            }))
            .unwrap();

        assert!(client.surfaces.is_empty());
        assert!(client.drain_output().is_empty());
    }

    /// A Planar `WireToSurface1` decodes to the pixels that were encoded, rather
    /// than falling through to the handler as an unsupported codec.
    ///
    /// The payload is built with IronRDP's own RDP6 encoder, so this exercises the
    /// real `RDP6_BITMAP_STREAM` framing ([MS-RDPEGDI] 2.2.2.5.1) that
    /// `RDPGFX_CODECID_PLANAR` carries, not a hand-rolled approximation.
    #[test]
    fn planar_wire_to_surface_decodes_to_rgba() {
        use ironrdp_graphics::rdp6::{BitmapStreamEncoder, RgbChannels};

        const W: u16 = 4;
        const H: u16 = 2;

        // Distinct per-channel values so a channel swap or row reversal fails.
        let mut rgb = Vec::new();
        for i in 0..u8::try_from(W).unwrap() * u8::try_from(H).unwrap() {
            rgb.extend_from_slice(&[i, 100 + i, 200 + i]);
        }

        for rle in [false, true] {
            let mut encoded = vec![0u8; rgb.len() * 4 + 64];
            let len = BitmapStreamEncoder::new(usize::from(W), usize::from(H))
                .encode_bitmap::<RgbChannels>(&rgb, &mut encoded, rle)
                .unwrap();
            encoded.truncate(len);

            let updates = Arc::new(Mutex::new(Vec::new()));
            let unhandled = Arc::new(Mutex::new(0));
            let mut client = GraphicsPipelineClient::new(
                Box::new(CapturingHandler {
                    updates: Arc::clone(&updates),
                    unhandled: Arc::clone(&unhandled),
                }),
                None,
            );

            let _ = client.handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
                surface_id: 1,
                width: W,
                height: H,
                pixel_format: PixelFormat::XRgb,
            }));

            client
                .handle_pdu(GfxPdu::WireToSurface1(crate::pdu::WireToSurface1Pdu {
                    surface_id: 1,
                    codec_id: Codec1Type::Planar,
                    pixel_format: PixelFormat::XRgb,
                    destination_rectangle: ExclusiveRectangle {
                        left: 0,
                        top: 0,
                        right: W,
                        bottom: H,
                    },
                    bitmap_data: encoded,
                }))
                .unwrap();

            assert_eq!(
                *unhandled.lock().expect("unhandled lock"),
                0,
                "rle={rle}: Planar must not fall through"
            );
            let updates = updates.lock().expect("updates lock");
            assert_eq!(updates.len(), 1, "rle={rle}");
            let (codec_id, width, height, data) = &updates[0];
            assert_eq!(*codec_id, Codec1Type::Planar, "rle={rle}");
            assert_eq!(*width, W, "rle={rle}");
            assert_eq!(*height, H, "rle={rle}");

            let expected: Vec<u8> = rgb.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 0xFF]).collect();
            assert_eq!(*data, expected, "rle={rle}");
        }
    }

    /// ClearCodec and Planar `WireToSurface1` decodes must also reach the compositor,
    /// not just the handler. Before this fix, `decode_avc420` and `handle_uncompressed`
    /// called `self.compositor.apply_bitmap`, but `decode_clearcodec` and `decode_planar`
    /// did not, so a server sending either codec painted nothing into the compositor
    /// surface even though the handler was correctly notified.
    #[test]
    fn clearcodec_and_planar_feed_the_compositor() {
        use ironrdp_graphics::clearcodec::ClearCodecEncoder;
        use ironrdp_graphics::rdp6::{BitmapStreamEncoder, RgbChannels};

        const W: u16 = 4;
        const H: u16 = 2;
        const ORIGIN_X: u16 = 10;
        const ORIGIN_Y: u16 = 20;

        let dest_rect = ExclusiveRectangle {
            left: 0,
            top: 0,
            right: W,
            bottom: H,
        };

        let mut bgra = Vec::new();
        for i in 0..u32::from(W) * u32::from(H) {
            let i = u8::try_from(i).unwrap();
            bgra.extend_from_slice(&[i, 100 + i, 200 + i, 0xFF]);
        }
        let clearcodec_data = ClearCodecEncoder::new().encode(&bgra, W, H);

        let mut rgb = Vec::new();
        for i in 0..u8::try_from(W).unwrap() * u8::try_from(H).unwrap() {
            rgb.extend_from_slice(&[i, 100 + i, 200 + i]);
        }
        let mut planar_encoded = vec![0u8; rgb.len() * 4 + 64];
        let len = BitmapStreamEncoder::new(usize::from(W), usize::from(H))
            .encode_bitmap::<RgbChannels>(&rgb, &mut planar_encoded, false)
            .unwrap();
        planar_encoded.truncate(len);

        for (codec_id, bitmap_data) in [
            (Codec1Type::ClearCodec, clearcodec_data),
            (Codec1Type::Planar, planar_encoded),
        ] {
            let mut client = GraphicsPipelineClient::new(Box::new(TestHandler), None);

            let _ = client.handle_pdu(GfxPdu::ResetGraphics(crate::pdu::ResetGraphicsPdu {
                width: 200,
                height: 100,
                monitors: vec![],
            }));
            let _ = client.handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
                surface_id: 1,
                width: W,
                height: H,
                pixel_format: PixelFormat::XRgb,
            }));
            let _ = client.handle_pdu(GfxPdu::MapSurfaceToOutput(crate::pdu::MapSurfaceToOutputPdu {
                surface_id: 1,
                output_origin_x: u32::from(ORIGIN_X),
                output_origin_y: u32::from(ORIGIN_Y),
            }));
            // Discard the delta from the surface becoming visible: only the
            // WireToSurface1 decode below is under test.
            let _ = client.handle_pdu(GfxPdu::EndFrame(crate::pdu::EndFramePdu { frame_id: 0 }));
            let _ = client.drain_output();

            client
                .handle_pdu(GfxPdu::WireToSurface1(crate::pdu::WireToSurface1Pdu {
                    surface_id: 1,
                    codec_id,
                    pixel_format: PixelFormat::XRgb,
                    destination_rectangle: dest_rect.clone(),
                    bitmap_data,
                }))
                .unwrap();
            let _ = client.handle_pdu(GfxPdu::EndFrame(crate::pdu::EndFramePdu { frame_id: 1 }));

            let updates = client.drain_output();
            assert_eq!(updates.len(), 1, "{codec_id:?}: the decode must reach the compositor");
            let update = &updates[0];
            assert_eq!(
                (
                    update.region.left,
                    update.region.top,
                    update.region.right,
                    update.region.bottom
                ),
                (ORIGIN_X, ORIGIN_Y, ORIGIN_X + W, ORIGIN_Y + H),
                "{codec_id:?}"
            );
        }
    }

    #[test]
    fn convert_rgb24_to_rgba_widens_and_marks_opaque() {
        let rgb = vec![
            0x10, 0x20, 0x30, // R=16, G=32, B=48
            0x40, 0x50, 0x60, // R=64, G=80, B=96
        ];

        assert_eq!(
            convert_rgb24_to_rgba(&rgb),
            vec![0x10, 0x20, 0x30, 0xFF, 0x40, 0x50, 0x60, 0xFF]
        );
    }

    #[test]
    fn state_transitions() {
        let mut client = GraphicsPipelineClient::new(Box::new(TestHandler), None);

        assert_eq!(client.state, ClientState::WaitingForConfirm);
        assert!(!client.is_active());

        let _ = client.handle_pdu(GfxPdu::CapabilitiesConfirm(
            crate::pdu::CapabilitiesConfirmPdu::from_typed(&CapabilitySet::V8 {
                flags: CapabilitiesV8Flags::empty(),
            }),
        ));
        assert_eq!(client.state, ClientState::Active);
        assert!(client.is_active());

        client.close(0);
        assert_eq!(client.state, ClientState::Closed);
        assert!(!client.is_active());
    }

    #[test]
    fn reset_graphics_clears_surfaces_and_frame_tracking() {
        let mut client = GraphicsPipelineClient::new(Box::new(TestHandler), None);

        let _ = client.handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
            surface_id: 1,
            width: 100,
            height: 100,
            pixel_format: PixelFormat::XRgb,
        }));
        assert_eq!(client.surfaces.len(), 1);

        // Simulate mid-stream state
        let _ = client.handle_pdu(GfxPdu::StartFrame(crate::pdu::StartFramePdu {
            timestamp: crate::pdu::Timestamp {
                milliseconds: 0,
                seconds: 0,
                minutes: 0,
                hours: 0,
            },
            frame_id: 42,
        }));
        assert!(client.current_frame_id.is_some());

        let _ = client.handle_pdu(GfxPdu::ResetGraphics(crate::pdu::ResetGraphicsPdu {
            width: 1920,
            height: 1080,
            monitors: vec![],
        }));

        assert!(client.surfaces.is_empty(), "surfaces should be cleared");
        assert!(client.current_frame_id.is_none(), "frame_id should be reset");
    }

    #[test]
    fn crop_decoded_frame_identity() {
        let data = vec![0xFFu8; 4 * 4 * 4];
        let cropped = crop_decoded_frame(&data, 4, 4, 4, 4);
        assert_eq!(cropped.len(), data.len());
    }

    #[test]
    fn crop_decoded_frame_macroblock_alignment() {
        // H.264 encodes 1920x1080 as 1920x1088 (rounded to 16-pixel macroblock boundary)
        let data = vec![0xAAu8; 1920 * 1088 * 4];
        let cropped = crop_decoded_frame(&data, 1920, 1088, 1920, 1080);
        assert_eq!(cropped.len(), 1920 * 1080 * 4);
    }

    #[test]
    fn convert_bgra_to_rgba_reorders_channels() {
        // BGRA input: [B, G, R, A] per pixel
        let bgra = vec![
            0xFF, 0x00, 0x00, 0xCC, // B=255, G=0, R=0, A=204 (blue)
            0x00, 0xFF, 0x00, 0x80, // B=0, G=255, R=0, A=128 (green)
        ];
        let rgba = convert_bgra_to_rgba(&bgra);
        // Expected: [R, G, B, A] per pixel
        assert_eq!(
            rgba,
            vec![
                0x00, 0x00, 0xFF, 0xCC, // R=0, G=0, B=255, A=204
                0x00, 0xFF, 0x00, 0x80, // R=0, G=255, B=0, A=128
            ]
        );
    }

    #[test]
    fn convert_uncompressed_bgrx_to_rgba() {
        // Wire format: [B, G, R, A] per pixel (0xAARRGGBB little-endian)
        let wire_pixels = vec![
            0x00, 0x80, 0xFF, 0xCC, // B=0, G=128, R=255, A=204
            0x10, 0x20, 0x30, 0x40, // B=16, G=32, R=48, A=64
        ];
        let rgba = convert_uncompressed_to_rgba(&wire_pixels);
        // Expected: [R, G, B, 0xFF] per pixel (alpha forced to opaque)
        assert_eq!(rgba, vec![0xFF, 0x80, 0x00, 0xFF, 0x30, 0x20, 0x10, 0xFF]);
    }

    /// The server prefers the most capable advertised set, so the default leads with
    /// the version 10 sets that enable AVC444, which `decode_avc444` paints.
    #[test]
    fn default_capabilities_lead_with_avc444() {
        let caps = TestHandler.capabilities();
        assert!(matches!(caps.first(), Some(CapabilitySet::V10_7 { .. })));
        assert!(CodecCapabilities::from_capability_set(&caps[0]).avc444);
    }

    /// AVC420 must stay advertised: it is the H.264 flavour a decoder without YUV
    /// planes decodes, once the start-time filter drops the AVC444 sets.
    #[test]
    fn default_capabilities_still_advertise_avc420() {
        assert!(
            TestHandler
                .capabilities()
                .iter()
                .any(|cap| CodecCapabilities::from_capability_set(cap).avc420),
            "no advertised set enables AVC420"
        );
    }

    #[test]
    fn reset_graphics_reports_only_materializable_output_extents() {
        let mut client = GraphicsPipelineClient::new(Box::new(TestHandler), None);
        client
            .handle_pdu(GfxPdu::ResetGraphics(crate::pdu::ResetGraphicsPdu {
                width: 19_200,
                height: 1080,
                monitors: vec![],
            }))
            .expect("wide multimon output fits compositor limits");
        assert_eq!(client.take_output_reset(), Some((19_200, 1080)));
        assert_eq!(client.take_output_reset(), None);
        client
            .handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
                surface_id: 7,
                width: 1,
                height: 1,
                pixel_format: PixelFormat::XRgb,
            }))
            .expect("create surface before rejected reset");

        assert!(
            client
                .handle_pdu(GfxPdu::ResetGraphics(crate::pdu::ResetGraphicsPdu {
                    width: 32_766,
                    height: 32_766,
                    monitors: vec![],
                }))
                .is_err(),
            "an output over the memory budget must be rejected before framebuffer allocation"
        );
        assert!(
            client.get_surface(7).is_none(),
            "ResetGraphics destroys prior surfaces even when its output extent is rejected"
        );
        assert!(
            client
                .handle_pdu(GfxPdu::ResetGraphics(crate::pdu::ResetGraphicsPdu {
                    width: 32_767,
                    height: 1,
                    monitors: vec![],
                }))
                .is_err(),
            "an output over the protocol dimension limit must be rejected"
        );
        assert_eq!(client.take_output_reset(), None);
    }

    fn progressive_client() -> GraphicsPipelineClient {
        let mut client = GraphicsPipelineClient::new(Box::new(TestHandler), None);
        client
            .handle_pdu(GfxPdu::ResetGraphics(crate::pdu::ResetGraphicsPdu {
                width: 64,
                height: 64,
                monitors: vec![],
            }))
            .unwrap();
        client
            .handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
                surface_id: 1,
                width: 64,
                height: 64,
                pixel_format: PixelFormat::XRgb,
            }))
            .unwrap();
        client
    }

    fn wire_progressive(client: &mut GraphicsPipelineClient, bitmap_data: Vec<u8>) -> PduResult<Vec<DvcMessage>> {
        client.handle_pdu(GfxPdu::WireToSurface2(WireToSurface2Pdu {
            surface_id: 1,
            codec_id: crate::pdu::Codec2Type::RemoteFxProgressive,
            codec_context_id: 7,
            pixel_format: PixelFormat::XRgb,
            bitmap_data,
        }))
    }

    fn progressive_context_stream(with_context: bool) -> Vec<u8> {
        use ironrdp_pdu::codecs::rfx::RfxRectangle;
        use ironrdp_pdu::codecs::rfx::progressive::{
            ProgressiveBlock, ProgressiveContextPdu, ProgressiveFrameBeginPdu, ProgressiveFrameEndPdu,
            ProgressiveRegion, ProgressiveSyncPdu, encode_progressive_stream,
        };

        let mut blocks = Vec::new();
        if with_context {
            blocks.push(ProgressiveBlock::Sync(ProgressiveSyncPdu));
            blocks.push(ProgressiveBlock::Context(ProgressiveContextPdu {
                context_id: 0,
                tile_size: 0x0040,
                flags: 0,
            }));
        }
        blocks.push(ProgressiveBlock::FrameBegin(ProgressiveFrameBeginPdu {
            frame_index: 0,
            region_count: 1,
        }));
        blocks.push(ProgressiveBlock::Region(ProgressiveRegion {
            tile_size: 0x40,
            rects: vec![RfxRectangle {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            }],
            quant_vals: vec![],
            quant_prog_vals: vec![],
            flags: 0,
            tiles: vec![],
        }));
        blocks.push(ProgressiveBlock::FrameEnd(ProgressiveFrameEndPdu));

        encode_progressive_stream(&blocks).unwrap()
    }

    fn progressive_tile_stream(
        tile_x: u16,
        tile_y: u16,
        region_rect: ironrdp_pdu::codecs::rfx::RfxRectangle,
    ) -> Vec<u8> {
        use ironrdp_graphics::progressive::{COEFFICIENTS_PER_COMPONENT, encode_first_pass};
        use ironrdp_pdu::codecs::rfx::progressive::{
            ComponentCodecQuant, ProgressiveBlock, ProgressiveContextPdu, ProgressiveFrameBeginPdu,
            ProgressiveFrameEndPdu, ProgressiveRegion, ProgressiveSyncPdu, ProgressiveTile, TileSimple,
            encode_progressive_stream,
        };

        let base_quant = ComponentCodecQuant::LOSSLESS;
        let mut component = [0i16; COEFFICIENTS_PER_COMPONENT];
        let mut component_data = [0u8; 8192];
        let component_len = encode_first_pass(
            &mut component,
            &mut component_data,
            &base_quant,
            &ComponentCodecQuant::LOSSLESS,
            false,
        )
        .unwrap();
        let component_data = &component_data[..component_len];

        encode_progressive_stream(&[
            ProgressiveBlock::Sync(ProgressiveSyncPdu),
            ProgressiveBlock::Context(ProgressiveContextPdu {
                context_id: 0,
                tile_size: 0x0040,
                flags: 0,
            }),
            ProgressiveBlock::FrameBegin(ProgressiveFrameBeginPdu {
                frame_index: 0,
                region_count: 1,
            }),
            ProgressiveBlock::Region(ProgressiveRegion {
                tile_size: 0x40,
                rects: vec![region_rect],
                quant_vals: vec![base_quant],
                quant_prog_vals: vec![],
                flags: 0,
                tiles: vec![ProgressiveTile::Simple(TileSimple {
                    quant_idx_y: 0,
                    quant_idx_cb: 0,
                    quant_idx_cr: 0,
                    x_idx: tile_x,
                    y_idx: tile_y,
                    flags: 0,
                    y_data: component_data,
                    cb_data: component_data,
                    cr_data: component_data,
                    tail_data: &[],
                })],
            }),
            ProgressiveBlock::FrameEnd(ProgressiveFrameEndPdu),
        ])
        .unwrap()
    }

    #[test]
    fn wire_to_surface2_renders_and_crops_progressive_edge_tile() {
        let updates = Arc::new(Mutex::new(Vec::new()));
        let unhandled = Arc::new(Mutex::new(0));
        let mut client = GraphicsPipelineClient::new(
            Box::new(CapturingHandler {
                updates: Arc::clone(&updates),
                unhandled,
            }),
            None,
        );
        client
            .handle_pdu(GfxPdu::ResetGraphics(crate::pdu::ResetGraphicsPdu {
                width: 100,
                height: 100,
                monitors: vec![],
            }))
            .unwrap();
        client
            .handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
                surface_id: 1,
                width: 100,
                height: 100,
                pixel_format: PixelFormat::XRgb,
            }))
            .unwrap();
        client
            .handle_pdu(GfxPdu::MapSurfaceToOutput(crate::pdu::MapSurfaceToOutputPdu {
                surface_id: 1,
                output_origin_x: 0,
                output_origin_y: 0,
            }))
            .unwrap();
        client
            .handle_pdu(GfxPdu::StartFrame(crate::pdu::StartFramePdu {
                timestamp: crate::pdu::Timestamp {
                    milliseconds: 0,
                    seconds: 0,
                    minutes: 0,
                    hours: 0,
                },
                frame_id: 1,
            }))
            .unwrap();
        client
            .handle_pdu(GfxPdu::EndFrame(crate::pdu::EndFramePdu { frame_id: 1 }))
            .unwrap();
        let _ = client.drain_output();
        client
            .handle_pdu(GfxPdu::StartFrame(crate::pdu::StartFramePdu {
                timestamp: crate::pdu::Timestamp {
                    milliseconds: 0,
                    seconds: 0,
                    minutes: 0,
                    hours: 0,
                },
                frame_id: 2,
            }))
            .unwrap();
        wire_progressive(&mut client, progressive_tile_stream(1, 1, rect(64, 64, 36, 36))).unwrap();
        client
            .handle_pdu(GfxPdu::EndFrame(crate::pdu::EndFramePdu { frame_id: 2 }))
            .unwrap();

        let updates = updates.lock().expect("updates lock");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, Codec1Type::Uncompressed);
        assert_eq!(updates[0].1, 36);
        assert_eq!(updates[0].2, 36);
        assert_eq!(updates[0].3.len(), 36 * 36 * 4);
        assert!(updates[0].3.iter().any(|&pixel| pixel != 0));

        let output = client.drain_output();
        assert_eq!(output.len(), 1);
        assert_eq!(
            output[0].region,
            ExclusiveRectangle {
                left: 64,
                top: 64,
                right: 100,
                bottom: 100,
            }
        );
        assert_eq!(output[0].data.len(), 36 * 36 * 4);
    }

    fn start_frame(client: &mut GraphicsPipelineClient, frame_id: u32) {
        client
            .handle_pdu(GfxPdu::StartFrame(crate::pdu::StartFramePdu {
                timestamp: crate::pdu::Timestamp {
                    milliseconds: 0,
                    seconds: 0,
                    minutes: 0,
                    hours: 0,
                },
                frame_id,
            }))
            .unwrap();
    }

    fn end_frame(client: &mut GraphicsPipelineClient, frame_id: u32) {
        client
            .handle_pdu(GfxPdu::EndFrame(crate::pdu::EndFramePdu { frame_id }))
            .unwrap();
    }

    fn rect(x: u16, y: u16, width: u16, height: u16) -> ironrdp_pdu::codecs::rfx::RfxRectangle {
        ironrdp_pdu::codecs::rfx::RfxRectangle { x, y, width, height }
    }

    #[test]
    fn wire_to_surface2_clips_compositor_output_to_region() {
        use ironrdp_graphics::progressive::{COEFFICIENTS_PER_COMPONENT, encode_first_pass};
        use ironrdp_pdu::codecs::rfx::progressive::{
            ComponentCodecQuant, ProgressiveBlock, ProgressiveContextPdu, ProgressiveFrameBeginPdu,
            ProgressiveFrameEndPdu, ProgressiveRegion, ProgressiveSyncPdu, ProgressiveTile, TileSimple,
            encode_progressive_stream,
        };

        // Send one real, neutral-gray TILE_SIMPLE, then reference it from a
        // second Progressive payload in the same RDPGFX frame.
        let (first_bitmap_data, shared_bitmap_data) = {
            let base_quant = ComponentCodecQuant::LOSSLESS;
            let mut component = [0i16; COEFFICIENTS_PER_COMPONENT];
            let mut component_data = [0u8; 8192];
            let component_len = encode_first_pass(
                &mut component,
                &mut component_data,
                &base_quant,
                &ComponentCodecQuant::LOSSLESS,
                false,
            )
            .unwrap();
            let component_data = &component_data[..component_len];

            let region = ProgressiveRegion {
                tile_size: 0x40,
                rects: vec![rect(8, 12, 16, 20)],
                quant_vals: vec![base_quant],
                quant_prog_vals: vec![],
                flags: 0,
                tiles: vec![ProgressiveTile::Simple(TileSimple {
                    quant_idx_y: 0,
                    quant_idx_cb: 0,
                    quant_idx_cr: 0,
                    x_idx: 0,
                    y_idx: 0,
                    flags: 0,
                    y_data: component_data,
                    cb_data: component_data,
                    cr_data: component_data,
                    tail_data: &[],
                })],
            };
            let shared_tile_region = ProgressiveRegion {
                tile_size: 0x40,
                rects: vec![rect(32, 40, 8, 10)],
                quant_vals: vec![],
                quant_prog_vals: vec![],
                flags: 0,
                tiles: vec![],
            };

            let first_bitmap_data = encode_progressive_stream(&[
                ProgressiveBlock::Sync(ProgressiveSyncPdu),
                ProgressiveBlock::Context(ProgressiveContextPdu {
                    context_id: 0,
                    tile_size: 0x0040,
                    flags: 0,
                }),
                ProgressiveBlock::FrameBegin(ProgressiveFrameBeginPdu {
                    frame_index: 0,
                    region_count: 1,
                }),
                ProgressiveBlock::Region(region),
                ProgressiveBlock::FrameEnd(ProgressiveFrameEndPdu),
            ])
            .unwrap();
            let shared_bitmap_data = encode_progressive_stream(&[
                ProgressiveBlock::FrameBegin(ProgressiveFrameBeginPdu {
                    frame_index: 1,
                    region_count: 1,
                }),
                ProgressiveBlock::Region(shared_tile_region),
                ProgressiveBlock::FrameEnd(ProgressiveFrameEndPdu),
            ])
            .unwrap();

            (first_bitmap_data, shared_bitmap_data)
        };

        let mut client = progressive_client();

        // Commit and drain the initial map separately so the next output can
        // only have been produced by WireToSurface2.
        start_frame(&mut client, 1);
        client
            .handle_pdu(GfxPdu::MapSurfaceToOutput(crate::pdu::MapSurfaceToOutputPdu {
                surface_id: 1,
                output_origin_x: 0,
                output_origin_y: 0,
            }))
            .unwrap();
        end_frame(&mut client, 1);
        let _ = client.drain_output();

        start_frame(&mut client, 2);
        wire_progressive(&mut client, first_bitmap_data).unwrap();
        wire_progressive(&mut client, shared_bitmap_data.clone()).unwrap();
        end_frame(&mut client, 2);

        let output = client.drain_output();
        assert_eq!(output.len(), 2);
        assert_eq!(
            output[0].region,
            ExclusiveRectangle {
                left: 8,
                top: 12,
                right: 24,
                bottom: 32,
            }
        );
        assert_eq!(output[0].data.len(), 16 * 20 * 4);
        assert!(output[0].data.iter().any(|&value| value != 0));
        assert_eq!(
            output[1].region,
            ExclusiveRectangle {
                left: 32,
                top: 40,
                right: 40,
                bottom: 50,
            }
        );
        assert_eq!(output[1].data.len(), 8 * 10 * 4);
        assert!(output[1].data.iter().any(|&value| value != 0));

        // A later RDPGFX frame cannot reference tiles from this completed frame.
        start_frame(&mut client, 3);
        wire_progressive(&mut client, shared_bitmap_data).unwrap();
        end_frame(&mut client, 3);
        assert!(client.drain_output().is_empty());
    }

    fn progressive_upgrade_stream() -> Vec<u8> {
        use ironrdp_pdu::codecs::rfx::progressive::{
            ComponentCodecQuant, ProgressiveBlock, ProgressiveFrameBeginPdu, ProgressiveFrameEndPdu, ProgressiveRegion,
            ProgressiveTile, TileUpgrade, encode_progressive_stream,
        };

        encode_progressive_stream(&[
            ProgressiveBlock::FrameBegin(ProgressiveFrameBeginPdu {
                frame_index: 1,
                region_count: 1,
            }),
            ProgressiveBlock::Region(ProgressiveRegion {
                tile_size: 0x40,
                rects: vec![rect(0, 0, 64, 64)],
                quant_vals: vec![ComponentCodecQuant::LOSSLESS],
                quant_prog_vals: vec![],
                flags: 0,
                tiles: vec![ProgressiveTile::Upgrade(TileUpgrade {
                    quant_idx_y: 0,
                    quant_idx_cb: 0,
                    quant_idx_cr: 0,
                    x_idx: 0,
                    y_idx: 0,
                    quality: 0xFF,
                    y_srl_data: &[],
                    y_raw_data: &[],
                    cb_srl_data: &[],
                    cb_raw_data: &[],
                    cr_srl_data: &[],
                    cr_raw_data: &[],
                })],
            }),
            ProgressiveBlock::FrameEnd(ProgressiveFrameEndPdu),
        ])
        .unwrap()
    }

    #[test]
    fn repeated_decode_failures_preserve_progressive_tiles_on_all_surfaces() {
        let updates = Arc::new(Mutex::new(Vec::new()));
        let mut client = progressive_client();
        client.handler = Box::new(CapturingHandler {
            updates: Arc::clone(&updates),
            unhandled: Arc::new(Mutex::new(0)),
        });
        client
            .handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
                surface_id: 2,
                width: 64,
                height: 64,
                pixel_format: PixelFormat::XRgb,
            }))
            .unwrap();
        for surface_id in [1, 2] {
            send(
                &mut client,
                surface_id,
                progressive_tile_stream(0, 0, rect(0, 0, 64, 64)),
            );
        }
        assert_eq!(updates.lock().unwrap().len(), 2);
        updates.lock().unwrap().clear();

        for _ in 0..64 {
            wire_progressive(&mut client, vec![0xFF; 32]).unwrap();
        }
        assert_eq!(client.dropped_updates, 64);
        for surface_id in [2, 1] {
            send(&mut client, surface_id, progressive_upgrade_stream());
        }
        let updates = updates.lock().unwrap();
        assert_eq!(
            updates.len(),
            2,
            "both contexts must retain the first-pass tile for UPGRADE"
        );
        assert_eq!(&updates[0].3[..4], &[128, 128, 128, 255]);
        assert_eq!(&updates[1].3[..4], &[128, 128, 128, 255]);

        fn send(client: &mut GraphicsPipelineClient, surface_id: u16, bitmap_data: Vec<u8>) {
            client
                .handle_pdu(GfxPdu::WireToSurface2(WireToSurface2Pdu {
                    surface_id,
                    codec_id: crate::pdu::Codec2Type::RemoteFxProgressive,
                    codec_context_id: 7,
                    pixel_format: PixelFormat::XRgb,
                    bitmap_data,
                }))
                .unwrap();
        }
    }

    #[test]
    fn repeated_decode_failures_preserve_clearcodec_glyphs_across_graphics_reset() {
        use ironrdp_graphics::clearcodec::ClearCodecEncoder;
        use ironrdp_pdu::codecs::clearcodec::FLAG_GLYPH_HIT;

        let updates = Arc::new(Mutex::new(Vec::new()));
        let mut client = progressive_client();
        client.handler = Box::new(CapturingHandler {
            updates: Arc::clone(&updates),
            unhandled: Arc::new(Mutex::new(0)),
        });
        let mut encoder = ClearCodecEncoder::new();
        let pixel = [0x12, 0x34, 0x56, 0xFF];
        send(&mut client, encoder.encode(&pixel, 1, 1));
        let glyph_hit = encoder.encode(&pixel, 1, 1);
        assert_ne!(glyph_hit[0] & FLAG_GLYPH_HIT, 0);

        for _ in 0..15 {
            send(&mut client, vec![0xFF; 16]);
        }
        client
            .handle_pdu(GfxPdu::ResetGraphics(crate::pdu::ResetGraphicsPdu {
                width: 64,
                height: 64,
                monitors: vec![],
            }))
            .unwrap();
        client
            .handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
                surface_id: 1,
                width: 64,
                height: 64,
                pixel_format: PixelFormat::XRgb,
            }))
            .unwrap();
        for _ in 0..49 {
            send(&mut client, vec![0xFF; 16]);
        }
        assert_eq!(client.dropped_updates, 64);
        send(&mut client, glyph_hit);
        assert_eq!(client.dropped_updates, 64, "cached glyph must still decode");
        let updates = updates.lock().unwrap();
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].3, updates[1].3);

        fn send(client: &mut GraphicsPipelineClient, bitmap_data: Vec<u8>) {
            client
                .handle_pdu(GfxPdu::WireToSurface1(crate::pdu::WireToSurface1Pdu {
                    surface_id: 1,
                    codec_id: Codec1Type::ClearCodec,
                    pixel_format: PixelFormat::XRgb,
                    destination_rectangle: ExclusiveRectangle {
                        left: 0,
                        top: 0,
                        right: 1,
                        bottom: 1,
                    },
                    bitmap_data,
                }))
                .unwrap();
        }
    }

    /// A REGION may be covered by a tile sent earlier in the same RDPGFX frame under
    /// another codec context. Skipping it left stale 64-pixel columns of an animation
    /// on screen until the next full repaint.
    #[test]
    fn region_reuses_tile_sent_under_another_codec_context() {
        use ironrdp_pdu::codecs::rfx::progressive::{
            ProgressiveBlock, ProgressiveFrameBeginPdu, ProgressiveFrameEndPdu, ProgressiveRegion,
            encode_progressive_stream,
        };

        let first = progressive_tile_stream(0, 0, rect(8, 12, 16, 20));
        let shared = encode_progressive_stream(&[
            ProgressiveBlock::FrameBegin(ProgressiveFrameBeginPdu {
                frame_index: 1,
                region_count: 1,
            }),
            ProgressiveBlock::Region(ProgressiveRegion {
                tile_size: 0x40,
                rects: vec![rect(32, 40, 8, 10)],
                quant_vals: vec![],
                quant_prog_vals: vec![],
                flags: 0,
                tiles: vec![],
            }),
            ProgressiveBlock::FrameEnd(ProgressiveFrameEndPdu),
        ])
        .unwrap();

        let decode_frame = |second_codec_context_id: u32| {
            let mut client = progressive_client();
            start_frame(&mut client, 1);
            client
                .handle_pdu(GfxPdu::MapSurfaceToOutput(crate::pdu::MapSurfaceToOutputPdu {
                    surface_id: 1,
                    output_origin_x: 0,
                    output_origin_y: 0,
                }))
                .unwrap();
            end_frame(&mut client, 1);
            let _ = client.drain_output();

            start_frame(&mut client, 2);
            wire_progressive(&mut client, first.clone()).unwrap();
            client
                .handle_pdu(GfxPdu::WireToSurface2(WireToSurface2Pdu {
                    surface_id: 1,
                    codec_id: crate::pdu::Codec2Type::RemoteFxProgressive,
                    codec_context_id: second_codec_context_id,
                    pixel_format: PixelFormat::XRgb,
                    bitmap_data: shared.clone(),
                }))
                .unwrap();
            end_frame(&mut client, 2);

            client
                .drain_output()
                .into_iter()
                .map(|update| (update.region, update.data))
                .collect::<Vec<_>>()
        };

        let other_context = decode_frame(8);
        let regions: Vec<_> = other_context.iter().map(|(region, _)| region.clone()).collect();
        assert_eq!(
            regions,
            vec![
                ExclusiveRectangle {
                    left: 8,
                    top: 12,
                    right: 24,
                    bottom: 32,
                },
                ExclusiveRectangle {
                    left: 32,
                    top: 40,
                    right: 40,
                    bottom: 50,
                },
            ],
            "the second context's REGION must paint from the tile sent earlier in the frame"
        );
        assert!(other_context[1].1.iter().any(|&value| value != 0));
        assert_eq!(
            other_context,
            decode_frame(7),
            "a REGION must paint the same pixels whichever codec context carries it"
        );
    }

    fn assert_progressive_context_is_deleted(clear: impl FnOnce(&mut GraphicsPipelineClient)) {
        let mut client = progressive_client();
        wire_progressive(&mut client, progressive_context_stream(true)).unwrap();
        clear(&mut client);
        assert!(wire_progressive(&mut client, progressive_context_stream(false)).is_ok());
        assert_eq!(client.dropped_updates, 1);
    }

    #[test]
    fn an_undecodable_clearcodec_update_is_dropped_not_fatal() {
        let mut client = progressive_client();

        let result = client.handle_pdu(GfxPdu::WireToSurface1(crate::pdu::WireToSurface1Pdu {
            surface_id: 1,
            codec_id: Codec1Type::ClearCodec,
            pixel_format: PixelFormat::XRgb,
            destination_rectangle: ExclusiveRectangle {
                left: 0,
                top: 0,
                right: 64,
                bottom: 64,
            },
            bitmap_data: vec![0xFF; 16],
        }));

        assert!(result.is_ok());
        assert_eq!(client.dropped_updates, 1);
    }

    #[test]
    fn progressive_context_survives_graphics_reset() {
        let updates = Arc::new(Mutex::new(Vec::new()));
        let mut client = progressive_client();
        client.handler = Box::new(CapturingHandler {
            updates: Arc::clone(&updates),
            unhandled: Arc::new(Mutex::new(0)),
        });
        wire_progressive(&mut client, progressive_tile_stream(0, 0, rect(0, 0, 64, 64))).unwrap();
        assert_eq!(updates.lock().unwrap().len(), 1);
        updates.lock().unwrap().clear();

        client
            .handle_pdu(GfxPdu::ResetGraphics(crate::pdu::ResetGraphicsPdu {
                width: 64,
                height: 64,
                monitors: vec![],
            }))
            .unwrap();
        client
            .handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
                surface_id: 1,
                width: 64,
                height: 64,
                pixel_format: PixelFormat::XRgb,
            }))
            .unwrap();

        // Windows never re-sends SYNC + CONTEXT after a reset, so a CONTEXT-less
        // continuation has to keep decoding.
        wire_progressive(&mut client, progressive_upgrade_stream()).unwrap();
        assert_eq!(client.dropped_updates(), 0);
        let updates = updates.lock().unwrap();
        assert_eq!(updates.len(), 1, "UPGRADE must retain its first-pass tile");
        assert_eq!(&updates[0].3[..4], &[128, 128, 128, 255]);
    }

    // oracle: spec MS-RDPEGFX §3.3.5.19 resets the codec and tile contexts in §3.3.1.1-3.3.1.2.
    #[test]
    fn protocol_reset_discards_progressive_context_and_tiles() {
        use crate::pdu::{CapabilitiesConfirmPdu, Codec2Type, CreateSurfacePdu};

        let updates = Arc::new(Mutex::new(Vec::new()));
        let mut client = GraphicsPipelineClient::new(
            Box::new(CapturingHandler {
                updates: Arc::clone(&updates),
                unhandled: Arc::new(Mutex::new(0)),
            }),
            None,
        );
        let process = |client: &mut GraphicsPipelineClient, pdu: &GfxPdu| {
            let bytes = ironrdp_core::encode_vec(pdu).unwrap();
            client.process(0, &zgfx::wrap_uncompressed(&bytes)).unwrap();
        };
        let confirm = GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu::from_typed(&CapabilitySet::V10_7 {
            flags: CapabilitiesV107Flags::empty(),
        }));
        let create = GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id: 1,
            width: 64,
            height: 64,
            pixel_format: PixelFormat::XRgb,
        });
        let wire = |bitmap_data| {
            GfxPdu::WireToSurface2(WireToSurface2Pdu {
                surface_id: 1,
                codec_id: Codec2Type::RemoteFxProgressive,
                codec_context_id: 7,
                pixel_format: PixelFormat::XRgb,
                bitmap_data,
            })
        };
        let seed = wire(progressive_tile_stream(0, 0, rect(0, 0, 64, 64)));
        let upgrade = wire(progressive_upgrade_stream());
        for pdu in [&confirm, &create, &seed, &upgrade] {
            process(&mut client, pdu);
        }
        assert_eq!(client.dropped_updates(), 0);
        assert_eq!(updates.lock().unwrap().len(), 2);
        updates.lock().unwrap().clear();
        assert!(client.request_protocol_reset().is_some());
        for pdu in [&confirm, &create, &upgrade] {
            process(&mut client, pdu);
        }
        assert_eq!(
            client.dropped_updates(),
            1,
            "the previous codec context must no longer exist"
        );
        process(&mut client, &wire(progressive_context_stream(true)));
        process(&mut client, &upgrade);
        assert!(
            updates.lock().unwrap().is_empty(),
            "a fresh context must not inherit the previous tile"
        );
        for pdu in [&seed, &upgrade] {
            process(&mut client, pdu);
        }
        assert_eq!(client.dropped_updates(), 1, "a fresh tile must allow refinements again");
        assert_eq!(updates.lock().unwrap().len(), 2);
    }

    #[test]
    fn progressive_tiles_survive_encoding_context_deletion() {
        let mut client = progressive_client();
        wire_progressive(&mut client, progressive_context_stream(true)).unwrap();
        client
            .handle_pdu(GfxPdu::DeleteEncodingContext(DeleteEncodingContextPdu {
                surface_id: 1,
                codec_context_id: 7,
            }))
            .unwrap();

        // The surface keeps its tiles and the band layout it was given, so a payload reusing
        // the id still decodes. Windows deletes a codec context as it opens the next one and
        // never repeats SYNC + CONTEXT.
        assert!(wire_progressive(&mut client, progressive_context_stream(false)).is_ok());
    }

    #[test]
    fn progressive_context_is_deleted_with_surface() {
        assert_progressive_context_is_deleted(|client| {
            client
                .handle_pdu(GfxPdu::DeleteSurface(crate::pdu::DeleteSurfacePdu { surface_id: 1 }))
                .unwrap();
            client
                .handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
                    surface_id: 1,
                    width: 64,
                    height: 64,
                    pixel_format: PixelFormat::XRgb,
                }))
                .unwrap();
        });
    }

    // ========================================================================
    // AVC420 region rectangles (regionRects) -- issue #2042
    //
    // Reference: this test adapts the region-rects harness proposed by issue
    // author se-wo (`c06_avc420_region_rects.rs`, se-wo analysis/chroma fork).
    // The recipe it encodes: an AVC420 WireToSurface1 carries an
    // `RFX_AVC420_METABLOCK` whose `regionRects` list is the set of sub-regions
    // that actually changed. Every rectangle takes its pixels from the SAME
    // (x, y) coordinate in the decoded frame (source == destination), and each
    // rectangle is a separate surface update. The destination rectangle on the
    // PDU is only the bounding box of those rects, NOT a copy source.
    //
    // Before the fix, `decode_avc420` ignored `stream.rectangles` entirely and
    // `crop_decoded_frame` copied a single bounding-box block starting at the
    // decoded frame's origin (0, 0). So a partial update whose region did not
    // sit at the origin was blitted with pixels taken from the top-left corner,
    // and the gaps between regions were overwritten -- exactly the "horizontal
    // lines + unfilled rectangle outlines" artifact reported downstream.
    // ========================================================================

    /// Deterministic stand-in for an H.264 decoder: returns a frame of the
    /// requested size where every pixel encodes its own coordinate as
    /// `R = x & 0xFF`, `G = y & 0xFF`, `B = 0`, `A = 0xFF`. This lets a test
    /// assert *which* source pixel landed in a destination region.
    struct CoordinateDecoder {
        width: u32,
        height: u32,
    }

    impl H264Decoder for CoordinateDecoder {
        fn decode(&mut self, _data: &[u8]) -> crate::decode::DecoderResult<crate::decode::DecodedFrame> {
            let (w, h) = (self.width, self.height);
            let mut data = Vec::with_capacity(usize::try_from(w * h * 4).unwrap_or(0));
            for y in 0..h {
                for x in 0..w {
                    let px = u8::try_from(x & 0xFF).unwrap_or(0);
                    let py = u8::try_from(y & 0xFF).unwrap_or(0);
                    data.extend_from_slice(&[px, py, 0, 0xFF]);
                }
            }
            Ok(crate::decode::DecodedFrame::new(data, w, h))
        }
    }

    /// Encode an `Avc420BitmapStream` wire payload from a set of region rects.
    /// The H.264 `data` is opaque to the test decoder, so a single byte suffices.
    fn encode_avc420_wire(rects: &[ExclusiveRectangle]) -> Vec<u8> {
        use ironrdp_core::{Encode as _, WriteCursor};

        use crate::pdu::QuantQuality;

        let stream = Avc420BitmapStream {
            rectangles: rects.to_vec(),
            quant_qual_vals: rects
                .iter()
                .map(|_| QuantQuality {
                    quantization_parameter: 22,
                    progressive: false,
                    quality: 100,
                })
                .collect(),
            data: &[0u8], // opaque to CoordinateDecoder
        };
        let mut buf = vec![0u8; stream.size()];
        let mut cursor = WriteCursor::new(&mut buf);
        stream.encode(&mut cursor).expect("encode avc420 stream");
        buf
    }

    #[test]
    fn avc420_size_mismatch_keeps_dimensions_in_error() {
        let mut client = GraphicsPipelineClient::new(
            Box::new(TestHandler),
            Some(Box::new(CoordinateDecoder { width: 2, height: 3 })),
        );
        let error = client
            .decode_avc420(
                1,
                &ExclusiveRectangle {
                    left: 0,
                    top: 0,
                    right: 4,
                    bottom: 5,
                },
                &encode_avc420_wire(&[]),
            )
            .unwrap_err();
        let diagnostic = format!("{error:?}");
        for dimension in ["frame_width=2", "frame_height=3", "dest_width=4", "dest_height=5"] {
            assert!(diagnostic.contains(dimension), "Missing {dimension} in {diagnostic}");
        }
    }

    /// A partial AVC420 update must draw each region rectangle from its OWN
    /// (x, y) in the decoded frame, as a separate surface update -- not blit
    /// one bounding-box block copied from the frame origin.
    #[test]
    fn avc420_region_rects_draw_from_matching_coordinates() {
        const SURFACE: u16 = 100;
        // Two disjoint regions, neither anchored at the origin, with a gap.
        let rect_a = ExclusiveRectangle {
            left: 0,
            top: 0,
            right: 16,
            bottom: 16,
        };
        let rect_b = ExclusiveRectangle {
            left: 32,
            top: 32,
            right: 48,
            bottom: 48,
        };
        // Bounding box of both regions -- this is what the server puts on the PDU.
        let dest_rect = ExclusiveRectangle {
            left: 0,
            top: 0,
            right: 48,
            bottom: 48,
        };

        let updates = Arc::new(Mutex::new(Vec::new()));
        let unhandled = Arc::new(Mutex::new(0));
        let mut client = GraphicsPipelineClient::new(
            Box::new(CapturingHandler {
                updates: Arc::clone(&updates),
                unhandled: Arc::clone(&unhandled),
            }),
            Some(Box::new(CoordinateDecoder { width: 64, height: 64 })),
        );

        client
            .handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
                surface_id: SURFACE,
                width: SURFACE,
                height: SURFACE,
                pixel_format: PixelFormat::XRgb,
            }))
            .expect("create surface");

        let bitmap_data = encode_avc420_wire(&[rect_a.clone(), rect_b.clone()]);
        client
            .handle_pdu(GfxPdu::WireToSurface1(crate::pdu::WireToSurface1Pdu {
                surface_id: SURFACE,
                codec_id: Codec1Type::Avc420,
                pixel_format: PixelFormat::XRgb,
                destination_rectangle: dest_rect,
                bitmap_data,
            }))
            .expect("decode avc420");

        assert_eq!(
            *unhandled.lock().expect("unhandled lock"),
            0,
            "AVC420 must not fall through"
        );

        let updates = updates.lock().expect("updates lock");
        // One surface update per region rectangle.
        assert_eq!(
            updates.len(),
            2,
            "expected one update per regionRect, got {}",
            updates.len()
        );

        // Helper: top-left pixel (R, G) of an update's RGBA buffer.
        let top_left = |data: &[u8]| (data[0], data[1]);
        // Region origin as the (R, G) the CoordinateDecoder stamps there.
        let origin_rg = |rect: &ExclusiveRectangle| {
            (
                u8::try_from(rect.left).expect("test coord fits u8"),
                u8::try_from(rect.top).expect("test coord fits u8"),
            )
        };

        // Match updates to regions by their dimensions and assert the source pixel.
        for (codec_id, width, height, data) in updates.iter() {
            assert_eq!(*codec_id, Codec1Type::Avc420);
            assert_eq!(*width, 16, "each region is 16 wide");
            assert_eq!(*height, 16, "each region is 16 tall");
            let (r, g) = top_left(data);
            // The CoordinateDecoder stamps R=x, G=y at each pixel. The top-left
            // pixel of a region drawn correctly must carry that region's own
            // (left, top). Region B lives at (32, 32); if the fix copies from the
            // frame origin, this would be (0, 0) instead.
            assert!(
                (r, g) == origin_rg(&rect_a) || (r, g) == origin_rg(&rect_b),
                "region top-left pixel ({r}, {g}) does not match either region origin; \
                 pixels were copied from the wrong source coordinate (issue #2042)"
            );
        }

        // Specifically prove region B is present with its correct origin pixel.
        let has_region_b = updates
            .iter()
            .any(|(_, _, _, data)| top_left(data) == origin_rg(&rect_b));
        assert!(
            has_region_b,
            "region B at (32, 32) was not drawn from its own coordinates -- \
             AVC420 partial update took pixels from the frame origin (issue #2042)"
        );
    }

    /// Pins the coordinate CONVENTION: regionRects are surface/frame-absolute,
    /// NOT relative to the destination rectangle's origin. The test above uses
    /// `dest_rect` at `(0, 0)`, where absolute and destRect-relative readings
    /// coincide and so cannot tell them apart. Here both the destination
    /// rectangle origin and the region origins are non-zero, so a regression to
    /// destRect-relative placement (subtracting `dest_rect.left/top` from the
    /// source offset) would copy from the wrong pixels and fail. MS-RDPEGFX
    /// 2.2.2.1 defines `dest_rect` as the bounding box of `regionRects`, which
    /// only holds if the rects share the surface/frame coordinate space.
    #[test]
    fn avc420_region_rects_are_surface_absolute_not_dest_relative() {
        const SURFACE: u16 = 200;
        // Two disjoint regions, both far from the origin.
        let rect_a = ExclusiveRectangle {
            left: 72,
            top: 72,
            right: 88,
            bottom: 88,
        };
        let rect_b = ExclusiveRectangle {
            left: 100,
            top: 104,
            right: 116,
            bottom: 120,
        };
        // Bounding box of both regions -- origin is non-zero (72, 72), which is
        // the whole point: a destRect-relative reading would offset by it.
        let dest_rect = ExclusiveRectangle {
            left: 72,
            top: 72,
            right: 116,
            bottom: 120,
        };

        let updates = Arc::new(Mutex::new(Vec::new()));
        let unhandled = Arc::new(Mutex::new(0));
        let mut client = GraphicsPipelineClient::new(
            Box::new(CapturingHandler {
                updates: Arc::clone(&updates),
                unhandled: Arc::clone(&unhandled),
            }),
            // Frame large enough to contain both regions at their absolute coords.
            Some(Box::new(CoordinateDecoder {
                width: 128,
                height: 128,
            })),
        );

        client
            .handle_pdu(GfxPdu::CreateSurface(crate::pdu::CreateSurfacePdu {
                surface_id: SURFACE,
                width: SURFACE,
                height: SURFACE,
                pixel_format: PixelFormat::XRgb,
            }))
            .expect("create surface");

        let bitmap_data = encode_avc420_wire(&[rect_a.clone(), rect_b.clone()]);
        client
            .handle_pdu(GfxPdu::WireToSurface1(crate::pdu::WireToSurface1Pdu {
                surface_id: SURFACE,
                codec_id: Codec1Type::Avc420,
                pixel_format: PixelFormat::XRgb,
                destination_rectangle: dest_rect,
                bitmap_data,
            }))
            .expect("decode avc420");

        assert_eq!(
            *unhandled.lock().expect("unhandled lock"),
            0,
            "AVC420 must not fall through"
        );

        let updates = updates.lock().expect("updates lock");
        assert_eq!(
            updates.len(),
            2,
            "expected one update per regionRect, got {}",
            updates.len()
        );

        let top_left = |data: &[u8]| (data[0], data[1]);
        let origin_rg = |rect: &ExclusiveRectangle| {
            (
                u8::try_from(rect.left).expect("test coord fits u8"),
                u8::try_from(rect.top).expect("test coord fits u8"),
            )
        };

        // Each update's top-left pixel must equal its region's ABSOLUTE origin
        // (72, 72) / (100, 104). A destRect-relative reading would instead copy
        // from (left - 72, top - 72), i.e. (0, 0) / (28, 32) -- caught here.
        let a_ok = updates
            .iter()
            .any(|(_, _, _, data)| top_left(data) == origin_rg(&rect_a));
        let b_ok = updates
            .iter()
            .any(|(_, _, _, data)| top_left(data) == origin_rg(&rect_b));
        assert!(
            a_ok,
            "region A must be drawn from its absolute origin (72, 72); \
             a destRect-relative reading would take (0, 0) instead (issue #2042)"
        );
        assert!(
            b_ok,
            "region B must be drawn from its absolute origin (100, 104); \
             a destRect-relative reading would take (28, 32) instead (issue #2042)"
        );
    }
    #[test]
    fn frame_ack_queue_depth_is_not_a_frame_count() {
        let mut handler = GraphicsPipelineClient::new(Box::new(TestHandler), None);
        handler.total_frames_decoded = 3;
        let responses = handler.handle_end_frame(7).unwrap();
        let bytes = ironrdp_core::encode_vec(responses[0].as_ref()).unwrap();
        let GfxPdu::FrameAcknowledge(ack) = ironrdp_core::decode::<GfxPdu>(&bytes).unwrap() else {
            panic!("expected frame ack")
        };
        assert_eq!(ack.frame_id, 7);
        assert_eq!(ack.total_frames_decoded, 4);
        assert_eq!(ack.queue_depth, QueueDepth::Unavailable);
    }
}
