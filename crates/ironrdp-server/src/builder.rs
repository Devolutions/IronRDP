use core::net::SocketAddr;
use core::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Arc;

use anyhow::Result;
use ironrdp_pdu::codecs::rfx::Quant;
use ironrdp_pdu::rdp::capability_sets::{BitmapCodecs, EntropyBits, server_codecs_capabilities};
use ironrdp_pdu::rdp::session_info::ServerAutoReconnect;
use tokio_rustls::TlsAcceptor;

use super::clipboard::CliprdrServerFactory;
use super::display::{DesktopSize, RdpServerDisplay};
#[cfg(feature = "egfx")]
use super::gfx::GfxServerFactory;
use super::handler::{KeyboardEvent, MouseEvent, RdpServerInputHandler};
use super::server::{
    ConnectionHandler, CredentialValidator, RdpServer, RdpServerOptions, RdpServerSecurity, StaticChannelFactory,
};
use crate::{DisplayUpdate, RdpServerDisplayUpdates, SoundServerFactory};

pub struct WantsAddr {}
pub struct WantsSecurity {
    addr: SocketAddr,
}
pub struct WantsHandler {
    addr: SocketAddr,
    security: RdpServerSecurity,
}
pub struct WantsDisplay {
    addr: SocketAddr,
    security: RdpServerSecurity,
    handler: Box<dyn RdpServerInputHandler>,
}
pub struct BuilderDone {
    addr: SocketAddr,
    security: RdpServerSecurity,
    codecs: BitmapCodecs,
    max_request_size: u32,
    handler: Box<dyn RdpServerInputHandler>,
    display: Box<dyn RdpServerDisplay>,
    static_channel_factories: Vec<Box<dyn StaticChannelFactory>>,
    cliprdr_factory: Option<Box<dyn CliprdrServerFactory>>,
    sound_factory: Option<Box<dyn SoundServerFactory>>,
    connection_handler: Option<Box<dyn ConnectionHandler>>,
    credential_validator: Option<Arc<dyn CredentialValidator>>,
    #[cfg(feature = "egfx")]
    gfx_factory: Option<Box<dyn GfxServerFactory>>,
    display_suppressed: Option<Arc<AtomicBool>>,
    autodetect_rtt: Option<Arc<AtomicU32>>,
    autodetect_baseline_rtt: Option<Arc<AtomicU32>>,
    honor_client_desktop_size: Option<DesktopSize>,
    auto_reconnect_cookie: Option<ServerAutoReconnect>,
    remotefx_quant: Quant,
    remotefx_entropy_coder: Option<EntropyBits>,
}

pub struct RdpServerBuilder<State> {
    state: State,
}

impl RdpServerBuilder<WantsAddr> {
    pub fn new() -> Self {
        Self { state: WantsAddr {} }
    }

    #[expect(clippy::unused_self)] // ensuring state transition from WantsAddr
    pub fn with_addr(self, addr: impl Into<SocketAddr>) -> RdpServerBuilder<WantsSecurity> {
        RdpServerBuilder {
            state: WantsSecurity { addr: addr.into() },
        }
    }
}

impl Default for RdpServerBuilder<WantsAddr> {
    fn default() -> Self {
        Self::new()
    }
}

impl RdpServerBuilder<WantsSecurity> {
    pub fn with_no_security(self) -> RdpServerBuilder<WantsHandler> {
        RdpServerBuilder {
            state: WantsHandler {
                addr: self.state.addr,
                security: RdpServerSecurity::None,
            },
        }
    }

    pub fn with_tls(self, acceptor: impl Into<TlsAcceptor>) -> RdpServerBuilder<WantsHandler> {
        RdpServerBuilder {
            state: WantsHandler {
                addr: self.state.addr,
                security: RdpServerSecurity::Tls(acceptor.into()),
            },
        }
    }

    pub fn with_hybrid(self, acceptor: impl Into<TlsAcceptor>, pub_key: Vec<u8>) -> RdpServerBuilder<WantsHandler> {
        RdpServerBuilder {
            state: WantsHandler {
                addr: self.state.addr,
                security: RdpServerSecurity::Hybrid((acceptor.into(), pub_key)),
            },
        }
    }
}

impl RdpServerBuilder<WantsHandler> {
    pub fn with_input_handler<H>(self, handler: H) -> RdpServerBuilder<WantsDisplay>
    where
        H: RdpServerInputHandler + 'static,
    {
        RdpServerBuilder {
            state: WantsDisplay {
                addr: self.state.addr,
                security: self.state.security,
                handler: Box::new(handler),
            },
        }
    }

    pub fn with_no_input(self) -> RdpServerBuilder<WantsDisplay> {
        RdpServerBuilder {
            state: WantsDisplay {
                addr: self.state.addr,
                security: self.state.security,
                handler: Box::new(NoopInputHandler),
            },
        }
    }
}

impl RdpServerBuilder<WantsDisplay> {
    pub fn with_display_handler<D>(self, display: D) -> RdpServerBuilder<BuilderDone>
    where
        D: RdpServerDisplay + 'static,
    {
        RdpServerBuilder {
            state: BuilderDone {
                addr: self.state.addr,
                security: self.state.security,
                handler: self.state.handler,
                display: Box::new(display),
                static_channel_factories: Vec::new(),
                sound_factory: None,
                cliprdr_factory: None,
                connection_handler: None,
                credential_validator: None,
                codecs: server_codecs_capabilities(&[]).expect("can't panic for &[]"),
                max_request_size: RdpServerOptions::DEFAULT_MAX_REQUEST_SIZE,
                #[cfg(feature = "egfx")]
                gfx_factory: None,
                display_suppressed: None,
                autodetect_rtt: None,
                autodetect_baseline_rtt: None,
                honor_client_desktop_size: None,
                auto_reconnect_cookie: None,
                remotefx_quant: Quant::default(),
                remotefx_entropy_coder: None,
            },
        }
    }

    pub fn with_no_display(self) -> RdpServerBuilder<BuilderDone> {
        RdpServerBuilder {
            state: BuilderDone {
                addr: self.state.addr,
                security: self.state.security,
                handler: self.state.handler,
                display: Box::new(NoopDisplay),
                static_channel_factories: Vec::new(),
                sound_factory: None,
                cliprdr_factory: None,
                connection_handler: None,
                credential_validator: None,
                codecs: server_codecs_capabilities(&[]).expect("can't panic for &[]"),
                max_request_size: RdpServerOptions::DEFAULT_MAX_REQUEST_SIZE,
                #[cfg(feature = "egfx")]
                gfx_factory: None,
                display_suppressed: None,
                autodetect_rtt: None,
                autodetect_baseline_rtt: None,
                honor_client_desktop_size: None,
                auto_reconnect_cookie: None,
                remotefx_quant: Quant::default(),
                remotefx_entropy_coder: None,
            },
        }
    }
}

impl RdpServerBuilder<BuilderDone> {
    /// Add a factory that attaches a fresh static-channel processor per connection.
    pub fn with_static_channel_factory(mut self, factory: Box<dyn StaticChannelFactory>) -> Self {
        self.state.static_channel_factories.push(factory);
        self
    }

    pub fn with_cliprdr_factory(mut self, cliprdr_factory: Option<Box<dyn CliprdrServerFactory>>) -> Self {
        self.state.cliprdr_factory = cliprdr_factory;
        self
    }

    pub fn with_sound_factory(mut self, sound: Option<Box<dyn SoundServerFactory>>) -> Self {
        self.state.sound_factory = sound;
        self
    }

    /// Configure EGFX (Graphics Pipeline Extension) for H.264 video streaming.
    #[cfg(feature = "egfx")]
    pub fn with_gfx_factory(mut self, gfx_factory: Option<Box<dyn GfxServerFactory>>) -> Self {
        self.state.gfx_factory = gfx_factory;
        self
    }

    pub fn with_bitmap_codecs(mut self, codecs: BitmapCodecs) -> Self {
        self.state.codecs = codecs;
        self
    }

    /// Sets the [MultifragmentUpdate] maximum reassembly buffer size advertised
    /// during capability exchange.
    ///
    /// Defaults to [`RdpServerOptions::DEFAULT_MAX_REQUEST_SIZE`] (8 MB).
    ///
    /// [MultifragmentUpdate]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/01717954-716a-424d-af35-28fb2b86df89
    pub fn with_max_request_size(mut self, max_request_size: u32) -> Self {
        self.state.max_request_size = max_request_size;
        self
    }

    /// Set a handler for connection lifecycle events (accept filtering,
    /// post-disconnect cleanup).
    pub fn with_connection_handler(mut self, handler: Option<Box<dyn ConnectionHandler>>) -> Self {
        self.state.connection_handler = handler;
        self
    }

    /// Share the server's "display suppressed" flag with the display
    /// backend before construction.
    ///
    /// The flag is `true` while the connected client has sent
    /// `SuppressOutput { desktop_rect: None }` (e.g., mstsc minimized).
    /// Display backends that want to skip frame emission while the
    /// client is minimized create one `Arc<AtomicBool>` in the
    /// application, hand a clone to the display, and pass the same
    /// `Arc` here so the server's per-connection PDU handler writes to
    /// the same instance the backend reads.
    ///
    /// When this is not called, the server allocates its own internal
    /// flag (still readable via [`RdpServer::display_suppressed_handle`])
    /// — useful when the backend can call `display_suppressed_handle()`
    /// after construction to obtain a handle, rather than sharing one in.
    pub fn with_display_suppressed_handle(mut self, handle: Arc<AtomicBool>) -> Self {
        self.state.display_suppressed = Some(handle);
        self
    }

    /// Negotiate each session at the desktop size the client requests in its
    /// Client Core Data, rather than the size reported by the display handler.
    ///
    /// The client's requested resolution is only carried in the GCC Client
    /// Core Data of the connection handshake; the size echoed back in the
    /// client's Confirm Active is the value it copied from the server's Demand
    /// Active (per [MS-RDPBCGR] 2.2.1.13.2) and so cannot reveal what the
    /// client asked for. With this enabled the acceptor first clamps the
    /// requested size to the operator maximum and then, if the clamped size is
    /// within the protocol-legal range, adopts it before Demand Active is sent,
    /// so the session starts at that size with no Deactivation-Reactivation
    /// resize. The display handler observes the negotiated size through
    /// [`RdpServerDisplay::request_initial_size`].
    ///
    /// Pass `Some(max)` to honor the client's request, clamped per dimension to
    /// `max`: the client may ask for a smaller desktop, but never a larger one.
    /// The desktop size is a client-controlled `u16` bounded only by the
    /// protocol ([200, 8192]); `max` is the ceiling the server is willing to
    /// render (for instance the host display's native resolution) so an
    /// untrusted client can't drive the framebuffer/encoder allocation off that
    /// number. Pass `None` (the default) to disable honoring and enforce the
    /// size reported by the display handler.
    ///
    /// # Precondition
    ///
    /// Only enable this with a [`RdpServerDisplay`] whose
    /// [`request_initial_size`] actually adopts (or at least intersects) the
    /// size it is given: the acceptor negotiates the client's size, but the
    /// server still builds its framebuffer/encoder from the size the display
    /// handler reports. A fixed-size handler that ignores the requested size
    /// can produce a mismatch that drops the client. Leave this disabled when
    /// the display handler serves a fixed framebuffer.
    ///
    /// [`request_initial_size`]: crate::RdpServerDisplay::request_initial_size
    pub fn with_honor_client_desktop_size(mut self, max: Option<DesktopSize>) -> Self {
        self.state.honor_client_desktop_size = max;
        self
    }

    /// Set a credential validator for TLS-mode connections.
    ///
    /// When set, credentials received from the client during
    /// `SecureSettingsExchange` (`ClientInfoPdu`) are passed to this
    /// validator before the session is established. Rejection or a backend
    /// error closes the connection. Pass `None` (the default) to skip
    /// validation entirely.
    ///
    /// A valid Server Auto-Reconnect Cookie bypasses this validator. Applications
    /// that must validate every connection should leave automatic reconnection
    /// disabled.
    ///
    /// Not used for CredSSP/Hybrid connections (those use pre-loaded
    /// credentials for NTLM challenge-response).
    pub fn with_credential_validator(mut self, validator: Option<Arc<dyn CredentialValidator>>) -> Self {
        self.state.credential_validator = validator;
        self
    }

    /// Inject a shared NetworkAutoDetect RTT handle (milliseconds, `u32::MAX`
    /// until the first measurement). The server writes the latest measured RTT
    /// to the same instance the backend reads. When not called, the server
    /// allocates its own (still readable via
    /// [`RdpServer::autodetect_rtt_handle`]). The value stays `u32::MAX` unless
    /// auto-detect is enabled via [`RdpServer::enable_autodetect`].
    pub fn with_autodetect_rtt_handle(mut self, handle: Arc<AtomicU32>) -> Self {
        self.state.autodetect_rtt = Some(handle);
        self
    }

    /// Inject a shared session-lifetime baseline RTT handle (milliseconds,
    /// `u32::MAX` until the first measurement; see
    /// [`RdpServer::autodetect_baseline_rtt_handle`] for what distinguishes
    /// this from [`Self::with_autodetect_rtt_handle`]). The server writes the
    /// latest baseline to the same instance the backend reads. When not
    /// called, the server allocates its own (still readable via
    /// [`RdpServer::autodetect_baseline_rtt_handle`]). The value stays
    /// `u32::MAX` unless auto-detect is enabled via
    /// [`RdpServer::enable_autodetect`].
    pub fn with_autodetect_baseline_rtt_handle(mut self, handle: Arc<AtomicU32>) -> Self {
        self.state.autodetect_baseline_rtt = Some(handle);
        self
    }

    /// Provision the Server Auto-Reconnect Cookie (MS-RDPBCGR 2.2.4.2
    /// `ARC_SC_PRIVATE_PACKET`) handed to the client during logon.
    ///
    /// When set to `Some`, the server sends a Save Session Info PDU carrying the
    /// cookie right after activation. It validates the returning client cookie,
    /// generates a fresh CSPRNG random whenever a client connects, and updates
    /// the active client hourly. Automatic reconnection requires TLS or Hybrid
    /// security, which provides the all-zero client random required for Enhanced
    /// RDP Security. `None` (the default) sends no cookie.
    ///
    /// See [`RdpServer::set_auto_reconnect_cookie`] for post-construction
    /// configuration and [`RdpServer::auto_reconnect_cookie_handle`] for
    /// updates while the server is running.
    pub fn with_auto_reconnect_cookie(mut self, cookie: Option<ServerAutoReconnect>) -> Self {
        self.state.auto_reconnect_cookie = cookie;
        self
    }

    /// Set the quantization values the RemoteFX encoder uses once selected.
    /// Defaults to [`Quant::default`], the same values Windows RDP servers
    /// send. Build a validated [`Quant`] with [`Quant::try_new`].
    ///
    /// Has no effect unless the client and server negotiate RemoteFX; this
    /// only changes the quantization RemoteFX uses when it is picked.
    pub fn with_remotefx_quant(mut self, quant: Quant) -> Self {
        self.state.remotefx_quant = quant;
        self
    }

    /// State a preferred RemoteFX entropy coder (RLGR1 or RLGR3). If the
    /// client's advertised TS_RFX_ICAP array includes it, the server uses
    /// it; otherwise the server falls back to whichever coder the client
    /// offered first.
    ///
    /// `None` (the default) always uses whichever coder the client offered
    /// first: [MS-RDPRFX] 3.1.5.1 has the server arbitrarily pick one
    /// supported TS_RFX_ICAP element rather than rank the array as a
    /// preference order. Has no effect unless the client and server
    /// negotiate RemoteFX.
    ///
    /// [MS-RDPRFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdprfx/
    pub fn with_remotefx_entropy_coder(mut self, coder: Option<EntropyBits>) -> Self {
        self.state.remotefx_entropy_coder = coder;
        self
    }

    pub fn build(self) -> RdpServer {
        let mut server = RdpServer::new(
            RdpServerOptions {
                addr: self.state.addr,
                security: self.state.security,
                codecs: self.state.codecs,
                max_request_size: self.state.max_request_size,
                honor_client_desktop_size: self.state.honor_client_desktop_size,
                remotefx_quant: self.state.remotefx_quant,
                remotefx_entropy_coder: self.state.remotefx_entropy_coder,
            },
            self.state.handler,
            self.state.display,
            self.state.static_channel_factories,
            self.state.sound_factory,
            self.state.cliprdr_factory,
            self.state.connection_handler,
            #[cfg(feature = "egfx")]
            self.state.gfx_factory,
            self.state.display_suppressed,
            self.state.autodetect_rtt,
            self.state.autodetect_baseline_rtt,
        );
        server.set_credential_validator(self.state.credential_validator);
        server.set_auto_reconnect_cookie(self.state.auto_reconnect_cookie);
        server
    }
}

struct NoopInputHandler;

impl RdpServerInputHandler for NoopInputHandler {
    fn keyboard(&mut self, _: KeyboardEvent) {}
    fn mouse(&mut self, _: MouseEvent) {}
}

struct NoopDisplayUpdates;

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for NoopDisplayUpdates {
    async fn next_update(&mut self) -> Result<Option<DisplayUpdate>> {
        let () = core::future::pending().await;
        unreachable!()
    }
}

struct NoopDisplay;

#[async_trait::async_trait]
impl RdpServerDisplay for NoopDisplay {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize { width: 0, height: 0 }
    }

    async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
        Ok(Box::new(NoopDisplayUpdates {}))
    }
}
