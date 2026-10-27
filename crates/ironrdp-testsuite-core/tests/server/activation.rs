use core::time::Duration;
use std::sync::{Arc, Mutex};

use ironrdp_async::FramedWrite as _;
use ironrdp_core::{WriteBuf, decode, encode_vec};
use ironrdp_pdu::mcs::{self, ConnectInitial};
use ironrdp_pdu::nego::{self, SecurityProtocol};
use ironrdp_pdu::rdp::capability_sets::CapabilitySet;
use ironrdp_pdu::rdp::headers::{ShareControlHeader, ShareControlPdu};
use ironrdp_pdu::x224::{X224, X224Data};
use ironrdp_server::{
    DesktopSize, DisplayContext, DisplayUpdate, RdpServer, RdpServerDisplay, RdpServerDisplayUpdates, ServerResult,
    TransportTls,
};
use ironrdp_testsuite_core::gcc::CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS;
use ironrdp_testsuite_core::rdp::{
    CLIENT_DEMAND_ACTIVE_PDU_BUFFER, CLIENT_FONT_LIST_BUFFER, CLIENT_INFO_PDU_BUFFER, CLIENT_SYNCHRONIZE_BUFFER,
    CONTROL_COOPERATE_BUFFER, CONTROL_REQUEST_CONTROL_BUFFER,
};
use ironrdp_tokio::TokioFramed;
use tokio::io::DuplexStream;
use tokio::sync::oneshot;

use super::acceptor::encode_send_data_request;

const CLIENT_SIZE: DesktopSize = DesktopSize {
    width: 1280,
    height: 720,
};

// oracle: mutant clients without desktop resize support never notify the display of their initial size
#[tokio::test]
async fn initial_size_is_requested_without_desktop_resize_support() {
    assert_eq!(activate(false, &[]).await, vec![CLIENT_SIZE]);
}

struct RecordingDisplay {
    size: DesktopSize,
    requests: Arc<Mutex<Vec<DesktopSize>>>,
    activated: Option<oneshot::Sender<()>>,
}

#[async_trait::async_trait]
impl RdpServerDisplay for RecordingDisplay {
    async fn size(&mut self) -> DesktopSize {
        self.size
    }

    async fn request_initial_size(&mut self, client_size: DesktopSize) -> DesktopSize {
        self.requests.lock().unwrap().push(client_size);
        self.size = client_size;
        self.size
    }

    async fn updates(&mut self, _ctx: DisplayContext) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
        self.activated.take().unwrap().send(()).unwrap();
        Ok(Box::new(PendingUpdates))
    }
}

struct PendingUpdates;

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for PendingUpdates {
    async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>> {
        core::future::pending().await
    }
}

async fn activate(desktop_resize_flag: bool, input: &[u8]) -> Vec<DesktopSize> {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (activated_tx, activated_rx) = oneshot::channel();
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_display_handler(RecordingDisplay {
            size: DesktopSize {
                width: 1920,
                height: 1080,
            },
            requests: Arc::clone(&requests),
            activated: Some(activated_tx),
        })
        .with_honor_client_desktop_size(Some(DesktopSize {
            width: 1920,
            height: 1080,
        }))
        .build();
    let (client, server_side) = tokio::io::duplex(65536);
    let client = async {
        let mut framed = TokioFramed::new(client);
        play_client(&mut framed, desktop_resize_flag, input).await;
        activated_rx.await.expect("server entered the client loop");
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        let (_result, ()) = tokio::join!(
            Box::pin(server.run_connection_with(server_side, TransportTls::AlreadyDone)),
            client
        );
    })
    .await
    .expect("activation completed");
    drop(server);
    Arc::try_unwrap(requests).unwrap().into_inner().unwrap()
}

async fn play_client(framed: &mut TokioFramed<DuplexStream>, desktop_resize_flag: bool, input: &[u8]) {
    let request = nego::ConnectionRequest {
        nego_data: None,
        flags: nego::RequestFlags::empty(),
        protocol: SecurityProtocol::empty(),
        correlation_info: None,
    };
    framed.write_all(&encode_vec(&X224(request)).unwrap()).await.unwrap();
    let _confirm = framed.read_pdu().await.unwrap();

    let mut blocks = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    blocks.network = None;
    blocks.core.desktop_width = CLIENT_SIZE.width;
    blocks.core.desktop_height = CLIENT_SIZE.height;
    let mut initial = WriteBuf::new();
    ironrdp_connector::encode_x224_packet(&ConnectInitial::with_gcc_blocks(blocks).unwrap(), &mut initial).unwrap();
    framed.write_all(initial.filled()).await.unwrap();
    let (_, response) = framed.read_pdu().await.unwrap();
    let payload = decode::<X224<X224Data<'_>>>(&response).unwrap().0;
    let response = decode::<mcs::ConnectResponse>(payload.data.as_ref()).unwrap();
    let io_channel_id = response.conference_create_response.gcc_blocks().network.io_channel;

    framed
        .write_all(
            &encode_vec(&X224(mcs::ErectDomainPdu {
                sub_height: 0,
                sub_interval: 0,
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    framed
        .write_all(&encode_vec(&X224(mcs::AttachUserRequest)).unwrap())
        .await
        .unwrap();
    let (_, confirm) = framed.read_pdu().await.unwrap();
    let user_channel_id = decode::<X224<mcs::AttachUserConfirm>>(&confirm).unwrap().0.initiator_id;
    for channel_id in [user_channel_id, io_channel_id] {
        framed
            .write_all(
                &encode_vec(&X224(mcs::ChannelJoinRequest {
                    initiator_id: user_channel_id,
                    channel_id,
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        let _confirm = framed.read_pdu().await.unwrap();
    }
    framed
        .write_all(&encode_send_data_request(
            user_channel_id,
            io_channel_id,
            &CLIENT_INFO_PDU_BUFFER,
        ))
        .await
        .unwrap();
    let _license = framed.read_pdu().await.unwrap();
    let (_, demand) = framed.read_pdu().await.unwrap();
    let indication = mcs::decode_send_data_indication(&demand).unwrap();
    let demand = indication.decode_user_data::<ShareControlHeader>().unwrap();
    let ShareControlPdu::ServerDemandActive(demand) = demand.share_control_pdu else {
        panic!("expected Demand Active");
    };
    let bitmap = demand
        .pdu
        .capability_sets
        .iter()
        .find_map(|capability| match capability {
            CapabilitySet::Bitmap(bitmap) => Some(bitmap),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        (bitmap.desktop_width, bitmap.desktop_height),
        (CLIENT_SIZE.width, CLIENT_SIZE.height)
    );

    let mut confirm = decode::<ShareControlHeader>(&CLIENT_DEMAND_ACTIVE_PDU_BUFFER).unwrap();
    let ShareControlPdu::ClientConfirmActive(active) = &mut confirm.share_control_pdu else {
        panic!("expected Confirm Active fixture");
    };
    for capability in &mut active.pdu.capability_sets {
        if let CapabilitySet::Bitmap(bitmap) = capability {
            bitmap.desktop_width = CLIENT_SIZE.width;
            bitmap.desktop_height = CLIENT_SIZE.height;
            bitmap.desktop_resize_flag = desktop_resize_flag;
        }
    }
    framed
        .write_all(&encode_send_data_request(
            user_channel_id,
            io_channel_id,
            &encode_vec(&confirm).unwrap(),
        ))
        .await
        .unwrap();
    for pdu in [
        &CLIENT_SYNCHRONIZE_BUFFER[..],
        &CONTROL_COOPERATE_BUFFER[..],
        &CONTROL_REQUEST_CONTROL_BUFFER[..],
    ] {
        framed
            .write_all(&encode_send_data_request(user_channel_id, io_channel_id, pdu))
            .await
            .unwrap();
    }
    framed.write_all(input).await.unwrap();
    framed
        .write_all(&encode_send_data_request(
            user_channel_id,
            io_channel_id,
            &CLIENT_FONT_LIST_BUFFER,
        ))
        .await
        .unwrap();
    for _ in 0..4 {
        let _response = framed.read_pdu().await.unwrap();
    }
}
