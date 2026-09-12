//! Integration coverage for `accept_finalize_with_multitransport`.
//!
//! Drives a real `Acceptor` over an in-memory duplex stream, playing the role
//! of the client by hand (raw PDU bytes, not a `ClientConnector`) so the test
//! exercises the acceptor's actual async driver rather than only the
//! synchronous `Acceptor::step()` surface already covered in `acceptor.rs`.

use std::sync::{Arc, Mutex};

use ironrdp_acceptor::{Acceptor, accept_finalize_with_multitransport};
use ironrdp_async::FramedWrite as _;
use ironrdp_connector::DesktopSize;
use ironrdp_core::{WriteBuf, decode, encode_buf, encode_vec};
use ironrdp_pdu::gcc::MultiTransportFlags;
use ironrdp_pdu::mcs::{self, ConnectInitial};
use ironrdp_pdu::nego::{self, SecurityProtocol};
use ironrdp_pdu::rdp::multitransport::{MultitransportRequestPdu, MultitransportResponsePdu, RequestedProtocol};
use ironrdp_pdu::x224::{X224, X224Data};
use ironrdp_testsuite_core::rdp::{
    CLIENT_DEMAND_ACTIVE_PDU_BUFFER, CLIENT_FONT_LIST_BUFFER, CLIENT_INFO_PDU_BUFFER, CLIENT_SYNCHRONIZE_BUFFER,
    CONTROL_COOPERATE_BUFFER, CONTROL_REQUEST_CONTROL_BUFFER,
};
use ironrdp_tokio::TokioFramed;
use tokio::io::DuplexStream;

use super::acceptor::{client_gcc_with_message_channel_and_multitransport, encode_send_data_request};

fn encode_x224_pdu<'a, T: ironrdp_pdu::x224::X224Pdu<'a>>(pdu: T) -> Vec<u8> {
    let mut buf = WriteBuf::new();
    encode_buf(&X224(pdu), &mut buf).unwrap();
    buf.filled().to_vec()
}

/// Reads one PDU from `framed` and returns its raw bytes (TPKT/X224 header
/// stripped, matching what `decode` in the rest of this file expects).
async fn read_pdu(framed: &mut TokioFramed<DuplexStream>) -> Vec<u8> {
    let (_, bytes) = framed.read_pdu().await.expect("read one PDU");
    bytes.to_vec()
}

/// Drives the client side of a full connection sequence up to (and through)
/// finalization, over `framed`, against an acceptor configured to offer
/// reliable UDP multitransport. Sends a (soft-sync-less) Initiate
/// Multitransport Response before the Confirm Active, which the acceptor
/// must tolerate rather than error on. Returns the same stream, still open,
/// the user and I/O channel IDs, for a caller that wants to keep driving it
/// (e.g. through reactivation), and the request_id the acceptor's Initiate
/// Multitransport Request carried, for a caller that wants to cross-check
/// it against what a handler observed.
async fn play_client(mut framed: TokioFramed<DuplexStream>) -> (TokioFramed<DuplexStream>, u16, u16, u32) {
    // Connection Request / Confirm. Plain RDP security (empty protocol) so no
    // TLS upgrade is needed and the acceptor drives straight through.
    let request = nego::ConnectionRequest {
        nego_data: None,
        flags: nego::RequestFlags::empty(),
        protocol: SecurityProtocol::empty(),
        correlation_info: None,
    };
    framed.write_all(&encode_x224_pdu(request)).await.unwrap();
    let _confirm = read_pdu(&mut framed).await;

    // MCS Connect Initial, advertising both a message channel and reliable
    // UDP multitransport support, so the acceptor has something to offer.
    let blocks = client_gcc_with_message_channel_and_multitransport(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));
    let connect_initial = ConnectInitial::with_gcc_blocks(blocks).unwrap();
    let mut initial_buf = WriteBuf::new();
    ironrdp_connector::encode_x224_packet(&connect_initial, &mut initial_buf).unwrap();
    framed.write_all(initial_buf.filled()).await.unwrap();

    let response_bytes = read_pdu(&mut framed).await;
    let payload = decode::<X224<X224Data<'_>>>(&response_bytes).unwrap().0;
    let response = decode::<mcs::ConnectResponse>(payload.data.as_ref()).unwrap();
    let server_blocks = response.conference_create_response.gcc_blocks();
    let io_channel_id = server_blocks.network.io_channel;
    let message_channel_id = server_blocks
        .message_channel
        .as_ref()
        .expect("acceptor must negotiate a message channel")
        .mcs_message_channel_id;

    // Channel connection: Erect Domain, Attach User, then join every channel
    // the server expects (user, I/O, message).
    framed
        .write_all(&encode_x224_pdu(mcs::ErectDomainPdu {
            sub_height: 0,
            sub_interval: 0,
        }))
        .await
        .unwrap();
    framed
        .write_all(&encode_x224_pdu(mcs::AttachUserRequest))
        .await
        .unwrap();

    let confirm_bytes = read_pdu(&mut framed).await;
    let attach_user_confirm = decode::<X224<mcs::AttachUserConfirm>>(&confirm_bytes).unwrap().0;
    let user_channel_id = attach_user_confirm.initiator_id;

    for channel_id in [user_channel_id, io_channel_id, message_channel_id] {
        framed
            .write_all(&encode_x224_pdu(mcs::ChannelJoinRequest {
                initiator_id: user_channel_id,
                channel_id,
            }))
            .await
            .unwrap();
        let _join_confirm = read_pdu(&mut framed).await;
    }

    // Client Info PDU on the I/O channel, then the license PDU comes back.
    framed
        .write_all(&encode_send_data_request(
            user_channel_id,
            io_channel_id,
            &CLIENT_INFO_PDU_BUFFER,
        ))
        .await
        .unwrap();
    let _license = read_pdu(&mut framed).await;

    // The Initiate Multitransport Request, on the message channel.
    let request_bytes = read_pdu(&mut framed).await;
    let indication = mcs::decode_send_data_indication(&request_bytes).unwrap();
    assert_eq!(
        indication.channel_id, message_channel_id,
        "request must go out on the message channel"
    );
    let request = indication.decode_user_data::<MultitransportRequestPdu>().unwrap();
    assert_eq!(request.requested_protocol, RequestedProtocol::UdpFecR);

    // Demand Active, on the I/O channel.
    let _demand_active = read_pdu(&mut framed).await;

    // A response arriving ahead of Confirm Active: the acceptor must
    // silently absorb it and keep waiting rather than erroring.
    // MS-RDPBCGR 2.2.15.2: S_OK MUST only be sent to a server advertising
    // SOFTSYNC_TCP_TO_UDP, which this test's offer does not include; the
    // legitimate response here is a failure code.
    let response = MultitransportResponsePdu::abort(request.request_id);
    framed
        .write_all(&encode_send_data_request(
            user_channel_id,
            message_channel_id,
            &encode_vec(&response).unwrap(),
        ))
        .await
        .unwrap();

    // Confirm Active, after the multitransport response: on the wire, the
    // response (if any) is sent while the client resolves its own
    // MultitransportPending state, strictly before it ever reads Demand
    // Active, so this ordering matches a real client. CapabilitiesWaitConfirm
    // must see this before the acceptor moves on to finalization.
    play_confirm_active_and_finalization(&mut framed, user_channel_id, io_channel_id).await;

    (framed, user_channel_id, io_channel_id, request.request_id)
}

/// Writes Confirm Active, then the finalization exchange (Synchronize,
/// Control Cooperate, Control Request Control, Font List) and reads the four
/// matching responses, all on `io_channel_id`. Shared by `play_client`'s
/// first pass and `play_reactivation_round`'s replay after a
/// Deactivation-Reactivation Sequence.
async fn play_confirm_active_and_finalization(
    framed: &mut TokioFramed<DuplexStream>,
    user_channel_id: u16,
    io_channel_id: u16,
) {
    framed
        .write_all(&encode_send_data_request(
            user_channel_id,
            io_channel_id,
            &CLIENT_DEMAND_ACTIVE_PDU_BUFFER,
        ))
        .await
        .unwrap();

    // Finalization: Synchronize, Control Cooperate, Control Request Control,
    // Font List, each accepted without a response until Font List lands.
    for pdu_bytes in [
        &CLIENT_SYNCHRONIZE_BUFFER[..],
        &CONTROL_COOPERATE_BUFFER[..],
        &CONTROL_REQUEST_CONTROL_BUFFER[..],
        &CLIENT_FONT_LIST_BUFFER[..],
    ] {
        framed
            .write_all(&encode_send_data_request(user_channel_id, io_channel_id, pdu_bytes))
            .await
            .unwrap();
    }

    // Four finalization responses, all on the I/O channel: Synchronize
    // Confirm, Control Cooperate Confirm, Control Granted Confirm, Font Map.
    for _ in 0..4 {
        let _ = read_pdu(framed).await;
    }
}

/// Plays a Deactivation-Reactivation round: reads the fresh Demand Active
/// (`new_deactivation_reactivation` rebuilds the acceptor straight into
/// `CapabilitiesSendServer`, skipping channel join, licensing, and
/// multitransport bootstrapping entirely, per its own doc comment), then
/// redoes Confirm Active and the finalization exchange. No multitransport
/// traffic is expected this round.
async fn play_reactivation_round(mut framed: TokioFramed<DuplexStream>, user_channel_id: u16, io_channel_id: u16) {
    let _demand_active = read_pdu(&mut framed).await;

    play_confirm_active_and_finalization(&mut framed, user_channel_id, io_channel_id).await;
}

/// The Initiate Multitransport Request must reach the caller through
/// `accept_finalize_with_multitransport`'s handler exactly once, with the
/// request the acceptor actually sent, and without the handler blocking the
/// rest of the handshake (which the client-side script above completes
/// concurrently).
#[tokio::test]
async fn multitransport_handler_fires_once_without_blocking_finalization() {
    let (client_stream, server_stream) = tokio::io::duplex(65536);

    let handler_calls: Arc<Mutex<Vec<(u32, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let handler_calls_for_server = Arc::clone(&handler_calls);

    // `accept_finalize_with_multitransport`'s generic handler closure is
    // `!Send` for the same structural reason `RdpServer`'s own future is
    // (see `finalize_timeout.rs`): a `LocalSet` is required rather than a
    // plain `tokio::spawn`.
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let server = tokio::task::spawn_local(async move {
                let mut acceptor = Acceptor::new(
                    SecurityProtocol::empty(),
                    DesktopSize {
                        width: 1920,
                        height: 1080,
                    },
                    Vec::new(),
                    None,
                );
                acceptor.set_multitransport_offer(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));

                let framed = TokioFramed::new(server_stream);
                let (_, result) = accept_finalize_with_multitransport(
                    framed,
                    &mut acceptor,
                    recording_handler(handler_calls_for_server),
                )
                .await
                .expect("acceptor finalize with multitransport");

                result
            });

            let client_framed = TokioFramed::new(client_stream);
            let (_client_framed, _user_channel_id, _io_channel_id, request_id) = play_client(client_framed).await;

            let result = server.await.expect("server task panicked");
            assert_eq!(result.static_channels.len(), 0);

            let calls = handler_calls.lock().unwrap();
            assert_eq!(calls.len(), 1, "handler must fire exactly once");
            assert_eq!(
                calls[0].0, request_id,
                "handler must receive the exact request the acceptor sent"
            );
            assert!(
                !calls[0].1,
                "soft-sync was not offered, so it must not be reported negotiated"
            );
        })
        .await;
}

/// Builds a handler for `accept_finalize_with_multitransport` that records
/// every `(request_id, soft_sync)` call into `calls`. Shared by both tests
/// below, which otherwise duplicated the identical `Arc::clone`-into-closure
/// plumbing around an inline handler.
fn recording_handler(calls: Arc<Mutex<Vec<(u32, bool)>>>) -> impl AsyncFnMut(MultitransportRequestPdu, bool) + Clone {
    move |request: MultitransportRequestPdu, soft_sync: bool| {
        let calls = Arc::clone(&calls);
        async move {
            calls.lock().unwrap().push((request.request_id, soft_sync));
        }
    }
}

/// A Deactivation-Reactivation Sequence rebuilds the acceptor via
/// `Acceptor::new_deactivation_reactivation()`, which carries the original
/// Initiate Multitransport Request forward without running bootstrapping
/// again. The handler must not fire a second time when the rebuilt acceptor
/// is driven through `accept_finalize_with_multitransport` for that
/// reactivation round.
#[tokio::test]
async fn multitransport_handler_does_not_fire_again_after_reactivation() {
    let (client_stream, server_stream) = tokio::io::duplex(65536);

    let handler_calls: Arc<Mutex<Vec<(u32, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let handler_calls_for_server = Arc::clone(&handler_calls);

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let server = tokio::task::spawn_local(async move {
                let desktop_size = DesktopSize {
                    width: 1920,
                    height: 1080,
                };
                let mut acceptor = Acceptor::new(SecurityProtocol::empty(), desktop_size, Vec::new(), None);
                acceptor.set_multitransport_offer(Some(MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR));

                let handler = recording_handler(handler_calls_for_server);

                let framed = TokioFramed::new(server_stream);
                let (framed, result) = accept_finalize_with_multitransport(framed, &mut acceptor, handler.clone())
                    .await
                    .expect("acceptor finalize with multitransport (first round)");

                let mut acceptor =
                    Acceptor::new_deactivation_reactivation(acceptor, result.static_channels, desktop_size)
                        .expect("rebuild acceptor for reactivation");

                let (_, result) = accept_finalize_with_multitransport(framed, &mut acceptor, handler)
                    .await
                    .expect("acceptor finalize with multitransport (reactivation round)");

                result
            });

            let client_framed = TokioFramed::new(client_stream);
            let (client_framed, user_channel_id, io_channel_id, _request_id) = play_client(client_framed).await;
            play_reactivation_round(client_framed, user_channel_id, io_channel_id).await;

            let result = server.await.expect("server task panicked");
            assert_eq!(result.static_channels.len(), 0);

            let calls = handler_calls.lock().unwrap();
            assert_eq!(
                calls.len(),
                1,
                "handler must not fire again for a reactivation that carries the same request forward"
            );
        })
        .await;
}
