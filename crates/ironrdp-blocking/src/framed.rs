use std::io::{self, Read, Write};

use bytes::{Bytes, BytesMut};
use ironrdp_connector::MonotonicInstant;
use ironrdp_pdu::PduHint;
use tracing::debug;

pub struct Framed<S> {
    stream: S,
    buf: BytesMut,
    /// When the socket read that filled `buf` completed. A PDU served from `buf`
    /// arrived at the read that filled it, not when the caller drained it.
    ///
    /// INVARIANT: this is `Some` whenever `buf` is non-empty.
    last_read_at: Option<MonotonicInstant>,
}

/// The bytes a [`Framed`] still had buffered when it was taken apart, and when they arrived.
///
/// A `Framed` is dismantled and rebuilt whenever the transport underneath it changes, most often
/// a TLS upgrade. Those bytes were read from the wire at a real moment, and a PDU decoded out of
/// them later still arrived then. Carrying the two together is what keeps the rebuilt `Framed`
/// able to stamp them honestly.
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
}

impl<S> Framed<S> {
    pub fn new(stream: S) -> Self {
        Self::new_with_leftover(stream, Leftover::none())
    }

    pub fn new_with_leftover(stream: S, leftover: Leftover) -> Self {
        Self {
            stream,
            buf: leftover.bytes,
            last_read_at: leftover.read_at,
        }
    }

    pub fn into_inner(self) -> (S, Leftover) {
        let leftover = Leftover {
            bytes: self.buf,
            read_at: self.last_read_at,
        };
        (self.stream, leftover)
    }

    pub fn into_inner_no_leftover(self) -> S {
        let (stream, leftover) = self.into_inner();
        debug_assert!(leftover.is_empty(), "unexpected leftover");
        stream
    }

    pub fn get_inner(&self) -> (&S, &BytesMut) {
        (&self.stream, &self.buf)
    }

    /// The underlying stream, without the buffer.
    ///
    /// Bytes appended to the buffer from the outside would have no arrival time, which is
    /// exactly what [`Leftover`] exists to prevent, so the buffer is not handed out mutably.
    pub fn get_inner_mut(&mut self) -> &mut S {
        &mut self.stream
    }

    pub fn peek(&self) -> &[u8] {
        &self.buf
    }
}

impl<S> Framed<S>
where
    S: Read,
{
    /// Accumulates at least `length` bytes and returns exactly `length` bytes along with when
    /// they arrived, keeping the leftover in the internal buffer.
    pub(crate) fn read_exact(&mut self, length: usize) -> io::Result<(BytesMut, MonotonicInstant)> {
        loop {
            // The `last_read_at` invariant makes this `Some` for any non-empty buffer, so a
            // complete frame is never held back waiting for a timestamp that will not come.
            if let Some(read_at) = self.last_read_at
                && self.buf.len() >= length
            {
                return Ok((self.buf.split_to(length), read_at));
            }

            self.buf.reserve(length.saturating_sub(self.buf.len()));

            let len = self.read()?;

            // Handle EOF
            if len == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "not enough bytes"));
            }
        }
    }

    /// Reads a standard RDP PDU frame.
    pub fn read_pdu(&mut self) -> io::Result<(ironrdp_pdu::Action, BytesMut)> {
        loop {
            // Try decoding and see if a frame has been received already
            match ironrdp_pdu::find_size(self.peek()) {
                Ok(Some(pdu_info)) => {
                    let (frame, _) = self.read_exact(pdu_info.length)?;

                    return Ok((pdu_info.action, frame));
                }
                Ok(None) => {
                    let len = self.read()?;

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
    pub fn read_by_hint(&mut self, hint: &dyn PduHint) -> io::Result<(Bytes, MonotonicInstant)> {
        loop {
            match hint.find_size(self.peek()).map_err(io::Error::other)? {
                Some((matched, length)) => {
                    let (bytes, read_at) = self.read_exact(length)?;
                    if matched {
                        return Ok((bytes.freeze(), read_at));
                    } else {
                        debug!("Received and lost an unexpected PDU");
                    }
                }
                None => {
                    let len = self.read()?;

                    // Handle EOF
                    if len == 0 {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "not enough bytes"));
                    }
                }
            };
        }
    }

    /// Reads from stream and fills internal buffer, returning how many bytes were read.
    fn read(&mut self) -> io::Result<usize> {
        // FIXME(perf): use read_buf (https://doc.rust-lang.org/std/io/trait.Read.html#method.read_buf)
        // once its stabilized. See tracking issue for RFC 2930: https://github.com/rust-lang/rust/issues/78485

        let mut read_bytes = [0u8; 1024];
        let len = self.stream.read(&mut read_bytes)?;

        if len > 0 {
            self.last_read_at = Some(monotonic_now());
            self.buf.extend_from_slice(&read_bytes[..len]);
        }

        Ok(len)
    }
}

impl<S> Framed<S>
where
    S: Write,
{
    /// Attempts to write an entire buffer into this `Framed`’s stream.
    pub fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.stream.write_all(buf)
    }
}

/// Reads the driver-owned monotonic clock. Epoch is the first call; only
/// differences are meaningful.
fn monotonic_now() -> MonotonicInstant {
    static EPOCH: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);
    MonotonicInstant::from_millis(u64::try_from(EPOCH.elapsed().as_millis()).unwrap_or(u64::MAX))
}
