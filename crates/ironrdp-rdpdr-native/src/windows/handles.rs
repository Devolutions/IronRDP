//! Owning Windows directory handles used as roots for RDPDR path resolution.

use core::fmt;
use core::mem::offset_of;
use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;

use windows::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows::Wdk::Storage::FileSystem::{
    FILE_ALLOCATION_INFORMATION, FILE_BASIC_INFORMATION, FILE_DIRECTORY_FILE, FILE_DISPOSITION_INFORMATION,
    FILE_INFORMATION_CLASS, FILE_OPEN, FILE_RENAME_INFORMATION, FILE_STANDARD_INFORMATION,
    FILE_SYNCHRONOUS_IO_NONALERT, FS_INFORMATION_CLASS, FileAllocationInformation, FileAttributeTagInformation,
    FileBasicInformation, FileDispositionInformation, FileEndOfFileInformation, FileRenameInformation,
    FileStandardInformation, FileStreamInformation, NTCREATEFILE_CREATE_DISPOSITION, NTCREATEFILE_CREATE_OPTIONS,
    NtCreateFile, NtFlushBuffersFile, NtFsControlFile, NtQueryDirectoryFile, NtQueryInformationFile,
    NtQueryVolumeInformationFile, NtReadFile, NtSetInformationFile, NtWriteFile,
};
use windows::Wdk::System::SystemServices::FILE_END_OF_FILE_INFORMATION;
use windows::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_IO_PENDING, HANDLE, NTSTATUS, OBJ_CASE_INSENSITIVE,
    OBJ_DONT_REPARSE, OBJECT_ATTRIBUTE_FLAGS, UNICODE_STRING,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ACCESS_RIGHTS, FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_TAG_INFO,
    FILE_FLAGS_AND_ATTRIBUTES, FILE_NOTIFY_CHANGE, FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_DELETE,
    FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, LOCKFILE_EXCLUSIVE_LOCK,
    LOCKFILE_FAIL_IMMEDIATELY, LockFileEx, ReadDirectoryChangesW, SYNCHRONIZE, UnlockFileEx,
};
use windows::Win32::System::IO::{
    CancelIoEx, GetOverlappedResult, IO_STATUS_BLOCK, OVERLAPPED, OVERLAPPED_0, OVERLAPPED_0_0,
};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess};
use windows::core::HRESULT;

use crate::windows::path::RelativePath;

const MAX_VOLUME_QUERY_SIZE: usize = 64 * 1024;
const MAX_STREAM_QUERY_SIZE: usize = 64 * 1024;
const MAX_DIRECTORY_QUERY_SIZE: usize = 128 * 1024;
const MAX_DIRECTORY_NOTIFY_SIZE: usize = 64 * 1024;
const FILE_ALLOCATED_RANGE_BUFFER_SIZE: usize = 16;
const SYNCHRONOUS_DIRECTORY_CREATE_OPTIONS: u32 = FILE_DIRECTORY_FILE.0 | FILE_SYNCHRONOUS_IO_NONALERT.0;
const ASYNCHRONOUS_DIRECTORY_CREATE_OPTIONS: u32 = FILE_DIRECTORY_FILE.0;
pub(crate) const FILE_OBJECTID_BUFFER_SIZE: usize = 64;
pub(super) const FILE_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
pub(crate) const RENAME_DESTINATION_DIRECTORY_ACCESS: FILE_ACCESS_RIGHTS = FILE_ACCESS_RIGHTS(
    FILE_TRAVERSE.0 | FILE_ADD_FILE.0 | FILE_ADD_SUBDIRECTORY.0 | FILE_READ_ATTRIBUTES.0 | SYNCHRONIZE.0,
);
const DIRECTORY_TRAVERSAL_ACCESS: FILE_ACCESS_RIGHTS =
    FILE_ACCESS_RIGHTS(FILE_TRAVERSE.0 | FILE_READ_ATTRIBUTES.0 | SYNCHRONIZE.0);

/// A validated native path for a logical-volume root.
pub(crate) struct RootDirectory {
    nt_path: Vec<u16>,
}

impl RootDirectory {
    /// Validates the native path of a logical-volume root.
    pub(crate) fn open(path: &Path) -> Result<Self, OpenDirectoryError> {
        Ok(Self {
            nt_path: volume_root_nt_path(path).ok_or(OpenDirectoryError::InvalidVolumeRoot)?,
        })
    }

    /// Opens a descendant directory relative to this redirected-volume root.
    ///
    /// Each native open is rooted at the preceding directory handle, so no
    /// host path join can replace the configured root.
    pub(crate) fn open_relative_directory(
        &self,
        path: &RelativePath,
    ) -> Result<Option<DirectoryHandle>, OpenDirectoryError> {
        self.open_relative_directory_with_final_options(path, SYNCHRONOUS_DIRECTORY_CREATE_OPTIONS)
    }

    /// Opens an independently-owned directory handle that can receive an
    /// overlapped change-notification request.
    pub(crate) fn open_relative_directory_for_notification(
        &self,
        path: &RelativePath,
    ) -> Result<Option<DirectoryHandle>, OpenDirectoryError> {
        if path.components().next().is_none() {
            return self
                .open_traversal_root_with_options(
                    FILE_READ_DATA | DIRECTORY_TRAVERSAL_ACCESS | SYNCHRONIZE,
                    ASYNCHRONOUS_DIRECTORY_CREATE_OPTIONS,
                )
                .map(Some);
        }

        self.open_relative_directory_with_final_options(path, ASYNCHRONOUS_DIRECTORY_CREATE_OPTIONS)
    }

    fn open_relative_directory_with_final_options(
        &self,
        path: &RelativePath,
        final_create_options: u32,
    ) -> Result<Option<DirectoryHandle>, OpenDirectoryError> {
        let mut current_directory = self.open_traversal_root()?;
        let final_component = path.components().count().saturating_sub(1);
        for (index, component) in path.components().enumerate() {
            let mut component = component.encode_utf16().collect::<Vec<_>>();
            let create_options = if index == final_component {
                final_create_options
            } else {
                SYNCHRONOUS_DIRECTORY_CREATE_OPTIONS
            };
            let child = DirectoryHandle(open_directory_with_access_and_options(
                current_directory.as_raw(),
                &mut component,
                FILE_READ_DATA | DIRECTORY_TRAVERSAL_ACCESS | SYNCHRONIZE,
                create_options,
            )?);
            current_directory = child;
        }

        Ok(Some(current_directory))
    }

    /// Opens a non-directory entry relative to this redirected-volume root.
    pub(crate) fn open_relative_file(
        &self,
        path: &RelativePath,
        options: FileOpenOptions,
    ) -> Result<(FileHandle, usize), OpenFileError> {
        let Some(file_name) = path.components().next_back() else {
            return self.open_root_file(options);
        };
        let parent_directory = self
            .open_relative_parent_directory(path, DIRECTORY_TRAVERSAL_ACCESS)
            .map_err(OpenFileError::Directory)?;
        let mut file_name = file_name.encode_utf16().collect::<Vec<_>>();
        open_file(parent_directory.as_raw(), &mut file_name, options)
    }

    /// Opens the parent directory of a validated path beneath this volume root.
    pub(crate) fn open_relative_parent_directory(
        &self,
        path: &RelativePath,
        final_access: FILE_ACCESS_RIGHTS,
    ) -> Result<DirectoryHandle, OpenDirectoryError> {
        let mut components = path.components().collect::<Vec<_>>();
        let _ = components.pop();
        if components.is_empty() {
            return self.open_traversal_root_with_access(final_access);
        }

        let mut current_directory = self.open_traversal_root()?;
        let final_component = components.len() - 1;
        for (index, component) in components.into_iter().enumerate() {
            let mut component = component.encode_utf16().collect::<Vec<_>>();
            let desired_access = if index == final_component {
                final_access
            } else {
                DIRECTORY_TRAVERSAL_ACCESS
            };
            let child = DirectoryHandle(open_directory_with_access(
                current_directory.as_raw(),
                &mut component,
                desired_access,
            )?);
            current_directory = child;
        }

        Ok(current_directory)
    }

    fn open_traversal_root(&self) -> Result<DirectoryHandle, OpenDirectoryError> {
        self.open_traversal_root_with_access(FILE_READ_DATA | DIRECTORY_TRAVERSAL_ACCESS)
    }

    fn open_traversal_root_with_access(
        &self,
        desired_access: FILE_ACCESS_RIGHTS,
    ) -> Result<DirectoryHandle, OpenDirectoryError> {
        self.open_traversal_root_with_options(desired_access, SYNCHRONOUS_DIRECTORY_CREATE_OPTIONS)
    }

    fn open_traversal_root_with_options(
        &self,
        desired_access: FILE_ACCESS_RIGHTS,
        create_options: u32,
    ) -> Result<DirectoryHandle, OpenDirectoryError> {
        let mut nt_path = self.nt_path.clone();
        open_directory_with_access_and_options(HANDLE::default(), &mut nt_path, desired_access, create_options)
            .map(DirectoryHandle)
    }

    fn open_root_file(&self, options: FileOpenOptions) -> Result<(FileHandle, usize), OpenFileError> {
        // Explorer opens a redirected volume for directory listing and then
        // immediately queries its metadata. MSTSC's root file object carries
        // this access even when the list request omits it; grant it only to
        // independently opened volume roots, never to descendant files.
        let options = FileOpenOptions {
            desired_access: options.desired_access | FILE_READ_ATTRIBUTES.0,
            ..options
        };
        let mut nt_path = self.nt_path.clone();
        open_file(HANDLE::default(), &mut nt_path, options)
    }
}

impl fmt::Debug for RootDirectory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RootDirectory").field(&"<owned>").finish()
    }
}

/// A non-cloneable directory handle opened below a redirected volume.
pub(crate) struct DirectoryHandle(HANDLE);

// SAFETY: Windows handles are process-wide kernel object references. This
// wrapper has unique ownership and only closes its handle on drop, so moving it
// between the client and RDP worker threads cannot create concurrent access.
unsafe impl Send for DirectoryHandle {}

impl DirectoryHandle {
    pub(crate) fn as_raw(&self) -> HANDLE {
        self.0
    }

    pub(crate) fn into_file_handle(self) -> FileHandle {
        let directory = core::mem::ManuallyDrop::new(self);
        FileHandle(directory.0)
    }
}

impl Drop for DirectoryHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` was returned by NtCreateFile and remains exclusively owned by this guard.
        if let Err(error) = unsafe { CloseHandle(self.0) } {
            tracing::warn!(error = %error, "Failed to close redirected-drive directory handle");
        }
    }
}

/// A non-cloneable local filesystem handle.
#[derive(Debug)]
pub(crate) struct FileHandle(HANDLE);

// SAFETY: Windows handles are process-wide kernel object references. This
// wrapper has unique ownership and only closes its handle on drop, so moving it
// between the client and RDP worker threads cannot create concurrent access.
unsafe impl Send for FileHandle {}

/// Synchronous completion from a native offset-based file I/O request.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NativeIoCompletion {
    pub(crate) status: NTSTATUS,
    pub(crate) transferred: usize,
}

/// Synchronous completion from a filesystem control request.
#[derive(Debug)]
pub(crate) struct NativeFsControlCompletion {
    pub(crate) status: NTSTATUS,
    pub(crate) output: Vec<u8>,
}

/// The request buffer cannot be represented by the native API.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NativeIoLengthError;

impl FileHandle {
    pub(crate) fn as_raw(&self) -> HANDLE {
        self.0
    }

    /// Duplicates this process-local handle for a deferred worker.
    ///
    /// The worker owns the duplicate so a file-table mutation cannot close a
    /// handle while native work is still using it.
    pub(crate) fn try_clone(&self) -> Result<Self, windows::core::Error> {
        // SAFETY: `GetCurrentProcess` takes no arguments and returns the
        // pseudo-handle for the current process.
        let process = unsafe { GetCurrentProcess() };
        let mut handle = HANDLE::default();

        // SAFETY: `self.0` is a valid handle owned by this process and `handle`
        // is writable storage for the duplicated process-local handle.
        unsafe {
            DuplicateHandle(
                process,
                self.0,
                process,
                core::ptr::addr_of_mut!(handle),
                0,
                false,
                DUPLICATE_SAME_ACCESS,
            )
        }?;

        Ok(Self(handle))
    }

    /// Applies a byte-range lock without allowing the RDP event loop to block.
    pub(crate) fn lock_range(&self, offset: u64, length: u64, exclusive: bool) -> Result<(), windows::core::Error> {
        let mut overlapped = Self::overlapped_at(offset);
        let (length_low, length_high) = split_u64(length);
        let mut flags = LOCKFILE_FAIL_IMMEDIATELY;
        if exclusive {
            flags |= LOCKFILE_EXCLUSIVE_LOCK;
        }

        // SAFETY: `self.0` is exclusively owned, `overlapped` remains valid
        // for the synchronous call, and the byte range is represented exactly
        // by the split low/high length values.
        unsafe {
            LockFileEx(
                self.0,
                flags,
                Some(0),
                length_low,
                length_high,
                core::ptr::addr_of_mut!(overlapped),
            )
        }
    }

    /// Applies each byte-range lock, releasing earlier ranges if a later lock
    /// cannot be acquired.
    pub(crate) fn lock_ranges(&self, ranges: &[(u64, u64)], exclusive: bool) -> Result<(), windows::core::Error> {
        let mut acquired = Vec::with_capacity(ranges.len());
        for &(offset, length) in ranges {
            if let Err(error) = self.lock_range(offset, length, exclusive) {
                for &(locked_offset, locked_length) in acquired.iter().rev() {
                    if let Err(rollback_error) = self.unlock_range(locked_offset, locked_length) {
                        tracing::warn!(
                            error = %rollback_error,
                            "Failed to roll back a partially acquired RDPDR byte-range lock"
                        );
                    }
                }
                return Err(error);
            }
            acquired.push((offset, length));
        }

        Ok(())
    }

    /// Releases a byte-range lock previously applied through [`Self::lock_range`].
    pub(crate) fn unlock_range(&self, offset: u64, length: u64) -> Result<(), windows::core::Error> {
        let mut overlapped = Self::overlapped_at(offset);
        let (length_low, length_high) = split_u64(length);

        // SAFETY: `self.0` is exclusively owned, `overlapped` remains valid
        // for the synchronous call, and the byte range is represented exactly
        // by the split low/high length values.
        unsafe {
            UnlockFileEx(
                self.0,
                Some(0),
                length_low,
                length_high,
                core::ptr::addr_of_mut!(overlapped),
            )
        }
    }

    /// Releases every requested byte-range lock, reporting the first native
    /// failure after attempting the complete request.
    pub(crate) fn unlock_ranges(&self, ranges: &[(u64, u64)]) -> Result<(), windows::core::Error> {
        let mut first_error = None;
        for &(offset, length) in ranges {
            if let Err(error) = self.unlock_range(offset, length)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }

        first_error.map_or(Ok(()), Err)
    }

    /// Waits synchronously for a directory change on a worker-owned handle.
    ///
    /// Cancellation is issued with `CancelSynchronousIo` against the worker
    /// thread, so this must never run on the RDP static-channel thread.
    #[cfg(test)]
    pub(crate) fn read_directory_changes(
        &self,
        watch_tree: bool,
        completion_filter: u32,
    ) -> Result<Vec<u8>, windows::core::Error> {
        let mut buffer = vec![0; MAX_DIRECTORY_NOTIFY_SIZE];
        let mut bytes_returned = 0;

        // SAFETY: `self.0` is a worker-owned duplicate, the buffer and byte
        // count remain valid for this synchronous call, and the caller
        // validates the completion filter before starting the worker.
        unsafe {
            ReadDirectoryChangesW(
                self.0,
                buffer.as_mut_ptr().cast(),
                u32::try_from(buffer.len()).expect("bounded directory notification buffer fits in u32"),
                watch_tree,
                FILE_NOTIFY_CHANGE(completion_filter),
                Some(core::ptr::addr_of_mut!(bytes_returned)),
                None,
                None,
            )
        }?;
        buffer.truncate(usize::try_from(bytes_returned).expect("Win32 byte count fits in usize"));
        Ok(buffer)
    }

    /// Registers an asynchronous directory-change request before returning.
    ///
    /// The returned operation owns the handle, buffer, and `OVERLAPPED` state
    /// until the worker has observed either a change or cancellation.
    pub(crate) fn begin_directory_changes(
        self,
        watch_tree: bool,
        completion_filter: u32,
    ) -> Result<Box<DirectoryChange>, windows::core::Error> {
        // SAFETY: no security attributes or name are supplied, and the returned
        // event is owned by `DirectoryChange` until the I/O operation ends.
        let event = unsafe { CreateEventW(None, true, false, None) }?;
        let mut operation = Box::new(DirectoryChange {
            handle: self,
            event,
            buffer: vec![0; MAX_DIRECTORY_NOTIFY_SIZE],
            overlapped: OVERLAPPED {
                Internal: 0,
                InternalHigh: 0,
                Anonymous: OVERLAPPED_0 {
                    Anonymous: OVERLAPPED_0_0 {
                        Offset: 0,
                        OffsetHigh: 0,
                    },
                },
                hEvent: event,
            },
        });

        // SAFETY: the asynchronous directory handle, its buffer, and its
        // OVERLAPPED state all remain owned by `operation` until completion.
        match unsafe {
            ReadDirectoryChangesW(
                operation.handle.as_raw(),
                operation.buffer.as_mut_ptr().cast(),
                u32::try_from(operation.buffer.len()).expect("bounded directory notification buffer fits in u32"),
                watch_tree,
                FILE_NOTIFY_CHANGE(completion_filter),
                None,
                Some(core::ptr::addr_of_mut!(operation.overlapped)),
                None,
            )
        } {
            Ok(()) => {
                tracing::debug!(watch_tree, completion_filter, "Registered RDPDR directory notification");
                Ok(operation)
            }
            Err(error) if error.code() == HRESULT::from_win32(ERROR_IO_PENDING.0) => {
                tracing::debug!(
                    watch_tree,
                    completion_filter,
                    "Registered pending RDPDR directory notification"
                );
                Ok(operation)
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn query_basic_information(&self) -> Result<FILE_BASIC_INFORMATION, NTSTATUS> {
        self.query_information(FileBasicInformation)
    }

    fn overlapped_at(offset: u64) -> OVERLAPPED {
        let (offset_low, offset_high) = split_u64(offset);
        OVERLAPPED {
            Internal: 0,
            InternalHigh: 0,
            Anonymous: OVERLAPPED_0 {
                Anonymous: OVERLAPPED_0_0 {
                    Offset: offset_low,
                    OffsetHigh: offset_high,
                },
            },
            hEvent: HANDLE::default(),
        }
    }

    pub(crate) fn query_standard_information(&self) -> Result<FILE_STANDARD_INFORMATION, NTSTATUS> {
        self.query_information(FileStandardInformation)
    }

    pub(crate) fn query_attribute_tag_information(&self) -> Result<FILE_ATTRIBUTE_TAG_INFO, NTSTATUS> {
        self.query_information(FileAttributeTagInformation)
    }

    /// Returns the native FILE_STREAM_INFORMATION buffer for every data stream
    /// attached to this file or directory.
    pub(crate) fn query_stream_information(&self) -> Result<Vec<u8>, NTSTATUS> {
        self.query_information_buffer(FileStreamInformation, MAX_STREAM_QUERY_SIZE)
    }

    pub(crate) fn query_volume_information(
        &self,
        information_class: FS_INFORMATION_CLASS,
    ) -> Result<Vec<u8>, NTSTATUS> {
        let mut information = vec![0; MAX_VOLUME_QUERY_SIZE];
        let mut io_status = IO_STATUS_BLOCK::default();

        // SAFETY:
        // - `self.0` is exclusively owned by this guard and remains open for
        //   the duration of the synchronous request.
        // - `information` and `io_status` remain valid writable storage for
        //   the duration of the call.
        let status = unsafe {
            NtQueryVolumeInformationFile(
                self.0,
                core::ptr::addr_of_mut!(io_status),
                information.as_mut_ptr().cast(),
                u32::try_from(information.len()).expect("bounded volume query buffer fits in u32"),
                information_class,
            )
        };
        if status.0 < 0 {
            return Err(status);
        }

        information.truncate(io_status.Information.min(information.len()));
        Ok(information)
    }

    pub(crate) fn query_directory_information(
        &self,
        information_class: FILE_INFORMATION_CLASS,
        pattern: Option<&[u16]>,
        restart_scan: bool,
    ) -> Result<Vec<u8>, NTSTATUS> {
        let mut information = vec![0; MAX_DIRECTORY_QUERY_SIZE];
        let mut io_status = IO_STATUS_BLOCK::default();
        let unicode_pattern = pattern.map(unicode_string).transpose()?;

        // SAFETY:
        // - `self.0` is exclusively owned by this guard and remains open for
        //   the duration of the synchronous request.
        // - `information`, `io_status`, and `unicode_pattern` remain valid
        //   for the duration of the native call.
        // - `return_single_entry` limits the response to one RDPDR directory
        //   entry, whose cursor the kernel keeps on the directory handle.
        let status = unsafe {
            NtQueryDirectoryFile(
                self.0,
                None,
                None,
                None,
                core::ptr::addr_of_mut!(io_status),
                information.as_mut_ptr().cast(),
                u32::try_from(information.len()).expect("bounded directory query buffer fits in u32"),
                information_class,
                true,
                unicode_pattern.as_ref().map(core::ptr::from_ref),
                restart_scan,
            )
        };
        if status.0 < 0 {
            return Err(status);
        }

        information.truncate(io_status.Information.min(information.len()));
        Ok(information)
    }

    pub(crate) fn set_end_of_file(&self, end_of_file: i64) -> Result<(), NTSTATUS> {
        self.set_information(
            FileEndOfFileInformation,
            &FILE_END_OF_FILE_INFORMATION { EndOfFile: end_of_file },
        )
    }

    pub(crate) fn set_basic_information(&self, information: FILE_BASIC_INFORMATION) -> Result<(), NTSTATUS> {
        self.set_information(FileBasicInformation, &information)
    }

    pub(crate) fn set_delete_pending(&self, delete_pending: bool) -> Result<(), NTSTATUS> {
        self.set_information(
            FileDispositionInformation,
            &FILE_DISPOSITION_INFORMATION {
                DeleteFile: delete_pending,
            },
        )
    }

    pub(crate) fn rename(
        &self,
        root_directory: HANDLE,
        file_name: &str,
        replace_if_exists: bool,
    ) -> Result<(), NTSTATUS> {
        let file_name = file_name.encode_utf16().collect::<Vec<_>>();
        let file_name_length = file_name
            .len()
            .checked_mul(size_of::<u16>())
            .and_then(|length| u32::try_from(length).ok())
            .ok_or(NTSTATUS(i32::from_ne_bytes(0xC000_000Du32.to_ne_bytes())))?;
        let header_length = offset_of!(FILE_RENAME_INFORMATION, FileName);
        let buffer_length = header_length
            .checked_add(usize::try_from(file_name_length).expect("u32 fits in usize"))
            .map(|length| length.max(size_of::<FILE_RENAME_INFORMATION>()))
            .ok_or(NTSTATUS(i32::from_ne_bytes(0xC000_000Du32.to_ne_bytes())))?;
        let mut information = vec![0; buffer_length];
        // The variable-length buffer has byte alignment, so encode its fixed
        // native prefix by field offset instead of casting it to the typed struct.
        information[offset_of!(FILE_RENAME_INFORMATION, Anonymous)] = u8::from(replace_if_exists);
        let root_directory_offset = offset_of!(FILE_RENAME_INFORMATION, RootDirectory);
        information[root_directory_offset..][..size_of::<HANDLE>()]
            .copy_from_slice(&root_directory.0.addr().to_le_bytes());
        let file_name_length_offset = offset_of!(FILE_RENAME_INFORMATION, FileNameLength);
        information[file_name_length_offset..][..size_of::<u32>()].copy_from_slice(&file_name_length.to_le_bytes());
        for (destination, code_unit) in information[header_length..]
            .chunks_exact_mut(size_of::<u16>())
            .zip(file_name)
        {
            destination.copy_from_slice(&code_unit.to_le_bytes());
        }

        self.set_information_buffer(
            FileRenameInformation,
            information.as_ptr().cast(),
            u32::try_from(buffer_length).expect("bounded rename buffer fits in u32"),
        )
    }

    pub(crate) fn set_allocation_size(&self, allocation_size: i64) -> Result<(), NTSTATUS> {
        self.set_information(
            FileAllocationInformation,
            &FILE_ALLOCATION_INFORMATION {
                AllocationSize: allocation_size,
            },
        )
    }

    pub(crate) fn read_at(&self, offset: i64, buffer: &mut [u8]) -> Result<NativeIoCompletion, NativeIoLengthError> {
        let mut io_status = IO_STATUS_BLOCK::default();
        let length = u32::try_from(buffer.len()).map_err(|_| NativeIoLengthError)?;

        // SAFETY:
        // - `self.0` is exclusively owned by this guard and remains open for
        //   the duration of the synchronous request.
        // - `io_status`, `offset`, and `buffer` are valid for the call.
        // - an explicit `offset` prevents this request from changing or
        //   consuming a shared file cursor.
        let status = unsafe {
            NtReadFile(
                self.0,
                None,
                None,
                None,
                core::ptr::addr_of_mut!(io_status),
                buffer.as_mut_ptr().cast(),
                length,
                Some(core::ptr::addr_of!(offset)),
                None,
            )
        };

        Ok(NativeIoCompletion {
            status,
            transferred: io_status.Information.min(buffer.len()),
        })
    }

    pub(crate) fn write_at(&self, offset: i64, buffer: &[u8]) -> Result<NativeIoCompletion, NativeIoLengthError> {
        let mut io_status = IO_STATUS_BLOCK::default();
        let length = u32::try_from(buffer.len()).map_err(|_| NativeIoLengthError)?;

        // SAFETY:
        // - `self.0` is exclusively owned by this guard and remains open for
        //   the duration of the synchronous request.
        // - `io_status`, `offset`, and `buffer` are valid for the call.
        // - an explicit `offset` prevents this request from changing or
        //   consuming a shared file cursor.
        let status = unsafe {
            NtWriteFile(
                self.0,
                None,
                None,
                None,
                core::ptr::addr_of_mut!(io_status),
                buffer.as_ptr().cast(),
                length,
                Some(core::ptr::addr_of!(offset)),
                None,
            )
        };

        Ok(NativeIoCompletion {
            status,
            transferred: io_status.Information.min(buffer.len()),
        })
    }

    /// Writes at the end of the file without resolving its path again.
    pub(crate) fn write_to_end(&self, buffer: &[u8]) -> Result<NativeIoCompletion, NativeIoLengthError> {
        // NtWriteFile interprets a byte offset of -1 as an append request.
        self.write_at(-1, buffer)
    }

    pub(crate) fn flush(&self) -> Result<(), NTSTATUS> {
        let mut io_status = IO_STATUS_BLOCK::default();

        // SAFETY: `self.0` is exclusively owned by this guard and `io_status`
        // remains valid writable storage for the synchronous request.
        let status = unsafe { NtFlushBuffersFile(self.0, core::ptr::addr_of_mut!(io_status)) };
        if status.0 < 0 {
            return Err(status);
        }

        Ok(())
    }

    /// Queries the file's compression format through a fixed-size FSCTL response.
    pub(crate) fn query_compression_format(&self) -> Result<[u8; 2], NTSTATUS> {
        const FSCTL_GET_COMPRESSION: u32 = 0x0009_003C;
        let mut compression_format = [0; 2];
        self.query_fixed_size_fsctl(FSCTL_GET_COMPRESSION, &mut compression_format)?;
        Ok(compression_format)
    }

    /// Queries the file's integrity metadata through its fixed-size FSCTL response.
    pub(crate) fn query_integrity_information(&self) -> Result<[u8; 16], NTSTATUS> {
        const FSCTL_GET_INTEGRITY_INFORMATION: u32 = 0x0009_027C;
        let mut integrity_information = [0; 16];
        self.query_fixed_size_fsctl(FSCTL_GET_INTEGRITY_INFORMATION, &mut integrity_information)?;
        Ok(integrity_information)
    }

    /// Creates an object ID when absent, then returns the file's 64-byte object-ID buffer.
    pub(crate) fn create_or_get_object_id(&self) -> Result<[u8; FILE_OBJECTID_BUFFER_SIZE], NTSTATUS> {
        const FSCTL_CREATE_OR_GET_OBJECT_ID: u32 = 0x0009_00C0;
        let mut object_id = [0; FILE_OBJECTID_BUFFER_SIZE];
        self.query_fixed_size_fsctl(FSCTL_CREATE_OR_GET_OBJECT_ID, &mut object_id)?;
        Ok(object_id)
    }

    /// Queries allocated byte ranges with a caller-validated range and output
    /// bound. The raw result is retained only for the matching RDPDR response.
    pub(crate) fn query_allocated_ranges(
        &self,
        range: &[u8; FILE_ALLOCATED_RANGE_BUFFER_SIZE],
        output_buffer_length: usize,
    ) -> NativeFsControlCompletion {
        const FSCTL_QUERY_ALLOCATED_RANGES: u32 = 0x0009_40CF;
        let mut output = vec![0; output_buffer_length];
        let mut io_status = IO_STATUS_BLOCK::default();

        // SAFETY:
        // - `self.0` is exclusively owned by this guard and remains open for
        //   the duration of the synchronous request.
        // - `range` is the exact 16-byte FILE_ALLOCATED_RANGE_BUFFER supplied
        //   by a validated RDPDR control request.
        // - `output` is bounded by the caller and remains valid for the call.
        // - `io_status` remains valid for the duration of the call.
        let status = unsafe {
            NtFsControlFile(
                self.0,
                None,
                None,
                None,
                core::ptr::addr_of_mut!(io_status),
                FSCTL_QUERY_ALLOCATED_RANGES,
                Some(range.as_ptr().cast()),
                u32::try_from(range.len()).expect("allocated range request size fits in u32"),
                (!output.is_empty()).then_some(output.as_mut_ptr().cast()),
                u32::try_from(output.len()).expect("bounded allocated range output fits in u32"),
            )
        };
        output.truncate(io_status.Information.min(output.len()));

        NativeFsControlCompletion { status, output }
    }

    fn query_fixed_size_fsctl(&self, control_code: u32, output: &mut [u8]) -> Result<(), NTSTATUS> {
        let mut io_status = IO_STATUS_BLOCK::default();

        // SAFETY:
        // - `self.0` is exclusively owned by this guard and remains open for
        //   the duration of the synchronous request.
        // - each supported fixed-size FSCTL has no input and writes exactly
        //   the required result into the caller-provided `output` buffer.
        // - `io_status` and the output buffer remain valid for the call.
        let status = unsafe {
            NtFsControlFile(
                self.0,
                None,
                None,
                None,
                core::ptr::addr_of_mut!(io_status),
                control_code,
                None,
                0,
                Some(output.as_mut_ptr().cast()),
                u32::try_from(output.len()).expect("fixed FSCTL output size fits in u32"),
            )
        };
        if status.0 < 0 {
            return Err(status);
        }
        if io_status.Information != output.len() {
            return Err(NTSTATUS(i32::from_ne_bytes(0xC000_0001u32.to_ne_bytes())));
        }

        Ok(())
    }

    fn query_information<T: Default>(&self, information_class: FILE_INFORMATION_CLASS) -> Result<T, NTSTATUS> {
        let mut information = T::default();
        let mut io_status = IO_STATUS_BLOCK::default();
        let length = u32::try_from(size_of::<T>()).expect("fixed native file information fits in u32");

        // SAFETY:
        // - `self.0` is exclusively owned by this guard and remains open for
        //   the duration of the synchronous request.
        // - each caller chooses the native structure required by
        //   `information_class`, and `information` is valid writable storage.
        // - `io_status` remains valid for the duration of the call.
        let status = unsafe {
            NtQueryInformationFile(
                self.0,
                core::ptr::addr_of_mut!(io_status),
                core::ptr::addr_of_mut!(information).cast(),
                length,
                information_class,
            )
        };
        if status.0 < 0 {
            return Err(status);
        }

        Ok(information)
    }

    fn query_information_buffer(
        &self,
        information_class: FILE_INFORMATION_CLASS,
        buffer_length: usize,
    ) -> Result<Vec<u8>, NTSTATUS> {
        let mut information = vec![0; buffer_length];
        let mut io_status = IO_STATUS_BLOCK::default();
        let length = u32::try_from(information.len()).expect("bounded native file information fits in u32");

        // SAFETY:
        // - `self.0` is exclusively owned by this guard and remains open for
        //   the duration of the synchronous request.
        // - `information` is writable storage for the requested native
        //   variable-length information class.
        // - `io_status` remains valid for the duration of the call.
        let status = unsafe {
            NtQueryInformationFile(
                self.0,
                core::ptr::addr_of_mut!(io_status),
                information.as_mut_ptr().cast(),
                length,
                information_class,
            )
        };
        if status.0 < 0 {
            return Err(status);
        }

        information.truncate(io_status.Information.min(information.len()));
        Ok(information)
    }

    fn set_information<T>(&self, information_class: FILE_INFORMATION_CLASS, information: &T) -> Result<(), NTSTATUS> {
        let length = u32::try_from(size_of::<T>()).expect("fixed native file information fits in u32");

        self.set_information_buffer(information_class, core::ptr::from_ref(information).cast(), length)
    }

    fn set_information_buffer(
        &self,
        information_class: FILE_INFORMATION_CLASS,
        information: *const core::ffi::c_void,
        length: u32,
    ) -> Result<(), NTSTATUS> {
        let mut io_status = IO_STATUS_BLOCK::default();

        // SAFETY:
        // - `self.0` is exclusively owned by this guard and remains open for
        //   the duration of the synchronous request.
        // - each caller chooses the native structure and buffer required by
        //   `information_class`, and `information` is valid readable storage.
        // - `io_status` remains valid for the duration of the call.
        let status = unsafe {
            NtSetInformationFile(
                self.0,
                core::ptr::addr_of_mut!(io_status),
                information,
                length,
                information_class,
            )
        };
        if status.0 < 0 {
            return Err(status);
        }

        Ok(())
    }
}

/// An asynchronous directory-change request registered on a dedicated handle.
pub(crate) struct DirectoryChange {
    handle: FileHandle,
    event: HANDLE,
    buffer: Vec<u8>,
    overlapped: OVERLAPPED,
}

// SAFETY: the operation has exclusive ownership of its kernel handle, buffer,
// and OVERLAPPED state while it runs on a deferred worker thread.
unsafe impl Send for DirectoryChange {}

impl DirectoryChange {
    pub(crate) fn handle(&self) -> HANDLE {
        self.handle.as_raw()
    }

    pub(crate) fn wait(mut self: Box<Self>) -> Result<Vec<u8>, windows::core::Error> {
        let mut bytes_returned = 0;

        // SAFETY: `self` keeps the file handle, OVERLAPPED state, and output
        // buffer alive until the registered operation has completed.
        unsafe {
            GetOverlappedResult(
                self.handle.as_raw(),
                core::ptr::addr_of_mut!(self.overlapped),
                core::ptr::addr_of_mut!(bytes_returned),
                true,
            )
        }?;
        let bytes_returned = usize::try_from(bytes_returned).expect("Win32 byte count fits in usize");
        let native_bytes_returned = self.overlapped.InternalHigh;
        let notification_bytes = bytes_returned.max(native_bytes_returned);
        self.buffer.truncate(notification_bytes);
        let notification_action = self.buffer.get(4..8).map(|bytes| {
            u32::from_le_bytes(
                bytes
                    .try_into()
                    .expect("the notification action field is exactly four bytes"),
            )
        });
        tracing::debug!(
            notification_bytes,
            ?notification_action,
            "Completed RDPDR directory notification"
        );
        Ok(core::mem::take(&mut self.buffer))
    }
}

impl Drop for DirectoryChange {
    fn drop(&mut self) {
        let mut bytes_returned = 0;

        // SAFETY: the handle, OVERLAPPED state, and output buffer remain valid
        // until GetOverlappedResult observes cancellation or normal completion.
        // This also covers a worker-spawn failure after ReadDirectoryChangesW
        // has registered asynchronous I/O.
        let _ = unsafe { CancelIoEx(self.handle.as_raw(), Some(core::ptr::addr_of_mut!(self.overlapped))) };
        let _ = unsafe {
            GetOverlappedResult(
                self.handle.as_raw(),
                core::ptr::addr_of_mut!(self.overlapped),
                core::ptr::addr_of_mut!(bytes_returned),
                true,
            )
        };

        // SAFETY: `self.event` is exclusively owned by this operation and no
        // longer needed once completion has been observed.
        if let Err(error) = unsafe { CloseHandle(self.event) } {
            tracing::warn!(error = %error, "Failed to close redirected-drive directory notification event");
        }
    }
}

fn split_u64(value: u64) -> (u32, u32) {
    let bytes = value.to_le_bytes();
    (
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
    )
}

impl Drop for FileHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` was returned by NtCreateFile and remains exclusively owned by this guard.
        if let Err(error) = unsafe { CloseHandle(self.0) } {
            tracing::warn!(error = %error, "Failed to close redirected-drive file handle");
        }
    }
}

/// Options for a synchronous, handle-relative native file open.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FileOpenOptions {
    pub(crate) desired_access: u32,
    pub(crate) allocation_size: Option<i64>,
    pub(crate) file_attributes: u32,
    pub(crate) share_access: u32,
    pub(crate) create_disposition: u32,
    pub(crate) create_options: u32,
}

/// Result from a native file open.
#[derive(Clone, Copy, Debug)]
pub(crate) enum OpenFileError {
    /// Opening a parent directory failed.
    Directory(OpenDirectoryError),
    /// The final native open failed.
    NtStatus(NTSTATUS),
}

pub(crate) const FILE_SUPERSEDED_INFORMATION: usize = 0;
pub(crate) const FILE_OPENED_INFORMATION: usize = 1;
pub(crate) const FILE_CREATED_INFORMATION: usize = 2;
pub(crate) const FILE_OVERWRITTEN_INFORMATION: usize = 3;

/// Failure while opening a selected redirected-volume root.
#[derive(Clone, Copy, Debug)]
pub(crate) enum OpenDirectoryError {
    /// The selection is not a logical Windows volume root (`C:\`).
    InvalidVolumeRoot,
    /// The native open failed without disclosing the host path.
    NtStatus(NTSTATUS),
}

impl fmt::Display for OpenDirectoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVolumeRoot => f.write_str("redirected drive must be a logical volume root"),
            Self::NtStatus(status) => write!(
                f,
                "opening redirected-drive root failed with NTSTATUS {:#010X}",
                status.0
            ),
        }
    }
}

impl core::error::Error for OpenDirectoryError {}

fn volume_root_nt_path(path: &Path) -> Option<Vec<u16>> {
    let path = path.as_os_str().encode_wide().collect::<Vec<_>>();

    let is_logical_volume_root = path.len() == 3
        && matches!(path[0], 65..=90 | 97..=122)
        && path[1] == u16::from(b':')
        && path[2] == u16::from(b'\\');
    if !is_logical_volume_root {
        return None;
    }

    let mut nt_path = r"\??\".encode_utf16().collect::<Vec<_>>();
    nt_path.extend(path);
    Some(nt_path)
}

fn unicode_string(value: &[u16]) -> Result<UNICODE_STRING, NTSTATUS> {
    let byte_len = value
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or(NTSTATUS(i32::from_ne_bytes(0xC000_000Du32.to_ne_bytes())))?;

    Ok(UNICODE_STRING {
        Length: byte_len,
        MaximumLength: byte_len,
        Buffer: windows::core::PWSTR::from_raw(value.as_ptr().cast_mut()),
    })
}

fn open_directory_with_access(
    root_directory: HANDLE,
    name: &mut [u16],
    desired_access: FILE_ACCESS_RIGHTS,
) -> Result<HANDLE, OpenDirectoryError> {
    open_directory_with_access_and_options(
        root_directory,
        name,
        desired_access,
        SYNCHRONOUS_DIRECTORY_CREATE_OPTIONS,
    )
}

fn open_directory_with_access_and_options(
    root_directory: HANDLE,
    name: &mut [u16],
    desired_access: FILE_ACCESS_RIGHTS,
    create_options: u32,
) -> Result<HANDLE, OpenDirectoryError> {
    let name_byte_len = name
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or(OpenDirectoryError::InvalidVolumeRoot)?;
    let mut unicode_name = UNICODE_STRING {
        Length: name_byte_len,
        MaximumLength: name_byte_len,
        Buffer: windows::core::PWSTR::from_raw(name.as_mut_ptr()),
    };
    let object_attributes = OBJECT_ATTRIBUTES {
        Length: u32::try_from(size_of::<OBJECT_ATTRIBUTES>()).expect("OBJECT_ATTRIBUTES size fits in u32"),
        RootDirectory: root_directory,
        ObjectName: core::ptr::addr_of_mut!(unicode_name),
        Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
        SecurityDescriptor: core::ptr::null(),
        SecurityQualityOfService: core::ptr::null(),
    };
    let mut handle = HANDLE::default();
    let mut io_status = IO_STATUS_BLOCK::default();

    // SAFETY:
    // - `name` and `unicode_name` stay alive for the entire call.
    // - `object_attributes`, `handle`, and `io_status` are valid writable pointers.
    // - `OBJ_DONT_REPARSE` ensures object-manager resolution fails instead of traversing a reparse point.
    let status = unsafe {
        NtCreateFile(
            core::ptr::addr_of_mut!(handle),
            desired_access,
            core::ptr::addr_of!(object_attributes),
            core::ptr::addr_of_mut!(io_status),
            None,
            FILE_ATTRIBUTE_NORMAL,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            NTCREATEFILE_CREATE_OPTIONS(create_options),
            None,
            0,
        )
    };
    if status.0 < 0 {
        return Err(OpenDirectoryError::NtStatus(status));
    }

    Ok(handle)
}

fn open_file(
    root_directory: HANDLE,
    name: &mut [u16],
    options: FileOpenOptions,
) -> Result<(FileHandle, usize), OpenFileError> {
    let name_byte_len = name
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or(OpenFileError::NtStatus(NTSTATUS(i32::from_ne_bytes(
            0xC000_000Du32.to_ne_bytes(),
        ))))?;
    let mut unicode_name = UNICODE_STRING {
        Length: name_byte_len,
        MaximumLength: name_byte_len,
        Buffer: windows::core::PWSTR::from_raw(name.as_mut_ptr()),
    };
    let object_attributes = OBJECT_ATTRIBUTES {
        Length: u32::try_from(size_of::<OBJECT_ATTRIBUTES>()).expect("OBJECT_ATTRIBUTES size fits in u32"),
        RootDirectory: root_directory,
        ObjectName: core::ptr::addr_of_mut!(unicode_name),
        Attributes: final_file_object_attributes(options.create_options),
        SecurityDescriptor: core::ptr::null(),
        SecurityQualityOfService: core::ptr::null(),
    };
    let mut handle = HANDLE::default();
    let mut io_status = IO_STATUS_BLOCK::default();

    // SAFETY:
    // - `name` and `unicode_name` stay alive for the entire call.
    // - `object_attributes`, `handle`, and `io_status` are valid writable pointers.
    // - `OBJ_DONT_REPARSE` prevents reparse-point traversal outside the volume
    //   unless the caller explicitly opens the final reparse-point object.
    let status = unsafe {
        NtCreateFile(
            core::ptr::addr_of_mut!(handle),
            FILE_ACCESS_RIGHTS(options.desired_access),
            core::ptr::addr_of!(object_attributes),
            core::ptr::addr_of_mut!(io_status),
            options.allocation_size.as_ref().map(core::ptr::from_ref),
            FILE_FLAGS_AND_ATTRIBUTES(options.file_attributes),
            FILE_SHARE_MODE(options.share_access),
            NTCREATEFILE_CREATE_DISPOSITION(options.create_disposition),
            NTCREATEFILE_CREATE_OPTIONS(options.create_options),
            None,
            0,
        )
    };
    if status.0 < 0 {
        return Err(OpenFileError::NtStatus(status));
    }

    Ok((FileHandle(handle), io_status.Information))
}

fn final_file_object_attributes(create_options: u32) -> OBJECT_ATTRIBUTE_FLAGS {
    let mut attributes = OBJ_CASE_INSENSITIVE;
    if create_options & FILE_OPEN_REPARSE_POINT == 0 {
        attributes |= OBJ_DONT_REPARSE;
    }
    attributes
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;
    use std::os::windows::io::AsRawHandle as _;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::thread;

    use windows::Wdk::Storage::FileSystem::FileDirectoryInformation;
    use windows::Win32::System::IO::CancelSynchronousIo;

    #[test]
    fn final_reparse_point_open_does_not_set_object_manager_reparse_blocking() {
        assert_ne!((final_file_object_attributes(0) & OBJ_DONT_REPARSE).0, 0);
        assert_eq!(
            (final_file_object_attributes(FILE_OPEN_REPARSE_POINT) & OBJ_DONT_REPARSE).0,
            0
        );
    }

    #[test]
    fn opens_the_system_volume_root() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let root = format!(r"{system_drive}\");

        let handle = RootDirectory::open(Path::new(&root)).expect("open system volume root");

        assert!(
            handle
                .open_relative_directory(&RelativePath::parse(r"\Windows").expect("valid relative path"))
                .expect("open child directory")
                .is_some()
        );
    }

    #[test]
    fn opens_a_child_directory_relative_to_the_system_volume() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let root = format!(r"{system_drive}\");
        let root = RootDirectory::open(Path::new(&root)).expect("open system volume root");
        let windows_directory = RelativePath::parse(r"\Windows").expect("valid relative path");

        let handle = root
            .open_relative_directory(&windows_directory)
            .expect("open child directory relative to root");

        assert!(handle.is_some());
    }

    #[test]
    fn opens_a_distinct_handle_for_the_volume_root_directory() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let root = RootDirectory::open(Path::new(&format!(r"{system_drive}\"))).expect("open system volume root");

        assert!(
            root.open_relative_directory(&RelativePath::parse(r"\").expect("valid root path"))
                .expect("open root directory relative to root")
                .is_some()
        );
    }

    #[test]
    fn opens_an_existing_file_relative_to_the_system_volume() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let root = format!(r"{system_drive}\");
        let root = RootDirectory::open(Path::new(&root)).expect("open system volume root");
        let kernel32 = RelativePath::parse(r"\Windows\System32\kernel32.dll").expect("valid relative path");

        let (_handle, information) = root
            .open_relative_file(
                &kernel32,
                FileOpenOptions {
                    desired_access: 0x0010_0081,
                    allocation_size: None,
                    file_attributes: FILE_ATTRIBUTE_NORMAL.0,
                    share_access: (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
                    create_disposition: FILE_OPEN.0,
                    create_options: 0x0000_0020,
                },
            )
            .expect("open kernel32 relative to system root");

        assert_eq!(information, FILE_OPENED_INFORMATION);
    }

    #[test]
    fn root_open_allows_metadata_queries_after_an_explorer_directory_list_request() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let root = RootDirectory::open(Path::new(&format!(r"{system_drive}\"))).expect("validate system volume root");
        let options = FileOpenOptions {
            desired_access: 0x0010_0001,
            allocation_size: None,
            file_attributes: FILE_ATTRIBUTE_NORMAL.0,
            share_access: (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
            create_disposition: FILE_OPEN.0,
            create_options: (FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT).0,
        };

        let (handle, _) = root.open_root_file(options).expect("open system volume root");

        handle
            .query_basic_information()
            .expect("query root metadata without an explicit read-attributes request");
    }

    #[test]
    fn independent_root_opens_do_not_block_directory_queries_behind_a_notification() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let root = RootDirectory::open(Path::new(&format!(r"{system_drive}\"))).expect("validate system volume root");
        let options = FileOpenOptions {
            desired_access: 0x0010_0001,
            allocation_size: None,
            file_attributes: FILE_ATTRIBUTE_NORMAL.0,
            share_access: (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
            create_disposition: FILE_OPEN.0,
            create_options: (FILE_DIRECTORY_FILE | FILE_SYNCHRONOUS_IO_NONALERT).0,
        };
        let (watch_handle, _) = root.open_root_file(options).expect("open watched system volume root");
        let (query_handle, _) = root.open_root_file(options).expect("open queried system volume root");
        let (started_sender, started_receiver) = mpsc::channel();
        let watcher = thread::spawn(move || {
            started_sender
                .send(())
                .expect("directory notification starter remains available");
            watch_handle.read_directory_changes(false, 1)
        });
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("directory notification worker starts");
        thread::sleep(Duration::from_millis(25));

        let (query_sender, query_receiver) = mpsc::channel();
        let query = thread::spawn(move || {
            query_sender
                .send(query_handle.query_directory_information(FileDirectoryInformation, None, true))
                .expect("directory query receiver remains available");
        });
        let query_result = query_receiver.recv_timeout(Duration::from_secs(1));

        while !watcher.is_finished() {
            // SAFETY: `watcher` owns a live Windows thread handle until it is joined.
            let _ = unsafe { CancelSynchronousIo(HANDLE(watcher.as_raw_handle())) };
            thread::sleep(Duration::from_millis(1));
        }
        let _ = watcher.join().expect("directory notification worker does not panic");

        match query_result {
            Ok(result) => {
                let _ = result.expect("independent root directory query succeeds");
            }
            Err(error) => {
                query
                    .join()
                    .expect("blocked directory query exits after notification cancellation");
                panic!("independent root directory query did not complete: {error}");
            }
        }
        query.join().expect("directory query worker does not panic");
    }

    #[test]
    fn asynchronous_directory_notification_observes_an_immediate_public_documents_change() {
        let public_documents =
            PathBuf::from(std::env::var("PUBLIC").expect("PUBLIC is set on Windows")).join("Documents");
        let temporary_directory = public_documents.join(format!(
            "ironrdp-rdpdr-async-notify-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time is after the Unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir(&temporary_directory).expect("create temporary directory");
        let volume_root = temporary_directory
            .ancestors()
            .last()
            .expect("temporary directory has a volume root");
        let relative_path = temporary_directory
            .strip_prefix(volume_root)
            .expect("temporary directory is beneath its volume root");
        let relative_path = RelativePath::parse(&format!(r"\{}", relative_path.display()))
            .expect("temporary directory has a valid path");

        let root = RootDirectory::open(volume_root).expect("open temporary volume root");
        let directory_handle = root
            .open_relative_directory_for_notification(&relative_path)
            .expect("open asynchronous directory notification handle")
            .expect("existing temporary directory")
            .into_file_handle();
        let mode: windows::Wdk::Storage::FileSystem::FILE_MODE_INFORMATION = directory_handle
            .query_information(windows::Wdk::Storage::FileSystem::FileModeInformation)
            .expect("query asynchronous directory handle mode");
        assert_eq!(mode.Mode & FILE_SYNCHRONOUS_IO_NONALERT.0, 0);
        let directory_change = directory_handle
            .begin_directory_changes(false, 1)
            .expect("register asynchronous directory notification");
        let notification_handle = directory_change.handle().0 as usize;
        let (result_sender, result_receiver) = mpsc::channel();
        let watcher = thread::spawn(move || {
            result_sender
                .send(directory_change.wait())
                .expect("directory notification result receiver remains available");
        });

        assert!(
            matches!(result_receiver.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "directory notification completed before a change occurred"
        );
        std::fs::write(temporary_directory.join("created.txt"), b"created").expect("create watched file");
        let result = match result_receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(result) => result,
            Err(error) => {
                // SAFETY: the worker owns the asynchronous directory handle
                // until it reports completion. Cancelling here guarantees the
                // test can join it before reporting the timeout.
                let _ = unsafe { CancelIoEx(HANDLE(notification_handle as *mut core::ffi::c_void), None) };
                let _ = result_receiver
                    .recv_timeout(Duration::from_secs(1))
                    .expect("cancelled directory notification worker exits");
                watcher.join().expect("directory notification worker does not panic");
                panic!("directory change notification did not complete: {error}");
            }
        };
        watcher.join().expect("directory notification worker does not panic");
        let result = result.expect("directory notification succeeds");

        assert!(!result.is_empty());
        std::fs::remove_dir_all(&temporary_directory).expect("remove temporary directory");
    }

    #[test]
    fn byte_range_locks_conflict_until_unlocked() {
        let temporary_directory = std::env::temp_dir();
        let volume_root = temporary_directory
            .ancestors()
            .last()
            .expect("temporary directory has a volume root");
        let file_path = temporary_directory.join(format!(
            "ironrdp-rdpdr-lock-test-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time is after the Unix epoch")
                .as_nanos()
        ));
        std::fs::write(&file_path, [0u8; 16]).expect("create temporary test file");

        {
            let root = RootDirectory::open(volume_root).expect("open temporary volume root");
            let relative_path = file_path
                .strip_prefix(volume_root)
                .expect("temporary file is beneath its volume root");
            let relative_path = RelativePath::parse(&format!(r"\{}", relative_path.display()))
                .expect("temporary file has a valid relative path");
            let options = FileOpenOptions {
                desired_access: 0x0010_0083,
                allocation_size: None,
                file_attributes: FILE_ATTRIBUTE_NORMAL.0,
                share_access: (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
                create_disposition: FILE_OPEN.0,
                create_options: 0x0000_0020,
            };
            let (first, _) = root
                .open_relative_file(&relative_path, options)
                .expect("open first temporary file handle");
            let (second, _) = root
                .open_relative_file(&relative_path, options)
                .expect("open second temporary file handle");

            first.lock_range(0, 8, true).expect("acquire exclusive byte-range lock");
            assert!(second.lock_range(0, 8, true).is_err());
            first.unlock_range(0, 8).expect("release exclusive byte-range lock");
            second.lock_range(0, 8, true).expect("acquire released byte-range lock");
            second.unlock_range(0, 8).expect("release second byte-range lock");
        }

        std::fs::remove_file(&file_path).expect("remove temporary test file");
    }

    #[test]
    fn multi_range_locks_rollback_on_conflict_and_unlock_all_ranges() {
        let temporary_directory = std::env::temp_dir();
        let volume_root = temporary_directory
            .ancestors()
            .last()
            .expect("temporary directory has a volume root");
        let file_path = temporary_directory.join(format!(
            "ironrdp-rdpdr-multi-lock-test-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time is after Unix epoch")
                .as_nanos()
        ));
        std::fs::write(&file_path, [0u8; 16]).expect("create temporary test file");

        {
            let root = RootDirectory::open(volume_root).expect("open temporary volume root");
            let relative_path = file_path
                .strip_prefix(volume_root)
                .expect("temporary file is beneath its volume root");
            let relative_path = RelativePath::parse(&format!(r"\{}", relative_path.display()))
                .expect("temporary file has a valid relative path");
            let options = FileOpenOptions {
                desired_access: 0x0010_0083,
                allocation_size: None,
                file_attributes: FILE_ATTRIBUTE_NORMAL.0,
                share_access: (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
                create_disposition: FILE_OPEN.0,
                create_options: 0x0000_0020,
            };
            let (first, _) = root
                .open_relative_file(&relative_path, options)
                .expect("open first temporary file handle");
            let (second, _) = root
                .open_relative_file(&relative_path, options)
                .expect("open second temporary file handle");
            let ranges = [(0, 4), (8, 4)];

            first.lock_ranges(&ranges, true).expect("acquire all byte-range locks");
            assert!(second.lock_range(0, 4, true).is_err());
            assert!(second.lock_range(8, 4, true).is_err());
            first.unlock_ranges(&ranges).expect("release all byte-range locks");
            second
                .lock_ranges(&ranges, true)
                .expect("acquire released byte-range locks");
            second.unlock_ranges(&ranges).expect("release second byte-range locks");
        }

        std::fs::remove_file(&file_path).expect("remove temporary test file");
    }

    #[test]
    fn directory_change_notifications_return_file_notify_information() {
        let temporary_directory = std::env::temp_dir().join(format!(
            "ironrdp-rdpdr-notify-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system time is after the Unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir(&temporary_directory).expect("create temporary test directory");
        let volume_root = temporary_directory
            .ancestors()
            .last()
            .expect("temporary directory has a volume root");

        {
            let root = RootDirectory::open(volume_root).expect("open temporary volume root");
            let relative_path = temporary_directory
                .strip_prefix(volume_root)
                .expect("temporary directory is beneath its volume root");
            let relative_path = RelativePath::parse(&format!(r"\{}", relative_path.display()))
                .expect("temporary directory has a valid path");
            let options = FileOpenOptions {
                desired_access: 0x0010_0081,
                allocation_size: None,
                file_attributes: FILE_ATTRIBUTE_NORMAL.0,
                share_access: (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0,
                create_disposition: FILE_OPEN.0,
                create_options: 0x0000_0021,
            };
            let (directory_handle, _) = root
                .open_relative_file(&relative_path, options)
                .expect("open directory notification handle");
            let worker_handle = directory_handle
                .try_clone()
                .expect("duplicate directory handle for notification worker");
            let (sender, receiver) = mpsc::channel();
            let worker = thread::spawn(move || {
                sender
                    .send(worker_handle.read_directory_changes(false, 1))
                    .expect("directory notification receiver remains available");
            });

            let mut notification = None;
            for index in 0..20 {
                std::fs::write(temporary_directory.join(format!("change-{index}")), b"change")
                    .expect("create watched file");
                if let Ok(result) = receiver.recv_timeout(Duration::from_millis(25)) {
                    notification = Some(result);
                    break;
                }
            }

            let buffer = notification
                .expect("receive directory notification")
                .expect("directory notification succeeds");
            worker.join().expect("directory notification worker does not panic");
            assert!(buffer.len() >= 12);
            assert_eq!(
                u32::from_le_bytes(buffer[4..8].try_into().expect("action is present")),
                1
            );
            assert_ne!(
                u32::from_le_bytes(buffer[8..12].try_into().expect("file name length is present")),
                0
            );
        }

        std::fs::remove_dir_all(&temporary_directory).expect("remove temporary test directory");
    }
}
