use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_rdpei::pdu::{
    CsReadyFlags, CsReadyPdu, PenContact, PenContactFlags, PenEventPdu, PenFrame, RdpInputProtocolVersion, RdpeiPdu,
    ScReadyFeatures, ScReadyPdu,
};
use ironrdp_rdpei::{RdpeiClient, RdpeiInputState};

const CHANNEL_ID: u32 = 7;

fn multipen_client() -> RdpeiClient {
    RdpeiClient::new(
        10,
        CsReadyFlags::ENABLE_MULTIPEN_INJECTION,
        RdpInputProtocolVersion::V300,
    )
}

fn send(client: &mut RdpeiClient, pdu: RdpeiPdu) -> Option<CsReadyPdu> {
    let payload = encode_vec(&pdu).expect("encode server PDU");
    let replies = client.process(CHANNEL_ID, &payload).expect("process server PDU");
    let reply = replies.first()?;
    match decode::<RdpeiPdu>(&encode_vec(reply.as_ref()).expect("encode reply")).expect("decode reply") {
        RdpeiPdu::CsReady(cs_ready) => Some(cs_ready),
        other => panic!("unexpected reply {other:?}"),
    }
}

fn pen_event(device_id: u8) -> PenEventPdu {
    let contact = PenContact::new(device_id, 0, 0, PenContactFlags::UPDATE | PenContactFlags::INRANGE);
    PenEventPdu::new(0, vec![PenFrame::new(0, vec![contact])])
}

// oracle: spec MS-RDPEI §2.2.3.2
#[test]
fn multipen_is_enabled_when_the_server_supports_it() {
    let mut client = multipen_client();
    let sc_ready =
        ScReadyPdu::new(RdpInputProtocolVersion::V300).with_features(ScReadyFeatures::MULTIPEN_INJECTION_SUPPORTED);
    let cs_ready = send(&mut client, RdpeiPdu::ScReady(sc_ready)).expect("CS_READY");
    assert_eq!(cs_ready.protocol_version, RdpInputProtocolVersion::V300);
    assert!(cs_ready.flags.contains(CsReadyFlags::ENABLE_MULTIPEN_INJECTION));
    assert_eq!(
        client.input_state(),
        RdpeiInputState {
            touch: true,
            pen: true,
            multipen: true,
            suspended: false,
        }
    );
    client
        .encode_pen_event(CHANNEL_ID, pen_event(3))
        .expect("second pen device");
}

// oracle: spec MS-RDPEI §2.2.3.7.1.1
#[test]
fn without_the_server_feature_pens_use_device_zero() {
    let mut client = multipen_client();
    let cs_ready = send(
        &mut client,
        RdpeiPdu::ScReady(ScReadyPdu::new(RdpInputProtocolVersion::V300)),
    )
    .expect("CS_READY");
    assert!(!cs_ready.flags.contains(CsReadyFlags::ENABLE_MULTIPEN_INJECTION));
    assert!(!client.input_state().multipen);
    client.encode_pen_event(CHANNEL_ID, pen_event(0)).expect("device zero");
    client
        .encode_pen_event(CHANNEL_ID, pen_event(1))
        .expect_err("nonzero deviceId without multipen");
}

// oracle: spec MS-RDPEI §2.2.3.2
#[test]
fn a_v200_server_gets_v200_and_pen_without_multipen() {
    let mut client = multipen_client();
    let cs_ready = send(
        &mut client,
        RdpeiPdu::ScReady(ScReadyPdu::new(RdpInputProtocolVersion::V200)),
    )
    .expect("CS_READY");
    assert_eq!(cs_ready.protocol_version, RdpInputProtocolVersion::V200);
    assert!(!cs_ready.flags.contains(CsReadyFlags::ENABLE_MULTIPEN_INJECTION));
    assert_eq!(
        client.input_state(),
        RdpeiInputState {
            touch: true,
            pen: true,
            multipen: false,
            suspended: false,
        }
    );
}

// oracle: spec MS-RDPEI §3.3.5.1
#[test]
fn a_v100_server_accepts_touch_only() {
    let mut client = multipen_client();
    send(
        &mut client,
        RdpeiPdu::ScReady(ScReadyPdu::new(RdpInputProtocolVersion::V100)),
    )
    .expect("CS_READY");
    assert_eq!(
        client.input_state(),
        RdpeiInputState {
            touch: true,
            pen: false,
            multipen: false,
            suspended: false,
        }
    );
}

// oracle: spec MS-RDPEI §3.3.5.4
#[test]
fn suspend_stops_and_resume_restarts_input() {
    let mut client = multipen_client();
    send(
        &mut client,
        RdpeiPdu::ScReady(ScReadyPdu::new(RdpInputProtocolVersion::V200)),
    )
    .expect("CS_READY");
    assert_eq!(send(&mut client, RdpeiPdu::SuspendInput), None);
    let suspended = client.input_state();
    assert!(suspended.suspended);
    assert!(suspended.touch && suspended.pen, "suspension keeps the negotiation");
    assert_eq!(send(&mut client, RdpeiPdu::ResumeInput), None);
    assert!(!client.input_state().suspended);
}

// oracle: spec MS-RDPEI §3.3.5.1
#[test]
fn closing_the_channel_forgets_the_negotiation() {
    let mut client = multipen_client();
    send(
        &mut client,
        RdpeiPdu::ScReady(ScReadyPdu::new(RdpInputProtocolVersion::V200)),
    )
    .expect("CS_READY");
    client.close(CHANNEL_ID);
    assert_eq!(client.input_state(), RdpeiInputState::default());
}
