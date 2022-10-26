use bit_field::BitField;
use bytes::{BufMut, BytesMut};
use ironrdp::{Action, RdpError};
use tokio_util::codec::{Decoder, Encoder};

use byteorder::{BigEndian, ReadBytesExt};

use num_traits::FromPrimitive;

use crate::transport::{Decoder as TransportDecoder, Encoder as TransportEncoder};
#[derive(Default)]
pub struct RdpFrameCodec {}

impl<T> Encoder<T> for RdpFrameCodec
where
    T: AsRef<[u8]>,
{
    type Error = RdpError;

    fn encode(&mut self, item: T, buf: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        buf.extend_from_slice(item.as_ref());
        Ok(())
    }
}

impl Decoder for RdpFrameCodec {
    type Item = BytesMut;
    type Error = ironrdp::RdpError;

    fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let mut stream = src.as_ref();
        if stream.is_empty() {
            return Ok(None);
        }
        let header = stream.read_u8()?;
        let action = header.get_bits(0..2);
        let action = Action::from_u8(action).ok_or(ironrdp::RdpError::InvalidActionCode(action))?;

        let length = match action {
            Action::X224 if stream.len() >= 3 => {
                let _reserved = stream.read_u8()?;

                stream.read_u16::<BigEndian>()?
            }
            Action::FastPath if !stream.is_empty() => {
                let a = stream.read_u8()?;
                if a & 0x80 != 0 {
                    if stream.is_empty() {
                        return Ok(None);
                    }
                    let b = stream.read_u8()?;
                    ((u16::from(a) & !0x80) << 8) + u16::from(b)
                } else {
                    u16::from(a)
                }
            }
            _ => {
                return Ok(None);
            }
        };

        if src.len() >= length as usize {
            Ok(Some(src.split_to(length as usize)))
        } else {
            Ok(None)
        }
    }
}

pub struct TrasnportCodec<T> {
    inner: RdpFrameCodec,
    transport: T,
}

impl<T> TrasnportCodec<T> {
    pub fn new(transport: T) -> Self {
        TrasnportCodec {
            inner: RdpFrameCodec::default(),
            transport,
        }
    }
}

impl<E, T> Encoder<E> for TrasnportCodec<T>
where
    T: TransportEncoder<Item = E>,
    <T as TransportEncoder>::Error: From<std::io::Error>,
    <T as TransportEncoder>::Error: From<RdpError>,
{
    type Error = <T as TransportEncoder>::Error;

    fn encode(&mut self, item: E, buf: &mut bytes::BytesMut) -> Result<(), Self::Error> {
        self.transport.encode(item, buf.writer())?;
        Ok(())
    }
}

impl<T> Decoder for TrasnportCodec<T>
where
    T: TransportDecoder,
    <T as TransportDecoder>::Error: From<std::io::Error>,
    <T as TransportDecoder>::Error: From<ironrdp::RdpError>,
{
    type Item = <T as TransportDecoder>::Item;
    type Error = <T as TransportDecoder>::Error;

    fn decode(&mut self, src: &mut bytes::BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let buf = self.inner.decode(src)?;
        if let Some(data) = buf {
            Ok(Some(self.transport.decode(data.as_ref())?))
        } else {
            Ok(None)
        }
    }
}
