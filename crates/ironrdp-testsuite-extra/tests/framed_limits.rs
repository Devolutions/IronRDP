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

#[expect(
    clippy::as_conversions,
    reason = "usize to u64 is lossless on every supported target"
)]
const PEER_STREAM_LEN: u64 = PEER_BYTES as u64;

/// Checks that the framer read at least up to the limit and then stopped.
///
/// The framer checks the limit before each read, so it may overshoot by one read; twice the limit allows for that as
/// long as no single read adds more than the limit.
/// The blocking framer reads 1024 bytes at a time, and the async framer reads into the spare capacity of a buffer that
/// grows by doubling, so a read started below the limit ends below twice the limit.
/// A framer that checks the limit only after reading also stays within this range, so this check cannot detect it.
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

// oracle: mutant async read_by_hint buffers without bound when the hint never decides (limit check removed)
#[tokio::test]
async fn async_read_by_hint_stops_buffering_when_the_hint_never_finds_a_size() {
    let mut framed = ironrdp_tokio::TokioFramed::new(tokio::io::repeat(0xAA).take(PEER_STREAM_LEN));

    let error = framed.read_by_hint(&UndecidedHint).await.unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_stopped_at_the_limit(framed.get_inner().0.limit());
}

// oracle: mutant blocking read_by_hint buffers without bound when the hint never decides (limit check removed)
#[test]
fn blocking_read_by_hint_stops_buffering_when_the_hint_never_finds_a_size() {
    let mut framed = ironrdp_blocking::Framed::new(io::repeat(0xAA).take(PEER_STREAM_LEN));

    let error = framed.read_by_hint(&UndecidedHint).unwrap_err();

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert_stopped_at_the_limit(framed.get_inner().0.limit());
}

/// A hint that reports an unmatched PDU of zero bytes, which skipping can never get past.
#[derive(Debug)]
struct ZeroLengthUnmatchedHint;

impl PduHint for ZeroLengthUnmatchedHint {
    fn find_size(&self, _: &[u8]) -> DecodeResult<Option<(bool, usize)>> {
        Ok(Some((false, 0)))
    }
}

/// Runs `f` on its own thread and fails if it has not returned within a few seconds.
///
/// A framer that loops without reading never returns, so the test fails instead of hanging.
fn within_timeout<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(f());
    });
    receiver
        .recv_timeout(core::time::Duration::from_secs(5))
        .expect("read_by_hint did not return")
}

// oracle: mutant async read_by_hint spins forever on a zero-length unmatched PDU (guard removed)
#[test]
fn async_read_by_hint_rejects_a_zero_length_unmatched_pdu() {
    let error = within_timeout(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build runtime");
        let mut framed = ironrdp_tokio::TokioFramed::new(tokio::io::empty());
        runtime
            .block_on(framed.read_by_hint(&ZeroLengthUnmatchedHint))
            .unwrap_err()
    });

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

// oracle: mutant blocking read_by_hint spins forever on a zero-length unmatched PDU (guard removed)
#[test]
fn blocking_read_by_hint_rejects_a_zero_length_unmatched_pdu() {
    let error = within_timeout(|| {
        let mut framed = ironrdp_blocking::Framed::new(io::empty());
        framed.read_by_hint(&ZeroLengthUnmatchedHint).unwrap_err()
    });

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}
