use std::io::{self, Read, Write};

use bytes::{Bytes, BytesMut};
use ironrdp_pdu::PduHint;

pub struct Framed<S> {
    stream: S,
    buf: BytesMut,
}

impl<S> Framed<S> {
    pub fn new(stream: S) -> Self {
        Self {
            stream,
            buf: BytesMut::new(),
        }
    }

    pub fn into_inner(self) -> (S, BytesMut) {
        (self.stream, self.buf)
    }

    pub fn into_inner_no_leftover(self) -> S {
        let (stream, leftover) = self.into_inner();
        debug_assert_eq!(leftover.len(), 0, "unexpected leftover");
        stream
    }

    pub fn get_inner(&self) -> (&S, &BytesMut) {
        (&self.stream, &self.buf)
    }

    pub fn get_inner_mut(&mut self) -> (&mut S, &mut BytesMut) {
        (&mut self.stream, &mut self.buf)
    }

    pub fn peek(&self) -> &[u8] {
        &self.buf
    }
}

impl<S> Framed<S>
where
    S: Read,
{
    /// Accumulates at least `length` bytes and returns exactly `length` bytes, keeping the leftover in the internal buffer.
    pub fn read_exact(&mut self, length: usize) -> io::Result<BytesMut> {
        loop {
            if self.buf.len() >= length {
                return Ok(self.buf.split_to(length));
            } else {
                self.buf.reserve(length - self.buf.len());
            }

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
                    let frame = self.read_exact(pdu_info.length)?;

                    return Ok((pdu_info.action, frame));
                }
                Ok(None) => {
                    let len = self.read()?;

                    // Handle EOF
                    if len == 0 {
                        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "not enough bytes"));
                    }
                }
                Err(e) => return Err(io::Error::new(io::ErrorKind::Other, e)),
            };
        }
    }

    /// Reads a frame using the provided PduHint.
    pub fn read_by_hint(&mut self, hint: &dyn PduHint) -> io::Result<Bytes> {
        loop {
            match hint
                .find_size(self.peek())
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?
            {
                Some(length) => {
                    return Ok(self.read_exact(length)?.freeze());
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
        self.stream.read(&mut self.buf)
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
