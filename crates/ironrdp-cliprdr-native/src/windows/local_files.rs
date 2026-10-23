//! Files offered from the local clipboard to the remote, and the answers to the remote's
//! `FileContentsRequest`s for them.

use core::cell::RefCell;
use core::ffi::c_void;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use ironrdp_cliprdr::MAX_LOCKED_FILE_LISTS;
use ironrdp_cliprdr::pdu::{FileContentsFlags, FileContentsRequest, FileContentsResponse, FileDescriptor};
use tracing::debug;
use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::System::Com::{IDataObject, IStream, STREAM_SEEK_SET, TYMED_HGLOBAL, TYMED_ISTREAM};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::ReleaseStgMedium;

use crate::windows::file_list::{LocalFileEntry, MAX_FILE_CONTENTS_CHUNK, is_directory, read_file_contents};
use crate::windows::utils::format_etc;

/// Format lists awaiting their response kept at most.
const MAX_PENDING_OFFERS: usize = 16;

/// Files on the local clipboard.
#[derive(Debug)]
pub(crate) enum LocalFileSource {
    /// Files and directories from `CF_HDROP`, read from disk.
    Paths(Vec<LocalFileEntry>),
    /// Files another application offers as `FileGroupDescriptorW` and `FileContents`
    /// (e.g. e-mail attachments or files inside an archive), read through its data object.
    Virtual(VirtualFiles),
}

#[derive(Debug)]
pub(crate) struct VirtualFiles {
    data_object: IDataObject,
    file_contents_format: u16,
    /// Each sent descriptor with its `lindex` in the source data object, which differs from
    /// the index in the list sent when entries were left out.
    files: Vec<(FileDescriptor, i32)>,
    /// Last stream opened, reused while the remote reads the same file in chunks.
    open_stream: RefCell<Option<(i32, IStream)>>,
}

impl VirtualFiles {
    pub(crate) fn new(data_object: IDataObject, file_contents_format: u16, files: Vec<(FileDescriptor, i32)>) -> Self {
        Self {
            data_object,
            file_contents_format,
            files,
            open_stream: RefCell::new(None),
        }
    }

    fn read(&self, request: &FileContentsRequest, descriptor: &FileDescriptor, lindex: i32) -> Option<Vec<u8>> {
        if request.flags.contains(FileContentsFlags::SIZE) {
            if let Some(size) = descriptor.file_size {
                return Some(size.to_le_bytes().to_vec());
            }
        }

        if let Some(stream) = self.cached_stream(lindex) {
            return read_stream(&stream, request);
        }

        let format = format_etc(self.file_contents_format, lindex, TYMED_ISTREAM.0 | TYMED_HGLOBAL.0);

        // SAFETY: `format` is a valid FORMATETC without a target device.
        let mut medium = match unsafe { self.data_object.GetData(&format) } {
            Ok(medium) => medium,
            Err(error) => {
                debug!(%error, lindex, "Failed to get local virtual file contents");
                return None;
            }
        };

        let data = if medium.tymed == TYMED_ISTREAM.0.cast_unsigned() {
            // SAFETY: `tymed` says `pstm` is the active member. Cloning adds the reference kept
            // past `ReleaseStgMedium`.
            let stream = unsafe { (*medium.u.pstm).clone() };
            stream.and_then(|stream| {
                let data = read_stream(&stream, request);
                *self.open_stream.borrow_mut() = Some((lindex, stream));
                data
            })
        } else if medium.tymed == TYMED_HGLOBAL.0.cast_unsigned() {
            // SAFETY: `tymed` says `hGlobal` is the active member.
            let hglobal = unsafe { medium.u.hGlobal };
            // SAFETY: The handle stays valid until `ReleaseStgMedium` below.
            unsafe { with_locked(hglobal, |contents| read_slice(contents, request)) }.flatten()
        } else {
            None
        };

        // SAFETY: `medium` came from `GetData` and is released once.
        unsafe { ReleaseStgMedium(&mut medium) };

        data
    }

    fn cached_stream(&self, lindex: i32) -> Option<IStream> {
        self.open_stream
            .borrow()
            .as_ref()
            .filter(|(open_lindex, _)| *open_lindex == lindex)
            .map(|(_, stream)| stream.clone())
    }
}

impl LocalFileSource {
    /// Descriptors to send with `ClipboardMessage::SendInitiateFileCopy`.
    pub(crate) fn descriptors(&self) -> Vec<FileDescriptor> {
        match self {
            Self::Paths(entries) => entries.iter().map(|entry| entry.descriptor.clone()).collect(),
            Self::Virtual(virtual_files) => virtual_files
                .files
                .iter()
                .map(|(descriptor, _)| descriptor.clone())
                .collect(),
        }
    }

    fn file_contents(&self, request: &FileContentsRequest) -> FileContentsResponse<'static> {
        let Ok(file_index) = usize::try_from(request.index) else {
            return FileContentsResponse::new_error(request.stream_id);
        };

        match self {
            Self::Paths(entries) => match entries.get(file_index) {
                Some(entry) => read_file_contents(request, entry),
                None => FileContentsResponse::new_error(request.stream_id),
            },
            Self::Virtual(virtual_files) => {
                let Some((descriptor, lindex)) = virtual_files.files.get(file_index) else {
                    return FileContentsResponse::new_error(request.stream_id);
                };

                if is_directory(descriptor) {
                    return FileContentsResponse::new_error(request.stream_id);
                }

                match virtual_files.read(request, descriptor, *lindex) {
                    Some(data) => FileContentsResponse::new_data_response(request.stream_id, data),
                    None => FileContentsResponse::new_error(request.stream_id),
                }
            }
        }
    }
}

/// Current local file offer, plus the snapshots the remote locked.
///
/// [MS-RDPECLIP] 3.1.5.4.6: a request carrying a `clipDataId` is served from the files that
/// were on the clipboard when that lock was taken, even if the clipboard changed since.
///
/// What the remote knows of is not the latest offer made here: a format list is only sent once
/// `Cliprdr` processes it, after which the remote answers it with a Format List Response, before
/// any Lock or request about it ([MS-RDPECLIP] 3.1.5.2.2). Offers are therefore queued as they
/// are made, and the one the remote acknowledged last is what locks snapshot and requests read.
#[derive(Debug, Default)]
pub(crate) struct LocalFiles {
    /// Offers sent, oldest first, waiting for their Format List Response (`None`: no files).
    offered: VecDeque<Option<Rc<LocalFileSource>>>,
    /// Offers dropped from the front of `offered` to bound it, whose responses are still due.
    dropped_offers: usize,
    /// The offer the remote acknowledged last.
    current: Option<Rc<LocalFileSource>>,
    locked: HashMap<u32, Rc<LocalFileSource>>,
}

impl LocalFiles {
    /// Records a format list sent to the remote, with the files it offers if any.
    pub(crate) fn offer(&mut self, source: Option<Rc<LocalFileSource>>) {
        if MAX_PENDING_OFFERS <= self.offered.len() {
            // The remote is not answering. The oldest offer goes, but its response is still
            // counted, so that later responses keep pairing with the right offers.
            self.offered.pop_front();
            self.dropped_offers = self.dropped_offers.saturating_add(1);
        }
        self.offered.push_back(source);
    }

    /// Handles the Format List Response for the oldest offer.
    pub(crate) fn acknowledge(&mut self, accepted: bool) {
        if self.dropped_offers != 0 {
            // The acknowledged offer's files are gone: serve none rather than another offer's.
            self.dropped_offers -= 1;
            self.current = None;
            return;
        }

        let Some(offered) = self.offered.pop_front() else {
            return;
        };

        // A rejected list leaves the remote with no local files, as in `Cliprdr`.
        self.current = if accepted { offered } else { None };
    }

    pub(crate) fn clear(&mut self) {
        self.offered.clear();
        self.dropped_offers = 0;
        self.current = None;
        self.locked.clear();
    }

    pub(crate) fn lock(&mut self, clip_data_id: u32) {
        let Some(current) = self.current.as_ref() else {
            // Locks also arrive when there are no local files, see `Cliprdr`.
            return;
        };

        // The same bound as `Cliprdr`, which tracks the same locks.
        if MAX_LOCKED_FILE_LISTS <= self.locked.len() && !self.locked.contains_key(&clip_data_id) {
            debug!(
                clip_data_id,
                "Too many locked local file lists; not snapshotting this one"
            );
            return;
        }

        self.locked.insert(clip_data_id, Rc::clone(current));
    }

    pub(crate) fn unlock(&mut self, clip_data_id: u32) {
        self.locked.remove(&clip_data_id);
    }

    pub(crate) fn file_contents(&self, request: &FileContentsRequest) -> FileContentsResponse<'static> {
        // Without a snapshot for its `clipDataId`, `Cliprdr` validated the request against the
        // current list, so it is served from there as well.
        let source = request
            .data_id
            .and_then(|data_id| self.locked.get(&data_id))
            .or(self.current.as_ref());

        match source {
            Some(source) => source.file_contents(request),
            None => {
                debug!(
                    stream_id = request.stream_id,
                    "File contents requested with no local files"
                );
                FileContentsResponse::new_error(request.stream_id)
            }
        }
    }
}

fn read_stream(stream: &IStream, request: &FileContentsRequest) -> Option<Vec<u8>> {
    if request.flags.contains(FileContentsFlags::SIZE) {
        let mut stat = Default::default();
        // SAFETY: `stat` is valid for writes; no name is requested, so nothing to free.
        unsafe { stream.Stat(&mut stat, windows::Win32::System::Com::STATFLAG_NONAME) }.ok()?;
        return Some(stat.cbSize.to_le_bytes().to_vec());
    }

    let position = i64::try_from(request.position).ok()?;
    // SAFETY: Plain COM call.
    unsafe { stream.Seek(position, STREAM_SEEK_SET, None) }.ok()?;

    let len = usize::try_from(request.requested_size.min(MAX_FILE_CONTENTS_CHUNK)).ok()?;
    let mut data = vec![0u8; len];
    let mut filled = 0usize;

    while let Some(rest) = data.get_mut(filled..).filter(|rest| !rest.is_empty()) {
        let mut read = 0u32;
        let wanted = u32::try_from(rest.len()).ok()?;
        // SAFETY: `rest` is valid for writes of `wanted` bytes.
        let result = unsafe { stream.Read(rest.as_mut_ptr().cast::<c_void>(), wanted, Some(&raw mut read)) };
        if result.is_err() {
            debug!(error = %windows::core::Error::from(result), "Failed to read local virtual file");
            return None;
        }
        if read == 0 {
            break;
        }
        filled += usize::try_from(read).ok()?;
    }

    data.truncate(filled);
    Some(data)
}

fn read_slice(contents: &[u8], request: &FileContentsRequest) -> Option<Vec<u8>> {
    if request.flags.contains(FileContentsFlags::SIZE) {
        let size = u64::try_from(contents.len()).ok()?;
        return Some(size.to_le_bytes().to_vec());
    }

    let start = usize::try_from(request.position).ok()?;
    let len = usize::try_from(request.requested_size.min(MAX_FILE_CONTENTS_CHUNK)).ok()?;
    let end = start.saturating_add(len).min(contents.len());

    Some(contents.get(start..end).unwrap_or_default().to_vec())
}

/// Reads `format` from `data_object` as an `HGLOBAL`.
pub(crate) fn hglobal_data(data_object: &IDataObject, format: u16) -> Option<Vec<u8>> {
    let format = format_etc(format, -1, TYMED_HGLOBAL.0);

    // SAFETY: `format` is a valid FORMATETC without a target device.
    let mut medium = match unsafe { data_object.GetData(&format) } {
        Ok(medium) => medium,
        Err(error) => {
            debug!(%error, format = format.cfFormat, "Failed to get local clipboard data");
            return None;
        }
    };

    let data = if medium.tymed == TYMED_HGLOBAL.0.cast_unsigned() {
        // SAFETY: `tymed` says `hGlobal` is the active member.
        let hglobal = unsafe { medium.u.hGlobal };
        // SAFETY: The handle stays valid until released below.
        unsafe { with_locked(hglobal, <[u8]>::to_vec) }
    } else {
        None
    };

    // SAFETY: `medium` came from `GetData` and is released once.
    unsafe { ReleaseStgMedium(&mut medium) };

    data
}

/// Runs `f` on the contents of `hglobal`, or returns `None` if it can't be locked.
///
/// # Safety
///
/// `hglobal` must be a valid global memory handle for the duration of the call.
unsafe fn with_locked<R>(hglobal: HGLOBAL, f: impl FnOnce(&[u8]) -> R) -> Option<R> {
    // SAFETY: The caller guarantees the handle is valid.
    let size = unsafe { GlobalSize(hglobal) };
    // SAFETY: Same as above.
    let ptr = unsafe { GlobalLock(hglobal) }.cast::<u8>().cast_const();
    if ptr.is_null() {
        return None;
    }

    // SAFETY: The locked block holds `size` bytes.
    let result = f(unsafe { core::slice::from_raw_parts(ptr, size) });

    // SAFETY: Locked just above.
    let _ = unsafe { GlobalUnlock(hglobal) };

    Some(result)
}
