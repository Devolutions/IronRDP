//! Plug and Play device redirection ([MS-RDPEPNP]) configuration and session wiring.
//!
//! [MS-RDPEPNP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepnp/

use core::fmt;
use std::sync::Arc;

use ironrdp_dvc::pdu::SoftSyncTunnelType;
use ironrdp_dvc::{DrdynvcClient, DvcMessageBatch};
use ironrdp_pdu::pdu_other_err;
use ironrdp_rdpepnp::client::{IoUplink, PnpBackend, PnpInfoClient, UplinkAction, UplinkRequest};
use ironrdp_rdpepnp::pdu::info::DeviceDescription;
use ironrdp_session::{SessionError, SessionErrorExt as _, SessionResult};
use tracing::{trace, warn};

use crate::rdp::RdpInputSender;

/// Creates the backend of one connection.
pub type PnpBackendFactory = Arc<dyn Fn() -> Box<dyn PnpBackend> + Send + Sync>;

/// Plug and Play devices to redirect and the backend that performs their I/O.
///
/// IronRDP has no native PnP device backend: the embedder describes the devices and implements
/// [`PnpBackend`]. The factory runs once per connection.
#[derive(Clone)]
pub struct PnpConfig {
    pub devices: Vec<DeviceDescription>,
    pub backend: PnpBackendFactory,
}

impl fmt::Debug for PnpConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PnpConfig")
            .field("devices", &self.devices)
            .finish_non_exhaustive()
    }
}

/// Registers the `PNPDR` channel and the `FileRedirectorChannel` listener.
pub(crate) fn attach(drdynvc: DrdynvcClient, config: &PnpConfig, sender: &RdpInputSender) -> DrdynvcClient {
    let info = match PnpInfoClient::new(config.devices.clone()) {
        Ok(info) => info,
        Err(error) => {
            warn!(%error, "Invalid PnP device list; PnP device redirection is disabled");
            return drdynvc;
        }
    };
    let sender = sender.clone();
    let uplink: IoUplink = Arc::new(move |request| {
        sender
            .send_pnp(request)
            .map_err(|_| pdu_other_err!("send PnP device replies to the event loop"))
    });
    let listener = info.file_redirector_listener((config.backend)(), uplink);
    drdynvc.with_dynamic_channel(info).with_listener(listener)
}

/// Turns a request of a `FileRedirectorChannel` instance into the DRDYNVC messages to send, if it still applies.
pub(crate) fn process_uplink_request(
    drdynvc: Option<&mut DrdynvcClient>,
    request: UplinkRequest,
) -> SessionResult<Option<(DvcMessageBatch, Option<SoftSyncTunnelType>)>> {
    let channel_id = request.channel_id();
    match request.into_action() {
        Some(UplinkAction::Send(messages)) => Ok(Some((
            DvcMessageBatch::try_new(channel_id, messages).map_err(SessionError::pdu)?,
            None,
        ))),
        Some(UplinkAction::Close) => Ok(drdynvc.and_then(|drdynvc| drdynvc.close_channel_batch(channel_id))),
        None => {
            trace!(channel_id, "Dropping a request of a closed PnP I/O channel instance");
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use ironrdp_core::encode_vec;
    use ironrdp_dvc::pdu::{CreateRequestPdu, DataPdu, DrdynvcClientPdu, DrdynvcDataPdu, DrdynvcServerPdu};
    use ironrdp_rdpepnp::client::{IoControlCompletion, PnpFile, ReadCompletion, WriteCompletion};
    use ironrdp_rdpepnp::pdu::HResult;
    use ironrdp_rdpepnp::pdu::info::{PnpInfoPdu, VersionPdu};
    use ironrdp_rdpepnp::pdu::io::{
        CapabilitiesRequest, CreateFileRequest, IoControlRequest, ReadRequest, RequestId, ServerIoPdu, WriteReply,
        WriteRequest,
    };
    use ironrdp_rdpepnp::{FILE_REDIRECTOR_CHANNEL_NAME, PNPDR_CHANNEL_NAME};
    use ironrdp_svc::SvcProcessor as _;

    use super::*;
    use crate::rdp::RdpInputEvent;

    const PNPDR_ID: u32 = 3;
    const IO_ID: u32 = 7;
    const DEVICE_ID: u32 = 4;

    type ParkedWrites = Arc<Mutex<Vec<WriteCompletion>>>;

    /// Opens every device; its files park writes and drop other requests.
    struct ParkingBackend(ParkedWrites);

    impl PnpBackend for ParkingBackend {
        fn create_file(
            &mut self,
            _request: &CreateFileRequest,
            _events: ironrdp_rdpepnp::client::CustomEventSender,
        ) -> Result<Box<dyn PnpFile>, HResult> {
            Ok(Box::new(ParkingFile(Arc::clone(&self.0))))
        }
    }

    struct ParkingFile(ParkedWrites);

    impl PnpFile for ParkingFile {
        fn read(&mut self, _request: ReadRequest, _completion: ReadCompletion) {}

        fn write(&mut self, _request: WriteRequest, completion: WriteCompletion) {
            self.0.lock().unwrap().push(completion);
        }

        fn io_control(&mut self, _request: IoControlRequest, _completion: IoControlCompletion) {}
    }

    fn device(client_device_id: u32) -> DeviceDescription {
        DeviceDescription {
            client_device_id,
            interface_guids: Vec::new(),
            hardware_ids: Vec::new(),
            compatibility_ids: Vec::new(),
            description: "Test Device".to_owned(),
            custom_flag: 0,
            container_id: None,
            device_caps: None,
        }
    }

    fn create(channel_id: u32, name: &str) -> Vec<u8> {
        encode_vec(&DrdynvcServerPdu::Create(CreateRequestPdu::new(
            channel_id,
            name.to_owned(),
        )))
        .unwrap()
    }

    fn data(channel_id: u32, payload: &impl ironrdp_core::Encode) -> Vec<u8> {
        let payload = encode_vec(payload).unwrap();
        encode_vec(&DrdynvcServerPdu::Data(DrdynvcDataPdu::Data(DataPdu::new(
            channel_id, payload,
        ))))
        .unwrap()
    }

    fn request_id(value: u32) -> RequestId {
        RequestId::new(value).unwrap()
    }

    fn write(id: u32) -> ServerIoPdu {
        ServerIoPdu::Write(WriteRequest {
            request_id: request_id(id),
            offset: 0,
            data: vec![1, 2, 3],
        })
    }

    /// Attaches PnP redirection and opens a file on a `FileRedirectorChannel` instance.
    fn open_file(sender: &RdpInputSender, writes: &ParkedWrites) -> DrdynvcClient {
        let backend = Arc::clone(writes);
        let config = PnpConfig {
            devices: vec![device(DEVICE_ID)],
            backend: Arc::new(move || Box::new(ParkingBackend(Arc::clone(&backend)))),
        };
        let mut drdynvc = attach(DrdynvcClient::new(), &config, sender);
        drdynvc.process(&create(PNPDR_ID, PNPDR_CHANNEL_NAME)).unwrap();
        let version = PnpInfoPdu::Version(VersionPdu {
            major_version: 1,
            minor_version: 6,
            capabilities: 1,
        });
        drdynvc.process(&data(PNPDR_ID, &version)).unwrap();
        drdynvc.process(&data(PNPDR_ID, &PnpInfoPdu::ServerLogon)).unwrap();
        drdynvc.process(&create(IO_ID, FILE_REDIRECTOR_CHANNEL_NAME)).unwrap();
        let capabilities = ServerIoPdu::Capabilities(CapabilitiesRequest {
            request_id: request_id(0),
            version: 6,
        });
        drdynvc.process(&data(IO_ID, &capabilities)).unwrap();
        let create_file = ServerIoPdu::CreateFile(CreateFileRequest {
            request_id: request_id(1),
            device_id: DEVICE_ID,
            desired_access: 0,
            share_mode: 0,
            creation_disposition: 0,
            flags_and_attributes: 0,
        });
        drdynvc.process(&data(IO_ID, &create_file)).unwrap();
        drdynvc
    }

    fn take_request(ordered: &mut tokio::sync::mpsc::UnboundedReceiver<RdpInputEvent>) -> UplinkRequest {
        match ordered.try_recv() {
            Ok(RdpInputEvent::Pnp(request)) => request,
            other => panic!("expected a PnP uplink request, got {other:?}"),
        }
    }

    // oracle: spec MS-RDPEPNP §3.2.5.2.2.6 (a Write Reply answers each Write Request)
    #[test]
    fn late_write_reply_is_sent_on_its_channel() {
        let (sender, _input, mut ordered, _, _) = RdpInputSender::channel_with_close_signal(1);
        let writes = ParkedWrites::default();
        let mut drdynvc = open_file(&sender, &writes);
        drdynvc.process(&data(IO_ID, &write(2))).unwrap();

        writes.lock().unwrap().pop().unwrap().complete(HResult::S_OK, 3);

        let request = take_request(&mut ordered);
        let (batch, tunnel) = process_uplink_request(Some(&mut drdynvc), request).unwrap().unwrap();
        assert_eq!(batch.channel_id(), IO_ID);
        assert!(tunnel.is_none());
        let [message] = batch.messages() else {
            panic!("expected one DRDYNVC message");
        };
        let DrdynvcClientPdu::Data(DrdynvcDataPdu::Data(pdu)) =
            ironrdp_core::decode(&message.encode_unframed_pdu().unwrap()).unwrap()
        else {
            panic!("expected unsplit DRDYNVC data");
        };
        let reply: WriteReply = ironrdp_core::decode(pdu.data()).unwrap();
        assert_eq!(
            reply,
            WriteReply {
                request_id: request_id(2),
                result: HResult::S_OK,
                bytes_written: 3,
            }
        );
    }

    // oracle: spec MS-RDPEPNP §3.1.1 (a duplicate request ID terminates the channel)
    #[test]
    fn protocol_violation_closes_the_channel() {
        let (sender, _input, mut ordered, _, _) = RdpInputSender::channel_with_close_signal(1);
        let writes = ParkedWrites::default();
        let mut drdynvc = open_file(&sender, &writes);
        drdynvc.process(&data(IO_ID, &write(2))).unwrap();
        drdynvc.process(&data(IO_ID, &write(2))).unwrap();

        let request = take_request(&mut ordered);
        let (batch, _) = process_uplink_request(Some(&mut drdynvc), request).unwrap().unwrap();

        assert_eq!(batch.channel_id(), IO_ID);
        assert!(
            drdynvc
                .get_dvc_by_channel_id::<ironrdp_rdpepnp::client::FileRedirectorClient>(IO_ID)
                .is_none()
        );
    }

    // oracle: spec MS-RDPEDYC §3.2.5.2 (after a close, the server can reuse the channel ID)
    #[test]
    fn reply_of_a_closed_channel_is_dropped() {
        let (sender, _input, mut ordered, _, _) = RdpInputSender::channel_with_close_signal(1);
        let writes = ParkedWrites::default();
        let mut drdynvc = open_file(&sender, &writes);
        drdynvc.process(&data(IO_ID, &write(2))).unwrap();
        writes.lock().unwrap().pop().unwrap().complete(HResult::S_OK, 3);
        let close = ironrdp_dvc::pdu::ClosePdu::new(IO_ID);
        drdynvc
            .process(&encode_vec(&DrdynvcServerPdu::Close(close)).unwrap())
            .unwrap();

        let request = take_request(&mut ordered);

        assert!(process_uplink_request(Some(&mut drdynvc), request).unwrap().is_none());
    }

    // oracle: spec MS-RDPEPNP §3.1.1 (unique client device IDs)
    #[test]
    fn invalid_device_list_leaves_the_channels_unregistered() {
        let (sender, _input) = RdpInputSender::channel(1);
        let config = PnpConfig {
            devices: vec![device(DEVICE_ID), device(DEVICE_ID)],
            backend: Arc::new(|| Box::new(ParkingBackend(ParkedWrites::default()))),
        };

        let drdynvc = attach(DrdynvcClient::new(), &config, &sender);

        assert!(!drdynvc.has_registered_dvc::<PnpInfoClient>());
    }
}
