use ironrdp_rdpdr::pdu::efs::{
    CreateDisposition, CreateOptions, DesiredAccess, FileDispositionInformation, FileInformationClass,
    FileInformationClassLevel, FileSystemInformationClassLevel, LockOperation, RdpLockInfo, SecurityInformation,
};
use ironrdp_server::RdpdrServerMessage;

/// One value per `RdpdrServerMessage` variant, matching `ironrdp-rdpdr::server::RdpdrServer`'s
/// fourteen `drive_*` methods. `RdpServer::dispatch_server_events` matches this enum
/// exhaustively (no wildcard arm) to route each variant to its `drive_*` call; this test
/// exercises the same exhaustive match here so that a future variant added to one side
/// without the other fails to compile instead of silently falling through a wildcard.
fn all_variants() -> Vec<RdpdrServerMessage> {
    vec![
        RdpdrServerMessage::Create {
            device_id: 1,
            path: "\\file.txt".to_owned(),
            desired_access: DesiredAccess::empty(),
            create_disposition: CreateDisposition::FILE_OPEN,
            create_options: CreateOptions::empty(),
        },
        RdpdrServerMessage::Read {
            device_id: 1,
            file_id: 2,
            length: 4096,
            offset: 0,
        },
        RdpdrServerMessage::Write {
            device_id: 1,
            file_id: 2,
            data: vec![1, 2, 3],
            offset: 0,
        },
        RdpdrServerMessage::Close {
            device_id: 1,
            file_id: 2,
        },
        RdpdrServerMessage::FlushBuffers {
            device_id: 1,
            file_id: 2,
        },
        RdpdrServerMessage::QueryInformation {
            device_id: 1,
            file_id: 2,
            info_class: FileInformationClassLevel::FILE_BASIC_INFORMATION,
        },
        RdpdrServerMessage::SetInformation {
            device_id: 1,
            file_id: 2,
            set_buffer: FileInformationClass::Disposition(FileDispositionInformation { delete_pending: 1 }),
        },
        RdpdrServerMessage::QueryDirectory {
            device_id: 1,
            file_id: 2,
            info_class: FileInformationClassLevel::FILE_DIRECTORY_INFORMATION,
            path: "\\".to_owned(),
            initial_query: true,
        },
        RdpdrServerMessage::NotifyChangeDirectory {
            device_id: 1,
            file_id: 2,
            watch_tree: false,
            completion_filter: 0,
        },
        RdpdrServerMessage::QueryVolumeInformation {
            device_id: 1,
            file_id: 2,
            fs_info_class: FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION,
        },
        RdpdrServerMessage::LockControl {
            device_id: 1,
            file_id: 2,
            operation: LockOperation::Shared,
            wait: false,
            locks: vec![RdpLockInfo { length: 1, offset: 0 }],
        },
        RdpdrServerMessage::QuerySecurity {
            device_id: 1,
            file_id: 2,
            security_information: SecurityInformation::empty(),
        },
        RdpdrServerMessage::SetSecurity {
            device_id: 1,
            file_id: 2,
            security_information: SecurityInformation::empty(),
            security_descriptor: vec![0u8; 4],
        },
        RdpdrServerMessage::DeviceControl {
            device_id: 1,
            file_id: 2,
            io_control_code: 0,
            input_buffer: Vec::new(),
            output_buffer_length: 0,
        },
    ]
}

/// Every variant clones and formats without panicking, and matches exhaustively (no
/// wildcard arm) against the fourteen variants above.
#[test]
fn rdpdr_server_message_variants_clone_debug_and_match_exhaustively() {
    for message in all_variants() {
        let cloned = message.clone();
        let _ = format!("{cloned:?}");

        match cloned {
            RdpdrServerMessage::Create { .. }
            | RdpdrServerMessage::Read { .. }
            | RdpdrServerMessage::Write { .. }
            | RdpdrServerMessage::Close { .. }
            | RdpdrServerMessage::FlushBuffers { .. }
            | RdpdrServerMessage::QueryInformation { .. }
            | RdpdrServerMessage::SetInformation { .. }
            | RdpdrServerMessage::QueryDirectory { .. }
            | RdpdrServerMessage::NotifyChangeDirectory { .. }
            | RdpdrServerMessage::QueryVolumeInformation { .. }
            | RdpdrServerMessage::LockControl { .. }
            | RdpdrServerMessage::QuerySecurity { .. }
            | RdpdrServerMessage::SetSecurity { .. }
            | RdpdrServerMessage::DeviceControl { .. } => {}
        }
    }
}
