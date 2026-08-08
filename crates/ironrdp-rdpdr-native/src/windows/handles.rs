//! Handle-relative Windows filesystem primitives for the narrow RDPDR backend.

use core::fmt;
use std::os::windows::ffi::OsStrExt as _;
use std::path::Path;

use windows::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows::Wdk::Storage::FileSystem::{
    FILE_BASIC_INFORMATION, FILE_INFORMATION_CLASS, FILE_OPEN, FILE_STANDARD_INFORMATION, FILE_SYNCHRONOUS_IO_NONALERT,
    FileAttributeTagInformation, FileBasicInformation, FileEndOfFileInformation, FileStandardInformation,
    NTCREATEFILE_CREATE_DISPOSITION, NTCREATEFILE_CREATE_OPTIONS, NtCreateFile, NtQueryInformationFile, NtReadFile,
    NtSetInformationFile, NtWriteFile,
};
use windows::Wdk::System::SystemServices::FILE_END_OF_FILE_INFORMATION;
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, NTSTATUS, OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, OBJECT_ATTRIBUTE_FLAGS, UNICODE_STRING,
};
use windows::Win32::Storage::FileSystem::{
    FILE_ACCESS_RIGHTS, FILE_ATTRIBUTE_NORMAL, FILE_FLAGS_AND_ATTRIBUTES, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
    FILE_SHARE_MODE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, SYNCHRONIZE,
};
use windows::Win32::System::IO::IO_STATUS_BLOCK;

use super::path::RelativePath;

const STATUS_INVALID_PARAMETER: NTSTATUS = NTSTATUS(i32::from_ne_bytes(0xC000_000Du32.to_ne_bytes()));
const STATUS_OBJECT_NAME_INVALID: NTSTATUS = NTSTATUS(i32::from_ne_bytes(0xC000_0033u32.to_ne_bytes()));
const DIRECTORY_OPEN_OPTIONS: u32 = FILE_SYNCHRONOUS_IO_NONALERT.0 | 0x0000_0001;

/// A validated native path for an explicitly configured logical-volume root.
pub(crate) struct RootDirectory {
    root_handle: DirectoryHandle,
}

impl RootDirectory {
    /// Validates an absolute logical drive root such as `C:\`.
    pub(crate) fn open(path: &Path) -> Result<Self, NTSTATUS> {
        let mut nt_path = volume_root_nt_path(path).ok_or(STATUS_OBJECT_NAME_INVALID)?;
        let root_handle = DirectoryHandle(open_directory(HANDLE::default(), &mut nt_path)?);

        Ok(Self { root_handle })
    }

    /// Opens a file or directory without ever joining server input to a host path.
    pub(crate) fn open_relative_file(
        &self,
        path: &RelativePath,
        options: FileOpenOptions,
    ) -> Result<(FileHandle, usize), NTSTATUS> {
        let mut components = path.components();
        let Some(file_name) = components.next_back() else {
            return self.open_root_file(options);
        };

        let mut parent = None;
        for component in components {
            let mut component = component.encode_utf16().collect::<Vec<_>>();
            let root_directory = parent
                .as_ref()
                .map_or(self.root_handle.as_raw(), DirectoryHandle::as_raw);
            parent = Some(DirectoryHandle(open_directory(root_directory, &mut component)?));
        }

        let mut file_name = file_name.encode_utf16().collect::<Vec<_>>();
        let root_directory = parent
            .as_ref()
            .map_or(self.root_handle.as_raw(), DirectoryHandle::as_raw);
        open_file(root_directory, &mut file_name, options)
    }

    fn open_root_file(&self, mut options: FileOpenOptions) -> Result<(FileHandle, usize), NTSTATUS> {
        options.desired_access |= FILE_READ_ATTRIBUTES.0;
        // An empty relative name opens the object identified by RootDirectory.
        let mut name = [];
        open_file(self.root_handle.as_raw(), &mut name, options)
    }
}

impl fmt::Debug for RootDirectory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("RootDirectory").field(&"<owned>").finish()
    }
}

struct DirectoryHandle(HANDLE);

// SAFETY: The handle has unique ownership, is only used through backend-owned
// state, and is closed exactly once by its Drop implementation.
unsafe impl Send for DirectoryHandle {}

impl DirectoryHandle {
    fn as_raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for DirectoryHandle {
    fn drop(&mut self) {
        // SAFETY: This guard owns the handle returned by NtCreateFile.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// A non-cloneable local filesystem handle.
#[derive(Debug)]
pub(crate) struct FileHandle(HANDLE);

// SAFETY: The handle has unique ownership, is only used through backend-owned
// state, and is closed exactly once by its Drop implementation.
unsafe impl Send for FileHandle {}

impl FileHandle {
    pub(crate) fn query_basic_information(&self) -> Result<FILE_BASIC_INFORMATION, NTSTATUS> {
        self.query_information(FileBasicInformation)
    }

    pub(crate) fn query_standard_information(&self) -> Result<FILE_STANDARD_INFORMATION, NTSTATUS> {
        self.query_information(FileStandardInformation)
    }

    pub(crate) fn query_attribute_tag_information(
        &self,
    ) -> Result<windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_TAG_INFO, NTSTATUS> {
        self.query_information(FileAttributeTagInformation)
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

    pub(crate) fn read_at(&self, offset: i64, buffer: &mut [u8]) -> Result<NativeIoCompletion, NTSTATUS> {
        let length = u32::try_from(buffer.len()).map_err(|_| STATUS_INVALID_PARAMETER)?;
        let mut io_status = IO_STATUS_BLOCK::default();

        // SAFETY: The handle is live, the output buffer and IO status remain
        // valid for the synchronous request, and the explicit offset is valid.
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

    pub(crate) fn write_at(&self, offset: i64, buffer: &[u8]) -> Result<NativeIoCompletion, NTSTATUS> {
        let length = u32::try_from(buffer.len()).map_err(|_| STATUS_INVALID_PARAMETER)?;
        let mut io_status = IO_STATUS_BLOCK::default();

        // SAFETY: The handle is live, the input buffer and IO status remain
        // valid for the synchronous request, and the explicit offset is valid.
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

    fn query_information<T: Default>(&self, information_class: FILE_INFORMATION_CLASS) -> Result<T, NTSTATUS> {
        let mut information = T::default();
        let mut io_status = IO_STATUS_BLOCK::default();
        let length = u32::try_from(size_of::<T>()).map_err(|_| STATUS_INVALID_PARAMETER)?;

        // SAFETY: `information` and `io_status` are writable for the complete
        // synchronous call, and `T` is the native structure for the class.
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

    fn set_information<T>(&self, information_class: FILE_INFORMATION_CLASS, information: &T) -> Result<(), NTSTATUS> {
        let mut io_status = IO_STATUS_BLOCK::default();
        let length = u32::try_from(size_of::<T>()).map_err(|_| STATUS_INVALID_PARAMETER)?;

        // SAFETY: `information` is valid immutable native input and
        // `io_status` remains writable for the synchronous call.
        let status = unsafe {
            NtSetInformationFile(
                self.0,
                core::ptr::addr_of_mut!(io_status),
                core::ptr::from_ref(information).cast_mut().cast(),
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

impl Drop for FileHandle {
    fn drop(&mut self) {
        // SAFETY: This guard owns the handle returned by NtCreateFile.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// A synchronous native read or write completion.
#[derive(Clone, Copy, Debug)]
pub(crate) struct NativeIoCompletion {
    pub(crate) status: NTSTATUS,
    pub(crate) transferred: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FileOpenOptions {
    pub(crate) desired_access: u32,
    pub(crate) allocation_size: Option<i64>,
    pub(crate) file_attributes: u32,
    pub(crate) share_access: u32,
    pub(crate) create_disposition: u32,
    pub(crate) create_options: u32,
}

pub(crate) const FILE_SUPERSEDED_INFORMATION: usize = 0;
pub(crate) const FILE_OPENED_INFORMATION: usize = 1;
pub(crate) const FILE_CREATED_INFORMATION: usize = 2;
pub(crate) const FILE_OVERWRITTEN_INFORMATION: usize = 3;

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

fn open_directory(root_directory: HANDLE, name: &mut [u16]) -> Result<HANDLE, NTSTATUS> {
    let name = unicode_string(name)?;
    let object_attributes = object_attributes(root_directory, &name);
    let mut handle = HANDLE::default();
    let mut io_status = IO_STATUS_BLOCK::default();
    let desired_access = FILE_ACCESS_RIGHTS(FILE_TRAVERSE.0 | SYNCHRONIZE.0);

    // SAFETY: Name, object attributes, output handle, and IO status remain
    // valid for this synchronous call. OBJ_DONT_REPARSE blocks every reparse
    // point while walking from the trusted root.
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
            NTCREATEFILE_CREATE_OPTIONS(DIRECTORY_OPEN_OPTIONS),
            None,
            0,
        )
    };
    if status.0 < 0 {
        return Err(status);
    }

    Ok(handle)
}

fn open_file(
    root_directory: HANDLE,
    name: &mut [u16],
    options: FileOpenOptions,
) -> Result<(FileHandle, usize), NTSTATUS> {
    let name = unicode_string(name)?;
    let object_attributes = object_attributes(root_directory, &name);
    let mut handle = HANDLE::default();
    let mut io_status = IO_STATUS_BLOCK::default();

    // SAFETY: Name, object attributes, output handle, allocation size, and IO
    // status remain valid for the call. OBJ_DONT_REPARSE prevents the final
    // component from escaping the selected logical volume.
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
            NTCREATEFILE_CREATE_OPTIONS(options.create_options | FILE_SYNCHRONOUS_IO_NONALERT.0),
            None,
            0,
        )
    };
    if status.0 < 0 {
        return Err(status);
    }

    Ok((FileHandle(handle), io_status.Information))
}

fn unicode_string(value: &mut [u16]) -> Result<UNICODE_STRING, NTSTATUS> {
    let byte_len = value
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or(STATUS_INVALID_PARAMETER)?;

    Ok(UNICODE_STRING {
        Length: byte_len,
        MaximumLength: byte_len,
        Buffer: windows::core::PWSTR::from_raw(value.as_mut_ptr()),
    })
}

fn object_attributes(root_directory: HANDLE, name: &UNICODE_STRING) -> OBJECT_ATTRIBUTES {
    OBJECT_ATTRIBUTES {
        Length: u32::try_from(size_of::<OBJECT_ATTRIBUTES>()).expect("OBJECT_ATTRIBUTES size fits in u32"),
        RootDirectory: root_directory,
        ObjectName: core::ptr::from_ref(name).cast_mut(),
        Attributes: OBJECT_ATTRIBUTE_FLAGS(OBJ_CASE_INSENSITIVE.0 | OBJ_DONT_REPARSE.0),
        SecurityDescriptor: core::ptr::null(),
        SecurityQualityOfService: core::ptr::null(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_logical_volume_roots() {
        assert!(RootDirectory::open(Path::new(r"C:\")).is_ok());
        assert!(matches!(
            RootDirectory::open(Path::new(r"C:\Windows")),
            Err(STATUS_OBJECT_NAME_INVALID)
        ));
        assert!(matches!(
            RootDirectory::open(Path::new(r"\\server\share\")),
            Err(STATUS_OBJECT_NAME_INVALID)
        ));
    }
}
