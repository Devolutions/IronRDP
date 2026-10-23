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

fn clipping_rectangles(
    rectangles: &[RfxRectangle],
    destination: &InclusiveRectangle,
    width: u16,
    height: u16,
) -> Region {
    let mut clipping_rectangles = Region::new();

    rectangles
        .iter()
        .map(|r| InclusiveRectangle {
            left: min(destination.left + r.x, destination.left + width - 1),
            top: min(destination.top + r.y, destination.top + height - 1),
            right: min(destination.left + r.x + r.width - 1, destination.left + width - 1),
            bottom: min(destination.top + r.y + r.height - 1, destination.top + height - 1),
        })
        .for_each(|r| clipping_rectangles.union_rectangle(r));

    clipping_rectangles
}

fn tiles_to_rectangles<'a>(
    tiles: &'a [Tile<'_>],
    destination: &'a InclusiveRectangle,
) -> impl Iterator<Item = InclusiveRectangle> + 'a {
    tiles.iter().map(|t| InclusiveRectangle {
        left: destination.left + t.x * TILE_SIZE,
        top: destination.top + t.y * TILE_SIZE,
        right: destination.left + t.x * TILE_SIZE + TILE_SIZE - 1,
        bottom: destination.top + t.y * TILE_SIZE + TILE_SIZE - 1,
    })
}

#[cfg(test)]
mod tests {
    use ironrdp_pdu::codecs::rfx::RfxRectangle;
    use ironrdp_pdu::geometry::InclusiveRectangle;

    use super::clipping_rectangles;

    /// A RemoteFX surface for a secondary monitor is placed at a non-zero
    /// framebuffer offset, and its channel (frame) dimensions describe only
    /// that surface's own content rather than the entire desktop.
    #[test]
    fn clipping_rectangles_track_surface_framebuffer_offset() {
        // Update for a second monitor with origin (2000, 1000) and dimensions 1000x1200
        let destination = InclusiveRectangle {
            left: 2000,
            top: 1000,
            right: 2999,
            bottom: 2199,
        };
        let channel_width = 1000;
        let channel_height = 1200;

        let rects = [RfxRectangle {
            x: 0,
            y: 0,
            width: channel_width,
            height: channel_height,
        }];

        let region = clipping_rectangles(&rects, &destination, channel_width, channel_height);

        assert_eq!(
            region.extents.left, 2000,
            "clip collapsed away from the surface's framebuffer position (left)"
        );
        assert_eq!(
            region.extents.right, 2999,
            "clip collapsed away from the surface's framebuffer position (right)"
        );
        assert_eq!(
            region.extents.top, 1000,
            "clip collapsed away from the surface's framebuffer position (top)"
        );
        assert_eq!(
            region.extents.bottom, 2199,
            "clip collapsed away from the surface's framebuffer position (bottom)"
        );
    }
}
