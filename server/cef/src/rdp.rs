use std::net::SocketAddr;
use std::num::{NonZeroU16, NonZeroUsize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::Context as _;
use bytes::Bytes;
use ironrdp_displaycontrol::pdu::DisplayControlMonitorLayout;
use ironrdp_pdu::rdp::server_error_info::{ErrorInfo, ProtocolIndependentCode};
use ironrdp_pdu::rdp::session_info::ServerAutoReconnect;
use ironrdp_server::{
    BitmapUpdate, ConnectionHandler, ConnectionInfo, ConnectionPolicy, DesktopSize, DisplayContext, DisplayUpdate,
    KeyboardEvent, MouseEvent, PenEventPdu, PixelFormat, PostConnectionAction, RdpInputProtocolVersion, RdpServer,
    RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler, RdpeiHandler, RdpeiServer, RdpeiServerFactory,
    ScReadyFeatures, ServerError, ServerEvent, ServerEventSender, ServerResult, TouchEventPdu,
};
use tokio::sync::watch;

use crate::audio::AudioInput;
use crate::camera::CameraInput;
use crate::clipboard::Clipboard;
use crate::cursor::CursorShape;
use crate::download::runtime::Downloads;
use crate::egfx::{EgfxOutput, EgfxPresenter, PresentOutcome};
use crate::location::{LocationInput, SessionViewConfig, base_zoom_for_scale};
use crate::sound::AudioOutput;
use crate::webauthn::WebAuthnInput;

#[derive(Clone)]
pub struct Frame {
    pub width: u16,
    pub height: u16,
    pub pixels: Bytes,
}

pub struct DisplayOutput {
    pub frames: watch::Receiver<Frame>,
    pub cursors: watch::Receiver<CursorShape>,
}

pub enum Command {
    Keyboard(KeyboardEvent),
    Mouse(MouseEvent),
    Touch(TouchEventPdu),
    Pen(PenEventPdu),
    CancelTouches,
    Resize(u16, u16),
    Reset(mpsc::SyncSender<()>),
}

#[derive(Clone)]
pub(crate) struct ChannelGuard {
    active: Arc<AtomicBool>,
    connection: Arc<AtomicBool>,
}

impl ChannelGuard {
    pub(crate) fn new(connection: &Arc<AtomicBool>) -> Self {
        Self {
            active: Arc::new(AtomicBool::new(true)),
            connection: Arc::clone(connection),
        }
    }

    pub(crate) fn invalidate(&self) {
        self.active.store(false, Ordering::Release);
    }

    pub(crate) fn is_current(&self) -> bool {
        self.active.load(Ordering::Acquire) && self.connection.load(Ordering::Acquire)
    }

    pub(crate) fn is_same_channel(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.active, &other.active)
    }

    pub(crate) fn close(&self, channel_id: u32) -> ServerEvent {
        let guard = self.clone();
        ServerEvent::CloseDynamicChannel {
            channel_id,
            is_current: Box::new(move || guard.is_current()),
        }
    }
}

pub struct Runtime {
    reactor: tokio::runtime::Runtime,
    #[cfg(unix)]
    interrupt: tokio::signal::unix::Signal,
    #[cfg(windows)]
    interrupt: tokio::signal::windows::CtrlC,
    #[cfg(unix)]
    terminate: tokio::signal::unix::Signal,
    #[cfg(windows)]
    terminate: tokio::signal::windows::CtrlBreak,
}

impl Runtime {
    pub fn new() -> anyhow::Result<Self> {
        let reactor = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        // Keep the receiver across timer ticks so SIGINT cannot fall between subscriptions.
        let interrupt = {
            let _guard = reactor.enter();
            #[cfg(unix)]
            let signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
            #[cfg(windows)]
            let signal = tokio::signal::windows::ctrl_c()?;
            signal
        };
        let terminate = {
            let _guard = reactor.enter();
            #[cfg(unix)]
            let signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            #[cfg(windows)]
            let signal = tokio::signal::windows::ctrl_break()?;
            signal
        };
        Ok(Self {
            reactor,
            interrupt,
            terminate,
        })
    }
}

#[derive(Clone)]
struct Bridge {
    commands: mpsc::SyncSender<Command>,
    frames: watch::Receiver<Frame>,
    cursors: watch::Receiver<CursorShape>,
    session_view: watch::Sender<SessionViewConfig>,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    downloads: Downloads,
    clipboard: Option<Clipboard>,
    egfx: EgfxOutput,
    sound: AudioOutput,
    audio: AudioInput,
    camera: CameraInput,
    location: LocationInput,
    webauthn: WebAuthnInput,
}

impl Bridge {
    fn send(&self, command: Command) -> bool {
        // Input delivery is no longer required once shutdown starts.
        match self.commands.try_send(command) {
            Ok(()) => true,
            Err(error) if !self.stop.load(Ordering::Acquire) => {
                tracing::error!(%error, "CEF input queue failed");
                self.failed.store(true, Ordering::Release);
                self.stop.store(true, Ordering::Release);
                false
            }
            Err(_) => false,
        }
    }

    fn size(&self) -> DesktopSize {
        let frame = self.frames.borrow();
        DesktopSize {
            width: frame.width,
            height: frame.height,
        }
    }
}

impl RdpServerInputHandler for Bridge {
    fn keyboard(&mut self, event: KeyboardEvent) {
        self.send(Command::Keyboard(event));
    }

    fn mouse(&mut self, event: MouseEvent) {
        self.send(Command::Mouse(event));
    }
}

struct TouchInput {
    bridge: Bridge,
}

impl ServerEventSender for TouchInput {
    fn set_sender(&mut self, _sender: tokio::sync::mpsc::UnboundedSender<ServerEvent>) {}
}

impl RdpeiServerFactory for TouchInput {
    fn build_server(&self) -> RdpeiServer {
        RdpeiServer::new(Box::new(TouchHandler {
            bridge: self.bridge.clone(),
        }))
        .with_protocol_version(RdpInputProtocolVersion::V300)
        .with_supported_features(ScReadyFeatures::MULTIPEN_INJECTION_SUPPORTED)
    }
}

struct TouchHandler {
    bridge: Bridge,
}

impl RdpeiHandler for TouchHandler {
    fn touch(&mut self, pdu: TouchEventPdu) {
        self.bridge.send(Command::Touch(pdu));
    }

    fn pen(&mut self, pdu: PenEventPdu) {
        self.bridge.send(Command::Pen(pdu));
    }

    fn close(&mut self) {
        self.bridge.send(Command::CancelTouches);
    }
}

impl ConnectionHandler for Bridge {
    fn on_connection_info(&mut self, info: &ConnectionInfo) {
        let _ = self.session_view.send_replace(SessionViewConfig {
            timezone: info.timezone_info.clone(),
            base_zoom: base_zoom_for_scale(info.desktop_scale_factor),
        });
    }

    fn on_disconnected(
        &mut self,
        _peer: SocketAddr,
        _duration: Duration,
        _error: Option<&ServerError>,
    ) -> PostConnectionAction {
        self.egfx.disconnected();
        self.sound.disconnected();
        self.audio.disconnected();
        self.camera.disconnected();
        self.location.disconnected();
        self.webauthn.disconnected();
        let clipboard_failed = self.clipboard.as_ref().is_some_and(|clipboard| {
            if let Err(error) = clipboard.disconnected() {
                tracing::error!(%error, "Clipboard cleanup failed");
                self.stop.store(true, Ordering::Release);
                true
            } else {
                false
            }
        });
        let pending = self.downloads.disconnected();
        let reset_failed = if self.stop.load(Ordering::Acquire) {
            false
        } else {
            let (completion, reset) = mpsc::sync_channel(0);
            self.send(Command::Reset(completion)) && reset.recv_timeout(Duration::from_secs(1)).is_err()
        };
        if reset_failed {
            tracing::error!("CEF clipboard reset did not complete before the next connection");
            self.stop.store(true, Ordering::Release);
        }
        if pending {
            // RDPDR events have no connection guard; never let a queued IRP reach another client.
            // UNTESTED: Acceptance must stop before another connection can consume the queued IRP.
            // Check real mid-transfer disconnect/reconnect when changing this fallback.
            tracing::warn!("Stopping server to fence an unacknowledged download IRP");
        }
        // Ending an active connection does not stop the listener.
        if pending || clipboard_failed || reset_failed || self.stop.load(Ordering::Acquire) {
            PostConnectionAction::Stop
        } else {
            PostConnectionAction::Continue
        }
    }
}

struct Updates {
    frames: watch::Receiver<Frame>,
    cursors: watch::Receiver<CursorShape>,
    egfx: EgfxPresenter,
    size: (u16, u16),
    initial_frame: bool,
    initial_cursor: bool,
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>> {
        loop {
            if self.egfx.has_pending_write() {
                self.egfx.finish_pending_write().await;
            }
            if self.initial_frame {
                let frame = self.frames.borrow_and_update().clone();
                if self.size != (frame.width, frame.height) {
                    self.size = (frame.width, frame.height);
                    self.initial_cursor = true;
                    return Ok(Some(DisplayUpdate::Resize(DesktopSize {
                        width: frame.width,
                        height: frame.height,
                    })));
                }
                match self.egfx.present(&frame) {
                    PresentOutcome::NotReady => {
                        self.initial_frame = false;
                        return Ok(Some(DisplayUpdate::Bitmap(BitmapUpdate {
                            x: 0,
                            y: 0,
                            width: NonZeroU16::new(frame.width).expect("validated display width"),
                            height: NonZeroU16::new(frame.height).expect("validated display height"),
                            format: PixelFormat::BgrA32,
                            stride: NonZeroUsize::new(usize::from(frame.width) * 4).expect("validated display stride"),
                            data: frame.pixels,
                        })));
                    }
                    PresentOutcome::Queued => {
                        self.initial_frame = false;
                        self.egfx.finish_pending_write().await;
                    }
                    PresentOutcome::Unchanged => {
                        self.initial_frame = false;
                    }
                    PresentOutcome::Backpressured => {
                        self.initial_frame = false;
                    }
                }
            }
            if self.initial_cursor {
                self.initial_cursor = false;
                let cursor = self.cursors.borrow_and_update().clone();
                return Ok(Some(cursor.to_display_update()));
            }
            if self.cursors.has_changed().unwrap_or(false) {
                let cursor = self.cursors.borrow_and_update().clone();
                return Ok(Some(cursor.to_display_update()));
            }
            if self.frames.has_changed().unwrap_or(false) {
                self.initial_frame = true;
                continue;
            }
            if self.egfx.needs_repaint() {
                self.initial_frame = true;
                continue;
            }
            let egfx_attached = self.egfx.is_attached();
            tokio::select! {
                changed = self.cursors.changed() => {
                    if changed.is_err() {
                        return Ok(None);
                    }
                    let cursor = self.cursors.borrow_and_update().clone();
                    return Ok(Some(cursor.to_display_update()));
                }
                changed = self.frames.changed() => {
                    if changed.is_err() {
                        return Ok(None);
                    }
                    self.initial_frame = true;
                }
                () = self.egfx.notified(), if egfx_attached => {
                    if self.egfx.needs_repaint() {
                        self.initial_frame = true;
                    }
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl RdpServerDisplay for Bridge {
    async fn size(&mut self) -> DesktopSize {
        Bridge::size(self)
    }

    async fn request_initial_size(&mut self, size: DesktopSize) -> DesktopSize {
        if (320..=3840).contains(&size.width) && (240..=2160).contains(&size.height) {
            self.send(Command::Resize(size.width, size.height));
            let wait = async {
                while {
                    let frame = self.frames.borrow();
                    (frame.width, frame.height) != (size.width, size.height)
                } {
                    if self.frames.changed().await.is_err() {
                        break;
                    }
                }
            };
            if tokio::time::timeout(Duration::from_secs(5), wait).await.is_err() {
                tracing::error!("Initial CEF resize timed out");
                self.failed.store(true, Ordering::Release);
                self.stop.store(true, Ordering::Release);
            }
        } else {
            tracing::warn!(
                width = size.width,
                height = size.height,
                "Rejected unsupported initial desktop size"
            );
        }
        Bridge::size(self)
    }

    async fn updates(&mut self, context: DisplayContext) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
        self.audio.connected();
        self.camera.connected();
        self.webauthn.connected();
        let size = Bridge::size(self);
        Ok(Box::new(Updates {
            frames: self.frames.clone(),
            cursors: self.cursors.clone(),
            egfx: self.egfx.presenter(context.gfx_handle),
            size: (size.width, size.height),
            initial_frame: true,
            initial_cursor: true,
        }))
    }

    fn request_layout(&mut self, layout: DisplayControlMonitorLayout) {
        let (width, height) = layout.desktop_size();
        if layout.monitors().len() == 1 && (320..=3840).contains(&width) && (240..=2160).contains(&height) {
            self.send(Command::Resize(
                u16::try_from(width).expect("bounded width"),
                u16::try_from(height).expect("bounded height"),
            ));
        } else {
            tracing::error!(width, height, "Rejected unsupported desktop layout");
            self.failed.store(true, Ordering::Release);
            self.stop.store(true, Ordering::Release);
        }
    }
}

/// Device redirection offered to the client.
pub struct Redirection {
    pub downloads: Downloads,
    pub clipboard: Option<Clipboard>,
    pub sound: AudioOutput,
    pub audio: AudioInput,
    pub camera: CameraInput,
    pub location: LocationInput,
    pub session_view: watch::Sender<SessionViewConfig>,
    pub webauthn: WebAuthnInput,
}

pub fn start(
    runtime: Runtime,
    addr: SocketAddr,
    artifacts: PathBuf,
    commands: mpsc::SyncSender<Command>,
    display: DisplayOutput,
    stop: Arc<AtomicBool>,
    redirection: Redirection,
) -> std::thread::JoinHandle<anyhow::Result<()>> {
    std::thread::spawn(move || {
        let result = run(
            runtime,
            addr,
            artifacts,
            commands,
            display,
            Arc::clone(&stop),
            redirection,
        );
        if let Err(error) = &result {
            tracing::error!(%error, "RDP server failed");
        }
        stop.store(true, Ordering::Release);
        result
    })
}

fn run(
    runtime: Runtime,
    addr: SocketAddr,
    artifacts: PathBuf,
    commands: mpsc::SyncSender<Command>,
    display: DisplayOutput,
    stop: Arc<AtomicBool>,
    redirection: Redirection,
) -> anyhow::Result<()> {
    let DisplayOutput { frames, cursors } = display;
    let Redirection {
        downloads,
        clipboard,
        mut sound,
        mut audio,
        mut camera,
        location,
        session_view,
        mut webauthn,
    } = redirection;
    let Runtime {
        reactor,
        mut interrupt,
        mut terminate,
    } = runtime;
    reactor.block_on(async move {
        let identity = rcgen::generate_simple_self_signed(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])?;
        std::fs::write(artifacts.join("server.pem"), identity.cert.pem())?;
        let provider = Arc::new(tokio_rustls::rustls::crypto::aws_lc_rs::default_provider());
        let mut reconnect_random = [0u8; 16];
        provider
            .secure_random
            .fill(&mut reconnect_random)
            .map_err(|_| anyhow::anyhow!("failed to generate RDP auto-reconnect secret"))?;
        let config = tokio_rustls::rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(
                vec![identity.cert.der().clone()],
                tokio_rustls::rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der()).into(),
            )?;
        let mut egfx = EgfxOutput::new();
        let failed = Arc::new(AtomicBool::new(false));
        let bridge = Bridge {
            commands,
            frames,
            cursors,
            session_view,
            stop: Arc::clone(&stop),
            failed: Arc::clone(&failed),
            downloads: downloads.clone(),
            clipboard: clipboard.clone(),
            egfx: egfx.clone(),
            sound: sound.clone(),
            audio: audio.clone(),
            camera: camera.clone(),
            location: location.clone(),
            webauthn: webauthn.clone(),
        };
        let mut server = RdpServer::builder()
            .with_addr(addr)
            .with_tls(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
            .with_input_handler(bridge.clone())
            .with_display_handler(bridge.clone())
            .with_connection_handler(Some(Box::new(bridge.clone())))
            .with_connection_policy(ConnectionPolicy::Reject)
            .with_auto_reconnect_cookie(Some(ServerAutoReconnect {
                logon_id: 1,
                random_bits: reconnect_random,
            }))
            .with_gfx_factory(Some(egfx.factory()))
            .with_dynamic_channel_factory(Box::new(location))
            .with_rdpdr_factory(Some(downloads.factory()))
            .with_cliprdr_factory(clipboard.as_ref().map(Clipboard::factory))
            .with_sound_factory(sound.factory())
            .with_rdpei_factory(Some(Box::new(TouchInput { bridge })))
            .with_honor_client_desktop_size(Some(DesktopSize {
                width: 3840,
                height: 2160,
            }))
            .build();
        egfx.set_sender(server.event_sender().clone());
        sound.set_sender(server.event_sender().clone());
        audio.set_sender(server.event_sender().clone());
        camera.set_sender(server.event_sender().clone());
        webauthn.set_sender(server.event_sender().clone());
        let handle = server.handle().clone();
        let disconnect = server.error_info_disconnect_handle();
        let signal_failed = Arc::clone(&failed);
        let control = tokio::spawn(async move {
            loop {
                if stop.load(Ordering::Acquire) {
                    // Report shutdown to an active client and stop the listener if it is idle.
                    if let Err(error) = disconnect.disconnect(ErrorInfo::ProtocolIndependentCode(
                        ProtocolIndependentCode::ServerShutdown,
                    )) {
                        tracing::error!(%error, "Failed to request client disconnect");
                        signal_failed.store(true, Ordering::Release);
                    }
                    if let Err(error) = handle.quit("browser server stopped") {
                        tracing::error!(%error, "Failed to request RDP shutdown");
                        signal_failed.store(true, Ordering::Release);
                    }
                    return;
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(25)) => {}
                    _ = interrupt.recv() => { stop.store(true, Ordering::Release); }
                    _ = terminate.recv() => { stop.store(true, Ordering::Release); }
                }
                if let Some(clipboard) = &clipboard {
                    clipboard.tick();
                }
                audio.tick();
                camera.tick();
                webauthn.tick();
            }
        });
        let handle = server.handle().clone();
        let ready = async {
            let addr = handle.local_addr().await?.context("RDP listener has no address")?;
            std::fs::write(artifacts.join("server.addr"), addr.to_string())?;
            tracing::info!(%addr, "Browser RDP server ready");
            Ok::<(), anyhow::Error>(())
        };
        let serving = async { server.run().await.map_err(anyhow::Error::from) };
        let result = tokio::try_join!(serving, ready);
        control.abort();
        anyhow::ensure!(!failed.load(Ordering::Acquire), "RDP bridge failed; see the server log");
        result.map(|_| ())
    })
}
