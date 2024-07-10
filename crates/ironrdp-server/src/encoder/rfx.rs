use ironrdp_graphics::color_conversion::to_64x64_ycbcr_tile;
use ironrdp_graphics::rfx_encode_component;
use ironrdp_pdu::codecs::rfx::{self, OperatingMode, RfxChannel, RfxChannelHeight, RfxChannelWidth};
use ironrdp_pdu::rdp::capability_sets::EntropyBits;
use ironrdp_pdu::{custom_err, PduBufferParsing, PduError};

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
    pub(crate) fn encode(&mut self, bitmap: &BitmapUpdate) -> Result<Vec<u8>, PduError> {
        let width = 2042;
        let height = 2043;
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
            width: RfxChannelWidth::new(width),
            height: RfxChannelHeight::new(height),
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

        let bpp = usize::from(bitmap.format.bytes_per_pixel());
        let width = usize::from(bitmap.width.get());
        let height = usize::from(bitmap.height.get());
        let stride = width * bpp;

        let tiles_x = (width + 63) / 64;
        let tiles_y = (height + 63) / 64;
        let ntiles = tiles_x * tiles_y;
        let mut tiles = Vec::with_capacity(ntiles);
        let mut data = vec![0u8; 64 * 64 * 3 * ntiles];
        let mut rest = data.as_mut_slice();

        for tile_y in 0..tiles_y {
            for tile_x in 0..tiles_x {
                let x = tile_x * 64;
                let y = tile_y * 64;
                let tile_width = std::cmp::min(width - x, 64);
                let tile_height = std::cmp::min(height - y, 64);

                let input = &bitmap.data[y * stride + x * bpp..];

                let y = &mut [0i16; 4096];
                let cb = &mut [0i16; 4096];
                let cr = &mut [0i16; 4096];
                to_64x64_ycbcr_tile(input, tile_width, tile_height, stride, bitmap.format, y, cb, cr);

                let (y_data, new_rest) = rest.split_at_mut(4096);
                let (cb_data, new_rest) = new_rest.split_at_mut(4096);
                let (cr_data, new_rest) = new_rest.split_at_mut(4096);
                rest = new_rest;
                let len = rfx_encode_component(y, y_data, &quant, entropy_algorithm).map_err(|e| custom_err!(e))?;
                let y_data = &y_data[..len];
                let len = rfx_encode_component(cb, cb_data, &quant, entropy_algorithm).map_err(|e| custom_err!(e))?;
                let cb_data = &cb_data[..len];
                let len = rfx_encode_component(cr, cr_data, &quant, entropy_algorithm).map_err(|e| custom_err!(e))?;
                let cr_data = &cr_data[..len];

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
                tiles.push(tile);
            }
        }

        let quants = vec![quant];
        let tile_set = rfx::TileSetPdu {
            entropy_algorithm,
            quants,
            tiles,
        };
        let frame_end = rfx::FrameEndPdu;

        let len = sync.buffer_length()
            + context.buffer_length()
            + channels.buffer_length()
            + version.buffer_length()
            + frame_begin.buffer_length()
            + region.buffer_length()
            + tile_set.buffer_length()
            + frame_end.buffer_length();
        let mut output = vec![0; len];
        let mut buffer = output.as_mut_slice();
        sync.to_buffer_consume(&mut buffer).map_err(|e| custom_err!(e))?;
        context.to_buffer_consume(&mut buffer).map_err(|e| custom_err!(e))?;
        channels.to_buffer_consume(&mut buffer).map_err(|e| custom_err!(e))?;
        version.to_buffer_consume(&mut buffer).map_err(|e| custom_err!(e))?;
        frame_begin.to_buffer_consume(&mut buffer).map_err(|e| custom_err!(e))?;
        region.to_buffer_consume(&mut buffer).map_err(|e| custom_err!(e))?;
        tile_set.to_buffer_consume(&mut buffer).map_err(|e| custom_err!(e))?;
        frame_end.to_buffer_consume(&mut buffer).map_err(|e| custom_err!(e))?;
        Ok(output)
    }
}
