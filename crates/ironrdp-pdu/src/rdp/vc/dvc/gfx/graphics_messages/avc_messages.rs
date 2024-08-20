use std::fmt::Debug;

use bit_field::BitField;
use bitflags::bitflags;

use crate::geometry::InclusiveRectangle;
use crate::{PduDecode, PduEncode, PduResult};
use ironrdp_core::{ReadCursor, WriteCursor};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuantQuality {
    pub quantization_parameter: u8,
    pub progressive: bool,
    pub quality: u8,
}

impl QuantQuality {
    const NAME: &'static str = "GfxQuantQuality";

    const FIXED_PART_SIZE: usize = 1 /* data */ + 1 /* quality */;
}

impl PduEncode for QuantQuality {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        let mut data = 0u8;
        data.set_bits(0..6, self.quantization_parameter);
        data.set_bit(7, self.progressive);
        dst.write_u8(data);
        dst.write_u8(self.quality);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for QuantQuality {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let data = src.read_u8();
        let qp = data.get_bits(0..6);
        let progressive = data.get_bit(7);
        let quality = src.read_u8();
        Ok(QuantQuality {
            quantization_parameter: qp,
            progressive,
            quality,
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Avc420BitmapStream<'a> {
    pub rectangles: Vec<InclusiveRectangle>,
    pub quant_qual_vals: Vec<QuantQuality>,
    pub data: &'a [u8],
}

impl Debug for Avc420BitmapStream<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Avc420BitmapStream")
            .field("rectangles", &self.rectangles)
            .field("quant_qual_vals", &self.quant_qual_vals)
            .field("data_len", &self.data.len())
            .finish()
    }
}

impl Avc420BitmapStream<'_> {
    const NAME: &'static str = "Avc420BitmapStream";

    const FIXED_PART_SIZE: usize = 4 /* nRect */;
}

impl PduEncode for Avc420BitmapStream<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(cast_length!("len", self.rectangles.len())?);
        for rectangle in &self.rectangles {
            rectangle.encode(dst)?;
        }
        for quant_qual_val in &self.quant_qual_vals {
            quant_qual_val.encode(dst)?;
        }
        dst.write_slice(self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        // Each rectangle is 8 bytes and 2 bytes for each quant val
        Self::FIXED_PART_SIZE + self.rectangles.len() * 10 + self.data.len()
    }
}

impl<'de> PduDecode<'de> for Avc420BitmapStream<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let num_regions = src.read_u32();
        let mut rectangles = Vec::with_capacity(num_regions as usize);
        let mut quant_qual_vals = Vec::with_capacity(num_regions as usize);
        for _ in 0..num_regions {
            rectangles.push(InclusiveRectangle::decode(src)?);
        }
        for _ in 0..num_regions {
            quant_qual_vals.push(QuantQuality::decode(src)?);
        }
        let data = src.remaining();
        Ok(Avc420BitmapStream {
            rectangles,
            quant_qual_vals,
            data,
        })
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

impl Avc444BitmapStream<'_> {
    const NAME: &'static str = "Avc444BitmapStream";

    const FIXED_PART_SIZE: usize = 4 /* streamInfo */;
}

impl PduEncode for Avc444BitmapStream<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        let mut stream_info = 0u32;
        stream_info.set_bits(0..30, cast_length!("stream1size", self.stream1.size())?);
        stream_info.set_bits(30..32, self.encoding.bits() as u32);
        dst.write_u32(stream_info);
        self.stream1.encode(dst)?;
        if let Some(stream) = self.stream2.as_ref() {
            stream.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let stream2_size = if let Some(stream) = self.stream2.as_ref() {
            stream.size()
        } else {
            0
        };

        Self::FIXED_PART_SIZE + self.stream1.size() + stream2_size
    }
}

impl<'de> PduDecode<'de> for Avc444BitmapStream<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let stream_info = src.read_u32();
        let stream_len = stream_info.get_bits(0..30);
        let encoding = Encoding::from_bits_truncate(stream_info.get_bits(30..32) as u8);

        if stream_len == 0 {
            if encoding == Encoding::LUMA_AND_CHROMA {
                return Err(invalid_message_err!("encoding", "invalid encoding"));
            }

            let stream1 = Avc420BitmapStream::decode(src)?;
            Ok(Avc444BitmapStream {
                encoding,
                stream1,
                stream2: None,
            })
        } else {
            let (mut stream1, mut stream2) = src.split_at(stream_len as usize);
            let stream1 = Avc420BitmapStream::decode(&mut stream1)?;
            let stream2 = if encoding == Encoding::LUMA_AND_CHROMA {
                Some(Avc420BitmapStream::decode(&mut stream2)?)
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
}
