//! MS-RDPESP serial and parallel port redirection.

use ironrdp_core::{Decode as _, ReadCursor, encode_vec, impl_as_any};
use ironrdp_pdu::{PduResult, pdu_other_err};
use ironrdp_rdpdr::pdu::RdpdrPdu;
use ironrdp_rdpdr::pdu::efs::{
    Capabilities, CapabilityMessage, ClientDeviceListAnnounce, CoreCapability, CoreCapabilityKind, DeviceCloseResponse,
    DeviceControlRequest, DeviceControlResponse, DeviceIoRequest, DeviceIoResponse, DeviceType, Devices, MajorFunction,
    MinorFunction, NtStatus, PORT_CAPABILITY_VERSION_01, ServerDeviceAnnounceResponse, ServerDriveIoRequest,
};
use ironrdp_rdpdr::pdu::esc::{ScardCall, ScardIoCtlCode};
use ironrdp_rdpdr::pdu::esp::{PortIoCtlCode, PortIoRequest, SerialQueueSize, SerialStatus, SerialTimeouts};
use ironrdp_rdpdr::{NoopRdpdrBackend, Rdpdr, RdpdrBackend, RdpdrPort, RdpdrPortKind};
use ironrdp_svc::{SvcMessage, SvcProcessor as _};

use super::{announce_client_id, announced_devices, encoded_server_client_id_confirm, ntstatus_at, read_u16, read_u32};

const SERIAL_ID: u32 = 2;
const PARALLEL_ID: u32 = 3;

/// Records the port requests it receives and completes them as a local driver would.
#[derive(Debug, Default)]
struct RecordingPortBackend {
    requests: Vec<PortIoRequest>,
    rejected_device_ids: Vec<u32>,
}

impl_as_any!(RecordingPortBackend);

impl RdpdrBackend for RecordingPortBackend {
    fn handle_server_device_announce_response(&mut self, pdu: ServerDeviceAnnounceResponse) -> PduResult<()> {
        if pdu.result_code != NtStatus::SUCCESS {
            self.rejected_device_ids.push(pdu.device_id);
        }
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
        Err(pdu_other_err!("port test backend received a filesystem IRP"))
    }

    fn handle_port_io_request(&mut self, req: PortIoRequest) -> PduResult<Vec<SvcMessage>> {
        let response = match &req {
            PortIoRequest::DeviceControl(control)
                if control.request.io_control_code == PortIoCtlCode::IOCTL_SERIAL_GET_BAUD_RATE =>
            {
                RdpdrPdu::DeviceControlResponse(DeviceControlResponse::with_raw_output(
                    control.request.header.clone(),
                    NtStatus::SUCCESS,
                    9600u32.to_le_bytes().to_vec(), // SERIAL_BAUD_RATE
                ))
            }
            PortIoRequest::Close(close) => RdpdrPdu::DeviceCloseResponse(DeviceCloseResponse {
                device_io_response: DeviceIoResponse::new(close.device_io_request.clone(), NtStatus::SUCCESS),
            }),
            other => other.failure_response(NtStatus::SUCCESS),
        };
        self.requests.push(req);
        Ok(vec![SvcMessage::from(response)])
    }
}

fn port_channel(backend: Box<dyn RdpdrBackend>) -> Rdpdr {
    Rdpdr::new(backend, "test".to_owned())
        .with_port(RdpdrPort::new(SERIAL_ID, RdpdrPortKind::Serial, "COM2".to_owned()).unwrap())
        .with_port(RdpdrPort::new(PARALLEL_ID, RdpdrPortKind::Parallel, "LPT1".to_owned()).unwrap())
}

/// Messages the client sent while initializing.
struct Initialized {
    client_capability: Vec<u8>,
    client_id_confirmed: Vec<SvcMessage>,
    logged_on: Vec<SvcMessage>,
}

/// Runs the initialization sequence up to User Logged On.
fn initialize(rdpdr: &mut Rdpdr, server_supports_ports: bool) -> Initialized {
    let client_id = announce_client_id(rdpdr, 12);
    let mut capabilities = vec![CapabilityMessage::new_general(0)];
    if server_supports_ports {
        capabilities.push(CapabilityMessage::new_port());
    }
    let server_capability = RdpdrPdu::CoreCapability(CoreCapability {
        capabilities,
        kind: CoreCapabilityKind::ServerCoreCapabilityRequest,
    });
    let responses = rdpdr.process(&encode_vec(&server_capability).unwrap()).unwrap();
    assert_eq!(responses.len(), 1);
    let client_capability = responses[0].encode_unframed_pdu().unwrap();

    let client_id_confirmed = rdpdr.process(&encoded_server_client_id_confirm(client_id)).unwrap();
    let logged_on = rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap();
    Initialized {
        client_capability,
        client_id_confirmed,
        logged_on,
    }
}

fn respond_to_announce(rdpdr: &mut Rdpdr, device_id: u32, result_code: NtStatus) {
    let responses = rdpdr
        .process(
            &encode_vec(&RdpdrPdu::ServerDeviceAnnounceResponse(ServerDeviceAnnounceResponse {
                device_id,
                result_code,
            }))
            .unwrap(),
        )
        .unwrap();
    assert!(responses.is_empty());
}

fn acknowledge(rdpdr: &mut Rdpdr, device_id: u32) {
    respond_to_announce(rdpdr, device_id, NtStatus::SUCCESS);
}

fn acknowledged_ports(backend: Box<dyn RdpdrBackend>) -> Rdpdr {
    let mut rdpdr = port_channel(backend);
    let initialized = initialize(&mut rdpdr, true);
    assert_eq!(initialized.client_id_confirmed.len(), 1);
    assert_eq!(initialized.logged_on.len(), 1);
    acknowledge(&mut rdpdr, SERIAL_ID);
    acknowledge(&mut rdpdr, PARALLEL_ID);
    rdpdr
}

fn encoded_port_request(device_id: u32, major_function: MajorFunction, body: &[u8]) -> Vec<u8> {
    let mut encoded = encode_vec(&RdpdrPdu::DeviceIoRequest(DeviceIoRequest {
        device_id,
        file_id: 2,
        completion_id: 0x10,
        major_function,
        minor_function: MinorFunction::from(0),
    }))
    .unwrap();
    encoded.extend_from_slice(body);
    encoded
}

fn encoded_get_baud_rate_request(device_id: u32) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&4u32.to_le_bytes()); // OutputBufferLength
    body.extend_from_slice(&0u32.to_le_bytes()); // InputBufferLength
    body.extend_from_slice(&PortIoCtlCode::IOCTL_SERIAL_GET_BAUD_RATE.0.to_le_bytes());
    body.extend_from_slice(&[0; 20]); // Padding
    encoded_port_request(device_id, MajorFunction::DeviceControl, &body)
}

fn recorded_requests(rdpdr: &Rdpdr) -> &[PortIoRequest] {
    &rdpdr.downcast_backend::<RecordingPortBackend>().unwrap().requests
}

#[test]
fn port_capability_wire_layout() {
    let mut capabilities = Capabilities::new();
    capabilities.add_port();
    capabilities.add_port();

    let encoded = encode_vec(&RdpdrPdu::CoreCapability(CoreCapability::new_response(
        capabilities.clone_inner(),
    )))
    .unwrap();
    assert_eq!(read_u16(&encoded[4..]), 2, "the port capability is added once");
    // CAP_PORT_TYPE, CapabilityLength 8, PORT_CAPABILITY_VERSION_01, no body.
    assert_eq!(&encoded[8 + 44..], &[0x03, 0x00, 0x08, 0x00, 0x01, 0x00, 0x00, 0x00]);
    assert_eq!(PORT_CAPABILITY_VERSION_01, 1);

    let RdpdrPdu::CoreCapability(decoded) = ironrdp_core::decode(&encoded).unwrap() else {
        panic!("expected CoreCapability");
    };
    assert!(decoded.supports_port());
    assert!(!decoded.supports_printer());
}

/// [MS-RDPESP] 4.1 shows these exact announcements for COM2 and LPT1.
#[test]
fn port_announces_match_ms_rdpesp_annotations() {
    let announce = |device_id, device_type, name: &str| {
        let mut devices = Devices::new();
        devices.add_port(device_id, device_type, name.to_owned());
        encode_vec(&RdpdrPdu::ClientDeviceListAnnounce(ClientDeviceListAnnounce {
            device_list: devices.clone_inner(),
        }))
        .unwrap()
    };
    let com2 = announce(1, DeviceType::Serial, "COM2");
    assert_eq!(
        com2,
        [
            0x72, 0x44, 0x41, 0x44, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x43, 0x4f,
            0x4d, 0x32, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ]
    );

    let lpt1 = announce(2, DeviceType::Parallel, "LPT1");
    assert_eq!(
        &lpt1[8..],
        &[
            0x02, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x4c, 0x50, 0x54, 0x31, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]
    );

    let decoded = ClientDeviceListAnnounce::decode(&mut ReadCursor::new(&com2[4..])).unwrap();
    assert_eq!(decoded.device_list[0].preferred_dos_name(), "COM2");
    assert!(decoded.device_list[0].device_data().is_empty());
}

#[test]
fn port_names_must_fit_preferred_dos_name() {
    for valid in ["COM1", "COM255", "LPT1", "PRN"] {
        assert!(
            RdpdrPort::new(1, RdpdrPortKind::Serial, valid.to_owned()).is_ok(),
            "{valid}"
        );
    }
    for invalid in ["", "COM1:", "COMPORT1", r"\\.\COM1", "COM 1", "C\u{d6}M1"] {
        assert!(
            RdpdrPort::new(1, RdpdrPortKind::Serial, invalid.to_owned()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn serial_ports_are_announced_before_logon_and_parallel_ports_after() {
    let mut rdpdr = port_channel(Box::new(RecordingPortBackend::default())).with_smartcard(9);
    let initialized = initialize(&mut rdpdr, true);

    assert_eq!(read_u16(&initialized.client_capability[4..]), 2);
    assert_eq!(read_u16(&initialized.client_capability[8 + 44..]), 0x0003);
    // Like mstsc: the smart cards, then the serial ports in their own announce.
    assert_eq!(initialized.client_id_confirmed.len(), 2);
    assert_eq!(
        announced_devices(&initialized.client_id_confirmed[0]),
        vec![(9, DeviceType::Smartcard)]
    );
    assert_eq!(
        announced_devices(&initialized.client_id_confirmed[1]),
        vec![(SERIAL_ID, DeviceType::Serial)]
    );
    assert_eq!(initialized.logged_on.len(), 1);
    assert_eq!(
        announced_devices(&initialized.logged_on[0]),
        vec![(PARALLEL_ID, DeviceType::Parallel)]
    );
}

/// Windows sends Client ID Confirm before its capability request when it re-announces the
/// channel for an existing session; serial ports are then announced at User Logged On.
#[test]
fn serial_ports_missed_at_client_id_confirm_are_announced_at_logon() {
    let mut rdpdr = port_channel(Box::new(RecordingPortBackend::default()));
    let client_id = announce_client_id(&mut rdpdr, 12);
    assert!(
        rdpdr
            .process(&encoded_server_client_id_confirm(client_id))
            .unwrap()
            .is_empty()
    );
    let server_capability = RdpdrPdu::CoreCapability(CoreCapability {
        capabilities: vec![CapabilityMessage::new_general(0), CapabilityMessage::new_port()],
        kind: CoreCapabilityKind::ServerCoreCapabilityRequest,
    });
    assert_eq!(
        rdpdr.process(&encode_vec(&server_capability).unwrap()).unwrap().len(),
        1
    );

    let logged_on = rdpdr.process(&encode_vec(&RdpdrPdu::UserLoggedon).unwrap()).unwrap();
    assert_eq!(logged_on.len(), 2);
    assert_eq!(announced_devices(&logged_on[0]), vec![(SERIAL_ID, DeviceType::Serial)]);
    assert_eq!(
        announced_devices(&logged_on[1]),
        vec![(PARALLEL_ID, DeviceType::Parallel)]
    );
}

#[test]
fn ports_are_not_announced_without_server_port_capability() {
    let mut rdpdr = port_channel(Box::new(RecordingPortBackend::default()));
    let initialized = initialize(&mut rdpdr, false);

    assert_eq!(
        read_u16(&initialized.client_capability[4..]),
        1,
        "only the general capability is echoed"
    );
    assert!(initialized.client_id_confirmed.is_empty());
    assert!(initialized.logged_on.is_empty());
}

/// Device control and close requests and responses from [MS-RDPESP] 4.4.
#[test]
fn port_irps_reach_the_backend_and_complete_with_the_spec_layout() {
    let mut rdpdr = acknowledged_ports(Box::new(RecordingPortBackend::default()));

    let get_baud_rate = [
        0x72, 0x44, 0x52, 0x49, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x0E, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x50, 0x00, 0x1b, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ];
    let responses = rdpdr.process(&get_baud_rate).unwrap();
    assert_eq!(responses.len(), 1);
    assert_eq!(
        responses[0].encode_unframed_pdu().unwrap(),
        [
            0x72, 0x44, 0x43, 0x49, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00,
            0x00, 0x00, 0x80, 0x25, 0x00, 0x00,
        ]
    );

    let mut close = vec![
        0x72, 0x44, 0x52, 0x49, 0x02, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    close.extend_from_slice(&[0; 32]);
    let responses = rdpdr.process(&close).unwrap();
    assert_eq!(
        responses[0].encode_unframed_pdu().unwrap(),
        [
            0x72, 0x44, 0x43, 0x49, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00,
        ]
    );

    let requests = recorded_requests(&rdpdr);
    assert_eq!(requests.len(), 2);
    let PortIoRequest::DeviceControl(control) = &requests[0] else {
        panic!("expected a device-control request");
    };
    assert_eq!(
        control.request.io_control_code,
        PortIoCtlCode::IOCTL_SERIAL_GET_BAUD_RATE
    );
    assert_eq!(control.request.output_buffer_length, 4);
    assert!(control.input_buffer.is_empty());
    assert!(matches!(requests[1], PortIoRequest::Close(_)));
}

#[test]
fn port_create_read_write_and_control_input_are_decoded() {
    let mut rdpdr = acknowledged_ports(Box::new(RecordingPortBackend::default()));

    let mut create = Vec::new();
    create.extend_from_slice(&0x0012_0089u32.to_le_bytes()); // DesiredAccess
    create.extend_from_slice(&0u64.to_le_bytes()); // AllocationSize
    create.extend_from_slice(&0u32.to_le_bytes()); // FileAttributes
    create.extend_from_slice(&7u32.to_le_bytes()); // SharedAccess
    create.extend_from_slice(&1u32.to_le_bytes()); // CreateDisposition
    create.extend_from_slice(&0u32.to_le_bytes()); // CreateOptions
    create.extend_from_slice(&0u32.to_le_bytes()); // PathLength
    rdpdr
        .process(&encoded_port_request(SERIAL_ID, MajorFunction::Create, &create))
        .unwrap();

    let mut read = Vec::new();
    read.extend_from_slice(&0x208u32.to_le_bytes()); // Length
    read.extend_from_slice(&0u64.to_le_bytes()); // Offset
    read.extend_from_slice(&[0; 20]); // Padding
    rdpdr
        .process(&encoded_port_request(SERIAL_ID, MajorFunction::Read, &read))
        .unwrap();

    let mut write = Vec::new();
    write.extend_from_slice(&3u32.to_le_bytes()); // Length
    write.extend_from_slice(&0u64.to_le_bytes()); // Offset
    write.extend_from_slice(&[0; 20]); // Padding
    write.extend_from_slice(b"AT\r");
    let responses = rdpdr
        .process(&encoded_port_request(PARALLEL_ID, MajorFunction::Write, &write))
        .unwrap();
    // Write completion: header, IoStatus, Length and one padding byte.
    assert_eq!(responses[0].encode_unframed_pdu().unwrap().len(), 4 + 12 + 4 + 1);

    let timeouts = SerialTimeouts {
        read_interval_timeout: u32::MAX,
        read_total_timeout_multiplier: 0,
        read_total_timeout_constant: 0,
        write_total_timeout_multiplier: 0,
        write_total_timeout_constant: 5000,
    };
    let input = encode_vec(&timeouts).unwrap();
    let mut control = Vec::new();
    control.extend_from_slice(&0u32.to_le_bytes()); // OutputBufferLength
    control.extend_from_slice(&u32::try_from(input.len()).unwrap().to_le_bytes()); // InputBufferLength
    control.extend_from_slice(&u32::from(PortIoCtlCode::IOCTL_SERIAL_SET_TIMEOUTS).to_le_bytes());
    control.extend_from_slice(&[0; 20]); // Padding
    control.extend_from_slice(&input);
    rdpdr
        .process(&encoded_port_request(SERIAL_ID, MajorFunction::DeviceControl, &control))
        .unwrap();

    let requests = recorded_requests(&rdpdr);
    assert_eq!(requests.len(), 4);
    let PortIoRequest::Create(create) = &requests[0] else {
        panic!("expected a create request");
    };
    assert!(create.path.is_empty());
    let PortIoRequest::Read(read) = &requests[1] else {
        panic!("expected a read request");
    };
    assert_eq!(read.length, 0x208);
    let PortIoRequest::Write(write) = &requests[2] else {
        panic!("expected a write request");
    };
    assert_eq!(write.device_io_request.device_id, PARALLEL_ID);
    assert_eq!(write.write_data, b"AT\r");
    let PortIoRequest::DeviceControl(control) = &requests[3] else {
        panic!("expected a device-control request");
    };
    assert_eq!(
        control.request.io_control_code,
        PortIoCtlCode::IOCTL_SERIAL_SET_TIMEOUTS
    );
    assert_eq!(control.input_buffer, input);
}

#[test]
fn unsupported_port_irps_are_completed_without_reaching_the_backend() {
    let mut rdpdr = acknowledged_ports(Box::new(RecordingPortBackend::default()));

    // IRP_MJ_QUERY_INFORMATION is not part of MS-RDPESP; its body is discarded.
    let responses = rdpdr
        .process(&encoded_port_request(
            SERIAL_ID,
            MajorFunction::QueryInformation,
            &[0x05, 0, 0, 0, 0, 0, 0, 0],
        ))
        .unwrap();
    assert_eq!(responses.len(), 1);
    let encoded = responses[0].encode_unframed_pdu().unwrap();
    assert_eq!(read_u32(&encoded[4..]), SERIAL_ID);
    assert_eq!(read_u32(&encoded[8..]), 0x10);
    assert_eq!(ntstatus_at(&encoded, 12), NtStatus::NOT_SUPPORTED);
    assert_eq!(&encoded[16..], &[0; 5]);

    // Trailing data after a fixed-layout port request is still malformed.
    let mut close = vec![0; 32];
    close.push(0);
    assert!(
        rdpdr
            .process(&encoded_port_request(SERIAL_ID, MajorFunction::Close, &close))
            .is_err()
    );
    assert!(recorded_requests(&rdpdr).is_empty());
}

#[test]
fn port_irps_before_the_announcement_response_reach_the_backend() {
    let mut rdpdr = port_channel(Box::new(RecordingPortBackend::default()));
    let initialized = initialize(&mut rdpdr, true);
    assert_eq!(initialized.client_id_confirmed.len(), 1);
    assert_eq!(initialized.logged_on.len(), 1);

    // Windows can send a port's first IRP just before the response accepting the port.
    for device_id in [SERIAL_ID, PARALLEL_ID] {
        let responses = rdpdr.process(&encoded_get_baud_rate_request(device_id)).unwrap();
        assert_eq!(responses.len(), 1);
        let encoded = responses[0].encode_unframed_pdu().unwrap();
        assert_eq!(read_u32(&encoded[4..]), device_id);
        assert_eq!(ntstatus_at(&encoded, 12), NtStatus::SUCCESS);
    }
    assert_eq!(recorded_requests(&rdpdr).len(), 2);
}

#[test]
fn port_irps_are_ignored_for_unannounced_and_rejected_ports() {
    let mut rdpdr = port_channel(Box::new(RecordingPortBackend::default()));
    initialize(&mut rdpdr, false);
    assert!(
        rdpdr
            .process(&encoded_get_baud_rate_request(SERIAL_ID))
            .unwrap()
            .is_empty()
    );

    let mut rdpdr = port_channel(Box::new(RecordingPortBackend::default()));
    initialize(&mut rdpdr, true);
    respond_to_announce(&mut rdpdr, SERIAL_ID, NtStatus::ACCESS_DENIED);
    // The backend sees the rejection, so it can release files the server opened before it.
    let backend = rdpdr.downcast_backend::<RecordingPortBackend>().unwrap();
    assert_eq!(backend.rejected_device_ids, [SERIAL_ID]);
    assert!(
        rdpdr
            .process(&encoded_get_baud_rate_request(SERIAL_ID))
            .unwrap()
            .is_empty()
    );
    assert!(
        rdpdr
            .process(&encoded_port_request(SERIAL_ID, MajorFunction::Close, &[0; 32]))
            .unwrap()
            .is_empty()
    );
    assert!(recorded_requests(&rdpdr).is_empty());
}

#[test]
fn control_input_longer_than_the_pdu_is_completed_as_invalid_parameter() {
    let mut rdpdr = acknowledged_ports(Box::new(RecordingPortBackend::default()));

    let mut control = Vec::new();
    control.extend_from_slice(&0u32.to_le_bytes()); // OutputBufferLength
    control.extend_from_slice(&8u32.to_le_bytes()); // InputBufferLength
    control.extend_from_slice(&u32::from(PortIoCtlCode::IOCTL_SERIAL_SET_QUEUE_SIZE).to_le_bytes());
    control.extend_from_slice(&[0; 20]); // Padding
    control.extend_from_slice(&[0; 4]); // Half of the declared input
    let responses = rdpdr
        .process(&encoded_port_request(SERIAL_ID, MajorFunction::DeviceControl, &control))
        .unwrap();
    assert_eq!(responses.len(), 1);
    let encoded = responses[0].encode_unframed_pdu().unwrap();
    assert_eq!(read_u32(&encoded[8..]), 0x10);
    assert_eq!(ntstatus_at(&encoded, 12), NtStatus::INVALID_PARAMETER);
    assert!(recorded_requests(&rdpdr).is_empty());
}

#[test]
fn serial_ioctl_buffers_are_checked_like_mstsc() {
    let timeouts = |interval, multiplier, constant| {
        encode_vec(&SerialTimeouts {
            read_interval_timeout: interval,
            read_total_timeout_multiplier: multiplier,
            read_total_timeout_constant: constant,
            write_total_timeout_multiplier: 0,
            write_total_timeout_constant: 0,
        })
        .unwrap()
    };

    for (code, input) in [
        (PortIoCtlCode::IOCTL_SERIAL_SET_BAUD_RATE, &[0x80, 0x25, 0][..]),
        (PortIoCtlCode::IOCTL_SERIAL_SET_QUEUE_SIZE, &[0; 7][..]),
        (PortIoCtlCode::IOCTL_SERIAL_SET_TIMEOUTS, &[0; 19][..]),
        (PortIoCtlCode::IOCTL_SERIAL_PURGE, &[0; 3][..]),
    ] {
        assert_eq!(
            code.check_buffers(input, 0),
            Err(NtStatus::BUFFER_TOO_SMALL),
            "{code:?}"
        );
    }
    assert_eq!(
        PortIoCtlCode::IOCTL_SERIAL_WAIT_ON_MASK.check_buffers(&[], 3),
        Err(NtStatus::BUFFER_TOO_SMALL)
    );
    assert_eq!(
        PortIoCtlCode::IOCTL_SERIAL_GET_COMMSTATUS.check_buffers(&[], 19),
        Err(NtStatus::BUFFER_TOO_SMALL)
    );
    assert_eq!(
        PortIoCtlCode::IOCTL_SERIAL_SET_TIMEOUTS.check_buffers(&timeouts(u32::MAX, u32::MAX, u32::MAX), 0),
        Err(NtStatus::INVALID_PARAMETER)
    );

    assert_eq!(
        PortIoCtlCode::IOCTL_SERIAL_SET_TIMEOUTS.check_buffers(&timeouts(u32::MAX, 0, 0), 0),
        Ok(())
    );
    assert_eq!(PortIoCtlCode::IOCTL_SERIAL_WAIT_ON_MASK.check_buffers(&[], 4), Ok(()));
    assert_eq!(
        PortIoCtlCode::IOCTL_SERIAL_GET_COMMSTATUS.check_buffers(&[], 20),
        Ok(())
    );
    assert_eq!(PortIoCtlCode::IOCTL_SERIAL_SET_DTR.check_buffers(&[], 0), Ok(()));
    assert_eq!(PortIoCtlCode::IOCTL_PAR_QUERY_INFORMATION.check_buffers(&[], 0), Ok(()));
}

#[test]
fn default_backend_completes_port_irps_as_not_supported() {
    let mut rdpdr = acknowledged_ports(Box::new(NoopRdpdrBackend));

    let mut create = vec![0; 28];
    create.extend_from_slice(&0u32.to_le_bytes()); // PathLength
    let responses = rdpdr
        .process(&encoded_port_request(SERIAL_ID, MajorFunction::Create, &create))
        .unwrap();
    let encoded = responses[0].encode_unframed_pdu().unwrap();
    // Create completion: header, IoStatus, FileId and Information.
    assert_eq!(encoded.len(), 4 + 12 + 4 + 1);
    assert_eq!(ntstatus_at(&encoded, 12), NtStatus::NOT_SUPPORTED);
    assert_eq!(read_u32(&encoded[16..]), 0);

    let mut control = Vec::new();
    control.extend_from_slice(&4u32.to_le_bytes()); // OutputBufferLength
    control.extend_from_slice(&0u32.to_le_bytes()); // InputBufferLength
    control.extend_from_slice(&u32::from(PortIoCtlCode::IOCTL_SERIAL_GET_MODEMSTATUS).to_le_bytes());
    control.extend_from_slice(&[0; 20]); // Padding
    let responses = rdpdr
        .process(&encoded_port_request(SERIAL_ID, MajorFunction::DeviceControl, &control))
        .unwrap();
    let encoded = responses[0].encode_unframed_pdu().unwrap();
    assert_eq!(ntstatus_at(&encoded, 12), NtStatus::NOT_SUPPORTED);
    assert_eq!(&encoded[16..], &[0; 4], "no output buffer");
}

fn round_trip<T>(value: T, size: usize)
where
    T: ironrdp_core::Encode + for<'de> ironrdp_core::Decode<'de> + PartialEq + core::fmt::Debug,
{
    let encoded = encode_vec(&value).unwrap();
    assert_eq!(encoded.len(), size, "{}", value.name());
    assert_eq!(T::decode(&mut ReadCursor::new(&encoded)).unwrap(), value);
}

#[test]
fn serial_buffers_round_trip_with_driver_sizes() {
    round_trip(
        SerialTimeouts {
            read_interval_timeout: 1,
            read_total_timeout_multiplier: 2,
            read_total_timeout_constant: 3,
            write_total_timeout_multiplier: 4,
            write_total_timeout_constant: 5,
        },
        20,
    );
    round_trip(
        SerialQueueSize {
            in_size: 4096,
            out_size: 4096,
        },
        8,
    );
    round_trip(
        SerialStatus {
            errors: 0x02,
            hold_reasons: 0,
            amount_in_in_queue: 12,
            amount_in_out_queue: 3,
            eof_received: 0,
            wait_for_immediate: 1,
        },
        20,
    );

    // Drivers may return SERIAL_STATUS without its trailing alignment padding.
    let mut status = encode_vec(&SerialStatus::default()).unwrap();
    status.truncate(18);
    assert_eq!(
        SerialStatus::decode(&mut ReadCursor::new(&status)).unwrap(),
        SerialStatus::default()
    );
    status.truncate(17);
    assert!(SerialStatus::decode(&mut ReadCursor::new(&status)).is_err());
}

#[test]
fn port_ioctl_codes_are_named() {
    assert_eq!(
        format!("{:?}", PortIoCtlCode::IOCTL_SERIAL_WAIT_ON_MASK),
        "IOCTL_SERIAL_WAIT_ON_MASK"
    );
    assert_eq!(format!("{:?}", PortIoCtlCode(0x001B_00FC)), "PortIoCtlCode(0x001B00FC)");
    assert_eq!(
        PortIoCtlCode::IOCTL_PAR_IS_PORT_FREE.name(),
        Some("IOCTL_PAR_IS_PORT_FREE")
    );
}
