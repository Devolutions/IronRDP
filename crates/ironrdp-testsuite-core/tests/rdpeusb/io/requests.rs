use ironrdp_rdpeusb::io::{InternalIoControlPacket, IoControlCompletionResult, IoControlPacket, IoctlInternalUsb};
use rstest::rstest;

use super::{
    CHANNEL_ID, ClientEvent, ConnectedDevice, DEVICE_TEXT_DESCRIPTION, DEVICE_TEXT_HRESULT, ServerEvent, only_message,
};

// Refs: [Query Device Text][2.2.6.5] and [Query Device Text Response][2.2.6.6].
// [2.2.6.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/d03a7696-2d56-4f20-b7a9-a5e72a045956
// [2.2.6.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/acffdcfa-c792-40a4-a8ee-c545ea5b0a38
#[test]
fn query_device_text_round_trip() {
    let mut device = ConnectedDevice::new();

    let request = device
        .server
        .query_device_text(1, 0x0409)
        .expect("query device text should succeed");
    let response = only_message(device.send_to_client(request));

    let ClientEvent::QueryDeviceText {
        channel_id,
        text_type,
        locale_id,
    } = device.next_client_event()
    else {
        panic!("expected query device text event");
    };
    assert_eq!(channel_id, CHANNEL_ID);
    assert_eq!(text_type, 1);
    assert_eq!(locale_id, 0x0409);

    assert!(device.send_to_server(response).is_empty());
    let ServerEvent::DeviceText(device_text) = device.next_server_event() else {
        panic!("expected device text event");
    };
    assert_eq!(device_text.hresult, DEVICE_TEXT_HRESULT);
    assert_eq!(device_text.description, DEVICE_TEXT_DESCRIPTION);
}

// Ref: [IO Control Completion][2.2.7.1].
// [2.2.7.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/b1722374-0658-47ba-8368-87bf9d3db4d4
#[rstest]
#[case::reset_port(
    IoControlPacket {
        ioctl_code: IoctlInternalUsb::ResetPort,
        input_buffer: Vec::new(),
        output_buffer_size: 0,
    },
    IoControlCompletionResult {
        hresult: 0,
        information: 0,
        output_buffer: Vec::new(),
    },
)]
#[case::get_port_status(
    IoControlPacket {
        ioctl_code: IoctlInternalUsb::GetPortStatus,
        input_buffer: Vec::new(),
        output_buffer_size: 4,
    },
    IoControlCompletionResult {
        hresult: 0,
        information: 4,
        output_buffer: vec![1, 0, 0, 0],
    },
)]
#[case::get_hub_name(
    IoControlPacket {
        ioctl_code: IoctlInternalUsb::GetHubName,
        input_buffer: Vec::new(),
        output_buffer_size: 8,
    },
    IoControlCompletionResult {
        hresult: 0,
        information: 4,
        output_buffer: vec![b'H', 0, b'1', 0],
    },
)]
fn io_control_pending_completion_round_trip(
    #[case] packet: IoControlPacket,
    #[case] completion: IoControlCompletionResult,
) {
    let mut device = ConnectedDevice::new();
    let expected_ioctl_code = packet.ioctl_code;
    let expected_input_buffer = packet.input_buffer.clone();
    let expected_output_buffer_size = packet.output_buffer_size;
    let expected_hresult = completion.hresult;
    let expected_information = completion.information;
    let expected_completion_output = completion.output_buffer.clone();

    let request = device.server.io_control(packet).expect("IO control should succeed");
    assert!(request.expects_completion);
    let request_id = request.request_id;
    assert!(device.send_to_client(request.message).is_empty());

    let ClientEvent::IoControl {
        channel_id,
        request_id: backend_request_id,
        request,
    } = device.next_client_event()
    else {
        panic!("expected IO control event");
    };
    assert_eq!(channel_id, CHANNEL_ID);
    assert_eq!(backend_request_id, request_id);
    assert_eq!(request.ioctl_code, expected_ioctl_code);
    assert_eq!(request.input_buffer, expected_input_buffer);
    assert_eq!(request.output_buffer_size, expected_output_buffer_size);

    let response = device
        .client
        .io_ctl_completion(request_id, completion)
        .expect("IO control completion should succeed");
    assert!(device.send_to_server(response).is_empty());

    let ServerEvent::IoControlCompleted {
        channel_id,
        request_id: backend_request_id,
        completion,
    } = device.next_server_event()
    else {
        panic!("expected IO control completion event");
    };
    assert_eq!(channel_id, CHANNEL_ID);
    assert_eq!(backend_request_id, request_id);
    assert_eq!(completion.hresult, expected_hresult);
    assert_eq!(completion.information, expected_information);
    assert_eq!(completion.output_buffer, expected_completion_output);
}

// Ref: [Internal IO Control Message][2.2.6.4].
// [2.2.6.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/c3f3e320-336d-4d1b-84c9-51e0ed330ffe
#[test]
fn internal_io_control_pending_completion_round_trip() {
    let mut device = ConnectedDevice::new();
    let request = InternalIoControlPacket::QueryBusTime;

    let request = device
        .server
        .internal_io_control(request)
        .expect("internal IO control should succeed");
    assert!(request.expects_completion);
    let request_id = request.request_id;
    assert!(device.send_to_client(request.message).is_empty());

    let ClientEvent::InternalIoControl {
        channel_id,
        request_id: backend_request_id,
        request,
    } = device.next_client_event()
    else {
        panic!("expected internal IO control event");
    };
    assert_eq!(channel_id, CHANNEL_ID);
    assert_eq!(backend_request_id, request_id);
    assert!(matches!(request, InternalIoControlPacket::QueryBusTime));

    let completion = IoControlCompletionResult {
        hresult: 0,
        information: 4,
        output_buffer: vec![42, 0, 0, 0],
    };
    let response = device
        .client
        .internal_io_ctl_completion(request_id, completion)
        .expect("internal IO control completion should succeed");
    assert!(device.send_to_server(response).is_empty());

    let ServerEvent::InternalIoControlCompleted {
        channel_id,
        request_id: backend_request_id,
        completion,
    } = device.next_server_event()
    else {
        panic!("expected internal IO control completion event");
    };
    assert_eq!(channel_id, CHANNEL_ID);
    assert_eq!(backend_request_id, request_id);
    assert_eq!(completion.hresult, 0);
    assert_eq!(completion.information, 4);
    assert_eq!(completion.output_buffer, [42, 0, 0, 0]);
}

// Ref: [Processing a Cancel Request Message][3.3.5.3.1].
// [3.3.5.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/d5315234-d9ba-42dc-bc1b-b421c57a21ae
#[test]
fn cancel_pending_request() {
    let mut device = ConnectedDevice::new();
    let request = device
        .server
        .io_control(IoControlPacket {
            ioctl_code: IoctlInternalUsb::GetPortStatus,
            input_buffer: Vec::new(),
            output_buffer_size: 4,
        })
        .expect("IO control should succeed");
    let request_id = request.request_id;
    assert!(device.send_to_client(request.message).is_empty());
    assert!(matches!(device.next_client_event(), ClientEvent::IoControl { .. }));

    let cancel = device
        .server
        .cancel_request(request_id)
        .expect("cancel request should succeed");
    assert!(device.send_to_client(cancel).is_empty());

    let ClientEvent::Cancel {
        channel_id,
        request_id: backend_request_id,
    } = device.next_client_event()
    else {
        panic!("expected cancel event");
    };
    assert_eq!(channel_id, CHANNEL_ID);
    assert_eq!(backend_request_id, request_id);
}
