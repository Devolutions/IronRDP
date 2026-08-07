//! Windows RDPDR directory enumeration on already-confined directory handles.

use ironrdp_pdu::PduResult;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    ClientDriveNotifyChangeDirectoryResponse, ClientDriveQueryDirectoryResponse, DeviceIoResponse,
    FileBothDirectoryInformation, FileDirectoryInformation, FileFullDirectoryInformation, FileInformationClass,
    FileInformationClassLevel, FileNamesInformation, NtStatus, ServerDriveNotifyChangeDirectoryRequest,
    ServerDriveQueryDirectoryRequest,
};
use ironrdp_svc::SvcMessage;
use windows::Wdk::Storage::FileSystem::{
    FileBothDirectoryInformation as NativeFileBothDirectoryInformation,
    FileDirectoryInformation as NativeFileDirectoryInformation,
    FileFullDirectoryInformation as NativeFileFullDirectoryInformation,
    FileNamesInformation as NativeFileNamesInformation,
};

use super::backend::WindowsRdpdrBackend;
use super::file::{file_for_request, file_for_request_mut};
use super::handles::{DirectoryHandle, FileHandle};
use super::path::RelativePath;
use super::status::{from_ntstatus, from_open_directory, from_path_policy};

const FILE_DIRECTORY_INFORMATION_NAME_OFFSET: usize = 64;
const FILE_FULL_DIRECTORY_INFORMATION_NAME_OFFSET: usize = 68;
const FILE_BOTH_DIRECTORY_INFORMATION_SHORT_NAME_OFFSET: usize = 70;
const FILE_BOTH_DIRECTORY_INFORMATION_NAME_OFFSET: usize = 94;
const FILE_NAMES_INFORMATION_NAME_OFFSET: usize = 12;
const SHORT_NAME_SIZE: usize = 24;
const FILE_NOTIFY_VALID_FILTERS: u32 = 0x0000_0fff;

pub(crate) fn query_information(
    backend: &mut WindowsRdpdrBackend,
    req: ServerDriveQueryDirectoryRequest,
) -> PduResult<Vec<SvcMessage>> {
    let initial_query = req.initial_query != 0;
    let (status, buffer) = match query_information_inner(backend, &req) {
        Ok(buffer) => (NtStatus::SUCCESS, Some(buffer)),
        Err(NtStatus::NO_MORE_FILES) if initial_query => (NtStatus::NO_SUCH_FILE, None),
        Err(status) => (status, None),
    };

    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
        ClientDriveQueryDirectoryResponse {
            device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
            buffer,
        },
    ))])
}

pub(crate) fn notify_change(
    backend: &mut WindowsRdpdrBackend,
    req: ServerDriveNotifyChangeDirectoryRequest,
) -> PduResult<Vec<SvcMessage>> {
    let status = if req.completion_filter == 0 || req.completion_filter & !FILE_NOTIFY_VALID_FILTERS != 0 {
        Err(NtStatus::INVALID_PARAMETER)
    } else {
        backend.schedule_directory_notification(req.clone())
    };

    match status {
        Ok(()) => Ok(Vec::new()),
        Err(status) => Ok(vec![SvcMessage::from(
            RdpdrPdu::ClientDriveNotifyChangeDirectoryResponse(ClientDriveNotifyChangeDirectoryResponse::new(
                req.device_io_request,
                status,
                Vec::new(),
            )),
        )]),
    }
}

pub(super) fn query_information_inner(
    backend: &mut WindowsRdpdrBackend,
    req: &ServerDriveQueryDirectoryRequest,
) -> Result<FileInformationClass, NtStatus> {
    let initial_query = req.initial_query != 0;
    let native_information_class = native_information_class(req.file_info_class_lvl.clone())?;
    let pattern = if initial_query {
        let query_path = query_path(&req.path)?;
        let device_id =
            file_for_request(backend, req.device_io_request.device_id, req.device_io_request.file_id)?.device_id;
        let directory_handle = open_query_directory(backend, device_id, &query_path.directory)?;
        let file = file_for_request_mut(backend, req.device_io_request.device_id, req.device_io_request.file_id)?;
        file.directory_query_handle = directory_handle;
        Some(query_path.pattern)
    } else {
        file_for_request(backend, req.device_io_request.device_id, req.device_io_request.file_id)?;
        None
    };
    let file = file_for_request(backend, req.device_io_request.device_id, req.device_io_request.file_id)?;
    let directory_handle = file.directory_query_handle.as_ref().unwrap_or(&file.handle);
    let native_buffer = directory_handle
        .query_directory_information(native_information_class, pattern.as_deref(), initial_query)
        .map_err(from_ntstatus)?;

    parse_information(req.file_info_class_lvl.clone(), &native_buffer)
}

fn native_information_class(
    information_class: FileInformationClassLevel,
) -> Result<windows::Wdk::Storage::FileSystem::FILE_INFORMATION_CLASS, NtStatus> {
    match information_class {
        FileInformationClassLevel::FILE_DIRECTORY_INFORMATION => Ok(NativeFileDirectoryInformation),
        FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION => Ok(NativeFileFullDirectoryInformation),
        FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION => Ok(NativeFileBothDirectoryInformation),
        FileInformationClassLevel::FILE_NAMES_INFORMATION => Ok(NativeFileNamesInformation),
        _ => Err(NtStatus::NOT_SUPPORTED),
    }
}

fn parse_information(
    information_class: FileInformationClassLevel,
    buffer: &[u8],
) -> Result<FileInformationClass, NtStatus> {
    match information_class {
        FileInformationClassLevel::FILE_DIRECTORY_INFORMATION => Ok(FileDirectoryInformation {
            next_entry_offset: 0,
            file_index: read_u32(buffer, 4)?,
            creation_time: read_i64(buffer, 8)?,
            last_access_time: read_i64(buffer, 16)?,
            last_write_time: read_i64(buffer, 24)?,
            change_time: read_i64(buffer, 32)?,
            end_of_file: read_i64(buffer, 40)?,
            allocation_size: read_i64(buffer, 48)?,
            file_attributes: read_file_attributes(buffer, 56)?,
            file_name: read_file_name(buffer, FILE_DIRECTORY_INFORMATION_NAME_OFFSET, 60)?,
        }
        .into()),
        FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION => Ok(FileFullDirectoryInformation {
            next_entry_offset: 0,
            file_index: read_u32(buffer, 4)?,
            creation_time: read_i64(buffer, 8)?,
            last_access_time: read_i64(buffer, 16)?,
            last_write_time: read_i64(buffer, 24)?,
            change_time: read_i64(buffer, 32)?,
            end_of_file: read_i64(buffer, 40)?,
            allocation_size: read_i64(buffer, 48)?,
            file_attributes: read_file_attributes(buffer, 56)?,
            ea_size: read_u32(buffer, 64)?,
            file_name: read_file_name(buffer, FILE_FULL_DIRECTORY_INFORMATION_NAME_OFFSET, 60)?,
        }
        .into()),
        FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION => {
            let short_name_length = read_u8(buffer, 68)?;
            if usize::from(short_name_length) > SHORT_NAME_SIZE {
                return Err(NtStatus::UNSUCCESSFUL);
            }
            let mut short_name = [0; SHORT_NAME_SIZE];
            short_name.copy_from_slice(read_bytes(
                buffer,
                FILE_BOTH_DIRECTORY_INFORMATION_SHORT_NAME_OFFSET,
                SHORT_NAME_SIZE,
            )?);

            Ok(FileBothDirectoryInformation {
                next_entry_offset: 0,
                file_index: read_u32(buffer, 4)?,
                creation_time: read_i64(buffer, 8)?,
                last_access_time: read_i64(buffer, 16)?,
                last_write_time: read_i64(buffer, 24)?,
                change_time: read_i64(buffer, 32)?,
                end_of_file: read_i64(buffer, 40)?,
                allocation_size: read_i64(buffer, 48)?,
                file_attributes: read_file_attributes(buffer, 56)?,
                ea_size: read_u32(buffer, 64)?,
                short_name_length: i8::try_from(short_name_length).map_err(|_| NtStatus::UNSUCCESSFUL)?,
                short_name,
                file_name: read_file_name(buffer, FILE_BOTH_DIRECTORY_INFORMATION_NAME_OFFSET, 60)?,
            }
            .into())
        }
        FileInformationClassLevel::FILE_NAMES_INFORMATION => Ok(FileNamesInformation {
            next_entry_offset: 0,
            file_index: read_u32(buffer, 4)?,
            file_name: read_file_name(buffer, FILE_NAMES_INFORMATION_NAME_OFFSET, 8)?,
        }
        .into()),
        _ => Err(NtStatus::NOT_SUPPORTED),
    }
}

struct DirectoryQueryPath {
    directory: RelativePath,
    pattern: Vec<u16>,
}

fn query_path(path: &str) -> Result<DirectoryQueryPath, NtStatus> {
    if path.starts_with('/') || path.starts_with(r"\\") {
        return Err(NtStatus::OBJECT_NAME_INVALID);
    }

    let path = path.strip_prefix('\\').unwrap_or(path);
    let (directory, pattern) = if path.is_empty() {
        ("", "*")
    } else {
        path.rsplit_once('\\')
            .map_or(("", path), |(directory, pattern)| (directory, pattern))
    };
    let directory = RelativePath::parse(directory).map_err(from_path_policy)?;
    if pattern.is_empty()
        || matches!(pattern, "." | "..")
        || pattern
            .chars()
            .any(|character| character <= '\u{1F}' || matches!(character, ':' | '/' | '"' | '<' | '>' | '|'))
    {
        return Err(NtStatus::OBJECT_NAME_INVALID);
    }

    let pattern = pattern.encode_utf16().collect::<Vec<_>>();
    if pattern.len() > 255 {
        return Err(NtStatus::OBJECT_NAME_INVALID);
    }

    Ok(DirectoryQueryPath { directory, pattern })
}

fn open_query_directory(
    backend: &WindowsRdpdrBackend,
    device_id: u32,
    directory: &RelativePath,
) -> Result<Option<FileHandle>, NtStatus> {
    let root = backend.roots.get(&device_id).ok_or(NtStatus::INVALID_PARAMETER)?;
    root.root
        .open_relative_directory(directory)
        .map_err(from_open_directory)
        .map(|directory| directory.map(DirectoryHandle::into_file_handle))
}

fn read_file_attributes(buffer: &[u8], offset: usize) -> Result<ironrdp_rdpdr::pdu::efs::FileAttributes, NtStatus> {
    Ok(ironrdp_rdpdr::pdu::efs::FileAttributes::from_bits_retain(read_u32(
        buffer, offset,
    )?))
}

fn read_file_name(buffer: &[u8], offset: usize, length_offset: usize) -> Result<String, NtStatus> {
    let length = usize::try_from(read_u32(buffer, length_offset)?).map_err(|_| NtStatus::UNSUCCESSFUL)?;
    if length % size_of::<u16>() != 0 {
        return Err(NtStatus::UNSUCCESSFUL);
    }

    let bytes = read_bytes(buffer, offset, length)?;
    let units = bytes
        .chunks_exact(size_of::<u16>())
        .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
        .collect::<Vec<_>>();

    String::from_utf16(&units).map_err(|_| NtStatus::UNSUCCESSFUL)
}

fn read_i64(buffer: &[u8], offset: usize) -> Result<i64, NtStatus> {
    Ok(i64::from_le_bytes(read_array(buffer, offset)?))
}

fn read_u32(buffer: &[u8], offset: usize) -> Result<u32, NtStatus> {
    Ok(u32::from_le_bytes(read_array(buffer, offset)?))
}

fn read_u8(buffer: &[u8], offset: usize) -> Result<u8, NtStatus> {
    buffer.get(offset).copied().ok_or(NtStatus::UNSUCCESSFUL)
}

fn read_array<const N: usize>(buffer: &[u8], offset: usize) -> Result<[u8; N], NtStatus> {
    read_bytes(buffer, offset, N)?
        .try_into()
        .map_err(|_| NtStatus::UNSUCCESSFUL)
}

fn read_bytes(buffer: &[u8], offset: usize, length: usize) -> Result<&[u8], NtStatus> {
    let end = offset.checked_add(length).ok_or(NtStatus::UNSUCCESSFUL)?;
    buffer.get(offset..end).ok_or(NtStatus::UNSUCCESSFUL)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    use ironrdp_rdpdr::RdpdrBackend;
    use ironrdp_rdpdr::pdu::efs::{
        CreateDisposition, CreateOptions, DesiredAccess, DeviceCreateRequest, DeviceIoRequest, FileAttributes,
        MajorFunction, MinorFunction, ServerDriveIoRequest, SharedAccess,
    };
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::windows::factory::RedirectedDrive;
    use crate::windows::file::create_inner;

    #[test]
    fn parses_all_directory_information_classes() {
        let name = "file.txt";
        let name_bytes = name.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>();

        for (information_class, name_offset) in [
            (
                FileInformationClassLevel::FILE_DIRECTORY_INFORMATION,
                FILE_DIRECTORY_INFORMATION_NAME_OFFSET,
            ),
            (
                FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION,
                FILE_FULL_DIRECTORY_INFORMATION_NAME_OFFSET,
            ),
            (
                FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION,
                FILE_BOTH_DIRECTORY_INFORMATION_NAME_OFFSET,
            ),
            (
                FileInformationClassLevel::FILE_NAMES_INFORMATION,
                FILE_NAMES_INFORMATION_NAME_OFFSET,
            ),
        ] {
            let mut buffer = vec![0; name_offset + name_bytes.len()];
            buffer[4..8].copy_from_slice(&7u32.to_le_bytes());
            buffer[8..12].copy_from_slice(&(u32::try_from(name_bytes.len()).expect("name length fits")).to_le_bytes());

            if information_class != FileInformationClassLevel::FILE_NAMES_INFORMATION {
                buffer[56..60].copy_from_slice(&FileAttributes::FILE_ATTRIBUTE_ARCHIVE.bits().to_le_bytes());
                buffer[60..64]
                    .copy_from_slice(&(u32::try_from(name_bytes.len()).expect("name length fits")).to_le_bytes());
            }
            if information_class == FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION
                || information_class == FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION
            {
                buffer[64..68].copy_from_slice(&5u32.to_le_bytes());
            }
            if information_class == FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION {
                buffer[68] = 0;
            }
            buffer[name_offset..].copy_from_slice(&name_bytes);

            let information = parse_information(information_class, &buffer).expect("valid native directory response");
            assert_eq!(file_name(&information), name);
        }
    }

    #[test]
    fn query_paths_select_a_confined_directory_and_pattern() {
        let wildcard = "*".encode_utf16().collect::<Vec<_>>();
        let query = query_path(r"\*").expect("valid wildcard");
        assert!(query.directory.components().next().is_none());
        assert_eq!(query.pattern, wildcard);

        let query = query_path("").expect("empty pattern defaults to wildcard");
        assert!(query.directory.components().next().is_none());
        assert_eq!(query.pattern, "*".encode_utf16().collect::<Vec<_>>());

        let query = query_path(r"\folder\*").expect("directory-qualified wildcard");
        assert_eq!(query.directory.components().collect::<Vec<_>>(), ["folder"]);
        assert_eq!(query.pattern, "*".encode_utf16().collect::<Vec<_>>());
        assert!(matches!(query_path(r"\..\*"), Err(NtStatus::OBJECT_NAME_INVALID)));
        assert!(matches!(
            query_path("outside/anything"),
            Err(NtStatus::OBJECT_NAME_INVALID)
        ));
    }

    #[test]
    fn malformed_native_directory_entries_are_rejected() {
        assert_eq!(
            parse_information(FileInformationClassLevel::FILE_NAMES_INFORMATION, &[]),
            Err(NtStatus::UNSUCCESSFUL)
        );
    }

    #[test]
    fn enumerates_an_opened_directory_for_every_supported_information_class() {
        let temporary_directory = TemporaryDirectory::create();
        std::fs::write(temporary_directory.0.join("example.txt"), b"content").expect("create directory entry");
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
        let mut backend = WindowsRdpdrBackend::new(vec![drive]).expect("open redirected root");
        let (root_file_id, _) =
            create_inner(&mut backend, &directory_create_request(r"\")).expect("open redirected root");
        query_information_inner(
            &mut backend,
            &query_request(root_file_id, FileInformationClassLevel::FILE_NAMES_INFORMATION),
        )
        .expect("enumerate redirected root through an independent synchronous handle");

        let path = format!(r"\{}", relative_path.display());
        let (file_id, _) = create_inner(&mut backend, &directory_create_request(&path)).expect("open test directory");

        for information_class in [
            FileInformationClassLevel::FILE_DIRECTORY_INFORMATION,
            FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION,
            FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION,
            FileInformationClassLevel::FILE_NAMES_INFORMATION,
        ] {
            let mut request = query_request(file_id, information_class);
            request.path = format!(r"\{}\*", relative_path.display());
            let information = query_information_inner(&mut backend, &request).expect("enumerate opened directory");
            assert!(!file_name(&information).is_empty());
        }

        let mut continuation_request = query_request(file_id, FileInformationClassLevel::FILE_NAMES_INFORMATION);
        continuation_request.path = format!(r"\{}\*", relative_path.display());
        continuation_request.initial_query = 0;
        continuation_request.path = r"\ignored\by-continuation".to_owned();
        let mut returned_entries = 0;
        loop {
            match query_information_inner(&mut backend, &continuation_request) {
                Ok(_) => returned_entries += 1,
                Err(NtStatus::NO_MORE_FILES) => break,
                Err(status) => panic!("unexpected continuation status: {status:?}"),
            }
            assert!(returned_entries < 16, "bounded directory must reach its end");
        }
        assert!(returned_entries > 0);
    }

    #[test]
    fn initial_query_enumerates_the_directory_named_by_path() {
        let temporary_directory = TemporaryDirectory::create();
        let target_directory = temporary_directory.0.join("query-target");
        let target_file_name = "only-in-query-target.txt";
        std::fs::create_dir(&target_directory).expect("create query target directory");
        std::fs::write(target_directory.join(target_file_name), b"content").expect("create query target entry");

        let root_path = temporary_directory
            .0
            .ancestors()
            .last()
            .expect("temporary directory has a volume root")
            .to_owned();
        let relative_target = target_directory
            .strip_prefix(&root_path)
            .expect("query target is below the redirected root");
        let drive = RedirectedDrive::new(1, "Test", root_path, false).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::new(vec![drive]).expect("open redirected root");
        let (root_file_id, _) =
            create_inner(&mut backend, &directory_create_request(r"\")).expect("open redirected root");
        let mut request = query_request(root_file_id, FileInformationClassLevel::FILE_NAMES_INFORMATION);
        request.path = format!(r"\{}\*", relative_target.display());

        let mut found_target_entry = false;
        for _ in 0..16 {
            match query_information_inner(&mut backend, &request) {
                Ok(information) => {
                    if file_name(&information) == target_file_name {
                        found_target_entry = true;
                        break;
                    }
                    request.initial_query = 0;
                }
                Err(NtStatus::NO_MORE_FILES) => break,
                Err(status) => panic!("unexpected directory query status: {status:?}"),
            }
        }

        assert!(
            found_target_entry,
            "the initial query must enumerate its requested directory"
        );
    }

    #[test]
    fn backend_delivers_directory_change_completion() {
        let (temporary_directory, mut backend, file_id) = backend_with_open_temporary_directory();
        let request = directory_notification_request(file_id);

        assert!(
            RdpdrBackend::handle_drive_io_request(
                &mut backend,
                ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(request),
            )
            .expect("schedule directory notification")
            .is_empty()
        );

        let mut completion = None;
        for index in 0..20 {
            std::fs::write(temporary_directory.0.join(format!("change-{index}")), b"change")
                .expect("create watched file");
            for _ in 0..4 {
                std::thread::sleep(Duration::from_millis(25));
                let mut messages =
                    RdpdrBackend::poll_deferred_messages(&mut backend).expect("poll directory notification");
                if let Some(message) = messages.pop() {
                    assert!(messages.is_empty(), "one watch has one completion");
                    completion = Some(message);
                    break;
                }
            }
            if completion.is_some() {
                break;
            }
        }

        let response = completion
            .expect("receive deferred directory notification")
            .encode_unframed_pdu()
            .expect("directory notification is encodable");
        assert_eq!(
            u32::from_le_bytes(response[12..16].try_into().expect("response status is present")),
            u32::from(NtStatus::SUCCESS)
        );
        assert!(
            u32::from_le_bytes(response[16..20].try_into().expect("buffer length is present")) >= 12,
            "a directory notification contains FILE_NOTIFY_INFORMATION"
        );
        assert_eq!(
            u32::from_le_bytes(response[24..28].try_into().expect("action is present")),
            1,
            "the created file produces FILE_ACTION_ADDED"
        );
    }

    #[test]
    fn nonzero_watch_tree_values_enable_subtree_notifications() {
        let (_temporary_directory, mut backend, file_id) = backend_with_open_temporary_directory();
        let mut request = directory_notification_request(file_id);
        request.watch_tree = 2;

        assert!(
            RdpdrBackend::handle_drive_io_request(
                &mut backend,
                ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(request),
            )
            .expect("schedule subtree directory notification")
            .is_empty()
        );

        let messages = RdpdrBackend::remove_drive(&mut backend, 1).expect("remove watched drive");
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn removing_dynamic_drive_cancels_directory_watch() {
        let (_temporary_directory, mut backend, file_id) = backend_with_open_temporary_directory();
        let request = directory_notification_request(file_id);

        assert!(
            RdpdrBackend::handle_drive_io_request(
                &mut backend,
                ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(request),
            )
            .expect("schedule directory notification")
            .is_empty()
        );

        let messages = RdpdrBackend::remove_drive(&mut backend, 1).expect("remove active dynamic drive");
        assert_eq!(messages.len(), 1);
        let response = messages
            .into_iter()
            .next()
            .expect("removed drive returns a cancellation completion")
            .encode_unframed_pdu()
            .expect("directory cancellation is encodable");
        assert_eq!(
            u32::from_le_bytes(response[12..16].try_into().expect("response status is present")),
            u32::from(NtStatus::CANCELLED)
        );
        assert_eq!(
            u32::from_le_bytes(response[16..20].try_into().expect("buffer length is present")),
            0
        );
        assert!(
            RdpdrBackend::poll_deferred_messages(&mut backend)
                .expect("poll removed drive")
                .is_empty()
        );
    }

    fn backend_with_open_temporary_directory() -> (TemporaryDirectory, WindowsRdpdrBackend, u32) {
        let temporary_directory = TemporaryDirectory::create();
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
        let mut backend = WindowsRdpdrBackend::new(vec![drive]).expect("open redirected root");
        let (file_id, _) = create_inner(
            &mut backend,
            &directory_create_request(&format!(r"\{}", relative_path.display())),
        )
        .expect("open test directory");
        (temporary_directory, backend, file_id)
    }

    fn directory_notification_request(file_id: u32) -> ServerDriveNotifyChangeDirectoryRequest {
        ServerDriveNotifyChangeDirectoryRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id,
                completion_id: 3,
                major_function: MajorFunction::DirectoryControl,
                minor_function: MinorFunction::IRP_MN_NOTIFY_CHANGE_DIRECTORY,
            },
            watch_tree: 0,
            completion_filter: 1,
        }
    }

    fn file_name(information: &FileInformationClass) -> &str {
        match information {
            FileInformationClass::BothDirectory(information) => &information.file_name,
            FileInformationClass::FullDirectory(information) => &information.file_name,
            FileInformationClass::Names(information) => &information.file_name,
            FileInformationClass::Directory(information) => &information.file_name,
            _ => panic!("expected a directory information class"),
        }
    }

    fn directory_create_request(path: &str) -> DeviceCreateRequest {
        DeviceCreateRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id: 0,
                completion_id: 1,
                major_function: MajorFunction::Create,
                minor_function: MinorFunction::from(0),
            },
            desired_access: DesiredAccess::from_bits_retain(0x0010_0001),
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::FILE_SHARE_READ
                | SharedAccess::FILE_SHARE_WRITE
                | SharedAccess::FILE_SHARE_DELETE,
            create_disposition: CreateDisposition::FILE_OPEN,
            create_options: CreateOptions::from_bits_retain(0x0000_0021),
            path: path.to_owned(),
        }
    }

    fn query_request(file_id: u32, file_info_class_lvl: FileInformationClassLevel) -> ServerDriveQueryDirectoryRequest {
        ServerDriveQueryDirectoryRequest {
            device_io_request: DeviceIoRequest {
                device_id: 1,
                file_id,
                completion_id: 2,
                major_function: MajorFunction::DirectoryControl,
                minor_function: MinorFunction::IRP_MN_QUERY_DIRECTORY,
            },
            file_info_class_lvl,
            initial_query: 1,
            path: r"\*".to_owned(),
        }
    }

    struct TemporaryDirectory(PathBuf);

    impl TemporaryDirectory {
        fn create() -> Self {
            let unique_name = format!(
                "ironrdp-rdpdr-native-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock is after Unix epoch")
                    .as_nanos()
            );
            let path = std::env::temp_dir().join(unique_name);
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
