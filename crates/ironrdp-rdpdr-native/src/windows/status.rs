//! Narrow status mapping for Windows RDPDR filesystem operations.

use windows::Win32::Foundation::NTSTATUS;

use ironrdp_rdpdr::pdu::efs::NtStatus;

use super::handles::{OpenDirectoryError, OpenFileError};
use super::path::PathPolicyError;

pub(crate) fn from_path_policy(error: PathPolicyError) -> NtStatus {
    match error {
        PathPolicyError::ReservedDevice => NtStatus::ACCESS_DENIED,
        PathPolicyError::Rooted | PathPolicyError::InvalidComponent | PathPolicyError::TooLong => {
            NtStatus::OBJECT_NAME_INVALID
        }
    }
}

pub(crate) fn from_open_file(error: OpenFileError) -> NtStatus {
    match error {
        OpenFileError::Directory(error) => from_open_directory(error),
        OpenFileError::NtStatus(status) => from_ntstatus(status),
    }
}

pub(crate) fn from_open_directory(error: OpenDirectoryError) -> NtStatus {
    match error {
        OpenDirectoryError::InvalidVolumeRoot => NtStatus::OBJECT_NAME_INVALID,
        OpenDirectoryError::NtStatus(status) => from_ntstatus(status),
    }
}

pub(crate) fn from_ntstatus(status: NTSTATUS) -> NtStatus {
    match u32::from_ne_bytes(status.0.to_ne_bytes()) {
        0xC000_0008 => NtStatus::INVALID_HANDLE,
        0xC000_000D => NtStatus::INVALID_PARAMETER,
        0xC000_000F | 0xC000_0034 => NtStatus::NO_SUCH_FILE,
        0xC000_0010 => NtStatus::INVALID_DEVICE_REQUEST,
        0xC000_0011 => NtStatus::END_OF_FILE,
        0x8000_0005 => NtStatus::BUFFER_OVERFLOW,
        0x8000_0006 => NtStatus::NO_MORE_FILES,
        0xC000_0022 => NtStatus::ACCESS_DENIED,
        0xC000_0023 => NtStatus::BUFFER_TOO_SMALL,
        0xC000_0033 => NtStatus::OBJECT_NAME_INVALID,
        0xC000_0035 => NtStatus::OBJECT_NAME_COLLISION,
        0xC000_003A | 0xC000_003B => NtStatus::OBJECT_PATH_NOT_FOUND,
        0xC000_0043 => NtStatus::SHARING_VIOLATION,
        0xC000_007F => NtStatus::DISK_FULL,
        0xC000_00BA => NtStatus::FILE_IS_A_DIRECTORY,
        0xC000_0101 => NtStatus::DIRECTORY_NOT_EMPTY,
        0xC000_0103 => NtStatus::NOT_A_DIRECTORY,
        0xC000_0120 => NtStatus::CANCELLED,
        0xC000_0279 | 0xC000_0280 | 0x8000_002D => NtStatus::ACCESS_DENIED,
        _ => NtStatus::from(u32::from_ne_bytes(status.0.to_ne_bytes())),
    }
}
