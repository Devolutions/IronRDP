use std::{fmt::Debug, io::Write};

use super::GraphicsMessagesError;
use crate::{PduBufferParsing, PduParsing, Rectangle};
use bit_field::BitField;
use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuantQuality {
    pub quantization_parameter: u8,
    pub progressive: bool,
    pub quality: u8,
}

impl PduParsing for QuantQuality {
    type Error = GraphicsMessagesError;

    fn from_buffer(mut stream: impl std::io::Read) -> Result<Self, Self::Error>
    where
        Self: Sized,
    {
        let data = stream.read_u8()?;
        let qp = data.get_bits(0..6);
        let progressive = data.get_bit(7);
        let quality = stream.read_u8()?;
        Ok(QuantQuality {
            quantization_parameter: qp,
            progressive,
            quality,
        })
    }

    fn to_buffer(&self, mut stream: impl std::io::Write) -> Result<(), Self::Error> {
        let mut data = 0u8;
        data.set_bits(0..6, self.quantization_parameter);
        data.set_bit(7, self.progressive);
        stream.write_u8(data)?;
        stream.write_u8(self.quality)?;
        Ok(())
    }

    fn buffer_length(&self) -> usize {
        2
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Avc420BitmapStream<'a> {
    pub rectangles: Vec<Rectangle>,
    pub quant_qual_vals: Vec<QuantQuality>,
    pub data: &'a [u8],
}

impl<'a> Debug for Avc420BitmapStream<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Avc420BitmapStream")
            .field("rectangles", &self.rectangles)
            .field("quant_qual_vals", &self.quant_qual_vals)
            .field("data_len", &self.data.len())
            .finish()
    }
}

impl<'a> PduBufferParsing<'a> for Avc420BitmapStream<'a> {
    type Error = GraphicsMessagesError;

    fn from_buffer_consume(mut buffer: &mut &'a [u8]) -> Result<Self, Self::Error> {
        let num_regions = buffer.read_u32::<LittleEndian>()?;
        let mut rectangles = Vec::with_capacity(num_regions as usize);
        let mut quant_qual_vals = Vec::with_capacity(num_regions as usize);
        for _ in 0..num_regions {
            rectangles.push(Rectangle::from_buffer(&mut buffer)?);
        }
        for _ in 0..num_regions {
            quant_qual_vals.push(QuantQuality::from_buffer(&mut buffer)?);
        }
        let data = buffer;
        Ok(Avc420BitmapStream {
            rectangles,
            quant_qual_vals,
            data,
        })
    }

    fn to_buffer_consume(&self, mut buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.rectangles.len() as u32)?;
        for rectangle in &self.rectangles {
            rectangle.to_buffer(&mut buffer)?;
        }
        for quant_qual_val in &self.quant_qual_vals {
            quant_qual_val.to_buffer(&mut buffer)?;
        }
        buffer.write_all(self.data)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        // Each rectangle is 8 bytes and 2 bytes for each quant val
        4 + self.rectangles.len() * 10 + self.data.len()
    }
}

bitflags! {
    #[derive(Default)]
    pub struct Encoding: u8 {
        const LUMA_AND_CHROMA = 0x00;
        const LUMA = 0x01;
        const CHROMA = 0x02;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Avc444BitmapStream<'a> {
    pub encoding: Encoding,
    pub stream1: Avc420BitmapStream<'a>,
    pub stream2: Option<Avc420BitmapStream<'a>>,
}

impl<'a> PduBufferParsing<'a> for Avc444BitmapStream<'a> {
    type Error = GraphicsMessagesError;

    fn from_buffer_consume(buffer: &mut &'a [u8]) -> Result<Self, Self::Error> {
        let stream_info = buffer.read_u32::<LittleEndian>()?;
        let stream_len = stream_info.get_bits(0..30);
        let encoding = Encoding::from_bits_truncate(stream_info.get_bits(30..32) as u8);

        if stream_len == 0 {
            if encoding == Encoding::LUMA_AND_CHROMA {
                return Err(GraphicsMessagesError::InvalidAvcEncoding);
            }

            let stream1 = Avc420BitmapStream::from_buffer_consume(buffer)?;
            Ok(Avc444BitmapStream {
                encoding,
                stream1,
                stream2: None,
            })
        } else {
            let (mut stream1, mut stream2) = buffer.split_at(stream_len as usize);
            let stream1 = Avc420BitmapStream::from_buffer_consume(&mut stream1)?;
            let stream2 = if encoding == Encoding::LUMA_AND_CHROMA {
                Some(Avc420BitmapStream::from_buffer_consume(&mut stream2)?)
            } else {
                None
            };
            Ok(Avc444BitmapStream {
                encoding,
                stream1,
                stream2,
            })
        }
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        let mut stream_info = 0u32;
        stream_info.set_bits(0..30, self.stream1.buffer_length() as u32);
        stream_info.set_bits(30..32, self.encoding.bits() as u32);
        buffer.write_u32::<LittleEndian>(stream_info)?;
        self.stream1.to_buffer_consume(buffer)?;
        if let Some(stream) = self.stream2.as_ref() {
            stream.to_buffer_consume(buffer)?;
        }
        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let stream2_len = if let Some(stream) = self.stream2.as_ref() {
            stream.buffer_length()
        } else {
            0
        };

        4 + self.stream1.buffer_length() + stream2_len
    }
}
