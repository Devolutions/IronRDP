use std::cmp::min;

use ironrdp_graphics::color_conversion::{self, YCbCrBuffer};
use ironrdp_graphics::rectangle_processing::Region;
use ironrdp_graphics::{dwt, quantization, rlgr, subband_reconstruction};
use ironrdp_pdu::codecs::rfx::{self, EntropyAlgorithm, Headers, Quant, RfxRectangle, Tile};
use ironrdp_pdu::geometry::Rectangle;
use ironrdp_pdu::PduBufferParsing;

use crate::image::DecodedImage;
use crate::{Error, Result};

const TILE_SIZE: u16 = 64;

pub type FrameId = u32;

pub struct DecodingContext {
    state: SequenceState,
    context: rfx::ContextPdu,
    channels: rfx::ChannelsPdu,
    decoding_tiles: DecodingTileContext,
}

impl Default for DecodingContext {
    fn default() -> Self {
        Self {
            state: SequenceState::HeaderMessages,
            context: rfx::ContextPdu {
                flags: rfx::OperatingMode::empty(),
                entropy_algorithm: rfx::EntropyAlgorithm::Rlgr1,
            },
            channels: rfx::ChannelsPdu(vec![]),
            decoding_tiles: DecodingTileContext::new(),
        }
    }
}

impl DecodingContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn decode(
        &mut self,
        image: &mut DecodedImage,
        destination: &Rectangle,
        input: &mut &[u8],
    ) -> Result<(FrameId, Rectangle)> {
        loop {
            match self.state {
                SequenceState::HeaderMessages => {
                    self.process_headers(input)?;
                }
                SequenceState::DataMessages => {
                    return self.process_data_messages(image, destination, input);
                }
            }
        }
    }

    fn process_headers(&mut self, input: &mut &[u8]) -> Result<()> {
        let _sync = rfx::SyncPdu::from_buffer_consume(input)?;

        let mut context = None;
        let mut channels = None;

        // headers can appear in any order: CodecVersions, Channels, Context
        for _ in 0..3 {
            match Headers::from_buffer_consume(input)? {
                Headers::Context(c) => context = Some(c),
                Headers::Channels(c) => channels = Some(c),
                Headers::CodecVersions(_) => (),
            }
        }
        let context = context.ok_or(Error::new("context header is missing"))?;
        let channels = channels.ok_or(Error::new("channels header is missing"))?;

        if channels.0.is_empty() {
            return Err(Error::new("no RFX channel announced"));
        }

        self.context = context;
        self.channels = channels;
        self.state = SequenceState::DataMessages;

        Ok(())
    }

    #[instrument(skip_all)]
    fn process_data_messages(
        &mut self,
        image: &mut DecodedImage,
        destination: &Rectangle,
        input: &mut &[u8],
    ) -> Result<(FrameId, Rectangle)> {
        let channel = self.channels.0.first().unwrap();
        let width = channel.width as u16;
        let height = channel.height as u16;
        let entropy_algorithm = self.context.entropy_algorithm;

        let frame_begin = rfx::FrameBeginPdu::from_buffer_consume(input)?;
        let mut region = rfx::RegionPdu::from_buffer_consume(input)?;
        let tile_set = rfx::TileSetPdu::from_buffer_consume(input)?;
        let _frame_end = rfx::FrameEndPdu::from_buffer_consume(input)?;

        if region.rectangles.is_empty() {
            region.rectangles = vec![RfxRectangle {
                x: 0,
                y: 0,
                width: channel.width as u16,
                height: channel.height as u16,
            }];
        }
        let region = region;

        debug!(frame_index = frame_begin.index);
        trace!(destination_rectangle = ?destination);
        trace!(context = ?self.context);
        trace!(channels = ?self.channels);
        trace!(?region);

        let clipping_rectangles = clipping_rectangles(region.rectangles.as_slice(), destination, width, height);
        trace!("Clipping rectangles: {:?}", clipping_rectangles);

        for (update_rectangle, tile_data) in tiles_to_rectangles(tile_set.tiles.as_slice(), destination)
            .zip(map_tiles_data(tile_set.tiles.as_slice(), tile_set.quants.as_slice()))
        {
            decode_tile(
                &tile_data,
                entropy_algorithm,
                self.decoding_tiles.tile_output.as_mut(),
                self.decoding_tiles.ycbcr_buffer.as_mut(),
                self.decoding_tiles.ycbcr_temp_buffer.as_mut(),
            )?;

            image.apply_tile(
                &self.decoding_tiles.tile_output,
                &clipping_rectangles,
                &update_rectangle,
                width,
            )?;
        }

        if self.context.flags.contains(rfx::OperatingMode::IMAGE_MODE) {
            self.state = SequenceState::HeaderMessages;
        }

        Ok((frame_begin.index, clipping_rectangles.extents))
    }
}

#[derive(Debug, Clone)]
struct DecodingTileContext {
    pub tile_output: Vec<u8>,
    pub ycbcr_buffer: Vec<Vec<i16>>,
    pub ycbcr_temp_buffer: Vec<i16>,
}

impl DecodingTileContext {
    fn new() -> Self {
        Self {
            tile_output: vec![0; TILE_SIZE as usize * TILE_SIZE as usize * 4],
            ycbcr_buffer: vec![vec![0; TILE_SIZE as usize * TILE_SIZE as usize]; 3],
            ycbcr_temp_buffer: vec![0; TILE_SIZE as usize * TILE_SIZE as usize],
        }
    }
}

fn decode_tile(
    tile: &TileData<'_>,
    entropy_algorithm: EntropyAlgorithm,
    output: &mut [u8],
    ycbcr_temp: &mut [Vec<i16>],
    temp: &mut [i16],
) -> Result<()> {
    for ((quant, data), ycbcr_buffer) in tile.quants.iter().zip(tile.data.iter()).zip(ycbcr_temp.iter_mut()) {
        decode_component(quant, entropy_algorithm, data, ycbcr_buffer.as_mut_slice(), temp)?;
    }

    let ycbcr_buffer = YCbCrBuffer {
        y: ycbcr_temp[0].as_slice(),
        cb: ycbcr_temp[1].as_slice(),
        cr: ycbcr_temp[2].as_slice(),
    };

    color_conversion::ycbcr_to_bgra(ycbcr_buffer, output).map_err(|e| Error::new("decode_tile").with_custom(e))?;

    Ok(())
}

fn decode_component(
    quant: &Quant,
    entropy_algorithm: EntropyAlgorithm,
    data: &[u8],
    output: &mut [i16],
    temp: &mut [i16],
) -> Result<()> {
    rlgr::decode(entropy_algorithm, data, output).map_err(|e| Error::new("decode_component").with_custom(e))?;
    subband_reconstruction::decode(&mut output[4032..]);
    quantization::decode(output, quant);
    dwt::decode(output, temp);

    Ok(())
}

fn clipping_rectangles(rectangles: &[RfxRectangle], destination: &Rectangle, width: u16, height: u16) -> Region {
    let mut clipping_rectangles = Region::new();

    rectangles
        .iter()
        .map(|r| Rectangle {
            left: min(destination.left + r.x, width - 1),
            top: min(destination.top + r.y, height - 1),
            right: min(destination.left + r.x + r.width, width - 1),
            bottom: min(destination.top + r.y + r.height, height - 1),
        })
        .for_each(|r| clipping_rectangles.union_rectangle(r));

    clipping_rectangles
}

fn tiles_to_rectangles<'a>(tiles: &'a [Tile<'_>], destination: &'a Rectangle) -> impl Iterator<Item = Rectangle> + 'a {
    tiles.iter().map(|t| Rectangle {
        left: destination.left + t.x * TILE_SIZE,
        top: destination.top + t.y * TILE_SIZE,
        right: destination.left + t.x * TILE_SIZE + TILE_SIZE,
        bottom: destination.top + t.y * TILE_SIZE + TILE_SIZE,
    })
}

fn map_tiles_data<'a>(tiles: &'_ [Tile<'a>], quants: &'_ [Quant]) -> Vec<TileData<'a>> {
    tiles
        .iter()
        .map(|t| TileData {
            quants: [
                quants[usize::from(t.y_quant_index)].clone(),
                quants[usize::from(t.cb_quant_index)].clone(),
                quants[usize::from(t.cr_quant_index)].clone(),
            ],
            data: [t.y_data, t.cb_data, t.cr_data],
        })
        .collect()
}

struct TileData<'a> {
    pub quants: [Quant; 3],
    pub data: [&'a [u8]; 3],
}

enum SequenceState {
    HeaderMessages,
    DataMessages,
}
