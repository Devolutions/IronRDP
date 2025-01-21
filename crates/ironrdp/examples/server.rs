//! Example of utilizing `ironrdp-server` crate.

#![allow(unused_crate_dependencies)] // False positives because there are both a library and a binary.
#![allow(clippy::print_stdout)]

#[macro_use]
extern crate tracing;

use core::num::NonZeroU16;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use ironrdp::cliprdr::backend::{CliprdrBackend, CliprdrBackendFactory};
use ironrdp::connector::DesktopSize;
use ironrdp::rdpsnd::pdu::ClientAudioFormatPdu;
use ironrdp::rdpsnd::server::{RdpsndServerHandler, RdpsndServerMessage};
use ironrdp::server::tokio::sync::mpsc::UnboundedSender;
use ironrdp::server::tokio::time::{self, sleep, Duration};
use ironrdp::server::{
    tokio, BitmapUpdate, CliprdrServerFactory, Credentials, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat,
    PixelOrder, RdpServer, RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler, ServerEvent,
    ServerEventSender, SoundServerFactory, TlsIdentityCtx,
};
use ironrdp_cliprdr_native::StubCliprdrBackend;
use rand::prelude::*;

const HELP: &str = "\
USAGE:
  cargo run --example=server -- [--bind-addr <SOCKET ADDRESS>] [--cert <CERTIFICATE>] [--key <CERTIFICATE KEY>] [--user USERNAME] [--pass PASSWORD] [--sec tls|hybrid]
";

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), anyhow::Error> {
    let action = match parse_args() {
        Ok(action) => action,
        Err(e) => {
            println!("{HELP}");
            return Err(e.context("invalid argument(s)"));
        }
    };

    setup_logging()?;

    match action {
        Action::ShowHelp => {
            println!("{HELP}");
            Ok(())
        }
        Action::Run {
            bind_addr,
            hybrid,
            user,
            pass,
            cert,
            key,
        } => run(bind_addr, hybrid, user, pass, cert, key).await,
    }
}

#[derive(Debug)]
enum Action {
    ShowHelp,
    Run {
        bind_addr: SocketAddr,
        hybrid: bool,
        user: String,
        pass: String,
        cert: Option<PathBuf>,
        key: Option<PathBuf>,
    },
}

fn parse_args() -> anyhow::Result<Action> {
    let mut args = pico_args::Arguments::from_env();

    let action = if args.contains(["-h", "--help"]) {
        Action::ShowHelp
    } else {
        let bind_addr = args
            .opt_value_from_str("--bind-addr")?
            .unwrap_or_else(|| "127.0.0.1:3389".parse().expect("valid hardcoded SocketAddr string"));

        let sec = args.opt_value_from_str("--sec")?.unwrap_or_else(|| "hybrid".to_owned());
        let hybrid = match sec.as_ref() {
            "tls" => false,
            "hybrid" => true,
            _ => anyhow::bail!("Unhandled security: '{sec}'"),
        };

        let cert = args.opt_value_from_str("--cert")?;
        let key = args.opt_value_from_str("--key")?;

        let user = args.opt_value_from_str("--user")?.unwrap_or_else(|| "user".to_owned());
        let pass = args.opt_value_from_str("--pass")?.unwrap_or_else(|| "pass".to_owned());

        Action::Run {
            bind_addr,
            hybrid,
            user,
            pass,
            cert,
            key,
        }
    };

    Ok(action)
}

fn setup_logging() -> anyhow::Result<()> {
    use tracing::metadata::LevelFilter;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    let fmt_layer = tracing_subscriber::fmt::layer().compact();

    let env_filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .with_env_var("IRONRDP_LOG")
        .from_env_lossy();

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(env_filter)
        .try_init()
        .context("failed to set tracing global subscriber")?;

    Ok(())
}

#[derive(Clone, Debug)]
struct Handler;

impl Handler {
    fn new() -> Self {
        Self
    }
}

impl RdpServerInputHandler for Handler {
    fn keyboard(&mut self, event: KeyboardEvent) {
        info!(?event, "keyboard");
    }

    fn mouse(&mut self, event: MouseEvent) {
        info!(?event, "mouse");
    }
}

const WIDTH: u16 = 1920;
const HEIGHT: u16 = 1080;

struct DisplayUpdates;

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for DisplayUpdates {
    async fn next_update(&mut self) -> Option<DisplayUpdate> {
        sleep(Duration::from_millis(100)).await;
        let mut rng = thread_rng();

        let top: u16 = rng.gen_range(0..HEIGHT);
        let height = NonZeroU16::new(rng.gen_range(1..=HEIGHT.checked_sub(top).unwrap())).unwrap();
        let left: u16 = rng.gen_range(0..WIDTH);
        let width = NonZeroU16::new(rng.gen_range(1..=WIDTH.checked_sub(left).unwrap())).unwrap();
        let capacity = usize::from(width.get())
            .checked_mul(usize::from(height.get()))
            .unwrap()
            .checked_mul(4)
            .unwrap();
        let mut data = Vec::with_capacity(capacity);
        for _ in 0..(data.capacity() / 4) {
            data.push(rng.r#gen());
            data.push(rng.r#gen());
            data.push(rng.r#gen());
            data.push(255);
        }

        info!("get_update +{left}+{top} {width}x{height}");
        let bitmap = BitmapUpdate {
            top,
            left,
            width,
            height,
            format: PixelFormat::BgrA32,
            order: PixelOrder::TopToBottom,
            data,
            stride: usize::from(width.get()).checked_mul(4).unwrap(),
        };
        Some(DisplayUpdate::Bitmap(bitmap))
    }
}

#[async_trait::async_trait]
impl RdpServerDisplay for Handler {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize {
            width: WIDTH,
            height: HEIGHT,
        }
    }

    async fn updates(&mut self) -> anyhow::Result<Box<dyn RdpServerDisplayUpdates>> {
        Ok(Box::new(DisplayUpdates {}))
    }
}

struct StubCliprdrServerFactory;

impl CliprdrBackendFactory for StubCliprdrServerFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(StubCliprdrBackend::new())
    }
}

impl ServerEventSender for StubCliprdrServerFactory {
    fn set_sender(&mut self, _sender: UnboundedSender<ServerEvent>) {}
}

impl CliprdrServerFactory for StubCliprdrServerFactory {}

#[derive(Debug)]
pub struct Inner {
    ev_sender: Option<UnboundedSender<ServerEvent>>,
}

struct StubSoundServerFactory {
    inner: Arc<Mutex<Inner>>,
}

impl ServerEventSender for StubSoundServerFactory {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        let mut inner = self.inner.lock().unwrap();

        inner.ev_sender = Some(sender);
    }
}

impl SoundServerFactory for StubSoundServerFactory {
    fn build_backend(&self) -> Box<dyn RdpsndServerHandler> {
        Box::new(SndHandler {
            inner: Arc::clone(&self.inner),
            task: None,
        })
    }
}

#[derive(Debug)]
struct SndHandler {
    inner: Arc<Mutex<Inner>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl RdpsndServerHandler for SndHandler {
    fn get_formats(&self) -> &[ironrdp_rdpsnd::pdu::AudioFormat] {
        use ironrdp_rdpsnd::pdu::{AudioFormat, WaveFormat};

        &[AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 2,
            n_samples_per_sec: 44100,
            n_avg_bytes_per_sec: 176400,
            n_block_align: 4,
            bits_per_sample: 16,
            data: None,
        }]
    }

    fn start(&mut self, client_format: &ClientAudioFormatPdu) -> Option<u16> {
        async fn generate_sine_wave(sample_rate: u32, frequency: f32, duration_ms: u64) -> Vec<u8> {
            use core::f32::consts::PI;

            let total_samples = u64::from(sample_rate / 1000).checked_mul(duration_ms).unwrap();
            let samples_per_wave_length = sample_rate as f32 / frequency;
            let amplitude = 32767.0; // Max amplitude for 16-bit audio

            let capacity = total_samples.checked_mul(2 + 2).unwrap();
            let mut samples = Vec::with_capacity(usize::try_from(capacity).unwrap());

            for n in 0..total_samples {
                let t = (n as f32 % samples_per_wave_length) / samples_per_wave_length;
                let sample = (t * 2.0 * PI).sin();
                #[allow(clippy::cast_possible_truncation)]
                let sample = (sample * amplitude) as i16;
                samples.extend_from_slice(&sample.to_le_bytes());
                samples.extend_from_slice(&sample.to_le_bytes());
            }

            samples
        }

        let inner = Arc::clone(&self.inner);
        self.task = Some(tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_millis(100));
            let mut ts = 0;
            loop {
                interval.tick().await;
                let data = generate_sine_wave(44100, 440.0, 100).await;
                let inner = inner.lock().unwrap();
                if let Some(sender) = inner.ev_sender.as_ref() {
                    let _ = sender.send(ServerEvent::Rdpsnd(RdpsndServerMessage::Wave(data, ts)));
                }
                ts = ts.wrapping_add(100);
            }
        }));

        debug!(?client_format);
        Some(0)
    }

    fn stop(&mut self) {
        let Some(task) = self.task.take() else {
            return;
        };
        task.abort();
    }
}

async fn run(
    bind_addr: SocketAddr,
    hybrid: bool,
    username: String,
    password: String,
    cert: Option<PathBuf>,
    key: Option<PathBuf>,
) -> anyhow::Result<()> {
    info!(%bind_addr, ?cert, ?key, "run");

    let handler = Handler::new();

    let server_builder = RdpServer::builder().with_addr(bind_addr);

    let server_builder = if let Some((cert_path, key_path)) = cert.as_deref().zip(key.as_deref()) {
        let identity = TlsIdentityCtx::init_from_paths(cert_path, key_path).context("failed to init TLS identity")?;
        let acceptor = identity.make_acceptor().context("failed to build TLS acceptor")?;

        if hybrid {
            server_builder.with_hybrid(acceptor, identity.pub_key)
        } else {
            server_builder.with_tls(acceptor)
        }
    } else {
        server_builder.with_no_security()
    };

    let cliprdr = Box::new(StubCliprdrServerFactory);

    let sound = Box::new(StubSoundServerFactory {
        inner: Arc::new(Mutex::new(Inner { ev_sender: None })),
    });

    let mut server = server_builder
        .with_input_handler(handler.clone())
        .with_display_handler(handler.clone())
        .with_cliprdr_factory(Some(cliprdr))
        .with_sound_factory(Some(sound))
        .build();

    server.set_credentials(Some(Credentials {
        username,
        password,
        domain: None,
    }));

    server.run().await
}
