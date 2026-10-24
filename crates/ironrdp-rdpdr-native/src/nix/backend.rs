use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::MetadataExt;

use ironrdp_core::impl_as_any;
use ironrdp_pdu::{PduResult, encode_err};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::*;
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_rdpdr::{
    RdpdrBackend, RdpdrBackendFactory, RdpdrBackendFactoryResult, RdpdrBackendProduct, RdpdrDrive, RdpdrPrinter,
};
use ironrdp_svc::SvcMessage;
use nix::dir::{Dir, OwningIter};
use nix::errno::Errno;
use nix::fcntl::{AtFlags, OFlag, renameat};
use nix::sys::resource::{Resource, getrlimit};
use nix::sys::stat::{FileStat, Mode, SFlag, dev_t, fstat, fstatat, mkdirat};
use nix::unistd::{UnlinkatFlags, ftruncate, unlinkat};
use tracing::{debug, trace, warn};

use super::path::{InvalidPath, RelativePath, Root, io_to_errno};
use super::printer::{PrintTarget, PrinterSpooler, unsupported_response, write_response};

/// The most files the server may hold open at once, however high the descriptor limit is.
const MAX_OPEN_FILES: usize = 1_024;

/// The largest read the server may request, so a single request cannot make the client allocate gigabytes.
const MAX_READ_LENGTH: usize = 1_024 * 1_024;

#[derive(Debug)]
pub struct NixRdpdrBackend {
    file_id: u32,
    /// The shared directory every server-supplied path resolves beneath.
    file_base: String,
    /// INVARIANT: holds at most `max_open_files` entries.
    open_files: HashMap<u32, OpenFile>,
    /// Bounds the open files, so the server cannot exhaust the descriptors of the client process.
    ///
    /// A quarter of the soft descriptor limit, at most `MAX_OPEN_FILES`.
    /// An open file holds up to three descriptors: the file, and a directory being enumerated with its stream.
    /// A quarter of the limit therefore leaves at least a quarter of the descriptors to the rest of the process.
    max_open_files: usize,
    /// Files to delete when their last open handle closes ([MS-FSA] 2.1.5.5 Server Requests Closing an Open).
    ///
    /// [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/d142c93a-72bc-4b05-9d96-8e00371c3308
    pending_deletes: HashSet<FileIdentity>,
    /// Print-job handling for the virtual printer, when one is announced.
    printer: Option<Printer>,
}

impl Default for NixRdpdrBackend {
    fn default() -> Self {
        Self::new(String::new())
    }
}

#[derive(Debug)]
struct OpenFile {
    file: std::fs::File,
    path: RelativePath,
    identity: FileIdentity,
    /// The enumeration the last initial query-directory request started on this file.
    listing: Option<DirectoryListing>,
}

/// Identifies a file independently of its name, so a rename does not lose track of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct FileIdentity {
    device: dev_t,
    inode: nix::libc::ino_t,
}

impl FileIdentity {
    fn of(stat: &FileStat) -> Self {
        Self {
            device: stat.st_dev,
            inode: stat.st_ino,
        }
    }
}

#[derive(Debug)]
struct DirectoryListing {
    /// The enumerated directory, which entry names are resolved against.
    directory: OwnedFd,
    /// `.` and `..`, still to be returned first, which every directory but the root lists ([MS-FSA] 2.1.5.6.3 Directory Information Queries).
    ///
    /// [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/fa8194e0-53ec-413b-8315-e8fa85396fd8
    dot_entries: &'static [&'static str],
    entries: OwningIter,
}

impl DirectoryListing {
    fn open(directory: OwnedFd, is_root: bool) -> nix::Result<Self> {
        let entries = Dir::from_fd(directory.try_clone().map_err(io_to_errno)?)?.into_iter();
        Ok(Self {
            directory,
            dot_entries: if is_root { &[] } else { &[".", ".."] },
            entries,
        })
    }

    /// Returns the next entry, or `None` once enumeration ends, skipping entries removed since it started.
    fn next_entry(&mut self) -> nix::Result<Option<DirectoryEntry>> {
        if let Some((name, rest)) = self.dot_entries.split_first() {
            self.dot_entries = rest;
            let stat = fstatat(self.directory.as_fd(), *name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
            return Ok(Some(DirectoryEntry {
                name: (*name).to_owned(),
                stat,
            }));
        }
        for entry in self.entries.by_ref() {
            let entry = entry?;
            let name = entry.file_name();
            if matches!(name.to_bytes(), b"." | b"..") {
                continue;
            }
            let Ok(stat) = fstatat(self.directory.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW) else {
                continue;
            };
            return Ok(Some(DirectoryEntry {
                name: name.to_string_lossy().into_owned(),
                stat,
            }));
        }
        Ok(None)
    }
}

struct DirectoryEntry {
    name: String,
    /// The entry itself, not the target of a symbolic link.
    stat: FileStat,
}

#[derive(Debug)]
struct Printer {
    device_id: u32,
    spooler: PrinterSpooler,
}

impl NixRdpdrBackend {
    /// Creates a backend that serves drive requests from the `file_base` directory.
    ///
    /// Server-supplied paths are resolved beneath `file_base` and never follow symbolic links, so the server cannot reach files outside it.
    pub fn new(file_base: String) -> Self {
        Self {
            file_id: 0,
            file_base,
            open_files: HashMap::new(),
            max_open_files: match getrlimit(Resource::RLIMIT_NOFILE) {
                Ok((soft_limit, _)) => usize::try_from(soft_limit / 4)
                    .unwrap_or(usize::MAX)
                    .min(MAX_OPEN_FILES),
                Err(error) => {
                    warn!(%error, "Failed to read the descriptor limit");
                    MAX_OPEN_FILES
                }
            },
            pending_deletes: HashSet::new(),
            printer: None,
        }
    }

    /// Closes `file_id`, deleting the file when it is marked for deletion and no other handle holds it.
    fn close_file(&mut self, file_id: u32) {
        let Some(open_file) = self.open_files.remove(&file_id) else {
            return;
        };
        let still_open = self
            .open_files
            .values()
            .any(|other| other.identity == open_file.identity);
        if !still_open
            && self.pending_deletes.remove(&open_file.identity)
            && let Err(error) = remove_file(&self.file_base, &open_file)
        {
            warn!(%error, "Delete on close error");
        }
    }

    /// Accepts print jobs for the virtual printer announced as `device_id` and sends them to
    /// `target`.
    #[must_use]
    pub fn with_printer(mut self, device_id: u32, target: PrintTarget) -> Self {
        self.printer = Some(Printer {
            device_id,
            spooler: PrinterSpooler::new(target),
        });
        self
    }
}

impl_as_any!(NixRdpdrBackend);

/// Builds a [`NixRdpdrBackend`] for every RDPDR channel lifetime.
///
/// `drives` are announced as redirected folders; `printer` announces a virtual
/// printer named after the client, whose jobs go to the given target. Drive
/// device IDs must be unique and nonzero, and the printer takes the highest ID
/// no drive uses; [`RdpdrBackendFactory::build_rdpdr_backend`] reports a
/// configuration that breaks this with a [`NixRdpdrBackendFactoryError`].
#[derive(Debug, Clone)]
pub struct NixRdpdrBackendFactory {
    file_base: String,
    drives: Vec<(u32, String)>,
    printer: Option<(String, PrintTarget)>,
}

impl NixRdpdrBackendFactory {
    pub fn new(file_base: String) -> Self {
        Self {
            file_base,
            drives: Vec::new(),
            printer: None,
        }
    }

    #[must_use]
    pub fn with_drive(mut self, device_id: u32, name: String) -> Self {
        self.drives.push((device_id, name));
        self
    }

    #[must_use]
    pub fn with_printer(mut self, name: String, target: PrintTarget) -> Self {
        self.printer = Some((name, target));
        self
    }
}

/// The device ID the virtual printer takes unless a drive uses it.
const DEFAULT_PRINTER_DEVICE_ID: u32 = u32::MAX - 1;

impl RdpdrBackendFactory for NixRdpdrBackendFactory {
    fn build_rdpdr_backend(&self) -> RdpdrBackendFactoryResult<RdpdrBackendProduct> {
        let mut device_ids = HashSet::with_capacity(self.drives.len());
        for (device_id, name) in &self.drives {
            if *device_id == 0 {
                return Err(Box::new(NixRdpdrBackendFactoryError::ReservedDeviceId));
            }
            if !is_valid_device_name(name) {
                return Err(Box::new(NixRdpdrBackendFactoryError::InvalidDriveName(*device_id)));
            }
            if !device_ids.insert(*device_id) {
                return Err(Box::new(NixRdpdrBackendFactoryError::DuplicateDeviceId(*device_id)));
            }
        }

        let mut backend = NixRdpdrBackend::new(self.file_base.clone());
        let mut printer = None;
        if let Some((name, target)) = &self.printer {
            if !is_valid_device_name(name) {
                return Err(Box::new(NixRdpdrBackendFactoryError::InvalidPrinterName));
            }
            let mut device_id = DEFAULT_PRINTER_DEVICE_ID;
            while device_ids.contains(&device_id) {
                device_id = device_id
                    .checked_sub(1)
                    .filter(|device_id| *device_id != 0)
                    .ok_or(NixRdpdrBackendFactoryError::NoPrinterDeviceId)?;
            }
            backend = backend.with_printer(device_id, target.clone());
            printer = Some(RdpdrPrinter::new(
                device_id,
                name.clone(),
                DEFAULT_PRINTER_DRIVER_NAME.to_owned(),
            ));
        }

        let drives = self
            .drives
            .iter()
            .map(|(id, name)| RdpdrDrive::new(*id, name.clone()))
            .collect();
        let mut product = RdpdrBackendProduct::new(Box::new(backend), drives);
        if let Some(printer) = printer {
            product = product.with_printer(printer);
        }
        Ok(product)
    }
}

/// Device names are announced as NUL-terminated strings, so an empty name or one with an embedded
/// NUL would reach the server empty or cut short.
fn is_valid_device_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('\0')
}

/// Invalid [`NixRdpdrBackendFactory`] configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NixRdpdrBackendFactoryError {
    /// A drive used device ID 0.
    ReservedDeviceId,
    /// More than one drive used the same device ID.
    DuplicateDeviceId(u32),
    /// The drives use every device ID the printer could take.
    NoPrinterDeviceId,
    /// The drive with this device ID has an empty name or one with an embedded NUL.
    InvalidDriveName(u32),
    /// The printer has an empty name or one with an embedded NUL.
    InvalidPrinterName,
}

impl core::fmt::Display for NixRdpdrBackendFactoryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ReservedDeviceId => f.write_str("RDPDR device ID 0 is reserved"),
            Self::DuplicateDeviceId(device_id) => write!(f, "duplicate RDPDR device ID {device_id}"),
            Self::NoPrinterDeviceId => f.write_str("no RDPDR device ID is available for the printer"),
            Self::InvalidDriveName(device_id) => {
                write!(
                    f,
                    "RDPDR drive {device_id} has an empty name or one with an embedded NUL"
                )
            }
            Self::InvalidPrinterName => f.write_str("the RDPDR printer has an empty name or one with an embedded NUL"),
        }
    }
}

impl core::error::Error for NixRdpdrBackendFactoryError {}

impl RdpdrBackend for NixRdpdrBackend {
    fn reset(&mut self) -> PduResult<()> {
        if let Some(printer) = self.printer.as_mut() {
            printer.spooler.reset();
        }
        // The server forgets its handles when the channel restarts, so they are closed as if it had closed them.
        let file_ids = self.open_files.keys().copied().collect::<Vec<_>>();
        for file_id in file_ids {
            self.close_file(file_id);
        }
        Ok(())
    }
    fn handle_server_device_announce_response(&mut self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        if self
            .printer
            .as_ref()
            .is_some_and(|printer| printer.device_id == pdu.device_id)
        {
            if pdu.result_code == NtStatus::SUCCESS {
                tracing::info!("The server accepted the redirected printer");
            } else {
                warn!(?pdu.result_code, "The server rejected the redirected printer");
            }
        }
        Ok(())
    }
    fn handle_printer_io_request(&mut self, req: PrinterIoRequest) -> PduResult<Vec<SvcMessage>> {
        match self.printer.as_mut() {
            Some(printer) => printer.spooler.handle(req),
            None => Ok(vec![SvcMessage::from(unsupported_response(req))]),
        }
    }
    fn reject_printer_write(&mut self, req: DeviceIoRequest) -> PduResult<Vec<SvcMessage>> {
        match self.printer.as_mut() {
            Some(printer) => printer.spooler.reject_write(req),
            None => Ok(vec![SvcMessage::from(write_response(req, 0, NtStatus::NOT_SUPPORTED))]),
        }
    }
    fn handle_scard_call(
        &mut self,
        _req: DeviceControlRequest<ScardIoCtlCode>,
        _call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }
    fn handle_drive_io_request(&mut self, req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        debug!("handle_drive_io_request:{:?}", req);
        match req {
            ServerDriveIoRequest::DeviceWriteRequest(req_inner) => write_device(self, req_inner),
            ServerDriveIoRequest::ServerCreateDriveRequest(req_inner) => create_drive(self, req_inner),
            ServerDriveIoRequest::DeviceReadRequest(req_inner) => read_device(self, req_inner),
            ServerDriveIoRequest::DeviceCloseRequest(req_inner) => close_device(self, req_inner),
            ServerDriveIoRequest::DeviceFlushBuffersRequest(req_inner) => Ok(vec![SvcMessage::from(
                RdpdrPdu::DeviceFlushBuffersResponse(DeviceFlushBuffersResponse {
                    device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::NOT_SUPPORTED),
                }),
            )]),
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
            ServerDriveIoRequest::ServerDriveQuerySecurityRequest(req_inner) => Ok(vec![SvcMessage::from(
                RdpdrPdu::ClientDriveQuerySecurityResponse(ClientDriveQuerySecurityResponse {
                    device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::NOT_SUPPORTED),
                    security_descriptor: None,
                }),
            )]),
            ServerDriveIoRequest::ServerDriveSetSecurityRequest(req_inner) => {
                let response = ClientDriveSetSecurityResponse::new(&req_inner, NtStatus::NOT_SUPPORTED)
                    .map_err(|error| encode_err!(error))?;
                Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveSetSecurityResponse(
                    response,
                ))])
            }
        }
    }
}

pub(crate) fn write_device(backend: &mut NixRdpdrBackend, req_inner: DeviceWriteRequest) -> PduResult<Vec<SvcMessage>> {
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

pub(crate) fn read_device(backend: &mut NixRdpdrBackend, req_inner: DeviceReadRequest) -> PduResult<Vec<SvcMessage>> {
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
        |file, request| {
            let length = usize::try_from(req_inner.length).unwrap();
            let (status, read_data) = if MAX_READ_LENGTH < length {
                warn!(length, "Rejected an oversized read");
                (NtStatus::INVALID_PARAMETER, Vec::new())
            } else {
                match read_inner(file, req_inner.offset, length) {
                    Ok(buf) => (NtStatus::SUCCESS, buf),
                    Err(error) => {
                        warn!(?error, "Read error");
                        (NtStatus::UNSUCCESSFUL, Vec::new())
                    }
                }
            };
            let res = RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
                device_io_reply: DeviceIoResponse::new(request, status),
                read_data,
            });
            Ok(vec![SvcMessage::from(res)])
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

pub(crate) fn close_device(backend: &mut NixRdpdrBackend, req_inner: DeviceCloseRequest) -> PduResult<Vec<SvcMessage>> {
    // The handle is closed whether or not a pending delete succeeds, so the close still reports success.
    backend.close_file(req_inner.device_io_request.file_id);
    let res = RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
        device_io_response: DeviceIoResponse::new(req_inner.device_io_request, NtStatus::SUCCESS),
    });
    Ok(vec![SvcMessage::from(res)])
}

pub(crate) fn query_information(
    backend: &mut NixRdpdrBackend,
    req_inner: ServerDriveQueryInformationRequest,
) -> PduResult<Vec<SvcMessage>> {
    match backend.open_files.get(&req_inner.device_io_request.file_id) {
        Some(OpenFile {
            file, path, identity, ..
        }) => match file.metadata() {
            Ok(meta) => {
                let name = path.file_name().unwrap_or_default();
                let file_attribute = get_file_attributes(meta.is_dir(), meta.permissions().readonly(), name);
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
                        delete_pending: if backend.pending_deletes.contains(identity) {
                            Boolean::True
                        } else {
                            Boolean::False
                        },
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
    backend: &mut NixRdpdrBackend,
    req_inner: ServerDriveQueryVolumeInformationRequest,
) -> PduResult<Vec<SvcMessage>> {
    match backend.open_files.get(&req_inner.device_io_request.file_id) {
        Some(OpenFile { file, .. }) => {
            if let Ok(statvfs) = nix::sys::statvfs::fstatvfs(file.as_fd()) {
                if FileSystemInformationClassLevel::FILE_FS_FULL_SIZE_INFORMATION == req_inner.fs_info_class_lvl {
                    #[cfg_attr(target_os = "macos", expect(clippy::unnecessary_fallible_conversions))]
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
                                        #[expect(
                                            clippy::allow_attributes,
                                            reason = "we have to use allow as the useless_conversion isn't triggered on some platforms"
                                        )]
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
                                #[cfg_attr(target_os = "macos", expect(clippy::unnecessary_fallible_conversions))]
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
    backend: &mut NixRdpdrBackend,
    req_inner: ServerDriveSetInformationRequest,
) -> PduResult<Vec<SvcMessage>> {
    let file_id = req_inner.device_io_request.file_id;
    // [MS-FSA] 2.1.5.15.12 FileRenameInformation refuses to rename a file marked for deletion or a directory holding open files.
    let rename_denied = matches!(req_inner.set_buffer, FileInformationClass::Rename(_))
        && backend.open_files.get(&file_id).is_some_and(|open_file| {
            backend.pending_deletes.contains(&open_file.identity)
                || backend
                    .open_files
                    .values()
                    .any(|other| other.path.is_inside(&open_file.path))
        });
    let mut renamed = None;
    let status = match backend.open_files.get_mut(&file_id) {
        Some(open_file) => match &req_inner.set_buffer {
            FileInformationClass::Rename(_) if rename_denied => NtStatus::ACCESS_DENIED,
            FileInformationClass::Rename(info) => match RelativePath::parse(&info.file_name) {
                Ok(to) => match rename(
                    &backend.file_base,
                    &open_file.path,
                    &to,
                    info.replace_if_exists == Boolean::True,
                ) {
                    Ok(()) => {
                        renamed = Some((open_file.identity, to));
                        NtStatus::SUCCESS
                    }
                    Err(status) => {
                        trace!(?status, "Rename file failed");
                        status
                    }
                },
                Err(InvalidPath) => {
                    warn!(file_name = ?info.file_name, "Rejected a rename target outside the shared directory");
                    NtStatus::OBJECT_NAME_INVALID
                }
            },
            // The allocation size is only a hint, so there is nothing to do.
            FileInformationClass::Allocation(_) => NtStatus::SUCCESS,
            FileInformationClass::Disposition(info) => match set_delete_pending(
                &backend.file_base,
                open_file,
                &mut backend.pending_deletes,
                info.delete_pending != 0,
            ) {
                Ok(()) => NtStatus::SUCCESS,
                Err(error) => {
                    warn!(%error, "Set delete pending error");
                    status_from_errno(error)
                }
            },
            FileInformationClass::EndOfFile(info) => match ftruncate(open_file.file.as_fd(), info.end_of_file) {
                Ok(()) => NtStatus::SUCCESS,
                Err(error) => {
                    warn!(%error, "Failed to set end of file");
                    status_from_errno(error)
                }
            },
            _ => {
                // TODO
                NtStatus::SUCCESS
            }
        },
        None => {
            warn!("No such file");
            NtStatus::NO_SUCH_FILE
        }
    };
    // Every handle to the renamed file follows it, so a later delete removes this file and not whatever takes the old name.
    if let Some((identity, to)) = renamed {
        for open_file in backend
            .open_files
            .values_mut()
            .filter(|open_file| open_file.identity == identity)
        {
            open_file.path = to.clone();
        }
    }
    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveSetInformationResponse(
        ClientDriveSetInformationResponse::new(&req_inner, status).map_err(|e| encode_err!(e))?,
    ))])
}

/// Renames `from` to `to` ([MS-FSA] 2.1.5.15.12 FileRenameInformation).
///
/// An existing `to` is a collision unless `replace_if_exists` is set, and a directory `to` is never replaced.
/// A `to` naming the source itself, as a case-only rename on a case-insensitive filesystem does, is not a collision.
/// The existence check and the rename are separate calls, so a local process creating `to` in between is overwritten.
///
/// [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/87f86c9b-6c2a-4803-84b7-131a74a434fa
fn rename(file_base: &str, from: &RelativePath, to: &RelativePath, replace_if_exists: bool) -> Result<(), NtStatus> {
    let root = Root::open(file_base).map_err(status_from_errno)?;
    let (from_parent, from_name) = from.open_parent(&root).map_err(status_from_parent_errno)?;
    let (to_parent, to_name) = to.open_parent(&root).map_err(status_from_parent_errno)?;
    let source = fstatat(from_parent.as_fd(), from_name, AtFlags::AT_SYMLINK_NOFOLLOW).map_err(status_from_errno)?;
    match fstatat(to_parent.as_fd(), to_name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(target) if FileIdentity::of(&target) == FileIdentity::of(&source) => {}
        Ok(_) if !replace_if_exists => return Err(NtStatus::OBJECT_NAME_COLLISION),
        Ok(target) if is_directory(&target) => return Err(NtStatus::ACCESS_DENIED),
        Ok(_) | Err(Errno::ENOENT) => {}
        Err(error) => return Err(status_from_errno(error)),
    }
    renameat(from_parent.as_fd(), from_name, to_parent.as_fd(), to_name).map_err(status_from_errno)
}

/// Marks `open_file` for deletion when its last handle closes, or clears the mark.
///
/// A directory is marked only when it is empty ([MS-FSA] 2.1.5.15.3 FileDispositionInformation).
///
/// [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/386d9ec5-e0f6-4853-b175-c05be01419e0
fn set_delete_pending(
    file_base: &str,
    open_file: &OpenFile,
    pending_deletes: &mut HashSet<FileIdentity>,
    delete_pending: bool,
) -> nix::Result<()> {
    if !delete_pending {
        pending_deletes.remove(&open_file.identity);
        return Ok(());
    }
    if is_directory(&fstat(open_file.file.as_fd())?) {
        let root = Root::open(file_base)?;
        let mut listing = DirectoryListing::open(open_file.path.open_directory(&root)?, true)?;
        if listing.next_entry()?.is_some() {
            return Err(Errno::ENOTEMPTY);
        }
    }
    pending_deletes.insert(open_file.identity);
    Ok(())
}

fn is_directory(stat: &FileStat) -> bool {
    SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT == SFlag::S_IFDIR
}

/// Maps a filesystem error to the status the Windows drive backend reports for the same condition.
fn status_from_errno(error: Errno) -> NtStatus {
    match error {
        Errno::ENOENT => NtStatus::NO_SUCH_FILE,
        Errno::EEXIST => NtStatus::OBJECT_NAME_COLLISION,
        Errno::ENOTDIR => NtStatus::NOT_A_DIRECTORY,
        Errno::EISDIR => NtStatus::FILE_IS_A_DIRECTORY,
        Errno::ENOTEMPTY => NtStatus::DIRECTORY_NOT_EMPTY,
        Errno::ENAMETOOLONG => NtStatus::OBJECT_NAME_INVALID,
        // A refused symbolic link or mount point is reported like a refused reparse point.
        Errno::EACCES | Errno::EPERM | Errno::ELOOP | Errno::EXDEV => NtStatus::ACCESS_DENIED,
        Errno::EROFS => NtStatus::MEDIA_WRITE_PROTECTED,
        Errno::ENOSPC | Errno::EDQUOT => NtStatus::DISK_FULL,
        Errno::EMFILE | Errno::ENFILE => NtStatus::INSUFFICIENT_RESOURCES,
        _ => NtStatus::UNSUCCESSFUL,
    }
}

/// Maps an error opening the directories that lead to a file, where a missing one is a missing path.
fn status_from_parent_errno(error: Errno) -> NtStatus {
    match error {
        Errno::ENOENT | Errno::ENOTDIR => NtStatus::OBJECT_PATH_NOT_FOUND,
        error => status_from_errno(error),
    }
}

/// Removes the file or empty directory `open_file` names, unless that name now refers to another file.
fn remove_file(file_base: &str, open_file: &OpenFile) -> nix::Result<()> {
    let root = Root::open(file_base)?;
    let (parent, name) = open_file.path.open_parent(&root)?;
    let stat = fstatat(parent.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
    if FileIdentity::of(&stat) != open_file.identity {
        return Err(Errno::ESTALE);
    }
    let flags = if is_directory(&stat) {
        UnlinkatFlags::RemoveDir
    } else {
        UnlinkatFlags::NoRemoveDir
    };
    unlinkat(parent.as_fd(), name, flags)
}

// in fact, it is time in secs which is very small
#[expect(clippy::arithmetic_side_effects)]
pub(crate) fn transform_to_filetime(time_in_secs: i64) -> i64 {
    let mut time = time_in_secs * 10000000;
    time += 116444736000000000;
    time
}

pub(crate) fn get_file_attributes(is_dir: bool, readonly: bool, file_name: &str) -> FileAttributes {
    let mut file_attribute = FileAttributes::empty();
    if is_dir {
        file_attribute |= FileAttributes::FILE_ATTRIBUTE_DIRECTORY;
    }
    if file_attribute.is_empty() {
        file_attribute |= FileAttributes::FILE_ATTRIBUTE_ARCHIVE;
    }

    if file_name.len() > 1 && file_name.starts_with('.') && file_name.as_bytes()[1] != b'.' {
        file_attribute |= FileAttributes::FILE_ATTRIBUTE_HIDDEN;
    }
    if readonly {
        file_attribute |= FileAttributes::FILE_ATTRIBUTE_READONLY;
    }
    file_attribute
}

fn make_query_dir_resp(
    entry: Result<DirectoryEntry, NtStatus>,
    device_io_request: DeviceIoRequest,
    file_class: FileInformationClassLevel,
) -> PduResult<Vec<SvcMessage>> {
    match entry {
        Err(status) => Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
            ClientDriveQueryDirectoryResponse {
                device_io_reply: DeviceIoResponse::new(device_io_request, status),
                buffer: None,
            },
        ))]),
        Ok(DirectoryEntry { name, stat }) => {
            if file_class == FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION {
                let is_dir = is_directory(&stat);
                let readonly =
                    !Mode::from_bits_truncate(stat.st_mode).intersects(Mode::S_IWUSR | Mode::S_IWGRP | Mode::S_IWOTH);
                let file_attribute = get_file_attributes(is_dir, readonly, &name);
                let info = FileBothDirectoryInformation::new(
                    transform_to_filetime(stat.st_ctime),
                    transform_to_filetime(stat.st_ctime),
                    transform_to_filetime(stat.st_atime),
                    transform_to_filetime(stat.st_mtime),
                    stat.st_size,
                    file_attribute,
                    name,
                );
                let info2 = FileInformationClass::BothDirectory(info);
                Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
                    ClientDriveQueryDirectoryResponse {
                        device_io_reply: DeviceIoResponse::new(device_io_request, NtStatus::SUCCESS),
                        buffer: Some(info2),
                    },
                ))])
            } else {
                warn!("Unsupported file class for query directory");
                Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveQueryDirectoryResponse(
                    ClientDriveQueryDirectoryResponse {
                        device_io_reply: DeviceIoResponse::new(device_io_request, NtStatus::NOT_SUPPORTED),
                        buffer: None,
                    },
                ))])
            }
        }
    }
}

pub(crate) fn query_directory(
    backend: &mut NixRdpdrBackend,
    req_inner: ServerDriveQueryDirectoryRequest,
) -> PduResult<Vec<SvcMessage>> {
    let entry = match backend.open_files.get_mut(&req_inner.device_io_request.file_id) {
        None => {
            warn!("No file to query directory");
            Err(NtStatus::NO_SUCH_FILE)
        }
        Some(open_file) if req_inner.initial_query == 0 => {
            match open_file
                .listing
                .as_mut()
                .map_or(Ok(None), DirectoryListing::next_entry)
            {
                Ok(entry) => entry.ok_or(NtStatus::NO_MORE_FILES),
                // An error is not the end of the listing, so the server does not take a partial listing for a complete one.
                Err(error) => {
                    warn!(%error, "Query directory error");
                    Err(status_from_errno(error))
                }
            }
        }
        Some(open_file) => {
            // An initial query restarts enumeration, so a failed or single-entry query must not resume an earlier listing.
            open_file.listing = None;
            match RelativePath::parse(&req_inner.path) {
                Ok(path) => start_query(&backend.file_base, open_file, &path)
                    .unwrap_or_else(|error| {
                        warn!(%error, "Query directory error");
                        None
                    })
                    .ok_or(NtStatus::NO_SUCH_FILE),
                Err(InvalidPath) => {
                    warn!(path = ?req_inner.path, "Rejected a query path outside the shared directory");
                    Err(NtStatus::OBJECT_NAME_INVALID)
                }
            }
        }
    };
    return make_query_dir_resp(entry, req_inner.device_io_request, req_inner.file_info_class_lvl);

    /// Returns the first match for `path`, where a final `*` component enumerates the directory that holds it.
    fn start_query(
        file_base: &str,
        open_file: &mut OpenFile,
        path: &RelativePath,
    ) -> nix::Result<Option<DirectoryEntry>> {
        let root = Root::open(file_base)?;
        let (parent, name) = path.open_parent(&root)?;
        if name == "*" {
            let mut listing = DirectoryListing::open(parent, path.is_in_root())?;
            let entry = listing.next_entry()?;
            open_file.listing = Some(listing);
            return Ok(entry);
        }

        let stat = fstatat(parent.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
        Ok(Some(DirectoryEntry {
            name: path.file_name().unwrap_or_default().to_owned(),
            stat,
        }))
    }
}

fn make_create_drive_resp(
    device_io_request: DeviceIoRequest,
    create_disposition: CreateDisposition,
    file_id: u32,
) -> PduResult<Vec<SvcMessage>> {
    let io_response = DeviceIoResponse::new(device_io_request, NtStatus::SUCCESS);
    let information = if create_disposition == CreateDisposition::FILE_OPEN_IF {
        Information::FILE_OPENED
    } else if create_disposition == CreateDisposition::FILE_OVERWRITE_IF {
        Information::FILE_OVERWRITTEN
    } else {
        Information::FILE_SUPERSEDED
    };
    let res = RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
        device_io_reply: io_response,
        file_id,
        information,
    });
    Ok(vec![SvcMessage::from(res)])
}
// in fact, index only needs to be different, so it is ok
#[expect(clippy::arithmetic_side_effects)]
pub(crate) fn create_drive(
    backend: &mut NixRdpdrBackend,
    req_inner: DeviceCreateRequest,
) -> PduResult<Vec<SvcMessage>> {
    let file_id = backend.file_id;
    backend.file_id += 1;

    let opened = if backend.max_open_files <= backend.open_files.len() {
        warn!("Rejected a create request beyond the open file limit");
        Err(NtStatus::INSUFFICIENT_RESOURCES)
    } else {
        match RelativePath::parse(&req_inner.path) {
            Ok(path) => open_file(&backend.file_base, &req_inner, &path).and_then(|file| {
                let identity = FileIdentity::of(&fstat(file.as_fd()).map_err(status_from_errno)?);
                // [MS-FSA] 2.1.5.1 Server Requests an Open of a File refuses a file marked for deletion.
                if backend.pending_deletes.contains(&identity) {
                    return Err(NtStatus::DELETE_PENDING);
                }
                Ok((file, path, identity))
            }),
            Err(InvalidPath) => {
                warn!(path = ?req_inner.path, "Rejected a create path outside the shared directory");
                Err(NtStatus::OBJECT_NAME_INVALID)
            }
        }
    };

    match opened {
        Ok((file, path, identity)) => {
            trace!(file_id, ?path, "Opened drive file");
            backend.open_files.insert(
                file_id,
                OpenFile {
                    file,
                    path,
                    identity,
                    listing: None,
                },
            );
            make_create_drive_resp(req_inner.device_io_request, req_inner.create_disposition, file_id)
        }
        Err(status) => {
            let io_response = DeviceIoResponse::new(req_inner.device_io_request, status);
            let res = RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
                device_io_reply: io_response,
                file_id,
                information: Information::empty(),
            });
            Ok(vec![SvcMessage::from(res)])
        }
    }
}

/// Opens or creates `path` beneath the shared directory as `req` asks.
fn open_file(file_base: &str, req: &DeviceCreateRequest, path: &RelativePath) -> Result<std::fs::File, NtStatus> {
    let root = Root::open(file_base).map_err(|error| {
        warn!(%error, "Open shared directory error");
        NtStatus::UNSUCCESSFUL
    })?;
    let is_directory_request = req.create_options.contains(CreateOptions::FILE_DIRECTORY_FILE);
    // [MS-SMB2] 2.2.13 SMB2 CREATE Request makes a directory request invalid with FILE_NON_DIRECTORY_FILE or a replacing disposition.
    if is_directory_request
        && (req.create_options.contains(CreateOptions::FILE_NON_DIRECTORY_FILE)
            || !matches!(
                req.create_disposition,
                CreateDisposition::FILE_CREATE | CreateDisposition::FILE_OPEN | CreateDisposition::FILE_OPEN_IF
            ))
    {
        warn!(disposition = ?req.create_disposition, "Rejected an invalid directory create request");
        return Err(NtStatus::INVALID_PARAMETER);
    }
    let (parent, name) = path.open_parent(&root).map_err(|error| {
        warn!(%error, ?path, "Open parent directory error");
        status_from_parent_errno(error)
    })?;

    // Statuses follow [MS-FSA] 2.1.5.1 Server Requests an Open of a File and 2.1.5.1.2 Open of an Existing File.
    let flags = match fstatat(parent.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(stat) if is_directory(&stat) => {
            if req.create_options.contains(CreateOptions::FILE_NON_DIRECTORY_FILE) {
                warn!("Attempt to open a directory as a file");
                return Err(NtStatus::FILE_IS_A_DIRECTORY);
            }
            // An existing directory cannot be created, overwritten or superseded.
            if !matches!(
                req.create_disposition,
                CreateDisposition::FILE_OPEN | CreateDisposition::FILE_OPEN_IF
            ) {
                warn!(disposition = ?req.create_disposition, "Attempt to replace an existing directory");
                return Err(if path.file_name().is_none() {
                    NtStatus::ACCESS_DENIED
                } else {
                    NtStatus::OBJECT_NAME_COLLISION
                });
            }
            OFlag::O_RDONLY | OFlag::O_DIRECTORY
        }
        Ok(_) if is_directory_request => {
            warn!("Attempt to open a file as a directory");
            return Err(if req.create_disposition == CreateDisposition::FILE_CREATE {
                NtStatus::OBJECT_NAME_COLLISION
            } else {
                NtStatus::NOT_A_DIRECTORY
            });
        }
        Err(Errno::ENOENT) if is_directory_request => {
            if req.create_disposition == CreateDisposition::FILE_OPEN {
                return Err(NtStatus::NO_SUCH_FILE);
            }
            mkdirat(parent.as_fd(), name, Mode::from_bits_truncate(0o777)).map_err(|error| {
                warn!(%error, ?path, "Create directory error");
                status_from_errno(error)
            })?;
            OFlag::O_RDONLY | OFlag::O_DIRECTORY
        }
        Err(error) if error != Errno::ENOENT => {
            warn!(%error, ?path, "Look up file error");
            return Err(status_from_errno(error));
        }
        _ => match req.create_disposition {
            CreateDisposition::FILE_OPEN_IF => OFlag::O_CREAT | OFlag::O_RDWR,
            CreateDisposition::FILE_CREATE => OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_RDWR,
            CreateDisposition::FILE_SUPERSEDE => OFlag::O_CREAT | OFlag::O_TRUNC | OFlag::O_RDWR,
            CreateDisposition::FILE_OPEN => OFlag::O_RDONLY,
            CreateDisposition::FILE_OVERWRITE => OFlag::O_TRUNC | OFlag::O_RDWR,
            CreateDisposition::FILE_OVERWRITE_IF => OFlag::O_CREAT | OFlag::O_TRUNC | OFlag::O_RDWR,
            disposition => {
                warn!(?disposition, "Unsupported create disposition");
                return Err(NtStatus::UNSUCCESSFUL);
            }
        },
    };
    root.open_at(parent.as_fd(), name, flags, Mode::from_bits_truncate(0o666))
        .map(std::fs::File::from)
        .map_err(|error| {
            warn!(%error, ?name, "Open file error");
            status_from_errno(error)
        })
}

pub(crate) fn process_dependent_file(
    backend: &mut NixRdpdrBackend,
    request: DeviceIoRequest,
    error_fx: impl Fn(DeviceIoRequest) -> PduResult<Vec<SvcMessage>>,
    fx: impl Fn(&mut std::fs::File, DeviceIoRequest) -> PduResult<Vec<SvcMessage>>,
) -> PduResult<Vec<SvcMessage>> {
    match backend.open_files.get_mut(&request.file_id) {
        None => error_fx(request),
        Some(open_file) => fx(&mut open_file.file, request),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_error(factory: &NixRdpdrBackendFactory) -> NixRdpdrBackendFactoryError {
        let Err(error) = factory.build_rdpdr_backend() else {
            panic!("the factory accepted an invalid configuration");
        };
        *error
            .downcast_ref::<NixRdpdrBackendFactoryError>()
            .expect("a factory configuration error")
    }

    #[test]
    fn the_factory_rejects_reserved_and_duplicate_drive_ids() {
        let factory = NixRdpdrBackendFactory::new("/tmp".to_owned());
        assert_eq!(
            build_error(&factory.clone().with_drive(0, "zero".to_owned())),
            NixRdpdrBackendFactoryError::ReservedDeviceId
        );
        assert_eq!(
            build_error(&factory.with_drive(1, "a".to_owned()).with_drive(1, "b".to_owned())),
            NixRdpdrBackendFactoryError::DuplicateDeviceId(1)
        );
    }

    #[test]
    fn the_factory_rejects_empty_and_nul_names() {
        let factory = NixRdpdrBackendFactory::new("/tmp".to_owned());
        assert_eq!(
            build_error(&factory.clone().with_drive(1, String::new())),
            NixRdpdrBackendFactoryError::InvalidDriveName(1)
        );
        assert_eq!(
            build_error(&factory.clone().with_drive(2, "home\0x".to_owned())),
            NixRdpdrBackendFactoryError::InvalidDriveName(2)
        );
        assert_eq!(
            build_error(&factory.with_printer(String::new(), PrintTarget::DefaultPrinter)),
            NixRdpdrBackendFactoryError::InvalidPrinterName
        );
    }

    fn header(file_id: u32, major_function: MajorFunction) -> DeviceIoRequest {
        DeviceIoRequest {
            device_id: 1,
            file_id,
            completion_id: 0,
            major_function,
            minor_function: MinorFunction::from(0),
        }
    }

    /// Opens the shared directory, returning the status and file ID of the Device Create Response.
    fn open_share(backend: &mut NixRdpdrBackend) -> (NtStatus, u32) {
        let request = DeviceCreateRequest {
            device_io_request: header(0, MajorFunction::Create),
            desired_access: DesiredAccess::empty(),
            allocation_size: 0,
            file_attributes: FileAttributes::empty(),
            shared_access: SharedAccess::empty(),
            create_disposition: CreateDisposition::FILE_OPEN,
            create_options: CreateOptions::empty(),
            path: r"\".to_owned(),
        };
        let response = create_drive(backend, request).expect("complete create");
        let response = response[0].encode_unframed_pdu().expect("encode create response");
        // RDPDR_HEADER (4 bytes), DeviceId and CompletionId precede IoStatus, and FileId follows it
        // ([MS-RDPEFS] 2.2.1.5.1 Device Create Response (DR_CREATE_RSP)).
        (
            NtStatus::from(u32::from_le_bytes(response[12..16].try_into().expect("status"))),
            u32::from_le_bytes(response[16..20].try_into().expect("file ID")),
        )
    }

    #[test]
    fn the_server_cannot_hold_more_files_open_than_the_cap() {
        let mut backend = NixRdpdrBackend::new(std::env::temp_dir().to_str().expect("UTF-8 path").to_owned());
        backend.max_open_files = 2;

        let (status, first) = open_share(&mut backend);
        assert_eq!(status, NtStatus::SUCCESS);
        assert_eq!(open_share(&mut backend).0, NtStatus::SUCCESS);
        // oracle: mutant cap check removed or off by one
        assert_eq!(open_share(&mut backend).0, NtStatus::INSUFFICIENT_RESOURCES);

        close_device(
            &mut backend,
            DeviceCloseRequest {
                device_io_request: header(first, MajorFunction::Close),
            },
        )
        .expect("complete close");
        assert_eq!(open_share(&mut backend).0, NtStatus::SUCCESS);
    }

    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/backend.rs
    #[test]
    fn reset_closes_the_files_the_server_forgot() {
        let mut backend = NixRdpdrBackend::new(std::env::temp_dir().to_str().expect("UTF-8 path").to_owned());
        backend.max_open_files = 1;
        assert_eq!(open_share(&mut backend).0, NtStatus::SUCCESS);

        backend.reset().expect("reset");

        assert_eq!(open_share(&mut backend).0, NtStatus::SUCCESS);
    }

    #[test]
    fn the_printer_takes_an_id_no_drive_uses() {
        let product = NixRdpdrBackendFactory::new("/tmp".to_owned())
            .with_drive(DEFAULT_PRINTER_DEVICE_ID, "taken".to_owned())
            .with_printer("printer".to_owned(), PrintTarget::DefaultPrinter)
            .build_rdpdr_backend()
            .expect("valid configuration");
        assert_eq!(
            product.printer().expect("printer").device_id(),
            DEFAULT_PRINTER_DEVICE_ID - 1
        );
    }
}
