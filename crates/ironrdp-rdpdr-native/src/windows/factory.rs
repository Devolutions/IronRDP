//! Windows RDPDR backend configuration.

use core::fmt;
use std::path::{Path, PathBuf};

use super::backend::WindowsRdpdrBackend;

/// Immutable logical-volume definition selected for Windows RDPDR redirection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedirectedDrive {
    device_id: u32,
    display_name: String,
    root_path: PathBuf,
    read_only: bool,
}

impl RedirectedDrive {
    /// Creates a drive definition for one full logical Windows volume.
    pub fn new(
        device_id: u32,
        display_name: impl Into<String>,
        root_path: impl Into<PathBuf>,
        read_only: bool,
    ) -> Result<Self, RedirectedDriveError> {
        if device_id == 0 {
            return Err(RedirectedDriveError::ReservedDeviceId);
        }

        let display_name = display_name.into();
        if display_name.is_empty() {
            return Err(RedirectedDriveError::EmptyDisplayName);
        }
        if display_name.contains('\0') {
            return Err(RedirectedDriveError::EmbeddedNul);
        }

        Ok(Self {
            device_id,
            display_name,
            root_path: root_path.into(),
            read_only,
        })
    }

    /// Returns the RDPDR filesystem device ID.
    #[must_use]
    pub fn device_id(&self) -> u32 {
        self.device_id
    }

    /// Returns the drive name announced through the portable RDPDR channel.
    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub(crate) fn root_path(&self) -> &Path {
        &self.root_path
    }

    pub(crate) fn read_only(&self) -> bool {
        self.read_only
    }
}

/// Invalid selected-drive configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirectedDriveError {
    /// Device ID zero is reserved for the smartcard channel.
    ReservedDeviceId,
    /// RDPDR requires a nonempty user-visible drive name.
    EmptyDisplayName,
    /// An embedded NUL would truncate the RDPDR Unicode device name.
    EmbeddedNul,
}

impl fmt::Display for RedirectedDriveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReservedDeviceId => f.write_str("RDPDR device ID zero is reserved"),
            Self::EmptyDisplayName => f.write_str("redirected drive display name must not be empty"),
            Self::EmbeddedNul => f.write_str("redirected drive display name must not contain NUL"),
        }
    }
}

impl core::error::Error for RedirectedDriveError {}

/// Creates one isolated backend from a fixed logical-volume definition.
///
/// The returned initial-drive list is intentionally shaped for
/// [`ironrdp_rdpdr::Rdpdr::with_drives`], keeping platform configuration out of
/// the portable RDPDR crate.
#[derive(Clone, Debug)]
pub struct WindowsRdpdrBackendFactory {
    drive: RedirectedDrive,
}

impl WindowsRdpdrBackendFactory {
    /// Configures the single logical-volume root supported by this baseline.
    #[must_use]
    pub fn new(drive: RedirectedDrive) -> Self {
        Self { drive }
    }

    /// Returns the initial `(device_id, name)` pair for `Rdpdr::with_drives`.
    #[must_use]
    pub fn initial_drives(&self) -> Vec<(u32, String)> {
        vec![(self.drive.device_id, self.drive.display_name.clone())]
    }

    /// Builds a backend with no active root handles.
    ///
    /// The portable channel activates initial drives with
    /// `RdpdrBackend::restore_drive` at the start of each server-announcement
    /// sequence.
    #[must_use]
    pub fn build(&self) -> WindowsRdpdrBackend {
        WindowsRdpdrBackend::from_drive(self.drive.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_preserves_portable_initial_drive_names() {
        let factory = WindowsRdpdrBackendFactory::new(
            RedirectedDrive::new(1, "System", r"C:\", false).expect("valid system drive"),
        );

        assert_eq!(factory.initial_drives(), vec![(1, "System".to_owned())]);
    }
}
