use ironrdp_core::{ReadCursor, WriteCursor, encode_vec, impl_as_any};
use ironrdp_pdu::PduResult;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    AnyIoCtlCode, Boolean, Capabilities, CapabilityMessage, ClientDeviceListAnnounce, ClientDeviceListRemove,
    ClientDriveLockControlResponse, ClientDriveNotifyChangeDirectoryResponse, ClientDriveQueryDirectoryResponse,
    ClientDriveQueryInformationResponse, ClientDriveQuerySecurityResponse, ClientDriveQueryVolumeInformationResponse,
    ClientDriveSetInformationResponse, ClientDriveSetSecurityResponse, ClientNameRequest, ClientNameRequestUnicodeFlag,
    CoreCapability, CoreCapabilityKind, CreateDisposition, CreateOptions, DEFAULT_PRINTER_DRIVER_NAME, DesiredAccess,
    DeviceAnnounceHeader, DeviceCloseRequest, DeviceCloseResponse, DeviceControlRequest, DeviceControlResponse,
    DeviceCreateRequest, DeviceCreateResponse, DeviceFlushBuffersRequest, DeviceFlushBuffersResponse, DeviceIoRequest,
    DeviceIoResponse, DeviceReadRequest, DeviceReadResponse, DeviceType, DeviceWriteRequest, DeviceWriteResponse,
    Devices, FileAllocationInformation, FileAttributeTagInformation, FileAttributes, FileBasicInformation,
    FileBothDirectoryInformation, FileDirectoryInformation, FileEndOfFileInformation, FileFsSizeInformation,
    FileFullDirectoryInformation, FileInformationClass, FileInformationClassLevel, FileNamesInformation,
    FileStandardInformation, FileSystemInformationClass, FileSystemInformationClassLevel, Information, LockOperation,
    MajorFunction, MinorFunction, NtStatus, PRINTER_CAPABILITY_VERSION_01, RDPDR_PRINTER_ANNOUNCE_FLAG_DEFAULTPRINTER,
    RDPDR_PRINTER_ANNOUNCE_FLAG_NETWORKPRINTER, RdpLockInfo, SecurityInformation, ServerDeviceAnnounceResponse,
    ServerDriveIoRequest, ServerDriveLockControlRequest, ServerDriveNotifyChangeDirectoryRequest,
    ServerDriveQueryDirectoryRequest, ServerDriveQueryInformationRequest, ServerDriveQuerySecurityRequest,
    ServerDriveQueryVolumeInformationRequest, ServerDriveSetInformationRequest, ServerDriveSetSecurityRequest,
    SharedAccess, VERSION_MINOR_RDP51, VersionAndIdPdu, VersionAndIdPduKind,
};
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_rdpdr::server::{NoopRdpdrServerBackend, RdpdrServer, RdpdrServerBackend};
use ironrdp_rdpdr::{NoopRdpdrBackend, Rdpdr, RdpdrBackend};

/// Encodes via a plain `Vec<u8>` buffer for the server-side types in this file, which have
/// their own inherent `encode`/`size` rather than implementing the crate-wide `Encode` trait
/// (their `decode` takes caller-supplied context `encode_vec` above has no way to provide).
fn encode_to_vec(size: usize, f: impl FnOnce(&mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()>) -> Vec<u8> {
    let mut buf = vec![0u8; size];
    f(&mut WriteCursor::new(&mut buf)).unwrap();
    buf
}
use ironrdp_svc::{SvcMessage, SvcProcessor as _};

#[derive(Debug, Default)]
struct ResetTrackingRdpdrBackend {
    lifecycle: Vec<&'static str>,
}

impl_as_any!(ResetTrackingRdpdrBackend);

impl RdpdrBackend for ResetTrackingRdpdrBackend {
    fn reset(&mut self) -> PduResult<()> {
        self.lifecycle.push("reset");
        Ok(())
    }

    fn restore_drive(&mut self, _device_id: u32) -> PduResult<()> {
        self.lifecycle.push("restore_drive");
        Ok(())
    }

    fn handle_server_device_announce_response(&mut self, _pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        Ok(())
    }

    fn handle_scard_call(
        &mut self,
        _req: DeviceControlRequest<ScardIoCtlCode>,
        _call: ScardCall,
    ) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }

    fn handle_drive_io_request(&mut self, _req: ServerDriveIoRequest) -> PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }
}

fn read_u16(bytes: &[u8]) -> u16 {
    u16::from_le_bytes(bytes[..2].try_into().unwrap())
}

fn read_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().unwrap())
}

fn read_u32_as_usize(bytes: &[u8]) -> usize {
    usize::try_from(read_u32(bytes)).expect("u32 fits in usize on supported targets")
}

fn ntstatus_at(bytes: &[u8], offset: usize) -> NtStatus {
    NtStatus::from(read_u32(&bytes[offset..]))
}

fn utf16le_to_string(bytes: &[u8]) -> String {
    assert_eq!(bytes.len() % 2, 0, "UTF-16LE buffers must be even length");
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16(&units).expect("round-trip UTF-16LE decode")
}

fn encoded_printer_announce(device: DeviceAnnounceHeader) -> Vec<u8> {
    encode_vec(&RdpdrPdu::ClientDeviceListAnnounce(ClientDeviceListAnnounce {
        device_list: vec![device],
    }))
    .unwrap()
}

fn encoded_server_client_id_confirm(client_id: u32) -> Vec<u8> {
    encode_vec(&RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
        version_major: 1,
        version_minor: 12,
        client_id,
        kind: VersionAndIdPduKind::ServerClientIdConfirm,
    }))
    .unwrap()
}

fn encoded_server_client_id_confirm_with_minor(version_minor: u16, client_id: u32) -> Vec<u8> {
    encode_vec(&RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
        version_major: 1,
        version_minor,
        client_id,
        kind: VersionAndIdPduKind::ServerClientIdConfirm,
    }))
    .unwrap()
}

fn announce_client_id(rdpdr: &mut Rdpdr, version_minor: u16) -> u32 {
    let responses = rdpdr
        .process(
            &encode_vec(&RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
                version_major: 1,
                version_minor,
                client_id: 0x1234,
                kind: VersionAndIdPduKind::ServerAnnounceRequest,
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(responses.len(), 2);

    let encoded = responses[0].encode_unframed_pdu().unwrap();
    read_u32(&encoded[8..])
}

fn encoded_printer_device_io_request(major_function: MajorFunction) -> Vec<u8> {
    encode_vec(&RdpdrPdu::DeviceIoRequest(DeviceIoRequest {
        device_id: 42,
        file_id: 1,
        completion_id: 0x100,
        major_function,
        minor_function: MinorFunction::from(0),
    }))
    .unwrap()
}

fn encoded_printer_device_control_request() -> Vec<u8> {
    let mut encoded = encoded_printer_device_io_request(MajorFunction::DeviceControl);
    encoded.extend_from_slice(&0u32.to_le_bytes()); // OutputBufferLength
    encoded.extend_from_slice(&0u32.to_le_bytes()); // InputBufferLength
    encoded.extend_from_slice(&0u32.to_le_bytes()); // IoControlCode
    encoded.extend_from_slice(&[0; 20]); // Padding
    encoded
}

fn announced_devices(message: &SvcMessage) -> Vec<(u32, DeviceType)> {
    let encoded = message.encode_unframed_pdu().unwrap();
    assert_eq!(&encoded[..4], &[0x72, 0x44, 0x41, 0x44]); // RDPDR + DEVICELIST_ANNOUNCE

    let mut offset = 4;
    let device_count = read_u32_as_usize(&encoded[offset..]);
    offset += 4;

    let mut devices = Vec::with_capacity(device_count);
    for _ in 0..device_count {
        let device_type = DeviceType::try_from(read_u32(&encoded[offset..])).unwrap();
        offset += 4;

        let device_id = read_u32(&encoded[offset..]);
        offset += 4;

        offset += 8; // PreferredDosName

        let device_data_length = read_u32_as_usize(&encoded[offset..]);
        offset += 4 + device_data_length;

        devices.push((device_id, device_type));
    }

    assert_eq!(offset, encoded.len());
    devices
}

fn printer_device_data(encoded: &[u8]) -> &[u8] {
    assert_eq!(&encoded[..4], &[0x72, 0x44, 0x41, 0x44]); // RDPDR + DEVICELIST_ANNOUNCE

    let mut offset = 4;
    assert_eq!(read_u32(&encoded[offset..]), 1);
    offset += 4;

    assert_eq!(read_u32(&encoded[offset..]), u32::from(DeviceType::Print));
    offset += 4;

    assert_eq!(read_u32(&encoded[offset..]), 42);
    offset += 4;

    assert_eq!(&encoded[offset..offset + 8], b"PRN1\0\0\0\0");
    offset += 8;

    let device_data_length = read_u32_as_usize(&encoded[offset..]);
    offset += 4;

    let body = &encoded[offset..offset + device_data_length];
    assert_eq!(offset + device_data_length, encoded.len());
    body
}

#[test]
fn printer_capability_wire_layout() {
    let mut caps = Capabilities::new();
    caps.add_printer();

    let pdu = RdpdrPdu::CoreCapability(CoreCapability::new_response(caps.clone_inner()));
    let encoded = encode_vec(&pdu).unwrap();

    assert_eq!(&encoded[..4], &[0x72, 0x44, 0x50, 0x43]); // RDPDR + CLIENT_CAPABILITY
    assert_eq!(read_u16(&encoded[4..]), 2);

    let general_cap_offset = 8;
    assert_eq!(read_u16(&encoded[general_cap_offset..]), 0x0001);
    let general_cap_length = usize::from(read_u16(&encoded[general_cap_offset + 2..]));
    assert_eq!(general_cap_length, 44);
    assert_eq!(read_u32(&encoded[general_cap_offset + 4..]), 0x0000_0002);
    assert_eq!(read_u32(&encoded[general_cap_offset + general_cap_length - 4..]), 0);

    let printer_cap_offset = general_cap_offset + general_cap_length;
    assert_eq!(read_u16(&encoded[printer_cap_offset..]), 0x0002);
    assert_eq!(read_u16(&encoded[printer_cap_offset + 2..]), 8);
    assert_eq!(
        read_u32(&encoded[printer_cap_offset + 4..]),
        PRINTER_CAPABILITY_VERSION_01
    );
    assert_eq!(printer_cap_offset + 8, encoded.len());
}

#[test]
fn printer_announce_body_layout_matches_freerdp_postscript_defaults() {
    let encoded = encoded_printer_announce(DeviceAnnounceHeader::new_printer(42, "PrintMe".to_owned()));
    let body = printer_device_data(&encoded);

    assert!(body.len() >= 24);

    let flags = read_u32(&body[0..]);
    let code_page = read_u32(&body[4..]);
    let pnp_name_len = read_u32_as_usize(&body[8..]);
    let driver_name_len = read_u32_as_usize(&body[12..]);
    let print_name_len = read_u32_as_usize(&body[16..]);
    let cached_fields_len = read_u32_as_usize(&body[20..]);

    assert_eq!(
        flags,
        RDPDR_PRINTER_ANNOUNCE_FLAG_DEFAULTPRINTER | RDPDR_PRINTER_ANNOUNCE_FLAG_NETWORKPRINTER
    );
    assert_eq!(code_page, 0);
    assert_eq!(pnp_name_len, 0);
    assert_eq!(cached_fields_len, 0);

    let mut offset = 24;
    let pnp_bytes = &body[offset..offset + pnp_name_len];
    offset += pnp_name_len;
    let driver_bytes = &body[offset..offset + driver_name_len];
    offset += driver_name_len;
    let print_bytes = &body[offset..offset + print_name_len];
    offset += print_name_len;

    assert_eq!(offset, body.len());
    assert_eq!(utf16le_to_string(pnp_bytes), "");
    assert_eq!(utf16le_to_string(driver_bytes), DEFAULT_PRINTER_DRIVER_NAME);
    assert_eq!(utf16le_to_string(print_bytes), "PrintMe");
}

#[test]
fn printer_capability_is_not_echoed_when_server_does_not_advertise_it() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned()).with_printer(42, "PrintMe".to_owned());
    let server_capability = RdpdrPdu::CoreCapability(CoreCapability {
        capabilities: vec![CapabilityMessage::new_general(0)],
        kind: CoreCapabilityKind::ServerCoreCapabilityRequest,
    });

    let responses = rdpdr.process(&encode_vec(&server_capability).unwrap()).unwrap();
    assert_eq!(responses.len(), 1);

    let encoded = responses[0].encode_unframed_pdu().unwrap();
    assert_eq!(&encoded[..4], &[0x72, 0x44, 0x50, 0x43]); // RDPDR + CLIENT_CAPABILITY
    assert_eq!(read_u16(&encoded[4..]), 1);
}

#[test]
fn printer_announce_respects_explicit_driver() {
    let encoded = encoded_printer_announce(DeviceAnnounceHeader::new_printer_with_driver(
        42,
        "PDF Printer".to_owned(),
        "Microsoft Print To PDF".to_owned(),
    ));
    let body = printer_device_data(&encoded);

    let pnp_name_len = read_u32_as_usize(&body[8..]);
    let driver_name_len = read_u32_as_usize(&body[12..]);
    let driver_bytes = &body[24 + pnp_name_len..24 + pnp_name_len + driver_name_len];

    assert_eq!(utf16le_to_string(driver_bytes), "Microsoft Print To PDF");
}

#[test]
fn filesystem_drive_announce_encodes_unicode_data_and_valid_dos_name() {
    let drive_name = "share \u{1F4C1}";
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned())
        .with_drives(Some(vec![(42, drive_name.to_owned())]));

    let client_id = announce_client_id(&mut rdpdr, 12);
    assert!(
        rdpdr
            .process(&encoded_server_client_id_confirm(client_id))
            .unwrap()
            .is_empty()
    );
    let responses = rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap();
    assert_eq!(responses.len(), 1);

    let encoded = responses[0].encode_unframed_pdu().unwrap();
    assert_eq!(&encoded[..4], &[0x72, 0x44, 0x41, 0x44]); // RDPDR + DEVICELIST_ANNOUNCE
    assert_eq!(read_u32(&encoded[4..]), 1);
    assert_eq!(read_u32(&encoded[8..]), u32::from(DeviceType::Filesystem));
    assert_eq!(read_u32(&encoded[12..]), 42);
    assert_eq!(&encoded[16..24], b"share \0\0");

    let device_data_length = read_u32_as_usize(&encoded[24..]);
    let device_data = &encoded[28..28 + device_data_length];
    assert_eq!(28 + device_data_length, encoded.len());
    assert_eq!(utf16le_to_string(device_data), drive_name);
}

#[test]
fn server_announce_reopens_post_logon_drive_announcements() {
    let mut rdpdr =
        Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned()).with_drives(Some(vec![(42, "share".to_owned())]));

    let client_id = announce_client_id(&mut rdpdr, 12);
    assert!(
        rdpdr
            .process(&encoded_server_client_id_confirm(client_id))
            .unwrap()
            .is_empty()
    );
    let responses = rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap();
    assert_eq!(announced_devices(&responses[0]), vec![(42, DeviceType::Filesystem)]);

    let client_id = announce_client_id(&mut rdpdr, 12);
    assert!(
        rdpdr
            .process(&encoded_server_client_id_confirm(client_id))
            .unwrap()
            .is_empty()
    );
    let responses = rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap();
    assert_eq!(announced_devices(&responses[0]), vec![(42, DeviceType::Filesystem)]);
}

#[test]
fn server_announce_resets_backend_before_restoring_drives() {
    let mut rdpdr = Rdpdr::new(Box::new(ResetTrackingRdpdrBackend::default()), "IronRDP".to_owned())
        .with_drives(Some(vec![(42, "share".to_owned())]));

    announce_client_id(&mut rdpdr, 12);

    assert_eq!(
        rdpdr.downcast_backend::<ResetTrackingRdpdrBackend>().unwrap().lifecycle,
        ["reset", "restore_drive"]
    );
}

#[test]
fn client_announce_reply_uses_minor_version_13() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned());

    let responses = rdpdr
        .process(
            &encode_vec(&RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
                version_major: 1,
                version_minor: 12,
                client_id: 0x1234,
                kind: VersionAndIdPduKind::ServerAnnounceRequest,
            }))
            .unwrap(),
        )
        .unwrap();

    let encoded = responses[0].encode_unframed_pdu().unwrap();
    assert_eq!(read_u16(&encoded[4..]), 1);
    assert_eq!(read_u16(&encoded[6..]), 13);
}

#[test]
fn version_and_id_pdu_decode_client_announce_reply_tags_the_reply_not_the_confirm() {
    let original = VersionAndIdPdu {
        version_major: 1,
        version_minor: 13,
        client_id: 0x1234,
        // The value written by encode() doesn't depend on kind: it's only a decode-side tag.
        kind: VersionAndIdPduKind::ClientAnnounceReply,
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));

    let decoded = VersionAndIdPdu::decode_client_announce_reply(&mut ReadCursor::new(&encoded)).unwrap();

    assert_eq!(decoded.kind, VersionAndIdPduKind::ClientAnnounceReply);
    assert_eq!(decoded.version_major, original.version_major);
    assert_eq!(decoded.version_minor, original.version_minor);
    assert_eq!(decoded.client_id, original.client_id);
}

#[test]
fn devices_add_printer_appends_printer_entry() {
    let mut devices = Devices::new();
    devices.add_printer(9, "Lobby Printer".to_owned());

    assert_eq!(devices.for_device_type(9).unwrap(), DeviceType::Print);
}

#[test]
fn printer_device_announce_is_deferred_until_user_loggedon() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned()).with_printer(42, "PrintMe".to_owned());

    let client_id = announce_client_id(&mut rdpdr, 12);
    assert!(
        rdpdr
            .process(&encoded_server_client_id_confirm(client_id))
            .unwrap()
            .is_empty()
    );

    let responses = rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap();
    assert_eq!(responses.len(), 1);

    assert_eq!(announced_devices(&responses[0]), vec![(42, DeviceType::Print)]);

    assert!(
        rdpdr
            .process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn smartcard_device_announce_remains_pre_logon() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned())
        .with_smartcard(1)
        .with_printer(42, "PrintMe".to_owned());

    let client_id = announce_client_id(&mut rdpdr, 12);
    let responses = rdpdr.process(&encoded_server_client_id_confirm(client_id)).unwrap();
    assert_eq!(responses.len(), 1);

    assert_eq!(announced_devices(&responses[0]), vec![(1, DeviceType::Smartcard)]);

    let responses = rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap();
    assert_eq!(responses.len(), 1);

    assert_eq!(announced_devices(&responses[0]), vec![(42, DeviceType::Print)]);
}

#[test]
fn rdp51_client_id_confirm_announces_all_devices_pre_logon() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned())
        .with_smartcard(1)
        .with_printer(42, "PrintMe".to_owned());

    let client_id = announce_client_id(&mut rdpdr, VERSION_MINOR_RDP51);
    let responses = rdpdr
        .process(&encoded_server_client_id_confirm_with_minor(
            VERSION_MINOR_RDP51,
            client_id,
        ))
        .unwrap();
    assert_eq!(responses.len(), 1);

    assert_eq!(
        announced_devices(&responses[0]),
        vec![(1, DeviceType::Smartcard), (42, DeviceType::Print)]
    );

    assert!(
        rdpdr
            .process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn printer_device_control_is_completed_with_empty_success_response() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned()).with_printer(42, "PrintMe".to_owned());

    let responses = rdpdr.process(&encoded_printer_device_control_request()).unwrap();
    assert_eq!(responses.len(), 1);

    let encoded = responses[0].encode_unframed_pdu().unwrap();
    assert_eq!(&encoded[..4], &[0x72, 0x44, 0x43, 0x49]); // RDPDR + DEVICE_IOCOMPLETION
    assert_eq!(ntstatus_at(&encoded, 12), NtStatus::SUCCESS);
    assert_eq!(read_u32(&encoded[16..]), 0); // OutputBufferLength
}

#[test]
fn unsupported_printer_irp_is_completed_by_svc_processor() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned()).with_printer(42, "PrintMe".to_owned());

    let responses = rdpdr
        .process(&encoded_printer_device_io_request(MajorFunction::Read))
        .unwrap();
    assert_eq!(responses.len(), 1);

    let encoded = responses[0].encode_unframed_pdu().unwrap();
    assert_eq!(&encoded[..4], &[0x72, 0x44, 0x43, 0x49]); // RDPDR + DEVICE_IOCOMPLETION
    assert_eq!(ntstatus_at(&encoded, 12), NtStatus::NOT_SUPPORTED);
}

#[test]
fn printer_cache_pdu_is_ignored_before_decode() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned()).with_printer(42, "PrintMe".to_owned());
    let pdu = [0x72, 0x44, 0x43, 0x50, 1, 2, 3, 4]; // RDPDR + PAKID_PRN_CACHE_DATA + ignored body

    assert!(rdpdr.process(&pdu).unwrap().is_empty());
}

// ─── Server-side decode / encode round-trips ──────────────────────────────
//
// The tests above exercise the existing client-side `Rdpdr` processor. These cover the new
// server-side PDU codec surface: decode() on what a client sends (responses, device lists,
// name/capability requests) and encode() on what a server sends (I/O requests). Each type is
// round-tripped through its own encode/decode rather than through `RdpdrPdu`, except where the
// dispatch itself (`RdpdrPdu::decode` / `RdpdrPdu::decode_io_completion`) is what's under test.

fn some_device_io_response() -> DeviceIoResponse {
    DeviceIoResponse {
        device_id: 7,
        completion_id: 99,
        io_status: NtStatus::SUCCESS,
    }
}

#[test]
fn client_name_request_round_trips_ascii_and_unicode() {
    for (kind, name) in [
        (ClientNameRequestUnicodeFlag::Ascii, "workstation1"),
        (ClientNameRequestUnicodeFlag::Unicode, "\u{5DE5}\u{4F5C}\u{7AD9}"),
    ] {
        let original = ClientNameRequest::new(name.to_owned(), kind);
        let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
        let decoded = ClientNameRequest::decode(&mut ReadCursor::new(&encoded)).unwrap();
        assert_eq!(decoded, original);
    }
}

#[test]
fn core_capability_decodes_both_server_and_client_packet_ids() {
    let response = RdpdrPdu::CoreCapability(CoreCapability::new_response(vec![CapabilityMessage::new_general(0)]));
    let encoded = encode_vec(&response).unwrap();
    let RdpdrPdu::CoreCapability(decoded) = ironrdp_core::decode(&encoded).unwrap() else {
        panic!("expected CoreCapability");
    };
    assert_eq!(decoded.kind, CoreCapabilityKind::ClientCoreCapabilityResponse);
    assert_eq!(decoded.capabilities.len(), 1);
}

#[test]
fn client_device_list_announce_round_trips() {
    let original = ClientDeviceListAnnounce {
        device_list: vec![
            DeviceAnnounceHeader::new_smartcard(1),
            DeviceAnnounceHeader::new_printer(2, "Lobby".to_owned()),
        ],
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDeviceListAnnounce::decode(&mut ReadCursor::new(&encoded)).unwrap();

    assert_eq!(decoded.device_list.len(), 2);
    assert_eq!(decoded.device_list[0].device_id(), 1);
    assert_eq!(decoded.device_list[0].preferred_dos_name(), "SCARD");
    assert_eq!(decoded.device_list[1].device_id(), 2);
}

#[test]
fn client_device_list_remove_round_trips() {
    let original = ClientDeviceListRemove {
        device_list: vec![1, 2, 3],
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDeviceListRemove::decode(&mut ReadCursor::new(&encoded)).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn device_create_response_round_trips() {
    let original = DeviceCreateResponse {
        device_io_reply: some_device_io_response(),
        file_id: 5,
        information: Information::file_opened(),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded =
        DeviceCreateResponse::decode(some_device_io_response(), &mut ReadCursor::new(&encoded[12..])).unwrap();
    assert_eq!(decoded.file_id, original.file_id);
    assert_eq!(decoded.information, original.information);
}

#[test]
fn device_read_response_round_trips() {
    let original = DeviceReadResponse {
        device_io_reply: some_device_io_response(),
        read_data: b"hello, drive".to_vec(),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = DeviceReadResponse::decode(some_device_io_response(), &mut ReadCursor::new(&encoded[12..])).unwrap();
    assert_eq!(decoded.read_data, original.read_data);
}

#[test]
fn device_write_response_round_trips() {
    let original = DeviceWriteResponse {
        device_io_reply: some_device_io_response(),
        length: 4096,
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = DeviceWriteResponse::decode(some_device_io_response(), &mut ReadCursor::new(&encoded[12..])).unwrap();
    assert_eq!(decoded.length, original.length);
}

#[test]
fn device_control_response_decodes_output_buffer_as_raw_bytes() {
    // DeviceControlResponse::new's output_buffer is a generic rpce::Encode trait object with no
    // decode counterpart (see DeviceControlResponse::decode's doc comment), so this constructs
    // the wire bytes directly rather than round-tripping through encode().
    let mut encoded = Vec::new();
    encoded.extend_from_slice(&4u32.to_le_bytes()); // OutputBufferLength
    encoded.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);

    let decoded = DeviceControlResponse::decode(some_device_io_response(), &mut ReadCursor::new(&encoded)).unwrap();
    let output_buffer = decoded.output_buffer.expect("non-empty output buffer");
    assert_eq!(output_buffer.size(), 4);

    // Re-encoding the decoded raw buffer must reproduce the exact bytes read.
    let re_encoded = encode_to_vec(output_buffer.size(), |dst| output_buffer.encode(dst));
    assert_eq!(re_encoded, [0xAA, 0xBB, 0xCC, 0xDD]);
}

#[test]
fn device_control_response_decodes_empty_output_buffer() {
    let encoded = 0u32.to_le_bytes();
    let decoded = DeviceControlResponse::decode(some_device_io_response(), &mut ReadCursor::new(&encoded)).unwrap();
    assert!(decoded.output_buffer.is_none());
}

#[test]
fn client_drive_query_information_response_round_trips_basic_class() {
    let original = ClientDriveQueryInformationResponse {
        device_io_response: some_device_io_response(),
        buffer: Some(FileInformationClass::Basic(FileBasicInformation {
            creation_time: 1,
            last_access_time: 2,
            last_write_time: 3,
            change_time: 4,
            file_attributes: FileAttributes::empty(),
        })),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDriveQueryInformationResponse::decode_for_class(
        FileInformationClassLevel::FILE_BASIC_INFORMATION,
        some_device_io_response(),
        &mut ReadCursor::new(&encoded[12..]),
    )
    .unwrap();
    assert_eq!(decoded.buffer, original.buffer);
}

#[test]
fn client_drive_query_information_response_round_trips_standard_class() {
    let original = ClientDriveQueryInformationResponse {
        device_io_response: some_device_io_response(),
        buffer: Some(FileInformationClass::Standard(FileStandardInformation {
            allocation_size: 4096,
            end_of_file: 1234,
            number_of_links: 1,
            delete_pending: Boolean::False,
            directory: Boolean::True,
        })),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDriveQueryInformationResponse::decode_for_class(
        FileInformationClassLevel::FILE_STANDARD_INFORMATION,
        some_device_io_response(),
        &mut ReadCursor::new(&encoded[12..]),
    )
    .unwrap();
    assert_eq!(decoded.buffer, original.buffer);
}

#[test]
fn client_drive_query_information_response_round_trips_attribute_tag_class() {
    let original = ClientDriveQueryInformationResponse {
        device_io_response: some_device_io_response(),
        buffer: Some(FileInformationClass::AttributeTag(FileAttributeTagInformation {
            file_attributes: FileAttributes::FILE_ATTRIBUTE_DIRECTORY,
            reparse_tag: 0,
        })),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDriveQueryInformationResponse::decode_for_class(
        FileInformationClassLevel::FILE_ATTRIBUTE_TAG_INFORMATION,
        some_device_io_response(),
        &mut ReadCursor::new(&encoded[12..]),
    )
    .unwrap();
    assert_eq!(decoded.buffer, original.buffer);
}

#[test]
fn client_drive_query_information_response_round_trips_no_buffer() {
    let original = ClientDriveQueryInformationResponse {
        device_io_response: some_device_io_response(),
        buffer: None,
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDriveQueryInformationResponse::decode_for_class(
        FileInformationClassLevel::FILE_BASIC_INFORMATION,
        some_device_io_response(),
        &mut ReadCursor::new(&encoded[12..]),
    )
    .unwrap();
    assert!(decoded.buffer.is_none());
}

#[test]
fn client_drive_query_directory_response_round_trips_both_directory_class() {
    let original = ClientDriveQueryDirectoryResponse {
        device_io_reply: some_device_io_response(),
        buffer: Some(FileInformationClass::BothDirectory(FileBothDirectoryInformation::new(
            10,
            20,
            30,
            40,
            4096,
            FileAttributes::FILE_ATTRIBUTE_ARCHIVE,
            "example.txt".to_owned(),
        ))),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDriveQueryDirectoryResponse::decode_for_class(
        FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION,
        some_device_io_response(),
        &mut ReadCursor::new(&encoded[12..]),
    )
    .unwrap();
    assert_eq!(decoded.buffer, original.buffer);
}

#[test]
fn client_drive_query_directory_response_round_trips_full_directory_class() {
    let original = ClientDriveQueryDirectoryResponse {
        device_io_reply: some_device_io_response(),
        buffer: Some(FileInformationClass::FullDirectory(FileFullDirectoryInformation::new(
            10,
            20,
            30,
            40,
            4096,
            FileAttributes::FILE_ATTRIBUTE_ARCHIVE,
            "example.txt".to_owned(),
        ))),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDriveQueryDirectoryResponse::decode_for_class(
        FileInformationClassLevel::FILE_FULL_DIRECTORY_INFORMATION,
        some_device_io_response(),
        &mut ReadCursor::new(&encoded[12..]),
    )
    .unwrap();
    assert_eq!(decoded.buffer, original.buffer);
}

#[test]
fn client_drive_query_directory_response_round_trips_directory_class() {
    let original = ClientDriveQueryDirectoryResponse {
        device_io_reply: some_device_io_response(),
        buffer: Some(FileInformationClass::Directory(FileDirectoryInformation::new(
            10,
            20,
            30,
            40,
            4096,
            FileAttributes::FILE_ATTRIBUTE_ARCHIVE,
            "example.txt".to_owned(),
        ))),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDriveQueryDirectoryResponse::decode_for_class(
        FileInformationClassLevel::FILE_DIRECTORY_INFORMATION,
        some_device_io_response(),
        &mut ReadCursor::new(&encoded[12..]),
    )
    .unwrap();
    assert_eq!(decoded.buffer, original.buffer);
}

#[test]
fn client_drive_query_directory_response_round_trips_names_class() {
    let original = ClientDriveQueryDirectoryResponse {
        device_io_reply: some_device_io_response(),
        buffer: Some(FileInformationClass::Names(FileNamesInformation::new(
            "example.txt".to_owned(),
        ))),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDriveQueryDirectoryResponse::decode_for_class(
        FileInformationClassLevel::FILE_NAMES_INFORMATION,
        some_device_io_response(),
        &mut ReadCursor::new(&encoded[12..]),
    )
    .unwrap();
    assert_eq!(decoded.buffer, original.buffer);
}

#[test]
fn client_drive_query_security_response_round_trips() {
    let original = ClientDriveQuerySecurityResponse {
        device_io_response: some_device_io_response(),
        security_descriptor: Some(vec![1, 2, 3, 4]),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded =
        ClientDriveQuerySecurityResponse::decode(some_device_io_response(), &mut ReadCursor::new(&encoded[12..]))
            .unwrap();
    assert_eq!(decoded.security_descriptor, original.security_descriptor);
}

#[test]
fn client_drive_set_security_response_round_trips() {
    let set_request = ServerDriveSetSecurityRequest {
        device_io_request: some_device_io_request(MajorFunction::SetSecurity, MinorFunction::from(0)),
        security_information: SecurityInformation::DACL,
        security_descriptor: vec![1, 2, 3, 4, 5, 6],
    };
    let original = ClientDriveSetSecurityResponse::new(&set_request, NtStatus::SUCCESS).unwrap();
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded =
        ClientDriveSetSecurityResponse::decode(some_device_io_response(), &mut ReadCursor::new(&encoded[12..]))
            .unwrap();
    assert_eq!(decoded.length, original.length);
}

#[test]
fn device_flush_buffers_response_decodes() {
    let decoded = DeviceFlushBuffersResponse::decode(some_device_io_response());
    assert_eq!(decoded.size(), some_device_io_response().size());
}

#[test]
fn device_close_response_round_trips() {
    let original = DeviceCloseResponse {
        device_io_response: some_device_io_response(),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = DeviceCloseResponse::decode(some_device_io_response(), &mut ReadCursor::new(&encoded[12..])).unwrap();
    assert_eq!(decoded, original);
}

#[test]
fn client_drive_query_directory_response_round_trips_basic_class() {
    let original = ClientDriveQueryDirectoryResponse {
        device_io_reply: some_device_io_response(),
        buffer: Some(FileInformationClass::Basic(FileBasicInformation {
            creation_time: 10,
            last_access_time: 20,
            last_write_time: 30,
            change_time: 40,
            file_attributes: FileAttributes::empty(),
        })),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDriveQueryDirectoryResponse::decode_for_class(
        FileInformationClassLevel::FILE_BASIC_INFORMATION,
        some_device_io_response(),
        &mut ReadCursor::new(&encoded[12..]),
    )
    .unwrap();
    assert_eq!(decoded.buffer, original.buffer);
}

#[test]
fn client_drive_query_directory_response_round_trips_empty_buffer() {
    let original = ClientDriveQueryDirectoryResponse {
        device_io_reply: some_device_io_response(),
        buffer: None,
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDriveQueryDirectoryResponse::decode_for_class(
        FileInformationClassLevel::FILE_BASIC_INFORMATION,
        some_device_io_response(),
        &mut ReadCursor::new(&encoded[12..]),
    )
    .unwrap();
    assert!(decoded.buffer.is_none());
}

#[test]
fn client_drive_query_volume_information_response_round_trips() {
    let original = ClientDriveQueryVolumeInformationResponse {
        device_io_reply: some_device_io_response(),
        buffer: Some(FileSystemInformationClass::from(FileFsSizeInformation {
            total_alloc_units: 1000,
            available_alloc_units: 500,
            sectors_per_alloc_unit: 8,
            bytes_per_sector: 512,
        })),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDriveQueryVolumeInformationResponse::decode_for_class(
        FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION,
        some_device_io_response(),
        &mut ReadCursor::new(&encoded[12..]),
    )
    .unwrap();
    assert_eq!(decoded.buffer, original.buffer);
}

#[test]
fn client_drive_set_information_response_round_trips() {
    let set_request = ServerDriveSetInformationRequest {
        device_io_request: DeviceIoRequest {
            device_id: 1,
            file_id: 2,
            completion_id: 3,
            major_function: MajorFunction::SetInformation,
            minor_function: MinorFunction::from(0),
        },
        set_buffer: FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 42 }),
    };
    let original = ClientDriveSetInformationResponse::new(&set_request, NtStatus::SUCCESS).unwrap();
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded =
        ClientDriveSetInformationResponse::decode(some_device_io_response(), &mut ReadCursor::new(&encoded[12..]))
            .unwrap();
    // Fields are private; encoding both and comparing bytes is the round-trip check.
    let re_encoded = encode_to_vec(decoded.size(), |dst| decoded.encode(dst));
    assert_eq!(re_encoded[12..], encoded[12..]);
}

#[test]
fn client_drive_notify_change_directory_response_round_trips() {
    let original = ClientDriveNotifyChangeDirectoryResponse::new(
        DeviceIoRequest {
            device_id: 1,
            file_id: 2,
            completion_id: 3,
            major_function: MajorFunction::DirectoryControl,
            minor_function: MinorFunction::IRP_MN_NOTIFY_CHANGE_DIRECTORY,
        },
        NtStatus::SUCCESS,
        vec![9, 9, 9],
    );
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ClientDriveNotifyChangeDirectoryResponse::decode(
        some_device_io_response(),
        &mut ReadCursor::new(&encoded[12..]),
    )
    .unwrap();
    assert_eq!(decoded.buffer, original.buffer);
}

#[test]
fn client_drive_lock_control_response_round_trips() {
    let original = ClientDriveLockControlResponse::new(
        DeviceIoRequest {
            device_id: 1,
            file_id: 2,
            completion_id: 3,
            major_function: MajorFunction::LockControl,
            minor_function: MinorFunction::from(0),
        },
        NtStatus::SUCCESS,
    );
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded =
        ClientDriveLockControlResponse::decode(original.device_io_reply.clone(), &mut ReadCursor::new(&encoded[12..]))
            .unwrap();
    assert_eq!(decoded, original);
}

fn some_device_io_request(major_function: MajorFunction, minor_function: MinorFunction) -> DeviceIoRequest {
    DeviceIoRequest {
        device_id: 1,
        file_id: 2,
        completion_id: 3,
        major_function,
        minor_function,
    }
}

#[test]
fn device_create_request_round_trips() {
    let original = DeviceCreateRequest {
        device_io_request: some_device_io_request(MajorFunction::Create, MinorFunction::from(0)),
        desired_access: DesiredAccess::empty(),
        allocation_size: 0,
        file_attributes: FileAttributes::empty(),
        shared_access: SharedAccess::empty(),
        create_disposition: CreateDisposition::FILE_OPEN,
        create_options: CreateOptions::empty(),
        path: "\\subdir\\file.txt".to_owned(),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ServerDriveIoRequest::decode(
        some_device_io_request(MajorFunction::Create, MinorFunction::from(0)),
        &mut ReadCursor::new(&encoded[20..]),
    )
    .unwrap();
    let ServerDriveIoRequest::ServerCreateDriveRequest(decoded) = decoded else {
        panic!("expected ServerCreateDriveRequest");
    };
    assert_eq!(decoded.path, original.path);
}

#[test]
fn server_drive_query_information_request_round_trips() {
    const WIRE: [u8; 52] = [
        1, 0, 0, 0, // DeviceId
        2, 0, 0, 0, // FileId
        3, 0, 0, 0, // CompletionId
        5, 0, 0, 0, // MajorFunction
        0, 0, 0, 0, // MinorFunction
        4, 0, 0, 0, // FsInformationClass
        0, 0, 0, 0, // Length
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // Padding
    ];
    let original = ServerDriveQueryInformationRequest {
        device_io_request: some_device_io_request(MajorFunction::QueryInformation, MinorFunction::from(0)),
        file_info_class_lvl: FileInformationClassLevel::FILE_BASIC_INFORMATION,
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    assert_eq!(encoded, WIRE);
    let mut src = ReadCursor::new(&encoded[20..]);
    let decoded = ServerDriveQueryInformationRequest::decode(
        some_device_io_request(MajorFunction::QueryInformation, MinorFunction::from(0)),
        &mut src,
    )
    .unwrap();
    assert_eq!(decoded.file_info_class_lvl, original.file_info_class_lvl);
    assert!(src.is_empty());
}

#[test]
fn server_drive_query_information_request_consumes_query_buffer() {
    let original = ServerDriveQueryInformationRequest {
        device_io_request: some_device_io_request(MajorFunction::QueryInformation, MinorFunction::from(0)),
        file_info_class_lvl: FileInformationClassLevel::FILE_BASIC_INFORMATION,
    };
    let mut encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    encoded[24..28].copy_from_slice(&3u32.to_le_bytes());
    encoded.extend_from_slice(&[1, 2, 3]);

    let mut src = ReadCursor::new(&encoded[20..]);
    let decoded = ServerDriveQueryInformationRequest::decode(
        some_device_io_request(MajorFunction::QueryInformation, MinorFunction::from(0)),
        &mut src,
    )
    .unwrap();
    assert_eq!(decoded.file_info_class_lvl, original.file_info_class_lvl);
    assert!(src.is_empty());
}

#[test]
fn server_drive_query_information_request_rejects_truncated_fields() {
    let original = ServerDriveQueryInformationRequest {
        device_io_request: some_device_io_request(MajorFunction::QueryInformation, MinorFunction::from(0)),
        file_info_class_lvl: FileInformationClassLevel::FILE_BASIC_INFORMATION,
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let request = some_device_io_request(MajorFunction::QueryInformation, MinorFunction::from(0));

    assert!(
        ServerDriveQueryInformationRequest::decode(request.clone(), &mut ReadCursor::new(&encoded[20..51])).is_err()
    );

    let mut missing_query_buffer = encoded;
    missing_query_buffer[24..28].copy_from_slice(&1u32.to_le_bytes());
    assert!(
        ServerDriveQueryInformationRequest::decode(request, &mut ReadCursor::new(&missing_query_buffer[20..])).is_err()
    );
}

#[test]
fn server_drive_query_directory_request_round_trips_with_path() {
    let original = ServerDriveQueryDirectoryRequest {
        device_io_request: some_device_io_request(
            MajorFunction::DirectoryControl,
            MinorFunction::IRP_MN_QUERY_DIRECTORY,
        ),
        file_info_class_lvl: FileInformationClassLevel::FILE_DIRECTORY_INFORMATION,
        initial_query: 1,
        path: "*".to_owned(),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ServerDriveQueryDirectoryRequest::decode(
        some_device_io_request(MajorFunction::DirectoryControl, MinorFunction::IRP_MN_QUERY_DIRECTORY),
        &mut ReadCursor::new(&encoded[20..]),
    )
    .unwrap();
    assert_eq!(decoded.path, original.path);
    assert_eq!(decoded.initial_query, original.initial_query);
}

#[test]
fn server_drive_query_directory_request_round_trips_continuation() {
    let original = ServerDriveQueryDirectoryRequest {
        device_io_request: some_device_io_request(
            MajorFunction::DirectoryControl,
            MinorFunction::IRP_MN_QUERY_DIRECTORY,
        ),
        file_info_class_lvl: FileInformationClassLevel::FILE_DIRECTORY_INFORMATION,
        initial_query: 0,
        path: String::new(),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ServerDriveQueryDirectoryRequest::decode(
        some_device_io_request(MajorFunction::DirectoryControl, MinorFunction::IRP_MN_QUERY_DIRECTORY),
        &mut ReadCursor::new(&encoded[20..]),
    )
    .unwrap();
    assert_eq!(decoded.initial_query, 0);
    assert_eq!(decoded.path, "");
}

#[test]
fn server_drive_query_volume_information_request_round_trips() {
    let original = ServerDriveQueryVolumeInformationRequest {
        device_io_request: some_device_io_request(MajorFunction::QueryVolumeInformation, MinorFunction::from(0)),
        fs_info_class_lvl: FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION,
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ServerDriveQueryVolumeInformationRequest::decode(
        some_device_io_request(MajorFunction::QueryVolumeInformation, MinorFunction::from(0)),
        &mut ReadCursor::new(&encoded[20..]),
    )
    .unwrap();
    assert_eq!(decoded.fs_info_class_lvl, original.fs_info_class_lvl);
}

#[test]
fn server_drive_set_information_request_round_trips() {
    let original = ServerDriveSetInformationRequest {
        device_io_request: some_device_io_request(MajorFunction::SetInformation, MinorFunction::from(0)),
        set_buffer: FileInformationClass::Allocation(FileAllocationInformation { allocation_size: 8192 }),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ServerDriveSetInformationRequest::decode(
        some_device_io_request(MajorFunction::SetInformation, MinorFunction::from(0)),
        &mut ReadCursor::new(&encoded[20..]),
    )
    .unwrap();
    assert_eq!(decoded.set_buffer, original.set_buffer);
}

#[test]
fn server_drive_notify_change_directory_request_round_trips() {
    let original = ServerDriveNotifyChangeDirectoryRequest {
        device_io_request: some_device_io_request(
            MajorFunction::DirectoryControl,
            MinorFunction::IRP_MN_NOTIFY_CHANGE_DIRECTORY,
        ),
        watch_tree: 1,
        completion_filter: 0x17,
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ServerDriveNotifyChangeDirectoryRequest::decode(
        some_device_io_request(
            MajorFunction::DirectoryControl,
            MinorFunction::IRP_MN_NOTIFY_CHANGE_DIRECTORY,
        ),
        &mut ReadCursor::new(&encoded[20..]),
    )
    .unwrap();
    assert_eq!(decoded.watch_tree, original.watch_tree);
    assert_eq!(decoded.completion_filter, original.completion_filter);
}

#[test]
fn server_drive_query_security_request_round_trips() {
    let original = ServerDriveQuerySecurityRequest {
        device_io_request: some_device_io_request(MajorFunction::QuerySecurity, MinorFunction::from(0)),
        security_information: SecurityInformation::OWNER | SecurityInformation::DACL,
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ServerDriveQuerySecurityRequest::decode(
        some_device_io_request(MajorFunction::QuerySecurity, MinorFunction::from(0)),
        &mut ReadCursor::new(&encoded[20..]),
    )
    .unwrap();
    assert_eq!(decoded.security_information, original.security_information);
}

#[test]
fn server_drive_set_security_request_round_trips() {
    let original = ServerDriveSetSecurityRequest {
        device_io_request: some_device_io_request(MajorFunction::SetSecurity, MinorFunction::from(0)),
        security_information: SecurityInformation::DACL,
        security_descriptor: vec![1, 2, 3, 4, 5],
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ServerDriveSetSecurityRequest::decode(
        some_device_io_request(MajorFunction::SetSecurity, MinorFunction::from(0)),
        &mut ReadCursor::new(&encoded[20..]),
    )
    .unwrap();
    assert_eq!(decoded.security_descriptor, original.security_descriptor);
}

#[test]
fn server_drive_lock_control_request_round_trips() {
    let original = ServerDriveLockControlRequest {
        device_io_request: some_device_io_request(MajorFunction::LockControl, MinorFunction::from(0)),
        operation: LockOperation::Exclusive,
        wait: true,
        locks: vec![RdpLockInfo { length: 100, offset: 0 }],
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = ServerDriveLockControlRequest::decode(
        some_device_io_request(MajorFunction::LockControl, MinorFunction::from(0)),
        &mut ReadCursor::new(&encoded[20..]),
    )
    .unwrap();
    assert_eq!(decoded.locks, original.locks);
    assert_eq!(decoded.wait, original.wait);
}

#[test]
fn device_read_request_round_trips() {
    let original = DeviceReadRequest {
        device_io_request: some_device_io_request(MajorFunction::Read, MinorFunction::from(0)),
        length: 4096,
        offset: 1024,
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = DeviceReadRequest::decode(
        some_device_io_request(MajorFunction::Read, MinorFunction::from(0)),
        &mut ReadCursor::new(&encoded[20..]),
    )
    .unwrap();
    assert_eq!(decoded.length, original.length);
    assert_eq!(decoded.offset, original.offset);
}

#[test]
fn device_write_request_round_trips() {
    let original = DeviceWriteRequest {
        device_io_request: some_device_io_request(MajorFunction::Write, MinorFunction::from(0)),
        offset: 2048,
        write_data: b"payload".to_vec(),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = DeviceWriteRequest::decode(
        some_device_io_request(MajorFunction::Write, MinorFunction::from(0)),
        &mut ReadCursor::new(&encoded[20..]),
    )
    .unwrap();
    assert_eq!(decoded.write_data, original.write_data);
    assert_eq!(decoded.offset, original.offset);
}

#[test]
fn device_control_request_round_trips_any_io_ctl_code() {
    let original = DeviceControlRequest {
        header: some_device_io_request(MajorFunction::DeviceControl, MinorFunction::from(0)),
        output_buffer_length: 1024,
        input_buffer_length: 4,
        io_control_code: AnyIoCtlCode(0x0009_0014),
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = DeviceControlRequest::<AnyIoCtlCode>::decode(
        some_device_io_request(MajorFunction::DeviceControl, MinorFunction::from(0)),
        &mut ReadCursor::new(&encoded[20..]),
    )
    .unwrap();
    assert_eq!(decoded.output_buffer_length, original.output_buffer_length);
    assert_eq!(decoded.input_buffer_length, original.input_buffer_length);
    assert_eq!(decoded.io_control_code, original.io_control_code);
}

#[test]
fn device_control_request_round_trips_scard_io_ctl_code() {
    let original = DeviceControlRequest {
        header: some_device_io_request(MajorFunction::DeviceControl, MinorFunction::from(0)),
        output_buffer_length: 256,
        input_buffer_length: 0,
        io_control_code: ScardIoCtlCode::EstablishContext,
    };
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    let decoded = DeviceControlRequest::<ScardIoCtlCode>::decode(
        some_device_io_request(MajorFunction::DeviceControl, MinorFunction::from(0)),
        &mut ReadCursor::new(&encoded[20..]),
    )
    .unwrap();
    assert_eq!(decoded.output_buffer_length, original.output_buffer_length);
    assert_eq!(decoded.io_control_code, original.io_control_code);
}

#[test]
fn device_close_request_encodes_full_fixed_size() {
    let original = DeviceCloseRequest::decode(some_device_io_request(MajorFunction::Close, MinorFunction::from(0)));
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    assert_eq!(encoded.len(), original.size());
}

#[test]
fn device_flush_buffers_request_encodes() {
    let original = DeviceFlushBuffersRequest::decode(some_device_io_request(
        MajorFunction::FlushBuffers,
        MinorFunction::from(0),
    ));
    let encoded = encode_to_vec(original.size(), |dst| original.encode(dst));
    assert_eq!(encoded.len(), original.size());
}

#[test]
fn decode_io_completion_dispatches_create() {
    let decoded = RdpdrPdu::decode_io_completion(
        MajorFunction::Create,
        MinorFunction::from(0),
        None,
        None,
        some_device_io_response(),
        &mut ReadCursor::new(&[5, 0, 0, 0, 1]), // file_id=5, information=FILE_OPENED
    )
    .unwrap();
    assert!(matches!(decoded, RdpdrPdu::DeviceCreateResponse(_)));
}

#[test]
fn decode_io_completion_dispatches_directory_control_by_minor_function() {
    let query_dir = RdpdrPdu::decode_io_completion(
        MajorFunction::DirectoryControl,
        MinorFunction::IRP_MN_QUERY_DIRECTORY,
        Some(FileInformationClassLevel::FILE_BASIC_INFORMATION),
        None,
        some_device_io_response(),
        &mut ReadCursor::new(&0u32.to_le_bytes()),
    )
    .unwrap();
    assert!(matches!(query_dir, RdpdrPdu::ClientDriveQueryDirectoryResponse(_)));

    let notify_change = RdpdrPdu::decode_io_completion(
        MajorFunction::DirectoryControl,
        MinorFunction::IRP_MN_NOTIFY_CHANGE_DIRECTORY,
        None,
        None,
        some_device_io_response(),
        &mut ReadCursor::new(&0u32.to_le_bytes()),
    )
    .unwrap();
    assert!(matches!(
        notify_change,
        RdpdrPdu::ClientDriveNotifyChangeDirectoryResponse(_)
    ));
}

#[test]
fn decode_io_completion_requires_info_class_for_query_information() {
    let err = RdpdrPdu::decode_io_completion(
        MajorFunction::QueryInformation,
        MinorFunction::from(0),
        None,
        None,
        some_device_io_response(),
        &mut ReadCursor::new(&0u32.to_le_bytes()),
    )
    .unwrap_err();
    assert!(err.to_string().contains("info_class"));
}

#[test]
fn decode_io_completion_rejects_set_volume_information() {
    let err = RdpdrPdu::decode_io_completion(
        MajorFunction::SetVolumeInformation,
        MinorFunction::from(0),
        None,
        None,
        some_device_io_response(),
        &mut ReadCursor::new(&[]),
    )
    .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("majorfunction"));
}

#[derive(Debug, Default)]
struct TrackingServerBackend {
    accepted: Vec<u32>,
    rejected_device_ids: Vec<u32>,
    removed: Vec<u32>,
    client_name: Option<String>,
    completions: Vec<&'static str>,
}

impl_as_any!(TrackingServerBackend);

impl RdpdrServerBackend for TrackingServerBackend {
    fn on_device_announce(&mut self, devices: &[DeviceAnnounceHeader]) -> Vec<(u32, bool)> {
        devices
            .iter()
            .map(|device| {
                let device_id = device.device_id();
                let accepted = !self.rejected_device_ids.contains(&device_id);
                if accepted {
                    self.accepted.push(device_id);
                }
                (device_id, accepted)
            })
            .collect()
    }

    fn on_device_remove(&mut self, device_ids: &[u32]) {
        self.removed.extend_from_slice(device_ids);
    }

    fn on_client_name(&mut self, computer_name: &str) {
        self.client_name = Some(computer_name.to_owned());
    }

    fn on_create_complete(&mut self, _response: &DeviceCreateResponse) -> PduResult<()> {
        self.completions.push("create");
        Ok(())
    }
    fn on_close_complete(&mut self, _response: &DeviceCloseResponse) -> PduResult<()> {
        self.completions.push("close");
        Ok(())
    }
    fn on_read_complete(&mut self, _response: &DeviceReadResponse) -> PduResult<()> {
        self.completions.push("read");
        Ok(())
    }
    fn on_write_complete(&mut self, _response: &DeviceWriteResponse) -> PduResult<()> {
        self.completions.push("write");
        Ok(())
    }
    fn on_flush_buffers_complete(&mut self, _response: &DeviceFlushBuffersResponse) -> PduResult<()> {
        self.completions.push("flush_buffers");
        Ok(())
    }
    fn on_device_control_complete(&mut self, _response: &DeviceControlResponse) -> PduResult<()> {
        self.completions.push("device_control");
        Ok(())
    }
    fn on_query_information_complete(&mut self, _response: &ClientDriveQueryInformationResponse) -> PduResult<()> {
        self.completions.push("query_information");
        Ok(())
    }
    fn on_set_information_complete(&mut self, _response: &ClientDriveSetInformationResponse) -> PduResult<()> {
        self.completions.push("set_information");
        Ok(())
    }
    fn on_query_directory_complete(&mut self, _response: &ClientDriveQueryDirectoryResponse) -> PduResult<()> {
        self.completions.push("query_directory");
        Ok(())
    }
    fn on_notify_change_directory_complete(
        &mut self,
        _response: &ClientDriveNotifyChangeDirectoryResponse,
    ) -> PduResult<()> {
        self.completions.push("notify_change_directory");
        Ok(())
    }
    fn on_query_volume_information_complete(
        &mut self,
        _response: &ClientDriveQueryVolumeInformationResponse,
    ) -> PduResult<()> {
        self.completions.push("query_volume_information");
        Ok(())
    }
    fn on_lock_control_complete(&mut self, _response: &ClientDriveLockControlResponse) -> PduResult<()> {
        self.completions.push("lock_control");
        Ok(())
    }
    fn on_query_security_complete(&mut self, _response: &ClientDriveQuerySecurityResponse) -> PduResult<()> {
        self.completions.push("query_security");
        Ok(())
    }
    fn on_set_security_complete(&mut self, _response: &ClientDriveSetSecurityResponse) -> PduResult<()> {
        self.completions.push("set_security");
        Ok(())
    }
}

/// Drives a fresh [`RdpdrServer`] through the full MS-RDPEFS initialization handshake
/// (`start()` through `ClientCoreCapabilityResponse`), leaving it in the `Active` state.
/// Returns the `client_id` the server generated so callers can build further
/// server-perspective client replies if needed.
fn handshake_to_active(server: &mut RdpdrServer) -> u32 {
    let started = server.start().unwrap();
    assert_eq!(started.len(), 1);
    let client_id = read_u32(&started[0].encode_unframed_pdu().unwrap()[4..]);

    let announce_reply = encode_vec(&RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
        version_major: 1,
        version_minor: VERSION_MINOR_RDP51,
        client_id,
        kind: VersionAndIdPduKind::ClientAnnounceReply,
    }))
    .unwrap();
    assert!(server.process(&announce_reply).unwrap().is_empty());

    let name_request = encode_vec(&RdpdrPdu::ClientNameRequest(ClientNameRequest::new(
        "test-client".to_owned(),
        ClientNameRequestUnicodeFlag::Unicode,
    )))
    .unwrap();
    let capability_and_confirm = server.process(&name_request).unwrap();
    assert_eq!(capability_and_confirm.len(), 2);

    let capability_response = encode_vec(&RdpdrPdu::CoreCapability(CoreCapability {
        capabilities: vec![CapabilityMessage::new_general(0), CapabilityMessage::new_drive()],
        kind: CoreCapabilityKind::ClientCoreCapabilityResponse,
    }))
    .unwrap();
    assert!(server.process(&capability_response).unwrap().is_empty());

    client_id
}

/// Extracts the `CompletionId` a `drive_*` method embedded in a sent
/// `PAKID_CORE_DEVICE_IOREQUEST` (`SharedHeader` at bytes 0..4, then `DeviceIoRequest`'s
/// `DeviceId`/`FileId`/`CompletionId` at 4..8/8..12/12..16).
fn completion_id_of(message: &SvcMessage) -> u32 {
    read_u32(&message.encode_unframed_pdu().unwrap()[12..])
}

fn some_completion(device_id: u32, completion_id: u32) -> DeviceIoResponse {
    DeviceIoResponse {
        device_id,
        completion_id,
        io_status: NtStatus::SUCCESS,
    }
}

#[test]
fn server_start_sends_server_announce_request() {
    let mut server = RdpdrServer::new(Box::new(NoopRdpdrServerBackend));
    let started = server.start().unwrap();
    assert_eq!(started.len(), 1);
    let encoded = started[0].encode_unframed_pdu().unwrap();
    // Component::RdpdrCtypCore (0x4472) + PacketId::CoreServerAnnounce (0x496E), both LE u16.
    assert_eq!(&encoded[..4], &[0x72, 0x44, 0x6E, 0x49]);
}

#[test]
fn full_handshake_reaches_active_state() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    // Only reachable once the handshake landed the server in `Active`: earlier states
    // reject drive I/O outright (see `drive_io_before_active_is_rejected` below).
    assert!(server.drive_close(1, 1).is_ok());
    assert_eq!(
        server
            .downcast_backend::<TrackingServerBackend>()
            .unwrap()
            .client_name
            .as_deref(),
        Some("test-client")
    );
}

#[test]
fn drive_io_before_active_is_rejected() {
    let mut server = RdpdrServer::new(Box::new(NoopRdpdrServerBackend));
    let err = server.drive_close(1, 1).unwrap_err();
    assert!(err.to_string().to_lowercase().contains("active"));
}

#[test]
fn duplicate_client_announce_reply_while_active_reinitializes() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    // A stale in-flight IRP from before the re-init must not reach the backend once
    // its (now-cleared) CompletionId shows up in a completion.
    let stale = server.drive_close(1, 1).unwrap();
    let stale_completion_id = completion_id_of(&stale[0]);

    let duplicate_announce = encode_vec(&RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
        version_major: 1,
        version_minor: VERSION_MINOR_RDP51,
        client_id: 0,
        kind: VersionAndIdPduKind::ClientAnnounceReply,
    }))
    .unwrap();
    assert!(server.process(&duplicate_announce).unwrap().is_empty());

    // Back in AwaitingAnnounce: drive I/O is rejected again until the handshake redoes.
    assert!(server.drive_close(1, 1).is_err());

    // Re-run the handshake from ClientNameRequest onward (the ClientAnnounceReply above
    // already landed) and confirm the session is usable again.
    let name_request = encode_vec(&RdpdrPdu::ClientNameRequest(ClientNameRequest::new(
        "test-client".to_owned(),
        ClientNameRequestUnicodeFlag::Unicode,
    )))
    .unwrap();
    server.process(&name_request).unwrap();
    let capability_response = encode_vec(&RdpdrPdu::CoreCapability(CoreCapability {
        capabilities: vec![CapabilityMessage::new_general(0), CapabilityMessage::new_drive()],
        kind: CoreCapabilityKind::ClientCoreCapabilityResponse,
    }))
    .unwrap();
    server.process(&capability_response).unwrap();
    assert!(server.drive_close(1, 1).is_ok());

    // The stale completion is now orphaned: ignored, not routed to the backend.
    let stale_completion = encode_vec(&RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
        device_io_response: some_completion(1, stale_completion_id),
    }))
    .unwrap();
    assert!(server.process(&stale_completion).unwrap().is_empty());
    assert!(
        !server
            .downcast_backend::<TrackingServerBackend>()
            .unwrap()
            .completions
            .contains(&"close")
    );
}

#[test]
fn device_announce_accepts_and_rejects_per_backend_decision() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend {
        rejected_device_ids: vec![2],
        ..Default::default()
    }));
    handshake_to_active(&mut server);

    let announce = encode_vec(&RdpdrPdu::ClientDeviceListAnnounce(ClientDeviceListAnnounce {
        device_list: vec![
            DeviceAnnounceHeader::new_smartcard(1),
            DeviceAnnounceHeader::new_smartcard(2),
        ],
    }))
    .unwrap();
    let responses = server.process(&announce).unwrap();
    assert_eq!(responses.len(), 2);

    for response in &responses {
        let encoded = response.encode_unframed_pdu().unwrap();
        let device_id = read_u32(&encoded[4..]);
        let result_code = ntstatus_at(&encoded, 8);
        if device_id == 1 {
            assert_eq!(result_code, NtStatus::SUCCESS);
        } else {
            assert_eq!(device_id, 2);
            assert_eq!(result_code, NtStatus::ACCESS_DENIED);
        }
    }

    let backend = server.downcast_backend::<TrackingServerBackend>().unwrap();
    assert_eq!(backend.accepted, vec![1]);
}

#[test]
fn device_list_remove_notifies_backend() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let announce = encode_vec(&RdpdrPdu::ClientDeviceListAnnounce(ClientDeviceListAnnounce {
        device_list: vec![DeviceAnnounceHeader::new_smartcard(1)],
    }))
    .unwrap();
    server.process(&announce).unwrap();

    let remove = encode_vec(&RdpdrPdu::ClientDeviceListRemove(ClientDeviceListRemove {
        device_list: vec![1],
    }))
    .unwrap();
    assert!(server.process(&remove).unwrap().is_empty());
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().removed,
        vec![1]
    );
}

#[test]
fn orphaned_completion_id_is_ignored() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let orphan = encode_vec(&RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
        device_io_response: some_completion(1, 0xDEAD_BEEF),
    }))
    .unwrap();
    assert!(server.process(&orphan).unwrap().is_empty());
    assert!(
        server
            .downcast_backend::<TrackingServerBackend>()
            .unwrap()
            .completions
            .is_empty()
    );
}

#[test]
fn drive_create_completion_round_trips_response_data() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server
        .drive_create(
            1,
            "\\subdir\\file.txt",
            DesiredAccess::FILE_READ_DATA_OR_FILE_LIST_DIRECTORY,
            CreateDisposition::FILE_OPEN,
            CreateOptions::empty(),
        )
        .unwrap();
    assert_eq!(sent.len(), 1);
    let completion_id = completion_id_of(&sent[0]);

    let completion = encode_vec(&RdpdrPdu::DeviceCreateResponse(DeviceCreateResponse {
        device_io_reply: some_completion(1, completion_id),
        file_id: 7,
        information: Information::FILE_OPENED,
    }))
    .unwrap();
    assert!(server.process(&completion).unwrap().is_empty());
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["create"]
    );
}

#[test]
fn drive_close_completion_round_trips() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server.drive_close(1, 7).unwrap();
    let completion_id = completion_id_of(&sent[0]);
    let completion = encode_vec(&RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
        device_io_response: some_completion(1, completion_id),
    }))
    .unwrap();
    server.process(&completion).unwrap();
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["close"]
    );
}

#[test]
fn drive_read_completion_round_trips() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server.drive_read(1, 7, 4096, 0).unwrap();
    let completion_id = completion_id_of(&sent[0]);
    let completion = encode_vec(&RdpdrPdu::DeviceReadResponse(DeviceReadResponse {
        device_io_reply: some_completion(1, completion_id),
        read_data: vec![1, 2, 3],
    }))
    .unwrap();
    server.process(&completion).unwrap();
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["read"]
    );
}

#[test]
fn drive_write_completion_round_trips() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server.drive_write(1, 7, vec![1, 2, 3], 0).unwrap();
    let completion_id = completion_id_of(&sent[0]);
    let completion = encode_vec(&RdpdrPdu::DeviceWriteResponse(DeviceWriteResponse {
        device_io_reply: some_completion(1, completion_id),
        length: 3,
    }))
    .unwrap();
    server.process(&completion).unwrap();
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["write"]
    );
}

#[test]
fn drive_flush_buffers_completion_round_trips() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server.drive_flush_buffers(1, 7).unwrap();
    let completion_id = completion_id_of(&sent[0]);
    let completion = encode_vec(&RdpdrPdu::DeviceFlushBuffersResponse(DeviceFlushBuffersResponse {
        device_io_response: some_completion(1, completion_id),
    }))
    .unwrap();
    server.process(&completion).unwrap();
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["flush_buffers"]
    );
}

#[test]
fn drive_device_control_completion_round_trips() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server.drive_device_control(1, 7, 0x1234, vec![1, 2, 3], 0).unwrap();
    let completion_id = completion_id_of(&sent[0]);
    let completion = encode_vec(&RdpdrPdu::DeviceControlResponse(DeviceControlResponse {
        device_io_reply: some_completion(1, completion_id),
        output_buffer: None,
    }))
    .unwrap();
    server.process(&completion).unwrap();
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["device_control"]
    );
}

#[test]
fn drive_query_information_completion_round_trips() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server
        .drive_query_information(1, 7, FileInformationClassLevel::FILE_BASIC_INFORMATION)
        .unwrap();
    let completion_id = completion_id_of(&sent[0]);
    let completion = encode_vec(&RdpdrPdu::ClientDriveQueryInformationResponse(
        ClientDriveQueryInformationResponse {
            device_io_response: some_completion(1, completion_id),
            buffer: None,
        },
    ))
    .unwrap();
    server.process(&completion).unwrap();
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["query_information"]
    );
}

#[test]
fn drive_set_information_completion_round_trips() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server
        .drive_set_information(
            1,
            7,
            FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 0 }),
        )
        .unwrap();
    let completion_id = completion_id_of(&sent[0]);
    let request = ServerDriveSetInformationRequest {
        device_io_request: DeviceIoRequest {
            device_id: 1,
            file_id: 7,
            completion_id,
            major_function: MajorFunction::SetInformation,
            minor_function: MinorFunction::from(0),
        },
        set_buffer: FileInformationClass::EndOfFile(FileEndOfFileInformation { end_of_file: 0 }),
    };
    let response = ClientDriveSetInformationResponse::new(&request, NtStatus::SUCCESS).unwrap();
    let completion = encode_vec(&RdpdrPdu::ClientDriveSetInformationResponse(response)).unwrap();
    server.process(&completion).unwrap();
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["set_information"]
    );
}

#[test]
fn drive_query_directory_completion_round_trips() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server
        .drive_query_directory(
            1,
            7,
            FileInformationClassLevel::FILE_BOTH_DIRECTORY_INFORMATION,
            "*",
            true,
        )
        .unwrap();
    let completion_id = completion_id_of(&sent[0]);
    let completion = encode_vec(&RdpdrPdu::ClientDriveQueryDirectoryResponse(
        ClientDriveQueryDirectoryResponse {
            device_io_reply: some_completion(1, completion_id),
            buffer: None,
        },
    ))
    .unwrap();
    server.process(&completion).unwrap();
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["query_directory"]
    );
}

#[test]
fn drive_notify_change_directory_completion_round_trips() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server.drive_notify_change_directory(1, 7, true, 0x1).unwrap();
    let completion_id = completion_id_of(&sent[0]);
    let completion = encode_vec(&RdpdrPdu::ClientDriveNotifyChangeDirectoryResponse(
        ClientDriveNotifyChangeDirectoryResponse {
            device_io_reply: some_completion(1, completion_id),
            buffer: Vec::new(),
        },
    ))
    .unwrap();
    server.process(&completion).unwrap();
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["notify_change_directory"]
    );
}

#[test]
fn drive_query_volume_information_completion_round_trips() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server
        .drive_query_volume_information(1, 7, FileSystemInformationClassLevel::FILE_FS_SIZE_INFORMATION)
        .unwrap();
    let completion_id = completion_id_of(&sent[0]);
    let completion = encode_vec(&RdpdrPdu::ClientDriveQueryVolumeInformationResponse(
        ClientDriveQueryVolumeInformationResponse {
            device_io_reply: some_completion(1, completion_id),
            buffer: None,
        },
    ))
    .unwrap();
    server.process(&completion).unwrap();
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["query_volume_information"]
    );
}

#[test]
fn drive_lock_control_completion_round_trips() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server
        .drive_lock_control(
            1,
            7,
            LockOperation::Exclusive,
            true,
            vec![RdpLockInfo { length: 10, offset: 0 }],
        )
        .unwrap();
    let completion_id = completion_id_of(&sent[0]);
    let completion = encode_vec(&RdpdrPdu::ClientDriveLockControlResponse(
        ClientDriveLockControlResponse {
            device_io_reply: some_completion(1, completion_id),
        },
    ))
    .unwrap();
    server.process(&completion).unwrap();
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["lock_control"]
    );
}

#[test]
fn drive_query_security_completion_round_trips() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server.drive_query_security(1, 7, SecurityInformation::OWNER).unwrap();
    let completion_id = completion_id_of(&sent[0]);
    let completion = encode_vec(&RdpdrPdu::ClientDriveQuerySecurityResponse(
        ClientDriveQuerySecurityResponse {
            device_io_response: some_completion(1, completion_id),
            security_descriptor: None,
        },
    ))
    .unwrap();
    server.process(&completion).unwrap();
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["query_security"]
    );
}

#[test]
fn drive_set_security_completion_round_trips() {
    let mut server = RdpdrServer::new(Box::new(TrackingServerBackend::default()));
    handshake_to_active(&mut server);

    let sent = server
        .drive_set_security(1, 7, SecurityInformation::DACL, vec![0x01, 0x02])
        .unwrap();
    let completion_id = completion_id_of(&sent[0]);
    let completion = encode_vec(&RdpdrPdu::ClientDriveSetSecurityResponse(
        ClientDriveSetSecurityResponse {
            device_io_response: some_completion(1, completion_id),
            length: 0,
        },
    ))
    .unwrap();
    server.process(&completion).unwrap();
    assert_eq!(
        server.downcast_backend::<TrackingServerBackend>().unwrap().completions,
        vec!["set_security"]
    );
}
