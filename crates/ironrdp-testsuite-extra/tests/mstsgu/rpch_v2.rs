use core::time::Duration;

use ironrdp_mstsgu::rpc::{
    PFC_FIRST_FRAG, PFC_LAST_FRAG, PTYPE_RTS, RPCH_OUT_CONTENT_LENGTH, RpcPduError, RpchFlowControlError,
    RpchInChannelRecycleState, RpchOutChannelRecycleState, RpchV2Error, RpchV2Settings, RpchV2Setup, RpchV2State,
    RtsCookie, RtsFlowControlAck, decode_rts_flow_control_ack, decode_rts_in_recycle_a4, decode_rts_out_recycle_a2,
    decode_rts_out_recycle_a6, decode_rts_out_recycle_a10, encode_rts_conn_a1, encode_rts_conn_b1,
    encode_rts_flow_control_ack, encode_rts_in_recycle_a1, encode_rts_in_recycle_a5, encode_rts_out_recycle_a3,
    encode_rts_out_recycle_a7, encode_rts_out_recycle_a11, encode_rts_ping,
};

const VIRTUAL_CONNECTION_COOKIE: RtsCookie = RtsCookie::new([0x10; RtsCookie::SIZE]);
const OUT_CHANNEL_COOKIE: RtsCookie = RtsCookie::new([0x20; RtsCookie::SIZE]);
const IN_CHANNEL_COOKIE: RtsCookie = RtsCookie::new([0x30; RtsCookie::SIZE]);
const ASSOCIATION_GROUP_ID: RtsCookie = RtsCookie::new([0x40; RtsCookie::SIZE]);
const RECEIVE_WINDOW_SIZE: u32 = 128 * 1024;
const CONNECTION_TIMEOUT: u32 = 120_000;
const IN_RECYCLE_CONNECTION_TIMEOUT: u32 = 240_000;

#[test]
fn conn_a1_and_conn_b1_encode_exact_rts_vectors() {
    let expected_out_request = [
        [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
        &76u16.to_le_bytes(),
        &0u16.to_le_bytes(),
        &0u32.to_le_bytes(),
        &0u16.to_le_bytes(),
        &4u16.to_le_bytes(),
        &6u32.to_le_bytes(),
        &1u32.to_le_bytes(),
        &3u32.to_le_bytes(),
        VIRTUAL_CONNECTION_COOKIE.as_bytes(),
        &3u32.to_le_bytes(),
        OUT_CHANNEL_COOKIE.as_bytes(),
        &0u32.to_le_bytes(),
        &RECEIVE_WINDOW_SIZE.to_le_bytes(),
    ]
    .concat();
    assert_eq!(expected_out_request.len(), RPCH_OUT_CONTENT_LENGTH);
    assert_eq!(
        encode_rts_conn_a1(VIRTUAL_CONNECTION_COOKIE, OUT_CHANNEL_COOKIE, RECEIVE_WINDOW_SIZE).expect("CONN/A1"),
        expected_out_request
    );

    let expected_in_request = [
        [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
        &104u16.to_le_bytes(),
        &0u16.to_le_bytes(),
        &0u32.to_le_bytes(),
        &0u16.to_le_bytes(),
        &6u16.to_le_bytes(),
        &6u32.to_le_bytes(),
        &1u32.to_le_bytes(),
        &3u32.to_le_bytes(),
        VIRTUAL_CONNECTION_COOKIE.as_bytes(),
        &3u32.to_le_bytes(),
        IN_CHANNEL_COOKIE.as_bytes(),
        &4u32.to_le_bytes(),
        &(256 * 1024u32).to_le_bytes(),
        &5u32.to_le_bytes(),
        &0u32.to_le_bytes(),
        &12u32.to_le_bytes(),
        ASSOCIATION_GROUP_ID.as_bytes(),
    ]
    .concat();
    assert_eq!(
        encode_rts_conn_b1(
            VIRTUAL_CONNECTION_COOKIE,
            IN_CHANNEL_COOKIE,
            256 * 1024,
            0,
            ASSOCIATION_GROUP_ID,
        )
        .expect("CONN/B1"),
        expected_in_request
    );
}

#[test]
fn setup_opens_after_conn_a3_and_conn_c2_while_ignoring_received_version() {
    let mut setup = setup(0);
    setup.start_in_request().expect("start IN request");
    assert_eq!(setup.state(), RpchV2State::InRequestStarted);
    let _ = setup.out_request_body().expect("CONN/A1");
    let _ = setup.in_request_initial_pdu().expect("CONN/B1");
    setup.accept_out_response(200).expect("successful OUT response");

    setup.receive_out_pdu(&conn_a3()).expect("CONN/A3");
    assert_eq!(setup.state(), RpchV2State::AwaitingC2);
    assert_eq!(setup.in_channel_ping_timeout(), Some(CONNECTION_TIMEOUT));

    setup.receive_out_pdu(&conn_c2(2)).expect("CONN/C2 Version is ignored");
    assert_eq!(setup.state(), RpchV2State::Open);
    assert_eq!(setup.connection_timeout(), Some(CONNECTION_TIMEOUT));
    assert_eq!(setup.peer_receive_window_size(), Some(RECEIVE_WINDOW_SIZE));
}

#[test]
fn setup_rejects_malformed_rts_and_wrong_sequence() {
    let mut invalid_setup = setup(0);
    assert_eq!(
        invalid_setup.in_request_initial_pdu(),
        Err(RpchV2Error::InvalidState {
            action: "send CONN/B1",
            state: RpchV2State::Initial,
        })
    );
    assert_eq!(invalid_setup.state(), RpchV2State::Failed);

    let mut opened = opened_setup(0);
    let mut a3 = conn_a3();
    a3[3] |= 0x04;
    assert_eq!(
        opened.receive_out_pdu(&a3),
        Err(RpchV2Error::InvalidState {
            action: "consume an OUT setup PDU",
            state: RpchV2State::Open,
        })
    );

    let mut setup = setup(0);
    setup.start_in_request().expect("start IN request");
    let _ = setup.out_request_body().expect("CONN/A1");
    let _ = setup.in_request_initial_pdu().expect("CONN/B1");
    setup.accept_out_response(200).expect("successful OUT response");
    let mut a3 = conn_a3();
    a3[3] |= 0x04;
    assert_eq!(
        setup.receive_out_pdu(&a3),
        Err(RpchV2Error::Rts(RpcPduError::InvalidRtsPfcFlags {
            actual: PFC_FIRST_FRAG | PFC_LAST_FRAG | 0x04,
        }))
    );
    assert_eq!(setup.state(), RpchV2State::Failed);
}

#[test]
fn ping_uses_half_keepalive_or_connection_timeout_and_has_an_exact_vector() {
    let mut schedule = opened_setup(0)
        .ping_schedule(Duration::from_secs(60), Duration::ZERO)
        .expect("opened setup has a ping schedule");
    assert!(!schedule.ping_due(Duration::from_secs(29)));
    assert!(schedule.ping_due(Duration::from_secs(30)));
    schedule.record_send(Duration::from_secs(30));
    assert!(!schedule.ping_due(Duration::from_secs(59)));
    assert!(schedule.ping_due(Duration::from_secs(60)));

    let connection_timeout = opened_setup(60_000)
        .ping_schedule(Duration::ZERO, Duration::ZERO)
        .expect("opened setup has a ping schedule");
    assert!(!connection_timeout.ping_due(Duration::from_secs(119)));
    assert!(connection_timeout.ping_due(Duration::from_secs(120)));

    let expected_ping = [
        [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
        &20u16.to_le_bytes(),
        &0u16.to_le_bytes(),
        &0u32.to_le_bytes(),
        &1u16.to_le_bytes(),
        &0u16.to_le_bytes(),
    ]
    .concat();
    assert_eq!(encode_rts_ping().expect("PING"), expected_ping);
}

#[test]
fn flow_control_accounts_windows_and_encodes_acknowledgements() {
    let mut flow_control = opened_setup(0).flow_control().expect("open flow control");
    flow_control.sent_rpc_pdu(64 * 1024).expect("send within peer window");
    assert_eq!(flow_control.send_available_window(), 64 * 1024);
    assert_eq!(
        flow_control.receive_flow_control_ack(RtsFlowControlAck::new(
            64 * 1024,
            RECEIVE_WINDOW_SIZE,
            IN_CHANNEL_COOKIE,
        )),
        Ok(true)
    );
    assert_eq!(flow_control.send_available_window(), RECEIVE_WINDOW_SIZE);
    assert_eq!(
        flow_control.receive_flow_control_ack(RtsFlowControlAck::new(
            0,
            RECEIVE_WINDOW_SIZE,
            RtsCookie::new([0xff; RtsCookie::SIZE]),
        )),
        Ok(false)
    );
    assert_eq!(
        flow_control.receive_flow_control_ack(RtsFlowControlAck::new(0, RECEIVE_WINDOW_SIZE, IN_CHANNEL_COOKIE)),
        Err(RpchFlowControlError::RegressingFlowControlAck {
            bytes_received: 0,
            previous_bytes_received: 64 * 1024,
        })
    );

    let mut exact_half_flow_control = opened_setup(0).flow_control().expect("open flow control");
    exact_half_flow_control
        .received_rpc_pdu(64 * 1024)
        .expect("queue received PDU");
    assert_eq!(
        exact_half_flow_control.consumed_rpc_pdu(64 * 1024),
        Ok(Some(RtsFlowControlAck::new(
            64 * 1024,
            RECEIVE_WINDOW_SIZE,
            OUT_CHANNEL_COOKIE,
        )))
    );

    flow_control.received_rpc_pdu(100 * 1024).expect("queue received PDU");
    let acknowledgement = flow_control
        .consumed_rpc_pdu(100 * 1024)
        .expect("consume received PDU")
        .expect("reclaimed more than half the receive window");
    assert_eq!(
        acknowledgement,
        RtsFlowControlAck::new(100 * 1024, RECEIVE_WINDOW_SIZE, OUT_CHANNEL_COOKIE)
    );

    let expected_ack = [
        [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
        &48u16.to_le_bytes(),
        &0u16.to_le_bytes(),
        &0u32.to_le_bytes(),
        &2u16.to_le_bytes(),
        &1u16.to_le_bytes(),
        &1u32.to_le_bytes(),
        &(100 * 1024u32).to_le_bytes(),
        &RECEIVE_WINDOW_SIZE.to_le_bytes(),
        OUT_CHANNEL_COOKIE.as_bytes(),
    ]
    .concat();
    let encoded_ack = encode_rts_flow_control_ack(acknowledgement).expect("flow-control ACK");
    assert_eq!(encoded_ack, expected_ack);
    assert_eq!(
        decode_rts_flow_control_ack(&encoded_ack).expect("decode flow-control ACK"),
        acknowledgement
    );
}

#[test]
fn channel_recycling_encodes_exact_r1_vectors_and_updates_defaults() {
    let successor_in_channel_cookie = RtsCookie::new([0x50; RtsCookie::SIZE]);
    let successor_out_channel_cookie = RtsCookie::new([0x60; RtsCookie::SIZE]);
    let mut setup = opened_setup(0);
    let mut flow_control = setup.flow_control().expect("open flow control");
    let mut ping_schedule = setup
        .ping_schedule(Duration::ZERO, Duration::ZERO)
        .expect("open ping schedule");
    {
        let mut recycling = setup.channel_recycling().expect("open setup");

        let expected_in_a1 = [
            [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &88u16.to_le_bytes(),
            &0u16.to_le_bytes(),
            &0u32.to_le_bytes(),
            &4u16.to_le_bytes(),
            &4u16.to_le_bytes(),
            &6u32.to_le_bytes(),
            &1u32.to_le_bytes(),
            &3u32.to_le_bytes(),
            VIRTUAL_CONNECTION_COOKIE.as_bytes(),
            &3u32.to_le_bytes(),
            IN_CHANNEL_COOKIE.as_bytes(),
            &3u32.to_le_bytes(),
            successor_in_channel_cookie.as_bytes(),
        ]
        .concat();
        assert_eq!(
            recycling
                .start_in_recycling(successor_in_channel_cookie)
                .expect("IN_R1/A1"),
            expected_in_a1
        );
        assert_eq!(recycling.in_state(), RpchInChannelRecycleState::AwaitingA4);

        let in_a4 = [
            [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &52u16.to_le_bytes(),
            &0u16.to_le_bytes(),
            &0u32.to_le_bytes(),
            &0u16.to_le_bytes(),
            &4u16.to_le_bytes(),
            &13u32.to_le_bytes(),
            &0u32.to_le_bytes(),
            &6u32.to_le_bytes(),
            &1u32.to_le_bytes(),
            &0u32.to_le_bytes(),
            &(8 * 1024u32).to_le_bytes(),
            &2u32.to_le_bytes(),
            &IN_RECYCLE_CONNECTION_TIMEOUT.to_le_bytes(),
        ]
        .concat();
        let decoded_a4 = decode_rts_in_recycle_a4(&in_a4).expect("IN_R1/A4");
        assert_eq!(decoded_a4.version(), 1);
        assert_eq!(decoded_a4.receive_window_size(), 8 * 1024);
        assert_eq!(decoded_a4.connection_timeout(), IN_RECYCLE_CONNECTION_TIMEOUT);

        let expected_in_a5 = [
            [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &40u16.to_le_bytes(),
            &0u16.to_le_bytes(),
            &0u32.to_le_bytes(),
            &0u16.to_le_bytes(),
            &1u16.to_le_bytes(),
            &3u32.to_le_bytes(),
            successor_in_channel_cookie.as_bytes(),
        ]
        .concat();
        assert_eq!(
            recycling.receive_in_recycle_a4(&in_a4).expect("IN_R1/A5"),
            expected_in_a5
        );
        assert_eq!(recycling.in_state(), RpchInChannelRecycleState::AwaitingA5);
        assert_eq!(recycling.default_in_channel_cookie(), successor_in_channel_cookie);
        assert_eq!(recycling.peer_receive_window_size(), 8 * 1024);
        recycling
            .finish_in_recycling(&mut flow_control, &mut ping_schedule)
            .expect("IN_R1/A5 sent");
        assert_eq!(recycling.in_state(), RpchInChannelRecycleState::Open);
        flow_control
            .sent_rpc_pdu(4 * 1024)
            .expect("send on successor IN channel");
        assert_eq!(
            flow_control.receive_flow_control_ack(RtsFlowControlAck::new(
                4 * 1024,
                8 * 1024,
                successor_in_channel_cookie,
            )),
            Ok(true)
        );
        assert!(!ping_schedule.ping_due(Duration::from_secs(239)));
        assert!(ping_schedule.ping_due(Duration::from_secs(240)));

        let out_a2 = [
            [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &28u16.to_le_bytes(),
            &0u16.to_le_bytes(),
            &0u32.to_le_bytes(),
            &4u16.to_le_bytes(),
            &1u16.to_le_bytes(),
            &13u32.to_le_bytes(),
            &0u32.to_le_bytes(),
        ]
        .concat();
        decode_rts_out_recycle_a2(&out_a2).expect("OUT_R1/A2");
        let expected_out_a3 = [
            [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &96u16.to_le_bytes(),
            &0u16.to_le_bytes(),
            &0u32.to_le_bytes(),
            &4u16.to_le_bytes(),
            &5u16.to_le_bytes(),
            &6u32.to_le_bytes(),
            &1u32.to_le_bytes(),
            &3u32.to_le_bytes(),
            VIRTUAL_CONNECTION_COOKIE.as_bytes(),
            &3u32.to_le_bytes(),
            OUT_CHANNEL_COOKIE.as_bytes(),
            &3u32.to_le_bytes(),
            successor_out_channel_cookie.as_bytes(),
            &0u32.to_le_bytes(),
            &RECEIVE_WINDOW_SIZE.to_le_bytes(),
        ]
        .concat();
        assert_eq!(
            recycling
                .receive_out_recycle_a2(&out_a2, successor_out_channel_cookie)
                .expect("OUT_R1/A3"),
            expected_out_a3
        );
        assert_eq!(recycling.out_state(), RpchOutChannelRecycleState::AwaitingA6);

        let out_a6 = [
            [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &44u16.to_le_bytes(),
            &0u16.to_le_bytes(),
            &0u32.to_le_bytes(),
            &0x10u16.to_le_bytes(),
            &3u16.to_le_bytes(),
            &13u32.to_le_bytes(),
            &0u32.to_le_bytes(),
            &6u32.to_le_bytes(),
            &1u32.to_le_bytes(),
            &2u32.to_le_bytes(),
            &CONNECTION_TIMEOUT.to_le_bytes(),
        ]
        .concat();
        let decoded_a6 = decode_rts_out_recycle_a6(&out_a6).expect("OUT_R1/A6");
        assert_eq!(decoded_a6.version(), 1);
        assert_eq!(decoded_a6.connection_timeout(), CONNECTION_TIMEOUT);

        let expected_out_a7 = [
            [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &48u16.to_le_bytes(),
            &0u16.to_le_bytes(),
            &0u32.to_le_bytes(),
            &0x10u16.to_le_bytes(),
            &2u16.to_le_bytes(),
            &13u32.to_le_bytes(),
            &2u32.to_le_bytes(),
            &3u32.to_le_bytes(),
            successor_out_channel_cookie.as_bytes(),
        ]
        .concat();
        assert_eq!(
            recycling.receive_out_recycle_a6(&out_a6).expect("OUT_R1/A7"),
            expected_out_a7
        );
        assert_eq!(recycling.out_state(), RpchOutChannelRecycleState::AwaitingA10);

        let out_a10 = [
            [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &24u16.to_le_bytes(),
            &0u16.to_le_bytes(),
            &0u32.to_le_bytes(),
            &0u16.to_le_bytes(),
            &1u16.to_le_bytes(),
            &10u32.to_le_bytes(),
        ]
        .concat();
        decode_rts_out_recycle_a10(&out_a10).expect("OUT_R1/A10");
        assert_eq!(
            recycling.receive_out_recycle_a10(&out_a10).expect("OUT_R1/A11"),
            out_a10
        );
        assert_eq!(recycling.out_state(), RpchOutChannelRecycleState::AwaitingA11);
        assert_eq!(recycling.default_out_channel_cookie(), successor_out_channel_cookie);
        recycling
            .finish_out_recycling(&mut flow_control)
            .expect("OUT_R1/A11 sent");
        assert_eq!(recycling.out_state(), RpchOutChannelRecycleState::Open);
        flow_control
            .received_rpc_pdu(64 * 1024)
            .expect("receive on successor OUT channel");
        assert_eq!(
            flow_control.consumed_rpc_pdu(64 * 1024),
            Ok(Some(RtsFlowControlAck::new(
                64 * 1024,
                RECEIVE_WINDOW_SIZE,
                successor_out_channel_cookie,
            )))
        );
        assert_eq!(
            encode_rts_in_recycle_a1(
                VIRTUAL_CONNECTION_COOKIE,
                IN_CHANNEL_COOKIE,
                successor_in_channel_cookie,
            )
            .expect("IN_R1/A1"),
            expected_in_a1
        );
        assert_eq!(
            encode_rts_in_recycle_a5(successor_in_channel_cookie).expect("IN_R1/A5"),
            expected_in_a5
        );
        assert_eq!(
            encode_rts_out_recycle_a3(
                VIRTUAL_CONNECTION_COOKIE,
                OUT_CHANNEL_COOKIE,
                successor_out_channel_cookie,
                RECEIVE_WINDOW_SIZE,
            )
            .expect("OUT_R1/A3"),
            expected_out_a3
        );
        assert_eq!(
            encode_rts_out_recycle_a7(successor_out_channel_cookie).expect("OUT_R1/A7"),
            expected_out_a7
        );
        assert_eq!(encode_rts_out_recycle_a11().expect("OUT_R1/A11"), out_a10);
    }
    assert_eq!(setup.in_channel_ping_timeout(), Some(IN_RECYCLE_CONNECTION_TIMEOUT));
    assert_eq!(setup.peer_receive_window_size(), Some(8 * 1024));
    assert_eq!(
        setup
            .flow_control()
            .expect("updated flow control")
            .send_available_window(),
        8 * 1024
    );
}

#[test]
fn channel_recycling_rejects_invalid_order_and_destinations() {
    let mut setup = opened_setup(0);
    let mut recycling = setup.channel_recycling().expect("open setup");
    let out_a2 = [
        [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
        &28u16.to_le_bytes(),
        &0u16.to_le_bytes(),
        &0u32.to_le_bytes(),
        &4u16.to_le_bytes(),
        &1u16.to_le_bytes(),
        &13u32.to_le_bytes(),
        &1u32.to_le_bytes(),
    ]
    .concat();
    assert_eq!(
        recycling.receive_out_recycle_a2(&out_a2, RtsCookie::new([0x60; RtsCookie::SIZE])),
        Err(ironrdp_mstsgu::rpc::RpchChannelRecycleError::Rts(
            RpcPduError::UnexpectedRtsDestination { expected: 0, actual: 1 }
        ))
    );
    assert_eq!(recycling.out_state(), RpchOutChannelRecycleState::Failed);
    assert_eq!(recycling.in_state(), RpchInChannelRecycleState::Failed);
    assert_eq!(
        recycling.start_in_recycling(RtsCookie::new([0x50; RtsCookie::SIZE])),
        Err(ironrdp_mstsgu::rpc::RpchChannelRecycleError::InvalidInState {
            action: "start IN channel recycling",
            state: RpchInChannelRecycleState::Failed,
        })
    );

    let mut setup = opened_setup(0);
    let mut recycling = setup.channel_recycling().expect("open setup");
    assert_eq!(
        recycling.receive_in_recycle_a4(&[]),
        Err(ironrdp_mstsgu::rpc::RpchChannelRecycleError::InvalidInState {
            action: "accept IN_R1/A4",
            state: RpchInChannelRecycleState::Open,
        })
    );
    assert_eq!(recycling.in_state(), RpchInChannelRecycleState::Failed);
    assert_eq!(recycling.out_state(), RpchOutChannelRecycleState::Failed);
    assert_eq!(
        recycling.receive_out_recycle_a2(&[], RtsCookie::new([0x60; RtsCookie::SIZE])),
        Err(ironrdp_mstsgu::rpc::RpchChannelRecycleError::InvalidOutState {
            action: "accept OUT_R1/A2",
            state: RpchOutChannelRecycleState::Failed,
        })
    );
}

fn setup(client_keepalive: u32) -> RpchV2Setup {
    RpchV2Setup::new(
        RpchV2Settings::new(RECEIVE_WINDOW_SIZE, 256 * 1024, client_keepalive).expect("valid settings"),
        VIRTUAL_CONNECTION_COOKIE,
        OUT_CHANNEL_COOKIE,
        IN_CHANNEL_COOKIE,
        ASSOCIATION_GROUP_ID,
    )
}

fn opened_setup(client_keepalive: u32) -> RpchV2Setup {
    let mut setup = setup(client_keepalive);
    setup.start_in_request().expect("start IN request");
    let _ = setup.out_request_body().expect("CONN/A1");
    let _ = setup.in_request_initial_pdu().expect("CONN/B1");
    setup.accept_out_response(200).expect("successful OUT response");
    setup.receive_out_pdu(&conn_a3()).expect("CONN/A3");
    setup.receive_out_pdu(&conn_c2(1)).expect("CONN/C2");
    setup
}

fn conn_a3() -> Vec<u8> {
    [
        [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
        &28u16.to_le_bytes(),
        &0u16.to_le_bytes(),
        &0u32.to_le_bytes(),
        &0u16.to_le_bytes(),
        &1u16.to_le_bytes(),
        &2u32.to_le_bytes(),
        &CONNECTION_TIMEOUT.to_le_bytes(),
    ]
    .concat()
}

fn conn_c2(version: u32) -> Vec<u8> {
    [
        [5, 0, PTYPE_RTS, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
        &44u16.to_le_bytes(),
        &0u16.to_le_bytes(),
        &0u32.to_le_bytes(),
        &0u16.to_le_bytes(),
        &3u16.to_le_bytes(),
        &6u32.to_le_bytes(),
        &version.to_le_bytes(),
        &0u32.to_le_bytes(),
        &RECEIVE_WINDOW_SIZE.to_le_bytes(),
        &2u32.to_le_bytes(),
        &CONNECTION_TIMEOUT.to_le_bytes(),
    ]
    .concat()
}
