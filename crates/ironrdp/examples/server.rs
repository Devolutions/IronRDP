//! Example of utilizing `ironrdp-server` crate.

#![allow(unused_crate_dependencies)] // False positives because there are both a library and a binary.
#![allow(clippy::print_stdout)]

use core::net::SocketAddr;
use core::num::{NonZeroU16, NonZeroUsize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use ironrdp::cliprdr::backend::{CliprdrBackend, CliprdrBackendFactory};
use ironrdp::connector::DesktopSize;
use ironrdp::rdpsnd::pdu::{AudioFormat, ClientAudioFormatPdu, WaveFormat};
use ironrdp::rdpsnd::server::{RdpsndServerHandler, RdpsndServerMessage};
use ironrdp::server::tokio::sync::mpsc::UnboundedSender;
use ironrdp::server::tokio::time::{self, sleep, Duration};
use ironrdp::server::{
    tokio, BitmapUpdate, CliprdrServerFactory, Credentials, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat,
    RdpServer, RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler, ServerEvent, ServerEventSender,
    SoundServerFactory, TlsIdentityCtx,
};
use ironrdp_cliprdr_native::StubCliprdrBackend;
use rand::prelude::*;
use tracing::{debug, info, warn};

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
    async fn next_update(&mut self) -> anyhow::Result<Option<DisplayUpdate>> {
        sleep(Duration::from_millis(100)).await;
        let mut rng = rand::rng();

        let y: u16 = rng.random_range(0..HEIGHT);
        let height = rng.random_range(1..=HEIGHT.checked_sub(y).expect("never underflow"));
        let height = NonZeroU16::new(height).expect("never zero");

        let x: u16 = rng.random_range(0..WIDTH);
        let width = rng.random_range(1..=WIDTH.checked_sub(x).expect("never underflow"));
        let width = NonZeroU16::new(width).expect("never zero");

        let capacity = NonZeroUsize::from(width)
            .checked_mul(NonZeroUsize::from(height))
            .expect("never overflow")
            .get()
            .checked_mul(4)
            .expect("never overflow");
        let mut data = Vec::with_capacity(capacity);
        for _ in 0..(data.capacity() / 4) {
            data.push(rng.random());
            data.push(rng.random());
            data.push(rng.random());
            data.push(255);
        }

        info!("get_update +{x}+{y} {width}x{height}");
        let stride = NonZeroUsize::from(width)
            .checked_mul(NonZeroUsize::new(4).expect("never zero"))
            .expect("never overflow");
        let bitmap = BitmapUpdate {
            x,
            y,
            width,
            height,
            format: PixelFormat::BgrA32,
            data: data.into(),
            stride,
        };
        Ok(Some(DisplayUpdate::Bitmap(bitmap)))
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
        let mut inner = self.inner.lock().expect("poisoned");
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

impl SndHandler {
    fn choose_format(&self, client_formats: &[AudioFormat]) -> Option<u16> {
        for (n, fmt) in client_formats.iter().enumerate() {
            if self.get_formats().contains(fmt) {
                return u16::try_from(n).ok();
            }
        }
        None
    }
}

impl RdpsndServerHandler for SndHandler {
    fn get_formats(&self) -> &[AudioFormat] {
        &[
            AudioFormat {
                format: WaveFormat::OPUS,
                n_channels: 2,
                n_samples_per_sec: 48000,
                n_avg_bytes_per_sec: 192000,
                n_block_align: 4,
                bits_per_sample: 16,
                data: None,
            },
            AudioFormat {
                format: WaveFormat::PCM,
                n_channels: 2,
                n_samples_per_sec: 44100,
                n_avg_bytes_per_sec: 176400,
                n_block_align: 4,
                bits_per_sample: 16,
                data: None,
            },
        ]
    }

    fn start(&mut self, client_format: &ClientAudioFormatPdu) -> Option<u16> {
        debug!(?client_format);

        let Some(nfmt) = self.choose_format(&client_format.formats) else {
            return Some(0);
        };

        let fmt = client_format.formats[usize::from(nfmt)].clone();

        let mut opus_enc = if fmt.format == WaveFormat::OPUS {
            let n_channels: opus2::Channels = match fmt.n_channels {
                1 => opus2::Channels::Mono,
                2 => opus2::Channels::Stereo,
                n => {
                    warn!("Invalid OPUS channels: {}", n);
                    return Some(0);
                }
            };

            match opus2::Encoder::new(fmt.n_samples_per_sec, n_channels, opus2::Application::Audio) {
                Ok(enc) => Some(enc),
                Err(err) => {
                    warn!("Failed to create OPUS encoder: {}", err);
                    return Some(0);
                }
            }
        } else {
            None
        };

        let inner = Arc::clone(&self.inner);
        self.task = Some(tokio::spawn(async move {
            let mut interval = time::interval(Duration::from_millis(20));
            let mut ts = 0;
            let mut phase = 0.0f32;
            loop {
                interval.tick().await;
                let wave = generate_sine_wave(fmt.n_samples_per_sec, 440.0, 20, &mut phase);

                let data = if let Some(ref mut enc) = opus_enc {
                    match enc.encode_vec(&wave, wave.len()) {
                        Ok(data) => data,
                        Err(err) => {
                            warn!("Failed to encode with OPUS: {}", err);
                            return;
                        }
                    }
                } else {
                    wave.into_iter().flat_map(|value| value.to_le_bytes()).collect()
                };

                let inner = inner.lock().expect("poisoned");
                if let Some(sender) = inner.ev_sender.as_ref() {
                    let _ = sender.send(ServerEvent::Rdpsnd(RdpsndServerMessage::Wave(data, ts)));
                }
                ts = ts.wrapping_add(100);
            }
        }));

        Some(nfmt)
    }

    fn stop(&mut self) {
        let Some(task) = self.task.take() else {
            return;
        };
        task.abort();
    }
}

fn generate_sine_wave(sample_rate: u32, frequency: f32, duration_ms: u64, phase: &mut f32) -> Vec<i16> {
    use core::f32::consts::PI;

    let total_samples = (u64::from(sample_rate) * duration_ms) / 1000;
    let delta_phase = 2.0 * PI * frequency / sample_rate as f32;
    let amplitude = 32767.0; // Max amplitude for 16-bit audio

    let capacity = (total_samples as usize) * 2; // 2 channels
    let mut samples = Vec::with_capacity(capacity);

    for _ in 0..total_samples {
        let sample = (*phase).sin();
        *phase += delta_phase;
        // Wrap phase to maintain precision and avoid overflow
        *phase %= 2.0 * PI;

        #[expect(clippy::cast_possible_truncation)]
        let sample_i16 = (sample * amplitude) as i16;

        // Write same sample to both channels (stereo)
        samples.push(sample_i16);
        samples.push(sample_i16);
    }

    samples
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
