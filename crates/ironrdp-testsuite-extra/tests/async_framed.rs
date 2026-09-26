//! Arrival-time stamping on [`Framed`].
//!
//! The connect-time bandwidth measurement in `ironrdp-connector` is only as
//! honest as `Framed::last_read_at`, and nothing else pins that down: the
//! connector tests drive `Sequence::step` with hand-picked instants, so a
//! regression that stopped stamping reads would leave them green while every
//! measured window silently collapsed to the unmeasurable floor.
//!
//! These live here rather than inline in `ironrdp-async` because that crate
//! sets `[lib] test = false`, so an inline `#[cfg(test)]` module would compile
//! and never run.

use core::future::{Ready, ready};
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use core::time::Duration;
use std::collections::VecDeque;
use std::io;

use ironrdp_async::bytes::BytesMut;
use ironrdp_async::{Framed, FramedRead, FramedWrite, NetworkClient, StreamWrapper};
use ironrdp_core::decode;
use ironrdp_pdu::mcs::McsMessage;
use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
use ironrdp_pdu::rdp::multitransport::{MultitransportRequestPdu, MultitransportResponsePdu, RequestedProtocol};
use ironrdp_pdu::x224::X224;

const USER_CHANNEL_ID: u16 = 1002;
const IO_CHANNEL_ID: u16 = 1003;
const MESSAGE_CHANNEL_ID: u16 = 1004;

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
    writes: Vec<u8>,
    /// Delay applied before each read completes, so consecutive reads land on
    /// distinguishable instants rather than relying on clock resolution.
    delay: Duration,
}

impl ChunkedStream {
    fn new(chunks: impl IntoIterator<Item = Vec<u8>>) -> Self {
        Self {
            chunks: chunks.into_iter().collect(),
            writes: Vec::new(),
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

impl FramedWrite for ChunkedStream {
    type WriteAllFut<'write>
        = Ready<io::Result<()>>
    where
        Self: 'write;

    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Self::WriteAllFut<'a> {
        self.writes.extend_from_slice(buf);
        ready(Ok(()))
    }
}

struct UnusedNetworkClient;

impl NetworkClient for UnusedNetworkClient {
    async fn send(
        &mut self,
        _: &ironrdp::connector::sspi::generator::NetworkRequest,
    ) -> ironrdp::connector::ConnectorResult<Vec<u8>> {
        Err(ironrdp::connector::general_err!("unexpected network request"))
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

#[test]
fn each_socket_read_advances_the_arrival_time() {
    // One frame per read, so each PDU is stamped by the read that carried it.
    let mut framed = Framed::<ChunkedStream>::new(ChunkedStream::new([tpkt(&[0xAA; 8]), tpkt(&[0xBB; 8])]));

    assert!(
        framed.last_read_at().is_none(),
        "an unread Framed has observed no arrival"
    );

    block_on(framed.read_pdu()).expect("first frame");
    let first = framed.last_read_at().expect("host build observes time");

    block_on(framed.read_pdu()).expect("second frame");
    let second = framed.last_read_at().expect("host build observes time");

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

    block_on(framed.read_pdu()).expect("first frame");
    let first = framed.last_read_at().expect("host build observes time");

    // Wait before draining the second one. `MonotonicInstant` counts whole
    // milliseconds, so without this the two drains would be indistinguishable
    // and the assertion below would hold even if the stamp were taken on drain
    // rather than on the read.
    std::thread::sleep(Duration::from_millis(10));

    block_on(framed.read_pdu()).expect("second frame");
    let second = framed.last_read_at().expect("host build observes time");

    assert_eq!(
        first, second,
        "a PDU served from the buffer keeps the arrival time of the read that filled it"
    );
}

#[test]
fn leftover_carried_into_a_new_framed_has_no_arrival_time() {
    // `ironrdp-tokio`'s split/unsplit helpers rebuild a `Framed` around bytes
    // that were read by the previous one. Those bytes did arrive, but not on
    // this `Framed`, and it has no way to know when: reporting an arrival here
    // would be inventing one.
    let leftover = BytesMut::from(&*tpkt(&[0xDD; 8]));
    let mut framed = Framed::<ChunkedStream>::new_with_leftover(ChunkedStream::new([]), leftover);

    block_on(framed.read_pdu()).expect("frame served entirely from leftover");

    assert!(
        framed.last_read_at().is_none(),
        "a frame served from leftover was never read by this Framed, so it has no arrival time"
    );
}

#[test]
fn multitransport_handler_error_reports_abort_and_preserves_the_error() {
    let mut connector = ironrdp::connector::ClientConnector::new(
        crate::e2e::default_client_config(),
        "127.0.0.1:3389".parse().unwrap(),
    );
    connector.state = ironrdp::connector::ClientConnectorState::EnhancedSecurityUpgrade {
        selected_protocol: ironrdp_pdu::nego::SecurityProtocol::empty(),
    };
    let should_upgrade = ironrdp_async::skip_connect_begin(&mut connector);
    let upgraded = ironrdp_async::mark_as_upgraded(should_upgrade, &mut connector);
    connector.state = ironrdp::connector::ClientConnectorState::MultitransportPending {
        io_channel_id: IO_CHANNEL_ID,
        user_channel_id: USER_CHANNEL_ID,
        message_channel_id: Some(MESSAGE_CHANNEL_ID),
        request: MultitransportRequestPdu {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::TRANSPORT_REQ,
            },
            request_id: 42,
            requested_protocol: RequestedProtocol::UdpFecR,
            security_cookie: [0; 16],
        },
        requests_seen: 1,
        soft_sync: true,
    };
    let mut framed = Framed::<ChunkedStream>::new(ChunkedStream::new([]));
    let mut network_client = UnusedNetworkClient;

    let error = block_on(ironrdp_async::connect_finalize_with_multitransport(
        upgraded,
        connector,
        &mut framed,
        &mut network_client,
        ironrdp::connector::ServerName::new("server"),
        Vec::new(),
        None,
        |request: MultitransportRequestPdu, soft_sync: bool| async move {
            assert_eq!(request.request_id, 42);
            assert!(soft_sync);
            Err(ironrdp::connector::general_err!("test multitransport setup failure"))
        },
    ))
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "[test multitransport setup failure] general error",
        "the handler error must win over abort reporting"
    );

    let (stream, _) = framed.get_inner();
    let X224(McsMessage::SendDataRequest(request)) = decode(stream.writes.as_slice()).unwrap() else {
        panic!("the handler error must send an abort response");
    };
    assert_eq!(request.channel_id, MESSAGE_CHANNEL_ID);

    let response: MultitransportResponsePdu = decode(&request.user_data).unwrap();
    assert_eq!(response.request_id, 42);
    assert_eq!(response.hr_response, MultitransportResponsePdu::E_ABORT);
}
