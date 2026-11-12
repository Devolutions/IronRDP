//! Windows drive redirection: native backend configuration and drive hotplug.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use ironrdp::client::config::{DriveRedirection, RdpdrConfig};
use ironrdp::client::rdp::RdpInputSender;
use ironrdp_daemon::drives::{logical_drive_factory, watch_logical_drives};
use ironrdp_rdpdr_native::{
    FIRST_NAMED_DRIVE_DEVICE_ID, LogicalDriveWatcher, RedirectedDrive, WindowsRdpdrBackendFactory,
};

use tracing::warn;

use crate::cli::NamedDrive;

/// Drives requested for one viewer session.
#[derive(Clone, Debug, Default)]
pub struct ViewerDrives {
    selection: DriveRedirection,
    named: Vec<NamedDrive>,
}

impl ViewerDrives {
    pub fn new(selection: DriveRedirection, named: Vec<NamedDrive>) -> Self {
        Self { selection, named }
    }

    /// Takes the drive selection from the session configuration, which is empty when device
    /// redirection is disabled (`ironrdp_rdpdr:i:0`).
    pub fn from_rdpdr_config(config: &RdpdrConfig, named: Vec<NamedDrive>) -> Self {
        let selection = if config.enabled {
            config.drives
        } else {
            DriveRedirection::default()
        };
        Self::new(selection, named)
    }

    /// Whether no drive is requested.
    pub fn is_empty(&self) -> bool {
        self.selection.is_empty() && self.named.is_empty()
    }

    /// Builds the RDPDR backend factory for these drives.
    ///
    /// With dynamic drives, the returned watcher announces volumes connected during the session
    /// through `input_sender` until it is dropped.
    pub fn backend_factory(
        &self,
        input_sender: &RdpInputSender,
    ) -> anyhow::Result<(WindowsRdpdrBackendFactory, Option<LogicalDriveWatcher>)> {
        let mut seen_names = BTreeSet::new();
        let mut seen_roots = BTreeSet::new();
        let named = self
            .named
            .iter()
            .zip(FIRST_NAMED_DRIVE_DEVICE_ID..)
            .map(|(drive, device_id)| {
                if !seen_names.insert(drive.name.to_uppercase()) {
                    anyhow::bail!("drive name {} is used more than once", drive.name);
                }
                let path = resolve_drive_root(&drive.path)?;
                if !seen_roots.insert(path.to_string_lossy().to_uppercase()) {
                    anyhow::bail!("drive path {} is redirected more than once", path.display());
                }
                RedirectedDrive::new(device_id, &drive.name, path, false)
                    .with_context(|| format!("invalid drive {}", drive.name))
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let factory = WindowsRdpdrBackendFactory::from_drives(named).context("invalid drive configuration")?;
        let (factory, selection) = logical_drive_factory(Some(factory), self.selection)?;
        let factory = factory.context("logical drive configuration dropped the drive factory")?;
        let watcher = selection.and_then(|selection| {
            watch_logical_drives(selection, input_sender.clone())
                .inspect_err(|error| warn!(%error, "Drive hotplug is unavailable; redirecting only the drives present at connect"))
                .ok()
        });
        Ok((factory, watcher))
    }
}

/// Resolves a named drive path to the canonical drive-letter directory the backend opens.
///
/// Canonicalizing resolves junctions and symbolic links in the configured path, such as a
/// redirected user profile, because the backend refuses to traverse reparse points.
fn resolve_drive_root(path: &Path) -> anyhow::Result<PathBuf> {
    let canonical = std::fs::canonicalize(path).with_context(|| format!("resolve drive path {}", path.display()))?;
    if !canonical.is_dir() {
        anyhow::bail!("drive path {} is not a directory", path.display());
    }
    let canonical = canonical.to_string_lossy();
    match canonical.strip_prefix(r"\\?\") {
        Some(root) if root.as_bytes().get(1) == Some(&b':') => Ok(PathBuf::from(root)),
        _ => anyhow::bail!(
            "drive path {} does not resolve to a drive-letter directory; map network shares to a drive letter",
            path.display()
        ),
    }
}
