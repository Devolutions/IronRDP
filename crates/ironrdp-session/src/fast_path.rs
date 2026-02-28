use std::sync::Arc;

use ironrdp_bulk::BulkCompressor;
use ironrdp_core::{decode_cursor, DecodeErrorKind, ReadCursor, WriteBuf};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::pointer::{DecodedPointer, PointerBitmapTarget};
use ironrdp_graphics::rdp6::BitmapStreamDecoder;
use ironrdp_graphics::rle::RlePixelFormat;
use ironrdp_pdu::codecs::rfx::FrameAcknowledgePdu;
use ironrdp_pdu::fast_path::{FastPathHeader, FastPathUpdate, FastPathUpdatePdu, Fragmentation};
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp_pdu::pointer::PointerUpdateData;
use ironrdp_pdu::rdp::capability_sets::{CodecId, CODEC_ID_NONE, CODEC_ID_REMOTEFX};
use ironrdp_pdu::rdp::headers::{CompressionFlags, ShareDataPdu};
use ironrdp_pdu::surface_commands::{FrameAction, FrameMarkerPdu, SurfaceCommand};
use tracing::{debug, trace, warn};

use crate::image::DecodedImage;
use crate::pointer::PointerCache;
use crate::{custom_err, reason_err, rfx, SessionError, SessionErrorExt as _, SessionResult};

/// Default Windows system palette (VGA colors).
/// First 10 and last 10 entries are fixed; middle 236 are black.
/// Per MS-RDPBCGR 2.2.9.1.1.3.1.1.
const DEFAULT_SYSTEM_PALETTE: [[u8; 3]; 256] = {
    let mut pal = [[0u8; 3]; 256];
    // Entries 0-9
    pal[0] = [0, 0, 0]; // Black
    pal[1] = [128, 0, 0]; // Dark Red
    pal[2] = [0, 128, 0]; // Dark Green
    pal[3] = [128, 128, 0]; // Dark Yellow
    pal[4] = [0, 0, 128]; // Dark Blue
    pal[5] = [128, 0, 128]; // Dark Magenta
    pal[6] = [0, 128, 128]; // Dark Cyan
    pal[7] = [192, 192, 192]; // Light Gray
    pal[8] = [128, 128, 128]; // Dark Gray
    pal[9] = [255, 0, 0]; // Red
                          // Entries 10-245: black (already zero-initialized)
                          // Entries 246-255
    pal[246] = [0, 255, 0]; // Green
    pal[247] = [255, 255, 0]; // Yellow
    pal[248] = [0, 0, 255]; // Blue
    pal[249] = [255, 0, 255]; // Magenta
    pal[250] = [0, 255, 255]; // Cyan
    pal[251] = [255, 255, 255]; // White
                                // 252-255 remain black
    pal
};

#[derive(Debug)]
pub enum UpdateKind {
    None,
    Region(InclusiveRectangle),
    PointerDefault,
    PointerHidden,
    PointerPosition { x: u16, y: u16 },
    PointerBitmap(Arc<DecodedPointer>),
}

pub struct Processor {
    complete_data: CompleteData,
    rfx_handler: rfx::DecodingContext,
    marker_processor: FrameMarkerProcessor,
    bitmap_stream_decoder: BitmapStreamDecoder,
    pointer_cache: PointerCache,
    use_system_pointer: bool,
    mouse_pos_update: Option<(u16, u16)>,
    enable_server_pointer: bool,
    pointer_software_rendering: bool,
    /// Bulk decompressor for server-to-client compressed PDUs.
    /// `None` when compression was not negotiated.
    bulk_decompressor: Option<BulkCompressor>,
    /// Current 8bpp color palette (RGB). Updated by Palette fast-path updates.
    palette: [[u8; 3]; 256],
    #[cfg(feature = "qoiz")]
    zdctx: zstd_safe::DCtx<'static>,
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

        let header = decode_cursor::<FastPathHeader>(&mut input).map_err(SessionError::decode)?;
        trace!(fast_path_header = ?header, "Received Fast-Path packet");

        // A single FastPath output PDU can contain multiple updates.
        // Loop over all updates within the PDU payload.
        while !input.is_empty() {
            let update_result = self.process_single_update(&mut input, image, output)?;
            processor_updates.extend(update_result);
        }

        Ok(processor_updates)
    }

    /// Process a single FastPath update from the cursor, advancing past it.
    fn process_single_update(
        &mut self,
        input: &mut ReadCursor<'_>,
        image: &mut DecodedImage,
        output: &mut WriteBuf,
    ) -> SessionResult<Vec<UpdateKind>> {
        let mut processor_updates = Vec::new();

        let update_pdu = decode_cursor::<FastPathUpdatePdu<'_>>(input).map_err(SessionError::decode)?;
        trace!(fast_path_update_fragmentation = ?update_pdu.fragmentation);

        // Decompress the payload if the server sent it compressed.
        let decompressed_data;
        let payload = if let Some(flags) = update_pdu.compression_flags {
            if flags.contains(CompressionFlags::COMPRESSED) || flags.contains(CompressionFlags::FLUSHED) {
                let bulk_flags =
                    u32::from(flags.bits()) | u32::from(update_pdu.compression_type.map_or(0, |ct| ct.as_u8()));

                if let Some(ref mut decompressor) = self.bulk_decompressor {
                    let decompressed = decompressor
                        .decompress(update_pdu.data, bulk_flags)
                        .map_err(|e| reason_err!("FastPath", "bulk decompression failed: {}", e))?;
                    // Copy decompressed data before accessing metrics (releases the mutable borrow).
                    decompressed_data = decompressed.to_vec();
                    debug!(
                        compressed_size = update_pdu.data.len(),
                        decompressed_size = decompressed_data.len(),
                        compression_type = ?update_pdu.compression_type,
                        compression_ratio = format_args!("{:.2}x", decompressor.compression_ratio()),
                        total_compressed = decompressor.total_compressed_bytes(),
                        total_uncompressed = decompressor.total_uncompressed_bytes(),
                        "Decompressed FastPath update"
                    );
                    decompressed_data.as_slice()
                } else {
                    warn!("Received compressed FastPath data but no decompressor is configured");
                    update_pdu.data
                }
            } else {
                // Compression flags present but COMPRESSED bit not set — pass data through.
                // Still need to inform the decompressor of FLUSHED/AT_FRONT flags even
                // without compressed payload.
                update_pdu.data
            }
        } else {
            update_pdu.data
        };

        let processed_complete_data = self.complete_data.process_data(payload, update_pdu.fragmentation);

        let update_code = update_pdu.update_code;

        let Some(data) = processed_complete_data else {
            return Ok(processor_updates);
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
                                Ok(()) => image.apply_rgb24(&buf, &update.rectangle, true)?,
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
                                Ok(RlePixelFormat::Rgb15) => image.apply_rgb15_bitmap(&buf, &update.rectangle)?,
                                Ok(RlePixelFormat::Rgb24) => image.apply_bgr24_bitmap(&buf, &update.rectangle)?,
                                Ok(RlePixelFormat::Rgb8) => {
                                    image.apply_rgb8_with_palette(&buf, &update.rectangle, &self.palette)?
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
                        // [MS-RDPBCGR] 2.2.9.1.1.3.1.2.2
                        trace!("Uncompressed raw bitmap");

                        let bpp = usize::from(update.bits_per_pixel);
                        let width = usize::from(update.width);
                        let bytes_per_pixel = bpp.div_ceil(8);
                        let row_bytes = width * bytes_per_pixel;
                        let padded_row_bytes = (row_bytes + 3) & !3;

                        if padded_row_bytes != row_bytes {
                            // Strip per-row padding before passing to the bitmap apply functions,
                            // which expect tightly packed pixel data.
                            buf.clear();
                            for row in update.bitmap_data.chunks(padded_row_bytes) {
                                let end = row_bytes.min(row.len());
                                buf.extend_from_slice(&row[..end]);
                            }

                            match update.bits_per_pixel {
                                8 => image.apply_rgb8_with_palette(&buf, &update.rectangle, &self.palette)?,
                                15 => image.apply_rgb15_bitmap(&buf, &update.rectangle)?,
                                16 => image.apply_rgb16_bitmap(&buf, &update.rectangle)?,
                                24 => image.apply_bgr24_bitmap(&buf, &update.rectangle)?,
                                32 => image.apply_rgb32_bitmap(&buf, PixelFormat::BgrX32, &update.rectangle)?,
                                _ => {
                                    warn!("Unsupported uncompressed bitmap depth: {bpp} bpp");
                                    update.rectangle.clone()
                                }
                            }
                        } else {
                            match update.bits_per_pixel {
                                8 => image.apply_rgb8_with_palette(
                                    update.bitmap_data,
                                    &update.rectangle,
                                    &self.palette,
                                )?,
                                15 => image.apply_rgb15_bitmap(update.bitmap_data, &update.rectangle)?,
                                16 => image.apply_rgb16_bitmap(update.bitmap_data, &update.rectangle)?,
                                24 => image.apply_bgr24_bitmap(update.bitmap_data, &update.rectangle)?,
                                32 => image.apply_rgb32_bitmap(
                                    update.bitmap_data,
                                    PixelFormat::BgrX32,
                                    &update.rectangle,
                                )?,
                                _ => {
                                    warn!("Unsupported uncompressed bitmap depth: {bpp} bpp");
                                    update.rectangle.clone()
                                }
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
                if !self.enable_server_pointer {
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

                        let decoded_pointer = Arc::new(
                            DecodedPointer::decode_color_pointer_attribute(&pointer, bitmap_target)
                                .map_err(|e| SessionError::custom("failed to decode color pointer attribute", e))?,
                        );

                        let _ = self
                            .pointer_cache
                            .insert(usize::from(cache_index), Arc::clone(&decoded_pointer));

                        if !self.pointer_software_rendering {
                            processor_updates.push(UpdateKind::PointerBitmap(Arc::clone(&decoded_pointer)));
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
                                processor_updates.push(UpdateKind::PointerBitmap(Arc::clone(&cached_pointer)));
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

                        let decoded_pointer = Arc::new(
                            DecodedPointer::decode_pointer_attribute(&pointer, bitmap_target)
                                .map_err(|e| SessionError::custom("failed to decode pointer attribute", e))?,
                        );

                        let _ = self
                            .pointer_cache
                            .insert(usize::from(cache_index), Arc::clone(&decoded_pointer));

                        if !self.pointer_software_rendering {
                            processor_updates.push(UpdateKind::PointerBitmap(Arc::clone(&decoded_pointer)));
                        } else if let Some(rect) = image.update_pointer(decoded_pointer)? {
                            processor_updates.push(UpdateKind::Region(rect));
                        }
                    }
                    PointerUpdateData::Large(pointer) => {
                        let cache_index = pointer.cache_index;

                        let decoded_pointer: Arc<DecodedPointer> = Arc::new(
                            DecodedPointer::decode_large_pointer_attribute(&pointer, bitmap_target)
                                .map_err(|e| SessionError::custom("failed to decode large pointer attribute", e))?,
                        );

                        let _ = self
                            .pointer_cache
                            .insert(usize::from(cache_index), Arc::clone(&decoded_pointer));

                        if !self.pointer_software_rendering {
                            processor_updates.push(UpdateKind::PointerBitmap(Arc::clone(&decoded_pointer)));
                        } else if let Some(rect) = image.update_pointer(decoded_pointer)? {
                            processor_updates.push(UpdateKind::Region(rect));
                        }
                    }
                };
            }
            Ok(FastPathUpdate::Palette(palette_data)) => {
                trace!("Received palette update");
                self.process_palette_update(&palette_data);
            }
            Err(e) => {
                // FIXME: This seems to be a way of special-handling the error case in FastPathUpdate::decode_cursor_with_code
                // to ignore the unsupported update PDUs, but this is a fragile logic and the rationale behind it is not
                // obvious.
                if let DecodeErrorKind::InvalidField { field, reason } = e.kind() {
                    warn!(field, reason, "Received invalid Fast-Path update");
                    processor_updates.push(UpdateKind::None);
                } else {
                    return Err(custom_err!("Fast-Path", e));
                }
            }
        };

        Ok(processor_updates)
    }

    /// Parse TS_UPDATE_PALETTE_DATA and update the session palette.
    /// Format: pad(2) + numberColors(u32) + N x TS_COLOR_QUAD [B, G, R, pad].
    fn process_palette_update(&mut self, data: &[u8]) {
        // MS-RDPBCGR 2.2.9.1.1.3.1.1: 2 bytes pad + 4 bytes numberColors + entries
        if data.len() < 6 {
            warn!("Palette update too short: {} bytes", data.len());
            return;
        }

        let number_colors = usize::try_from(u32::from_le_bytes([data[2], data[3], data[4], data[5]])).unwrap_or(256);
        let entry_data = &data[6..];

        if entry_data.len() < number_colors * 4 {
            warn!(
                "Palette data truncated: expected {} bytes for {} colors, got {}",
                number_colors * 4,
                number_colors,
                entry_data.len()
            );
            return;
        }

        let count = number_colors.min(256);
        for i in 0..count {
            let offset = i * 4;
            // TS_COLOR_QUAD: Blue, Green, Red, Pad
            self.palette[i] = [entry_data[offset + 2], entry_data[offset + 1], entry_data[offset]];
        }

        debug!("Updated palette with {} colors", count);
    }

    fn process_surface_commands(
        &mut self,
        image: &mut DecodedImage,
        output: &mut WriteBuf,
        surface_commands: Vec<SurfaceCommand<'_>>,
    ) -> SessionResult<InclusiveRectangle> {
        let mut update_rectangle = None;

        for command in surface_commands {
            match command {
                SurfaceCommand::SetSurfaceBits(bits) | SurfaceCommand::StreamSurfaceBits(bits) => {
                    let codec_id = CodecId::from_u8(bits.extended_bitmap_data.codec_id).ok_or_else(|| {
                        reason_err!(
                            "Fast-Path",
                            "unexpected codec ID: {:x}",
                            bits.extended_bitmap_data.codec_id
                        )
                    })?;

                    trace!(?codec_id, "Surface bits");

                    let destination = bits.destination;
                    // TODO(@pacmancoder): Correct rectangle conversion logic should
                    // be revisited when `rectangle_processing.rs` from
                    // `ironrdp-graphics` will be refactored to use generic `Rectangle`
                    // trait instead of hardcoded `InclusiveRectangle`.
                    let destination = InclusiveRectangle {
                        left: destination.left,
                        top: destination.top,
                        right: destination.right - 1,
                        bottom: destination.bottom - 1,
                    };
                    match codec_id {
                        CODEC_ID_NONE => {
                            let ext_data = bits.extended_bitmap_data;
                            let rectangle = match ext_data.bpp {
                                16 => image.apply_rgb16_bitmap(ext_data.data, &destination)?,
                                24 => image.apply_bgr24_bitmap(ext_data.data, &destination)?,
                                32 => image.apply_rgb32_bitmap(ext_data.data, PixelFormat::BgrX32, &destination)?,
                                bpp => {
                                    warn!("Unsupported surface CODEC_ID_NONE bpp: {bpp}");
                                    continue;
                                }
                            };
                            update_rectangle = update_rectangle
                                .map(|rect: InclusiveRectangle| rect.union(&rectangle))
                                .or(Some(rectangle));
                        }
                        CODEC_ID_REMOTEFX => {
                            let mut data = ReadCursor::new(bits.extended_bitmap_data.data);
                            while !data.is_empty() {
                                let (_frame_id, rectangle) = self.rfx_handler.decode(image, &destination, &mut data)?;
                                update_rectangle = update_rectangle
                                    .map(|rect: InclusiveRectangle| rect.union(&rectangle))
                                    .or(Some(rectangle));
                            }
                        }
                        #[cfg(feature = "qoi")]
                        ironrdp_pdu::rdp::capability_sets::CODEC_ID_QOI => {
                            qoi_apply(
                                image,
                                destination,
                                bits.extended_bitmap_data.data,
                                &mut update_rectangle,
                            )?;
                        }
                        #[cfg(feature = "qoiz")]
                        ironrdp_pdu::rdp::capability_sets::CODEC_ID_QOIZ => {
                            let compressed = &bits.extended_bitmap_data.data;
                            let mut input = zstd_safe::InBuffer::around(compressed);
                            let mut data = vec![0; compressed.len() * 4];
                            let mut pos = 0;
                            loop {
                                let mut output = zstd_safe::OutBuffer::around_pos(data.as_mut_slice(), pos);
                                self.zdctx
                                    .decompress_stream(&mut output, &mut input)
                                    .map_err(zstd_safe::get_error_name)
                                    .map_err(|e| reason_err!("zstd", "{}", e))?;
                                pos = output.pos();
                                if pos == output.capacity() {
                                    data.resize(data.capacity() * 2, 0);
                                } else {
                                    break;
                                }
                            }

                            qoi_apply(image, destination, &data, &mut update_rectangle)?;
                        }
                        _ => {
                            warn!("Unsupported codec ID: {}", bits.extended_bitmap_data.codec_id);
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

        Ok(update_rectangle.unwrap_or_else(InclusiveRectangle::empty))
    }
}

#[cfg(feature = "qoi")]
fn qoi_apply(
    image: &mut DecodedImage,
    destination: InclusiveRectangle,
    data: &[u8],
    update_rectangle: &mut Option<InclusiveRectangle>,
) -> SessionResult<()> {
    let (header, decoded) = qoi::decode_to_vec(data).map_err(|e| reason_err!("QOI decode", "{}", e))?;
    match header.channels {
        qoi::Channels::Rgb => {
            let rectangle = image.apply_rgb24(&decoded, &destination, false)?;

            *update_rectangle = update_rectangle
                .as_ref()
                .map(|rect: &InclusiveRectangle| rect.union(&rectangle))
                .or(Some(rectangle));
        }
        qoi::Channels::Rgba => {
            warn!("Unsupported RGBA QOI data");
        }
    }
    Ok(())
}

pub struct ProcessorBuilder {
    pub io_channel_id: u16,
    pub user_channel_id: u16,
    /// Ignore server pointer updates.
    pub enable_server_pointer: bool,
    /// Use software rendering mode for pointer bitmap generation. When this option is active,
    /// `UpdateKind::PointerBitmap` will not be generated. Remote pointer will be drawn
    /// via software rendering on top of the output image.
    pub pointer_software_rendering: bool,
    /// Bulk decompressor for server-to-client compressed PDUs.
    /// `None` when compression was not negotiated.
    pub bulk_decompressor: Option<BulkCompressor>,
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
            enable_server_pointer: self.enable_server_pointer,
            pointer_software_rendering: self.pointer_software_rendering,
            bulk_decompressor: self.bulk_decompressor,
            palette: DEFAULT_SYSTEM_PALETTE,
            #[cfg(feature = "qoiz")]
            zdctx: zstd_safe::DCtx::default(),
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

                self.fragmented_data.take()
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
