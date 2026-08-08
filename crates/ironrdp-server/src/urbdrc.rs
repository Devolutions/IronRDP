use ironrdp_dvc::DynamicChannelId;
use ironrdp_pdu::PduResult;
use ironrdp_rdpeusb::{
    io::{InternalIoControlPacket, IoControlPacket, RequestId, TransferInPacket, TransferOutPacket, UsbRetractReason},
    server::{UrbdrcControlServerBackend, UrbdrcDeviceServerBackend},
};
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use crate::ServerEvent;

#[derive(Debug)]
pub enum UrbdrcServerMessage {
    AddChan,
    Device {
        dvc_id: u32,
        dev_msg: UrbdrcDeviceServerMessage,
    },
}

#[derive(Debug)]
pub enum UrbdrcDeviceServerMessage {
    QueryDeviceText {
        text_type: u32,
        locale_id: u32,
    },
    Io {
        data: ServerDeviceIoData,
        /// Completion signal for the server-allocated request metadata.
        tx: oneshot::Sender<SentIoRequest>,
    },
    CancelRequest(RequestId),
    Retract(UsbRetractReason),
}

/// Server-to-client USB I/O request data.
#[derive(Debug)]
pub enum ServerDeviceIoData {
    IoControl(IoControlPacket),
    InternalIoControl(InternalIoControlPacket),
    TransferIn(TransferInPacket),
    TransferOut(TransferOutPacket),
}

/// Metadata for an I/O request after it has been written to the transport.
#[derive(Debug)]
pub struct SentIoRequest {
    /// Request ID allocated by the RDPEUSB device state machine.
    pub request_id: RequestId,
    /// Whether the client is expected to send a completion for this request.
    pub expects_completion: bool,
}

/// Creates per-device URBDRC backends.
pub trait DeviceFactory {
    /// Creates the [UrbdrcDeviceServerBackend] for a newly opened device channel.
    fn create_device(&mut self, handle: UsbDeviceHandle) -> Option<Box<dyn UrbdrcDeviceServerBackend>>;
}

#[derive(Debug, Clone)]
pub(crate) struct UsbControlHandle {
    event_sender: UnboundedSender<ServerEvent>,
}

impl UsbControlHandle {
    pub(crate) fn new(event_sender: UnboundedSender<ServerEvent>) -> Self {
        Self { event_sender }
    }
}

impl UrbdrcControlServerBackend for UsbControlHandle {
    fn create_device_chan(&mut self) -> PduResult<()> {
        let _ = self.event_sender.send(ServerEvent::Usb(UrbdrcServerMessage::AddChan));
        Ok(())
    }
}

/// Handle used by a device backend to send server-to-client requests.
#[derive(Debug, Clone)]
pub struct UsbDeviceHandle {
    sender: UnboundedSender<ServerEvent>,
    channel_id: DynamicChannelId,
}

impl UsbDeviceHandle {
    pub(crate) fn new(sender: UnboundedSender<ServerEvent>, channel_id: DynamicChannelId) -> Self {
        Self { sender, channel_id }
    }

    fn send_device_message(&self, dev_msg: UrbdrcDeviceServerMessage) -> anyhow::Result<()> {
        self.sender
            .send(ServerEvent::Usb(UrbdrcServerMessage::Device {
                dvc_id: self.channel_id,
                dev_msg,
            }))
            .map_err(|_error| anyhow::anyhow!("failed to send usb device request"))
    }

    async fn send_io_message(&self, data: ServerDeviceIoData) -> anyhow::Result<SentIoRequest> {
        let (tx, rx) = oneshot::channel();
        self.send_device_message(UrbdrcDeviceServerMessage::Io { data, tx })?;
        rx.await
            .map_err(|_| anyhow::anyhow!("failed to get usb device request result"))
    }

    // TODO(#1209): replace anyhow::Result with typed error

    /// Sends a transfer-in request and returns its request metadata once written.
    pub async fn transfer_in_request(&self, packet: TransferInPacket) -> anyhow::Result<SentIoRequest> {
        self.send_io_message(ServerDeviceIoData::TransferIn(packet)).await
    }

    /// Sends a transfer-out request and returns its request metadata once written.
    pub async fn transfer_out_request(&self, packet: TransferOutPacket) -> anyhow::Result<SentIoRequest> {
        self.send_io_message(ServerDeviceIoData::TransferOut(packet)).await
    }

    /// Sends an IO_CONTROL request and returns its request metadata once written.
    pub async fn ioctl_request(&self, packet: IoControlPacket) -> anyhow::Result<SentIoRequest> {
        self.send_io_message(ServerDeviceIoData::IoControl(packet)).await
    }

    /// Sends an INTERNAL_IO_CONTROL request and returns its request metadata once written.
    pub async fn internal_ioctl_request(&self, packet: InternalIoControlPacket) -> anyhow::Result<SentIoRequest> {
        self.send_io_message(ServerDeviceIoData::InternalIoControl(packet))
            .await
    }

    /// Sends a query-device-text request.
    pub fn query_device_text_request(&self, text_type: u32, locale_id: u32) -> anyhow::Result<()> {
        self.send_device_message(UrbdrcDeviceServerMessage::QueryDeviceText { text_type, locale_id })
    }

    /// Sends a device-retract request.
    pub fn retract_request(&self, reason: UsbRetractReason) -> anyhow::Result<()> {
        self.send_device_message(UrbdrcDeviceServerMessage::Retract(reason))
    }

    /// Sends a cancel request for a pending I/O request.
    pub fn cancel_request(&self, request_id: u32) -> anyhow::Result<()> {
        self.send_device_message(UrbdrcDeviceServerMessage::CancelRequest(request_id))
    }
}
