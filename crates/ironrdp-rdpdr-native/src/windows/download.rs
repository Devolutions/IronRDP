use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use ironrdp_core::impl_as_any;
use ironrdp_pdu::{PduResult, encode_err, pdu_other_err};
use ironrdp_rdpdr::backend::noop::NoopRdpdrBackend;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    Boolean, ClientDriveLockControlResponse, ClientDriveNotifyChangeDirectoryResponse,
    ClientDriveQueryDirectoryResponse, ClientDriveQueryInformationResponse, ClientDriveQuerySecurityResponse,
    ClientDriveQueryVolumeInformationResponse, ClientDriveSetInformationResponse, ClientDriveSetSecurityResponse,
    CreateDisposition, CreateOptions, DesiredAccess, DeviceCloseRequest, DeviceCloseResponse, DeviceControlRequest,
    DeviceControlResponse, DeviceCreateRequest, DeviceCreateResponse, DeviceFlushBuffersRequest,
    DeviceFlushBuffersResponse, DeviceIoResponse, DeviceReadResponse, DeviceWriteRequest, DeviceWriteResponse,
    FileAttributes, FileInformationClass, FileInformationClassLevel, Information, NtStatus,
    ServerDeviceAnnounceResponse, ServerDriveIoRequest, ServerDriveQueryInformationRequest,
    ServerDriveSetInformationRequest,
};
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_rdpdr::{RdpdrBackend, RdpdrBackendFactory, RdpdrBackendFactoryResult, RdpdrBackendProduct, RdpdrDrive};
use ironrdp_svc::SvcMessage;

use super::file_table::FileTable;
use super::handles::{FILE_CREATED_INFORMATION, FileHandle, FileOpenOptions, RootDirectory};
use super::path::RelativePath;
use super::status::{from_ntstatus, from_open_file, from_path_policy};

const DEVICE_ID: u32 = 1;
const MAX_OPEN_FILES: usize = 1_024;
const MAX_WRITE_LENGTH: usize = 1_024 * 1_024;
const MAX_FILE_LENGTH: u64 = 1_024 * 1_024 * 1_024;
const MAX_DOWNLOAD_MESSAGE_SIZE: usize = 4 /* SharedHeader */
    + 20 /* DeviceIoRequest */
    + 4 /* Length */
    + 8 /* Offset */
    + 20 /* Padding */
    + MAX_WRITE_LENGTH;

/// A pinned directory for the download-only RDPDR device.
#[derive(Clone, Debug)]
pub struct WindowsDownloadSinkFactory {
    root: Arc<DownloadRoot>,
}

impl WindowsDownloadSinkFactory {
    /// Opens and pins an existing absolute drive-letter directory without following reparse points.
    ///
    /// Untrusted local processes must not be able to modify the directory or create hard links to its files.
    pub fn new(directory: &Path) -> io::Result<Self> {
        let root = RootDirectory::open(directory).map_err(io::Error::other)?;
        let directory = root.pin_for_downloads().map_err(io::Error::other)?;
        Ok(Self {
            root: Arc::new(DownloadRoot {
                directory,
                files: Mutex::new(DownloadFiles::default()),
            }),
        })
    }

    /// Builds a download-only backend for one connection.
    #[must_use]
    pub fn build(&self) -> WindowsDownloadSink {
        WindowsDownloadSink {
            root: Arc::clone(&self.root),
            open_files: FileTable::new(MAX_OPEN_FILES),
            active: false,
            noop: NoopRdpdrBackend,
        }
    }
}

impl RdpdrBackendFactory for WindowsDownloadSinkFactory {
    fn build_rdpdr_backend(&self) -> RdpdrBackendFactoryResult<RdpdrBackendProduct> {
        Ok(RdpdrBackendProduct::new(
            Box::new(self.build()),
            vec![RdpdrDrive::new(DEVICE_ID, "DL".to_owned())],
        ))
    }
}

/// A download-only Windows RDPDR backend.
#[derive(Debug)]
pub struct WindowsDownloadSink {
    root: Arc<DownloadRoot>,
    open_files: FileTable<DownloadFile>,
    active: bool,
    noop: NoopRdpdrBackend,
}

#[derive(Debug)]
struct DownloadRoot {
    directory: FileHandle,
    files: Mutex<DownloadFiles>,
}

#[derive(Debug, Default)]
struct DownloadFiles {
    /// INVARIANT: counts open files, pending cleanups, and creates in progress across all connections.
    reserved: usize,
    pending_cleanup: Vec<FileHandle>,
}

impl DownloadRoot {
    fn retry_cleanup(&self) -> Result<(), NtStatus> {
        let mut files = self.files.lock().expect("download root lock is not poisoned");
        Self::retry_cleanup_locked(&mut files)
    }

    fn reserve_file(&self) -> Result<(), NtStatus> {
        let mut files = self.files.lock().expect("download root lock is not poisoned");
        Self::retry_cleanup_locked(&mut files)?;
        if MAX_OPEN_FILES <= files.reserved {
            return Err(NtStatus::INSUFFICIENT_RESOURCES);
        }
        files.reserved += 1;
        Ok(())
    }

    fn release_reservation(&self) {
        let mut files = self.files.lock().expect("download root lock is not poisoned");
        files.reserved = files.reserved.checked_sub(1).expect("download has a reserved handle");
    }

    fn close_file(&self, handle: FileHandle, published: bool) -> Result<(), NtStatus> {
        let mut files = self.files.lock().expect("download root lock is not poisoned");
        if !published && let Err(error) = handle.set_delete_pending(true) {
            tracing::warn!(?error, "Failed to clean up an incomplete download");
            files.pending_cleanup.push(handle);
            return Err(from_ntstatus(error));
        }
        drop(handle);
        files.reserved = files.reserved.checked_sub(1).expect("download has a reserved handle");
        Ok(())
    }

    fn retry_cleanup_locked(files: &mut DownloadFiles) -> Result<(), NtStatus> {
        let before = files.pending_cleanup.len();
        let mut failure = None;
        files
            .pending_cleanup
            .retain(|handle| match handle.set_delete_pending(true) {
                Ok(()) => false,
                Err(error) => {
                    failure.get_or_insert(from_ntstatus(error));
                    true
                }
            });
        files.reserved = files
            .reserved
            .checked_sub(before - files.pending_cleanup.len())
            .expect("pending cleanups have reserved handles");
        failure.map_or(Ok(()), Err)
    }
}

#[derive(Debug)]
struct DownloadFile {
    handle: Option<FileHandle>,
    root: Arc<DownloadRoot>,
    may_delete: bool,
    published: bool,
}

impl Drop for DownloadFile {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = self.root.close_file(handle, self.published);
        }
    }
}

impl WindowsDownloadSink {
    fn retry_cleanup(&self) -> PduResult<()> {
        self.root.retry_cleanup().map_err(|error| {
            tracing::warn!(?error, "Failed to retry incomplete download cleanup");
            pdu_other_err!("retrying incomplete download cleanup")
        })
    }

    fn close(&mut self, request: DeviceCloseRequest) -> PduResult<Vec<SvcMessage>> {
        let status = if self.active && request.device_io_request.device_id == DEVICE_ID {
            match self.open_files.remove(request.device_io_request.file_id) {
                Some(mut file) => {
                    let handle = file.handle.take().expect("open downloads retain their handles");
                    self.root
                        .close_file(handle, file.published)
                        .map_or_else(|status| status, |_| NtStatus::SUCCESS)
                }
                None => NtStatus::INVALID_HANDLE,
            }
        } else {
            NtStatus::INVALID_HANDLE
        };
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCloseResponse(
            DeviceCloseResponse {
                device_io_response: DeviceIoResponse::new(request.device_io_request, status),
            },
        ))])
    }

    fn create(&mut self, request: DeviceCreateRequest) -> PduResult<Vec<SvcMessage>> {
        let (status, file_id, information) = match self.create_inner(&request) {
            Ok(file_id) => (NtStatus::SUCCESS, file_id, Information::file_superseded()),
            Err(status) => (status, 0, Information::empty()),
        };
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(
            DeviceCreateResponse {
                device_io_reply: DeviceIoResponse::new(request.device_io_request, status),
                file_id,
                information,
            },
        ))])
    }

    fn create_inner(&mut self, request: &DeviceCreateRequest) -> Result<u32, NtStatus> {
        if !self.active || request.device_io_request.device_id != DEVICE_ID {
            return Err(NtStatus::INVALID_PARAMETER);
        }
        let name = flat_name(&request.path)?;
        let allowed_access = DesiredAccess::GENERIC_WRITE
            | DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE
            | DesiredAccess::FILE_APPEND_DATA_OR_FILE_ADD_SUBDIRECTORY
            | DesiredAccess::FILE_WRITE_ATTRIBUTES
            | DesiredAccess::FILE_WRITE_EA
            | DesiredAccess::FILE_READ_ATTRIBUTES
            | DesiredAccess::READ_CONTROL
            | DesiredAccess::DELETE
            | DesiredAccess::SYNCHRONIZE;
        let allowed_options = CreateOptions::FILE_NON_DIRECTORY_FILE | CreateOptions::FILE_SYNCHRONOUS_IO_NONALERT;
        if request.create_disposition != CreateDisposition::FILE_CREATE
            || request.desired_access.bits() & !allowed_access.bits() != 0
            || !request
                .desired_access
                .intersects(DesiredAccess::GENERIC_WRITE | DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE)
            || request.create_options.bits() & !allowed_options.bits() != 0
            || request.allocation_size != 0
        {
            return Err(NtStatus::ACCESS_DENIED);
        }

        let reservation = self
            .open_files
            .reserve_file_id()
            .map_err(|_| NtStatus::INSUFFICIENT_RESOURCES)?;
        if let Err(status) = self.root.reserve_file() {
            self.open_files.release_file_id(reservation);
            return Err(status);
        }
        let options = FileOpenOptions {
            desired_access: request.desired_access.bits()
                | (DesiredAccess::DELETE | DesiredAccess::SYNCHRONIZE | DesiredAccess::FILE_READ_ATTRIBUTES).bits(),
            allocation_size: None,
            file_attributes: FileAttributes::FILE_ATTRIBUTE_NORMAL.bits(),
            share_access: 0,
            create_disposition: u32::from(CreateDisposition::FILE_CREATE),
            create_options: allowed_options.bits(),
        };
        let (handle, information) = match self.root.directory.open_child(&name, options) {
            Ok(result) => result,
            Err(error) => {
                self.root.release_reservation();
                self.open_files.release_file_id(reservation);
                return Err(from_open_file(error));
            }
        };
        let file = DownloadFile {
            handle: Some(handle),
            root: Arc::clone(&self.root),
            may_delete: request.desired_access.contains(DesiredAccess::DELETE),
            published: false,
        };
        if information != FILE_CREATED_INFORMATION {
            self.open_files.release_file_id(reservation);
            return Err(NtStatus::UNSUCCESSFUL);
        }
        Ok(self.open_files.insert_reserved(reservation, file))
    }

    fn write(&self, request: DeviceWriteRequest) -> PduResult<Vec<SvcMessage>> {
        let result = (|| {
            if MAX_WRITE_LENGTH < request.write_data.len()
                || request
                    .offset
                    .checked_add(u64::try_from(request.write_data.len()).map_err(|_| NtStatus::INVALID_PARAMETER)?)
                    .is_none_or(|end| MAX_FILE_LENGTH < end)
            {
                return Err(NtStatus::INVALID_PARAMETER);
            }
            let handle = self.writable(request.device_io_request.device_id, request.device_io_request.file_id)?;
            let offset = i64::try_from(request.offset).map_err(|_| NtStatus::INVALID_PARAMETER)?;
            let completion = handle
                .write_at(offset, &request.write_data)
                .map_err(|_| NtStatus::INVALID_PARAMETER)?;
            if completion.status.0 < 0 {
                return Err(from_ntstatus(completion.status));
            }
            u32::try_from(completion.transferred).map_err(|_| NtStatus::UNSUCCESSFUL)
        })();
        let (status, length) = result.map_or_else(|status| (status, 0), |length| (NtStatus::SUCCESS, length));
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceWriteResponse(
            DeviceWriteResponse {
                device_io_reply: DeviceIoResponse::new(request.device_io_request, status),
                length,
            },
        ))])
    }

    fn flush(&self, request: DeviceFlushBuffersRequest) -> PduResult<Vec<SvcMessage>> {
        let status = self
            .file(request.device_io_request.device_id, request.device_io_request.file_id)
            .and_then(|file| {
                file.handle
                    .as_ref()
                    .expect("open downloads retain their handles")
                    .flush()
                    .map_err(from_ntstatus)
            })
            .map_or_else(|status| status, |_| NtStatus::SUCCESS);
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceFlushBuffersResponse(
            DeviceFlushBuffersResponse {
                device_io_response: DeviceIoResponse::new(request.device_io_request, status),
            },
        ))])
    }

    fn query_information(&self, request: ServerDriveQueryInformationRequest) -> PduResult<Vec<SvcMessage>> {
        let result = (|| {
            let file = self.file(request.device_io_request.device_id, request.device_io_request.file_id)?;
            match request.file_info_class_lvl {
                FileInformationClassLevel::FILE_BASIC_INFORMATION
                | FileInformationClassLevel::FILE_STANDARD_INFORMATION
                | FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION => super::file::query_handle_information(
                    file.handle.as_ref().expect("open downloads retain their handles"),
                    &request.file_info_class_lvl,
                ),
                _ => Err(NtStatus::ACCESS_DENIED),
            }
        })();
        let (status, buffer) = result.map_or_else(|status| (status, None), |(status, buffer)| (status, Some(buffer)));
        Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryInformationResponse(
            ClientDriveQueryInformationResponse {
                device_io_response: DeviceIoResponse::new(request.device_io_request, status),
                buffer,
            },
        ))])
    }

    fn set_information(&mut self, request: ServerDriveSetInformationRequest) -> PduResult<Vec<SvcMessage>> {
        let status = self
            .set_information_inner(&request)
            .map_or_else(|status| status, |_| NtStatus::SUCCESS);
        let response = ClientDriveSetInformationResponse::new(&request, status).map_err(|error| encode_err!(error))?;
        Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveSetInformationResponse(
            response,
        ))])
    }

    fn set_information_inner(&mut self, request: &ServerDriveSetInformationRequest) -> Result<(), NtStatus> {
        let file_id = request.device_io_request.file_id;
        let handle = self.writable(request.device_io_request.device_id, file_id)?;
        let file = self.open_files.get(file_id).ok_or(NtStatus::INVALID_HANDLE)?;
        if !file.may_delete {
            return Err(NtStatus::ACCESS_DENIED);
        }
        match &request.set_buffer {
            FileInformationClass::Rename(information) if information.replace_if_exists == Boolean::False => {
                let name = flat_name(&information.file_name)?;
                handle
                    .rename(self.root.directory.as_raw(), &name, false)
                    .map_err(from_ntstatus)?;
                self.open_files
                    .get_mut(file_id)
                    .expect("published download is still open")
                    .published = true;
                Ok(())
            }
            _ => Err(NtStatus::ACCESS_DENIED),
        }
    }

    fn writable(&self, device_id: u32, file_id: u32) -> Result<&FileHandle, NtStatus> {
        let file = self.file(device_id, file_id)?;
        if file.published {
            return Err(NtStatus::ACCESS_DENIED);
        }
        let handle = file.handle.as_ref().expect("open downloads retain their handles");
        let information = handle.query_standard_information().map_err(from_ntstatus)?;
        // UNTESTED: Untrusted local processes cannot create hard links to in-progress files.
        // A link can be created after this check, so the approved directory and its files must exclude such access.
        // When changing the directory configuration, verify that this local trust boundary still holds.
        if information.NumberOfLinks != 1 || information.Directory || information.DeletePending {
            return Err(NtStatus::ACCESS_DENIED);
        }
        Ok(handle)
    }

    fn file(&self, device_id: u32, file_id: u32) -> Result<&DownloadFile, NtStatus> {
        if !self.active || device_id != DEVICE_ID {
            return Err(NtStatus::INVALID_HANDLE);
        }
        self.open_files.get(file_id).ok_or(NtStatus::INVALID_HANDLE)
    }
}

fn flat_name(path: &str) -> Result<String, NtStatus> {
    let path = RelativePath::parse(path).map_err(from_path_policy)?;
    let mut components = path.components();
    let name = components.next().ok_or(NtStatus::OBJECT_NAME_INVALID)?;
    if components.next().is_some() || name.contains(':') {
        return Err(NtStatus::OBJECT_NAME_INVALID);
    }
    Ok(name.to_owned())
}

impl_as_any!(WindowsDownloadSink);

impl RdpdrBackend for WindowsDownloadSink {
    fn max_filesystem_message_size(&self) -> Option<usize> {
        Some(MAX_DOWNLOAD_MESSAGE_SIZE)
    }

    fn reset(&mut self) -> PduResult<()> {
        self.active = false;
        self.open_files.clear();
        self.retry_cleanup()
    }

    fn restore_drive(&mut self, device_id: u32) -> PduResult<()> {
        if device_id != DEVICE_ID {
            return Err(pdu_other_err!("download device is not configured"));
        }
        self.retry_cleanup()?;
        self.active = true;
        Ok(())
    }

    fn add_drive(&mut self, device_id: u32) -> PduResult<()> {
        if self.active {
            return Err(pdu_other_err!("download device is already active"));
        }
        self.restore_drive(device_id)
    }

    fn remove_drive(&mut self, device_id: u32) -> PduResult<Vec<SvcMessage>> {
        if device_id != DEVICE_ID || !self.active {
            return Err(pdu_other_err!("download device is not active"));
        }
        self.open_files.clear();
        self.retry_cleanup()?;
        self.active = false;
        Ok(Vec::new())
    }

    fn handle_server_device_announce_response(&mut self, response: ServerDeviceAnnounceResponse) -> PduResult<()> {
        self.noop.handle_server_device_announce_response(response)
    }

    fn handle_scard_call(
        &mut self,
        request: DeviceControlRequest<ScardIoCtlCode>,
        call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        self.noop.handle_scard_call(request, call)
    }

    fn handle_drive_io_request(&mut self, request: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        let response = match request {
            ServerDriveIoRequest::ServerCreateDriveRequest(request) => return self.create(request),
            ServerDriveIoRequest::DeviceCloseRequest(request) => return self.close(request),
            ServerDriveIoRequest::DeviceWriteRequest(request) => return self.write(request),
            ServerDriveIoRequest::DeviceFlushBuffersRequest(request) => return self.flush(request),
            ServerDriveIoRequest::ServerDriveQueryInformationRequest(request) => {
                return self.query_information(request);
            }
            ServerDriveIoRequest::ServerDriveSetInformationRequest(request) => return self.set_information(request),
            ServerDriveIoRequest::DeviceReadRequest(request) => RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
                device_io_reply: DeviceIoResponse::new(request.device_io_request, NtStatus::ACCESS_DENIED),
                read_data: Vec::new(),
            }),
            ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(request) => {
                RdpdrPdu::ClientDriveQueryDirectoryResponse(ClientDriveQueryDirectoryResponse {
                    device_io_reply: DeviceIoResponse::new(request.device_io_request, NtStatus::ACCESS_DENIED),
                    buffer: None,
                })
            }
            ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(request) => {
                RdpdrPdu::ClientDriveNotifyChangeDirectoryResponse(ClientDriveNotifyChangeDirectoryResponse::new(
                    request.device_io_request,
                    NtStatus::ACCESS_DENIED,
                    Vec::new(),
                ))
            }
            ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(request) => {
                RdpdrPdu::ClientDriveQueryVolumeInformationResponse(ClientDriveQueryVolumeInformationResponse {
                    device_io_reply: DeviceIoResponse::new(request.device_io_request, NtStatus::ACCESS_DENIED),
                    buffer: None,
                })
            }
            ServerDriveIoRequest::DeviceControlRequest(request) => {
                RdpdrPdu::DeviceControlResponse(DeviceControlResponse {
                    device_io_reply: DeviceIoResponse::new(request.header, NtStatus::ACCESS_DENIED),
                    output_buffer: None,
                })
            }
            ServerDriveIoRequest::ServerDriveLockControlRequest(request) => {
                RdpdrPdu::ClientDriveLockControlResponse(ClientDriveLockControlResponse {
                    device_io_reply: DeviceIoResponse::new(request.device_io_request, NtStatus::ACCESS_DENIED),
                })
            }
            ServerDriveIoRequest::ServerDriveQuerySecurityRequest(request) => {
                RdpdrPdu::ClientDriveQuerySecurityResponse(ClientDriveQuerySecurityResponse {
                    device_io_response: DeviceIoResponse::new(request.device_io_request, NtStatus::ACCESS_DENIED),
                    security_descriptor: None,
                })
            }
            ServerDriveIoRequest::ServerDriveSetSecurityRequest(request) => RdpdrPdu::ClientDriveSetSecurityResponse(
                ClientDriveSetSecurityResponse::new(&request, NtStatus::ACCESS_DENIED)
                    .map_err(|error| encode_err!(error))?,
            ),
        };
        Ok(vec![SvcMessage::from(response)])
    }
}
