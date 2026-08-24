use ironrdp_rdpeusb::{
    io::{
        CompletionData, IoControlCompletionResult, TransferInCompletionResult, TransferOutCompletionResult,
        TsUrbInKind, TsUrbOutKind, UrbFunction,
    },
    pdu::{
        completion::ts_urb_result::{
            TsUrbGetCurrFrameNumResult, TsUrbIsochTransferResult, TsUrbResult, TsUrbResultHeader, TsUrbResultPayload,
            TsUrbSelectConfigResult, TsUrbSelectInterfaceResult, TsUsbdInterfaceInfoResult,
        },
        usb_dev::IoctlInternalUsb,
        utils::UsbdIsoPacketDesc,
    },
    usb::{
        CompletionError, ConversionError, TransferRequest, bulk_or_interrupt, control_transfer, current_frame_number,
        current_frame_number_completion, get_configuration, get_configuration_completion, get_descriptor,
        get_descriptor_completion, get_interface, get_interface_completion, isochronous, isochronous_completion,
        pipe_request_completion, reset_device, reset_device_completion, reset_pipe_and_clear_stall,
        select_configuration, select_configuration_completion, select_interface, select_interface_completion,
        transfer_completion, unconfigure,
    },
};
use ironrdp_usb::{
    Direction, InterfaceSelection, UsbSpeed,
    control::{GetDescriptorRequest, Recipient, RequestKind, RequestType, SetupPacket, standard_request},
    descriptor::{ConfigurationDescriptorSet, descriptor_type},
    endpoint::EndpointAddress,
    transfer::{
        DataTransferRequest, IsochronousPacketCompletion, IsochronousTransferOutput, IsochronousTransferRequest,
        TransferCompletion, UsbError,
    },
};

const USBD_TRANSFER_DIRECTION_IN: u32 = 0x0000_0001;
const USBD_SHORT_TRANSFER_OK: u32 = 0x0000_0002;
const USBD_DEFAULT_PIPE_TRANSFER: u32 = 0x0000_0008;
const USBD_START_ISO_TRANSFER_ASAP: u32 = 0x0000_0004;
const DEFAULT_CONTROL_PIPE_HANDLE: u32 = 0;

const USBD_STATUS_CANCELED: u32 = 0xc001_0000;
const USBD_STATUS_STALL_PID: u32 = 0xc000_0004;
const USBD_STATUS_BABBLE_DETECTED: u32 = 0xc000_0012;
const USBD_STATUS_TIMEOUT: u32 = 0xc000_6000;
const USBD_STATUS_DEVICE_GONE: u32 = 0xc000_7000;

const CONFIGURATION: [u8; 57] = [
    9, 2, 57, 0, 2, 1, 0, 0x80, 50, // configuration
    9, 4, 0, 0, 2, 0xff, 0, 0, 0, // interface 0, alternate 0
    7, 5, 0x81, 3, 0x40, 0x10, 1, // high-bandwidth interrupt IN
    7, 5, 0x02, 2, 0x00, 0x02, 0, // bulk OUT
    9, 4, 1, 0, 0, 0xff, 0, 0, 0, // interface 1, alternate 0
    9, 4, 1, 1, 1, 0xff, 0, 0, 0, // interface 1, alternate 1
    7, 5, 0x83, 3, 32, 0, 4, // interrupt IN
];

fn setup(
    direction: Direction,
    kind: RequestKind,
    recipient: Recipient,
    request: u8,
    value: u16,
    index: u16,
    length: u16,
) -> SetupPacket {
    SetupPacket {
        request_type: RequestType::new(direction, kind, recipient),
        request,
        value,
        index,
        length,
    }
}

fn selection(interface: u8, alternate_setting: u8) -> InterfaceSelection {
    InterfaceSelection {
        interface,
        alternate_setting,
    }
}

#[test]
fn feature_out_uses_rdpeusb_transfer_in() {
    let setup = setup(
        Direction::Out,
        RequestKind::STANDARD,
        Recipient::ENDPOINT,
        standard_request::SET_FEATURE,
        0,
        0x81,
        0,
    );

    let TransferRequest::In(request) = control_transfer(setup, Vec::new()).unwrap() else {
        panic!("feature request used TRANSFER_OUT_REQUEST");
    };
    assert_eq!(request.output_buffer_size, 0);
    assert_eq!(request.ts_urb.func, UrbFunction::URB_FUNCTION_SET_FEATURE_TO_ENDPOINT);
    let TsUrbInKind::CtlFeatReq(urb) = request.ts_urb.kind else {
        panic!("feature request used the wrong TS_URB variant");
    };
    assert_eq!(urb.feat_selector, 0);
    assert_eq!(urb.index, 0x81);
}

#[test]
fn descriptor_request_preserves_typed_fields() {
    let setup = setup(
        Direction::In,
        RequestKind::STANDARD,
        Recipient::INTERFACE,
        standard_request::GET_DESCRIPTOR,
        0x2203,
        0x0409,
        64,
    );

    let TransferRequest::In(request) = control_transfer(setup, Vec::new()).unwrap() else {
        panic!("GET_DESCRIPTOR used TRANSFER_OUT_REQUEST");
    };
    assert_eq!(request.output_buffer_size, 64);
    assert_eq!(
        request.ts_urb.func,
        UrbFunction::URB_FUNCTION_GET_DESCRIPTOR_FROM_INTERFACE
    );
    let TsUrbInKind::CtlDescReq(urb) = request.ts_urb.kind else {
        panic!("GET_DESCRIPTOR used the wrong TS_URB variant");
    };
    assert_eq!(urb.index, 3);
    assert_eq!(urb.desc_type, 0x22);
    assert_eq!(urb.lang_id, 0x0409);
}

#[test]
fn typed_get_descriptor_rejects_an_unrepresentable_recipient() {
    let error = get_descriptor(GetDescriptorRequest {
        recipient: Recipient::VENDOR_SPECIFIC,
        descriptor_type: descriptor_type::DEVICE,
        descriptor_index: 0,
        index: 0,
        requested_length: 18,
    })
    .unwrap_err();

    assert_eq!(
        error,
        ConversionError::UnsupportedDescriptorRecipient {
            recipient: Recipient::VENDOR_SPECIFIC
        }
    );
}

#[test]
fn get_descriptor_completion_returns_usb_data() {
    let output = vec![18, descriptor_type::DEVICE, 0, 2];

    assert_eq!(
        get_descriptor_completion(transfer_in_completion(0, 0, output.clone())).unwrap(),
        Ok(output.clone())
    );
    assert_eq!(
        get_descriptor_completion(transfer_in_completion(1, 0, output.clone())).unwrap(),
        Ok(output)
    );
    assert_eq!(
        get_descriptor_completion(transfer_in_completion(0, 0, Vec::new())).unwrap(),
        Ok(Vec::new())
    );
}

#[test]
fn get_descriptor_completion_maps_usb_failures() {
    for (usbd_status, expected) in [
        (USBD_STATUS_CANCELED, UsbError::Cancelled),
        (USBD_STATUS_STALL_PID, UsbError::Stall),
        (USBD_STATUS_BABBLE_DETECTED, UsbError::Overflow),
        (USBD_STATUS_TIMEOUT, UsbError::Timeout),
        (USBD_STATUS_DEVICE_GONE, UsbError::NoDevice),
        (0xdead_beef, UsbError::Error),
    ] {
        assert_eq!(
            get_descriptor_completion(transfer_in_completion(0, usbd_status, vec![1, 2, 3])).unwrap(),
            Err(expected)
        );
    }

    assert_eq!(
        get_descriptor_completion(transfer_in_completion(0x8000_4005, 0, vec![1, 2, 3])).unwrap(),
        Err(UsbError::Error)
    );
}

#[test]
fn get_descriptor_completion_rejects_a_mismatched_result() {
    let result = TsUrbResult {
        header: TsUrbResultHeader { usbd_status: 0 },
        payload: None,
    };
    assert_eq!(
        get_descriptor_completion(CompletionData::TransferOut(TransferOutCompletionResult {
            ts_urb_result: result,
            hresult: 0,
            output_buffer_size: 0,
        }))
        .unwrap_err(),
        CompletionError::ExpectedTransferIn
    );

    let result = TsUrbResult {
        header: TsUrbResultHeader { usbd_status: 0 },
        payload: Some(TsUrbResultPayload::FrameNum(TsUrbGetCurrFrameNumResult {
            frame_number: 42,
        })),
    };
    assert_eq!(
        get_descriptor_completion(CompletionData::TransferIn(TransferInCompletionResult {
            ts_urb_result: result,
            hresult: 0,
            output_buffer: Vec::new(),
        }))
        .unwrap_err(),
        CompletionError::UnexpectedUrbResultPayload
    );
}

fn transfer_in_completion(hresult: u32, usbd_status: u32, output_buffer: Vec<u8>) -> CompletionData {
    CompletionData::TransferIn(TransferInCompletionResult {
        ts_urb_result: TsUrbResult {
            header: TsUrbResultHeader { usbd_status },
            payload: None,
        },
        hresult,
        output_buffer,
    })
}

#[test]
fn class_in_accepts_short_control_responses() {
    let setup = SetupPacket {
        request_type: RequestType::new(Direction::In, RequestKind::CLASS, Recipient::INTERFACE),
        request: 0x81,
        value: 0x0200,
        index: 3,
        length: 8,
    };

    let TransferRequest::In(request) = control_transfer(setup, Vec::new()).unwrap() else {
        panic!("class IN request used TRANSFER_OUT_REQUEST");
    };
    let TsUrbInKind::VendorClassReq(urb) = request.ts_urb.kind else {
        panic!("class request used the wrong TS_URB variant");
    };
    assert_eq!(urb.transfer_flags, USBD_TRANSFER_DIRECTION_IN | USBD_SHORT_TRANSFER_OK);
}

#[test]
fn unknown_recipient_falls_back_without_losing_setup_fields() {
    let setup = SetupPacket {
        request_type: RequestType::new(Direction::In, RequestKind::VENDOR, Recipient::VENDOR_SPECIFIC),
        request: 0xa5,
        value: 0x1234,
        index: 0x5678,
        length: 17,
    };

    let TransferRequest::In(request) = control_transfer(setup, Vec::new()).unwrap() else {
        panic!("vendor IN request used TRANSFER_OUT_REQUEST");
    };
    assert_eq!(request.output_buffer_size, 17);
    assert_eq!(request.ts_urb.func, UrbFunction::URB_FUNCTION_CONTROL_TRANSFER);
    let TsUrbInKind::CtlTransfer(urb) = request.ts_urb.kind else {
        panic!("unknown recipient did not use generic control transfer");
    };
    assert_eq!(urb.pipe, DEFAULT_CONTROL_PIPE_HANDLE);
    assert_eq!(
        urb.transfer_flags,
        USBD_TRANSFER_DIRECTION_IN | USBD_SHORT_TRANSFER_OK | USBD_DEFAULT_PIPE_TRANSFER
    );
    assert_eq!(urb.setup_packet.request_type, setup.request_type.raw());
    assert_eq!(urb.setup_packet.request, setup.request);
    assert_eq!(urb.setup_packet.value, setup.value);
    assert_eq!(urb.setup_packet.index, setup.index);
    assert_eq!(urb.setup_packet.length, setup.length);
}

#[test]
fn noncanonical_typed_requests_fall_back_without_losing_fields() {
    let requests = [
        setup(
            Direction::In,
            RequestKind::STANDARD,
            Recipient::DEVICE,
            standard_request::GET_STATUS,
            1,
            0,
            2,
        ),
        setup(
            Direction::In,
            RequestKind::STANDARD,
            Recipient::DEVICE,
            standard_request::GET_CONFIGURATION,
            0,
            1,
            1,
        ),
        setup(
            Direction::In,
            RequestKind::STANDARD,
            Recipient::INTERFACE,
            standard_request::GET_INTERFACE,
            1,
            3,
            1,
        ),
        setup(
            Direction::In,
            RequestKind::STANDARD,
            Recipient::ENDPOINT,
            standard_request::CLEAR_FEATURE,
            0,
            0x81,
            0,
        ),
    ];

    for setup in requests {
        let TransferRequest::In(request) = control_transfer(setup, Vec::new()).unwrap() else {
            panic!("noncanonical USB IN request used TRANSFER_OUT_REQUEST");
        };
        let TsUrbInKind::CtlTransfer(urb) = request.ts_urb.kind else {
            panic!("noncanonical request lost fields in a typed TS_URB");
        };
        assert_eq!(urb.setup_packet.request_type, setup.request_type.raw());
        assert_eq!(urb.setup_packet.request, setup.request);
        assert_eq!(urb.setup_packet.value, setup.value);
        assert_eq!(urb.setup_packet.index, setup.index);
        assert_eq!(urb.setup_packet.length, setup.length);
    }
}

#[test]
fn stateful_standard_requests_do_not_use_generic_control() {
    let requests = [
        setup(
            Direction::Out,
            RequestKind::STANDARD,
            Recipient::DEVICE,
            standard_request::SET_ADDRESS,
            5,
            0,
            0,
        ),
        setup(
            Direction::Out,
            RequestKind::STANDARD,
            Recipient::DEVICE,
            standard_request::SET_CONFIGURATION,
            1,
            0,
            0,
        ),
        setup(
            Direction::Out,
            RequestKind::STANDARD,
            Recipient::INTERFACE,
            standard_request::SET_INTERFACE,
            1,
            0,
            0,
        ),
    ];

    for setup in requests {
        assert_eq!(
            control_transfer(setup, Vec::new()).unwrap_err(),
            ConversionError::StatefulStandardRequest { request: setup.request }
        );
    }
}

#[test]
fn control_data_shape_is_validated_before_translation() {
    let input = setup(
        Direction::In,
        RequestKind::STANDARD,
        Recipient::DEVICE,
        standard_request::GET_STATUS,
        0,
        0,
        2,
    );
    assert_eq!(
        control_transfer(input, vec![0]).unwrap_err(),
        ConversionError::InTransferHasData { actual: 1 }
    );

    let output = SetupPacket {
        request_type: RequestType::new(Direction::Out, RequestKind::VENDOR, Recipient::DEVICE),
        request: 1,
        value: 0,
        index: 0,
        length: 2,
    };
    assert_eq!(
        control_transfer(output, vec![0]).unwrap_err(),
        ConversionError::OutTransferLengthMismatch { expected: 2, actual: 1 }
    );
}

#[test]
fn general_bulk_or_interrupt_validates_directional_data() {
    let endpoint = EndpointAddress::from_raw(0x81).unwrap();
    let TransferRequest::In(input) = bulk_or_interrupt(
        7,
        DataTransferRequest {
            endpoint,
            length: 512,
            data: Vec::new(),
        },
    )
    .unwrap() else {
        panic!("bulk IN used TRANSFER_OUT_REQUEST");
    };
    assert_eq!(input.output_buffer_size, 512);
    let TsUrbInKind::BulkInterruptTransfer(urb) = input.ts_urb.kind else {
        panic!("bulk IN used the wrong TS_URB variant");
    };
    assert_eq!(urb.pipe_handle, 7);
    assert_eq!(urb.transfer_flags, USBD_TRANSFER_DIRECTION_IN | USBD_SHORT_TRANSFER_OK);

    assert_eq!(
        bulk_or_interrupt(
            7,
            DataTransferRequest {
                endpoint,
                length: 1,
                data: vec![1],
            },
        )
        .unwrap_err(),
        ConversionError::InTransferHasData { actual: 1 }
    );

    let endpoint = EndpointAddress::from_raw(0x02).unwrap();
    let TransferRequest::Out(output) = bulk_or_interrupt(
        9,
        DataTransferRequest {
            endpoint,
            length: 3,
            data: vec![1, 2, 3],
        },
    )
    .unwrap() else {
        panic!("bulk OUT used TRANSFER_IN_REQUEST");
    };
    assert_eq!(output.output_buffer, [1, 2, 3]);
    assert!(!output.ts_urb.no_ack);
    let TsUrbOutKind::BulkInterruptTransfer(urb) = output.ts_urb.kind else {
        panic!("bulk OUT used the wrong TS_URB variant");
    };
    assert_eq!(urb.pipe_handle, 9);
    assert_eq!(urb.transfer_flags, 0);

    assert_eq!(
        bulk_or_interrupt(
            9,
            DataTransferRequest {
                endpoint,
                length: 3,
                data: vec![1, 2],
            },
        )
        .unwrap_err(),
        ConversionError::OutTransferLengthMismatch { expected: 3, actual: 2 }
    );
}

#[test]
fn general_transfer_completion_preserves_directional_results() {
    assert_eq!(
        transfer_completion(transfer_in_completion(0, 0, vec![1, 2, 3])).unwrap(),
        TransferCompletion {
            status: Ok(()),
            actual_length: 3,
            data: vec![1, 2, 3]
        }
    );

    assert_eq!(
        transfer_completion(CompletionData::TransferOut(TransferOutCompletionResult {
            ts_urb_result: TsUrbResult {
                header: TsUrbResultHeader {
                    usbd_status: USBD_STATUS_STALL_PID,
                },
                payload: None,
            },
            hresult: 0,
            output_buffer_size: 17,
        }))
        .unwrap(),
        TransferCompletion {
            status: Err(UsbError::Stall),
            actual_length: 17,
            data: Vec::new()
        }
    );
}

#[test]
fn configuration_and_interface_queries_use_typed_urbs() {
    let request = get_configuration();
    assert_eq!(request.output_buffer_size, 1);
    assert_eq!(request.ts_urb.func, UrbFunction::URB_FUNCTION_GET_CONFIGURATION);
    assert!(matches!(request.ts_urb.kind, TsUrbInKind::CtlGetConfig(_)));
    assert_eq!(
        get_configuration_completion(transfer_in_completion(0, 0, vec![3])).unwrap(),
        Ok(3)
    );

    let request = get_interface(5);
    assert_eq!(request.output_buffer_size, 1);
    assert_eq!(request.ts_urb.func, UrbFunction::URB_FUNCTION_GET_INTERFACE);
    let TsUrbInKind::CtlGetIface(urb) = request.ts_urb.kind else {
        panic!("GET_INTERFACE used the wrong TS_URB variant");
    };
    assert_eq!(urb.interface, 5);
    assert_eq!(
        get_interface_completion(transfer_in_completion(0, 0, vec![2])).unwrap(),
        Ok(2)
    );
    assert_eq!(
        get_interface_completion(transfer_in_completion(0, USBD_STATUS_STALL_PID, Vec::new())).unwrap(),
        Err(UsbError::Stall)
    );
    assert_eq!(
        get_configuration_completion(transfer_in_completion(0, 0, Vec::new())).unwrap_err(),
        CompletionError::ExpectedOneByteOutput { actual: 0 }
    );
}

#[test]
fn isochronous_builder_uses_prefix_offsets_and_no_ack_only_for_out() {
    let output_endpoint = EndpointAddress::from_raw(0x02).unwrap();
    let TransferRequest::Out(output) = isochronous(
        19,
        IsochronousTransferRequest {
            endpoint: output_endpoint,
            start_frame: None,
            data: vec![1, 2, 3, 4, 5, 6],
            packets: vec![2, 1, 3],
        },
        true,
    )
    .unwrap() else {
        panic!("isochronous OUT used TRANSFER_IN_REQUEST");
    };
    assert!(output.ts_urb.no_ack);
    let TsUrbOutKind::IsochTransfer(urb) = output.ts_urb.kind else {
        panic!("isochronous OUT used the wrong TS_URB variant");
    };
    assert_eq!(urb.pipe_handle, 19);
    assert_eq!(urb.transfer_flags, USBD_START_ISO_TRANSFER_ASAP);
    assert_eq!(urb.start_frame, 0);
    assert_eq!(
        urb.iso_packet
            .iter()
            .map(|packet| (packet.offset, packet.length, packet.status))
            .collect::<Vec<_>>(),
        [(0, 0, 0), (2, 0, 0), (3, 0, 0)]
    );

    let input_endpoint = EndpointAddress::from_raw(0x82).unwrap();
    assert_eq!(
        isochronous(
            20,
            IsochronousTransferRequest {
                endpoint: input_endpoint,
                start_frame: Some(42),
                data: Vec::new(),
                packets: vec![4],
            },
            true,
        )
        .unwrap_err(),
        ConversionError::NoAckIsochronousIn
    );
    let TransferRequest::In(input) = isochronous(
        20,
        IsochronousTransferRequest {
            endpoint: input_endpoint,
            start_frame: Some(42),
            data: Vec::new(),
            packets: vec![4, 8],
        },
        false,
    )
    .unwrap() else {
        panic!("isochronous IN used TRANSFER_OUT_REQUEST");
    };
    assert_eq!(input.output_buffer_size, 12);
    let TsUrbInKind::IsochTransfer(urb) = input.ts_urb.kind else {
        panic!("isochronous IN used the wrong TS_URB variant");
    };
    assert_eq!(urb.transfer_flags, USBD_TRANSFER_DIRECTION_IN);
    assert_eq!(urb.start_frame, 42);
}

#[test]
fn isochronous_completion_validates_and_unpacks_packet_results() {
    let stall = i32::from_ne_bytes(USBD_STATUS_STALL_PID.to_ne_bytes());
    let completion = CompletionData::TransferIn(TransferInCompletionResult {
        ts_urb_result: TsUrbResult {
            header: TsUrbResultHeader { usbd_status: 0 },
            payload: Some(TsUrbResultPayload::Isoch(TsUrbIsochTransferResult {
                start_frame: 101,
                error_count: 1,
                iso_packet: vec![
                    UsbdIsoPacketDesc {
                        offset: 0,
                        length: 3,
                        status: 0,
                    },
                    UsbdIsoPacketDesc {
                        offset: 3,
                        length: 0,
                        status: stall,
                    },
                    UsbdIsoPacketDesc {
                        offset: 3,
                        length: 2,
                        status: 0,
                    },
                ],
            })),
        },
        hresult: 0,
        // Failed packet data is omitted by RDPEUSB and successful packet data
        // is packed in packet order.
        output_buffer: vec![1, 2, 3, 8, 9],
    });

    assert_eq!(
        isochronous_completion(completion, &[4, 3, 5]).unwrap(),
        Ok(IsochronousTransferOutput {
            start_frame: 101,
            actual_length: 5,
            data: vec![1, 2, 3, 8, 9],
            packets: vec![
                IsochronousPacketCompletion {
                    status: Ok(()),
                    actual_length: 3,
                },
                IsochronousPacketCompletion {
                    status: Err(UsbError::Stall),
                    actual_length: 0,
                },
                IsochronousPacketCompletion {
                    status: Ok(()),
                    actual_length: 2,
                },
            ],
        })
    );
}

#[test]
fn isochronous_completion_rejects_inconsistent_input_data() {
    let completion = |output_buffer| {
        CompletionData::TransferIn(TransferInCompletionResult {
            ts_urb_result: TsUrbResult {
                header: TsUrbResultHeader { usbd_status: 0 },
                payload: Some(TsUrbResultPayload::Isoch(TsUrbIsochTransferResult {
                    start_frame: 1,
                    error_count: 0,
                    iso_packet: vec![UsbdIsoPacketDesc {
                        offset: 17,
                        length: 2,
                        status: 0,
                    }],
                })),
            },
            hresult: 0,
            output_buffer,
        })
    };

    assert_eq!(
        isochronous_completion(completion(vec![1]), &[4]).unwrap_err(),
        CompletionError::IsochronousDataLengthMismatch { expected: 2, actual: 1 }
    );
}

#[test]
fn isochronous_completion_rejects_a_packet_longer_than_requested() {
    let completion = CompletionData::TransferIn(TransferInCompletionResult {
        ts_urb_result: TsUrbResult {
            header: TsUrbResultHeader { usbd_status: 0 },
            payload: Some(TsUrbResultPayload::Isoch(TsUrbIsochTransferResult {
                start_frame: 1,
                error_count: 0,
                iso_packet: vec![
                    UsbdIsoPacketDesc {
                        offset: 0,
                        length: 4,
                        status: 0,
                    },
                    UsbdIsoPacketDesc {
                        offset: 4,
                        length: 5,
                        status: 0,
                    },
                ],
            })),
        },
        hresult: 0,
        output_buffer: Vec::new(),
    });

    assert_eq!(
        isochronous_completion(completion, &[4, 4]).unwrap_err(),
        CompletionError::IsochronousPacketLengthExceeded {
            packet: 1,
            requested: 4,
            actual: 5,
        }
    );
}

#[test]
fn isochronous_completion_preserves_overall_failure_without_packet_validation() {
    let completion = CompletionData::TransferIn(TransferInCompletionResult {
        ts_urb_result: TsUrbResult {
            header: TsUrbResultHeader {
                usbd_status: USBD_STATUS_STALL_PID,
            },
            payload: Some(TsUrbResultPayload::Isoch(TsUrbIsochTransferResult {
                start_frame: 0,
                error_count: 0,
                iso_packet: Vec::new(),
            })),
        },
        hresult: 0,
        output_buffer: Vec::new(),
    });

    assert_eq!(
        isochronous_completion(completion, &[4, 4]).unwrap(),
        Err(UsbError::Stall)
    );
}

#[test]
fn isochronous_out_completion_uses_the_envelope_actual_length() {
    let stall = i32::from_ne_bytes(USBD_STATUS_STALL_PID.to_ne_bytes());
    let completion = CompletionData::TransferOut(TransferOutCompletionResult {
        ts_urb_result: TsUrbResult {
            header: TsUrbResultHeader { usbd_status: 0 },
            payload: Some(TsUrbResultPayload::Isoch(TsUrbIsochTransferResult {
                start_frame: 101,
                error_count: 1,
                iso_packet: vec![
                    UsbdIsoPacketDesc {
                        offset: 0,
                        length: 4,
                        status: 0,
                    },
                    UsbdIsoPacketDesc {
                        offset: 4,
                        length: 0,
                        status: stall,
                    },
                ],
            })),
        },
        hresult: 0,
        output_buffer_size: 7,
    });

    assert_eq!(
        isochronous_completion(completion, &[4, 3]).unwrap(),
        Ok(IsochronousTransferOutput {
            start_frame: 101,
            actual_length: 7,
            data: Vec::new(),
            packets: vec![
                IsochronousPacketCompletion {
                    status: Ok(()),
                    actual_length: 4,
                },
                IsochronousPacketCompletion {
                    status: Err(UsbError::Stall),
                    actual_length: 0,
                },
            ],
        })
    );
}

#[test]
fn select_configuration_carries_full_descriptor_and_selected_interfaces() {
    let descriptor = ConfigurationDescriptorSet::parse(&CONFIGURATION)
        .unwrap()
        .validate()
        .unwrap();
    let selections = [selection(0, 0), selection(1, 1)];

    let request = select_configuration(descriptor, &selections, UsbSpeed::High).unwrap();
    assert_eq!(request.output_buffer_size, 0);
    assert_eq!(request.ts_urb.func, UrbFunction::URB_FUNCTION_SELECT_CONFIGURATION);
    let TsUrbInKind::SelectConfig(urb) = request.ts_urb.kind else {
        panic!("selection used the wrong TS_URB variant");
    };
    let config = urb.desc.expect("configuration descriptor is present");
    assert_eq!(config.total_length, u16::try_from(CONFIGURATION.len()).unwrap());
    assert_eq!(config.trailing, CONFIGURATION[9..]);
    assert_eq!(urb.usbd_ifaces.len(), 2);
    assert_eq!(urb.usbd_ifaces[0].interface_number, 0);
    assert_eq!(urb.usbd_ifaces[0].alternate_setting, 0);
    assert_eq!(urb.usbd_ifaces[0].ts_usbd_pipe_info.len(), 2);
    assert_eq!(urb.usbd_ifaces[0].ts_usbd_pipe_info[0].max_packet_size, 192);
    assert_eq!(urb.usbd_ifaces[0].ts_usbd_pipe_info[1].max_packet_size, 512);
    assert_eq!(urb.usbd_ifaces[1].interface_number, 1);
    assert_eq!(urb.usbd_ifaces[1].alternate_setting, 1);
}

#[test]
fn select_interface_uses_the_selected_alternate_setting() {
    let descriptor = ConfigurationDescriptorSet::parse(&CONFIGURATION)
        .unwrap()
        .validate()
        .unwrap();

    let request = select_interface(42, descriptor, selection(1, 1), UsbSpeed::High).unwrap();
    assert_eq!(request.output_buffer_size, 0);
    assert_eq!(request.ts_urb.func, UrbFunction::URB_FUNCTION_SELECT_INTERFACE);
    let TsUrbInKind::SelectIface(urb) = request.ts_urb.kind else {
        panic!("interface selection used the wrong TS_URB variant");
    };
    assert_eq!(urb.config_handle, 42);
    assert_eq!(urb.usbd_iface.interface_number, 1);
    assert_eq!(urb.usbd_iface.alternate_setting, 1);
    assert_eq!(urb.usbd_iface.ts_usbd_pipe_info[0].max_packet_size, 32);
}

#[test]
fn selection_completions_return_opaque_rdpeusb_results() {
    let configuration_result = TsUrbSelectConfigResult {
        config_handle: 42,
        interface: Vec::new(),
    };
    let completion = CompletionData::TransferIn(TransferInCompletionResult {
        ts_urb_result: TsUrbResult {
            header: TsUrbResultHeader { usbd_status: 0 },
            payload: Some(TsUrbResultPayload::SelectConfig(configuration_result.clone())),
        },
        hresult: 0,
        output_buffer: Vec::new(),
    });
    assert_eq!(
        select_configuration_completion(completion).unwrap(),
        Ok(configuration_result)
    );

    let interface_result = TsUrbSelectInterfaceResult {
        interface: TsUsbdInterfaceInfoResult {
            interface_number: 3,
            alternate_setting: 1,
            class: 0xff,
            sub_class: 0,
            protocol: 0,
            interface_handle: 77,
            pipes: Vec::new(),
        },
    };
    let completion = CompletionData::TransferIn(TransferInCompletionResult {
        ts_urb_result: TsUrbResult {
            header: TsUrbResultHeader { usbd_status: 0 },
            payload: Some(TsUrbResultPayload::SelectIface(interface_result.clone())),
        },
        hresult: 0,
        output_buffer: Vec::new(),
    });
    assert_eq!(select_interface_completion(completion).unwrap(), Ok(interface_result));
}

#[test]
fn pipe_frame_and_reset_helpers_use_the_required_rdpeusb_envelopes() {
    let request = reset_pipe_and_clear_stall(55);
    assert_eq!(request.output_buffer_size, 0);
    assert_eq!(
        request.ts_urb.func,
        UrbFunction::URB_FUNCTION_SYNC_RESET_PIPE_AND_CLEAR_STALL
    );
    let TsUrbInKind::PipeReq(urb) = request.ts_urb.kind else {
        panic!("pipe reset used the wrong TS_URB variant");
    };
    assert_eq!(urb.pipe_handle, 55);
    assert_eq!(
        pipe_request_completion(transfer_in_completion(0, 0, Vec::new())).unwrap(),
        Ok(())
    );

    let request = current_frame_number();
    assert_eq!(request.output_buffer_size, 0);
    assert_eq!(request.ts_urb.func, UrbFunction::URB_FUNCTION_GET_CURRENT_FRAME_NUMBER);
    assert!(matches!(request.ts_urb.kind, TsUrbInKind::GetCurFrameNum(_)));
    let completion = CompletionData::TransferIn(TransferInCompletionResult {
        ts_urb_result: TsUrbResult {
            header: TsUrbResultHeader { usbd_status: 0 },
            payload: Some(TsUrbResultPayload::FrameNum(TsUrbGetCurrFrameNumResult {
                frame_number: 1234,
            })),
        },
        hresult: 0,
        output_buffer: Vec::new(),
    });
    assert_eq!(current_frame_number_completion(completion).unwrap(), Ok(1234));

    let request = reset_device();
    assert_eq!(request.ioctl_code, IoctlInternalUsb::ResetPort);
    assert!(request.input_buffer.is_empty());
    assert_eq!(request.output_buffer_size, 0);
    assert_eq!(
        reset_device_completion(CompletionData::IoControl(IoControlCompletionResult {
            hresult: 0,
            information: 0,
            output_buffer: Vec::new(),
        }))
        .unwrap(),
        Ok(())
    );
}

#[test]
fn selection_rejects_inconsistent_interface_sets() {
    let descriptor = ConfigurationDescriptorSet::parse(&CONFIGURATION)
        .unwrap()
        .validate()
        .unwrap();
    assert_eq!(
        select_configuration(descriptor, &[selection(0, 0), selection(0, 0)], UsbSpeed::High).unwrap_err(),
        ConversionError::DuplicateInterface { interface: 0 }
    );
    assert_eq!(
        select_configuration(descriptor, &[selection(0, 0)], UsbSpeed::High).unwrap_err(),
        ConversionError::MissingInterfaceSelection { interface: 1 }
    );
    let missing = selection(7, 0);
    assert_eq!(
        select_configuration(descriptor, &[missing], UsbSpeed::Full).unwrap_err(),
        ConversionError::InterfaceNotFound { selection: missing }
    );
}

#[test]
fn unconfigure_has_an_empty_transfer_in_request() {
    let request = unconfigure();
    assert_eq!(request.output_buffer_size, 0);
    let TsUrbInKind::SelectConfig(urb) = request.ts_urb.kind else {
        panic!("unconfigure used the wrong TS_URB variant");
    };
    assert!(urb.desc.is_none());
    assert!(urb.usbd_ifaces.is_empty());
}

#[test]
fn selection_rejects_high_bandwidth_bits_at_other_speeds() {
    let mut bytes = CONFIGURATION;
    bytes[22] = 0x40;
    bytes[23] = 0x08;
    let descriptor = ConfigurationDescriptorSet::parse(&bytes).unwrap().validate().unwrap();

    assert_eq!(
        select_configuration(descriptor, &[selection(0, 0), selection(1, 1)], UsbSpeed::Full).unwrap_err(),
        ConversionError::InvalidMaximumPacketSize {
            selection: selection(0, 0),
            raw: 0x0840
        }
    );
}

#[test]
fn selection_enforces_usb2_speed_dependent_packet_sizes() {
    let mut full_speed_bulk = CONFIGURATION;
    full_speed_bulk[22] = 64;
    full_speed_bulk[23] = 0;
    full_speed_bulk[29] = 64;
    full_speed_bulk[30] = 0;
    let descriptor = ConfigurationDescriptorSet::parse(&full_speed_bulk)
        .unwrap()
        .validate()
        .unwrap();
    select_configuration(descriptor, &[selection(0, 0), selection(1, 1)], UsbSpeed::Full).unwrap();

    let mut invalid_full_speed_bulk = full_speed_bulk;
    invalid_full_speed_bulk[29] = 0;
    invalid_full_speed_bulk[30] = 2;
    let descriptor = ConfigurationDescriptorSet::parse(&invalid_full_speed_bulk)
        .unwrap()
        .validate()
        .unwrap();
    assert_eq!(
        select_configuration(descriptor, &[selection(0, 0), selection(1, 1)], UsbSpeed::Full).unwrap_err(),
        ConversionError::InvalidMaximumPacketSize {
            selection: selection(0, 0),
            raw: 512
        }
    );

    let mut invalid_high_speed_bulk = CONFIGURATION;
    invalid_high_speed_bulk[29] = 64;
    invalid_high_speed_bulk[30] = 0;
    let descriptor = ConfigurationDescriptorSet::parse(&invalid_high_speed_bulk)
        .unwrap()
        .validate()
        .unwrap();
    assert_eq!(
        select_configuration(descriptor, &[selection(0, 0), selection(1, 1)], UsbSpeed::High).unwrap_err(),
        ConversionError::InvalidMaximumPacketSize {
            selection: selection(0, 0),
            raw: 64
        }
    );
}

#[test]
fn selection_enforces_default_interface_zero_isochronous_bandwidth() {
    let mut zero_bandwidth = CONFIGURATION;
    zero_bandwidth[21] = 1;
    zero_bandwidth[22] = 0;
    zero_bandwidth[23] = 0;
    let descriptor = ConfigurationDescriptorSet::parse(&zero_bandwidth)
        .unwrap()
        .validate()
        .unwrap();
    let request = select_configuration(descriptor, &[selection(0, 0), selection(1, 1)], UsbSpeed::High).unwrap();
    let TsUrbInKind::SelectConfig(urb) = request.ts_urb.kind else {
        panic!("configuration selection used the wrong TS_URB variant");
    };
    assert_eq!(urb.usbd_ifaces[0].ts_usbd_pipe_info[0].max_packet_size, 0);

    let mut nonzero_bandwidth = zero_bandwidth;
    nonzero_bandwidth[22] = 1;
    let descriptor = ConfigurationDescriptorSet::parse(&nonzero_bandwidth)
        .unwrap()
        .validate()
        .unwrap();
    assert_eq!(
        select_configuration(descriptor, &[selection(0, 0), selection(1, 1)], UsbSpeed::High).unwrap_err(),
        ConversionError::InvalidMaximumPacketSize {
            selection: selection(0, 0),
            raw: 1
        }
    );
}
