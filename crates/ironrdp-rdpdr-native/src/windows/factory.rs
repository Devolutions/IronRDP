use core::fmt;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ironrdp_rdpdr::backend::{RdpdrBackendFactory, RdpdrBackendFactoryResult, RdpdrBackendProduct, RdpdrDrive};

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

    pub(crate) fn device_id(&self) -> u32 {
        self.device_id
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
    /// Device ID zero is reserved for the existing smartcard channel.
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

/// Thread-safe registry of Windows volume definitions available for one RDPDR
/// connection lifetime.
///
/// A host may discover a logical volume after connection startup. The registry
/// lets the ActiveX apartment make that definition visible to its worker-owned
/// backend before the corresponding RDPDR device announcement is requested.
#[derive(Clone, Debug)]
pub struct WindowsRdpdrDriveRegistry {
    drives: Arc<Mutex<HashMap<u32, RedirectedDrive>>>,
}

impl WindowsRdpdrDriveRegistry {
    pub(crate) fn new(drives: Vec<RedirectedDrive>) -> Result<Self, WindowsRdpdrDriveRegistryError> {
        let registry = Self {
            drives: Arc::new(Mutex::new(HashMap::new())),
        };
        registry.register_drives(drives)?;
        Ok(registry)
    }

    /// Registers newly discovered logical volumes for later dynamic activation.
    ///
    /// Existing IDs must retain their original immutable definition. This avoids
    /// retargeting a live RDPDR device to a different local filesystem root.
    pub fn register_drives(
        &self,
        drives: impl IntoIterator<Item = RedirectedDrive>,
    ) -> Result<(), WindowsRdpdrDriveRegistryError> {
        let mut registry = self
            .drives
            .lock()
            .map_err(|_| WindowsRdpdrDriveRegistryError::Poisoned)?;

        let mut additions = HashMap::new();
        for drive in drives {
            match registry.get(&drive.device_id()) {
                Some(existing) if existing != &drive => {
                    return Err(WindowsRdpdrDriveRegistryError::ConflictingDeviceId(drive.device_id()));
                }
                Some(_) => {}
                None => match additions.get(&drive.device_id()) {
                    Some(existing) if existing != &drive => {
                        return Err(WindowsRdpdrDriveRegistryError::ConflictingDeviceId(drive.device_id()));
                    }
                    Some(_) => {}
                    None => {
                        additions.insert(drive.device_id(), drive);
                    }
                },
            }
        }

        registry.extend(additions);
        Ok(())
    }

    pub(crate) fn drive(&self, device_id: u32) -> Result<Option<RedirectedDrive>, WindowsRdpdrDriveRegistryError> {
        let registry = self
            .drives
            .lock()
            .map_err(|_| WindowsRdpdrDriveRegistryError::Poisoned)?;
        Ok(registry.get(&device_id).cloned())
    }

    pub(crate) fn drives_for(
        &self,
        device_ids: &HashSet<u32>,
    ) -> Result<Vec<RedirectedDrive>, WindowsRdpdrDriveRegistryError> {
        let registry = self
            .drives
            .lock()
            .map_err(|_| WindowsRdpdrDriveRegistryError::Poisoned)?;

        device_ids
            .iter()
            .map(|device_id| {
                registry
                    .get(device_id)
                    .cloned()
                    .ok_or(WindowsRdpdrDriveRegistryError::UnknownDeviceId(*device_id))
            })
            .collect()
    }
}

/// Failure while updating or reading a Windows dynamic-drive registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowsRdpdrDriveRegistryError {
    /// A device ID was already associated with a different logical volume.
    ConflictingDeviceId(u32),
    /// A requested device has no registered logical-volume definition.
    UnknownDeviceId(u32),
    /// The registry cannot safely recover after a writer panicked while holding its lock.
    Poisoned,
}

impl fmt::Display for WindowsRdpdrDriveRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConflictingDeviceId(device_id) => {
                write!(f, "RDPDR device ID {device_id} is already mapped to a different volume")
            }
            Self::UnknownDeviceId(device_id) => write!(f, "RDPDR device ID {device_id} has no registered volume"),
            Self::Poisoned => f.write_str("RDPDR dynamic-drive registry is unavailable"),
        }
    }
}

impl core::error::Error for WindowsRdpdrDriveRegistryError {}

/// Builds an isolated Windows RDPDR backend for each connection attempt.
#[derive(Clone, Debug)]
pub struct WindowsRdpdrBackendFactory {
    drive_registry: WindowsRdpdrDriveRegistry,
    initial_device_ids: HashSet<u32>,
}

impl WindowsRdpdrBackendFactory {
    /// Creates a factory for the immutable selected-drive snapshot.
    pub fn new(drives: Vec<RedirectedDrive>) -> Result<Self, RedirectedDriveFactoryError> {
        let mut device_ids = HashSet::with_capacity(drives.len());
        for drive in &drives {
            if !device_ids.insert(drive.device_id()) {
                return Err(RedirectedDriveFactoryError::DuplicateDeviceId(drive.device_id()));
            }
        }

        let initial_device_ids = drives.iter().map(RedirectedDrive::device_id).collect();
        let drive_registry = WindowsRdpdrDriveRegistry::new(drives).map_err(RedirectedDriveFactoryError::Registry)?;

        Ok(Self {
            drive_registry,
            initial_device_ids,
        })
    }

    /// Returns the registry shared with the backend produced by this factory.
    #[must_use]
    pub fn drive_registry(&self) -> WindowsRdpdrDriveRegistry {
        self.drive_registry.clone()
    }

    /// Restricts the drives announced at connection time while retaining all
    /// supplied drives for safe dynamic activation during that connection.
    #[must_use]
    pub fn with_initial_device_ids(mut self, initial_device_ids: impl IntoIterator<Item = u32>) -> Self {
        self.initial_device_ids = initial_device_ids.into_iter().collect();
        self
    }
}

impl RdpdrBackendFactory for WindowsRdpdrBackendFactory {
    fn build_rdpdr_backend(&self) -> RdpdrBackendFactoryResult<RdpdrBackendProduct> {
        let backend =
            WindowsRdpdrBackend::new_with_active_drives(self.drive_registry.clone(), self.initial_device_ids.clone())?;
        let initial_drives = self
            .drive_registry
            .drives_for(&self.initial_device_ids)?
            .iter()
            .map(|drive| RdpdrDrive {
                device_id: drive.device_id,
                name: drive.display_name.clone(),
            })
            .collect();

        Ok(RdpdrBackendProduct::new(Box::new(backend), initial_drives))
    }
}

/// Invalid selected-drive factory configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedirectedDriveFactoryError {
    /// More than one selected drive used the same RDPDR device ID.
    DuplicateDeviceId(u32),
    /// The dynamic-drive registry rejected a volume definition.
    Registry(WindowsRdpdrDriveRegistryError),
}

impl fmt::Display for RedirectedDriveFactoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateDeviceId(device_id) => write!(f, "duplicate RDPDR device ID {device_id}"),
            Self::Registry(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for RedirectedDriveFactoryError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_drive_factory_opens_the_system_volume() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let root = format!(r"{system_drive}\");
        let drive = RedirectedDrive::new(1, "System", root, false).expect("valid selected drive");
        let factory = WindowsRdpdrBackendFactory::new(vec![drive]).expect("unique device ID");

        let product = factory.build_rdpdr_backend().expect("open selected root");

        assert_eq!(
            product.initial_drives,
            vec![RdpdrDrive {
                device_id: 1,
                name: "System".to_owned(),
            }]
        );
        assert!(product.backend.as_any().is::<WindowsRdpdrBackend>());
    }

    #[test]
    fn selected_drive_factory_rejects_duplicate_device_ids() {
        let first = RedirectedDrive::new(1, "First", r"C:\", false).expect("valid first drive");
        let second = RedirectedDrive::new(1, "Second", r"D:\", false).expect("valid second drive");

        assert!(matches!(
            WindowsRdpdrBackendFactory::new(vec![first, second]),
            Err(RedirectedDriveFactoryError::DuplicateDeviceId(1))
        ));
    }

    #[test]
    fn dynamic_drive_factory_keeps_unselected_roots_inactive_until_requested() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let root = format!(r"{system_drive}\");
        let initial = RedirectedDrive::new(1, "Initial", root.clone(), false).expect("valid initial drive");
        let dynamic = RedirectedDrive::new(2, "Dynamic", root, false).expect("valid dynamic drive");
        let factory = WindowsRdpdrBackendFactory::new(vec![initial, dynamic])
            .expect("unique device IDs")
            .with_initial_device_ids([1]);

        let mut product = factory.build_rdpdr_backend().expect("open selected root");
        let backend = product
            .backend
            .as_any_mut()
            .downcast_mut::<WindowsRdpdrBackend>()
            .expect("Windows backend");
        assert!(backend.roots.contains_key(&1));
        assert!(!backend.roots.contains_key(&2));

        ironrdp_rdpdr::RdpdrBackend::add_drive(backend, 2).expect("activate dynamic drive");
        assert!(backend.roots.contains_key(&2));
        ironrdp_rdpdr::RdpdrBackend::remove_drive(backend, 2).expect("remove dynamic drive");
        assert!(!backend.roots.contains_key(&2));
    }

    #[test]
    fn failed_dynamic_activation_keeps_the_device_retryable() {
        let unavailable =
            RedirectedDrive::new(1, "Unavailable", "not-a-volume", false).expect("valid drive definition");
        let factory = WindowsRdpdrBackendFactory::new(vec![unavailable])
            .expect("unique device ID")
            .with_initial_device_ids([]);
        let mut product = factory.build_rdpdr_backend().expect("build inactive backend");
        let backend = product
            .backend
            .as_any_mut()
            .downcast_mut::<WindowsRdpdrBackend>()
            .expect("Windows backend");

        assert!(ironrdp_rdpdr::RdpdrBackend::add_drive(backend, 1).is_err());
        assert!(!backend.is_drive_active(1));
        assert!(!backend.roots.contains_key(&1));
    }

    #[test]
    fn dynamic_registry_makes_late_volume_definitions_available_to_the_backend() {
        let initial = RedirectedDrive::new(1, "Initial", r"C:\", false).expect("valid initial drive");
        let factory = WindowsRdpdrBackendFactory::new(vec![initial])
            .expect("unique device ID")
            .with_initial_device_ids([]);
        let registry = factory.drive_registry();
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let dynamic =
            RedirectedDrive::new(2, "Dynamic", format!(r"{system_drive}\"), false).expect("valid dynamic drive");
        registry.register_drives([dynamic]).expect("register dynamic volume");

        let mut product = factory.build_rdpdr_backend().expect("build inactive backend");
        let backend = product
            .backend
            .as_any_mut()
            .downcast_mut::<WindowsRdpdrBackend>()
            .expect("Windows backend");
        ironrdp_rdpdr::RdpdrBackend::add_drive(backend, 2).expect("activate registered dynamic volume");

        assert!(backend.roots.contains_key(&2));
    }

    #[test]
    fn dynamic_drive_registration_is_atomic_when_a_later_definition_conflicts() {
        let initial = RedirectedDrive::new(1, "Initial", r"C:\", false).expect("valid initial drive");
        let registry = WindowsRdpdrDriveRegistry::new(vec![initial]).expect("create drive registry");
        let valid_addition = RedirectedDrive::new(2, "Dynamic", r"D:\", false).expect("valid dynamic drive");
        let conflicting = RedirectedDrive::new(1, "Conflicting", r"E:\", false).expect("valid conflicting drive");

        assert!(matches!(
            registry.register_drives([valid_addition, conflicting]),
            Err(WindowsRdpdrDriveRegistryError::ConflictingDeviceId(1))
        ));
        assert!(
            registry
                .drive(2)
                .expect("read drive registry after rejected batch")
                .is_none()
        );
    }
}
