//! How much `Framed::read_by_hint` buffers while the hint cannot find a PDU size.
//!
//! A hint that never decides must not make the framer buffer everything the peer sends,
//! because an unauthenticated peer could then exhaust the server's memory.

use std::io::{self, Read as _};

use ironrdp_core::DecodeResult;
use ironrdp_pdu::PduHint;
use tokio::io::AsyncReadExt as _;

/// What the peer sends before closing the connection, far more than any framer should buffer.
const PEER_BYTES: u64 = 16 * 1024 * 1024;

/// The buffering limit `read_by_hint` documents.
const LIMIT: u64 = 64 * 1024;

/// Checks that the framer read at least up to the limit and then stopped.
///
/// The framer checks the limit before each read, so it may overshoot by one read; twice the limit allows for that.
fn assert_stopped_at_the_limit(unread: u64) {
    let read = PEER_BYTES - unread;
    assert!(
        (LIMIT..2 * LIMIT).contains(&read),
        "read {read} bytes, but the framer must stop once {LIMIT} bytes are buffered"
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

// oracle: spec crates/ironrdp-async/src/framed.rs §read_by_hint (fails with InvalidData once 64 KiB are buffered without a size)
#[tokio::test]
async fn async_read_by_hint_stops_buffering_when_the_hint_never_finds_a_size() {
    let mut framed = ironrdp_tokio::TokioFramed::new(tokio::io::repeat(0xAA).take(PEER_BYTES));

    let error = framed.read_by_hint(&UndecidedHint).await.unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_stopped_at_the_limit(framed.get_inner().0.limit());
}

// oracle: spec crates/ironrdp-blocking/src/framed.rs §read_by_hint (fails with InvalidData once 64 KiB are buffered without a size)
#[test]
fn blocking_read_by_hint_stops_buffering_when_the_hint_never_finds_a_size() {
    let mut framed = ironrdp_blocking::Framed::new(io::repeat(0xAA).take(PEER_BYTES));

    let error = framed.read_by_hint(&UndecidedHint).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_stopped_at_the_limit(framed.get_inner().0.limit());
}
