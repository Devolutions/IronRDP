use std::io;

use bytes::{Bytes, BytesMut};
use ironrdp_connector::MonotonicInstant;
use ironrdp_connector::{Sequence, SequenceResult, Written};
use ironrdp_core::WriteBuf;
use ironrdp_pdu::PduHint;
use tracing::{debug, trace};

// TODO: investigate if we could use static async fn / return position impl trait in traits when stabilized:
// https://github.com/rust-lang/rust/issues/91611

pub trait FramedRead {
    type ReadFut<'read>: Future<Output = io::Result<usize>> + 'read
    where
        Self: 'read;

    /// Reads from stream and fills internal buffer
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If you use it as the event in a
    /// `tokio::select!` statement and some other branch
    /// completes first, then it is guaranteed that no data was read.
    fn read<'a>(&'a mut self, buf: &'a mut BytesMut) -> Self::ReadFut<'a>;
}

pub trait FramedWrite {
    type WriteAllFut<'write>: Future<Output = io::Result<()>> + 'write
    where
        Self: 'write;

    /// Writes an entire buffer into this stream.
    ///
    /// # Cancel safety
    ///
    /// This method is not cancellation safe. If it is used as the event
    /// in a `tokio::select!` statement and some other
    /// branch completes first, then the provided buffer may have been
    /// partially written, but future calls to `write_all` will start over
    /// from the beginning of the buffer.
    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Self::WriteAllFut<'a>;
}

pub trait StreamWrapper: Sized {
    type InnerStream;

    fn from_inner(stream: Self::InnerStream) -> Self;

    fn into_inner(self) -> Self::InnerStream;

    fn get_inner(&self) -> &Self::InnerStream;

    fn get_inner_mut(&mut self) -> &mut Self::InnerStream;
}

pub struct Framed<S> {
    stream: S,
    buf: BytesMut,
    /// When the socket read that filled `buf` completed.
    ///
    /// A PDU served entirely from `buf` arrived at the socket read that filled it,
    /// not at the moment the caller happened to drain it, so this is the honest
    /// arrival time for anything read out of `buf`.
    ///
    /// INVARIANT: this is `Some` whenever `buf` is non-empty.
    last_read_at: Option<MonotonicInstant>,
}

/// The bytes a [`Framed`] still had buffered when it was taken apart, and when they arrived.
///
/// A `Framed` is dismantled and rebuilt whenever the transport underneath it changes: a TLS
/// upgrade, a `tokio` split, a websocket handed over to the connector. Those bytes were read
/// from the wire at a real moment, and a PDU decoded out of them later still arrived then.
/// Carrying the two together is what keeps the rebuilt `Framed` able to stamp them honestly.
///
/// The type is opaque on purpose: it is only ever produced by [`Framed::into_inner`] (or
/// [`Leftover::none`], which carries nothing), so bytes can never be paired with a timestamp
/// they did not come from, nor arrive without one.
#[derive(Debug)]
pub struct Leftover {
    bytes: BytesMut,
    /// INVARIANT: this is `Some` whenever `bytes` is non-empty.
    read_at: Option<MonotonicInstant>,
}

impl Leftover {
    /// Nothing carried over.
    pub fn none() -> Self {
        Self {
            bytes: BytesMut::new(),
            read_at: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Returns the buffered bytes without exposing mutation.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl<S> Framed<S> {
    pub fn peek(&self) -> &[u8] {
        &self.buf
    }
}

impl<S> Framed<S>
where
    S: StreamWrapper,
{
    pub fn new(stream: S::InnerStream) -> Self {
        Self::new_with_leftover(stream, Leftover::none())
    }

    pub fn new_with_leftover(stream: S::InnerStream, leftover: Leftover) -> Self {
        Self {
            stream: S::from_inner(stream),
            buf: leftover.bytes,
            last_read_at: leftover.read_at,
        }
    }

    pub fn into_inner(self) -> (S::InnerStream, Leftover) {
        let leftover = Leftover {
            bytes: self.buf,
            read_at: self.last_read_at,
        };
        (self.stream.into_inner(), leftover)
    }

    pub fn into_inner_no_leftover(self) -> S::InnerStream {
        let (stream, leftover) = self.into_inner();
        debug_assert!(leftover.is_empty(), "unexpected leftover");
        stream
    }

    pub fn get_inner(&self) -> (&S::InnerStream, &BytesMut) {
        (self.stream.get_inner(), &self.buf)
    }

    /// The underlying stream, without the buffer.
    ///
    /// Bytes appended to the buffer from the outside would have no arrival time, which is
    /// exactly what [`Leftover`] exists to prevent, so the buffer is not handed out mutably.
    pub fn get_inner_mut(&mut self) -> &mut S::InnerStream {
        self.stream.get_inner_mut()
    }
}

impl<S> Framed<S>
where
    S: FramedRead,
{
    /// Accumulates at least `length` bytes and returns exactly `length` bytes along with when
    /// they arrived, keeping the leftover in the internal buffer.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If you use it as the event in a
    /// `tokio::select!` statement and some other branch
    /// completes first, then it is safe to drop the future and re-create it later.
    /// Data may have been read, but it will be stored in the internal buffer.
    pub(crate) async fn read_exact(&mut self, length: usize) -> io::Result<(BytesMut, MonotonicInstant)> {
        if length == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "zero PDU size"));
        }

        loop {
            // The `last_read_at` invariant makes this `Some` for any non-empty buffer, so a
            // complete frame is never held back waiting for a timestamp that will not come.
            if let Some(read_at) = self.last_read_at
                && self.buf.len() >= length
            {
                return Ok((self.buf.split_to(length), read_at));
            }

            self.buf.reserve(length.saturating_sub(self.buf.len()));

            let len = self.read().await?;

            // Handle EOF
            if len == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "not enough bytes"));
            }
        }
    }

    /// Reads a standard RDP PDU frame.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If you use it as the event in a
    /// `tokio::select!` statement and some other branch
    /// completes first, then it is safe to drop the future and re-create it later.
    /// Data may have been read, but it will be stored in the internal buffer.
    pub async fn read_pdu(&mut self) -> io::Result<(ironrdp_pdu::Action, BytesMut)> {
        loop {
            // Try decoding and see if a frame has been received already
            match ironrdp_pdu::find_size(self.peek()) {
                Ok(Some(pdu_info)) => {
                    let (frame, _) = self.read_exact(pdu_info.length).await?;

                    return Ok((pdu_info.action, frame));
                }
                Ok(None) => {
                    let len = self.read().await?;

                    // Handle EOF
                    if len == 0 {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "not enough bytes"));
                    }
                }
                Err(e) => return Err(io::Error::other(e)),
            };
        }
    }

    /// Reads a frame using the provided PduHint, along with when it arrived from the socket.
    ///
    /// The instant is the read that produced the bytes, which for a frame served out of the
    /// internal buffer is an earlier read than this call.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If you use it as the event in a
    /// `tokio::select!` statement and some other branch
    /// completes first, then it is safe to drop the future and re-create it later.
    /// Data may have been read, but it will be stored in the internal buffer.
    pub async fn read_by_hint(&mut self, hint: &dyn PduHint) -> io::Result<(Bytes, MonotonicInstant)> {
        loop {
            match hint.find_size(self.peek()).map_err(io::Error::other)? {
                Some((matched, length)) => {
                    let (bytes, read_at) = self.read_exact(length).await?;
                    if matched {
                        return Ok((bytes.freeze(), read_at));
                    } else {
                        debug!("Received and lost an unexpected PDU");
                    }
                }
                None => {
                    let len = self.read().await?;

                    // Handle EOF
                    if len == 0 {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "not enough bytes"));
                    }
                }
            };
        }
    }

    /// Reads from stream and fills internal buffer, returning how many bytes were read.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. If you use it as the event in a
    /// `tokio::select!` statement and some other branch
    /// completes first, then it is guaranteed that no data was read.
    async fn read(&mut self) -> io::Result<usize> {
        let len = self.stream.read(&mut self.buf).await?;

        if len > 0 {
            self.last_read_at = Some(monotonic_now());
        }

        Ok(len)
    }
}

impl<S> FramedWrite for Framed<S>
where
    S: FramedWrite,
{
    type WriteAllFut<'write>
        = S::WriteAllFut<'write>
    where
        Self: 'write;

    /// Attempts to write an entire buffer into this `Framed`’s stream.
    ///
    /// # Cancel safety
    ///
    /// This method is not cancellation safe. If it is used as the event
    /// in a `tokio::select!` statement and some other
    /// branch completes first, then the provided buffer may have been
    /// partially written, but future calls to `write_all` will start over
    /// from the beginning of the buffer.
    fn write_all<'a>(&'a mut self, buf: &'a [u8]) -> Self::WriteAllFut<'a> {
        self.stream.write_all(buf)
    }
}

pub async fn single_sequence_step<S>(
    framed: &mut Framed<S>,
    sequence: &mut dyn Sequence,
    buf: &mut WriteBuf,
) -> SequenceResult<()>
where
    S: FramedWrite + FramedRead,
{
    buf.clear();
    let written = single_sequence_step_read(framed, sequence, buf).await?;
    single_sequence_step_write(framed, buf, written).await
}

pub async fn single_sequence_step_read<S>(
    framed: &mut Framed<S>,
    sequence: &mut dyn Sequence,
    buf: &mut WriteBuf,
) -> SequenceResult<Written>
where
    S: FramedRead,
{
    buf.clear();

    if let Some(next_pdu_hint) = sequence.next_pdu_hint() {
        debug!(
            connector.state = sequence.state().name(),
            hint = ?next_pdu_hint,
            "Wait for PDU"
        );

        let (pdu, received_at) = framed
            .read_by_hint(next_pdu_hint)
            .await
            .map_err(|e| ironrdp_connector::custom_err!("read frame by hint", e))?;

        trace!(length = pdu.len(), "PDU received");

        sequence.step(&pdu, received_at, buf)
    } else {
        sequence.step_no_input(buf)
    }
}

async fn single_sequence_step_write<S>(
    framed: &mut Framed<S>,
    buf: &mut WriteBuf,
    written: Written,
) -> SequenceResult<()>
where
    S: FramedWrite,
{
    if let Some(response_len) = written.size() {
        debug_assert_eq!(buf.filled_len(), response_len);
        let response = buf.filled();
        trace!(response_len, "Send response");
        framed
            .write_all(response)
            .await
            .map_err(|e| ironrdp_connector::custom_err!("write all", e))?;
    }

    Ok(())
}

/// Reads the driver-owned monotonic clock.
///
/// The epoch is the first call; only differences are meaningful.
///
/// `web_time::Instant` is `std::time::Instant` everywhere except
/// `wasm32-unknown-unknown`, where `std`'s panics and this one reads
/// `Performance.now()` instead. This crate is reached from `ironrdp-web` through
/// `ironrdp-futures`, so without it the browser build has no clock and every
/// measurement there is lost.
fn monotonic_now() -> MonotonicInstant {
    static EPOCH: std::sync::LazyLock<web_time::Instant> = std::sync::LazyLock::new(web_time::Instant::now);
    MonotonicInstant::from_millis(u64::try_from(EPOCH.elapsed().as_millis()).unwrap_or(u64::MAX))
}
