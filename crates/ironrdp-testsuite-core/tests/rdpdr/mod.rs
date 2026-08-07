use core::sync::atomic::{AtomicUsize, Ordering};
use ironrdp_core::encode_vec;
use ironrdp_core::impl_as_any;
use ironrdp_pdu::PduResult;
use ironrdp_rdpdr::RdpdrBackend;
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    Capabilities, CapabilityMessage, ClientDeviceListAnnounce, CoreCapability, CoreCapabilityKind,
    DEFAULT_PRINTER_DRIVER_NAME, DeviceAnnounceHeader, DeviceControlRequest, DeviceIoRequest, DeviceType, Devices,
    MajorFunction, MinorFunction, NtStatus, PRINTER_CAPABILITY_VERSION_01, RDPDR_PRINTER_ANNOUNCE_FLAG_DEFAULTPRINTER,
    RDPDR_PRINTER_ANNOUNCE_FLAG_NETWORKPRINTER, ServerDeviceAnnounceResponse, VERSION_MINOR_RDP51, VersionAndIdPdu,
    VersionAndIdPduKind,
};
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_rdpdr::{NoopRdpdrBackend, Rdpdr};
use ironrdp_svc::{SvcMessage, SvcProcessor as _};
use std::sync::Arc;

#[derive(Debug)]
struct ResetTrackingBackend {
    resets: Arc<AtomicUsize>,
}

impl_as_any!(ResetTrackingBackend);

impl RdpdrBackend for ResetTrackingBackend {
    fn reset(&mut self) -> PduResult<()> {
        self.resets.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn handle_server_device_announce_response(&mut self, _pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        Ok(())
    }

    fn handle_scard_call(&mut self, _req: DeviceControlRequest<ScardIoCtlCode>, _call: ScardCall) -> PduResult<()> {
        Ok(())
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

fn encoded_server_announce_request() -> Vec<u8> {
    encode_vec(&RdpdrPdu::VersionAndIdPdu(VersionAndIdPdu {
        version_major: 1,
        version_minor: 12,
        client_id: 0x1234,
        kind: VersionAndIdPduKind::ServerAnnounceRequest,
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
    let client_id = read_u32(&encoded[8..]);
    assert_eq!(
        rdpdr
            .process(&encoded_server_capability_request())
            .expect("server capability request")
            .len(),
        1
    );
    client_id
}

fn encoded_server_capability_request() -> Vec<u8> {
    encode_vec(&RdpdrPdu::CoreCapability(CoreCapability {
        capabilities: vec![CapabilityMessage::new_general(0)],
        kind: CoreCapabilityKind::ServerCoreCapabilityRequest,
    }))
    .unwrap()
}

fn initialize_rdpdr(rdpdr: &mut Rdpdr) -> u32 {
    announce_client_id(rdpdr, 12)
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

fn encoded_drive_set_volume_information_request() -> Vec<u8> {
    let mut encoded = encode_vec(&RdpdrPdu::DeviceIoRequest(DeviceIoRequest {
        device_id: 42,
        file_id: 1,
        completion_id: 0x100,
        major_function: MajorFunction::SetVolumeInformation,
        minor_function: MinorFunction::from(0),
    }))
    .unwrap();
    encoded.extend_from_slice(&2u32.to_le_bytes()); // FileFsLabelInformation
    encoded.extend_from_slice(&6u32.to_le_bytes()); // Length
    encoded.extend_from_slice(&[0; 24]); // Padding
    encoded.extend_from_slice(b"\0\0\0\0\0\0"); // FileFsLabelInformation buffer
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

fn drive_device_data(encoded: &[u8]) -> (&[u8], &[u8]) {
    assert_eq!(&encoded[..4], &[0x72, 0x44, 0x41, 0x44]); // RDPDR + DEVICELIST_ANNOUNCE

    let mut offset = 4;
    assert_eq!(read_u32(&encoded[offset..]), 1);
    offset += 4;

    assert_eq!(read_u32(&encoded[offset..]), u32::from(DeviceType::Filesystem));
    offset += 4;

    offset += 4; // DeviceId
    let preferred_dos_name = &encoded[offset..offset + 8];
    offset += 8;

    let device_data_length = read_u32_as_usize(&encoded[offset..]);
    offset += 4;

    let device_data = &encoded[offset..offset + device_data_length];
    assert_eq!(offset + device_data_length, encoded.len());
    (preferred_dos_name, device_data)
}

#[test]
fn drive_announce_uses_unicode_device_data_and_a_valid_preferred_dos_name() {
    let mut rdpdr =
        Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned()).with_drives(Some(vec![(42, "C:".to_owned())]));

    let client_id = initialize_rdpdr(&mut rdpdr);
    assert!(
        rdpdr
            .process(&encoded_server_client_id_confirm(client_id))
            .unwrap()
            .is_empty()
    );
    let responses = rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap();

    assert_eq!(responses.len(), 1);
    let encoded = responses[0].encode_unframed_pdu().unwrap();
    let (preferred_dos_name, device_data) = drive_device_data(&encoded);

    assert_eq!(preferred_dos_name, b"C:\0\0\0\0\0\0");
    assert_eq!(device_data, b"C\0:\0\0\0");
    assert_eq!(utf16le_to_string(device_data), "C:");
}

#[test]
fn server_announce_resets_the_post_logon_device_gate() {
    let mut rdpdr =
        Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned()).with_drives(Some(vec![(42, "C:".to_owned())]));

    let client_id = initialize_rdpdr(&mut rdpdr);
    assert!(
        rdpdr
            .process(&encoded_server_client_id_confirm(client_id))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        announced_devices(&rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap()[0]),
        vec![(42, DeviceType::Filesystem)]
    );

    assert_eq!(rdpdr.process(&encoded_server_announce_request()).unwrap().len(), 2);
    assert_eq!(rdpdr.process(&encoded_server_capability_request()).unwrap().len(), 1);
    assert!(
        rdpdr
            .process(&encoded_server_client_id_confirm(0x1234))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        announced_devices(&rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap()[0]),
        vec![(42, DeviceType::Filesystem)]
    );
}

#[test]
fn server_announce_resets_the_backend_before_reinitialization() {
    let resets = Arc::new(AtomicUsize::new(0));
    let backend = ResetTrackingBackend {
        resets: Arc::clone(&resets),
    };
    let mut rdpdr = Rdpdr::new(Box::new(backend), "IronRDP".to_owned());

    assert_eq!(resets.load(Ordering::Relaxed), 0);
    assert_eq!(rdpdr.process(&encoded_server_announce_request()).unwrap().len(), 2);
    assert_eq!(resets.load(Ordering::Relaxed), 1);
}

#[test]
fn client_announce_advertises_minor_version_13() {
    let mut rdpdr = Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned());

    let responses = rdpdr.process(&encoded_server_announce_request()).unwrap();
    let client_announce = responses[0].encode_unframed_pdu().unwrap();

    assert_eq!(read_u16(&client_announce[4..]), 1);
    assert_eq!(read_u16(&client_announce[6..]), 13);
    assert_eq!(read_u32(&client_announce[8..]), 0x1234);
}

#[test]
fn unsupported_set_volume_information_is_completed_without_tearing_down_rdpdr() {
    let mut rdpdr =
        Rdpdr::new(Box::new(NoopRdpdrBackend), "IronRDP".to_owned()).with_drives(Some(vec![(42, "C:".to_owned())]));

    let responses = rdpdr.process(&encoded_drive_set_volume_information_request()).unwrap();
    assert_eq!(responses.len(), 1);

    let encoded = responses[0].encode_unframed_pdu().unwrap();
    assert_eq!(&encoded[..4], &[0x72, 0x44, 0x43, 0x49]); // RDPDR + DEVICE_IOCOMPLETION
    assert_eq!(read_u32(&encoded[4..]), 42);
    assert_eq!(read_u32(&encoded[8..]), 0x100);
    assert_eq!(ntstatus_at(&encoded, 12), NtStatus::NOT_SUPPORTED);
    assert_eq!(read_u32(&encoded[16..]), 6);
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

    rdpdr.process(&encoded_server_announce_request()).unwrap();
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
