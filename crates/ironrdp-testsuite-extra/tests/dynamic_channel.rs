use core::any::TypeId;
use core::net::Ipv4Addr;
use core::task::Poll;
use core::time::Duration;

use ironrdp_core::{ReadCursor, decode_cursor, encode_vec};
use ironrdp_dvc::pdu::{
    CapabilitiesResponsePdu, CapsVersion, CreateResponsePdu, CreationStatus, DataPdu, DrdynvcClientPdu, DrdynvcDataPdu,
    DrdynvcServerPdu,
};
use ironrdp_dvc::{DrdynvcServer, DvcMessageBatch};
use ironrdp_pdu::mcs::SendDataIndication;
use ironrdp_pdu::rdp::vc::ChannelPduHeader;
use ironrdp_pdu::x224::X224;
use ironrdp_server::{EchoDvcBridge, RdpServer, ServerEvent, SharedWriter, dispatch_static_channel_events};
use ironrdp_svc::{StaticChannelSet, SvcMessage, SvcProcessor};
use ironrdp_tokio::TokioFramed;
use rstest::rstest;
use tokio::io::AsyncReadExt as _;
use tokio::sync::oneshot;

const SVC_ID: u16 = 1004;
const PAYLOAD: &[u8] = b"native channel data";

fn server() -> RdpServer {
    RdpServer::builder()
        .with_addr((Ipv4Addr::LOCALHOST, 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .build()
}

fn registered(server: &RdpServer) -> DrdynvcServer {
    let mut drdynvc = DrdynvcServer::new().with_dynamic_channel(EchoDvcBridge::new(server.echo_handle().clone()));
    drdynvc.start().unwrap();
    drdynvc
        .process(
            &encode_vec(&DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(
                CapsVersion::V3,
            )))
            .unwrap(),
        )
        .unwrap();
    drdynvc
}

fn opened(server: &RdpServer) -> (DrdynvcServer, u32) {
    let mut drdynvc = registered(server);
    let channel_id = drdynvc.get_channel_id_by_type::<EchoDvcBridge>().unwrap();
    drdynvc
        .process(
            &encode_vec(&DrdynvcClientPdu::Create(CreateResponsePdu::new(
                channel_id,
                CreationStatus::OK,
            )))
            .unwrap(),
        )
        .unwrap();
    (drdynvc, channel_id)
}

fn channels(drdynvc: DrdynvcServer) -> StaticChannelSet {
    let mut channels = StaticChannelSet::new();
    channels.insert(drdynvc);
    channels.attach_channel_id(TypeId::of::<DrdynvcServer>(), SVC_ID);
    channels
}

fn write(channel_id: u32) -> (ServerEvent, oneshot::Receiver<()>) {
    let batch = DvcMessageBatch::try_new(
        channel_id,
        vec![SvcMessage::from(DrdynvcDataPdu::Data(DataPdu::new(
            channel_id,
            PAYLOAD.to_vec(),
        )))],
    )
    .unwrap();
    let (completion, processed) = oneshot::channel();
    (ServerEvent::DynamicChannelWithCompletion(batch, completion), processed)
}

// oracle: mutant routing an opened DVC batch to another static/dynamic destination or altering its payload.
#[tokio::test]
async fn writes_to_the_opened_channel_preserve_payload_and_destination() {
    let mut server = server();
    let (drdynvc, channel_id) = opened(&server);
    let (event, processed) = write(channel_id);
    let (stream, mut peer) = tokio::io::duplex(128);
    let mut transport = TokioFramed::new(stream);
    dispatch_static_channel_events(
        &mut server,
        channels(drdynvc),
        &mut vec![event],
        &mut SharedWriter::new(&mut transport),
    )
    .await
    .unwrap();
    processed.await.unwrap();
    drop(transport);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).await.unwrap();
    let mut frames = ReadCursor::new(&bytes);
    let frame: X224<SendDataIndication<'_>> = decode_cursor(&mut frames).unwrap();
    assert_eq!(frame.0.channel_id, SVC_ID);
    assert_eq!(frame.0.initiator_id, 1002);
    let mut payload = ReadCursor::new(frame.0.user_data.as_ref());
    let _: ChannelPduHeader = decode_cursor(&mut payload).unwrap();
    let pdu: DrdynvcServerPdu = decode_cursor(&mut payload).unwrap();
    let DrdynvcServerPdu::Data(data) = pdu else {
        panic!("expected DVC data");
    };
    assert_eq!(data.channel_id(), channel_id);
    let DrdynvcDataPdu::Data(data) = data else {
        panic!("expected unfragmented DVC data");
    };
    assert_eq!(data.data(), PAYLOAD);
    assert!(payload.is_empty());
    assert!(frames.is_empty());
}

#[derive(Clone, Copy)]
enum Unavailable {
    Missing,
    Unjoined,
    Pending,
    Closed,
    Unknown,
    Migrated,
}

// oracle: mutant acknowledging or transmitting a batch for an unavailable or sideband-only channel.
#[rstest]
#[case::missing(Unavailable::Missing)]
#[case::unjoined(Unavailable::Unjoined)]
#[case::pending(Unavailable::Pending)]
#[case::closed(Unavailable::Closed)]
#[case::unknown(Unavailable::Unknown)]
#[case::migrated_before_response(Unavailable::Migrated)]
#[tokio::test]
async fn unavailable_destination_is_rejected(#[case] unavailable: Unavailable) {
    let mut server = server();
    let (mut drdynvc, mut channel_id) = opened(&server);
    let channels = match unavailable {
        Unavailable::Missing => StaticChannelSet::new(),
        Unavailable::Unjoined => {
            let mut channels = StaticChannelSet::new();
            channels.insert(drdynvc);
            channels
        }
        Unavailable::Pending => {
            drdynvc = registered(&server);
            channel_id = drdynvc.get_channel_id_by_type::<EchoDvcBridge>().unwrap();
            channels(drdynvc)
        }
        Unavailable::Closed => {
            drdynvc.close_channel(channel_id).unwrap();
            channels(drdynvc)
        }
        Unavailable::Unknown => {
            channel_id = u32::MAX;
            channels(drdynvc)
        }
        Unavailable::Migrated => {
            drdynvc.request_reliable_udp(vec![channel_id]).unwrap();
            channels(drdynvc)
        }
    };
    let (event, processed) = write(channel_id);
    let (stream, mut peer) = tokio::io::duplex(128);
    let mut transport = TokioFramed::new(stream);
    dispatch_static_channel_events(
        &mut server,
        channels,
        &mut vec![event],
        &mut SharedWriter::new(&mut transport),
    )
    .await
    .unwrap();
    assert!(processed.await.is_err(), "an unsent batch was acknowledged");
    drop(transport);
    let mut bytes = Vec::new();
    peer.read_to_end(&mut bytes).await.unwrap();
    assert!(bytes.is_empty(), "rejected data reached the main transport");
}

// oracle: mutant completing a DVC event before the entire transport write succeeds.
#[tokio::test]
async fn completion_waits_for_a_stalled_transport_write() {
    let mut server = server();
    let (drdynvc, channel_id) = opened(&server);
    let (event, mut processed) = write(channel_id);
    let (stream, mut peer) = tokio::io::duplex(1);
    let mut transport = TokioFramed::new(stream);
    let mut writer = SharedWriter::new(&mut transport);
    let mut events = vec![event];
    let dispatch = dispatch_static_channel_events(&mut server, channels(drdynvc), &mut events, &mut writer);
    tokio::pin!(dispatch);
    core::future::poll_fn(|cx| {
        assert!(dispatch.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(processed.try_recv(), Err(oneshot::error::TryRecvError::Empty));
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut bytes = [0; 128];
        loop {
            tokio::select! {
                sent = &mut dispatch => break sent.unwrap(),
                read = peer.read(&mut bytes) => assert_ne!(read.unwrap(), 0),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(processed.try_recv(), Ok(()));
}

// oracle: mutant acknowledging a failed transport write.
#[tokio::test]
async fn transport_failure_closes_completion() {
    let mut server = server();
    let (drdynvc, channel_id) = opened(&server);
    let (event, processed) = write(channel_id);
    let (stream, peer) = tokio::io::duplex(1);
    drop(peer);
    let mut transport = TokioFramed::new(stream);
    assert!(
        dispatch_static_channel_events(
            &mut server,
            channels(drdynvc),
            &mut vec![event],
            &mut SharedWriter::new(&mut transport),
        )
        .await
        .is_err()
    );
    assert!(processed.await.is_err());
}

// oracle: mutant retaining or acknowledging the previous connection's queued DVC batch.
#[tokio::test]
async fn discarded_session_data_closes_completion() {
    let mut server = server();
    let (_, channel_id) = opened(&server);
    let (event, processed) = write(channel_id);
    server.event_sender().send(event).unwrap();
    server.discard_stale_session_events().await;
    assert!(processed.await.is_err());
}
