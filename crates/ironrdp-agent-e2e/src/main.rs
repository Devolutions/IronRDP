//! Live-RDP smoke test driver: shells out to a real, compiled `ironrdp-agent` binary and drives it
//! through a full connect/input/screenshot cycle against a genuine interactive RDP desktop.
//!
//! This intentionally never links `ironrdp-agent` (or any of its dependencies) as a library: it
//! only ever spawns the compiled binary and reads its plain-text stdout, exercising the exact
//! surface (argument parsing, `print_payload`'s output) a real caller — human or LLM — hits.

use core::time::Duration;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Instant;

use anyhow::Context as _;

const DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Stride, in pixels, used to sample the decoded screenshot for the non-blank check.
const STRIDE_PIXELS: usize = 37;

struct Cli {
    agent_bin: PathBuf,
    host: String,
    port: u16,
    username: String,
    password_env: String,
    domain: Option<String>,
    artifacts_dir: PathBuf,
}

fn parse_cli() -> anyhow::Result<Cli> {
    let mut args = pico_args::Arguments::from_env();
    let cli = Cli {
        agent_bin: args.value_from_str("--agent-bin").context("missing --agent-bin")?,
        host: args.value_from_str("--host").context("missing --host")?,
        port: args.opt_value_from_str("--port")?.unwrap_or(3389),
        username: args.value_from_str("--username").context("missing --username")?,
        password_env: args
            .value_from_str("--password-env")
            .context("missing --password-env")?,
        domain: args.opt_value_from_str("--domain")?,
        artifacts_dir: args
            .value_from_str("--artifacts-dir")
            .context("missing --artifacts-dir")?,
    };
    args.finish();
    Ok(cli)
}

fn main() -> anyhow::Result<()> {
    let cli = parse_cli()?;
    std::fs::create_dir_all(&cli.artifacts_dir)
        .with_context(|| format!("create artifacts directory {}", cli.artifacts_dir.display()))?;

    let _daemon = spawn_daemon(&cli.agent_bin, &cli.artifacts_dir)?;
    wait_daemon_ready(&cli.agent_bin)?;

    if let Err(error) = drive_session(&cli) {
        // Best-effort diagnostic screenshot for CI triage; its own errors are not propagated.
        let _ = take_screenshot(&cli.agent_bin, &cli.artifacts_dir.join("live-rdp-failure.png"));
        return Err(error);
    }

    Ok(())
}

/// Runs `ironrdp-agent` with `args`, returning its stdout as text. Fails if the process does not
/// exit successfully.
fn run_agent(agent_bin: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(agent_bin)
        .args(args)
        .output()
        .with_context(|| format!("spawn {} {args:?}", agent_bin.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "ironrdp-agent {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Owns the daemon child process: kills and reaps it on every exit path (success, error, panic).
struct DaemonGuard(Child);

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_daemon(agent_bin: &Path, artifacts_dir: &Path) -> anyhow::Result<DaemonGuard> {
    let stdout =
        std::fs::File::create(artifacts_dir.join("ironrdp-agent.stdout.log")).context("create daemon stdout log")?;
    let stderr =
        std::fs::File::create(artifacts_dir.join("ironrdp-agent.stderr.log")).context("create daemon stderr log")?;

    let child = Command::new(agent_bin)
        .arg("daemon-start")
        .arg("--prop")
        .arg("ironrdp_autologon:i:1")
        .arg("--prop")
        .arg("enablecredsspsupport:i:0")
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .with_context(|| format!("spawn {} daemon-start", agent_bin.display()))?;

    Ok(DaemonGuard(child))
}

fn wait_daemon_ready(agent_bin: &Path) -> anyhow::Result<()> {
    let deadline = Instant::now() + DAEMON_READY_TIMEOUT;
    loop {
        if run_agent(agent_bin, &["status"]).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!("daemon did not become ready within {DAEMON_READY_TIMEOUT:?}");
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn drive_session(cli: &Cli) -> anyhow::Result<()> {
    connect(cli)?;
    wait_connected(&cli.agent_bin)?;

    run_agent(&cli.agent_bin, &["mouse-move", "--x", "200", "--y", "200"])?;
    run_agent(
        &cli.agent_bin,
        &["mouse-button", "--button", "right", "--pressed", "true"],
    )?;
    run_agent(
        &cli.agent_bin,
        &["mouse-button", "--button", "right", "--pressed", "false"],
    )?;

    let screenshot_path = cli.artifacts_dir.join("live-rdp.png");
    take_screenshot(&cli.agent_bin, &screenshot_path)?;
    assert_non_blank_png(&screenshot_path)?;

    run_agent(
        &cli.agent_bin,
        &["key-scancode", "--scancode", "0x01", "--pressed", "true"],
    )?;
    run_agent(
        &cli.agent_bin,
        &["key-scancode", "--scancode", "0x01", "--pressed", "false"],
    )?;

    run_agent(&cli.agent_bin, &["disconnect"])?;

    Ok(())
}

/// Reads the password from `--password-env`'s named environment variable right before building the
/// connect argv, so it is never stored longer than needed and never logged: unlike [`run_agent`],
/// the failure path here must not echo `args` back (they carry the password in cleartext).
fn connect(cli: &Cli) -> anyhow::Result<()> {
    let password = std::env::var(&cli.password_env).with_context(|| format!("read env var {}", cli.password_env))?;

    let server = format!("{}:{}", cli.host, cli.port);
    let mut args: Vec<&str> = vec![
        "connect",
        "--server",
        server.as_str(),
        "--username",
        cli.username.as_str(),
        "--password",
        password.as_str(),
    ];
    if let Some(domain) = &cli.domain {
        args.push("--domain");
        args.push(domain.as_str());
    }

    let output = Command::new(&cli.agent_bin)
        .args(&args)
        .output()
        .with_context(|| format!("spawn {} connect", cli.agent_bin.display()))?;
    anyhow::ensure!(
        output.status.success(),
        "ironrdp-agent connect failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    Ok(())
}

fn wait_connected(agent_bin: &Path) -> anyhow::Result<()> {
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    loop {
        let status = run_agent(agent_bin, &["status"])?;
        let state = status
            .lines()
            .find_map(|line| line.strip_prefix("state: "))
            .with_context(|| format!("no 'state:' line in status output: {status}"))?;

        match state {
            "Connected" => return Ok(()),
            "Failed" => {
                let detail = status.lines().find_map(|line| line.strip_prefix("detail: "));
                anyhow::bail!("connection failed: {}", detail.unwrap_or("no detail available"));
            }
            _ => {}
        }

        if Instant::now() >= deadline {
            anyhow::bail!("session did not reach 'Connected' within {CONNECT_TIMEOUT:?} (last state: {state})");
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn take_screenshot(agent_bin: &Path, path: &Path) -> anyhow::Result<()> {
    let path_str = path.to_str().context("artifacts path is not valid UTF-8")?;
    run_agent(agent_bin, &["screenshot", path_str])?;
    Ok(())
}

/// Decodes the PNG at `path` and asserts it is not a uniform (blank) frame, by sampling a strided
/// pixel grid and checking that at least one sampled pixel differs from the first.
fn assert_non_blank_png(path: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let decoder = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = decoder.read_info().context("read PNG header")?;
    let mut buffer = vec![0; reader.output_buffer_size().context("PNG output buffer size")?];
    let info = reader.next_frame(&mut buffer).context("decode PNG frame")?;
    let bytes = &buffer[..info.buffer_size()];

    let channels = info.color_type.samples();
    anyhow::ensure!(bytes.len() >= channels, "decoded PNG frame is empty");

    let first_pixel = &bytes[..channels];
    let non_blank = bytes
        .chunks_exact(channels)
        .step_by(STRIDE_PIXELS)
        .any(|pixel| pixel != first_pixel);

    anyhow::ensure!(
        non_blank,
        "screenshot {} looks blank: every sampled pixel matches the first",
        path.display()
    );

    Ok(())
}
