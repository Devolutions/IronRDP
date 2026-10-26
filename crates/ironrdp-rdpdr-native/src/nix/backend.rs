use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
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
    /// Files to delete when their last open handle closes ([MS-FSA] 2.1.5.5 Server Requests Closing an Open), each with the name it was marked through.
    ///
    /// That name is removed rather than the path of whichever handle closes last, so a file with several hard links loses the link the server marked.
    /// [MS-FSA] marks each link separately, while one name is kept per file here: marking through another link replaces it, and every link of a marked file refuses new opens.
    ///
    /// [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/d142c93a-72bc-4b05-9d96-8e00371c3308
    pending_deletes: HashMap<FileIdentity, RelativePath>,
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
    /// The directories from the shared directory down to the one holding this file, as the server last reached it.
    ///
    /// A directory holding open files is found by these rather than by path, so another spelling of a name or a local rename does not hide one.
    ancestors: Vec<FileIdentity>,
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
            // UNTESTED: only an entry removed since enumeration started is skipped, and any other error fails the listing.
            // Skipping an entry that cannot be inspected, as in a directory the process may not search, would make the directory look empty, so a delete could be marked on it.
            // The tests can run as root, which no permission stops, so after changing this, check that only ENOENT is skipped.
            let stat = match fstatat(self.directory.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW) {
                Ok(stat) => stat,
                Err(Errno::ENOENT) => continue,
                Err(error) => return Err(error),
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
            pending_deletes: HashMap::new(),
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
            && let Some(path) = self.pending_deletes.remove(&open_file.identity)
            && let Err(error) = remove_file(&self.file_base, &path, open_file.identity)
        {
            warn!(%error, "Delete on close error");
        }
    }

    /// Returns an ID no open file holds, counting up and wrapping around after the last one.
    ///
    /// At most `max_open_files` files are open, far fewer than there are IDs, so the search ends.
    fn next_file_id(&mut self) -> u32 {
        loop {
            let file_id = self.file_id;
            self.file_id = self.file_id.wrapping_add(1);
            if !self.open_files.contains_key(&file_id) {
                return file_id;
            }
        }
    }

    /// Closes every open file as if the server had closed it, so pending deletes run.
    fn close_all_files(&mut self) {
        let file_ids = self.open_files.keys().copied().collect::<Vec<_>>();
        for file_id in file_ids {
            self.close_file(file_id);
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

impl Drop for NixRdpdrBackend {
    /// The session is over, so the handles the server left open are closed and their pending deletes run.
    fn drop(&mut self) {
        self.close_all_files();
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
        self.close_all_files();
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
    // A failed delete on close is only logged, and the close still reports success.
    // [MS-FSA] 2.1.5.5 Server Requests Closing an Open defines no failure status, and Windows does not report a failed delete on close to the caller closing the handle either.
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
                    let is_delete_pending = backend.pending_deletes.contains_key(identity);
                    let standard_info = FileStandardInformation {
                        allocation_size: i64::try_from(meta.size()).unwrap(),
                        end_of_file: i64::try_from(meta.size()).unwrap(),
                        // A directory has one link, whatever Unix counts for `.` and subdirectories, and the link marked for deletion is not counted ([MS-FSA] 2.1.5.12.27 FileStandardInformation).
                        number_of_links: if meta.is_dir() {
                            1
                        } else {
                            u32::try_from(meta.nlink()).unwrap()
                        }
                        .saturating_sub(u32::from(is_delete_pending)),
                        delete_pending: if is_delete_pending {
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
            backend.pending_deletes.contains_key(&open_file.identity)
                || backend
                    .open_files
                    .values()
                    .any(|other| other.ancestors.contains(&open_file.identity))
        });
    // A rename target that is open cannot be replaced, so the open files are collected before one of them is borrowed mutably.
    let open_identities = if matches!(req_inner.set_buffer, FileInformationClass::Rename(_)) {
        backend
            .open_files
            .values()
            .map(|open_file| open_file.identity)
            .collect()
    } else {
        HashSet::new()
    };
    let mut renamed = None;
    let status = match backend.open_files.get_mut(&file_id) {
        Some(open_file) => match &req_inner.set_buffer {
            FileInformationClass::Rename(_) if rename_denied => NtStatus::ACCESS_DENIED,
            // The shared directory itself is no name to rename to.
            FileInformationClass::Rename(info) => match RelativePath::parse(&info.file_name)
                .and_then(|to| to.file_name().is_some().then_some(to).ok_or(InvalidPath))
            {
                Ok(to) => match rename(
                    &backend.file_base,
                    open_file,
                    &to,
                    info.replace_if_exists == Boolean::True,
                    &backend.pending_deletes,
                    &open_identities,
                ) {
                    Ok(ancestors) => {
                        renamed = Some((open_file.identity, open_file.path.clone(), to, ancestors));
                        NtStatus::SUCCESS
                    }
                    Err(status) => {
                        trace!(?status, "Rename file failed");
                        status
                    }
                },
                Err(InvalidPath) => {
                    trace!(file_name = ?info.file_name, "Rejected an invalid rename target");
                    NtStatus::OBJECT_NAME_INVALID
                }
            },
            // The allocation size is only a hint, so there is nothing to do.
            FileInformationClass::Allocation(_) => NtStatus::SUCCESS,
            FileInformationClass::Disposition(info) => {
                match set_delete_pending(open_file, &mut backend.pending_deletes, info.delete_pending != 0) {
                    Ok(()) => NtStatus::SUCCESS,
                    Err(error) => {
                        trace!(%error, "Set delete pending failed");
                        status_from_errno(error)
                    }
                }
            }
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
    // Every handle opened through the renamed link follows it, so a later delete removes this file and not whatever takes the old name.
    // A handle whose path no longer names the file was opened through that link, possibly spelled differently on a case-insensitive filesystem.
    // A handle opened through another hard link of the file still finds it at its path, and keeps that link.
    if let Some((identity, from, to, ancestors)) = renamed {
        for open_file in backend
            .open_files
            .values_mut()
            .filter(|open_file| open_file.identity == identity)
        {
            if open_file.path == from || !names_file(&backend.file_base, &open_file.path, identity) {
                open_file.path = to.clone();
                open_file.ancestors = ancestors.clone();
            }
        }
    }
    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveSetInformationResponse(
        ClientDriveSetInformationResponse::new(&req_inner, status).map_err(|e| encode_err!(e))?,
    ))])
}

/// Renames the file `open_file` holds to `to` ([MS-FSA] 2.1.5.15.12 FileRenameInformation), and returns the directories leading to `to`.
///
/// The handle's path is resolved again, so it fails with STATUS_NO_SUCH_FILE once that name refers to another file than the handle's.
/// An existing `to` is a collision unless `replace_if_exists` is set.
/// A directory `to`, or one with open handles, is never replaced, and one marked for deletion fails with STATUS_DELETE_PENDING.
/// A `to` naming the source itself, as a case-only rename on a case-insensitive filesystem does, is not a collision, and a `to` naming another hard link of the source removes the source link.
/// The existence check and the rename are separate calls, so a local process creating `to` in between is overwritten.
/// The target directory is opened as a create opens it, so a path through a directory marked for deletion is refused with STATUS_DELETE_PENDING.
///
/// [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/87f86c9b-6c2a-4803-84b7-131a74a434fa
fn rename(
    file_base: &str,
    open_file: &OpenFile,
    to: &RelativePath,
    replace_if_exists: bool,
    pending_deletes: &HashMap<FileIdentity, RelativePath>,
    open_identities: &HashSet<FileIdentity>,
) -> Result<Vec<FileIdentity>, NtStatus> {
    let root = Root::open(file_base).map_err(status_from_errno)?;
    let (from_parent, from_name) = open_file.path.open_parent(&root).map_err(status_from_parent_errno)?;
    let (to_parent, to_name, to_ancestors) = to.open_parent_with_ancestors(&root).map_err(status_from_parent_errno)?;
    let to_ancestors = to_ancestors.iter().map(FileIdentity::of).collect::<Vec<_>>();
    if to_ancestors
        .iter()
        .any(|ancestor| pending_deletes.contains_key(ancestor))
    {
        return Err(NtStatus::DELETE_PENDING);
    }
    let to_parent_identity = FileIdentity::of(&fstat(to_parent.as_fd()).map_err(status_from_errno)?);
    let source = fstatat(from_parent.as_fd(), from_name, AtFlags::AT_SYMLINK_NOFOLLOW).map_err(status_from_errno)?;
    if FileIdentity::of(&source) != open_file.identity {
        return Err(NtStatus::NO_SUCH_FILE);
    }
    match fstatat(to_parent.as_fd(), to_name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(target) if FileIdentity::of(&target) == open_file.identity => {
            // `to` is either the source link under another spelling, as a case- or normalization-insensitive filesystem resolves it, or another hard link of the same file.
            // POSIX rename leaves two hard links of one file both in place, while [MS-FSA] removes the source link.
            // A link lives in one directory, so in another directory `to` is another link; in the same one it is only if an entry is stored under exactly that name.
            // Otherwise the rename goes ahead, which at most changes the spelling of the only link and never removes it.
            let same_directory =
                FileIdentity::of(&fstat(from_parent.as_fd()).map_err(status_from_errno)?) == to_parent_identity;
            let is_another_link = source.st_nlink >= 2
                && (!same_directory
                    || from_name != to_name
                        && contains_entry(to_parent.as_fd(), to_name).map_err(status_from_errno)?);
            if is_another_link {
                unlinkat(from_parent.as_fd(), from_name, UnlinkatFlags::NoRemoveDir).map_err(status_from_errno)?;
                return Ok(to_ancestors);
            }
        }
        Ok(_) if !replace_if_exists => return Err(NtStatus::OBJECT_NAME_COLLISION),
        Ok(target) if is_directory(&target) => return Err(NtStatus::ACCESS_DENIED),
        Ok(target) if pending_deletes.contains_key(&FileIdentity::of(&target)) => {
            return Err(NtStatus::DELETE_PENDING);
        }
        Ok(target) if open_identities.contains(&FileIdentity::of(&target)) => return Err(NtStatus::ACCESS_DENIED),
        Ok(_) | Err(Errno::ENOENT) => {}
        Err(error) => return Err(status_from_errno(error)),
    }
    renameat(from_parent.as_fd(), from_name, to_parent.as_fd(), to_name).map_err(status_from_errno)?;
    Ok(to_ancestors)
}

/// Whether `directory` holds an entry stored under exactly the bytes of `name`, whatever spellings the filesystem treats as equal.
fn contains_entry(directory: BorrowedFd<'_>, name: &str) -> nix::Result<bool> {
    // A fresh descriptor reads the directory from its start without moving the caller's read position.
    let directory = nix::fcntl::openat(
        directory,
        ".",
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    for entry in Dir::from_fd(directory)? {
        if entry?.file_name().to_bytes() == name.as_bytes() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Marks `open_file` for deletion when its last handle closes, or clears the mark.
///
/// A directory is marked only when it is empty ([MS-FSA] 2.1.5.15.3 FileDispositionInformation).
///
/// [MS-FSA]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/386d9ec5-e0f6-4853-b175-c05be01419e0
fn set_delete_pending(
    open_file: &OpenFile,
    pending_deletes: &mut HashMap<FileIdentity, RelativePath>,
    delete_pending: bool,
) -> nix::Result<()> {
    if !delete_pending {
        pending_deletes.remove(&open_file.identity);
        return Ok(());
    }
    // The shared directory cannot be removed through the share, and a mark on it would refuse every create in it until its handles close.
    if open_file.path.file_name().is_none() {
        return Err(Errno::EACCES);
    }
    if is_directory(&fstat(open_file.file.as_fd())?) {
        // The open directory itself is read, not whatever its path names now.
        // A duplicate of the handle would share its read position, so a second mark would read on from the end of the first.
        let directory = nix::fcntl::openat(
            open_file.file.as_fd(),
            ".",
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
            Mode::empty(),
        )?;
        let mut listing = DirectoryListing::open(directory, true)?;
        if listing.next_entry()?.is_some() {
            return Err(Errno::ENOTEMPTY);
        }
    }
    pending_deletes.insert(open_file.identity, open_file.path.clone());
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

/// Whether `path` still names the file `identity`.
fn names_file(file_base: &str, path: &RelativePath, identity: FileIdentity) -> bool {
    Root::open(file_base)
        .and_then(|root| {
            let (parent, name) = path.open_parent(&root)?;
            fstatat(parent.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW)
        })
        .is_ok_and(|stat| FileIdentity::of(&stat) == identity)
}

/// Removes the file or empty directory `path` names, unless that name now refers to another file than `identity`.
fn remove_file(file_base: &str, path: &RelativePath, identity: FileIdentity) -> nix::Result<()> {
    let root = Root::open(file_base)?;
    let (parent, name) = path.open_parent(&root)?;
    let stat = fstatat(parent.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW)?;
    if FileIdentity::of(&stat) != identity {
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
            match RelativePath::parse_query(&req_inner.path) {
                // Only a missing name or an empty listing is no match; any other error is reported, so the server does not take it for an empty directory.
                Ok((directory, pattern)) => match start_query(&backend.file_base, open_file, &directory, pattern) {
                    Ok(entry) => entry.ok_or(NtStatus::NO_SUCH_FILE),
                    Err(error) => {
                        trace!(%error, "Initial query directory failed");
                        Err(status_from_errno(error))
                    }
                },
                Err(InvalidPath) => {
                    trace!(path = ?req_inner.path, "Rejected an invalid query path");
                    Err(NtStatus::OBJECT_NAME_INVALID)
                }
            }
        }
    };
    return make_query_dir_resp(entry, req_inner.device_io_request, req_inner.file_info_class_lvl);

    /// Returns the first match for `pattern` in `directory`, where a `*` pattern enumerates the directory.
    fn start_query(
        file_base: &str,
        open_file: &mut OpenFile,
        directory: &RelativePath,
        pattern: &str,
    ) -> nix::Result<Option<DirectoryEntry>> {
        let root = Root::open(file_base)?;
        let opened = directory.open_directory(&root)?;
        if pattern == "*" {
            let mut listing = DirectoryListing::open(opened, directory.file_name().is_none())?;
            let entry = listing.next_entry()?;
            open_file.listing = Some(listing);
            return Ok(entry);
        }

        let stat = fstatat(opened.as_fd(), pattern, AtFlags::AT_SYMLINK_NOFOLLOW)?;
        Ok(Some(DirectoryEntry {
            name: pattern.to_owned(),
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
pub(crate) fn create_drive(
    backend: &mut NixRdpdrBackend,
    req_inner: DeviceCreateRequest,
) -> PduResult<Vec<SvcMessage>> {
    let file_id = backend.next_file_id();

    let opened = if backend.max_open_files <= backend.open_files.len() {
        warn!("Rejected a create request beyond the open file limit");
        Err(NtStatus::INSUFFICIENT_RESOURCES)
    } else {
        match RelativePath::parse(&req_inner.path) {
            Ok(path) => open_file(&backend.file_base, &req_inner, &path, &backend.pending_deletes)
                .map(|(file, identity, ancestors)| (file, path, identity, ancestors)),
            Err(InvalidPath) => {
                trace!(path = ?req_inner.path, "Rejected an invalid create path");
                Err(NtStatus::OBJECT_NAME_INVALID)
            }
        }
    };

    match opened {
        Ok((file, path, identity, ancestors)) => {
            trace!(file_id, ?path, "Opened drive file");
            backend.open_files.insert(
                file_id,
                OpenFile {
                    file,
                    path,
                    identity,
                    ancestors,
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

/// Opens or creates `path` beneath the shared directory as `req` asks, and returns it with its identity and the directories leading to it.
///
/// A file marked for deletion is refused before it is overwritten, so a failed open leaves its contents intact.
fn open_file(
    file_base: &str,
    req: &DeviceCreateRequest,
    path: &RelativePath,
    pending_deletes: &HashMap<FileIdentity, RelativePath>,
) -> Result<(std::fs::File, FileIdentity, Vec<FileIdentity>), NtStatus> {
    let root = Root::open(file_base).map_err(|error| {
        warn!(%error, "Open shared directory error");
        NtStatus::UNSUCCESSFUL
    })?;
    // [MS-SMB2] 2.2.13 SMB2 CREATE Request makes any disposition but the six it lists invalid.
    if !matches!(
        req.create_disposition,
        CreateDisposition::FILE_SUPERSEDE
            | CreateDisposition::FILE_OPEN
            | CreateDisposition::FILE_CREATE
            | CreateDisposition::FILE_OPEN_IF
            | CreateDisposition::FILE_OVERWRITE
            | CreateDisposition::FILE_OVERWRITE_IF
    ) {
        warn!(disposition = ?req.create_disposition, "Rejected an invalid create disposition");
        return Err(NtStatus::INVALID_PARAMETER);
    }
    let is_directory_request = req.create_options.contains(CreateOptions::FILE_DIRECTORY_FILE);
    // It also makes a directory request invalid with FILE_NON_DIRECTORY_FILE or a replacing disposition.
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
    let (parent, name, ancestors) = path.open_parent_with_ancestors(&root).map_err(|error| {
        trace!(%error, ?path, "Open parent directory failed");
        status_from_parent_errno(error)
    })?;
    let ancestors = ancestors.iter().map(FileIdentity::of).collect::<Vec<_>>();
    // [MS-FSA] 2.1.5.1 Server Requests an Open of a File refuses a path through any directory marked for deletion, so nothing is created in it.
    // A local process can still add entries to such a directory, and its delete then fails when its last handle closes.
    if ancestors.iter().any(|ancestor| pending_deletes.contains_key(ancestor)) {
        return Err(NtStatus::DELETE_PENDING);
    }

    // Statuses follow [MS-FSA] 2.1.5.1 Server Requests an Open of a File and 2.1.5.1.2 Open of an Existing File.
    let flags = match fstatat(parent.as_fd(), name, AtFlags::AT_SYMLINK_NOFOLLOW) {
        Ok(stat) if is_directory(&stat) => {
            if req.create_options.contains(CreateOptions::FILE_NON_DIRECTORY_FILE) {
                trace!("Refused to open a directory as a file");
                return Err(NtStatus::FILE_IS_A_DIRECTORY);
            }
            // An existing directory cannot be created, overwritten or superseded.
            if !matches!(
                req.create_disposition,
                CreateDisposition::FILE_OPEN | CreateDisposition::FILE_OPEN_IF
            ) {
                trace!(disposition = ?req.create_disposition, "Refused to replace an existing directory");
                return Err(if path.file_name().is_none() {
                    NtStatus::ACCESS_DENIED
                } else {
                    NtStatus::OBJECT_NAME_COLLISION
                });
            }
            OFlag::O_RDONLY | OFlag::O_DIRECTORY
        }
        Ok(_) if is_directory_request => {
            trace!("Refused to open a file as a directory");
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
                trace!(%error, ?path, "Create directory failed");
                status_from_errno(error)
            })?;
            OFlag::O_RDONLY | OFlag::O_DIRECTORY
        }
        Err(error) if error != Errno::ENOENT => {
            trace!(%error, ?path, "Look up file failed");
            return Err(status_from_errno(error));
        }
        _ => match req.create_disposition {
            CreateDisposition::FILE_OPEN_IF => OFlag::O_CREAT | OFlag::O_RDWR,
            CreateDisposition::FILE_CREATE => OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_RDWR,
            CreateDisposition::FILE_SUPERSEDE => OFlag::O_CREAT | OFlag::O_RDWR,
            CreateDisposition::FILE_OPEN => OFlag::O_RDONLY,
            CreateDisposition::FILE_OVERWRITE => OFlag::O_RDWR,
            CreateDisposition::FILE_OVERWRITE_IF => OFlag::O_CREAT | OFlag::O_RDWR,
            // Refused before the lookup.
            _ => return Err(NtStatus::INVALID_PARAMETER),
        },
    };
    let file = root
        .open_at(parent.as_fd(), name, flags, Mode::from_bits_truncate(0o666))
        .map_err(|error| {
            trace!(%error, ?name, "Open file failed");
            status_from_errno(error)
        })?;
    let stat = fstat(file.as_fd()).map_err(status_from_errno)?;
    let identity = FileIdentity::of(&stat);
    // Windows refuses to open a file marked for deletion, where [MS-FSA] 2.1.5.1 Server Requests an Open of a File states this only for the directories on the path.
    if pending_deletes.contains_key(&identity) {
        return Err(NtStatus::DELETE_PENDING);
    }
    // Truncating only once the open can no longer fail keeps a refused file intact, where O_TRUNC would empty it first.
    // Like O_TRUNC, this leaves anything but a regular file, such as a FIFO, unchanged.
    let is_regular_file = SFlag::from_bits_truncate(stat.st_mode) & SFlag::S_IFMT == SFlag::S_IFREG;
    if is_regular_file
        && matches!(
            req.create_disposition,
            CreateDisposition::FILE_SUPERSEDE
                | CreateDisposition::FILE_OVERWRITE
                | CreateDisposition::FILE_OVERWRITE_IF
        )
    {
        ftruncate(file.as_fd(), 0).map_err(|error| {
            warn!(%error, ?name, "Truncate file error");
            status_from_errno(error)
        })?;
    }
    Ok((std::fs::File::from(file), identity, ancestors))
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

    // oracle: reference crates/ironrdp-rdpdr-native/src/windows/file_table.rs
    #[test]
    fn a_new_file_never_takes_the_id_of_an_open_one() {
        let mut backend = NixRdpdrBackend::new(std::env::temp_dir().to_str().expect("UTF-8 path").to_owned());
        let (status, first) = open_share(&mut backend);
        assert_eq!(status, NtStatus::SUCCESS);
        // The counter has come round to an ID that is still open.
        backend.file_id = first;

        let (status, second) = open_share(&mut backend);

        assert_eq!(status, NtStatus::SUCCESS);
        assert_ne!(second, first);
    }

    // oracle: mutant file ID counter overflows instead of wrapping
    #[test]
    fn file_ids_wrap_around_after_the_last_one() {
        let mut backend = NixRdpdrBackend::new(std::env::temp_dir().to_str().expect("UTF-8 path").to_owned());
        backend.file_id = u32::MAX;

        assert_eq!(open_share(&mut backend), (NtStatus::SUCCESS, u32::MAX));
        assert_eq!(open_share(&mut backend), (NtStatus::SUCCESS, 0));
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
