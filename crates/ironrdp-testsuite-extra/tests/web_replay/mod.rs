mod process;

use ironrdp_web_replay::PduBuffer;
use rstest::rstest;

#[test]
fn pdu_buffer_empty_timestamp_is_none() {
    // oracle: mutant The mutant returns a timestamp from an empty buffer.
    let buf = PduBuffer::new();
    assert!(buf.peek_timestamp().is_none());
}

#[rstest]
#[case::nan(f64::NAN)]
#[case::positive_infinity(f64::INFINITY)]
#[case::negative_infinity(f64::NEG_INFINITY)]
fn pdu_buffer_rejects_non_finite_timestamps(#[case] timestamp_ms: f64) {
    // oracle: mutant The mutant queues a non-finite timestamp and stalls FIFO processing.
    let mut buf = PduBuffer::new();
    assert!(
        buf.push_pdu(timestamp_ms, ironrdp_web_replay::PduSource::Client, &[0x01])
            .is_err(),
        "non-finite timestamp should be rejected"
    );
    assert_eq!(buf.count(), 0);
}

#[test]
fn pdu_buffer_peek_source_returns_direction() {
    let mut buf = PduBuffer::new();
    buf.push_pdu(100.0, ironrdp_web_replay::PduSource::Client, &[0x01])
        .unwrap();
    buf.push_pdu(200.0, ironrdp_web_replay::PduSource::Server, &[0x02])
        .unwrap();
    // oracle: mutant The mutant returns the direction of the last queued PDU instead of the next PDU.
    assert_eq!(buf.peek_source(), Some(ironrdp_web_replay::PduSource::Client));
}

#[test]
fn pdu_buffer_clear_empties_buffer() {
    // oracle: mutant The mutant clears only the front PDU.
    let mut buf = PduBuffer::new();
    buf.push_pdu(100.0, ironrdp_web_replay::PduSource::Server, &[0x01])
        .unwrap();
    buf.push_pdu(200.0, ironrdp_web_replay::PduSource::Server, &[0x02])
        .unwrap();
    assert_eq!(buf.count(), 2);

    buf.clear();
    assert_eq!(buf.count(), 0);
    assert!(buf.peek_timestamp().is_none());
    assert_eq!(buf.peek_source(), None);
}
