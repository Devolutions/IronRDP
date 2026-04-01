mod process;

use ironrdp_web_replay::{PduBuffer, PduSource};

#[test]
fn pdu_buffer_empty_returns_nan_timestamp() {
    let buf = PduBuffer::new();
    assert!(buf.peek_timestamp().is_none());
}

#[test]
fn pdu_buffer_push_and_peek_timestamp() {
    let mut buf = PduBuffer::new();
    buf.push_pdu(100.0, PduSource::Server, &[0x01]);
    assert_eq!(buf.peek_timestamp(), Some(100.0));
}

#[test]
fn pdu_buffer_maintains_fifo_order() {
    let mut buf = PduBuffer::new();
    buf.push_pdu(100.0, PduSource::Server, &[0x01]);
    buf.push_pdu(200.0, PduSource::Server, &[0x02]);
    buf.push_pdu(50.0, PduSource::Client, &[0x03]);

    // FIFO order: first pushed = first peeked, regardless of timestamp
    assert_eq!(buf.peek_timestamp(), Some(100.0));
}

#[test]
fn pdu_buffer_peek_source_returns_direction() {
    let mut buf = PduBuffer::new();
    buf.push_pdu(100.0, PduSource::Client, &[0x01]);
    assert_eq!(buf.peek_source(), Some(PduSource::Client));
}

#[test]
fn pdu_buffer_count_tracks_size() {
    let mut buf = PduBuffer::new();
    assert_eq!(buf.count(), 0);

    buf.push_pdu(100.0, PduSource::Server, &[0x01]);
    buf.push_pdu(200.0, PduSource::Server, &[0x02]);
    buf.push_pdu(300.0, PduSource::Client, &[0x03]);
    assert_eq!(buf.count(), 3);
}

#[test]
fn pdu_buffer_clear_empties_buffer() {
    let mut buf = PduBuffer::new();
    buf.push_pdu(100.0, PduSource::Server, &[0x01]);
    buf.push_pdu(200.0, PduSource::Server, &[0x02]);
    assert_eq!(buf.count(), 2);

    buf.clear();
    assert_eq!(buf.count(), 0);
    assert!(buf.peek_timestamp().is_none());
    assert_eq!(buf.peek_source(), None);
}
