//! Example of utilizing `ironrdp-server` crate.

#[macro_use]
extern crate tracing;

use std::fs::File;
use std::io::BufReader;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use ironrdp_cliprdr::backend::{CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr_native::StubCliprdrBackend;
use ironrdp_connector::DesktopSize;
use ironrdp_rdpsnd::pdu::ClientAudioFormatPdu;
use ironrdp_rdpsnd::server::{RdpsndServerHandler, RdpsndServerMessage};
use ironrdp_server::{
    BitmapUpdate, CliprdrServerFactory, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat, PixelOrder, RdpServer,
    RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler, ServerEvent, ServerEventSender,
    SoundServerFactory,
};
use rand::prelude::*;
use rustls::ServerConfig;
use rustls_pemfile::{certs, pkcs8_private_keys};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::time::{self, sleep, Duration};
use tokio_rustls::TlsAcceptor;

const HELP: &str = "\
USAGE:
  cargo run --example=server -- --host <HOSTNAME> --port <PORT>
";

#[tokio::main]
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
        Action::Run { host, port, cert, key } => run(host, port, cert, key).await,
    }
}

#[derive(Debug)]
enum Action {
    ShowHelp,
    Run {
        host: String,
        port: u16,
        cert: Option<String>,
        key: Option<String>,
    },
}

fn parse_args() -> anyhow::Result<Action> {
    let mut args = pico_args::Arguments::from_env();

    let action = if args.contains(["-h", "--help"]) {
        Action::ShowHelp
    } else {
        let host = args.opt_value_from_str("--host")?.unwrap_or(String::from("localhost"));
        let port = args.opt_value_from_str("--port")?.unwrap_or(3389);
        let cert = args.opt_value_from_str("--cert")?;
        let key = args.opt_value_from_str("--key")?;
        Action::Run { host, port, cert, key }
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

fn acceptor(cert_path: &str, key_path: &str) -> anyhow::Result<TlsAcceptor> {
    let cert = certs(&mut BufReader::new(File::open(cert_path)?))
        .next()
        .context("no certificate")??;
    let key = pkcs8_private_keys(&mut BufReader::new(File::open(key_path)?))
        .next()
        .context("no private key")??;

    let mut server_config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::Certificate(cert.as_ref().to_vec())],
            rustls::PrivateKey(key.secret_pkcs8_der().to_vec()),
        )
        .expect("bad certificate/key");

    // This adds support for the SSLKEYLOGFILE env variable (https://wiki.wireshark.org/TLS#using-the-pre-master-secret)
    server_config.key_log = Arc::new(rustls::KeyLogFile::new());

    Ok(TlsAcceptor::from(Arc::new(server_config)))
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
        let mut rng = rand::thread_rng();

        let top: u16 = rng.gen_range(0..HEIGHT);
        let height = NonZeroU16::new(rng.gen_range(1..=HEIGHT - top)).unwrap();
        let left: u16 = rng.gen_range(0..WIDTH);
        let width = NonZeroU16::new(rng.gen_range(1..=WIDTH - left)).unwrap();
        let mut data = Vec::with_capacity(4 * usize::from(width.get()) * usize::from(height.get()));
        for _ in 0..(data.capacity() / 4) {
            data.push(rng.gen());
            data.push(rng.gen());
            data.push(rng.gen());
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

struct StubCliprdrServerFactory {}

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
    ev_sender: Option<mpsc::UnboundedSender<ServerEvent>>,
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
            inner: self.inner.clone(),
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
            use std::f32::consts::PI;

            let total_samples = (sample_rate as u64 * duration_ms) / 1000;
            let samples_per_wave_length = sample_rate as f32 / frequency;
            let amplitude = 32767.0; // Max amplitude for 16-bit audio

            let mut samples = Vec::with_capacity(total_samples as usize * 2 * 2);

            for n in 0..total_samples {
                let t = (n as f32 % samples_per_wave_length) / samples_per_wave_length;
                let sample = (t * 2.0 * PI).sin();
                let sample = (sample * amplitude) as i16;
                samples.extend_from_slice(&sample.to_le_bytes());
                samples.extend_from_slice(&sample.to_le_bytes());
            }

            samples
        }

        let inner = self.inner.clone();
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
                ts += 100;
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

async fn run(host: String, port: u16, cert: Option<String>, key: Option<String>) -> anyhow::Result<()> {
    info!(host, port, cert, key, "run");
    let handler = Handler::new();

    let tls = cert
        .as_ref()
        .zip(key.as_ref())
        .map(|(cert, key)| acceptor(cert, key).unwrap());

    let addr = SocketAddr::new(host.parse::<IpAddr>()?, port);

    let server = RdpServer::builder().with_addr(addr);
    let server = if let Some(tls) = tls {
        server.with_tls(tls)
    } else {
        server.with_no_security()
    };

    let cliprdr = Box::new(StubCliprdrServerFactory {});
    let sound = Box::new(StubSoundServerFactory {
        inner: Arc::new(Mutex::new(Inner { ev_sender: None })),
    });

    let mut server = server
        .with_input_handler(handler.clone())
        .with_display_handler(handler.clone())
        .with_cliprdr_factory(Some(cliprdr))
        .with_sound_factory(Some(sound))
        .build();

    server.run().await
}
