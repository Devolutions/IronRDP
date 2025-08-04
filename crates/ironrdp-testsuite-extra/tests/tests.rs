#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary

use core::future::Future;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use ironrdp::connector;
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::pdu::{self, gcc};
use ironrdp::server::{
    self, DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat, RdpServer, RdpServerDisplay,
    RdpServerDisplayUpdates, RdpServerInputHandler, ServerEvent, TlsIdentityCtx,
};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{self, ActiveStage, ActiveStageOutput};
use ironrdp_async::{Framed, FramedWrite as _};
use ironrdp_testsuite_extra as _;
use ironrdp_tls::TlsStream;
use ironrdp_tokio::TokioStream;
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::{oneshot, Mutex};
use tracing::debug;

const DESKTOP_WIDTH: u16 = 1024;
const DESKTOP_HEIGHT: u16 = 768;
const USERNAME: &str = "";
const PASSWORD: &str = "";

#[tokio::test]
async fn test_client_server() {
    client_server(default_client_config(), |stage, framed, _display_tx| async {
        (stage, framed)
    })
    .await
}

#[tokio::test]
async fn test_deactivation_reactivation() {
    let client_config = default_client_config();
    let mut image = DecodedImage::new(
        PixelFormat::RgbA32,
        client_config.desktop_size.width,
        client_config.desktop_size.height,
    );
    client_server(client_config, |mut stage, mut framed, display_tx| async move {
        display_tx
            .send(DisplayUpdate::Resize(DesktopSize {
                width: 2048,
                height: 2048,
            }))
            .unwrap();
        {
            let (action, payload) = framed.read_pdu().await.expect("valid PDU");
            let outputs = stage.process(&mut image, action, &payload).expect("stage process");
            let out = outputs.into_iter().next().unwrap();
            match out {
                ActiveStageOutput::DeactivateAll(mut connection_activation) => {
                    // TODO: factor this out in common client code
                    // Execute the Deactivation-Reactivation Sequence:
                    // https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/dfc234ce-481a-4674-9a5d-2a7bafb14432
                    debug!("Received Server Deactivate All PDU, executing Deactivation-Reactivation Sequence");
                    let mut buf = pdu::WriteBuf::new();
                    'activation_seq: loop {
                        let written = ironrdp_async::single_sequence_step_read(
                            &mut framed,
                            &mut *connection_activation,
                            &mut buf,
                        )
                        .await
                        .map_err(|e| session::custom_err!("read deactivation-reactivation sequence step", e))
                        .unwrap();

                        if written.size().is_some() {
                            framed
                                .write_all(buf.filled())
                                .await
                                .map_err(|e| session::custom_err!("write deactivation-reactivation sequence step", e))
                                .unwrap();
                        }

                        if let connector::connection_activation::ConnectionActivationState::Finalized {
                            io_channel_id,
                            user_channel_id,
                            desktop_size,
                            enable_server_pointer,
                            pointer_software_rendering,
                        } = connection_activation.state
                        {
                            debug!(?desktop_size, "Deactivation-Reactivation Sequence completed");
                            // Update image size with the new desktop size.
                            // image = DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                            // Update the active stage with the new channel IDs and pointer settings.
                            stage.set_fastpath_processor(
                                session::fast_path::ProcessorBuilder {
                                    io_channel_id,
                                    user_channel_id,
                                    enable_server_pointer,
                                    pointer_software_rendering,
                                }
                                .build(),
                            );
                            stage.set_enable_server_pointer(enable_server_pointer);
                            break 'activation_seq;
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        (stage, framed)
    })
    .await
}

type DisplayUpdatesRx = Arc<Mutex<UnboundedReceiver<DisplayUpdate>>>;

struct TestDisplayUpdates {
    rx: DisplayUpdatesRx,
}

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for TestDisplayUpdates {
    async fn next_update(&mut self) -> Option<DisplayUpdate> {
        let mut rx = self.rx.lock().await;

        rx.recv().await
    }
}

struct TestDisplay {
    rx: DisplayUpdatesRx,
}

#[async_trait::async_trait]
impl RdpServerDisplay for TestDisplay {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize {
            width: DESKTOP_WIDTH,
            height: DESKTOP_HEIGHT,
        }
    }

    async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
        Ok(Box::new(TestDisplayUpdates {
            rx: Arc::clone(&self.rx),
        }))
    }
}

struct TestInputHandler;
impl RdpServerInputHandler for TestInputHandler {
    fn keyboard(&mut self, _: KeyboardEvent) {}
    fn mouse(&mut self, _: MouseEvent) {}
}

async fn client_server<F, Fut>(client_config: connector::Config, clientfn: F)
where
    F: FnOnce(ActiveStage, Framed<TokioStream<TlsStream<TcpStream>>>, UnboundedSender<DisplayUpdate>) -> Fut + 'static,
    Fut: Future<Output = (ActiveStage, Framed<TokioStream<TlsStream<TcpStream>>>)>,
{
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();

    let cert_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-cert.pem");
    let key_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/certs/server-key.pem");
    let identity = TlsIdentityCtx::init_from_paths(&cert_path, &key_path).expect("failed to init TLS identity");
    let acceptor = identity.make_acceptor().expect("failed to build TLS acceptor");

    let (display_tx, display_rx) = mpsc::unbounded_channel();
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_tls(acceptor)
        .with_input_handler(TestInputHandler)
        .with_display_handler(TestDisplay {
            rx: Arc::new(Mutex::new(display_rx)),
        })
        .build();
    server.set_credentials(Some(server::Credentials {
        username: USERNAME.into(),
        password: PASSWORD.into(),
        domain: None,
    }));
    let ev = server.event_sender().clone();

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let server = tokio::task::spawn_local(async move {
                server.run().await.unwrap();
            });

            let client = tokio::task::spawn_local(async move {
                let (tx, rx) = oneshot::channel();
                ev.send(ServerEvent::GetLocalAddr(tx)).unwrap();
                let server_addr = rx.await.unwrap().unwrap();
                let tcp_stream = TcpStream::connect(server_addr).await.expect("TCP connect");
                let client_addr = tcp_stream.local_addr().expect("local_addr");
                let mut framed = ironrdp_tokio::TokioFramed::new(tcp_stream);
                let mut connector = connector::ClientConnector::new(client_config, client_addr);
                let should_upgrade = ironrdp_async::connect_begin(&mut framed, &mut connector)
                    .await
                    .expect("begin connection");
                let initial_stream = framed.into_inner_no_leftover();
                let (upgraded_stream, server_public_key) = ironrdp_tls::upgrade(initial_stream, "localhost")
                    .await
                    .expect("TLS upgrade");
                let upgraded = ironrdp_tokio::mark_as_upgraded(should_upgrade, &mut connector);
                let mut upgraded_framed = ironrdp_tokio::TokioFramed::new(upgraded_stream);
                let connection_result = ironrdp_async::connect_finalize(
                    upgraded,
                    &mut upgraded_framed,
                    connector,
                    "localhost".into(),
                    server_public_key,
                    None,
                    None,
                )
                .await
                .expect("finalize connection");

                let active_stage = ActiveStage::new(connection_result);
                let (active_stage, mut upgraded_framed) = clientfn(active_stage, upgraded_framed, display_tx).await;
                let outputs = active_stage.graceful_shutdown().expect("shutdown");
                for out in outputs {
                    match out {
                        ActiveStageOutput::ResponseFrame(frame) => {
                            upgraded_framed.write_all(&frame).await.expect("write frame");
                        }
                        _ => unimplemented!(),
                    }
                }

                // server should probably send TLS close_notify
                while let Ok(pdu) = upgraded_framed.read_pdu().await {
                    debug!(?pdu);
                }
                ev.send(ServerEvent::Quit("bye".into())).unwrap();
            });

            tokio::try_join!(server, client).expect("join");
        })
        .await;
}

// Maybe implement Default for Config
fn default_client_config() -> connector::Config {
    connector::Config {
        desktop_size: DesktopSize {
            width: DESKTOP_WIDTH,
            height: DESKTOP_HEIGHT,
        },
        desktop_scale_factor: 0, // Default to 0 per FreeRDP
        enable_tls: true,
        enable_credssp: true,
        credentials: connector::Credentials::UsernamePassword {
            username: USERNAME.into(),
            password: PASSWORD.into(),
        },
        domain: None,
        client_build: semver::Version::parse(env!("CARGO_PKG_VERSION"))
            .map(|version| version.major * 100 + version.minor * 10 + version.patch)
            .unwrap_or(0)
            .try_into()
            .unwrap(),
        client_name: "ironrdp".into(),
        keyboard_type: gcc::KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: "".into(),
        bitmap: None,
        dig_product_id: "".into(),
        // NOTE: hardcode this value like in freerdp
        // https://github.com/FreeRDP/FreeRDP/blob/4e24b966c86fdf494a782f0dfcfc43a057a2ea60/libfreerdp/core/settings.c#LL49C34-L49C70
        client_dir: "C:\\Windows\\System32\\mstscax.dll".into(),
        #[cfg(windows)]
        platform: MajorPlatformType::WINDOWS,
        #[cfg(target_os = "macos")]
        platform: MajorPlatformType::MACINTOSH,
        #[cfg(target_os = "ios")]
        platform: MajorPlatformType::IOS,
        #[cfg(target_os = "linux")]
        platform: MajorPlatformType::UNIX,
        #[cfg(target_os = "android")]
        platform: MajorPlatformType::ANDROID,
        #[cfg(target_os = "freebsd")]
        platform: MajorPlatformType::UNIX,
        #[cfg(target_os = "dragonfly")]
        platform: MajorPlatformType::UNIX,
        #[cfg(target_os = "openbsd")]
        platform: MajorPlatformType::UNIX,
        #[cfg(target_os = "netbsd")]
        platform: MajorPlatformType::UNIX,
        hardware_id: None,
        request_data: None,
        autologon: false,
        no_audio_playback: false,
        license_cache: None,
        enable_server_pointer: true,
        pointer_software_rendering: true,
        performance_flags: Default::default(),
    }
}
