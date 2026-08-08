//! Narrow status mapping for Windows RDPDR filesystem operations.

use windows::Win32::Foundation::NTSTATUS;

use ironrdp_rdpdr::pdu::efs::NtStatus;

use super::path::PathPolicyError;

pub(crate) fn from_path_policy(error: PathPolicyError) -> NtStatus {
    match error {
        PathPolicyError::ReservedDevice => NtStatus::ACCESS_DENIED,
        PathPolicyError::Rooted | PathPolicyError::InvalidComponent | PathPolicyError::TooLong => {
            NtStatus::OBJECT_NAME_INVALID
        }
    }
}

pub(crate) fn from_ntstatus(status: NTSTATUS) -> NtStatus {
    NtStatus::from(u32::from_ne_bytes(status.0.to_ne_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_native_status_values() {
        let status = NTSTATUS(i32::from_ne_bytes(0xC000_0034u32.to_ne_bytes()));

        assert_eq!(from_ntstatus(status), NtStatus::from(0xC000_0034));
    }
}
