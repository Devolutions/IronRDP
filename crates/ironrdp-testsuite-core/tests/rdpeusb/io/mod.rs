use std::sync::mpsc::{self, Receiver, Sender};

use ironrdp_core::encode_vec;
use ironrdp_dvc::{DvcMessage, DvcProcessor as _};
use ironrdp_pdu::PduResult;
use ironrdp_rdpeusb::client::{UrbdrcDeviceBackend, UrbdrcDeviceClient};
use ironrdp_rdpeusb::io::{
    DeviceAnnounce, DeviceText, InternalIoControlPacket, IoControlCompletionResult, IoControlPacket, RequestId,
    TransferInCompletionResult, TransferInPacket, TransferOutCompletionResult, TransferOutPacket,
};
use ironrdp_rdpeusb::pdu::header::InterfaceId;
use ironrdp_rdpeusb::server::{UrbdrcDeviceServer, UrbdrcDeviceServerBackend};

use super::simple_device_info;

const CHANNEL_ID: u32 = 11;
const DEVICE_TEXT_DESCRIPTION: &str = "Test USB device";
const DEVICE_TEXT_HRESULT: u32 = 0;

#[derive(Debug)]
enum ClientEvent {
    QueryDeviceText {
        channel_id: u32,
        text_type: u32,
        locale_id: u32,
    },
    IoControl {
        channel_id: u32,
        request_id: RequestId,
        request: IoControlPacket,
    },
    InternalIoControl {
        channel_id: u32,
        request_id: RequestId,
        request: InternalIoControlPacket,
    },
    TransferIn {
        channel_id: u32,
        request_id: RequestId,
        request: TransferInPacket,
    },
    TransferOut {
        channel_id: u32,
        request_id: RequestId,
        request: TransferOutPacket,
    },
    TransferOutNoAck {
        channel_id: u32,
        request_id: RequestId,
        request: TransferOutPacket,
    },
    Cancel {
        channel_id: u32,
        request_id: RequestId,
    },
}

#[derive(Debug)]
enum ServerEvent {
    DeviceText(DeviceText),
    IoControlCompleted {
        channel_id: u32,
        request_id: RequestId,
        completion: IoControlCompletionResult,
    },
    InternalIoControlCompleted {
        channel_id: u32,
        request_id: RequestId,
        completion: IoControlCompletionResult,
    },
    TransferInCompleted {
        channel_id: u32,
        request_id: RequestId,
        completion: TransferInCompletionResult,
    },
    TransferOutCompleted {
        channel_id: u32,
        request_id: RequestId,
        completion: TransferOutCompletionResult,
    },
}

struct ChannelClientBackend {
    events: Sender<ClientEvent>,
}

impl ChannelClientBackend {
    fn send(&self, event: ClientEvent) {
        self.events
            .send(event)
            .expect("client event receiver should remain connected");
    }
}

impl UrbdrcDeviceBackend for ChannelClientBackend {
    fn device_info(&mut self, _channel_id: u32) -> PduResult<ironrdp_rdpeusb::io::DeviceInfo> {
        Ok(simple_device_info())
    }

    fn cancel_request(&mut self, request_id: RequestId, channel_id: u32) {
        self.send(ClientEvent::Cancel { channel_id, request_id });
    }

    fn query_device_text(&mut self, channel_id: u32, text_type: u32, locale_id: u32) -> PduResult<DeviceText> {
        self.send(ClientEvent::QueryDeviceText {
            channel_id,
            text_type,
            locale_id,
        });
        Ok(DeviceText {
            hresult: DEVICE_TEXT_HRESULT,
            description: DEVICE_TEXT_DESCRIPTION.to_owned(),
        })
    }

    fn io_control(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        request: IoControlPacket,
    ) -> PduResult<Option<IoControlCompletionResult>> {
        self.send(ClientEvent::IoControl {
            channel_id,
            request_id,
            request,
        });
        Ok(None)
    }

    fn internal_io_control(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        request: InternalIoControlPacket,
    ) -> PduResult<Option<IoControlCompletionResult>> {
        self.send(ClientEvent::InternalIoControl {
            channel_id,
            request_id,
            request,
        });
        Ok(None)
    }

    fn transfer_in(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        request: TransferInPacket,
    ) -> PduResult<Option<TransferInCompletionResult>> {
        self.send(ClientEvent::TransferIn {
            channel_id,
            request_id,
            request,
        });
        Ok(None)
    }

    fn transfer_out(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        request: TransferOutPacket,
    ) -> PduResult<Option<TransferOutCompletionResult>> {
        self.send(ClientEvent::TransferOut {
            channel_id,
            request_id,
            request,
        });
        Ok(None)
    }

    fn transfer_out_no_ack(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        request: TransferOutPacket,
    ) -> PduResult<()> {
        self.send(ClientEvent::TransferOutNoAck {
            channel_id,
            request_id,
            request,
        });
        Ok(())
    }

    fn retract(&mut self, _channel_id: u32) -> PduResult<()> {
        Ok(())
    }
}

struct ChannelDeviceServerBackend {
    events: Sender<ServerEvent>,
}

impl ChannelDeviceServerBackend {
    fn send(&self, event: ServerEvent) {
        self.events
            .send(event)
            .expect("server event receiver should remain connected");
    }
}

impl UrbdrcDeviceServerBackend for ChannelDeviceServerBackend {
    fn add_device(&mut self, _device: DeviceAnnounce) -> PduResult<()> {
        Ok(())
    }

    fn device_text(&mut self, device_text: DeviceText) {
        self.send(ServerEvent::DeviceText(device_text));
    }

    fn io_control_completed(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        completion: IoControlCompletionResult,
    ) -> PduResult<()> {
        self.send(ServerEvent::IoControlCompleted {
            channel_id,
            request_id,
            completion,
        });
        Ok(())
    }

    fn internal_io_control_completed(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        completion: IoControlCompletionResult,
    ) -> PduResult<()> {
        self.send(ServerEvent::InternalIoControlCompleted {
            channel_id,
            request_id,
            completion,
        });
        Ok(())
    }

    fn transfer_in_completed(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        completion: TransferInCompletionResult,
    ) -> PduResult<()> {
        self.send(ServerEvent::TransferInCompleted {
            channel_id,
            request_id,
            completion,
        });
        Ok(())
    }

    fn transfer_out_completed(
        &mut self,
        channel_id: u32,
        request_id: RequestId,
        completion: TransferOutCompletionResult,
    ) -> PduResult<()> {
        self.send(ServerEvent::TransferOutCompleted {
            channel_id,
            request_id,
            completion,
        });
        Ok(())
    }
}

struct ConnectedDevice {
    client: UrbdrcDeviceClient,
    server: UrbdrcDeviceServer,
    client_events: Receiver<ClientEvent>,
    server_events: Receiver<ServerEvent>,
}

impl ConnectedDevice {
    fn new() -> Self {
        let udev_iface = InterfaceId::try_from(4).expect("valid device interface id");
        let completion_iface = InterfaceId::try_from(5).expect("valid completion interface id");
        let (client_events_tx, client_events) = mpsc::channel();
        let (server_events_tx, server_events) = mpsc::channel();

        let client_backend = Box::new(ChannelClientBackend {
            events: client_events_tx,
        });
        let server_backend = Box::new(ChannelDeviceServerBackend {
            events: server_events_tx,
        });
        let mut client = UrbdrcDeviceClient::new(udev_iface, client_backend).expect("device client should be created");
        let mut server =
            UrbdrcDeviceServer::new(server_backend, completion_iface).expect("device server should be created");

        let mut to_client = server.start(CHANNEL_ID).expect("server start should succeed");
        let mut settled = false;
        for _ in 0..16 {
            let mut to_server = Vec::new();
            for message in to_client {
                to_server.extend(process_message(&mut client, message));
            }
            if to_server.is_empty() {
                settled = true;
                break;
            }

            to_client = Vec::new();
            for message in to_server {
                to_client.extend(process_message(&mut server, message));
            }
            if to_client.is_empty() {
                settled = true;
                break;
            }
        }
        assert!(settled, "device DVC setup should settle");
        assert!(client.ready_for_io());

        Self {
            client,
            server,
            client_events,
            server_events,
        }
    }

    fn send_to_client(&mut self, message: DvcMessage) -> Vec<DvcMessage> {
        process_message(&mut self.client, message)
    }

    fn send_to_server(&mut self, message: DvcMessage) -> Vec<DvcMessage> {
        process_message(&mut self.server, message)
    }

    fn next_client_event(&self) -> ClientEvent {
        self.client_events.try_recv().expect("client backend should be called")
    }

    fn next_server_event(&self) -> ServerEvent {
        self.server_events.try_recv().expect("server backend should be called")
    }
}

fn process_message(processor: &mut dyn ironrdp_dvc::DvcProcessor, message: DvcMessage) -> Vec<DvcMessage> {
    let payload = encode_vec(message.as_ref()).expect("DVC message should encode");
    processor
        .process(CHANNEL_ID, &payload)
        .expect("DVC message should process")
}

fn only_message(mut messages: Vec<DvcMessage>) -> DvcMessage {
    assert_eq!(messages.len(), 1);
    messages.pop().expect("one message should be present")
}

mod requests;
mod transfers;
