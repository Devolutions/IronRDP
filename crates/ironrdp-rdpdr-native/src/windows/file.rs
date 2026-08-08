use ironrdp_pdu::{PduResult, encode_err};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    Boolean, ClientDriveQueryInformationResponse, ClientDriveSetInformationResponse, CreateDisposition,
    DeviceCloseRequest, DeviceCloseResponse, DeviceCreateRequest, DeviceCreateResponse, DeviceFlushBuffersRequest,
    DeviceFlushBuffersResponse, DeviceIoResponse, DeviceReadRequest, DeviceReadResponse, DeviceWriteRequest,
    DeviceWriteResponse, FileAttributeTagInformation, FileAttributes, FileBasicInformation, FileInformationClass,
    FileInformationClassLevel, FileStandardInformation, FileStreamInformation, Information, NtStatus,
    ServerDriveQueryInformationRequest, ServerDriveSetInformationRequest,
};
use ironrdp_svc::SvcMessage;
use tracing::debug;
use windows::Wdk::Storage::FileSystem::FILE_BASIC_INFORMATION as NativeFileBasicInformation;

use super::backend::{OpenFile, WindowsRdpdrBackend};
use super::handles::{
    FILE_CREATED_INFORMATION, FILE_OPEN_REPARSE_POINT, FILE_OPENED_INFORMATION, FILE_OVERWRITTEN_INFORMATION,
    FILE_SUPERSEDED_INFORMATION, FileOpenOptions, RENAME_DESTINATION_DIRECTORY_ACCESS,
};
use super::path::RelativePath;
use super::security;
use super::status::{from_ntstatus, from_open_directory, from_open_file, from_path_policy};

const SYNCHRONIZE: u32 = 0x0010_0000;
const FILE_DIRECTORY_FILE: u32 = 0x0000_0001;
const FILE_WRITE_THROUGH: u32 = 0x0000_0002;
const FILE_SEQUENTIAL_ONLY: u32 = 0x0000_0004;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x0000_0020;
const FILE_NON_DIRECTORY_FILE: u32 = 0x0000_0040;
const FILE_RANDOM_ACCESS: u32 = 0x0000_0800;
const FILE_DELETE_ON_CLOSE: u32 = 0x0000_1000;
const FILE_OPEN_BY_FILE_ID: u32 = 0x0000_2000;
const FILE_OPEN_FOR_BACKUP_INTENT: u32 = 0x0000_4000;
const FILE_OPEN_NO_RECALL: u32 = 0x0040_0000;
const FILE_OPEN_FOR_FREE_SPACE_QUERY: u32 = 0x0080_0000;
const ALLOWED_CREATE_OPTIONS: u32 = FILE_DIRECTORY_FILE
    | FILE_WRITE_THROUGH
    | FILE_SEQUENTIAL_ONLY
    | FILE_SYNCHRONOUS_IO_NONALERT
    | FILE_NON_DIRECTORY_FILE
    | FILE_RANDOM_ACCESS
    | FILE_DELETE_ON_CLOSE
    | FILE_OPEN_FOR_BACKUP_INTENT
    | FILE_OPEN_REPARSE_POINT
    // These hints affect local caching and capacity-query behavior only; they
    // neither widen the requested access nor change path resolution.
    | FILE_OPEN_NO_RECALL
    | FILE_OPEN_FOR_FREE_SPACE_QUERY;

const FILE_WRITE_DATA: u32 = 0x0000_0002;
const FILE_APPEND_DATA: u32 = 0x0000_0004;
const FILE_WRITE_EA: u32 = 0x0000_0010;
const FILE_READ_ATTRIBUTES: u32 = 0x0000_0080;
const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
const DELETE: u32 = 0x0001_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const GENERIC_ALL: u32 = 0x1000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const SECURITY_MUTATION_ACCESS: u32 = WRITE_DAC | WRITE_OWNER | security::ACCESS_SYSTEM_SECURITY;
const READ_ONLY_MUTATION_ACCESS: u32 = FILE_WRITE_DATA
    | FILE_APPEND_DATA
    | FILE_WRITE_EA
    | FILE_WRITE_ATTRIBUTES
    | DELETE
    | SECURITY_MUTATION_ACCESS
    | MAXIMUM_ALLOWED
    | GENERIC_ALL
    | GENERIC_WRITE;
const MAX_STATIC_IO_SIZE: usize = 1024 * 1024;

pub(crate) fn create(backend: &mut WindowsRdpdrBackend, req: DeviceCreateRequest) -> PduResult<Vec<SvcMessage>> {
    let response = match create_inner(backend, &req) {
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

pub(crate) fn close(backend: &mut WindowsRdpdrBackend, req: DeviceCloseRequest) -> PduResult<Vec<SvcMessage>> {
    let mut messages = Vec::new();
    let status = if matches!(
        backend.open_files.get(req.device_io_request.file_id),
        Some(file) if file.device_id == req.device_io_request.device_id
    ) {
        backend
            .open_files
            .remove(req.device_io_request.file_id)
            .expect("a file ID checked in the file table is still present");
        messages.extend(
            backend.cancel_deferred_file_operations(req.device_io_request.device_id, req.device_io_request.file_id),
        );
        NtStatus::SUCCESS
    } else {
        NtStatus::INVALID_HANDLE
    };

    messages.push(SvcMessage::from(RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
        device_io_response: DeviceIoResponse::new(req.device_io_request, status),
    })));
    Ok(messages)
}

pub(crate) fn flush(backend: &WindowsRdpdrBackend, req: DeviceFlushBuffersRequest) -> PduResult<Vec<SvcMessage>> {
    let status = flush_inner(backend, &req).map_or_else(|status| status, |_| NtStatus::SUCCESS);
    Ok(vec![SvcMessage::from(RdpdrPdu::DeviceFlushBuffersResponse(
        DeviceFlushBuffersResponse {
            device_io_response: DeviceIoResponse::new(req.device_io_request, status),
        },
    ))])
}

pub(crate) fn read(backend: &mut WindowsRdpdrBackend, req: DeviceReadRequest) -> PduResult<Vec<SvcMessage>> {
    let (status, read_data) = match read_inner(backend, &req) {
        Ok(read_data) => (NtStatus::SUCCESS, read_data),
        Err(status) => (status, Vec::new()),
    };

    Ok(vec![SvcMessage::from(RdpdrPdu::DeviceReadResponse(
        DeviceReadResponse {
            device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
            read_data,
        },
    ))])
}

pub(crate) fn write(backend: &mut WindowsRdpdrBackend, req: DeviceWriteRequest) -> PduResult<Vec<SvcMessage>> {
    let (status, length) = match write_inner(backend, &req) {
        Ok(completion) => match u32::try_from(completion.transferred) {
            Ok(length) => (completion_status(completion.status), length),
            Err(_) => (NtStatus::UNSUCCESSFUL, 0),
        },
        Err(status) => (status, 0),
    };

    Ok(vec![SvcMessage::from(RdpdrPdu::DeviceWriteResponse(
        DeviceWriteResponse {
            device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
            length,
        },
    ))])
}

pub(crate) fn query_information(
    backend: &mut WindowsRdpdrBackend,
    req: ServerDriveQueryInformationRequest,
) -> PduResult<Vec<SvcMessage>> {
    let information_class = u32::from(req.file_info_class_lvl.clone());
    let (status, buffer) = match query_information_inner_with_status(backend, &req) {
        Ok((status, buffer)) => (status, Some(buffer)),
        Err(status) => (status, None),
    };
    let response_length = buffer.as_ref().map_or(0, FileInformationClass::size);
    debug!(
        information_class,
        ?status,
        response_length,
        "Completed filesystem query-information IRP"
    );

    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryInformationResponse(
        ClientDriveQueryInformationResponse {
            device_io_response: DeviceIoResponse::new(req.device_io_request, status),
            buffer,
        },
    ))])
}

pub(crate) fn set_information(
    backend: &mut WindowsRdpdrBackend,
    req: ServerDriveSetInformationRequest,
) -> PduResult<Vec<SvcMessage>> {
    let status = set_information_inner(backend, &req).map_or_else(|status| status, |_| NtStatus::SUCCESS);
    let response = ClientDriveSetInformationResponse::new(&req, status).map_err(|error| encode_err!(error))?;

    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveSetInformationResponse(
        response,
    ))])
}

pub(super) fn create_inner(
    backend: &mut WindowsRdpdrBackend,
    req: &DeviceCreateRequest,
) -> Result<(u32, Information), NtStatus> {
    let (read_only, root) = backend
        .roots
        .get(&req.device_io_request.device_id)
        .map(|root| (root.read_only, &root.root))
        .ok_or(NtStatus::INVALID_PARAMETER)?;
    let path = RelativePath::parse(&req.path).map_err(from_path_policy)?;
    let is_root = path.components().len() == 0;
    validate_create_request(req, read_only, is_root)?;
    let allocation_size = i64::try_from(req.allocation_size).map_err(|_| NtStatus::INVALID_PARAMETER)?;

    if is_root {
        if req.create_options.bits() & FILE_NON_DIRECTORY_FILE != 0 {
            return Err(NtStatus::FILE_IS_A_DIRECTORY);
        }
        if !matches!(
            u32::from(req.create_disposition),
            x if x == u32::from(CreateDisposition::FILE_OPEN)
                || x == u32::from(CreateDisposition::FILE_OPEN_IF)
        ) {
            return Err(NtStatus::OBJECT_NAME_COLLISION);
        }
        if req.create_options.bits() & FILE_DELETE_ON_CLOSE != 0 {
            return Err(NtStatus::ACCESS_DENIED);
        }
        if req.allocation_size != 0 {
            return Err(NtStatus::INVALID_PARAMETER);
        }
    }

    // Explorer queries metadata immediately after directory listings and file
    // opens, even when its CREATE request omitted FILE_READ_ATTRIBUTES. Match
    // MSTSC's compatibility access without granting data, write, or security
    // permissions the server did not request.
    let desired_access = req.desired_access.bits() | SYNCHRONIZE | FILE_READ_ATTRIBUTES;
    let options = FileOpenOptions {
        desired_access,
        allocation_size: (req.allocation_size != 0).then_some(allocation_size),
        file_attributes: req.file_attributes.bits(),
        share_access: req.shared_access.bits(),
        create_disposition: u32::from(req.create_disposition),
        // A remote server must not be able to use this hint to exercise the
        // local process's backup or restore privileges during ACL evaluation.
        create_options: (req.create_options.bits() & !FILE_OPEN_FOR_BACKUP_INTENT) | FILE_SYNCHRONOUS_IO_NONALERT,
    };
    let _security_privilege = security::enable_for_access_system_security(req.desired_access.bits())?;
    let (handle, information) = root.open_relative_file(&path, options).map_err(|error| {
        let status = from_open_file(error);
        debug!(
            device_id = req.device_io_request.device_id,
            completion_id = req.device_io_request.completion_id,
            ?status,
            "Native filesystem create failed"
        );
        status
    })?;
    let information = create_information(information)?;
    let file_id = backend
        .open_files
        .insert(OpenFile {
            device_id: req.device_io_request.device_id,
            read_only,
            path,
            handle,
            directory_query_handle: None,
        })
        .map_err(|_| NtStatus::UNSUCCESSFUL)?;

    Ok((file_id, information))
}

fn read_inner(backend: &WindowsRdpdrBackend, req: &DeviceReadRequest) -> Result<Vec<u8>, NtStatus> {
    let length = usize::try_from(req.length).map_err(|_| NtStatus::INVALID_PARAMETER)?;
    validate_io_length(length)?;
    let offset = read_offset(req.offset)?;
    let file = file_for_request(backend, req.device_io_request.device_id, req.device_io_request.file_id)?;

    if length == 0 {
        return Ok(Vec::new());
    }

    let mut read_data = vec![0; length];
    let completion = file
        .handle
        .read_at(offset, &mut read_data)
        .map_err(|_| NtStatus::INVALID_PARAMETER)?;
    if completion.status.0 < 0 {
        return Err(from_ntstatus(completion.status));
    }

    read_data.truncate(completion.transferred);
    Ok(read_data)
}

fn write_inner(
    backend: &WindowsRdpdrBackend,
    req: &DeviceWriteRequest,
) -> Result<super::handles::NativeIoCompletion, NtStatus> {
    validate_io_length(req.write_data.len())?;
    let offset = write_offset(req.offset)?;
    let file = file_for_request(backend, req.device_io_request.device_id, req.device_io_request.file_id)?;
    if file.read_only {
        return Err(NtStatus::MEDIA_WRITE_PROTECTED);
    }

    if req.write_data.is_empty() {
        return Ok(super::handles::NativeIoCompletion {
            status: windows::Win32::Foundation::NTSTATUS(0),
            transferred: 0,
        });
    }

    match offset {
        WriteOffset::Explicit(offset) => file.handle.write_at(offset, &req.write_data),
        WriteOffset::Append => file.handle.write_to_end(&req.write_data),
    }
    .map_err(|_| NtStatus::INVALID_PARAMETER)
}

fn flush_inner(backend: &WindowsRdpdrBackend, req: &DeviceFlushBuffersRequest) -> Result<(), NtStatus> {
    file_for_request(backend, req.device_io_request.device_id, req.device_io_request.file_id)?
        .handle
        .flush()
        .map_err(from_ntstatus)
}

#[cfg(test)]
fn query_information_inner(
    backend: &WindowsRdpdrBackend,
    req: &ServerDriveQueryInformationRequest,
) -> Result<FileInformationClass, NtStatus> {
    query_information_inner_with_status(backend, req).map(|(_, information)| information)
}

fn query_information_inner_with_status(
    backend: &WindowsRdpdrBackend,
    req: &ServerDriveQueryInformationRequest,
) -> Result<(NtStatus, FileInformationClass), NtStatus> {
    let file = file_for_request(backend, req.device_io_request.device_id, req.device_io_request.file_id)?;

    match req.file_info_class_lvl {
        FileInformationClassLevel::FILE_BASIC_INFORMATION => {
            let information = file.handle.query_basic_information().map_err(from_ntstatus)?;
            Ok((
                NtStatus::SUCCESS,
                FileBasicInformation {
                    creation_time: information.CreationTime,
                    last_access_time: information.LastAccessTime,
                    last_write_time: information.LastWriteTime,
                    change_time: information.ChangeTime,
                    file_attributes: FileAttributes::from_bits_retain(information.FileAttributes),
                }
                .into(),
            ))
        }
        FileInformationClassLevel::FILE_STANDARD_INFORMATION => {
            let information = file.handle.query_standard_information().map_err(from_ntstatus)?;
            Ok((
                NtStatus::SUCCESS,
                FileStandardInformation {
                    allocation_size: information.AllocationSize,
                    end_of_file: information.EndOfFile,
                    number_of_links: information.NumberOfLinks,
                    delete_pending: Boolean::from(u8::from(information.DeletePending)),
                    directory: Boolean::from(u8::from(information.Directory)),
                }
                .into(),
            ))
        }
        FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION => {
            let information = file.handle.query_attribute_tag_information().map_err(from_ntstatus)?;
            Ok((
                NtStatus::SUCCESS,
                FileAttributeTagInformation {
                    file_attributes: FileAttributes::from_bits_retain(information.FileAttributes),
                    reparse_tag: information.ReparseTag,
                }
                .into(),
            ))
        }
        FileInformationClassLevel::FILE_STREAM_INFORMATION => {
            let completion = file.handle.query_stream_information().map_err(from_ntstatus)?;
            Ok((
                from_ntstatus(completion.status),
                FileStreamInformation::from_buffer(completion.output).into(),
            ))
        }
        _ => Err(NtStatus::NOT_SUPPORTED),
    }
}

fn set_information_inner(
    backend: &mut WindowsRdpdrBackend,
    req: &ServerDriveSetInformationRequest,
) -> Result<(), NtStatus> {
    let file = file_for_request(backend, req.device_io_request.device_id, req.device_io_request.file_id)?;
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
        FileInformationClass::Allocation(information) => {
            if information.allocation_size < 0 {
                return Err(NtStatus::INVALID_PARAMETER);
            }
            file.handle
                .set_allocation_size(information.allocation_size)
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
        FileInformationClass::Disposition(information) => {
            let delete_pending = match information.delete_pending {
                0 => false,
                1 => true,
                _ => return Err(NtStatus::INVALID_PARAMETER),
            };
            file.handle.set_delete_pending(delete_pending).map_err(from_ntstatus)
        }
        FileInformationClass::Rename(information) => {
            let destination_path = RelativePath::parse(&information.file_name).map_err(from_path_policy)?;
            let file_name = destination_path
                .components()
                .next_back()
                .ok_or(NtStatus::OBJECT_NAME_INVALID)?;
            let root = backend.roots.get(&file.device_id).ok_or(NtStatus::INVALID_PARAMETER)?;
            let parent_directory = root
                .root
                .open_relative_parent_directory(&destination_path, RENAME_DESTINATION_DIRECTORY_ACCESS)
                .map_err(from_open_directory)?;

            file.handle
                .rename(
                    parent_directory.as_raw(),
                    file_name,
                    information.replace_if_exists == Boolean::True,
                )
                .map_err(from_ntstatus)?;
            file_for_request_mut(backend, req.device_io_request.device_id, req.device_io_request.file_id)?.path =
                destination_path;
            Ok(())
        }
        _ => Err(NtStatus::NOT_SUPPORTED),
    }
}

fn validate_create_request(req: &DeviceCreateRequest, read_only: bool, is_root: bool) -> Result<(), NtStatus> {
    let desired_access = req.desired_access.bits();
    let create_options = req.create_options.bits();
    let disposition = u32::from(req.create_disposition);
    let delete_on_close = create_options & FILE_DELETE_ON_CLOSE != 0;

    if i64::try_from(req.allocation_size).is_err() {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    if delete_on_close && desired_access & DELETE == 0 {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    if create_options & FILE_OPEN_BY_FILE_ID != 0 || create_options & !ALLOWED_CREATE_OPTIONS != 0 {
        return Err(NtStatus::NOT_SUPPORTED);
    }
    if create_options & FILE_DIRECTORY_FILE != 0 && create_options & FILE_NON_DIRECTORY_FILE != 0 {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    if !matches!(
        disposition,
        x if x == u32::from(CreateDisposition::FILE_SUPERSEDE)
            || x == u32::from(CreateDisposition::FILE_OPEN)
            || x == u32::from(CreateDisposition::FILE_CREATE)
            || x == u32::from(CreateDisposition::FILE_OPEN_IF)
            || x == u32::from(CreateDisposition::FILE_OVERWRITE)
            || x == u32::from(CreateDisposition::FILE_OVERWRITE_IF)
    ) {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    if create_options & FILE_DIRECTORY_FILE != 0
        && !matches!(
            disposition,
            x if x == u32::from(CreateDisposition::FILE_OPEN)
                || x == u32::from(CreateDisposition::FILE_CREATE)
                || x == u32::from(CreateDisposition::FILE_OPEN_IF)
        )
    {
        return Err(NtStatus::INVALID_PARAMETER);
    }

    let mutating_disposition = matches!(
        disposition,
        x if x == u32::from(CreateDisposition::FILE_SUPERSEDE)
            || x == u32::from(CreateDisposition::FILE_CREATE)
            || (x == u32::from(CreateDisposition::FILE_OPEN_IF) && !is_root)
            || x == u32::from(CreateDisposition::FILE_OVERWRITE)
            || x == u32::from(CreateDisposition::FILE_OVERWRITE_IF)
    );
    if read_only && (desired_access & READ_ONLY_MUTATION_ACCESS != 0 || mutating_disposition || delete_on_close) {
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

fn read_offset(offset: u64) -> Result<i64, NtStatus> {
    i64::try_from(offset).map_err(|_| NtStatus::INVALID_PARAMETER)
}

#[derive(Debug, PartialEq, Eq)]
enum WriteOffset {
    Explicit(i64),
    Append,
}

fn write_offset(offset: u64) -> Result<WriteOffset, NtStatus> {
    // MS-RDPEFS 2.2.1.4.4 reserves the all-ones append sentinel for clients
    // that announce minor version 0x000D or later.
    if offset == u64::MAX {
        return Ok(WriteOffset::Append);
    }

    i64::try_from(offset)
        .map(WriteOffset::Explicit)
        .map_err(|_| NtStatus::INVALID_PARAMETER)
}

pub(super) fn file_for_request(
    backend: &WindowsRdpdrBackend,
    device_id: u32,
    file_id: u32,
) -> Result<&OpenFile, NtStatus> {
    match backend.open_files.get(file_id) {
        Some(file) if file.device_id == device_id => Ok(file),
        _ => Err(NtStatus::INVALID_HANDLE),
    }
}

pub(super) fn file_for_request_mut(
    backend: &mut WindowsRdpdrBackend,
    device_id: u32,
    file_id: u32,
) -> Result<&mut OpenFile, NtStatus> {
    match backend.open_files.get_mut(file_id) {
        Some(file) if file.device_id == device_id => Ok(file),
        _ => Err(NtStatus::INVALID_HANDLE),
    }
}

fn completion_status(status: windows::Win32::Foundation::NTSTATUS) -> NtStatus {
    if status.0 < 0 {
        from_ntstatus(status)
    } else {
        NtStatus::SUCCESS
    }
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
    use ironrdp_rdpdr::pdu::efs::{
        CreateOptions, DesiredAccess, DeviceIoRequest, FileAllocationInformation, FileDispositionInformation,
        FileEndOfFileInformation, FileRenameInformation, FileSystemInformationClass, FileSystemInformationClassLevel,
        MajorFunction, MinorFunction, ServerDriveQueryVolumeInformationRequest, SharedAccess,
    };
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::windows::factory::RedirectedDrive;
    use crate::windows::volume::query_information_inner as query_volume_information_inner;

    #[test]
    fn create_information_matches_nt_create_file_results() {
        assert_eq!(
            create_information(FILE_SUPERSEDED_INFORMATION),
            Ok(Information::file_superseded())
        );
        assert_eq!(
            create_information(FILE_OPENED_INFORMATION),
            Ok(Information::file_opened())
        );
        assert_eq!(
            create_information(FILE_CREATED_INFORMATION),
            Ok(Information::file_created())
        );
        assert_eq!(
            create_information(FILE_OVERWRITTEN_INFORMATION),
            Ok(Information::file_overwritten())
        );
    }

    #[test]
    fn read_only_drives_reject_mutating_create_requests() {
        let request = create_request(FILE_WRITE_DATA, CreateDisposition::FILE_OPEN, 0, r"\example.txt");

        assert_eq!(
            validate_create_request(&request, true, false),
            Err(NtStatus::MEDIA_WRITE_PROTECTED)
        );
    }

    #[test]
    fn create_requests_allow_security_descriptor_access() {
        let request = create_request(WRITE_DAC, CreateDisposition::FILE_OPEN, 0, r"\example.txt");

        assert_eq!(validate_create_request(&request, false, false), Ok(()));
    }

    #[test]
    fn create_requests_allow_backup_intent() {
        let request = create_request(
            FILE_WRITE_DATA,
            CreateDisposition::FILE_OPEN_IF,
            FILE_OPEN_FOR_BACKUP_INTENT,
            r"\example.txt",
        );

        assert_eq!(validate_create_request(&request, false, false), Ok(()));
    }

    #[test]
    fn create_requests_allow_non_mutating_cache_and_capacity_hints() {
        for option in [FILE_OPEN_NO_RECALL, FILE_OPEN_FOR_FREE_SPACE_QUERY] {
            let request = create_request(
                FILE_READ_ATTRIBUTES,
                CreateDisposition::FILE_OPEN,
                option,
                r"\example.txt",
            );

            assert_eq!(validate_create_request(&request, false, false), Ok(()));
        }
    }

    #[test]
    fn backup_intent_does_not_change_the_native_open_access_check() {
        let temporary_directory = TemporaryDirectory::new();
        let root_path = temporary_directory
            .0
            .ancestors()
            .last()
            .expect("temporary directory has a volume root")
            .to_owned();
        let relative_path = temporary_directory
            .0
            .strip_prefix(&root_path)
            .expect("temporary directory is beneath its volume root");
        let drive = RedirectedDrive::new(1, "Test", root_path, false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::from_active_drive(drive);
        let request = create_request(
            0x0010_0001,
            CreateDisposition::FILE_OPEN,
            FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_FOR_BACKUP_INTENT,
            &format!(r"\{}", relative_path.display()),
        );

        assert!(create_inner(&mut backend, &request).is_ok());
    }

    #[test]
    fn delete_on_close_requires_delete_access() {
        let request = create_request(
            FILE_WRITE_DATA,
            CreateDisposition::FILE_OPEN,
            FILE_DELETE_ON_CLOSE,
            r"\example.txt",
        );

        assert_eq!(
            validate_create_request(&request, false, false),
            Err(NtStatus::INVALID_PARAMETER)
        );
        assert_eq!(
            validate_create_request(&request, true, false),
            Err(NtStatus::INVALID_PARAMETER)
        );

        let mut request = create_request(FILE_WRITE_DATA, CreateDisposition::FILE_OPEN, 0, r"\example.txt");
        request.allocation_size = u64::MAX;
        assert_eq!(
            validate_create_request(&request, false, false),
            Err(NtStatus::INVALID_PARAMETER)
        );
    }

    #[test]
    fn root_open_rejects_inapplicable_mutation_options() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let drive =
            RedirectedDrive::new(1, "Test", format!(r"{system_drive}\"), false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::from_active_drive(drive);

        let delete_request = create_request(DELETE, CreateDisposition::FILE_OPEN, FILE_DELETE_ON_CLOSE, r"\");
        assert_eq!(
            create_inner(&mut backend, &delete_request),
            Err(NtStatus::ACCESS_DENIED)
        );

        let mut allocation_request = create_request(0x0000_0001, CreateDisposition::FILE_OPEN, 0, r"\");
        allocation_request.allocation_size = 1;
        assert_eq!(
            create_inner(&mut backend, &allocation_request),
            Err(NtStatus::INVALID_PARAMETER)
        );
    }

    #[test]
    fn root_open_if_opens_the_existing_volume_root() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let drive = RedirectedDrive::new(1, "Test", format!(r"{system_drive}\"), true).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::from_active_drive(drive);
        let request = create_request(
            FILE_READ_ATTRIBUTES,
            CreateDisposition::FILE_OPEN_IF,
            FILE_DIRECTORY_FILE,
            r"\",
        );

        assert_eq!(
            create_inner(&mut backend, &request),
            Ok((1, Information::file_opened()))
        );
    }

    #[test]
    fn directory_open_allows_metadata_queries_after_an_explorer_directory_list_request() {
        let temporary_directory = TemporaryDirectory::new();
        let root_path = temporary_directory
            .0
            .ancestors()
            .last()
            .expect("temporary directory has a volume root")
            .to_owned();
        let relative_path = temporary_directory
            .0
            .strip_prefix(&root_path)
            .expect("temporary directory is beneath its volume root");
        let drive = RedirectedDrive::new(1, "Test", root_path, false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::from_active_drive(drive);
        let mut request = create_request(
            0x0010_0001,
            CreateDisposition::FILE_OPEN,
            FILE_DIRECTORY_FILE,
            &format!(r"\{}", relative_path.display()),
        );
        request.shared_access = SharedAccess::from_bits_retain(0x0000_0007);

        let (file_id, _) = create_inner(&mut backend, &request).expect("open directory for listing");
        let watcher_handle = backend
            .open_directory_for_notification(1, file_id)
            .expect("reopen an independent directory watcher handle");
        let open_handle = backend
            .open_files
            .get(file_id)
            .expect("open directory remains in the file table")
            .handle
            .as_raw();

        assert_ne!(watcher_handle.as_raw(), open_handle);

        query_information_inner(
            &backend,
            &query_request(file_id, FileInformationClassLevel::FILE_BASIC_INFORMATION),
        )
        .expect("query directory metadata without an explicit read-attributes request");
    }

    #[test]
    fn file_open_allows_metadata_queries_after_an_explorer_file_open_request() {
        let temporary_file = TemporaryFile::create();
        let root_path = temporary_file
            .0
            .ancestors()
            .last()
            .expect("temporary file has a volume root")
            .to_owned();
        let relative_path = temporary_file
            .0
            .strip_prefix(&root_path)
            .expect("temporary file is beneath its volume root");
        let drive = RedirectedDrive::new(1, "Test", root_path, false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::from_active_drive(drive);
        let request = create_request(
            SYNCHRONIZE,
            CreateDisposition::FILE_OPEN,
            0,
            &format!(r"\{}", relative_path.display()),
        );

        let (file_id, _) = create_inner(&mut backend, &request).expect("open file");

        query_information_inner(
            &backend,
            &query_request(file_id, FileInformationClassLevel::FILE_BASIC_INFORMATION),
        )
        .expect("query file metadata without an explicit read-attributes request");
    }

    #[test]
    fn file_open_with_reparse_point_option_allows_an_ordinary_file() {
        let temporary_file = TemporaryFile::create();
        let root_path = temporary_file
            .0
            .ancestors()
            .last()
            .expect("temporary file has a volume root")
            .to_owned();
        let relative_path = temporary_file
            .0
            .strip_prefix(&root_path)
            .expect("temporary file is beneath its volume root");
        let drive = RedirectedDrive::new(1, "Test", root_path, false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::from_active_drive(drive);

        create_inner(
            &mut backend,
            &create_request(
                FILE_READ_ATTRIBUTES,
                CreateDisposition::FILE_OPEN,
                FILE_OPEN_REPARSE_POINT,
                &format!(r"\{}", relative_path.display()),
            ),
        )
        .expect("open ordinary file with reparse-point option");
    }

    #[test]
    fn read_response_preserves_requested_non_eof_length() {
        let temporary_file = TemporaryFile::new();
        let expected_data = vec![0x5A; MAX_STATIC_IO_SIZE];
        std::fs::write(&temporary_file.0, &expected_data).expect("write temporary file");
        let root_path = temporary_file
            .0
            .ancestors()
            .last()
            .expect("temporary file has a volume root")
            .to_owned();
        let relative_path = temporary_file
            .0
            .strip_prefix(&root_path)
            .expect("temporary file is beneath its volume root");
        let drive = RedirectedDrive::new(1, "Test", root_path, false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::from_active_drive(drive);
        let (file_id, _) = create_inner(
            &mut backend,
            &create_request(
                0x0010_0081,
                CreateDisposition::FILE_OPEN,
                0,
                &format!(r"\{}", relative_path.display()),
            ),
        )
        .expect("open temporary file");
        let response = read(
            &mut backend,
            DeviceReadRequest {
                device_io_request: DeviceIoRequest {
                    device_id: 1,
                    file_id,
                    completion_id: 1,
                    major_function: MajorFunction::Read,
                    minor_function: MinorFunction::from(0),
                },
                length: u32::try_from(MAX_STATIC_IO_SIZE).expect("maximum I/O size fits in u32"),
                offset: 0,
            },
        )
        .expect("read temporary file")
        .into_iter()
        .next()
        .expect("read produces one completion")
        .encode_unframed_pdu()
        .expect("read completion is encodable");

        assert_eq!(response.len(), expected_data.len() + 20);
        assert_eq!(&response[20..], expected_data);
    }

    #[test]
    fn read_response_honors_large_offset() {
        let temporary_file = TemporaryFile::new();
        let mut file_data = vec![0; MAX_STATIC_IO_SIZE * 9];
        for (block, data) in file_data.chunks_exact_mut(MAX_STATIC_IO_SIZE).enumerate() {
            data.fill(u8::try_from(block).expect("test block fits in u8"));
        }
        std::fs::write(&temporary_file.0, file_data).expect("write temporary file");
        let root_path = temporary_file
            .0
            .ancestors()
            .last()
            .expect("temporary file has a volume root")
            .to_owned();
        let relative_path = temporary_file
            .0
            .strip_prefix(&root_path)
            .expect("temporary file is beneath its volume root");
        let drive = RedirectedDrive::new(1, "Test", root_path, false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::from_active_drive(drive);
        let (file_id, _) = create_inner(
            &mut backend,
            &create_request(
                0x0010_0081,
                CreateDisposition::FILE_OPEN,
                0,
                &format!(r"\{}", relative_path.display()),
            ),
        )
        .expect("open temporary file");
        let response = read(
            &mut backend,
            DeviceReadRequest {
                device_io_request: DeviceIoRequest {
                    device_id: 1,
                    file_id,
                    completion_id: 1,
                    major_function: MajorFunction::Read,
                    minor_function: MinorFunction::from(0),
                },
                length: u32::try_from(MAX_STATIC_IO_SIZE).expect("maximum I/O size fits in u32"),
                offset: u64::try_from(MAX_STATIC_IO_SIZE * 7).expect("test offset fits in u64"),
            },
        )
        .expect("read temporary file")
        .into_iter()
        .next()
        .expect("read produces one completion")
        .encode_unframed_pdu()
        .expect("read completion is encodable");

        assert_eq!(response.len(), MAX_STATIC_IO_SIZE + 20);
        assert_eq!(&response[20..], vec![7; MAX_STATIC_IO_SIZE]);
    }

    #[test]
    fn create_applies_allocation_size_and_deletes_on_close() {
        let temporary_file = TemporaryFile::new();
        let root_path = temporary_file
            .0
            .ancestors()
            .last()
            .expect("temporary file has a volume root")
            .to_owned();
        let relative_path = temporary_file
            .0
            .strip_prefix(&root_path)
            .expect("temporary file is beneath its volume root");
        let drive = RedirectedDrive::new(1, "Test", root_path, false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::from_active_drive(drive);
        let mut request = create_request(
            FILE_WRITE_DATA | FILE_READ_ATTRIBUTES | DELETE,
            CreateDisposition::FILE_CREATE,
            FILE_DELETE_ON_CLOSE,
            &format!(r"\{}", relative_path.display()),
        );
        request.allocation_size = 4_096;

        let (file_id, information) = create_inner(&mut backend, &request).expect("create temporary file");
        assert_eq!(information, Information::file_created());
        let FileInformationClass::Standard(standard_information) = query_information_inner(
            &backend,
            &query_request(file_id, FileInformationClassLevel::FILE_STANDARD_INFORMATION),
        )
        .expect("query created file information") else {
            panic!("standard query must return FileStandardInformation");
        };
        assert!(standard_information.allocation_size >= 4_096);

        close(
            &mut backend,
            DeviceCloseRequest {
                device_io_request: DeviceIoRequest {
                    device_id: 1,
                    file_id,
                    completion_id: 1,
                    major_function: MajorFunction::Close,
                    minor_function: MinorFunction::from(0),
                },
            },
        )
        .expect("close delete-on-close file");
        assert!(!temporary_file.0.exists());
    }

    #[test]
    fn static_file_operations_use_explicit_and_append_offsets_with_handle_bound_information_queries() {
        let mut temporary_file = TemporaryFile::create();
        let root_path = temporary_file
            .0
            .ancestors()
            .last()
            .expect("temporary file has a volume root")
            .to_owned();
        let relative_path = temporary_file
            .0
            .strip_prefix(&root_path)
            .expect("temporary file is beneath its volume root");
        let drive = RedirectedDrive::new(1, "Test", root_path.clone(), false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::from_active_drive(drive);
        let path = format!(r"\{}", relative_path.display());
        let request = create_request(0x0001_0183, CreateDisposition::FILE_OPEN, 0, &path);
        let (file_id, _) = create_inner(&mut backend, &request).expect("open temporary file");
        let write_request = DeviceWriteRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id,
                completion_id: 2,
                major_function: MajorFunction::Write,
                minor_function: MinorFunction::from(0),
            },
            offset: 1,
            write_data: b"X".to_vec(),
        };

        let write = write_inner(&backend, &write_request).expect("write through opaque handle");
        assert_eq!(completion_status(write.status), NtStatus::SUCCESS);
        assert_eq!(write.transferred, 1);

        let append_request = DeviceWriteRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id,
                completion_id: 3,
                major_function: MajorFunction::Write,
                minor_function: MinorFunction::from(0),
            },
            offset: u64::MAX,
            write_data: b"!".to_vec(),
        };
        let append = write_inner(&backend, &append_request).expect("append through opaque handle");
        assert_eq!(completion_status(append.status), NtStatus::SUCCESS);
        assert_eq!(append.transferred, 1);

        let flush_response = flush(
            &backend,
            DeviceFlushBuffersRequest {
                device_io_request: DeviceIoRequest {
                    device_id: 1,
                    file_id,
                    completion_id: 3,
                    major_function: MajorFunction::FlushBuffers,
                    minor_function: MinorFunction::from(0),
                },
            },
        )
        .expect("flush through opaque handle")
        .into_iter()
        .next()
        .expect("flush produces one completion")
        .encode_unframed_pdu()
        .expect("flush completion is encodable");
        assert_eq!(flush_response.len(), 16);
        assert_eq!(
            u32::from_le_bytes(flush_response[12..16].try_into().expect("response status is present")),
            u32::from(NtStatus::SUCCESS)
        );

        let read_request = DeviceReadRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id,
                completion_id: 4,
                major_function: MajorFunction::Read,
                minor_function: MinorFunction::from(0),
            },
            length: 6,
            offset: 0,
        };
        assert_eq!(
            read_inner(&backend, &read_request).expect("read through opaque handle"),
            b"hXllo!"
        );
        assert!(matches!(
            file_for_request(&backend, 2, file_id),
            Err(NtStatus::INVALID_HANDLE)
        ));

        let basic_information = query_information_inner(
            &backend,
            &query_request(file_id, FileInformationClassLevel::FILE_BASIC_INFORMATION),
        )
        .expect("query basic information");
        let FileInformationClass::Basic(basic_information) = basic_information else {
            panic!("basic query must return FileBasicInformation");
        };
        assert!(
            !basic_information
                .file_attributes
                .contains(FileAttributes::FILE_ATTRIBUTE_DIRECTORY)
        );

        let standard_information = query_information_inner(
            &backend,
            &query_request(file_id, FileInformationClassLevel::FILE_STANDARD_INFORMATION),
        )
        .expect("query standard information");
        let FileInformationClass::Standard(standard_information) = standard_information else {
            panic!("standard query must return FileStandardInformation");
        };
        assert_eq!(standard_information.end_of_file, 6);
        assert!(standard_information.allocation_size >= standard_information.end_of_file);
        assert_eq!(standard_information.directory, Boolean::False);

        let attribute_tag_information = query_information_inner(
            &backend,
            &query_request(file_id, FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION),
        )
        .expect("query attribute tag information");
        let FileInformationClass::AttributeTag(attribute_tag_information) = attribute_tag_information else {
            panic!("attribute tag query must return FileAttributeTagInformation");
        };
        assert_eq!(attribute_tag_information.reparse_tag, 0);

        let volume_information = query_volume_information_inner(
            &backend,
            &query_volume_request(file_id, FileSystemInformationClassLevel::FILE_FS_VOLUME_INFORMATION),
        )
        .expect("query volume information");
        let FileSystemInformationClass::FileFsVolumeInformation(volume_information) = volume_information else {
            panic!("volume query must return FileFsVolumeInformation");
        };
        assert!(!volume_information.volume_label.contains('\0'));

        let size_information = query_volume_information_inner(
            &backend,
            &query_volume_request(file_id, FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION),
        )
        .expect("query volume size information");
        let FileSystemInformationClass::FileFsSizeInformation(size_information) = size_information else {
            panic!("size query must return FileFsSizeInformation");
        };
        assert!(size_information.total_alloc_units >= size_information.available_alloc_units);
        assert_ne!(size_information.sectors_per_alloc_unit, 0);
        assert_ne!(size_information.bytes_per_sector, 0);

        let attribute_information = query_volume_information_inner(
            &backend,
            &query_volume_request(file_id, FileSystemInformationClassLevel::FILE_FS_ATTRIBUTE_INFORMATION),
        )
        .expect("query volume attribute information");
        let FileSystemInformationClass::FileFsAttributeInformation(attribute_information) = attribute_information
        else {
            panic!("attribute query must return FileFsAttributeInformation");
        };
        assert_ne!(attribute_information.max_component_name_len, 0);
        assert!(!attribute_information.file_system_name.is_empty());

        let full_size_information = query_volume_information_inner(
            &backend,
            &query_volume_request(file_id, FileSystemInformationClassLevel::FILE_FS_FULL_SIZE_INFORMATION),
        )
        .expect("query full volume size information");
        let FileSystemInformationClass::FileFsFullSizeInformation(full_size_information) = full_size_information else {
            panic!("full size query must return FileFsFullSizeInformation");
        };
        assert!(full_size_information.total_alloc_units >= full_size_information.caller_available_alloc_units);
        assert_ne!(full_size_information.sectors_per_alloc_unit, 0);
        assert_ne!(full_size_information.bytes_per_sector, 0);

        let device_information = query_volume_information_inner(
            &backend,
            &query_volume_request(file_id, FileSystemInformationClassLevel::FILE_FS_DEVICE_INFORMATION),
        );
        assert_eq!(device_information, Err(NtStatus::INVALID_DEVICE_REQUEST));

        set_information_inner(
            &mut backend,
            &set_request(
                file_id,
                FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 1 }),
            ),
        )
        .expect("set end of file");
        let standard_information = query_information_inner(
            &backend,
            &query_request(file_id, FileInformationClassLevel::FILE_STANDARD_INFORMATION),
        )
        .expect("query standard information after truncation");
        let FileInformationClass::Standard(standard_information) = standard_information else {
            panic!("standard query must return FileStandardInformation");
        };
        assert_eq!(standard_information.end_of_file, 1);

        set_information_inner(
            &mut backend,
            &set_request(
                file_id,
                FileInformationClass::Allocation(FileAllocationInformation { allocation_size: 4_096 }),
            ),
        )
        .expect("set allocation size");
        let standard_information = query_information_inner(
            &backend,
            &query_request(file_id, FileInformationClassLevel::FILE_STANDARD_INFORMATION),
        )
        .expect("query standard information after allocation");
        let FileInformationClass::Standard(standard_information) = standard_information else {
            panic!("standard query must return FileStandardInformation");
        };
        assert!(standard_information.allocation_size >= 4_096);

        set_information_inner(
            &mut backend,
            &set_request(
                file_id,
                FileInformationClass::Basic(FileBasicInformation {
                    creation_time: 0,
                    last_access_time: 0,
                    last_write_time: 0,
                    change_time: 0,
                    file_attributes: FileAttributes::FILE_ATTRIBUTE_HIDDEN,
                }),
            ),
        )
        .expect("set basic file information");
        let basic_information = query_information_inner(
            &backend,
            &query_request(file_id, FileInformationClassLevel::FILE_BASIC_INFORMATION),
        )
        .expect("query basic information after attribute update");
        let FileInformationClass::Basic(basic_information) = basic_information else {
            panic!("basic query must return FileBasicInformation");
        };
        assert!(
            basic_information
                .file_attributes
                .contains(FileAttributes::FILE_ATTRIBUTE_HIDDEN)
        );

        assert_eq!(
            set_information_inner(
                &mut backend,
                &set_request(
                    file_id,
                    FileInformationClass::Basic(FileBasicInformation {
                        creation_time: -3,
                        last_access_time: 0,
                        last_write_time: 0,
                        change_time: 0,
                        file_attributes: FileAttributes::empty(),
                    }),
                ),
            ),
            Err(NtStatus::INVALID_PARAMETER)
        );

        let renamed_path = temporary_file.0.with_file_name("z");
        let renamed_relative_path = renamed_path
            .strip_prefix(&root_path)
            .expect("renamed temporary file is beneath its volume root");
        set_information_inner(
            &mut backend,
            &set_request(
                file_id,
                FileInformationClass::Rename(FileRenameInformation {
                    replace_if_exists: Boolean::False,
                    file_name: format!(r"\{}", renamed_relative_path.display()),
                }),
            ),
        )
        .expect("rename through a reparse-safe destination parent");
        assert!(!temporary_file.0.exists());
        assert!(renamed_path.exists());
        temporary_file.0 = renamed_path;

        set_information_inner(
            &mut backend,
            &set_request(
                file_id,
                FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 1 }),
            ),
        )
        .expect("mark temporary file for deletion");
        let standard_information = query_information_inner(
            &backend,
            &query_request(file_id, FileInformationClassLevel::FILE_STANDARD_INFORMATION),
        )
        .expect("query standard information after marking for deletion");
        let FileInformationClass::Standard(standard_information) = standard_information else {
            panic!("standard query must return FileStandardInformation");
        };
        assert_eq!(standard_information.delete_pending, Boolean::True);

        let _ = close(
            &mut backend,
            DeviceCloseRequest {
                device_io_request: DeviceIoRequest {
                    device_id: 1,
                    file_id,
                    completion_id: 7,
                    major_function: MajorFunction::Close,
                    minor_function: MinorFunction::from(0),
                },
            },
        )
        .expect("close deleted temporary file");
        assert!(!temporary_file.0.exists());
    }

    #[test]
    fn static_file_operations_open_alternate_data_streams() {
        let temporary_file = TemporaryFile::create();
        let root_path = temporary_file
            .0
            .ancestors()
            .last()
            .expect("temporary file has a volume root")
            .to_owned();
        let relative_path = temporary_file
            .0
            .strip_prefix(&root_path)
            .expect("temporary file is beneath its volume root");
        let drive = RedirectedDrive::new(1, "Test", root_path, false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::from_active_drive(drive);
        let stream_name = "ironrdp-rdpdr";
        let request = create_request(
            FILE_WRITE_DATA,
            CreateDisposition::FILE_OPEN_IF,
            0,
            &format!(r"\{}:{}", relative_path.display(), stream_name),
        );
        let (file_id, _) = create_inner(&mut backend, &request).expect("open alternate data stream");

        let write_request = DeviceWriteRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id,
                completion_id: 2,
                major_function: MajorFunction::Write,
                minor_function: MinorFunction::from(0),
            },
            offset: 0,
            write_data: b"stream".to_vec(),
        };
        assert_eq!(
            write_inner(&backend, &write_request)
                .expect("write alternate data stream")
                .transferred,
            6
        );
        close(
            &mut backend,
            DeviceCloseRequest {
                device_io_request: DeviceIoRequest {
                    device_id: 1,
                    file_id,
                    completion_id: 3,
                    major_function: MajorFunction::Close,
                    minor_function: MinorFunction::from(0),
                },
            },
        )
        .expect("close alternate data stream");

        assert_eq!(
            std::fs::read(format!("{}:{}", temporary_file.0.display(), stream_name))
                .expect("read alternate data stream"),
            b"stream"
        );

        let base_request = create_request(
            FILE_READ_ATTRIBUTES,
            CreateDisposition::FILE_OPEN,
            0,
            &format!(r"\{}", relative_path.display()),
        );
        let (base_file_id, _) = create_inner(&mut backend, &base_request).expect("open base file for stream query");
        let FileInformationClass::Stream(stream_information) = query_information_inner(
            &backend,
            &query_request(base_file_id, FileInformationClassLevel::FILE_STREAM_INFORMATION),
        )
        .expect("query file streams") else {
            panic!("stream query must return FileStreamInformation");
        };
        let mut encoded = vec![0; stream_information.size()];
        stream_information
            .encode(&mut ironrdp_core::WriteCursor::new(&mut encoded))
            .expect("encode stream information");
        let expected_stream_name = format!(":{stream_name}:$DATA")
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        assert!(
            encoded
                .windows(expected_stream_name.len())
                .any(|window| window == expected_stream_name),
            "stream query must enumerate the named alternate data stream"
        );
    }

    #[test]
    fn rename_supports_directories_and_replace_if_exists() {
        let temporary_directory = TemporaryDirectory::new();
        let source_directory = temporary_directory.0.join("source-directory");
        let renamed_directory = temporary_directory.0.join("renamed-directory");
        std::fs::create_dir(&source_directory).expect("create source directory");

        let root_path = temporary_directory
            .0
            .ancestors()
            .last()
            .expect("temporary directory has a volume root")
            .to_owned();
        let source_relative_path = source_directory
            .strip_prefix(&root_path)
            .expect("source directory is beneath its volume root");
        let renamed_relative_path = renamed_directory
            .strip_prefix(&root_path)
            .expect("renamed directory is beneath its volume root");
        let drive = RedirectedDrive::new(1, "Test", root_path.clone(), false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::from_active_drive(drive);
        let request = create_request(
            DELETE | FILE_READ_ATTRIBUTES,
            CreateDisposition::FILE_OPEN,
            FILE_DIRECTORY_FILE,
            &format!(r"\{}", source_relative_path.display()),
        );
        let (directory_file_id, _) = create_inner(&mut backend, &request).expect("open source directory");

        set_information_inner(
            &mut backend,
            &set_request(
                directory_file_id,
                FileInformationClass::Rename(FileRenameInformation {
                    replace_if_exists: Boolean::False,
                    file_name: format!(r"\{}", renamed_relative_path.display()),
                }),
            ),
        )
        .expect("rename directory through a reparse-safe destination parent");
        assert!(!source_directory.exists());
        assert!(renamed_directory.is_dir());
        assert_eq!(
            file_for_request(&backend, 1, directory_file_id)
                .expect("renamed directory remains open")
                .path,
            RelativePath::parse(&format!(r"\{}", renamed_relative_path.display()))
                .expect("renamed directory has a valid redirected path")
        );
        close(
            &mut backend,
            DeviceCloseRequest {
                device_io_request: DeviceIoRequest {
                    device_id: 1,
                    file_id: directory_file_id,
                    completion_id: 8,
                    major_function: MajorFunction::Close,
                    minor_function: MinorFunction::from(0),
                },
            },
        )
        .expect("close renamed directory");

        let source_file = temporary_directory.0.join("source-file");
        let destination_parent = temporary_directory.0.join("destination-parent");
        let destination_file = destination_parent.join("destination-file");
        std::fs::create_dir(&destination_parent).expect("create destination parent directory");
        std::fs::write(&source_file, b"source").expect("create source file");
        std::fs::write(&destination_file, b"destination").expect("create destination file");
        let source_relative_path = source_file
            .strip_prefix(&root_path)
            .expect("source file is beneath its volume root");
        let destination_relative_path = destination_file
            .strip_prefix(&root_path)
            .expect("destination file is beneath its volume root");
        let request = create_request(
            DELETE | FILE_READ_ATTRIBUTES,
            CreateDisposition::FILE_OPEN,
            0,
            &format!(r"\{}", source_relative_path.display()),
        );
        let (file_id, _) = create_inner(&mut backend, &request).expect("open source file");

        assert_eq!(
            set_information_inner(
                &mut backend,
                &set_request(
                    file_id,
                    FileInformationClass::Rename(FileRenameInformation {
                        replace_if_exists: Boolean::False,
                        file_name: format!(r"\{}", destination_relative_path.display()),
                    }),
                ),
            ),
            Err(NtStatus::OBJECT_NAME_COLLISION)
        );
        assert!(source_file.is_file());
        assert_eq!(
            std::fs::read(&destination_file).expect("read destination file"),
            b"destination"
        );

        set_information_inner(
            &mut backend,
            &set_request(
                file_id,
                FileInformationClass::Rename(FileRenameInformation {
                    replace_if_exists: Boolean::True,
                    file_name: format!(r"\{}", destination_relative_path.display()),
                }),
            ),
        )
        .expect("replace destination file during rename");
        assert!(!source_file.exists());
        close(
            &mut backend,
            DeviceCloseRequest {
                device_io_request: DeviceIoRequest {
                    device_id: 1,
                    file_id,
                    completion_id: 9,
                    major_function: MajorFunction::Close,
                    minor_function: MinorFunction::from(0),
                },
            },
        )
        .expect("close renamed file");
        assert_eq!(
            std::fs::read(&destination_file).expect("read replaced destination file"),
            b"source"
        );
    }

    #[test]
    fn static_io_rejects_oversized_or_invalid_offsets() {
        assert_eq!(
            validate_io_length(MAX_STATIC_IO_SIZE + 1),
            Err(NtStatus::INVALID_PARAMETER)
        );
        assert_eq!(read_offset(u64::MAX), Err(NtStatus::INVALID_PARAMETER));
        assert!(matches!(write_offset(u64::MAX), Ok(WriteOffset::Append)));
        assert_eq!(write_offset(1), Ok(WriteOffset::Explicit(1)));
    }

    fn create_request(
        desired_access: u32,
        create_disposition: CreateDisposition,
        create_options: u32,
        path: &str,
    ) -> DeviceCreateRequest {
        DeviceCreateRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id: 0,
                completion_id: 1,
                major_function: MajorFunction::Create,
                minor_function: MinorFunction::from(0),
            },
            desired_access: DesiredAccess::from_bits_retain(desired_access),
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::empty(),
            create_disposition,
            create_options: CreateOptions::from_bits_retain(create_options),
            path: path.to_owned(),
        }
    }

    fn query_request(
        file_id: u32,
        file_info_class_lvl: FileInformationClassLevel,
    ) -> ServerDriveQueryInformationRequest {
        ServerDriveQueryInformationRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id,
                completion_id: 4,
                major_function: MajorFunction::QueryInformation,
                minor_function: MinorFunction::from(0),
            },
            file_info_class_lvl,
        }
    }

    fn query_volume_request(
        file_id: u32,
        fs_info_class_lvl: FileSystemInformationClassLevel,
    ) -> ServerDriveQueryVolumeInformationRequest {
        ServerDriveQueryVolumeInformationRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id,
                completion_id: 5,
                major_function: MajorFunction::QueryVolumeInformation,
                minor_function: MinorFunction::from(0),
            },
            fs_info_class_lvl,
        }
    }

    fn set_request(file_id: u32, set_buffer: FileInformationClass) -> ServerDriveSetInformationRequest {
        ServerDriveSetInformationRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id,
                completion_id: 6,
                major_function: MajorFunction::SetInformation,
                minor_function: MinorFunction::from(0),
            },
            set_buffer,
        }
    }

    struct TemporaryFile(PathBuf);

    impl TemporaryFile {
        fn create() -> Self {
            let path = Self::new_path();
            std::fs::write(&path, b"hello").expect("create temporary file");
            Self(path)
        }

        fn new() -> Self {
            Self(Self::new_path())
        }

        fn new_path() -> PathBuf {
            let unique_name = format!(
                "ironrdp-rdpdr-native-{}-{}.tmp",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock is after Unix epoch")
                    .as_nanos()
            );
            std::env::temp_dir().join(unique_name)
        }
    }

    impl Drop for TemporaryFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    struct TemporaryDirectory(PathBuf);

    impl TemporaryDirectory {
        fn new() -> Self {
            let path = TemporaryFile::new_path();
            std::fs::create_dir(&path).expect("create temporary directory");
            Self(path)
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
