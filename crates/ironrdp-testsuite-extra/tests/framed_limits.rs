//! How much `Framed::read_by_hint` buffers while the hint cannot find a PDU size.
//!
//! A hint that never decides must not make the framer buffer everything the peer sends,
//! because an unauthenticated peer could then exhaust the server's memory.

use std::io::{self, Read as _};

use ironrdp_core::DecodeResult;
use ironrdp_pdu::{MAX_UNSIZED_BUFFER_LEN, PduHint};
use tokio::io::AsyncReadExt as _;

/// What the peer sends before closing the connection.
///
/// It exceeds the range [`assert_stopped_at_the_limit`] accepts, so a framer that reads everything fails the check.
const PEER_BYTES: usize = 4 * MAX_UNSIZED_BUFFER_LEN;

fn peer_stream_len() -> u64 {
    u64::try_from(PEER_BYTES).expect("PEER_BYTES fits in u64")
}

/// Checks that the framer read at least up to the limit and then stopped.
///
/// The framer checks the limit before each read, so it may overshoot by one read; twice the limit allows for that.
fn assert_stopped_at_the_limit(unread: u64) {
    let read = PEER_BYTES - usize::try_from(unread).expect("unread bytes fit in usize");
    assert!(
        (MAX_UNSIZED_BUFFER_LEN..2 * MAX_UNSIZED_BUFFER_LEN).contains(&read),
        "read {read} bytes, but the framer must stop once {MAX_UNSIZED_BUFFER_LEN} bytes are buffered"
    );
}

/// A hint that never determines a size, as a hint fed input it cannot frame would behave if it did not reject it.
#[derive(Debug)]
struct UndecidedHint;

impl PduHint for UndecidedHint {
    fn find_size(&self, _: &[u8]) -> DecodeResult<Option<(bool, usize)>> {
        Ok(None)
    }
}

// oracle: spec crates/ironrdp-async/src/framed.rs §read_by_hint and crates/ironrdp-pdu/src/lib.rs §MAX_UNSIZED_BUFFER_LEN (fails with InvalidData once the limit is buffered without a size)
#[tokio::test]
async fn async_read_by_hint_stops_buffering_when_the_hint_never_finds_a_size() {
    let mut framed = ironrdp_tokio::TokioFramed::new(tokio::io::repeat(0xAA).take(peer_stream_len()));

    let error = framed.read_by_hint(&UndecidedHint).await.unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_stopped_at_the_limit(framed.get_inner().0.limit());
}

// oracle: spec crates/ironrdp-blocking/src/framed.rs §read_by_hint and crates/ironrdp-pdu/src/lib.rs §MAX_UNSIZED_BUFFER_LEN (fails with InvalidData once the limit is buffered without a size)
#[test]
fn blocking_read_by_hint_stops_buffering_when_the_hint_never_finds_a_size() {
    let mut framed = ironrdp_blocking::Framed::new(io::repeat(0xAA).take(peer_stream_len()));

    let error = framed.read_by_hint(&UndecidedHint).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_stopped_at_the_limit(framed.get_inner().0.limit());
}
