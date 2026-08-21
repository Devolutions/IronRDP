//! Arrival-time stamping on [`Framed`].
//!
//! The connect-time bandwidth measurement in `ironrdp-connector` is only as
//! honest as the instant `Framed` hands to `Sequence::step`, and nothing else
//! pins that down: the connector tests drive `step` with hand-picked instants,
//! so a regression that stopped stamping reads would leave them green while
//! every measured window silently collapsed to the unmeasurable floor.
//!
//! These live here rather than inline in `ironrdp-async` because that crate
//! sets `[lib] test = false`, so an inline `#[cfg(test)]` module would compile
//! and never run.

use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use core::time::Duration;
use std::collections::VecDeque;
use std::io;

use ironrdp::pdu::{PduHint, X224_HINT};
use ironrdp_async::bytes::BytesMut;
use ironrdp_async::{Framed, FramedRead, StreamWrapper};

/// Drives a future to completion on the current thread. The mock below never
/// yields, so polling in a loop is enough and saves pulling in a runtime.
fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = core::pin::pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(value) = fut.as_mut().poll(&mut cx) {
            return value;
        }
    }
}

/// A stream that hands over pre-arranged chunks, one per read, so a test can
/// decide exactly which PDUs share a socket read and which get their own.
struct ChunkedStream {
    chunks: VecDeque<Vec<u8>>,
    /// Delay applied before each read completes, so consecutive reads land on
    /// distinguishable instants rather than relying on clock resolution.
    delay: Duration,
}

impl ChunkedStream {
    fn new(chunks: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self {
            chunks: chunks.into_iter().collect(),
            delay: Duration::from_millis(5),
        }
    }
}

impl StreamWrapper for ChunkedStream {
    type InnerStream = Self;

    fn from_inner(stream: Self::InnerStream) -> Self {
        stream
    }

    fn into_inner(self) -> Self::InnerStream {
        self
    }

    fn get_inner(&self) -> &Self::InnerStream {
        self
    }

    fn get_inner_mut(&mut self) -> &mut Self::InnerStream {
        self
    }
}

impl FramedRead for ChunkedStream {
    type ReadFut<'read>
        = Pin<Box<dyn Future<Output = io::Result<usize>> + 'read>>
    where
        Self: 'read;

    fn read<'a>(&'a mut self, buf: &'a mut BytesMut) -> Self::ReadFut<'a> {
        Box::pin(async move {
            std::thread::sleep(self.delay);
            match self.chunks.pop_front() {
                Some(chunk) => {
                    buf.extend_from_slice(&chunk);
                    Ok(chunk.len())
                }
                None => Ok(0),
            }
        })
    }
}

/// Smallest frame `ironrdp_pdu::find_size` will accept: a TPKT header whose
/// length field covers itself plus `payload`.
fn tpkt(payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(4 + payload.len()).expect("frame fits");
    let mut frame = vec![0x03, 0x00];
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

#[derive(Debug)]
struct ZeroSizeHint;

impl PduHint for ZeroSizeHint {
    fn find_size(&self, _bytes: &[u8]) -> ironrdp::core::DecodeResult<Option<(bool, usize)>> {
        Ok(Some((true, 0)))
    }
}

#[test]
fn zero_size_hint_fails_before_reading() {
    let mut framed = Framed::<ChunkedStream>::new(ChunkedStream::new([tpkt(&[0xAA; 8])]));

    let error = block_on(framed.read_by_hint(&ZeroSizeHint)).expect_err("zero PDU size must fail");

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let (stream, leftover) = framed.into_inner();
    assert_eq!(stream.chunks.len(), 1, "the stream must not be read");
    assert!(leftover.is_empty());
}

#[test]
fn each_socket_read_advances_the_arrival_time() {
    // One frame per read, so each PDU is stamped by the read that carried it.
    let mut framed = Framed::<ChunkedStream>::new(ChunkedStream::new([tpkt(&[0xAA; 8]), tpkt(&[0xBB; 8])]));

    let (_, first) = block_on(framed.read_by_hint(&X224_HINT)).expect("first frame");
    let (_, second) = block_on(framed.read_by_hint(&X224_HINT)).expect("second frame");

    assert!(
        second.duration_since(first) >= Duration::from_millis(2),
        "the second read must stamp a later arrival than the first, got {:?}",
        second.duration_since(first)
    );
}

#[test]
fn pdus_sharing_a_socket_read_share_its_arrival_time() {
    // Both frames arrive in one read. The second is served from the buffer, so
    // it arrived when that read completed, not when the caller drained it.
    let mut chunk = tpkt(&[0xAA; 8]);
    chunk.extend_from_slice(&tpkt(&[0xBB; 8]));
    let mut framed = Framed::<ChunkedStream>::new(ChunkedStream::new([chunk]));

    let (_, first) = block_on(framed.read_by_hint(&X224_HINT)).expect("first frame");

    // Wait before draining the second one. `MonotonicInstant` counts whole
    // milliseconds, so without this the two drains would be indistinguishable
    // and the assertion below would hold even if the stamp were taken on drain
    // rather than on the read.
    std::thread::sleep(Duration::from_millis(10));

    let (_, second) = block_on(framed.read_by_hint(&X224_HINT)).expect("second frame");

    assert_eq!(
        first, second,
        "a PDU served from the buffer keeps the arrival time of the read that filled it"
    );
}

#[test]
fn leftover_carried_into_a_new_framed_keeps_its_arrival_time() {
    // `ironrdp-tokio`'s split/unsplit helpers and the client's TLS upgrade rebuild a `Framed`
    // around bytes the previous one had already read. Those bytes arrived at that earlier read,
    // and the rebuilt `Framed` has to keep saying so: a PDU decoded out of them did not arrive
    // when the new `Framed` was built.
    let carried_frame = tpkt(&[0xDD; 8]);
    let mut chunk = tpkt(&[0xAA; 8]);
    chunk.extend_from_slice(&carried_frame);
    let mut framed = Framed::<ChunkedStream>::new(ChunkedStream::new([chunk]));

    let (_, read_at) = block_on(framed.read_by_hint(&X224_HINT)).expect("first frame");

    let (stream, leftover) = framed.into_inner();
    assert!(!leftover.is_empty(), "the second frame is still buffered");
    assert_eq!(leftover.as_bytes(), carried_frame);

    std::thread::sleep(Duration::from_millis(10));

    let mut framed = Framed::<ChunkedStream>::new_with_leftover(stream, leftover);
    let (_, carried_read_at) = block_on(framed.read_by_hint(&X224_HINT)).expect("frame served from leftover");

    assert_eq!(
        read_at, carried_read_at,
        "a frame carried over as leftover still arrived at the read that produced it"
    );
}
