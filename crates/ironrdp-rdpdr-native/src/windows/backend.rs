use std::collections::HashMap;

use ironrdp_core::impl_as_any;
use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_rdpdr::RdpdrBackend;
use ironrdp_rdpdr::pdu::efs::{
    AnyIoCtlCode, DecodedDeviceControlRequest, DeviceControlRequest, ServerDeviceAnnounceResponse, ServerDriveIoRequest,
};
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_svc::SvcMessage;

use super::factory::RedirectedDrive;
use super::file_table::FileTable;
use super::handles::{FileHandle, RootDirectory};
use super::path::RelativePath;
use super::pending::DeferredOperations;
use super::scard::ScardSession;
use super::status::from_open_directory;
use super::{control, directory, file, locks, security, volume};

const DEFAULT_MAX_OPEN_FILES: usize = 1_024;

/// Windows RDPDR filesystem backend for a fixed per-connection drive snapshot.
///
/// The static filesystem operation matrix is implemented incrementally. Until an
/// operation has a confined native implementation, the backend emits its typed
/// `STATUS_NOT_SUPPORTED` response rather than leaving an IRP unanswered.
#[derive(Debug)]
pub struct WindowsRdpdrBackend {
    drives: HashMap<u32, RedirectedDrive>,
    pub(super) roots: HashMap<u32, RedirectedRoot>,
    pub(super) open_files: FileTable<OpenFile>,
    deferred_operations: DeferredOperations,
    scard: Option<ScardSession>,
}

impl WindowsRdpdrBackend {
    #[cfg(test)]
    pub(crate) fn from_drive(drive: RedirectedDrive) -> Self {
        Self::from_drives(vec![drive], false)
    }

    pub(crate) fn from_drives(drives: Vec<RedirectedDrive>, smartcard: bool) -> Self {
        Self {
            drives: drives.into_iter().map(|drive| (drive.device_id(), drive)).collect(),
            roots: HashMap::new(),
            open_files: FileTable::new(DEFAULT_MAX_OPEN_FILES),
            deferred_operations: DeferredOperations::new(),
            scard: smartcard.then(ScardSession::new),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_active_drive(drive: RedirectedDrive) -> Self {
        let device_id = drive.device_id();
        let mut backend = Self::from_drive(drive);
        backend
            .activate_drive(device_id)
            .expect("test redirected drive must activate");
        backend
    }

    fn activate_drive(&mut self, device_id: u32) -> PduResult<()> {
        let drive = self
            .drives
            .get(&device_id)
            .ok_or_else(|| pdu_other_err!("windows RDPDR drive is not configured"))?;
        if self.roots.contains_key(&device_id) {
            return Err(pdu_other_err!("windows RDPDR drive is already active"));
        }

        let root = RootDirectory::open(drive.root_path())
            .map_err(|error| pdu_other_err!("open configured Windows RDPDR volume root").with_source(error))?;
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
        Ok(())
    }

    fn deactivate_drive(&mut self, device_id: u32) -> PduResult<Vec<SvcMessage>> {
        if !self.drives.contains_key(&device_id) || !self.roots.contains_key(&device_id) {
            return Err(pdu_other_err!("windows RDPDR drive is not configured"));
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

    pub(super) fn cancel_deferred_file_operations(&mut self, device_id: u32, file_id: u32) -> Vec<SvcMessage> {
        self.deferred_operations.cancel_file(device_id, file_id)
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
        if let Some(scard) = self.scard.as_mut() {
            scard.reset();
        }
        self.open_files.clear();
        self.roots.clear();
        Ok(())
    }

    fn restore_drive(&mut self, device_id: u32) -> PduResult<()> {
        self.activate_drive(device_id)
    }

    fn handle_server_device_announce_response(&mut self, _pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        Ok(())
    }

    fn handle_scard_call(
        &mut self,
        req: DeviceControlRequest<ScardIoCtlCode>,
        call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        // Always complete MS-RDPESC IRPs. The channel may announce device ID 0
        // even when the factory left `with_smartcard(false)`, and dropping the
        // IRP hangs the server. Lazy-create the stub session on first use so
        // announce and backend cannot diverge.
        self.scard.get_or_insert_with(ScardSession::new).handle_call(req, call)
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
        let mut messages = self.deferred_operations.poll();
        if let Some(scard) = self.scard.as_mut() {
            messages.extend(scard.poll());
        }
        Ok(messages)
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

#[cfg(test)]
mod tests {
    use ironrdp_rdpdr::pdu::efs::{DeviceIoRequest, MajorFunction, MinorFunction};
    use ironrdp_rdpdr::pdu::esc::{EstablishContextCall, ScardIoCtlCode, Scope};

    use super::*;

    #[test]
    fn backend_restores_each_selected_drive() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let root = format!(r"{system_drive}\");
        let mut backend = WindowsRdpdrBackend::from_drives(
            vec![
                RedirectedDrive::new(1, "System", &root, false).expect("valid system drive"),
                RedirectedDrive::new(2, "System copy", root, false).expect("valid system drive copy"),
            ],
            false,
        );

        ironrdp_rdpdr::RdpdrBackend::restore_drive(&mut backend, 1).expect("restore first selected drive");
        ironrdp_rdpdr::RdpdrBackend::restore_drive(&mut backend, 2).expect("restore second selected drive");

        assert!(backend.roots.contains_key(&1));
        assert!(backend.roots.contains_key(&2));
    }

    #[test]
    fn scard_irp_completes_when_factory_smartcard_flag_is_off() {
        // Channel announce can enable device 0 without factory.with_smartcard(true).
        let mut backend = WindowsRdpdrBackend::from_drives(Vec::new(), false);
        assert!(backend.scard.is_none());

        let req = DeviceControlRequest {
            header: DeviceIoRequest {
                device_id: 0,
                file_id: 0,
                completion_id: 11,
                major_function: MajorFunction::DeviceControl,
                minor_function: MinorFunction::from(0),
            },
            output_buffer_length: 2048,
            input_buffer_length: 0,
            io_control_code: ScardIoCtlCode::EstablishContext,
        };

        let messages = ironrdp_rdpdr::RdpdrBackend::handle_scard_call(
            &mut backend,
            req,
            ScardCall::EstablishContextCall(EstablishContextCall { scope: Scope::User }),
        )
        .expect("lazy stub must complete the IRP");

        assert_eq!(messages.len(), 1);
        assert!(backend.scard.is_some());
    }
}
