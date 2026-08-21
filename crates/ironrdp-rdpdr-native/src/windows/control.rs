//! Constrained filesystem Device Control support for redirected Windows files.

use ironrdp_core::{EncodeResult, WriteCursor, ensure_size};
use ironrdp_pdu::PduResult;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{AnyIoCtlCode, DeviceControlRequest, DeviceControlResponse, NtStatus};
use ironrdp_rdpdr::pdu::esc::rpce;
use ironrdp_svc::SvcMessage;

use super::backend::WindowsRdpdrBackend;
use super::file::file_for_request;
use super::handles::FILE_OBJECTID_BUFFER_SIZE;
use super::status::from_ntstatus;

/// `FSCTL_CREATE_OR_GET_OBJECT_ID` obtains an existing handle-bound object ID
/// or creates one when the local filesystem supports that operation.
const FSCTL_CREATE_OR_GET_OBJECT_ID: u32 = 0x0009_00C0;
/// `FSCTL_GET_COMPRESSION` is a handle-bound query with no input and a fixed
/// two-byte `COMPRESSION_FORMAT` output.
const FSCTL_GET_COMPRESSION: u32 = 0x0009_003C;
/// `FSCTL_QUERY_ALLOCATED_RANGES` reads only allocation metadata for an
/// already-open file. It never exposes volume-wide allocation state.
const FSCTL_QUERY_ALLOCATED_RANGES: u32 = 0x0009_40CF;
/// `FSCTL_GET_INTEGRITY_INFORMATION` reports integrity metadata associated
/// with an already-open file or directory.
const FSCTL_GET_INTEGRITY_INFORMATION: u32 = 0x0009_027C;
const COMPRESSION_FORMAT_SIZE: usize = 2;
const FILE_ALLOCATED_RANGE_BUFFER_SIZE: usize = 16;
const INTEGRITY_INFORMATION_SIZE: usize = 16;
const MAX_DEVICE_CONTROL_OUTPUT_SIZE: usize = 1024 * 1024;

pub(crate) fn handle(
    backend: &WindowsRdpdrBackend,
    req: DeviceControlRequest<AnyIoCtlCode>,
    input_buffer: &[u8],
) -> PduResult<Vec<SvcMessage>> {
    let (status, output_buffer): (NtStatus, Option<Box<dyn rpce::Encode>>) =
        match handle_inner_with_input(backend, &req, input_buffer) {
            Ok(completion) => (completion.status, Some(Box::new(completion.output))),
            Err(status) => (status, None),
        };
    let response = DeviceControlResponse::new(req, status, output_buffer);

    Ok(vec![SvcMessage::from(RdpdrPdu::DeviceControlResponse(response))])
}

fn handle_inner_with_input(
    backend: &WindowsRdpdrBackend,
    req: &DeviceControlRequest<AnyIoCtlCode>,
    input_buffer: &[u8],
) -> Result<DeviceControlCompletion, NtStatus> {
    validate_declared_input_length(req, input_buffer)?;
    let file = file_for_request(backend, req.header.device_id, req.header.file_id)?;

    match req.io_control_code.0 {
        FSCTL_CREATE_OR_GET_OBJECT_ID => {
            validate_create_or_get_object_id_request(req, input_buffer)?;
            if file.read_only {
                return Err(NtStatus::MEDIA_WRITE_PROTECTED);
            }
            let object_id = file.handle.create_or_get_object_id().map_err(from_ntstatus)?;

            Ok(DeviceControlCompletion::success(object_id.to_vec()))
        }
        FSCTL_GET_COMPRESSION => {
            validate_get_compression_request(req, input_buffer)?;
            let format = file.handle.query_compression_format().map_err(from_ntstatus)?;

            Ok(DeviceControlCompletion::success(format.to_vec()))
        }
        FSCTL_QUERY_ALLOCATED_RANGES => {
            let (range, output_buffer_length) = validate_query_allocated_ranges_request(req, input_buffer)?;
            let completion = file.handle.query_allocated_ranges(&range, output_buffer_length);
            let status = if completion.status.0 >= 0 {
                NtStatus::SUCCESS
            } else {
                from_ntstatus(completion.status)
            };
            if status != NtStatus::SUCCESS && status != NtStatus::BUFFER_OVERFLOW {
                return Err(status);
            }
            if completion.output.len() % FILE_ALLOCATED_RANGE_BUFFER_SIZE != 0 {
                return Err(NtStatus::UNSUCCESSFUL);
            }

            Ok(DeviceControlCompletion {
                status,
                output: DeviceControlOutput(completion.output),
            })
        }
        FSCTL_GET_INTEGRITY_INFORMATION => {
            validate_get_integrity_information_request(req, input_buffer)?;
            let information = file.handle.query_integrity_information().map_err(from_ntstatus)?;

            Ok(DeviceControlCompletion::success(information.to_vec()))
        }
        _ => Err(NtStatus::NOT_SUPPORTED),
    }
}

fn validate_declared_input_length(
    req: &DeviceControlRequest<AnyIoCtlCode>,
    input_buffer: &[u8],
) -> Result<(), NtStatus> {
    let input_buffer_length = usize::try_from(req.input_buffer_length).map_err(|_| NtStatus::INVALID_PARAMETER)?;
    if input_buffer_length != input_buffer.len() {
        return Err(NtStatus::INVALID_PARAMETER);
    }

    Ok(())
}

fn validate_create_or_get_object_id_request(
    req: &DeviceControlRequest<AnyIoCtlCode>,
    input_buffer: &[u8],
) -> Result<(), NtStatus> {
    if !input_buffer.is_empty() {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    let output_buffer_length = usize::try_from(req.output_buffer_length).map_err(|_| NtStatus::INVALID_PARAMETER)?;
    if output_buffer_length < FILE_OBJECTID_BUFFER_SIZE {
        return Err(NtStatus::INVALID_PARAMETER);
    }

    Ok(())
}

fn validate_get_compression_request(
    req: &DeviceControlRequest<AnyIoCtlCode>,
    input_buffer: &[u8],
) -> Result<(), NtStatus> {
    if !input_buffer.is_empty() {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    let output_buffer_length = usize::try_from(req.output_buffer_length).map_err(|_| NtStatus::INVALID_PARAMETER)?;
    if output_buffer_length < COMPRESSION_FORMAT_SIZE {
        return Err(NtStatus::INVALID_PARAMETER);
    }

    Ok(())
}

fn validate_get_integrity_information_request(
    req: &DeviceControlRequest<AnyIoCtlCode>,
    input_buffer: &[u8],
) -> Result<(), NtStatus> {
    if !input_buffer.is_empty() {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    let output_buffer_length = usize::try_from(req.output_buffer_length).map_err(|_| NtStatus::INVALID_PARAMETER)?;
    if output_buffer_length < INTEGRITY_INFORMATION_SIZE {
        return Err(NtStatus::INVALID_PARAMETER);
    }

    Ok(())
}

fn validate_query_allocated_ranges_request(
    req: &DeviceControlRequest<AnyIoCtlCode>,
    input_buffer: &[u8],
) -> Result<([u8; FILE_ALLOCATED_RANGE_BUFFER_SIZE], usize), NtStatus> {
    let range: [u8; FILE_ALLOCATED_RANGE_BUFFER_SIZE] =
        input_buffer.try_into().map_err(|_| NtStatus::INVALID_PARAMETER)?;
    let file_offset = i64::from_le_bytes(range[..8].try_into().expect("fixed range offset size"));
    let length = i64::from_le_bytes(range[8..].try_into().expect("fixed range length size"));
    if file_offset < 0 || length < 0 || file_offset.checked_add(length).is_none() {
        return Err(NtStatus::INVALID_PARAMETER);
    }

    let output_buffer_length = usize::try_from(req.output_buffer_length).map_err(|_| NtStatus::INVALID_PARAMETER)?;
    if output_buffer_length > MAX_DEVICE_CONTROL_OUTPUT_SIZE {
        return Err(NtStatus::INVALID_PARAMETER);
    }
    if output_buffer_length < FILE_ALLOCATED_RANGE_BUFFER_SIZE {
        return Err(NtStatus::BUFFER_TOO_SMALL);
    }

    Ok((range, output_buffer_length))
}

#[derive(Debug)]
struct DeviceControlCompletion {
    status: NtStatus,
    output: DeviceControlOutput,
}

impl DeviceControlCompletion {
    fn success(output: Vec<u8>) -> Self {
        Self {
            status: NtStatus::SUCCESS,
            output: DeviceControlOutput(output),
        }
    }
}

#[derive(Debug)]
struct DeviceControlOutput(Vec<u8>);

impl ironrdp_core::Encode for DeviceControlOutput {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "DeviceControlOutput"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl rpce::Encode for DeviceControlOutput {}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_rdpdr::pdu::efs::{
        CreateDisposition, CreateOptions, DesiredAccess, DeviceCreateRequest, DeviceIoRequest, FileAttributes,
        MajorFunction, MinorFunction, SharedAccess,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::windows::factory::RedirectedDrive;

    fn get_compression_request(
        input_buffer_length: u32,
        output_buffer_length: u32,
    ) -> DeviceControlRequest<AnyIoCtlCode> {
        DeviceControlRequest {
            header: DeviceIoRequest {
                device_id: 1,
                file_id: 1,
                completion_id: 1,
                major_function: MajorFunction::DeviceControl,
                minor_function: MinorFunction::from(0),
            },
            output_buffer_length,
            input_buffer_length,
            io_control_code: AnyIoCtlCode(FSCTL_GET_COMPRESSION),
        }
    }

    fn create_or_get_object_id_request(
        input_buffer_length: u32,
        output_buffer_length: u32,
    ) -> DeviceControlRequest<AnyIoCtlCode> {
        let mut req = get_compression_request(input_buffer_length, output_buffer_length);
        req.io_control_code = AnyIoCtlCode(FSCTL_CREATE_OR_GET_OBJECT_ID);
        req
    }

    fn query_allocated_ranges_request(
        input_buffer_length: u32,
        output_buffer_length: u32,
    ) -> DeviceControlRequest<AnyIoCtlCode> {
        let mut req = get_compression_request(input_buffer_length, output_buffer_length);
        req.io_control_code = AnyIoCtlCode(FSCTL_QUERY_ALLOCATED_RANGES);
        req
    }

    fn get_integrity_information_request(
        input_buffer_length: u32,
        output_buffer_length: u32,
    ) -> DeviceControlRequest<AnyIoCtlCode> {
        let mut req = get_compression_request(input_buffer_length, output_buffer_length);
        req.io_control_code = AnyIoCtlCode(FSCTL_GET_INTEGRITY_INFORMATION);
        req
    }

    fn allocated_range(file_offset: i64, length: i64) -> [u8; FILE_ALLOCATED_RANGE_BUFFER_SIZE] {
        let mut range = [0; FILE_ALLOCATED_RANGE_BUFFER_SIZE];
        range[..8].copy_from_slice(&file_offset.to_le_bytes());
        range[8..].copy_from_slice(&length.to_le_bytes());
        range
    }

    fn u32_size(value: usize) -> u32 {
        u32::try_from(value).expect("fixed protocol size fits in u32")
    }

    #[test]
    fn get_compression_requires_an_empty_input_buffer() {
        let req = get_compression_request(1, u32_size(COMPRESSION_FORMAT_SIZE));

        assert_eq!(
            validate_get_compression_request(&req, &[1]),
            Err(NtStatus::INVALID_PARAMETER)
        );
    }

    #[test]
    fn get_compression_requires_the_fixed_output_buffer() {
        let req = get_compression_request(0, u32_size(COMPRESSION_FORMAT_SIZE) - 1);

        assert_eq!(
            validate_get_compression_request(&req, &[]),
            Err(NtStatus::INVALID_PARAMETER)
        );
    }

    #[test]
    fn create_or_get_object_id_requires_an_empty_input_and_full_output_buffer() {
        let input = [1];
        let req = create_or_get_object_id_request(
            u32::try_from(input.len()).expect("request size fits in u32"),
            u32_size(FILE_OBJECTID_BUFFER_SIZE),
        );

        assert_eq!(
            validate_create_or_get_object_id_request(&req, &input),
            Err(NtStatus::INVALID_PARAMETER)
        );
        assert_eq!(
            validate_create_or_get_object_id_request(
                &create_or_get_object_id_request(0, u32_size(FILE_OBJECTID_BUFFER_SIZE) - 1),
                &[]
            ),
            Err(NtStatus::INVALID_PARAMETER)
        );
    }

    #[test]
    fn get_integrity_information_requires_an_empty_input_and_fixed_output_buffer() {
        let input = [1];
        let req = get_integrity_information_request(
            u32::try_from(input.len()).expect("request size fits in u32"),
            u32::try_from(INTEGRITY_INFORMATION_SIZE - 1).expect("response size fits in u32"),
        );

        assert_eq!(
            validate_get_integrity_information_request(&req, &input),
            Err(NtStatus::INVALID_PARAMETER)
        );
        assert_eq!(
            validate_get_integrity_information_request(
                &get_integrity_information_request(0, u32_size(INTEGRITY_INFORMATION_SIZE) - 1),
                &[]
            ),
            Err(NtStatus::INVALID_PARAMETER)
        );
    }

    #[test]
    fn device_control_rejects_declared_input_length_mismatch() {
        let req = get_compression_request(1, u32_size(COMPRESSION_FORMAT_SIZE));

        assert_eq!(
            validate_declared_input_length(&req, &[]),
            Err(NtStatus::INVALID_PARAMETER)
        );
    }

    #[test]
    fn query_allocated_ranges_requires_an_exact_input_buffer() {
        let req = query_allocated_ranges_request(
            u32::try_from(FILE_ALLOCATED_RANGE_BUFFER_SIZE - 1).expect("request size fits in u32"),
            u32::try_from(FILE_ALLOCATED_RANGE_BUFFER_SIZE).expect("response size fits in u32"),
        );

        assert_eq!(
            validate_query_allocated_ranges_request(&req, &[0; FILE_ALLOCATED_RANGE_BUFFER_SIZE - 1]),
            Err(NtStatus::INVALID_PARAMETER)
        );
    }

    #[test]
    fn query_allocated_ranges_rejects_invalid_ranges() {
        let req = query_allocated_ranges_request(
            u32::try_from(FILE_ALLOCATED_RANGE_BUFFER_SIZE).expect("request size fits in u32"),
            u32::try_from(FILE_ALLOCATED_RANGE_BUFFER_SIZE).expect("response size fits in u32"),
        );

        for range in [allocated_range(-1, 1), allocated_range(i64::MAX, 1)] {
            assert_eq!(
                validate_query_allocated_ranges_request(&req, &range),
                Err(NtStatus::INVALID_PARAMETER)
            );
        }
    }

    #[test]
    fn query_allocated_ranges_accepts_an_empty_range() {
        let range = allocated_range(0, 0);
        let req = query_allocated_ranges_request(
            u32::try_from(range.len()).expect("request size fits in u32"),
            u32::try_from(FILE_ALLOCATED_RANGE_BUFFER_SIZE).expect("response size fits in u32"),
        );

        assert!(validate_query_allocated_ranges_request(&req, &range).is_ok());
    }

    #[test]
    fn query_allocated_ranges_bounds_output_without_requiring_alignment() {
        let input = allocated_range(0, 1);
        let too_small = query_allocated_ranges_request(
            u32::try_from(input.len()).expect("request size fits in u32"),
            u32::try_from(FILE_ALLOCATED_RANGE_BUFFER_SIZE - 1).expect("response size fits in u32"),
        );
        assert_eq!(
            validate_query_allocated_ranges_request(&too_small, &input),
            Err(NtStatus::BUFFER_TOO_SMALL)
        );

        let unaligned = query_allocated_ranges_request(
            u32::try_from(input.len()).expect("request size fits in u32"),
            u32::try_from(FILE_ALLOCATED_RANGE_BUFFER_SIZE + 1).expect("response size fits in u32"),
        );
        assert!(validate_query_allocated_ranges_request(&unaligned, &input).is_ok());

        let too_large = query_allocated_ranges_request(
            u32::try_from(input.len()).expect("request size fits in u32"),
            u32::try_from(MAX_DEVICE_CONTROL_OUTPUT_SIZE + FILE_ALLOCATED_RANGE_BUFFER_SIZE)
                .expect("bounded output size fits in u32"),
        );
        assert_eq!(
            validate_query_allocated_ranges_request(&too_large, &input),
            Err(NtStatus::INVALID_PARAMETER)
        );
    }

    #[test]
    fn filesystem_controls_query_the_opened_file_handle() {
        let temporary_path = std::env::temp_dir().join(format!(
            "ironrdp-rdpdr-native-{}-{}.tmp",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after Unix epoch")
                .as_nanos()
        ));
        std::fs::write(&temporary_path, b"compression query").expect("create temporary file");
        let root_path = temporary_path
            .ancestors()
            .last()
            .expect("temporary file has a volume root")
            .to_owned();
        let relative_path = temporary_path
            .strip_prefix(&root_path)
            .expect("temporary file is beneath its volume root");

        {
            let drive = RedirectedDrive::new(1, "Test", root_path.clone(), false).expect("valid redirected drive");
            let mut backend = WindowsRdpdrBackend::from_active_drive(drive);
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
                    desired_access: DesiredAccess::FILE_READ_DATA_OR_FILE_LIST_DIRECTORY
                        | DesiredAccess::FILE_WRITE_ATTRIBUTES,
                    allocation_size: 0,
                    file_attributes: FileAttributes::empty(),
                    shared_access: SharedAccess::empty(),
                    create_disposition: CreateDisposition::FILE_OPEN,
                    create_options: CreateOptions::empty(),
                    path: format!(r"\{}", relative_path.display()),
                },
            )
            .expect("open temporary file");
            let mut req = get_compression_request(0, u32_size(COMPRESSION_FORMAT_SIZE));
            req.header.file_id = file_id;

            let format = handle_inner_with_input(&backend, &req, &[])
                .expect("query compression format")
                .output;
            let format = u16::from_le_bytes(format.0.try_into().expect("compression format has two bytes"));
            assert!(matches!(format, 0..=4));

            let mut req = create_or_get_object_id_request(0, u32_size(FILE_OBJECTID_BUFFER_SIZE));
            req.header.file_id = file_id;
            let object_id = handle_inner_with_input(&backend, &req, &[])
                .expect("create or query object ID through opened file")
                .output;
            assert_eq!(object_id.0.len(), FILE_OBJECTID_BUFFER_SIZE);

            let range = allocated_range(0, 4_096);
            let mut req = query_allocated_ranges_request(
                u32::try_from(range.len()).expect("allocated range request size fits in u32"),
                u32::try_from(FILE_ALLOCATED_RANGE_BUFFER_SIZE).expect("allocated range response size fits in u32"),
            );
            req.header.file_id = file_id;

            let completion =
                handle_inner_with_input(&backend, &req, &range).expect("query allocated ranges through opened file");
            assert_eq!(completion.status, NtStatus::SUCCESS);
            assert!(completion.output.0.len() <= FILE_ALLOCATED_RANGE_BUFFER_SIZE);
            assert_eq!(completion.output.0.len() % FILE_ALLOCATED_RANGE_BUFFER_SIZE, 0);

            let mut req = get_integrity_information_request(0, u32_size(INTEGRITY_INFORMATION_SIZE));
            req.header.file_id = file_id;
            match handle_inner_with_input(&backend, &req, &[]) {
                Ok(completion) => {
                    assert_eq!(completion.status, NtStatus::SUCCESS);
                    assert_eq!(completion.output.0.len(), INTEGRITY_INFORMATION_SIZE);
                }
                Err(status) => assert_eq!(status, NtStatus::INVALID_DEVICE_REQUEST),
            }
        }

        {
            let drive = RedirectedDrive::new(1, "Test", root_path, true).expect("valid read-only redirected drive");
            let mut backend = WindowsRdpdrBackend::from_active_drive(drive);
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
                    desired_access: DesiredAccess::FILE_READ_DATA_OR_FILE_LIST_DIRECTORY,
                    allocation_size: 0,
                    file_attributes: FileAttributes::empty(),
                    shared_access: SharedAccess::empty(),
                    create_disposition: CreateDisposition::FILE_OPEN,
                    create_options: CreateOptions::empty(),
                    path: format!(r"\{}", relative_path.display()),
                },
            )
            .expect("open temporary file on read-only redirected drive");
            let mut req = create_or_get_object_id_request(0, u32_size(FILE_OBJECTID_BUFFER_SIZE));
            req.header.file_id = file_id;

            let status = handle_inner_with_input(&backend, &req, &[])
                .expect_err("read-only redirected drive rejects object-ID creation");
            assert_eq!(status, NtStatus::MEDIA_WRITE_PROTECTED);
        }

        std::fs::remove_file(temporary_path).expect("remove temporary file");
    }
}
