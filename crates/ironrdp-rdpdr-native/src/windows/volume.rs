//! Windows RDPDR volume-information queries bound to an opened redirected file.

use core::mem::offset_of;

use ironrdp_pdu::PduResult;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    Boolean, ClientDriveQueryVolumeInformationResponse, ClientDriveSetVolumeInformationResponse,
    FileFsAttributeInformation, FileFsFullSizeInformation, FileFsSizeInformation, FileFsVolumeInformation,
    FileSystemAttributes, FileSystemInformationClass, FileSystemInformationClassLevel, NtStatus,
    ServerDriveQueryVolumeInformationRequest, ServerDriveSetVolumeInformationRequest,
};
use ironrdp_svc::SvcMessage;
use tracing::debug;
use windows::Wdk::Storage::FileSystem::{
    FILE_FS_ATTRIBUTE_INFORMATION, FileFsAttributeInformation as NativeFileFsAttributeInformation,
    FileFsFullSizeInformation as NativeFileFsFullSizeInformation, FileFsSizeInformation as NativeFileFsSizeInformation,
    FileFsVolumeInformation as NativeFileFsVolumeInformation,
};
use windows::Wdk::System::SystemServices::{
    FILE_FS_FULL_SIZE_INFORMATION, FILE_FS_SIZE_INFORMATION, FILE_FS_VOLUME_INFORMATION,
};

use super::backend::WindowsRdpdrBackend;
use super::file::file_for_request;
use super::status::from_ntstatus;

pub(crate) fn query_information(
    backend: &mut WindowsRdpdrBackend,
    req: ServerDriveQueryVolumeInformationRequest,
) -> PduResult<Vec<SvcMessage>> {
    let information_class = &req.fs_info_class_lvl;
    let (status, buffer) = match query_information_inner(backend, &req) {
        Ok(buffer) => (NtStatus::SUCCESS, Some(buffer)),
        Err(status) => (status, None),
    };
    let named_streams = match buffer.as_ref() {
        Some(FileSystemInformationClass::FileFsAttributeInformation(information)) => Some(
            information
                .file_system_attributes
                .contains(FileSystemAttributes::FILE_NAMED_STREAMS),
        ),
        _ => None,
    };
    debug!(
        ?information_class,
        ?status,
        ?named_streams,
        "Completed filesystem query-volume-information IRP"
    );

    Ok(vec![SvcMessage::from(
        RdpdrPdu::ClientDriveQueryVolumeInformationResponse(ClientDriveQueryVolumeInformationResponse::new(
            req.device_io_request,
            status,
            buffer,
        )),
    )])
}

/// Completes a volume-label request without modifying the local volume.
///
/// Redirected drives expose a confined view of a logical volume. Relabeling
/// would mutate host-wide metadata outside that view, so report an explicit
/// policy denial rather than treating the defined RDPDR request as unknown.
pub(crate) fn set_information(
    backend: &WindowsRdpdrBackend,
    req: ServerDriveSetVolumeInformationRequest,
) -> PduResult<Vec<SvcMessage>> {
    let status = file_for_request(backend, req.device_io_request.device_id, req.device_io_request.file_id)
        .map_or_else(|status| status, |_| NtStatus::ACCESS_DENIED);

    Ok(vec![SvcMessage::from(
        RdpdrPdu::ClientDriveSetVolumeInformationResponse(ClientDriveSetVolumeInformationResponse::new(req, status)),
    )])
}

pub(super) fn query_information_inner(
    backend: &WindowsRdpdrBackend,
    req: &ServerDriveQueryVolumeInformationRequest,
) -> Result<FileSystemInformationClass, NtStatus> {
    let file = file_for_request(backend, req.device_io_request.device_id, req.device_io_request.file_id)?;

    match req.fs_info_class_lvl {
        FileSystemInformationClassLevel::FILE_FS_VOLUME_INFORMATION => {
            let buffer = file
                .handle
                .query_volume_information(NativeFileFsVolumeInformation)
                .map_err(from_ntstatus)?;
            volume_information(&buffer).map(Into::into)
        }
        FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION => {
            let buffer = file
                .handle
                .query_volume_information(NativeFileFsSizeInformation)
                .map_err(from_ntstatus)?;
            let information = read_native::<FILE_FS_SIZE_INFORMATION>(&buffer)?;
            Ok(FileFsSizeInformation {
                total_alloc_units: information.TotalAllocationUnits,
                available_alloc_units: information.AvailableAllocationUnits,
                sectors_per_alloc_unit: information.SectorsPerAllocationUnit,
                bytes_per_sector: information.BytesPerSector,
            }
            .into())
        }
        FileSystemInformationClassLevel::FILE_FS_ATTRIBUTE_INFORMATION => {
            let buffer = file
                .handle
                .query_volume_information(NativeFileFsAttributeInformation)
                .map_err(from_ntstatus)?;
            attribute_information(&buffer, file.read_only).map(Into::into)
        }
        FileSystemInformationClassLevel::FILE_FS_FULL_SIZE_INFORMATION => {
            let buffer = file
                .handle
                .query_volume_information(NativeFileFsFullSizeInformation)
                .map_err(from_ntstatus)?;
            let information = read_native::<FILE_FS_FULL_SIZE_INFORMATION>(&buffer)?;
            Ok(FileFsFullSizeInformation {
                total_alloc_units: information.TotalAllocationUnits,
                caller_available_alloc_units: information.CallerAvailableAllocationUnits,
                actual_available_alloc_units: information.ActualAvailableAllocationUnits,
                sectors_per_alloc_unit: information.SectorsPerAllocationUnit,
                bytes_per_sector: information.BytesPerSector,
            }
            .into())
        }
        // `mstscax.dll!W32Drive::MsgIrpQueryVolumeInfo` recognizes only
        // FileFsVolumeInformation, FileFsSizeInformation, FileFsAttributeInformation,
        // and FileFsFullSizeInformation. It maps ERROR_INVALID_FUNCTION for this
        // otherwise valid MS-RDPEFS request to STATUS_INVALID_DEVICE_REQUEST.
        FileSystemInformationClassLevel::FILE_FS_DEVICE_INFORMATION => Err(NtStatus::INVALID_DEVICE_REQUEST),
        _ => Err(NtStatus::NOT_SUPPORTED),
    }
}

fn volume_information(buffer: &[u8]) -> Result<FileFsVolumeInformation, NtStatus> {
    let label_offset = offset_of!(FILE_FS_VOLUME_INFORMATION, VolumeLabel);
    let creation_time = read_i64(buffer, 0)?;
    let serial_number = read_u32(buffer, 8)?;
    let label_length = read_u32(buffer, 12)?;
    let supports_objects = read_u8(buffer, 16)?;

    let volume_label = read_utf16(buffer, label_offset, label_length)?;
    let volume_label = volume_label.strip_suffix('\0').unwrap_or(&volume_label);

    Ok(FileFsVolumeInformation {
        volume_creation_time: creation_time,
        volume_serial_number: serial_number,
        supports_objects: Boolean::from(supports_objects),
        // MS-FSCC limits the redirected volume label to 32 characters even
        // when the local file system reports a longer name.
        volume_label: volume_label.chars().take(32).collect(),
    })
}

fn attribute_information(buffer: &[u8], read_only: bool) -> Result<FileFsAttributeInformation, NtStatus> {
    let file_system_name_offset = offset_of!(FILE_FS_ATTRIBUTE_INFORMATION, FileSystemName);
    let file_system_attributes = read_u32(buffer, 0)?;
    let maximum_component_name_length = u32::try_from(read_i32(buffer, 4)?).map_err(|_| NtStatus::UNSUCCESSFUL)?;
    let file_system_name_length = read_u32(buffer, 8)?;
    let mut file_system_attributes = FileSystemAttributes::from_bits_retain(file_system_attributes);
    if read_only {
        file_system_attributes.insert(FileSystemAttributes::FILE_READ_ONLY_VOLUME);
    }

    Ok(FileFsAttributeInformation {
        file_system_attributes,
        max_component_name_len: maximum_component_name_length,
        file_system_name: read_utf16(buffer, file_system_name_offset, file_system_name_length)?,
    })
}

fn read_native<T: Copy>(buffer: &[u8]) -> Result<T, NtStatus> {
    let bytes = buffer.get(..size_of::<T>()).ok_or(NtStatus::UNSUCCESSFUL)?;

    // SAFETY: `bytes` contains a complete native structure; unaligned access is
    // required because the query buffer is stored as bytes.
    Ok(unsafe { bytes.as_ptr().cast::<T>().read_unaligned() })
}

fn read_i64(buffer: &[u8], offset: usize) -> Result<i64, NtStatus> {
    Ok(i64::from_le_bytes(read_array(buffer, offset)?))
}

fn read_i32(buffer: &[u8], offset: usize) -> Result<i32, NtStatus> {
    Ok(i32::from_le_bytes(read_array(buffer, offset)?))
}

fn read_u32(buffer: &[u8], offset: usize) -> Result<u32, NtStatus> {
    Ok(u32::from_le_bytes(read_array(buffer, offset)?))
}

fn read_u8(buffer: &[u8], offset: usize) -> Result<u8, NtStatus> {
    buffer.get(offset).copied().ok_or(NtStatus::UNSUCCESSFUL)
}

fn read_array<const N: usize>(buffer: &[u8], offset: usize) -> Result<[u8; N], NtStatus> {
    let end = offset.checked_add(N).ok_or(NtStatus::UNSUCCESSFUL)?;
    buffer
        .get(offset..end)
        .ok_or(NtStatus::UNSUCCESSFUL)?
        .try_into()
        .map_err(|_| NtStatus::UNSUCCESSFUL)
}

fn read_utf16(buffer: &[u8], offset: usize, byte_length: u32) -> Result<String, NtStatus> {
    let byte_length = usize::try_from(byte_length).map_err(|_| NtStatus::UNSUCCESSFUL)?;
    if byte_length % size_of::<u16>() != 0 {
        return Err(NtStatus::UNSUCCESSFUL);
    }

    let end = offset.checked_add(byte_length).ok_or(NtStatus::UNSUCCESSFUL)?;
    let bytes = buffer.get(offset..end).ok_or(NtStatus::UNSUCCESSFUL)?;
    let utf16 = bytes
        .chunks_exact(size_of::<u16>())
        .map(|code_unit| u16::from_le_bytes([code_unit[0], code_unit[1]]))
        .collect::<Vec<_>>();
    Ok(String::from_utf16_lossy(&utf16))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironrdp_rdpdr::pdu::efs::{
        CreateDisposition, CreateOptions, DesiredAccess, DeviceCreateRequest, DeviceIoRequest, FileAttributes,
        MajorFunction, MinorFunction, SharedAccess,
    };
    use windows::Wdk::Storage::FileSystem::FILE_OPEN;

    use crate::windows::factory::RedirectedDrive;

    #[test]
    fn read_only_volume_configuration_is_reported_to_the_server() {
        let attributes = attribute_information(
            &vec![0; offset_of!(FILE_FS_ATTRIBUTE_INFORMATION, FileSystemName)],
            true,
        )
        .expect("valid empty native attribute response");
        assert!(
            attributes
                .file_system_attributes
                .contains(FileSystemAttributes::FILE_READ_ONLY_VOLUME)
        );
    }

    #[test]
    fn volume_names_reject_malformed_native_lengths() {
        assert_eq!(read_utf16(&[], 0, 1), Err(NtStatus::UNSUCCESSFUL));
        assert_eq!(read_utf16(&[], 0, 2), Err(NtStatus::UNSUCCESSFUL));
    }

    #[test]
    fn volume_label_mutations_are_explicitly_denied() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set");
        let drive = RedirectedDrive::new(1, "Test", format!(r"{system_drive}\"), true).expect("valid redirected drive");
        let mut backend = WindowsRdpdrBackend::new(vec![drive]).expect("open redirected root");
        let (file_id, _) = crate::windows::file::create_inner(
            &mut backend,
            &DeviceCreateRequest {
                device_io_request: DeviceIoRequest {
                    device_id: 1,
                    file_id: 0,
                    completion_id: 1,
                    major_function: MajorFunction::Create,
                    minor_function: MinorFunction::from(0),
                },
                desired_access: DesiredAccess::from_bits_retain(0x0000_0080),
                allocation_size: 0,
                file_attributes: FileAttributes::empty(),
                shared_access: SharedAccess::from_bits_retain(0x0000_0007),
                create_disposition: CreateDisposition::from(FILE_OPEN.0),
                create_options: CreateOptions::from_bits_retain(0x0000_0001),
                path: r"\".to_owned(),
            },
        )
        .expect("open redirected root");
        let messages = set_information(
            &backend,
            ServerDriveSetVolumeInformationRequest {
                device_io_request: DeviceIoRequest {
                    device_id: 1,
                    file_id,
                    completion_id: 3,
                    major_function: MajorFunction::SetVolumeInformation,
                    minor_function: MinorFunction::from(0),
                },
                set_volume_buffer_length: 4,
                volume_label: "host volume".to_owned(),
            },
        )
        .expect("defined volume-label mutations have a completion");

        let response = messages
            .into_iter()
            .next()
            .expect("the request has one completion")
            .encode_unframed_pdu()
            .expect("the completion is encodable");
        let status = u32::from_le_bytes(response[12..16].try_into().expect("response status is present"));

        assert_eq!(status, u32::from(NtStatus::ACCESS_DENIED));
        backend.open_files.remove(file_id);
        let messages = set_information(
            &backend,
            ServerDriveSetVolumeInformationRequest {
                device_io_request: DeviceIoRequest {
                    device_id: 1,
                    file_id,
                    completion_id: 4,
                    major_function: MajorFunction::SetVolumeInformation,
                    minor_function: MinorFunction::from(0),
                },
                set_volume_buffer_length: 4,
                volume_label: "host volume".to_owned(),
            },
        )
        .expect("stale file IDs receive a completion");

        let response = messages
            .into_iter()
            .next()
            .expect("the request has one completion")
            .encode_unframed_pdu()
            .expect("the completion is encodable");
        let status = u32::from_le_bytes(response[12..16].try_into().expect("response status is present"));

        assert_eq!(status, u32::from(NtStatus::INVALID_HANDLE));
    }

    #[test]
    fn long_volume_labels_are_truncated_to_the_protocol_limit() {
        let label = "a".repeat(40);
        let label_offset = offset_of!(FILE_FS_VOLUME_INFORMATION, VolumeLabel);
        let mut buffer = vec![0; label_offset + label.len() * size_of::<u16>()];
        buffer[12..16]
            .copy_from_slice(&(u32::try_from(label.len() * size_of::<u16>()).expect("length fits")).to_le_bytes());
        for (offset, code_unit) in label.encode_utf16().enumerate() {
            let start = label_offset + offset * size_of::<u16>();
            buffer[start..start + size_of::<u16>()].copy_from_slice(&code_unit.to_le_bytes());
        }

        let information = volume_information(&buffer).expect("valid native volume information");

        assert_eq!(information.volume_label, "a".repeat(32));
    }

    #[test]
    fn volume_label_terminator_is_not_retained_in_the_protocol_value() {
        let label_offset = offset_of!(FILE_FS_VOLUME_INFORMATION, VolumeLabel);
        let mut buffer = vec![0; label_offset + 4];
        buffer[12..16].copy_from_slice(&4_u32.to_le_bytes());
        buffer[label_offset..label_offset + 4].copy_from_slice(&[b'C', 0, 0, 0]);

        let information = volume_information(&buffer).expect("valid terminated native volume information");

        assert_eq!(information.volume_label, "C");
    }
}
