//! Windows Sandbox integration for ironrdp-agent.
//!
//! Resolves RDP connection parameters via the private WindowsSandboxServer gRPC
//! (`sandboxserver.SandboxCore` on `\\.\pipe\wsandbox\<md5(user SID)>`), then
//! expands them into a [`PropertySet`] the daemon already understands.
//!
//! Prefer creating the sandbox with official `wsb start`, then:
//! `ironrdp-agent connect --sandbox-id <id>`.

#![cfg(windows)]
// CLI-facing summaries intentionally print to stdout (same pattern as `cli.rs`).
#![allow(clippy::print_stdout, clippy::print_stderr)]

use anyhow::Context as _;
use ironrdp_cfg::PropertySetExt as _;
use ironrdp_propertyset::PropertySet;

use crate::sandbox_grpc;

/// Parsed `RdpClientConfig` from WindowsSandboxServer (subset we need).
#[derive(Debug, Clone)]
pub(crate) struct SandboxRdpConfig {
    pub sandbox_id: String,
    pub vm_id: String,
    pub username: String,
    pub password: String,
    pub rdp_transport: String,
    pub ip_address: String,
    pub pipe_path: Option<String>,
    pub clipboard_redirection: bool,
    pub smartcard_redirection: bool,
}

/// Re-apply NamedPipe transport security after user property merges.
///
/// No-op unless `ironrdp_named_pipe` is set, so a Sandbox TCP config (TLS+CredSSP) is not
/// rewritten into plain security. When the pipe path is present, TLS/CredSSP stay off and
/// autologon stays on so we do not advertise enhanced protocols the pipe server rejects.
pub(crate) fn reassert_named_pipe_security(ps: &mut PropertySet) {
    if ps.named_pipe().is_none() {
        return;
    }
    ps.set_enable_tls(false);
    ps.set_enable_credssp_support(false);
    ps.set_autologon(true);
}

/// List running sandbox ids via WindowsSandboxServer.
pub(crate) fn list_sandbox_ids() -> anyhow::Result<Vec<String>> {
    sandbox_grpc::enumerate_sandbox_vms()
}

/// Start a sandbox through WindowsSandboxServer and return its RDP configuration.
///
/// `recipe` is the Windows Sandbox configuration XML.
/// When it is absent, the server uses its default configuration.
/// `sandbox_id` lets callers select a stable ID; otherwise the server assigns one.
pub(crate) fn start_sandbox(recipe: Option<&str>, sandbox_id: Option<&str>) -> anyhow::Result<SandboxRdpConfig> {
    let xml = sandbox_grpc::start_sandbox(recipe, sandbox_id).context("StartSandbox")?;
    Ok(parse_config_xml(&xml, sandbox_id.unwrap_or_default()))
}

/// Fetch `RdpClientConfig` for a running sandbox.
pub(crate) fn get_rdp_config(sandbox_id: &str) -> anyhow::Result<SandboxRdpConfig> {
    let xml = sandbox_grpc::get_rdp_client_config_xml(sandbox_id)
        .with_context(|| format!("GetRdpClientConfig for {sandbox_id}"))?;
    Ok(parse_config_xml(&xml, sandbox_id))
}

/// Shut down a running sandbox via gRPC.
pub(crate) fn stop_sandbox(sandbox_id: &str) -> anyhow::Result<()> {
    sandbox_grpc::shutdown_sandbox(sandbox_id)
}

fn parse_config_xml(xml: &str, fallback_sandbox_id: &str) -> SandboxRdpConfig {
    let trim_braces = |s: String| s.trim_matches(|c| c == '{' || c == '}').to_owned();

    let mut sandbox_id = trim_braces(sandbox_grpc::xml_local_value(xml, "SandboxId"));
    if sandbox_id.is_empty() {
        sandbox_id = fallback_sandbox_id.to_owned();
    }
    let vm_id = trim_braces(sandbox_grpc::xml_local_value(xml, "VMId"));
    let username = sandbox_grpc::xml_local_value(xml, "Username");
    let password = sandbox_grpc::xml_local_value(xml, "Password");
    let rdp_transport = sandbox_grpc::xml_local_value(xml, "RdpTransport");
    let ip_address = sandbox_grpc::xml_local_value(xml, "IpAddress");
    let clipboard_redirection = parse_bool_xml(&sandbox_grpc::xml_local_value(xml, "ClipboardRedirection"));
    let smartcard_redirection = parse_bool_xml(&sandbox_grpc::xml_local_value(xml, "SmartCardRedirection"));
    let pipe_path = if vm_id.is_empty() {
        None
    } else {
        Some(format!(r"\\.\pipe\{vm_id}"))
    };

    SandboxRdpConfig {
        sandbox_id,
        vm_id,
        username,
        password,
        rdp_transport,
        ip_address,
        pipe_path,
        clipboard_redirection,
        smartcard_redirection,
    }
}

fn parse_bool_xml(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes")
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
    ps.set_enable_smartcard(cfg.smartcard_redirection);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_config_xml_named_pipe() {
        let xml = "
            <RdpClientConfig>
              <SandboxId>{8825f947-7d05-46e5-9efb-317ca83500ec}</SandboxId>
              <VMId>{606cf61b-dd6c-4d4c-8700-4af99a73f7ab}</VMId>
              <Username>WDAGUtilityAccount</Username>
              <Password>pw</Password>
              <RdpTransport>NamedPipe</RdpTransport>
              <ClipboardRedirection>true</ClipboardRedirection>
              <SmartCardRedirection>false</SmartCardRedirection>
            </RdpClientConfig>
        ";
        let cfg = parse_config_xml(xml, "fallback");
        assert_eq!(cfg.sandbox_id, "8825f947-7d05-46e5-9efb-317ca83500ec");
        assert_eq!(cfg.vm_id, "606cf61b-dd6c-4d4c-8700-4af99a73f7ab");
        assert_eq!(cfg.username, "WDAGUtilityAccount");
        assert_eq!(cfg.password, "pw");
        assert_eq!(cfg.rdp_transport, "NamedPipe");
        assert!(cfg.clipboard_redirection);
        assert!(!cfg.smartcard_redirection);
        assert_eq!(
            cfg.pipe_path.as_deref(),
            Some(r"\\.\pipe\606cf61b-dd6c-4d4c-8700-4af99a73f7ab")
        );

        let props = properties_from_config(&cfg).expect("named pipe props");
        assert!(props.named_pipe().is_some());
        assert_eq!(props.enable_tls(), Some(false));
        assert_eq!(props.enable_credssp_support(), Some(false));
        assert_eq!(props.enable_smartcard(), Some(false));
    }

    #[test]
    fn properties_from_config_sets_smartcard_redirection() {
        let cfg = SandboxRdpConfig {
            sandbox_id: "id".into(),
            vm_id: "vm".into(),
            username: "u".into(),
            password: "p".into(),
            rdp_transport: "NamedPipe".into(),
            ip_address: String::new(),
            pipe_path: Some(r"\\.\pipe\vm".into()),
            clipboard_redirection: true,
            smartcard_redirection: true,
        };
        let props = properties_from_config(&cfg).expect("named pipe props");
        assert_eq!(props.enable_smartcard(), Some(true));
    }

    #[test]
    fn reassert_named_pipe_security_only_when_pipe_set() {
        let mut tcp = PropertySet::new();
        tcp.set_enable_tls(true);
        tcp.set_enable_credssp_support(true);
        tcp.set_autologon(false);
        reassert_named_pipe_security(&mut tcp);
        assert_eq!(tcp.enable_tls(), Some(true));
        assert_eq!(tcp.enable_credssp_support(), Some(true));
        assert_eq!(tcp.autologon(), Some(false));

        let mut pipe = PropertySet::new();
        pipe.set_named_pipe(r"\\.\pipe\test");
        pipe.set_enable_tls(true);
        pipe.set_enable_credssp_support(true);
        pipe.set_autologon(false);
        reassert_named_pipe_security(&mut pipe);
        assert_eq!(pipe.enable_tls(), Some(false));
        assert_eq!(pipe.enable_credssp_support(), Some(false));
        assert_eq!(pipe.autologon(), Some(true));
    }

    #[test]
    fn properties_from_config_tcp_keeps_credssp() {
        let cfg = SandboxRdpConfig {
            sandbox_id: "id".into(),
            vm_id: "vm".into(),
            username: "u".into(),
            password: "p".into(),
            rdp_transport: "Tcp".into(),
            ip_address: "10.0.0.2".into(),
            pipe_path: Some(r"\\.\pipe\vm".into()),
            clipboard_redirection: false,
            smartcard_redirection: false,
        };
        let mut props = properties_from_config(&cfg).expect("tcp props");
        assert!(props.named_pipe().is_none());
        assert_eq!(props.enable_tls(), Some(true));
        assert_eq!(props.enable_credssp_support(), Some(true));
        reassert_named_pipe_security(&mut props);
        assert_eq!(props.enable_tls(), Some(true));
        assert_eq!(props.enable_credssp_support(), Some(true));
    }
}
