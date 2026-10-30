use std::sync::{Arc, Mutex};

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::pdu::{DrdynvcClientPdu, DrdynvcDataPdu};
use ironrdp_dvc::{DvcChannelListener as _, DvcClientProcessor, DvcMessage, DvcProcessor as _};
use ironrdp_rdpepnp::client::{
    CustomEventSender, IoControlCompletion, IoUplink, PnpBackend, PnpFile, PnpInfoClient, ReadCompletion,
    UplinkMessages, WriteCompletion,
};
use ironrdp_rdpepnp::pdu::info::{DeviceAdditionPdu, DeviceDescription, PnpInfoPdu, VersionPdu};
use ironrdp_rdpepnp::pdu::io::{
    CapabilitiesReply, CapabilitiesRequest, CreateFileReply, CreateFileRequest, CustomEvent, IoCancelRequest,
    IoControlReply, IoControlRequest, ReadReply, ReadRequest, RequestId, ServerIoPdu, WriteReply, WriteRequest,
};
use ironrdp_rdpepnp::pdu::{Guid, HResult};
use ironrdp_svc::SvcMessage;
use rstest::rstest;

const PNPDR_CHANNEL_ID: u32 = 3;
const IO_CHANNEL_ID: u32 = 7;

// [MS-RDPEPNP] section 4.1 samples.
const SERVER_VERSION: [u8; 20] = [
    0x14, 0x00, 0x00, 0x00, // Size
    0x65, 0x00, 0x00, 0x00, // PacketId: IRPDR_ID_VERSION
    0x01, 0x00, 0x00, 0x00, // MajorVersion
    0x06, 0x00, 0x00, 0x00, // MinorVersion
    0x01, 0x00, 0x00, 0x00, // Capabilities
];
const CLIENT_VERSION: [u8; 20] = SERVER_VERSION;
const AUTHENTICATED_CLIENT: [u8; 8] = [
    0x08, 0x00, 0x00, 0x00, // Size
    0x67, 0x00, 0x00, 0x00, // PacketId: IRPDR_ID_SERVER_LOGON
];

// [MS-RDPEPNP] section 4.2 samples.
const DEVICE_ADDITION: [u8; 106] = [
    0x6a, 0x00, 0x00, 0x00, // Size
    0x66, 0x00, 0x00, 0x00, // PacketId: IRPDR_ID_REDIRECT_DEVICES
    0x01, 0x00, 0x00, 0x00, // DeviceCount
    0x04, 0x00, 0x00, 0x00, // ClientDeviceID
    0x56, 0x00, 0x00, 0x00, // DataSize
    0x10, 0x00, 0x00, 0x00, // cbInterfaceLength
    0x46, 0x9c, 0x4a, 0x2b, 0x8d, 0x65, 0xf2, 0x4a, 0xa9, 0x1d, 0x1e, 0x69, 0x18, 0x61, 0x70,
    0x6c, // InterfaceGUIDArray
    0x12, 0x00, 0x00, 0x00, // cbHardwareIdLength
    0x57, 0x00, 0x55, 0x00, 0x44, 0x00, 0x46, 0x00, 0x5c, 0x00, 0x4c, 0x00, 0x42, 0x00, 0x00, 0x00, 0x00,
    0x00, // HardwareId: "WUDF\LB\0\0"
    0x00, 0x00, 0x00, 0x00, // cbCompatIdLength
    0x1c, 0x00, 0x00, 0x00, // cbDeviceDescriptionLength
    0x54, 0x00, 0x73, 0x00, 0x20, 0x00, 0x46, 0x00, 0x61, 0x00, 0x6b, 0x00, 0x65, 0x00, 0x20, 0x00, 0x44, 0x00, 0x65,
    0x00, 0x76, 0x00, 0x69, 0x00, 0x63, 0x00, 0x65, 0x00, // DeviceDescription: "Ts Fake Device"
    0x04, 0x00, 0x00, 0x00, // CustomFlagLength
    0x02, 0x00, 0x00, 0x00, // CustomFlag
];
const DEVICE_REMOVAL: [u8; 12] = [
    0x0c, 0x00, 0x00, 0x00, // Size
    0x68, 0x00, 0x00, 0x00, // PacketId: IRPDR_ID_UNREDIRECT_DEVICE
    0x04, 0x00, 0x00, 0x00, // ClientDeviceID
];

// [MS-RDPEPNP] sections 4.3 and 4.4 samples.
const CAPABILITIES_REQUEST: [u8; 10] = [
    0x00, 0x00, 0x00, 0x00, // RequestId, UnusedBits
    0x05, 0x00, 0x00, 0x00, // FunctionId: CAPABILITIES_REQUEST
    0x06, 0x00, // Version
];
const CAPABILITIES_REPLY: [u8; 6] = [
    0x00, 0x00, 0x00, 0x00, // RequestId, PacketType
    0x06, 0x00, // Version
];
const CREATE_FILE_REQUEST: [u8; 28] = [
    0x00, 0x00, 0x00, 0x00, // RequestId, UnusedBits
    0x04, 0x00, 0x00, 0x00, // FunctionId: CREATE_FILE_REQUEST
    0x04, 0x00, 0x00, 0x00, // DeviceId
    0x00, 0x00, 0x00, 0xc0, // dwDesiredAccess
    0x03, 0x00, 0x00, 0x00, // dwShareMode
    0x03, 0x00, 0x00, 0x00, // dwCreationDisposition
    0x80, 0x00, 0x00, 0x40, // dwFlagsAndAttributes
];
const CREATE_FILE_REPLY: [u8; 8] = [
    0x00, 0x00, 0x00, 0x00, // RequestId, PacketType
    0x00, 0x00, 0x00, 0x00, // Result
];
const READ_REQUEST: [u8; 20] = [
    0x00, 0x00, 0x00, 0x00, // RequestId, UnusedBits
    0x00, 0x00, 0x00, 0x00, // FunctionId: READ_REQUEST
    0x08, 0x00, 0x00, 0x00, // cbBytesToRead
    0x01, 0x00, 0x00, 0x70, // OffsetHigh
    0xff, 0xff, 0xff, 0xff, // OffsetLow
];
const READ_REPLY: [u8; 21] = [
    0x00, 0x00, 0x00, 0x00, // RequestId, PacketType
    0x00, 0x00, 0x00, 0x00, // Result
    0x08, 0x00, 0x00, 0x00, // cbBytesRead
    0x2d, 0x00, 0x00, 0x00, 0x20, 0x72, 0x00, 0x00, // Data
    0x00, // UnusedByte
];
const WRITE_REQUEST: [u8; 29] = [
    0x00, 0x00, 0x00, 0x00, // RequestId, UnusedBits
    0x01, 0x00, 0x00, 0x00, // FunctionId: WRITE_REQUEST
    0x08, 0x00, 0x00, 0x00, // cbWrite
    0x00, 0x00, 0x00, 0x00, // OffsetHigh
    0x01, 0x00, 0x00, 0x00, // OffsetLow
    0x01, 0x00, 0x00, 0x00, 0x2d, 0x00, 0x00, 0x00, // Data
    0x20, // UnusedByte
];
const WRITE_REPLY: [u8; 12] = [
    0x00, 0x00, 0x00, 0x00, // RequestId, PacketType
    0x00, 0x00, 0x00, 0x00, // Result
    0x08, 0x00, 0x00, 0x00, // cbBytesWritten
];
const IOCONTROL_REQUEST: [u8; 37] = [
    0x00, 0x00, 0x00, 0x00, // RequestId, UnusedBits
    0x02, 0x00, 0x00, 0x00, // FunctionId: IOCONTROL_REQUEST
    0x40, 0x24, 0x22, 0x00, // IoCode
    0x10, 0x00, 0x00, 0x00, // cbIn
    0x08, 0x00, 0x00, 0x00, // cbOut
    0x02, 0x00, 0x00, 0x00, 0x2d, 0x00, 0x00, 0x00, 0x20, 0x72, 0x00, 0x00, 0x6c, 0x59, 0x00, 0x00, // DataIn
    0x00, // UnusedByte
];
const IOCONTROL_REPLY: [u8; 21] = READ_REPLY;
const IOCANCEL_REQUEST: [u8; 12] = [
    0xff, 0xff, 0xff, 0xff, // RequestId, UnusedBits
    0x06, 0x00, 0x00, 0x00, // FunctionId: SPECIFIC_IOCANCEL_REQUEST
    0x00, 0x00, 0x00, 0x00, // UnusedBits, idToCancel
];
const CUSTOM_EVENT: [u8; 33] = [
    0x00, 0x00, 0x00, 0x01, // RequestId, PacketType: CUSTOM_EVENT_PACKET
    0x11, 0x11, 0x11, 0x11, 0x80, 0x80, 0x5f, 0x42, 0x92, 0x2a, 0xda, 0xbf, 0x3d, 0xe3, 0xf6,
    0x9a, // CustomEventGUID
    0x08, 0x00, 0x00, 0x00, // cbData
    0x20, 0x4c, 0x0f, 0x00, 0xc4, 0x00, 0x0f, 0x00, // Data
    0x00, // UnusedByte
];

const READ_DATA: [u8; 8] = [0x2d, 0x00, 0x00, 0x00, 0x20, 0x72, 0x00, 0x00];

fn request_id(value: u32) -> RequestId {
    RequestId::new(value).expect("24-bit request ID")
}

/// The device of the [MS-RDPEPNP] section 4.2 sample.
fn sample_device() -> DeviceDescription {
    DeviceDescription {
        client_device_id: 4,
        interface_guids: vec![Guid::new(
            0x2b4a_9c46,
            0x658d,
            0x4af2,
            [0xa9, 0x1d, 0x1e, 0x69, 0x18, 0x61, 0x70, 0x6c],
        )],
        hardware_ids: vec![r"WUDF\LB".to_owned()],
        compatibility_ids: Vec::new(),
        description: "Ts Fake Device".to_owned(),
        custom_flag: 2,
        container_id: None,
        device_caps: None,
    }
}

fn encode_messages(messages: &[DvcMessage]) -> Vec<Vec<u8>> {
    messages
        .iter()
        .map(|message| encode_vec(message.as_ref()).expect("encode DVC message"))
        .collect()
}

fn svc_payloads(messages: &[SvcMessage], channel_id: u32) -> Vec<Vec<u8>> {
    messages
        .iter()
        .map(|message| {
            let encoded = message.encode_unframed_pdu().expect("encode DRDYNVC PDU");
            let DrdynvcClientPdu::Data(pdu) = decode(&encoded).expect("decode DRDYNVC PDU") else {
                panic!("expected DRDYNVC data");
            };
            assert_eq!(pdu.channel_id(), channel_id);
            let DrdynvcDataPdu::Data(data) = pdu else {
                panic!("expected unsplit DRDYNVC data");
            };
            data.data().to_vec()
        })
        .collect()
}

fn ready_info_client(devices: Vec<DeviceDescription>) -> PnpInfoClient {
    let mut client = PnpInfoClient::new(devices).expect("valid devices");
    client.start(PNPDR_CHANNEL_ID).expect("start PNPDR");
    client
        .process(PNPDR_CHANNEL_ID, &SERVER_VERSION)
        .expect("server version");
    client
        .process(PNPDR_CHANNEL_ID, &AUTHENTICATED_CLIENT)
        .expect("authenticated client");
    client
}

/// What the test backend observed, and the completions it holds back.
#[derive(Default)]
struct Observed {
    created: Vec<CreateFileRequest>,
    cancelled: Vec<RequestId>,
    events: Option<CustomEventSender>,
    parked_reads: Vec<ReadCompletion>,
    parked_writes: Vec<WriteCompletion>,
    closed: bool,
}

type SharedObserved = Arc<Mutex<Observed>>;

/// Opens every device; see [`TestFile`] for how requests complete.
struct TestBackend(SharedObserved);

impl PnpBackend for TestBackend {
    fn create_file(
        &mut self,
        request: &CreateFileRequest,
        events: CustomEventSender,
    ) -> Result<Box<dyn PnpFile>, HResult> {
        let mut observed = self.0.lock().unwrap();
        observed.created.push(*request);
        observed.events = Some(events);
        Ok(Box::new(TestFile(Arc::clone(&self.0))))
    }
}

/// Completes 8-byte reads at once with [`READ_DATA`] and parks other reads and every write.
/// IOControl requests complete at once with [`READ_DATA`].
struct TestFile(SharedObserved);

impl PnpFile for TestFile {
    fn read(&mut self, request: ReadRequest, completion: ReadCompletion) {
        if request.bytes_to_read == 8 {
            completion.complete(HResult::S_OK, READ_DATA.to_vec());
        } else {
            self.0.lock().unwrap().parked_reads.push(completion);
        }
    }

    fn write(&mut self, _request: WriteRequest, completion: WriteCompletion) {
        self.0.lock().unwrap().parked_writes.push(completion);
    }

    fn io_control(&mut self, _request: IoControlRequest, completion: IoControlCompletion) {
        completion.complete(HResult::S_OK, READ_DATA.to_vec());
    }

    fn cancel(&mut self, request_id: RequestId) {
        self.0.lock().unwrap().cancelled.push(request_id);
    }
}

impl Drop for TestFile {
    fn drop(&mut self) {
        // Parked completions are released by the test, not here, to observe their replies.
        if let Ok(mut observed) = self.0.lock() {
            observed.closed = true;
        }
    }
}

struct IoChannel {
    processor: Box<dyn DvcClientProcessor>,
    observed: SharedObserved,
    uplinked: Arc<Mutex<Vec<UplinkMessages>>>,
}

impl IoChannel {
    /// Opens a `FileRedirectorChannel` instance for the devices that `info` announced.
    fn open(info: &PnpInfoClient) -> Self {
        let observed = SharedObserved::default();
        let uplinked = Arc::new(Mutex::new(Vec::new()));
        let uplink: IoUplink = {
            let uplinked = Arc::clone(&uplinked);
            Arc::new(move |messages: UplinkMessages| {
                assert_eq!(messages.channel_id(), IO_CHANNEL_ID);
                uplinked.lock().unwrap().push(messages);
                Ok(())
            })
        };
        let mut listener = info.file_redirector_listener(Box::new(TestBackend(Arc::clone(&observed))), uplink);
        let mut processor = listener.create(IO_CHANNEL_ID).expect("listener creates a processor");
        assert!(processor.start(IO_CHANNEL_ID).expect("start").is_empty());
        Self {
            processor,
            observed,
            uplinked,
        }
    }

    /// Opens a channel and runs the capabilities and CreateFile exchanges of the section 4.3 and 4.4 samples.
    fn open_file(info: &PnpInfoClient) -> Self {
        let mut channel = Self::open(info);
        channel.process(&CAPABILITIES_REQUEST);
        assert_eq!(channel.process(&CREATE_FILE_REQUEST), [CREATE_FILE_REPLY.to_vec()]);
        channel
    }

    fn process(&mut self, payload: &[u8]) -> Vec<Vec<u8>> {
        let messages = self.processor.process(IO_CHANNEL_ID, payload).expect("process");
        encode_messages(&messages)
    }

    /// Takes the uplinked replies as the session loop would write them now.
    fn take_uplinked(&self) -> Vec<Vec<u8>> {
        let queued = core::mem::take(&mut *self.uplinked.lock().unwrap());
        let messages: Vec<SvcMessage> = queued
            .into_iter()
            .filter_map(UplinkMessages::take_if_open)
            .flatten()
            .collect();
        svc_payloads(&messages, IO_CHANNEL_ID)
    }
}

fn read_request(request_id: u32, bytes_to_read: u32) -> Vec<u8> {
    encode_vec(&ServerIoPdu::Read(ReadRequest {
        request_id: self::request_id(request_id),
        bytes_to_read,
        offset: 0,
    }))
    .expect("encode read request")
}

// oracle: spec MS-RDPEPNP §4.1
#[test]
fn server_version_decodes_as_in_spec_sample() {
    let pdu: PnpInfoPdu = decode(&SERVER_VERSION).expect("decode server version");
    assert_eq!(
        pdu,
        PnpInfoPdu::Version(VersionPdu {
            major_version: 1,
            minor_version: 6,
            capabilities: 1,
        })
    );
    assert_eq!(
        decode::<PnpInfoPdu>(&AUTHENTICATED_CLIENT).unwrap(),
        PnpInfoPdu::ServerLogon
    );
}

// oracle: spec MS-RDPEPNP §4.2
#[test]
fn device_addition_matches_spec_sample() {
    let pdu = PnpInfoPdu::RedirectDevices(DeviceAdditionPdu {
        devices: vec![sample_device()],
    });
    assert_eq!(encode_vec(&pdu).unwrap(), DEVICE_ADDITION);
    assert_eq!(decode::<PnpInfoPdu>(&DEVICE_ADDITION).unwrap(), pdu);
}

// oracle: spec MS-RDPEPNP §2.2.1.3.1.1 (DataSize counts the fields after itself, hand-derived)
#[test]
fn container_id_and_device_caps_extend_the_description() {
    let mut device = sample_device();
    device.container_id = Some(Guid::new(1, 2, 3, [4; 8]));
    device.device_caps = Some(0x0000_000c);
    let encoded = encode_vec(&device).unwrap();

    let tail: Vec<u8> = [
        &[0x10, 0x00, 0x00, 0x00][..],                     // cbContainerId
        &[1, 0, 0, 0, 2, 0, 3, 0, 4, 4, 4, 4, 4, 4, 4, 4], // ContainerId
        &[0x04, 0x00, 0x00, 0x00],                         // cbDeviceCaps
        &[0x0c, 0x00, 0x00, 0x00],                         // DeviceCaps
    ]
    .concat();
    assert_eq!(encoded[4..8], (0x56u32 + 28).to_le_bytes()); // DataSize
    assert!(encoded.ends_with(&tail));
    assert_eq!(decode::<DeviceDescription>(&encoded).unwrap(), device);
}

// oracle: spec MS-RDPEPNP §2.2.1.3.1.1 (DeviceCaps follows ContainerId)
#[test]
fn device_caps_without_container_id_is_rejected() {
    let mut device = sample_device();
    device.device_caps = Some(0x0000_0004);
    assert!(PnpInfoClient::new(vec![device]).is_err());
}

// oracle: spec MS-RDPEPNP §3.1.1 (unique client device IDs)
#[test]
fn duplicate_client_device_ids_are_rejected() {
    assert!(PnpInfoClient::new(vec![sample_device(), sample_device()]).is_err());
}

// oracle: spec MS-RDPEPNP §4.1
#[test]
fn client_answers_server_version_with_its_version() {
    let mut client = PnpInfoClient::new(vec![sample_device()]).unwrap();
    client.start(PNPDR_CHANNEL_ID).unwrap();

    let replies = encode_messages(&client.process(PNPDR_CHANNEL_ID, &SERVER_VERSION).unwrap());

    assert_eq!(replies, [CLIENT_VERSION.to_vec()]);
    assert!(!client.is_ready());
}

// oracle: spec MS-RDPEPNP §3.2.5.1.1.3, §4.2
#[test]
fn devices_are_announced_after_the_authenticated_client_message() {
    let mut client = PnpInfoClient::new(vec![sample_device()]).unwrap();
    client.start(PNPDR_CHANNEL_ID).unwrap();
    client.process(PNPDR_CHANNEL_ID, &SERVER_VERSION).unwrap();

    let replies = encode_messages(&client.process(PNPDR_CHANNEL_ID, &AUTHENTICATED_CLIENT).unwrap());

    assert_eq!(replies, [DEVICE_ADDITION.to_vec()]);
    assert!(client.is_ready());
}

// oracle: spec MS-RDPEPNP §3.2.5.1.2.1
#[test]
fn device_added_before_authentication_waits_for_it() {
    let mut client = PnpInfoClient::new(Vec::new()).unwrap();
    client.start(PNPDR_CHANNEL_ID).unwrap();
    client.process(PNPDR_CHANNEL_ID, &SERVER_VERSION).unwrap();

    assert!(client.add_device(sample_device()).unwrap().is_empty());

    let replies = encode_messages(&client.process(PNPDR_CHANNEL_ID, &AUTHENTICATED_CLIENT).unwrap());
    assert_eq!(replies, [DEVICE_ADDITION.to_vec()]);
}

// oracle: spec MS-RDPEPNP §4.2
#[test]
fn device_added_and_removed_while_ready_is_announced_at_once() {
    let mut client = ready_info_client(Vec::new());

    let added = client.add_device(sample_device()).unwrap();
    assert_eq!(svc_payloads(&added, PNPDR_CHANNEL_ID), [DEVICE_ADDITION.to_vec()]);

    let removed = client.remove_device(4).unwrap();
    assert_eq!(svc_payloads(&removed, PNPDR_CHANNEL_ID), [DEVICE_REMOVAL.to_vec()]);
}

// oracle: spec MS-RDPEPNP §4.3, §4.4
#[rstest]
#[case::capabilities(&CAPABILITIES_REQUEST, ServerIoPdu::Capabilities(CapabilitiesRequest {
    request_id: request_id(0),
    version: 6,
}))]
#[case::create_file(&CREATE_FILE_REQUEST, ServerIoPdu::CreateFile(CreateFileRequest {
    request_id: request_id(0),
    device_id: 4,
    desired_access: 0xc000_0000,
    share_mode: 3,
    creation_disposition: 3,
    flags_and_attributes: 0x4000_0080,
}))]
#[case::read(&READ_REQUEST, ServerIoPdu::Read(ReadRequest {
    request_id: request_id(0),
    bytes_to_read: 8,
    offset: 0x7000_0001_ffff_ffff,
}))]
#[case::write(&WRITE_REQUEST, ServerIoPdu::Write(WriteRequest {
    request_id: request_id(0),
    offset: 1,
    data: vec![0x01, 0x00, 0x00, 0x00, 0x2d, 0x00, 0x00, 0x00],
}))]
#[case::io_control(&IOCONTROL_REQUEST, ServerIoPdu::IoControl(IoControlRequest {
    request_id: request_id(0),
    io_code: 0x0022_2440,
    output_size: 8,
    input: vec![0x02, 0x00, 0x00, 0x00, 0x2d, 0x00, 0x00, 0x00, 0x20, 0x72, 0x00, 0x00, 0x6c, 0x59, 0x00, 0x00],
    output: Vec::new(),
}))]
#[case::io_cancel(&IOCANCEL_REQUEST, ServerIoPdu::SpecificIoCancel(IoCancelRequest {
    request_id: request_id(0x00ff_ffff),
    id_to_cancel: request_id(0),
}))]
fn server_requests_decode_as_in_spec_samples(#[case] bytes: &[u8], #[case] expected: ServerIoPdu) {
    assert_eq!(decode::<ServerIoPdu>(bytes).unwrap(), expected);
}

// oracle: spec MS-RDPEPNP §4.3, §4.4
#[rstest]
#[case::capabilities(encode_vec(&CapabilitiesReply { request_id: request_id(0), version: 6 }), &CAPABILITIES_REPLY)]
#[case::create_file(encode_vec(&CreateFileReply { request_id: request_id(0), result: HResult::S_OK }), &CREATE_FILE_REPLY)]
#[case::read(encode_vec(&ReadReply { request_id: request_id(0), result: HResult::S_OK, data: READ_DATA.to_vec() }), &READ_REPLY)]
#[case::write(encode_vec(&WriteReply { request_id: request_id(0), result: HResult::S_OK, bytes_written: 8 }), &WRITE_REPLY)]
#[case::io_control(encode_vec(&IoControlReply { request_id: request_id(0), result: HResult::S_OK, data: READ_DATA.to_vec() }), &IOCONTROL_REPLY)]
#[case::custom_event(encode_vec(&CustomEvent {
    event_guid: Guid::new(0x1111_1111, 0x8080, 0x425f, [0x92, 0x2a, 0xda, 0xbf, 0x3d, 0xe3, 0xf6, 0x9a]),
    data: vec![0x20, 0x4c, 0x0f, 0x00, 0xc4, 0x00, 0x0f, 0x00],
}), &CUSTOM_EVENT)]
fn client_replies_encode_as_in_spec_samples(
    #[case] encoded: ironrdp_core::EncodeResult<Vec<u8>>,
    #[case] expected: &[u8],
) {
    assert_eq!(encoded.unwrap(), expected);
}

// oracle: spec MS-ERREF §2.1.2 (HRESULT_FROM_WIN32)
#[rstest]
#[case::success(0, 0)]
#[case::file_not_found(0x0000_0002, 0x8007_0002)]
#[case::insufficient_buffer(0x0000_007A, 0x8007_007A)]
#[case::already_an_hresult(0x8000_4005, 0x8000_4005)]
fn win32_codes_map_to_hresults(#[case] code: u32, #[case] expected: u32) {
    assert_eq!(HResult::from_win32(code), HResult(expected));
}

// oracle: spec MS-RDPEPNP §4.3
#[test]
fn io_channel_answers_capabilities_with_its_version() {
    let info = ready_info_client(vec![sample_device()]);
    let mut channel = IoChannel::open(&info);

    assert_eq!(channel.process(&CAPABILITIES_REQUEST), [CAPABILITIES_REPLY.to_vec()]);
}

// oracle: spec MS-RDPEPNP §3.2.5.2.2.1, §4.4
#[test]
fn create_file_opens_the_announced_device() {
    let info = ready_info_client(vec![sample_device()]);
    let channel = IoChannel::open_file(&info);

    let created = &channel.observed.lock().unwrap().created;
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].device_id, 4);
    assert_eq!(created[0].desired_access, 0xc000_0000);
}

// oracle: spec MS-RDPEPNP §3.2.5.2.2.1 (the device ID identifies an announced device)
#[test]
fn create_file_for_an_unannounced_device_fails() {
    let info = ready_info_client(Vec::new());
    let mut channel = IoChannel::open(&info);
    channel.process(&CAPABILITIES_REQUEST);

    let replies = channel.process(&CREATE_FILE_REQUEST);

    let reply: CreateFileReply = decode(&replies[0]).unwrap();
    assert!(!reply.result.is_success());
    assert!(channel.observed.lock().unwrap().created.is_empty());
}

// oracle: spec MS-RDPEPNP §4.4
#[test]
fn read_completed_by_the_backend_is_replied_to() {
    let info = ready_info_client(vec![sample_device()]);
    let mut channel = IoChannel::open_file(&info);

    assert_eq!(channel.process(&READ_REQUEST), [READ_REPLY.to_vec()]);
}

// oracle: spec MS-RDPEPNP §4.4
#[test]
fn io_control_completed_by_the_backend_is_replied_to() {
    let info = ready_info_client(vec![sample_device()]);
    let mut channel = IoChannel::open_file(&info);

    assert_eq!(channel.process(&IOCONTROL_REQUEST), [IOCONTROL_REPLY.to_vec()]);
}

// oracle: spec MS-RDPEPNP §4.4
#[test]
fn write_completed_later_is_sent_through_the_uplink() {
    let info = ready_info_client(vec![sample_device()]);
    let mut channel = IoChannel::open_file(&info);

    assert!(channel.process(&WRITE_REQUEST).is_empty());
    let completion = channel.observed.lock().unwrap().parked_writes.pop().unwrap();
    completion.complete(HResult::S_OK, 8);

    assert_eq!(channel.take_uplinked(), [WRITE_REPLY.to_vec()]);
}

// oracle: spec MS-RDPEPNP §3.2.5.2.2.9 (every request still gets its reply)
#[test]
fn dropped_completion_replies_with_an_error() {
    let info = ready_info_client(vec![sample_device()]);
    let mut channel = IoChannel::open_file(&info);
    channel.process(&WRITE_REQUEST);

    drop(channel.observed.lock().unwrap().parked_writes.pop());

    let replies = channel.take_uplinked();
    let reply: WriteReply = decode(&replies[0]).unwrap();
    assert_eq!(reply.request_id, request_id(0));
    assert!(!reply.result.is_success());
}

// oracle: spec MS-RDPEPNP §2.2.2.3.8 (cbBytesReadReturned does not exceed cbOut)
#[test]
fn io_control_output_larger_than_cb_out_is_not_sent() {
    let info = ready_info_client(vec![sample_device()]);
    let mut channel = IoChannel::open_file(&info);
    let request = ServerIoPdu::IoControl(IoControlRequest {
        request_id: request_id(1),
        io_code: 0x0022_2440,
        output_size: 4,
        input: Vec::new(),
        output: Vec::new(),
    });

    let replies = channel.process(&encode_vec(&request).unwrap());

    let reply: IoControlReply = decode(&replies[0]).unwrap();
    assert!(!reply.result.is_success());
    assert!(reply.data.len() <= 4);
}

// oracle: spec MS-RDPEPNP §3.2.5.2.2.7 (ERROR_INSUFFICIENT_BUFFER), spec MS-ERREF §2.1.2 (HRESULT_FROM_WIN32 of 0x7A)
#[test]
fn io_control_with_a_mismatched_data_out_fails_with_insufficient_buffer() {
    let info = ready_info_client(vec![sample_device()]);
    let mut channel = IoChannel::open_file(&info);
    let request = [
        0x01, 0x00, 0x00, 0x00, // RequestId, UnusedBits
        0x02, 0x00, 0x00, 0x00, // FunctionId: IOCONTROL_REQUEST
        0x40, 0x24, 0x22, 0x00, // IoCode
        0x00, 0x00, 0x00, 0x00, // cbIn
        0x08, 0x00, 0x00, 0x00, // cbOut
        0xaa, 0xbb, 0xcc, 0xdd, // DataOut, 4 bytes instead of 8
        0x00, // UnusedByte
    ];

    let replies = channel.process(&request);

    let reply: IoControlReply = decode(&replies[0]).unwrap();
    assert_eq!(reply.request_id, request_id(1));
    assert_eq!(reply.result, HResult(0x8007_007a));
}

// oracle: spec MS-RDPEPNP §3.2.5.2.2.9
#[test]
fn cancel_is_forwarded_only_while_the_request_is_pending() {
    let info = ready_info_client(vec![sample_device()]);
    let mut channel = IoChannel::open_file(&info);
    channel.process(&read_request(0, 16));

    channel.process(&IOCANCEL_REQUEST);
    let completion = channel.observed.lock().unwrap().parked_reads.pop().unwrap();
    completion.complete(HResult::E_ABORT, Vec::new());
    channel.process(&IOCANCEL_REQUEST);

    assert_eq!(channel.observed.lock().unwrap().cancelled, [request_id(0)]);
    assert_eq!(channel.take_uplinked().len(), 1);
}

// oracle: spec MS-RDPEPNP §3.1.1 (a duplicate request ID terminates the channel)
#[test]
fn duplicate_request_id_disables_the_channel() {
    let info = ready_info_client(vec![sample_device()]);
    let mut channel = IoChannel::open_file(&info);
    channel.process(&read_request(5, 16));

    assert!(channel.process(&read_request(5, 16)).is_empty());
    assert!(channel.observed.lock().unwrap().closed);
    let completion = channel.observed.lock().unwrap().parked_reads.pop().unwrap();
    completion.complete(HResult::S_OK, Vec::new());
    assert!(channel.take_uplinked().is_empty());
    assert!(channel.process(&READ_REQUEST).is_empty());
}

// oracle: spec MS-RDPEPNP §2.2.2.1.1 (an unknown FunctionId terminates the channel)
#[test]
fn unknown_function_id_disables_the_channel() {
    let info = ready_info_client(vec![sample_device()]);
    let mut channel = IoChannel::open_file(&info);
    let unknown = [
        0x01, 0x00, 0x00, 0x00, // RequestId, UnusedBits
        0x03, 0x00, 0x00, 0x00, // FunctionId: unassigned
    ];

    assert!(channel.process(&unknown).is_empty());

    assert!(channel.observed.lock().unwrap().closed);
    assert!(channel.process(&READ_REQUEST).is_empty());
}

// oracle: spec MS-RDPEPNP §3.1.1 (closing the channel invalidates its requests), §3.2.5.2.2.2
#[test]
fn closing_the_channel_closes_the_file_and_discards_replies() {
    let info = ready_info_client(vec![sample_device()]);
    let mut channel = IoChannel::open_file(&info);
    channel.process(&WRITE_REQUEST);

    channel.processor.close(IO_CHANNEL_ID);

    assert!(channel.observed.lock().unwrap().closed);
    let completion = channel.observed.lock().unwrap().parked_writes.pop().unwrap();
    completion.complete(HResult::S_OK, 8);
    assert!(channel.take_uplinked().is_empty());
}

// oracle: spec MS-RDPEPNP §3.1.1 (closing the channel invalidates its requests)
#[test]
fn reply_queued_before_the_channel_closes_is_not_written() {
    let info = ready_info_client(vec![sample_device()]);
    let mut channel = IoChannel::open_file(&info);
    channel.process(&WRITE_REQUEST);
    let completion = channel.observed.lock().unwrap().parked_writes.pop().unwrap();
    completion.complete(HResult::S_OK, 8);
    assert_eq!(channel.uplinked.lock().unwrap().len(), 1);

    channel.processor.close(IO_CHANNEL_ID);

    assert!(channel.take_uplinked().is_empty());
}

// oracle: spec MS-RDPEPNP §3.2.5.2.2.10, §4.4
#[test]
fn custom_events_are_sent_when_the_server_supports_them() {
    let info = ready_info_client(vec![sample_device()]);
    let channel = IoChannel::open_file(&info);
    let events = channel.observed.lock().unwrap().events.clone().unwrap();
    let guid = Guid::new(
        0x1111_1111,
        0x8080,
        0x425f,
        [0x92, 0x2a, 0xda, 0xbf, 0x3d, 0xe3, 0xf6, 0x9a],
    );

    events
        .send(guid, vec![0x20, 0x4c, 0x0f, 0x00, 0xc4, 0x00, 0x0f, 0x00])
        .unwrap();

    assert_eq!(channel.take_uplinked(), [CUSTOM_EVENT.to_vec()]);
}

// oracle: spec MS-RDPEPNP §3.2.5.2.1.1 (no custom events below version 6)
#[test]
fn custom_events_are_refused_by_a_version_4_server() {
    let info = ready_info_client(vec![sample_device()]);
    let mut channel = IoChannel::open(&info);
    let mut capabilities = CAPABILITIES_REQUEST;
    capabilities[8] = 0x04; // Version
    channel.process(&capabilities);
    channel.process(&CREATE_FILE_REQUEST);
    let events = channel.observed.lock().unwrap().events.clone().unwrap();

    assert!(events.send(Guid::new(0, 0, 0, [0; 8]), Vec::new()).is_err());
    assert!(channel.take_uplinked().is_empty());
}
