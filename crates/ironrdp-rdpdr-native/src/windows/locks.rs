//! Non-blocking byte-range locking for redirected Windows files.

use ironrdp_pdu::PduResult;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    ClientDriveLockControlResponse, DeviceIoResponse, LockOperation, NtStatus, RdpLockInfo,
    ServerDriveLockControlRequest,
};
use ironrdp_svc::SvcMessage;

use super::backend::WindowsRdpdrBackend;
use super::file::file_for_request;

pub(crate) fn control(
    backend: &mut WindowsRdpdrBackend,
    req: ServerDriveLockControlRequest,
) -> PduResult<Vec<SvcMessage>> {
    if req.wait && matches!(req.operation, LockOperation::Shared | LockOperation::Exclusive) {
        return defer_waiting_lock(backend, req);
    }

    let status = control_inner(backend, &req).map_or_else(|status| status, |_| NtStatus::SUCCESS);
    let response = ClientDriveLockControlResponse {
        device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
    };

    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveLockControlResponse(
        response,
    ))])
}

fn control_inner(backend: &WindowsRdpdrBackend, req: &ServerDriveLockControlRequest) -> Result<(), NtStatus> {
    if req.locks.is_empty() {
        return Err(NtStatus::INVALID_PARAMETER);
    }

    let ranges = normalized_ranges(&req.locks)?;
    let file = file_for_request(backend, req.device_io_request.device_id, req.device_io_request.file_id)?;

    match req.operation {
        LockOperation::Shared | LockOperation::Exclusive => file
            .handle
            .lock_ranges(&ranges, req.operation == LockOperation::Exclusive)
            .map_err(|error| lock_error_status(&error)),
        LockOperation::Unlock | LockOperation::UnlockMultiple => file
            .handle
            .unlock_ranges(&ranges)
            .map_err(|error| lock_error_status(&error)),
    }
}

fn defer_waiting_lock(
    backend: &mut WindowsRdpdrBackend,
    req: ServerDriveLockControlRequest,
) -> PduResult<Vec<SvcMessage>> {
    let ranges = match normalized_ranges(&req.locks) {
        Ok(ranges) => ranges,
        Err(status) => return immediate_response(req, status),
    };
    let exclusive = match req.operation {
        LockOperation::Shared => false,
        LockOperation::Exclusive => true,
        LockOperation::Unlock | LockOperation::UnlockMultiple => unreachable!("only lock operations are deferred"),
    };

    match backend.schedule_waiting_lock(req.device_io_request.clone(), ranges, exclusive) {
        Ok(()) => Ok(Vec::new()),
        Err(status) => immediate_response(req, status),
    }
}

fn immediate_response(req: ServerDriveLockControlRequest, status: NtStatus) -> PduResult<Vec<SvcMessage>> {
    let response = ClientDriveLockControlResponse {
        device_io_reply: DeviceIoResponse::new(req.device_io_request, status),
    };
    Ok(vec![SvcMessage::from(RdpdrPdu::ClientDriveLockControlResponse(
        response,
    ))])
}

fn normalized_ranges(locks: &[RdpLockInfo]) -> Result<Vec<(u64, u64)>, NtStatus> {
    if locks.is_empty() {
        return Err(NtStatus::INVALID_PARAMETER);
    }

    Ok(locks.iter().map(|lock| (lock.offset, lock.length)).collect())
}

pub(super) fn lock_error_status(error: &windows::core::Error) -> NtStatus {
    match u32::from_ne_bytes(error.code().0.to_ne_bytes()) {
        0x8007_0005 => NtStatus::ACCESS_DENIED,
        0x8007_0006 => NtStatus::INVALID_HANDLE,
        0x8007_0021 => NtStatus::LOCK_NOT_GRANTED,
        0x8007_0057 => NtStatus::INVALID_PARAMETER,
        _ => NtStatus::UNSUCCESSFUL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_rdpdr::pdu::efs::{
        CreateDisposition, CreateOptions, DesiredAccess, DeviceCreateRequest, DeviceIoRequest, FileAttributes,
        MajorFunction, MinorFunction, SharedAccess,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::windows::factory::RedirectedDrive;

    #[test]
    fn lock_control_preserves_all_requested_ranges_including_zero_length_regions() {
        assert_eq!(
            normalized_ranges(&[
                RdpLockInfo { offset: 0, length: 0 },
                RdpLockInfo { offset: 8, length: 4 },
            ]),
            Ok(vec![(0, 0), (8, 4)])
        );
    }

    #[test]
    fn lock_control_handles_multiple_ranges_and_waiting_unlocks() {
        let file_path = std::env::temp_dir().join(format!(
            "ironrdp-rdpdr-lock-control-test-{}-{}.bin",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after Unix epoch")
                .as_nanos()
        ));
        std::fs::write(&file_path, [0u8; 16]).expect("create temporary file");
        let root_path = file_path
            .ancestors()
            .last()
            .expect("temporary file has a volume root")
            .to_owned();
        let relative_path = file_path
            .strip_prefix(&root_path)
            .expect("temporary file is beneath its volume root");

        {
            let drive = RedirectedDrive::new(1, "Test", root_path.clone(), false).expect("valid redirected drive");
            let mut backend = WindowsRdpdrBackend::new(vec![drive]).expect("open redirected root");
            let (file_id, _) = super::super::file::create_inner(
                &mut backend,
                &DeviceCreateRequest {
                    device_io_request: DeviceIoRequest {
                        device_id: 1,
                        file_id: 0,
                        completion_id: 1,
                        major_function: MajorFunction::Create,
                        minor_function: MinorFunction::from(0),
                    },
                    desired_access: DesiredAccess::from_bits_retain(0x0010_0083),
                    allocation_size: 0,
                    file_attributes: FileAttributes::empty(),
                    shared_access: SharedAccess::from_bits_retain(0x0000_0007),
                    create_disposition: CreateDisposition::FILE_OPEN,
                    create_options: CreateOptions::empty(),
                    path: format!(r"\{}", relative_path.display()),
                },
            )
            .expect("open temporary file");
            let ranges = vec![
                RdpLockInfo { offset: 0, length: 4 },
                RdpLockInfo { offset: 8, length: 4 },
            ];

            let lock_response = control(
                &mut backend,
                ServerDriveLockControlRequest {
                    device_io_request: DeviceIoRequest {
                        device_id: 1,
                        file_id,
                        completion_id: 2,
                        major_function: MajorFunction::LockControl,
                        minor_function: MinorFunction::from(0),
                    },
                    operation: LockOperation::Exclusive,
                    wait: false,
                    locks: ranges.clone(),
                },
            )
            .expect("lock control response");
            assert_eq!(lock_response.len(), 1);
            let lock_response = lock_response
                .into_iter()
                .next()
                .expect("one lock response")
                .encode_unframed_pdu()
                .expect("lock response is encodable");
            assert_eq!(
                u32::from_le_bytes(lock_response[12..16].try_into().expect("response status is present")),
                u32::from(NtStatus::SUCCESS)
            );

            let unlock_response = control(
                &mut backend,
                ServerDriveLockControlRequest {
                    device_io_request: DeviceIoRequest {
                        device_id: 1,
                        file_id,
                        completion_id: 3,
                        major_function: MajorFunction::LockControl,
                        minor_function: MinorFunction::from(0),
                    },
                    operation: LockOperation::Unlock,
                    wait: true,
                    locks: ranges,
                },
            )
            .expect("unlock control response");
            assert_eq!(unlock_response.len(), 1);
            let unlock_response = unlock_response
                .into_iter()
                .next()
                .expect("one unlock response")
                .encode_unframed_pdu()
                .expect("unlock response is encodable");
            assert_eq!(
                u32::from_le_bytes(unlock_response[12..16].try_into().expect("response status is present")),
                u32::from(NtStatus::SUCCESS)
            );
        }

        std::fs::remove_file(&file_path).expect("remove temporary file");
    }
}
