//! PDU processing for RDP session replay.
//!
//! Provides a simplified processing pipeline for replay.

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

const MAX_DESKTOP_DIM: u16 = 8192;

/// Configuration for building a [`ReplayProcessor`].
///
/// Defaults reflect common RDP session values.
/// Supply negotiated values when available; the demo recording format does not contain them.
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
#[derive(Debug, Clone)]
pub enum PointerState {
    /// A custom pointer bitmap.
    Bitmap(Arc<DecodedPointer>),
    /// The system default pointer.
    Default,
    /// A hidden pointer.
    Hidden,
}

/// Classification of replay processing errors.
#[non_exhaustive]
#[derive(Debug)]
pub enum ReplayErrorKind {
    /// A decode error from `ironrdp_core`.
    Decode(ironrdp_core::DecodeError),
    /// A session processing error.
    Session(Box<ironrdp_session::SessionError>),
    /// A PDU timestamp is not finite.
    InvalidTimestamp,
}

impl core::fmt::Display for ReplayErrorKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Decode(_) => write!(f, "decode error"),
            Self::Session(_) => write!(f, "session error"),
            Self::InvalidTimestamp => write!(f, "invalid PDU timestamp"),
        }
    }
}

impl core::error::Error for ReplayErrorKind {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            Self::Session(e) => Some(e.as_ref()),
            Self::InvalidTimestamp => None,
        }
    }
}

/// Error type for replay PDU processing.
pub type ReplayError = ironrdp_error::Error<ReplayErrorKind>;

/// Convenience result alias for replay operations.
pub type ReplayResult<T> = Result<T, ReplayError>;

/// Outcome of processing a single PDU.
#[derive(Debug)]
pub enum ProcessResult {
    /// A Fast-Path result for graphics or pointers.
    FastPath(UpdateKind),

    /// A pointer position from client input.
    ClientPointerPosition { x: u16, y: u16 },

    /// A desktop resolution change that requires resizing the framebuffer.
    ResolutionChanged { width: u16, height: u16 },

    /// The session was deactivated and may reactivate with a new resolution.
    SessionDeactivated,

    /// The session ended.
    SessionEnded,
}

impl ProcessResult {
    /// Whether this result represents a visual change such as graphics, pointer, or resolution.
    /// Non-visual results such as session lifecycle events do not require canvas redraws.
    pub fn is_visual(&self) -> bool {
        !matches!(
            self,
            ProcessResult::FastPath(UpdateKind::None) | ProcessResult::SessionEnded | ProcessResult::SessionDeactivated
        )
    }
}

/// Accumulated result of processing PDUs up to a target timestamp.
///
/// Contains all state changes from the processed PDUs.
/// The caller applies canvas-specific side effects such as resize, blit, and cursor compositing.
#[derive(Debug)]
pub struct ProcessTillResult {
    /// The number of PDUs successfully processed from the buffer.
    /// Errored PDUs are consumed and counted in `errors.len()` instead.
    /// The total drained count is `pdus_processed + errors.len()`.
    pub pdus_processed: u32,
    /// A new resolution reported by a processed PDU, if any.
    pub new_resolution: Option<(u16, u16)>,
    /// Whether a SessionEnded PDU was encountered.
    pub session_ended: bool,
    /// Whether any visual result was produced and requires a canvas redraw.
    pub frame_dirty: bool,
    /// The last mouse position from client or server pointer events, if any.
    pub last_mouse_position: Option<(u16, u16)>,
    /// Accumulated processing errors. Failed PDUs are skipped.
    pub errors: Vec<ReplayError>,
}

/// A stateful processor for replay PDU handling.
pub struct ReplayProcessor {
    /// The Fast-Path processor for graphics and pointer handling.
    fast_path_processor: fast_path::Processor,

    /// Whether to suppress visual results such as graphics and pointer updates during seeking.
    /// Session state results are always emitted.
    update_canvas: bool,

    /// The current pointer state for post-seek UI synchronization.
    current_pointer: PointerState,

    /// Reusable response buffer for `fast_path_processor.process()`.
    ///
    /// Replay mode sends no responses, so the buffer is cleared and reused to avoid repeated allocations.
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
    /// Errors from individual PDUs are collected in `ProcessTillResult::errors`.
    /// The offending PDU is skipped, and processing continues with the next PDU.
    ///
    /// The caller resizes the canvas to `new_resolution`, blits the framebuffer when `frame_dirty && update_canvas`, and updates the cursor.
    ///
    /// `pdus_processed` counts only successfully processed PDUs.
    /// Errored PDUs are consumed from the buffer and counted in `errors.len()` instead.
    /// The total drained count is `pdus_processed + errors.len()`.
    pub fn process_till(
        &mut self,
        buffer: &mut PduBuffer,
        image: &mut DecodedImage,
        target_ms: f64,
    ) -> ProcessTillResult {
        let mut pdus_processed: u32 = 0;
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
            new_resolution,
            session_ended,
            frame_dirty,
            last_mouse_position,
            errors,
        }
    }

    /// Dispatch a raw PDU to the appropriate processing function.
    ///
    /// Uses the recording direction to route client/server Fast-Path and X.224 PDUs.
    /// Malformed PDUs return an error to the caller.
    ///
    /// [2.2.8.1.2] "Client Fast-Path Input Event PDU" in MS-RDPBCGR defines input security flags.
    /// [2.2.9.1.2] "Server Fast-Path Update PDU" defines output security flags.
    /// [2.2.8.1.2.2.3] "Fast-Path Mouse Event" defines the mouse coordinate fields.
    /// [2.2.1.13.1] "Server Demand Active PDU" defines the server X.224 control PDU.
    /// [2.2.7.1.2] "Bitmap Capability Set" defines the desktop dimensions.
    ///
    /// [2.2.8.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
    /// [2.2.9.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
    /// [2.2.8.1.2.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
    /// [2.2.1.13.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
    /// [2.2.7.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
    pub fn process_pdu(
        &mut self,
        image: &mut DecodedImage,
        source: PduSource,
        pdu: &[u8],
    ) -> ReplayResult<Vec<ProcessResult>> {
        let action = match ironrdp_pdu::find_size(pdu) {
            Ok(Some(info)) => info.action,
            Ok(None) => {
                return Err(ReplayError::new(
                    "replay",
                    ReplayErrorKind::Decode(ironrdp_core::not_enough_bytes_err(
                        "find_size",
                        pdu.len(),
                        2, // Minimum bytes needed to determine PDU size.
                        Some(0),
                    )),
                ));
            }
            Err(error) => return Err(ReplayError::new("replay", ReplayErrorKind::Decode(error))),
        };

        if action == Action::FastPath && pdu[0] & 0xc0 != 0 {
            return Err(ReplayError::new(
                "replay",
                ReplayErrorKind::Decode(ironrdp_core::invalid_field_err(
                    "replay",
                    "fast-path security flags",
                    "encrypted or signed fast-path data is not supported",
                    Some(0),
                )),
            ));
        }

        match (action, source) {
            (Action::FastPath, PduSource::Server) => self.process_server_pdu(image, pdu),
            (Action::FastPath, PduSource::Client) => self.process_client_pdu(pdu),
            (Action::X224, PduSource::Server) => Self::process_x224(pdu),
            (Action::X224, PduSource::Client) => Ok(Vec::new()),
        }
    }

    /// Processes a server PDU for graphics and pointers.
    fn process_server_pdu(&mut self, image: &mut DecodedImage, pdu: &[u8]) -> ReplayResult<Vec<ProcessResult>> {
        self.response_buffer.clear();
        let updates = self
            .fast_path_processor
            .process(image, pdu, &mut self.response_buffer, &mut None)
            .map_err(|error| ReplayError::new("replay", ReplayErrorKind::Session(Box::new(error))))?;

        let mut results = Vec::new();
        for update in updates {
            // Track pointer state changes even during seeking.
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

            // Skip visual results during seeking to avoid unnecessary rendering.
            // Non-visual results are always emitted for state tracking.
            let result = ProcessResult::FastPath(update);
            if !result.is_visual() || self.update_canvas {
                results.push(result);
            }
        }

        Ok(results)
    }

    /// Processes a client PDU for mouse or keyboard input.
    fn process_client_pdu(&self, pdu: &[u8]) -> ReplayResult<Vec<ProcessResult>> {
        // Client input PDUs are stateless, so skip them during seeking.
        if !self.update_canvas {
            return Ok(Vec::new());
        }

        let input =
            decode::<FastPathInput>(pdu).map_err(|error| ReplayError::new("replay", ReplayErrorKind::Decode(error)))?;

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

    /// Processes an X.224 PDU for session control and resolution changes.
    ///
    /// [MS-RDPBCGR] [section 2.2.1.13.1, "Server Demand Active PDU"] carries the resolution capability set.
    /// [MS-RDPBCGR] [section 2.2.7.1.2, "Bitmap Capability Set"] defines the desktop dimensions.
    ///
    /// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
    fn process_x224(pdu: &[u8]) -> ReplayResult<Vec<ProcessResult>> {
        let x224 = decode::<X224<McsMessage<'_>>>(pdu)
            .map_err(|error| ReplayError::new("replay", ReplayErrorKind::Decode(error)))?;

        match x224.0 {
            McsMessage::SendDataIndication(sdi) => {
                let header = decode::<ShareControlHeader>(&sdi.user_data)
                    .map_err(|error| ReplayError::new("replay", ReplayErrorKind::Decode(error)))?;

                match &header.share_control_pdu {
                    ShareControlPdu::ServerDemandActive(demand_active) => {
                        if let Some((width, height)) = demand_active.pdu.capability_sets.iter().find_map(|c| match c {
                            CapabilitySet::Bitmap(b) => Some((b.desktop_width, b.desktop_height)),
                            _ => None,
                        }) {
                            if !(0 < width && width <= MAX_DESKTOP_DIM && 0 < height && height <= MAX_DESKTOP_DIM) {
                                return Err(ReplayError::new(
                                    "replay",
                                    ReplayErrorKind::Decode(ironrdp_core::invalid_field_err(
                                        "replay",
                                        "desktop size",
                                        "invalid desktop dimensions for replay",
                                        // The decoded structure does not retain its byte cursor.
                                        None,
                                    )),
                                ));
                            }
                            // Resolution changes update session state during seeking too.
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
