use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;
use std::sync::Arc;

use async_trait::async_trait;
use ironrdp_async::FramedWrite as _;
use ironrdp_core::{WriteBuf, decode, encode_vec};
use ironrdp_dvc::DrdynvcServer;
use ironrdp_dvc::pdu::{
    CapabilitiesResponsePdu, CapsVersion, CreateResponsePdu, CreationStatus, DrdynvcClientPdu, DrdynvcServerPdu,
};
use ironrdp_echo::server::EchoServer;
use ironrdp_pdu::gcc::ClientNetworkData;
use ironrdp_pdu::mcs::{self, ConnectInitial};
use ironrdp_pdu::nego::{self, SecurityProtocol};
use ironrdp_pdu::rdp::headers::{ShareControlHeader, ShareControlPdu};
use ironrdp_pdu::x224::{X224, X224Data};
use ironrdp_server::{
    DesktopSize, DisplayContext, DisplayUpdate, DynamicChannelFactory, RdpServer, RdpServerDisplay,
    RdpServerDisplayUpdates, ServerResult,
};
use ironrdp_svc::{StaticVirtualChannel, SvcMessage, client_encode_svc_messages, make_channel_definition};
use ironrdp_testsuite_core::gcc::CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS;
use ironrdp_testsuite_core::rdp::{
    CLIENT_DEMAND_ACTIVE_PDU_BUFFER, CLIENT_FONT_LIST_BUFFER, CLIENT_INFO_PDU_BUFFER, CLIENT_SYNCHRONIZE_BUFFER,
    CONTROL_COOPERATE_BUFFER, CONTROL_REQUEST_CONTROL_BUFFER,
};
use ironrdp_tokio::TokioFramed;
use tokio::io::DuplexStream;
use tokio::sync::mpsc;

use super::acceptor::encode_send_data_request;

struct EchoFactory(Arc<AtomicU32>);

impl DynamicChannelFactory for EchoFactory {
    fn attach(&self, channels: DrdynvcServer) -> DrdynvcServer {
        let channels = channels.with_dynamic_channel(EchoServer::new().with_initial_request(vec![0x71, 0x82]));
        self.0.store(
            channels.get_channel_id_by_type::<EchoServer>().unwrap(),
            Ordering::Relaxed,
        );
        channels
    }
}

struct Display {
    receiver: Option<mpsc::Receiver<DisplayUpdate>>,
}

struct Updates {
    receiver: mpsc::Receiver<DisplayUpdate>,
}

#[async_trait]
impl RdpServerDisplay for Display {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize {
            width: 1280,
            height: 720,
        }
    }

    async fn updates(&mut self, _: DisplayContext) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
        let receiver = self.receiver.take().unwrap_or_else(|| mpsc::channel(1).1);
        Ok(Box::new(Updates { receiver }))
    }

    async fn offers_display_control(&mut self) -> bool {
        false
    }
}

#[async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>> {
        match self.receiver.recv().await {
            Some(update) => Ok(Some(update)),
            None => core::future::pending().await,
        }
    }
}

async fn read_frame(client: &mut TokioFramed<DuplexStream>) -> Vec<u8> {
    tokio::time::timeout(Duration::from_secs(5), client.read_pdu())
        .await
        .expect("server must answer without losing channel traffic")
        .unwrap()
        .1
        .to_vec()
}

async fn read_dvc(client: &mut TokioFramed<DuplexStream>, channel: u16) -> DrdynvcServerPdu {
    let frame = read_frame(client).await;
    let data = decode::<X224<mcs::SendDataIndication<'_>>>(&frame).unwrap().0;
    assert_eq!(data.channel_id, channel);
    // oracle: spec MS-RDPBCGR §2.2.6.1.1: CHANNEL_PDU_HEADER is eight bytes.
    decode(&data.user_data[8..]).unwrap()
}

async fn confirm_and_finalize(client: &mut TokioFramed<DuplexStream>, user: u16, io: u16) {
    client
        .write_all(&encode_send_data_request(user, io, &CLIENT_DEMAND_ACTIVE_PDU_BUFFER))
        .await
        .unwrap();
    finalize(client, user, io).await;
}

async fn finalize(client: &mut TokioFramed<DuplexStream>, user: u16, io: u16) {
    for data in [
        CLIENT_SYNCHRONIZE_BUFFER.as_slice(),
        CONTROL_COOPERATE_BUFFER.as_slice(),
        CONTROL_REQUEST_CONTROL_BUFFER.as_slice(),
        CLIENT_FONT_LIST_BUFFER.as_slice(),
    ] {
        client
            .write_all(&encode_send_data_request(user, io, data))
            .await
            .unwrap();
    }
    for _ in 0..4 {
        read_frame(client).await;
    }
}

#[tokio::test(start_paused = true)]
async fn server_restores_channels_before_replaying_reactivation_backlog() {
    // oracle: spec MS-RDPBCGR §1.3.3 and MS-RDPEDYC §3.3.5.1: an accepted DVC delivers data across resize.
    let (sender, receiver) = mpsc::channel(1);
    let echo_id = Arc::new(AtomicU32::new(u32::MAX));
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_display_handler(Display {
            receiver: Some(receiver),
        })
        .with_dynamic_channel_factory(Box::new(EchoFactory(Arc::clone(&echo_id))))
        .build();
    let (client, server_stream) = tokio::io::duplex(16 * 1024);
    let client = async move {
        let mut client = TokioFramed::new(client);
        client
            .write_all(
                &encode_vec(&X224(nego::ConnectionRequest {
                    nego_data: None,
                    flags: nego::RequestFlags::empty(),
                    protocol: SecurityProtocol::empty(),
                    correlation_info: None,
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        read_frame(&mut client).await;
        let mut blocks = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
        blocks.network = Some(ClientNetworkData {
            channels: vec![make_channel_definition(
                &StaticVirtualChannel::new(DrdynvcServer::new()),
            )],
        });
        let initial = ConnectInitial::with_gcc_blocks(blocks).unwrap();
        let mut buffer = WriteBuf::new();
        ironrdp_connector::encode_x224_packet(&initial, &mut buffer).unwrap();
        client.write_all(buffer.filled()).await.unwrap();
        let response = read_frame(&mut client).await;
        let response = decode::<X224<X224Data<'_>>>(&response).unwrap().0;
        let response = decode::<mcs::ConnectResponse>(&response.data).unwrap();
        let network = &response.conference_create_response.gcc_blocks().network;
        let io = network.io_channel;
        let dvc = network.channel_ids[0];
        client
            .write_all(
                &encode_vec(&X224(mcs::ErectDomainPdu {
                    sub_height: 0,
                    sub_interval: 0,
                }))
                .unwrap(),
            )
            .await
            .unwrap();
        client
            .write_all(&encode_vec(&X224(mcs::AttachUserRequest)).unwrap())
            .await
            .unwrap();
        let attach = read_frame(&mut client).await;
        let user = decode::<X224<mcs::AttachUserConfirm>>(&attach).unwrap().0.initiator_id;
        for channel in [user, io, dvc] {
            client
                .write_all(
                    &encode_vec(&X224(mcs::ChannelJoinRequest {
                        initiator_id: user,
                        channel_id: channel,
                    }))
                    .unwrap(),
                )
                .await
                .unwrap();
            read_frame(&mut client).await;
        }
        client
            .write_all(&encode_send_data_request(user, io, &CLIENT_INFO_PDU_BUFFER))
            .await
            .unwrap();
        read_frame(&mut client).await;
        read_frame(&mut client).await;
        confirm_and_finalize(&mut client, user, io).await;
        assert!(matches!(
            read_dvc(&mut client, dvc).await,
            DrdynvcServerPdu::Capabilities(_)
        ));
        let caps = DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V2));
        client
            .write_all(&client_encode_svc_messages(vec![SvcMessage::from(caps)], dvc, user).unwrap())
            .await
            .unwrap();
        let create = loop {
            let DrdynvcServerPdu::Create(create) = read_dvc(&mut client, dvc).await else {
                panic!("expected DVC creation");
            };
            if create.channel_id() == echo_id.load(Ordering::Relaxed) {
                break create;
            }
        };
        sender
            .send(DisplayUpdate::Resize(DesktopSize {
                width: 1024,
                height: 768,
            }))
            .await
            .unwrap();
        let deactivate = read_frame(&mut client).await;
        let deactivate = decode::<X224<mcs::SendDataIndication<'_>>>(&deactivate).unwrap().0;
        assert!(matches!(
            decode::<ShareControlHeader>(&deactivate.user_data)
                .unwrap()
                .share_control_pdu,
            ShareControlPdu::ServerDeactivateAll(_)
        ));
        read_frame(&mut client).await;
        client
            .write_all(&encode_send_data_request(user, io, &CLIENT_DEMAND_ACTIVE_PDU_BUFFER))
            .await
            .unwrap();
        let reply = DrdynvcClientPdu::Create(CreateResponsePdu::new(create.channel_id(), CreationStatus::OK));
        client
            .write_all(&client_encode_svc_messages(vec![SvcMessage::from(reply)], dvc, user).unwrap())
            .await
            .unwrap();
        finalize(&mut client, user, io).await;
        let DrdynvcServerPdu::Data(data) = read_dvc(&mut client, dvc).await else {
            panic!("expected initial ECHO request");
        };
        assert_eq!(data.channel_id(), create.channel_id());
        let ironrdp_dvc::pdu::DrdynvcDataPdu::Data(data) = data else {
            panic!("small ECHO message must fit one chunk");
        };
        assert_eq!(data.into_data(), vec![0x71, 0x82]);
        client
            .write_all(
                &encode_vec(&X224(mcs::DisconnectProviderUltimatum {
                    reason: mcs::DisconnectReason::UserRequested,
                }))
                .unwrap(),
            )
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(server.run_connection(server_stream), client);
    result.unwrap();
}
