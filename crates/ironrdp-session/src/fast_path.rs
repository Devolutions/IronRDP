use std::rc::Rc;

use ironrdp_graphics::pointer::{DecodedPointer, PointerBitmapTarget};
use ironrdp_graphics::rdp6::BitmapStreamDecoder;
use ironrdp_graphics::rle::RlePixelFormat;
use ironrdp_pdu::codecs::rfx::FrameAcknowledgePdu;
use ironrdp_pdu::cursor::ReadCursor;
use ironrdp_pdu::fast_path::{FastPathHeader, FastPathUpdate, FastPathUpdatePdu, Fragmentation};
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp_pdu::pointer::PointerUpdateData;
use ironrdp_pdu::rdp::headers::ShareDataPdu;
use ironrdp_pdu::surface_commands::{FrameAction, FrameMarkerPdu, SurfaceCommand};
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::{decode_cursor, PduErrorKind};

use crate::image::DecodedImage;
use crate::pointer::PointerCache;
use crate::utils::CodecId;
use crate::{rfx, SessionError, SessionErrorExt, SessionResult};

#[derive(Debug)]
pub enum UpdateKind {
    None,
    Region(InclusiveRectangle),
    PointerDefault,
    PointerHidden,
    PointerPosition { x: u16, y: u16 },
    NativePointerUpdate(Rc<DecodedPointer>),
}

pub struct Processor {
    complete_data: CompleteData,
    rfx_handler: rfx::DecodingContext,
    marker_processor: FrameMarkerProcessor,
    bitmap_stream_decoder: BitmapStreamDecoder,
    pointer_cache: PointerCache,
    use_system_pointer: bool,
    mouse_pos_update: Option<(u16, u16)>,
    no_server_pointer: bool,
    pointer_software_rendering: bool,
}

impl Processor {
    pub fn update_mouse_pos(&mut self, x: u16, y: u16) {
        self.mouse_pos_update = Some((x, y));
    }

    /// Process input fast path frame and return list of updates.
    pub fn process(
        &mut self,
        image: &mut DecodedImage,
        input: &[u8],
        output: &mut WriteBuf,
    ) -> SessionResult<Vec<UpdateKind>> {
        let mut processor_updates = Vec::new();

        if let Some((x, y)) = self.mouse_pos_update.take() {
            if let Some(rect) = image.move_pointer(x, y)? {
                processor_updates.push(UpdateKind::Region(rect));
            }
        }

        let mut input = ReadCursor::new(input);

        let header = decode_cursor::<FastPathHeader>(&mut input).map_err(SessionError::pdu)?;
        debug!(fast_path_header = ?header, "Received Fast-Path packet");

        let update_pdu = decode_cursor::<FastPathUpdatePdu<'_>>(&mut input).map_err(SessionError::pdu)?;
        trace!(fast_path_update_fragmentation = ?update_pdu.fragmentation);

        let processed_complete_data = self
            .complete_data
            .process_data(update_pdu.data, update_pdu.fragmentation);

        let update_code = update_pdu.update_code;

        let Some(data) = processed_complete_data else {
            return Ok(Vec::new());
        };

        let update = FastPathUpdate::decode_with_code(data.as_slice(), update_code);

        match update {
            Ok(FastPathUpdate::SurfaceCommands(surface_commands)) => {
                trace!("Received Surface Commands: {} pieces", surface_commands.len());
                let update_region = self.process_surface_commands(image, output, surface_commands)?;
                processor_updates.push(UpdateKind::Region(update_region));
            }
            Ok(FastPathUpdate::Bitmap(bitmap_update)) => {
                trace!("Received bitmap update");

                let mut buf = Vec::new();
                let mut update_kind = UpdateKind::None;

                for update in bitmap_update.rectangles {
                    trace!("{update:?}");
                    buf.clear();

                    // Bitmap data is either compressed or uncompressed, depending
                    // on whether the BITMAP_COMPRESSION flag is present in the
                    // flags field.
                    let update_rectangle = if update
                        .compression_flags
                        .contains(ironrdp_pdu::bitmap::Compression::BITMAP_COMPRESSION)
                    {
                        if update.bits_per_pixel == 32 {
                            // Compressed bitmaps at a color depth of 32 bpp are compressed using RDP 6.0
                            // Bitmap Compression and stored inside an RDP 6.0 Bitmap Compressed Stream
                            // structure ([MS-RDPEGDI] section 2.2.2.5.1).
                            debug!("32 bpp compressed RDP6_BITMAP_STREAM");

                            match self.bitmap_stream_decoder.decode_bitmap_stream_to_rgb24(
                                update.bitmap_data,
                                &mut buf,
                                usize::from(update.width),
                                usize::from(update.height),
                            ) {
                                Ok(()) => image.apply_rgb24_bitmap(&buf, &update.rectangle)?,
                                Err(err) => {
                                    warn!("Invalid RDP6_BITMAP_STREAM: {err}");
                                    update.rectangle.clone()
                                }
                            }
                        } else {
                            // Compressed bitmaps not in 32 bpp format are compressed using Interleaved
                            // RLE and encapsulated in an RLE Compressed Bitmap Stream structure (section
                            // 2.2.9.1.1.3.1.2.4).
                            debug!(bpp = update.bits_per_pixel, "Non-32 bpp compressed RLE_BITMAP_STREAM",);

                            match ironrdp_graphics::rle::decompress(
                                update.bitmap_data,
                                &mut buf,
                                usize::from(update.width),
                                usize::from(update.height),
                                usize::from(update.bits_per_pixel),
                            ) {
                                Ok(RlePixelFormat::Rgb16) => image.apply_rgb16_bitmap(&buf, &update.rectangle)?,

                                // TODO: support other pixel formats…
                                Ok(format @ (RlePixelFormat::Rgb8 | RlePixelFormat::Rgb15 | RlePixelFormat::Rgb24)) => {
                                    warn!("Received RLE-compressed bitmap with unsupported color depth: {format:?}");
                                    update.rectangle.clone()
                                }

                                Err(e) => {
                                    warn!("Invalid RLE-compressed bitmap: {e}");
                                    update.rectangle.clone()
                                }
                            }
                        }
                    } else {
                        // Uncompressed bitmap data is formatted as a bottom-up, left-to-right series of
                        // pixels. Each pixel is a whole number of bytes. Each row contains a multiple of
                        // four bytes (including up to three bytes of padding, as necessary).
                        trace!("Uncompressed raw bitmap");

                        match update.bits_per_pixel {
                            16 => image.apply_rgb16_bitmap(update.bitmap_data, &update.rectangle)?,
                            // TODO: support other pixel formats…
                            unsupported => {
                                warn!("Invalid raw bitmap with {unsupported} bytes per pixels");
                                update.rectangle.clone()
                            }
                        }
                    };

                    match update_kind {
                        UpdateKind::Region(current) => {
                            update_kind = UpdateKind::Region(current.union(&update_rectangle))
                        }
                        _ => update_kind = UpdateKind::Region(update_rectangle),
                    }
                }

                processor_updates.push(update_kind);
            }
            Ok(FastPathUpdate::Pointer(update)) => {
                if self.no_server_pointer {
                    return Ok(processor_updates);
                }

                let bitmap_target = if self.pointer_software_rendering {
                    PointerBitmapTarget::Software
                } else {
                    PointerBitmapTarget::Accelerated
                };

                match update {
                    PointerUpdateData::SetHidden => {
                        processor_updates.push(UpdateKind::PointerHidden);
                        if self.pointer_software_rendering && !self.use_system_pointer {
                            self.use_system_pointer = true;
                            if let Some(rect) = image.hide_pointer()? {
                                processor_updates.push(UpdateKind::Region(rect));
                            }
                        }
                    }
                    PointerUpdateData::SetDefault => {
                        processor_updates.push(UpdateKind::PointerDefault);
                        if self.pointer_software_rendering && !self.use_system_pointer {
                            self.use_system_pointer = true;
                            if let Some(rect) = image.hide_pointer()? {
                                processor_updates.push(UpdateKind::Region(rect));
                            }
                        }
                    }
                    PointerUpdateData::SetPosition(position) => {
                        if self.use_system_pointer || !self.pointer_software_rendering {
                            processor_updates.push(UpdateKind::PointerPosition {
                                x: position.x,
                                y: position.y,
                            });
                        } else if let Some(rect) = image.move_pointer(position.x, position.y)? {
                            processor_updates.push(UpdateKind::Region(rect));
                        }
                    }
                    PointerUpdateData::Color(pointer) => {
                        let cache_index = pointer.cache_index;

                        let decoded_pointer = Rc::new(
                            DecodedPointer::decode_color_pointer_attribute(&pointer, bitmap_target)
                                .expect("Failed to decode color pointer attribute"),
                        );

                        let _ = self
                            .pointer_cache
                            .insert(usize::from(cache_index), Rc::clone(&decoded_pointer));

                        if !self.pointer_software_rendering {
                            processor_updates.push(UpdateKind::NativePointerUpdate(Rc::clone(&decoded_pointer)));
                        } else if let Some(rect) = image.update_pointer(decoded_pointer)? {
                            processor_updates.push(UpdateKind::Region(rect));
                        }
                    }
                    PointerUpdateData::Cached(cached) => {
                        let cache_index = cached.cache_index;

                        if let Some(cached_pointer) = self.pointer_cache.get(usize::from(cache_index)) {
                            // Disable system pointer
                            processor_updates.push(UpdateKind::PointerHidden);
                            self.use_system_pointer = false;
                            // Send graphics update
                            if !self.pointer_software_rendering {
                                processor_updates.push(UpdateKind::NativePointerUpdate(Rc::clone(&cached_pointer)));
                            } else if let Some(rect) = image.update_pointer(cached_pointer)? {
                                processor_updates.push(UpdateKind::Region(rect));
                            } else {
                                // In case pointer was hidden previously
                                if let Some(rect) = image.show_pointer()? {
                                    processor_updates.push(UpdateKind::Region(rect));
                                }
                            }
                        } else {
                            warn!("Cached pointer not found {}", cache_index);
                        }
                    }
                    PointerUpdateData::New(pointer) => {
                        let cache_index = pointer.color_pointer.cache_index;

                        let decoded_pointer = Rc::new(
                            DecodedPointer::decode_pointer_attribute(&pointer, bitmap_target)
                                .expect("Failed to decode pointer attribute"),
                        );

                        let _ = self
                            .pointer_cache
                            .insert(usize::from(cache_index), Rc::clone(&decoded_pointer));

                        if !self.pointer_software_rendering {
                            processor_updates.push(UpdateKind::NativePointerUpdate(Rc::clone(&decoded_pointer)));
                        } else if let Some(rect) = image.update_pointer(decoded_pointer)? {
                            processor_updates.push(UpdateKind::Region(rect));
                        }
                    }
                    PointerUpdateData::Large(pointer) => {
                        let cache_index = pointer.cache_index;

                        let decoded_pointer: Rc<DecodedPointer> = Rc::new(
                            DecodedPointer::decode_large_pointer_attribute(&pointer, bitmap_target)
                                .expect("Failed to decode large pointer attribute"),
                        );

                        let _ = self
                            .pointer_cache
                            .insert(usize::from(cache_index), Rc::clone(&decoded_pointer));

                        if !self.pointer_software_rendering {
                            processor_updates.push(UpdateKind::NativePointerUpdate(Rc::clone(&decoded_pointer)));
                        } else if let Some(rect) = image.update_pointer(decoded_pointer)? {
                            processor_updates.push(UpdateKind::Region(rect));
                        }
                    }
                };
            }
            Err(e) => {
                if let PduErrorKind::InvalidMessage { field, reason } = e.kind {
                    warn!(field, reason, "Received invalid Fast-Path update");
                    processor_updates.push(UpdateKind::None);
                } else {
                    return Err(custom_err!("Fast-Path", e));
                }
            }
        };

        Ok(processor_updates)
    }

    fn process_surface_commands(
        &mut self,
        image: &mut DecodedImage,
        output: &mut WriteBuf,
        surface_commands: Vec<SurfaceCommand<'_>>,
    ) -> SessionResult<InclusiveRectangle> {
        let mut update_rectangle = InclusiveRectangle::empty();

        for command in surface_commands {
            match command {
                SurfaceCommand::SetSurfaceBits(bits) | SurfaceCommand::StreamSurfaceBits(bits) => {
                    trace!("Surface bits");

                    let codec_id = CodecId::from_u8(bits.extended_bitmap_data.codec_id).ok_or_else(|| {
                        reason_err!(
                            "Fast-Path",
                            "unexpected codec ID: {:x}",
                            bits.extended_bitmap_data.codec_id
                        )
                    })?;

                    match codec_id {
                        CodecId::RemoteFx => {
                            let destination = bits.destination;
                            let mut data = bits.extended_bitmap_data.data;

                            while !data.is_empty() {
                                // TODO(@pacmancoder): Correct rectangle conversion logic should
                                // be revisited when `rectangle_processing.rs` from
                                // `ironrdp-graphics` will be refactored to use generic `Rectangle`
                                // trait instead of hardcoded `InclusiveRectangle`.
                                let destination = InclusiveRectangle {
                                    left: destination.left,
                                    top: destination.top,
                                    right: destination.right,
                                    bottom: destination.bottom,
                                };
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
                    self.marker_processor.process(&marker, output)?;
                }
            }
        }

        Ok(update_rectangle)
    }
}

pub struct ProcessorBuilder {
    pub io_channel_id: u16,
    pub user_channel_id: u16,
    pub no_server_pointer: bool,
    pub pointer_software_rendering: bool,
}

impl ProcessorBuilder {
    pub fn build(self) -> Processor {
        Processor {
            complete_data: CompleteData::new(),
            rfx_handler: rfx::DecodingContext::new(),
            marker_processor: FrameMarkerProcessor::new(self.user_channel_id, self.io_channel_id),
            bitmap_stream_decoder: BitmapStreamDecoder::default(),
            pointer_cache: PointerCache::default(),
            use_system_pointer: true,
            mouse_pos_update: None,
            no_server_pointer: self.no_server_pointer,
            pointer_software_rendering: self.pointer_software_rendering,
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
    user_channel_id: u16,
    io_channel_id: u16,
}

impl FrameMarkerProcessor {
    fn new(user_channel_id: u16, io_channel_id: u16) -> Self {
        Self {
            user_channel_id,
            io_channel_id,
        }
    }

    fn process(&mut self, marker: &FrameMarkerPdu, output: &mut WriteBuf) -> SessionResult<()> {
        match marker.frame_action {
            FrameAction::Begin => Ok(()),
            FrameAction::End => {
                ironrdp_connector::legacy::encode_share_data(
                    self.user_channel_id,
                    self.io_channel_id,
                    0,
                    ShareDataPdu::FrameAcknowledge(FrameAcknowledgePdu {
                        frame_id: marker.frame_id.unwrap_or(0),
                    }),
                    output,
                )
                .map_err(crate::legacy::map_error)?;

                Ok(())
            }
        }
    }
}
