use ironrdp_core::{cast_length, other_err, EncodeResult};
use ironrdp_graphics::color_conversion::to_64x64_ycbcr_tile;
use ironrdp_graphics::rfx_encode_component;
use ironrdp_graphics::rlgr::RlgrError;
use ironrdp_pdu::codecs::rfx::{self, OperatingMode, RfxChannel, RfxChannelHeight, RfxChannelWidth};
use ironrdp_pdu::rdp::capability_sets::EntropyBits;
use ironrdp_pdu::PduBufferParsing;

use crate::BitmapUpdate;

#[derive(Debug)]
pub(crate) struct RfxEncoder {
    entropy_algorithm: rfx::EntropyAlgorithm,
}

impl RfxEncoder {
    pub(crate) fn new(entropy_bits: EntropyBits) -> Self {
        let entropy_algorithm = match entropy_bits {
            EntropyBits::Rlgr1 => rfx::EntropyAlgorithm::Rlgr1,
            EntropyBits::Rlgr3 => rfx::EntropyAlgorithm::Rlgr3,
        };
        Self { entropy_algorithm }
    }

    // FIXME: rewrite to use WriteCursor
    pub(crate) fn encode(&mut self, bitmap: &BitmapUpdate) -> EncodeResult<Vec<u8>> {
        let width = bitmap.width.get();
        let height = bitmap.height.get();
        let entropy_algorithm = self.entropy_algorithm;

        // header messages
        // FIXME: skip if unnecessary?
        let sync = rfx::SyncPdu;
        let context = rfx::ContextPdu {
            flags: OperatingMode::IMAGE_MODE,
            entropy_algorithm,
        };
        let context = rfx::Headers::Context(context);
        let channels = rfx::ChannelsPdu(vec![RfxChannel {
            width: RfxChannelWidth::new(cast_length!("width", width)?),
            height: RfxChannelHeight::new(cast_length!("height", height)?),
        }]);
        let channels = rfx::Headers::Channels(channels);
        let version = rfx::CodecVersionsPdu;
        let version = rfx::Headers::CodecVersions(version);

        // data messages
        let frame_begin = rfx::FrameBeginPdu {
            index: 0,
            number_of_regions: 1,
        };
        let width = bitmap.width.get();
        let height = bitmap.height.get();
        let rectangles = vec![rfx::RfxRectangle {
            x: 0,
            y: 0,
            width,
            height,
        }];
        let region = rfx::RegionPdu { rectangles };
        let quant = rfx::Quant::default();

        let (encoder, mut data) = UpdateEncoder::new(bitmap, quant.clone(), entropy_algorithm);
        let tiles = encoder.encode(&mut data)?;

        let quants = vec![quant];
        let tile_set = rfx::TileSetPdu {
            entropy_algorithm,
            quants,
            tiles,
        };
        let frame_end = rfx::FrameEndPdu;

        macro_rules! encode {
            ($($element:expr),+) => {
                {
                    let len: usize = 0 $( + $element.buffer_length() )+;
                    let mut output = vec![0; len];
                    let mut buffer = output.as_mut_slice();

                    $(
                        $element.to_buffer_consume(&mut buffer).map_err(|e| other_err!("rfxenc", source: e))?;
                    )+

                    Ok(output)
                }
            };
        }

        encode!(
            sync,
            context,
            channels,
            version,
            frame_begin,
            region,
            tile_set,
            frame_end
        )
    }
}

pub(crate) struct UpdateEncoder<'a> {
    bitmap: &'a BitmapUpdate,
    quant: rfx::Quant,
    entropy_algorithm: rfx::EntropyAlgorithm,
}

struct UpdateEncoderData(Vec<u8>);

struct EncodedTile<'a> {
    y_data: &'a [u8],
    cb_data: &'a [u8],
    cr_data: &'a [u8],
}

impl<'a> UpdateEncoder<'a> {
    fn new(
        bitmap: &'a BitmapUpdate,
        quant: rfx::Quant,
        entropy_algorithm: rfx::EntropyAlgorithm,
    ) -> (Self, UpdateEncoderData) {
        let this = Self {
            bitmap,
            quant,
            entropy_algorithm,
        };
        let data = this.alloc_data();

        (this, data)
    }

    fn alloc_data(&self) -> UpdateEncoderData {
        let (tiles_x, tiles_y) = self.tiles_xy();

        UpdateEncoderData(vec![0u8; 64 * 64 * 3 * tiles_x * tiles_y])
    }

    fn tiles_xy(&self) -> (usize, usize) {
        (
            self.bitmap.width.get().div_ceil(64).into(),
            self.bitmap.height.get().div_ceil(64).into(),
        )
    }

    fn encode(&self, data: &'a mut UpdateEncoderData) -> EncodeResult<Vec<rfx::Tile<'a>>> {
        let (tiles_x, tiles_y) = self.tiles_xy();

        let chunks = data.0.chunks_mut(64 * 64 * 3);
        let tiles: Vec<_> = (0..tiles_y).flat_map(|y| (0..tiles_x).map(move |x| (x, y))).collect();

        chunks
            .zip(tiles)
            .map(|(buf, (tile_x, tile_y))| {
                let EncodedTile {
                    y_data,
                    cb_data,
                    cr_data,
                } = self
                    .encode_tile(tile_x, tile_y, buf)
                    .map_err(|e| other_err!("rfxenc", source: e))?;

                let tile = rfx::Tile {
                    y_quant_index: 0,
                    cb_quant_index: 0,
                    cr_quant_index: 0,
                    x: u16::try_from(tile_x).unwrap(),
                    y: u16::try_from(tile_y).unwrap(),
                    y_data,
                    cb_data,
                    cr_data,
                };
                Ok(tile)
            })
            .collect()
    }

    fn encode_tile<'b>(&self, tile_x: usize, tile_y: usize, buf: &'b mut [u8]) -> Result<EncodedTile<'b>, RlgrError> {
        assert!(buf.len() >= 4096 * 3);

        let bpp: usize = self.bitmap.format.bytes_per_pixel().into();
        let width: usize = self.bitmap.width.get().into();
        let height: usize = self.bitmap.height.get().into();

        let x = tile_x * 64;
        let y = tile_y * 64;
        let tile_width = std::cmp::min(width - x, 64);
        let tile_height = std::cmp::min(height - y, 64);
        let input = &self.bitmap.data[y * self.bitmap.stride + x * bpp..];

        let y = &mut [0i16; 4096];
        let cb = &mut [0i16; 4096];
        let cr = &mut [0i16; 4096];
        to_64x64_ycbcr_tile(
            input,
            tile_width,
            tile_height,
            self.bitmap.stride,
            self.bitmap.format,
            y,
            cb,
            cr,
        );

        let (y_data, buf) = buf.split_at_mut(4096);
        let (cb_data, cr_data) = buf.split_at_mut(4096);

        let len = rfx_encode_component(y, y_data, &self.quant, self.entropy_algorithm)?;
        let y_data = &y_data[..len];
        let len = rfx_encode_component(cb, cb_data, &self.quant, self.entropy_algorithm)?;
        let cb_data = &cb_data[..len];
        let len = rfx_encode_component(cr, cr_data, &self.quant, self.entropy_algorithm)?;
        let cr_data = &cr_data[..len];

        Ok(EncodedTile {
            y_data,
            cb_data,
            cr_data,
        })
    }
}

#[cfg(feature = "__bench")]
pub(crate) mod bench {
    use super::*;

    pub fn rfx_enc_tile(
        bitmap: &BitmapUpdate,
        quant: &rfx::Quant,
        algo: rfx::EntropyAlgorithm,
        tile_x: usize,
        tile_y: usize,
    ) {
        let (enc, mut data) = UpdateEncoder::new(bitmap, quant.clone(), algo);

        enc.encode_tile(tile_x, tile_y, &mut data.0).unwrap();
    }

    pub fn rfx_enc(bitmap: &BitmapUpdate, quant: &rfx::Quant, algo: rfx::EntropyAlgorithm) {
        let (enc, mut data) = UpdateEncoder::new(bitmap, quant.clone(), algo);

        enc.encode(&mut data).unwrap();
    }
}
