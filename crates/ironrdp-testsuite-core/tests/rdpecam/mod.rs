use std::sync::{Arc, Mutex};

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::pdu::{DrdynvcClientPdu, DrdynvcDataPdu};
use ironrdp_dvc::{DvcChannelListener as _, DvcProcessor as _};
use ironrdp_rdpecam::client::CameraBackend;
use ironrdp_rdpecam::pdu::{DeviceMessage, DevicePdu, EnumerationPdu};
use ironrdp_rdpecam::{
    CameraRedirectionSession, DeviceClient, DeviceDescriptor, ErrorCode, MediaFormat, MediaType, PropertyDescription,
    PropertyId, PropertyMode, PropertySet, PropertyValue, ProtocolVersion, SampleDelivery, SampleSink, SampleUplink,
    StartStreamInfo, StreamDescription,
};

#[derive(Debug)]
struct FakeCamera {
    media_type: MediaType,
    activate_count: usize,
    deactivate_count: usize,
    start_count: usize,
    stop_count: usize,
    sample_requests: usize,
    fail_next_start: bool,
    sink: Option<SampleSink>,
    brightness: PropertyValue,
}

impl FakeCamera {
    fn new() -> Self {
        Self {
            media_type: media_type(),
            activate_count: 0,
            deactivate_count: 0,
            start_count: 0,
            stop_count: 0,
            sample_requests: 0,
            fail_next_start: false,
            sink: None,
            brightness: PropertyValue {
                mode: PropertyMode::AUTO,
                value: 50,
            },
        }
    }

    fn with_media_type(media_type: MediaType) -> Self {
        Self {
            media_type,
            ..Self::new()
        }
    }
}

const BRIGHTNESS: PropertyDescription = PropertyDescription {
    property_set: PropertySet::VIDEO_PROC_AMP,
    property_id: PropertyId::VIDEO_PROC_AMP_BRIGHTNESS,
    capabilities: PropertyMode(PropertyMode::MANUAL.0 | PropertyMode::AUTO.0),
    min_value: 0,
    max_value: 100,
    step: 1,
    default_value: 50,
};

impl CameraBackend for FakeCamera {
    fn activate(&mut self) -> Result<(), ErrorCode> {
        self.activate_count += 1;
        Ok(())
    }

    fn deactivate(&mut self) -> Result<(), ErrorCode> {
        self.deactivate_count += 1;
        Ok(())
    }

    fn streams(&mut self) -> Result<Vec<StreamDescription>, ErrorCode> {
        Ok(vec![StreamDescription::color(true, false)])
    }

    fn media_types(&mut self, stream_index: u8) -> Result<Vec<MediaType>, ErrorCode> {
        if stream_index != 0 {
            return Err(ErrorCode::InvalidStreamNumber);
        }
        Ok(vec![self.media_type])
    }

    fn current_media_type(&mut self, stream_index: u8) -> Result<MediaType, ErrorCode> {
        if stream_index != 0 {
            return Err(ErrorCode::InvalidStreamNumber);
        }
        Ok(self.media_type)
    }

    fn start_streams(&mut self, streams: &[StartStreamInfo], sink: SampleSink) -> Result<(), ErrorCode> {
        if core::mem::take(&mut self.fail_next_start) {
            return Err(ErrorCode::Unexpected);
        }
        assert_eq!(
            streams,
            &[StartStreamInfo {
                stream_index: 0,
                media_type: self.media_type,
            }]
        );
        self.start_count += 1;
        self.sink = Some(sink);
        Ok(())
    }

    fn stop_streams(&mut self) -> Result<(), ErrorCode> {
        self.stop_count += 1;
        Ok(())
    }

    fn sample_requested(&mut self, stream_index: u8) {
        assert_eq!(stream_index, 0);
        self.sample_requests += 1;
    }

    fn properties(&mut self) -> Result<Vec<PropertyDescription>, ErrorCode> {
        Ok(vec![BRIGHTNESS])
    }

    fn property_value(
        &mut self,
        property_set: PropertySet,
        property_id: PropertyId,
    ) -> Result<PropertyValue, ErrorCode> {
        assert_eq!(
            (property_set, property_id),
            (BRIGHTNESS.property_set, BRIGHTNESS.property_id)
        );
        Ok(self.brightness)
    }

    fn set_property_value(
        &mut self,
        property_set: PropertySet,
        property_id: PropertyId,
        value: PropertyValue,
    ) -> Result<(), ErrorCode> {
        assert_eq!(
            (property_set, property_id),
            (BRIGHTNESS.property_set, BRIGHTNESS.property_id)
        );
        self.brightness = value;
        Ok(())
    }

    fn shutdown(&mut self) {
        self.stop_count += 1;
        self.deactivate_count += 1;
    }
}

#[test]
fn enumeration_negotiates_before_advertising_selected_devices() {
    let session = CameraRedirectionSession::new();
    let descriptor = DeviceDescriptor::new("Selected camera".into(), "RDCamera_Device_0".into()).unwrap();
    let device = session.select_device(descriptor.clone()).unwrap();
    let mut listener = device.listener(|| Some(FakeCamera::new()), Uplink::new().callback());
    let mut client = session.enumeration_client(vec![device]).unwrap();

    assert!(listener.create(42).is_none());
    let initial = client.start(41).unwrap();
    assert_eq!(
        decode_message::<EnumerationPdu>(&initial),
        EnumerationPdu::SelectVersionRequest(ProtocolVersion::V2)
    );
    assert!(!client.ready());

    let response = encode_vec(&EnumerationPdu::SelectVersionResponse(ProtocolVersion::V1)).unwrap();
    let advertised = client.process(41, &response).unwrap();
    assert!(client.ready());
    assert_eq!(session.negotiated_version(), Some(ProtocolVersion::V1));
    assert_eq!(
        decode_message::<EnumerationPdu>(&advertised),
        EnumerationPdu::DeviceAdded {
            version: ProtocolVersion::V1,
            device: descriptor,
        }
    );
    assert!(listener.create(42).is_some());
    // The listener can serve the device channel again after the server closes it.
    assert!(listener.create(43).is_some());
}

#[test]
fn cameras_added_before_negotiation_are_advertised_with_the_others() {
    let session = CameraRedirectionSession::new();
    let first = DeviceDescriptor::new("First camera".into(), "RDCamera_Device_0".into()).unwrap();
    let second = DeviceDescriptor::new("Second camera".into(), "RDCamera_Device_1".into()).unwrap();
    let mut client = session
        .enumeration_client(vec![session.select_device(first.clone()).unwrap()])
        .unwrap();
    client.start(41).unwrap();

    // Hotplug can race the version negotiation.
    assert!(
        client
            .add_device(session.select_device(second.clone()).unwrap())
            .unwrap()
            .is_empty()
    );
    let response = encode_vec(&EnumerationPdu::SelectVersionResponse(ProtocolVersion::V2)).unwrap();
    let advertised = client.process(41, &response).unwrap();
    let advertised: Vec<EnumerationPdu> = advertised
        .iter()
        .map(|message| decode(&encode_vec(&**message).unwrap()).unwrap())
        .collect();
    assert_eq!(
        advertised,
        [
            EnumerationPdu::DeviceAdded {
                version: ProtocolVersion::V2,
                device: first,
            },
            EnumerationPdu::DeviceAdded {
                version: ProtocolVersion::V2,
                device: second,
            },
        ]
    );
    assert_eq!(
        client.advertised_channels().collect::<Vec<_>>(),
        ["RDCamera_Device_0", "RDCamera_Device_1"]
    );
}

#[test]
fn enumeration_advertises_with_the_negotiated_version_2() {
    let session = CameraRedirectionSession::new();
    let descriptor = DeviceDescriptor::new("Selected camera".into(), "RDCamera_Device_0".into()).unwrap();
    let device = session.select_device(descriptor.clone()).unwrap();
    let mut client = session.enumeration_client(vec![device]).unwrap();
    client.start(41).unwrap();

    let response = encode_vec(&EnumerationPdu::SelectVersionResponse(ProtocolVersion::V2)).unwrap();
    let advertised = client.process(41, &response).unwrap();
    assert_eq!(session.negotiated_version(), Some(ProtocolVersion::V2));
    assert_eq!(
        decode_message::<EnumerationPdu>(&advertised),
        EnumerationPdu::DeviceAdded {
            version: ProtocolVersion::V2,
            device: descriptor,
        }
    );

    let removed = client.remove_device("RDCamera_Device_0").unwrap();
    assert_eq!(removed.len(), 1);
}

#[test]
fn device_state_machine_streams_only_after_activation_and_media_validation() {
    let uplink = Uplink::new();
    let device = negotiated_device(ProtocolVersion::V1);
    let mut client = DeviceClient::new(&device, FakeCamera::new(), uplink.callback());
    client.start(7).unwrap();

    assert_eq!(
        exchange(&mut client, 7, DevicePdu::SampleRequest(0)),
        Some(DevicePdu::SampleErrorResponse {
            stream_index: 0,
            error: ErrorCode::NotInitialized,
        })
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::ActivateDeviceRequest),
        Some(DevicePdu::SuccessResponse)
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::ActivateDeviceRequest),
        Some(DevicePdu::SuccessResponse)
    );
    assert_eq!(client.with_backend(|backend| backend.activate_count).unwrap(), 1);

    assert_eq!(
        exchange(&mut client, 7, DevicePdu::StreamListRequest),
        Some(DevicePdu::StreamListResponse(vec![StreamDescription::color(
            true, false
        )]))
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::MediaTypeListRequest(1)),
        Some(DevicePdu::ErrorResponse(ErrorCode::InvalidStreamNumber))
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::StopStreamsRequest),
        Some(DevicePdu::SuccessResponse)
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::SampleRequest(0)),
        Some(DevicePdu::SampleErrorResponse {
            stream_index: 0,
            error: ErrorCode::InvalidRequest,
        })
    );

    let start = StartStreamInfo {
        stream_index: 0,
        media_type: media_type(),
    };
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::StartStreamsRequest(vec![start])),
        Some(DevicePdu::SuccessResponse)
    );
    let first_sink = client.with_backend(|backend| backend.sink.clone()).unwrap().unwrap();
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::StartStreamsRequest(vec![start])),
        Some(DevicePdu::SuccessResponse)
    );
    assert!(!first_sink.is_active());
    assert!(client.is_streaming());
    client.with_backend(|backend| backend.fail_next_start = true);
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::StartStreamsRequest(vec![start])),
        Some(DevicePdu::ErrorResponse(ErrorCode::Unexpected))
    );
    assert!(client.is_streaming());

    // A sample captured before the server asks for one is not sent.
    let sink = client.with_backend(|backend| backend.sink.clone()).unwrap().unwrap();
    assert!(sink.is_active());
    assert_eq!(sink.send_sample(0, vec![0x55; 12]), SampleDelivery::NotRequested);
    assert!(uplink.take().is_empty());

    // Sample Requests are answered asynchronously, once per request.
    assert_eq!(exchange(&mut client, 7, DevicePdu::SampleRequest(0)), None);
    assert_eq!(client.with_backend(|backend| backend.sample_requests).unwrap(), 1);
    assert_eq!(sink.pending_requests(0), 1);
    assert_eq!(sink.send_sample(0, vec![0x55; 12]), SampleDelivery::Sent);
    assert_eq!(sink.send_sample(0, vec![0x55; 12]), SampleDelivery::NotRequested);
    assert_eq!(
        uplink.take(),
        vec![(
            7,
            DeviceMessage::new(
                ProtocolVersion::V1,
                DevicePdu::SampleResponse {
                    stream_index: 0,
                    sample: vec![0x55; 12],
                }
            )
        )]
    );

    // A sample that does not match the selected media type is answered with an error.
    assert_eq!(exchange(&mut client, 7, DevicePdu::SampleRequest(0)), None);
    assert_eq!(sink.send_sample(0, vec![0x55; 11]), SampleDelivery::Rejected);
    assert_eq!(
        uplink.take(),
        vec![(
            7,
            DeviceMessage::new(
                ProtocolVersion::V1,
                DevicePdu::SampleErrorResponse {
                    stream_index: 0,
                    error: ErrorCode::InvalidMediaType,
                }
            )
        )]
    );

    // A sample the session cannot accept leaves the request outstanding.
    assert_eq!(exchange(&mut client, 7, DevicePdu::SampleRequest(0)), None);
    uplink.fail_next();
    assert_eq!(sink.send_sample(0, vec![0x55; 12]), SampleDelivery::Failed);
    assert_eq!(sink.pending_requests(0), 1);

    assert_eq!(
        exchange(&mut client, 7, DevicePdu::StopStreamsRequest),
        Some(DevicePdu::SuccessResponse)
    );
    assert_eq!(sink.send_sample(0, vec![0x55; 12]), SampleDelivery::Stopped);
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::DeactivateDeviceRequest),
        Some(DevicePdu::SuccessResponse)
    );
    assert!(client.is_activated());
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::DeactivateDeviceRequest),
        Some(DevicePdu::SuccessResponse)
    );
    assert!(!client.is_activated());
    assert_eq!(client.with_backend(|backend| backend.deactivate_count).unwrap(), 1);
}

#[test]
fn channels_of_one_device_share_activation_and_streams() {
    let uplink = Uplink::new();
    let device = negotiated_device(ProtocolVersion::V2);
    let created = Arc::new(core::sync::atomic::AtomicUsize::new(0));
    let counter = Arc::clone(&created);
    let mut listener = device.listener(
        move || {
            counter.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
            Some(FakeCamera::new())
        },
        uplink.callback(),
    );
    let mut control = listener.create(20).unwrap();
    let mut stream = listener.create(21).unwrap();
    // Both channels drive one backend.
    assert_eq!(created.load(core::sync::atomic::Ordering::SeqCst), 1);
    control.start(20).unwrap();
    stream.start(21).unwrap();

    let send = |processor: &mut Box<dyn ironrdp_dvc::DvcClientProcessor>, channel_id, pdu| {
        let encoded = encode_vec(&DeviceMessage::new(ProtocolVersion::V2, pdu)).unwrap();
        let response = processor.process(channel_id, &encoded).unwrap();
        (!response.is_empty()).then(|| decode_message::<DeviceMessage>(&response).pdu)
    };
    // Windows activates on one channel and streams on another.
    assert_eq!(
        send(&mut control, 20, DevicePdu::ActivateDeviceRequest),
        Some(DevicePdu::SuccessResponse)
    );
    assert_eq!(
        send(&mut stream, 21, DevicePdu::StreamListRequest),
        Some(DevicePdu::StreamListResponse(vec![StreamDescription::color(
            true, false
        )]))
    );
    let start = StartStreamInfo {
        stream_index: 0,
        media_type: media_type(),
    };
    assert_eq!(
        send(&mut stream, 21, DevicePdu::StartStreamsRequest(vec![start])),
        Some(DevicePdu::SuccessResponse)
    );
    assert_eq!(send(&mut control, 20, DevicePdu::SampleRequest(0)), None);

    // The sample answers on the channel that asked for it.
    let client = stream.as_any().downcast_ref::<DeviceClient<FakeCamera>>().unwrap();
    let sink = client.with_backend(|backend| backend.sink.clone()).unwrap().unwrap();
    assert_eq!(sink.send_sample(0, vec![0x55; 12]), SampleDelivery::Sent);
    assert_eq!(uplink.take()[0].0, 20);

    // Closing the activating channel releases the device.
    control.close(20);
    assert!(!sink.is_active());
    assert_eq!(
        send(&mut stream, 21, DevicePdu::StreamListRequest),
        Some(DevicePdu::ErrorResponse(ErrorCode::NotInitialized))
    );
}

#[test]
fn overlapping_sample_requests_are_answered_on_their_own_channels() {
    let uplink = Uplink::new();
    let device = negotiated_device(ProtocolVersion::V2);
    let mut listener = device.listener(|| Some(FakeCamera::new()), uplink.callback());
    let mut first = listener.create(20).unwrap();
    let mut second = listener.create(21).unwrap();
    first.start(20).unwrap();
    second.start(21).unwrap();

    let send = |processor: &mut Box<dyn ironrdp_dvc::DvcClientProcessor>, channel_id, pdu| {
        let encoded = encode_vec(&DeviceMessage::new(ProtocolVersion::V2, pdu)).unwrap();
        let response = processor.process(channel_id, &encoded).unwrap();
        (!response.is_empty()).then(|| decode_message::<DeviceMessage>(&response).pdu)
    };
    send(&mut first, 20, DevicePdu::ActivateDeviceRequest);
    let start = StartStreamInfo {
        stream_index: 0,
        media_type: media_type(),
    };
    assert_eq!(
        send(&mut first, 20, DevicePdu::StartStreamsRequest(vec![start])),
        Some(DevicePdu::SuccessResponse)
    );
    assert_eq!(send(&mut second, 21, DevicePdu::SampleRequest(0)), None);
    assert_eq!(send(&mut first, 20, DevicePdu::SampleRequest(0)), None);

    let client = first.as_any().downcast_ref::<DeviceClient<FakeCamera>>().unwrap();
    let sink = client.with_backend(|backend| backend.sink.clone()).unwrap().unwrap();
    assert_eq!(sink.pending_requests(0), 2);
    assert_eq!(sink.send_sample(0, vec![0x55; 12]), SampleDelivery::Sent);
    assert_eq!(sink.send_sample(0, vec![0x55; 12]), SampleDelivery::Sent);
    let channels: Vec<u32> = uplink.take().into_iter().map(|(channel_id, _)| channel_id).collect();
    assert_eq!(channels, [21, 20]);
}

#[test]
fn deactivating_on_another_channel_takes_over_the_activation() {
    let uplink = Uplink::new();
    let device = negotiated_device(ProtocolVersion::V2);
    let mut listener = device.listener(|| Some(FakeCamera::new()), uplink.callback());
    let mut first = listener.create(20).unwrap();
    let mut second = listener.create(21).unwrap();
    first.start(20).unwrap();
    second.start(21).unwrap();

    let send = |processor: &mut Box<dyn ironrdp_dvc::DvcClientProcessor>, channel_id, pdu| {
        let encoded = encode_vec(&DeviceMessage::new(ProtocolVersion::V2, pdu)).unwrap();
        let response = processor.process(channel_id, &encoded).unwrap();
        (!response.is_empty()).then(|| decode_message::<DeviceMessage>(&response).pdu)
    };
    assert_eq!(
        send(&mut first, 20, DevicePdu::ActivateDeviceRequest),
        Some(DevicePdu::SuccessResponse)
    );
    assert_eq!(
        send(&mut second, 21, DevicePdu::DeactivateDeviceRequest),
        Some(DevicePdu::SuccessResponse)
    );
    assert_eq!(
        send(&mut second, 21, DevicePdu::ActivateDeviceRequest),
        Some(DevicePdu::SuccessResponse)
    );
    let start = StartStreamInfo {
        stream_index: 0,
        media_type: media_type(),
    };
    assert_eq!(
        send(&mut second, 21, DevicePdu::StartStreamsRequest(vec![start])),
        Some(DevicePdu::SuccessResponse)
    );

    // The first channel no longer holds an activation, so closing it leaves the stream running.
    first.close(20);
    let client = second.as_any().downcast_ref::<DeviceClient<FakeCamera>>().unwrap();
    assert!(client.is_activated());
    assert!(client.is_streaming());
    assert_eq!(send(&mut second, 21, DevicePdu::SampleRequest(0)), None);
}

#[test]
fn closing_the_activating_channel_releases_an_idle_device() {
    let uplink = Uplink::new();
    let device = negotiated_device(ProtocolVersion::V2);
    let mut listener = device.listener(|| Some(FakeCamera::new()), uplink.callback());
    let mut control = listener.create(20).unwrap();
    let observer = listener.create(21).unwrap();
    control.start(20).unwrap();
    let encoded = encode_vec(&DeviceMessage::new(
        ProtocolVersion::V2,
        DevicePdu::ActivateDeviceRequest,
    ))
    .unwrap();
    control.process(20, &encoded).unwrap();

    // Activated but not streaming: closing the channel still shuts the backend down.
    control.close(20);
    let client = observer.as_any().downcast_ref::<DeviceClient<FakeCamera>>().unwrap();
    assert!(!client.is_activated());
    assert_eq!(client.with_backend(|backend| backend.deactivate_count).unwrap(), 1);
}

#[test]
fn requests_with_another_version_header_are_rejected() {
    let device = negotiated_device(ProtocolVersion::V2);
    let mut client = DeviceClient::new(&device, FakeCamera::new(), Uplink::new().callback());
    client.start(7).unwrap();
    assert_eq!(
        exchange_versioned(&mut client, 7, ProtocolVersion::V1, DevicePdu::ActivateDeviceRequest),
        Some(DeviceMessage::new(
            ProtocolVersion::V2,
            DevicePdu::ErrorResponse(ErrorCode::InvalidMessage)
        ))
    );
    assert_eq!(client.with_backend(|backend| backend.activate_count).unwrap(), 0);

    assert_eq!(
        exchange(&mut client, 7, DevicePdu::ActivateDeviceRequest),
        Some(DevicePdu::SuccessResponse)
    );
    let start = StartStreamInfo {
        stream_index: 0,
        media_type: media_type(),
    };
    assert_eq!(
        exchange_versioned(
            &mut client,
            7,
            ProtocolVersion::V1,
            DevicePdu::StartStreamsRequest(vec![start])
        ),
        Some(DeviceMessage::new(
            ProtocolVersion::V2,
            DevicePdu::ErrorResponse(ErrorCode::InvalidMessage)
        ))
    );
    assert!(!client.is_streaming());
    assert_eq!(
        exchange_versioned(&mut client, 7, ProtocolVersion::V1, DevicePdu::SampleRequest(0)),
        Some(DeviceMessage::new(
            ProtocolVersion::V2,
            DevicePdu::SampleErrorResponse {
                stream_index: 0,
                error: ErrorCode::InvalidMessage,
            }
        ))
    );

    let device = negotiated_device(ProtocolVersion::V1);
    let mut client = DeviceClient::new(&device, FakeCamera::new(), Uplink::new().callback());
    client.start(8).unwrap();
    assert_eq!(
        exchange_versioned(&mut client, 8, ProtocolVersion::V2, DevicePdu::ActivateDeviceRequest),
        Some(DeviceMessage::new(
            ProtocolVersion::V1,
            DevicePdu::ErrorResponse(ErrorCode::InvalidMessage)
        ))
    );
    assert!(!client.is_activated());
}

#[test]
fn compressed_media_types_are_streamed_with_picture_checks() {
    let uplink = Uplink::new();
    let mjpeg = MediaType::new(MediaFormat::Mjpg, 640, 480, 30, 1, 1, 1, MediaType::DECODING_REQUIRED).unwrap();
    let device = negotiated_device(ProtocolVersion::V2);
    let mut client = DeviceClient::new(&device, FakeCamera::with_media_type(mjpeg), uplink.callback());
    client.start(7).unwrap();

    assert_eq!(
        exchange(&mut client, 7, DevicePdu::ActivateDeviceRequest),
        Some(DevicePdu::SuccessResponse)
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::MediaTypeListRequest(0)),
        Some(DevicePdu::MediaTypeListResponse(vec![mjpeg]))
    );
    let start = StartStreamInfo {
        stream_index: 0,
        media_type: mjpeg,
    };
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::StartStreamsRequest(vec![start])),
        Some(DevicePdu::SuccessResponse)
    );
    let sink = client.with_backend(|backend| backend.sink.clone()).unwrap().unwrap();

    assert_eq!(exchange(&mut client, 7, DevicePdu::SampleRequest(0)), None);
    assert_eq!(sink.send_sample(0, vec![0, 0, 0, 1, 0x67]), SampleDelivery::Rejected);
    assert_eq!(exchange(&mut client, 7, DevicePdu::SampleRequest(0)), None);
    assert_eq!(sink.send_sample(0, vec![0xFF, 0xD8, 0xFF, 0xD9]), SampleDelivery::Sent);
    let sent = uplink.take();
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[1].1,
        DeviceMessage::new(
            ProtocolVersion::V2,
            DevicePdu::SampleResponse {
                stream_index: 0,
                sample: vec![0xFF, 0xD8, 0xFF, 0xD9],
            }
        )
    );

    let h264 = MediaType::new(MediaFormat::H264, 640, 480, 30, 1, 1, 1, MediaType::DECODING_REQUIRED).unwrap();
    assert!(h264.validate_sample(&[0, 0, 0, 1, 0x67]));
    assert!(h264.validate_sample(&[0, 0, 1, 0x65]));
    assert!(!h264.validate_sample(&[0xFF, 0xD8]));
}

#[test]
fn version_2_properties_are_listed_read_and_validated_before_writes() {
    let device = negotiated_device(ProtocolVersion::V2);
    let mut client = DeviceClient::new(&device, FakeCamera::new(), Uplink::new().callback());
    client.start(7).unwrap();
    assert_eq!(client.version(), ProtocolVersion::V2);

    assert_eq!(
        exchange_versioned(&mut client, 7, ProtocolVersion::V2, DevicePdu::ActivateDeviceRequest),
        Some(DeviceMessage::new(ProtocolVersion::V2, DevicePdu::SuccessResponse))
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::PropertyListRequest),
        Some(DevicePdu::PropertyListResponse(vec![BRIGHTNESS]))
    );
    assert_eq!(
        exchange(
            &mut client,
            7,
            DevicePdu::PropertyValueRequest {
                property_set: PropertySet::VIDEO_PROC_AMP,
                property_id: PropertyId::VIDEO_PROC_AMP_BRIGHTNESS,
            }
        ),
        Some(DevicePdu::PropertyValueResponse(PropertyValue {
            mode: PropertyMode::AUTO,
            value: 50,
        }))
    );
    assert_eq!(
        exchange(
            &mut client,
            7,
            DevicePdu::PropertyValueRequest {
                property_set: PropertySet(9),
                property_id: PropertyId::VIDEO_PROC_AMP_BRIGHTNESS,
            }
        ),
        Some(DevicePdu::ErrorResponse(ErrorCode::SetNotFound))
    );
    assert_eq!(
        exchange(
            &mut client,
            7,
            DevicePdu::PropertyValueRequest {
                property_set: PropertySet::CAMERA_CONTROL,
                property_id: PropertyId::CAMERA_CONTROL_ZOOM,
            }
        ),
        Some(DevicePdu::ErrorResponse(ErrorCode::ItemNotFound))
    );

    let set = |mode, value| DevicePdu::SetPropertyValueRequest {
        property_set: PropertySet::VIDEO_PROC_AMP,
        property_id: PropertyId::VIDEO_PROC_AMP_BRIGHTNESS,
        value: PropertyValue { mode, value },
    };
    assert_eq!(
        exchange(&mut client, 7, set(PropertyMode::MANUAL, 101)),
        Some(DevicePdu::ErrorResponse(ErrorCode::InvalidRequest))
    );
    assert_eq!(
        exchange(&mut client, 7, set(PropertyMode(3), 10)),
        Some(DevicePdu::ErrorResponse(ErrorCode::InvalidRequest))
    );
    assert_eq!(
        exchange(&mut client, 7, set(PropertyMode::MANUAL, 75)),
        Some(DevicePdu::SuccessResponse)
    );
    assert_eq!(
        client.with_backend(|backend| backend.brightness).unwrap(),
        PropertyValue {
            mode: PropertyMode::MANUAL,
            value: 75,
        }
    );
}

#[test]
fn version_1_channel_rejects_property_messages() {
    let device = negotiated_device(ProtocolVersion::V1);
    let mut client = DeviceClient::new(&device, FakeCamera::new(), Uplink::new().callback());
    client.start(7).unwrap();
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::ActivateDeviceRequest),
        Some(DevicePdu::SuccessResponse)
    );

    // A property message with a version 2 header on a version 1 channel is out of sequence.
    assert_eq!(
        exchange_versioned(&mut client, 7, ProtocolVersion::V2, DevicePdu::PropertyListRequest),
        Some(DeviceMessage::new(
            ProtocolVersion::V1,
            DevicePdu::ErrorResponse(ErrorCode::InvalidMessage)
        ))
    );
    // A property message with a version 1 header is malformed.
    let response = client.process(7, &[1, 0x14]).unwrap();
    assert_eq!(
        decode_message::<DeviceMessage>(&response).pdu,
        DevicePdu::ErrorResponse(ErrorCode::InvalidMessage)
    );
}

#[test]
fn malformed_device_message_is_rejected_without_backend_activation() {
    let device = negotiated_device(ProtocolVersion::V1);
    let mut client = DeviceClient::new(&device, FakeCamera::new(), Uplink::new().callback());
    client.start(7).unwrap();

    let response = client.process(7, &[1]).unwrap();
    assert_eq!(
        decode_message::<DeviceMessage>(&response).pdu,
        DevicePdu::ErrorResponse(ErrorCode::InvalidMessage)
    );
    assert_eq!(client.with_backend(|backend| backend.activate_count).unwrap(), 0);

    let response = encode_vec(&DeviceMessage::new(ProtocolVersion::V1, DevicePdu::SuccessResponse)).unwrap();
    assert!(client.process(7, &response).unwrap().is_empty());
}

#[test]
fn media_type_validation_enforces_plane_geometry_and_sample_length() {
    assert!(MediaType::new(MediaFormat::Nv12, 3, 2, 30, 1, 1, 1, 0).is_err());
    assert!(MediaType::new(MediaFormat::Rgb32, 8192, 8192, 30, 1, 1, 1, 0).is_err());
    assert!(MediaType::new(MediaFormat::Rgb32, 3840, 2160, 30, 1, 1, 1, 0).is_ok());

    let media_type = media_type();
    assert!(media_type.validate_sample(&[0; 12]));
    assert!(!media_type.validate_sample(&[0; 11]));

    let message = DeviceMessage::new(
        ProtocolVersion::V1,
        DevicePdu::StartStreamsRequest(vec![StartStreamInfo {
            stream_index: 0,
            media_type,
        }]),
    );
    let encoded = encode_vec(&message).unwrap();
    assert_eq!(decode::<DeviceMessage>(&encoded).unwrap(), message);
}

#[test]
fn post_removal_request_returns_protocol_error_without_aborting_processing() {
    let session = CameraRedirectionSession::new();
    let descriptor = DeviceDescriptor::new("Selected camera".into(), "RDCamera_Device_0".into()).unwrap();
    let device = session.select_device(descriptor).unwrap();
    let mut enumeration = session.enumeration_client(vec![device.clone()]).unwrap();
    enumeration.start(41).unwrap();
    let response = encode_vec(&EnumerationPdu::SelectVersionResponse(ProtocolVersion::V1)).unwrap();
    enumeration.process(41, &response).unwrap();

    let mut client = DeviceClient::new(&device, FakeCamera::new(), Uplink::new().callback());
    client.start(7).unwrap();
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::ActivateDeviceRequest),
        Some(DevicePdu::SuccessResponse)
    );
    enumeration.remove_device("RDCamera_Device_0").unwrap();

    assert_eq!(
        exchange(&mut client, 7, DevicePdu::SampleRequest(0)),
        Some(DevicePdu::SampleErrorResponse {
            stream_index: 0,
            error: ErrorCode::NotInitialized,
        })
    );
    assert_eq!(client.with_backend(|backend| backend.deactivate_count).unwrap(), 1);
}

#[test]
fn decoder_bounds_arrays_before_allocating_and_retains_unknown_advertisement_bits() {
    let mut oversized = vec![1, 10];
    oversized.resize(2 + 256 * 5, 0);
    assert!(decode::<DeviceMessage>(&oversized).is_err());

    let mut oversized_properties = vec![2, 0x15];
    oversized_properties.resize(2 + 257 * 19, 0);
    assert!(decode::<DeviceMessage>(&oversized_properties).is_err());

    let stream = StreamDescription {
        frame_source_types: StreamDescription::COLOR | 0x8000,
        selected: true,
        can_be_shared: false,
    };
    let message = DeviceMessage::new(ProtocolVersion::V1, DevicePdu::StreamListResponse(vec![stream]));
    let encoded = encode_vec(&message).unwrap();
    assert_eq!(decode::<DeviceMessage>(&encoded).unwrap(), message);

    let property = PropertyDescription {
        capabilities: PropertyMode(0x80 | PropertyMode::MANUAL.0),
        ..BRIGHTNESS
    };
    let message = DeviceMessage::new(ProtocolVersion::V2, DevicePdu::PropertyListResponse(vec![property]));
    let encoded = encode_vec(&message).unwrap();
    assert_eq!(decode::<DeviceMessage>(&encoded).unwrap(), message);

    // Version 2 messages cannot be encoded with a version 1 header.
    assert!(encode_vec(&DeviceMessage::new(ProtocolVersion::V1, DevicePdu::PropertyListRequest)).is_err());
}

#[test]
fn wire_fixtures_match_ms_rdpecam_version_1_layouts() {
    let descriptor = DeviceDescriptor::new("A".into(), "C".into()).unwrap();
    let enumeration_fixtures = [
        (EnumerationPdu::SelectVersionRequest(ProtocolVersion::V1), vec![1, 3]),
        (EnumerationPdu::SelectVersionResponse(ProtocolVersion::V1), vec![1, 4]),
        (
            EnumerationPdu::DeviceAdded {
                version: ProtocolVersion::V1,
                device: descriptor,
            },
            vec![1, 5, b'A', 0, 0, 0, b'C', 0],
        ),
        (
            EnumerationPdu::DeviceRemoved {
                version: ProtocolVersion::V1,
                channel_name: "C".into(),
            },
            vec![1, 6, b'C', 0],
        ),
    ];
    for (pdu, expected) in enumeration_fixtures {
        assert_eq!(encode_vec(&pdu).unwrap(), expected);
        assert_eq!(decode::<EnumerationPdu>(&expected).unwrap(), pdu);
    }

    let media_type = media_type();
    let media = media_type_fixture();
    let mut stream_list = vec![1, 10];
    stream_list.extend_from_slice(&[1, 0, 1, 1, 0]);
    let mut media_list = vec![1, 12];
    media_list.extend_from_slice(&media);
    let mut current_media = vec![1, 14];
    current_media.extend_from_slice(&media);
    let mut start_streams = vec![1, 15, 2];
    start_streams.extend_from_slice(&media);
    let device_fixtures = [
        (DevicePdu::SuccessResponse, vec![1, 1]),
        (
            DevicePdu::ErrorResponse(ErrorCode::InvalidRequest),
            vec![1, 2, 4, 0, 0, 0],
        ),
        (DevicePdu::ActivateDeviceRequest, vec![1, 7]),
        (DevicePdu::DeactivateDeviceRequest, vec![1, 8]),
        (DevicePdu::StreamListRequest, vec![1, 9]),
        (
            DevicePdu::StreamListResponse(vec![StreamDescription::color(true, false)]),
            stream_list,
        ),
        (DevicePdu::MediaTypeListRequest(2), vec![1, 11, 2]),
        (DevicePdu::MediaTypeListResponse(vec![media_type]), media_list),
        (DevicePdu::CurrentMediaTypeRequest(2), vec![1, 13, 2]),
        (DevicePdu::CurrentMediaTypeResponse(media_type), current_media),
        (
            DevicePdu::StartStreamsRequest(vec![StartStreamInfo {
                stream_index: 2,
                media_type,
            }]),
            start_streams,
        ),
        (DevicePdu::StopStreamsRequest, vec![1, 16]),
        (DevicePdu::SampleRequest(2), vec![1, 17, 2]),
        (
            DevicePdu::SampleResponse {
                stream_index: 2,
                sample: vec![0xAA, 0xBB],
            },
            vec![1, 18, 2, 0xAA, 0xBB],
        ),
        (
            DevicePdu::SampleErrorResponse {
                stream_index: 2,
                error: ErrorCode::InvalidStreamNumber,
            },
            vec![1, 19, 2, 5, 0, 0, 0],
        ),
    ];
    for (pdu, expected) in device_fixtures {
        let message = DeviceMessage::new(ProtocolVersion::V1, pdu);
        assert_eq!(encode_vec(&message).unwrap(), expected);
        assert_eq!(decode::<DeviceMessage>(&expected).unwrap(), message);
    }

    assert!(decode::<DeviceMessage>(&[1, 2, 4, 0, 0]).is_err());
    assert!(decode::<DeviceMessage>(&[1, 7, 0]).is_err());
    assert!(decode::<DeviceMessage>(&[1, 11]).is_err());
    assert!(decode::<DeviceMessage>(&[1, 19, 0, 5, 0, 0]).is_err());
    assert!(decode::<DeviceMessage>(&[3, 1]).is_err());
}

#[test]
fn wire_fixtures_match_ms_rdpecam_version_2_layouts() {
    assert_eq!(
        encode_vec(&EnumerationPdu::SelectVersionRequest(ProtocolVersion::V2)).unwrap(),
        vec![2, 3]
    );
    assert_eq!(
        decode::<EnumerationPdu>(&[2, 4]).unwrap(),
        EnumerationPdu::SelectVersionResponse(ProtocolVersion::V2)
    );

    let mut property_list = vec![2, 0x15];
    property_list.extend_from_slice(&[
        2, // PropertySet
        2, // PropertyId
        3, // Capabilities
        0, 0, 0, 0, // MinValue
        100, 0, 0, 0, // MaxValue
        1, 0, 0, 0, // Step
        50, 0, 0, 0, // DefaultValue
    ]);
    let device_fixtures = [
        (DevicePdu::SuccessResponse, vec![2, 1]),
        (
            DevicePdu::ErrorResponse(ErrorCode::OperationNotSupported),
            vec![2, 2, 10, 0, 0, 0],
        ),
        (DevicePdu::PropertyListRequest, vec![2, 0x14]),
        (DevicePdu::PropertyListResponse(vec![BRIGHTNESS]), property_list),
        (DevicePdu::PropertyListResponse(Vec::new()), vec![2, 0x15]),
        (
            DevicePdu::PropertyValueRequest {
                property_set: PropertySet::CAMERA_CONTROL,
                property_id: PropertyId::CAMERA_CONTROL_FOCUS,
            },
            vec![2, 0x16, 1, 2],
        ),
        (
            DevicePdu::PropertyValueResponse(PropertyValue {
                mode: PropertyMode::MANUAL,
                value: -2,
            }),
            vec![2, 0x17, 1, 0xFE, 0xFF, 0xFF, 0xFF],
        ),
        (
            DevicePdu::SetPropertyValueRequest {
                property_set: PropertySet::CAMERA_CONTROL,
                property_id: PropertyId::CAMERA_CONTROL_ZOOM,
                value: PropertyValue {
                    mode: PropertyMode::AUTO,
                    value: 0,
                },
            },
            vec![2, 0x18, 1, 6, 2, 0, 0, 0, 0],
        ),
    ];
    for (pdu, expected) in device_fixtures {
        let message = DeviceMessage::new(ProtocolVersion::V2, pdu);
        assert_eq!(encode_vec(&message).unwrap(), expected);
        assert_eq!(decode::<DeviceMessage>(&expected).unwrap(), message);
    }

    assert!(decode::<DeviceMessage>(&[2, 0x16, 1]).is_err());
    assert!(decode::<DeviceMessage>(&[2, 0x15, 0]).is_err());
    assert!(decode::<DeviceMessage>(&[2, 0x18, 1, 6, 2, 0, 0, 0]).is_err());
}

/// Records the Sample Responses a [`SampleSink`] sends through its uplink.
#[derive(Clone)]
struct Uplink {
    sent: Arc<Mutex<Vec<(u32, DeviceMessage)>>>,
    fail_next: Arc<Mutex<bool>>,
}

impl Uplink {
    fn new() -> Self {
        Self {
            sent: Arc::default(),
            fail_next: Arc::default(),
        }
    }

    fn callback(&self) -> SampleUplink {
        let sent = Arc::clone(&self.sent);
        let fail_next = Arc::clone(&self.fail_next);
        Arc::new(move |channel_id, messages| {
            if core::mem::take(&mut *fail_next.lock().unwrap()) {
                return Err(ironrdp_pdu::pdu_other_err!("uplink is full"));
            }
            for message in messages {
                let encoded = message.encode_unframed_pdu().unwrap();
                let DrdynvcClientPdu::Data(data) = decode::<DrdynvcClientPdu>(&encoded).unwrap() else {
                    panic!("expected a DVC data PDU");
                };
                assert_eq!(data.channel_id(), channel_id);
                let DrdynvcDataPdu::Data(data) = data else {
                    panic!("expected an unfragmented DVC data PDU");
                };
                sent.lock()
                    .unwrap()
                    .push((channel_id, decode::<DeviceMessage>(data.data()).unwrap()));
            }
            Ok(())
        })
    }

    fn fail_next(&self) {
        *self.fail_next.lock().unwrap() = true;
    }

    fn take(&self) -> Vec<(u32, DeviceMessage)> {
        core::mem::take(&mut *self.sent.lock().unwrap())
    }
}

fn exchange(client: &mut DeviceClient<FakeCamera>, channel_id: u32, request: DevicePdu) -> Option<DevicePdu> {
    let version = client.version();
    exchange_versioned(client, channel_id, version, request).map(|response| {
        assert_eq!(response.version, version);
        response.pdu
    })
}

fn exchange_versioned(
    client: &mut DeviceClient<FakeCamera>,
    channel_id: u32,
    version: ProtocolVersion,
    request: DevicePdu,
) -> Option<DeviceMessage> {
    let encoded = encode_vec(&DeviceMessage::new(version, request)).unwrap();
    let response = client.process(channel_id, &encoded).unwrap();
    if response.is_empty() {
        return None;
    }
    Some(decode_message(&response))
}

fn decode_message<T>(messages: &[ironrdp_dvc::DvcMessage]) -> T
where
    T: for<'de> ironrdp_core::Decode<'de>,
{
    assert_eq!(messages.len(), 1);
    let encoded = encode_vec(&*messages[0]).unwrap();
    decode(&encoded).unwrap()
}

fn media_type() -> MediaType {
    MediaType::new(MediaFormat::Rgb24, 2, 2, 30, 1, 1, 1, 0).unwrap()
}

fn media_type_fixture() -> Vec<u8> {
    vec![
        6, // Format
        2, 0, 0, 0, // Width
        2, 0, 0, 0, // Height
        30, 0, 0, 0, // FrameRateNumerator
        1, 0, 0, 0, // FrameRateDenominator
        1, 0, 0, 0, // PixelAspectRatioNumerator
        1, 0, 0, 0, // PixelAspectRatioDenominator
        0, // Flags
    ]
}

fn negotiated_device(version: ProtocolVersion) -> ironrdp_rdpecam::RedirectedDevice {
    let session = CameraRedirectionSession::new();
    let descriptor = DeviceDescriptor::new("Selected camera".into(), "RDCamera_Device_0".into()).unwrap();
    let device = session.select_device(descriptor).unwrap();
    let mut enumeration = session.enumeration_client(vec![device.clone()]).unwrap();
    enumeration.start(41).unwrap();
    let response = encode_vec(&EnumerationPdu::SelectVersionResponse(version)).unwrap();
    enumeration.process(41, &response).unwrap();
    device
}
