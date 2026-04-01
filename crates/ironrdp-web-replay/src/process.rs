//! PDU processing for RDP session replay.
//!
//! Provides a simplified processing pipeline for replay

use std::sync::Arc;

use ironrdp_core::{WriteBuf, decode};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_pdu::Action;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent};
use ironrdp_pdu::mcs::McsMessage;
use ironrdp_pdu::rdp::capability_sets::CapabilitySet;
use ironrdp_pdu::rdp::headers::{ShareControlHeader, ShareControlPdu};
use ironrdp_pdu::x224::X224;
use ironrdp_session::fast_path;
pub use ironrdp_session::fast_path::UpdateKind;
use ironrdp_session::image::DecodedImage;

use crate::PduSource;
use crate::buffer::PduBuffer;

/// Configuration for building a [`ReplayProcessor`].
///
/// Defaults reflect the most common values found in RDP recordings.
/// A fully correct implementation should extract these from the recorded
/// MCS Connect Response and Server Demand Active exchange.
#[derive(Debug, Clone)]
pub struct ReplayProcessorConfig {
    /// MCS I/O channel ID from the server's MCS Connect Response.
    ///
    /// Default: `1003` — the most common value assigned by RDP servers.
    pub io_channel_id: u16,

    /// MCS user channel ID assigned during the MCS Attach-User exchange.
    ///
    /// Default: `1002` — the most common value assigned by RDP servers.
    pub user_channel_id: u16,

    /// Share ID from the Server Demand Active PDU.
    ///
    /// Default: `0x0001_0000` — the most common value assigned by RDP servers.
    pub share_id: u32,
}

impl Default for ReplayProcessorConfig {
    fn default() -> Self {
        Self {
            io_channel_id: 1003,
            user_channel_id: 1002,
            share_id: 0x0001_0000,
        }
    }
}

/// Current pointer state for UI synchronization after seeking.
///
/// Defined locally because [`DecodedImage`] does not currently expose pointer
/// state. Could be upstreamed to `ironrdp-session` in the future.
#[derive(Debug, Clone)]
pub enum PointerState {
    /// Custom pointer bitmap
    Bitmap(Arc<DecodedPointer>),
    /// Use system default pointer
    Default,
    /// Pointer is hidden
    Hidden,
}

/// Classification of replay processing errors.
#[non_exhaustive]
#[derive(Debug)]
pub enum ReplayErrorKind {
    /// Wraps a decode error from `ironrdp_core`
    Decode(ironrdp_core::DecodeError),
    /// Wraps a session processing error
    Session(Box<ironrdp_session::SessionError>),
}

impl core::fmt::Display for ReplayErrorKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Decode(_) => write!(f, "decode error"),
            Self::Session(_) => write!(f, "session error"),
        }
    }
}

impl core::error::Error for ReplayErrorKind {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            Self::Session(e) => Some(e.as_ref()),
        }
    }
}

/// Error type for replay PDU processing.
pub type ReplayError = ironrdp_error::Error<ReplayErrorKind>;

/// Convenience result alias for replay operations.
pub type ReplayResult<T> = Result<T, ReplayError>;

/// Extension trait providing named constructors for [`ReplayError`].
pub trait ReplayErrorExt {
    fn decode(error: ironrdp_core::DecodeError) -> Self;
    fn session(error: ironrdp_session::SessionError) -> Self;
}

impl ReplayErrorExt for ReplayError {
    fn decode(error: ironrdp_core::DecodeError) -> Self {
        Self::new("replay", ReplayErrorKind::Decode(error))
    }

    fn session(error: ironrdp_session::SessionError) -> Self {
        Self::new("replay", ReplayErrorKind::Session(Box::new(error)))
    }
}

/// Outcome of processing a single PDU
#[derive(Debug)]
pub enum ProcessResult {
    /// FastPath result (graphics, pointers) - delegates to upstream type
    FastPath(UpdateKind),

    /// Pointer position from client input recording
    ClientPointerPosition { x: u16, y: u16 },

    /// Desktop resolution changed - caller should resize framebuffer
    ResolutionChanged { width: u16, height: u16 },

    /// Session deactivated - expect reactivation with new resolution
    SessionDeactivated,

    /// Session ended
    SessionEnded,
}

impl ProcessResult {
    /// Whether this result represents a visual change (graphics, pointer, resolution).
    /// Non-visual results (e.g. session lifecycle) don't require canvas redraws.
    pub fn is_visual(&self) -> bool {
        !matches!(
            self,
            ProcessResult::FastPath(UpdateKind::None) | ProcessResult::SessionEnded | ProcessResult::SessionDeactivated
        )
    }
}

/// Accumulated result of processing PDUs up to a target timestamp.
///
/// Returned by [`ReplayProcessor::process_till()`]. Contains all state
/// changes from the processed PDUs, allowing the caller to apply
/// canvas-specific side effects (resize, blit, cursor compositing).
#[derive(Debug)]
pub struct ProcessTillResult {
    /// Number of PDUs successfully processed from the buffer.
    /// Errored PDUs are consumed but counted in `errors.len()` instead.
    /// Total PDUs drained from the buffer = `pdus_processed + errors.len()`.
    pub pdus_processed: u32,
    /// Whether any PDU triggered a resolution change.
    pub resolution_changed: bool,
    /// New resolution if `resolution_changed` is true.
    pub new_resolution: Option<(u16, u16)>,
    /// Whether a SessionEnded PDU was encountered.
    pub session_ended: bool,
    /// Whether any visual result was produced (requires canvas redraw).
    pub frame_dirty: bool,
    /// Last mouse position from client or server pointer events, if any.
    pub last_mouse_position: Option<(u16, u16)>,
    /// Accumulated processing errors (logged, not fatal — PDU skipped).
    pub errors: Vec<ReplayError>,
}

/// Stateful processor for replay PDU handling
pub struct ReplayProcessor {
    /// FastPath processor for graphics/pointer handling
    fast_path_processor: fast_path::Processor,

    /// When false, suppresses visual results (GraphicsUpdate, Pointer*) during seeking.
    /// Session state results (ResolutionChanged, SessionDeactivated, SessionEnded) are always emitted.
    update_canvas: bool,

    /// Current pointer state, tracked for post-seek UI synchronization
    current_pointer: PointerState,

    /// Reusable response buffer for `fast_path_processor.process()`.
    ///
    /// In replay mode, no responses are sent, so this is cleared and reused
    /// on each call to avoid repeated allocation.
    response_buffer: WriteBuf,
}

impl ReplayProcessor {
    pub fn new(config: &ReplayProcessorConfig) -> Self {
        let fast_path_processor = fast_path::ProcessorBuilder {
            io_channel_id: config.io_channel_id,
            user_channel_id: config.user_channel_id,
            share_id: config.share_id,
            enable_server_pointer: true,
            pointer_software_rendering: false,
            bulk_decompressor: None,
        }
        .build();

        Self {
            fast_path_processor,
            update_canvas: true,
            current_pointer: PointerState::Default,
            response_buffer: WriteBuf::new(),
        }
    }

    /// Set whether to emit visual results (GraphicsUpdate, Pointer*).
    /// Set to `false` during seeking to suppress canvas updates.
    pub fn set_update_canvas(&mut self, update: bool) {
        self.update_canvas = update;
    }

    /// Returns whether visual results are being emitted.
    pub fn update_canvas(&self) -> bool {
        self.update_canvas
    }

    /// Returns the current pointer state for UI synchronization after seeking.
    pub fn current_pointer_state(&self) -> &PointerState {
        &self.current_pointer
    }

    /// Process all buffered PDUs up to `target_ms`.
    ///
    /// Drains the buffer, processes each PDU, and returns accumulated results.
    /// Errors from individual PDUs are collected in `ProcessTillResult::errors`
    /// and the offending PDU is skipped — processing continues with the next PDU.
    ///
    /// The caller is responsible for:
    /// - Resizing the canvas to match `new_resolution` (if `resolution_changed`).
    ///   The `DecodedImage` is already reallocated internally.
    /// - Blitting the framebuffer to the canvas (if `frame_dirty && update_canvas`)
    /// - Updating cursor visual state from pointer results
    ///
    /// `pdus_processed` counts only successfully processed PDUs. Errored PDUs
    /// are consumed from the buffer but counted in `errors.len()` instead.
    /// Total PDUs drained = `pdus_processed + errors.len()`.
    pub fn process_till(
        &mut self,
        buffer: &mut PduBuffer,
        image: &mut DecodedImage,
        target_ms: f64,
    ) -> ProcessTillResult {
        let mut pdus_processed: u32 = 0;
        let mut resolution_changed = false;
        let mut new_resolution = None;
        let mut session_ended = false;
        let mut frame_dirty = false;
        let mut last_mouse_position = None;
        let mut errors = Vec::new();

        while buffer.peek_timestamp().is_some_and(|ts| ts <= target_ms) {
            let pdu = match buffer.pop_pdu() {
                Some(p) => p,
                None => break,
            };

            let results = match self.process_pdu(image, pdu.source, &pdu.data) {
                Ok(r) => r,
                Err(e) => {
                    errors.push(e);
                    continue;
                }
            };

            for result in &results {
                match result {
                    ProcessResult::ResolutionChanged { width, height } => {
                        *image = DecodedImage::new(PixelFormat::RgbA32, *width, *height);
                        resolution_changed = true;
                        new_resolution = Some((*width, *height));
                    }
                    ProcessResult::FastPath(UpdateKind::PointerPosition { x, y })
                    | ProcessResult::ClientPointerPosition { x, y } => {
                        last_mouse_position = Some((*x, *y));
                    }
                    ProcessResult::SessionEnded => {
                        session_ended = true;
                    }
                    _ => {}
                }
                frame_dirty |= result.is_visual();
            }

            pdus_processed = pdus_processed.saturating_add(1);
        }

        ProcessTillResult {
            pdus_processed,
            resolution_changed,
            new_resolution,
            session_ended,
            frame_dirty,
            last_mouse_position,
            errors,
        }
    }

    /// Dispatch a raw PDU to the appropriate processing function.
    ///
    /// Uses `source` from the recording to route between client/server FastPath and X224.
    /// Malformed PDUs are skipped with a console warning.
    pub fn process_pdu(
        &mut self,
        image: &mut DecodedImage,
        source: PduSource,
        pdu: &[u8],
    ) -> ReplayResult<Vec<ProcessResult>> {
        let action = match ironrdp_pdu::find_size(pdu) {
            Ok(Some(info)) => info.action,
            Ok(None) => {
                return Err(ReplayError::decode(ironrdp_core::not_enough_bytes_err(
                    "find_size",
                    pdu.len(),
                    2, // minimum bytes needed to determine PDU size
                )));
            }
            Err(e) => return Err(ReplayError::decode(e)),
        };

        match (action, source) {
            (Action::FastPath, PduSource::Server) => self.process_server_pdu(image, pdu),
            (Action::FastPath, PduSource::Client) => self.process_client_pdu(pdu),
            (Action::X224, _) => Self::process_x224(pdu),
        }
    }

    /// Process a server PDU (graphics, pointers)
    fn process_server_pdu(&mut self, image: &mut DecodedImage, pdu: &[u8]) -> ReplayResult<Vec<ProcessResult>> {
        self.response_buffer.clear();
        let updates = self
            .fast_path_processor
            .process(image, pdu, &mut self.response_buffer)
            .map_err(ReplayError::session)?;

        let mut results = Vec::new();
        for update in updates {
            // Track pointer state changes (always, even during seeking).
            match &update {
                UpdateKind::PointerBitmap(pointer) => {
                    self.current_pointer = PointerState::Bitmap(Arc::clone(pointer));
                }
                UpdateKind::PointerDefault => {
                    self.current_pointer = PointerState::Default;
                }
                UpdateKind::PointerHidden => {
                    self.current_pointer = PointerState::Hidden;
                }
                _ => {}
            }

            // During seeking, skip visual results to avoid unnecessary rendering.
            // Non-visual results are always emitted for state tracking.
            let result = ProcessResult::FastPath(update);
            if !result.is_visual() || self.update_canvas {
                results.push(result);
            }
        }

        Ok(results)
    }

    /// Process a client PDU (mouse/keyboard input)
    fn process_client_pdu(&self, pdu: &[u8]) -> ReplayResult<Vec<ProcessResult>> {
        // Client input PDUs are stateless; skip during seeking.
        if !self.update_canvas {
            return Ok(Vec::new());
        }

        let input = decode::<FastPathInput>(pdu).map_err(ReplayError::decode)?;

        let mut results = Vec::new();
        for event in input.input_events() {
            match event {
                FastPathInputEvent::MouseEvent(mouse) => {
                    results.push(ProcessResult::ClientPointerPosition {
                        x: mouse.x_position,
                        y: mouse.y_position,
                    });
                }
                FastPathInputEvent::MouseEventEx(mouse) => {
                    results.push(ProcessResult::ClientPointerPosition {
                        x: mouse.x_position,
                        y: mouse.y_position,
                    });
                }
                // Keyboard and other input events are not processed during replay.
                _ => {}
            }
        }

        Ok(results)
    }

    /// Process an X224 PDU (session control, resolution changes)
    fn process_x224(pdu: &[u8]) -> ReplayResult<Vec<ProcessResult>> {
        let x224 = decode::<X224<McsMessage<'_>>>(pdu).map_err(ReplayError::decode)?;

        match x224.0 {
            McsMessage::SendDataIndication(sdi) => {
                let Ok(header) = decode::<ShareControlHeader>(&sdi.user_data) else {
                    return Ok(vec![]);
                };

                match &header.share_control_pdu {
                    ShareControlPdu::ServerDemandActive(demand_active) => {
                        if let Some((width, height)) = demand_active.pdu.capability_sets.iter().find_map(|c| match c {
                            CapabilitySet::Bitmap(b) => Some((b.desktop_width, b.desktop_height)),
                            _ => None,
                        }) {
                            // Always emit ResolutionChanged (even during seeking)
                            Ok(vec![ProcessResult::ResolutionChanged { width, height }])
                        } else {
                            Ok(vec![])
                        }
                    }
                    ShareControlPdu::ServerDeactivateAll(_) => Ok(vec![ProcessResult::SessionDeactivated]),
                    _ => Ok(vec![]),
                }
            }
            McsMessage::DisconnectProviderUltimatum(_) => Ok(vec![ProcessResult::SessionEnded]),
            _ => Ok(vec![]),
        }
    }
}
