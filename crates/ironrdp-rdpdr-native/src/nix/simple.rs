use ironrdp_pdu::PduResult;
use ironrdp_svc::{impl_as_any, SvcMessage};
use libc::ftruncate;
use nix::libc::{closedir, opendir, readdir, DIR};

use ironrdp_rdpdr::pdu::efs::*;
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::RdpdrBackend;
use std::ffi::{CStr, CString};
use std::io::Read;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::fs::MetadataExt;

#[derive(Debug)]
pub struct SimpleRdpdrBackend {
    pub file_id: u32,
    pub file_base: String,
    pub file_map: std::collections::HashMap<u32, std::fs::File>,
    pub file_path_map: std::collections::HashMap<u32, String>,
    pub file_dir_map: std::collections::HashMap<u32, DirBox>,
}

#[derive(Debug)]
pub struct DirBox {
    pub dir: *mut DIR,
}
// SAFETY: it is safe to send *DIR to a different thread, and it is only accessed by one thread at one time.
unsafe impl Send for DirBox {}

impl_as_any!(SimpleRdpdrBackend);

impl RdpdrBackend for SimpleRdpdrBackend {
    fn handle_server_device_announce_response(&mut self, _pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        Ok(())
    }
    fn handle_scard_call(&mut self, _req: DeviceControlRequest<ScardIoCtlCode>, _call: ScardCall) -> PduResult<()> {
        Ok(())
    }
    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        debug!("handle_drive_io_request:{:?}", req);
        match req {
            ServerDriveIoRequest::DeviceWriteRequest(req_inner) => write_device(self, req_inner),
            ServerDriveIoRequest::ServerCreateDriveRequest(req_inner) => create_drive(self, req_inner),
            ServerDriveIoRequest::DeviceReadRequest(req_inner) => read_device(self, req_inner),
            ServerDriveIoRequest::DeviceCloseRequest(req_inner) => close_device(self, req_inner),
            ServerDriveIoRequest::ServerDriveNotifyChangeDirectoryRequest(_) => {
                // TODO
                Ok(Vec::new())
            }
            ServerDriveIoRequest::ServerDriveQueryDirectoryRequest(req_inner) => query_directory(self, req_inner),
            ServerDriveIoRequest::ServerDriveQueryInformationRequest(req_inner) => query_information(self, req_inner),
            ServerDriveIoRequest::ServerDriveQueryVolumeInformationRequest(req_inner) => {
                query_volume_information(self, req_inner)
            }
            ServerDriveIoRequest::ServerDriveSetInformationRequest(req_inner) => set_information(self, req_inner),
            ServerDriveIoRequest::DeviceControlRequest(req_inner) => Ok(vec![SvcMessage::from(
                RdpdrPdu::DeviceControlResponse(DeviceControlResponse {
                    device_io_reply: DeviceIoResponse::new(req_inner.header, NtStatus::SUCCESS),
                    output_buffer: None,
                }),
            )]),
            ServerDriveIoRequest::ServerDriveLockControlRequest(_) => {
                // TODO
                Ok(Vec::new())
            }
        }
    }
}

pub(crate) fn write_device(
    backend: &mut SimpleRdpdrBackend,
    req_inner: DeviceWriteRequest,
) -> PduResult<Vec<SvcMessage>> {
    return process_dependent_file(
        backend,
        req_inner.device_io_request,
        |request| {
            let res = RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
                device_io_reply: DeviceIoResponse::new(request, NtStatus::NO_SUCH_FILE),
                length: 0u32,
            });
            Ok(vec![SvcMessage::from(res)])
        },
        |file, request| match write_inner(file, req_inner.offset, &req_inner.write_data) {
            Ok(length) => {
                if length == req_inner.write_data.len() {
                    Ok(vec![SvcMessage::from(RdpdrPdu::DeviceWriteResponse(
                        DeviceWriteResponse {
                            device_io_reply: DeviceIoResponse::new(request, NtStatus::SUCCESS),
                            length: u32::try_from(req_inner.write_data.len()).unwrap(),
                        },
                    ))])
                } else {
                    warn!(
                        "Written content len:{} is not equal to {}",
                        length,
                        req_inner.write_data.len()
                    );
                    let res = RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
                        device_io_reply: DeviceIoResponse::new(request, NtStatus::UNSUCCESSFUL),
                        length: 0u32,
                    });
                    Ok(vec![SvcMessage::from(res)])
                }
            }
            Err(error) => {
                warn!(%error, "Write error");
                let res = RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
                    device_io_reply: DeviceIoResponse::new(request, NtStatus::UNSUCCESSFUL),
                    length: 0u32,
                });
                Ok(vec![SvcMessage::from(res)])
            }
        },
    );
    fn write_inner(file: &mut std::fs::File, offset: u64, write_data: &[u8]) -> std::io::Result<usize> {
        let sf = SeekFrom::Start(offset);
        file.seek(sf)?;
        let length = file.write(write_data)?;
        file.flush()?;
        Ok(length)
    }
}
pub(crate) fn read_device(
    backend: &mut SimpleRdpdrBackend,
    req_inner: DeviceReadRequest,
) -> PduResult<Vec<SvcMessage>> {
    return process_dependent_file(
        backend,
        req_inner.device_io_request,
        |request| {
            let res = RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
                device_io_reply: DeviceIoResponse::new(request, NtStatus::NO_SUCH_FILE),
                read_data: Vec::new(),
            });
            Ok(vec![SvcMessage::from(res)])
        },
        |file, request| match read_inner(file, req_inner.offset, usize::try_from(req_inner.length).unwrap()) {
            Ok(buf) => {
                let res = RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
                    device_io_reply: DeviceIoResponse::new(request, NtStatus::SUCCESS),
                    read_data: buf,
                });
                Ok(vec![SvcMessage::from(res)])
            }
            Err(error) => {
                warn!(?error, "Read error");
                let res = RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
                    device_io_reply: DeviceIoResponse::new(request, NtStatus::UNSUCCESSFUL),
                    read_data: Vec::new(),
                });
                Ok(vec![SvcMessage::from(res)])
            }
        },
    );
    fn read_inner(file: &mut std::fs::File, offset: u64, length: usize) -> std::io::Result<Vec<u8>> {
        let sf = SeekFrom::Start(offset);
        file.seek(sf)?;
        let mut buf = vec![0; length];

        let length = file.read(&mut buf)?;
        buf.resize(length, 0u8);
        Ok(buf)
    }
}
pub(crate) fn close_device(
    backend: &mut SimpleRdpdrBackend,
    req_inner: DeviceCloseRequest,
) -> PduResult<Vec<SvcMessage>> {
    backend.file_map.remove(&req_inner.device_io_request.file_id);
    backend.file_path_map.remove(&req_inner.device_io_request.file_id);
    if let Some(dir_box) = backend.file_dir_map.remove(&req_inner.device_io_request.file_id) {
        // SAFETY: it is safe to close dir when we close device
        unsafe {
            closedir(dir_box.dir);
        }
    }
    let res = RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
        device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
    });
    Ok(vec![SvcMessage::from(res)])
}

pub(crate) fn query_information(
    backend: &mut SimpleRdpdrBackend,
    req_inner: ServerDriveQueryInformationRequest,
) -> PduResult<Vec<SvcMessage>> {
    match backend.file_map.get(&req_inner.device_io_request.file_id) {
        Some(file) => match file.metadata() {
            Ok(meta) => {
                let path = backend
                    .file_path_map
                    .get(&req_inner.device_io_request.file_id)
                    .cloned()
                    .unwrap_or_default();
                let name_index = match path.rfind('/') {
                    // in fact, index only needs to be different for existing requests
                    #[allow(clippy::arithmetic_side_effects)]
                    Some(index) => index + 1,
                    None => 0,
                };
                let name = &path[name_index..];
                let file_attribute = get_file_attributes(&meta, name);
                if FileInformationClassLevel::FILE_BASIC_INFORMATION == req_inner.file_info_class_lvl {
                    let basic_info = FileBasicInformation {
                        creation_time: transform_to_filetime(meta.ctime()),
                        last_access_time: transform_to_filetime(meta.atime()),
                        last_write_time: transform_to_filetime(meta.mtime()),
                        change_time: transform_to_filetime(meta.ctime()),
                        file_attributes: file_attribute,
                    };
                    let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                        device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                        buffer: Some(FileInformationClass::Basic(basic_info)),
                    });
                    Ok(vec![SvcMessage::from(res)])
                } else if FileInformationClassLevel::FILE_STANDARD_INFORMATION == req_inner.file_info_class_lvl {
                    let dir = if meta.is_dir() { Boolean::True } else { Boolean::False };
                    let standard_info = FileStandardInformation {
                        allocation_size: i64::try_from(meta.size()).unwrap(),
                        end_of_file: i64::try_from(meta.size()).unwrap(),
                        number_of_links: u32::try_from(meta.nlink()).unwrap(),
                        delete_pending: Boolean::False,
                        directory: dir,
                    };
                    let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                        device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                        buffer: Some(FileInformationClass::Standard(standard_info)),
                    });
                    Ok(vec![SvcMessage::from(res)])
                } else if FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION == req_inner.file_info_class_lvl {
                    let info = FileAttributeTagInformation {
                        file_attributes: file_attribute,
                        reparse_tag: 0,
                    };
                    let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                        device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                        buffer: Some(FileInformationClass::AttributeTag(info)),
                    });
                    Ok(vec![SvcMessage::from(res)])
                } else {
                    warn!("unsupported file class");
                    let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                        device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::UNSUCCESSFUL),
                        buffer: None,
                    });
                    Ok(vec![SvcMessage::from(res)])
                }
            }
            Err(error) => {
                warn!(?error, "Get file metadata error");
                let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                    device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::UNSUCCESSFUL),
                    buffer: None,
                });
                Ok(vec![SvcMessage::from(res)])
            }
        },
        None => {
            warn!("no such file");
            let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::NO_SUCH_FILE),
                buffer: None,
            });
            Ok(vec![SvcMessage::from(res)])
        }
    }
}

pub(crate) fn query_volume_information(
    backend: &mut SimpleRdpdrBackend,
    req_inner: ServerDriveQueryVolumeInformationRequest,
) -> PduResult<Vec<SvcMessage>> {
    match backend.file_map.get(&req_inner.device_io_request.file_id) {
        Some(file) => {
            if let Ok(statvfs) = nix::sys::statvfs::fstatvfs(file.as_fd()) {
                if FileSystemInformationClassLevel::FILE_FS_FULL_SIZE_INFORMATION == req_inner.fs_info_class_lvl {
                    let info = FileFsFullSizeInformation {
                        total_alloc_units: i64::try_from(statvfs.blocks()).unwrap(),
                        caller_available_alloc_units: i64::try_from(statvfs.blocks_available()).unwrap(),
                        actual_available_alloc_units: i64::try_from(statvfs.blocks_available()).unwrap(),
                        sectors_per_alloc_unit: u32::try_from(statvfs.fragment_size()).unwrap(),
                        bytes_per_sector: 1,
                    };
                    Ok(vec![SvcMessage::from(
                        RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
                            ClientDriveQueryVolumeInformationResponse {
                                device_io_reply: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                                buffer: Some(FileSystemInformationClass::FileFsFullSizeInformation(info)),
                            },
                        ),
                    )])
                } else if FileSystemInformationClassLevel::FILE_FS_ATTRIBUTE_INFORMATION == req_inner.fs_info_class_lvl
                {
                    Ok(vec![SvcMessage::from(
                        RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
                            ClientDriveQueryVolumeInformationResponse {
                                device_io_reply: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                                buffer: Some(FileSystemInformationClass::FileFsAttributeInformation(
                                    FileFsAttributeInformation {
                                        file_system_attributes: FileSystemAttributes::FILE_CASE_SENSITIVE_SEARCH
                                            | FileSystemAttributes::FILE_CASE_PRESERVED_NAMES
                                            | FileSystemAttributes::FILE_UNICODE_ON_DISK,
                                        max_component_name_len: 260,
                                        file_system_name: "FAT32".to_owned(),
                                    },
                                )),
                            },
                        ),
                    )])
                } else if FileSystemInformationClassLevel::FILE_FS_VOLUME_INFORMATION == req_inner.fs_info_class_lvl {
                    Ok(vec![SvcMessage::from(
                        RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
                            ClientDriveQueryVolumeInformationResponse {
                                device_io_reply: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                                buffer: Some(FileSystemInformationClass::FileFsVolumeInformation(
                                    FileFsVolumeInformation {
                                        volume_creation_time: transform_to_filetime(file.metadata().unwrap().ctime()),
                                        // blocks_available() may have different integer type on different platforms.
                                        // so we need to cast it to u32 uniformly. so if it is u32, it will emit 'useless conversion'
                                        // warning, i choose to mute it.
                                        #[allow(clippy::useless_conversion)]
                                        volume_serial_number: u32::try_from(statvfs.blocks_available()).unwrap(),
                                        supports_objects: Boolean::False,
                                        volume_label: "IRON_RDP".to_owned(),
                                    },
                                )),
                            },
                        ),
                    )])
                } else if FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION == req_inner.fs_info_class_lvl {
                    Ok(vec![SvcMessage::from(
                        RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
                            ClientDriveQueryVolumeInformationResponse {
                                device_io_reply: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
                                buffer: Some(FileSystemInformationClass::FileFsSizeInformation(
                                    FileFsSizeInformation {
                                        total_alloc_units: i64::try_from(statvfs.blocks()).unwrap(),
                                        available_alloc_units: i64::try_from(statvfs.blocks_free()).unwrap(),
                                        sectors_per_alloc_unit: u32::try_from(statvfs.fragment_size()).unwrap(),
                                        bytes_per_sector: 1,
                                    },
                                )),
                            },
                        ),
                    )])
                } else {
                    warn!("unsupported volume class");
                    Ok(vec![SvcMessage::from(
                        RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
                            ClientDriveQueryVolumeInformationResponse {
                                device_io_reply: DeviceIoResponse::new(
                                    req_inner.device_io_request,
                                    NtStatus::UNSUCCESSFUL,
                                ),
                                buffer: None,
                            },
                        ),
                    )])
                }
            } else {
                warn!("no such file");
                let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                    device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::NO_SUCH_FILE),
                    buffer: None,
                });
                Ok(vec![SvcMessage::from(res)])
            }
        }
        None => {
            warn!("no such file");
            let res = RdpdrPdu::ClientDriveQueryInformationResponse(ClientDriveQueryInformationResponse {
                device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::NO_SUCH_FILE),
                buffer: None,
            });
            Ok(vec![SvcMessage::from(res)])
        }
    }
}
pub(crate) fn set_information(
    backend: &mut SimpleRdpdrBackend,
    req_inner: ServerDriveSetInformationRequest,
) -> PduResult<Vec<SvcMessage>> {
    match backend.file_path_map.get(&req_inner.device_io_request.file_id) {
        Some(file) => {
            match &req_inner.set_buffer {
                FileInformationClass::Rename(info) => {
                    let mut to = backend.file_base.clone();
                    to.push_str(&info.file_name.replace('\\', "/"));
                    if let Err(error) = std::fs::rename(file, to) {
                        warn!(?error, "Rename file error");
                        let res = RdpdrPdu::ClientDriveSetInformationResponse(ClientDriveSetInformationResponse::new(
                            &req_inner,
                            NtStatus::UNSUCCESSFUL,
                        )?);
                        return Ok(vec![SvcMessage::from(res)]);
                    }
                }
                FileInformationClass::Allocation(_) => {
                    //nothing to do
                }
                FileInformationClass::Disposition(_) => {
                    if let Err(error) = std::fs::remove_file(file) {
                        warn!(?error, "Remove file error");
                        let res = RdpdrPdu::ClientDriveSetInformationResponse(ClientDriveSetInformationResponse::new(
                            &req_inner,
                            NtStatus::UNSUCCESSFUL,
                        )?);
                        return Ok(vec![SvcMessage::from(res)]);
                    }
                }
                FileInformationClass::EndOfFile(info) => {
                    let mut set_end_res = -1;
                    // SAFETY: it is safe to call on *nix platform
                    unsafe {
                        if let Some(file) = backend.file_map.get(&req_inner.device_io_request.file_id) {
                            set_end_res = ftruncate(file.as_raw_fd(), info.end_of_file);
                        }
                    }
                    if set_end_res < 0 {
                        warn!("set file end error, code is:{}", set_end_res);
                        let res = RdpdrPdu::ClientDriveSetInformationResponse(ClientDriveSetInformationResponse::new(
                            &req_inner,
                            NtStatus::UNSUCCESSFUL,
                        )?);
                        return Ok(vec![SvcMessage::from(res)]);
                    }
                }
                _ => {
                    // TODO
                }
            }
        }
        None => {
            warn!("no such file");
            let res = RdpdrPdu::ClientDriveSetInformationResponse(ClientDriveSetInformationResponse::new(
                &req_inner,
                NtStatus::NO_SUCH_FILE,
            )?);
            return Ok(vec![SvcMessage::from(res)]);
        }
    }
    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveSetInformationResponse(
        ClientDriveSetInformationResponse::new(&req_inner, NtStatus::SUCCESS)?,
    ))])
}
// in fact, it is time in secs which is very small
#[allow(clippy::arithmetic_side_effects)]
pub(crate) fn transform_to_filetime(time_in_secs: i64) -> i64 {
    let mut time = time_in_secs * 10000000;
    time += 116444736000000000;
    time
}
pub(crate) fn get_file_attributes(meta: &std::fs::Metadata, file_name: &str) -> FileAttributes {
    let mut file_attribute = FileAttributes::empty();
    if meta.is_dir() {
        file_attribute |= FileAttributes::FILE_ATTRIBUTE_DIRECTORY;
    }
    if file_attribute.is_empty() {
        file_attribute |= FileAttributes::FILE_ATTRIBUTE_ARCHIVE;
    }

    if file_name.len() > 1 && file_name.starts_with('.') && file_name.as_bytes()[1] != b'.' {
        file_attribute |= FileAttributes::FILE_ATTRIBUTE_HIDDEN;
    }
    if meta.permissions().readonly() {
        file_attribute |= FileAttributes::FILE_ATTRIBUTE_READONLY;
    }
    file_attribute
}
pub(crate) fn make_query_dir_resp(
    find_file_name: Option<String>,
    device_io_request: DeviceIoRequest,
    file_class: FileInformationClassLevel,
    initial_query: bool,
) -> PduResult<Vec<SvcMessage>> {
    let not_found_status = if initial_query {
        NtStatus::NO_SUCH_FILE
    } else {
        NtStatus::NO_MORE_FILES
    };
    match find_file_name {
        None => Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
            ClientDriveQueryDirectoryResponse {
                device_io_reply: DeviceIoResponse::new(device_io_request, not_found_status),
                buffer: None,
            },
        ))]),
        Some(file_full_path) => {
            // in fact, it represents file name, so it is not very large
            #[allow(clippy::arithmetic_side_effects)]
            let file_last_slash = if let Some(index) = file_full_path.rfind('/') {
                index + 1
            } else {
                0
            };
            let file_name = &file_full_path[file_last_slash..];
            match std::fs::metadata(&file_full_path) {
                Ok(meta) => {
                    let file_attribute = get_file_attributes(&meta, file_name);
                    if file_class == FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION {
                        let info = FileBothDirectoryInformation::new(
                            transform_to_filetime(meta.ctime()),
                            transform_to_filetime(meta.ctime()),
                            transform_to_filetime(meta.atime()),
                            transform_to_filetime(meta.mtime()),
                            i64::try_from(meta.size()).unwrap(),
                            file_attribute,
                            file_name.to_owned(),
                        );
                        let info2 = FileInformationClass::BothDirectory(info);
                        Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
                            ClientDriveQueryDirectoryResponse {
                                device_io_reply: DeviceIoResponse::new(device_io_request, NtStatus::SUCCESS),
                                buffer: Some(info2),
                            },
                        ))])
                    } else {
                        warn!("unsupported file class for query directory");
                        Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
                            ClientDriveQueryDirectoryResponse {
                                device_io_reply: DeviceIoResponse::new(device_io_request, NtStatus::NOT_SUPPORTED),
                                buffer: None,
                            },
                        ))])
                    }
                }
                Err(error) => {
                    warn!(%error, "Get metadata error");
                    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
                        ClientDriveQueryDirectoryResponse {
                            device_io_reply: DeviceIoResponse::new(device_io_request, not_found_status),
                            buffer: None,
                        },
                    ))])
                }
            }
        }
    }
}
pub(crate) fn query_directory(
    backend: &mut SimpleRdpdrBackend,
    req_inner: ServerDriveQueryDirectoryRequest,
) -> PduResult<Vec<SvcMessage>> {
    match backend.file_path_map.get(&req_inner.device_io_request.file_id) {
        Some(parent_pos_for_next) => {
            let mut find_file_name = None;
            if req_inner.initial_query > 0 {
                if req_inner.path.ends_with('*') {
                    let mut parent = backend.file_base.clone();
                    let query_path = req_inner.path.replace('\\', "/");
                    let len = query_path.len();
                    // path ends with *, so its len > 0
                    #[allow(clippy::arithmetic_side_effects)]
                    parent.push_str(&query_path[0..len - 1]);
                    let dirp = if let Ok(real_parent_c) = CString::new(parent.clone()) {
                        // SAFETY: it is safe to call this on *nix platform
                        unsafe { opendir(real_parent_c.as_ptr()) }
                    } else {
                        std::ptr::null::<DIR>().cast_mut()
                    };
                    if !dirp.is_null() {
                        loop {
                            // SAFETY: it is safe to call readdir on *nix platform
                            let first = unsafe { readdir(dirp) };
                            if first.is_null() {
                                break;
                            }
                            // SAFETY: it is safe to call this for rdp
                            let dir_name = unsafe { &(*first).d_name };
                            // SAFETY: it is safe to call this for rdp
                            let dir_name_in_rs =
                                unsafe { CStr::from_ptr(dir_name as *const i8).to_string_lossy().into_owned() };
                            if dir_name_in_rs == "." || dir_name_in_rs == ".." {
                                continue;
                            }
                            parent.push_str(&dir_name_in_rs);
                            find_file_name = Some(parent);
                            break;
                        }
                        backend
                            .file_dir_map
                            .insert(req_inner.device_io_request.file_id, DirBox { dir: dirp });
                    }
                } else {
                    let mut full_path = backend.file_base.clone();
                    let query_path = req_inner.path.replace('\\', "/");
                    full_path.push_str(&query_path);
                    find_file_name = Some(full_path);
                }
                make_query_dir_resp(
                    find_file_name,
                    req_inner.device_io_request,
                    req_inner.file_info_class_lvl,
                    true,
                )
            } else {
                if let Some(dir_box) = backend.file_dir_map.get_mut(&req_inner.device_io_request.file_id) {
                    // SAFETY: it is safe to call readdir in multiple threads
                    let next = unsafe { readdir(dir_box.dir) };
                    if !next.is_null() {
                        // SAFETY:
                        let file_name = unsafe { &(*next).d_name };
                        // SAFETY: it is safe to call CStr::from_ptr(file name) for rdp
                        let mx = unsafe { CStr::from_ptr(file_name as *const i8).to_string_lossy().into_owned() };
                        let mut full_path = parent_pos_for_next.clone();
                        if !full_path.ends_with('/') {
                            full_path.push('/');
                        }
                        full_path.push_str(&mx);
                        find_file_name = Some(full_path);
                    }
                }
                make_query_dir_resp(
                    find_file_name,
                    req_inner.device_io_request,
                    req_inner.file_info_class_lvl,
                    false,
                )
            }
        }
        None => {
            warn!("no file to query directory");
            Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
                ClientDriveQueryDirectoryResponse {
                    device_io_reply: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::NO_SUCH_FILE),
                    buffer: None,
                },
            ))])
        }
    }
}
fn make_create_drive_resp(
    device_io_request: DeviceIoRequest,
    create_disposation: CreateDisposition,
    file_id: u32,
) -> PduResult<Vec<SvcMessage>> {
    let io_response = DeviceIoResponse::new(device_io_request, NtStatus::SUCCESS);
    let information = match create_disposation {
        CreateDisposition::FILE_CREATE
        | CreateDisposition::FILE_SUPERSEDE
        | CreateDisposition::FILE_OPEN
        | CreateDisposition::FILE_OVERWRITE => Information::FILE_SUPERSEDED,
        CreateDisposition::FILE_OPEN_IF => Information::FILE_OPENED,
        CreateDisposition::FILE_OVERWRITE_IF => Information::FILE_OVERWRITTEN,
        _ => Information::empty(),
    };
    let res = RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
        device_io_reply: io_response,
        file_id,
        information,
    });
    Ok(vec![SvcMessage::from(res)])
}
// in fact, index only needs to be different, so it is ok
#[allow(clippy::arithmetic_side_effects)]
pub(crate) fn create_drive(
    backend: &mut SimpleRdpdrBackend,
    req_inner: DeviceCreateRequest,
) -> PduResult<Vec<SvcMessage>> {
    let file_id = backend.file_id;
    backend.file_id += 1;
    let mut path = String::from(backend.file_base.as_str());
    path.push_str(&req_inner.path.replace('\\', "/"));
    // first process directory
    match std::fs::metadata(&path) {
        Ok(meta) => {
            if meta.is_dir() {
                if req_inner.create_disposition == CreateDisposition::FILE_CREATE {
                    warn!("Attempt to create directory, but it exists");
                    let io_response = DeviceIoResponse::new(req_inner.device_io_request, NtStatus::UNSUCCESSFUL);
                    let res = RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
                        device_io_reply: io_response,
                        file_id,
                        information: Information::empty(),
                    });
                    return Ok(vec![SvcMessage::from(res)]);
                }
                if req_inner.create_options.bits() & CreateOptions::FILE_NON_DIRECTORY_FILE.bits() != 0 {
                    warn!("Attempt to create a file, but it is a directory");
                    let io_response = DeviceIoResponse::new(req_inner.device_io_request, NtStatus::UNSUCCESSFUL);
                    let res = RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
                        device_io_reply: io_response,
                        file_id,
                        information: Information::empty(),
                    });
                    return Ok(vec![SvcMessage::from(res)]);
                }
                // Return afterwards
                // This can be unified with the condition for opening the file.
            } else if req_inner.create_options.bits() & CreateOptions::FILE_DIRECTORY_FILE.bits() != 0 {
                warn!("Attempt to create a directory, but it is a file");
                let io_response = DeviceIoResponse::new(req_inner.device_io_request, NtStatus::NOT_A_DIRECTORY);
                let res = RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
                    device_io_reply: io_response,
                    file_id,
                    information: Information::empty(),
                });
                return Ok(vec![SvcMessage::from(res)]);
            }
        }
        Err(_) => {
            if req_inner.create_options.bits() & CreateOptions::FILE_DIRECTORY_FILE.bits() != 0 {
                warn!("Attempt to create a directory,but it is a file");
                if (req_inner.create_disposition == CreateDisposition::FILE_CREATE
                    || req_inner.create_disposition == CreateDisposition::FILE_OPEN_IF)
                    && std::fs::create_dir_all(path.as_str()).is_ok()
                {
                    let mut fs = std::fs::OpenOptions::new();
                    match fs.read(true).open(&path) {
                        Ok(file) => {
                            debug!("create drive file_id:{},path:{}", file_id, path);
                            backend.file_map.insert(file_id, file);
                            backend.file_path_map.insert(file_id, path.clone());
                            return make_create_drive_resp(
                                req_inner.device_io_request,
                                req_inner.create_disposition,
                                file_id,
                            );
                        }
                        Err(error) => {
                            warn!(%error, "Open file dir error");
                            //return by downside
                        }
                    }
                }
                //create disposition is not correct
                let io_response = DeviceIoResponse::new(req_inner.device_io_request, NtStatus::UNSUCCESSFUL);
                let res = RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
                    device_io_reply: io_response,
                    file_id,
                    information: Information::empty(),
                });
                return Ok(vec![SvcMessage::from(res)]);
            }
        }
    }

    let mut fs = std::fs::OpenOptions::new();
    if CreateDisposition::FILE_OPEN_IF == req_inner.create_disposition {
        fs.create(true).write(true).read(true);
    }
    if CreateDisposition::FILE_CREATE == req_inner.create_disposition {
        fs.create_new(true).write(true).read(true);
    }
    if CreateDisposition::FILE_SUPERSEDE == req_inner.create_disposition {
        fs.create(true).write(true).append(true).read(true);
    }
    if CreateDisposition::FILE_OPEN == req_inner.create_disposition {
        fs.read(true);
    }
    if CreateDisposition::FILE_OVERWRITE == req_inner.create_disposition {
        fs.write(true).truncate(true).read(true);
    }
    if CreateDisposition::FILE_OVERWRITE_IF == req_inner.create_disposition {
        fs.write(true).truncate(true).create(true).read(true);
    }

    match fs.open(&path) {
        Ok(file) => {
            debug!("create drive file_id:{},path:{}", file_id, path);
            backend.file_map.insert(file_id, file);
            backend.file_path_map.insert(file_id, path.clone());
            make_create_drive_resp(req_inner.device_io_request, req_inner.create_disposition, file_id)
        }
        Err(error) => {
            warn!(?error, "Open file error for path:{}", path);
            let io_response = DeviceIoResponse::new(req_inner.device_io_request, NtStatus::UNSUCCESSFUL);
            let res = RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
                device_io_reply: io_response,
                file_id,
                information: Information::empty(),
            });
            Ok(vec![SvcMessage::from(res)])
        }
    }
}
pub(crate) fn process_dependent_file(
    backend: &mut SimpleRdpdrBackend,
    request: DeviceIoRequest,
    error_fx: impl Fn(DeviceIoRequest) -> PduResult<Vec<SvcMessage>>,
    fx: impl Fn(&mut std::fs::File, DeviceIoRequest) -> PduResult<Vec<SvcMessage>>,
) -> PduResult<Vec<SvcMessage>> {
    match backend.file_map.get_mut(&request.file_id) {
        None => error_fx(request),
        Some(file) => fx(file, request),
    }
}
