mod client;
mod contact;

use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_rdpei::pdu::{
    CsReadyFlags, CsReadyPdu, DismissHoveringTouchContactPdu, PenContact, PenContactFlags, PenEventPdu, PenFrame,
    RdpInputProtocolVersion, RdpeiPdu, ScReadyPdu, TouchContact, TouchContactFlags, TouchEventPdu, TouchFrame,
};
use ironrdp_rdpei::{RdpeiHandler, RdpeiServer};
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

struct CloseTrackingHandler {
    closed: Arc<AtomicBool>,
}

impl RdpeiHandler for CloseTrackingHandler {
    fn close(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

// oracle: mutant omitting RdpeiHandler::close when the RDPEI dynamic channel closes.
#[test]
fn server_close_notifies_handler() {
    let closed = Arc::new(AtomicBool::new(false));
    let mut server = RdpeiServer::new(Box::new(CloseTrackingHandler {
        closed: Arc::clone(&closed),
    }));

    server.start(7).expect("start RDPEI server");
    assert!(!closed.load(Ordering::Relaxed));

    server.close(7);
    assert!(closed.load(Ordering::Relaxed));
}

// oracle: mutant leaving RdpeiServer without a 64 KiB DVC message size bound.
#[test]
fn rdpei_server_bounds_inbound_dvc_messages() {
    let server = RdpeiServer::new(Box::new(CloseTrackingHandler {
        closed: Arc::new(AtomicBool::new(false)),
    }));
    assert_eq!(server.max_message_size(), Some(64 * 1024));
}

// oracle: spec MS-RDPEI §2.2.3.3, §2.2.3.3.1, §2.2.3.7, §2.2.3.7.1
#[test]
fn touch_and_pen_decoders_reject_truncated_counts() {
    // Truncated TouchFrame / PenFrame claiming 0x7FFF contacts with no contact bytes left.
    assert!(decode::<TouchFrame>(&[0xFF, 0xFF, 0x00]).is_err());
    assert!(decode::<PenFrame>(&[0xFF, 0xFF, 0x00]).is_err());

    // Truncated TouchEventPdu / PenEventPdu claiming 0x7FFF frames with no frame bytes left.
    assert!(decode::<TouchEventPdu>(&[0x03, 0x00, 0x09, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF]).is_err());
    assert!(decode::<PenEventPdu>(&[0x08, 0x00, 0x09, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF]).is_err());
}

// oracle: mutant under-sizing the minimum contact or frame wire size in Vec::with_capacity.
#[test]
fn touch_and_pen_decoders_preallocate_exact_capacity_for_minimal_items() {
    let touch_contacts = (0..3)
        .map(|id| TouchContact::new(id, 0, 0, TouchContactFlags::UP))
        .collect();
    let touch_frame = TouchFrame::new(0, touch_contacts);
    let decoded_touch_frame: TouchFrame =
        decode(&encode_vec(&touch_frame).expect("encode touch frame")).expect("decode touch frame");
    assert_eq!(decoded_touch_frame.contacts.capacity(), 3);

    let touch_event = TouchEventPdu::new(
        0,
        vec![
            TouchFrame::new(0, Vec::new()),
            TouchFrame::new(1, Vec::new()),
            TouchFrame::new(2, Vec::new()),
        ],
    );
    let decoded_touch: TouchEventPdu = decode(&encode_vec(&touch_event).expect("encode touch event")).expect("decode");
    assert_eq!(decoded_touch.frames.capacity(), 3);

    let pen_contacts = (0..3)
        .map(|id| PenContact::new(id, 0, 0, PenContactFlags::UP))
        .collect();
    let pen_frame = PenFrame::new(0, pen_contacts);
    let decoded_pen_frame: PenFrame =
        decode(&encode_vec(&pen_frame).expect("encode pen frame")).expect("decode pen frame");
    assert_eq!(decoded_pen_frame.contacts.capacity(), 3);

    let pen_event = PenEventPdu::new(
        0,
        vec![
            PenFrame::new(0, Vec::new()),
            PenFrame::new(1, Vec::new()),
            PenFrame::new(2, Vec::new()),
        ],
    );
    let decoded_pen: PenEventPdu = decode(&encode_vec(&pen_event).expect("encode pen event")).expect("decode");
    assert_eq!(decoded_pen.frames.capacity(), 3);
}
