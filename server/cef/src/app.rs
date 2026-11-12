use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use anyhow::Context as _;
use cef::{ImplBrowser as _, ImplBrowserHost as _, ImplFrame as _};
use clap::Parser;
#[cfg(unix)]
use nix::sys::signal::{SigSet, SigmaskHow, Signal};
use tokio::sync::watch;

use crate::clipboard::{Clipboard, Snapshot};
use crate::download::{Limits, runtime::Downloads};
use crate::model::{Action, Model};
use crate::private_clipboard::PrivateClipboard;
use crate::remote_devices::{RemoteAuthenticator, RemoteMedia};
use crate::{input, native, platform, rdp};

#[cfg(windows)]
static STARTUP_INTERRUPT: AtomicBool = AtomicBool::new(false);

#[derive(Parser)]
#[command(about = "A native CEF browser served directly over loopback RDP")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:3399")]
    bind_addr: SocketAddr,
    #[arg(long, default_value = "about:blank")]
    url: String,
    #[arg(long, default_value_t = 1280)]
    width: u16,
    #[arg(long, default_value_t = 800)]
    height: u16,
    #[arg(long, default_value = "artifacts/run")]
    artifacts: PathBuf,
    #[arg(long)]
    font: Option<PathBuf>,
    #[arg(long)]
    exit_after_seconds: Option<u64>,
    #[arg(long)]
    downloads: bool,
    #[arg(long, default_value_t = 1024 * 1024 * 1024)]
    download_max_file_bytes: u64,
    #[arg(long, default_value_t = 4 * 1024 * 1024 * 1024)]
    download_max_session_bytes: u64,
    #[arg(long, default_value_t = 2)]
    download_max_concurrent: usize,
    /// Redirect clipboard content; the private-clipboard CEF runtime is always required.
    #[arg(long)]
    clipboard: bool,
    /// Feed a synthetic camera, microphone and authenticator in place of the client's devices.
    #[cfg(feature = "device-simulator")]
    #[arg(long)]
    simulate_devices: bool,
}

pub(crate) fn run(main_args: &cef::MainArgs, sandbox_info: *mut u8) -> anyhow::Result<()> {
    #[cfg(windows)]
    install_startup_interrupt_handler()?;
    // UNTESTED: Native initializers must not leave SIGINT unblocked on another thread before this mask.
    // Such a thread can receive an interrupt before cleanup is ready; check startup threads when changing linked frameworks.
    #[cfg(unix)]
    let (interrupt, previous_mask) = {
        let interrupt = SigSet::from(Signal::SIGINT);
        let previous_mask = interrupt
            .thread_swap_mask(SigmaskHow::SIG_BLOCK)
            .context("failed to defer startup interrupt")?;
        (interrupt, previous_mask)
    };
    let args = Args::parse();
    anyhow::ensure!(
        args.bind_addr.ip().is_loopback(),
        "only loopback RDP addresses are supported"
    );
    anyhow::ensure!(native::allowed_url(&args.url), "unsupported initial URL");
    let download_limits = Limits {
        file_bytes: args.download_max_file_bytes,
        session_bytes: args.download_max_session_bytes,
        concurrent: args.download_max_concurrent,
    }
    .validate()?;
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .try_init()
        .map_err(|error| anyhow::anyhow!("failed to initialize logging: {error}"))?;
    let mut model = Model::new(u32::from(args.width), u32::from(args.height))?;
    let font = match &args.font {
        Some(path) => path.clone(),
        None => platform::default_font()?,
    };
    model.set_font(std::fs::read(&font).with_context(|| format!("failed to read toolbar font {}", font.display()))?)?;
    std::fs::create_dir_all(&args.artifacts)?;
    let artifacts = args.artifacts.canonicalize()?;
    #[cfg(windows)]
    let artifacts = {
        // UNTESTED: Converted UNC paths must name the same directory without a verbatim prefix.
        // Check a network share when changing Windows path handling.
        let canonical = artifacts.to_str().context("CEF artifact path must be valid Unicode")?;
        let path = if let Some(path) = canonical.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{path}"))
        } else if let Some(path) = canonical.strip_prefix(r"\\?\") {
            anyhow::ensure!(
                path.as_bytes().get(1) == Some(&b':'),
                "CEF artifact path must resolve to a drive or UNC path"
            );
            PathBuf::from(path)
        } else {
            PathBuf::from(canonical)
        };
        anyhow::ensure!(
            path.canonicalize().context("failed to resolve CEF artifact path")? == artifacts,
            "CEF artifact path changes when removing its verbatim prefix"
        );
        path
    };
    anyhow::ensure!(
        !artifacts.join("server.addr").exists(),
        "artifact directory already contains a server address"
    );
    let profile = tempfile::Builder::new().prefix("profile-").tempdir_in(&artifacts)?;
    std::fs::write(
        artifacts.join("profile.path"),
        profile.path().to_string_lossy().as_bytes(),
    )?;
    tracing::info!(path = %profile.path().display(), "Created fresh browser profile");
    #[cfg(windows)]
    if STARTUP_INTERRUPT.load(Ordering::Acquire) {
        return finish_profile(profile, true);
    }
    #[cfg(windows)]
    let cef_log = {
        let path = artifacts.join("cef.log");
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .context("failed to create CEF log")?;
        path
    };
    #[cfg(not(windows))]
    let cef_log = profile.path().join("cef.log");
    let (downloads, download_worker) = Downloads::new(profile.path(), args.downloads, download_limits)?;
    #[cfg(feature = "device-simulator")]
    let media_capture = args.simulate_devices;
    #[cfg(not(feature = "device-simulator"))]
    let media_capture = false;
    let engine = native::Engine::new(
        main_args,
        sandbox_info,
        profile.path(),
        &cef_log,
        downloads.clone(),
        media_capture,
    );
    let mut engine = match engine {
        Ok(engine) => engine,
        Err(error) => {
            tracing::error!(%error, "CEF browser initialization failed");
            downloads.stop();
            let worker = download_worker
                .join()
                .map_err(|_| anyhow::anyhow!("download worker panicked during browser startup"));
            finish_profile(profile, worker.is_ok())?;
            worker?;
            return Err(error);
        }
    };
    #[cfg(windows)]
    if STARTUP_INTERRUPT.load(Ordering::Acquire) {
        let closed = engine.shutdown();
        downloads.stop();
        let worker = download_worker
            .join()
            .map_err(|_| anyhow::anyhow!("download worker panicked during browser startup"));
        let log = verify_cef_log(&cef_log);
        let cleanup = finish_profile(profile, closed.is_ok() && worker.is_ok() && log.is_ok());
        for (phase, outcome) in [
            ("CEF shutdown", &closed),
            ("download worker", &worker),
            ("CEF log", &log),
            ("profile removal", &cleanup),
        ] {
            if let Err(error) = outcome {
                tracing::error!(phase, %error, "Browser cleanup failed");
            }
        }
        worker?;
        closed?;
        log?;
        return cleanup;
    }
    let content = model.content_dimensions();
    let started = private_clipboard(args.clipboard).and_then(|private| {
        let devices = remote_devices(&args)?;
        engine.create(
            model.active_id(),
            &args.url,
            u16::try_from(content.0)?,
            u16::try_from(content.1)?,
            model.generation(),
        )?;
        Ok((private, devices))
    });
    let (private, devices) = match started {
        Ok(started) => started,
        Err(error) => {
            tracing::error!(%error, "CEF browser startup failed");
            let closed = engine.shutdown();
            if let Err(error) = &closed {
                tracing::error!(%error, "CEF shutdown failed");
            }
            downloads.stop();
            let worker = download_worker
                .join()
                .map_err(|_| anyhow::anyhow!("download worker panicked during browser startup"));
            finish_profile(profile, closed.is_ok() && worker.is_ok())?;
            worker?;
            closed?;
            return Err(error);
        }
    };
    engine.activate(model.active_id());
    let runtime = rdp::Runtime::new();
    #[cfg(unix)]
    let runtime = runtime.and_then(|runtime| {
        if previous_mask.contains(Signal::SIGINT) {
            interrupt.thread_block()
        } else {
            interrupt.thread_unblock()
        }
        .context("failed to restore interrupt mask")?;
        Ok(runtime)
    });
    let runtime = match runtime {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!(%error, "RDP signal initialization failed");
            let closed = engine.shutdown();
            if let Err(error) = &closed {
                tracing::error!(%error, "CEF shutdown failed");
            }
            downloads.stop();
            let worker = download_worker
                .join()
                .map_err(|_| anyhow::anyhow!("download worker panicked during browser startup"));
            finish_profile(profile, closed.is_ok() && worker.is_ok())?;
            worker?;
            closed?;
            return Err(error);
        }
    };
    let initial = rdp::Frame {
        width: args.width,
        height: args.height,
        pixels: model.compose().into(),
    };
    let (frames, receiver) = watch::channel(initial);
    let (sender, commands) = mpsc::sync_channel(256);
    #[cfg(feature = "device-simulator")]
    let simulated_file = if args.simulate_devices && private.is_some() {
        let file = profile.path().join("simulated-clipboard.txt");
        std::fs::write(&file, "remote file ✓").context("failed to stage the simulated clipboard file")?;
        Some(file)
    } else {
        None
    };
    let clipboard = private.map(|private| {
        let (bridge, incoming) = Clipboard::new();
        ClipboardLink {
            private,
            bridge,
            incoming,
            #[cfg(feature = "device-simulator")]
            simulated_file,
        }
    });
    let stop = Arc::new(AtomicBool::new(false));
    let server = rdp::start(
        runtime,
        args.bind_addr,
        artifacts.clone(),
        sender,
        receiver,
        Arc::clone(&stop),
        rdp::Redirection {
            downloads: downloads.clone(),
            clipboard: clipboard.as_ref().map(|link| link.bridge.clone()),
        },
    );
    let mut input = input::Input::default();
    let channels = Channels {
        frames: &frames,
        commands,
        clipboard,
    };
    let result = run_browser(&args, &mut model, &mut engine, &mut input, channels, &stop);
    drop(devices);
    stop.store(true, Ordering::Release);
    input.release_content(&engine);
    downloads.cancel_all();
    let closed = engine.shutdown();
    downloads.stop();
    let worker = download_worker
        .join()
        .map_err(|_| anyhow::anyhow!("download worker panicked"));
    let network = server
        .join()
        .map_err(|_| anyhow::anyhow!("RDP thread panicked"))
        .and_then(|result| result);
    #[cfg(not(windows))]
    let log_preservation = if cef_log.is_file() {
        std::fs::copy(&cef_log, artifacts.join("cef.log"))
            .map(|_| ())
            .context("failed to preserve CEF log")
    } else {
        Ok(())
    };
    #[cfg(windows)]
    let log_preservation = verify_cef_log(&cef_log);
    let profile_cleanup = finish_profile(
        profile,
        closed.is_ok() && worker.is_ok() && network.is_ok() && log_preservation.is_ok(),
    );
    for (phase, outcome) in [
        ("CEF shutdown", &closed),
        ("download worker", &worker),
        ("RDP thread", &network),
        ("CEF log", &log_preservation),
        ("profile removal", &profile_cleanup),
    ] {
        if let Err(error) = outcome {
            tracing::error!(phase, %error, "Browser cleanup failed");
        }
    }
    worker?;
    log_preservation?;
    result?;
    closed?;
    network?;
    profile_cleanup?;
    tracing::info!("Browser server stopped");
    Ok(())
}

#[cfg(windows)]
fn verify_cef_log(path: &std::path::Path) -> anyhow::Result<()> {
    let metadata = std::fs::metadata(path).context("failed to preserve CEF log")?;
    anyhow::ensure!(metadata.is_file(), "CEF log is not a file");
    Ok(())
}

/// Connections between the browser loop and the RDP thread.
struct Channels<'a> {
    frames: &'a watch::Sender<rdp::Frame>,
    commands: mpsc::Receiver<rdp::Command>,
    clipboard: Option<ClipboardLink>,
}

/// Browser side of clipboard redirection, owned by the CEF UI thread.
struct ClipboardLink {
    private: PrivateClipboard,
    bridge: Clipboard,
    incoming: watch::Receiver<Option<Snapshot>>,
    /// Pasted in place of every client copy, standing in for CLIPRDR file transfer.
    #[cfg(feature = "device-simulator")]
    simulated_file: Option<std::path::PathBuf>,
}

impl ClipboardLink {
    fn exchange(&mut self) -> anyhow::Result<()> {
        if self.incoming.has_changed()? {
            if let Some(snapshot) = self.incoming.borrow_and_update().as_ref() {
                #[cfg(feature = "device-simulator")]
                if let Some(file) = &self.simulated_file {
                    return self.private.write_files(&[file.as_path()]);
                }
                self.private.write(snapshot)?;
            }
        }
        if let Some(snapshot) = self.private.take_change()? {
            self.bridge.offer(snapshot);
        }
        Ok(())
    }
}

/// Loads the private clipboard, which is required to keep pages off the host clipboard.
fn private_clipboard(redirect: bool) -> anyhow::Result<Option<PrivateClipboard>> {
    let private = PrivateClipboard::load().context("browser startup requires the private-clipboard CEF runtime")?;
    Ok(redirect.then_some(private))
}

/// Remote capture devices and authenticator, kept on the CEF UI thread for the browser's lifetime.
struct RemoteDevices {
    #[cfg(feature = "device-simulator")]
    _simulator: Option<crate::device_simulator::DeviceSimulator>,
    _authenticator: RemoteAuthenticator,
}

/// Loads the remote devices, which are required to keep pages off host cameras, microphones and authenticators.
// UNTESTED: without the simulator, no WebAuthn handler is installed, so requests fail rather than reaching host authenticators.
// Recheck with a plain bundle when changing the authenticator wiring or the WebAuthn switch.
fn remote_devices(args: &Args) -> anyhow::Result<RemoteDevices> {
    const REQUIRED: &str = "browser startup requires the remote-device CEF runtime";
    let media = RemoteMedia::load().context(REQUIRED)?;
    #[cfg_attr(
        not(feature = "device-simulator"),
        expect(unused_mut, reason = "only the simulator installs a handler")
    )]
    let mut authenticator = RemoteAuthenticator::load().context(REQUIRED)?;
    #[cfg(feature = "device-simulator")]
    let simulator = args
        .simulate_devices
        .then(|| crate::device_simulator::DeviceSimulator::start(media, &mut authenticator))
        .transpose()?;
    #[cfg(not(feature = "device-simulator"))]
    let _ = (media, args);
    Ok(RemoteDevices {
        #[cfg(feature = "device-simulator")]
        _simulator: simulator,
        _authenticator: authenticator,
    })
}

fn finish_profile(profile: tempfile::TempDir, cleanup_complete: bool) -> anyhow::Result<()> {
    if cleanup_complete {
        #[cfg(not(windows))]
        {
            profile.close().context("failed to remove the browser profile")
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION;

            let path = profile.keep();
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => break Ok(()),
                    Err(error)
                        if error.raw_os_error().and_then(|code| u32::try_from(code).ok())
                            == Some(ERROR_SHARING_VIOLATION)
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    Err(error) => {
                        break Err(error)
                            .with_context(|| format!("failed to remove the browser profile {}", path.display()));
                    }
                }
            }
        }
    } else {
        let path = profile.keep();
        tracing::warn!(path = %path.display(), "Retained browser profile after incomplete cleanup");
        Ok(())
    }
}

fn run_browser(
    args: &Args,
    model: &mut Model,
    engine: &mut native::Engine,
    input: &mut input::Input,
    channels: Channels<'_>,
    stop: &AtomicBool,
) -> anyhow::Result<()> {
    let Channels {
        frames,
        commands,
        mut clipboard,
    } = channels;
    let mut popup_positions = BTreeMap::new();
    let mut dirty = true;
    let start = Instant::now();
    let mut last_frame = start;
    while !stop.load(Ordering::Acquire) {
        #[cfg(windows)]
        if STARTUP_INTERRUPT.load(Ordering::Acquire) {
            stop.store(true, Ordering::Release);
            break;
        }
        if args
            .exit_after_seconds
            .is_some_and(|seconds| start.elapsed() >= Duration::from_secs(seconds))
        {
            break;
        }
        for command in commands.try_iter().take(256) {
            let action = match command {
                rdp::Command::Keyboard(event) => input.keyboard(model, engine, event),
                rdp::Command::Mouse(event) => input.mouse(model, engine, event),
                rdp::Command::Resize(width, height) => {
                    if model.dimensions() != (u32::from(width), u32::from(height)) {
                        model.resize(u32::from(width), u32::from(height))?;
                        let (width, height) = model.content_dimensions();
                        engine.resize(u16::try_from(width)?, u16::try_from(height)?, model.generation());
                        popup_positions.clear();
                    }
                    None
                }
                rdp::Command::Reset => {
                    input.release_content(engine);
                    input.modifiers = Default::default();
                    None
                }
            };
            if let Some(action) = action {
                perform(model, engine, input, action)?;
            }
            dirty = true;
        }
        if let Some(link) = &mut clipboard {
            link.exchange()?;
        }
        for event in engine.poll()? {
            match event {
                native::Event::Paint {
                    tab,
                    popup,
                    generation,
                    width,
                    height,
                    pixels,
                } => {
                    if !model.tabs().iter().any(|entry| entry.id == tab) {
                        continue;
                    }
                    if generation != model.generation() {
                        tracing::debug!(tab, generation, "Discarded stale-generation CEF paint");
                        continue;
                    }
                    let result = if popup {
                        model.paint_popup(
                            tab,
                            generation,
                            popup_positions.get(&tab).copied().unwrap_or((0, 0)),
                            u32::from(width),
                            u32::from(height),
                            &pixels,
                        )
                    } else {
                        if model.content_dimensions() != (u32::from(width), u32::from(height)) {
                            tracing::debug!(tab, width, height, "Discarded stale-size CEF paint");
                            continue;
                        }
                        model.paint_view(tab, generation, u32::from(width), u32::from(height), &pixels)
                    };
                    result?;
                    dirty |= tab == model.active_id();
                }
                native::Event::Popup { tab, rect } => {
                    popup_positions.insert(tab, (rect.x, rect.y));
                }
                native::Event::HidePopup(tab) => {
                    if model.tabs().iter().any(|entry| entry.id == tab) {
                        model.hide_popup(tab)?;
                        dirty = true;
                    }
                }
                native::Event::Url(tab, url) => {
                    if model.tabs().iter().any(|entry| entry.id == tab) {
                        model.update_tab(tab, None, Some(&url), None, None)?;
                        dirty = true;
                    }
                }
                native::Event::Title(tab, title) => {
                    if model.tabs().iter().any(|entry| entry.id == tab) {
                        model.update_tab(tab, Some(&title), None, None, None)?;
                        dirty = true;
                    }
                }
                native::Event::Loading(tab, loading) => {
                    if model.tabs().iter().any(|entry| entry.id == tab) {
                        model.update_tab(tab, None, None, Some(loading), None)?;
                        dirty = true;
                    }
                }
                native::Event::Error(tab, error) => {
                    tracing::warn!(tab, %error, "Browser operation failed");
                    if model.tabs().iter().any(|entry| entry.id == tab) {
                        model.set_error(tab, Some(&error))?;
                        dirty = true;
                    }
                }
                native::Event::Open(url) => {
                    open_tab(model, engine, input, &url)?;
                    dirty = true;
                }
                native::Event::Closed(tab) => {
                    popup_positions.remove(&tab);
                }
            }
        }
        let previous_content = model.content_dimensions();
        dirty |= model.set_download_status(&engine.download_status())?;
        let (width, height) = model.content_dimensions();
        if previous_content != (width, height) {
            engine.resize(u16::try_from(width)?, u16::try_from(height)?, model.generation());
            popup_positions.clear();
        }
        if dirty && last_frame.elapsed() >= Duration::from_millis(33) {
            let (width, height) = model.dimensions();
            frames.send_replace(rdp::Frame {
                width: u16::try_from(width)?,
                height: u16::try_from(height)?,
                pixels: model.compose().into(),
            });
            dirty = false;
            last_frame = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(3));
    }
    Ok(())
}

#[cfg(windows)]
fn install_startup_interrupt_handler() -> anyhow::Result<()> {
    use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};

    // UNTESTED: CEF must not discard this handler before the Tokio handler is installed.
    // Interrupt throughout CEF startup when changing the bootstrap.
    // SAFETY: The handler uses only a static atomic and remains valid for the process lifetime.
    if unsafe { SetConsoleCtrlHandler(Some(handler), 1) } == 0 {
        return Err(std::io::Error::last_os_error()).context("failed to install Windows console interrupt handler");
    }

    return Ok(());

    unsafe extern "system" fn handler(event: u32) -> i32 {
        if event == CTRL_C_EVENT || event == CTRL_BREAK_EVENT {
            STARTUP_INTERRUPT.store(true, Ordering::Release);
            1
        } else {
            0
        }
    }
}

fn open_tab(model: &mut Model, engine: &mut native::Engine, input: &mut input::Input, url: &str) -> anyhow::Result<()> {
    if !native::allowed_url(url) {
        model.set_error(model.active_id(), Some("unsupported navigation URL"))?;
        return Ok(());
    }
    input.release_content(engine);
    let old = model.active_id();
    let id = match model.new_tab(url) {
        Ok(id) => id,
        Err(error) => {
            model.set_error(old, Some(&error.to_string()))?;
            return Ok(());
        }
    };
    let (width, height) = model.content_dimensions();
    if let Err(error) = engine.create(
        id,
        url,
        u16::try_from(width)?,
        u16::try_from(height)?,
        model.generation(),
    ) {
        model.close_tab(id)?;
        return Err(error);
    }
    engine.activate(id);
    Ok(())
}

fn perform(
    model: &mut Model,
    engine: &mut native::Engine,
    input: &mut input::Input,
    action: Action,
) -> anyhow::Result<()> {
    input.release_content(engine);
    match action {
        Action::CancelDownloads => engine.cancel_downloads(),
        Action::NewTab => open_tab(model, engine, input, "about:blank")?,
        Action::Activate { id } => {
            model.activate(id)?;
            engine.activate(id);
        }
        Action::CloseTab { id } => {
            engine.close(id)?;
            model.close_tab(id)?;
            if model.tabs().is_empty() {
                open_tab(model, engine, input, "about:blank")?;
            } else {
                engine.activate(model.active_id());
            }
        }
        Action::Navigate { tab, mut url } => {
            if !url.contains(':') {
                url = format!("https://{url}");
            }
            if !native::allowed_url(&url) {
                model.set_error(tab, Some("only HTTP, HTTPS, and about:blank URLs are supported"))?;
            } else if let Some(browser) = engine.browser(tab) {
                model.set_error(tab, None)?;
                if let Some(frame) = browser.main_frame() {
                    frame.load_url(Some(&url.as_str().into()));
                }
                if let Some(host) = browser.host() {
                    host.set_focus(1);
                }
            }
        }
        Action::Back { id } => {
            if let Some(browser) = engine.browser(id) {
                browser.go_back();
            }
        }
        Action::Forward { id } => {
            if let Some(browser) = engine.browser(id) {
                browser.go_forward();
            }
        }
        Action::Reload { id } => {
            if let Some(browser) = engine.browser(id) {
                browser.reload();
            }
        }
    }
    Ok(())
}
