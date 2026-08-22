use ironrdp_core::{decode, encode_vec};
use ironrdp_rdpei::pdu::{
    CsReadyFlags, CsReadyPdu, DismissHoveringTouchContactPdu, PenContact, PenContactFlags, PenEventPdu, PenFrame,
    RdpInputProtocolVersion, RdpeiPdu, ScReadyPdu, TouchContact, TouchContactFlags, TouchEventPdu, TouchFrame,
};
use ironrdp_testsuite_core::assert_eq_hex;

#[test]
fn sc_ready_v200_round_trip() {
    let pdu = RdpeiPdu::ScReady(ScReadyPdu::new(RdpInputProtocolVersion::V200));
    let encoded = encode_vec(&pdu).expect("encode SC_READY");
    assert_eq!(decode::<RdpeiPdu>(&encoded).expect("decode SC_READY"), pdu);
    // EVENTID_SC_READY = 1, pduLength includes the 6-byte header, protocolVersion = 0x00020000.
    assert_eq_hex!(
        encoded,
        [
            0x01, 0x00, // eventId
            0x0A, 0x00, 0x00, 0x00, // pduLength = 10
            0x00, 0x00, 0x02, 0x00, // protocolVersion V200
        ]
    );
}

#[test]
fn cs_ready_round_trip() {
    let pdu = RdpeiPdu::CsReady(CsReadyPdu::new(
        CsReadyFlags::SHOW_TOUCH_VISUALS,
        RdpInputProtocolVersion::V200,
        10,
    ));
    let encoded = encode_vec(&pdu).expect("encode CS_READY");
    assert_eq!(decode::<RdpeiPdu>(&encoded).expect("decode CS_READY"), pdu);
}

#[test]
fn touch_event_with_pressure_round_trip() {
    let contact = TouchContact::new(
        1,
        100,
        200,
        TouchContactFlags::DOWN | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT,
    )
    .with_pressure(512);
    let pdu = RdpeiPdu::Touch(TouchEventPdu::new(16, vec![TouchFrame::new(0, vec![contact])]));
    let encoded = encode_vec(&pdu).expect("encode touch");
    assert_eq!(decode::<RdpeiPdu>(&encoded).expect("decode touch"), pdu);
}

#[test]
fn pen_event_round_trip() {
    let contact = PenContact::new(
        0,
        50,
        -20,
        PenContactFlags::UPDATE | PenContactFlags::INRANGE | PenContactFlags::INCONTACT,
    )
    .with_pressure(1024)
    .with_tilt(10, -5);
    let pdu = RdpeiPdu::Pen(PenEventPdu::new(32, vec![PenFrame::new(1000, vec![contact])]));
    let encoded = encode_vec(&pdu).expect("encode pen");
    assert_eq!(decode::<RdpeiPdu>(&encoded).expect("decode pen"), pdu);
}

#[test]
fn suspend_resume_dismiss_round_trip() {
    let suspend = encode_vec(&RdpeiPdu::SuspendInput).expect("encode suspend");
    assert_eq!(
        decode::<RdpeiPdu>(&suspend).expect("decode suspend"),
        RdpeiPdu::SuspendInput
    );
    assert_eq_hex!(suspend, [0x04, 0x00, 0x06, 0x00, 0x00, 0x00]);

    let resume = encode_vec(&RdpeiPdu::ResumeInput).expect("encode resume");
    assert_eq!(
        decode::<RdpeiPdu>(&resume).expect("decode resume"),
        RdpeiPdu::ResumeInput
    );
    assert_eq_hex!(resume, [0x05, 0x00, 0x06, 0x00, 0x00, 0x00]);

    let dismiss = RdpeiPdu::DismissHoveringTouchContact(DismissHoveringTouchContactPdu::new(3));
    let encoded = encode_vec(&dismiss).expect("encode dismiss");
    assert_eq!(decode::<RdpeiPdu>(&encoded).expect("decode dismiss"), dismiss);
    assert_eq_hex!(encoded, [0x06, 0x00, 0x07, 0x00, 0x00, 0x00, 0x03]);
}
