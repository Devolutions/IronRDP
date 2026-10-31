//! The client loop's outbound queue over generated sequences of PDUs.

use core::pin::Pin;
use core::sync::atomic::Ordering;
use core::task::{Context, Poll};
use std::rc::Rc;

use ironrdp_async::FramedWrite as _;
use ironrdp_server::{OutboundQueue, QueuedWriter};
use ironrdp_tokio::TokioFramed;
use proptest::prelude::*;

/// Accepts at most `chunk` bytes per write and fails once it holds `limit`
/// bytes.
struct FailingSink {
    data: Vec<u8>,
    limit: usize,
    chunk: usize,
}

impl tokio::io::AsyncWrite for FailingSink {
    fn poll_write(mut self: Pin<&mut Self>, _: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let room = self.limit - self.data.len();
        if room == 0 {
            return Poll::Ready(Err(std::io::ErrorKind::BrokenPipe.into()));
        }
        let n = buf.len().min(room).min(self.chunk);
        self.data.extend_from_slice(&buf[..n]);
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// `(reply, bytes)` PDUs, each filled with its index so that order shows.
fn pdus() -> impl Strategy<Value = Vec<(bool, Vec<u8>)>> {
    prop::collection::vec((any::<bool>(), 0usize..64), 0..24).prop_map(|pdus| {
        pdus.into_iter()
            .enumerate()
            .map(|(i, (reply, len))| (reply, vec![u8::try_from(i).expect("fewer than 256 PDUs"); len]))
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// PDUs reach the transport in the order they were queued, also those
    /// queued after `finish`, up to the byte where the transport fails. Both
    /// byte counts cover exactly what waits to be written, the write counter
    /// counts the PDUs written whole, and a stopped queue accepts no more.
    // oracle: reference model: a FIFO of byte strings, written to a sink that fails after a chosen byte count
    #[test]
    fn queued_pdus_are_written_in_order_and_counted(
        pdus in pdus(),
        split in any::<prop::sample::Index>(),
        limit in 0usize..1600,
        chunk in 1usize..80,
    ) {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(async {
                let queue = Rc::new(OutboundQueue::default());
                let mut other = QueuedWriter::new(Rc::clone(&queue));
                let mut reply = other.for_replies();
                let finish_at = split.index(pdus.len() + 1);
                for (i, (is_reply, bytes)) in pdus.iter().enumerate() {
                    if i == finish_at {
                        queue.finish();
                    }
                    let writer = if *is_reply { &mut reply } else { &mut other };
                    writer.write_all(bytes).await.expect("queue");
                }
                if finish_at == pdus.len() {
                    queue.finish();
                }
                let total: usize = pdus.iter().map(|(_, b)| b.len()).sum();
                let replies: usize = pdus.iter().filter(|(r, _)| *r).map(|(_, b)| b.len()).sum();
                prop_assert_eq!(queue.queued_bytes(), (total, replies));

                let mut transport = TokioFramed::new(FailingSink { data: Vec::new(), limit, chunk });
                let result = queue.write_queued(&mut transport).await;

                // The model: the PDUs in queue order, cut where the sink fails.
                let wire: Vec<u8> = pdus.iter().flat_map(|(_, b)| b.iter().copied()).collect();
                let mut whole = 0u64;
                let mut end = 0;
                for (_, bytes) in &pdus {
                    end += bytes.len();
                    if limit < end {
                        break;
                    }
                    whole += 1;
                }
                let sink = transport.into_inner_no_leftover();
                prop_assert_eq!(&*sink.data, &wire[..total.min(limit)]);
                prop_assert_eq!(result.is_err(), limit < total);
                prop_assert_eq!(other.write_counter().load(Ordering::Relaxed), whole);
                prop_assert_eq!(queue.queued_bytes(), (0, 0));
                prop_assert!(other.write_all(b"late").await.is_err());
                Ok(())
            })?;
    }
}
