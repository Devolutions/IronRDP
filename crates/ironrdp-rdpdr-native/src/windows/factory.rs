//! Windows RDPDR backend configuration.

use core::fmt;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ironrdp_rdpdr::{
    RdpdrBackendFactory, RdpdrBackendFactoryResult, RdpdrBackendProduct, RdpdrDrive, RdpdrPort, RdpdrPrinter,
};
use tracing::warn;

use super::backend::WindowsRdpdrBackend;
use super::handles::RootDirectory;
use super::logical_drives::{LogicalDrive, LogicalDriveSelection, LogicalDriveSet};
use super::port::{WindowsPortRedirection, resolve_ports};
use super::printer::{RdpdrWorkerThreadHooks, WindowsPrinterRedirection, WindowsPrinterSelection};

const DEFAULT_PRINTER_DEVICE_ID: u32 = u32::MAX - 1;
/// First device ID used for redirected ports, well above drive letters and named drives and
/// below the printer IDs counted down from the top of the range.
const FIRST_PORT_DEVICE_ID: u32 = 0x4000_0000;

/// Immutable logical-volume definition selected for Windows RDPDR redirection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedirectedDrive {
    device_id: u32,
    display_name: String,
    root_path: PathBuf,
    read_only: bool,
}

impl RedirectedDrive {
    /// Creates a drive definition for a logical Windows volume (`C:\`) or a directory on one.
    ///
    /// The root must be an absolute, normalized drive-letter path: relative, `..`, verbatim
    /// (`\\?\`), device, and UNC paths are rejected. The root is opened only when the drive is
    /// announced, without following reparse points.
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
        let root_path = root_path.into();
        if RootDirectory::open(&root_path).is_err() {
            return Err(RedirectedDriveError::InvalidRootPath);
        }

        Ok(Self {
            device_id,
            display_name,
            root_path,
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
    /// The root is not an absolute, normalized path on a logical volume.
    InvalidRootPath,
}

impl fmt::Display for RedirectedDriveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReservedDeviceId => f.write_str("device ID zero is reserved for RDPDR"),
            Self::EmptyDisplayName => f.write_str("redirected drive display name must not be empty"),
            Self::EmbeddedNul => f.write_str("redirected drive display name must not contain NUL"),
            Self::InvalidRootPath => {
                f.write_str("redirected drive root must be an absolute path on a logical volume (C:\\dir)")
            }
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
    logical_drives: Option<LogicalDriveSelection>,
    dynamic_drives: bool,
    smartcard: bool,
    printers: WindowsPrinterRedirection,
    worker_thread_hooks: Option<RdpdrWorkerThreadHooks>,
    ports: WindowsPortRedirection,
}

impl WindowsRdpdrBackendFactory {
    /// Configures the single logical-volume root supported by this baseline.
    #[must_use]
    pub fn new(drive: RedirectedDrive) -> Self {
        let device_id = drive.device_id();
        Self {
            drives: vec![drive],
            initial_drive_ids: vec![device_id],
            logical_drives: None,
            dynamic_drives: false,
            smartcard: false,
            printers: WindowsPrinterRedirection::disabled(),
            worker_thread_hooks: None,
            ports: WindowsPortRedirection::default(),
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
            logical_drives: None,
            dynamic_drives: false,
            smartcard: false,
            printers: WindowsPrinterRedirection::disabled(),
            worker_thread_hooks: None,
            ports: WindowsPortRedirection::default(),
        })
    }

    /// Registers every drive letter and announces the selected letters mapped at each connection.
    ///
    /// Drive letters use the device IDs `1..=26` (see [`LogicalDrive::device_id`]), which must not
    /// already be configured, and announce the names `A:` to `Z:`, which configured drives must
    /// not use (see [`LogicalDrive::is_drive_letter_name`]). Share `selection` with a
    /// [`LogicalDriveWatcher`](super::LogicalDriveWatcher) to redirect drives connected later.
    pub fn with_logical_drives(
        mut self,
        selection: LogicalDriveSelection,
    ) -> Result<Self, RedirectedDriveFactoryError> {
        for drive in LogicalDriveSet::ALL.iter() {
            if self
                .drives
                .iter()
                .any(|configured| configured.device_id() == drive.device_id())
            {
                return Err(RedirectedDriveFactoryError::DuplicateDeviceId(drive.device_id()));
            }
        }
        if let Some(configured) = self
            .drives
            .iter()
            .find(|configured| LogicalDrive::is_drive_letter_name(configured.display_name()))
        {
            return Err(RedirectedDriveFactoryError::ReservedDriveName(configured.device_id()));
        }
        self.drives
            .extend(LogicalDriveSet::ALL.iter().map(LogicalDrive::redirected_drive));
        self.logical_drives = Some(selection);
        Ok(self)
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

    /// Enables default-printer discovery when [`RdpdrBackendFactory::build_rdpdr_backend`] builds the product.
    ///
    /// Shorthand for [`Self::with_printer_redirection`] with [`WindowsPrinterSelection::Default`];
    /// `false` disables printer redirection.
    #[must_use]
    pub fn with_default_printer(self, enabled: bool) -> Self {
        self.with_printer_redirection(if enabled {
            WindowsPrinterRedirection::new(WindowsPrinterSelection::Default)
        } else {
            WindowsPrinterRedirection::disabled()
        })
    }

    /// Returns whether products redirect any printer.
    #[must_use]
    pub fn default_printer(&self) -> bool {
        self.printers.selection().is_some()
    }

    /// Selects the printers discovered when [`RdpdrBackendFactory::build_rdpdr_backend`] builds the product.
    #[must_use]
    pub fn with_printer_redirection(mut self, printers: WindowsPrinterRedirection) -> Self {
        self.printers = printers;
        self
    }

    /// Returns the printer redirection settings.
    #[must_use]
    pub fn printer_redirection(&self) -> &WindowsPrinterRedirection {
        &self.printers
    }

    /// Selects the serial and parallel ports announced by each product.
    ///
    /// Ports are enumerated when [`RdpdrBackendFactory::build_rdpdr_backend`] builds the
    /// product, so each connection announces the ports present at that time.
    #[must_use]
    pub fn with_port_redirection(mut self, ports: WindowsPortRedirection) -> Self {
        self.ports = ports;
        self
    }

    /// Returns the serial and parallel port selection.
    #[must_use]
    pub fn port_redirection(&self) -> &WindowsPortRedirection {
        &self.ports
    }

    /// Configures host lifetime hooks for native worker threads.
    #[must_use]
    pub fn with_worker_thread_hooks(mut self, hooks: RdpdrWorkerThreadHooks) -> Self {
        self.worker_thread_hooks = Some(hooks);
        self
    }

    /// Returns the initial `(device_id, name)` pair for `Rdpdr::with_drives`.
    ///
    /// With [`Self::with_logical_drives`], this includes the selected letters mapped now, without
    /// probing their volumes.
    #[must_use]
    pub fn initial_drives(&self) -> Vec<(u32, String)> {
        let logical_drives = self
            .logical_drives
            .as_ref()
            .map(LogicalDriveSelection::mapped)
            .unwrap_or_default();
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
            .chain(
                logical_drives
                    .iter()
                    .map(|drive| (drive.device_id(), drive.display_name())),
            )
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

    fn build_product(&self, printers: Vec<RdpdrPrinter>, ports: Vec<RdpdrPort>) -> RdpdrBackendProduct {
        let mut product = RdpdrBackendProduct::new(
            Box::new(
                WindowsRdpdrBackend::from_configuration(
                    self.drives.clone(),
                    printers.clone(),
                    self.worker_thread_hooks,
                )
                .with_ports(ports.clone()),
            ),
            self.initial_drives()
                .into_iter()
                .map(|(device_id, name)| RdpdrDrive::new(device_id, name))
                .collect(),
        )
        .with_drive_hotplug(self.dynamic_drives);
        if !printers.is_empty()
            && let Some(store) = self.printers.cache_store()
        {
            product = product.with_printer_cache_store(Arc::clone(store));
        }
        product.with_printers(printers).with_ports(ports)
    }
}

impl RdpdrBackendFactory for WindowsRdpdrBackendFactory {
    fn build_rdpdr_backend(&self) -> RdpdrBackendFactoryResult<RdpdrBackendProduct> {
        let mut next_device_id = Some(DEFAULT_PRINTER_DEVICE_ID);
        let printers = match self
            .printers
            .discover(|| self.allocate_printer_device_id(&mut next_device_id))
        {
            Ok(printers) => printers,
            Err(error) => {
                warn!(%error, "Printer discovery failed; continuing without printer redirection");
                Vec::new()
            }
        };
        let ports = resolve_ports(&self.ports, self.port_device_ids());
        Ok(self.build_product(printers, ports))
    }
}

impl WindowsRdpdrBackendFactory {
    /// Yields port device IDs counting up from [`FIRST_PORT_DEVICE_ID`], skipping configured drives.
    ///
    /// Printer IDs count down from [`DEFAULT_PRINTER_DEVICE_ID`], so the two ranges only meet
    /// past billions of devices.
    fn port_device_ids(&self) -> impl FnMut() -> Option<u32> + '_ {
        let mut next = Some(FIRST_PORT_DEVICE_ID);
        move || loop {
            let device_id = next?;
            next = device_id.checked_add(1).filter(|id| *id < DEFAULT_PRINTER_DEVICE_ID);
            if !self.drives.iter().any(|drive| drive.device_id() == device_id) {
                return Some(device_id);
            }
        }
    }

    /// Returns the next printer device ID at or below `next`, counting down and skipping drive IDs.
    fn allocate_printer_device_id(&self, next: &mut Option<u32>) -> Option<u32> {
        let mut device_id = (*next)?;
        while self.drives.iter().any(|drive| drive.device_id() == device_id) {
            device_id = device_id.checked_sub(1).filter(|device_id| *device_id != 0)?;
        }
        *next = device_id.checked_sub(1).filter(|device_id| *device_id != 0);
        Some(device_id)
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
    /// A configured drive uses a drive-letter name while drive letters are also redirected.
    ReservedDriveName(u32),
}

impl fmt::Display for RedirectedDriveFactoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateDeviceId(device_id) => write!(f, "duplicate RDPDR device ID {device_id}"),
            Self::RepeatedInitialSelection(device_id) => write!(f, "duplicate initial RDPDR device ID {device_id}"),
            Self::UnconfiguredInitialSelection(device_id) => {
                write!(f, "initial RDPDR device ID {device_id} is not configured")
            }
            Self::ReservedDriveName(device_id) => write!(
                f,
                "RDPDR drive {device_id} uses a drive-letter name reserved for redirected drive letters"
            ),
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
    fn factory_tracks_default_printer_enablement() {
        let factory = WindowsRdpdrBackendFactory::from_drives(Vec::new())
            .expect("empty drive list is valid")
            .with_default_printer(true);
        assert!(factory.default_printer());
        assert!(!factory.with_default_printer(false).default_printer());
    }

    #[test]
    fn factory_product_preserves_default_printer_metadata() {
        let factory = WindowsRdpdrBackendFactory::from_drives(Vec::new()).expect("empty drive list is valid");
        let product = factory.build_product(
            vec![
                RdpdrPrinter::new(
                    DEFAULT_PRINTER_DEVICE_ID,
                    "Office Printer".to_owned(),
                    "Office Driver".to_owned(),
                )
                .with_network(false),
            ],
            Vec::new(),
        );

        let printer = product.printer().expect("printer metadata is present");
        assert_eq!(printer.device_id(), DEFAULT_PRINTER_DEVICE_ID);
        assert_eq!(printer.name(), "Office Printer");
        assert_eq!(printer.driver_name(), "Office Driver");
        assert!(!printer.network());
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
    fn factory_announces_present_logical_drives() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let system_drive = LogicalDrive::from_letter(system_drive.chars().next().expect("nonempty system drive"))
            .expect("system drive is a letter");
        let factory = WindowsRdpdrBackendFactory::from_drives(vec![
            RedirectedDrive::new(27, "home", r"C:\Users", false).expect("valid named drive"),
        ])
        .expect("valid drive configuration")
        .with_logical_drives(LogicalDriveSelection::new([system_drive].into_iter().collect()))
        .expect("drive letters do not collide");

        assert_eq!(
            factory.initial_drives(),
            vec![
                (27, "home".to_owned()),
                (system_drive.device_id(), system_drive.display_name())
            ]
        );
        let unannounced = LogicalDrive::from_letter(if system_drive.letter() == 'Z' { 'Y' } else { 'Z' })
            .expect("valid drive letter");
        factory
            .build()
            .add_drive(unannounced.device_id())
            .expect("every drive letter can be activated dynamically");
    }

    #[test]
    fn logical_drives_reject_colliding_device_ids() {
        let result = WindowsRdpdrBackendFactory::from_drives(vec![
            RedirectedDrive::new(3, "Data", r"D:\", false).expect("valid data drive"),
        ])
        .expect("valid drive configuration")
        .with_logical_drives(LogicalDriveSelection::default());

        assert!(matches!(result, Err(RedirectedDriveFactoryError::DuplicateDeviceId(3))));
    }

    #[test]
    fn logical_drives_reject_drive_letter_names() {
        let named = |name| {
            WindowsRdpdrBackendFactory::from_drives(vec![
                RedirectedDrive::new(27, name, r"C:\Windows", false).expect("valid named drive"),
            ])
            .expect("valid drive configuration")
        };

        assert!(matches!(
            named("c:").with_logical_drives(LogicalDriveSelection::default()),
            Err(RedirectedDriveFactoryError::ReservedDriveName(27))
        ));
        assert!(
            named("Windows")
                .with_logical_drives(LogicalDriveSelection::default())
                .is_ok()
        );
    }

    #[test]
    fn redirected_drives_reject_invalid_roots() {
        for root in [
            r"relative\dir",
            r"C:\Users\..\Windows",
            r"\\?\C:\Users",
            r"\\server\share",
        ] {
            assert_eq!(
                RedirectedDrive::new(27, "Data", root, false),
                Err(RedirectedDriveError::InvalidRootPath),
                "{root}"
            );
        }
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

    #[test]
    fn printer_device_id_avoids_configured_drives() {
        let factory = WindowsRdpdrBackendFactory::from_drives(vec![
            RedirectedDrive::new(DEFAULT_PRINTER_DEVICE_ID, "reserved", r"C:\", false)
                .expect("all nonzero drive IDs are valid"),
        ])
        .expect("drive configuration is valid");

        let mut next = Some(DEFAULT_PRINTER_DEVICE_ID);
        assert_eq!(
            factory.allocate_printer_device_id(&mut next),
            Some(DEFAULT_PRINTER_DEVICE_ID - 1)
        );
        assert_eq!(
            factory.allocate_printer_device_id(&mut next),
            Some(DEFAULT_PRINTER_DEVICE_ID - 2)
        );
    }
}
