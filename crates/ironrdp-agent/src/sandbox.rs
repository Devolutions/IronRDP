//! Windows Sandbox integration for ironrdp-agent.
//!
//! Resolves RDP connection parameters via the private WindowsSandboxServer gRPC
//! (`sandboxserver.SandboxCore` on `\\.\pipe\wsandbox\<md5(user SID)>`), then
//! expands them into a [`PropertySet`] the daemon already understands.
//!
//! The gRPC client is a small C# file-based app under `tools/windows_sandbox_grpc.cs`
//! (requires `dotnet`). Prefer creating the sandbox with official `wsb start`, then:
//! `ironrdp-agent connect --sandbox-id <id>`.

#![cfg(windows)]
// CLI-facing summaries intentionally print to stdout (same pattern as `cli.rs`).
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;
use std::process::Command;

use anyhow::Context as _;
use ironrdp_cfg::PropertySetExt as _;
use ironrdp_propertyset::PropertySet;
use serde::Deserialize;

/// Parsed `RdpClientConfig` from WindowsSandboxServer (subset we need).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct SandboxRdpConfig {
    pub sandbox_id: String,
    pub vm_id: String,
    pub username: String,
    pub password: String,
    pub rdp_transport: String,
    #[serde(default)]
    pub ip_address: String,
    pub pipe_path: Option<String>,
    #[serde(default)]
    pub clipboard_redirection: bool,
    #[serde(default)]
    pub smartcard_redirection: bool,
}

#[derive(Debug, Deserialize)]
struct ListReply {
    sandbox_ids: Vec<String>,
}

/// Embedded so release binaries still work without shipping the `.cs` next to the exe.
const HELPER_SOURCE: &str = include_str!("../tools/windows_sandbox_grpc.cs");

fn helper_script_path() -> anyhow::Result<PathBuf> {
    if let Ok(env_path) = std::env::var("IRONRDP_SANDBOX_GRPC_HELPER") {
        let p = PathBuf::from(env_path);
        if p.is_file() {
            return Ok(p);
        }
        anyhow::bail!("IRONRDP_SANDBOX_GRPC_HELPER points to missing file: {}", p.display());
    }

    // Prefer a helper shipped beside the agent binary (release packaging / manual install).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let beside = dir.join("windows_sandbox_grpc.cs");
            if beside.is_file() {
                return Ok(beside);
            }
        }
    }

    // Dev tree / `cargo run -p ironrdp-agent`.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/windows_sandbox_grpc.cs");
    if manifest.is_file() {
        return Ok(manifest);
    }

    // Materialize the embedded source for installed binaries that did not ship the helper file.
    let dir = std::env::temp_dir().join("ironrdp-agent-sandbox");
    std::fs::create_dir_all(&dir).context("create temp dir for windows sandbox gRPC helper")?;
    let path = dir.join("windows_sandbox_grpc.cs");
    let needs_write = match std::fs::read_to_string(&path) {
        Ok(existing) => existing != HELPER_SOURCE,
        Err(_) => true,
    };
    if needs_write {
        std::fs::write(&path, HELPER_SOURCE).context("write embedded windows sandbox gRPC helper")?;
    }
    Ok(path)
}

/// Re-apply NamedPipe transport security after user property merges.
///
/// Clipboard and credentials may be overridden by the caller; TLS/CredSSP must stay off for
/// the default Sandbox named-pipe path so we do not advertise enhanced protocols the server
/// will reject (and so we do not silently open a plaintext TCP session).
pub(crate) fn reassert_named_pipe_security(ps: &mut PropertySet) {
    ps.set_enable_tls(false);
    ps.set_enable_credssp_support(false);
    ps.set_autologon(true);
}

fn run_helper(args: &[&str]) -> anyhow::Result<String> {
    let script = helper_script_path()?;
    // File-based `dotnet run` may emit compiler warnings on stdout; keep only the JSON object line.
    let output = Command::new("dotnet")
        .arg("run")
        .arg(&script)
        .arg("--")
        .args(args)
        .env("DOTNET_NOLOGO", "1")
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .output()
        .with_context(|| {
            format!(
                "failed to spawn `dotnet run {}` (is the .NET SDK installed?)",
                script.display()
            )
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let json_line = stdout
        .lines()
        .rev()
        .find(|l| {
            let t = l.trim_start();
            t.starts_with('{') || t.starts_with('[')
        })
        .map(str::trim)
        .unwrap_or("");

    if !output.status.success() {
        anyhow::bail!(
            "windows sandbox gRPC helper failed (exit {:?}): {}{}",
            output.status.code(),
            stderr.trim(),
            if stdout.trim().is_empty() {
                String::new()
            } else {
                format!(" | {stdout}")
            }
        );
    }

    if json_line.is_empty() {
        anyhow::bail!(
            "windows sandbox gRPC helper produced no JSON (stdout={stdout:?}, stderr={})",
            stderr.trim()
        );
    }

    Ok(json_line.to_owned())
}

/// List running sandbox ids via WindowsSandboxServer.
pub(crate) fn list_sandbox_ids() -> anyhow::Result<Vec<String>> {
    let stdout = run_helper(&["list"])?;
    let reply: ListReply = serde_json::from_str(stdout.trim()).context("parse list JSON")?;
    Ok(reply.sandbox_ids)
}

/// Fetch `RdpClientConfig` for a running sandbox.
pub(crate) fn get_rdp_config(sandbox_id: &str) -> anyhow::Result<SandboxRdpConfig> {
    let stdout = run_helper(&["config", sandbox_id])?;
    serde_json::from_str(stdout.trim()).context("parse config JSON")
}

/// Shut down a running sandbox via gRPC.
pub(crate) fn stop_sandbox(sandbox_id: &str) -> anyhow::Result<()> {
    let _ = run_helper(&["stop", sandbox_id])?;
    Ok(())
}

/// Expand a sandbox id into connect properties (NamedPipe + PROTOCOL_RDP defaults).
pub(crate) fn properties_for_sandbox_id(sandbox_id: &str) -> anyhow::Result<PropertySet> {
    let cfg = get_rdp_config(sandbox_id)?;
    properties_from_config(&cfg)
}

/// Expand an already-fetched config into connect properties.
pub(crate) fn properties_from_config(cfg: &SandboxRdpConfig) -> anyhow::Result<PropertySet> {
    let mut ps = PropertySet::new();

    ps.set_sandbox_id(cfg.sandbox_id.clone());
    ps.set_username(cfg.username.clone());
    ps.set_clear_text_password(cfg.password.clone());

    // Destination is still required by ConfigBuilder; use a dummy host derived from VM id.
    let dest_name = if cfg.vm_id.is_empty() {
        "windows-sandbox".to_owned()
    } else {
        cfg.vm_id.clone()
    };
    ps.set_full_address(&ironrdp_cfg::TargetAddr {
        host: ironrdp_cfg::TargetHost::Domain(dest_name),
        port: None,
    });
    ps.set_server_port(3389);

    // Sandbox NamedPipe path: standard RDP security, no TLS/CredSSP, autologon Client Info.
    let transport = cfg.rdp_transport.to_ascii_lowercase();
    if transport.contains("namedpipe") || transport == "0" || transport.is_empty() {
        let pipe = cfg
            .pipe_path
            .clone()
            .filter(|p| !p.is_empty())
            .or_else(|| {
                if cfg.vm_id.is_empty() {
                    None
                } else {
                    Some(format!(r"\\.\pipe\{}", cfg.vm_id))
                }
            })
            .context("sandbox RdpClientConfig missing VMId/pipe path for NamedPipe transport")?;
        ps.set_named_pipe(pipe);
        ps.set_enable_tls(false);
        ps.set_enable_credssp_support(false);
        ps.set_autologon(true);
    } else if transport.contains("tcp") || transport == "2" {
        if cfg.ip_address.is_empty() {
            anyhow::bail!("sandbox TCP transport selected but IpAddress is empty");
        }
        ps.set_full_address(&ironrdp_cfg::TargetAddr {
            host: ironrdp_cfg::TargetHost::Domain(cfg.ip_address.clone()),
            port: None,
        });
        // TCP mode uses CredSSP in the product client.
        ps.set_enable_tls(true);
        ps.set_enable_credssp_support(true);
    } else if transport.contains("local") || transport == "1" {
        anyhow::bail!(
            "sandbox Local (VMConnect :2179 + PCB) transport is not implemented yet; \
             start the sandbox without an RdpTransport override (default NamedPipe)"
        );
    } else {
        anyhow::bail!("unsupported sandbox RdpTransport '{transport}'");
    }

    ps.set_redirect_clipboard(cfg.clipboard_redirection);
    let _ = cfg.smartcard_redirection;

    Ok(ps)
}

/// Expand a raw pipe path + credentials into connect properties (escape hatch).
pub(crate) fn properties_for_pipe(pipe_path: &str, username: &str, password: &str) -> PropertySet {
    let mut ps = PropertySet::new();
    let pipe = if pipe_path.starts_with(r"\\.\pipe\") || pipe_path.starts_with(r"\\?\pipe\") {
        pipe_path.to_owned()
    } else {
        format!(r"\\.\pipe\{pipe_path}")
    };
    ps.set_named_pipe(pipe);
    ps.set_username(username.to_owned());
    ps.set_clear_text_password(password.to_owned());
    ps.set_full_address(&ironrdp_cfg::TargetAddr {
        host: ironrdp_cfg::TargetHost::Domain("windows-sandbox".into()),
        port: None,
    });
    ps.set_server_port(3389);
    ps.set_enable_tls(false);
    ps.set_enable_credssp_support(false);
    ps.set_autologon(true);
    ps
}

/// Print a redacted config summary (no password) for `sandbox config`.
pub(crate) fn print_config_summary(cfg: &SandboxRdpConfig) {
    println!("sandbox_id: {}", cfg.sandbox_id);
    println!("vm_id:      {}", cfg.vm_id);
    println!("transport:  {}", cfg.rdp_transport);
    println!("username:   {}", cfg.username);
    println!(
        "password:   {}",
        if cfg.password.is_empty() { "(empty)" } else { "(set)" }
    );
    if let Some(pipe) = &cfg.pipe_path {
        println!("pipe:       {pipe}");
    }
    if !cfg.ip_address.is_empty() {
        println!("ip:         {}", cfg.ip_address);
    }
}
