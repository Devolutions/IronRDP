use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;
use std::sync::Arc;

use async_trait::async_trait;
use ironrdp_async::FramedWrite as _;
use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::DrdynvcServer;
use ironrdp_dvc::pdu::{
    CapabilitiesResponsePdu, CapsVersion, CreateResponsePdu, CreationStatus, DrdynvcClientPdu, DrdynvcServerPdu,
};
use ironrdp_echo::server::EchoServer;
use ironrdp_pdu::gcc::ClientNetworkData;
use ironrdp_pdu::input::fast_path::{FastPathInput, FastPathInputEvent, KeyboardFlags};
use ironrdp_pdu::input::scan_code::KeyboardFlags as ScanCodeFlags;
use ironrdp_pdu::input::{InputEvent, InputEventPdu, ScanCodePdu};
use ironrdp_pdu::mcs;
use ironrdp_pdu::rdp::headers::{ShareControlHeader, ShareControlPdu, ShareDataPdu};
use ironrdp_pdu::x224::X224;
use ironrdp_server::{
    DesktopSize, DisplayContext, DisplayUpdate, DynamicChannelFactory, KeyboardEvent, MouseEvent, RdpServer,
    RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler, ServerResult,
};
use ironrdp_svc::{StaticVirtualChannel, SvcMessage, client_encode_svc_messages, make_channel_definition};
use ironrdp_tokio::TokioFramed;
use tokio::io::DuplexStream;
use tokio::sync::mpsc;

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

struct InputRecorder(mpsc::UnboundedSender<KeyboardEvent>);

impl RdpServerInputHandler for InputRecorder {
    fn keyboard(&mut self, event: KeyboardEvent) {
        self.0.send(event).expect("input receiver is alive");
    }

    fn mouse(&mut self, _event: MouseEvent) {}
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

#[tokio::test(start_paused = true)]
async fn factory_channel_remains_registered_across_reactivation() {
    // mutant: A resize-triggered reactivation drops a factory-owned Echo processor or its pending request.
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
        let activation = super::activation::play_client_with_network(
            &mut client,
            true,
            &[],
            None,
            Some(ClientNetworkData {
                channels: vec![make_channel_definition(
                    &StaticVirtualChannel::new(DrdynvcServer::new()),
                )],
            }),
        )
        .await;
        let user = activation.user_channel_id;
        let [dvc] = activation.static_channel_ids.as_slice() else {
            panic!("expected one dynamic-channel static virtual channel");
        };
        assert!(matches!(
            read_dvc(&mut client, *dvc).await,
            DrdynvcServerPdu::Capabilities(_)
        ));
        let caps = DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V2));
        client
            .write_all(&client_encode_svc_messages(vec![SvcMessage::from(caps)], *dvc, user).unwrap())
            .await
            .unwrap();
        let create = loop {
            let DrdynvcServerPdu::Create(create) = read_dvc(&mut client, *dvc).await else {
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
        super::activation::reactivate_client_with_capabilities(
            &mut client,
            DesktopSize {
                width: 1024,
                height: 768,
            },
            Vec::new(),
        )
        .await;
        let reply = DrdynvcClientPdu::Create(CreateResponsePdu::new(create.channel_id(), CreationStatus::OK));
        client
            .write_all(&client_encode_svc_messages(vec![SvcMessage::from(reply)], *dvc, user).unwrap())
            .await
            .unwrap();
        let DrdynvcServerPdu::Data(data) = read_dvc(&mut client, *dvc).await else {
            panic!("expected initial ECHO request");
        };
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

// oracle: spec MS-RDPBCGR §3.3.5.8.1.1 and §3.3.5.8.1.2: in-flight keyboard events reach the session.
#[tokio::test]
async fn in_flight_keyboard_input_survives_reactivation_before_confirm() {
    let (display_tx, display_rx) = mpsc::channel(1);
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_input_handler(InputRecorder(input_tx))
        .with_display_handler(Display {
            receiver: Some(display_rx),
        })
        .build();
    let (client, server_stream) = tokio::io::duplex(16 * 1024);
    let client = async move {
        let mut client = TokioFramed::new(client);
        let activation = super::activation::play_client_with_network(&mut client, true, &[], None, None).await;
        let size = DesktopSize {
            width: 1024,
            height: 768,
        };
        display_tx.send(DisplayUpdate::Resize(size)).await.unwrap();
        let deactivate = read_frame(&mut client).await;
        let deactivate = decode::<X224<mcs::SendDataIndication<'_>>>(&deactivate).unwrap().0;
        assert!(matches!(
            decode::<ShareControlHeader>(&deactivate.user_data)
                .unwrap()
                .share_control_pdu,
            ShareControlPdu::ServerDeactivateAll(_)
        ));

        let pressed = FastPathInput::single(FastPathInputEvent::KeyboardEvent(KeyboardFlags::empty(), 0x1e));
        let slow = ShareDataPdu::Input(InputEventPdu(vec![
            InputEvent::ScanCode(ScanCodePdu {
                flags: ScanCodeFlags::DOWN,
                key_code: 0x30,
            }),
            InputEvent::ScanCode(ScanCodePdu {
                flags: ScanCodeFlags::RELEASE,
                key_code: 0x30,
            }),
        ]));
        let slow = super::acceptor::encode_send_data_request(
            activation.user_channel_id,
            deactivate.channel_id,
            &super::acceptor::encode_client_share_data(slow),
        );
        let released = FastPathInput::single(FastPathInputEvent::KeyboardEvent(KeyboardFlags::RELEASE, 0x1e));
        // Input already in flight can arrive after the server sends Deactivate All.
        client.write_all(&encode_vec(&pressed).unwrap()).await.unwrap();
        client.write_all(&slow).await.unwrap();
        client.write_all(&encode_vec(&released).unwrap()).await.unwrap();
        super::activation::reactivate_client_with_capabilities(&mut client, size, Vec::new()).await;

        let mut events = Vec::new();
        for _ in 0..4 {
            events.push(
                tokio::time::timeout(Duration::from_secs(5), input_rx.recv())
                    .await
                    .expect("in-flight keyboard event is delivered")
                    .expect("input receiver remains open"),
            );
        }
        assert!(matches!(
            events.as_slice(),
            [
                KeyboardEvent::Pressed {
                    code: 0x1e,
                    extended: false
                },
                KeyboardEvent::Pressed {
                    code: 0x30,
                    extended: false
                },
                KeyboardEvent::Released {
                    code: 0x30,
                    extended: false
                },
                KeyboardEvent::Released {
                    code: 0x1e,
                    extended: false
                }
            ]
        ));
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
