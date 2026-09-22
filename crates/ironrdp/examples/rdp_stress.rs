//! Standalone RDP resize-stability stress harness.
//!
//! Connects straight to an RDP server over TCP + TLS/CredSSP, negotiates EGFX and
//! Display Control, then drives resolution changes and key injection while grading
//! the decoded framebuffer. No browser and no WASM: everything here talks to
//! `ironrdp-session` directly, so a failure points at the protocol/decode path
//! rather than at a client's canvas plumbing.
//!
//! Three metrics, all computed locally so they cannot be fooled by the code under test:
//!
//! - black tiles: how much of the picture is missing outright.
//! - stale tiles: how much of the picture is still the previous frame stretched over the
//!   new desktop size, i.e. content the server never repainted after ResetGraphics.
//!   This is what "torn"/"ghosted" looks like on screen.
//! - seam score: edge energy on the 64-pixel RemoteFX tile grid relative to tile
//!   interiors. Mismatched tiles show up as a hard grid.
//!
//! # Usage example
//!
//! ```shell
//! cargo run --example=rdp_stress --features "session,connector,graphics,dvc,displaycontrol" -- \
//!     --host rdp.example.com -u Administrator --rounds 10 --out-dir /tmp/rdp-stress
//! ```
//!
//! The password is read from `--password` or, preferably, the `RDP_PASSWORD` env var.

#![allow(unused_crate_dependencies)] // false positives because there is both a library and a binary
#![allow(clippy::print_stdout)]
// The grading code is percentage arithmetic over tile and pixel counts: every value is a
// small count or a 0..=100 ratio, so f32 has room to spare and a lost fraction of a
// percent cannot change a verdict.
#![allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation
)]

use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;
use std::io::Write as _;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context as _;
use ironrdp::connector::connection_activation::{ConnectionActivationFactory, ConnectionActivationState};
use ironrdp::connector::{self, ConnectionResult, Credentials};
use ironrdp::core::WriteBuf;
use ironrdp::pdu::gcc::{ConnectionType, KeyboardType};
use ironrdp::pdu::input::fast_path::{FastPathInputEvent, KeyboardFlags};
use ironrdp::pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput};
use ironrdp_displaycontrol::client::DisplayControlClient;
use ironrdp_dvc::DrdynvcClient;
use ironrdp_egfx::client::{GraphicsPipelineClient, GraphicsPipelineHandler, Surface};
use ironrdp_pdu::rdp::client_info::{CompressionType, PerformanceFlags, TimezoneInfo};
use sspi::network_client::reqwest_network_client::ReqwestNetworkClient;
use tokio_rustls::rustls;
use tracing::{debug, info};

const HELP: &str = "\
USAGE:
  cargo run --example=rdp_stress --features \"session,connector,graphics,dvc,displaycontrol\" -- \\
      --host <HOSTNAME> [--port <PORT>] -u <USERNAME> [-p <PASSWORD>] [-d <DOMAIN>]
      [--sizes <WxH,WxH,...>] [--rounds <N>] [--settle-ms <MS>] [--threshold <PCT>]
      [--no-credssp] [--autologon]
      [--burst <N>] [--burst-gap-ms <MS>] [--jitter] [--keys <COMBO,COMBO>]
      [--redraw none|auto|refresh|suppress]
      [--out-dir <DIR>] [--dump-all]

The password is taken from RDP_PASSWORD when -p is omitted. --no-credssp drops NLA,
which servers like xrdp (security_layer=tls) need.

--threshold is the percentage of graded tiles allowed to be black *or* stale before a
step counts as a failure; the process exits non-zero if any step does.

--sizes cycles desktop sizes; a big jump such as 1280x720,1920x1080 is what entering
fullscreen does. --burst replays a dragged window edge: several Display Control
requests in a row, the last one being the target size. --keys accepts ctrl+alt+del,
ctrl+esc, win, esc and is sent once per round.

--redraw asks the server to repaint the whole desktop after each resolution change:
refresh sends Refresh Rect, suppress toggles Suppress Output, auto prefers whichever
the server advertised. The default (none) matches what the web client does today.
";

/// Blackness metric: side of the sampling tile, in pixels.
const TILE: usize = 16;
/// A tile counts as black when at least this fraction of its pixels are black.
const TILE_BLACK_FRACTION: f32 = 0.995;
/// RemoteFX progressive tile side: the grid stale content and seams align to.
const GFX_TILE: usize = 64;
/// A tile counts as stale when this fraction of its pixels still match the stretch.
const TILE_STALE_FRACTION: f32 = 0.98;
/// Tiles flatter than this (max-min per channel) carry no evidence either way.
const FLAT_TILE_RANGE: u8 = 12;

fn main() -> anyhow::Result<()> {
    let config = match parse_args() {
        Ok(Some(config)) => config,
        Ok(None) => {
            println!("{HELP}");
            return Ok(());
        }
        Err(e) => {
            println!("{HELP}");
            return Err(e.context("invalid argument(s)"));
        }
    };

    setup_logging()?;

    if let Some(dir) = &config.out_dir {
        std::fs::create_dir_all(dir).context("create output directory")?;
    }

    let failures = run(config)?;

    if failures > 0 {
        anyhow::bail!("{failures} step(s) exceeded the black-tile or stale-tile threshold");
    }

    Ok(())
}

#[derive(Debug)]
struct Config {
    host: String,
    port: u16,
    no_credssp: bool,
    autologon: bool,
    username: String,
    password: String,
    domain: Option<String>,
    sizes: Vec<(u16, u16)>,
    rounds: u32,
    settle: Duration,
    threshold: f32,
    burst: u32,
    burst_gap: Duration,
    jitter: bool,
    keys: Vec<String>,
    redraw: Redraw,
    out_dir: Option<PathBuf>,
    dump_all: bool,
}

fn parse_args() -> anyhow::Result<Option<Config>> {
    let mut args = pico_args::Arguments::from_env();

    if args.contains(["-h", "--help"]) {
        return Ok(None);
    }

    let password = match args.opt_value_from_str(["-p", "--password"])? {
        Some(password) => password,
        None => std::env::var("RDP_PASSWORD").context("no -p/--password and no RDP_PASSWORD in env")?,
    };

    let sizes = match args.opt_value_from_str::<_, String>("--sizes")? {
        Some(spec) => parse_sizes(&spec)?,
        // Deliberately not round numbers: neither dimension is a multiple of the 64-pixel
        // progressive tile, so every resize leaves partial tiles at the right and bottom
        // edges — where stale content and seams show up first. A 1920x1080-style pair
        // divides evenly and hides exactly the bug this harness looks for.
        None => vec![(1558, 964), (1828, 964)],
    };

    let keys = match args.opt_value_from_str::<_, String>("--keys")? {
        Some(spec) => spec.split(',').map(|k| k.trim().to_lowercase()).collect(),
        None if args.contains("--cad") => vec!["ctrl+alt+del".to_owned()],
        None => Vec::new(),
    };

    let redraw = match args.opt_value_from_str::<_, String>("--redraw")?.as_deref() {
        None | Some("none") => Redraw::None,
        Some("auto") => Redraw::Auto,
        Some("refresh") => Redraw::RefreshRect,
        Some("suppress") => Redraw::SuppressOutput,
        Some(other) => anyhow::bail!("unknown --redraw mode: {other}"),
    };

    Ok(Some(Config {
        host: args.value_from_str("--host")?,
        port: args.opt_value_from_str("--port")?.unwrap_or(3389),
        no_credssp: args.contains("--no-credssp"),
        autologon: args.contains("--autologon"),
        username: args.value_from_str(["-u", "--username"])?,
        password,
        domain: args.opt_value_from_str(["-d", "--domain"])?,
        sizes,
        rounds: args.opt_value_from_str("--rounds")?.unwrap_or(5),
        settle: Duration::from_millis(args.opt_value_from_str("--settle-ms")?.unwrap_or(15_000)),
        threshold: args.opt_value_from_str("--threshold")?.unwrap_or(6.0),
        burst: args.opt_value_from_str("--burst")?.unwrap_or(1).max(1),
        burst_gap: Duration::from_millis(args.opt_value_from_str("--burst-gap-ms")?.unwrap_or(120)),
        jitter: args.contains("--jitter"),
        keys,
        redraw,
        out_dir: args.opt_value_from_str("--out-dir")?,
        dump_all: args.contains("--dump-all"),
    }))
}

/// How to nudge the server into repainting after a resolution change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Redraw {
    None,
    Auto,
    RefreshRect,
    SuppressOutput,
}

fn parse_sizes(spec: &str) -> anyhow::Result<Vec<(u16, u16)>> {
    let sizes = spec
        .split(',')
        .map(|entry| {
            let (w, h) = entry
                .trim()
                .split_once(['x', 'X'])
                .with_context(|| format!("expected WxH, got {entry}"))?;
            Ok((w.trim().parse()?, h.trim().parse()?))
        })
        .collect::<anyhow::Result<Vec<(u16, u16)>>>()?;

    anyhow::ensure!(!sizes.is_empty(), "--sizes is empty");

    Ok(sizes)
}

fn setup_logging() -> anyhow::Result<()> {
    use tracing::metadata::LevelFilter;
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

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

/// EGFX pipeline counters, so a black frame can be attributed to what the server did.
#[derive(Default)]
struct Stats {
    resets: AtomicU32,
    surfaces_created: AtomicU32,
    surfaces_deleted: AtomicU32,
}

impl Stats {
    fn snapshot(&self) -> (u32, u32, u32) {
        (
            self.resets.load(Ordering::Relaxed),
            self.surfaces_created.load(Ordering::Relaxed),
            self.surfaces_deleted.load(Ordering::Relaxed),
        )
    }
}

struct EgfxHandler {
    stats: Arc<Stats>,
}

/// Everything the pump needs besides the wire: counters, and the ability to reactivate.
struct Ctx {
    stats: Arc<Stats>,
    activation: ConnectionActivationFactory,
    refresh_rect_support: bool,
    suppress_output_support: bool,
}

impl GraphicsPipelineHandler for EgfxHandler {
    fn on_reset_graphics(&mut self, width: u32, height: u32) {
        self.stats.resets.fetch_add(1, Ordering::Relaxed);
        debug!(width, height, "ResetGraphics");
    }

    fn on_surface_created(&mut self, _surface: &Surface) {
        self.stats.surfaces_created.fetch_add(1, Ordering::Relaxed);
    }

    fn on_surface_deleted(&mut self, _surface_id: u16) {
        self.stats.surfaces_deleted.fetch_add(1, Ordering::Relaxed);
    }
}

fn run(config: Config) -> anyhow::Result<u32> {
    let stats = Arc::new(Stats::default());

    let connector_config = build_config(&config);
    let (connection_result, mut framed) =
        connect(connector_config, &config.host, config.port, Arc::clone(&stats)).context("connect")?;

    println!(
        "connected desktop={}x{} gfx={:?}",
        connection_result.desktop_size.width, connection_result.desktop_size.height, connection_result.compression_type
    );

    let mut image = DecodedImage::new(
        ironrdp_graphics::image_processing::PixelFormat::RgbA32,
        connection_result.desktop_size.width,
        connection_result.desktop_size.height,
    );

    let ctx = Ctx {
        stats: Arc::clone(&stats),
        activation: connection_result.activation_factory,
        refresh_rect_support: connection_result.refresh_rect_support,
        suppress_output_support: connection_result.suppress_output_support,
    };

    let mut stage = ActiveStageBuilder {
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

    // Shorten the read timeout now that CredSSP is done: idle detection drives every wait below.
    set_read_timeout(&mut framed, Duration::from_millis(400))?;

    let mut failures = 0;
    // `gate` is false for steps whose result is not the client's to get right: the
    // Ctrl+Alt+Del security desktop, for one, is legitimately an all-black screen.
    let mut grade = |step: &str, measured: &Measurement, image: &DecodedImage, gate: bool| -> anyhow::Result<()> {
        report(step, measured);
        let failed = measured.failed(&config);
        if failed && gate {
            failures += 1;
        }
        if let Some(dir) = &config.out_dir {
            if config.dump_all || failed {
                save_png(image, dir, step)?;
            }
        }
        Ok(())
    };

    let first = settle(&mut framed, &mut stage, &mut image, &config, None, &ctx)?;
    grade("first-frame", &first, &image, true)?;

    let mut next_size = 0;
    for round in 1..=config.rounds {
        for key in &config.keys {
            let before = snapshot(&image);
            send_keys(&mut framed, &mut stage, &mut image, key)?;
            let measured = settle(&mut framed, &mut stage, &mut image, &config, Some(&before), &ctx)?;
            grade(&format!("round{round}-{key}"), &measured, &image, false)?;
        }

        for _ in 0..config.sizes.len() {
            let (width, height) = config.sizes[next_size % config.sizes.len()];
            next_size += 1;
            let width = if config.jitter {
                jitter_width(width, &config)
            } else {
                width
            };

            let before = snapshot(&image);
            let resets_before = stats.resets.load(Ordering::Relaxed);
            resize_burst(
                &mut framed,
                &mut stage,
                &mut image,
                width,
                height,
                config.burst,
                config.burst_gap,
                &ctx,
            )?;

            if config.redraw != Redraw::None {
                // Let ResetGraphics land first, otherwise the server answers the redraw
                // request against the old desktop size.
                pump(&mut framed, &mut stage, &mut image, Duration::from_millis(1200), &ctx)?;
                request_redraw(&mut framed, &stage, &image, &ctx, config.redraw)?;
            }

            // A burst chains several resets, so the stretched reference is no longer
            // bit-exact and the stale metric would lie: only grade it on single resets.
            let reference = (config.burst == 1).then_some(&before);
            let mut measured = settle(&mut framed, &mut stage, &mut image, &config, reference, &ctx)?;
            // Resets can land while the burst is still being sent, before settle starts watching.
            measured.resets_during = stats.resets.load(Ordering::Relaxed) - resets_before;
            grade(
                &format!("round{round}-resize-{width}x{height}"),
                &measured,
                &image,
                true,
            )?;
        }
    }

    let (resets, created, deleted) = stats.snapshot();
    println!("--- summary: failures={failures} resets={resets} surfaces_created={created} surfaces_deleted={deleted}");

    Ok(failures)
}

/// A copy of the framebuffer, used as the "what did the previous frame look like" reference.
struct Frame {
    data: Vec<u8>,
    width: usize,
    height: usize,
}

fn snapshot(image: &DecodedImage) -> Frame {
    Frame {
        data: image.data().to_vec(),
        width: usize::from(image.width()),
        height: usize::from(image.height()),
    }
}

/// One grading of the decoded framebuffer.
#[derive(Debug)]
struct Measurement {
    size: (u16, u16),
    black_tile_pct: f32,
    black_pixel_pct: f32,
    /// Columns that are black for more than half the rows, as pixel ranges.
    black_bands: Vec<(usize, usize)>,
    /// Share of textured tiles still identical to the stretched previous frame.
    stale_tile_pct: Option<f32>,
    stale_bands: Vec<(usize, usize)>,
    seam: f32,
    pdus: u32,
    graphics_updates: u32,
    reactivations: u32,
    waited: Duration,
    resets_during: u32,
}

impl Measurement {
    /// Whether the picture is unacceptable, by the same rule the settle loop uses to
    /// decide it is done waiting.
    ///
    /// Stale tiles count, not just black ones. Stale *is* the tearing this harness
    /// exists to catch: a resize that leaves the previous frame stretched over the new
    /// desktop is fully painted and perfectly non-black, so grading on blackness alone
    /// reports success on exactly the bug under test.
    fn failed(&self, config: &Config) -> bool {
        self.black_tile_pct > config.threshold || self.stale_tile_pct.is_some_and(|stale| stale > config.threshold)
    }
}

fn report(step: &str, m: &Measurement) {
    println!(
        "{step}: {}x{} black_tiles={:.2}% black_px={:.2}% black_bands={} stale_tiles={} stale_bands={} seam={:.2} pdus={} gfx={} resets={} reactivations={} waited={}ms",
        m.size.0,
        m.size.1,
        m.black_tile_pct,
        m.black_pixel_pct,
        fmt_bands(&m.black_bands),
        m.stale_tile_pct
            .map_or_else(|| String::from("n/a"), |p| format!("{p:.2}%")),
        fmt_bands(&m.stale_bands),
        m.seam,
        m.pdus,
        m.graphics_updates,
        m.resets_during,
        m.reactivations,
        m.waited.as_millis(),
    );
}

fn fmt_bands(bands: &[(usize, usize)]) -> String {
    if bands.is_empty() {
        return String::from("-");
    }
    bands
        .iter()
        .map(|(x0, x1)| format!("{x0}..{x1}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Pick a width near `target`, the way a dragged window edge lands on arbitrary sizes.
fn jitter_width(target: u16, config: &Config) -> u16 {
    use rand::Rng as _;
    let low = config.sizes.iter().map(|(w, _)| *w).min().unwrap_or(target);
    let high = config.sizes.iter().map(|(w, _)| *w).max().unwrap_or(target);
    let span = i32::from(high.saturating_sub(low)).max(4);
    let delta = rand::rng().random_range(-span / 4..=span / 4);
    i32::from(target)
        .saturating_add(delta)
        .clamp(i32::from(low), i32::from(high))
        .try_into()
        .unwrap_or(target)
}

/// Fire several Display Control requests back to back, ending on the target size.
///
/// A single request is the easy case; the interesting one is a storm of them, where
/// a later ResetGraphics can land while the framebuffer is still being repainted.
#[expect(clippy::too_many_arguments, reason = "stress knobs, all independent")]
fn resize_burst(
    framed: &mut UpgradedFramed,
    stage: &mut ActiveStage,
    image: &mut DecodedImage,
    width: u16,
    height: u16,
    burst: u32,
    gap: Duration,
    ctx: &Ctx,
) -> anyhow::Result<()> {
    for step in 0..burst {
        // Intermediate steps walk toward the target, the last one nails it exactly.
        // Repeating the target size also covers the same-size ResetGraphics case.
        let intermediate = if step + 1 == burst {
            width
        } else {
            let offset = u16::try_from((burst - step - 1) * 24).unwrap_or(0);
            width.saturating_add(offset).max(640)
        };

        request_resize(framed, stage, image, u32::from(intermediate), u32::from(height), ctx)?;

        if step + 1 < burst {
            pump(framed, stage, image, gap, ctx)?;
        }
    }

    Ok(())
}

/// Ask the server to repaint the whole desktop.
///
/// The web client deliberately skips this after ResetGraphics, on the theory that with
/// RDPGFX these PDUs do not invalidate the surface cache. This switch exists to check
/// that theory against a real server instead of taking it on faith.
fn request_redraw(
    framed: &mut UpgradedFramed,
    stage: &ActiveStage,
    image: &DecodedImage,
    ctx: &Ctx,
    mode: Redraw,
) -> anyhow::Result<()> {
    let (refresh, suppress) = match mode {
        Redraw::None => return Ok(()),
        Redraw::Auto => (ctx.refresh_rect_support, ctx.suppress_output_support),
        Redraw::RefreshRect => (true, false),
        Redraw::SuppressOutput => (false, true),
    };

    let redraw_frames = stage
        .request_full_redraw(image.width(), image.height(), refresh, suppress)
        .context("encode full redraw request")?;

    if redraw_frames.is_empty() {
        println!("   redraw requested but the server advertised neither Refresh Rect nor Suppress Output");
    }

    for frame in redraw_frames {
        framed.write_all(&frame).context("write redraw request")?;
    }

    Ok(())
}

/// Inject a key combo. Ctrl+Alt+Del drives the server through a full output reset;
/// Ctrl+Esc is the cheap way to prove the input path works at all (Start menu).
fn send_keys(
    framed: &mut UpgradedFramed,
    stage: &mut ActiveStage,
    image: &mut DecodedImage,
    combo: &str,
) -> anyhow::Result<()> {
    const CTRL: (u8, bool) = (0x1D, false);
    const ALT: (u8, bool) = (0x38, false);
    const DELETE: (u8, bool) = (0x53, true);
    const ESCAPE: (u8, bool) = (0x01, false);
    const WIN: (u8, bool) = (0x5B, true);

    let keys: &[(u8, bool)] = match combo {
        "ctrl+alt+del" | "cad" => &[CTRL, ALT, DELETE],
        "ctrl+esc" => &[CTRL, ESCAPE],
        "win" => &[WIN],
        "esc" => &[ESCAPE],
        other => anyhow::bail!("unknown key combo: {other}"),
    };

    let flags = |extended: bool, release: bool| {
        let mut flags = KeyboardFlags::empty();
        if extended {
            flags |= KeyboardFlags::EXTENDED;
        }
        if release {
            flags |= KeyboardFlags::RELEASE;
        }
        flags
    };

    let mut events: Vec<FastPathInputEvent> = keys
        .iter()
        .map(|&(code, ext)| FastPathInputEvent::KeyboardEvent(flags(ext, false), code))
        .collect();
    events.extend(
        keys.iter()
            .rev()
            .map(|&(code, ext)| FastPathInputEvent::KeyboardEvent(flags(ext, true), code)),
    );

    for out in stage
        .process_fastpath_input(image, &events)
        .with_context(|| format!("encode {combo}"))?
    {
        if let ActiveStageOutput::ResponseFrame(frame) = out {
            framed.write_all(&frame).with_context(|| format!("write {combo}"))?;
        }
    }

    Ok(())
}

/// Ask the server for a new desktop size over Display Control, retrying until the
/// channel reports its capabilities.
fn request_resize(
    framed: &mut UpgradedFramed,
    stage: &mut ActiveStage,
    image: &mut DecodedImage,
    width: u32,
    height: u32,
    ctx: &Ctx,
) -> anyhow::Result<()> {
    for attempt in 1..=30 {
        match stage.encode_resize(width, height, None, None) {
            Some(frame) => {
                let frame = frame.context("encode Display Control resize")?;
                framed.write_all(&frame).context("write resize")?;
                debug!(width, height, attempt, "Display Control resize sent");
                return Ok(());
            }
            None => {
                // Capabilities not in yet: keep the session moving and try again.
                pump(framed, stage, image, Duration::from_millis(500), ctx)?;
            }
        }
    }

    anyhow::bail!("Display Control never became ready; cannot drive a resize")
}

/// Pump the session until the picture is healthy again, or until the settle budget runs out.
fn settle(
    framed: &mut UpgradedFramed,
    stage: &mut ActiveStage,
    image: &mut DecodedImage,
    config: &Config,
    reference: Option<&Frame>,
    ctx: &Ctx,
) -> anyhow::Result<Measurement> {
    let started = Instant::now();
    let resets_before = ctx.stats.resets.load(Ordering::Relaxed);
    let mut pdus = 0;
    let mut graphics_updates = 0;
    let mut reactivations = 0;

    loop {
        let outcome = pump(framed, stage, image, Duration::from_millis(500), ctx)?;
        pdus += outcome.pdus;
        graphics_updates += outcome.graphics_updates;
        reactivations += outcome.reactivations;

        let elapsed = started.elapsed();
        let mut measured = measure(image, reference);
        measured.pdus = pdus;
        measured.graphics_updates = graphics_updates;
        measured.reactivations = reactivations;
        measured.waited = elapsed;
        measured.resets_during = ctx.stats.resets.load(Ordering::Relaxed) - resets_before;

        if outcome.terminated {
            anyhow::bail!("server terminated the session after {}ms", elapsed.as_millis());
        }

        // Stop as soon as the picture looks finished, but only after the server had a
        // chance to speak: an immediate sample still shows the pre-change frame.
        let healthy = !measured.failed(config);

        if elapsed >= budget_floor() && (healthy || elapsed >= config.settle) {
            return Ok(measured);
        }
    }
}

/// Minimum time to keep watching, so an early healthy sample cannot end the step.
fn budget_floor() -> Duration {
    Duration::from_secs(3)
}

#[derive(Default)]
struct PumpOutcome {
    pdus: u32,
    graphics_updates: u32,
    reactivations: u32,
    terminated: bool,
}

/// Run the Deactivation-Reactivation Sequence the server asked for.
///
/// Ctrl+Alt+Del switches the server to the secure desktop, which deactivates the
/// share. A client that ignores this gets no further output at all and keeps showing
/// a stretched copy of the last frame, so the harness has to do what a real client does.
fn reactivate(
    framed: &mut UpgradedFramed,
    stage: &mut ActiveStage,
    image: &mut DecodedImage,
    factory: &ConnectionActivationFactory,
) -> anyhow::Result<()> {
    use ironrdp::connector::Sequence as _;

    // Reactivation is a request/response handshake; the idle-poll timeout would break it.
    set_read_timeout(framed, Duration::from_secs(10))?;
    let restore = |framed: &mut UpgradedFramed| set_read_timeout(framed, Duration::from_millis(400));

    let mut sequence = factory.create();
    let mut buf = WriteBuf::new();

    loop {
        buf.clear();

        let written = match sequence.next_pdu_hint() {
            Some(hint) => {
                let pdu = framed.read_by_hint(hint).context("read activation PDU")?;
                sequence.step(&pdu, framed.last_read_at(), &mut buf)
            }
            None => sequence.step_no_input(&mut buf),
        }
        .context("activation step")?;

        if let Some(len) = written.size() {
            framed.write_all(&buf[..len]).context("write activation response")?;
        }

        if let ConnectionActivationState::Finalized {
            desktop_size,
            share_id,
            enable_server_pointer,
            pointer_software_rendering,
            static_channel_chunk_size,
            window_support_level,
            ..
        } = sequence.connection_activation_state()
        {
            // Start the new desktop empty rather than carrying the old frame across:
            // the harness must not invent content it then measures.
            *image = DecodedImage::new(
                ironrdp_graphics::image_processing::PixelFormat::RgbA32,
                desktop_size.width,
                desktop_size.height,
            );
            if !stage.reactivate(
                sequence.io_channel_id(),
                sequence.user_channel_id(),
                share_id,
                enable_server_pointer,
                pointer_software_rendering,
                static_channel_chunk_size,
            ) {
                restore(framed)?;
                anyhow::bail!("invalid static channel chunk size during reactivation");
            }
            stage.set_window_support_level(window_support_level);
            debug!(?desktop_size, "reactivated");
            restore(framed)?;
            return Ok(());
        }
    }
}

/// Read and process PDUs for at most `budget`, answering with any response frames.
fn pump(
    framed: &mut UpgradedFramed,
    stage: &mut ActiveStage,
    image: &mut DecodedImage,
    budget: Duration,
    ctx: &Ctx,
) -> anyhow::Result<PumpOutcome> {
    let started = Instant::now();
    let mut outcome = PumpOutcome::default();

    while started.elapsed() < budget {
        let (action, payload) = match framed.read_pdu() {
            Ok(pdu) => pdu,
            Err(e) if is_idle(&e) => break,
            Err(e) => return Err(anyhow::Error::new(e).context("read frame")),
        };

        outcome.pdus += 1;

        for out in stage.process(image, action, &payload).context("process frame")? {
            match out {
                ActiveStageOutput::ResponseFrame(frame) => framed.write_all(&frame).context("write response")?,
                ActiveStageOutput::GraphicsUpdate(_) => outcome.graphics_updates += 1,
                ActiveStageOutput::DeactivateAll => {
                    reactivate(framed, stage, image, &ctx.activation).context("reactivate")?;
                    outcome.reactivations += 1;
                }
                ActiveStageOutput::Terminate(_) => {
                    outcome.terminated = true;
                    return Ok(outcome);
                }
                _ => {}
            }
        }
    }

    Ok(outcome)
}

fn is_idle(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted
    )
}

/// Grade the framebuffer: black content, stale content, and tile-grid seams.
///
/// Deliberately written from scratch instead of reusing anything from
/// `ironrdp-session`, so the measurement cannot be fooled by the code under test.
fn measure(image: &DecodedImage, reference: Option<&Frame>) -> Measurement {
    let current = Frame {
        data: image.data().to_vec(),
        width: usize::from(image.width()),
        height: usize::from(image.height()),
    };

    let mut measurement = Measurement {
        size: (image.width(), image.height()),
        black_tile_pct: 100.0,
        black_pixel_pct: 100.0,
        black_bands: Vec::new(),
        stale_tile_pct: None,
        stale_bands: Vec::new(),
        seam: 0.0,
        pdus: 0,
        graphics_updates: 0,
        reactivations: 0,
        waited: Duration::ZERO,
        resets_during: 0,
    };

    if current.width == 0 || current.height == 0 || current.data.len() < current.width * current.height * 4 {
        return measurement;
    }

    let (black_tile_pct, black_pixel_pct, black_bands) = black_stats(&current);
    measurement.black_tile_pct = black_tile_pct;
    measurement.black_pixel_pct = black_pixel_pct;
    measurement.black_bands = black_bands;
    measurement.seam = seam_score(&current);

    if let Some(previous) = reference {
        let stretched = stretch(previous, current.width, current.height);
        let (stale_pct, stale_bands) = stale_stats(&current, &stretched);
        measurement.stale_tile_pct = Some(stale_pct);
        measurement.stale_bands = stale_bands;
    }

    measurement
}

fn black_stats(frame: &Frame) -> (f32, f32, Vec<(usize, usize)>) {
    let is_black = |x: usize, y: usize| {
        let i = (y * frame.width + x) * 4;
        frame.data[i + 3] == 0 || (frame.data[i] == 0 && frame.data[i + 1] == 0 && frame.data[i + 2] == 0)
    };

    let cols = frame.width / TILE;
    let rows = frame.height / TILE;
    let mut black_tiles = 0usize;
    let mut black_pixels = 0usize;
    let mut col_black = vec![0usize; cols];

    for row in 0..rows {
        for (col, black_rows) in col_black.iter_mut().enumerate() {
            let mut black = 0usize;
            for y in 0..TILE {
                for x in 0..TILE {
                    if is_black(col * TILE + x, row * TILE + y) {
                        black += 1;
                    }
                }
            }
            black_pixels += black;
            if black as f32 >= (TILE * TILE) as f32 * TILE_BLACK_FRACTION {
                black_tiles += 1;
                *black_rows += 1;
            }
        }
    }

    let tiles = (cols * rows).max(1);
    let sampled = (tiles * TILE * TILE) as f32;

    (
        black_tiles as f32 * 100.0 / tiles as f32,
        black_pixels as f32 * 100.0 / sampled,
        column_bands(&col_black, rows, TILE),
    )
}

/// Nearest-neighbour stretch, matching what the session does to carry a frame across
/// a resolution change: a pixel that still matches it was never repainted.
fn stretch(previous: &Frame, width: usize, height: usize) -> Frame {
    let mut data = vec![0u8; width * height * 4];
    if previous.width == 0 || previous.height == 0 || width == 0 || height == 0 {
        return Frame { data, width, height };
    }

    for y in 0..height {
        let src_y = y * previous.height / height;
        for x in 0..width {
            let src_x = x * previous.width / width;
            let src = (src_y * previous.width + src_x) * 4;
            let dst = (y * width + x) * 4;
            data[dst..dst + 4].copy_from_slice(&previous.data[src..src + 4]);
        }
    }

    Frame { data, width, height }
}

/// Share of textured 64x64 tiles still identical to the stretched previous frame.
///
/// Flat tiles (a plain wallpaper, a black band) are skipped: a correct repaint of a
/// flat area is indistinguishable from a stale one, so they carry no evidence.
fn stale_stats(current: &Frame, stretched: &Frame) -> (f32, Vec<(usize, usize)>) {
    let cols = current.width / GFX_TILE;
    let rows = current.height / GFX_TILE;
    let mut considered = 0usize;
    let mut stale = 0usize;
    let mut col_stale = vec![0usize; cols];

    for row in 0..rows {
        for (col, stale_rows) in col_stale.iter_mut().enumerate() {
            let mut matching = 0usize;
            let mut min = [255u8; 3];
            let mut max = [0u8; 3];

            for y in 0..GFX_TILE {
                for x in 0..GFX_TILE {
                    let i = ((row * GFX_TILE + y) * current.width + col * GFX_TILE + x) * 4;
                    for channel in 0..3 {
                        min[channel] = min[channel].min(current.data[i + channel]);
                        max[channel] = max[channel].max(current.data[i + channel]);
                    }
                    if current.data[i..i + 3] == stretched.data[i..i + 3] {
                        matching += 1;
                    }
                }
            }

            let flat = (0..3).all(|c| max[c].saturating_sub(min[c]) <= FLAT_TILE_RANGE);
            if flat {
                continue;
            }

            considered += 1;
            if matching as f32 >= (GFX_TILE * GFX_TILE) as f32 * TILE_STALE_FRACTION {
                stale += 1;
                *stale_rows += 1;
            }
        }
    }

    if considered == 0 {
        return (0.0, Vec::new());
    }

    (
        stale as f32 * 100.0 / considered as f32,
        column_bands(&col_stale, rows, GFX_TILE),
    )
}

/// Edge energy on the 64-pixel tile grid relative to tile interiors.
///
/// A correct picture has no idea where the tile grid is, so the ratio sits near 1.
/// Mismatched or half-updated tiles paint a visible grid and push it up.
fn seam_score(frame: &Frame) -> f32 {
    let luma = |x: usize, y: usize| {
        let i = (y * frame.width + x) * 4;
        i32::from(frame.data[i]) * 299 + i32::from(frame.data[i + 1]) * 587 + i32::from(frame.data[i + 2]) * 114
    };

    let mut boundary = 0i64;
    let mut boundary_n = 0i64;
    let mut interior = 0i64;
    let mut interior_n = 0i64;

    for y in 0..frame.height {
        for x in 1..frame.width {
            let delta = i64::from((luma(x, y) - luma(x - 1, y)).abs());
            if x % GFX_TILE == 0 {
                boundary += delta;
                boundary_n += 1;
            } else if x % GFX_TILE == GFX_TILE / 2 {
                interior += delta;
                interior_n += 1;
            }
        }
    }

    if boundary_n == 0 || interior_n == 0 || interior == 0 {
        return 0.0;
    }

    (boundary as f64 / boundary_n as f64 / (interior as f64 / interior_n as f64)) as f32
}

/// Collapse per-column tile counts into pixel ranges where most rows are affected.
fn column_bands(col_counts: &[usize], rows: usize, tile: usize) -> Vec<(usize, usize)> {
    let mut bands = Vec::new();
    let affected = |col: usize| rows > 0 && col_counts[col] * 2 > rows;

    let mut col = 0;
    while col < col_counts.len() {
        if affected(col) {
            let start = col;
            while col < col_counts.len() && affected(col) {
                col += 1;
            }
            bands.push((start * tile, col * tile));
        } else {
            col += 1;
        }
    }

    bands
}

fn save_png(image: &DecodedImage, dir: &Path, step: &str) -> anyhow::Result<()> {
    let buffer: image::ImageBuffer<image::Rgba<u8>, _> =
        image::ImageBuffer::from_raw(u32::from(image.width()), u32::from(image.height()), image.data())
            .context("invalid image")?;
    let path = dir.join(format!("{step}.png"));
    buffer.save(&path).context("save image to disk")?;
    println!("   dumped {}", path.display());
    Ok(())
}

fn build_config(config: &Config) -> connector::Config {
    connector::Config {
        credentials: Credentials::UsernamePassword {
            username: config.username.clone(),
            password: config.password.clone(),
        },
        domain: config.domain.clone(),
        // xrdp defaults to security_layer=tls and speaks no CredSSP, so it needs plain
        // TLS: keeping NLA on makes the handshake fail before a single frame arrives.
        enable_tls: config.no_credssp,
        enable_credssp: !config.no_credssp,
        enable_standard_rdp_security: false,
        keyboard_type: KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_layout: 0,
        keyboard_functional_keys_count: 12,
        connection_type: ConnectionType::Lan,
        ime_file_name: String::new(),
        dig_product_id: String::new(),
        desktop_size: connector::DesktopSize {
            width: config.sizes[0].0,
            height: config.sizes[0].1,
        },
        monitor_layout: None,
        bitmap: None,
        client_build: 0,
        client_name: "ironrdp-rdp-stress".to_owned(),
        client_dir: "C:\\Windows\\System32\\mstscax.dll".to_owned(),

        #[cfg(windows)]
        platform: MajorPlatformType::WINDOWS,
        #[cfg(target_os = "macos")]
        platform: MajorPlatformType::MACINTOSH,
        #[cfg(target_os = "linux")]
        platform: MajorPlatformType::UNIX,

        enable_server_pointer: false,
        request_data: None,
        autologon: config.autologon,
        enable_audio_playback: false,
        enable_audio_capture: false,
        compression_type: Some(CompressionType::Rdp61),
        pointer_software_rendering: true,
        multitransport_flags: None,
        // The whole point of this harness: exercise the EGFX / ResetGraphics path.
        support_dyn_vc_gfx_protocol: true,
        performance_flags: PerformanceFlags::default(),
        desktop_scale_factor: 0,
        hardware_id: None,
        license_cache: None,
        timezone_info: TimezoneInfo::default(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        remote_application_mode: false,
        rail_support_level: ironrdp_pdu::rdp::capability_sets::RailSupportLevel::empty(),
    }
}

type UpgradedFramed = ironrdp_blocking::Framed<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>;

fn set_read_timeout(framed: &mut UpgradedFramed, timeout: Duration) -> anyhow::Result<()> {
    let (stream, _) = framed.get_inner_mut();
    stream.sock.set_read_timeout(Some(timeout)).context("set_read_timeout")
}

fn connect(
    config: connector::Config,
    server_name: &str,
    port: u16,
    stats: Arc<Stats>,
) -> anyhow::Result<(ConnectionResult, UpgradedFramed)> {
    let server_addr = lookup_addr(server_name, port).context("lookup addr")?;

    info!(%server_addr, "Looked up server address");

    let tcp_stream = TcpStream::connect(server_addr).context("TCP connect")?;
    // An emulated (amd64-on-arm64) xrdp box can take ~30s just to answer the TLS
    // negotiation, so the handshake needs far more slack than a frame read.
    tcp_stream
        .set_read_timeout(Some(Duration::from_secs(90)))
        .context("set_read_timeout")?;

    let client_addr = tcp_stream.local_addr().context("get socket local address")?;

    let mut framed = ironrdp_blocking::Framed::new(tcp_stream);

    // EGFX plus Display Control is what a modern client (and MSRDC) negotiates; without
    // both, a resize never reaches ResetGraphics and there is nothing to reproduce.
    let drdynvc = DrdynvcClient::new()
        .with_dynamic_channel(DisplayControlClient::new(|_| Ok(Vec::new())))
        .with_dynamic_channel(GraphicsPipelineClient::new(Box::new(EgfxHandler { stats }), None));

    let mut connector = connector::ClientConnector::new(config, client_addr).with_static_channel(drdynvc);

    let should_upgrade = ironrdp_blocking::connect_begin(&mut framed, &mut connector).context("begin connection")?;

    debug!("TLS upgrade");

    let initial_stream = framed.into_inner_no_leftover();
    let (upgraded_stream, server_public_key) =
        tls_upgrade(initial_stream, server_name.to_owned()).context("TLS upgrade")?;

    let upgraded = ironrdp_blocking::mark_as_upgraded(should_upgrade, &mut connector);

    let mut upgraded_framed = ironrdp_blocking::Framed::new(upgraded_stream);

    let mut network_client = ReqwestNetworkClient;
    let connection_result = ironrdp_blocking::connect_finalize(
        upgraded,
        connector,
        &mut upgraded_framed,
        &mut network_client,
        server_name.to_owned().into(),
        server_public_key,
        None,
    )
    .context("finalize connection")?;

    Ok((connection_result, upgraded_framed))
}

fn lookup_addr(hostname: &str, port: u16) -> anyhow::Result<core::net::SocketAddr> {
    use std::net::ToSocketAddrs as _;
    let addr = (hostname, port)
        .to_socket_addrs()?
        .next()
        .context("socket address not found")?;
    Ok(addr)
}

fn tls_upgrade(
    stream: TcpStream,
    server_name: String,
) -> anyhow::Result<(rustls::StreamOwned<rustls::ClientConnection, TcpStream>, Vec<u8>)> {
    let mut config = rustls::client::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(danger::NoCertificateVerification))
        .with_no_client_auth();

    config.key_log = Arc::new(rustls::KeyLogFile::new());
    config.resumption = rustls::client::Resumption::disabled();

    let config = Arc::new(config);

    let server_name = server_name.try_into()?;

    let client = rustls::ClientConnection::new(config, server_name)?;

    let mut tls_stream = rustls::StreamOwned::new(client, stream);

    tls_stream.flush()?;

    let cert = tls_stream
        .conn
        .peer_certificates()
        .and_then(|certificates| certificates.first())
        .context("peer certificate is missing")?;

    let server_public_key = extract_tls_server_public_key(cert)?;

    Ok((tls_stream, server_public_key))
}

fn extract_tls_server_public_key(cert: &[u8]) -> anyhow::Result<Vec<u8>> {
    use x509_cert::der::Decode as _;

    let cert = x509_cert::Certificate::from_der(cert)?;

    debug!(subject = %cert.tbs_certificate().subject());

    let server_public_key = cert
        .tbs_certificate()
        .subject_public_key_info()
        .subject_public_key
        .as_bytes()
        .context("subject public key BIT STRING is not aligned")?
        .to_owned();

    Ok(server_public_key)
}

mod danger {
    use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use tokio_rustls::rustls::{DigitallySignedStruct, Error, SignatureScheme, pki_types};

    #[derive(Debug)]
    pub(super) struct NoCertificateVerification;

    impl ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _: &pki_types::CertificateDer<'_>,
            _: &[pki_types::CertificateDer<'_>],
            _: &pki_types::ServerName<'_>,
            _: &[u8],
            _: pki_types::UnixTime,
        ) -> Result<ServerCertVerified, Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &pki_types::CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &pki_types::CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            vec![
                SignatureScheme::RSA_PKCS1_SHA1,
                SignatureScheme::ECDSA_SHA1_Legacy,
                SignatureScheme::RSA_PKCS1_SHA256,
                SignatureScheme::ECDSA_NISTP256_SHA256,
                SignatureScheme::RSA_PKCS1_SHA384,
                SignatureScheme::ECDSA_NISTP384_SHA384,
                SignatureScheme::RSA_PKCS1_SHA512,
                SignatureScheme::ECDSA_NISTP521_SHA512,
                SignatureScheme::RSA_PSS_SHA256,
                SignatureScheme::RSA_PSS_SHA384,
                SignatureScheme::RSA_PSS_SHA512,
                SignatureScheme::ED25519,
                SignatureScheme::ED448,
            ]
        }
    }
}
