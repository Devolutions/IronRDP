use core::net::SocketAddr;
use core::num::{NonZeroU16, NonZeroUsize};
#[cfg(feature = "rdpdr")]
use core::sync::atomic::{AtomicBool, Ordering};
#[cfg(feature = "location")]
use core::sync::atomic::{AtomicU8, Ordering as LocationOrdering};
use core::time::Duration;
use std::io;
use std::sync::Arc;
#[cfg(feature = "location")]
use std::sync::mpsc as std_mpsc;
#[cfg(feature = "location")]
use std::time::Instant;

#[cfg(feature = "clipboard")]
pub use ironrdp_cliprdr::backend::CliprdrBackendFactory;
use ironrdp_connector::connection_activation::ConnectionActivationState;
use ironrdp_connector::{ConnectionResult, ConnectorResult};
use ironrdp_core::WriteBuf;
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_displaycontrol::pdu::MonitorLayoutEntry;
use ironrdp_dvc::DvcMessageBatch;
use ironrdp_dvc::pdu::SoftSyncTunnelType;
#[cfg(any(all(windows, feature = "dvc-com-plugin"), feature = "sound"))]
use ironrdp_dvc::{DvcChannelListener, DvcClientProcessor, DynamicChannelId};
use ironrdp_echo::client::EchoClient;
use ironrdp_egfx::client::{GraphicsPipelineClient, GraphicsPipelineHandler};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_pdu::gcc::{ChannelName, Monitor};
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::input::MousePdu;
use ironrdp_pdu::input::fast_path::FastPathInputEvent;
use ironrdp_pdu::input::mouse::PointerFlags;
#[cfg(any(
    feature = "dvc-pipe-proxy",
    all(windows, feature = "dvc-com-plugin"),
    feature = "sound",
    all(windows, feature = "webauthn")
))]
use ironrdp_pdu::pdu_other_err;
use ironrdp_pdu::rdp::multitransport::MultitransportResponsePdu;
use ironrdp_pdu::rdp::session_info::ServerAutoReconnect;
#[cfg(feature = "rdpdr")]
pub use ironrdp_rdpdr::backend::{
    RdpdrBackendFactory, RdpdrBackendFactoryResult, RdpdrBackendProduct, RdpdrDrive, RdpdrPrinter,
};
use ironrdp_rdpei::RdpeiClient;
use ironrdp_rdpei::pdu::TouchEventPdu;
#[cfg(feature = "location")]
use ironrdp_rdpel::client::LocationClient;
#[cfg(any(feature = "clipboard", feature = "rdpdr"))]
use ironrdp_session::ActiveStage;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{
    ActiveStageBuilder, ActiveStageOutput, GracefulDisconnectReason, SessionErrorExt as _, SessionResult,
};
use ironrdp_svc::SvcMessage;
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{FramedWrite, single_sequence_step_read, split_tokio_framed};
use smallvec::SmallVec;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
#[cfg(any(feature = "clipboard", all(windows, feature = "dvc-com-plugin")))]
use tracing::error;
use tracing::{debug, info, trace, warn};

#[cfg(feature = "clipboard")]
use crate::config::ClipboardType;
#[cfg(feature = "clipboard")]
use ironrdp_cliprdr::backend::ClipboardMessage;
#[cfg(all(windows, feature = "dvc-com-plugin"))]
use ironrdp_dvc_com_plugin::load_dvc_plugin_listeners;
#[cfg(feature = "dvc-pipe-proxy")]
use ironrdp_dvc_pipe_proxy::DvcNamedPipeProxy;
#[cfg(all(windows, feature = "webauthn"))]
use ironrdp_rdpewa::{RdpewaClient, RdpewaClientListener};
#[cfg(all(windows, feature = "webauthn"))]
use ironrdp_rdpewa_native::{WindowsRdpewaBackend, WindowsRdpewaSessionState};
#[cfg(feature = "sound")]
use ironrdp_rdpsnd_native::{RdpeaiCaptureBackend, cpal};

use crate::config::{Config, RDCleanPathConfig, Transport};
use crate::rail::{RailClient, RailControlEvent, RailEvent, RailInputEvent};
use ironrdp_rail::pdu::{ExecutePdu, ExecuteResultPdu};

// ── Public event types ────────────────────────────────────────────────────────

/// Explains why a dynamic display update must reconnect instead of completing in-session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayResizeFallbackReason {
    /// Display Control is not configured or its dynamic channel became unavailable.
    DisplayControlUnavailable,
    /// The server did not send the required Display Control capabilities PDU in time.
    CapabilitiesTimedOut,
    /// The server did not reactivate the session after a monitor-layout request in time.
    ReactivationTimedOut,
}

/// Explains why a locally accepted RemoteApp launch could not be processed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RailExecuteFailureReason {
    /// The active session has no RAIL static-channel processor.
    RailUnavailable,
    /// The RAIL client rejected the Execute request before it could be sent.
    QueueRejected,
    /// The active stage could not encode the queued RAIL messages.
    MessageProcessingFailed,
}

#[derive(Debug)]
pub enum RdpOutputEvent {
    /// Connection negotiation and activation have completed.
    Connected,
    /// The server-reported remote monitor layout.
    ///
    /// The server may send this during activation or after activation.
    MonitorLayout(Vec<Monitor>),
    /// Server Save Session Info notification.
    ///
    /// The PDU can contain sensitive session data, so this event does not expose its contents.
    LoginComplete,
    Image {
        buffer: Vec<u32>,
        width: NonZeroU16,
        height: NonZeroU16,
    },
    ConnectionFailure(ironrdp_connector::ConnectorError),
    PointerDefault,
    PointerHidden,
    PointerPosition {
        x: u16,
        y: u16,
    },
    PointerBitmap(Arc<DecodedPointer>),
    /// RAIL Windowing Alternate Secondary Drawing Orders.
    ///
    /// The owned payload begins with the slow-path `Orders` update type and is delivered only
    /// after the Window List support level negotiated for the current activation is validated.
    WindowingOrders(Vec<u8>),
    /// The server completed the RAIL static-channel handshake.
    RailHandshake {
        handshake_ex_flags: Option<u32>,
        initialization_message_count: usize,
        queued_execute_count: usize,
    },
    /// Queued RAIL input was released after server desktop synchronization.
    RailDesktopSynchronized {
        released_execute_count: usize,
    },
    /// Queued RAIL input was released after the post-handshake fallback delay.
    RailPostHandshakeQueueReleased {
        released_execute_count: usize,
    },
    /// The server completed a RemoteApp launch request.
    RailExecuteResult(ExecuteResultPdu),
    /// A locally accepted RemoteApp launch could not be processed.
    ///
    /// The event carries only the launch correlation fields, never its working directory or
    /// arguments.
    RailExecuteFailed {
        executable: String,
        flags: u16,
        reason: RailExecuteFailureReason,
    },
    /// The server supplied an application identity for a remote window.
    RailApplicationId {
        window_id: u32,
        application_id: String,
        process_id: Option<u32>,
        process_image_name: Option<String>,
    },
    /// A portable server-originated RAIL control for the embedding host.
    RailControl(RailControlEvent),
    /// A full-desktop redraw was requested after the initial logon notification.
    PostLogonDisplayRedraw,
    /// A malformed bitmap update was discarded and a capability-gated full redraw was sent.
    MalformedBitmapDisplayRedraw,
    /// Dynamic Display Control could not update the session in place.
    ///
    /// The next connection attempt uses the requested desktop size.
    DisplayResizeFallback(DisplayResizeFallbackReason),
    /// An active session was interrupted and a cookie-based reconnect is about to start.
    ///
    /// `attempt` is one-based and never exceeds `maximum_attempts`.
    AutoReconnecting {
        /// The server did not provide a protocol-level disconnect code for the transport loss.
        disconnect_reason: u32,
        attempt: u32,
        maximum_attempts: u32,
        response: oneshot::Sender<AutoReconnectDecision>,
    },
    /// A cookie-based reconnect has completed successfully.
    AutoReconnected,
    Terminated(SessionResult<GracefulDisconnectReason>),
}

/// A tightly packed changed region from the composited desktop framebuffer.
///
/// Pixels use the same `0x00RRGGBB` representation as [`RdpOutputEvent::Image`].
/// `region` uses inclusive coordinates in the full framebuffer described by `width` and `height`.
#[derive(Debug)]
pub struct DesktopUpdate {
    buffer: Vec<u32>,
    width: NonZeroU16,
    height: NonZeroU16,
    region: InclusiveRectangle,
}

impl DesktopUpdate {
    /// Builds a validated desktop update.
    #[must_use]
    pub fn new(buffer: Vec<u32>, width: NonZeroU16, height: NonZeroU16, region: InclusiveRectangle) -> Option<Self> {
        let region_width = region.right.checked_sub(region.left)?.checked_add(1)?;
        let region_height = region.bottom.checked_sub(region.top)?.checked_add(1)?;
        if region.right >= width.get() || region.bottom >= height.get() {
            return None;
        }
        let pixel_count = usize::from(region_width).checked_mul(usize::from(region_height))?;
        (buffer.len() == pixel_count).then_some(Self {
            buffer,
            width,
            height,
            region,
        })
    }

    /// Returns the packed `0x00RRGGBB` pixels.
    pub fn buffer(&self) -> &[u32] {
        &self.buffer
    }

    /// Returns the full framebuffer width.
    pub fn width(&self) -> NonZeroU16 {
        self.width
    }

    /// Returns the full framebuffer height.
    pub fn height(&self) -> NonZeroU16 {
        self.height
    }

    /// Returns the changed inclusive framebuffer region.
    pub fn region(&self) -> InclusiveRectangle {
        self.region.clone()
    }

    /// Decomposes the update into its packed pixels, framebuffer extent, and region.
    pub fn into_parts(self) -> (Vec<u32>, NonZeroU16, NonZeroU16, InclusiveRectangle) {
        (self.buffer, self.width, self.height, self.region)
    }
}

impl RdpOutputEvent {
    /// Classifies how this event should be queued when the output channel is
    /// full: [`crate::output_channel::DropPolicy::MustDeliver`] for events where
    /// loss would be a real bug (a failed connection, a completed RAIL launch),
    /// [`crate::output_channel::DropPolicy::LatestOnly`] for high-frequency
    /// display state where only the newest value matters.
    ///
    /// See <https://github.com/Devolutions/IronRDP/issues/1330> for the design
    /// rationale.
    pub fn drop_policy(&self) -> crate::output_channel::DropPolicy {
        use crate::output_channel::DropPolicy;

        match self {
            RdpOutputEvent::Image { .. }
            | RdpOutputEvent::PointerDefault
            | RdpOutputEvent::PointerHidden
            | RdpOutputEvent::PointerPosition { .. }
            | RdpOutputEvent::PointerBitmap(_) => DropPolicy::LatestOnly,
            _ => DropPolicy::MustDeliver,
        }
    }
}

/// Controls whether a pending automatic reconnect may proceed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AutoReconnectDecision {
    Continue,
    Stop,
}

/// Failure reported after a queued location update reaches the active session loop.
#[cfg(feature = "location")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocationInputError {
    ChannelUnavailable,
    ChannelNotReady,
    EncodingFailed,
}

/// Failure to enqueue a location update in the bounded session input queue.
#[cfg(feature = "location")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocationQueueError {
    Full,
    Closed,
}

/// Failure while waiting for a queued location request to finish.
#[cfg(feature = "location")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocationDeliveryError {
    Timeout,
    Closed,
}

#[cfg(feature = "location")]
const LOCATION_REQUEST_PENDING: u8 = 0;
#[cfg(feature = "location")]
const LOCATION_REQUEST_COMMITTED: u8 = 1;
#[cfg(feature = "location")]
const LOCATION_REQUEST_CANCELLED: u8 = 2;

/// Completion handle for one bounded location request.
#[cfg(feature = "location")]
#[derive(Debug)]
pub struct LocationDelivery {
    deadline: Instant,
    state: Arc<AtomicU8>,
    response: std_mpsc::Receiver<Result<(), LocationInputError>>,
}

#[cfg(feature = "location")]
impl LocationDelivery {
    /// Waits until the session loop commits or rejects this request.
    pub fn wait(self) -> Result<Result<(), LocationInputError>, LocationDeliveryError> {
        match self
            .response
            .recv_timeout(self.deadline.saturating_duration_since(Instant::now()))
        {
            Ok(result) => Ok(result),
            Err(std_mpsc::RecvTimeoutError::Disconnected) => Err(LocationDeliveryError::Closed),
            Err(std_mpsc::RecvTimeoutError::Timeout) => {
                match self.state.compare_exchange(
                    LOCATION_REQUEST_PENDING,
                    LOCATION_REQUEST_CANCELLED,
                    LocationOrdering::AcqRel,
                    LocationOrdering::Acquire,
                ) {
                    Ok(_) => Err(LocationDeliveryError::Timeout),
                    Err(LOCATION_REQUEST_COMMITTED) => self.response.recv().map_err(|_| LocationDeliveryError::Closed),
                    Err(_) => Err(LocationDeliveryError::Timeout),
                }
            }
        }
    }
}

/// One caller-supplied location request queued for the active session loop.
#[cfg(feature = "location")]
pub struct LocationRequest {
    latitude: f64,
    longitude: f64,
    altitude: i32,
    deadline: Instant,
    state: Arc<AtomicU8>,
    response: std_mpsc::SyncSender<Result<(), LocationInputError>>,
}

#[cfg(feature = "location")]
impl core::fmt::Debug for LocationRequest {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LocationRequest").finish_non_exhaustive()
    }
}

#[cfg(feature = "location")]
impl LocationRequest {
    /// Returns the explicitly supplied coordinates without logging or persistence.
    pub fn coordinates(&self) -> (f64, f64, i32) {
        (self.latitude, self.longitude, self.altitude)
    }

    /// Completes a request in tests or alternate session-loop integrations.
    pub fn complete(self, result: Result<(), LocationInputError>) {
        if self.try_commit() {
            let _ = self.response.send(result);
        }
    }

    fn is_cancelled_or_expired(&self) -> bool {
        if Instant::now() >= self.deadline {
            let _ = self.state.compare_exchange(
                LOCATION_REQUEST_PENDING,
                LOCATION_REQUEST_CANCELLED,
                LocationOrdering::AcqRel,
                LocationOrdering::Acquire,
            );
        }
        self.state.load(LocationOrdering::Acquire) == LOCATION_REQUEST_CANCELLED
    }

    fn try_commit(&self) -> bool {
        !self.is_cancelled_or_expired()
            && self
                .state
                .compare_exchange(
                    LOCATION_REQUEST_PENDING,
                    LOCATION_REQUEST_COMMITTED,
                    LocationOrdering::AcqRel,
                    LocationOrdering::Acquire,
                )
                .is_ok()
    }

    fn send_response(self, result: Result<(), LocationInputError>) {
        let _ = self.response.send(result);
    }
}

#[derive(Debug)]
pub enum RdpInputEvent {
    Resize {
        width: u16,
        height: u16,
        scale_factor: u32,
        /// Physical display size in millimetres (width, height).
        physical_size: Option<(u32, u32)>,
    },
    FastPath(SmallVec<[FastPathInputEvent; 2]>),
    /// Multitouch frames for MS-RDPEI (`Microsoft::Windows::RDS::Input`).
    Touch(TouchEventPdu),
    /// Pen frames for MS-RDPEI (`RDPINPUT_PEN_EVENT_PDU`).
    Pen(ironrdp_rdpei::pdu::PenEventPdu),
    /// Dismiss a hovering touch contact over MS-RDPEI.
    DismissHoveringTouchContact {
        contact_id: u8,
    },
    Close,
    #[cfg(feature = "clipboard")]
    Clipboard(ClipboardMessage),
    SendDvcMessages {
        channel_id: u32,
        messages: Vec<SvcMessage>,
    },
    SendStaticChannelData {
        channel_name: ChannelName,
        data: Vec<u8>,
    },
    /// Activates and announces a preconfigured RDPDR filesystem device.
    #[cfg(feature = "rdpdr")]
    AddRdpdrDrive {
        device_id: u32,
        name: String,
    },
    /// Removes an active RDPDR filesystem device.
    #[cfg(feature = "rdpdr")]
    RemoveRdpdrDrive {
        device_id: u32,
    },
    #[cfg(feature = "location")]
    Location(LocationRequest),
    /// Requests a RemoteApp launch over the RAIL static channel.
    RailExecute(ExecutePdu),
    /// Queues a client-originated RAIL input event.
    Rail(RailInputEvent),
}

/// Maximum number of ordinary input events retained while the session is unable to process them.
pub const RDP_INPUT_EVENT_QUEUE_CAPACITY: usize = 128;

/// Sends input to an [`RdpClient`] without allowing an unbounded queue to accumulate.
///
/// [`Self::request_close`] is independent of this bounded queue, so a host can always cancel a
/// connection attempt or active session even when ordinary input is backpressured.
#[derive(Clone, Debug)]
pub struct RdpInputSender {
    input_sender: mpsc::Sender<RdpInputEvent>,
    #[cfg_attr(
        not(feature = "clipboard"),
        expect(dead_code, reason = "clipboard input is unavailable without the clipboard feature")
    )]
    clipboard_sender: mpsc::UnboundedSender<RdpInputEvent>,
    close_sender: watch::Sender<bool>,
    graceful_close_sender: watch::Sender<bool>,
    #[cfg(feature = "rdpdr")]
    rdpdr_drive_hotplug_available: Arc<AtomicBool>,
}

impl RdpInputSender {
    /// Creates a bounded input queue for integration tests.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero.
    pub fn channel(capacity: usize) -> (Self, mpsc::Receiver<RdpInputEvent>) {
        let (sender, receiver, _, _, _) = Self::channel_with_close_signal(capacity);
        (sender, receiver)
    }

    fn channel_with_close_signal(
        capacity: usize,
    ) -> (
        Self,
        mpsc::Receiver<RdpInputEvent>,
        mpsc::UnboundedReceiver<RdpInputEvent>,
        watch::Receiver<bool>,
        watch::Receiver<bool>,
    ) {
        let (input_sender, input_receiver) = mpsc::channel(capacity);
        let (clipboard_sender, clipboard_receiver) = mpsc::unbounded_channel();
        let (close_sender, close_receiver) = watch::channel(false);
        let (graceful_close_sender, graceful_close_receiver) = watch::channel(false);
        (
            Self {
                input_sender,
                clipboard_sender,
                close_sender,
                graceful_close_sender,
                #[cfg(feature = "rdpdr")]
                rdpdr_drive_hotplug_available: Arc::new(AtomicBool::new(false)),
            },
            input_receiver,
            clipboard_receiver,
            close_receiver,
            graceful_close_receiver,
        )
    }

    /// Attempts to enqueue ordinary input without blocking the calling thread.
    pub fn try_send(&self, event: RdpInputEvent) -> Result<(), mpsc::error::TrySendError<RdpInputEvent>> {
        self.input_sender.try_send(event)
    }

    /// Reserves capacity for ordinary input without blocking.
    ///
    /// Callers that maintain local input state can reserve first, then update that state only after
    /// the input event is guaranteed to fit in the bounded queue.
    pub fn try_reserve(&self) -> Result<mpsc::Permit<'_, RdpInputEvent>, mpsc::error::TrySendError<()>> {
        self.input_sender.try_reserve()
    }

    /// Queues a RemoteApp launch without allowing the ordinary input queue to grow unbounded.
    pub fn try_send_rail_execute(&self, execute: ExecutePdu) -> Result<(), mpsc::error::TrySendError<RdpInputEvent>> {
        self.input_sender.try_send(RdpInputEvent::RailExecute(execute))
    }

    /// Queues a client-originated RAIL input event.
    pub fn try_send_rail_input(&self, event: RailInputEvent) -> Result<(), mpsc::error::TrySendError<RdpInputEvent>> {
        self.input_sender.try_send(RdpInputEvent::Rail(event))
    }

    /// Queues one explicit location update and returns its bounded completion receiver.
    #[cfg(feature = "location")]
    pub fn try_send_location(
        &self,
        latitude: f64,
        longitude: f64,
        altitude: i32,
        timeout: Duration,
    ) -> Result<LocationDelivery, LocationQueueError> {
        let now = Instant::now();
        let deadline = now.checked_add(timeout).unwrap_or(now);
        let state = Arc::new(AtomicU8::new(LOCATION_REQUEST_PENDING));
        let (response, receiver) = std_mpsc::sync_channel(1);
        self.input_sender
            .try_send(RdpInputEvent::Location(LocationRequest {
                latitude,
                longitude,
                altitude,
                deadline,
                state: Arc::clone(&state),
                response,
            }))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => LocationQueueError::Full,
                mpsc::error::TrySendError::Closed(_) => LocationQueueError::Closed,
            })?;
        Ok(LocationDelivery {
            deadline,
            state,
            response: receiver,
        })
    }

    /// Enqueues a clipboard protocol message independently of ordinary bounded input.
    ///
    /// Clipboard messages form an ordered CLIPRDR transaction, so dropping one while applying
    /// backpressure to keyboard and pointer input can desynchronize the clipboard channel.
    #[cfg(feature = "clipboard")]
    pub fn send_clipboard(&self, message: ClipboardMessage) -> Result<(), mpsc::error::SendError<RdpInputEvent>> {
        self.clipboard_sender.send(RdpInputEvent::Clipboard(message))
    }

    /// Requests immediate session cancellation, bypassing the bounded input queue.
    pub fn request_close(&self) {
        self.close_sender.send_replace(true);
    }

    /// Requests a graceful RDP shutdown after the connection becomes active.
    ///
    /// This bypasses the bounded input queue so a full queue cannot prevent the client from
    /// sending the RDP Shutdown Request PDU. Use [`request_close`](Self::request_close) to
    /// immediately cancel a connection attempt or active session instead.
    pub fn request_graceful_close(&self) {
        self.graceful_close_sender.send_replace(true);
    }

    /// Returns whether the active session negotiated support for RDPDR drive hotplug.
    #[cfg(feature = "rdpdr")]
    pub fn rdpdr_drive_hotplug_available(&self) -> bool {
        self.rdpdr_drive_hotplug_available.load(Ordering::Acquire)
    }

    #[cfg(feature = "rdpdr")]
    fn set_rdpdr_drive_hotplug_available(&self, available: bool) {
        self.rdpdr_drive_hotplug_available.store(available, Ordering::Release);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResizeRequest {
    width: u16,
    height: u16,
    scale_factor: u32,
    physical_size: Option<(u32, u32)>,
}

struct TimedResizeRequest {
    request: ResizeRequest,
    deadline: tokio::time::Instant,
}

const DISPLAY_CONTROL_READY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Default)]
struct ResizeQueue {
    in_flight: Option<TimedResizeRequest>,
    pending: Option<TimedResizeRequest>,
}

impl ResizeQueue {
    fn deadline(&self) -> Option<tokio::time::Instant> {
        match (self.in_flight.as_ref(), self.pending.as_ref()) {
            (Some(in_flight), Some(pending)) => Some(core::cmp::min(in_flight.deadline, pending.deadline)),
            (Some(in_flight), None) => Some(in_flight.deadline),
            (None, Some(pending)) => Some(pending.deadline),
            (None, None) => None,
        }
    }

    fn defer(&mut self, request: ResizeRequest) {
        self.pending = Some(TimedResizeRequest {
            request,
            deadline: tokio::time::Instant::now() + DISPLAY_CONTROL_READY_TIMEOUT,
        });
    }

    fn mark_in_flight(&mut self, request: ResizeRequest) {
        self.in_flight = Some(TimedResizeRequest {
            request,
            deadline: tokio::time::Instant::now() + DISPLAY_CONTROL_READY_TIMEOUT,
        });
    }

    fn completed(&mut self) {
        self.in_flight = None;
    }

    fn timed_out_request(&self, now: tokio::time::Instant) -> Option<(ResizeRequest, DisplayResizeFallbackReason)> {
        if let Some(in_flight) = self.in_flight.as_ref()
            && now >= in_flight.deadline
        {
            return Some((
                self.pending
                    .as_ref()
                    .map_or(in_flight.request, |pending| pending.request),
                DisplayResizeFallbackReason::ReactivationTimedOut,
            ));
        }

        self.pending
            .as_ref()
            .filter(|pending| now >= pending.deadline)
            .map(|pending| (pending.request, DisplayResizeFallbackReason::CapabilitiesTimedOut))
    }
}

// ── RdpClient ─────────────────────────────────────────────────────────────────

pub struct RdpClient {
    config: Config,
    output_event_sender: crate::output_channel::OutputEventSender,
    input_event_sender: RdpInputSender,
    input_event_receiver: mpsc::Receiver<RdpInputEvent>,
    clipboard_event_receiver: mpsc::UnboundedReceiver<RdpInputEvent>,
    close_receiver: watch::Receiver<bool>,
    graceful_close_receiver: watch::Receiver<bool>,
    auto_reconnect_maximum_attempts: Option<u32>,
    desktop_update_handler: Option<Box<dyn Fn(DesktopUpdate) + Send + Sync>>,
    #[cfg(feature = "clipboard")]
    cliprdr_backend_factory: Option<Box<dyn CliprdrBackendFactory + Send>>,
    #[cfg(feature = "rdpdr")]
    rdpdr_backend_factory: Option<Box<dyn RdpdrBackendFactory + Send>>,
}

impl RdpClient {
    pub fn new(config: Config, output_event_sender: crate::output_channel::OutputEventSender) -> Self {
        let (
            input_event_sender,
            input_event_receiver,
            clipboard_event_receiver,
            close_receiver,
            graceful_close_receiver,
        ) = RdpInputSender::channel_with_close_signal(RDP_INPUT_EVENT_QUEUE_CAPACITY);
        Self {
            config,
            output_event_sender,
            input_event_sender,
            input_event_receiver,
            clipboard_event_receiver,
            close_receiver,
            graceful_close_receiver,
            auto_reconnect_maximum_attempts: None,
            desktop_update_handler: None,
            #[cfg(feature = "clipboard")]
            cliprdr_backend_factory: None,
            #[cfg(feature = "rdpdr")]
            rdpdr_backend_factory: None,
        }
    }

    /// Supplies a CLIPRDR backend whose owner remains on the embedding application's event-loop
    /// thread for the lifetime of this client.
    #[cfg(feature = "clipboard")]
    #[must_use]
    pub fn with_cliprdr_backend_factory(mut self, factory: Box<dyn CliprdrBackendFactory + Send>) -> Self {
        self.cliprdr_backend_factory = Some(factory);
        self
    }

    /// Supplies a factory that creates a new RDPDR backend and drive set for every connection attempt.
    ///
    /// RDPDR is attached only when it is enabled and the product contains at least one filesystem device.
    #[cfg(feature = "rdpdr")]
    #[must_use]
    pub fn with_rdpdr_backend_factory(mut self, factory: Box<dyn RdpdrBackendFactory + Send>) -> Self {
        self.rdpdr_backend_factory = Some(factory);
        self
    }

    /// Return a clone of the input-event sender for injecting keyboard, mouse, and clipboard
    /// events from the GUI thread.
    pub fn input_sender(&self) -> RdpInputSender {
        self.input_event_sender.clone()
    }

    /// Enables automatic reconnection after an active-session interruption.
    ///
    /// A reconnect is attempted only when the server has supplied an auto-reconnect cookie.
    /// A value of zero disables retries.
    #[must_use]
    pub fn with_auto_reconnect(mut self, maximum_attempts: u32) -> Self {
        self.auto_reconnect_maximum_attempts = Some(maximum_attempts);
        self
    }

    /// Delivers tightly packed dirty regions instead of full [`RdpOutputEvent::Image`] snapshots.
    ///
    /// The first update for each framebuffer extent covers the full framebuffer.
    /// Other output events continue through the configured output channel.
    #[must_use]
    pub fn with_desktop_update_handler<F>(mut self, handler: F) -> Self
    where
        F: Fn(DesktopUpdate) + Send + Sync + 'static,
    {
        self.desktop_update_handler = Some(Box::new(handler));
        self
    }

    pub async fn run(mut self) {
        if *self.close_receiver.borrow_and_update() {
            self.emit_user_initiated_termination();
            return;
        }

        // ── Clipboard initialisation (compile-time gated) ─────────────────────
        //
        // On Windows the WinClipboard object must outlive the entire connection loop, so we
        // keep it alive via `_win_clipboard`.  On non-Windows a StubClipboard backend is used
        // and its ownership can be released immediately after the factory is extracted.
        #[cfg(all(windows, feature = "clipboard"))]
        #[expect(
            clippy::collection_is_never_read,
            reason = "binding owns the Windows clipboard so it stays alive for the connection's lifetime"
        )]
        let _win_clipboard;

        #[cfg(feature = "clipboard")]
        let cliprdr_factory: Option<Box<dyn CliprdrBackendFactory + Send>>;

        #[cfg(feature = "clipboard")]
        {
            let host_factory = self.cliprdr_backend_factory.take();
            match (self.config.channels.clipboard, host_factory) {
                (ClipboardType::Enable, Some(factory)) => {
                    cliprdr_factory = Some(factory);
                    #[cfg(windows)]
                    {
                        _win_clipboard = None;
                    }
                }
                (ClipboardType::Disable, _) => {
                    cliprdr_factory = None;
                    #[cfg(windows)]
                    {
                        _win_clipboard = None;
                    }
                }
                (ClipboardType::Stub, _) => {
                    use ironrdp_cliprdr_native::StubClipboard;
                    let stub = StubClipboard::new();
                    cliprdr_factory = Some(stub.backend_factory());
                    #[cfg(windows)]
                    {
                        _win_clipboard = None;
                    }
                }
                (ClipboardType::Enable, None) => {
                    #[cfg(windows)]
                    {
                        use crate::clipboard::ClientClipboardMessageProxy;
                        use ironrdp_cliprdr_native::WinClipboard;
                        match WinClipboard::new(ClientClipboardMessageProxy::new(self.input_event_sender.clone())) {
                            Ok(win_cb) => {
                                cliprdr_factory = Some(win_cb.backend_factory());
                                _win_clipboard = Some(win_cb);
                            }
                            Err(e) => {
                                let _ = self
                                    .output_event_sender
                                    .send(RdpOutputEvent::ConnectionFailure(ironrdp_connector::custom_err!(
                                        "Windows clipboard initialization",
                                        e
                                    )))
                                    .await;
                                return;
                            }
                        }
                    }

                    #[cfg(not(windows))]
                    {
                        use ironrdp_cliprdr_native::StubClipboard;
                        let stub = StubClipboard::new();
                        cliprdr_factory = Some(stub.backend_factory());
                    }
                }
            }
        }

        // Resolve the per-connection cliprdr factory reference once.  `Option<&dyn …>` is `Copy`,
        // so it can be threaded into every connect attempt across reconnects.
        #[cfg(feature = "clipboard")]
        let cliprdr_factory: CliprdrFactoryRef<'_> = cliprdr_factory.as_deref();
        #[cfg(not(feature = "clipboard"))]
        let cliprdr_factory: CliprdrFactoryRef<'_> = core::marker::PhantomData;
        #[cfg(feature = "rdpdr")]
        let rdpdr_factory = self.rdpdr_backend_factory.take();
        #[cfg(feature = "rdpdr")]
        let rdpdr_factory: RdpdrFactoryRef<'_> = rdpdr_factory.as_deref();
        #[cfg(not(feature = "rdpdr"))]
        let rdpdr_factory: RdpdrFactoryRef<'_> = core::marker::PhantomData;

        // ── Connection + session loop ─────────────────────────────────────────
        let auto_reconnect_policy = self.auto_reconnect_maximum_attempts.map(AutoReconnectPolicy::new);
        let mut auto_reconnect_cookie = None;
        let mut reconnect_attempt = 0;

        loop {
            if *self.close_receiver.borrow_and_update() {
                self.emit_user_initiated_termination();
                break;
            }

            // Only a transport-loss retry may attach the session-bound ARC cookie. Display
            // fallback reconnects establish a new desktop session and must not reuse it.
            let reconnect_cookie = (reconnect_attempt != 0)
                .then_some(auto_reconnect_cookie.as_ref())
                .flatten();
            let used_auto_reconnect_cookie = reconnect_cookie.is_some();
            let (connection_result, framed, udp_tunnel) = match &self.config.transport {
                Transport::Direct => match Box::pin(cancelable_operation(
                    connect_direct(
                        &self.config,
                        &self.input_event_sender,
                        &self.output_event_sender,
                        cliprdr_factory,
                        rdpdr_factory,
                        reconnect_cookie,
                    ),
                    &mut self.close_receiver,
                ))
                .await
                {
                    Some(Ok(result)) => result,
                    Some(Err(error)) => {
                        if self
                            .try_auto_reconnect(
                                auto_reconnect_policy,
                                &mut reconnect_attempt,
                                auto_reconnect_cookie.as_ref(),
                            )
                            .await
                        {
                            continue;
                        }
                        if !self.send_output_event(RdpOutputEvent::ConnectionFailure(error)).await {
                            self.emit_user_initiated_termination();
                        }
                        break;
                    }
                    None => {
                        self.emit_user_initiated_termination();
                        break;
                    }
                },

                #[cfg(feature = "gateway")]
                Transport::Gateway(gw) => {
                    let connect_result = if gw.prefer_direct {
                        Box::pin(connect_preferring_direct(
                            &mut self.close_receiver,
                            connect_direct(
                                &self.config,
                                &self.input_event_sender,
                                &self.output_event_sender,
                                cliprdr_factory,
                                rdpdr_factory,
                                reconnect_cookie,
                            ),
                            || {
                                connect_gateway(
                                    &self.config,
                                    gw,
                                    &self.input_event_sender,
                                    &self.output_event_sender,
                                    cliprdr_factory,
                                    rdpdr_factory,
                                    reconnect_cookie,
                                )
                            },
                        ))
                        .await
                    } else {
                        Box::pin(cancelable_operation(
                            connect_gateway(
                                &self.config,
                                gw,
                                &self.input_event_sender,
                                &self.output_event_sender,
                                cliprdr_factory,
                                rdpdr_factory,
                                reconnect_cookie,
                            ),
                            &mut self.close_receiver,
                        ))
                        .await
                    };

                    match connect_result {
                        Some(Ok(result)) => result,
                        Some(Err(error)) => {
                            if self
                                .try_auto_reconnect(
                                    auto_reconnect_policy,
                                    &mut reconnect_attempt,
                                    auto_reconnect_cookie.as_ref(),
                                )
                                .await
                            {
                                continue;
                            }
                            if !self.send_output_event(RdpOutputEvent::ConnectionFailure(error)).await {
                                self.emit_user_initiated_termination();
                            }
                            break;
                        }
                        None => {
                            self.emit_user_initiated_termination();
                            break;
                        }
                    }
                }

                Transport::RDCleanPath(rdcp) => match Box::pin(cancelable_operation(
                    connect_rdcleanpath_transport(
                        &self.config,
                        rdcp,
                        &self.input_event_sender,
                        &self.output_event_sender,
                        cliprdr_factory,
                        rdpdr_factory,
                        reconnect_cookie,
                    ),
                    &mut self.close_receiver,
                ))
                .await
                {
                    Some(Ok(result)) => result,
                    Some(Err(error)) => {
                        if self
                            .try_auto_reconnect(
                                auto_reconnect_policy,
                                &mut reconnect_attempt,
                                auto_reconnect_cookie.as_ref(),
                            )
                            .await
                        {
                            continue;
                        }
                        if !self.send_output_event(RdpOutputEvent::ConnectionFailure(error)).await {
                            self.emit_user_initiated_termination();
                        }
                        break;
                    }
                    None => {
                        self.emit_user_initiated_termination();
                        break;
                    }
                },

                #[cfg(windows)]
                Transport::NamedPipe { path } => match Box::pin(cancelable_operation(
                    connect_named_pipe(
                        &self.config,
                        path,
                        &self.input_event_sender,
                        &self.output_event_sender,
                        cliprdr_factory,
                        rdpdr_factory,
                        reconnect_cookie,
                    ),
                    &mut self.close_receiver,
                ))
                .await
                {
                    Some(Ok(result)) => result,
                    Some(Err(error)) => {
                        if self
                            .try_auto_reconnect(
                                auto_reconnect_policy,
                                &mut reconnect_attempt,
                                auto_reconnect_cookie.as_ref(),
                            )
                            .await
                        {
                            continue;
                        }
                        if !self.send_output_event(RdpOutputEvent::ConnectionFailure(error)).await {
                            self.emit_user_initiated_termination();
                        }
                        break;
                    }
                    None => {
                        self.emit_user_initiated_termination();
                        break;
                    }
                },
            };

            // A successful ARC connection consumes the session-bound cookie.
            // Only a subsequent Save Session Info PDU may provide a replacement.
            if used_auto_reconnect_cookie {
                auto_reconnect_cookie = None;
            }

            if reconnect_attempt == 0 {
                if let Some(monitor_layout) = connection_result.monitor_layout.as_ref()
                    && !self
                        .send_output_event(RdpOutputEvent::MonitorLayout(monitor_layout.monitors.clone()))
                        .await
                {
                    self.emit_user_initiated_termination();
                    break;
                }
                if !self.send_output_event(RdpOutputEvent::Connected).await {
                    self.emit_user_initiated_termination();
                    break;
                }
            }

            let input_keepalive_interval =
                match (self.config.input_keepalive_interval, self.config.fake_events_interval) {
                    (Some(input_keepalive), Some(fake_events)) => Some(input_keepalive.min(fake_events)),
                    (input_keepalive, fake_events) => input_keepalive.or(fake_events),
                };
            match active_session(
                framed,
                connection_result,
                udp_tunnel,
                self.config.rail_initial_execute.clone(),
                &self.output_event_sender,
                self.desktop_update_handler.as_deref(),
                &mut self.input_event_receiver,
                &mut self.clipboard_event_receiver,
                &mut self.close_receiver,
                &mut self.graceful_close_receiver,
                self.config.input_send_interval,
                input_keepalive_interval,
                &mut auto_reconnect_cookie,
                &mut reconnect_attempt,
            )
            .await
            {
                Ok(RdpControlFlow::ReconnectWithNewSize { width, height, reason }) => {
                    if !self
                        .send_output_event(RdpOutputEvent::DisplayResizeFallback(reason))
                        .await
                    {
                        self.emit_user_initiated_termination();
                        break;
                    }
                    // A resize fallback intentionally establishes a new session, not an
                    // automatic reconnection to the old one.
                    auto_reconnect_cookie = None;
                    reconnect_attempt = 0;
                    self.config.connector.desktop_size.width = width;
                    self.config.connector.desktop_size.height = height;
                }
                Ok(RdpControlFlow::TerminatedGracefully(reason)) => {
                    if !self.send_output_event(RdpOutputEvent::Terminated(Ok(reason))).await {
                        self.emit_user_initiated_termination();
                    }
                    break;
                }
                Ok(RdpControlFlow::TransportFailure(error)) => {
                    if self
                        .try_auto_reconnect(
                            auto_reconnect_policy,
                            &mut reconnect_attempt,
                            auto_reconnect_cookie.as_ref(),
                        )
                        .await
                    {
                        continue;
                    }
                    if !self.send_output_event(RdpOutputEvent::Terminated(Err(error))).await {
                        self.emit_user_initiated_termination();
                    }
                    break;
                }
                Err(e) => {
                    if !self.send_output_event(RdpOutputEvent::Terminated(Err(e))).await {
                        self.emit_user_initiated_termination();
                    }

                    break;
                }
            }
        }
    }

    async fn send_output_event(&mut self, event: RdpOutputEvent) -> bool {
        send_cancellable_output_event(&self.output_event_sender, event, &mut self.close_receiver)
            .await
            .unwrap_or(false)
    }

    async fn try_auto_reconnect(
        &mut self,
        policy: Option<AutoReconnectPolicy>,
        reconnect_attempt: &mut u32,
        cookie: Option<&ServerAutoReconnect>,
    ) -> bool {
        let Some(policy) = policy else {
            return false;
        };
        let Some(attempt) = policy.next_attempt(*reconnect_attempt, cookie.is_some()) else {
            return false;
        };

        *reconnect_attempt = attempt;
        self.confirm_auto_reconnect(attempt, policy.maximum_attempts).await
    }

    async fn confirm_auto_reconnect(&mut self, attempt: u32, maximum_attempts: u32) -> bool {
        let (response, receiver) = oneshot::channel();
        if !self
            .send_output_event(RdpOutputEvent::AutoReconnecting {
                disconnect_reason: 0,
                attempt,
                maximum_attempts,
                response,
            })
            .await
        {
            return false;
        }

        let decision = cancelable_operation(
            tokio::time::timeout(Duration::from_secs(30), receiver),
            &mut self.close_receiver,
        )
        .await;
        if !matches!(decision, Some(Ok(Ok(AutoReconnectDecision::Continue)))) {
            return false;
        }

        // Keep bounded retries from exhausting their budget before a transient
        // network failure has had time to clear. A close during the wait is
        // handled by the connection loop before another connection attempt starts.
        cancelable_operation(tokio::time::sleep(Duration::from_secs(1)), &mut self.close_receiver)
            .await
            .is_some()
    }

    fn emit_user_initiated_termination(&self) {
        let _ = self
            .output_event_sender
            .try_send(RdpOutputEvent::Terminated(Ok(GracefulDisconnectReason::UserInitiated)));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AutoReconnectPolicy {
    maximum_attempts: u32,
}

impl AutoReconnectPolicy {
    const fn new(maximum_attempts: u32) -> Self {
        Self { maximum_attempts }
    }

    const fn next_attempt(self, previous_attempt: u32, has_cookie: bool) -> Option<u32> {
        if has_cookie && previous_attempt < self.maximum_attempts {
            Some(previous_attempt + 1)
        } else {
            None
        }
    }
}

fn is_transport_read_error(error: &io::Error) -> bool {
    !error
        .get_ref()
        .is_some_and(|source| source.is::<ironrdp_core::DecodeError>())
}

fn is_transport_session_error(error: &(dyn core::error::Error + 'static)) -> bool {
    let mut source = Some(error);
    while let Some(error) = source {
        if let Some(error) = error.downcast_ref::<io::Error>() {
            return is_transport_read_error(error);
        }
        source = error.source();
    }
    false
}

async fn cancelable_operation<T>(
    operation: impl Future<Output = T>,
    close_receiver: &mut watch::Receiver<bool>,
) -> Option<T> {
    if *close_receiver.borrow_and_update() {
        return None;
    }

    tokio::select! {
        biased;
        _ = close_receiver.changed() => None,
        result = operation => Some(result),
    }
}

/// Detect-mode connection: try a direct RDP connection, then the gateway.
///
/// Returns `None` if the session is cancelled before a connection result is
/// produced. A cancelled direct attempt does not start the gateway.
#[doc(hidden)]
pub async fn connect_preferring_direct<T, E, GFut>(
    close_receiver: &mut watch::Receiver<bool>,
    connect_direct: impl Future<Output = Result<T, E>>,
    connect_gateway: impl FnOnce() -> GFut,
) -> Option<Result<T, E>>
where
    E: core::fmt::Display,
    GFut: Future<Output = Result<T, E>>,
{
    match Box::pin(cancelable_operation(connect_direct, close_receiver)).await {
        Some(Ok(result)) => Some(Ok(result)),
        Some(Err(direct_error)) => {
            info!(
                error = %direct_error,
                "Direct connection failed; falling back to RD Gateway"
            );
            Box::pin(cancelable_operation(connect_gateway(), close_receiver)).await
        }
        None => None,
    }
}

async fn send_cancellable_output_event(
    output_event_sender: &crate::output_channel::OutputEventSender,
    event: RdpOutputEvent,
    close_receiver: &mut watch::Receiver<bool>,
) -> Result<bool, mpsc::error::SendError<RdpOutputEvent>> {
    output_event_sender.send_cancellable(event, close_receiver).await
}

async fn send_active_output_event(
    output_event_sender: &crate::output_channel::OutputEventSender,
    event: RdpOutputEvent,
    close_receiver: &mut watch::Receiver<bool>,
) -> SessionResult<bool> {
    send_cancellable_output_event(output_event_sender, event, close_receiver)
        .await
        .map_err(|error| ironrdp_session::custom_err!("output_event_sender", error))
}

// ── Connector builder ─────────────────────────────────────────────────────────

/// Reference to the cliprdr backend factory threaded into the connect helpers.
///
/// Collapses to a zero-sized placeholder when the `clipboard` feature is disabled, so the
/// connect-helper signatures don't need `#[cfg]` on this parameter.
#[cfg(feature = "clipboard")]
type CliprdrFactoryRef<'a> = Option<&'a (dyn CliprdrBackendFactory + Send)>;
#[cfg(not(feature = "clipboard"))]
type CliprdrFactoryRef<'a> = core::marker::PhantomData<&'a ()>;
#[cfg(feature = "rdpdr")]
type RdpdrFactoryRef<'a> = Option<&'a (dyn RdpdrBackendFactory + Send)>;
#[cfg(not(feature = "rdpdr"))]
type RdpdrFactoryRef<'a> = core::marker::PhantomData<&'a ()>;

#[cfg(all(feature = "gateway", any(feature = "clipboard", feature = "rdpdr")))]
const HTTP_TUNNEL_REDIR_DISABLE_ALL: u32 = 0x4000_0000;
#[cfg(all(feature = "gateway", feature = "rdpdr"))]
const HTTP_TUNNEL_REDIR_DISABLE_DRIVE: u32 = 0x0000_0001;
#[cfg(all(feature = "gateway", feature = "clipboard"))]
const HTTP_TUNNEL_REDIR_DISABLE_CLIPBOARD: u32 = 0x0000_0008;

#[cfg(all(feature = "gateway", any(feature = "clipboard", feature = "rdpdr")))]
#[derive(Clone, Copy)]
struct GatewayRedirectionPolicy {
    flags: Option<u32>,
}

#[cfg(all(feature = "gateway", any(feature = "clipboard", feature = "rdpdr")))]
impl GatewayRedirectionPolicy {
    fn from_flags(flags: Option<u32>) -> Self {
        Self { flags }
    }

    #[cfg(feature = "clipboard")]
    fn disables_clipboard(self) -> bool {
        self.disables(HTTP_TUNNEL_REDIR_DISABLE_CLIPBOARD)
    }

    #[cfg(feature = "rdpdr")]
    fn disables_drive(self) -> bool {
        self.disables(HTTP_TUNNEL_REDIR_DISABLE_DRIVE)
    }

    #[cfg(any(feature = "clipboard", feature = "rdpdr"))]
    fn flags(self) -> Option<u32> {
        self.flags
    }

    #[cfg(any(feature = "clipboard", feature = "rdpdr"))]
    fn disables_all(self) -> bool {
        self.flags
            .is_some_and(|flags| flags & HTTP_TUNNEL_REDIR_DISABLE_ALL != 0)
    }

    fn disables(self, disabled_flag: u32) -> bool {
        self.flags.is_some_and(|flags| flags & disabled_flag != 0) || self.disables_all()
    }
}

#[cfg(feature = "rdpdr")]
#[derive(Debug)]
struct RdpdrBackendBuildError(Box<dyn core::error::Error + Send + Sync>);

#[cfg(feature = "rdpdr")]
impl core::fmt::Display for RdpdrBackendBuildError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(feature = "rdpdr")]
impl core::error::Error for RdpdrBackendBuildError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}

#[cfg(feature = "rdpdr")]
fn build_rdpdr_channel(
    factory: RdpdrFactoryRef<'_>,
    config: &crate::config::RdpdrConfig,
    allow_drives: bool,
) -> ConnectorResult<Option<ironrdp_rdpdr::Rdpdr>> {
    if !config.enabled {
        return Ok(None);
    }
    let Some(factory) = factory else {
        return Ok(None);
    };

    let product = factory
        .build_rdpdr_backend()
        .map_err(|error| ironrdp_connector::custom_err!("build RDPDR backend", RdpdrBackendBuildError(error)))?;
    let drive_hotplug = allow_drives && product.drive_hotplug();
    let printer = product.printer().cloned();
    let (backend, initial_drives) = product.into_parts();
    // Gateway drive restrictions do not apply to smartcard redirection, which shares RDPDR.
    let initial_drives = if allow_drives { initial_drives } else { Vec::new() };

    #[cfg(feature = "smartcard")]
    let smartcard = config.smartcard;
    #[cfg(not(feature = "smartcard"))]
    let smartcard = false;

    if let Some(printer) = &printer {
        let collides_with_drive = initial_drives
            .iter()
            .any(|drive| drive.device_id() == printer.device_id());
        let collides_with_smartcard = smartcard && printer.device_id() == 0;
        if collides_with_drive || collides_with_smartcard {
            return Err(ironrdp_connector::custom_err!(
                "build RDPDR channel",
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "RDPDR printer device ID is already configured"
                )
            ));
        }
    }

    if initial_drives.is_empty() && !smartcard && !drive_hotplug && printer.is_none() {
        return Ok(None);
    }

    let initial_drives = initial_drives
        .into_iter()
        .map(RdpdrDrive::into_parts)
        .collect::<Vec<_>>();

    // Do not advertise drive capability for smartcard-only products.
    // Skipping `with_drives` omits CAP_DRIVE_REDIRECT, so later
    // `Rdpdr::add_dynamic_drive` hot-plug is unavailable on that session.
    // Prefer attaching an empty drive list only when drive hot-plug is required.
    let mut rdpdr_channel = ironrdp_rdpdr::Rdpdr::new(backend, "IronRDP".to_owned());
    if !initial_drives.is_empty() || drive_hotplug {
        rdpdr_channel = rdpdr_channel.with_drives(Some(initial_drives));
    }
    if let Some(printer) = printer {
        let (device_id, name, driver_name, network) = printer.into_parts();
        rdpdr_channel = rdpdr_channel.with_printer_driver_and_network(device_id, name, driver_name, network);
    }

    #[cfg(feature = "smartcard")]
    let rdpdr_channel = if smartcard {
        rdpdr_channel.with_smartcard(0)
    } else {
        rdpdr_channel
    };

    Ok(Some(rdpdr_channel))
}

#[cfg(any(feature = "sound", feature = "rdpdr"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RdpsndBackendKind {
    #[cfg(feature = "sound")]
    Playback,
    #[cfg(feature = "rdpdr")]
    Noop,
}

#[cfg(any(feature = "sound", feature = "rdpdr"))]
fn rdpsnd_backend_kind(audio_playback: bool, rdpdr_attached: bool) -> Option<RdpsndBackendKind> {
    match (audio_playback, rdpdr_attached) {
        #[cfg(feature = "sound")]
        (true, _) => Some(RdpsndBackendKind::Playback),
        #[cfg(feature = "rdpdr")]
        (_, true) => Some(RdpsndBackendKind::Noop),
        _ => None,
    }
}

/// Listener that hands out a fresh [`ironrdp_rdpeai::client::RdpeaiClient`] on every
/// `DVC Create Request` for the AUDIO_INPUT channel.
///
/// Windows opens and closes the AUDIO_INPUT (MS-RDPEAI) dynamic virtual channel repeatedly
/// over the lifetime of a session, as applications on the remote side grab and release the
/// microphone. Registering this channel via `with_dynamic_channel` (the `OnceListener` path)
/// hands out its `RdpeaiClient` exactly once: after the first Close, `OnceListener::create`
/// keeps returning `None`, so the client answers every later Create Request with
/// `NO_LISTENER` and the microphone silently stops working for the rest of the session.
/// `with_listener` + this factory recreates the processor (and the underlying capture
/// backend/cpal stream) on every open, matching how the RDPEWA listener above is wired.
#[cfg(feature = "sound")]
struct RdpeaiListener {
    sender: RdpInputSender,
}

#[cfg(feature = "sound")]
impl DvcChannelListener for RdpeaiListener {
    fn channel_name(&self) -> &str {
        ironrdp_rdpeai::CHANNEL_NAME
    }

    fn create(&mut self, _channel_id: DynamicChannelId) -> Option<Box<dyn DvcClientProcessor>> {
        let sender = self.sender.clone();
        Some(Box::new(ironrdp_rdpeai::client::RdpeaiClient::new(
            Box::new(RdpeaiCaptureBackend::new()),
            Box::new(move |channel_id, messages| {
                sender
                    .try_send(RdpInputEvent::SendDvcMessages { channel_id, messages })
                    .map_err(|_| pdu_other_err!("send AUDIO_INPUT messages to the event loop"))?;
                Ok(())
            }),
        )))
    }

    fn is_available(&self) -> bool {
        true
    }
}

#[cfg(feature = "udp")]
fn reliable_udp_multitransport_flags(enabled: bool) -> Option<ironrdp_pdu::gcc::MultiTransportFlags> {
    enabled.then_some(
        ironrdp_pdu::gcc::MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR
            | ironrdp_pdu::gcc::MultiTransportFlags::SOFT_SYNC_TCP_TO_UDP,
    )
}

/// Build a fully wired [`ironrdp_connector::ClientConnector`] with all feature-gated channels attached.
///
/// This helper is used by all transport paths.
/// The CLIPRDR and RDPDR backends are built here for each connection attempt.
#[expect(
    clippy::too_many_arguments,
    reason = "the connector combines independent channel factories, carrier capabilities, and reconnect state"
)]
fn build_connector(
    config: &Config,
    client_addr: SocketAddr,
    event_senders: (&RdpInputSender, &crate::output_channel::OutputEventSender),
    cliprdr_factory: CliprdrFactoryRef<'_>,
    rdpdr_factory: RdpdrFactoryRef<'_>,
    rdpdr_drives_allowed: bool,
    enable_udp: bool,
    auto_reconnect_cookie: Option<&ServerAutoReconnect>,
) -> ConnectorResult<ironrdp_connector::ClientConnector> {
    let (input_sender, output_event_sender) = event_senders;

    // `input_sender` is only consumed by the optional DVC wirings below, and `cliprdr_factory`
    // only by the optional CLIPRDR attachment; discard them explicitly when those are compiled out.
    #[cfg(not(any(
        feature = "dvc-pipe-proxy",
        all(windows, feature = "dvc-com-plugin"),
        feature = "sound",
        all(windows, feature = "webauthn")
    )))]
    let _ = input_sender;
    #[cfg(not(feature = "clipboard"))]
    let _ = cliprdr_factory;
    #[cfg(not(feature = "rdpdr"))]
    let _ = (rdpdr_factory, rdpdr_drives_allowed);
    #[cfg(not(all(windows, feature = "vmconnect")))]
    let _ = output_event_sender;

    // The client-side compositor (ironrdp-egfx) holds the surface pixel state and
    // the session drains it into the framebuffer, so the graphics pipeline's
    // per-command notification callbacks are unused here. Without an H.264 decoder
    // the client advertises only the non-AVC capability sets it can actually decode.
    struct EgfxHandler;
    impl GraphicsPipelineHandler for EgfxHandler {}

    let mut drdynvc = ironrdp_dvc::DrdynvcClient::new()
        .with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new())))
        .with_dynamic_channel(EchoClient::new())
        .with_dynamic_channel(RdpeiClient::default())
        .with_dynamic_channel(GraphicsPipelineClient::new(Box::new(EgfxHandler), None));

    #[cfg(feature = "location")]
    if config.channels.location {
        drdynvc = drdynvc.with_dynamic_channel(LocationClient::new());
    }

    #[cfg(all(windows, feature = "vmconnect"))]
    if config.vmconnect_framebuffer_redirection() {
        let output_event_sender = output_event_sender.clone();
        drdynvc = drdynvc.with_dynamic_channel(ironrdp_vmconnect::FrameBufferClient::new(
            move |buffer, width, height| {
                let event = RdpOutputEvent::Image {
                    buffer,
                    width: NonZeroU16::new(width).expect("fbr validates nonzero width"),
                    height: NonZeroU16::new(height).expect("fbr validates nonzero height"),
                };
                // `Image` is `DropPolicy::LatestOnly` (see `RdpOutputEvent::drop_policy`), so
                // `try_send` always succeeds: a full queue is impossible for this variant, it
                // always just replaces whatever frame was pending. The `Full` arm below is
                // unreachable for this call site but kept for the (unlikely) case this
                // closure is ever reused for a `MustDeliver` event.
                match output_event_sender.try_send(event) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        trace!("Dropping a Hyper-V FBR frame because the output queue is full");
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        debug!("Dropping a Hyper-V FBR frame because the output queue is closed");
                    }
                }
            },
        ));
    }

    // Attach DVC pipe proxies.
    #[cfg(feature = "dvc-pipe-proxy")]
    for proxy in &config.dvc_pipe_proxies {
        let channel_name = proxy.channel_name.clone();
        let pipe_name = proxy.pipe_name.clone();
        trace!(%channel_name, %pipe_name, "Creating DVC pipe proxy");
        let sender = input_sender.clone();
        drdynvc = drdynvc.with_dynamic_channel(DvcNamedPipeProxy::new(
            &channel_name,
            &pipe_name,
            move |channel_id, messages| {
                sender
                    .try_send(RdpInputEvent::SendDvcMessages { channel_id, messages })
                    .map_err(|_| pdu_other_err!("send DVC messages to the event loop"))?;
                Ok(())
            },
        ));
    }

    // WebAuthn redirection (Windows + webauthn feature).
    // Prefer System32\webauthn.dll via the DVC COM plugin host — that is the MSTSC path and the only
    // way to handle hash-only MS-RDPEWA hosts that omit clientDataJSON. Fall back to the pure-Rust
    // WebAuthN* backend when the plugin cannot be loaded.
    // IRONRDP_WEBAUTHN_FORCE_NATIVE=1 skips COM and always uses WindowsRdpewaBackend (smoke/debug).
    #[cfg(all(windows, feature = "webauthn"))]
    let mut webauthn_com_loaded = false;
    #[cfg(all(windows, feature = "webauthn"))]
    if config.channels.webauthn {
        let force_native = std::env::var_os("IRONRDP_WEBAUTHN_FORCE_NATIVE").is_some_and(|v| {
            let v = v.to_string_lossy();
            !(v.is_empty() || v == "0" || v.eq_ignore_ascii_case("false"))
        });
        if force_native {
            info!("IRONRDP_WEBAUTHN_FORCE_NATIVE set; skipping webauthn.dll COM path");
        } else {
            match ironrdp_dvc_com_plugin::webauthn_dll_path() {
                Ok(webauthn_dll) => {
                    let sender_clone = input_sender.clone();
                    match load_dvc_plugin_listeners(&webauthn_dll, move || {
                        let sender = sender_clone.clone();
                        Box::new(move |channel_id, messages| {
                            sender
                                .try_send(RdpInputEvent::SendDvcMessages { channel_id, messages })
                                .map_err(|_| pdu_other_err!("send webauthn.dll DVC messages to the event loop"))?;
                            Ok(())
                        })
                    }) {
                        Ok(listeners) => {
                            for listener in listeners {
                                info!(
                                    channel_name = %listener.channel_name(),
                                    "Registering webauthn.dll COM DVC channel listener"
                                );
                                if listener.channel_name() == ironrdp_rdpewa::CHANNEL_NAME {
                                    webauthn_com_loaded = true;
                                }
                                drdynvc = drdynvc.with_listener(listener);
                            }
                            if !webauthn_com_loaded {
                                warn!("webauthn.dll loaded but did not register WebAuthN_Channel");
                            }
                        }
                        Err(e) => {
                            warn!(
                                error = %e,
                                "Failed to load webauthn.dll COM plugin; falling back to native RDPEWA backend"
                            );
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        error = %e,
                        "Failed to resolve webauthn.dll; falling back to native RDPEWA backend"
                    );
                }
            }
        }

        if !webauthn_com_loaded {
            let sender = input_sender.clone();
            let parent_hwnd = config.webauthn_parent_hwnd.unwrap_or(0);
            info!(parent_hwnd, "Registering native RDPEWA WebAuthn channel listener");
            let write_callback = Arc::new(move |channel_id, messages| {
                sender
                    .try_send(RdpInputEvent::SendDvcMessages { channel_id, messages })
                    .map_err(|_| pdu_other_err!("send RDPEWA DVC messages to the event loop"))?;
                Ok(())
            });
            let session_state = Arc::new(WindowsRdpewaSessionState::default());
            drdynvc = drdynvc.with_listener(RdpewaClientListener::new(move || {
                let write_callback = Arc::clone(&write_callback);
                let session_state = Arc::clone(&session_state);
                RdpewaClient::new(Box::new(WindowsRdpewaBackend::new_with_session_state(
                    parent_hwnd,
                    session_state,
                )))
                .with_write_callback(move |channel_id, messages| write_callback(channel_id, messages))
            }));
        }
    }

    // Load additional DVC COM plugins (Windows + dvc-com-plugin feature).
    #[cfg(all(windows, feature = "dvc-com-plugin"))]
    {
        for plugin_path in &config.dvc_plugins {
            #[cfg(all(windows, feature = "webauthn"))]
            if config.channels.webauthn
                && plugin_path
                    .file_name()
                    .is_some_and(|file_name| file_name.to_string_lossy().eq_ignore_ascii_case("webauthn.dll"))
            {
                debug!(
                    dll = %plugin_path.display(),
                    "Skipping explicit webauthn.dll plugin; already handled by webauthn feature"
                );
                continue;
            }

            info!(dll = %plugin_path.display(), "Loading DVC COM plugin");
            let sender_clone = input_sender.clone();
            match load_dvc_plugin_listeners(plugin_path, move || {
                let sender = sender_clone.clone();
                Box::new(move |channel_id, messages| {
                    sender
                        .try_send(RdpInputEvent::SendDvcMessages { channel_id, messages })
                        .map_err(|_| pdu_other_err!("send COM DVC messages to the event loop"))?;
                    Ok(())
                })
            }) {
                Ok(listeners) => {
                    for listener in listeners {
                        info!(channel_name = %listener.channel_name(), "Registered COM DVC channel listener");
                        drdynvc = drdynvc.with_listener(listener);
                    }
                }
                Err(e) => {
                    error!(dll = %plugin_path.display(), error = %e, "Failed to load DVC COM plugin");
                }
            }
        }
    }

    // Attach user-defined DVC channels from the extension registry.
    for attach_dvc in &config.extensions.dvc_channels {
        attach_dvc(&mut drdynvc, &config.properties);
    }

    // AUDIO_INPUT (MS-RDPEAI) microphone capture DVC.
    #[cfg(feature = "sound")]
    if config.channels.audio_capture {
        drdynvc = drdynvc.with_listener(RdpeaiListener {
            sender: input_sender.clone(),
        });
    }

    // Clone the connector config so we can apply runtime overrides before handing it to the
    // connector.  We want to set `enable_audio_playback` consistently with `channels.sound`.
    let mut connector_config = config.connector.clone();
    #[cfg(feature = "udp")]
    {
        connector_config.multitransport_flags = reliable_udp_multitransport_flags(enable_udp);
    }
    #[cfg(not(feature = "udp"))]
    let _ = enable_udp;

    // If sound is disabled at runtime (or the feature is off) ensure the connector doesn't
    // advertise audio support, which would confuse the server.
    #[cfg(not(feature = "sound"))]
    {
        connector_config.enable_audio_playback = false;
        connector_config.enable_audio_capture = false;
    }
    #[cfg(feature = "sound")]
    if !config.channels.sound {
        connector_config.enable_audio_playback = false;
    }
    #[cfg(feature = "sound")]
    {
        connector_config.enable_audio_capture = config.channels.audio_capture;
    }

    // Honor the runtime QOI/QOIZ codec toggles. Both codecs are compiled in and advertised by
    // default, but can be disabled at runtime; when disabled we drop them from the advertised
    // bitmap codec list so the server won't negotiate them.
    #[cfg(any(feature = "qoi", feature = "qoiz"))]
    if let Some(bitmap) = connector_config.bitmap.as_mut() {
        use ironrdp_pdu::rdp::capability_sets::CodecProperty;

        bitmap.codecs.0.retain(|codec| match codec.property {
            #[cfg(feature = "qoi")]
            CodecProperty::Qoi => config.channels.qoi,
            #[cfg(feature = "qoiz")]
            CodecProperty::QoiZ => config.channels.qoiz,
            _ => true,
        });
    }

    #[cfg(any(feature = "sound", feature = "rdpdr"))]
    let audio_playback = connector_config.enable_audio_playback;
    let rail_client = connector_config.remote_application_mode.then(|| {
        let rail_client = RailClient::new(
            connector_config.client_build,
            connector_config.desktop_size.width,
            connector_config.desktop_size.height,
        );
        if let Some(flags) = config.rail_client_status_flags {
            rail_client.with_client_status_flags(flags)
        } else {
            rail_client
        }
    });

    let mut connector =
        ironrdp_connector::ClientConnector::new(connector_config, client_addr).with_static_channel(drdynvc);

    if let Some(load_balance_info) = &config.load_balance_info {
        connector = connector.with_load_balance_info(load_balance_info.clone());
    }

    if config.administrative_session {
        connector = connector.with_cluster_data(ironrdp_pdu::gcc::ClientClusterData {
            flags: ironrdp_pdu::gcc::RedirectionFlags::REDIRECTION_SUPPORTED
                | ironrdp_pdu::gcc::RedirectionFlags::REDIRECTED_SESSION_FIELD_VALID,
            redirection_version: ironrdp_pdu::gcc::RedirectionVersion::V6,
            redirected_session_id: 0,
        });
    }

    if let Some(rail_client) = rail_client {
        connector = connector.with_static_channel(rail_client);
    }

    #[cfg(feature = "rdpdr")]
    let rdpdr_channel =
        build_rdpdr_channel(rdpdr_factory, &config.channels.rdpdr, rdpdr_drives_allowed)?.or_else(|| {
            config
                .channels
                .rdpdr
                .enabled
                .then(|| ironrdp_rdpdr::Rdpdr::new(Box::new(ironrdp_rdpdr::NoopRdpdrBackend), "IronRDP".to_owned()))
        });
    #[cfg(feature = "rdpdr")]
    input_sender.set_rdpdr_drive_hotplug_available(
        rdpdr_channel
            .as_ref()
            .is_some_and(ironrdp_rdpdr::Rdpdr::drive_hotplug_available),
    );

    // Windows servers only issue RDPDR traffic when RDPSND is also advertised.
    #[cfg(any(feature = "sound", feature = "rdpdr"))]
    {
        #[cfg(feature = "rdpdr")]
        let rdpdr_attached = rdpdr_channel.is_some();
        #[cfg(not(feature = "rdpdr"))]
        let rdpdr_attached = false;

        if let Some(kind) = rdpsnd_backend_kind(audio_playback, rdpdr_attached) {
            match kind {
                #[cfg(feature = "sound")]
                RdpsndBackendKind::Playback => {
                    connector = connector.with_static_channel(
                        ironrdp_rdpsnd::client::Rdpsnd::new(Box::new(cpal::RdpsndBackend::new()))
                            .with_quality_mode(config.audio_quality_mode.into_rdpsnd()),
                    );
                }
                #[cfg(feature = "rdpdr")]
                RdpsndBackendKind::Noop => {
                    connector = connector.with_static_channel(
                        ironrdp_rdpsnd::client::Rdpsnd::new(Box::new(ironrdp_rdpsnd::client::NoopRdpsndBackend))
                            .with_quality_mode(config.audio_quality_mode.into_rdpsnd()),
                    );
                }
            }
        }
    }

    #[cfg(feature = "rdpdr")]
    if let Some(rdpdr_channel) = rdpdr_channel {
        connector = connector.with_static_channel(rdpdr_channel);
    }

    // Attach CLIPRDR (clipboard redirection). The backend is built fresh per connection.
    #[cfg(feature = "clipboard")]
    if let Some(factory) = cliprdr_factory {
        let backend = factory.build_cliprdr_backend();
        connector.attach_static_channel(ironrdp_cliprdr::Cliprdr::new(backend));
    }

    // Attach user-defined static channels from the extension registry.
    for attach_sc in &config.extensions.static_channels {
        attach_sc(&mut connector, &config.properties);
    }

    if let Some(cookie) = auto_reconnect_cookie {
        connector = connector.with_auto_reconnect_cookie(cookie.clone());
    }

    Ok(connector)
}

// ── Transport-specific connect helpers ────────────────────────────────────────

trait AsyncReadWrite: AsyncRead + AsyncWrite {}
impl<T> AsyncReadWrite for T where T: AsyncRead + AsyncWrite {}
type UpgradedFramed = ironrdp_tokio::TokioFramed<Box<dyn AsyncReadWrite + Unpin + Send + Sync>>;

// mstsc's input handler reads this threshold from the `EventsAtOnce` property, whose default is 10.
const INPUT_BATCH_EVENT_LIMIT: usize = 10;

struct FastPathInputBatcher {
    interval: Duration,
    last_send: tokio::time::Instant,
    pending: SmallVec<[FastPathInputEvent; INPUT_BATCH_EVENT_LIMIT]>,
}

impl FastPathInputBatcher {
    fn new(interval: Option<Duration>, now: tokio::time::Instant) -> Self {
        Self {
            interval: interval.unwrap_or(Duration::ZERO),
            last_send: now,
            pending: SmallVec::new(),
        }
    }

    fn deadline(&self) -> Option<tokio::time::Instant> {
        (!self.pending.is_empty()).then_some(self.last_send + self.interval)
    }

    fn queue(
        &mut self,
        events: impl IntoIterator<Item = FastPathInputEvent>,
        now: tokio::time::Instant,
    ) -> Option<SmallVec<[FastPathInputEvent; INPUT_BATCH_EVENT_LIMIT]>> {
        let mut force = false;
        for event in events {
            force |= !matches!(
                event,
                FastPathInputEvent::MouseEvent(MousePdu {
                    flags: PointerFlags::MOVE,
                    number_of_wheel_rotation_units: 0,
                    ..
                })
            );
            self.pending.push(event);
        }
        if force
            || self.interval.is_zero()
            || self.pending.len() >= INPUT_BATCH_EVENT_LIMIT
            || now.duration_since(self.last_send) >= self.interval
        {
            self.flush(now)
        } else {
            None
        }
    }

    fn queue_forced(
        &mut self,
        events: impl IntoIterator<Item = FastPathInputEvent>,
        now: tokio::time::Instant,
    ) -> SmallVec<[FastPathInputEvent; INPUT_BATCH_EVENT_LIMIT]> {
        self.pending.extend(events);
        self.flush(now).unwrap_or_default()
    }

    fn flush(&mut self, now: tokio::time::Instant) -> Option<SmallVec<[FastPathInputEvent; INPUT_BATCH_EVENT_LIMIT]>> {
        if self.pending.is_empty() {
            return None;
        }
        self.last_send = now;
        Some(core::mem::take(&mut self.pending))
    }
}

#[derive(Default)]
struct UdpTunnel {
    #[cfg(feature = "udp")]
    transport: Option<ironrdp_rdpeudp_tokio::UdpTransport>,
    #[cfg(feature = "udp")]
    bootstrap: Option<UdpBootstrapConfig>,
    #[cfg(feature = "udp")]
    attempted_protocols: Vec<ironrdp_pdu::rdp::multitransport::RequestedProtocol>,
}

#[cfg(feature = "udp")]
#[derive(Clone)]
struct UdpBootstrapConfig {
    peer: SocketAddr,
    server_name: String,
    tls: ironrdp_rdpeudp_tokio::UdpTlsConfig,
}

#[cfg(feature = "udp")]
#[derive(Default)]
struct UdpBootstrapState {
    transport: Option<ironrdp_rdpeudp_tokio::UdpTransport>,
    attempted_protocols: Vec<ironrdp_pdu::rdp::multitransport::RequestedProtocol>,
}

#[cfg(feature = "udp")]
async fn bootstrap_udp_transport(
    request: ironrdp_pdu::rdp::multitransport::MultitransportRequestPdu,
    config: UdpBootstrapConfig,
) -> ConnectorResult<ironrdp_rdpeudp_tokio::UdpTransport> {
    let mut bootstrap = ironrdp_rdpeudp_tokio::MultitransportBootstrap::new(request);
    bootstrap
        .connect(
            config.peer,
            config.server_name,
            ironrdp_rdpeudp::ConnectionConfig::default(),
            config.tls,
        )
        .await
        .map_err(|error| {
            let failure = match error.kind() {
                ironrdp_rdpeudp_tokio::UdpTransportErrorKind::Socket(_) => "socket",
                ironrdp_rdpeudp_tokio::UdpTransportErrorKind::Handshake(_) => "handshake",
                ironrdp_rdpeudp_tokio::UdpTransportErrorKind::HandshakeTimeout => "handshake-timeout",
                ironrdp_rdpeudp_tokio::UdpTransportErrorKind::Tls(_) => "tls",
                ironrdp_rdpeudp_tokio::UdpTransportErrorKind::TlsTimeout => "tls-timeout",
                ironrdp_rdpeudp_tokio::UdpTransportErrorKind::Rdpemt(_) => "rdpemt",
                ironrdp_rdpeudp_tokio::UdpTransportErrorKind::TunnelTimeout => "tunnel-timeout",
                ironrdp_rdpeudp_tokio::UdpTransportErrorKind::TunnelRejected { .. } => "tunnel-rejected",
                ironrdp_rdpeudp_tokio::UdpTransportErrorKind::DriverPanic => "driver-panic",
                ironrdp_rdpeudp_tokio::UdpTransportErrorKind::Driver(_) => "driver",
                ironrdp_rdpeudp_tokio::UdpTransportErrorKind::UnsupportedProtocol { .. } => "unsupported-protocol",
                ironrdp_rdpeudp_tokio::UdpTransportErrorKind::PayloadTooLarge { .. } => "payload-too-large",
                _ => "unknown",
            };
            warn!(failure, "UDP multitransport bootstrap failed");
            ironrdp_connector::custom_err!("UDP multitransport bootstrap", error)
        })?;
    bootstrap
        .take_transport()
        .ok_or_else(|| ironrdp_connector::general_err!("successful UDP bootstrap did not produce a transport"))
}

type ConnectOutput = (ConnectionResult, UpgradedFramed, UdpTunnel);

/// Direct TCP → TLS connection (no gateway).
async fn connect_direct(
    config: &Config,
    input_sender: &RdpInputSender,
    output_event_sender: &crate::output_channel::OutputEventSender,
    cliprdr_factory: CliprdrFactoryRef<'_>,
    rdpdr_factory: RdpdrFactoryRef<'_>,
    auto_reconnect_cookie: Option<&ServerAutoReconnect>,
) -> ConnectorResult<ConnectOutput> {
    let dest = config.destination.to_string();
    let stream = TcpStream::connect(&dest)
        .await
        .map_err(|e| ironrdp_connector::custom_err!("TCP connect", e))?;
    #[cfg(feature = "vmconnect")]
    let pcb_deadline = tokio::time::Instant::now() + ironrdp_vmconnect::PCB_TRANSMIT_DEADLINE;
    let client_addr = stream
        .local_addr()
        .map_err(|e| ironrdp_connector::custom_err!("get socket local address", e))?;
    let framed = ironrdp_tokio::TokioFramed::new(stream);
    #[cfg(feature = "udp")]
    let enable_udp = config.udp_transport_enabled && !config.connector.enable_standard_rdp_security && {
        #[cfg(feature = "vmconnect")]
        {
            config.vm_id().is_none()
        }
        #[cfg(not(feature = "vmconnect"))]
        {
            true
        }
    };
    #[cfg(not(feature = "udp"))]
    let enable_udp = false;
    #[cfg(feature = "udp")]
    let udp_peer = if enable_udp {
        Some(
            framed
                .get_inner()
                .0
                .peer_addr()
                .map_err(|e| ironrdp_connector::custom_err!("get socket peer address", e))?,
        )
    } else {
        None
    };

    let connector = build_connector(
        config,
        client_addr,
        (input_sender, output_event_sender),
        cliprdr_factory,
        rdpdr_factory,
        true,
        enable_udp,
        auto_reconnect_cookie,
    )?;
    #[cfg(feature = "vmconnect")]
    if config.vm_id().is_some() {
        return Box::pin(vmconnect_handshake_and_finalize(
            framed,
            connector,
            config,
            pcb_deadline,
        ))
        .await;
    }
    #[cfg(not(feature = "udp"))]
    let udp_peer = None;
    Box::pin(security_upgrade_and_finalize(framed, connector, config, udp_peer)).await
}

/// Windows named-pipe RDP stream (e.g. Windows Sandbox `\\.\pipe\{VMId}`).
///
/// Opens a duplex byte-mode client pipe and runs the connector. When TLS/CredSSP are disabled
/// (Sandbox NamedPipe default), negotiation stays on PROTOCOL_RDP with ENCRYPTION_LEVEL_NONE.
#[cfg(windows)]
async fn connect_named_pipe(
    config: &Config,
    pipe_path: &str,
    input_sender: &RdpInputSender,
    output_event_sender: &crate::output_channel::OutputEventSender,
    cliprdr_factory: CliprdrFactoryRef<'_>,
    rdpdr_factory: RdpdrFactoryRef<'_>,
    auto_reconnect_cookie: Option<&ServerAutoReconnect>,
) -> ConnectorResult<ConnectOutput> {
    use tokio::net::windows::named_pipe::ClientOptions;

    let path = if pipe_path.starts_with(r"\\.\pipe\") || pipe_path.starts_with(r"\\?\pipe\") {
        pipe_path.to_owned()
    } else {
        format!(r"\\.\pipe\{pipe_path}")
    };

    info!(%path, "Connecting over Windows named pipe");

    let stream = ClientOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| ironrdp_connector::custom_err!("named pipe connect", e))?;

    // Named pipes have no socket address; use a dummy loopback address for Client Info.
    let client_addr = SocketAddr::from(([127, 0, 0, 1], 0));
    let framed = ironrdp_tokio::TokioFramed::new(stream);
    let connector = build_connector(
        config,
        client_addr,
        (input_sender, output_event_sender),
        cliprdr_factory,
        rdpdr_factory,
        true,
        false,
        auto_reconnect_cookie,
    )?;

    Box::pin(security_upgrade_and_finalize(framed, connector, config, None)).await
}

/// RDS gateway TCP → gateway auth → TLS connection.
#[cfg(feature = "gateway")]
async fn connect_gateway(
    config: &Config,
    gw: &crate::config::GatewayConfig,
    input_sender: &RdpInputSender,
    output_event_sender: &crate::output_channel::OutputEventSender,
    cliprdr_factory: CliprdrFactoryRef<'_>,
    rdpdr_factory: RdpdrFactoryRef<'_>,
    auto_reconnect_cookie: Option<&ServerAutoReconnect>,
) -> ConnectorResult<ConnectOutput> {
    use ironrdp_mstsgu::GwConnectTarget;

    // Target resource host/port come from Config::destination and are forwarded in the
    // MS-TSGU channel-create packet (enables non-3389 RDP and VMConnect port 2179).
    let gw_target = GwConnectTarget {
        gw_endpoint: gw.endpoint.clone(),
        gw_user: gw.username.clone(),
        gw_pass: gw.password.clone(),
        smart_card: None,
        server: config.destination.name().to_owned(),
    };

    let (gw_stream, client_addr) = ironrdp_mstsgu::GwClient::connect_with_port_and_certificate_validation(
        &gw_target,
        &config.connector.client_name,
        config.destination.port(),
        config.certificate_validation(),
        config.certificate_validation_callback().cloned(),
    )
    .await
    .map_err(|e| ironrdp_connector::custom_err!("GW connect", e))?;

    let tunnel_policy = gw_stream.tunnel_policy().clone();
    #[cfg(any(feature = "clipboard", feature = "rdpdr"))]
    let gateway_redirection_policy = GatewayRedirectionPolicy::from_flags(tunnel_policy.redirection_flags);
    #[cfg(feature = "clipboard")]
    let cliprdr_factory = {
        if gateway_redirection_policy.disables_clipboard() && cliprdr_factory.is_some() {
            debug!(
                redirection_flags = ?gateway_redirection_policy.flags(),
                "Suppressing CLIPRDR due to gateway device-redirection policy"
            );
        }
        (!gateway_redirection_policy.disables_clipboard())
            .then_some(cliprdr_factory)
            .flatten()
    };
    #[cfg(feature = "rdpdr")]
    let rdpdr_drives_allowed = !gateway_redirection_policy.disables_drive();
    #[cfg(feature = "rdpdr")]
    let rdpdr_factory = {
        if gateway_redirection_policy.disables_all() && rdpdr_factory.is_some() {
            debug!(
                redirection_flags = ?gateway_redirection_policy.flags(),
                "Suppressing RDPDR due to gateway device-redirection policy"
            );
        }
        if gateway_redirection_policy.disables_drive()
            && !gateway_redirection_policy.disables_all()
            && rdpdr_factory.is_some()
        {
            debug!(
                redirection_flags = ?gateway_redirection_policy.flags(),
                "Suppressing RDPDR drive redirection due to gateway device-redirection policy"
            );
        }
        (!gateway_redirection_policy.disables_all())
            .then_some(rdpdr_factory)
            .flatten()
    };
    #[cfg(not(feature = "rdpdr"))]
    let rdpdr_drives_allowed = true;
    #[cfg(not(any(feature = "clipboard", feature = "rdpdr")))]
    let _ = tunnel_policy;

    let framed = ironrdp_tokio::TokioFramed::new(gw_stream);

    let connector = build_connector(
        config,
        client_addr,
        (input_sender, output_event_sender),
        cliprdr_factory,
        rdpdr_factory,
        rdpdr_drives_allowed,
        false,
        auto_reconnect_cookie,
    )?;
    #[cfg(feature = "vmconnect")]
    if config.vm_id().is_some() {
        // The Hyper-V TCP connection is created by the gateway during channel-create, so
        // the MS-RDPEPS PCB deadline starts once that channel is established.
        let pcb_deadline = tokio::time::Instant::now() + ironrdp_vmconnect::PCB_TRANSMIT_DEADLINE;
        return Box::pin(vmconnect_handshake_and_finalize(
            framed,
            connector,
            config,
            pcb_deadline,
        ))
        .await;
    }
    Box::pin(security_upgrade_and_finalize(framed, connector, config, None)).await
}

/// RDCleanPath WebSocket → RDCleanPath handshake connection.
async fn connect_rdcleanpath_transport(
    config: &Config,
    rdcp: &RDCleanPathConfig,
    input_sender: &RdpInputSender,
    output_event_sender: &crate::output_channel::OutputEventSender,
    cliprdr_factory: CliprdrFactoryRef<'_>,
    rdpdr_factory: RdpdrFactoryRef<'_>,
    auto_reconnect_cookie: Option<&ServerAutoReconnect>,
) -> ConnectorResult<ConnectOutput> {
    let hostname = rdcp
        .url
        .host_str()
        .ok_or_else(|| ironrdp_connector::general_err!("host missing from the URL"))?;
    let port = rdcp.url.port_or_known_default().unwrap_or(443);

    let socket = TcpStream::connect((hostname, port))
        .await
        .map_err(|e| ironrdp_connector::custom_err!("TCP connect", e))?;
    socket
        .set_nodelay(true)
        .map_err(|e| ironrdp_connector::custom_err!("set TCP_NODELAY", e))?;
    let client_addr = socket
        .local_addr()
        .map_err(|e| ironrdp_connector::custom_err!("get socket local address", e))?;

    let (ws, _) = tokio_tungstenite::client_async_tls(rdcp.url.as_str(), socket)
        .await
        .map_err(|e| ironrdp_connector::custom_err!("WS connect", e))?;
    let ws = crate::ws::websocket_compat(ws);
    let mut framed = ironrdp_tokio::TokioFramed::new(ws);

    let mut connector = build_connector(
        config,
        client_addr,
        (input_sender, output_event_sender),
        cliprdr_factory,
        rdpdr_factory,
        true,
        false,
        auto_reconnect_cookie,
    )?;

    let destination = config.destination.to_string();
    let mut network_client = ReqwestNetworkClient::new();
    let server_name = ironrdp_connector::ServerName::from(&config.destination);
    let (upgraded, server_public_key) = rdcleanpath_handshake(
        &mut framed,
        &mut connector,
        &mut network_client,
        RDCleanPathHandshakeParams {
            server_name: server_name.clone(),
            destination,
            proxy_auth_token: rdcp.auth_token.clone(),
            #[cfg(feature = "vmconnect")]
            vmconnect: config
                .vm_id()
                .zip(config.vmconnect_mode())
                .map(|(id, mode)| (id.to_owned(), mode)),
            kerberos_config: config.kerberos_config.clone(),
        },
    )
    .await?;

    let connection_result = ironrdp_tokio::connect_finalize(
        upgraded,
        connector,
        &mut framed,
        &mut network_client,
        server_name,
        server_public_key,
        config.kerberos_config.clone(),
    )
    .await?;

    let (ws, leftover_bytes) = framed.into_inner();
    let erased_stream: Box<dyn AsyncReadWrite + Unpin + Send + Sync> = Box::new(ws);
    let upgraded_framed = ironrdp_tokio::TokioFramed::new_with_leftover(erased_stream, leftover_bytes);

    Ok((connection_result, upgraded_framed, UdpTunnel::default()))
}

// ── Shared security upgrade + finalize ────────────────────────────────────────

/// After X.224 negotiation, either perform TLS (enhanced security) or mark a no-op upgrade
/// for standard RDP security / plain local transports (Windows Sandbox named pipe).
async fn security_upgrade_and_finalize<S>(
    mut framed: ironrdp_tokio::TokioFramed<S>,
    mut connector: ironrdp_connector::ClientConnector,
    config: &Config,
    #[cfg_attr(
        not(feature = "udp"),
        expect(
            unused_variables,
            reason = "the UDP endpoint is used only when UDP support is compiled in"
        )
    )]
    udp_peer: Option<SocketAddr>,
) -> ConnectorResult<ConnectOutput>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    let should_upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector).await?;

    // Only the explicit standard-RDP opt-in (NamedPipe / local plain transports) skips TLS.
    // A bare `enable_tls = false` + `enable_credssp = false` still fails earlier in the connector.
    if config.connector.enable_standard_rdp_security && !config.connector.enable_tls && !config.connector.enable_credssp
    {
        debug!("Skipping TLS upgrade (standard RDP security / plain transport)");
        let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
        let (stream, leftover_bytes) = framed.into_inner();
        let erased_stream: Box<dyn AsyncReadWrite + Unpin + Send + Sync> = Box::new(stream);
        let mut upgraded_framed = ironrdp_tokio::TokioFramed::new_with_leftover(erased_stream, leftover_bytes);

        let connection_result = ironrdp_tokio::connect_finalize(
            upgraded,
            connector,
            &mut upgraded_framed,
            &mut ReqwestNetworkClient::new(),
            (&config.destination).into(),
            Vec::new(),
            config.kerberos_config.clone(),
        )
        .await?;

        return Ok((connection_result, upgraded_framed, UdpTunnel::default()));
    }

    debug!("TLS upgrade");

    let (initial_stream, leftover_bytes) = framed.into_inner();

    let tls_upgrade = if let Some(callback) = config.certificate_validation_callback() {
        ironrdp_tls::upgrade_with_certificate_validation_callback_for_endpoint(
            initial_stream,
            config.destination.name(),
            &config.destination.to_string(),
            Arc::clone(callback),
        )
        .await
    } else {
        ironrdp_tls::upgrade_with_certificate_validation(
            initial_stream,
            config.destination.name(),
            config.certificate_validation(),
        )
        .await
    };
    let (tls_stream, tls_cert) = tls_upgrade.map_err(|e| ironrdp_connector::custom_err!("TLS upgrade", e))?;

    let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);

    let erased_stream: Box<dyn AsyncReadWrite + Unpin + Send + Sync> = Box::new(tls_stream);
    let mut upgraded_framed = ironrdp_tokio::TokioFramed::new_with_leftover(erased_stream, leftover_bytes);

    let server_public_key = ironrdp_tls::extract_tls_server_public_key(&tls_cert)
        .ok_or_else(|| ironrdp_connector::general_err!("unable to extract tls server public key"))?
        .to_owned();

    #[cfg(feature = "udp")]
    if let Some(udp_peer) = udp_peer {
        let mut udp_state = UdpBootstrapState::default();
        let udp_config = UdpBootstrapConfig {
            peer: udp_peer,
            server_name: config.destination.name().to_owned(),
            tls: ironrdp_rdpeudp_tokio::UdpTlsConfig {
                certificate_validation: config.certificate_validation,
                certificate_validation_callback: config.certificate_validation_callback.clone(),
                certificate_validation_endpoint: config.destination.to_string(),
            },
        };
        let connection_result = ironrdp_tokio::connect_finalize_with_multitransport(
            upgraded,
            connector,
            &mut upgraded_framed,
            &mut ReqwestNetworkClient::new(),
            (&config.destination).into(),
            server_public_key,
            config.kerberos_config.clone(),
            async |request: ironrdp_pdu::rdp::multitransport::MultitransportRequestPdu, soft_sync: bool| {
                if udp_state.attempted_protocols.contains(&request.requested_protocol) {
                    return Ok(ironrdp_connector::MultitransportResult::Failure(
                        MultitransportResponsePdu::E_ABORT,
                    ));
                }
                udp_state.attempted_protocols.push(request.requested_protocol);

                if !soft_sync {
                    warn!(
                        request_id = request.request_id,
                        requested_protocol = ?request.requested_protocol,
                        "Rejecting multitransport request without Soft-Sync"
                    );
                    return Ok(ironrdp_connector::MultitransportResult::Failure(
                        MultitransportResponsePdu::E_ABORT,
                    ));
                }

                match bootstrap_udp_transport(request, udp_config.clone()).await {
                    Ok(transport) => {
                        udp_state.transport = Some(transport);
                        Ok(ironrdp_connector::MultitransportResult::Success)
                    }
                    Err(error) => {
                        warn!(%error, "Reliable UDP bootstrap failed; continuing with TCP");
                        Ok(ironrdp_connector::MultitransportResult::Failure(
                            MultitransportResponsePdu::E_ABORT,
                        ))
                    }
                }
            },
        )
        .await?;
        let udp_tunnel = UdpTunnel {
            transport: udp_state.transport,
            bootstrap: Some(udp_config),
            attempted_protocols: udp_state.attempted_protocols,
        };
        return Ok((connection_result, upgraded_framed, udp_tunnel));
    }

    let connection_result = ironrdp_tokio::connect_finalize(
        upgraded,
        connector,
        &mut upgraded_framed,
        &mut ReqwestNetworkClient::new(),
        (&config.destination).into(),
        server_public_key,
        config.kerberos_config.clone(),
    )
    .await?;

    Ok((connection_result, upgraded_framed, UdpTunnel::default()))
}

/// Hyper-V console connect via ironrdp-vmconnect, then shared RDP tail.
#[cfg(feature = "vmconnect")]
async fn vmconnect_handshake_and_finalize<S>(
    mut framed: ironrdp_tokio::TokioFramed<S>,
    mut connector: ironrdp_connector::ClientConnector,
    config: &Config,
    pcb_deadline: tokio::time::Instant,
) -> ConnectorResult<ConnectOutput>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static,
{
    let vm_id = config
        .vm_id()
        .ok_or_else(|| ironrdp_connector::general_err!("vmconnect path requires a VM ID"))?;
    let mode = config
        .vmconnect_mode()
        .ok_or_else(|| ironrdp_connector::general_err!("vmconnect path requires a console mode"))?;

    // MS-RDPEPS: complete PCB within ten seconds of TCP connection creation.
    let pcb_sent = tokio::time::timeout_at(
        pcb_deadline,
        ironrdp_vmconnect::send_preconnection_blob(&mut framed, vm_id, mode),
    )
    .await
    .map_err(|_| ironrdp_connector::general_err!("timed out writing preconnection blob"))??;

    debug!("TLS upgrade");

    let (initial_stream, leftover_bytes) = framed.into_inner();
    let (tls_stream, tls_cert) = ironrdp_tls::upgrade(initial_stream, config.destination.name())
        .await
        .map_err(|e| ironrdp_connector::custom_err!("TLS upgrade", e))?;

    let erased_stream: Box<dyn AsyncReadWrite + Unpin + Send + Sync> = Box::new(tls_stream);
    let mut upgraded_framed = ironrdp_tokio::TokioFramed::new_with_leftover(erased_stream, leftover_bytes);

    let server_public_key = ironrdp_tls::extract_tls_server_public_key(&tls_cert)
        .ok_or_else(|| ironrdp_connector::general_err!("unable to extract tls server public key"))?
        .to_owned();

    let mut network_client = ReqwestNetworkClient::new();
    let server_name = ironrdp_connector::ServerName::from(&config.destination);
    #[cfg(windows)]
    let upgraded = if config.vmconnect_current_user() {
        ironrdp_vmconnect::connect_front_with_current_user(
            pcb_sent,
            &mut upgraded_framed,
            &mut connector,
            server_name.clone(),
            &server_public_key,
        )
        .await?
    } else {
        ironrdp_vmconnect::connect_front(
            pcb_sent,
            &mut upgraded_framed,
            &mut connector,
            &mut network_client,
            server_name.clone(),
            &server_public_key,
            config.kerberos_config.clone(),
        )
        .await?
    };
    #[cfg(not(windows))]
    let upgraded = ironrdp_vmconnect::connect_front(
        pcb_sent,
        &mut upgraded_framed,
        &mut connector,
        &mut network_client,
        server_name.clone(),
        &server_public_key,
        config.kerberos_config.clone(),
    )
    .await?;

    let connection_result = ironrdp_tokio::connect_finalize(
        upgraded,
        connector,
        &mut upgraded_framed,
        &mut network_client,
        server_name,
        server_public_key,
        config.kerberos_config.clone(),
    )
    .await?;

    Ok((connection_result, upgraded_framed, UdpTunnel::default()))
}

// ── RDCleanPath handshake ─────────────────────────────────────────────────────

struct RDCleanPathHandshakeParams {
    server_name: ironrdp_connector::ServerName,
    destination: String,
    proxy_auth_token: String,
    #[cfg(feature = "vmconnect")]
    vmconnect: Option<(String, ironrdp_vmconnect::Mode)>,
    kerberos_config: Option<ironrdp_connector::credssp::KerberosConfig>,
}

async fn rdcleanpath_handshake<S, N>(
    framed: &mut ironrdp_tokio::Framed<S>,
    connector: &mut ironrdp_connector::ClientConnector,
    network_client: &mut N,
    params: RDCleanPathHandshakeParams,
) -> ConnectorResult<(ironrdp_tokio::Upgraded, Vec<u8>)>
where
    S: ironrdp_tokio::FramedRead + FramedWrite,
    N: ironrdp_tokio::NetworkClient,
{
    use ironrdp_connector::Sequence as _;
    use x509_cert::der::Decode as _;

    #[derive(Clone, Copy, Debug)]
    struct RDCleanPathHint;
    const RDCLEANPATH_HINT: RDCleanPathHint = RDCleanPathHint;

    impl ironrdp_pdu::PduHint for RDCleanPathHint {
        fn find_size(&self, bytes: &[u8]) -> ironrdp_core::DecodeResult<Option<(bool, usize)>> {
            match ironrdp_rdcleanpath::RDCleanPathPdu::detect(bytes) {
                ironrdp_rdcleanpath::DetectionResult::Detected { total_length, .. } => Ok(Some((true, total_length))),
                ironrdp_rdcleanpath::DetectionResult::NotEnoughBytes => Ok(None),
                ironrdp_rdcleanpath::DetectionResult::Failed => {
                    Err(ironrdp_core::other_err!("RDCleanPathHint", "detection failed"))
                }
            }
        }
    }

    let RDCleanPathHandshakeParams {
        server_name,
        destination,
        proxy_auth_token,
        #[cfg(feature = "vmconnect")]
        vmconnect,
        kerberos_config,
    } = params;

    #[cfg(feature = "vmconnect")]
    let request_vmconnect = vmconnect.is_some();
    #[cfg(not(feature = "vmconnect"))]
    let request_vmconnect = false;

    let mut buf = WriteBuf::new();
    info!("Begin RDCleanPath connection procedure");

    {
        let rdcleanpath_req = {
            #[cfg(feature = "vmconnect")]
            if let Some((vm_id, mode)) = vmconnect {
                let pcb_payload = ironrdp_vmconnect::preconnection_blob_payload(&vm_id, mode)?;
                ironrdp_rdcleanpath::RDCleanPathPdu::new_vmconnect_request(destination, proxy_auth_token, pcb_payload)
                    .map_err(|e| ironrdp_connector::custom_err!("build VMConnect RDCleanPath request", e))?
            } else {
                build_ordinary_rdcleanpath_request(connector, &mut buf, destination, proxy_auth_token)?
            }
            #[cfg(not(feature = "vmconnect"))]
            {
                build_ordinary_rdcleanpath_request(connector, &mut buf, destination, proxy_auth_token)?
            }
        };
        debug!(
            destination = ?rdcleanpath_req.destination,
            has_preconnection_blob = rdcleanpath_req.preconnection_blob.is_some(),
            has_x224_connection_pdu = rdcleanpath_req.x224_connection_pdu.is_some(),
            "Send RDCleanPath request"
        );
        let rdcleanpath_req = rdcleanpath_req
            .to_der()
            .map_err(|e| ironrdp_connector::custom_err!("encode RDCleanPath request", e))?;
        framed
            .write_all(&rdcleanpath_req)
            .await
            .map_err(|e| ironrdp_connector::custom_err!("couldn't write RDCleanPath request", e))?;
    }

    {
        let rdcleanpath_res = framed
            .read_by_hint(&RDCLEANPATH_HINT)
            .await
            .map_err(|e| ironrdp_connector::custom_err!("read RDCleanPath response", e))?;
        let rdcleanpath_res = ironrdp_rdcleanpath::RDCleanPathPdu::from_der(&rdcleanpath_res)
            .map_err(|e| ironrdp_connector::custom_err!("decode RDCleanPath response", e))?;
        debug!(message = ?rdcleanpath_res, "Received RDCleanPath PDU");

        let (x224_connection_response, server_cert_chain) = match (
            request_vmconnect,
            rdcleanpath_res
                .into_message()
                .map_err(|e| ironrdp_connector::custom_err!("invalid RDCleanPath PDU", e))?,
        ) {
            (_, ironrdp_rdcleanpath::RDCleanPathMessage::Request { .. })
            | (_, ironrdp_rdcleanpath::RDCleanPathMessage::VmConnectRequest { .. }) => {
                return Err(ironrdp_connector::general_err!(
                    "received unexpected RDCleanPath type (request)"
                ));
            }
            (
                false,
                ironrdp_rdcleanpath::RDCleanPathMessage::Response {
                    x224_connection_response,
                    server_cert_chain,
                    server_addr: _,
                },
            ) => (Some(x224_connection_response), server_cert_chain),
            (
                true,
                ironrdp_rdcleanpath::RDCleanPathMessage::VmConnectResponse {
                    server_cert_chain,
                    server_addr: _,
                },
            ) => (None, server_cert_chain),
            (true, ironrdp_rdcleanpath::RDCleanPathMessage::Response { .. }) => {
                return Err(ironrdp_connector::general_err!(
                    "response from RDCleanPath includes X.224 for a VMConnect request"
                ));
            }
            (false, ironrdp_rdcleanpath::RDCleanPathMessage::VmConnectResponse { .. }) => {
                return Err(ironrdp_connector::general_err!(
                    "response from RDCleanPath is missing X.224 for an ordinary request"
                ));
            }
            (_, ironrdp_rdcleanpath::RDCleanPathMessage::GeneralErr(error)) => {
                return Err(ironrdp_connector::custom_err!("received RDCleanPath error", error));
            }
            (
                _,
                ironrdp_rdcleanpath::RDCleanPathMessage::NegotiationErr {
                    x224_connection_response,
                },
            ) => {
                if let Ok(x224_confirm) = ironrdp_core::decode::<
                    ironrdp_pdu::x224::X224<ironrdp_pdu::nego::ConnectionConfirm>,
                >(&x224_connection_response)
                {
                    if let ironrdp_pdu::nego::ConnectionConfirm::Failure { code } = x224_confirm.0 {
                        let negotiation_failure = ironrdp_connector::NegotiationFailure::from(code);
                        return Err(ironrdp_connector::ConnectorError::new(
                            "RDP negotiation failed",
                            ironrdp_connector::ConnectorErrorKind::Negotiation(negotiation_failure),
                        ));
                    }
                }
                return Err(ironrdp_connector::general_err!(
                    "received RDCleanPath negotiation error"
                ));
            }
        };

        let server_cert = server_cert_chain
            .into_iter()
            .next()
            .ok_or_else(|| ironrdp_connector::general_err!("server cert chain missing from rdcleanpath response"))?;

        let cert = x509_cert::Certificate::from_der(server_cert.as_bytes())
            .map_err(|e| ironrdp_connector::custom_err!("server cert decode", e))?;

        let server_public_key = cert
            .tbs_certificate()
            .subject_public_key_info()
            .subject_public_key
            .as_bytes()
            .ok_or_else(|| ironrdp_connector::general_err!("subject public key BIT STRING is not aligned"))?
            .to_owned();

        let upgraded = match x224_connection_response {
            Some(x224_connection_response) => {
                let ironrdp_connector::ClientConnectorState::ConnectionInitiationWaitConfirm { .. } = connector.state
                else {
                    return Err(ironrdp_connector::general_err!(
                        "invalid connector state (wait confirm)"
                    ));
                };
                debug_assert!(connector.next_pdu_hint().is_some());

                buf.clear();
                let written = connector.step(x224_connection_response.as_bytes(), None, &mut buf)?;
                debug_assert!(written.is_nothing());

                let should_upgrade = ironrdp_tokio::skip_connect_begin(connector);
                ironrdp_tokio::mark_as_upgraded(should_upgrade, connector)
            }
            None => {
                #[cfg(feature = "vmconnect")]
                {
                    ironrdp_vmconnect::connect_front(
                        ironrdp_vmconnect::pcb_sent_via_proxy(),
                        framed,
                        connector,
                        network_client,
                        server_name,
                        &server_public_key,
                        kerberos_config,
                    )
                    .await?
                }
                #[cfg(not(feature = "vmconnect"))]
                {
                    let _ = (framed, network_client, server_name, kerberos_config);
                    return Err(ironrdp_connector::general_err!(
                        "vmconnect response from RDCleanPath requires the vmconnect feature"
                    ));
                }
            }
        };

        Ok((upgraded, server_public_key))
    }
}

fn build_ordinary_rdcleanpath_request(
    connector: &mut ironrdp_connector::ClientConnector,
    buf: &mut WriteBuf,
    destination: String,
    proxy_auth_token: String,
) -> ConnectorResult<ironrdp_rdcleanpath::RDCleanPathPdu> {
    use ironrdp_connector::Sequence as _;

    let ironrdp_connector::ClientConnectorState::ConnectionInitiationSendRequest = connector.state else {
        return Err(ironrdp_connector::general_err!(
            "invalid connector state (send request)"
        ));
    };
    debug_assert!(connector.next_pdu_hint().is_none());
    let written = connector.step_no_input(buf)?;
    let x224_pdu_len = written.size().expect("written size");
    debug_assert_eq!(x224_pdu_len, buf.filled_len());
    let x224_pdu = buf.filled().to_vec();
    ironrdp_rdcleanpath::RDCleanPathPdu::new_request(x224_pdu, destination, proxy_auth_token, None)
        .map_err(|e| ironrdp_connector::custom_err!("new RDCleanPath request", e))
}

// ── Active session ────────────────────────────────────────────────────────────

enum RdpControlFlow {
    ReconnectWithNewSize {
        width: u16,
        height: u16,
        reason: DisplayResizeFallbackReason,
    },
    TransportFailure(ironrdp_session::SessionError),
    TerminatedGracefully(GracefulDisconnectReason),
}

struct ActiveSessionIteration {
    outputs: Vec<ActiveStageOutput>,
    dvc_batch: Option<DvcMessageBatch>,
}

impl ActiveSessionIteration {
    fn outputs(outputs: Vec<ActiveStageOutput>) -> Self {
        Self {
            outputs,
            dvc_batch: None,
        }
    }

    fn dvc(dvc_batch: DvcMessageBatch) -> Self {
        Self {
            outputs: Vec::new(),
            dvc_batch: Some(dvc_batch),
        }
    }

    fn with_outputs(dvc_batch: DvcMessageBatch, outputs: Vec<ActiveStageOutput>) -> Self {
        Self {
            outputs,
            dvc_batch: Some(dvc_batch),
        }
    }
}

#[cfg(feature = "rdpdr")]
fn poll_deferred_rdpdr_output(active_stage: &mut ActiveStage) -> SessionResult<Option<ActiveStageOutput>> {
    let messages = match active_stage.get_svc_processor_mut::<ironrdp_rdpdr::Rdpdr>() {
        Some(rdpdr) => rdpdr
            .poll_deferred_messages()
            .map_err(|error| ironrdp_session::custom_err!("poll deferred RDPDR messages", error))?,
        None => Vec::new(),
    };
    if messages.is_empty() {
        Ok(None)
    } else {
        Ok(Some(ActiveStageOutput::ResponseFrame(
            active_stage.process_svc_messages_by_name(&ironrdp_rdpdr::Rdpdr::NAME, messages)?,
        )))
    }
}

#[cfg(feature = "rdpdr")]
fn process_rdpdr_drive_change(
    active_stage: &mut ActiveStage,
    device_id: u32,
    name: Option<String>,
) -> SessionResult<Vec<ActiveStageOutput>> {
    let Some(rdpdr) = active_stage.get_svc_processor_mut::<ironrdp_rdpdr::Rdpdr>() else {
        warn!(device_id, "Ignoring dynamic drive change because RDPDR is disabled");
        return Ok(Vec::new());
    };
    let messages = match name {
        Some(name) => rdpdr.add_dynamic_drive(device_id, name),
        None => rdpdr.remove_drive(device_id),
    };
    let messages = match messages {
        Ok(messages) => messages,
        Err(error) => {
            warn!(device_id, %error, "Unable to apply dynamic RDPDR drive change");
            return Ok(Vec::new());
        }
    };
    if messages.is_empty() {
        return Ok(Vec::new());
    }

    let frame = active_stage.process_svc_messages_by_name(&ironrdp_rdpdr::Rdpdr::NAME, messages)?;
    if frame.is_empty() {
        Ok(Vec::new())
    } else {
        Ok(vec![ActiveStageOutput::ResponseFrame(frame)])
    }
}

fn pack_desktop_update(
    image: &DecodedImage,
    width: NonZeroU16,
    height: NonZeroU16,
    region: InclusiveRectangle,
) -> SessionResult<DesktopUpdate> {
    let region_width = region
        .right
        .checked_sub(region.left)
        .and_then(|width| width.checked_add(1))
        .ok_or_else(|| ironrdp_session::general_err!("invalid desktop update horizontal bounds"))?;
    let region_height = region
        .bottom
        .checked_sub(region.top)
        .and_then(|height| height.checked_add(1))
        .ok_or_else(|| ironrdp_session::general_err!("invalid desktop update vertical bounds"))?;
    if region.right >= width.get() || region.bottom >= height.get() {
        return Err(ironrdp_session::general_err!(
            "desktop update exceeds framebuffer bounds"
        ));
    }

    let pixel_count = usize::from(region_width)
        .checked_mul(usize::from(region_height))
        .ok_or_else(|| ironrdp_session::general_err!("desktop update pixel count overflow"))?;
    let mut buffer = Vec::new();
    buffer
        .try_reserve_exact(pixel_count)
        .map_err(|_| ironrdp_session::general_err!("allocate desktop update buffer"))?;

    let source_width = NonZeroUsize::from(width).get();
    let left = usize::from(region.left);
    let row_pixel_count = usize::from(region_width);
    for y in region.top..=region.bottom {
        let pixel_offset = usize::from(y)
            .checked_mul(source_width)
            .and_then(|offset| offset.checked_add(left))
            .ok_or_else(|| ironrdp_session::general_err!("desktop update source offset overflow"))?;
        let byte_offset = pixel_offset
            .checked_mul(4)
            .ok_or_else(|| ironrdp_session::general_err!("desktop update byte offset overflow"))?;
        let byte_len = row_pixel_count
            .checked_mul(4)
            .ok_or_else(|| ironrdp_session::general_err!("desktop update row length overflow"))?;
        let byte_end = byte_offset
            .checked_add(byte_len)
            .ok_or_else(|| ironrdp_session::general_err!("desktop update row end overflow"))?;
        let row = image
            .data()
            .get(byte_offset..byte_end)
            .ok_or_else(|| ironrdp_session::general_err!("desktop update source row is out of bounds"))?;
        buffer.extend(row.chunks_exact(4).map(|pixel| {
            let r = pixel[0];
            let g = pixel[1];
            let b = pixel[2];
            u32::from_be_bytes([0, r, g, b])
        }));
    }

    DesktopUpdate::new(buffer, width, height, region)
        .ok_or_else(|| ironrdp_session::general_err!("packed desktop update is inconsistent"))
}

#[expect(
    clippy::too_many_arguments,
    reason = "the active loop owns independent transport, input, clipboard, and cancellation sources"
)]
async fn active_session(
    framed: UpgradedFramed,
    connection_result: ConnectionResult,
    #[cfg(feature = "udp")] mut udp_tunnel: UdpTunnel,
    #[cfg(not(feature = "udp"))] _udp_tunnel: UdpTunnel,
    initial_rail_execute: Option<ExecutePdu>,
    output_event_sender: &crate::output_channel::OutputEventSender,
    desktop_update_handler: Option<&(dyn Fn(DesktopUpdate) + Send + Sync)>,
    input_event_receiver: &mut mpsc::Receiver<RdpInputEvent>,
    clipboard_event_receiver: &mut mpsc::UnboundedReceiver<RdpInputEvent>,
    close_receiver: &mut watch::Receiver<bool>,
    graceful_close_receiver: &mut watch::Receiver<bool>,
    input_send_interval: Option<Duration>,
    fake_events_interval: Option<Duration>,
    auto_reconnect_cookie: &mut Option<ServerAutoReconnect>,
    reconnect_attempt: &mut u32,
) -> SessionResult<RdpControlFlow> {
    let (mut reader, mut writer) = split_tokio_framed(framed);
    let desktop_size = connection_result.desktop_size;
    let multitransport_soft_sync = connection_result.multitransport_soft_sync();
    let mut refresh_rect_support = connection_result.refresh_rect_support;
    let mut suppress_output_support = connection_result.suppress_output_support;
    let window_support_level = connection_result.window_support_level;
    let mut image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
    let mut desktop_update_extent = None;

    // We retain the factory to drive the Deactivation-Reactivation Sequence locally.
    let activation_factory = connection_result.activation_factory;

    let mut active_stage = ActiveStageBuilder {
        static_channels: connection_result.static_channels,
        user_channel_id: connection_result.user_channel_id,
        io_channel_id: connection_result.io_channel_id,
        message_channel_id: connection_result.message_channel_id,
        share_id: connection_result.share_id,
        compression_type: connection_result.compression_type,
        enable_server_pointer: connection_result.enable_server_pointer,
        pointer_software_rendering: connection_result.pointer_software_rendering,
    }
    .build();
    #[cfg(feature = "udp")]
    if udp_tunnel.transport.is_some() {
        active_stage.enable_reliable_udp_dvc_tunnel()?;
        trace!("Reliable UDP tunnel established; awaiting DVC Soft-Sync");
    }
    active_stage.set_window_support_level(window_support_level);
    if let Some(execute) = initial_rail_execute {
        let rail_client = active_stage
            .get_svc_processor_mut::<RailClient>()
            .ok_or_else(|| ironrdp_session::general_err!("RemoteApp launch requested without a RAIL static channel"))?;
        rail_client
            .queue_execute(execute)
            .map_err(|error| ironrdp_session::custom_err!("queue initial RemoteApp launch", error))?;
    }

    // Timer interval for driving clipboard lock timeouts.
    let mut cleanup_interval = tokio::time::interval(Duration::from_secs(5));
    #[cfg(feature = "rdpdr")]
    let mut rdpdr_deferred_interval = tokio::time::interval(Duration::from_millis(50));

    // Anti-idle: track the time of the last real input and the last known mouse position so we can
    // synthesize a no-op mouse move when the session has been idle for too long. Default to the
    // middle of the screen so a synthetic move before any real input doesn't snap the pointer to a
    // corner.
    let now = tokio::time::Instant::now();
    let mut last_input = now;
    let mut last_mouse_pos = (desktop_size.width / 2, desktop_size.height / 2);
    let mut input_batcher = FastPathInputBatcher::new(input_send_interval, now);
    let mut fake_events_interval =
        fake_events_interval.map(|interval| tokio::time::interval(core::cmp::max(interval, Duration::from_secs(1))));
    let mut resize_queue = ResizeQueue::default();
    let mut rail_queue_release_deadline = None;
    let mut graceful_shutdown_sent = false;
    let mut post_logon_redraw_requested = false;
    let mut pending_udp_payload: Option<Vec<u8>> = None;
    let mut initial_outputs = if *graceful_close_receiver.borrow_and_update() {
        graceful_shutdown_sent = true;
        Some(active_stage.graceful_shutdown()?)
    } else {
        None
    };

    if *close_receiver.borrow_and_update() {
        return Ok(RdpControlFlow::TerminatedGracefully(
            GracefulDisconnectReason::UserInitiated,
        ));
    }
    #[cfg(not(feature = "clipboard"))]
    let _ = clipboard_event_receiver;

    let disconnect_reason = 'outer: loop {
        let resize_deadline = resize_queue.deadline();
        let input_batch_deadline = input_batcher.deadline();
        let mut malformed_bitmap_redraw_queued = false;
        let clipboard_event = async {
            #[cfg(feature = "clipboard")]
            {
                clipboard_event_receiver.recv().await
            }
            #[cfg(not(feature = "clipboard"))]
            {
                core::future::pending::<Option<RdpInputEvent>>().await
            }
        };
        let rdpdr_deferred = async {
            #[cfg(feature = "rdpdr")]
            rdpdr_deferred_interval.tick().await;
            #[cfg(not(feature = "rdpdr"))]
            core::future::pending::<()>().await;
        };
        let buffered_udp_iteration = if initial_outputs.is_none() && active_stage.reliable_udp_dvc_tunnel_in_use() {
            match pending_udp_payload.take() {
                Some(payload) => Some(ActiveSessionIteration::dvc(
                    active_stage.process_dvc_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &payload)?,
                )),
                None => None,
            }
        } else {
            None
        };
        let mut iteration = if let Some(outputs) = initial_outputs.take() {
            ActiveSessionIteration::outputs(outputs)
        } else if let Some(iteration) = buffered_udp_iteration {
            iteration
        } else {
            #[cfg(feature = "udp")]
            if active_stage.reliable_udp_dvc_tunnel_in_use() && udp_tunnel.transport.is_none() {
                return Ok(RdpControlFlow::TransportFailure(ironrdp_session::general_err!(
                    "reliable UDP tunnel closed"
                )));
            }
            tokio::select! {
                _ = close_receiver.changed() => {
                    break 'outer GracefulDisconnectReason::UserInitiated;
                }
                _ = graceful_close_receiver.changed() => {
                    let outputs = if *graceful_close_receiver.borrow_and_update() && !graceful_shutdown_sent {
                        graceful_shutdown_sent = true;
                        active_stage.graceful_shutdown()?
                    } else {
                        Vec::new()
                    };
                    ActiveSessionIteration::outputs(outputs)
                }
                frame = reader.read_pdu() => {
                    let (action, payload) = match frame {
                        Ok(frame) => frame,
                        Err(error) if is_transport_read_error(&error) => {
                            return Ok(RdpControlFlow::TransportFailure(
                                ironrdp_session::custom_err!("read frame", error),
                            ));
                        }
                        Err(error) => return Err(ironrdp_session::custom_err!("read frame", error)),
                    };
                    trace!(?action, frame_length = payload.len(), "Frame received");
                    let mut outputs = active_stage.process(&mut image, action, &payload)?;
                    #[cfg(feature = "rdpdr")]
                    if let Some(output) = poll_deferred_rdpdr_output(&mut active_stage)? {
                        outputs.push(output);
                    }
                    if outputs.iter().any(|output| matches!(output, ActiveStageOutput::AutoReconnectFailed)) {
                        *auto_reconnect_cookie = None;
                        // The server rejected the cookie but may complete this
                        // connection with the configured credentials instead.
                        if *reconnect_attempt != 0 {
                            if !send_active_output_event(
                                output_event_sender,
                                RdpOutputEvent::Connected,
                                close_receiver,
                            )
                            .await?
                            {
                                return Ok(RdpControlFlow::TerminatedGracefully(
                                    GracefulDisconnectReason::UserInitiated,
                                ));
                            }
                            *reconnect_attempt = 0;
                        }
                    }
                    if *reconnect_attempt != 0
                        && outputs.iter().any(|output| {
                            matches!(
                                output,
                                ActiveStageOutput::SaveSessionInfo { .. }
                                    | ActiveStageOutput::GraphicsUpdate(_)
                                    | ActiveStageOutput::PointerDefault
                                    | ActiveStageOutput::PointerHidden
                                    | ActiveStageOutput::PointerPosition { .. }
                                    | ActiveStageOutput::PointerBitmap(_)
                            )
                        })
                    {
                        if !send_active_output_event(
                            output_event_sender,
                            RdpOutputEvent::AutoReconnected,
                            close_receiver,
                        )
                        .await?
                        {
                            return Ok(RdpControlFlow::TerminatedGracefully(
                                GracefulDisconnectReason::UserInitiated,
                            ));
                        }
                        *reconnect_attempt = 0;
                    }
                    if active_stage.take_bitmap_recovery_request() {
                        let redraw_frames = active_stage.request_full_redraw(
                            image.width(),
                            image.height(),
                            refresh_rect_support,
                            suppress_output_support,
                        )?;
                        malformed_bitmap_redraw_queued = !redraw_frames.is_empty();
                        outputs.extend(redraw_frames.into_iter().map(ActiveStageOutput::ResponseFrame));
                    }
                    ActiveSessionIteration::outputs(outputs)
                }
                udp_payload = async {
                    #[cfg(feature = "udp")]
                    {
                        match (udp_tunnel.transport.as_mut(), pending_udp_payload.is_none()) {
                            (Some(transport), true) => transport.recv().await,
                            (Some(_), false) | (None, _) => {
                                core::future::pending::<Option<Vec<u8>>>().await
                            }
                        }
                    }
                    #[cfg(not(feature = "udp"))]
                    {
                        core::future::pending::<Option<Vec<u8>>>().await
                    }
                } => {
                    match udp_payload {
                    None => {
                        if active_stage.reliable_udp_dvc_tunnel_in_use() {
                            return Ok(RdpControlFlow::TransportFailure(
                                ironrdp_session::general_err!("reliable UDP tunnel closed"),
                            ));
                        }
                        #[cfg(feature = "udp")]
                        {
                            udp_tunnel.transport = None;
                        }
                        active_stage.disable_reliable_udp_dvc_tunnel()?;
                        warn!("Reliable UDP tunnel closed before Soft-Sync; continuing with TCP");
                        ActiveSessionIteration::outputs(Vec::new())
                    }
                    Some(payload) if payload.is_empty() => {
                        trace!("Ignoring reliable UDP tunnel PDU without higher-layer data");
                        ActiveSessionIteration::outputs(Vec::new())
                    }
                    Some(payload) => {
                        if active_stage.reliable_udp_dvc_tunnel_in_use() {
                            let batch =
                                active_stage.process_dvc_tunnel(SoftSyncTunnelType::RELIABLE_UDP, &payload)?;
                            ActiveSessionIteration::dvc(batch)
                        } else {
                            // The server can send on UDP immediately after its Soft-Sync request,
                            // before the independently ordered request arrives over TCP. Stop
                            // polling here so the transport retains subsequent ordered payloads.
                            pending_udp_payload = Some(payload);
                            ActiveSessionIteration::outputs(Vec::new())
                        }
                    }
                    }
                }
                clipboard_event = clipboard_event => {
                    #[cfg(feature = "clipboard")]
                    {
                        let Some(RdpInputEvent::Clipboard(event)) = clipboard_event else {
                            return Err(ironrdp_session::general_err!("clipboard event channel closed"));
                        };
                        ActiveSessionIteration::outputs(process_clipboard_message(&mut active_stage, event)?)
                    }
                    #[cfg(not(feature = "clipboard"))]
                    {
                        let _ = clipboard_event;
                        unreachable!("clipboard receive is pending without the clipboard feature")
                    }
                }
                input_event = input_event_receiver.recv() => {
                    let input_event = input_event.ok_or_else(|| ironrdp_session::general_err!("GUI is stopped"))?;

                    last_input = tokio::time::Instant::now();

                    match input_event {
                    RdpInputEvent::Resize { width, height, scale_factor, physical_size } => {
                        trace!(width, height, "Resize event");
                        let width = u32::from(width);
                        let height = u32::from(height);
                        // TODO: Make adjust_display_size take and return width and height as u16.
                        // From the function's doc comment, the width and height values must be less than or equal to 8192 pixels.
                        // Therefore, we can remove unnecessary casts from u16 to u32 and back.
                        let (width, height) = MonitorLayoutEntry::adjust_display_size(width, height);
                        debug!(width, height, "Adjusted display size");
                        let request = ResizeRequest {
                            width: u16::try_from(width).expect("always in the range"),
                            height: u16::try_from(height).expect("always in the range"),
                            scale_factor,
                            physical_size,
                        };
                        if resize_queue.in_flight.is_some() || active_stage.display_control_ready() == Some(false) {
                            resize_queue.defer(request);
                            ActiveSessionIteration::outputs(Vec::new())
                        } else if let Some(dvc_batch) = active_stage.prepare_resize(
                            u32::from(request.width),
                            u32::from(request.height),
                            Some(request.scale_factor),
                            request.physical_size,
                        ) {
                            resize_queue.mark_in_flight(request);
                            let mut outputs = Vec::new();
                            if let Some(messages) = active_stage
                                .get_svc_processor_mut::<RailClient>()
                                .map(|rail_client| rail_client.update_desktop_size(request.width, request.height))
                            {
                                let frame = active_stage.process_svc_processor_messages(messages)?;
                                if !frame.is_empty() {
                                    outputs.push(ActiveStageOutput::ResponseFrame(frame));
                                }
                            }
                            ActiveSessionIteration::with_outputs(dvc_batch?, outputs)
                        } else {
                            // TODO(#271): use the "auto-reconnect cookie": https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/15b0d1c9-2891-4adb-a45e-deb4aeeeab7c
                            debug!("Reconnecting with new size");
                            return Ok(RdpControlFlow::ReconnectWithNewSize {
                                width: request.width,
                                height: request.height,
                                reason: DisplayResizeFallbackReason::DisplayControlUnavailable,
                            })
                        }
                    }
                    RdpInputEvent::FastPath(events) => {
                        trace!(?events);
                        for event in &events {
                            if let FastPathInputEvent::MouseEvent(mouse) = event {
                                last_mouse_pos = (mouse.x_position, mouse.y_position);
                            }
                        }
                        ActiveSessionIteration::outputs(match input_batcher.queue(events, tokio::time::Instant::now()) {
                            Some(events) => active_stage.process_fastpath_input(&mut image, &events)?,
                            None => Vec::new(),
                        })
                    }
                    RdpInputEvent::Touch(event) => {
                        trace!(frames = event.frames.len(), "RDPEI touch event");
                        match active_stage.prepare_rdpei_touch(event) {
                            Some(Ok(batch)) => ActiveSessionIteration::dvc(batch),
                            Some(Err(error)) => {
                                // Surface encode failures so producers can resync contact state.
                                warn!(%error, "Failed to encode RDPEI touch event");
                                ActiveSessionIteration::outputs(Vec::new())
                            }
                            None => {
                                // Channel missing / not ready / suspended: warn rather than
                                // silently dropping, which desynchronizes the contact FSM.
                                warn!("Dropping RDPEI touch event: channel unavailable, not ready, or suspended");
                                ActiveSessionIteration::outputs(Vec::new())
                            }
                        }
                    }
                    RdpInputEvent::Pen(event) => {
                        trace!(frames = event.frames.len(), "RDPEI pen event");
                        match active_stage.prepare_rdpei_pen(event) {
                            Some(Ok(batch)) => ActiveSessionIteration::dvc(batch),
                            Some(Err(error)) => {
                                warn!(%error, "Failed to encode RDPEI pen event");
                                ActiveSessionIteration::outputs(Vec::new())
                            }
                            None => {
                                debug!(
                                    "Dropping RDPEI pen event: channel not ready, suspended, or pen disallowed"
                                );
                                ActiveSessionIteration::outputs(Vec::new())
                            }
                        }
                    }
                    RdpInputEvent::DismissHoveringTouchContact { contact_id } => {
                        trace!(contact_id, "RDPEI dismiss hovering touch contact");
                        match active_stage.prepare_rdpei_dismiss_hovering(contact_id) {
                            Some(Ok(batch)) => ActiveSessionIteration::dvc(batch),
                            Some(Err(error)) => {
                                warn!(%error, "Failed to encode RDPEI dismiss hovering");
                                ActiveSessionIteration::outputs(Vec::new())
                            }
                            None => ActiveSessionIteration::outputs(Vec::new()),
                        }
                    }
                    RdpInputEvent::Close => ActiveSessionIteration::outputs(active_stage.graceful_shutdown()?),
                    #[cfg(feature = "clipboard")]
                    RdpInputEvent::Clipboard(event) => {
                        ActiveSessionIteration::outputs(process_clipboard_message(&mut active_stage, event)?)
                    }
                    RdpInputEvent::SendDvcMessages { channel_id, messages } => {
                        trace!(channel_id, ?messages, "Send DVC messages");
                        ActiveSessionIteration::dvc(
                            DvcMessageBatch::try_new(channel_id, messages).map_err(ironrdp_session::SessionError::pdu)?,
                        )
                    }
                    RdpInputEvent::SendStaticChannelData { channel_name, data } => {
                        let outputs = match active_stage.process_svc_messages_by_name(&channel_name, vec![SvcMessage::from(data)]) {
                            Ok(frame) => vec![ActiveStageOutput::ResponseFrame(frame)],
                            Err(error) => {
                                warn!(?channel_name, %error, "Unable to send static channel data");
                                Vec::new()
                            }
                        };
                        ActiveSessionIteration::outputs(outputs)
                    }
                    #[cfg(feature = "rdpdr")]
                    RdpInputEvent::AddRdpdrDrive { device_id, name } => {
                        ActiveSessionIteration::outputs(process_rdpdr_drive_change(&mut active_stage, device_id, Some(name))?)
                    }
                    #[cfg(feature = "rdpdr")]
                    RdpInputEvent::RemoveRdpdrDrive { device_id } => {
                        ActiveSessionIteration::outputs(process_rdpdr_drive_change(&mut active_stage, device_id, None)?)
                    }
                    #[cfg(feature = "location")]
                    RdpInputEvent::Location(request) => {
                        if request.is_cancelled_or_expired() {
                            ActiveSessionIteration::outputs(Vec::new())
                        } else {
                            let (latitude, longitude, altitude) = request.coordinates();
                            let prepared = match active_stage.get_dvc::<LocationClient>() {
                                None => Err(LocationInputError::ChannelUnavailable),
                                Some(dvc) if !dvc.processor().ready() => Err(LocationInputError::ChannelNotReady),
                                Some(dvc) => {
                                    let channel_id = dvc.channel_id();
                                    dvc.processor()
                                        .prepare_location(latitude, longitude, altitude)
                                        .map_err(|error| {
                                            warn!(%error, "Unable to encode location update");
                                            LocationInputError::EncodingFailed
                                        })
                                        .and_then(|(prepared, messages)| {
                                            DvcMessageBatch::try_new(channel_id, messages)
                                                .map(|batch| (prepared, batch))
                                                .map_err(|error| {
                                                    warn!(%error, "Unable to prepare location update");
                                                    LocationInputError::EncodingFailed
                                                })
                                        })
                                }
                            };

                            match prepared {
                                Err(error) => {
                                    request.complete(Err(error));
                                    ActiveSessionIteration::outputs(Vec::new())
                                }
                                Ok((prepared, batch)) => {
                                    if request.try_commit() {
                                        if let Some(mut dvc) = active_stage.get_dvc_mut::<LocationClient>() {
                                            dvc.processor_mut().commit_location(prepared);
                                            request.send_response(Ok(()));
                                            ActiveSessionIteration::dvc(batch)
                                        } else {
                                            request.send_response(Err(LocationInputError::ChannelUnavailable));
                                            ActiveSessionIteration::outputs(Vec::new())
                                        }
                                    } else {
                                        ActiveSessionIteration::outputs(Vec::new())
                                    }
                                }
                            }
                        }
                    }
                    RdpInputEvent::RailExecute(execute) => {
                        let executable = execute.executable.clone();
                        let flags = execute.flags;
                        let (messages, failure_reason) = match active_stage.get_svc_processor_mut::<RailClient>() {
                            Some(rail_client) => match rail_client.queue_execute(execute) {
                                Ok(messages) => (Some(messages), None),
                                Err(error) => {
                                    warn!(%error, "Unable to queue RAIL Execute request");
                                    (None, Some(RailExecuteFailureReason::QueueRejected))
                                }
                            },
                            None => {
                                warn!("Unable to queue RAIL Execute request because RAIL is disabled");
                                (None, Some(RailExecuteFailureReason::RailUnavailable))
                            }
                        };
                        let outputs = match messages {
                            Some(messages) => match active_stage.process_svc_processor_messages(messages) {
                                Ok(frame) => (!frame.is_empty())
                                    .then_some(ActiveStageOutput::ResponseFrame(frame))
                                    .into_iter()
                                    .collect(),
                                Err(error) => {
                                    warn!(%error, "Unable to process RAIL Execute request");
                                    if !send_active_output_event(
                                        output_event_sender,
                                        RdpOutputEvent::RailExecuteFailed {
                                            executable,
                                            flags,
                                            reason: RailExecuteFailureReason::MessageProcessingFailed,
                                        },
                                        close_receiver,
                                    )
                                    .await?
                                    {
                                        return Ok(RdpControlFlow::TerminatedGracefully(
                                            GracefulDisconnectReason::UserInitiated,
                                        ));
                                    }
                                    Vec::new()
                                }
                            },
                            None => {
                                let reason = failure_reason.expect("RAIL Execute failure reason must be recorded");
                                if !send_active_output_event(
                                    output_event_sender,
                                    RdpOutputEvent::RailExecuteFailed {
                                        executable,
                                        flags,
                                        reason,
                                    },
                                    close_receiver,
                                )
                                .await?
                                {
                                    return Ok(RdpControlFlow::TerminatedGracefully(
                                        GracefulDisconnectReason::UserInitiated,
                                    ));
                                }
                                Vec::new()
                            }
                        };
                        ActiveSessionIteration::outputs(outputs)
                    }
                    RdpInputEvent::Rail(event) => {
                        let messages = match active_stage.get_svc_processor_mut::<RailClient>() {
                            Some(rail_client) => match rail_client.queue_input(event) {
                                Ok(messages) => Some(messages),
                                Err(error) => {
                                    warn!(%error, "Unable to queue RAIL input");
                                    None
                                }
                            },
                            None => {
                                warn!("Unable to queue RAIL input because RAIL is disabled");
                                None
                            }
                        };
                        let outputs = match messages {
                            Some(messages) => match active_stage.process_svc_processor_messages(messages) {
                                Ok(frame) => (!frame.is_empty())
                                    .then_some(ActiveStageOutput::ResponseFrame(frame))
                                    .into_iter()
                                    .collect(),
                                Err(error) => {
                                    warn!(%error, "Unable to process RAIL input");
                                    Vec::new()
                                }
                            },
                            None => Vec::new(),
                        };
                        ActiveSessionIteration::outputs(outputs)
                    }
                    }
                }
                _ = cleanup_interval.tick() => {
                // Drive clipboard lock timeout cleanup.
                #[cfg(feature = "clipboard")]
                let outputs = if let Some(cliprdr_client) = active_stage.get_svc_processor_mut::<ironrdp_cliprdr::CliprdrClient>() {
                    match cliprdr_client.drive_timeouts() {
                        Ok(svc_messages) => {
                            let frame = active_stage.process_svc_processor_messages(svc_messages)?;
                            if !frame.is_empty() {
                                vec![ActiveStageOutput::ResponseFrame(frame)]
                            } else {
                                Vec::new()
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "Clipboard timeout cleanup failed");
                            Vec::new()
                        }
                    }
                } else {
                    Vec::new()
                };
                #[cfg(not(feature = "clipboard"))]
                let outputs = Vec::new();
                ActiveSessionIteration::outputs(outputs)
                }
                _ = rdpdr_deferred => {
                    #[cfg(feature = "rdpdr")]
                    let outputs = {
                        poll_deferred_rdpdr_output(&mut active_stage)?.into_iter().collect()
                    };
                    #[cfg(not(feature = "rdpdr"))]
                    let outputs = Vec::new();
                    ActiveSessionIteration::outputs(outputs)
                }
                _ = async {
                match resize_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => core::future::pending().await,
                }
                } => {
                let (request, reason) = resize_queue
                    .timed_out_request(tokio::time::Instant::now())
                    .expect("resize deadline must correspond to a queued request");
                return Ok(RdpControlFlow::ReconnectWithNewSize {
                    width: request.width,
                    height: request.height,
                    reason,
                });
                }
                _ = async {
                    match rail_queue_release_deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => core::future::pending().await,
                    }
                } => {
                    rail_queue_release_deadline = None;
                    let messages = active_stage
                        .get_svc_processor_mut::<RailClient>()
                        .map(RailClient::release_queued_after_handshake)
                        .transpose()
                        .map_err(|error| ironrdp_session::custom_err!("RAIL", error))?;
                    let outputs = match messages {
                        Some(messages) => {
                            let frame = active_stage.process_svc_processor_messages(messages)?;
                            (!frame.is_empty())
                                .then_some(ActiveStageOutput::ResponseFrame(frame))
                                .into_iter()
                                .collect()
                        }
                        None => Vec::new(),
                    };
                    ActiveSessionIteration::outputs(outputs)
                }
                _ = async {
                    match input_batch_deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => core::future::pending().await,
                    }
                } => {
                    ActiveSessionIteration::outputs(match input_batcher.flush(tokio::time::Instant::now()) {
                        Some(events) => active_stage.process_fastpath_input(&mut image, &events)?,
                        None => Vec::new(),
                    })
                }
                _ = async { match fake_events_interval.as_mut() {
                Some(interval) => interval.tick().await,
                None => core::future::pending().await,
                }} => {
                // Anti-idle: synthesize a no-op mouse move if the session has been idle for at least
                // the configured interval, keeping the connection alive without user interaction.
                let outputs = if last_input.elapsed() >= fake_events_interval.as_ref().map_or(Duration::MAX, |i| i.period()) {
                    last_input = tokio::time::Instant::now();
                    let mut events = SmallVec::<[FastPathInputEvent; 2]>::new();
                    events.push(FastPathInputEvent::MouseEvent(MousePdu {
                        flags: PointerFlags::MOVE,
                        number_of_wheel_rotation_units: 0,
                        x_position: last_mouse_pos.0,
                        y_position: last_mouse_pos.1,
                    }));
                    let events = input_batcher.queue_forced(events, tokio::time::Instant::now());
                    active_stage.process_fastpath_input(&mut image, &events)?
                } else {
                    Vec::new()
                };
                ActiveSessionIteration::outputs(outputs)
                }
            }
        };

        if let Some(batch) = iteration.dvc_batch {
            let channel_id = batch.channel_id();
            let messages = batch.into_messages();
            #[cfg(feature = "udp")]
            let route_over_udp =
                active_stage.dvc_tunnel_for_channel(channel_id) == Some(SoftSyncTunnelType::RELIABLE_UDP);
            #[cfg(not(feature = "udp"))]
            let route_over_udp = {
                let _ = channel_id;
                false
            };
            if route_over_udp {
                #[cfg(feature = "udp")]
                {
                    let Some(transport) = udp_tunnel.transport.as_ref() else {
                        return Ok(RdpControlFlow::TransportFailure(ironrdp_session::general_err!(
                            "reliable UDP tunnel is unavailable for a Soft-Sync channel"
                        )));
                    };
                    for message in messages {
                        let payload = message
                            .encode_unframed_pdu()
                            .map_err(|error| ironrdp_session::custom_err!("encode tunneled DVC message", error))?;
                        let Some(result) = cancelable_operation(transport.send(payload), close_receiver).await else {
                            return Ok(RdpControlFlow::TerminatedGracefully(
                                GracefulDisconnectReason::UserInitiated,
                            ));
                        };
                        if let Err(error) = result {
                            return Ok(RdpControlFlow::TransportFailure(ironrdp_session::custom_err!(
                                "write reliable UDP tunnel data",
                                error
                            )));
                        }
                    }
                }
            } else {
                let frame = active_stage.encode_dvc_messages(messages)?;
                // Preserve DVC-before-associated-output ordering, notably Display Control
                // resize before the corresponding RAIL desktop-size update.
                iteration.outputs.insert(0, ActiveStageOutput::ResponseFrame(frame));
            }
        }

        for out in iteration.outputs {
            match out {
                ActiveStageOutput::AutoReconnectCookie(cookie) => {
                    *auto_reconnect_cookie = Some(cookie);
                    debug!("Received a Server Auto-Reconnect Cookie");
                }
                // The status pre-scan above discarded the reconnect state before
                // success can be reported; this output has no host-facing event.
                ActiveStageOutput::AutoReconnectFailed => {}
                ActiveStageOutput::ResponseFrame(frame) => {
                    let Some(result) = cancelable_operation(writer.write_all(&frame), close_receiver).await else {
                        return Ok(RdpControlFlow::TerminatedGracefully(
                            GracefulDisconnectReason::UserInitiated,
                        ));
                    };
                    if let Err(error) = result {
                        return Ok(RdpControlFlow::TransportFailure(ironrdp_session::custom_err!(
                            "write response",
                            error
                        )));
                    }
                }
                ActiveStageOutput::GraphicsUpdate(region) => {
                    let width =
                        NonZeroU16::new(image.width()).ok_or_else(|| ironrdp_session::general_err!("width is zero"))?;
                    let height = NonZeroU16::new(image.height())
                        .ok_or_else(|| ironrdp_session::general_err!("height is zero"))?;
                    if let Some(handler) = desktop_update_handler {
                        let extent = (width, height);
                        let region = if desktop_update_extent == Some(extent) {
                            region
                        } else {
                            desktop_update_extent = Some(extent);
                            InclusiveRectangle {
                                left: 0,
                                top: 0,
                                right: width.get() - 1,
                                bottom: height.get() - 1,
                            }
                        };
                        handler(pack_desktop_update(&image, width, height, region)?);
                        continue;
                    }

                    let buffer = pack_desktop_update(
                        &image,
                        width,
                        height,
                        InclusiveRectangle {
                            left: 0,
                            top: 0,
                            right: width.get() - 1,
                            bottom: height.get() - 1,
                        },
                    )?
                    .buffer;
                    if !send_active_output_event(
                        output_event_sender,
                        RdpOutputEvent::Image { buffer, width, height },
                        close_receiver,
                    )
                    .await?
                    {
                        return Ok(RdpControlFlow::TerminatedGracefully(
                            GracefulDisconnectReason::UserInitiated,
                        ));
                    }
                }
                ActiveStageOutput::PointerDefault => {
                    if !send_active_output_event(output_event_sender, RdpOutputEvent::PointerDefault, close_receiver)
                        .await?
                    {
                        return Ok(RdpControlFlow::TerminatedGracefully(
                            GracefulDisconnectReason::UserInitiated,
                        ));
                    }
                }
                ActiveStageOutput::PointerHidden => {
                    if !send_active_output_event(output_event_sender, RdpOutputEvent::PointerHidden, close_receiver)
                        .await?
                    {
                        return Ok(RdpControlFlow::TerminatedGracefully(
                            GracefulDisconnectReason::UserInitiated,
                        ));
                    }
                }
                ActiveStageOutput::PointerPosition { x, y } => {
                    if !send_active_output_event(
                        output_event_sender,
                        RdpOutputEvent::PointerPosition { x, y },
                        close_receiver,
                    )
                    .await?
                    {
                        return Ok(RdpControlFlow::TerminatedGracefully(
                            GracefulDisconnectReason::UserInitiated,
                        ));
                    }
                }
                ActiveStageOutput::PointerBitmap(pointer) => {
                    if !send_active_output_event(
                        output_event_sender,
                        RdpOutputEvent::PointerBitmap(pointer),
                        close_receiver,
                    )
                    .await?
                    {
                        return Ok(RdpControlFlow::TerminatedGracefully(
                            GracefulDisconnectReason::UserInitiated,
                        ));
                    }
                }
                ActiveStageOutput::MonitorLayout(monitors) => {
                    if !send_active_output_event(
                        output_event_sender,
                        RdpOutputEvent::MonitorLayout(monitors),
                        close_receiver,
                    )
                    .await?
                    {
                        return Ok(RdpControlFlow::TerminatedGracefully(
                            GracefulDisconnectReason::UserInitiated,
                        ));
                    }
                }
                ActiveStageOutput::WindowingOrders(orders) => {
                    if !send_active_output_event(
                        output_event_sender,
                        RdpOutputEvent::WindowingOrders(orders),
                        close_receiver,
                    )
                    .await?
                    {
                        return Ok(RdpControlFlow::TerminatedGracefully(
                            GracefulDisconnectReason::UserInitiated,
                        ));
                    }
                }
                ActiveStageOutput::SaveSessionInfo { logon_complete: true } => {
                    if !post_logon_redraw_requested {
                        post_logon_redraw_requested = true;
                        let redraw_frames = active_stage.request_full_redraw(
                            image.width(),
                            image.height(),
                            refresh_rect_support,
                            suppress_output_support,
                        )?;
                        let redraw_requested = !redraw_frames.is_empty();

                        for frame in redraw_frames {
                            let Some(result) = cancelable_operation(writer.write_all(&frame), close_receiver).await
                            else {
                                return Ok(RdpControlFlow::TerminatedGracefully(
                                    GracefulDisconnectReason::UserInitiated,
                                ));
                            };
                            if let Err(error) = result {
                                return Ok(RdpControlFlow::TransportFailure(ironrdp_session::custom_err!(
                                    "write post-logon redraw request",
                                    error
                                )));
                            }
                        }

                        if redraw_requested
                            && !send_active_output_event(
                                output_event_sender,
                                RdpOutputEvent::PostLogonDisplayRedraw,
                                close_receiver,
                            )
                            .await?
                        {
                            return Ok(RdpControlFlow::TerminatedGracefully(
                                GracefulDisconnectReason::UserInitiated,
                            ));
                        }
                    }
                    if !send_active_output_event(output_event_sender, RdpOutputEvent::LoginComplete, close_receiver)
                        .await?
                    {
                        return Ok(RdpControlFlow::TerminatedGracefully(
                            GracefulDisconnectReason::UserInitiated,
                        ));
                    }
                }
                ActiveStageOutput::SaveSessionInfo { logon_complete: false } => {}
                ActiveStageOutput::DeactivateAll => {
                    // Deactivation-Reactivation Sequence:
                    // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
                    debug!("Executing Deactivation-Reactivation Sequence");
                    let mut connection_activation = activation_factory.create();
                    let mut buf = WriteBuf::new();
                    'activation_seq: loop {
                        let step = single_sequence_step_read(&mut reader, &mut connection_activation, &mut buf);
                        let written = if let Some(in_flight) = resize_queue.in_flight.as_ref() {
                            match cancelable_operation(
                                tokio::time::timeout_at(in_flight.deadline, step),
                                close_receiver,
                            )
                            .await
                            {
                                Some(Ok(result)) => result,
                                Some(Err(_)) => {
                                    let request = resize_queue
                                        .pending
                                        .as_ref()
                                        .map_or(in_flight.request, |pending| pending.request);
                                    return Ok(RdpControlFlow::ReconnectWithNewSize {
                                        width: request.width,
                                        height: request.height,
                                        reason: DisplayResizeFallbackReason::ReactivationTimedOut,
                                    });
                                }
                                None => {
                                    return Ok(RdpControlFlow::TerminatedGracefully(
                                        GracefulDisconnectReason::UserInitiated,
                                    ));
                                }
                            }
                        } else {
                            let Some(result) = cancelable_operation(step, close_receiver).await else {
                                return Ok(RdpControlFlow::TerminatedGracefully(
                                    GracefulDisconnectReason::UserInitiated,
                                ));
                            };
                            result
                        };
                        let written = match written {
                            Ok(written) => written,
                            Err(error) if is_transport_session_error(&error) => {
                                return Ok(RdpControlFlow::TransportFailure(ironrdp_session::custom_err!(
                                    "read deactivation-reactivation sequence step",
                                    error
                                )));
                            }
                            Err(error) => {
                                return Err(ironrdp_session::custom_err!(
                                    "read deactivation-reactivation sequence step",
                                    error
                                ));
                            }
                        };
                        if written.size().is_some() {
                            let Some(result) =
                                cancelable_operation(writer.write_all(buf.filled()), close_receiver).await
                            else {
                                return Ok(RdpControlFlow::TerminatedGracefully(
                                    GracefulDisconnectReason::UserInitiated,
                                ));
                            };
                            if let Err(error) = result {
                                return Ok(RdpControlFlow::TransportFailure(ironrdp_session::custom_err!(
                                    "write deactivation-reactivation sequence step",
                                    error
                                )));
                            }
                        }
                        if let ConnectionActivationState::Finalized {
                            desktop_size,
                            share_id,
                            input_flags: _,
                            enable_server_pointer,
                            pointer_software_rendering,
                            static_channel_chunk_size,
                            refresh_rect_support: reactivated_refresh_rect_support,
                            suppress_output_support: reactivated_suppress_output_support,
                            window_support_level,
                            ..
                        } = connection_activation.connection_activation_state()
                        {
                            debug!(?desktop_size, "Deactivation-Reactivation Sequence completed");
                            image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                            desktop_update_extent = None;
                            resize_queue.completed();
                            if !active_stage.reactivate(
                                connection_activation.io_channel_id(),
                                connection_activation.user_channel_id(),
                                share_id,
                                enable_server_pointer,
                                pointer_software_rendering,
                                static_channel_chunk_size,
                            ) {
                                return Err(ironrdp_session::general_err!("invalid static channel chunk size"));
                            }
                            active_stage.set_window_support_level(window_support_level);
                            if let Some(monitor_layout) = connection_activation.monitor_layout()
                                && !send_active_output_event(
                                    output_event_sender,
                                    RdpOutputEvent::MonitorLayout(monitor_layout.monitors),
                                    close_receiver,
                                )
                                .await?
                            {
                                return Ok(RdpControlFlow::TerminatedGracefully(
                                    GracefulDisconnectReason::UserInitiated,
                                ));
                            }
                            if let Some(messages) =
                                active_stage.get_svc_processor_mut::<RailClient>().map(|rail_client| {
                                    rail_client.update_desktop_size(desktop_size.width, desktop_size.height)
                                })
                            {
                                let frame = active_stage.process_svc_processor_messages(messages)?;
                                if !frame.is_empty() {
                                    let Some(result) =
                                        cancelable_operation(writer.write_all(&frame), close_receiver).await
                                    else {
                                        return Ok(RdpControlFlow::TerminatedGracefully(
                                            GracefulDisconnectReason::UserInitiated,
                                        ));
                                    };
                                    if let Err(error) = result {
                                        return Ok(RdpControlFlow::TransportFailure(ironrdp_session::custom_err!(
                                            "write RAIL desktop size update",
                                            error
                                        )));
                                    }
                                }
                            }
                            refresh_rect_support = reactivated_refresh_rect_support;
                            suppress_output_support = reactivated_suppress_output_support;
                            break 'activation_seq;
                        }
                    }
                }
                ActiveStageOutput::MultitransportRequest(request) => {
                    #[cfg(feature = "udp")]
                    let (outcome, established_transport) =
                        if udp_tunnel.attempted_protocols.contains(&request.requested_protocol) {
                            warn!(
                                request_id = request.request_id,
                                requested_protocol = ?request.requested_protocol,
                                "Rejecting duplicate multitransport request"
                            );
                            (
                                ironrdp_connector::MultitransportResult::Failure(MultitransportResponsePdu::E_ABORT),
                                None,
                            )
                        } else if !multitransport_soft_sync {
                            udp_tunnel.attempted_protocols.push(request.requested_protocol);
                            warn!(
                                request_id = request.request_id,
                                requested_protocol = ?request.requested_protocol,
                                "Rejecting multitransport request without Soft-Sync"
                            );
                            (
                                ironrdp_connector::MultitransportResult::Failure(MultitransportResponsePdu::E_ABORT),
                                None,
                            )
                        } else if let Some(config) = udp_tunnel.bootstrap.clone() {
                            udp_tunnel.attempted_protocols.push(request.requested_protocol);
                            let Some(result) =
                                cancelable_operation(bootstrap_udp_transport(request.clone(), config), close_receiver)
                                    .await
                            else {
                                return Ok(RdpControlFlow::TerminatedGracefully(
                                    GracefulDisconnectReason::UserInitiated,
                                ));
                            };
                            match result {
                                Ok(transport) => (ironrdp_connector::MultitransportResult::Success, Some(transport)),
                                Err(error) => {
                                    warn!(
                                        request_id = request.request_id,
                                        requested_protocol = ?request.requested_protocol,
                                        %error,
                                        "Reliable UDP bootstrap failed; continuing with TCP"
                                    );
                                    (
                                        ironrdp_connector::MultitransportResult::Failure(
                                            MultitransportResponsePdu::E_ABORT,
                                        ),
                                        None,
                                    )
                                }
                            }
                        } else {
                            udp_tunnel.attempted_protocols.push(request.requested_protocol);
                            (
                                ironrdp_connector::MultitransportResult::Failure(MultitransportResponsePdu::E_ABORT),
                                None,
                            )
                        };
                    #[cfg(not(feature = "udp"))]
                    let outcome = {
                        debug!(
                            request_id = request.request_id,
                            requested_protocol = ?request.requested_protocol,
                            "Rejecting multitransport request because UDP support is disabled"
                        );
                        ironrdp_connector::MultitransportResult::Failure(MultitransportResponsePdu::E_ABORT)
                    };

                    if let Some(response) = outcome.response_pdu(request.request_id, multitransport_soft_sync) {
                        let frame = active_stage.encode_multitransport_response(&response)?;
                        let Some(result) = cancelable_operation(writer.write_all(&frame), close_receiver).await else {
                            return Ok(RdpControlFlow::TerminatedGracefully(
                                GracefulDisconnectReason::UserInitiated,
                            ));
                        };
                        if let Err(error) = result {
                            return Ok(RdpControlFlow::TransportFailure(ironrdp_session::custom_err!(
                                "write multitransport response",
                                error
                            )));
                        }
                    }
                    #[cfg(feature = "udp")]
                    if let Some(transport) = established_transport {
                        udp_tunnel.transport = Some(transport);
                        active_stage.enable_reliable_udp_dvc_tunnel()?;
                    }
                }
                ActiveStageOutput::AutoDetect(request) => {
                    debug!(?request, "Auto-detect");
                }
                ActiveStageOutput::Terminate(reason) => break 'outer reason,
            }
        }

        if malformed_bitmap_redraw_queued
            && !send_active_output_event(
                output_event_sender,
                RdpOutputEvent::MalformedBitmapDisplayRedraw,
                close_receiver,
            )
            .await?
        {
            return Ok(RdpControlFlow::TerminatedGracefully(
                GracefulDisconnectReason::UserInitiated,
            ));
        }

        for event in active_stage
            .get_svc_processor_mut::<RailClient>()
            .map(RailClient::take_events)
            .unwrap_or_default()
        {
            let output_event = match event {
                RailEvent::Handshake {
                    handshake_ex_flags,
                    initialization_message_count,
                    queued_execute_count,
                } => {
                    rail_queue_release_deadline = Some(tokio::time::Instant::now() + Duration::from_millis(250));
                    RdpOutputEvent::RailHandshake {
                        handshake_ex_flags,
                        initialization_message_count,
                        queued_execute_count,
                    }
                }
                RailEvent::DesktopSynchronized { released_execute_count } => {
                    RdpOutputEvent::RailDesktopSynchronized { released_execute_count }
                }
                RailEvent::PostHandshakeQueueReleased { released_execute_count } => {
                    RdpOutputEvent::RailPostHandshakeQueueReleased { released_execute_count }
                }
                RailEvent::ExecuteResult(result) => RdpOutputEvent::RailExecuteResult(result),
                RailEvent::ApplicationId {
                    window_id,
                    application_id,
                    process_id,
                    process_image_name,
                } => RdpOutputEvent::RailApplicationId {
                    window_id,
                    application_id,
                    process_id,
                    process_image_name,
                },
                RailEvent::Control(control) => RdpOutputEvent::RailControl(control),
            };
            if !send_active_output_event(output_event_sender, output_event, close_receiver).await? {
                return Ok(RdpControlFlow::TerminatedGracefully(
                    GracefulDisconnectReason::UserInitiated,
                ));
            }
        }

        if resize_queue.in_flight.is_none()
            && let Some(pending) = resize_queue.pending.as_ref()
        {
            let request = pending.request;
            match active_stage.display_control_ready() {
                Some(true) => {
                    let batch = active_stage
                        .prepare_resize(
                            u32::from(request.width),
                            u32::from(request.height),
                            Some(request.scale_factor),
                            request.physical_size,
                        )
                        .ok_or_else(|| ironrdp_session::general_err!("Display Control became unavailable"))??;
                    resize_queue.pending = None;
                    resize_queue.mark_in_flight(request);
                    let channel_id = batch.channel_id();
                    let messages = batch.into_messages();
                    #[cfg(feature = "udp")]
                    let route_over_udp =
                        active_stage.dvc_tunnel_for_channel(channel_id) == Some(SoftSyncTunnelType::RELIABLE_UDP);
                    #[cfg(not(feature = "udp"))]
                    let route_over_udp = {
                        let _ = channel_id;
                        false
                    };
                    if route_over_udp {
                        #[cfg(feature = "udp")]
                        {
                            let Some(transport) = udp_tunnel.transport.as_ref() else {
                                return Ok(RdpControlFlow::TransportFailure(ironrdp_session::general_err!(
                                    "reliable UDP tunnel is unavailable for a Soft-Sync channel"
                                )));
                            };
                            for message in messages {
                                let payload = message
                                    .encode_unframed_pdu()
                                    .map_err(|error| ironrdp_session::custom_err!("encode tunneled resize", error))?;
                                let Some(result) = cancelable_operation(transport.send(payload), close_receiver).await
                                else {
                                    return Ok(RdpControlFlow::TerminatedGracefully(
                                        GracefulDisconnectReason::UserInitiated,
                                    ));
                                };
                                if let Err(error) = result {
                                    return Ok(RdpControlFlow::TransportFailure(ironrdp_session::custom_err!(
                                        "write pending resize over reliable UDP",
                                        error
                                    )));
                                }
                            }
                        }
                    } else {
                        let response_frame = active_stage.encode_dvc_messages(messages)?;
                        let Some(result) =
                            cancelable_operation(writer.write_all(&response_frame), close_receiver).await
                        else {
                            return Ok(RdpControlFlow::TerminatedGracefully(
                                GracefulDisconnectReason::UserInitiated,
                            ));
                        };
                        if let Err(error) = result {
                            return Ok(RdpControlFlow::TransportFailure(ironrdp_session::custom_err!(
                                "write pending resize",
                                error
                            )));
                        }
                    }
                }
                None => {
                    debug!("Reconnecting because Display Control is unavailable");
                    return Ok(RdpControlFlow::ReconnectWithNewSize {
                        width: request.width,
                        height: request.height,
                        reason: DisplayResizeFallbackReason::DisplayControlUnavailable,
                    });
                }
                Some(false) => {}
            }
        }
    };

    Ok(RdpControlFlow::TerminatedGracefully(disconnect_reason))
}

#[cfg(feature = "clipboard")]
fn process_clipboard_message(
    active_stage: &mut ActiveStage,
    event: ClipboardMessage,
) -> SessionResult<Vec<ActiveStageOutput>> {
    let Some(cliprdr_client) = active_stage.get_svc_processor_mut::<ironrdp_cliprdr::CliprdrClient>() else {
        warn!("Clipboard event received, but Cliprdr is not available");
        return Ok(Vec::new());
    };

    let svc_messages = match event {
        ClipboardMessage::SendInitiateCopy(formats) => Some(
            cliprdr_client
                .initiate_copy(&formats)
                .map_err(|e| ironrdp_session::custom_err!("CLIPRDR", e))?,
        ),
        ClipboardMessage::SendInitiateFileCopy(files) => Some(
            cliprdr_client
                .initiate_file_copy(files)
                .map_err(|e| ironrdp_session::custom_err!("CLIPRDR", e))?,
        ),
        ClipboardMessage::SendFormatData(response) => Some(
            cliprdr_client
                .submit_format_data(response)
                .map_err(|e| ironrdp_session::custom_err!("CLIPRDR", e))?,
        ),
        ClipboardMessage::SendInitiatePaste(format) => Some(
            cliprdr_client
                .initiate_paste(format)
                .map_err(|e| ironrdp_session::custom_err!("CLIPRDR", e))?,
        ),
        ClipboardMessage::SendFileContentsRequest(request) => Some(
            cliprdr_client
                .request_file_contents(request)
                .map_err(|e| ironrdp_session::custom_err!("CLIPRDR", e))?,
        ),
        ClipboardMessage::SendFileContentsResponse(response) => Some(
            cliprdr_client
                .submit_file_contents(response)
                .map_err(|e| ironrdp_session::custom_err!("CLIPRDR", e))?,
        ),
        ClipboardMessage::Error(error) => {
            error!("Clipboard backend error: {error}");
            None
        }
    };

    let Some(svc_messages) = svc_messages else {
        return Ok(Vec::new());
    };

    let frame = active_stage.process_svc_processor_messages(svc_messages)?;
    Ok(vec![ActiveStageOutput::ResponseFrame(frame)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "rdpdr")]
    use core::any::TypeId;
    #[cfg(feature = "rdpdr")]
    use core::sync::atomic::AtomicUsize;

    #[cfg(feature = "rdpdr")]
    use ironrdp_core::encode_vec;
    use ironrdp_pdu::input::fast_path::KeyboardFlags;
    #[cfg(feature = "rdpdr")]
    use ironrdp_rdpdr::RdpdrBackend;
    #[cfg(feature = "rdpdr")]
    use ironrdp_rdpdr::pdu::RdpdrPdu;
    #[cfg(feature = "rdpdr")]
    use ironrdp_rdpdr::pdu::efs::{
        CapabilityMessage, CoreCapability, CoreCapabilityKind, DeviceControlRequest, DeviceType,
        ServerDeviceAnnounceResponse, ServerDriveIoRequest, VERSION_MINOR_12, VersionAndIdPdu, VersionAndIdPduKind,
    };
    #[cfg(feature = "rdpdr")]
    use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
    #[cfg(feature = "rdpdr")]
    use ironrdp_svc::{StaticChannelSet, SvcProcessor as _};

    fn mouse_move(x: u16, y: u16) -> FastPathInputEvent {
        FastPathInputEvent::MouseEvent(MousePdu {
            flags: PointerFlags::MOVE,
            number_of_wheel_rotation_units: 0,
            x_position: x,
            y_position: y,
        })
    }

    #[test]
    fn input_batcher_delays_mouse_moves_until_the_minimum_interval() {
        let start = tokio::time::Instant::now();
        let mut batcher = FastPathInputBatcher::new(Some(Duration::from_millis(100)), start);

        assert!(batcher.queue([mouse_move(1, 1)], start).is_none());
        assert_eq!(batcher.deadline(), Some(start + Duration::from_millis(100)));
        assert!(
            batcher
                .queue([mouse_move(2, 2)], start + Duration::from_millis(99))
                .is_none()
        );

        let events = batcher
            .flush(start + Duration::from_millis(100))
            .expect("pending mouse movement");
        assert_eq!(events.as_slice(), [mouse_move(1, 1), mouse_move(2, 2)]);
        assert_eq!(batcher.deadline(), None);
    }

    #[test]
    fn input_batcher_sends_forced_and_full_batches_immediately() {
        let start = tokio::time::Instant::now();
        let mut batcher = FastPathInputBatcher::new(Some(Duration::from_millis(100)), start);
        assert!(batcher.queue([mouse_move(1, 1)], start).is_none());

        let events = batcher
            .queue(
                [FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1e)],
                start + Duration::from_millis(1),
            )
            .expect("keyboard input forces the pending batch");
        assert_eq!(events.len(), 2);

        let events = batcher
            .queue(
                (0..INPUT_BATCH_EVENT_LIMIT).map(|position| {
                    let position = u16::try_from(position).expect("test event count fits in u16");
                    mouse_move(position, position)
                }),
                start + Duration::from_millis(2),
            )
            .expect("the native event-count threshold forces the batch");
        assert_eq!(events.len(), INPUT_BATCH_EVENT_LIMIT);

        assert!(
            batcher
                .queue([mouse_move(20, 20)], start + Duration::from_millis(3))
                .is_none()
        );
        let events = batcher.queue_forced([mouse_move(20, 20)], start + Duration::from_millis(4));
        assert_eq!(events.as_slice(), [mouse_move(20, 20), mouse_move(20, 20)]);
        assert_eq!(batcher.deadline(), None);
    }

    #[cfg(feature = "location")]
    #[test]
    fn location_delivery_timeout_cancels_queued_request() {
        let (sender, mut receiver) = RdpInputSender::channel(1);
        let delivery = sender
            .try_send_location(45.5, -73.5, 100, Duration::ZERO)
            .expect("queue location request");

        assert_eq!(delivery.wait(), Err(LocationDeliveryError::Timeout));
        let RdpInputEvent::Location(request) = receiver.try_recv().expect("queued location request") else {
            panic!("expected location request");
        };
        assert!(request.is_cancelled_or_expired());
    }

    #[cfg(feature = "location")]
    #[test]
    fn committed_location_delivery_wins_timeout_race() {
        let (sender, mut receiver) = RdpInputSender::channel(1);
        let delivery = sender
            .try_send_location(45.5, -73.5, 100, Duration::from_secs(1))
            .expect("queue location request");
        let RdpInputEvent::Location(request) = receiver.try_recv().expect("queued location request") else {
            panic!("expected location request");
        };
        request.complete(Ok(()));

        assert_eq!(delivery.wait(), Ok(Ok(())));
    }

    #[cfg(feature = "rdpdr")]
    #[derive(Debug)]
    struct TestRdpdrBackend {
        instance: usize,
        deferred_messages: Vec<SvcMessage>,
        dynamic_drive_count: Arc<AtomicUsize>,
    }

    #[cfg(feature = "rdpdr")]
    impl TestRdpdrBackend {
        fn new(instance: usize) -> Self {
            Self {
                instance,
                deferred_messages: Vec::new(),
                dynamic_drive_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        fn with_deferred_message() -> Self {
            Self {
                instance: 0,
                deferred_messages: vec![SvcMessage::from(RdpdrPdu::EmptyResponse)],
                dynamic_drive_count: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[cfg(feature = "rdpdr")]
    ironrdp_core::impl_as_any!(TestRdpdrBackend);

    #[cfg(feature = "rdpdr")]
    impl RdpdrBackend for TestRdpdrBackend {
        fn handle_server_device_announce_response(
            &mut self,
            _pdu: ServerDeviceAnnounceResponse,
        ) -> ironrdp_pdu::PduResult<()> {
            Ok(())
        }

        fn handle_scard_call(
            &mut self,
            _req: DeviceControlRequest<ScardIoCtlCode>,
            _call: ScardCall,
        ) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
            Ok(Vec::new())
        }

        fn handle_drive_io_request(&mut self, _req: ServerDriveIoRequest) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
            Ok(Vec::new())
        }

        fn poll_deferred_messages(&mut self) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
            Ok(core::mem::take(&mut self.deferred_messages))
        }

        fn add_drive(&mut self, _device_id: u32) -> ironrdp_pdu::PduResult<()> {
            self.dynamic_drive_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn remove_drive(&mut self, _device_id: u32) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
            self.dynamic_drive_count.fetch_sub(1, Ordering::SeqCst);
            Ok(Vec::new())
        }
    }

    #[cfg(feature = "rdpdr")]
    #[derive(Debug)]
    struct CountingRdpdrFactory {
        builds: AtomicUsize,
        initial_drives: Vec<RdpdrDrive>,
        drive_hotplug: bool,
        printer: Option<RdpdrPrinter>,
    }

    #[cfg(feature = "rdpdr")]
    impl CountingRdpdrFactory {
        fn new(initial_drives: Vec<RdpdrDrive>) -> Self {
            Self {
                builds: AtomicUsize::new(0),
                initial_drives,
                drive_hotplug: false,
                printer: None,
            }
        }

        fn with_drive_hotplug(mut self) -> Self {
            self.drive_hotplug = true;
            self
        }

        fn with_printer(mut self, printer: RdpdrPrinter) -> Self {
            self.printer = Some(printer);
            self
        }
    }

    #[cfg(feature = "rdpdr")]
    impl RdpdrBackendFactory for CountingRdpdrFactory {
        fn build_rdpdr_backend(&self) -> RdpdrBackendFactoryResult<RdpdrBackendProduct> {
            let instance = self.builds.fetch_add(1, Ordering::SeqCst) + 1;
            let mut product =
                RdpdrBackendProduct::new(Box::new(TestRdpdrBackend::new(instance)), self.initial_drives.clone())
                    .with_drive_hotplug(self.drive_hotplug);
            if let Some(printer) = &self.printer {
                product = product.with_printer(printer.clone());
            }
            Ok(product)
        }
    }

    #[cfg(feature = "clipboard")]
    fn no_cliprdr_factory() -> CliprdrFactoryRef<'static> {
        None
    }

    #[cfg(not(feature = "clipboard"))]
    fn no_cliprdr_factory() -> CliprdrFactoryRef<'static> {
        core::marker::PhantomData
    }

    #[cfg(feature = "rdpdr")]
    fn test_config() -> Config {
        crate::config::ConfigBuilder::new()
            .with_destination(crate::config::Destination::from_parts("127.0.0.1", 3389))
            .with_username("user")
            .with_password("password")
            .with_client_build(1)
            .with_client_dir(r"C:\")
            .with_platform(ironrdp_pdu::rdp::capability_sets::MajorPlatformType::WINDOWS)
            .with_client_name("client")
            .with_rdpdr(true)
            .build()
            .expect("test configuration should build")
    }

    fn resize_request(width: u16, height: u16) -> ResizeRequest {
        ResizeRequest {
            width,
            height,
            scale_factor: 100,
            physical_size: None,
        }
    }

    #[cfg(feature = "udp")]
    #[test]
    fn reliable_udp_advertises_only_the_supported_soft_sync_transport() {
        assert!(reliable_udp_multitransport_flags(false).is_none());
        assert_eq!(
            reliable_udp_multitransport_flags(true),
            Some(
                ironrdp_pdu::gcc::MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR
                    | ironrdp_pdu::gcc::MultiTransportFlags::SOFT_SYNC_TCP_TO_UDP
            )
        );
    }

    #[cfg(all(feature = "gateway", feature = "clipboard", feature = "rdpdr"))]
    #[test]
    fn gateway_redirection_policy_interprets_flags() {
        let no_policy = GatewayRedirectionPolicy::from_flags(None);
        assert!(!no_policy.disables_clipboard());
        assert!(!no_policy.disables_drive());

        let enable_all = GatewayRedirectionPolicy::from_flags(Some(0x8000_0000));
        assert!(!enable_all.disables_clipboard());
        assert!(!enable_all.disables_drive());

        let disable_clipboard = GatewayRedirectionPolicy::from_flags(Some(HTTP_TUNNEL_REDIR_DISABLE_CLIPBOARD));
        assert!(disable_clipboard.disables_clipboard());
        assert!(!disable_clipboard.disables_drive());

        let disable_drive = GatewayRedirectionPolicy::from_flags(Some(HTTP_TUNNEL_REDIR_DISABLE_DRIVE));
        assert!(!disable_drive.disables_clipboard());
        assert!(disable_drive.disables_drive());

        let disable_all = GatewayRedirectionPolicy::from_flags(Some(
            HTTP_TUNNEL_REDIR_DISABLE_ALL | HTTP_TUNNEL_REDIR_DISABLE_CLIPBOARD,
        ));
        assert!(disable_all.disables_clipboard());
        assert!(disable_all.disables_drive());
        assert!(disable_all.disables_all());
    }

    #[test]
    fn resize_queue_coalesces_later_requests_until_reactivation() {
        let first = resize_request(1024, 768);
        let latest = resize_request(1600, 900);
        let mut queue = ResizeQueue::default();
        queue.mark_in_flight(first);
        let deadline = queue.deadline().expect("in-flight resize must have a deadline");
        queue.defer(latest);

        assert_eq!(
            queue.timed_out_request(deadline),
            Some((latest, DisplayResizeFallbackReason::ReactivationTimedOut))
        );
        queue.completed();
        assert!(queue.in_flight.is_none());
        assert_eq!(queue.pending.as_ref().map(|pending| pending.request), Some(latest));
    }

    #[test]
    fn resize_queue_times_out_while_waiting_for_display_control_capabilities() {
        let request = resize_request(1280, 720);
        let mut queue = ResizeQueue::default();
        queue.defer(request);
        let deadline = queue.deadline().expect("pending resize must have a deadline");

        assert_eq!(queue.timed_out_request(deadline - Duration::from_millis(1)), None);
        assert_eq!(
            queue.timed_out_request(deadline),
            Some((request, DisplayResizeFallbackReason::CapabilitiesTimedOut))
        );
    }

    #[test]
    fn auto_reconnect_policy_requires_a_cookie_and_respects_its_limit() {
        let policy = AutoReconnectPolicy::new(2);

        assert_eq!(policy.next_attempt(0, false), None);
        assert_eq!(policy.next_attempt(0, true), Some(1));
        assert_eq!(policy.next_attempt(1, true), Some(2));
        assert_eq!(policy.next_attempt(2, true), None);
    }

    #[test]
    fn zero_auto_reconnect_limit_disables_retries() {
        assert_eq!(AutoReconnectPolicy::new(0).next_attempt(0, true), None);
    }

    #[test]
    fn protocol_decode_errors_do_not_trigger_auto_reconnect() {
        let decode_error = ironrdp_pdu::find_size(&[0x01]).expect_err("invalid fast-path action must fail");
        let protocol_error = io::Error::other(decode_error);

        assert!(!is_transport_read_error(&protocol_error));
        assert!(is_transport_read_error(&io::Error::from(
            io::ErrorKind::ConnectionReset
        )));

        let transport_error = ironrdp_session::custom_err!(
            "read activation",
            ironrdp_connector::custom_err!("read frame", io::Error::from(io::ErrorKind::ConnectionReset))
        );
        assert!(is_transport_session_error(&transport_error));

        let protocol_error = ironrdp_session::custom_err!(
            "read activation",
            ironrdp_connector::custom_err!("read frame", protocol_error)
        );
        assert!(!is_transport_session_error(&protocol_error));
    }

    #[test]
    fn input_sender_bounds_events_but_close_bypasses_the_queue() {
        let (sender, mut receiver, _, mut close_receiver, _) = RdpInputSender::channel_with_close_signal(1);
        sender
            .try_send(RdpInputEvent::Resize {
                width: 1024,
                height: 768,
                scale_factor: 100,
                physical_size: None,
            })
            .expect("the first event fits");
        assert!(
            sender
                .try_send(RdpInputEvent::Resize {
                    width: 1280,
                    height: 720,
                    scale_factor: 100,
                    physical_size: None,
                })
                .is_err()
        );

        sender.request_close();
        assert!(close_receiver.has_changed().expect("close sender is alive"));
        assert!(*close_receiver.borrow_and_update());
        assert!(matches!(receiver.try_recv(), Ok(RdpInputEvent::Resize { .. })));
    }

    #[test]
    fn graceful_close_bypasses_the_input_queue_without_cancelling_the_session() {
        let (sender, mut receiver, _, close_receiver, mut graceful_close_receiver) =
            RdpInputSender::channel_with_close_signal(1);
        sender
            .try_send(RdpInputEvent::Resize {
                width: 1024,
                height: 768,
                scale_factor: 100,
                physical_size: None,
            })
            .expect("the first event fits");

        sender.request_graceful_close();

        assert!(!close_receiver.has_changed().expect("close sender is alive"));
        assert!(
            graceful_close_receiver
                .has_changed()
                .expect("graceful close sender is alive")
        );
        assert!(*graceful_close_receiver.borrow_and_update());
        assert!(matches!(receiver.try_recv(), Ok(RdpInputEvent::Resize { .. })));
    }

    #[cfg(feature = "rdpdr")]
    #[test]
    fn disabled_rdpdr_does_not_build_a_backend() {
        let factory = CountingRdpdrFactory::new(vec![RdpdrDrive::new(42, "fixtures".to_owned())]);
        let config = crate::config::RdpdrConfig {
            enabled: false,
            #[cfg(feature = "smartcard")]
            smartcard: false,
        };

        assert!(
            build_rdpdr_channel(Some(&factory), &config, true)
                .expect("disabled RDPDR should not fail")
                .is_none()
        );
        assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    }

    #[cfg(feature = "rdpdr")]
    #[test]
    fn empty_rdpdr_product_omits_the_channel() {
        let factory = CountingRdpdrFactory::new(Vec::new());
        // Default RdpdrConfig enables smartcard when the feature is on; disable it so
        // this case still covers an empty drive product with no smartcard device.
        let config = crate::config::RdpdrConfig {
            enabled: true,
            #[cfg(feature = "smartcard")]
            smartcard: false,
        };

        assert!(
            build_rdpdr_channel(Some(&factory), &config, true)
                .expect("empty RDPDR product should not fail")
                .is_none()
        );
        assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
    }

    #[cfg(feature = "rdpdr")]
    #[test]
    fn printer_only_rdpdr_product_attaches_the_channel() {
        let factory = CountingRdpdrFactory::new(Vec::new()).with_printer(
            RdpdrPrinter::new(42, "Office Printer".to_owned(), "Office Driver".to_owned()).with_network(false),
        );
        let config = crate::config::RdpdrConfig {
            enabled: true,
            #[cfg(feature = "smartcard")]
            smartcard: false,
        };

        let mut rdpdr = build_rdpdr_channel(Some(&factory), &config, true)
            .expect("printer-only RDPDR product should build")
            .expect("printer metadata keeps the RDPDR channel attached");
        let client_id = 0x1234_5678;
        rdpdr
            .process(
                &encode_vec(&RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
                    version_major: 1,
                    version_minor: VERSION_MINOR_12,
                    client_id,
                    kind: VersionAndIdPduKind::ServerAnnounceRequest,
                }))
                .expect("encode server announce"),
            )
            .expect("process server announce");
        rdpdr
            .process(
                &encode_vec(&RdpdrPdu::CoreCapability(CoreCapability {
                    capabilities: vec![CapabilityMessage::new_general(0), CapabilityMessage::new_printer()],
                    kind: CoreCapabilityKind::ServerCoreCapabilityRequest,
                }))
                .expect("encode server capability"),
            )
            .expect("process server capability");
        assert!(
            rdpdr
                .process(
                    &encode_vec(&RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
                        version_major: 1,
                        version_minor: VERSION_MINOR_12,
                        client_id,
                        kind: VersionAndIdPduKind::ServerClientIdConfirm,
                    }))
                    .expect("encode client ID confirm"),
                )
                .expect("process client ID confirm")
                .is_empty()
        );

        let announcements = rdpdr
            .process(&encode_vec(&RdpdrPdu::UserLoggedon).expect("encode user logged on"))
            .expect("process user logged on");
        assert_eq!(announcements.len(), 1);
        let wire = announcements[0]
            .encode_unframed_pdu()
            .expect("encode printer announcement");
        assert_eq!(
            u32::from_le_bytes(wire[8..12].try_into().unwrap()),
            u32::from(DeviceType::Print)
        );
        assert_eq!(u32::from_le_bytes(wire[12..16].try_into().unwrap()), 42);
        assert_eq!(
            u32::from_le_bytes(wire[28..32].try_into().unwrap()),
            ironrdp_rdpdr::pdu::efs::RDPDR_PRINTER_ANNOUNCE_FLAG_DEFAULTPRINTER
        );
    }

    #[cfg(feature = "rdpdr")]
    #[test]
    fn rdpdr_rejects_duplicate_printer_and_drive_device_ids() {
        let factory = CountingRdpdrFactory::new(vec![RdpdrDrive::new(42, "drive".to_owned())]).with_printer(
            RdpdrPrinter::new(42, "Office Printer".to_owned(), "Office Driver".to_owned()),
        );
        let config = crate::config::RdpdrConfig {
            enabled: true,
            #[cfg(feature = "smartcard")]
            smartcard: false,
        };

        assert!(build_rdpdr_channel(Some(&factory), &config, true).is_err());
    }

    #[cfg(feature = "rdpdr")]
    #[test]
    fn hotplug_only_rdpdr_product_negotiates_drive_capability() {
        let factory = CountingRdpdrFactory::new(Vec::new()).with_drive_hotplug();
        let config = crate::config::RdpdrConfig {
            enabled: true,
            #[cfg(feature = "smartcard")]
            smartcard: false,
        };

        let mut rdpdr = build_rdpdr_channel(Some(&factory), &config, true)
            .expect("hotplug-only RDPDR product should not fail")
            .expect("hotplug-only product should build a channel");
        let server_capability = RdpdrPdu::CoreCapability(CoreCapability {
            capabilities: vec![CapabilityMessage::new_general(0), CapabilityMessage::new_drive()],
            kind: CoreCapabilityKind::ServerCoreCapabilityRequest,
        });
        rdpdr
            .process(&encode_vec(&server_capability).expect("encode server capability"))
            .expect("process server capability");
        assert!(
            rdpdr
                .add_dynamic_drive(42, "E:".to_owned())
                .expect("hotplug capability was configured before negotiation")
                .is_empty()
        );
        assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
    }

    #[cfg(feature = "rdpdr")]
    #[test]
    fn drive_restricted_rdpdr_without_smartcard_omits_the_channel() {
        let factory = CountingRdpdrFactory::new(vec![RdpdrDrive::new(42, "fixtures".to_owned())]);
        let config = crate::config::RdpdrConfig {
            enabled: true,
            #[cfg(feature = "smartcard")]
            smartcard: false,
        };

        assert!(
            build_rdpdr_channel(Some(&factory), &config, false)
                .expect("drive-restricted RDPDR should not fail")
                .is_none()
        );
        assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
    }

    #[cfg(all(feature = "rdpdr", feature = "smartcard"))]
    #[test]
    fn drive_restricted_rdpdr_preserves_smartcard_redirection() {
        let factory = CountingRdpdrFactory::new(vec![RdpdrDrive::new(42, "fixtures".to_owned())]);
        let config = crate::config::RdpdrConfig {
            enabled: true,
            smartcard: true,
        };

        let mut rdpdr = build_rdpdr_channel(Some(&factory), &config, false)
            .expect("drive-restricted RDPDR should not fail")
            .expect("smartcard-only product should build a channel");

        assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
        assert!(
            rdpdr
                .downcast_backend::<TestRdpdrBackend>()
                .expect("test backend should be retained")
                .instance
                >= 1
        );

        let client_id = 0x1234_5678;
        let server_announce = RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
            version_major: 1,
            version_minor: VERSION_MINOR_12,
            client_id,
            kind: VersionAndIdPduKind::ServerAnnounceRequest,
        });
        assert_eq!(
            rdpdr
                .process(&encode_vec(&server_announce).expect("encode server announce"))
                .expect("process server announce")
                .len(),
            2
        );

        let client_confirm = RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
            version_major: 1,
            version_minor: VERSION_MINOR_12,
            client_id,
            kind: VersionAndIdPduKind::ServerClientIdConfirm,
        });
        let announcements = rdpdr
            .process(&encode_vec(&client_confirm).expect("encode client ID confirm"))
            .expect("process client ID confirm");
        assert_eq!(announcements.len(), 1);
        let wire = announcements[0]
            .encode_unframed_pdu()
            .expect("encode device announcement");
        assert_eq!(
            u16::from_le_bytes(wire[2..4].try_into().expect("device announcement packet ID")),
            u16::from(ironrdp_rdpdr::pdu::PacketId::CoreDevicelistAnnounce)
        );
        assert_eq!(u32::from_le_bytes(wire[4..8].try_into().expect("device count")), 1);
        assert_eq!(
            u32::from_le_bytes(wire[8..12].try_into().expect("device type")),
            u32::from(DeviceType::Smartcard)
        );
        assert_eq!(u32::from_le_bytes(wire[12..16].try_into().expect("device ID")), 0);

        let server_capability = RdpdrPdu::CoreCapability(CoreCapability {
            capabilities: vec![
                CapabilityMessage::new_general(0),
                CapabilityMessage::new_drive(),
                CapabilityMessage::new_smartcard(),
            ],
            kind: CoreCapabilityKind::ServerCoreCapabilityRequest,
        });
        let capability_response = rdpdr
            .process(&encode_vec(&server_capability).expect("encode server capability"))
            .expect("process server capability");
        assert_eq!(capability_response.len(), 1);
        assert_eq!(
            capability_response[0]
                .encode_unframed_pdu()
                .expect("encode client capability"),
            encode_vec(&RdpdrPdu::CoreCapability(CoreCapability::new_response(vec![
                CapabilityMessage::new_general(1),
                CapabilityMessage::new_smartcard(),
            ])))
            .expect("encode expected client capability")
        );

        assert!(
            rdpdr
                .process(&encode_vec(&RdpdrPdu::UserLoggedon).expect("encode user logged on"))
                .expect("process user logged on")
                .is_empty()
        );
    }

    #[cfg(feature = "rdpdr")]
    #[test]
    fn rdpdr_factory_builds_a_fresh_backend_for_each_attempt() {
        let factory = CountingRdpdrFactory::new(vec![RdpdrDrive::new(42, "fixtures".to_owned())]);
        let config = crate::config::RdpdrConfig {
            enabled: true,
            #[cfg(feature = "smartcard")]
            smartcard: false,
        };

        let first = build_rdpdr_channel(Some(&factory), &config, true)
            .expect("first connection attempt should build")
            .expect("first product contains a filesystem device");
        let second = build_rdpdr_channel(Some(&factory), &config, true)
            .expect("second connection attempt should build")
            .expect("second product contains a filesystem device");

        assert_eq!(factory.builds.load(Ordering::SeqCst), 2);
        assert_eq!(
            first
                .downcast_backend::<TestRdpdrBackend>()
                .expect("first test backend should be retained")
                .instance,
            1
        );
        assert_eq!(
            second
                .downcast_backend::<TestRdpdrBackend>()
                .expect("second test backend should be retained")
                .instance,
            2
        );
    }

    #[cfg(feature = "rdpdr")]
    #[test]
    fn rdpdr_announces_the_factory_device_metadata() {
        let factory = CountingRdpdrFactory::new(vec![RdpdrDrive::new(42, "fixtures".to_owned())]);
        let config = crate::config::RdpdrConfig {
            enabled: true,
            #[cfg(feature = "smartcard")]
            smartcard: false,
        };
        let mut rdpdr = build_rdpdr_channel(Some(&factory), &config, true)
            .expect("RDPDR channel should build")
            .expect("product contains a filesystem device");
        let client_id = 0x1234_5678;

        let server_announce = RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
            version_major: 1,
            version_minor: VERSION_MINOR_12,
            client_id,
            kind: VersionAndIdPduKind::ServerAnnounceRequest,
        });
        assert_eq!(
            rdpdr
                .process(&encode_vec(&server_announce).expect("encode server announce"))
                .expect("process server announce")
                .len(),
            2
        );

        let client_confirm = RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
            version_major: 1,
            version_minor: VERSION_MINOR_12,
            client_id,
            kind: VersionAndIdPduKind::ServerClientIdConfirm,
        });
        assert!(
            rdpdr
                .process(&encode_vec(&client_confirm).expect("encode client ID confirm"))
                .expect("process client ID confirm")
                .is_empty()
        );
        let announcements = rdpdr
            .process(&encode_vec(&RdpdrPdu::UserLoggedon).expect("encode user logged on"))
            .expect("process user logged on");
        assert_eq!(announcements.len(), 1);

        let wire = announcements[0]
            .encode_unframed_pdu()
            .expect("encode device announcement");
        assert_eq!(
            u16::from_le_bytes(wire[2..4].try_into().expect("device announcement packet ID")),
            u16::from(ironrdp_rdpdr::pdu::PacketId::CoreDevicelistAnnounce)
        );
        assert_eq!(u32::from_le_bytes(wire[4..8].try_into().expect("device count")), 1);
        assert_eq!(u32::from_le_bytes(wire[12..16].try_into().expect("device ID")), 42);

        let expected_name = "fixtures"
            .encode_utf16()
            .chain(core::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        assert_eq!(
            usize::try_from(u32::from_le_bytes(wire[24..28].try_into().expect("device data length")))
                .expect("device data length fits usize"),
            expected_name.len()
        );
        assert_eq!(&wire[28..], expected_name);
    }

    #[cfg(feature = "rdpdr")]
    #[test]
    fn rdpdr_static_channel_is_attached_with_rdpsnd() {
        let factory = CountingRdpdrFactory::new(vec![RdpdrDrive::new(42, "fixtures".to_owned())]);
        let config = test_config();
        let (input_sender, _) = RdpInputSender::channel(1);
        let (output_event_sender, _) = crate::output_channel::output_channel(1);
        let mut connector = build_connector(
            &config,
            SocketAddr::from(([127, 0, 0, 1], 0)),
            (&input_sender, &output_event_sender),
            no_cliprdr_factory(),
            Some(&factory),
            true,
            false,
            None,
        )
        .expect("RDPDR connector should build");

        assert!(input_sender.rdpdr_drive_hotplug_available());
        assert!(
            connector
                .get_static_channel_processor::<ironrdp_rdpdr::Rdpdr>()
                .is_some()
        );
        assert!(
            connector
                .get_static_channel_processor::<ironrdp_rdpsnd::client::Rdpsnd>()
                .is_some()
        );
    }

    #[cfg(feature = "rdpdr")]
    #[test]
    fn noop_rdpdr_fallback_does_not_report_drive_hotplug() {
        let config = test_config();
        let (input_sender, _) = RdpInputSender::channel(1);
        let (output_event_sender, _) = crate::output_channel::output_channel(1);
        let mut connector = build_connector(
            &config,
            SocketAddr::from(([127, 0, 0, 1], 0)),
            (&input_sender, &output_event_sender),
            no_cliprdr_factory(),
            None,
            true,
            false,
            None,
        )
        .expect("Noop RDPDR connector should build");

        assert!(!input_sender.rdpdr_drive_hotplug_available());
        assert!(
            connector
                .get_static_channel_processor::<ironrdp_rdpdr::Rdpdr>()
                .is_some()
        );
    }

    #[cfg(feature = "rdpdr")]
    #[test]
    fn deferred_rdpdr_messages_use_the_static_channel() {
        let mut static_channels = StaticChannelSet::new();
        assert!(
            static_channels
                .insert(ironrdp_rdpdr::Rdpdr::new(
                    Box::new(TestRdpdrBackend::with_deferred_message()),
                    "test".to_owned(),
                ))
                .is_none()
        );
        assert!(
            static_channels
                .attach_channel_id(TypeId::of::<ironrdp_rdpdr::Rdpdr>(), 1005)
                .is_none()
        );
        let mut active_stage = ActiveStageBuilder {
            static_channels,
            user_channel_id: 1001,
            io_channel_id: 1003,
            message_channel_id: None,
            share_id: 0,
            compression_type: None,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();

        let output = poll_deferred_rdpdr_output(&mut active_stage)
            .expect("deferred RDPDR messages should encode")
            .expect("deferred RDPDR message should produce a static-channel frame");
        let ActiveStageOutput::ResponseFrame(frame) = output else {
            panic!("expected a static-channel response frame");
        };
        assert!(!frame.is_empty());
        assert!(
            poll_deferred_rdpdr_output(&mut active_stage)
                .expect("polling an empty backend should succeed")
                .is_none()
        );
    }

    #[cfg(feature = "rdpdr")]
    #[test]
    fn dynamic_drive_changes_reach_the_rdpdr_processor() {
        let backend = TestRdpdrBackend::new(0);
        let dynamic_drive_count = Arc::clone(&backend.dynamic_drive_count);
        let mut static_channels = StaticChannelSet::new();
        assert!(
            static_channels
                .insert(ironrdp_rdpdr::Rdpdr::new(Box::new(backend), "test".to_owned()).with_drives(None))
                .is_none()
        );
        let mut active_stage = ActiveStageBuilder {
            static_channels,
            user_channel_id: 1001,
            io_channel_id: 1003,
            message_channel_id: None,
            share_id: 0,
            compression_type: None,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();

        assert!(
            process_rdpdr_drive_change(&mut active_stage, 7, Some("E:".to_owned()))
                .expect("dynamic add is accepted before post-logon announcement")
                .is_empty()
        );
        assert_eq!(dynamic_drive_count.load(Ordering::SeqCst), 1);

        assert!(
            process_rdpdr_drive_change(&mut active_stage, 7, None)
                .expect("dynamic removal is accepted")
                .is_empty()
        );
        assert_eq!(dynamic_drive_count.load(Ordering::SeqCst), 0);
    }

    #[cfg(feature = "clipboard")]
    #[test]
    fn clipboard_messages_bypass_the_bounded_input_queue() {
        let (sender, _input_receiver, mut clipboard_receiver, _, _) = RdpInputSender::channel_with_close_signal(1);
        sender
            .try_send(RdpInputEvent::Resize {
                width: 1024,
                height: 768,
                scale_factor: 100,
                physical_size: None,
            })
            .expect("the first event fits");

        sender
            .send_clipboard(ClipboardMessage::Error(Box::new(io::Error::other(
                "test clipboard message",
            ))))
            .expect("clipboard messages use a dedicated queue");

        assert!(matches!(
            clipboard_receiver.try_recv(),
            Ok(RdpInputEvent::Clipboard(ClipboardMessage::Error(_)))
        ));
    }

    #[tokio::test]
    async fn windowing_orders_are_delivered_to_the_output_consumer() {
        let (output_sender, mut output_receiver) = crate::output_channel::output_channel(1);
        let (_close_sender, mut close_receiver) = watch::channel(false);

        assert!(
            send_active_output_event(
                &output_sender,
                RdpOutputEvent::WindowingOrders(vec![0, 0, 1, 2, 3]),
                &mut close_receiver,
            )
            .await
            .expect("deliver windowing orders")
        );
        assert!(matches!(
            output_receiver.recv().await,
            Some(RdpOutputEvent::WindowingOrders(data)) if data == [0, 0, 1, 2, 3]
        ));
    }

    #[tokio::test]
    async fn local_rail_execute_failure_is_delivered_without_terminating_the_session() {
        let (output_sender, mut output_receiver) = crate::output_channel::output_channel(1);
        let (_close_sender, mut close_receiver) = watch::channel(false);

        assert!(
            send_active_output_event(
                &output_sender,
                RdpOutputEvent::RailExecuteFailed {
                    executable: "notepad.exe".to_owned(),
                    flags: 0,
                    reason: RailExecuteFailureReason::QueueRejected,
                },
                &mut close_receiver,
            )
            .await
            .expect("deliver local RAIL Execute failure")
        );
        assert!(matches!(
            output_receiver.recv().await,
            Some(RdpOutputEvent::RailExecuteFailed {
                executable,
                flags: 0,
                reason: RailExecuteFailureReason::QueueRejected,
            }) if executable == "notepad.exe"
        ));
    }

    #[tokio::test]
    async fn output_send_is_cancelled_when_the_consumer_is_backpressured() {
        let (output_sender, _output_receiver) = crate::output_channel::output_channel(1);
        output_sender
            .try_send(RdpOutputEvent::Connected)
            .expect("the first output event fills the queue");
        let (close_sender, mut close_receiver) = watch::channel(false);

        let send = send_cancellable_output_event(&output_sender, RdpOutputEvent::LoginComplete, &mut close_receiver);
        let close = async {
            tokio::task::yield_now().await;
            close_sender.send_replace(true);
        };
        let (delivered, ()) = tokio::join!(send, close);

        assert!(!delivered.expect("cancellation is not an output error"));
    }

    #[cfg(all(feature = "sound", feature = "rdpdr"))]
    #[test]
    fn rdpsnd_backend_kind_prefers_playback_over_noop() {
        assert_eq!(rdpsnd_backend_kind(true, true), Some(RdpsndBackendKind::Playback));
        assert_eq!(rdpsnd_backend_kind(true, false), Some(RdpsndBackendKind::Playback));
        assert_eq!(rdpsnd_backend_kind(false, true), Some(RdpsndBackendKind::Noop));
        assert_eq!(rdpsnd_backend_kind(false, false), None);
    }

    #[cfg(all(feature = "sound", not(feature = "rdpdr")))]
    #[test]
    fn rdpsnd_backend_kind_playback_only_without_rdpdr() {
        assert_eq!(rdpsnd_backend_kind(true, false), Some(RdpsndBackendKind::Playback));
        assert_eq!(rdpsnd_backend_kind(false, false), None);
    }
}
