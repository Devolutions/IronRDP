use std::io;

use bytes::BytesMut;
use ironrdp_core::codecs::rfx::FrameAcknowledgePdu;
use ironrdp_core::fast_path::{
    FastPathError, FastPathHeader, FastPathUpdate, FastPathUpdatePdu, Fragmentation, UpdateCode,
};
use ironrdp_core::geometry::Rectangle;
use ironrdp_core::surface_commands::{FrameAction, FrameMarkerPdu, SurfaceCommand};
use ironrdp_core::{PduBufferParsing, ShareDataPdu};
use num_traits::FromPrimitive;

use super::codecs::rfx;
use crate::frame::{Frame as _, ShareDataFrame};
use crate::image::DecodedImage;
use crate::utils::CodecId;
use crate::{ChannelIdentificators, RdpError};

pub struct Processor {
    complete_data: CompleteData,
    rfx_handler: rfx::DecodingContext,
    marker_processor: FrameMarkerProcessor,
}

impl Processor {
    // Returns true if image buffer was updated, false otherwise
    pub fn process(
        &mut self,
        image: &mut DecodedImage,
        header: &FastPathHeader,
        input: &[u8],
        mut output: impl io::Write,
    ) -> Result<Option<Rectangle>, RdpError> {
        debug!("Got Fast-Path Header: {:?}", header);

        let update_pdu = FastPathUpdatePdu::from_buffer(input)?;
        debug!("Fast-Path Update fragmentation: {:?}", update_pdu.fragmentation);

        let processed_complete_data = self
            .complete_data
            .process_data(update_pdu.data, update_pdu.fragmentation);
        let update_code = update_pdu.update_code;

        let Some(data) = processed_complete_data else {
            return Ok(None);
        };

        let update = FastPathUpdate::from_buffer_with_code(data.as_slice(), update_code);

        match update {
            Ok(FastPathUpdate::SurfaceCommands(surface_commands)) => {
                trace!("Received Surface Commands: {} pieces", surface_commands.len());
                let update_region = self.process_surface_commands(image, &mut output, surface_commands)?;
                Ok(Some(update_region))
            }
            Ok(FastPathUpdate::Bitmap(bitmap_update)) => {
                trace!("Received bitmap update");

                let mut buf = BytesMut::new();
                let mut update_rectangle: Option<Rectangle> = None;

                for update in bitmap_update.rectangles {
                    trace!("{update:?}");
                    buf.clear();

                    // Bitmap data is either compressed or uncompressed, depending
                    // on whether the BITMAP_COMPRESSION flag is present in the
                    // flags field.
                    if update
                        .compression_flags
                        .contains(ironrdp_core::bitmap::Compression::BITMAP_COMPRESSION)
                    {
                        if update.bits_per_pixel == 32 {
                            // Compressed bitmaps at a color depth of 32 bpp are compressed using RDP 6.0
                            // Bitmap Compression and stored inside an RDP 6.0 Bitmap Compressed Stream
                            // structure ([MS-RDPEGDI] section 2.2.2.5.1).
                            trace!("32 bpp compressed RDP6_BITMAP_STREAM");
                            warn!("RDP6_BITMAP_STREAM is not yet supported");
                        } else {
                            // Compressed bitmaps not in 32 bpp format are compressed using Interleaved
                            // RLE and encapsulated in an RLE Compressed Bitmap Stream structure (section
                            // 2.2.9.1.1.3.1.2.4).
                            trace!(
                                "Non-32 bpp compressed RLE_BITMAP_STREAM (bpp: {})",
                                update.bits_per_pixel
                            );

                            ironrdp_graphics::rle::decompress(
                                update.bitmap_data,
                                &mut buf,
                                update.width,
                                update.height,
                                update.bits_per_pixel,
                            );

                            // TODO: support other pixel formats…
                            image.apply_rgb16_bitmap(&buf, &update.rectangle);
                        }
                    } else {
                        // Uncompressed bitmap data is formatted as a bottom-up, left-to-right series of
                        // pixels. Each pixel is a whole number of bytes. Each row contains a multiple of
                        // four bytes (including up to three bytes of padding, as necessary).
                        trace!("Uncompressed raw bitmap");

                        // TODO: support other pixel formats…
                        image.apply_rgb16_bitmap(update.bitmap_data, &update.rectangle);
                    }

                    match update_rectangle {
                        Some(current) => update_rectangle = Some(current.union(&update.rectangle)),
                        None => update_rectangle = Some(update.rectangle),
                    }
                }

                Ok(update_rectangle)
            }
            Err(FastPathError::UnsupportedFastPathUpdate(code))
                if code == UpdateCode::Orders || code == UpdateCode::Palette =>
            {
                warn!("Received unsupported Fast-Path update: {code:?}");
                Ok(None)
            }
            Err(FastPathError::UnsupportedFastPathUpdate(update_code)) => {
                debug!("Received unsupported Fast-Path update: {:?}", update_code);
                Ok(None)
            }
            Err(FastPathError::BitmapError(error)) => {
                warn!("Received invalid bitmap: {:?}", error);
                Ok(None)
            }
            Err(e) => Err(RdpError::from(e)),
        }
    }

    fn process_surface_commands(
        &mut self,
        image: &mut DecodedImage,
        mut output: impl io::Write,
        surface_commands: Vec<SurfaceCommand<'_>>,
    ) -> Result<Rectangle, RdpError> {
        let mut update_rectangle = Rectangle::empty();

        for command in surface_commands {
            match command {
                SurfaceCommand::SetSurfaceBits(bits) | SurfaceCommand::StreamSurfaceBits(bits) => {
                    trace!("Surface bits");
                    let codec_id = CodecId::from_u8(bits.extended_bitmap_data.codec_id)
                        .ok_or(RdpError::UnexpectedCodecId(bits.extended_bitmap_data.codec_id))?;
                    match codec_id {
                        CodecId::RemoteFx => {
                            let destination = bits.destination;
                            let mut data = bits.extended_bitmap_data.data;

                            while !data.is_empty() {
                                let (_frame_id, rectangle) = self.rfx_handler.decode(image, &destination, &mut data)?;
                                update_rectangle = update_rectangle.union(&rectangle);
                            }
                        }
                    }
                }
                SurfaceCommand::FrameMarker(marker) => {
                    trace!(
                        "Frame marker: action {:?} with ID #{}",
                        marker.frame_action,
                        marker.frame_id.unwrap_or(0)
                    );
                    self.marker_processor.process(&marker, &mut output)?;
                }
            }
        }

        Ok(update_rectangle)
    }
}

pub struct ProcessorBuilder {
    pub global_channel_id: u16,
    pub initiator_id: u16,
}

impl ProcessorBuilder {
    pub fn build(self) -> Processor {
        Processor {
            complete_data: CompleteData::new(),
            rfx_handler: rfx::DecodingContext::new(),
            marker_processor: FrameMarkerProcessor::new(self.initiator_id, self.global_channel_id),
        }
    }
}

#[derive(Debug, PartialEq)]
struct CompleteData {
    fragmented_data: Option<Vec<u8>>,
}

impl CompleteData {
    fn new() -> Self {
        Self { fragmented_data: None }
    }

    fn process_data(&mut self, data: &[u8], fragmentation: Fragmentation) -> Option<Vec<u8>> {
        match fragmentation {
            Fragmentation::Single => {
                self.check_data_is_empty();

                Some(data.to_vec())
            }
            Fragmentation::First => {
                self.check_data_is_empty();

                self.fragmented_data = Some(data.to_vec());

                None
            }
            Fragmentation::Next => {
                self.append_data(data);

                None
            }
            Fragmentation::Last => {
                self.append_data(data);

                let complete_data = self.fragmented_data.take().unwrap();

                Some(complete_data)
            }
        }
    }

    fn check_data_is_empty(&mut self) {
        if self.fragmented_data.is_some() {
            warn!("Skipping pending Fast-Path Update internal multiple elements data");
            self.fragmented_data = None;
        }
    }

    fn append_data(&mut self, data: &[u8]) {
        if let Some(fragmented_data) = self.fragmented_data.as_mut() {
            fragmented_data.extend_from_slice(data);
        } else {
            warn!("Got unexpected Next fragmentation PDU without prior First fragmentation PDU");
        }
    }
}

struct FrameMarkerProcessor {
    initiator_id: u16,
    global_channel_id: u16,
}

impl FrameMarkerProcessor {
    fn new(initiator_id: u16, global_channel_id: u16) -> Self {
        Self {
            initiator_id,
            global_channel_id,
        }
    }

    fn process(&mut self, marker: &FrameMarkerPdu, output: impl io::Write) -> Result<(), RdpError> {
        match marker.frame_action {
            FrameAction::Begin => Ok(()),
            FrameAction::End => ShareDataFrame {
                channel_ids: ChannelIdentificators {
                    initiator_id: self.initiator_id,
                    channel_id: self.global_channel_id,
                },
                share_id: 0,
                pdu_source: self.initiator_id,
                pdu: ShareDataPdu::FrameAcknowledge(FrameAcknowledgePdu {
                    frame_id: marker.frame_id.unwrap_or(0),
                }),
            }
            .encode(output),
        }
    }
}
