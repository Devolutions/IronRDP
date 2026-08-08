//! Narrow Windows filesystem backend for the portable RDPDR contract.
//!
//! The IRPs implemented here correspond to [MS-RDPEFS] 2.2.3.3.1 through
//! 2.2.3.3.4 and 2.2.3.3.8 through 2.2.3.3.9. Native information buffers use
//! [MS-FSCC] 2.4.7, 2.4.14, and 2.4.47.
//!
//! [MS-RDPEFS]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs
//! [MS-FSCC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc

use ironrdp_core::impl_as_any;
use ironrdp_pdu::{PduResult, encode_err, pdu_other_err};
use ironrdp_rdpdr::RdpdrBackend;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    AnyIoCtlCode, Boolean, ClientDriveLockControlResponse, ClientDriveNotifyChangeDirectoryResponse,
    ClientDriveQueryDirectoryResponse, ClientDriveQueryInformationResponse, ClientDriveQueryVolumeInformationResponse,
    ClientDriveSetInformationResponse, DecodedDeviceControlRequest, DeviceCloseRequest, DeviceCloseResponse,
    DeviceControlRequest, DeviceControlResponse, DeviceCreateRequest, DeviceCreateResponse, DeviceIoResponse,
    DeviceReadRequest, DeviceReadResponse, DeviceWriteRequest, DeviceWriteResponse, FileAttributeTagInformation,
    FileAttributes, FileBasicInformation, FileInformationClass, FileInformationClassLevel, FileStandardInformation,
    Information, NtStatus, ServerDeviceAnnounceResponse, ServerDriveIoRequest, ServerDriveQueryInformationRequest,
    ServerDriveSetInformationRequest,
};
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_svc::SvcMessage;
use windows::Wdk::Storage::FileSystem::FILE_BASIC_INFORMATION as NativeFileBasicInformation;

use super::factory::RedirectedDrive;
use super::file_table::FileTable;
use super::handles::{
    FILE_CREATED_INFORMATION, FILE_OPENED_INFORMATION, FILE_OVERWRITTEN_INFORMATION, FILE_SUPERSEDED_INFORMATION,
    FileHandle, FileOpenOptions, RootDirectory,
};
use super::path::RelativePath;
use super::status::{from_ntstatus, from_path_policy};

const DEFAULT_MAX_OPEN_FILES: usize = 1_024;
const MAX_STATIC_IO_SIZE: usize = 1_024 * 1_024;
const SYNCHRONIZE: u32 = 0x0010_0000;
const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
const FILE_WRITE_DATA: u32 = 0x0000_0002;
const FILE_APPEND_DATA: u32 = 0x0000_0004;
const FILE_WRITE_EA: u32 = 0x0000_0010;
const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
const DELETE: u32 = 0x0001_0000;
const GENERIC_ALL: u32 = 0x1000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
const FILE_WRITE_THROUGH: u32 = 0x0000_0002;
const FILE_SEQUENTIAL_ONLY: u32 = 0x0000_0004;
const FILE_RANDOM_ACCESS: u32 = 0x0000_0800;
const FILE_DELETE_ON_CLOSE: u32 = 0x0000_1000;
const FILE_OPEN_BY_FILE_ID: u32 = 0x0000_2000;
const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const ALLOWED_CREATE_OPTIONS: u32 =
    FILE_DIRECTORY_FILE | FILE_NON_DIRECTORY_FILE | FILE_WRITE_THROUGH | FILE_SEQUENTIAL_ONLY | FILE_RANDOM_ACCESS;

/// Windows backend that safely opens files below explicitly configured roots.
#[derive(Debug)]
pub struct WindowsRdpdrBackend {
    drive: RedirectedDrive,
    root: Option<RedirectedRoot>,
    open_files: FileTable<OpenFile>,
}

impl WindowsRdpdrBackend {
    pub(crate) fn from_drive(drive: RedirectedDrive) -> Self {
        Self {
            drive,
            root: None,
            open_files: FileTable::new(DEFAULT_MAX_OPEN_FILES),
        }
    }

    fn activate_drive(&mut self, device_id: u32) -> PduResult<()> {
        if device_id != self.drive.device_id() {
            return Err(pdu_other_err!("Windows RDPDR drive is not configured"));
        }
        if self.root.is_some() {
            return Err(pdu_other_err!("Windows RDPDR drive is already active"));
        }
        let root = RootDirectory::open(self.drive.root_path())
            .map_err(|_status| pdu_other_err!("open configured Windows RDPDR volume root"))?;
        self.root = Some(RedirectedRoot {
            root,
            read_only: self.drive.read_only(),
        });
        Ok(())
    }

    fn create(&mut self, req: DeviceCreateRequest) -> PduResult<Vec<SvcMessage>> {
        let response = match self.create_inner(&req) {
            Ok((file_id, information)) => DeviceCreateResponse {
                device_io_reply: DeviceIoResponse::new(req.device_io_request, NtStatus::SUCCESS),
                file_id,
                information,
            },
            Err(status) => DeviceCreateResponse {
                device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
                file_id: 0,
                information: Information::empty(),
            },
        };

        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(response))])
    }

    fn create_inner(&mut self, req: &DeviceCreateRequest) -> Result<(u32, Information), NtStatus> {
        let path = RelativePath::parse(&req.path).map_err(from_path_policy)?;
        let is_root = path.components().next().is_none();
        if req.device_io_request.device_id != self.drive.device_id() {
            return Err(NtStatus::INVALID_PARAMETER);
        }
        let root = self.root.as_ref().ok_or(NtStatus::INVALID_PARAMETER)?;
        validate_create_request(req, root.read_only, is_root)?;

        if is_root {
            let disposition = u32::from(req.create_disposition);
            if !matches!(
                disposition,
                x if x == u32::from(ironrdp_rdpdr::pdu::efs::CreateDisposition::FILE_OPEN)
                    || x == u32::from(ironrdp_rdpdr::pdu::efs::CreateDisposition::FILE_OPEN_IF)
            ) {
                return Err(NtStatus::OBJECT_NAME_COLLISION);
            }
            if req.create_options.bits() & FILE_NON_DIRECTORY_FILE != 0 {
                return Err(NtStatus::FILE_IS_A_DIRECTORY);
            }
            if req.allocation_size != 0 {
                return Err(NtStatus::INVALID_PARAMETER);
            }
        }

        let allocation_size = i64::try_from(req.allocation_size).map_err(|_| NtStatus::INVALID_PARAMETER)?;
        let options = FileOpenOptions {
            // Explorer often opens an object for enumeration and then asks for
            // metadata without separately requesting FILE_READ_ATTRIBUTES.
            desired_access: req.desired_access.bits() | SYNCHRONIZE | FILE_READ_ATTRIBUTES,
            allocation_size: (req.allocation_size != 0).then_some(allocation_size),
            file_attributes: req.file_attributes.bits(),
            share_access: req.shared_access.bits(),
            create_disposition: u32::from(req.create_disposition),
            create_options: req.create_options.bits(),
        };
        let reservation = self.open_files.reserve_file_id().map_err(|error| match error {
            super::file_table::FileTableError::CapacityExceeded
            | super::file_table::FileTableError::IdSpaceExhausted => NtStatus::from(0xC000_009A),
        })?;
        let file_id = reservation.file_id();
        let (handle, native_information) = match root.root.open_relative_file(&path, options) {
            Ok(result) => result,
            Err(status) => {
                self.open_files.release_file_id(reservation);
                return Err(from_ntstatus(status));
            }
        };
        let information = match create_information(native_information) {
            Ok(information) => information,
            Err(status) => {
                self.open_files.release_file_id(reservation);
                return Err(status);
            }
        };
        self.open_files.insert(
            reservation,
            OpenFile {
                read_only: root.read_only,
                handle,
            },
        );

        Ok((file_id, information))
    }

    fn close(&mut self, req: DeviceCloseRequest) -> PduResult<Vec<SvcMessage>> {
        let status = match self.open_files.get(req.device_io_request.file_id) {
            Some(_) if req.device_io_request.device_id == self.drive.device_id() => {
                let _ = self.open_files.remove(req.device_io_request.file_id);
                NtStatus::SUCCESS
            }
            _ => NtStatus::INVALID_HANDLE,
        };
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCloseResponse(
            DeviceCloseResponse {
                device_io_response: DeviceIoResponse::new(req.device_io_request, status),
            },
        ))])
    }

    fn read(&self, req: DeviceReadRequest) -> PduResult<Vec<SvcMessage>> {
        let (status, read_data) = self
            .read_inner(&req)
            .map_or_else(|status| (status, Vec::new()), |data| (NtStatus::SUCCESS, data));
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceReadResponse(
            DeviceReadResponse {
                device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
                read_data,
            },
        ))])
    }

    fn read_inner(&self, req: &DeviceReadRequest) -> Result<Vec<u8>, NtStatus> {
        let length = usize::try_from(req.length).map_err(|_| NtStatus::INVALID_PARAMETER)?;
        validate_io_length(length)?;
        let offset = i64::try_from(req.offset).map_err(|_| NtStatus::INVALID_PARAMETER)?;
        let file = self.file_for_request(req.device_io_request.device_id, req.device_io_request.file_id)?;
        let mut data = vec![0; length];
        let completion = file.handle.read_at(offset, &mut data).map_err(from_ntstatus)?;
        if completion.status.0 < 0 {
            return Err(from_ntstatus(completion.status));
        }
        data.truncate(completion.transferred);
        Ok(data)
    }

    fn write(&self, req: DeviceWriteRequest) -> PduResult<Vec<SvcMessage>> {
        let (status, length) = self
            .write_inner(&req)
            .map_or_else(|status| (status, 0), |length| (NtStatus::SUCCESS, length));
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceWriteResponse(
            DeviceWriteResponse {
                device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
                length,
            },
        ))])
    }

    fn write_inner(&self, req: &DeviceWriteRequest) -> Result<u32, NtStatus> {
        validate_io_length(req.write_data.len())?;
        let offset = if req.offset == u64::MAX {
            -1 // FILE_WRITE_TO_END_OF_FILE
        } else {
            i64::try_from(req.offset).map_err(|_| NtStatus::INVALID_PARAMETER)?
        };
        let file = self.file_for_request(req.device_io_request.device_id, req.device_io_request.file_id)?;
        if file.read_only {
            return Err(NtStatus::MEDIA_WRITE_PROTECTED);
        }

        let completion = file.handle.write_at(offset, &req.write_data).map_err(from_ntstatus)?;
        if completion.status.0 < 0 {
            return Err(from_ntstatus(completion.status));
        }
        if completion.transferred != req.write_data.len() {
            return Err(NtStatus::UNSUCCESSFUL);
        }
        file.handle.flush().map_err(from_ntstatus)?;
        u32::try_from(completion.transferred).map_err(|_| NtStatus::UNSUCCESSFUL)
    }

    fn query_information(&self, req: ServerDriveQueryInformationRequest) -> PduResult<Vec<SvcMessage>> {
        let (status, buffer) = self
            .query_information_inner(&req)
            .map_or_else(|status| (status, None), |buffer| (NtStatus::SUCCESS, Some(buffer)));
        Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryInformationResponse(
            ClientDriveQueryInformationResponse {
                device_io_response: DeviceIoResponse::new(req.device_io_request, status),
                buffer,
            },
        ))])
    }

    fn query_information_inner(
        &self,
        req: &ServerDriveQueryInformationRequest,
    ) -> Result<FileInformationClass, NtStatus> {
        let file = self.file_for_request(req.device_io_request.device_id, req.device_io_request.file_id)?;

        match req.file_info_class_lvl {
            FileInformationClassLevel::FILE_BASIC_INFORMATION => {
                let information = file.handle.query_basic_information().map_err(from_ntstatus)?;
                Ok(FileBasicInformation {
                    creation_time: information.CreationTime,
                    last_access_time: information.LastAccessTime,
                    last_write_time: information.LastWriteTime,
                    change_time: information.ChangeTime,
                    file_attributes: FileAttributes::from_bits_retain(information.FileAttributes),
                }
                .into())
            }
            FileInformationClassLevel::FILE_STANDARD_INFORMATION => {
                let information = file.handle.query_standard_information().map_err(from_ntstatus)?;
                Ok(FileStandardInformation {
                    allocation_size: information.AllocationSize,
                    end_of_file: information.EndOfFile,
                    number_of_links: information.NumberOfLinks,
                    delete_pending: Boolean::from(u8::from(information.DeletePending)),
                    directory: Boolean::from(u8::from(information.Directory)),
                }
                .into())
            }
            FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION => {
                let information = file.handle.query_attribute_tag_information().map_err(from_ntstatus)?;
                Ok(FileAttributeTagInformation {
                    file_attributes: FileAttributes::from_bits_retain(information.FileAttributes),
                    reparse_tag: information.ReparseTag,
                }
                .into())
            }
            _ => Err(NtStatus::NOT_SUPPORTED),
        }
    }

    fn set_information(&self, req: ServerDriveSetInformationRequest) -> PduResult<Vec<SvcMessage>> {
        let status = self
            .set_information_inner(&req)
            .map_or_else(|status| status, |_| NtStatus::SUCCESS);
        let response = ClientDriveSetInformationResponse::new(&req, status).map_err(|error| encode_err!(error))?;
        Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveSetInformationResponse(
            response,
        ))])
    }

    fn set_information_inner(&self, req: &ServerDriveSetInformationRequest) -> Result<(), NtStatus> {
        let file = self.file_for_request(req.device_io_request.device_id, req.device_io_request.file_id)?;
        if file.read_only {
            return Err(NtStatus::MEDIA_WRITE_PROTECTED);
        }

        match &req.set_buffer {
            FileInformationClass::EndOfFile(information) => {
                if information.end_of_file < 0 {
                    return Err(NtStatus::INVALID_PARAMETER);
                }
                file.handle
                    .set_end_of_file(information.end_of_file)
                    .map_err(from_ntstatus)
            }
            FileInformationClass::Basic(information) => {
                if [
                    information.creation_time,
                    information.last_access_time,
                    information.last_write_time,
                    information.change_time,
                ]
                .into_iter()
                .any(|time| time < -2)
                {
                    return Err(NtStatus::INVALID_PARAMETER);
                }
                file.handle
                    .set_basic_information(NativeFileBasicInformation {
                        CreationTime: information.creation_time,
                        LastAccessTime: information.last_access_time,
                        LastWriteTime: information.last_write_time,
                        ChangeTime: information.change_time,
                        FileAttributes: information.file_attributes.bits(),
                    })
                    .map_err(from_ntstatus)
            }
            _ => Err(NtStatus::NOT_SUPPORTED),
        }
    }

    fn file_for_request(&self, device_id: u32, file_id: u32) -> Result<&OpenFile, NtStatus> {
        if device_id != self.drive.device_id() {
            return Err(NtStatus::INVALID_HANDLE);
        }

        self.open_files.get(file_id).ok_or(NtStatus::INVALID_HANDLE)
    }

    fn unsupported_control(req: DeviceControlRequest<AnyIoCtlCode>) -> PduResult<Vec<SvcMessage>> {
        Ok(vec![SvcMessage::from(RdpdrPdu::DeviceControlResponse(
            DeviceControlResponse::new(req, NtStatus::NOT_SUPPORTED, None),
        ))])
    }
}

impl_as_any!(WindowsRdpdrBackend);

impl RdpdrBackend for WindowsRdpdrBackend {
    fn reset(&mut self) -> PduResult<()> {
        self.open_files.clear();
        self.root = None;
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

    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        match req {
            ServerDriveIoRequest::ServerCreateDriveRequest(req) => self.create(req),
            ServerDriveIoRequest::DeviceCloseRequest(req) => self.close(req),
            ServerDriveIoRequest::DeviceReadRequest(req) => self.read(req),
            ServerDriveIoRequest::DeviceWriteRequest(req) => self.write(req),
            ServerDriveIoRequest::ServerDriveQueryInformationRequest(req) => self.query_information(req),
            ServerDriveIoRequest::ServerDriveSetInformationRequest(req) => self.set_information(req),
            ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(req) => Ok(vec![SvcMessage::from(
                RdpdrPdu::ClientDriveQueryDirectoryResponse(ClientDriveQueryDirectoryResponse {
                    device_io_reply: DeviceIoResponse::new(req.device_io_request, NtStatus::NOT_SUPPORTED),
                    buffer: None,
                }),
            )]),
            ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(req) => Ok(vec![SvcMessage::from(
                RdpdrPdu::ClientDriveNotifyChangeDirectoryResponse(ClientDriveNotifyChangeDirectoryResponse::new(
                    req.device_io_request,
                    NtStatus::NOT_SUPPORTED,
                    Vec::new(),
                )),
            )]),
            ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(req) => Ok(vec![SvcMessage::from(
                RdpdrPdu::ClientDriveQueryVolumeInformationResponse(ClientDriveQueryVolumeInformationResponse::new(
                    req.device_io_request,
                    NtStatus::NOT_SUPPORTED,
                    None,
                )),
            )]),
            ServerDriveIoRequest::ServerDriveLockControlRequest(req) => {
                Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveLockControlResponse(
                    ClientDriveLockControlResponse::new(req.device_io_request, NtStatus::NOT_SUPPORTED),
                ))])
            }
            ServerDriveIoRequest::DeviceControlRequest(req) => Self::unsupported_control(req),
        }
    }

    fn handle_drive_device_control(
        &mut self,
        req: DecodedDeviceControlRequest<AnyIoCtlCode>,
    ) -> PduResult<Vec<SvcMessage>> {
        Self::unsupported_control(req.request)
    }
}

#[derive(Debug)]
struct RedirectedRoot {
    root: RootDirectory,
    read_only: bool,
}

#[derive(Debug)]
struct OpenFile {
    read_only: bool,
    handle: FileHandle,
}

fn validate_create_request(req: &DeviceCreateRequest, read_only: bool, is_root: bool) -> Result<(), NtStatus> {
    let desired_access = req.desired_access.bits();
    let options = req.create_options.bits();
    let disposition = u32::from(req.create_disposition);

    if i64::try_from(req.allocation_size).is_err() {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    if options & (FILE_OPEN_BY_FILE_ID | FILE_OPEN_REPARSE_POINT | FILE_DELETE_ON_CLOSE) != 0
        || options & !ALLOWED_CREATE_OPTIONS != 0
    {
        return Err(NtStatus::NOT_SUPPORTED);
    }
    if options & FILE_DIRECTORY_FILE != 0 && options & FILE_NON_DIRECTORY_FILE != 0 {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    if !matches!(
        disposition,
        x if x == u32::from(ironrdp_rdpdr::pdu::efs::CreateDisposition::FILE_SUPERSEDE)
            || x == u32::from(ironrdp_rdpdr::pdu::efs::CreateDisposition::FILE_OPEN)
            || x == u32::from(ironrdp_rdpdr::pdu::efs::CreateDisposition::FILE_CREATE)
            || x == u32::from(ironrdp_rdpdr::pdu::efs::CreateDisposition::FILE_OPEN_IF)
            || x == u32::from(ironrdp_rdpdr::pdu::efs::CreateDisposition::FILE_OVERWRITE)
            || x == u32::from(ironrdp_rdpdr::pdu::efs::CreateDisposition::FILE_OVERWRITE_IF)
    ) {
        return Err(NtStatus::INVALID_PARAMETER);
    }

    let mutating_disposition = matches!(
        disposition,
        x if x == u32::from(ironrdp_rdpdr::pdu::efs::CreateDisposition::FILE_SUPERSEDE)
            || x == u32::from(ironrdp_rdpdr::pdu::efs::CreateDisposition::FILE_CREATE)
            || (x == u32::from(ironrdp_rdpdr::pdu::efs::CreateDisposition::FILE_OPEN_IF) && !is_root)
            || x == u32::from(ironrdp_rdpdr::pdu::efs::CreateDisposition::FILE_OVERWRITE)
            || x == u32::from(ironrdp_rdpdr::pdu::efs::CreateDisposition::FILE_OVERWRITE_IF)
    );
    let mutating_access = FILE_WRITE_DATA
        | FILE_APPEND_DATA
        | FILE_WRITE_EA
        | FILE_WRITE_ATTRIBUTES
        | DELETE
        | GENERIC_ALL
        | GENERIC_WRITE;
    if read_only && (desired_access & mutating_access != 0 || mutating_disposition) {
        return Err(NtStatus::MEDIA_WRITE_PROTECTED);
    }

    Ok(())
}

fn validate_io_length(length: usize) -> Result<(), NtStatus> {
    if length > MAX_STATIC_IO_SIZE {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    Ok(())
}

fn create_information(information: usize) -> Result<Information, NtStatus> {
    match information {
        FILE_SUPERSEDED_INFORMATION => Ok(Information::file_superseded()),
        FILE_OPENED_INFORMATION => Ok(Information::file_opened()),
        FILE_CREATED_INFORMATION => Ok(Information::file_created()),
        FILE_OVERWRITTEN_INFORMATION => Ok(Information::file_overwritten()),
        _ => Err(NtStatus::UNSUCCESSFUL),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use ironrdp_rdpdr::pdu::efs::{
        CreateDisposition, CreateOptions, DesiredAccess, FileEndOfFileInformation, LockOperation, MajorFunction,
        MinorFunction, ServerDriveLockControlRequest, ServerDriveNotifyChangeDirectoryRequest, SharedAccess,
    };

    use crate::windows::factory::WindowsRdpdrBackendFactory;

    #[test]
    fn regular_file_lifecycle_is_handle_relative_and_bounded() {
        let fixture = Fixture::new();
        let mut backend = fixture.backend();
        activate(&mut backend);

        let create = create_request(&fixture.relative(r"root\report.txt"), CreateDisposition::FILE_OPEN_IF);
        let create_response = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create))
            .expect("complete create");
        assert_eq!(response_status(&create_response), NtStatus::SUCCESS);
        let file_id = response_file_id(&create_response);

        let write = DeviceWriteRequest {
            device_io_request: request_header(file_id, MajorFunction::Write),
            offset: 0,
            write_data: b"hello".to_vec(),
        };
        let write_response = backend
            .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(write))
            .expect("complete write");
        assert_eq!(response_status(&write_response), NtStatus::SUCCESS);

        let append_response = backend
            .handle_drive_io_request(ServerDriveIoRequest::DeviceWriteRequest(DeviceWriteRequest {
                device_io_request: request_header(file_id, MajorFunction::Write),
                offset: u64::MAX,
                write_data: b"!".to_vec(),
            }))
            .expect("complete append");
        assert_eq!(response_status(&append_response), NtStatus::SUCCESS);

        let read = DeviceReadRequest {
            device_io_request: request_header(file_id, MajorFunction::Read),
            length: 6,
            offset: 0,
        };
        let read_response = backend
            .handle_drive_io_request(ServerDriveIoRequest::DeviceReadRequest(read))
            .expect("complete read");
        assert_eq!(response_status(&read_response), NtStatus::SUCCESS);
        let bytes = read_response[0].encode_unframed_pdu().expect("encode read response");
        assert_eq!(&bytes[20..], b"hello!");

        for file_info_class_lvl in [
            FileInformationClassLevel::FILE_BASIC_INFORMATION,
            FileInformationClassLevel::FILE_STANDARD_INFORMATION,
            FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION,
        ] {
            let query_response = backend
                .handle_drive_io_request(ServerDriveIoRequest::ServerDriveQueryInformationRequest(
                    ServerDriveQueryInformationRequest {
                        device_io_request: request_header(file_id, MajorFunction::QueryInformation),
                        file_info_class_lvl,
                    },
                ))
                .expect("complete metadata query");
            assert_eq!(response_status(&query_response), NtStatus::SUCCESS);
        }

        let set = ServerDriveSetInformationRequest {
            device_io_request: request_header(file_id, MajorFunction::SetInformation),
            set_buffer: FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 5 }),
        };
        let set_response = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerDriveSetInformationRequest(set))
            .expect("complete set information");
        assert_eq!(response_status(&set_response), NtStatus::SUCCESS);
        assert_eq!(
            std::fs::read(fixture.root.join("root").join("report.txt")).expect("read test file"),
            b"hello"
        );

        let close_response = backend
            .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
                device_io_request: request_header(file_id, MajorFunction::Close),
            }))
            .expect("complete close");
        assert_eq!(response_status(&close_response), NtStatus::SUCCESS);
    }

    #[test]
    fn volume_root_requests_use_the_trusted_root_handle() {
        let fixture = Fixture::new();
        let mut backend = fixture.backend();
        activate(&mut backend);

        let mut create = create_request(r"\", CreateDisposition::FILE_OPEN);
        create.desired_access = DesiredAccess::from_bits_retain(FILE_READ_ATTRIBUTES | SYNCHRONIZE);
        let response = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create))
            .expect("complete root create");
        assert_eq!(response_status(&response), NtStatus::SUCCESS);

        let close = backend
            .handle_drive_io_request(ServerDriveIoRequest::DeviceCloseRequest(DeviceCloseRequest {
                device_io_request: request_header(response_file_id(&response), MajorFunction::Close),
            }))
            .expect("complete root close");
        assert_eq!(response_status(&close), NtStatus::SUCCESS);
    }

    #[test]
    fn hostile_paths_never_open_outside_the_selected_volume() {
        let fixture = Fixture::new();
        let mut backend = fixture.backend();
        activate(&mut backend);

        for path in [
            r"\..\outside.txt",
            r"\??\C:\outside.txt",
            r"\root\CON",
            r"\root\file:stream",
        ] {
            let response = backend
                .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create_request(
                    path,
                    CreateDisposition::FILE_OPEN_IF,
                )))
                .expect("complete hostile create");
            assert_ne!(response_status(&response), NtStatus::SUCCESS);
        }
    }

    #[test]
    fn backup_intent_is_not_forwarded_to_windows() {
        let fixture = Fixture::new();
        let mut backend = fixture.backend();
        activate(&mut backend);

        let mut create = create_request(&fixture.relative(r"root\backup.txt"), CreateDisposition::FILE_OPEN_IF);
        create.create_options = CreateOptions::from_bits_retain(0x0000_4000);
        let response = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create))
            .expect("complete backup intent create");

        assert_eq!(response_status(&response), NtStatus::NOT_SUPPORTED);
        assert!(!fixture.root.join("root").join("backup.txt").exists());
    }

    #[test]
    fn reparse_points_cannot_escape_the_trusted_volume_root() {
        let fixture = Fixture::new();
        let outside = fixture.sandbox.join("outside");
        std::fs::create_dir_all(&outside).expect("create outside directory");
        std::fs::write(outside.join("sentinel.txt"), b"outside").expect("write sentinel");

        let link = fixture.root.join("root").join("escape");
        if let Err(error) = std::os::windows::fs::symlink_dir(&outside, &link) {
            if error.raw_os_error() == Some(1314) {
                return;
            }
            panic!("create test reparse point: {error}");
        }

        let mut backend = fixture.backend();
        activate(&mut backend);
        let response = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerCreateDriveRequest(create_request(
                &fixture.relative(r"root\escape\sentinel.txt"),
                CreateDisposition::FILE_OPEN,
            )))
            .expect("complete reparse create");

        assert_ne!(response_status(&response), NtStatus::SUCCESS);
        assert_eq!(
            std::fs::read(outside.join("sentinel.txt")).expect("read sentinel"),
            b"outside"
        );
    }

    #[test]
    fn unsupported_device_control_receives_a_completion() {
        let fixture = Fixture::new();
        let mut backend = fixture.backend();
        activate(&mut backend);
        let request = DeviceControlRequest {
            header: request_header(0, MajorFunction::DeviceControl),
            output_buffer_length: 0,
            input_buffer_length: 0,
            io_control_code: AnyIoCtlCode(0),
        };

        let response = backend
            .handle_drive_io_request(ServerDriveIoRequest::DeviceControlRequest(request))
            .expect("complete unsupported control");
        assert_eq!(response_status(&response), NtStatus::NOT_SUPPORTED);
    }

    #[test]
    fn unsupported_directory_notifications_and_locks_receive_completions() {
        let fixture = Fixture::new();
        let mut backend = fixture.backend();
        activate(&mut backend);

        let notification = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(
                ServerDriveNotifyChangeDirectoryRequest {
                    device_io_request: request_header(0, MajorFunction::DirectoryControl),
                    watch_tree: 0,
                    completion_filter: 0,
                },
            ))
            .expect("complete unsupported notification");
        assert_eq!(response_status(&notification), NtStatus::NOT_SUPPORTED);

        let lock = backend
            .handle_drive_io_request(ServerDriveIoRequest::ServerDriveLockControlRequest(
                ServerDriveLockControlRequest {
                    device_io_request: request_header(0, MajorFunction::LockControl),
                    operation: LockOperation::Exclusive,
                    wait: false,
                    locks: Vec::new(),
                },
            ))
            .expect("complete unsupported lock");
        assert_eq!(response_status(&lock), NtStatus::NOT_SUPPORTED);
    }

    fn activate(backend: &mut WindowsRdpdrBackend) {
        RdpdrBackend::restore_drive(backend, 1).expect("activate test drive");
    }

    fn create_request(path: &str, disposition: CreateDisposition) -> DeviceCreateRequest {
        DeviceCreateRequest {
            device_io_request: request_header(0, MajorFunction::Create),
            desired_access: DesiredAccess::from_bits_retain(0x0010_0083),
            allocation_size: 0,
            file_attributes: FileAttributes::FILE_ATTRIBUTE_NORMAL,
            shared_access: SharedAccess::from_bits_retain(0x0000_0007),
            create_disposition: disposition,
            create_options: CreateOptions::empty(),
            path: path.to_owned(),
        }
    }

    fn request_header(file_id: u32, major_function: MajorFunction) -> ironrdp_rdpdr::pdu::efs::DeviceIoRequest {
        ironrdp_rdpdr::pdu::efs::DeviceIoRequest {
            device_id: 1,
            file_id,
            completion_id: 7,
            major_function,
            minor_function: MinorFunction::from(0),
        }
    }

    fn response_status(messages: &[SvcMessage]) -> NtStatus {
        let response = messages[0].encode_unframed_pdu().expect("encode RDPDR response");
        NtStatus::from(u32::from_le_bytes(
            response[12..16].try_into().expect("response status"),
        ))
    }

    fn response_file_id(messages: &[SvcMessage]) -> u32 {
        let response = messages[0].encode_unframed_pdu().expect("encode create response");
        u32::from_le_bytes(response[16..20].try_into().expect("create file ID"))
    }

    struct Fixture {
        sandbox: PathBuf,
        root: PathBuf,
        volume_root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let workspace = std::env::current_dir().expect("current workspace directory");
            let volume_root = workspace
                .ancestors()
                .last()
                .expect("workspace has a volume root")
                .to_owned();
            let unique = format!(
                "rdpdr-native-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock is after Unix epoch")
                    .as_nanos()
            );
            let sandbox = workspace.join("target").join(unique);
            let root = sandbox.join("root");
            std::fs::create_dir_all(root.join("root")).expect("create fixture root");
            Self {
                sandbox,
                root,
                volume_root,
            }
        }

        fn backend(&self) -> WindowsRdpdrBackend {
            WindowsRdpdrBackendFactory::new(
                RedirectedDrive::new(1, "Test", &self.volume_root, false).expect("valid test drive"),
            )
            .build()
        }

        fn relative(&self, suffix: &str) -> String {
            let relative = self
                .root
                .strip_prefix(&self.volume_root)
                .expect("fixture is beneath its volume root");
            format!(r"\{}\{}", relative.display(), suffix).replace('/', r"\")
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.sandbox);
        }
    }
}
