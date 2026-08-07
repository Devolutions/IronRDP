use core::fmt;
use std::collections::HashMap;

use ironrdp_core::impl_as_any;
use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_rdpdr::RdpdrBackend;
use ironrdp_rdpdr::pdu::efs::{
    AnyIoCtlCode, DecodedDeviceControlRequest, DeviceControlRequest, ServerDeviceAnnounceResponse, ServerDriveIoRequest,
};
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_svc::SvcMessage;

use super::control;
use super::directory;
use super::factory::{RedirectedDrive, WindowsRdpdrDriveRegistry};
use super::file;
use super::file_table::FileTable;
use super::handles::{FileHandle, OpenDirectoryError, RootDirectory};
use super::locks;
use super::path::RelativePath;
use super::pending::DeferredOperations;
use super::security;
use super::status::from_open_directory;
use super::volume;

const DEFAULT_MAX_OPEN_FILES: usize = 1_024;

/// Windows RDPDR filesystem backend for a fixed per-connection drive snapshot.
///
/// The static filesystem operation matrix is implemented incrementally. Until an
/// operation has a confined native implementation, the backend emits its typed
/// `STATUS_NOT_SUPPORTED` response rather than leaving an IRP unanswered.
#[derive(Debug)]
pub struct WindowsRdpdrBackend {
    drive_registry: WindowsRdpdrDriveRegistry,
    active_device_ids: std::collections::HashSet<u32>,
    pub(super) roots: HashMap<u32, RedirectedRoot>,
    pub(super) open_files: FileTable<OpenFile>,
    deferred_operations: DeferredOperations,
}

impl WindowsRdpdrBackend {
    #[cfg(test)]
    pub(crate) fn new(drives: Vec<RedirectedDrive>) -> Result<Self, BackendCreationError> {
        let active_device_ids = drives.iter().map(RedirectedDrive::device_id).collect();
        let drive_registry = WindowsRdpdrDriveRegistry::new(drives).map_err(BackendCreationError::Registry)?;
        Self::new_with_active_drives(drive_registry, active_device_ids)
    }

    pub(crate) fn new_with_active_drives(
        drive_registry: WindowsRdpdrDriveRegistry,
        active_device_ids: std::collections::HashSet<u32>,
    ) -> Result<Self, BackendCreationError> {
        let active_drives = drive_registry
            .drives_for(&active_device_ids)
            .map_err(BackendCreationError::Registry)?;
        let roots = open_roots(&active_drives)?;

        Ok(Self {
            drive_registry,
            active_device_ids,
            roots,
            open_files: FileTable::new(DEFAULT_MAX_OPEN_FILES),
            deferred_operations: DeferredOperations::new(),
        })
    }

    #[cfg(test)]
    pub(super) fn is_drive_active(&self, device_id: u32) -> bool {
        self.active_device_ids.contains(&device_id)
    }

    fn activate_drive(&mut self, device_id: u32) -> PduResult<()> {
        if self.active_device_ids.contains(&device_id) {
            return Err(pdu_other_err!("dynamic windows RDPDR drive is already active"));
        }

        let drive = self
            .drive_registry
            .drive(device_id)
            .map_err(|error| pdu_other_err!("read dynamic windows RDPDR drive registry").with_source(error))?
            .ok_or_else(|| pdu_other_err!("dynamic windows RDPDR drive is not available"))?;
        let root = RootDirectory::open(drive.root_path())
            .map_err(|error| pdu_other_err!("open dynamic windows RDPDR root").with_source(error))?;
        let previous = self.roots.insert(
            device_id,
            RedirectedRoot {
                root,
                read_only: drive.read_only(),
            },
        );
        debug_assert!(
            previous.is_none(),
            "an inactive RDPDR drive cannot retain a root handle"
        );
        self.active_device_ids.insert(device_id);
        Ok(())
    }

    fn deactivate_drive(&mut self, device_id: u32) -> PduResult<Vec<SvcMessage>> {
        if !self.active_device_ids.remove(&device_id) {
            return Err(pdu_other_err!("dynamic windows RDPDR drive is not active"));
        }

        let cancelled = self.deferred_operations.cancel_drive(device_id);
        self.open_files.retain(|file| file.device_id != device_id);
        self.roots.remove(&device_id);
        Ok(cancelled)
    }

    pub(super) fn schedule_waiting_lock(
        &mut self,
        request: ironrdp_rdpdr::pdu::efs::DeviceIoRequest,
        ranges: Vec<(u64, u64)>,
        exclusive: bool,
    ) -> Result<(), ironrdp_rdpdr::pdu::efs::NtStatus> {
        let handle = file::file_for_request(self, request.device_id, request.file_id)?
            .handle
            .try_clone()
            .map_err(|_| ironrdp_rdpdr::pdu::efs::NtStatus::UNSUCCESSFUL)?;
        self.deferred_operations
            .schedule_waiting_lock(request, ranges, exclusive, handle)
    }

    pub(super) fn cancel_deferred_file_operations(
        &mut self,
        device_id: u32,
        file_id: u32,
        handle: &FileHandle,
    ) -> Vec<SvcMessage> {
        self.deferred_operations.cancel_file(device_id, file_id, handle)
    }

    pub(super) fn schedule_directory_notification(
        &mut self,
        request: ironrdp_rdpdr::pdu::efs::ServerDriveNotifyChangeDirectoryRequest,
    ) -> Result<(), ironrdp_rdpdr::pdu::efs::NtStatus> {
        let handle = self
            .open_directory_for_notification(request.device_io_request.device_id, request.device_io_request.file_id)?;
        self.deferred_operations
            .schedule_directory_notification(request, handle)
    }

    pub(super) fn open_directory_for_notification(
        &self,
        device_id: u32,
        file_id: u32,
    ) -> Result<FileHandle, ironrdp_rdpdr::pdu::efs::NtStatus> {
        let path = file::file_for_request(self, device_id, file_id)?.path.clone();

        self.roots
            .get(&device_id)
            .ok_or(ironrdp_rdpdr::pdu::efs::NtStatus::INVALID_PARAMETER)?
            .root
            .open_relative_directory_for_notification(&path)
            .map_err(from_open_directory)?
            .map(|directory| directory.into_file_handle())
            .ok_or(ironrdp_rdpdr::pdu::efs::NtStatus::NOT_A_DIRECTORY)
    }
}

impl_as_any!(WindowsRdpdrBackend);

impl RdpdrBackend for WindowsRdpdrBackend {
    fn supports_drive_security(&self) -> bool {
        true
    }

    fn reset(&mut self) -> PduResult<()> {
        self.deferred_operations.reset();
        self.open_files.clear();
        self.roots.clear();
        self.active_device_ids.clear();
        Ok(())
    }

    fn restore_drive(&mut self, device_id: u32) -> PduResult<()> {
        self.activate_drive(device_id)
    }

    fn handle_server_device_announce_response(&mut self, _pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        Ok(())
    }

    fn handle_scard_call(&mut self, _req: DeviceControlRequest<ScardIoCtlCode>, _call: ScardCall) -> PduResult<()> {
        Ok(())
    }

    fn add_drive(&mut self, device_id: u32) -> PduResult<()> {
        self.activate_drive(device_id)
    }

    fn remove_drive(&mut self, device_id: u32) -> PduResult<Vec<SvcMessage>> {
        self.deactivate_drive(device_id)
    }

    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        match req {
            ServerDriveIoRequest::ServerCreateDriveRequest(req) => file::create(self, req),
            ServerDriveIoRequest::DeviceCloseRequest(req) => file::close(self, req),
            ServerDriveIoRequest::DeviceReadRequest(req) => file::read(self, req),
            ServerDriveIoRequest::DeviceWriteRequest(req) => file::write(self, req),
            ServerDriveIoRequest::DeviceFlushBuffersRequest(req) => file::flush(self, req),
            ServerDriveIoRequest::ServerDriveQueryInformationRequest(req) => file::query_information(self, req),
            ServerDriveIoRequest::ServerDriveQuerySecurityRequest(req) => security::query(self, req),
            ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(req) => volume::query_information(self, req),
            ServerDriveIoRequest::ServerDriveSetVolumeInformationRequest(req) => volume::set_information(self, req),
            ServerDriveIoRequest::ServerDriveSetInformationRequest(req) => file::set_information(self, req),
            ServerDriveIoRequest::ServerDriveSetSecurityRequest(req) => security::set(self, req),
            ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(req) => directory::query_information(self, req),
            ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(req) => directory::notify_change(self, req),
            ServerDriveIoRequest::ServerDriveLockControlRequest(req) => locks::control(self, req),
            ServerDriveIoRequest::DeviceControlRequest(req) => control::handle(self, req, &[]),
        }
    }

    fn handle_drive_device_control(
        &mut self,
        req: DecodedDeviceControlRequest<AnyIoCtlCode>,
    ) -> PduResult<Vec<SvcMessage>> {
        control::handle(self, req.request, &req.input_buffer)
    }

    fn poll_deferred_messages(&mut self) -> PduResult<Vec<SvcMessage>> {
        Ok(self.deferred_operations.poll())
    }
}

#[derive(Debug)]
pub(super) struct RedirectedRoot {
    pub(super) root: RootDirectory,
    pub(super) read_only: bool,
}

/// An opaque local handle bound to the RDPDR device that opened it.
#[derive(Debug)]
pub(super) struct OpenFile {
    pub(super) device_id: u32,
    pub(super) read_only: bool,
    /// Validated path used to reopen an independent directory watcher handle.
    pub(super) path: RelativePath,
    pub(super) handle: FileHandle,
    /// The directory selected by the most recent initial query for this file ID.
    ///
    /// MS-RDPEFS requires continuation queries to ignore their `Path` field, so
    /// this owns the native cursor selected by the preceding initial query.
    pub(super) directory_query_handle: Option<FileHandle>,
}

fn open_roots(drives: &[RedirectedDrive]) -> Result<HashMap<u32, RedirectedRoot>, BackendCreationError> {
    let mut roots = HashMap::with_capacity(drives.len());

    for drive in drives {
        let root = RootDirectory::open(drive.root_path()).map_err(|source| BackendCreationError::Root {
            device_id: drive.device_id(),
            source,
        })?;
        let previous = roots.insert(
            drive.device_id(),
            RedirectedRoot {
                root,
                read_only: drive.read_only(),
            },
        );
        debug_assert!(previous.is_none(), "the factory validates device IDs");
    }

    Ok(roots)
}

/// Failure while constructing a Windows backend from its selected-drive snapshot.
#[derive(Debug)]
pub(crate) enum BackendCreationError {
    Root { device_id: u32, source: OpenDirectoryError },
    Registry(super::factory::WindowsRdpdrDriveRegistryError),
}

impl fmt::Display for BackendCreationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root { device_id, source } => write!(f, "open redirected drive {device_id}: {source}"),
            Self::Registry(source) => source.fmt(f),
        }
    }
}

impl core::error::Error for BackendCreationError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Root { source, .. } => Some(source),
            Self::Registry(source) => Some(source),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_filesystem_device_announce_response_is_ignored() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let drive =
            RedirectedDrive::new(1, "System", format!(r"{system_drive}\"), false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::new(vec![drive]).expect("open system drive");

        backend
            .handle_server_device_announce_response(ServerDeviceAnnounceResponse {
                device_id: 0,
                result_code: ironrdp_rdpdr::pdu::efs::NtStatus::SUCCESS,
            })
            .expect("a smartcard response is not a Windows filesystem response");
        assert!(backend.is_drive_active(1));
    }

    #[test]
    fn reset_releases_roots_until_the_new_sequence_restores_them() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let drive =
            RedirectedDrive::new(1, "System", format!(r"{system_drive}\"), false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::new(vec![drive]).expect("open system drive");

        backend
            .handle_server_device_announce_response(ServerDeviceAnnounceResponse {
                device_id: 1,
                result_code: ironrdp_rdpdr::pdu::efs::NtStatus::ACCESS_DENIED,
            })
            .expect("record a rejected drive");
        assert!(backend.remove_drive(1).expect("release a rejected drive").is_empty());
        assert!(!backend.is_drive_active(1));

        RdpdrBackend::reset(&mut backend).expect("release prior RDPDR sequence");
        assert!(!backend.is_drive_active(1));
        assert!(!backend.roots.contains_key(&1));

        RdpdrBackend::restore_drive(&mut backend, 1).expect("restore configured drive");
        assert!(backend.is_drive_active(1));
        assert!(backend.roots.contains_key(&1));
    }
}
