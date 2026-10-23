use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::{DvcChannelListener as _, DvcMessage, DvcProcessor as _};
use ironrdp_pdu::PduResult;
use ironrdp_rdpeusb::CHANNEL_NAME;
use ironrdp_rdpeusb::client::{
    DeviceManagerBackend, UrbdrcControlClient, UrbdrcDeviceBackend, UrbdrcDeviceClient, UrbdrcListener,
};
use ironrdp_rdpeusb::io::*;
use ironrdp_rdpeusb::pdu::caps::{Capability, RimExchangeCapabilityRequest};
use ironrdp_rdpeusb::pdu::completion::ts_urb_result::Raw;
use ironrdp_rdpeusb::pdu::header::InterfaceId;
use ironrdp_rdpeusb::pdu::iface_manipulation::InterfaceRelease;
use ironrdp_rdpeusb::pdu::notify::{ChannelCreated, Direction};
use ironrdp_rdpeusb::pdu::sink::AddVirtualChannel;
use ironrdp_rdpeusb::pdu::{
    UrbdrcClientControlPdu, UrbdrcClientDevicePdu, UrbdrcServerControlPdu, UrbdrcServerDevicePdu,
};

use super::{encode_pdu, proxy_iface_id, simple_device_info};

fn decode_control_msg(message: &DvcMessage) -> UrbdrcClientControlPdu {
    let encoded = encode_vec(message.as_ref()).expect("encode should succeed");
    decode(&encoded).expect("decode should succeed")
}

fn decode_device_msg(message: &DvcMessage) -> UrbdrcClientDevicePdu<Raw> {
    let encoded = encode_vec(message.as_ref()).expect("encode should succeed");
    decode(&encoded).expect("decode should succeed")
}

#[derive(Default)]
struct DeviceManagerState {
    control_channel: Option<u32>,
    device_channels: Vec<u32>,
    pending_devices: VecDeque<Box<dyn UrbdrcDeviceBackend>>,
}

struct TestDeviceManager {
    state: Arc<Mutex<DeviceManagerState>>,
}

impl TestDeviceManager {
    fn new(state: Arc<Mutex<DeviceManagerState>>) -> Self {
        Self { state }
    }
}

impl DeviceManagerBackend for TestDeviceManager {
    fn control_channel_assigned(&mut self, channel_id: u32) {
        let mut state = self
            .state
            .lock()
            .expect("device manager state lock should not be poisoned");
        assert!(
            state.control_channel.replace(channel_id).is_none(),
            "control channel should only be assigned once"
        );
    }

    fn take_device_for_channel(&mut self, channel_id: u32) -> Option<Box<dyn UrbdrcDeviceBackend>> {
        let mut state = self
            .state
            .lock()
            .expect("device manager state lock should not be poisoned");

        state.pending_devices.pop_front().inspect(|_| {
            state.device_channels.push(channel_id);
        })
    }
}

struct NoopDeviceClientBackend {
    device_info: DeviceInfo,
}

impl NoopDeviceClientBackend {
    fn new(device_info: DeviceInfo) -> Self {
        Self { device_info }
    }
}

impl UrbdrcDeviceBackend for NoopDeviceClientBackend {
    fn device_info(&mut self, _channel_id: u32) -> PduResult<DeviceInfo> {
        Ok(self.device_info.clone())
    }

    fn cancel_request(&mut self, _request_id: RequestId, _channel_id: u32) {}

    fn query_device_text(&mut self, _channel_id: u32, _text_type: u32, _locale_id: u32) -> PduResult<DeviceText> {
        Ok(DeviceText {
            hresult: 0,
            description: String::new(),
        })
    }

    fn io_control(
        &mut self,
        _channel_id: u32,
        _request_id: RequestId,
        _request: IoControlPacket,
    ) -> PduResult<Option<IoControlCompletionResult>> {
        Ok(None)
    }

    fn internal_io_control(
        &mut self,
        _channel_id: u32,
        _request_id: RequestId,
        _request: InternalIoControlPacket,
    ) -> PduResult<Option<IoControlCompletionResult>> {
        Ok(None)
    }

    fn transfer_in(
        &mut self,
        _channel_id: u32,
        _request_id: RequestId,
        _request: TransferInPacket,
    ) -> PduResult<Option<TransferInCompletionResult>> {
        Ok(None)
    }

    fn transfer_out(
        &mut self,
        _channel_id: u32,
        _request_id: RequestId,
        _request: TransferOutPacket,
    ) -> PduResult<Option<TransferOutCompletionResult>> {
        Ok(None)
    }

    fn transfer_out_no_ack(
        &mut self,
        _channel_id: u32,
        _request_id: RequestId,
        _request: TransferOutPacket,
    ) -> PduResult<()> {
        Ok(())
    }

    fn retract(&mut self, _channel_id: u32) -> PduResult<()> {
        Ok(())
    }
}

// Ref: [Channel Setup Sequence][1.3.1.1]
// [1.3.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/55bb34fc-7fd0-4aca-8739-5fb6759b66fc
#[test]
fn channel_setup_sequence() {
    let manager_state = Arc::new(Mutex::new(DeviceManagerState::default()));

    {
        let mut state = manager_state
            .lock()
            .expect("device manager state lock should not be poisoned");
        state
            .pending_devices
            .push_back(Box::new(NoopDeviceClientBackend::new(simple_device_info())));
        state
            .pending_devices
            .push_back(Box::new(NoopDeviceClientBackend::new(simple_device_info())));
    }

    let callback_manager_state = Arc::clone(&manager_state);

    // when channel is settled, send `ADD_VIRTUAL_CHANNEL`
    let on_capability_exchanged = Box::new(move || {
        let pending_device_count = callback_manager_state
            .lock()
            .expect("device manager state lock should not be poisoned")
            .pending_devices
            .len();
        let mut messages = Vec::with_capacity(pending_device_count);
        for _ in 0..pending_device_count {
            let message: DvcMessage = Box::new(AddVirtualChannel { msg_id: 0 });
            messages.push(message);
        }

        Ok(messages)
    });

    let manager = TestDeviceManager::new(Arc::clone(&manager_state));
    let mut listener = UrbdrcListener::new(on_capability_exchanged, Box::new(manager));

    assert_eq!(listener.channel_name(), CHANNEL_NAME);

    let mut control = listener
        .create(10)
        .expect("first URBDRC create should return control client");

    let control = &mut control
        .as_any_mut()
        .downcast_mut::<UrbdrcControlClient>()
        .expect("first processor should be a control client");

    assert!(!control.ready());

    let resp = control.start(10).expect("start should succeed");
    assert_eq!(resp.len(), 0);

    let resp = control
        .process(
            10,
            &encode_pdu(&UrbdrcServerControlPdu::Caps(RimExchangeCapabilityRequest {
                msg_id: 7,
                capability: Capability::RimCapabilityVersion01,
            })),
        )
        .expect("capability exchange should succeed");

    assert_eq!(resp.len(), 1);
    let UrbdrcClientControlPdu::Caps(response) = decode_control_msg(&resp[0]) else {
        panic!("expected capability response");
    };
    assert_eq!(response.msg_id, 7);
    assert_eq!(response.capability, Capability::RimCapabilityVersion01);
    assert_eq!(response.result, 0);

    let resp = control
        .process(
            10,
            &encode_pdu(&UrbdrcServerControlPdu::ChanCreated(ChannelCreated {
                msg_id: 8,
                direction: Direction::ToClient,
            })),
        )
        .expect("channel-created notification should succeed");
    assert_eq!(resp.len(), 1);
    let UrbdrcClientControlPdu::ChanCreated(response) = decode_control_msg(&resp[0]) else {
        panic!("expected channel-created response");
    };
    assert_eq!(response.msg_id, 8);
    assert_eq!(response.direction, Direction::ToServer);

    let resp = control
        .process(
            10,
            &encode_pdu(&UrbdrcServerControlPdu::IfaceRelease(InterfaceRelease {
                iface_id: proxy_iface_id(InterfaceId::NOTIFY_CLIENT),
                msg_id: 9,
            })),
        )
        .expect("notification release should succeed");

    // on capability exchanged message
    assert_eq!(resp.len(), 2);
    assert!(control.ready());

    for message in &resp {
        assert!(matches!(
            decode_control_msg(message),
            UrbdrcClientControlPdu::AddChan(_)
        ));
    }

    let device = listener
        .create(11)
        .expect("second URBDRC create should return device client");
    assert!(device.as_any().downcast_ref::<UrbdrcDeviceClient>().is_some());

    let device = listener
        .create(12)
        .expect("third URBDRC create should return device client");
    assert!(device.as_any().downcast_ref::<UrbdrcDeviceClient>().is_some());

    assert!(
        listener.create(13).is_none(),
        "listener should reject extra URBDRC creates when no device backend is pending"
    );

    let state = manager_state
        .lock()
        .expect("device manager state lock should not be poisoned");
    assert_eq!(state.control_channel, Some(10));
    assert_eq!(state.device_channels, [11, 12]);
    assert!(state.pending_devices.is_empty());
}

// Ref: [New Device Sequence][1.3.1.2]
// [1.3.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/7e3da218-9cdc-4ebd-bb76-e70202c7f264
#[test]
fn new_device_sequence() {
    let udev_iface = InterfaceId::try_from(4).expect("valid device interface id");
    let backend = Box::new(NoopDeviceClientBackend::new(simple_device_info()));
    let mut client = UrbdrcDeviceClient::new(udev_iface, backend).expect("device client should be created");

    assert!(!client.ready_for_io());

    let resp = client
        .process(
            99,
            &encode_pdu(&UrbdrcServerDevicePdu::ChanCreated(ChannelCreated {
                msg_id: 21,
                direction: Direction::ToClient,
            })),
        )
        .expect("channel-created notification should succeed");
    assert_eq!(resp.len(), 1);
    let UrbdrcClientDevicePdu::ChanCreated(response) = decode_device_msg(&resp[0]) else {
        panic!("expected channel-created response");
    };
    assert_eq!(response.msg_id, 21);
    assert_eq!(response.direction, Direction::ToServer);
    assert!(!client.ready_for_io());

    let resp = client
        .process(
            99,
            &encode_pdu(&UrbdrcServerDevicePdu::IfaceRelease(InterfaceRelease {
                iface_id: proxy_iface_id(InterfaceId::NOTIFY_CLIENT),
                msg_id: 22,
            })),
        )
        .expect("notification release should succeed");
    assert_eq!(resp.len(), 1);
    assert!(client.ready_for_io());

    let UrbdrcClientDevicePdu::AddDev(add_device) = decode_device_msg(&resp[0]) else {
        panic!("expected add device");
    };
    assert_eq!(add_device.usb_device, udev_iface);
}

// Ref: [Processing a Cancel Request Message][3.3.5.3.1]
// [3.3.5.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/d5315234-d9ba-42dc-bc1b-b421c57a21ae
#[test]
fn cancelled_request_stays_pending_until_the_backend_completes_it() {
    use ironrdp_rdpeusb::pdu::usb_dev::{CancelRequest, IoControl, RegisterRequestCallback};

    let udev_iface = InterfaceId::try_from(4).expect("valid device interface id");
    let completion_iface = InterfaceId::try_from(5).expect("valid completion interface id");
    let backend = Box::new(NoopDeviceClientBackend::new(simple_device_info()));
    let mut client = UrbdrcDeviceClient::new(udev_iface, backend).expect("device client should be created");

    client
        .process(
            99,
            &encode_pdu(&UrbdrcServerDevicePdu::IfaceRelease(InterfaceRelease {
                iface_id: proxy_iface_id(InterfaceId::NOTIFY_CLIENT),
                msg_id: 1,
            })),
        )
        .expect("notification release should succeed");
    client
        .process(
            99,
            &encode_pdu(&UrbdrcServerDevicePdu::RegReqCb(RegisterRequestCallback {
                msg_id: 2,
                udev_iface,
                request_completion: Some(completion_iface),
            })),
        )
        .expect("request callback registration should succeed");

    let pending = client
        .process(
            99,
            &encode_pdu(&UrbdrcServerDevicePdu::IoCtl(IoControl {
                msg_id: 3,
                udev_iface,
                ioctl_code: IoctlInternalUsb::GetPortStatus,
                input_buffer: Vec::new(),
                output_buffer_size: 4,
                req_id: 42,
            })),
        )
        .expect("IO control should be accepted");
    assert!(pending.is_empty(), "the backend leaves the request pending");

    let cancel = client
        .process(
            99,
            &encode_pdu(&UrbdrcServerDevicePdu::CancelReq(CancelRequest {
                msg_id: 4,
                udev_iface,
                req_id: 42,
            })),
        )
        .expect("cancel request should be accepted");
    assert!(cancel.is_empty());

    // The backend reports the cancellation, which still reaches the server.
    let completion = client
        .io_ctl_completion(
            42,
            IoControlCompletionResult {
                hresult: 0x8000_4004, /* E_ABORT */
                information: 0,
                output_buffer: Vec::new(),
            },
        )
        .expect("a cancelled request is still pending until completed");
    let UrbdrcClientDevicePdu::IoctlComp(completion) = decode_device_msg(&completion) else {
        panic!("expected an IO control completion");
    };
    assert_eq!(completion.request_id, 42);

    assert!(
        client
            .io_ctl_completion(
                42,
                IoControlCompletionResult {
                    hresult: 0,
                    information: 0,
                    output_buffer: Vec::new(),
                },
            )
            .is_err(),
        "a request completes only once"
    );
}

// Windows Server registers a request completion interface ID below 4, which coincides with the
// default interfaces of the control channel. It is valid: the server allocates it in its own space.
#[test]
fn request_completion_interface_may_reuse_a_default_interface_id() {
    use ironrdp_rdpeusb::pdu::usb_dev::{IoControl, RegisterRequestCallback};

    let udev_iface = InterfaceId::try_from(4).expect("valid device interface id");
    let backend = Box::new(NoopDeviceClientBackend::new(simple_device_info()));
    let mut client = UrbdrcDeviceClient::new(udev_iface, backend).expect("device client should be created");

    client
        .process(
            99,
            &encode_pdu(&UrbdrcServerDevicePdu::IfaceRelease(InterfaceRelease {
                iface_id: proxy_iface_id(InterfaceId::NOTIFY_CLIENT),
                msg_id: 1,
            })),
        )
        .expect("notification release should succeed");
    client
        .process(
            99,
            &encode_pdu(&UrbdrcServerDevicePdu::RegReqCb(RegisterRequestCallback {
                msg_id: 2,
                udev_iface,
                request_completion: Some(InterfaceId::DEVICE_SINK),
            })),
        )
        .expect("a request completion interface below 4 is accepted");

    client
        .process(
            99,
            &encode_pdu(&UrbdrcServerDevicePdu::IoCtl(IoControl {
                msg_id: 3,
                udev_iface,
                ioctl_code: IoctlInternalUsb::GetPortStatus,
                input_buffer: Vec::new(),
                output_buffer_size: 4,
                req_id: 7,
            })),
        )
        .expect("IO control should be accepted");
    let completion = client
        .io_ctl_completion(
            7,
            IoControlCompletionResult {
                hresult: 0,
                information: 0,
                output_buffer: Vec::new(),
            },
        )
        .expect("the request is pending");

    // The completion goes to the registered interface. Server-side decoding would route this
    // interface ID to the device sink, so check the header itself: InterfaceId with the proxy mask.
    let encoded = encode_vec(completion.as_ref()).expect("encode should succeed");
    let interface = u32::from_le_bytes(encoded[..4].try_into().expect("four bytes"));
    assert_eq!(interface, 0x4000_0001);
}
