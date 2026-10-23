use core::cmp::min;

use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::rectangle_processing::Region;
use ironrdp_graphics::rfx::{TILE_SIZE, TileDecoder};
use ironrdp_pdu::codecs::rfx::{self, EntropyAlgorithm, RfxRectangle, Tile};
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp_pdu::{Decode as _, ReadCursor, decode_cursor};
use tracing::{instrument, trace};

use crate::image::DecodedImage;
use crate::{SessionResult, custom_err, general_err, reason_err};

pub type FrameId = u32;

pub struct DecodingContext {
    context: rfx::ContextPdu,
    channels: rfx::ChannelsPdu,
    tile_decoder: TileDecoder,
}

impl Default for DecodingContext {
    fn default() -> Self {
        Self {
            context: rfx::ContextPdu {
                flags: rfx::OperatingMode::empty(),
                entropy_algorithm: EntropyAlgorithm::Rlgr1,
            },
            channels: rfx::ChannelsPdu(Vec::new()),
            tile_decoder: TileDecoder::new(),
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
        destination: &InclusiveRectangle,
        input: &mut ReadCursor<'_>,
    ) -> SessionResult<(FrameId, InclusiveRectangle)> {
        loop {
            let block = rfx::Block::decode(input).map_err(|e| custom_err!("decode block", e))?;

            match block {
                rfx::Block::Sync(_) => {
                    self.process_sync(input)?;
                }
                rfx::Block::CodecChannel(rfx::CodecChannel::FrameBegin(f)) => {
                    return self.process_frame(f, input, image, destination);
                }
                _ => {
                    return Err(reason_err!(
                        "rfx::DecodingContext",
                        "unexpected RFX block type: {:?}",
                        block.block_type()
                    ));
                }
            }
        }
    }

    fn process_sync(&mut self, input: &mut ReadCursor<'_>) -> SessionResult<()> {
        self.process_headers(input)
    }

    fn process_headers(&mut self, input: &mut ReadCursor<'_>) -> SessionResult<()> {
        let mut context = None;
        let mut channels = None;

        // headers can appear in any order: CodecVersions, Channels, Context
        for _ in 0..3 {
            match decode_cursor(input).map_err(|e| custom_err!("decode headers", e))? {
                rfx::Block::CodecChannel(rfx::CodecChannel::Context(c)) => context = Some(c),
                rfx::Block::Channels(c) => channels = Some(c),
                rfx::Block::CodecVersions(_) => (),
                _ => {
                    return Err(general_err!("unexpected RFX block type"));
                }
            }
        }

        let context = context.ok_or_else(|| general_err!("context header is missing"))?;
        let channels = channels.ok_or_else(|| general_err!("channels header is missing"))?;

        if channels.0.is_empty() {
            return Err(general_err!("no RFX channel announced"));
        }

        self.context = context;
        self.channels = channels;

        Ok(())
    }

    #[instrument(skip_all)]
    fn process_frame(
        &mut self,
        frame_begin: rfx::FrameBeginPdu,
        input: &mut ReadCursor<'_>,
        image: &mut DecodedImage,
        destination: &InclusiveRectangle,
    ) -> SessionResult<(FrameId, InclusiveRectangle)> {
        let channel = self
            .channels
            .0
            .first()
            .ok_or_else(|| general_err!("no RFX channel found"))?;
        let width = channel.width.try_into().map_err(|_| general_err!("invalid width"))?;
        let height = channel.height.try_into().map_err(|_| general_err!("invalid height"))?;
        let entropy_algorithm = self.context.entropy_algorithm;

        let region: rfx::Block<'_> = decode_cursor(input).map_err(|e| custom_err!("decode region", e))?;
        let mut region = match region {
            rfx::Block::CodecChannel(rfx::CodecChannel::Region(region)) => region,
            _ => return Err(general_err!("unexpected block type")),
        };
        let tile_set: rfx::Block<'_> = decode_cursor(input).map_err(|e| custom_err!("decode tile_set", e))?;
        let tile_set = match tile_set {
            rfx::Block::CodecChannel(rfx::CodecChannel::TileSet(t)) => t,
            _ => return Err(general_err!("unexpected block type")),
        };
        let frame_end: rfx::Block<'_> = decode_cursor(input).map_err(|e| custom_err!("decode frame_end", e))?;
        if !matches!(frame_end, rfx::Block::CodecChannel(rfx::CodecChannel::FrameEnd(_))) {
            return Err(general_err!("unexpected block type"));
        }

        if region.rectangles.is_empty() {
            region.rectangles = vec![RfxRectangle {
                x: 0,
                y: 0,
                width,
                height,
            }];
        }
        let region = region;

        trace!(frame_index = frame_begin.index);
        trace!(destination_rectangle = ?destination);
        trace!(context = ?self.context);
        trace!(channels = ?self.channels);
        trace!(?region);

        let clipping_rectangles = clipping_rectangles(region.rectangles.as_slice(), destination, width, height);
        trace!("Clipping rectangles: {:?}", clipping_rectangles);

        let mut final_update_rectangle = clipping_rectangles.extents.clone();

        for (update_rectangle, tile) in tiles_to_rectangles(tile_set.tiles.as_slice(), destination).zip(&tile_set.tiles)
        {
            let Some(update_rectangle) = update_rectangle else {
                continue;
            };

            let tile_output = self
                .tile_decoder
                .decode(tile, &tile_set.quants, entropy_algorithm)
                .map_err(|e| custom_err!("decode_tile", e))?;

            let current_update_rectangle = image.apply_tile(
                tile_output,
                PixelFormat::RgbA32,
                &clipping_rectangles,
                &update_rectangle,
            )?;

            final_update_rectangle = final_update_rectangle.union(&current_update_rectangle);
        }

        Ok((frame_begin.index, final_update_rectangle))
    }
}

#[cfg_attr(feature = "__test", visibility::make(pub))]
fn clipping_rectangles(
    rectangles: &[RfxRectangle],
    destination: &InclusiveRectangle,
    width: u16,
    height: u16,
) -> Region {
    let mut clipping_rectangles = Region::new();

    // Exclusive bounds in u32, so that no server-supplied position or size can wrap.
    let right_limit = min(u32::from(destination.left) + u32::from(width), u32::from(u16::MAX) + 1);
    let bottom_limit = min(u32::from(destination.top) + u32::from(height), u32::from(u16::MAX) + 1);

    rectangles
        .iter()
        .filter_map(|r| {
            let left = min(u32::from(destination.left) + u32::from(r.x), right_limit);
            let top = min(u32::from(destination.top) + u32::from(r.y), bottom_limit);
            let right = min(left + u32::from(r.width), right_limit);
            let bottom = min(top + u32::from(r.height), bottom_limit);

            // An empty rectangle covers nothing.
            if left == right || top == bottom {
                return None;
            }

            // INVARIANT: left < right <= right_limit <= u16::MAX + 1, and likewise for top and bottom,
            // so every conversion below succeeds.
            Some(InclusiveRectangle {
                left: u16::try_from(left).ok()?,
                top: u16::try_from(top).ok()?,
                right: u16::try_from(right - 1).ok()?,
                bottom: u16::try_from(bottom - 1).ok()?,
            })
        })
        .for_each(|r| clipping_rectangles.union_rectangle(r));

    clipping_rectangles
}

/// Returns `None` for a tile that starts past the u16 range, since no clipping rectangle can reach it.
/// A tile crossing the end of the range is cut there, like the clipping rectangles.
#[cfg_attr(feature = "__test", visibility::make(pub))]
fn tiles_to_rectangles<'a>(
    tiles: &'a [Tile<'_>],
    destination: &'a InclusiveRectangle,
) -> impl Iterator<Item = Option<InclusiveRectangle>> + 'a {
    tiles.iter().map(|t| {
        let left = t.x.checked_mul(TILE_SIZE)?.checked_add(destination.left)?;
        let top = t.y.checked_mul(TILE_SIZE)?.checked_add(destination.top)?;

        Some(InclusiveRectangle {
            left,
            top,
            right: left.saturating_add(TILE_SIZE - 1),
            bottom: top.saturating_add(TILE_SIZE - 1),
        })
    })
}
