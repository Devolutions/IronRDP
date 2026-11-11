use super::*;

/// Bounds each write allocation and the largest file offset accepted from the server.
pub(super) const MAX_WRITE_LENGTH: usize = 1_024 * 1_024;
const MAX_FILE_LENGTH: u64 = 1_024 * 1_024 * 1_024;

pub(super) fn handle(backend: &mut NixRdpdrBackend, request: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
    let response = match request {
        ServerDriveIoRequest::ServerCreateDriveRequest(request) => return create(backend, request),
        ServerDriveIoRequest::DeviceWriteRequest(request) => {
            let status = writable(
                backend,
                request.device_io_request.device_id,
                request.device_io_request.file_id,
            )
            .and_then(|_| {
                if MAX_WRITE_LENGTH < request.write_data.len()
                    || request
                        .offset
                        .checked_add(u64::try_from(request.write_data.len()).unwrap_or(u64::MAX))
                        .is_none_or(|end| MAX_FILE_LENGTH < end)
                {
                    Err(NtStatus::INVALID_PARAMETER)
                } else {
                    Ok(())
                }
            });
            return match status {
                Ok(()) => write_device(backend, request),
                Err(status) => Ok(vec![SvcMessage::from(write_response(
                    request.device_io_request,
                    0,
                    status,
                ))]),
            };
        }
        ServerDriveIoRequest::DeviceCloseRequest(request) => {
            if file(
                backend,
                request.device_io_request.device_id,
                request.device_io_request.file_id,
            )
            .is_ok()
            {
                return close_device(backend, request);
            }
            RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
                device_io_response: DeviceIoResponse::new(request.device_io_request, NtStatus::NO_SUCH_FILE),
            })
        }
        ServerDriveIoRequest::DeviceFlushBuffersRequest(request) => {
            let status = file(
                backend,
                request.device_io_request.device_id,
                request.device_io_request.file_id,
            )
            .and_then(|file| {
                file.file
                    .sync_all()
                    .map_err(|error| status_from_errno(io_to_errno(error)))
            })
            .err()
            .unwrap_or(NtStatus::SUCCESS);
            RdpdrPdu::DeviceFlushBuffersResponse(DeviceFlushBuffersResponse {
                device_io_response: DeviceIoResponse::new(request.device_io_request, status),
            })
        }
        ServerDriveIoRequest::ServerDriveQueryInformationRequest(request) => {
            if file(
                backend,
                request.device_io_request.device_id,
                request.device_io_request.file_id,
            )
            .is_ok()
            {
                return query_information(backend, request);
            }
            RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                device_io_response: DeviceIoResponse::new(request.device_io_request, NtStatus::NO_SUCH_FILE),
                buffer: None,
            })
        }
        ServerDriveIoRequest::ServerDriveSetInformationRequest(request) => {
            let status = set(backend, &request).err().unwrap_or(NtStatus::SUCCESS);
            RdpdrPdu::ClientDriveSetInformationResponse(
                ClientDriveSetInformationResponse::new(&request, status).map_err(|error| encode_err!(error))?,
            )
        }
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
        ServerDriveIoRequest::DeviceControlRequest(request) => RdpdrPdu::DeviceControlResponse(DeviceControlResponse {
            device_io_reply: DeviceIoResponse::new(request.header, NtStatus::ACCESS_DENIED),
            output_buffer: None,
        }),
        ServerDriveIoRequest::ServerDriveLockControlRequest(request) => {
            return Ok(vec![locks::response(
                request.device_io_request,
                NtStatus::ACCESS_DENIED,
            )]);
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

fn flat_path(name: &str) -> Result<RelativePath, NtStatus> {
    // A single leading backslash is the RDPDR root-relative spelling, not a host absolute path.
    let component = name.strip_prefix('\\').unwrap_or(name);
    if component.is_empty() || component.contains('\\') {
        return Err(NtStatus::OBJECT_NAME_INVALID);
    }
    RelativePath::parse(name).map_err(|_| NtStatus::OBJECT_NAME_INVALID)
}

fn create(backend: &mut NixRdpdrBackend, request: DeviceCreateRequest) -> PduResult<Vec<SvcMessage>> {
    let file_id = backend.next_file_id();
    let opened = (|| {
        let path = flat_path(&request.path)?;
        let allowed_access = DesiredAccess::GENERIC_WRITE
            | DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE
            | DesiredAccess::FILE_APPEND_DATA_OR_FILE_ADD_SUBDIRECTORY
            | DesiredAccess::FILE_WRITE_ATTRIBUTES
            | DesiredAccess::FILE_WRITE_EA
            | DesiredAccess::FILE_READ_ATTRIBUTES
            | DesiredAccess::READ_CONTROL
            | DesiredAccess::DELETE
            | DesiredAccess::SYNCHRONIZE;
        if request.create_disposition != CreateDisposition::FILE_CREATE
            || request.desired_access.bits() & !allowed_access.bits() != 0
            || !request
                .desired_access
                .intersects(DesiredAccess::GENERIC_WRITE | DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE)
            || request
                .create_options
                .intersects(CreateOptions::FILE_DIRECTORY_FILE | CreateOptions::FILE_DELETE_ON_CLOSE)
        {
            return Err(NtStatus::ACCESS_DENIED);
        }
        if backend.max_open_files <= backend.open_files.len() {
            return Err(NtStatus::INSUFFICIENT_RESOURCES);
        }
        let download_root = backend.download_root.as_ref().expect("download mode");
        download_root.reserve_file().map_err(status_from_errno)?;
        let file = match download_root
            .directory
            .create_exclusive_file(path.file_name().expect("flat path has a name"))
        {
            Ok(file) => file,
            Err(error) => {
                download_root.release_file();
                return Err(status_from_errno(error));
            }
        };
        let identity = match fstat(file.as_fd()) {
            Ok(stat) => FileIdentity::of(&stat),
            Err(error) => {
                download_root.retain_created_file(file, path);
                return Err(status_from_errno(error));
            }
        };
        Ok(OpenFile {
            file: file.into(),
            device_id: request.device_io_request.device_id,
            path,
            identity,
            ancestors: Vec::new(),
            may_delete: request.desired_access.contains(DesiredAccess::DELETE),
            delete_on_close: false,
            listing: None,
            published: false,
        })
    })();
    match opened {
        Ok(file) => {
            backend.open_files.insert(file_id, file);
            make_create_drive_resp(request.device_io_request, request.create_disposition, file_id)
        }
        Err(status) => Ok(vec![SvcMessage::from(RdpdrPdu::DeviceCreateResponse(
            DeviceCreateResponse {
                device_io_reply: DeviceIoResponse::new(request.device_io_request, status),
                file_id,
                information: Information::empty(),
            },
        ))]),
    }
}

fn file(backend: &NixRdpdrBackend, device_id: u32, file_id: u32) -> Result<&OpenFile, NtStatus> {
    backend
        .open_files
        .get(&file_id)
        .filter(|file| file.device_id == device_id)
        .ok_or(NtStatus::NO_SUCH_FILE)
}

fn writable(backend: &NixRdpdrBackend, device_id: u32, file_id: u32) -> Result<(), NtStatus> {
    let file = file(backend, device_id, file_id)?;
    if file.published || backend.pending_deletes.contains_key(&file.identity) {
        return Err(NtStatus::ACCESS_DENIED);
    }
    let stat = fstat(file.file.as_fd()).map_err(status_from_errno)?;
    if stat.st_nlink != 1 {
        return Err(NtStatus::ACCESS_DENIED);
    }
    Ok(())
}

fn set(backend: &mut NixRdpdrBackend, request: &ServerDriveSetInformationRequest) -> Result<(), NtStatus> {
    let file_id = request.device_io_request.file_id;
    writable(backend, request.device_io_request.device_id, file_id)?;
    let file = backend.open_files.get_mut(&file_id).ok_or(NtStatus::NO_SUCH_FILE)?;
    if !file.may_delete {
        return Err(NtStatus::ACCESS_DENIED);
    }
    match &request.set_buffer {
        FileInformationClass::Rename(info) => {
            if info.replace_if_exists != Boolean::False {
                return Err(NtStatus::ACCESS_DENIED);
            }
            let to = flat_path(&info.file_name)?;
            let root = &backend.download_root.as_ref().expect("download mode").directory;
            let (parent, from) = file.path.open_parent(root).map_err(status_from_errno)?;
            verify_owned(parent.as_fd(), from, file.identity).map_err(status_from_errno)?;
            let to_name = to.file_name().ok_or(NtStatus::OBJECT_NAME_INVALID)?;
            rename_exclusive(parent.as_fd(), from, to_name).map_err(status_from_errno)?;
            file.path = to;
            file.published = true;
            Ok(())
        }
        FileInformationClass::Disposition(info) => {
            if info.delete_pending != 0 {
                backend.pending_deletes.insert(file.identity, file.path.clone());
            }
            Ok(())
        }
        _ => Err(NtStatus::ACCESS_DENIED),
    }
}

fn verify_owned(parent: BorrowedFd<'_>, name: &str, identity: FileIdentity) -> nix::Result<()> {
    // UNTESTED: the caller keeps the download directory inaccessible to untrusted local writers.
    // POSIX cannot atomically tie unlink or a source rename to an inode identity, so a local writer could replace a validated name.
    // When changing directory configuration, verify that it requires a dedicated, trusted directory.
    let stat = fstatat(parent, name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
    if FileIdentity::of(&stat) != identity || SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT != SFlag::S_IFREG
    {
        return Err(Errno::ESTALE);
    }
    Ok(())
}

pub(super) fn remove_owned(root: &Root, path: &RelativePath, identity: FileIdentity) -> nix::Result<()> {
    let (parent, name) = path.open_parent(root)?;
    verify_owned(parent.as_fd(), name, identity)?;
    unlinkat(parent.as_fd(), name, UnlinkatFlags::NoRemoveDir)
}

#[cfg(target_os = "linux")]
fn rename_exclusive(parent: BorrowedFd<'_>, from: &str, to: &str) -> nix::Result<()> {
    nix::fcntl::renameat2(parent, from, parent, to, nix::fcntl::RenameFlags::RENAME_NOREPLACE)
}

#[cfg(target_os = "macos")]
fn rename_exclusive(parent: BorrowedFd<'_>, from: &str, to: &str) -> nix::Result<()> {
    use std::os::fd::AsRawFd as _;

    let from = std::ffi::CString::new(from).map_err(|_| Errno::EINVAL)?;
    let to = std::ffi::CString::new(to).map_err(|_| Errno::EINVAL)?;
    // SAFETY: both names are live NUL-terminated strings and parent remains a live directory descriptor.
    let result = unsafe {
        nix::libc::renameatx_np(
            parent.as_raw_fd(),
            from.as_ptr(),
            parent.as_raw_fd(),
            to.as_ptr(),
            nix::libc::RENAME_EXCL,
        )
    };
    Errno::result(result).map(|_| ())
}
