//! Per-server storage for printer settings the server sends back ([MS-RDPEPC] cache data).
//!
//! [MS-RDPEPC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepc/

use std::path::PathBuf;

const PATH_ENV: &str = "IRONRDP_AGENT_PRINTER_CACHE";

/// Returns the printer cache directory for `server`, usually `host:port`.
///
/// This is the location shared by the agent-family hosts (daemon, agent, and viewer), next to
/// their known-certificates store.
///
/// The root is `IRONRDP_AGENT_PRINTER_CACHE` when set, otherwise
/// `%APPDATA%\ironrdp-agent\printer-cache` on Windows and
/// `$XDG_CONFIG_HOME/ironrdp-agent/printer-cache` (or `~/.config/…`) elsewhere. Each server gets
/// its own subdirectory, named after the host with characters outside `[A-Za-z0-9.-]` replaced.
pub fn directory(server: &str) -> anyhow::Result<PathBuf> {
    let root = match std::env::var_os(PATH_ENV).filter(|path| !path.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => crate::known_certificates::agent_config_root()?.join("printer-cache"),
    };

    let server = server
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    anyhow::ensure!(
        !server.is_empty() && server.chars().any(|c| c != '.'),
        "server name cannot name a printer cache directory"
    );
    Ok(root.join(server))
}

/// Persists server printer settings for `server` when `factory` redirects printers.
///
/// A missing cache location is logged; the factory is returned without a store.
#[cfg(windows)]
pub fn with_store(
    factory: ironrdp_rdpdr_native::WindowsRdpdrBackendFactory,
    server: &str,
) -> ironrdp_rdpdr_native::WindowsRdpdrBackendFactory {
    if !factory.default_printer() {
        return factory;
    }
    match directory(server) {
        Ok(directory) => {
            let printers = factory
                .printer_redirection()
                .clone()
                .with_cache_store(std::sync::Arc::new(
                    ironrdp_rdpdr_native::FileRdpdrPrinterCacheStore::new(directory),
                ));
            factory.with_printer_redirection(printers)
        }
        Err(error) => {
            tracing::warn!(error = format!("{error:#}"), "Printer settings will not persist");
            factory
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_names_become_safe_directory_names() {
        let server_directory = directory("RDS01.Example.com:3389").expect("directory");
        assert_eq!(
            server_directory.file_name().and_then(|name| name.to_str()),
            Some("rds01.example.com_3389")
        );
        assert!(directory("..").is_err());
        assert!(directory("").is_err());
    }
}
