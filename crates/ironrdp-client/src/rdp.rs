use core::net::SocketAddr;
use core::num::NonZeroU16;
use core::time::Duration;
use std::sync::Arc;

#[cfg(feature = "clipboard")]
pub use ironrdp_cliprdr::backend::CliprdrBackendFactory;
use ironrdp_connector::connection_activation::ConnectionActivationState;
use ironrdp_connector::{ConnectionResult, ConnectorResult};
use ironrdp_core::WriteBuf;
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_displaycontrol::pdu::MonitorLayoutEntry;
#[cfg(all(windows, feature = "dvc-com-plugin"))]
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_echo::client::EchoClient;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::input::MousePdu;
use ironrdp_pdu::input::fast_path::FastPathInputEvent;
use ironrdp_pdu::input::mouse::PointerFlags;
#[cfg(any(feature = "dvc-pipe-proxy", all(windows, feature = "dvc-com-plugin")))]
use ironrdp_pdu::pdu_other_err;
#[cfg(feature = "rdpdr")]
pub use ironrdp_rdpdr::backend::{RdpdrBackendFactory, RdpdrBackendFactoryResult, RdpdrBackendProduct, RdpdrDrive};
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput, GracefulDisconnectReason, SessionResult};
use ironrdp_svc::SvcMessage;
use ironrdp_tokio::reqwest::ReqwestNetworkClient;
use ironrdp_tokio::{FramedWrite, single_sequence_step_read, split_tokio_framed};
use smallvec::SmallVec;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
#[cfg(any(feature = "clipboard", all(windows, feature = "dvc-com-plugin")))]
use tracing::error;
use tracing::{debug, info, trace, warn};

#[cfg(feature = "clipboard")]
use crate::config::ClipboardType;
#[cfg(feature = "clipboard")]
use ironrdp_cliprdr::backend::ClipboardMessage;
#[cfg(all(windows, feature = "dvc-com-plugin"))]
use ironrdp_dvc_com_plugin::load_dvc_plugin;
#[cfg(feature = "dvc-pipe-proxy")]
use ironrdp_dvc_pipe_proxy::DvcNamedPipeProxy;
#[cfg(feature = "sound")]
use ironrdp_rdpsnd_native::cpal;

use crate::config::{Config, RDCleanPathConfig, Transport};

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

#[derive(Debug)]
pub enum RdpOutputEvent {
    /// Connection negotiation and activation have completed.
    Connected,
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
    /// A full-desktop redraw was requested after the initial logon notification.
    PostLogonDisplayRedraw,
    /// A malformed bitmap update was discarded and a capability-gated full redraw was sent.
    MalformedBitmapDisplayRedraw,
    /// Dynamic Display Control could not update the session in place.
    ///
    /// The next connection attempt uses the requested desktop size.
    DisplayResizeFallback(DisplayResizeFallbackReason),
    Terminated(SessionResult<GracefulDisconnectReason>),
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
    output_event_sender: mpsc::Sender<RdpOutputEvent>,
    input_event_sender: RdpInputSender,
    input_event_receiver: mpsc::Receiver<RdpInputEvent>,
    clipboard_event_receiver: mpsc::UnboundedReceiver<RdpInputEvent>,
    close_receiver: watch::Receiver<bool>,
    graceful_close_receiver: watch::Receiver<bool>,
    #[cfg(feature = "clipboard")]
    cliprdr_backend_factory: Option<Box<dyn CliprdrBackendFactory + Send>>,
    #[cfg(feature = "rdpdr")]
    rdpdr_backend_factory: Option<Box<dyn RdpdrBackendFactory + Send>>,
}

impl RdpClient {
    pub fn new(config: Config, output_event_sender: mpsc::Sender<RdpOutputEvent>) -> Self {
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
        loop {
            if *self.close_receiver.borrow_and_update() {
                self.emit_user_initiated_termination();
                break;
            }

            let (connection_result, framed) = match &self.config.transport {
                Transport::Direct => match Box::pin(cancelable_operation(
                    connect_direct(&self.config, &self.input_event_sender, cliprdr_factory, rdpdr_factory),
                    &mut self.close_receiver,
                ))
                .await
                {
                    Some(Ok(result)) => result,
                    Some(Err(error)) => {
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
                Transport::Gateway(gw) => match Box::pin(cancelable_operation(
                    connect_gateway(
                        &self.config,
                        gw,
                        &self.input_event_sender,
                        cliprdr_factory,
                        rdpdr_factory,
                    ),
                    &mut self.close_receiver,
                ))
                .await
                {
                    Some(Ok(result)) => result,
                    Some(Err(error)) => {
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

                Transport::RDCleanPath(rdcp) => match Box::pin(cancelable_operation(
                    connect_rdcleanpath_transport(
                        &self.config,
                        rdcp,
                        &self.input_event_sender,
                        cliprdr_factory,
                        rdpdr_factory,
                    ),
                    &mut self.close_receiver,
                ))
                .await
                {
                    Some(Ok(result)) => result,
                    Some(Err(error)) => {
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
                        cliprdr_factory,
                        rdpdr_factory,
                    ),
                    &mut self.close_receiver,
                ))
                .await
                {
                    Some(Ok(result)) => result,
                    Some(Err(error)) => {
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

            if !self.send_output_event(RdpOutputEvent::Connected).await {
                self.emit_user_initiated_termination();
                break;
            }

            match active_session(
                framed,
                connection_result,
                &self.output_event_sender,
                &mut self.input_event_receiver,
                &mut self.clipboard_event_receiver,
                &mut self.close_receiver,
                &mut self.graceful_close_receiver,
                self.config.fake_events_interval,
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
                    self.config.connector.desktop_size.width = width;
                    self.config.connector.desktop_size.height = height;
                }
                Ok(RdpControlFlow::TerminatedGracefully(reason)) => {
                    if !self.send_output_event(RdpOutputEvent::Terminated(Ok(reason))).await {
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

    fn emit_user_initiated_termination(&self) {
        let _ = self
            .output_event_sender
            .try_send(RdpOutputEvent::Terminated(Ok(GracefulDisconnectReason::UserInitiated)));
    }
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

async fn send_cancellable_output_event(
    output_event_sender: &mpsc::Sender<RdpOutputEvent>,
    event: RdpOutputEvent,
    close_receiver: &mut watch::Receiver<bool>,
) -> Result<bool, mpsc::error::SendError<RdpOutputEvent>> {
    match cancelable_operation(output_event_sender.send(event), close_receiver).await {
        Some(result) => result.map(|()| true),
        None => Ok(false),
    }
}

async fn send_active_output_event(
    output_event_sender: &mpsc::Sender<RdpOutputEvent>,
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
) -> ConnectorResult<Option<ironrdp_rdpdr::Rdpdr>> {
    if !config.enabled {
        return Ok(None);
    }
    let Some(factory) = factory else {
        return Ok(None);
    };

    let (backend, initial_drives) = factory
        .build_rdpdr_backend()
        .map_err(|error| ironrdp_connector::custom_err!("build RDPDR backend", RdpdrBackendBuildError(error)))?
        .into_parts();
    if initial_drives.is_empty() {
        return Ok(None);
    }

    let initial_drives = initial_drives
        .into_iter()
        .map(RdpdrDrive::into_parts)
        .collect::<Vec<_>>();
    let rdpdr_channel = ironrdp_rdpdr::Rdpdr::new(backend, "IronRDP".to_owned()).with_drives(Some(initial_drives));

    #[cfg(feature = "smartcard")]
    let rdpdr_channel = if config.smartcard {
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

/// Build a fully wired [`ironrdp_connector::ClientConnector`] with all feature-gated channels attached.
///
/// This helper is used by all transport paths.
/// The CLIPRDR and RDPDR backends are built here for each connection attempt.
fn build_connector(
    config: &Config,
    client_addr: SocketAddr,
    input_sender: &RdpInputSender,
    cliprdr_factory: CliprdrFactoryRef<'_>,
    rdpdr_factory: RdpdrFactoryRef<'_>,
) -> ConnectorResult<ironrdp_connector::ClientConnector> {
    // `input_sender` is only consumed by the optional DVC wirings below, and `cliprdr_factory`
    // only by the optional CLIPRDR attachment; discard them explicitly when those are compiled out.
    #[cfg(not(any(feature = "dvc-pipe-proxy", all(windows, feature = "dvc-com-plugin"))))]
    let _ = input_sender;
    #[cfg(not(feature = "clipboard"))]
    let _ = cliprdr_factory;
    #[cfg(not(feature = "rdpdr"))]
    let _ = rdpdr_factory;

    let mut drdynvc = ironrdp_dvc::DrdynvcClient::new()
        .with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new())))
        .with_dynamic_channel(EchoClient::new());

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

    // Load DVC COM plugins (Windows + dvc-com-plugin feature).
    #[cfg(all(windows, feature = "dvc-com-plugin"))]
    {
        for plugin_path in &config.dvc_plugins {
            info!(dll = %plugin_path.display(), "Loading DVC COM plugin");
            let sender_clone = input_sender.clone();
            match load_dvc_plugin(plugin_path, move || {
                let sender = sender_clone.clone();
                Box::new(move |channel_id, messages| {
                    sender
                        .try_send(RdpInputEvent::SendDvcMessages { channel_id, messages })
                        .map_err(|_| pdu_other_err!("send COM DVC messages to the event loop"))?;
                    Ok(())
                })
            }) {
                Ok(channels) => {
                    for channel in channels {
                        info!(channel_name = %channel.channel_name(), "Registered COM DVC channel");
                        drdynvc = drdynvc.with_dynamic_channel(channel);
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

    // Clone the connector config so we can apply runtime overrides before handing it to the
    // connector.  We want to set `enable_audio_playback` consistently with `channels.sound`.
    let mut connector_config = config.connector.clone();

    // If sound is disabled at runtime (or the feature is off) ensure the connector doesn't
    // advertise audio support, which would confuse the server.
    #[cfg(not(feature = "sound"))]
    {
        connector_config.enable_audio_playback = false;
    }
    #[cfg(feature = "sound")]
    if !config.channels.sound {
        connector_config.enable_audio_playback = false;
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

    let mut connector =
        ironrdp_connector::ClientConnector::new(connector_config, client_addr).with_static_channel(drdynvc);

    #[cfg(feature = "rdpdr")]
    let rdpdr_channel = build_rdpdr_channel(rdpdr_factory, &config.channels.rdpdr)?;

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
                    connector = connector.with_static_channel(ironrdp_rdpsnd::client::Rdpsnd::new(Box::new(
                        cpal::RdpsndBackend::new(),
                    )));
                }
                #[cfg(feature = "rdpdr")]
                RdpsndBackendKind::Noop => {
                    connector = connector.with_static_channel(ironrdp_rdpsnd::client::Rdpsnd::new(Box::new(
                        ironrdp_rdpsnd::client::NoopRdpsndBackend,
                    )));
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

    Ok(connector)
}

// ── Transport-specific connect helpers ────────────────────────────────────────

trait AsyncReadWrite: AsyncRead + AsyncWrite {}
impl<T> AsyncReadWrite for T where T: AsyncRead + AsyncWrite {}
type UpgradedFramed = ironrdp_tokio::TokioFramed<Box<dyn AsyncReadWrite + Unpin + Send + Sync>>;

/// Direct TCP → TLS connection (no gateway).
async fn connect_direct(
    config: &Config,
    input_sender: &RdpInputSender,
    cliprdr_factory: CliprdrFactoryRef<'_>,
    rdpdr_factory: RdpdrFactoryRef<'_>,
) -> ConnectorResult<(ConnectionResult, UpgradedFramed)> {
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

    let connector = build_connector(config, client_addr, input_sender, cliprdr_factory, rdpdr_factory)?;
    #[cfg(feature = "vmconnect")]
    if config.vm_id().is_some() {
        return vmconnect_handshake_and_finalize(framed, connector, config, pcb_deadline).await;
    }
    security_upgrade_and_finalize(framed, connector, config).await
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
    cliprdr_factory: CliprdrFactoryRef<'_>,
    rdpdr_factory: RdpdrFactoryRef<'_>,
) -> ConnectorResult<(ConnectionResult, UpgradedFramed)> {
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
    let connector = build_connector(config, client_addr, input_sender, cliprdr_factory, rdpdr_factory)?;

    security_upgrade_and_finalize(framed, connector, config).await
}

/// RDS gateway TCP → gateway auth → TLS connection.
#[cfg(feature = "gateway")]
async fn connect_gateway(
    config: &Config,
    gw: &crate::config::GatewayConfig,
    input_sender: &RdpInputSender,
    cliprdr_factory: CliprdrFactoryRef<'_>,
    rdpdr_factory: RdpdrFactoryRef<'_>,
) -> ConnectorResult<(ConnectionResult, UpgradedFramed)> {
    use ironrdp_mstsgu::GwConnectTarget;

    // VMConnect needs destination port 2179; GwConnectTarget does not carry it yet (TODO below).
    #[cfg(feature = "vmconnect")]
    if config.vm_id().is_some() {
        return Err(ironrdp_connector::general_err!(
            "vmconnect cannot be used over an RDS gateway until the target port is propagated"
        ));
    }

    // Build the GwConnectTarget.  `server` is the RDP target derived from `config.destination`.
    // TODO: preserve the destination port; ironrdp-mstsgu may currently hard-code 3389.
    let gw_target = GwConnectTarget {
        gw_endpoint: gw.endpoint.clone(),
        gw_user: gw.username.clone(),
        gw_pass: gw.password.clone(),
        server: config.destination.name().to_owned(),
    };

    let (gw_stream, client_addr) = ironrdp_mstsgu::GwClient::connect(&gw_target, &config.connector.client_name)
        .await
        .map_err(|e| ironrdp_connector::custom_err!("GW connect", e))?;

    let framed = ironrdp_tokio::TokioFramed::new(gw_stream);

    let connector = build_connector(config, client_addr, input_sender, cliprdr_factory, rdpdr_factory)?;
    security_upgrade_and_finalize(framed, connector, config).await
}

/// RDCleanPath WebSocket → RDCleanPath handshake connection.
async fn connect_rdcleanpath_transport(
    config: &Config,
    rdcp: &RDCleanPathConfig,
    input_sender: &RdpInputSender,
    cliprdr_factory: CliprdrFactoryRef<'_>,
    rdpdr_factory: RdpdrFactoryRef<'_>,
) -> ConnectorResult<(ConnectionResult, UpgradedFramed)> {
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

    let mut connector = build_connector(config, client_addr, input_sender, cliprdr_factory, rdpdr_factory)?;

    let destination = config.destination.to_string();
    let mut network_client = ReqwestNetworkClient::new();
    let server_name = ironrdp_connector::ServerName::from(&config.destination);
    let (upgraded, server_public_key) = rdcleanpath_handshake(
        &mut framed,
        &mut connector,
        &mut network_client,
        server_name.clone(),
        destination,
        rdcp.auth_token.clone(),
        config.vm_id().zip(config.vmconnect_mode()),
        config.kerberos_config.clone(),
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

    Ok((connection_result, upgraded_framed))
}

// ── Shared security upgrade + finalize ────────────────────────────────────────

/// After X.224 negotiation, either perform TLS (enhanced security) or mark a no-op upgrade
/// for standard RDP security / plain local transports (Windows Sandbox named pipe).
async fn security_upgrade_and_finalize<S>(
    mut framed: ironrdp_tokio::TokioFramed<S>,
    mut connector: ironrdp_connector::ClientConnector,
    config: &Config,
) -> ConnectorResult<(ConnectionResult, UpgradedFramed)>
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

        return Ok((connection_result, upgraded_framed));
    }

    debug!("TLS upgrade");

    let (initial_stream, leftover_bytes) = framed.into_inner();

    let tls_upgrade = if let Some(callback) = config.certificate_validation_callback() {
        ironrdp_tls::upgrade_with_certificate_validation_callback(
            initial_stream,
            config.destination.name(),
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

    Ok((connection_result, upgraded_framed))
}

/// Hyper-V console connect via ironrdp-vmconnect, then shared RDP tail.
#[cfg(feature = "vmconnect")]
async fn vmconnect_handshake_and_finalize<S>(
    mut framed: ironrdp_tokio::TokioFramed<S>,
    mut connector: ironrdp_connector::ClientConnector,
    config: &Config,
    pcb_deadline: tokio::time::Instant,
) -> ConnectorResult<(ConnectionResult, UpgradedFramed)>
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

    Ok((connection_result, upgraded_framed))
}

// ── RDCleanPath handshake ─────────────────────────────────────────────────────

async fn rdcleanpath_handshake<S, N>(
    framed: &mut ironrdp_tokio::Framed<S>,
    connector: &mut ironrdp_connector::ClientConnector,
    network_client: &mut N,
    server_name: ironrdp_connector::ServerName,
    destination: String,
    proxy_auth_token: String,
    vmconnect: Option<(&str, ironrdp_vmconnect::Mode)>,
    kerberos_config: Option<ironrdp_connector::credssp::KerberosConfig>,
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

    let mut buf = WriteBuf::new();
    info!("Begin RDCleanPath connection procedure");

    {
        let rdcleanpath_req = if let Some((vm_id, mode)) = vmconnect {
            let preconnection_blob = ironrdp_vmconnect::encode_preconnection_blob_string(vm_id, mode)?;
            Ok(ironrdp_rdcleanpath::RDCleanPathPdu::new_request_with_pcb(
                destination,
                proxy_auth_token,
                preconnection_blob,
            ))
        } else {
            let ironrdp_connector::ClientConnectorState::ConnectionInitiationSendRequest = connector.state else {
                return Err(ironrdp_connector::general_err!(
                    "invalid connector state (send request)"
                ));
            };
            debug_assert!(connector.next_pdu_hint().is_none());
            let written = connector.step_no_input(&mut buf)?;
            let x224_pdu_len = written.size().expect("written size");
            debug_assert_eq!(x224_pdu_len, buf.filled_len());
            let x224_pdu = buf.filled().to_vec();
            ironrdp_rdcleanpath::RDCleanPathPdu::new_request(x224_pdu, destination, proxy_auth_token, None)
        }
        .map_err(|e| ironrdp_connector::custom_err!("new RDCleanPath request", e))?;
        debug!(
            destination = ?rdcleanpath_req.destination,
            has_preconnection_blob = rdcleanpath_req.preconnection_blob.is_some(),
            has_x224_connection_pdu = rdcleanpath_req.x224_connection_pdu.is_some(),
            "Send RDCleanPath request"
        );
        let rdcleanpath_req = rdcleanpath_req
            .to_der()
            .map_err(|e| ironrdp_connector::custom_err!("RDCleanPath request encode", e))?;
        framed
            .write_all(&rdcleanpath_req)
            .await
            .map_err(|e| ironrdp_connector::custom_err!("couldn't write RDCleanPath request", e))?;
    }

    {
        let rdcleanpath_res = framed
            .read_by_hint(&RDCLEANPATH_HINT)
            .await
            .map_err(|e| ironrdp_connector::custom_err!("read RDCleanPath response", e))?
        ;
        let rdcleanpath_res = ironrdp_rdcleanpath::RDCleanPathPdu::from_der(&rdcleanpath_res)
            .map_err(|e| ironrdp_connector::custom_err!("RDCleanPath response decode", e))?
        ;
        debug!(message = ?rdcleanpath_res, "Received RDCleanPath PDU");

        let (x224_connection_response, server_cert_chain) = match rdcleanpath_res
            .into_enum()
            .map_err(|e| ironrdp_connector::custom_err!("invalid RDCleanPath PDU", e))?
        {
            ironrdp_rdcleanpath::RDCleanPath::Request { .. } => {
                return Err(ironrdp_connector::general_err!(
                    "received unexpected RDCleanPath type (request)"
                ));
            }
            ironrdp_rdcleanpath::RDCleanPath::Response {
                x224_connection_response,
                server_cert_chain,
                server_addr: _,
            } => (x224_connection_response, server_cert_chain),
            ironrdp_rdcleanpath::RDCleanPath::GeneralErr(error) => {
                return Err(ironrdp_connector::custom_err!("received RDCleanPath error", error));
            }
            ironrdp_rdcleanpath::RDCleanPath::NegotiationErr {
                x224_connection_response,
            } => {
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

        let upgraded = if let Some(x224_connection_response) = x224_connection_response {
            let ironrdp_connector::ClientConnectorState::ConnectionInitiationWaitConfirm { .. } = connector.state else {
                return Err(ironrdp_connector::general_err!(
                    "invalid connector state (wait confirm)"
                ));
            };
            debug_assert!(connector.next_pdu_hint().is_some());

            buf.clear();
            let written = connector.step(x224_connection_response.as_bytes(), &mut buf)?;
            debug_assert!(written.is_nothing());

            let should_upgrade = ironrdp_tokio::skip_connect_begin(connector);
            ironrdp_tokio::mark_as_upgraded(should_upgrade, connector)
        } else {
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
        };

        Ok((upgraded, server_public_key))
    }
}

// ── Active session ────────────────────────────────────────────────────────────

enum RdpControlFlow {
    ReconnectWithNewSize {
        width: u16,
        height: u16,
        reason: DisplayResizeFallbackReason,
    },
    TerminatedGracefully(GracefulDisconnectReason),
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

#[expect(
    clippy::too_many_arguments,
    reason = "the active loop owns independent transport, input, clipboard, and cancellation sources"
)]
async fn active_session(
    framed: UpgradedFramed,
    connection_result: ConnectionResult,
    output_event_sender: &mpsc::Sender<RdpOutputEvent>,
    input_event_receiver: &mut mpsc::Receiver<RdpInputEvent>,
    clipboard_event_receiver: &mut mpsc::UnboundedReceiver<RdpInputEvent>,
    close_receiver: &mut watch::Receiver<bool>,
    graceful_close_receiver: &mut watch::Receiver<bool>,
    fake_events_interval: Option<Duration>,
) -> SessionResult<RdpControlFlow> {
    let (mut reader, mut writer) = split_tokio_framed(framed);
    let desktop_size = connection_result.desktop_size;
    let mut refresh_rect_support = connection_result.refresh_rect_support;
    let mut suppress_output_support = connection_result.suppress_output_support;
    let mut image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);

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

    // Timer interval for driving clipboard lock timeouts.
    let mut cleanup_interval = tokio::time::interval(Duration::from_secs(5));
    #[cfg(feature = "rdpdr")]
    let mut rdpdr_deferred_interval = tokio::time::interval(Duration::from_millis(50));

    // Anti-idle: track the time of the last real input and the last known mouse position so we can
    // synthesize a no-op mouse move when the session has been idle for too long. Default to the
    // middle of the screen so a synthetic move before any real input doesn't snap the pointer to a
    // corner.
    let mut last_input = tokio::time::Instant::now();
    let mut last_mouse_pos = (desktop_size.width / 2, desktop_size.height / 2);
    let mut fake_events_interval =
        fake_events_interval.map(|interval| tokio::time::interval(core::cmp::max(interval, Duration::from_secs(1))));
    let mut resize_queue = ResizeQueue::default();
    let mut graceful_shutdown_sent = false;
    let mut post_logon_redraw_requested = false;
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
        let outputs = if let Some(outputs) = initial_outputs.take() {
            outputs
        } else {
            tokio::select! {
                _ = close_receiver.changed() => {
                    break 'outer GracefulDisconnectReason::UserInitiated;
                }
                _ = graceful_close_receiver.changed() => {
                    if *graceful_close_receiver.borrow_and_update() && !graceful_shutdown_sent {
                        graceful_shutdown_sent = true;
                        active_stage.graceful_shutdown()?
                    } else {
                        Vec::new()
                    }
                }
                frame = reader.read_pdu() => {
                    let (action, payload) = frame.map_err(|e| ironrdp_session::custom_err!("read frame", e))?;
                    trace!(?action, frame_length = payload.len(), "Frame received");
                    let mut outputs = active_stage.process(&mut image, action, &payload)?;
                    #[cfg(feature = "rdpdr")]
                    if let Some(output) = poll_deferred_rdpdr_output(&mut active_stage)? {
                        outputs.push(output);
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
                    outputs
                }
                clipboard_event = clipboard_event => {
                    #[cfg(feature = "clipboard")]
                    {
                        let Some(RdpInputEvent::Clipboard(event)) = clipboard_event else {
                            return Err(ironrdp_session::general_err!("clipboard event channel closed"));
                        };
                        process_clipboard_message(&mut active_stage, event)?
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
                            Vec::new()
                        } else if let Some(response_frame) = active_stage.encode_resize(
                            u32::from(request.width),
                            u32::from(request.height),
                            Some(request.scale_factor),
                            request.physical_size,
                        ) {
                            resize_queue.mark_in_flight(request);
                            vec![ActiveStageOutput::ResponseFrame(response_frame?)]
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
                        active_stage.process_fastpath_input(&mut image, &events)?
                    }
                    RdpInputEvent::Close => {
                        active_stage.graceful_shutdown()?
                    }
                    #[cfg(feature = "clipboard")]
                    RdpInputEvent::Clipboard(event) => {
                        process_clipboard_message(&mut active_stage, event)?
                    }
                    RdpInputEvent::SendDvcMessages { channel_id, messages } => {
                        trace!(channel_id, ?messages, "Send DVC messages");
                        let frame = active_stage.encode_dvc_messages(messages)?;
                        vec![ActiveStageOutput::ResponseFrame(frame)]
                    }
                    RdpInputEvent::SendStaticChannelData { channel_name, data } => {
                        match active_stage.process_svc_messages_by_name(&channel_name, vec![SvcMessage::from(data)]) {
                            Ok(frame) => vec![ActiveStageOutput::ResponseFrame(frame)],
                            Err(error) => {
                                warn!(?channel_name, %error, "Unable to send static channel data");
                                Vec::new()
                            }
                        }
                    }
                    }
                }
                _ = cleanup_interval.tick() => {
                // Drive clipboard lock timeout cleanup.
                #[cfg(feature = "clipboard")]
                if let Some(cliprdr_client) = active_stage.get_svc_processor_mut::<ironrdp_cliprdr::CliprdrClient>() {
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
                }
                #[cfg(not(feature = "clipboard"))]
                Vec::new()
                }
                _ = rdpdr_deferred => {
                    #[cfg(feature = "rdpdr")]
                    {
                        poll_deferred_rdpdr_output(&mut active_stage)?.into_iter().collect()
                    }
                    #[cfg(not(feature = "rdpdr"))]
                    {
                        Vec::new()
                    }
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
                _ = async { match fake_events_interval.as_mut() {
                Some(interval) => interval.tick().await,
                None => core::future::pending().await,
                }} => {
                // Anti-idle: synthesize a no-op mouse move if the session has been idle for at least
                // the configured interval, keeping the connection alive without user interaction.
                if last_input.elapsed() >= fake_events_interval.as_ref().map_or(Duration::MAX, |i| i.period()) {
                    last_input = tokio::time::Instant::now();
                    let mut events = SmallVec::<[FastPathInputEvent; 2]>::new();
                    events.push(FastPathInputEvent::MouseEvent(MousePdu {
                        flags: PointerFlags::MOVE,
                        number_of_wheel_rotation_units: 0,
                        x_position: last_mouse_pos.0,
                        y_position: last_mouse_pos.1,
                    }));
                    active_stage.process_fastpath_input(&mut image, &events)?
                } else {
                    Vec::new()
                }
                }
            }
        };

        for out in outputs {
            match out {
                ActiveStageOutput::AutoReconnectCookie(_cookie) => {
                    // The connector can now return this to the server via
                    // `with_auto_reconnect_cookie`. Holding it across a dropped
                    // connection and reconnecting with it is the remaining part of
                    // #271; see the TODO at the reconnect site.
                    debug!("Received a Server Auto-Reconnect Cookie");
                }
                ActiveStageOutput::ResponseFrame(frame) => {
                    let Some(result) = cancelable_operation(writer.write_all(&frame), close_receiver).await else {
                        return Ok(RdpControlFlow::TerminatedGracefully(
                            GracefulDisconnectReason::UserInitiated,
                        ));
                    };
                    result.map_err(|e| ironrdp_session::custom_err!("write response", e))?;
                }
                ActiveStageOutput::GraphicsUpdate(_region) => {
                    let buffer: Vec<u32> = image
                        .data()
                        .chunks_exact(4)
                        .map(|pixel| {
                            let r = pixel[0];
                            let g = pixel[1];
                            let b = pixel[2];
                            u32::from_be_bytes([0, r, g, b])
                        })
                        .collect();
                    if !send_active_output_event(
                        output_event_sender,
                        RdpOutputEvent::Image {
                            buffer,
                            width: NonZeroU16::new(image.width())
                                .ok_or_else(|| ironrdp_session::general_err!("width is zero"))?,
                            height: NonZeroU16::new(image.height())
                                .ok_or_else(|| ironrdp_session::general_err!("height is zero"))?,
                        },
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
                            result.map_err(|error| {
                                ironrdp_session::custom_err!("write post-logon redraw request", error)
                            })?;
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
                        }
                        .map_err(|e| ironrdp_session::custom_err!("read deactivation-reactivation sequence step", e))?;
                        if written.size().is_some() {
                            let Some(result) =
                                cancelable_operation(writer.write_all(buf.filled()), close_receiver).await
                            else {
                                return Ok(RdpControlFlow::TerminatedGracefully(
                                    GracefulDisconnectReason::UserInitiated,
                                ));
                            };
                            result.map_err(|e| {
                                ironrdp_session::custom_err!("write deactivation-reactivation sequence step", e)
                            })?;
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
                        } = connection_activation.connection_activation_state()
                        {
                            debug!(?desktop_size, "Deactivation-Reactivation Sequence completed");
                            image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
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
                            refresh_rect_support = reactivated_refresh_rect_support;
                            suppress_output_support = reactivated_suppress_output_support;
                            break 'activation_seq;
                        }
                    }
                }
                ActiveStageOutput::MultitransportRequest(pdu) => {
                    debug!(
                        request_id = pdu.request_id,
                        requested_protocol = ?pdu.requested_protocol,
                        "Multitransport request received (UDP transport not implemented)"
                    );
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

        if resize_queue.in_flight.is_none()
            && let Some(pending) = resize_queue.pending.as_ref()
        {
            let request = pending.request;
            match active_stage.display_control_ready() {
                Some(true) => {
                    let response_frame = active_stage
                        .encode_resize(
                            u32::from(request.width),
                            u32::from(request.height),
                            Some(request.scale_factor),
                            request.physical_size,
                        )
                        .ok_or_else(|| ironrdp_session::general_err!("Display Control became unavailable"))??;
                    resize_queue.pending = None;
                    resize_queue.mark_in_flight(request);
                    let Some(result) = cancelable_operation(writer.write_all(&response_frame), close_receiver).await
                    else {
                        return Ok(RdpControlFlow::TerminatedGracefully(
                            GracefulDisconnectReason::UserInitiated,
                        ));
                    };
                    result.map_err(|e| ironrdp_session::custom_err!("write pending resize", e))?;
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
    use core::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(feature = "rdpdr")]
    use ironrdp_core::encode_vec;
    #[cfg(feature = "rdpdr")]
    use ironrdp_rdpdr::RdpdrBackend;
    #[cfg(feature = "rdpdr")]
    use ironrdp_rdpdr::pdu::RdpdrPdu;
    #[cfg(feature = "rdpdr")]
    use ironrdp_rdpdr::pdu::efs::{
        DeviceControlRequest, ServerDeviceAnnounceResponse, ServerDriveIoRequest, VERSION_MINOR_12, VersionAndIdPdu,
        VersionAndIdPduKind,
    };
    #[cfg(feature = "rdpdr")]
    use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
    #[cfg(feature = "rdpdr")]
    use ironrdp_svc::{StaticChannelSet, SvcProcessor as _};

    #[cfg(feature = "rdpdr")]
    #[derive(Debug)]
    struct TestRdpdrBackend {
        instance: usize,
        deferred_messages: Vec<SvcMessage>,
    }

    #[cfg(feature = "rdpdr")]
    impl TestRdpdrBackend {
        fn new(instance: usize) -> Self {
            Self {
                instance,
                deferred_messages: Vec::new(),
            }
        }

        fn with_deferred_message() -> Self {
            Self {
                instance: 0,
                deferred_messages: vec![SvcMessage::from(RdpdrPdu::EmptyResponse)],
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
        ) -> ironrdp_pdu::PduResult<()> {
            Ok(())
        }

        fn handle_drive_io_request(&mut self, _req: ServerDriveIoRequest) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
            Ok(Vec::new())
        }

        fn poll_deferred_messages(&mut self) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
            Ok(core::mem::take(&mut self.deferred_messages))
        }
    }

    #[cfg(feature = "rdpdr")]
    #[derive(Debug)]
    struct CountingRdpdrFactory {
        builds: AtomicUsize,
        initial_drives: Vec<RdpdrDrive>,
    }

    #[cfg(feature = "rdpdr")]
    impl CountingRdpdrFactory {
        fn new(initial_drives: Vec<RdpdrDrive>) -> Self {
            Self {
                builds: AtomicUsize::new(0),
                initial_drives,
            }
        }
    }

    #[cfg(feature = "rdpdr")]
    impl RdpdrBackendFactory for CountingRdpdrFactory {
        fn build_rdpdr_backend(&self) -> RdpdrBackendFactoryResult<RdpdrBackendProduct> {
            let instance = self.builds.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(RdpdrBackendProduct::new(
                Box::new(TestRdpdrBackend::new(instance)),
                self.initial_drives.clone(),
            ))
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

    #[test]
    fn input_sender_reservation_prevents_state_changes_without_queue_capacity() {
        let (sender, mut receiver) = RdpInputSender::channel(1);
        let permit = sender.try_reserve().expect("the empty queue has capacity");
        assert!(sender.try_reserve().is_err());

        permit.send(RdpInputEvent::Resize {
            width: 1024,
            height: 768,
            scale_factor: 100,
            physical_size: None,
        });

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
            build_rdpdr_channel(Some(&factory), &config)
                .expect("disabled RDPDR should not fail")
                .is_none()
        );
        assert_eq!(factory.builds.load(Ordering::SeqCst), 0);
    }

    #[cfg(feature = "rdpdr")]
    #[test]
    fn empty_rdpdr_product_omits_the_channel() {
        let factory = CountingRdpdrFactory::new(Vec::new());

        assert!(
            build_rdpdr_channel(Some(&factory), &crate::config::RdpdrConfig::default())
                .expect("empty RDPDR product should not fail")
                .is_none()
        );
        assert_eq!(factory.builds.load(Ordering::SeqCst), 1);
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

        let first = build_rdpdr_channel(Some(&factory), &config)
            .expect("first connection attempt should build")
            .expect("first product contains a filesystem device");
        let second = build_rdpdr_channel(Some(&factory), &config)
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
        let mut rdpdr = build_rdpdr_channel(Some(&factory), &config)
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
        let mut connector = build_connector(
            &config,
            SocketAddr::from(([127, 0, 0, 1], 0)),
            &input_sender,
            no_cliprdr_factory(),
            Some(&factory),
        )
        .expect("RDPDR connector should build");

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
            .send_clipboard(ClipboardMessage::Error(Box::new(std::io::Error::other(
                "test clipboard message",
            ))))
            .expect("clipboard messages use a dedicated queue");

        assert!(matches!(
            clipboard_receiver.try_recv(),
            Ok(RdpInputEvent::Clipboard(ClipboardMessage::Error(_)))
        ));
    }

    #[tokio::test]
    async fn output_send_is_cancelled_when_the_consumer_is_backpressured() {
        let (output_sender, _output_receiver) = mpsc::channel(1);
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
}
