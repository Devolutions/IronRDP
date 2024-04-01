//! Example of utilizing `ironrdp-server` crate.

#[macro_use]
extern crate tracing;

use std::fs::File;
use std::io::BufReader;
use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU16;
use std::sync::Arc;

use anyhow::Context as _;
use ironrdp_cliprdr_native::StubClipboard;
use ironrdp_connector::DesktopSize;
use ironrdp_server::{
    BitmapUpdate, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat, PixelOrder, RdpServer, RdpServerDisplay,
    RdpServerDisplayUpdates, RdpServerInputHandler,
};
use rand::prelude::*;
use rustls::ServerConfig;
use rustls_pemfile::{certs, pkcs8_private_keys};
use tokio::time::{sleep, Duration};
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

    let cliprdr = StubClipboard::new();

    let mut server = server
        .with_input_handler(handler.clone())
        .with_display_handler(handler.clone())
        .with_cliprdr_factory(Some(cliprdr.backend_factory()))
        .build();

    server.run().await
}
