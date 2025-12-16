//! Server-side EGFX implementation
//!
//! This module provides complete server-side support for the Graphics Pipeline Extension
//! (MS-RDPEGFX), enabling H.264 AVC420/AVC444 video streaming to RDP clients.
//!
//! # Protocol Compliance
//!
//! This implementation follows MS-RDPEGFX specification requirements:
//!
//! - **Capability Negotiation**: Supports V8, V8.1, V10, V10.1-V10.7
//! - **Surface Management**: Multi-surface support with proper lifecycle
//! - **Frame Flow Control**: Tracks unacknowledged frames per spec
//! - **Codec Support**: AVC420, AVC444, with extensibility for others
//!
//! # Architecture
//!
//! The server follows this message flow:
//!
//! ```text
//! Client                                  Server
//!    |                                       |
//!    |--- CapabilitiesAdvertise ------------>|
//!    |                                       | (negotiate capabilities)
//!    |<----------- CapabilitiesConfirm ------|
//!    |<----------- ResetGraphics ------------|
//!    |<----------- CreateSurface ------------|
//!    |<----------- MapSurfaceToOutput -------|
//!    |                                       |
//!    |  (For each frame:)                    |
//!    |<----------- StartFrame ---------------|
//!    |<----------- WireToSurface1/2 ---------|  (H.264 data)
//!    |<----------- EndFrame -----------------|
//!    |                                       |
//!    |--- FrameAcknowledge ----------------->|  (flow control)
//!    |--- QoeFrameAcknowledge -------------->|  (optional, V10+)
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use ironrdp_egfx::server::{GraphicsPipelineServer, GraphicsPipelineHandler};
//!
//! struct MyHandler;
//!
//! impl GraphicsPipelineHandler for MyHandler {
//!     fn capabilities_advertise(&mut self, caps: &CapabilitiesAdvertisePdu) {
//!         // Client sent capabilities
//!     }
//!
//!     fn on_ready(&mut self, negotiated: &CapabilitySet) {
//!         // Server is ready to send frames
//!     }
//! }
//!
//! let server = GraphicsPipelineServer::new(Box::new(MyHandler));
//! ```

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::gcc::Monitor;
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::{decode_err, PduResult};
use tracing::{debug, trace, warn};

use crate::pdu::{
    encode_avc420_bitmap_stream, Avc420BitmapStream, Avc420Region, Avc444BitmapStream, CacheImportOfferPdu,
    CacheImportReplyPdu, CapabilitiesAdvertisePdu, CapabilitiesConfirmPdu, CapabilitiesV103Flags,
    CapabilitiesV104Flags, CapabilitiesV107Flags, CapabilitiesV10Flags, CapabilitiesV81Flags, CapabilitiesV8Flags,
    CapabilitySet, Codec1Type, CreateSurfacePdu, DeleteSurfacePdu, Encoding, EndFramePdu, FrameAcknowledgePdu, GfxPdu,
    MapSurfaceToOutputPdu, PixelFormat, QoeFrameAcknowledgePdu, ResetGraphicsPdu, StartFramePdu, Timestamp,
    WireToSurface1Pdu,
};
use crate::CHANNEL_NAME;

// ============================================================================
// Constants
// ============================================================================

/// Default maximum frames in flight before applying backpressure
const DEFAULT_MAX_FRAMES_IN_FLIGHT: u32 = 3;

/// Special queue depth value indicating client has disabled acknowledgments
const SUSPEND_FRAME_ACK_QUEUE_DEPTH: u32 = 0xFFFFFFFF;

// ============================================================================
// Surface Management
// ============================================================================

/// Surface state tracked by server
///
/// Per MS-RDPEGFX, the server maintains an "Offscreen Surfaces ADM element"
/// which is a list of surfaces created on the client.
#[derive(Debug, Clone)]
pub struct Surface {
    /// Surface identifier (unique per session)
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

impl Surface {
    fn new(id: u16, width: u16, height: u16, pixel_format: PixelFormat) -> Self {
        Self {
            id,
            width,
            height,
            pixel_format,
            is_mapped: false,
            output_origin_x: 0,
            output_origin_y: 0,
        }
    }
}

/// Multi-surface management
///
/// Implements the "Offscreen Surfaces ADM element" from MS-RDPEGFX.
#[derive(Debug, Default)]
pub struct SurfaceManager {
    surfaces: HashMap<u16, Surface>,
    next_surface_id: u16,
}

impl SurfaceManager {
    /// Create a new surface manager
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a new surface ID
    pub fn allocate_id(&mut self) -> u16 {
        let id = self.next_surface_id;
        self.next_surface_id = self.next_surface_id.wrapping_add(1);
        id
    }

    /// Register a surface
    pub fn insert(&mut self, surface: Surface) {
        self.surfaces.insert(surface.id, surface);
    }

    /// Remove a surface
    pub fn remove(&mut self, surface_id: u16) -> Option<Surface> {
        self.surfaces.remove(&surface_id)
    }

    /// Get a surface by ID
    pub fn get(&self, surface_id: u16) -> Option<&Surface> {
        self.surfaces.get(&surface_id)
    }

    /// Get a mutable surface by ID
    pub fn get_mut(&mut self, surface_id: u16) -> Option<&mut Surface> {
        self.surfaces.get_mut(&surface_id)
    }

    /// Check if a surface exists
    pub fn contains(&self, surface_id: u16) -> bool {
        self.surfaces.contains_key(&surface_id)
    }

    /// Get all surface IDs
    pub fn surface_ids(&self) -> impl Iterator<Item = u16> + '_ {
        self.surfaces.keys().copied()
    }

    /// Clear all surfaces
    pub fn clear(&mut self) {
        self.surfaces.clear();
    }

    /// Number of surfaces
    pub fn len(&self) -> usize {
        self.surfaces.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }
}

// ============================================================================
// Frame Tracking
// ============================================================================

/// Information about a frame awaiting acknowledgment
///
/// Per MS-RDPEGFX, the server maintains an "Unacknowledged Frames ADM element"
/// which tracks frames sent but not yet acknowledged.
#[derive(Debug, Clone)]
pub struct FrameInfo {
    /// Frame identifier
    pub frame_id: u32,
    /// Frame timestamp
    pub timestamp: Timestamp,
    /// When the frame was sent
    pub sent_at: Instant,
    /// Approximate size in bytes
    pub size_bytes: usize,
}

/// Quality of Experience metrics from client
#[derive(Debug, Clone)]
pub struct QoeMetrics {
    /// Frame ID this relates to
    pub frame_id: u32,
    /// Client timestamp when decode started
    pub timestamp: u32,
    /// Time difference for serial encode (microseconds)
    pub time_diff_se: u16,
    /// Time difference for decode and render (microseconds)
    pub time_diff_dr: u16,
}

/// Frame tracking for flow control
///
/// Implements the "Unacknowledged Frames ADM element" from MS-RDPEGFX.
#[derive(Debug)]
pub struct FrameTracker {
    /// Frames sent but not yet acknowledged
    unacknowledged: HashMap<u32, FrameInfo>,
    /// Last reported client queue depth
    client_queue_depth: u32,
    /// Whether client has suspended acknowledgments
    ack_suspended: bool,
    /// Next frame ID to assign
    next_frame_id: u32,
    /// Maximum frames in flight before backpressure
    max_in_flight: u32,
    /// Total frames sent
    total_sent: u64,
    /// Total frames acknowledged
    total_acked: u64,
}

impl Default for FrameTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameTracker {
    /// Create a new frame tracker
    pub fn new() -> Self {
        Self {
            unacknowledged: HashMap::new(),
            client_queue_depth: 0,
            ack_suspended: false,
            next_frame_id: 0,
            max_in_flight: DEFAULT_MAX_FRAMES_IN_FLIGHT,
            total_sent: 0,
            total_acked: 0,
        }
    }

    /// Set maximum frames in flight
    pub fn set_max_in_flight(&mut self, max: u32) {
        self.max_in_flight = max;
    }

    /// Allocate a new frame ID and track it
    pub fn begin_frame(&mut self, timestamp: Timestamp) -> u32 {
        let frame_id = self.next_frame_id;
        self.next_frame_id = self.next_frame_id.wrapping_add(1);

        self.unacknowledged.insert(
            frame_id,
            FrameInfo {
                frame_id,
                timestamp,
                sent_at: Instant::now(),
                size_bytes: 0,
            },
        );

        self.total_sent += 1;
        frame_id
    }

    /// Update frame size after encoding
    pub fn set_frame_size(&mut self, frame_id: u32, size_bytes: usize) {
        if let Some(info) = self.unacknowledged.get_mut(&frame_id) {
            info.size_bytes = size_bytes;
        }
    }

    /// Handle frame acknowledgment from client
    pub fn acknowledge(&mut self, frame_id: u32, queue_depth: u32) -> Option<FrameInfo> {
        // Update queue depth
        if queue_depth == SUSPEND_FRAME_ACK_QUEUE_DEPTH {
            self.ack_suspended = true;
            self.client_queue_depth = 0;
        } else {
            self.ack_suspended = false;
            self.client_queue_depth = queue_depth;
        }

        // Remove and return the frame info
        let info = self.unacknowledged.remove(&frame_id);
        if info.is_some() {
            self.total_acked += 1;
        }
        info
    }

    /// Number of frames in flight
    #[expect(
        clippy::cast_possible_truncation,
        clippy::as_conversions,
        reason = "frame count will never exceed u32::MAX"
    )]
    pub fn in_flight(&self) -> u32 {
        self.unacknowledged.len() as u32
    }

    /// Check if backpressure should be applied
    pub fn should_backpressure(&self) -> bool {
        !self.ack_suspended && self.in_flight() >= self.max_in_flight
    }

    /// Get client queue depth
    pub fn client_queue_depth(&self) -> u32 {
        self.client_queue_depth
    }

    /// Check if acknowledgments are suspended
    pub fn is_ack_suspended(&self) -> bool {
        self.ack_suspended
    }

    /// Get total frames sent
    pub fn total_sent(&self) -> u64 {
        self.total_sent
    }

    /// Get total frames acknowledged
    pub fn total_acked(&self) -> u64 {
        self.total_acked
    }

    /// Clear all tracking state
    pub fn clear(&mut self) {
        self.unacknowledged.clear();
        self.client_queue_depth = 0;
        self.ack_suspended = false;
    }
}

// ============================================================================
// Capability Negotiation
// ============================================================================

/// Codec capabilities determined from negotiation
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
    /// Extract codec capabilities from a capability set
    fn from_capability_set(cap: &CapabilitySet) -> Self {
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
                // V10.3 lacks SMALL_CACHE flag
                avc420: !flags.contains(CapabilitiesV103Flags::AVC_DISABLED),
                avc444: !flags.contains(CapabilitiesV103Flags::AVC_DISABLED),
                small_cache: false,
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
            CapabilitySet::Unknown(_) => Self::default(),
        }
    }
}

/// Priority order for capability negotiation (highest to lowest)
fn capability_priority(cap: &CapabilitySet) -> u32 {
    match cap {
        CapabilitySet::V10_7 { .. } => 12,
        CapabilitySet::V10_6Err { .. } => 11,
        CapabilitySet::V10_6 { .. } => 10,
        CapabilitySet::V10_5 { .. } => 9,
        CapabilitySet::V10_4 { .. } => 8,
        CapabilitySet::V10_3 { .. } => 7,
        CapabilitySet::V10_2 { .. } => 6,
        CapabilitySet::V10_1 => 5,
        CapabilitySet::V10 { .. } => 4,
        CapabilitySet::V8_1 { .. } => 3,
        CapabilitySet::V8 { .. } => 2,
        _ => 0,
    }
}

/// Negotiate the best capability set between client and server
fn negotiate_capabilities(client_caps: &[CapabilitySet], server_caps: &[CapabilitySet]) -> Option<CapabilitySet> {
    // Sort server capabilities by priority (highest first)
    let mut server_sorted: Vec<_> = server_caps.iter().collect();
    server_sorted.sort_by_key(|cap| core::cmp::Reverse(capability_priority(cap)));

    // Find highest priority server cap that client also supports
    for server_cap in server_sorted {
        for client_cap in client_caps {
            if core::mem::discriminant(client_cap) == core::mem::discriminant(server_cap) {
                return Some(server_cap.clone());
            }
        }
    }

    None
}

// ============================================================================
// Handler Trait
// ============================================================================

/// Handler trait for server-side EGFX events
///
/// Implement this trait to receive callbacks when the EGFX channel state changes
/// or when client messages are received.
pub trait GraphicsPipelineHandler: Send {
    /// Called when the client advertises its capabilities
    ///
    /// This is informational - the server will automatically negotiate
    /// based on [`preferred_capabilities()`](Self::preferred_capabilities).
    fn capabilities_advertise(&mut self, pdu: &CapabilitiesAdvertisePdu);

    /// Called when the EGFX channel is ready to send frames
    ///
    /// At this point, capability negotiation is complete.
    /// The handler should create surfaces and start sending frames.
    fn on_ready(&mut self, negotiated: &CapabilitySet);

    /// Called when a frame has been acknowledged by the client
    ///
    /// # Arguments
    ///
    /// * `frame_id` - The acknowledged frame
    /// * `queue_depth` - Client's reported queue depth (bytes buffered)
    fn on_frame_ack(&mut self, _frame_id: u32, _queue_depth: u32) {}

    /// Called when QoE metrics are received from client (V10+)
    fn on_qoe_metrics(&mut self, _metrics: QoeMetrics) {}

    /// Called when a surface is created
    fn on_surface_created(&mut self, _surface: &Surface) {}

    /// Called when a surface is deleted
    fn on_surface_deleted(&mut self, _surface_id: u16) {}

    /// Called when the EGFX channel is closed
    fn on_close(&mut self) {}

    /// Returns the server's preferred capabilities
    ///
    /// Override this to customize codec support. The default enables
    /// AVC420/AVC444 with V10.7 and V8.1 as fallback.
    fn preferred_capabilities(&self) -> Vec<CapabilitySet> {
        vec![
            // Prefer V10.7 with AVC enabled
            CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::SMALL_CACHE,
            },
            // V10 fallback
            CapabilitySet::V10 {
                flags: CapabilitiesV10Flags::SMALL_CACHE,
            },
            // V8.1 with AVC420
            CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::AVC420_ENABLED | CapabilitiesV81Flags::SMALL_CACHE,
            },
            // V8 basic fallback
            CapabilitySet::V8 {
                flags: CapabilitiesV8Flags::SMALL_CACHE,
            },
        ]
    }

    /// Returns the maximum frames in flight before backpressure
    fn max_frames_in_flight(&self) -> u32 {
        DEFAULT_MAX_FRAMES_IN_FLIGHT
    }

    /// Called when client offers to import cached bitmaps
    ///
    /// Return the list of cache slot IDs to accept.
    /// Default rejects all (returns empty).
    fn on_cache_import_offer(&mut self, _offer: &CacheImportOfferPdu) -> Vec<u16> {
        vec![]
    }
}

// ============================================================================
// Server State Machine
// ============================================================================

/// Server state machine states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServerState {
    /// Waiting for client CapabilitiesAdvertise
    WaitingForCapabilities,
    /// Channel is ready, can send frames
    Ready,
    /// Performing a resize operation
    Resizing,
    /// Channel has been closed
    Closed,
}

// ============================================================================
// Graphics Pipeline Server
// ============================================================================

/// Server for the Graphics Pipeline Virtual Channel (EGFX)
///
/// This server handles capability negotiation, surface management,
/// and H.264 frame transmission to RDP clients per MS-RDPEGFX specification.
pub struct GraphicsPipelineServer {
    handler: Box<dyn GraphicsPipelineHandler>,

    // State management
    state: ServerState,
    negotiated_caps: Option<CapabilitySet>,
    codec_caps: CodecCapabilities,

    // Surface management (Offscreen Surfaces ADM element)
    surfaces: SurfaceManager,

    // Frame tracking (Unacknowledged Frames ADM element)
    frames: FrameTracker,

    // Graphics output buffer dimensions
    output_width: u16,
    output_height: u16,

    // Output queue for PDUs that need to be sent
    output_queue: VecDeque<GfxPdu>,
}

impl GraphicsPipelineServer {
    /// Create a new GraphicsPipelineServer
    pub fn new(handler: Box<dyn GraphicsPipelineHandler>) -> Self {
        let max_frames = handler.max_frames_in_flight();
        let mut frames = FrameTracker::new();
        frames.set_max_in_flight(max_frames);

        Self {
            handler,
            state: ServerState::WaitingForCapabilities,
            negotiated_caps: None,
            codec_caps: CodecCapabilities::default(),
            surfaces: SurfaceManager::new(),
            frames,
            output_width: 0,
            output_height: 0,
            output_queue: VecDeque::new(),
        }
    }

    // ========================================================================
    // State Queries
    // ========================================================================

    /// Check if the server is ready to send frames
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.state == ServerState::Ready
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

    /// Check if AVC420 (H.264 4:2:0) is available
    #[must_use]
    pub fn supports_avc420(&self) -> bool {
        self.codec_caps.avc420
    }

    /// Check if AVC444 (H.264 4:4:4) is available
    #[must_use]
    pub fn supports_avc444(&self) -> bool {
        self.codec_caps.avc444
    }

    /// Get the graphics output buffer dimensions
    #[must_use]
    pub fn output_dimensions(&self) -> (u16, u16) {
        (self.output_width, self.output_height)
    }

    // ========================================================================
    // Surface Management
    // ========================================================================

    /// Create a new surface
    ///
    /// Queues CreateSurface PDU and returns the surface ID.
    /// Returns `None` if not ready.
    pub fn create_surface(&mut self, width: u16, height: u16) -> Option<u16> {
        self.create_surface_with_format(width, height, PixelFormat::XRgb)
    }

    /// Create a new surface with specific pixel format
    pub fn create_surface_with_format(&mut self, width: u16, height: u16, pixel_format: PixelFormat) -> Option<u16> {
        if self.state != ServerState::Ready && self.state != ServerState::Resizing {
            return None;
        }

        let surface_id = self.surfaces.allocate_id();
        let surface = Surface::new(surface_id, width, height, pixel_format);

        // Queue CreateSurface PDU
        self.output_queue.push_back(GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id,
            width,
            height,
            pixel_format,
        }));

        self.handler.on_surface_created(&surface);
        self.surfaces.insert(surface);

        debug!(surface_id, width, height, ?pixel_format, "Created surface");
        Some(surface_id)
    }

    /// Delete a surface
    ///
    /// Queues DeleteSurface PDU. Returns `false` if surface doesn't exist.
    pub fn delete_surface(&mut self, surface_id: u16) -> bool {
        if self.surfaces.remove(surface_id).is_none() {
            return false;
        }

        // Queue DeleteSurface PDU
        self.output_queue
            .push_back(GfxPdu::DeleteSurface(DeleteSurfacePdu { surface_id }));

        self.handler.on_surface_deleted(surface_id);
        debug!(surface_id, "Deleted surface");
        true
    }

    /// Map a surface to the graphics output buffer
    pub fn map_surface_to_output(&mut self, surface_id: u16, origin_x: u32, origin_y: u32) -> bool {
        let Some(surface) = self.surfaces.get_mut(surface_id) else {
            return false;
        };

        surface.is_mapped = true;
        surface.output_origin_x = origin_x;
        surface.output_origin_y = origin_y;

        self.output_queue
            .push_back(GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
                surface_id,
                output_origin_x: origin_x,
                output_origin_y: origin_y,
            }));

        debug!(surface_id, origin_x, origin_y, "Mapped surface to output");
        true
    }

    /// Get a surface by ID
    #[must_use]
    pub fn get_surface(&self, surface_id: u16) -> Option<&Surface> {
        self.surfaces.get(surface_id)
    }

    /// Get all surface IDs
    pub fn surface_ids(&self) -> impl Iterator<Item = u16> + '_ {
        self.surfaces.surface_ids()
    }

    // ========================================================================
    // Resize Handling
    // ========================================================================

    /// Resize the graphics output buffer
    ///
    /// This initiates a resize sequence:
    /// 1. Sends ResetGraphics with new dimensions
    /// 2. Deletes existing surfaces
    /// 3. Transitions to Ready state
    ///
    /// After calling this, create new surfaces for the new dimensions.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.resize_with_monitors(width, height, Vec::new());
    }

    /// Resize with explicit monitor configuration
    pub fn resize_with_monitors(&mut self, width: u16, height: u16, monitors: Vec<Monitor>) {
        if self.state != ServerState::Ready {
            debug!("Cannot resize: not in Ready state");
            return;
        }

        debug!(width, height, monitors = monitors.len(), "Initiating resize");

        self.state = ServerState::Resizing;
        self.output_width = width;
        self.output_height = height;

        // Delete all existing surfaces
        let surface_ids: Vec<_> = self.surfaces.surface_ids().collect();
        for id in surface_ids {
            self.delete_surface(id);
        }

        // Clear frame tracking
        self.frames.clear();

        // Send ResetGraphics
        self.output_queue.push_back(GfxPdu::ResetGraphics(ResetGraphicsPdu {
            width: u32::from(width),
            height: u32::from(height),
            monitors,
        }));

        // Return to Ready state
        self.state = ServerState::Ready;
    }

    // ========================================================================
    // Flow Control
    // ========================================================================

    /// Check if backpressure should be applied
    ///
    /// Returns `true` if too many frames are in flight and the caller
    /// should drop or delay new frames.
    #[must_use]
    pub fn should_backpressure(&self) -> bool {
        self.frames.should_backpressure()
    }

    /// Get the number of frames currently in flight (awaiting ACK)
    #[must_use]
    pub fn frames_in_flight(&self) -> u32 {
        self.frames.in_flight()
    }

    /// Get the last reported client queue depth
    #[must_use]
    pub fn client_queue_depth(&self) -> u32 {
        self.frames.client_queue_depth()
    }

    /// Set the maximum frames in flight before backpressure
    pub fn set_max_frames_in_flight(&mut self, max: u32) {
        self.frames.set_max_in_flight(max);
    }

    // ========================================================================
    // Frame Sending
    // ========================================================================

    /// Convert timestamp in milliseconds to Timestamp struct
    #[expect(
        clippy::as_conversions,
        reason = "arithmetic results bounded and fit in target types"
    )]
    fn make_timestamp(timestamp_ms: u32) -> Timestamp {
        Timestamp {
            milliseconds: (timestamp_ms % 1000) as u16,
            seconds: ((timestamp_ms / 1000) % 60) as u8,
            minutes: ((timestamp_ms / 60000) % 60) as u8,
            hours: ((timestamp_ms / 3600000) % 24) as u16,
        }
    }

    /// Compute bounding rectangle from regions
    fn compute_dest_rect(regions: &[Avc420Region], default_width: u16, default_height: u16) -> InclusiveRectangle {
        if let Some(first) = regions.first() {
            let mut left = first.left;
            let mut top = first.top;
            let mut right = first.right;
            let mut bottom = first.bottom;

            for r in regions.iter().skip(1) {
                left = left.min(r.left);
                top = top.min(r.top);
                right = right.max(r.right);
                bottom = bottom.max(r.bottom);
            }

            InclusiveRectangle {
                left,
                top,
                right,
                bottom,
            }
        } else {
            InclusiveRectangle {
                left: 0,
                top: 0,
                right: default_width.saturating_sub(1),
                bottom: default_height.saturating_sub(1),
            }
        }
    }

    /// Queue an H.264 AVC420 frame for transmission
    ///
    /// # Arguments
    ///
    /// * `surface_id` - Target surface
    /// * `h264_data` - H.264 encoded data in AVC format (use `annex_b_to_avc` if needed)
    /// * `regions` - List of regions describing the frame
    /// * `timestamp_ms` - Frame timestamp in milliseconds
    ///
    /// # Returns
    ///
    /// `Some(frame_id)` if the frame was queued, `None` if backpressure is active,
    /// server is not ready, or AVC420 is not supported.
    pub fn send_avc420_frame(
        &mut self,
        surface_id: u16,
        h264_data: &[u8],
        regions: &[Avc420Region],
        timestamp_ms: u32,
    ) -> Option<u32> {
        if !self.is_ready() {
            debug!("EGFX not ready, dropping frame");
            return None;
        }

        if !self.supports_avc420() {
            debug!("AVC420 not supported, dropping frame");
            return None;
        }

        if self.should_backpressure() {
            trace!(frames_in_flight = self.frames.in_flight(), "EGFX backpressure active");
            return None;
        }

        let Some(surface) = self.surfaces.get(surface_id) else {
            debug!(surface_id, "Surface not found, dropping frame");
            return None;
        };

        let timestamp = Self::make_timestamp(timestamp_ms);
        let frame_id = self.frames.begin_frame(timestamp);

        // Build the bitmap data
        let bitmap_data = encode_avc420_bitmap_stream(regions, h264_data);

        // Determine destination rectangle
        let dest_rect = Self::compute_dest_rect(regions, surface.width, surface.height);

        // Queue the frame PDUs
        self.output_queue
            .push_back(GfxPdu::StartFrame(StartFramePdu { timestamp, frame_id }));

        self.output_queue.push_back(GfxPdu::WireToSurface1(WireToSurface1Pdu {
            surface_id,
            codec_id: Codec1Type::Avc420,
            pixel_format: surface.pixel_format,
            destination_rectangle: dest_rect,
            bitmap_data,
        }));

        self.output_queue.push_back(GfxPdu::EndFrame(EndFramePdu { frame_id }));

        trace!(frame_id, surface_id, "Queued AVC420 frame");
        Some(frame_id)
    }

    /// Queue an H.264 AVC444 frame for transmission
    ///
    /// AVC444 uses two streams: one for luma (Y) and one for chroma (UV).
    /// If only luma data is provided, set `chroma_data` to `None`.
    ///
    /// # Arguments
    ///
    /// * `surface_id` - Target surface
    /// * `luma_data` - H.264 encoded luma (Y) data in AVC format
    /// * `luma_regions` - Regions for luma stream
    /// * `chroma_data` - Optional H.264 encoded chroma (UV) data
    /// * `chroma_regions` - Regions for chroma stream (required if chroma_data provided)
    /// * `timestamp_ms` - Frame timestamp in milliseconds
    ///
    /// # Returns
    ///
    /// `Some(frame_id)` if the frame was queued, `None` if not supported or backpressured.
    pub fn send_avc444_frame(
        &mut self,
        surface_id: u16,
        luma_data: &[u8],
        luma_regions: &[Avc420Region],
        chroma_data: Option<&[u8]>,
        chroma_regions: Option<&[Avc420Region]>,
        timestamp_ms: u32,
    ) -> Option<u32> {
        if !self.is_ready() {
            debug!("EGFX not ready, dropping frame");
            return None;
        }

        if !self.supports_avc444() {
            debug!("AVC444 not supported, dropping frame");
            return None;
        }

        if self.should_backpressure() {
            trace!(frames_in_flight = self.frames.in_flight(), "EGFX backpressure active");
            return None;
        }

        let Some(surface) = self.surfaces.get(surface_id) else {
            debug!(surface_id, "Surface not found, dropping frame");
            return None;
        };

        let timestamp = Self::make_timestamp(timestamp_ms);
        let frame_id = self.frames.begin_frame(timestamp);

        // Build luma stream
        let luma_rectangles: Vec<_> = luma_regions.iter().map(Avc420Region::to_rectangle).collect();
        let luma_quant_vals: Vec<_> = luma_regions.iter().map(Avc420Region::to_quant_quality).collect();

        let stream1 = Avc420BitmapStream {
            rectangles: luma_rectangles,
            quant_qual_vals: luma_quant_vals,
            data: luma_data,
        };

        // Build chroma stream if provided
        let (encoding, stream2) = if let (Some(chroma), Some(chroma_regs)) = (chroma_data, chroma_regions) {
            let chroma_rectangles: Vec<_> = chroma_regs.iter().map(Avc420Region::to_rectangle).collect();
            let chroma_quant_vals: Vec<_> = chroma_regs.iter().map(Avc420Region::to_quant_quality).collect();

            (
                Encoding::LUMA_AND_CHROMA,
                Some(Avc420BitmapStream {
                    rectangles: chroma_rectangles,
                    quant_qual_vals: chroma_quant_vals,
                    data: chroma,
                }),
            )
        } else {
            (Encoding::LUMA, None)
        };

        let avc444_stream = Avc444BitmapStream {
            encoding,
            stream1,
            stream2,
        };

        // Encode the AVC444 stream
        let bitmap_data = encode_avc444_bitmap_stream(&avc444_stream);

        // Determine destination rectangle
        let dest_rect = Self::compute_dest_rect(luma_regions, surface.width, surface.height);

        // Queue the frame PDUs
        self.output_queue
            .push_back(GfxPdu::StartFrame(StartFramePdu { timestamp, frame_id }));

        self.output_queue.push_back(GfxPdu::WireToSurface1(WireToSurface1Pdu {
            surface_id,
            codec_id: Codec1Type::Avc444,
            pixel_format: surface.pixel_format,
            destination_rectangle: dest_rect,
            bitmap_data,
        }));

        self.output_queue.push_back(GfxPdu::EndFrame(EndFramePdu { frame_id }));

        trace!(frame_id, surface_id, "Queued AVC444 frame");
        Some(frame_id)
    }

    // ========================================================================
    // Output Management
    // ========================================================================

    /// Drain the output queue and return PDUs to send
    ///
    /// Call this method to get pending PDUs that need to be sent to the client.
    #[expect(clippy::as_conversions, reason = "Box<T> to Box<dyn Trait> coercion")]
    pub fn drain_output(&mut self) -> Vec<DvcMessage> {
        self.output_queue
            .drain(..)
            .map(|pdu| Box::new(pdu) as DvcMessage)
            .collect()
    }

    /// Check if there are pending PDUs to send
    #[must_use]
    pub fn has_pending_output(&self) -> bool {
        !self.output_queue.is_empty()
    }

    // ========================================================================
    // Internal Message Handlers
    // ========================================================================

    /// Handle capability negotiation
    fn handle_capabilities_advertise(&mut self, pdu: CapabilitiesAdvertisePdu) {
        debug!(?pdu, "Received CapabilitiesAdvertise");

        // Notify handler
        self.handler.capabilities_advertise(&pdu);

        // Get server's preferred capabilities
        let server_caps = self.handler.preferred_capabilities();

        // Negotiate best match
        let negotiated = negotiate_capabilities(&pdu.0, &server_caps).unwrap_or_else(|| {
            // Fallback to V8.1 with AVC420
            warn!("No matching capabilities, falling back to V8.1");
            CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::AVC420_ENABLED,
            }
        });

        debug!(?negotiated, "Negotiated capabilities");

        // Extract codec capabilities
        self.codec_caps = CodecCapabilities::from_capability_set(&negotiated);
        self.negotiated_caps = Some(negotiated.clone());

        // Queue CapabilitiesConfirm
        self.output_queue
            .push_back(GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu(negotiated.clone())));

        // Transition to ready state
        self.state = ServerState::Ready;

        // Notify handler
        self.handler.on_ready(&negotiated);

        debug!(
            avc420 = self.codec_caps.avc420,
            avc444 = self.codec_caps.avc444,
            "EGFX server ready"
        );
    }

    /// Handle frame acknowledgment
    fn handle_frame_acknowledge(&mut self, pdu: FrameAcknowledgePdu) {
        trace!(?pdu, "Received FrameAcknowledge");

        // Convert QueueDepth enum to u32 for tracking
        let queue_depth_u32 = pdu.queue_depth.to_u32();

        if let Some(info) = self.frames.acknowledge(pdu.frame_id, queue_depth_u32) {
            let latency = info.sent_at.elapsed();
            trace!(frame_id = pdu.frame_id, ?latency, "Frame acknowledged");
        }

        self.handler.on_frame_ack(pdu.frame_id, queue_depth_u32);
    }

    /// Handle QoE frame acknowledgment
    fn handle_qoe_frame_acknowledge(&mut self, pdu: QoeFrameAcknowledgePdu) {
        trace!(?pdu, "Received QoeFrameAcknowledge");

        let metrics = QoeMetrics {
            frame_id: pdu.frame_id,
            timestamp: pdu.timestamp,
            time_diff_se: pdu.time_diff_se,
            time_diff_dr: pdu.time_diff_dr,
        };

        self.handler.on_qoe_metrics(metrics);
    }

    /// Handle cache import offer
    fn handle_cache_import_offer(&mut self, pdu: CacheImportOfferPdu) {
        debug!(entries = pdu.cache_entries.len(), "Received CacheImportOffer");

        // Ask handler which entries to accept
        let accepted = self.handler.on_cache_import_offer(&pdu);

        // Send reply
        self.output_queue
            .push_back(GfxPdu::CacheImportReply(CacheImportReplyPdu { cache_slots: accepted }));
    }
}

impl_as_any!(GraphicsPipelineServer);

impl DvcProcessor for GraphicsPipelineServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        debug!("EGFX channel started");
        // Server doesn't send anything at start - waits for client CapabilitiesAdvertise
        Ok(vec![])
    }

    fn close(&mut self, _channel_id: u32) {
        debug!("EGFX channel closed");
        self.state = ServerState::Closed;
        self.handler.on_close();
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu = decode(payload).map_err(|e| decode_err!(e))?;

        match pdu {
            GfxPdu::CapabilitiesAdvertise(pdu) => {
                self.handle_capabilities_advertise(pdu);
            }
            GfxPdu::FrameAcknowledge(pdu) => {
                self.handle_frame_acknowledge(pdu);
            }
            GfxPdu::QoeFrameAcknowledge(pdu) => {
                self.handle_qoe_frame_acknowledge(pdu);
            }
            GfxPdu::CacheImportOffer(pdu) => {
                self.handle_cache_import_offer(pdu);
            }
            _ => {
                warn!(?pdu, "Unhandled client GFX PDU");
            }
        }

        // Return any queued output
        Ok(self.drain_output())
    }
}

impl DvcServerProcessor for GraphicsPipelineServer {}

// ============================================================================
// AVC444 Encoding Helper
// ============================================================================

/// Encode an AVC444 bitmap stream to bytes
fn encode_avc444_bitmap_stream(stream: &Avc444BitmapStream<'_>) -> Vec<u8> {
    use ironrdp_pdu::{Encode as _, WriteCursor};

    let size = stream.size();
    let mut buf = vec![0u8; size];
    let mut cursor = WriteCursor::new(&mut buf);

    stream
        .encode(&mut cursor)
        .expect("encode_avc444_bitmap_stream: encoding failed");

    buf
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    struct TestHandler {
        ready: bool,
        negotiated: Option<CapabilitySet>,
        acked_frames: Vec<(u32, u32)>,
        qoe_metrics: Vec<QoeMetrics>,
        created_surfaces: Vec<u16>,
        deleted_surfaces: Vec<u16>,
    }

    impl TestHandler {
        fn new() -> Self {
            Self {
                ready: false,
                negotiated: None,
                acked_frames: Vec::new(),
                qoe_metrics: Vec::new(),
                created_surfaces: Vec::new(),
                deleted_surfaces: Vec::new(),
            }
        }
    }

    impl GraphicsPipelineHandler for TestHandler {
        fn capabilities_advertise(&mut self, _pdu: &CapabilitiesAdvertisePdu) {}

        fn on_ready(&mut self, negotiated: &CapabilitySet) {
            self.ready = true;
            self.negotiated = Some(negotiated.clone());
        }

        fn on_frame_ack(&mut self, frame_id: u32, queue_depth: u32) {
            self.acked_frames.push((frame_id, queue_depth));
        }

        fn on_qoe_metrics(&mut self, metrics: QoeMetrics) {
            self.qoe_metrics.push(metrics);
        }

        fn on_surface_created(&mut self, surface: &Surface) {
            self.created_surfaces.push(surface.id);
        }

        fn on_surface_deleted(&mut self, surface_id: u16) {
            self.deleted_surfaces.push(surface_id);
        }
    }

    #[test]
    fn test_server_creation() {
        let handler = Box::new(TestHandler::new());
        let server = GraphicsPipelineServer::new(handler);

        assert!(!server.is_ready());
        assert_eq!(server.frames_in_flight(), 0);
        assert!(!server.supports_avc420());
        assert!(!server.supports_avc444());
    }

    #[test]
    fn test_surface_manager() {
        let mut manager = SurfaceManager::new();

        // Allocate IDs
        let id1 = manager.allocate_id();
        let id2 = manager.allocate_id();
        assert_ne!(id1, id2);

        // Insert surfaces
        manager.insert(Surface::new(id1, 1920, 1080, PixelFormat::XRgb));
        manager.insert(Surface::new(id2, 800, 600, PixelFormat::XRgb));

        assert_eq!(manager.len(), 2);
        assert!(manager.contains(id1));
        assert!(manager.contains(id2));

        // Get surface
        let surface = manager.get(id1).unwrap();
        assert_eq!(surface.width, 1920);
        assert_eq!(surface.height, 1080);

        // Remove surface
        manager.remove(id1);
        assert_eq!(manager.len(), 1);
        assert!(!manager.contains(id1));
    }

    #[test]
    fn test_frame_tracker() {
        let mut tracker = FrameTracker::new();
        tracker.set_max_in_flight(2);

        // Begin frames
        let ts = Timestamp {
            milliseconds: 0,
            seconds: 0,
            minutes: 0,
            hours: 0,
        };
        let frame1 = tracker.begin_frame(ts);
        let frame2 = tracker.begin_frame(ts);

        assert_eq!(tracker.in_flight(), 2);
        assert!(tracker.should_backpressure());

        // Acknowledge one
        let info = tracker.acknowledge(frame1, 100);
        assert!(info.is_some());
        assert_eq!(tracker.in_flight(), 1);
        assert!(!tracker.should_backpressure());
        assert_eq!(tracker.client_queue_depth(), 100);

        // Acknowledge the other
        tracker.acknowledge(frame2, 50);
        assert_eq!(tracker.in_flight(), 0);
        assert_eq!(tracker.client_queue_depth(), 50);
    }

    #[test]
    fn test_frame_tracker_ack_suspend() {
        let mut tracker = FrameTracker::new();
        tracker.set_max_in_flight(1);

        let ts = Timestamp {
            milliseconds: 0,
            seconds: 0,
            minutes: 0,
            hours: 0,
        };
        let frame1 = tracker.begin_frame(ts);

        assert!(tracker.should_backpressure());

        // Suspend acknowledgments
        tracker.acknowledge(frame1, SUSPEND_FRAME_ACK_QUEUE_DEPTH);
        assert!(tracker.is_ack_suspended());
        assert!(!tracker.should_backpressure()); // Should not backpressure when suspended
    }

    #[test]
    fn test_capability_negotiation() {
        // Client advertises V8.1 and V10
        let client_caps = vec![
            CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::AVC420_ENABLED,
            },
            CapabilitySet::V10 {
                flags: CapabilitiesV10Flags::empty(),
            },
        ];

        // Server prefers V10.7, V10, V8.1
        let server_caps = vec![
            CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::SMALL_CACHE,
            },
            CapabilitySet::V10 {
                flags: CapabilitiesV10Flags::SMALL_CACHE,
            },
            CapabilitySet::V8_1 {
                flags: CapabilitiesV81Flags::AVC420_ENABLED,
            },
        ];

        let negotiated = negotiate_capabilities(&client_caps, &server_caps);
        assert!(negotiated.is_some());

        // Should select V10 (highest common version)
        let cap = negotiated.unwrap();
        assert!(matches!(cap, CapabilitySet::V10 { .. }));
    }

    #[test]
    fn test_codec_capabilities() {
        // V8.1 with AVC420
        let cap = CapabilitySet::V8_1 {
            flags: CapabilitiesV81Flags::AVC420_ENABLED,
        };
        let codec = CodecCapabilities::from_capability_set(&cap);
        assert!(codec.avc420);
        assert!(!codec.avc444);

        // V10 with AVC enabled
        let cap = CapabilitySet::V10 {
            flags: CapabilitiesV10Flags::SMALL_CACHE,
        };
        let codec = CodecCapabilities::from_capability_set(&cap);
        assert!(codec.avc420);
        assert!(codec.avc444);

        // V10 with AVC disabled
        let cap = CapabilitySet::V10 {
            flags: CapabilitiesV10Flags::AVC_DISABLED,
        };
        let codec = CodecCapabilities::from_capability_set(&cap);
        assert!(!codec.avc420);
        assert!(!codec.avc444);
    }

    #[test]
    fn test_server_not_ready() {
        let handler = Box::new(TestHandler::new());
        let mut server = GraphicsPipelineServer::new(handler);

        // Should return None when not ready
        let h264_data = vec![0x00, 0x00, 0x00, 0x01, 0x67];
        let regions = vec![Avc420Region::full_frame(1920, 1080, 22)];

        let result = server.send_avc420_frame(0, &h264_data, &regions, 0);
        assert!(result.is_none());
    }

    #[test]
    fn test_backpressure() {
        let handler = Box::new(TestHandler::new());
        let mut server = GraphicsPipelineServer::new(handler);

        // Force ready state for testing
        server.state = ServerState::Ready;
        server.codec_caps.avc420 = true;
        server.set_max_frames_in_flight(2);

        // Create a surface
        let surface_id = server.surfaces.allocate_id();
        server
            .surfaces
            .insert(Surface::new(surface_id, 1920, 1080, PixelFormat::XRgb));

        let h264_data = vec![0x00, 0x00, 0x00, 0x01, 0x67];
        let regions = vec![Avc420Region::full_frame(1920, 1080, 22)];

        // First two frames should succeed
        assert!(server.send_avc420_frame(surface_id, &h264_data, &regions, 0).is_some());
        assert!(server.send_avc420_frame(surface_id, &h264_data, &regions, 16).is_some());

        // Third should fail due to backpressure
        assert!(server.should_backpressure());
        assert!(server.send_avc420_frame(surface_id, &h264_data, &regions, 33).is_none());
    }

    #[test]
    fn test_surface_lifecycle() {
        let handler = Box::new(TestHandler::new());
        let mut server = GraphicsPipelineServer::new(handler);

        // Force ready state
        server.state = ServerState::Ready;

        // Create surface
        let id = server.create_surface(1920, 1080);
        assert!(id.is_some());
        let surface_id = id.unwrap();

        // Verify surface exists
        assert!(server.get_surface(surface_id).is_some());
        let surface = server.get_surface(surface_id).unwrap();
        assert_eq!(surface.width, 1920);
        assert_eq!(surface.height, 1080);

        // Map to output
        assert!(server.map_surface_to_output(surface_id, 0, 0));
        let surface = server.get_surface(surface_id).unwrap();
        assert!(surface.is_mapped);

        // Delete surface
        assert!(server.delete_surface(surface_id));
        assert!(server.get_surface(surface_id).is_none());

        // Should have queued CreateSurface, MapSurfaceToOutput, DeleteSurface PDUs
        let output = server.drain_output();
        assert_eq!(output.len(), 3);
    }

    #[test]
    fn test_resize() {
        let handler = Box::new(TestHandler::new());
        let mut server = GraphicsPipelineServer::new(handler);

        // Force ready state with a surface
        server.state = ServerState::Ready;
        server.output_width = 1920;
        server.output_height = 1080;
        let surface_id = server.surfaces.allocate_id();
        server
            .surfaces
            .insert(Surface::new(surface_id, 1920, 1080, PixelFormat::XRgb));

        // Resize
        server.resize(2560, 1440);

        // Surface should be deleted
        assert!(server.get_surface(surface_id).is_none());

        // Output dimensions should be updated
        assert_eq!(server.output_dimensions(), (2560, 1440));

        // Should have queued DeleteSurface and ResetGraphics
        let output = server.drain_output();
        assert!(output.len() >= 2);
    }
}
