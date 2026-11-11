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
use ironrdp_server::{
    BitmapUpdate, ConnectionHandler, ConnectionPolicy, DesktopSize, DisplayContext, DisplayUpdate, KeyboardEvent,
    MouseEvent, PixelFormat, PostConnectionAction, RdpServer, RdpServerDisplay, RdpServerDisplayUpdates,
    RdpServerInputHandler, ServerError, ServerResult,
};
use tokio::sync::watch;

use crate::download::runtime::Downloads;

#[derive(Clone)]
pub struct Frame {
    pub width: u16,
    pub height: u16,
    pub pixels: Bytes,
}

pub enum Command {
    Keyboard(KeyboardEvent),
    Mouse(MouseEvent),
    Resize(u16, u16),
    Reset,
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
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    downloads: Downloads,
}

impl Bridge {
    fn send(&self, command: Command) {
        // Input delivery is no longer required once shutdown starts.
        if let Err(error) = self.commands.try_send(command)
            && !self.stop.load(Ordering::Acquire)
        {
            tracing::error!(%error, "CEF input queue failed");
            self.failed.store(true, Ordering::Release);
            self.stop.store(true, Ordering::Release);
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

impl ConnectionHandler for Bridge {
    fn on_disconnected(
        &mut self,
        _peer: SocketAddr,
        _duration: Duration,
        _error: Option<&ServerError>,
    ) -> PostConnectionAction {
        let pending = self.downloads.disconnected();
        if !self.stop.load(Ordering::Acquire) {
            self.send(Command::Reset);
        }
        if pending {
            // RDPDR events have no connection guard; never let a queued IRP reach another client.
            // UNTESTED: Acceptance must stop before another connection can consume the queued IRP.
            // Check real mid-transfer disconnect/reconnect when changing this fallback.
            tracing::warn!("Stopping server to fence an unacknowledged download IRP");
        }
        // Ending an active connection does not stop the listener.
        if pending || self.stop.load(Ordering::Acquire) {
            PostConnectionAction::Stop
        } else {
            PostConnectionAction::Continue
        }
    }
}

struct Updates {
    frames: watch::Receiver<Frame>,
    size: (u16, u16),
    initial: bool,
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>> {
        if !self.initial && self.frames.changed().await.is_err() {
            return Ok(None);
        }
        let frame = self.frames.borrow_and_update().clone();
        if self.size != (frame.width, frame.height) {
            self.size = (frame.width, frame.height);
            self.initial = true;
            return Ok(Some(DisplayUpdate::Resize(DesktopSize {
                width: frame.width,
                height: frame.height,
            })));
        }
        self.initial = false;
        Ok(Some(DisplayUpdate::Bitmap(BitmapUpdate {
            x: 0,
            y: 0,
            width: NonZeroU16::new(frame.width).expect("validated display width"),
            height: NonZeroU16::new(frame.height).expect("validated display height"),
            format: PixelFormat::BgrA32,
            stride: NonZeroUsize::new(usize::from(frame.width) * 4).expect("validated display stride"),
            data: frame.pixels,
        })))
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

    async fn updates(&mut self, _context: DisplayContext) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
        let size = Bridge::size(self);
        Ok(Box::new(Updates {
            frames: self.frames.clone(),
            size: (size.width, size.height),
            initial: true,
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

pub fn start(
    runtime: Runtime,
    addr: SocketAddr,
    artifacts: PathBuf,
    commands: mpsc::SyncSender<Command>,
    frames: watch::Receiver<Frame>,
    stop: Arc<AtomicBool>,
    downloads: Downloads,
) -> std::thread::JoinHandle<anyhow::Result<()>> {
    std::thread::spawn(move || {
        let result = run(runtime, addr, artifacts, commands, frames, Arc::clone(&stop), downloads);
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
    frames: watch::Receiver<Frame>,
    stop: Arc<AtomicBool>,
    downloads: Downloads,
) -> anyhow::Result<()> {
    let Runtime {
        reactor,
        mut interrupt,
        mut terminate,
    } = runtime;
    reactor.block_on(async move {
        let identity = rcgen::generate_simple_self_signed(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])?;
        std::fs::write(artifacts.join("server.pem"), identity.cert.pem())?;
        let config = tokio_rustls::rustls::ServerConfig::builder_with_provider(Arc::new(
            tokio_rustls::rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(
            vec![identity.cert.der().clone()],
            tokio_rustls::rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der()).into(),
        )?;
        let failed = Arc::new(AtomicBool::new(false));
        let bridge = Bridge {
            commands,
            frames,
            stop: Arc::clone(&stop),
            failed: Arc::clone(&failed),
            downloads: downloads.clone(),
        };
        let mut server = RdpServer::builder()
            .with_addr(addr)
            .with_tls(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
            .with_input_handler(bridge.clone())
            .with_display_handler(bridge.clone())
            .with_connection_handler(Some(Box::new(bridge)))
            .with_connection_policy(ConnectionPolicy::Reject)
            .with_rdpdr_factory(Some(downloads.factory()))
            .with_honor_client_desktop_size(Some(DesktopSize {
                width: 3840,
                height: 2160,
            }))
            .build();
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
