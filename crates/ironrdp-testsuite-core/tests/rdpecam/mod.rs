use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::{DvcChannelListener as _, DvcProcessor as _};
use ironrdp_rdpecam::client::CameraBackend;
use ironrdp_rdpecam::pdu::{DevicePdu, EnumerationPdu};
use ironrdp_rdpecam::{
    CameraRedirectionSession, DeviceClient, DeviceDescriptor, ErrorCode, MediaFormat, MediaType, ProtocolVersion,
    StartStreamInfo, StreamDescription,
};

#[derive(Debug)]
struct FakeCamera {
    media_type: MediaType,
    activate_count: usize,
    deactivate_count: usize,
    start_count: usize,
    stop_count: usize,
    sample_count: usize,
    fail_next_start: bool,
}

impl FakeCamera {
    fn new() -> Self {
        Self {
            media_type: media_type(),
            activate_count: 0,
            deactivate_count: 0,
            start_count: 0,
            stop_count: 0,
            sample_count: 0,
            fail_next_start: false,
        }
    }
}

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

    fn start_streams(&mut self, streams: &[StartStreamInfo]) -> Result<(), ErrorCode> {
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
        Ok(())
    }

    fn stop_streams(&mut self) -> Result<(), ErrorCode> {
        self.stop_count += 1;
        Ok(())
    }

    fn sample(&mut self, stream_index: u8) -> Result<Vec<u8>, ErrorCode> {
        if stream_index != 0 {
            return Err(ErrorCode::InvalidStreamNumber);
        }
        self.sample_count += 1;
        Ok(vec![0x55; 12])
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
    let mut listener = device.listener(FakeCamera::new());
    let mut client = session.enumeration_client(vec![device]).unwrap();

    assert!(listener.create(42).is_none());
    let initial = client.start(41).unwrap();
    assert_eq!(
        decode_message::<EnumerationPdu>(&initial),
        EnumerationPdu::SelectVersionRequest(ProtocolVersion::V1)
    );
    assert!(!client.ready());

    let response = encode_vec(&EnumerationPdu::SelectVersionResponse(ProtocolVersion::V1)).unwrap();
    let advertised = client.process(41, &response).unwrap();
    assert!(client.ready());
    assert_eq!(
        decode_message::<EnumerationPdu>(&advertised),
        EnumerationPdu::DeviceAdded {
            version: ProtocolVersion::V1,
            device: descriptor,
        }
    );
    assert!(listener.create(42).is_some());
}

#[test]
fn device_state_machine_streams_only_after_activation_and_media_validation() {
    let device = negotiated_device();
    let mut client = DeviceClient::new(&device, FakeCamera::new());
    client.start(7).unwrap();

    assert_eq!(
        exchange(&mut client, 7, DevicePdu::SampleRequest(0)),
        DevicePdu::SampleErrorResponse {
            stream_index: 0,
            error: ErrorCode::NotInitialized,
        }
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::ActivateDeviceRequest),
        DevicePdu::SuccessResponse
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::ActivateDeviceRequest),
        DevicePdu::SuccessResponse
    );
    assert_eq!(client.backend().activate_count, 1);

    assert_eq!(
        exchange(&mut client, 7, DevicePdu::StreamListRequest),
        DevicePdu::StreamListResponse(vec![StreamDescription::color(true, false)])
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::MediaTypeListRequest(1)),
        DevicePdu::ErrorResponse(ErrorCode::InvalidStreamNumber)
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::StopStreamsRequest),
        DevicePdu::SuccessResponse
    );

    let start = StartStreamInfo {
        stream_index: 0,
        media_type: media_type(),
    };
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::StartStreamsRequest(vec![start])),
        DevicePdu::SuccessResponse
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::StartStreamsRequest(vec![start])),
        DevicePdu::SuccessResponse
    );
    assert!(client.is_streaming());
    client.backend_mut().fail_next_start = true;
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::StartStreamsRequest(vec![start])),
        DevicePdu::ErrorResponse(ErrorCode::Unexpected)
    );
    assert!(client.is_streaming());
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::SampleRequest(0)),
        DevicePdu::SampleResponse {
            stream_index: 0,
            sample: vec![0x55; 12],
        }
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::StopStreamsRequest),
        DevicePdu::SuccessResponse
    );
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::DeactivateDeviceRequest),
        DevicePdu::SuccessResponse
    );
    assert!(client.is_activated());
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::DeactivateDeviceRequest),
        DevicePdu::SuccessResponse
    );
    assert!(!client.is_activated());
    assert_eq!(client.backend().deactivate_count, 1);
}

#[test]
fn malformed_device_message_is_rejected_without_backend_activation() {
    let device = negotiated_device();
    let mut client = DeviceClient::new(&device, FakeCamera::new());
    client.start(7).unwrap();

    let response = client.process(7, &[1]).unwrap();
    assert_eq!(
        decode_message::<DevicePdu>(&response),
        DevicePdu::ErrorResponse(ErrorCode::InvalidMessage)
    );
    assert_eq!(client.backend().activate_count, 0);

    let response = encode_vec(&DevicePdu::SuccessResponse).unwrap();
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

    let pdu = DevicePdu::StartStreamsRequest(vec![StartStreamInfo {
        stream_index: 0,
        media_type,
    }]);
    let encoded = encode_vec(&pdu).unwrap();
    assert_eq!(decode::<DevicePdu>(&encoded).unwrap(), pdu);
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

    let mut client = DeviceClient::new(&device, FakeCamera::new());
    client.start(7).unwrap();
    assert_eq!(
        exchange(&mut client, 7, DevicePdu::ActivateDeviceRequest),
        DevicePdu::SuccessResponse
    );
    enumeration.remove_device("RDCamera_Device_0").unwrap();

    assert_eq!(
        exchange(&mut client, 7, DevicePdu::SampleRequest(0)),
        DevicePdu::SampleErrorResponse {
            stream_index: 0,
            error: ErrorCode::NotInitialized,
        }
    );
    assert_eq!(client.backend().deactivate_count, 1);
}

#[test]
fn decoder_bounds_arrays_before_allocating_and_retains_unknown_advertisement_bits() {
    let mut oversized = vec![1, 10];
    oversized.resize(2 + 256 * 5, 0);
    assert!(decode::<DevicePdu>(&oversized).is_err());

    let stream = StreamDescription {
        frame_source_types: StreamDescription::COLOR | 0x8000,
        selected: true,
        can_be_shared: false,
    };
    let pdu = DevicePdu::StreamListResponse(vec![stream]);
    let encoded = encode_vec(&pdu).unwrap();
    assert_eq!(decode::<DevicePdu>(&encoded).unwrap(), pdu);
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
        assert_eq!(encode_vec(&pdu).unwrap(), expected);
        assert_eq!(decode::<DevicePdu>(&expected).unwrap(), pdu);
    }

    assert!(decode::<DevicePdu>(&[1, 2, 4, 0, 0]).is_err());
    assert!(decode::<DevicePdu>(&[1, 7, 0]).is_err());
    assert!(decode::<DevicePdu>(&[1, 11]).is_err());
    assert!(decode::<DevicePdu>(&[1, 19, 0, 5, 0, 0]).is_err());
}

fn exchange(client: &mut DeviceClient<FakeCamera>, channel_id: u32, request: DevicePdu) -> DevicePdu {
    let encoded = encode_vec(&request).unwrap();
    let response = client.process(channel_id, &encoded).unwrap();
    decode_message(&response)
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

fn negotiated_device() -> ironrdp_rdpecam::RedirectedDevice {
    let session = CameraRedirectionSession::new();
    let descriptor = DeviceDescriptor::new("Selected camera".into(), "RDCamera_Device_0".into()).unwrap();
    let device = session.select_device(descriptor).unwrap();
    let mut enumeration = session.enumeration_client(vec![device.clone()]).unwrap();
    enumeration.start(41).unwrap();
    let response = encode_vec(&EnumerationPdu::SelectVersionResponse(ProtocolVersion::V1)).unwrap();
    enumeration.process(41, &response).unwrap();
    device
}
