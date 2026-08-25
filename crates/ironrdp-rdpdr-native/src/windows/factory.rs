//! Windows RDPDR backend configuration.

use core::fmt;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ironrdp_rdpdr::{RdpdrBackendFactory, RdpdrBackendFactoryResult, RdpdrBackendProduct, RdpdrDrive};

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
            Self::ReservedDeviceId => f.write_str("device ID zero is reserved for RDPDR"),
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
///
/// Smartcard redirection is optional and independent of drives: an empty drive
/// list is valid for smartcard-only sessions.
#[derive(Clone, Debug)]
pub struct WindowsRdpdrBackendFactory {
    drives: Vec<RedirectedDrive>,
    initial_drive_ids: Vec<u32>,
    dynamic_drives: bool,
    smartcard: bool,
}

impl WindowsRdpdrBackendFactory {
    /// Configures the single logical-volume root supported by this baseline.
    #[must_use]
    pub fn new(drive: RedirectedDrive) -> Self {
        let device_id = drive.device_id();
        Self {
            drives: vec![drive],
            initial_drive_ids: vec![device_id],
            dynamic_drives: false,
            smartcard: false,
        }
    }

    /// Configures the logical-volume roots selected for one connection.
    pub fn from_drives(drives: Vec<RedirectedDrive>) -> Result<Self, RedirectedDriveFactoryError> {
        let initial_drive_ids = drives.iter().map(RedirectedDrive::device_id).collect();
        Self::from_drive_configuration(drives, initial_drive_ids)
    }

    /// Configures all logical-volume roots that may be activated during one connection.
    ///
    /// Only IDs in `initial_drive_ids` are announced during channel initialization.
    /// Other configured roots remain available for a later dynamic announcement.
    pub fn from_drive_configuration(
        drives: Vec<RedirectedDrive>,
        initial_drive_ids: Vec<u32>,
    ) -> Result<Self, RedirectedDriveFactoryError> {
        let mut device_ids = HashSet::with_capacity(drives.len());
        for drive in &drives {
            if !device_ids.insert(drive.device_id()) {
                return Err(RedirectedDriveFactoryError::DuplicateDeviceId(drive.device_id()));
            }
        }
        let mut selected_ids = HashSet::with_capacity(initial_drive_ids.len());
        for device_id in &initial_drive_ids {
            if !selected_ids.insert(*device_id) {
                return Err(RedirectedDriveFactoryError::RepeatedInitialSelection(*device_id));
            }
            if !device_ids.contains(device_id) {
                return Err(RedirectedDriveFactoryError::UnconfiguredInitialSelection(*device_id));
            }
        }

        Ok(Self {
            drives,
            initial_drive_ids,
            dynamic_drives: false,
            smartcard: false,
        })
    }

    /// Keeps RDPDR drive capability available for later logical-volume hotplug.
    #[must_use]
    pub fn with_dynamic_drives(mut self, enabled: bool) -> Self {
        self.dynamic_drives = enabled;
        self
    }

    /// Records whether products intend WinSCard smartcard redirection with this factory.
    ///
    /// This is product configuration state used when cloning or resolving factories (for example
    /// smartcard-only sessions with an empty drive list). The Windows backend always includes a
    /// `ScardSession`; MS-RDPESC IRPs only arrive after the portable channel announces the device
    /// via [`ironrdp_rdpdr::Rdpdr::with_smartcard`] / the client builder. Products must keep that
    /// announcement aligned with this flag and must not attach an empty-drive factory when the flag
    /// is `false`.
    #[must_use]
    pub fn with_smartcard(mut self, enabled: bool) -> Self {
        self.smartcard = enabled;
        self
    }

    /// Returns whether products requested WinSCard smartcard redirection on this factory.
    #[must_use]
    pub fn smartcard(&self) -> bool {
        self.smartcard
    }

    /// Returns the initial `(device_id, name)` pair for `Rdpdr::with_drives`.
    #[must_use]
    pub fn initial_drives(&self) -> Vec<(u32, String)> {
        self.initial_drive_ids
            .iter()
            .map(|device_id| {
                let drive = self
                    .drives
                    .iter()
                    .find(|drive| drive.device_id == *device_id)
                    .expect("initial RDPDR drive IDs are validated when the factory is created");
                (drive.device_id, drive.display_name.clone())
            })
            .collect()
    }

    /// Builds a backend with no active root handles.
    ///
    /// The portable channel activates initial drives with
    /// `RdpdrBackend::restore_drive` at the start of each server-announcement
    /// sequence.
    #[must_use]
    pub fn build(&self) -> WindowsRdpdrBackend {
        WindowsRdpdrBackend::from_drives(self.drives.clone())
    }
}

impl RdpdrBackendFactory for WindowsRdpdrBackendFactory {
    fn build_rdpdr_backend(&self) -> RdpdrBackendFactoryResult<RdpdrBackendProduct> {
        Ok(RdpdrBackendProduct::new(
            Box::new(self.build()),
            self.initial_drives()
                .into_iter()
                .map(|(device_id, name)| RdpdrDrive::new(device_id, name))
                .collect(),
        )
        .with_drive_hotplug(self.dynamic_drives))
    }
}

/// Invalid selected-drive factory configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirectedDriveFactoryError {
    /// More than one selected drive used the same RDPDR device ID.
    DuplicateDeviceId(u32),
    /// An initial drive ID was listed more than once.
    RepeatedInitialSelection(u32),
    /// An initial drive ID has no matching configured logical volume.
    UnconfiguredInitialSelection(u32),
}

impl fmt::Display for RedirectedDriveFactoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateDeviceId(device_id) => write!(f, "duplicate RDPDR device ID {device_id}"),
            Self::RepeatedInitialSelection(device_id) => write!(f, "duplicate initial RDPDR device ID {device_id}"),
            Self::UnconfiguredInitialSelection(device_id) => {
                write!(f, "initial RDPDR device ID {device_id} is not configured")
            }
        }
    }
}

impl core::error::Error for RedirectedDriveFactoryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_rdpdr::RdpdrBackend as _;

    #[test]
    fn factory_preserves_portable_initial_drive_names() {
        let factory = WindowsRdpdrBackendFactory::new(
            RedirectedDrive::new(1, "System", r"C:\", false).expect("valid system drive"),
        );

        assert_eq!(factory.initial_drives(), vec![(1, "System".to_owned())]);
    }

    #[test]
    fn factory_builds_a_portable_rdpdr_product() {
        let factory = WindowsRdpdrBackendFactory::new(
            RedirectedDrive::new(1, "System", r"C:\", false).expect("valid system drive"),
        );

        let product = factory.build_rdpdr_backend().expect("build RDPDR product");
        assert_eq!(
            product
                .initial_drives()
                .iter()
                .map(|drive| (drive.device_id(), drive.name()))
                .collect::<Vec<_>>(),
            vec![(1, "System")]
        );
    }

    #[test]
    fn factory_preserves_multiple_selected_drives() {
        let factory = WindowsRdpdrBackendFactory::from_drives(vec![
            RedirectedDrive::new(1, "System", r"C:\", false).expect("valid system drive"),
            RedirectedDrive::new(2, "Data", r"D:\", false).expect("valid data drive"),
        ])
        .expect("unique device IDs");

        assert_eq!(
            factory.initial_drives(),
            vec![(1, "System".to_owned()), (2, "Data".to_owned())]
        );
    }

    #[test]
    fn factory_keeps_unannounced_drives_available_to_the_backend() {
        let factory = WindowsRdpdrBackendFactory::from_drive_configuration(
            vec![
                RedirectedDrive::new(1, "System", r"C:\", false).expect("valid system drive"),
                RedirectedDrive::new(2, "Removable", r"E:\", false).expect("valid removable drive"),
            ],
            vec![1],
        )
        .expect("valid drive configuration");

        assert_eq!(factory.initial_drives(), vec![(1, "System".to_owned())]);
        let mut backend = factory.build();
        backend
            .add_drive(2)
            .expect("configured drive can be activated dynamically");
    }

    #[test]
    fn factory_tracks_smartcard_enablement() {
        let factory = WindowsRdpdrBackendFactory::from_drives(Vec::new())
            .expect("empty drive list is valid")
            .with_smartcard(true);
        assert!(factory.smartcard());
        assert!(factory.initial_drives().is_empty());
        assert!(!factory.with_smartcard(false).smartcard());
    }

    #[test]
    fn factory_tracks_dynamic_drive_enablement() {
        let product = WindowsRdpdrBackendFactory::from_drives(Vec::new())
            .expect("empty drive list is valid")
            .with_dynamic_drives(true)
            .build_rdpdr_backend()
            .expect("build hotplug-only product");

        assert!(product.drive_hotplug());
        assert!(product.initial_drives().is_empty());
    }

    #[test]
    fn factory_rejects_duplicate_device_ids() {
        let result = WindowsRdpdrBackendFactory::from_drives(vec![
            RedirectedDrive::new(1, "System", r"C:\", false).expect("valid system drive"),
            RedirectedDrive::new(1, "Data", r"D:\", false).expect("valid data drive"),
        ]);

        assert!(matches!(result, Err(RedirectedDriveFactoryError::DuplicateDeviceId(1))));
    }

    #[test]
    fn factory_rejects_invalid_initial_drive_ids() {
        let drive = RedirectedDrive::new(1, "System", r"C:\", false).expect("valid system drive");

        assert!(matches!(
            WindowsRdpdrBackendFactory::from_drive_configuration(vec![drive.clone()], vec![1, 1]),
            Err(RedirectedDriveFactoryError::RepeatedInitialSelection(1))
        ));
        assert!(matches!(
            WindowsRdpdrBackendFactory::from_drive_configuration(vec![drive], vec![2]),
            Err(RedirectedDriveFactoryError::UnconfiguredInitialSelection(2))
        ));
    }
}
