#[cfg(test)]
mod tests;

use std::{
    cmp::min,
    sync::{Arc, Mutex},
};

use ironrdp::{
    codecs::rfx::{
        self, color_conversion,
        color_conversion::YCbCrBuffer,
        dwt,
        image_processing::{ImageRegion, ImageRegionMut, PixelFormat},
        quantization,
        rectangles_processing::Region,
        rlgr, subband_reconstruction, EntropyAlgorithm, Headers, Quant, RfxRectangle, Tile,
    },
    PduBufferParsing, Rectangle,
};
use lazy_static::lazy_static;
use log::debug;

use crate::{active_session::DecodedImage, RdpError};

const TILE_SIZE: u16 = 64;
const SOURCE_PIXEL_FORMAT: PixelFormat = PixelFormat::BgrX32;

lazy_static! {
    static ref SOURCE_STRIDE: u16 = TILE_SIZE * u16::from(SOURCE_PIXEL_FORMAT.bytes_per_pixel());
}

pub type FrameId = u32;

pub struct DecodingContext {
    state: SequenceState,
    context: rfx::ContextPdu,
    channels: rfx::ChannelsPdu,
    destination_pixel_format: PixelFormat,
    decoding_tiles: DecodingTileContext,
}

impl DecodingContext {
    pub fn new(destination_pixel_format: PixelFormat) -> Self {
        Self {
            state: SequenceState::HeaderMessages,
            context: rfx::ContextPdu {
                flags: rfx::OperatingMode::empty(),
                entropy_algorithm: rfx::EntropyAlgorithm::Rlgr1,
            },
            channels: rfx::ChannelsPdu(vec![]),
            destination_pixel_format,
            decoding_tiles: DecodingTileContext::new(),
        }
    }

    pub fn decode(
        &mut self,
        destination: &Rectangle,
        input: &mut &[u8],
        image: Arc<Mutex<DecodedImage>>,
    ) -> Result<FrameId, RdpError> {
        loop {
            match self.state {
                SequenceState::HeaderMessages => {
                    self.process_headers(input)?;
                }
                SequenceState::DataMessages => {
                    let frame_id = self.process_data_messages(destination, input, image)?;

                    return Ok(frame_id);
                }
            }
        }
    }

    fn process_headers(&mut self, input: &mut &[u8]) -> Result<(), RdpError> {
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
        let context = context.ok_or(RdpError::MandatoryHeaderIsAbsent)?;
        let channels = channels.ok_or(RdpError::MandatoryHeaderIsAbsent)?;

        if channels.0.is_empty() {
            return Err(RdpError::NoRfxChannelsAnnounced);
        }

        self.context = context;
        self.channels = channels;
        self.state = SequenceState::DataMessages;

        Ok(())
    }

    fn process_data_messages(
        &mut self,
        destination: &Rectangle,
        input: &mut &[u8],
        image: Arc<Mutex<DecodedImage>>,
    ) -> Result<FrameId, RdpError> {
        let width = self.channels.0.first().unwrap().width as u16;
        let height = self.channels.0.first().unwrap().height as u16;
        let entropy_algorithm = self.context.entropy_algorithm;

        let frame_begin = rfx::FrameBeginPdu::from_buffer_consume(input)?;
        let mut region = rfx::RegionPdu::from_buffer_consume(input)?;
        let tile_set = rfx::TileSetPdu::from_buffer_consume(input)?;
        let _frame_end = rfx::FrameEndPdu::from_buffer_consume(input)?;

        if region.rectangles.is_empty() {
            let channel = self.channels.0.first().unwrap();
            region.rectangles = vec![RfxRectangle {
                x: 0,
                y: 0,
                width: channel.width as u16,
                height: channel.height as u16,
            }];
        }
        let region = region;

        debug!("Frame #{}: ", frame_begin.index);
        debug!("Destination rectangle: {:?}", destination);
        debug!("Context: {:?}", self.context);
        debug!("Channels: {:?}", self.channels);
        debug!("Region: {:?}", region);

        let clipping_rectangles =
            clipping_rectangles(region.rectangles.as_slice(), destination, width, height);
        debug!("Clipping rectangles: {:?}", clipping_rectangles);
        let clipping_rectangles_ref = &clipping_rectangles;

        for (update_rectangle, tile_data) in
            tiles_to_rectangles(tile_set.tiles.as_slice(), destination).zip(map_tiles_data(
                tile_set.tiles.as_slice(),
                tile_set.quants.as_slice(),
            ))
        {
            decode_tile(
                &tile_data,
                entropy_algorithm,
                self.decoding_tiles.tile_output.as_mut(),
                self.decoding_tiles.ycbcr_buffer.as_mut(),
                self.decoding_tiles.ycbcr_temp_buffer.as_mut(),
            )?;

            process_decoded_tile(
                self.decoding_tiles.tile_output.as_slice(),
                clipping_rectangles_ref,
                &update_rectangle,
                width,
                self.destination_pixel_format,
                &image,
            )?;
        }

        if self.context.flags.contains(rfx::OperatingMode::IMAGE_MODE) {
            self.state = SequenceState::HeaderMessages;
        }

        Ok(frame_begin.index)
    }
}

fn process_decoded_tile(
    tile_output: &[u8],
    clipping_rectangles: &Region,
    update_rectangle: &Rectangle,
    width: u16,
    destination_pixel_format: PixelFormat,
    image: &Arc<Mutex<DecodedImage>>,
) -> Result<(), RdpError> {
    debug!("Tile: {:?}", update_rectangle);

    let update_region = clipping_rectangles.intersect_rectangle(update_rectangle);
    for region_rectangle in update_region.rectangles() {
        let source_x = region_rectangle.left - update_rectangle.left;
        let source_y = region_rectangle.top - update_rectangle.top;
        let source_image_region = ImageRegion {
            region: Rectangle {
                left: source_x,
                top: source_y,
                right: source_x + region_rectangle.width(),
                bottom: source_y + region_rectangle.height(),
            },
            step: *SOURCE_STRIDE,
            pixel_format: SOURCE_PIXEL_FORMAT,
            data: tile_output,
        };

        let mut output = image.lock().unwrap();
        let mut destination_image_region = ImageRegionMut {
            region: region_rectangle.clone(),
            step: width * u16::from(SOURCE_PIXEL_FORMAT.bytes_per_pixel()),
            pixel_format: destination_pixel_format,
            data: output.get_mut(),
        };
        debug!("Source image region: {:?}", source_image_region.region);
        debug!(
            "Destination image region: {:?}",
            destination_image_region.region
        );

        source_image_region.copy_to(&mut destination_image_region)?;
    }

    Ok(())
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
) -> Result<(), RdpError> {
    for ((quant, data), ycbcr_buffer) in tile
        .quants
        .iter()
        .zip(tile.data.iter())
        .zip(ycbcr_temp.iter_mut())
    {
        decode_component(
            quant,
            entropy_algorithm,
            data,
            ycbcr_buffer.as_mut_slice(),
            temp,
        )?;
    }

    let ycbcr_buffer = YCbCrBuffer {
        y: ycbcr_temp[0].as_slice(),
        cb: ycbcr_temp[1].as_slice(),
        cr: ycbcr_temp[2].as_slice(),
    };

    color_conversion::ycbcr_to_rgb(ycbcr_buffer, output)?;

    Ok(())
}

fn decode_component(
    quant: &Quant,
    entropy_algorithm: EntropyAlgorithm,
    data: &[u8],
    output: &mut [i16],
    temp: &mut [i16],
) -> Result<(), RdpError> {
    rlgr::decode(entropy_algorithm, data, output)?;
    subband_reconstruction::decode(&mut output[4032..]);
    quantization::decode(output, quant);
    dwt::decode(output, temp);

    Ok(())
}

fn clipping_rectangles(
    rectangles: &[RfxRectangle],
    destination: &Rectangle,
    width: u16,
    height: u16,
) -> Region {
    let mut clipping_rectangles = Region::new();

    rectangles
        .iter()
        .map(|r| Rectangle {
            left: min(destination.left + r.x, width),
            top: min(destination.top + r.y, height),
            right: min(destination.left + r.x + r.width, width),
            bottom: min(destination.top + r.y + r.height, height),
        })
        .for_each(|r| clipping_rectangles.union_rectangle(r));

    clipping_rectangles
}

fn tiles_to_rectangles<'a>(
    tiles: &'a [Tile<'_>],
    destination: &'a Rectangle,
) -> impl Iterator<Item = Rectangle> + 'a {
    tiles.iter().map(move |t| Rectangle {
        left: destination.left + t.x * TILE_SIZE,
        top: destination.top + t.y * TILE_SIZE,
        right: destination.left + t.x * TILE_SIZE + TILE_SIZE,
        bottom: destination.top + t.y * TILE_SIZE + TILE_SIZE,
    })
}

fn map_tiles_data<'a>(tiles: &'_ [Tile<'a>], quants: &'_ [Quant]) -> Vec<TileData<'a>> {
    tiles
        .iter()
        .map(move |t| TileData {
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
