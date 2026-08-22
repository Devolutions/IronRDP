use std::sync::mpsc::{self, Sender, TryRecvError};

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::{DvcMessage, DvcProcessor as _};
use ironrdp_pdu::PduResult;
use ironrdp_rdpeusb::CHANNEL_NAME;
use ironrdp_rdpeusb::io::device::add_device_from_info;
use ironrdp_rdpeusb::io::{
    DeviceAnnounce, DeviceText, IoControlCompletionResult, RequestId, TransferInCompletionResult,
    TransferOutCompletionResult,
};
use ironrdp_rdpeusb::pdu::caps::{Capability, RimExchangeCapabilityResponse};
use ironrdp_rdpeusb::pdu::header::InterfaceId;
use ironrdp_rdpeusb::pdu::notify::{ChannelCreated, Direction};
use ironrdp_rdpeusb::pdu::sink::AddVirtualChannel;
use ironrdp_rdpeusb::pdu::{
    UrbdrcClientControlPdu, UrbdrcClientDevicePdu, UrbdrcServerControlPdu, UrbdrcServerDevicePdu,
};
use ironrdp_rdpeusb::server::{
    UrbdrcControlServer, UrbdrcControlServerBackend, UrbdrcDeviceServer, UrbdrcDeviceServerBackend,
};

use super::{encode_pdu, proxy_iface_id, simple_device_info};

fn decode_control_msg(message: &DvcMessage) -> UrbdrcServerControlPdu {
    let encoded = encode_vec(message.as_ref()).expect("encode should succeed");
    decode(&encoded).expect("decode should succeed")
}

fn decode_device_msg(message: &DvcMessage) -> UrbdrcServerDevicePdu {
    let encoded = encode_vec(message.as_ref()).expect("encode should succeed");
    decode(&encoded).expect("decode should succeed")
}

struct TestControlBackend {
    device_channel_created: Sender<()>,
}

impl UrbdrcControlServerBackend for TestControlBackend {
    fn create_device_chan(&mut self) -> PduResult<()> {
        self.device_channel_created
            .send(())
            .expect("device channel receiver should remain connected");
        Ok(())
    }
}

struct TestDeviceBackend {
    device_announced: Sender<DeviceAnnounce>,
}

impl UrbdrcDeviceServerBackend for TestDeviceBackend {
    fn add_device(&mut self, device: DeviceAnnounce) -> PduResult<()> {
        self.device_announced
            .send(device)
            .expect("device announcement receiver should remain connected");
        Ok(())
    }

    fn device_text(&mut self, _device_text: DeviceText) {}

    fn io_control_completed(
        &mut self,
        _channel_id: u32,
        _request_id: RequestId,
        _completion: IoControlCompletionResult,
    ) -> PduResult<()> {
        Ok(())
    }

    fn internal_io_control_completed(
        &mut self,
        _channel_id: u32,
        _request_id: RequestId,
        _completion: IoControlCompletionResult,
    ) -> PduResult<()> {
        Ok(())
    }

    fn transfer_in_completed(
        &mut self,
        _channel_id: u32,
        _request_id: RequestId,
        _completion: TransferInCompletionResult,
    ) -> PduResult<()> {
        Ok(())
    }

    fn transfer_out_completed(
        &mut self,
        _channel_id: u32,
        _request_id: RequestId,
        _completion: TransferOutCompletionResult,
    ) -> PduResult<()> {
        Ok(())
    }
}

// Ref: [Channel Setup Sequence][1.3.1.1]
// [1.3.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/55bb34fc-7fd0-4aca-8739-5fb6759b66fc
#[test]
fn capability_exchange_sequence() {
    let (device_channel_created, channel_created_rx) = mpsc::channel();
    let backend = Box::new(TestControlBackend { device_channel_created });
    let mut server = UrbdrcControlServer::new(backend);

    assert_eq!(server.channel_name(), CHANNEL_NAME);

    let resp = server.start(10).expect("start should succeed");
    assert_eq!(resp.len(), 1);
    let UrbdrcServerControlPdu::Caps(request) = decode_control_msg(&resp[0]) else {
        panic!("expected capability request");
    };
    assert_eq!(request.capability, Capability::RimCapabilityVersion01);

    let resp = server
        .process(
            10,
            &encode_pdu(&UrbdrcClientControlPdu::Caps(RimExchangeCapabilityResponse {
                msg_id: request.msg_id,
                capability: Capability::RimCapabilityVersion01,
                result: 0,
            })),
        )
        .expect("capability response should succeed");
    assert_eq!(resp.len(), 2);

    let UrbdrcServerControlPdu::IfaceRelease(release) = decode_control_msg(&resp[0]) else {
        panic!("expected capabilities interface release");
    };
    assert_eq!(release.iface_id, u32::from(InterfaceId::CAPABILITIES));

    let UrbdrcServerControlPdu::ChanCreated(channel_created_request) = decode_control_msg(&resp[1]) else {
        panic!("expected channel-created request");
    };
    assert_eq!(channel_created_request.direction, Direction::ToClient);

    let resp = server
        .process(
            10,
            &encode_pdu(&UrbdrcClientControlPdu::ChanCreated(ChannelCreated {
                msg_id: channel_created_request.msg_id,
                direction: Direction::ToServer,
            })),
        )
        .expect("channel-created response should succeed");
    assert_eq!(resp.len(), 1);

    let UrbdrcServerControlPdu::IfaceRelease(release) = decode_control_msg(&resp[0]) else {
        panic!("expected notification interface release");
    };
    assert_eq!(release.iface_id, proxy_iface_id(InterfaceId::NOTIFY_CLIENT));

    let resp = server
        .process(
            10,
            &encode_pdu(&UrbdrcClientControlPdu::AddChan(AddVirtualChannel { msg_id: 0 })),
        )
        .expect("add virtual channel should succeed");
    assert!(resp.is_empty());
    channel_created_rx.try_recv().expect("backend should be notified");
    assert!(
        matches!(channel_created_rx.try_recv(), Err(TryRecvError::Empty)),
        "backend should be notified exactly once"
    );
}

// Ref: [New Device Sequence][1.3.1.2]
// [1.3.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/7e3da218-9cdc-4ebd-bb76-e70202c7f264
#[test]
fn new_device_sequence() {
    let udev_iface = InterfaceId::try_from(4).expect("valid device interface id");
    let completion_iface = InterfaceId::try_from(5).expect("valid completion interface id");
    let (device_announced, announcement_rx) = mpsc::channel();
    let backend = Box::new(TestDeviceBackend { device_announced });
    let mut server = UrbdrcDeviceServer::new(backend, completion_iface).expect("device server should be created");

    assert_eq!(server.channel_name(), CHANNEL_NAME);

    let resp = server.start(11).expect("start should succeed");
    assert_eq!(resp.len(), 1);
    let UrbdrcServerDevicePdu::ChanCreated(channel_created_request) = decode_device_msg(&resp[0]) else {
        panic!("expected channel-created request");
    };
    assert_eq!(channel_created_request.direction, Direction::ToClient);

    let resp = server
        .process(
            11,
            &encode_pdu(&UrbdrcClientDevicePdu::ChanCreated(ChannelCreated {
                msg_id: channel_created_request.msg_id,
                direction: Direction::ToServer,
            })),
        )
        .expect("channel-created response should succeed");
    assert_eq!(resp.len(), 1);
    let UrbdrcServerDevicePdu::IfaceRelease(release) = decode_device_msg(&resp[0]) else {
        panic!("expected notification interface release");
    };
    assert_eq!(release.iface_id, proxy_iface_id(InterfaceId::NOTIFY_CLIENT));

    let add_device = add_device_from_info(udev_iface, &simple_device_info()).expect("ADD_DEVICE should be generated");
    let resp = server
        .process(11, &encode_pdu(&UrbdrcClientDevicePdu::AddDev(add_device)))
        .expect("add device should succeed");
    assert_eq!(resp.len(), 2);

    let UrbdrcServerDevicePdu::IfaceRelease(release) = decode_device_msg(&resp[0]) else {
        panic!("expected device sink interface release");
    };
    assert_eq!(release.iface_id, proxy_iface_id(InterfaceId::DEVICE_SINK));

    let UrbdrcServerDevicePdu::RegReqCb(register) = decode_device_msg(&resp[1]) else {
        panic!("expected request callback registration");
    };
    assert_eq!(register.udev_iface, udev_iface);
    assert_eq!(register.request_completion, Some(completion_iface));

    announcement_rx
        .try_recv()
        .expect("backend should receive device announcement");
    assert!(
        matches!(announcement_rx.try_recv(), Err(TryRecvError::Empty)),
        "device should be announced exactly once"
    );
}
