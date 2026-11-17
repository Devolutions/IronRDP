mod process;

use ironrdp_web_replay::PduBuffer;

#[test]
fn pdu_buffer_empty_timestamp_is_none() {
    // oracle: mutant return a timestamp from an empty buffer.
    let buf = PduBuffer::new();
    assert!(buf.peek_timestamp().is_none());
}

#[test]
fn pdu_buffer_empty_timestamp_js_is_nan() {
    // oracle: mutant return a numeric timestamp from an empty JavaScript buffer.
    let buf = PduBuffer::new();
    assert!(buf.peek_timestamp_js().is_nan());
}

#[test]
fn pdu_buffer_peek_source_returns_direction() {
    let mut buf = PduBuffer::new();
    buf.push_pdu(100.0, ironrdp_web_replay::PduSource::Client, &[0x01]);
    buf.push_pdu(200.0, ironrdp_web_replay::PduSource::Server, &[0x02]);
    // oracle: mutant return the direction of the last queued PDU instead of the next PDU.
    assert_eq!(buf.peek_source(), Some(ironrdp_web_replay::PduSource::Client));
}

#[test]
fn pdu_buffer_clear_empties_buffer() {
    // oracle: mutant clear only the front PDU.
    let mut buf = PduBuffer::new();
    buf.push_pdu(100.0, ironrdp_web_replay::PduSource::Server, &[0x01]);
    buf.push_pdu(200.0, ironrdp_web_replay::PduSource::Server, &[0x02]);
    assert_eq!(buf.count(), 2);

    buf.clear();
    assert_eq!(buf.count(), 0);
    assert!(buf.peek_timestamp().is_none());
    assert_eq!(buf.peek_source(), None);
}
