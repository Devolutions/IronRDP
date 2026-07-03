use ironrdp_rdpeusb::io::{
    TransferInCompletionResult, TransferInPacket, TransferOutCompletionResult, TransferOutPacket, TsUrbInKind,
    TsUrbInPacket, TsUrbOutKind, TsUrbOutPacket, UrbFunction,
};
use ironrdp_rdpeusb::pdu::completion::ts_urb_result::{TsUrbResult, TsUrbResultHeader, TsUrbResultPayload};
use ironrdp_rdpeusb::pdu::usb_dev::ts_urb::utils::SetupPacket;
use ironrdp_rdpeusb::pdu::usb_dev::ts_urb::{
    TsUrbBulkOrInterruptTransfer, TsUrbControlGetConfigRequest, TsUrbControlGetInterfaceRequest,
    TsUrbControlGetStatusRequest, TsUrbControlTransfer, TsUrbControlVendorClassRequest, TsUrbIsochTransfer,
};
use ironrdp_rdpeusb::pdu::utils::UsbdIsoPacketDesc;
use rstest::rstest;

use super::{CHANNEL_ID, ClientEvent, ConnectedDevice, ServerEvent};

fn successful_urb_result() -> TsUrbResult {
    TsUrbResult {
        header: TsUrbResultHeader { usbd_status: 0 },
        payload: TsUrbResultPayload::Raw(Vec::new()),
    }
}

// Refs: [URB Completion][2.2.7.2] and [URB Completion No Data][2.2.7.3].
// [2.2.7.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/5bfa9c84-a74b-4942-9d09-e770b21081eb
// [2.2.7.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/994fac8f-d258-47a6-aa35-48783abe49ec
#[rstest]
#[case::get_configuration(
    TransferInPacket {
        ts_urb: TsUrbInPacket {
            kind: TsUrbInKind::CtlGetConfig(TsUrbControlGetConfigRequest),
            func: UrbFunction::URB_FUNCTION_GET_CONFIGURATION,
        },
        output_buffer_size: 1,
    },
    vec![0x01],
)]
#[case::get_configuration_without_data(
    TransferInPacket {
        ts_urb: TsUrbInPacket {
            kind: TsUrbInKind::CtlGetConfig(TsUrbControlGetConfigRequest),
            func: UrbFunction::URB_FUNCTION_GET_CONFIGURATION,
        },
        output_buffer_size: 1,
    },
    Vec::new(),
)]
#[case::get_interface(
    TransferInPacket {
        ts_urb: TsUrbInPacket {
            kind: TsUrbInKind::CtlGetIface(TsUrbControlGetInterfaceRequest { interface: 2 }),
            func: UrbFunction::URB_FUNCTION_GET_INTERFACE,
        },
        output_buffer_size: 1,
    },
    vec![0x02],
)]
#[case::get_status(
    TransferInPacket {
        ts_urb: TsUrbInPacket {
            kind: TsUrbInKind::CtlGetStatus(TsUrbControlGetStatusRequest { index: 0x81 }),
            func: UrbFunction::URB_FUNCTION_GET_STATUS_FROM_ENDPOINT,
        },
        output_buffer_size: 2,
    },
    vec![0x01, 0x00],
)]
fn transfer_in_completion_round_trip(#[case] packet: TransferInPacket, #[case] output_buffer: Vec<u8>) {
    let mut device = ConnectedDevice::new();
    let expected_kind = packet.ts_urb.kind.clone();
    let expected_func = packet.ts_urb.func;
    let expected_output_buffer_size = packet.output_buffer_size;
    let request = device.server.transfer_in(packet).expect("transfer in should succeed");
    assert!(request.expects_completion);
    let request_id = request.request_id;
    assert!(device.send_to_client(request.message).is_empty());

    let ClientEvent::TransferIn {
        channel_id,
        request_id: backend_request_id,
        request,
    } = device.next_client_event()
    else {
        panic!("expected transfer in event");
    };
    assert_eq!(channel_id, CHANNEL_ID);
    assert_eq!(backend_request_id, request_id);
    assert_eq!(request.ts_urb.kind, expected_kind);
    assert_eq!(request.ts_urb.func, expected_func);
    assert_eq!(request.output_buffer_size, expected_output_buffer_size);

    let response = device
        .client
        .transfer_in_completion(
            request_id,
            TransferInCompletionResult {
                ts_urb_result: successful_urb_result(),
                hresult: 0,
                output_buffer: output_buffer.clone(),
            },
        )
        .expect("transfer in completion should succeed");
    assert!(device.send_to_server(response).is_empty());

    let ServerEvent::TransferInCompleted {
        channel_id,
        request_id: backend_request_id,
        completion,
    } = device.next_server_event()
    else {
        panic!("expected transfer in completion event");
    };
    assert_eq!(channel_id, CHANNEL_ID);
    assert_eq!(backend_request_id, request_id);
    assert_eq!(completion.ts_urb_result, successful_urb_result());
    assert_eq!(completion.hresult, 0);
    assert_eq!(completion.output_buffer, output_buffer);
}

// Ref: [Transfer Out Request][2.2.6.8].
// [2.2.6.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/6d6c85b2-47bb-4674-975a-dc7d8ed684cd
#[rstest]
#[case::bulk_or_interrupt(
    TransferOutPacket {
        ts_urb: TsUrbOutPacket {
            kind: TsUrbOutKind::BulkInterruptTransfer(TsUrbBulkOrInterruptTransfer {
                pipe_handle: 7,
                transfer_flags: 0,
            }),
            no_ack: false,
            func: UrbFunction::URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER,
        },
        output_buffer: vec![1, 2, 3],
    },
    TransferOutCompletionResult {
        ts_urb_result: successful_urb_result(),
        hresult: 0,
        output_buffer_size: 3,
    },
)]
#[case::control(
    TransferOutPacket {
        ts_urb: TsUrbOutPacket {
            kind: TsUrbOutKind::CtlTransfer(TsUrbControlTransfer {
                pipe: 0,
                transfer_flags: 0,
                setup_packet: SetupPacket {
                    request_type: 0,
                    request: 9,
                    value: 1,
                    index: 0,
                    length: 0,
                },
            }),
            no_ack: false,
            func: UrbFunction::URB_FUNCTION_CONTROL_TRANSFER,
        },
        output_buffer: Vec::new(),
    },
    TransferOutCompletionResult {
        ts_urb_result: successful_urb_result(),
        hresult: 0,
        output_buffer_size: 0,
    },
)]
#[case::vendor(
    TransferOutPacket {
        ts_urb: TsUrbOutPacket {
            kind: TsUrbOutKind::VendorClassReq(TsUrbControlVendorClassRequest {
                transfer_flags: 0,
                request: 1,
                value: 2,
                index: 3,
            }),
            no_ack: false,
            func: UrbFunction::URB_FUNCTION_VENDOR_DEVICE,
        },
        output_buffer: vec![4, 5],
    },
    TransferOutCompletionResult {
        ts_urb_result: successful_urb_result(),
        hresult: 0,
        output_buffer_size: 2,
    },
)]
fn transfer_out_completion_round_trip(
    #[case] packet: TransferOutPacket,
    #[case] completion: TransferOutCompletionResult,
) {
    let mut device = ConnectedDevice::new();
    let expected_kind = packet.ts_urb.kind.clone();
    let expected_func = packet.ts_urb.func;
    let expected_output_buffer = packet.output_buffer.clone();
    let expected_ts_urb_result = completion.ts_urb_result.clone();
    let expected_hresult = completion.hresult;
    let expected_output_buffer_size = completion.output_buffer_size;
    let request = device.server.transfer_out(packet).expect("transfer out should succeed");
    assert!(request.expects_completion);
    let request_id = request.request_id;
    assert!(device.send_to_client(request.message).is_empty());

    let ClientEvent::TransferOut {
        channel_id,
        request_id: backend_request_id,
        request,
    } = device.next_client_event()
    else {
        panic!("expected transfer out event");
    };
    assert_eq!(channel_id, CHANNEL_ID);
    assert_eq!(backend_request_id, request_id);
    assert_eq!(request.ts_urb.kind, expected_kind);
    assert!(!request.ts_urb.no_ack);
    assert_eq!(request.ts_urb.func, expected_func);
    assert_eq!(request.output_buffer, expected_output_buffer);

    let response = device
        .client
        .transfer_out_completion(request_id, completion)
        .expect("transfer out completion should succeed");
    assert!(device.send_to_server(response).is_empty());

    let ServerEvent::TransferOutCompleted {
        channel_id,
        request_id: backend_request_id,
        completion,
    } = device.next_server_event()
    else {
        panic!("expected transfer out completion event");
    };
    assert_eq!(channel_id, CHANNEL_ID);
    assert_eq!(backend_request_id, request_id);
    assert_eq!(completion.ts_urb_result, expected_ts_urb_result);
    assert_eq!(completion.hresult, expected_hresult);
    assert_eq!(completion.output_buffer_size, expected_output_buffer_size);
}

#[test]
fn transfer_out_no_ack() {
    let mut device = ConnectedDevice::new();
    let request = device
        .server
        .transfer_out(TransferOutPacket {
            ts_urb: TsUrbOutPacket {
                kind: TsUrbOutKind::IsochTransfer(TsUrbIsochTransfer {
                    pipe_handle: 7,
                    transfer_flags: 0,
                    start_frame: 100,
                    error_count: 0,
                    iso_packet: vec![UsbdIsoPacketDesc {
                        offset: 0,
                        length: 3,
                        status: 0,
                    }],
                }),
                no_ack: true,
                func: UrbFunction::URB_FUNCTION_ISOCH_TRANSFER,
            },
            output_buffer: vec![1, 2, 3],
        })
        .expect("no-ack transfer out should succeed");
    assert!(!request.expects_completion);
    let request_id = request.request_id;
    assert!(device.send_to_client(request.message).is_empty());

    let ClientEvent::TransferOutNoAck {
        channel_id,
        request_id: backend_request_id,
        request,
    } = device.next_client_event()
    else {
        panic!("expected no-ack transfer out event");
    };
    assert_eq!(channel_id, CHANNEL_ID);
    assert_eq!(backend_request_id, request_id);
    let TsUrbOutKind::IsochTransfer(urb) = request.ts_urb.kind else {
        panic!("expected isochronous transfer");
    };
    assert_eq!(urb.pipe_handle, 7);
    assert_eq!(urb.transfer_flags, 0);
    assert_eq!(urb.start_frame, 100);
    assert_eq!(urb.error_count, 0);
    assert_eq!(urb.iso_packet.len(), 1);
    assert_eq!(urb.iso_packet[0].offset, 0);
    assert_eq!(urb.iso_packet[0].length, 3);
    assert_eq!(urb.iso_packet[0].status, 0);
    assert!(request.ts_urb.no_ack);
    assert_eq!(request.ts_urb.func, UrbFunction::URB_FUNCTION_ISOCH_TRANSFER);
    assert_eq!(request.output_buffer, [1, 2, 3]);
}
