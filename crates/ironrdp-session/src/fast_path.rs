use std::sync::Arc;

use ironrdp_bulk::{BulkCompressor, CompressionType};
use ironrdp_core::{DecodeErrorKind, ReadCursor, WriteBuf, decode_cursor};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::pointer::{DecodedPointer, PointerBitmapTarget, PointerError};
use ironrdp_graphics::rdp6::BitmapStreamDecoder;
use ironrdp_graphics::rle::RlePixelFormat;
use ironrdp_pdu::bitmap::BitmapUpdateData;
use ironrdp_pdu::codecs::rfx::FrameAcknowledgePdu;
use ironrdp_pdu::fast_path::{FastPathHeader, FastPathUpdate, FastPathUpdatePdu, Fragmentation};
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp_pdu::pointer::PointerUpdateData;
use ironrdp_pdu::rdp::capability_sets::{CODEC_ID_NONE, CODEC_ID_REMOTEFX, CodecId};
use ironrdp_pdu::rdp::headers::{CompressionFlags, ShareDataPdu};
use ironrdp_pdu::surface_commands::{FrameAction, FrameMarkerPdu, SurfaceCommand};
use tracing::{debug, trace, warn};

use crate::image::DecodedImage;
use crate::palette::Palette;
use crate::pointer::PointerCache;
use crate::{SessionError, SessionErrorExt as _, SessionResult, custom_err, reason_err, rfx};

#[cfg(test)]
mod tests {
    use ironrdp_pdu::bitmap::{BitmapData, Compression};
    use ironrdp_pdu::pointer::{ColorPointerAttribute, Point16, PointerAttribute};

    use super::*;

    #[test]
    fn raw_bitmap_removes_byte_padding_without_changing_source_stride() {
        let mut processor = ProcessorBuilder {
            io_channel_id: 0,
            user_channel_id: 0,
            share_id: 0,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();
        let mut image = DecodedImage::new(PixelFormat::RgbA32, 4, 3);

        // Two bottom-up BGR rows of three source pixels each. The final source
        // column is outside the destination and each row has three pad bytes.
        let bitmap_data = [
            3, 2, 1, 6, 5, 4, 9, 8, 7, 0xff, 0xff, 0xff, // bottom row
            15, 14, 13, 18, 17, 16, 21, 20, 19, 0xff, 0xff, 0xff, // top row
        ];
        let bitmap_update = BitmapUpdateData {
            rectangles: vec![BitmapData {
                rectangle: InclusiveRectangle {
                    left: 1,
                    top: 0,
                    right: 2,
                    bottom: 1,
                },
                width: 3,
                height: 2,
                bits_per_pixel: 24,
                compression_flags: Compression::empty(),
                compressed_data_header: None,
                bitmap_data: &bitmap_data,
            }],
        };

        processor.process_bitmap_update(&mut image, bitmap_update).unwrap();

        let pixel = |x: usize, y: usize| -> [u8; 4] {
            let offset = (y * usize::from(image.width()) + x) * 4;
            image.data()[offset..offset + 4]
                .try_into()
                .expect("pixel has four channels")
        };
        assert_eq!(pixel(1, 0), [13, 14, 15, 255]);
        assert_eq!(pixel(2, 0), [16, 17, 18, 255]);
        assert_eq!(pixel(1, 1), [1, 2, 3, 255]);
        assert_eq!(pixel(2, 1), [4, 5, 6, 255]);
        assert_eq!(pixel(3, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn unsupported_new_pointer_uses_default_and_evicts_cached_shape() {
        let mut processor = ProcessorBuilder {
            io_channel_id: 0,
            user_channel_id: 0,
            share_id: 0,
            enable_server_pointer: true,
            pointer_software_rendering: false,
        }
        .build();
        processor
            .pointer_cache
            .insert(0, Arc::new(DecodedPointer::new_invisible()));
        let mut image = DecodedImage::new(PixelFormat::RgbA32, 4, 3);
        let pointer = PointerAttribute {
            xor_bpp: 2,
            color_pointer: ColorPointerAttribute {
                cache_index: 0,
                hot_spot: Point16 { x: 0, y: 0 },
                width: 1,
                height: 1,
                xor_mask: &[],
                and_mask: &[],
            },
        };

        let updates = processor
            .process_pointer_update(&mut image, PointerUpdateData::New(pointer))
            .expect("unsupported pointer must not fail the session");

        assert!(matches!(updates.as_slice(), [UpdateKind::PointerDefault]));
        assert!(!processor.pointer_cache.is_cached(0));
    }

    /// The decompressor is owned by the library, but a session that never receives a
    /// compressed update should not pay for the algorithm contexts. `ironrdp-web` never
    /// negotiates compression, so this is its normal case.
    #[test]
    fn decompressor_is_not_built_until_an_update_needs_it() {
        use ironrdp_core::encode_vec;
        use ironrdp_pdu::fast_path::UpdateCode;

        let mut processor = ProcessorBuilder {
            io_channel_id: 0,
            user_channel_id: 0,
            share_id: 0,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();

        assert!(
            processor.bulk_decompressor.is_none(),
            "a freshly built processor must not allocate the bulk contexts"
        );

        let frame = encode_vec(&FastPathUpdatePdu {
            fragmentation: Fragmentation::Single,
            update_code: UpdateCode::Bitmap,
            compression_flags: None,
            compression_type: None,
            data: &[],
        })
        .expect("encode uncompressed FastPath update");

        let mut image = DecodedImage::new(PixelFormat::RgbA32, 4, 4);
        let mut output = WriteBuf::new();
        // The empty bitmap payload does not decode; only the allocation behaviour on the
        // way there is under test, so the update result is deliberately ignored.
        let _result = processor.process_single_update(&mut ReadCursor::new(&frame), &mut image, &mut output);

        assert!(
            processor.bulk_decompressor.is_none(),
            "an update carrying no compression flags must not build a decompressor"
        );
    }
}

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
    /// Bulk decompressor for server-to-client compressed PDUs. Owned by the library
    /// and built on the first update that needs one, so a compressed update is always
    /// decodable without a session that never receives one paying for the algorithm
    /// contexts (the two XCRUSH history buffers are 2 MB each). This is not a
    /// consumer-visible option: `ProcessorBuilder` has no corresponding field, so
    /// there is no way to construct a `Processor` that cannot decompress.
    bulk_decompressor: Option<BulkCompressor>,
    /// Current 8bpp color palette. Updated by Palette fast-path updates.
    palette: Palette,
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

                // Built on first use. The construction-time type does not constrain what
                // can be decoded: `decompress` selects the algorithm per update from the
                // packet's own type bits, so Rdp61 here is an arbitrary starting point.
                let decompressor = self
                    .bulk_decompressor
                    .get_or_insert_with(|| BulkCompressor::new(CompressionType::Rdp61));
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
                let updates = self.process_bitmap_update(image, bitmap_update)?;
                processor_updates.extend(updates);
            }
            Ok(FastPathUpdate::Pointer(update)) => {
                let updates = self.process_pointer_update(image, update)?;
                processor_updates.extend(updates);
            }
            Ok(FastPathUpdate::Palette(palette_data)) => {
                trace!("Received palette update");
                self.process_palette_update(palette_data);
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

    /// Process a palette update shared between fast-path and slow-path pipelines.
    pub(crate) fn process_palette_update(&mut self, palette_data: &[u8]) {
        self.palette.process_update(palette_data);
    }

    /// Process a bitmap update, shared between fast-path and slow-path pipelines.
    pub fn process_bitmap_update(
        &mut self,
        image: &mut DecodedImage,
        bitmap_update: BitmapUpdateData<'_>,
    ) -> SessionResult<Vec<UpdateKind>> {
        let mut buf = Vec::new();
        let mut update_kind = UpdateKind::None;

        for update in bitmap_update.rectangles {
            trace!("{update:?}");
            buf.clear();

            let width = update.width.min(update.rectangle.width());
            let height = update.height.min(update.rectangle.height());
            if width == 0 || height == 0 {
                warn!(
                    bitmap_width = update.width,
                    bitmap_height = update.height,
                    destination = ?update.rectangle,
                    "Skipping bitmap with an empty source or destination"
                );
                continue;
            }

            // `width` and `height` describe the source bitmap layout, while
            // destination bounds describe where its visible portion is drawn.
            // Servers can send source rows wider than the destination; preserve
            // that source stride and discard only the excess source extent.
            let bitmap_rectangle = InclusiveRectangle {
                left: update.rectangle.left,
                top: update.rectangle.top,
                right: update.rectangle.left + width - 1,
                bottom: update.rectangle.top + height - 1,
            };

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
                        // The RDP 6 decoder writes its planes in row-major (top-down) order.
                        Ok(()) => image.apply_rgb24(&buf, &bitmap_rectangle, update.width, false)?,
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
                        Ok(RlePixelFormat::Rgb16) => image.apply_rgb16_bitmap(&buf, &bitmap_rectangle, update.width)?,
                        Ok(RlePixelFormat::Rgb15) => image.apply_rgb15_bitmap(&buf, &bitmap_rectangle, update.width)?,
                        Ok(RlePixelFormat::Rgb24) => image.apply_bgr24_bitmap(&buf, &bitmap_rectangle, update.width)?,
                        Ok(RlePixelFormat::Rgb8) => image.apply_rgb8_with_palette(
                            &buf,
                            &bitmap_rectangle,
                            self.palette.colors(),
                            update.width,
                        )?,

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
                    // Strip only byte padding; the tightly packed rows still retain
                    // `update.width` pixels and therefore their source stride.
                    buf.clear();
                    for row in update
                        .bitmap_data
                        .chunks_exact(padded_row_bytes)
                        .take(usize::from(update.height))
                    {
                        buf.extend_from_slice(&row[..row_bytes]);
                    }

                    match update.bits_per_pixel {
                        8 => image.apply_rgb8_with_palette(
                            &buf,
                            &bitmap_rectangle,
                            self.palette.colors(),
                            update.width,
                        )?,
                        15 => image.apply_rgb15_bitmap(&buf, &bitmap_rectangle, update.width)?,
                        16 => image.apply_rgb16_bitmap(&buf, &bitmap_rectangle, update.width)?,
                        24 => image.apply_bgr24_bitmap(&buf, &bitmap_rectangle, update.width)?,
                        32 => image.apply_rgb32_bitmap(&buf, PixelFormat::BgrX32, &bitmap_rectangle, update.width)?,
                        _ => {
                            warn!("Unsupported uncompressed bitmap depth: {bpp} bpp");
                            update.rectangle.clone()
                        }
                    }
                } else {
                    match update.bits_per_pixel {
                        8 => image.apply_rgb8_with_palette(
                            update.bitmap_data,
                            &bitmap_rectangle,
                            self.palette.colors(),
                            update.width,
                        )?,
                        15 => image.apply_rgb15_bitmap(update.bitmap_data, &bitmap_rectangle, update.width)?,
                        16 => image.apply_rgb16_bitmap(update.bitmap_data, &bitmap_rectangle, update.width)?,
                        24 => image.apply_bgr24_bitmap(update.bitmap_data, &bitmap_rectangle, update.width)?,
                        32 => image.apply_rgb32_bitmap(
                            update.bitmap_data,
                            PixelFormat::BgrX32,
                            &bitmap_rectangle,
                            update.width,
                        )?,
                        _ => {
                            warn!("Unsupported uncompressed bitmap depth: {bpp} bpp");
                            update.rectangle.clone()
                        }
                    }
                }
            };

            match update_kind {
                UpdateKind::Region(current) => update_kind = UpdateKind::Region(current.union(&update_rectangle)),
                _ => update_kind = UpdateKind::Region(update_rectangle),
            }
        }

        Ok(vec![update_kind])
    }

    /// Process a pointer update, shared between fast-path and slow-path pipelines.
    pub fn process_pointer_update(
        &mut self,
        image: &mut DecodedImage,
        update: PointerUpdateData<'_>,
    ) -> SessionResult<Vec<UpdateKind>> {
        let mut processor_updates = Vec::new();

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

                let decoded_pointer = match DecodedPointer::decode_pointer_attribute_with_palette(
                    &pointer,
                    bitmap_target,
                    Some(self.palette.colors()),
                ) {
                    Ok(pointer) => Arc::new(pointer),
                    Err(error) => {
                        self.fallback_after_pointer_decode_failure(image, &mut processor_updates, cache_index, error)?;
                        return Ok(processor_updates);
                    }
                };

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

                let decoded_pointer = match DecodedPointer::decode_large_pointer_attribute_with_palette(
                    &pointer,
                    bitmap_target,
                    Some(self.palette.colors()),
                ) {
                    Ok(pointer) => Arc::new(pointer),
                    Err(error) => {
                        self.fallback_after_pointer_decode_failure(image, &mut processor_updates, cache_index, error)?;
                        return Ok(processor_updates);
                    }
                };

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

        Ok(processor_updates)
    }

    fn fallback_after_pointer_decode_failure(
        &mut self,
        image: &mut DecodedImage,
        processor_updates: &mut Vec<UpdateKind>,
        cache_index: u16,
        error: PointerError,
    ) -> SessionResult<()> {
        let error_kind = match error {
            PointerError::InvalidXorMaskSize { .. } => "InvalidXorMaskSize",
            PointerError::InvalidAndMaskSize { .. } => "InvalidAndMaskSize",
            PointerError::NotSupportedBpp { .. } => "NotSupportedBpp",
            PointerError::Pdu(_) => "Pdu",
        };
        warn!(pointer_error = error_kind, "Ignoring unsupported pointer update");
        let _ = self.pointer_cache.remove(usize::from(cache_index));

        if self.pointer_software_rendering && !self.use_system_pointer {
            self.use_system_pointer = true;
            if let Some(rect) = image.hide_pointer()? {
                processor_updates.push(UpdateKind::Region(rect));
            }
        }

        processor_updates.push(UpdateKind::PointerDefault);
        Ok(())
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
                            let source_width = destination.width();
                            let rectangle = match ext_data.bpp {
                                8 => image.apply_rgb8_with_palette(
                                    ext_data.data,
                                    &destination,
                                    self.palette.colors(),
                                    source_width,
                                )?,
                                15 => image.apply_rgb15_bitmap(ext_data.data, &destination, source_width)?,
                                16 => image.apply_rgb16_bitmap(ext_data.data, &destination, source_width)?,
                                24 => image.apply_bgr24_bitmap(ext_data.data, &destination, source_width)?,
                                32 => image.apply_rgb32_bitmap(
                                    ext_data.data,
                                    PixelFormat::BgrX32,
                                    &destination,
                                    source_width,
                                )?,
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

    // Guard against a decoded buffer that doesn't match the destination
    // rectangle. `apply_rgb24`/`apply_rgba32` derive the row count from the
    // decoded length, and the only bounds check downstream (`rect_fits`)
    // validates the rectangle against the image, not the buffer against the
    // rectangle. A malformed/oversized QOI payload would otherwise drive the
    // per-row index past `self.data` and panic (client-side DoS).
    let channels = match header.channels {
        qoi::Channels::Rgb => 3,
        qoi::Channels::Rgba => 4,
    };
    let expected = usize::from(destination.width()) * usize::from(destination.height()) * channels;
    if decoded.len() != expected {
        return Err(reason_err!(
            "QOI decode",
            "decoded {} bytes, expected {} for {}x{} ({} channels)",
            decoded.len(),
            expected,
            destination.width(),
            destination.height(),
            channels
        ));
    }

    let rectangle = match header.channels {
        qoi::Channels::Rgb => image.apply_rgb24(&decoded, &destination, destination.width(), false)?,
        qoi::Channels::Rgba => image.apply_rgba32(&decoded, &destination, false)?,
    };

    *update_rectangle = update_rectangle
        .as_ref()
        .map(|rect: &InclusiveRectangle| rect.union(&rectangle))
        .or(Some(rectangle));
    Ok(())
}

pub struct ProcessorBuilder {
    pub io_channel_id: u16,
    pub user_channel_id: u16,
    pub share_id: u32,
    /// Ignore server pointer updates.
    pub enable_server_pointer: bool,
    /// Use software rendering mode for pointer bitmap generation. When this option is active,
    /// `UpdateKind::PointerBitmap` will not be generated. Remote pointer will be drawn
    /// via software rendering on top of the output image.
    pub pointer_software_rendering: bool,
}

impl ProcessorBuilder {
    pub fn build(self) -> Processor {
        Processor {
            complete_data: CompleteData::new(),
            rfx_handler: rfx::DecodingContext::new(),
            marker_processor: FrameMarkerProcessor::new(self.user_channel_id, self.io_channel_id, self.share_id),
            bitmap_stream_decoder: BitmapStreamDecoder::default(),
            pointer_cache: PointerCache::default(),
            use_system_pointer: true,
            mouse_pos_update: None,
            enable_server_pointer: self.enable_server_pointer,
            pointer_software_rendering: self.pointer_software_rendering,
            // Built on the first update that needs it; see the field documentation.
            bulk_decompressor: None,
            palette: Palette::system_default(),
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
    share_id: u32,
}

impl FrameMarkerProcessor {
    fn new(user_channel_id: u16, io_channel_id: u16, share_id: u32) -> Self {
        Self {
            user_channel_id,
            io_channel_id,
            share_id,
        }
    }

    fn process(&mut self, marker: &FrameMarkerPdu, output: &mut WriteBuf) -> SessionResult<()> {
        match marker.frame_action {
            FrameAction::Begin => Ok(()),
            FrameAction::End => {
                ironrdp_pdu::rdp::headers::encode_share_data(
                    self.user_channel_id,
                    self.io_channel_id,
                    self.share_id,
                    ShareDataPdu::FrameAcknowledge(FrameAcknowledgePdu {
                        frame_id: marker.frame_id.unwrap_or(0),
                    }),
                    output,
                )
                .map_err(SessionError::encode)?;

                Ok(())
            }
        }
    }
}
