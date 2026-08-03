use std::sync::Arc;

use ironrdp_bulk::{BulkCompressor, BulkError};
use ironrdp_core::{DecodeErrorKind, ReadCursor, WriteBuf, decode_cursor};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::pointer::{DecodedPointer, PointerBitmapTarget, PointerError};
use ironrdp_graphics::rdp6::BitmapStreamDecoder;
use ironrdp_graphics::rle::RlePixelFormat;
use ironrdp_pdu::bitmap::BitmapUpdateData;
use ironrdp_pdu::codecs::rfx::FrameAcknowledgePdu;
use ironrdp_pdu::fast_path::{FastPathHeader, FastPathUpdate, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp_pdu::pointer::PointerUpdateData;
use ironrdp_pdu::rdp::capability_sets::{CODEC_ID_NONE, CODEC_ID_REMOTEFX, CodecId};
use ironrdp_pdu::rdp::headers::{CompressionFlags, ShareDataPdu};
use ironrdp_pdu::surface_commands::{FrameAction, FrameMarkerPdu, SurfaceCommand};
use tracing::{debug, trace, warn};

use crate::image::DecodedImage;
use crate::palette::Palette;
use crate::pointer::PointerCache;
use crate::{SessionError, SessionErrorExt as _, SessionErrorKind, SessionResult, reason_err, rfx};

/// A bounded category for a failed Fast-Path bulk decompression operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BulkDecompressionErrorKind {
    UnsupportedCompressionType,
    InvalidCompressedData,
    OutputBufferTooSmall,
    HistoryBufferOverflow,
    UnexpectedEndOfInput,
}

impl BulkDecompressionErrorKind {
    /// Returns the stable, value-free trace label for this category.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedCompressionType => "UnsupportedCompressionType",
            Self::InvalidCompressedData => "InvalidCompressedData",
            Self::OutputBufferTooSmall => "OutputBufferTooSmall",
            Self::HistoryBufferOverflow => "HistoryBufferOverflow",
            Self::UnexpectedEndOfInput => "UnexpectedEndOfInput",
        }
    }
}

/// Bounded metadata describing a failed Fast-Path bulk decompression operation.
///
/// This deliberately retains protocol metadata only. It never retains the compressed data or
/// detailed decoder error text, which can contain information derived from the remote endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FastPathBulkDecompressionFailure {
    compression_flags: u8,
    compression_type: Option<u8>,
    update_code: u8,
    fragmentation: u8,
    payload_length: usize,
    error_kind: BulkDecompressionErrorKind,
}

impl FastPathBulkDecompressionFailure {
    fn new(attributes: FragmentAttributes, payload_length: usize, error: &BulkError) -> Self {
        let error_kind = match error {
            BulkError::UnsupportedCompressionType(_) => BulkDecompressionErrorKind::UnsupportedCompressionType,
            BulkError::InvalidCompressedData(_) => BulkDecompressionErrorKind::InvalidCompressedData,
            BulkError::OutputBufferTooSmall { .. } => BulkDecompressionErrorKind::OutputBufferTooSmall,
            BulkError::HistoryBufferOverflow => BulkDecompressionErrorKind::HistoryBufferOverflow,
            BulkError::UnexpectedEndOfInput => BulkDecompressionErrorKind::UnexpectedEndOfInput,
        };

        Self {
            compression_flags: attributes.compression_flags.map_or(0, |flags| flags.bits()),
            compression_type: attributes
                .compression_type
                .map(|compression_type| compression_type.as_u8()),
            update_code: attributes.update_code.as_u8(),
            fragmentation: attributes.fragmentation.as_u8(),
            payload_length,
            error_kind,
        }
    }

    /// Returns the Fast-Path bulk compression control flags.
    pub const fn compression_flags(self) -> u8 {
        self.compression_flags
    }

    /// Returns the Fast-Path bulk compression type, if the PDU provided one.
    pub const fn compression_type(self) -> Option<u8> {
        self.compression_type
    }

    /// Returns the Fast-Path update code.
    pub const fn update_code(self) -> u8 {
        self.update_code
    }

    /// Returns the initial Fast-Path fragmentation mode.
    pub const fn fragmentation(self) -> u8 {
        self.fragmentation
    }

    /// Returns the failing fragment's compressed payload size in bytes.
    pub const fn payload_length(self) -> usize {
        self.payload_length
    }

    /// Returns the bounded bulk decoder error category.
    pub const fn error_kind(self) -> BulkDecompressionErrorKind {
        self.error_kind
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
    /// Current 8bpp color palette. Updated by Palette fast-path updates.
    palette: Palette,
    /// A malformed bitmap was discarded and the client should request a complete visual recovery.
    ///
    /// Only one request is issued per activation to prevent a malformed server stream from
    /// repeatedly triggering redraw traffic.
    bitmap_recovery_requested: bool,
    bitmap_recovery_pending: bool,
    #[cfg(feature = "qoiz")]
    zdctx: zstd_safe::DCtx<'static>,
}

impl Processor {
    pub fn update_mouse_pos(&mut self, x: u16, y: u16) {
        self.mouse_pos_update = Some((x, y));
    }

    /// Returns whether a malformed visual update requires a one-time full redraw request.
    pub(crate) fn take_bitmap_recovery_request(&mut self) -> bool {
        core::mem::take(&mut self.bitmap_recovery_pending)
    }

    fn request_bitmap_recovery(&mut self) {
        if !self.bitmap_recovery_requested {
            self.bitmap_recovery_requested = true;
            self.bitmap_recovery_pending = true;
        }
    }

    /// Process input fast path frame and return list of updates.
    pub fn process(
        &mut self,
        image: &mut DecodedImage,
        input: &[u8],
        output: &mut WriteBuf,
        bulk_decompressor: &mut Option<BulkCompressor>,
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
            let update_result = self.process_single_update(&mut input, image, output, bulk_decompressor)?;
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
        bulk_decompressor: &mut Option<BulkCompressor>,
    ) -> SessionResult<Vec<UpdateKind>> {
        let mut processor_updates = Vec::new();

        let raw_update_code = input.remaining().first().map(|header| header & 0x0F);
        let update_pdu = match decode_cursor::<FastPathUpdatePdu<'_>>(input) {
            Ok(update_pdu) => update_pdu,
            Err(error)
                if raw_update_code.is_some_and(is_visual_update_code)
                    && matches!(error.kind(), DecodeErrorKind::NotEnoughBytes { .. }) =>
            {
                let DecodeErrorKind::NotEnoughBytes { received, expected } = error.kind() else {
                    return Err(SessionError::decode(error));
                };
                let discarded_bytes = input.read_remaining().len();
                self.complete_data.discard();
                if raw_update_code == Some(UpdateCode::Bitmap.as_u8()) {
                    self.request_bitmap_recovery();
                }
                warn!(
                    update_code = ?raw_update_code,
                    payload_length = discarded_bytes,
                    received, expected, "Ignoring truncated Fast-Path visual update PDU"
                );
                processor_updates.push(UpdateKind::None);
                return Ok(processor_updates);
            }
            Err(error) => return Err(SessionError::decode(error)),
        };
        trace!(fast_path_update_fragmentation = ?update_pdu.fragmentation);

        let attributes = FragmentAttributes::from(&update_pdu);
        let decompressed_data;
        let update_data = if attributes.compression_flags.is_some_and(|flags| !flags.is_empty()) {
            decompressed_data = Self::decompress_fragment_data(update_pdu.data, attributes, bulk_decompressor)?;
            decompressed_data.as_slice()
        } else {
            update_pdu.data
        };

        let Some(data) = self
            .complete_data
            .process_data(update_data, update_pdu.fragmentation, attributes)?
        else {
            return Ok(processor_updates);
        };
        let FragmentedUpdate { data, attributes } = data;

        let update = FastPathUpdate::decode_with_code(data.as_slice(), attributes.update_code);

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
                match e.kind() {
                    DecodeErrorKind::InvalidField { field, reason } => {
                        warn!(field, reason, "Ignoring invalid Fast-Path update");
                        processor_updates.push(UpdateKind::None);
                    }
                    DecodeErrorKind::NotEnoughBytes { received, expected }
                        if is_visual_update_code(attributes.update_code.as_u8()) =>
                    {
                        warn!(
                            update_code = attributes.update_code.as_u8(),
                            fragmentation = attributes.fragmentation.as_u8(),
                            payload_length = data.len(),
                            received,
                            expected,
                            "Ignoring truncated Fast-Path visual update"
                        );
                        if attributes.update_code == UpdateCode::Bitmap {
                            self.request_bitmap_recovery();
                        }
                        processor_updates.push(UpdateKind::None);
                    }
                    _ => {
                        let mut session_error = SessionError::decode(e);
                        session_error.set_context("FastPathUpdate");
                        return Err(session_error);
                    }
                }
            }
        };

        Ok(processor_updates)
    }

    /// Process a palette update shared between fast-path and slow-path pipelines.
    pub(crate) fn process_palette_update(&mut self, palette_data: &[u8]) {
        self.palette.process_update(palette_data);
    }
    fn decompress_fragment_data(
        data: &[u8],
        attributes: FragmentAttributes,
        bulk_decompressor: &mut Option<BulkCompressor>,
    ) -> SessionResult<Vec<u8>> {
        let decompressor = bulk_decompressor
            .as_mut()
            .ok_or_else(|| reason_err!("FastPath", "received compression control flags without a decompressor"))?;
        let bulk_flags = u32::from(attributes.compression_flags.map_or(0, |flags| flags.bits()))
            | u32::from(
                attributes
                    .compression_type
                    .map_or(0, |compression_type| compression_type.as_u8()),
            );
        let decompressed = decompressor.decompress(data, bulk_flags).map_err(|error| {
            SessionError::new(
                "FastPath bulk decompression",
                SessionErrorKind::FastPathBulkDecompression(FastPathBulkDecompressionFailure::new(
                    attributes,
                    data.len(),
                    &error,
                )),
            )
        })?;
        let decompressed_data = decompressed.to_vec();
        debug!(
            compressed_size = data.len(),
            decompressed_size = decompressed_data.len(),
            compression_type = ?attributes.compression_type,
            compression_ratio = format_args!("{:.2}x", decompressor.compression_ratio()),
            total_compressed = decompressor.total_compressed_bytes(),
            total_uncompressed = decompressor.total_uncompressed_bytes(),
            "Decompressed Fast-Path update fragment"
        );
        Ok(decompressed_data)
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
            let Some(update_rectangle) = image.bitmap_destination(&update.rectangle, update.width, update.height)
            else {
                warn!("Ignoring bitmap update with an invalid declared destination");
                self.request_bitmap_recovery();
                continue;
            };
            buf.clear();

            let apply_bitmap =
                |result: SessionResult<InclusiveRectangle>| -> SessionResult<Option<InclusiveRectangle>> {
                    match result {
                        Ok(rectangle) => Ok(Some(rectangle)),
                        Err(error) if matches!(error.kind(), SessionErrorKind::InvalidBitmapSourceLength) => {
                            warn!("Ignoring bitmap update with an invalid source length");
                            Ok(None)
                        }
                        Err(error) => Err(error),
                    }
                };

            // Bitmap data is either compressed or uncompressed, depending
            // on whether the BITMAP_COMPRESSION flag is present in the
            // flags field.
            let applied_rectangle = if update
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
                        // RDP6 bitmap streams are bottom-up, so reverse them while decoding to
                        // keep the framebuffer top-down. This matches FreeRDP's GDI frontend,
                        // which passes `vFlip = TRUE` to its planar bitmap decoder.
                        Ok(()) => apply_bitmap(image.apply_rgb24(&buf, &update_rectangle, update.width, true))?,
                        Err(err) => {
                            warn!("Invalid RDP6_BITMAP_STREAM: {err}");
                            None
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
                        Ok(RlePixelFormat::Rgb16) => {
                            apply_bitmap(image.apply_rgb16_bitmap(&buf, &update_rectangle, update.width))?
                        }
                        Ok(RlePixelFormat::Rgb15) => {
                            apply_bitmap(image.apply_rgb15_bitmap(&buf, &update_rectangle, update.width))?
                        }
                        Ok(RlePixelFormat::Rgb24) => {
                            apply_bitmap(image.apply_bgr24_bitmap(&buf, &update_rectangle, update.width))?
                        }
                        Ok(RlePixelFormat::Rgb8) => apply_bitmap(image.apply_rgb8_with_palette(
                            &buf,
                            &update_rectangle,
                            self.palette.colors(),
                            update.width,
                        ))?,

                        Err(e) => {
                            warn!("Invalid RLE-compressed bitmap: {e}");
                            None
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
                        8 => apply_bitmap(image.apply_rgb8_with_palette(
                            &buf,
                            &update_rectangle,
                            self.palette.colors(),
                            update.width,
                        ))?,
                        15 => apply_bitmap(image.apply_rgb15_bitmap(&buf, &update_rectangle, update.width))?,
                        16 => apply_bitmap(image.apply_rgb16_bitmap(&buf, &update_rectangle, update.width))?,
                        24 => apply_bitmap(image.apply_bgr24_bitmap(&buf, &update_rectangle, update.width))?,
                        32 => apply_bitmap(image.apply_rgb32_bitmap(
                            &buf,
                            PixelFormat::BgrX32,
                            &update_rectangle,
                            update.width,
                        ))?,
                        _ => {
                            warn!("Unsupported uncompressed bitmap depth: {bpp} bpp");
                            None
                        }
                    }
                } else {
                    match update.bits_per_pixel {
                        8 => apply_bitmap(image.apply_rgb8_with_palette(
                            update.bitmap_data,
                            &update_rectangle,
                            self.palette.colors(),
                            update.width,
                        ))?,
                        15 => {
                            apply_bitmap(image.apply_rgb15_bitmap(update.bitmap_data, &update_rectangle, update.width))?
                        }
                        16 => {
                            apply_bitmap(image.apply_rgb16_bitmap(update.bitmap_data, &update_rectangle, update.width))?
                        }
                        24 => {
                            apply_bitmap(image.apply_bgr24_bitmap(update.bitmap_data, &update_rectangle, update.width))?
                        }
                        32 => apply_bitmap(image.apply_rgb32_bitmap(
                            update.bitmap_data,
                            PixelFormat::BgrX32,
                            &update_rectangle,
                            update.width,
                        ))?,
                        _ => {
                            warn!("Unsupported uncompressed bitmap depth: {bpp} bpp");
                            None
                        }
                    }
                }
            };

            if let Some(update_rectangle) = applied_rectangle {
                match update_kind {
                    UpdateKind::Region(current) => update_kind = UpdateKind::Region(current.union(&update_rectangle)),
                    _ => update_kind = UpdateKind::Region(update_rectangle),
                }
            } else {
                self.request_bitmap_recovery();
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

                let decoded_pointer = match DecodedPointer::decode_color_pointer_attribute(&pointer, bitmap_target) {
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
                                ),
                                15 => image.apply_rgb15_bitmap(ext_data.data, &destination, source_width),
                                16 => image.apply_rgb16_bitmap(ext_data.data, &destination, source_width),
                                24 => image.apply_bgr24_bitmap(ext_data.data, &destination, source_width),
                                32 => image.apply_rgb32_bitmap(
                                    ext_data.data,
                                    PixelFormat::BgrX32,
                                    &destination,
                                    source_width,
                                ),
                                bpp => {
                                    warn!("Unsupported surface CODEC_ID_NONE bpp: {bpp}");
                                    continue;
                                }
                            };
                            let rectangle = match rectangle {
                                Ok(rectangle) => rectangle,
                                Err(error) if matches!(error.kind(), SessionErrorKind::InvalidBitmapSourceLength) => {
                                    warn!("Ignoring surface bitmap with an invalid source length");
                                    self.request_bitmap_recovery();
                                    continue;
                                }
                                Err(error) => return Err(error),
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
            palette: Palette::system_default(),
            bitmap_recovery_requested: false,
            bitmap_recovery_pending: false,
            #[cfg(feature = "qoiz")]
            zdctx: zstd_safe::DCtx::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FragmentAttributes {
    fragmentation: Fragmentation,
    update_code: UpdateCode,
    compression_flags: Option<CompressionFlags>,
    compression_type: Option<ironrdp_pdu::rdp::client_info::CompressionType>,
}

impl From<&FastPathUpdatePdu<'_>> for FragmentAttributes {
    fn from(update: &FastPathUpdatePdu<'_>) -> Self {
        Self {
            fragmentation: update.fragmentation,
            update_code: update.update_code,
            compression_flags: update.compression_flags,
            compression_type: update.compression_type,
        }
    }
}

#[derive(Debug, PartialEq)]
struct FragmentedUpdate {
    data: Vec<u8>,
    attributes: FragmentAttributes,
}

#[derive(Debug, PartialEq)]
struct CompleteData {
    fragmented_data: Option<FragmentedUpdate>,
}

impl CompleteData {
    fn new() -> Self {
        Self { fragmented_data: None }
    }

    fn process_data(
        &mut self,
        data: &[u8],
        fragmentation: Fragmentation,
        attributes: FragmentAttributes,
    ) -> SessionResult<Option<FragmentedUpdate>> {
        match fragmentation {
            Fragmentation::Single => {
                self.check_data_is_empty();

                Ok(Some(FragmentedUpdate {
                    data: data.to_vec(),
                    attributes,
                }))
            }
            Fragmentation::First => {
                self.check_data_is_empty();

                self.fragmented_data = Some(FragmentedUpdate {
                    data: data.to_vec(),
                    attributes,
                });

                Ok(None)
            }
            Fragmentation::Next => {
                self.append_data(data, attributes)?;

                Ok(None)
            }
            Fragmentation::Last => {
                self.append_data(data, attributes)?;

                Ok(self.fragmented_data.take())
            }
        }
    }

    fn check_data_is_empty(&mut self) {
        if self.fragmented_data.is_some() {
            warn!("Skipping pending Fast-Path Update internal multiple elements data");
            self.fragmented_data = None;
        }
    }

    fn discard(&mut self) {
        if self.fragmented_data.take().is_some() {
            warn!("Discarding pending Fast-Path update after malformed visual data");
        }
    }

    fn append_data(&mut self, data: &[u8], attributes: FragmentAttributes) -> SessionResult<()> {
        if let Some(fragmented_update) = self.fragmented_data.as_mut() {
            if fragmented_update.attributes.update_code != attributes.update_code
                || fragmented_update.attributes.compression_flags.is_some() != attributes.compression_flags.is_some()
            {
                return Err(reason_err!("FastPath", "inconsistent fragmented update metadata"));
            }

            // MS-RDPBCGR 3.2.5.9.3.1 requires equal updateCode and header compression
            // subfield values, but does not require equal compressionFlags. Each fragment
            // has already applied its own compression flags before reassembly.
            fragmented_update.data.extend_from_slice(data);
            Ok(())
        } else {
            Err(reason_err!(
                "FastPath",
                "received a non-initial fragment without an active fragment sequence"
            ))
        }
    }
}

fn is_visual_update_code(update_code: u8) -> bool {
    update_code == UpdateCode::Bitmap.as_u8()
        || matches!(
            update_code,
            code if code == UpdateCode::HiddenPointer.as_u8()
                || code == UpdateCode::DefaultPointer.as_u8()
                || code == UpdateCode::PositionPointer.as_u8()
                || code == UpdateCode::ColorPointer.as_u8()
                || code == UpdateCode::CachedPointer.as_u8()
                || code == UpdateCode::NewPointer.as_u8()
                || code == UpdateCode::LargePointer.as_u8()
        )
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

#[cfg(test)]
mod tests {
    use ironrdp_pdu::bitmap::{BitmapData, Compression};
    use ironrdp_pdu::geometry::ExclusiveRectangle;
    use ironrdp_pdu::pointer::{ColorPointerAttribute, Point16, PointerAttribute};
    use ironrdp_pdu::surface_commands::{ExtendedBitmapDataPdu, SurfaceBitsPdu};

    use ironrdp_graphics::rdp6::{BitmapStreamEncoder, RgbChannels};

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
    fn rdp6_bitmap_update_flips_bottom_up_scanlines_before_blitting() {
        for rle in [false, true] {
            let mut processor = ProcessorBuilder {
                io_channel_id: 0,
                user_channel_id: 0,
                share_id: 0,
                enable_server_pointer: false,
                pointer_software_rendering: false,
            }
            .build();
            let mut image = DecodedImage::new(PixelFormat::RgbA32, 2, 2);

            // Keep the first stream row distinct from the last so the row flip is observable.
            let wire_rgb = [
                30, 31, 32, 40, 41, 42, // first stream row
                10, 11, 12, 20, 21, 22, // last stream row
            ];
            let mut bitmap_data = vec![0; wire_rgb.len() + 8];
            let written = BitmapStreamEncoder::new(2, 2)
                .encode_bitmap::<RgbChannels>(&wire_rgb, &mut bitmap_data, rle)
                .unwrap();

            let bitmap_update = BitmapUpdateData {
                rectangles: vec![BitmapData {
                    rectangle: InclusiveRectangle {
                        left: 0,
                        top: 0,
                        right: 1,
                        bottom: 1,
                    },
                    width: 2,
                    height: 2,
                    bits_per_pixel: 32,
                    compression_flags: Compression::BITMAP_COMPRESSION,
                    compressed_data_header: None,
                    bitmap_data: &bitmap_data[..written],
                }],
            };

            processor.process_bitmap_update(&mut image, bitmap_update).unwrap();

            let pixel = |x: usize, y: usize| -> [u8; 4] {
                let offset = (y * usize::from(image.width()) + x) * 4;
                image.data()[offset..offset + 4]
                    .try_into()
                    .expect("pixel has four channels")
            };
            assert_eq!(pixel(0, 0), [10, 11, 12, 255], "RLE: {rle}");
            assert_eq!(pixel(1, 0), [20, 21, 22, 255], "RLE: {rle}");
            assert_eq!(pixel(0, 1), [30, 31, 32, 255], "RLE: {rle}");
            assert_eq!(pixel(1, 1), [40, 41, 42, 255], "RLE: {rle}");
        }
    }

    #[test]
    fn malformed_bitmap_requests_one_recovery_without_terminating_the_session() {
        let mut processor = ProcessorBuilder {
            io_channel_id: 1003,
            user_channel_id: 1001,
            share_id: 1,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();
        let mut image = DecodedImage::new(PixelFormat::RgbA32, 1, 1);
        let mut frame = ironrdp_core::encode_vec(&FastPathHeader::new(
            ironrdp_pdu::fast_path::EncryptionFlags::empty(),
            1 /* update header */ + 2 /* update data length */ + 3_200, /* truncated update data */
        ))
        .unwrap();
        frame.push(UpdateCode::Bitmap.as_u8());
        frame.extend_from_slice(&48_788u16.to_le_bytes());
        frame.resize(frame.len() + 3_200, 0);
        let mut output = WriteBuf::new();
        let mut bulk_decompressor = None;

        assert!(
            processor
                .process(&mut image, &frame, &mut output, &mut bulk_decompressor)
                .is_ok()
        );
        assert!(processor.take_bitmap_recovery_request());
        assert!(!processor.take_bitmap_recovery_request());

        assert!(
            processor
                .process(&mut image, &frame, &mut output, &mut bulk_decompressor)
                .is_ok()
        );
        assert!(!processor.take_bitmap_recovery_request());
    }

    #[test]
    fn malformed_bitmap_content_requests_recovery_without_terminating_the_session() {
        let mut processor = ProcessorBuilder {
            io_channel_id: 1003,
            user_channel_id: 1001,
            share_id: 1,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();
        let mut image = DecodedImage::new(PixelFormat::RgbA32, 1, 1);
        let bitmap_update = BitmapUpdateData {
            rectangles: vec![BitmapData {
                rectangle: InclusiveRectangle {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                width: 1,
                height: 1,
                bits_per_pixel: 16,
                compression_flags: Compression::empty(),
                compressed_data_header: None,
                bitmap_data: &[],
            }],
        };

        assert!(processor.process_bitmap_update(&mut image, bitmap_update).is_ok());
        assert!(processor.take_bitmap_recovery_request());
    }

    #[test]
    fn malformed_surface_bitmap_requests_recovery_without_terminating_the_session() {
        let mut processor = ProcessorBuilder {
            io_channel_id: 1003,
            user_channel_id: 1001,
            share_id: 1,
            enable_server_pointer: false,
            pointer_software_rendering: false,
        }
        .build();
        let mut image = DecodedImage::new(PixelFormat::RgbA32, 1, 1);
        let mut output = WriteBuf::new();
        let surface_bits = SurfaceBitsPdu {
            destination: ExclusiveRectangle {
                left: 0,
                top: 0,
                right: 1,
                bottom: 1,
            },
            extended_bitmap_data: ExtendedBitmapDataPdu {
                bpp: 16,
                codec_id: 0,
                width: 1,
                height: 1,
                header: None,
                data: &[],
            },
        };

        assert!(
            processor
                .process_surface_commands(
                    &mut image,
                    &mut output,
                    vec![SurfaceCommand::SetSurfaceBits(surface_bits)]
                )
                .is_ok()
        );
        assert!(processor.take_bitmap_recovery_request());
    }

    #[test]
    fn truncated_fast_path_pointer_updates_do_not_terminate_the_session() {
        let mut processor = ProcessorBuilder {
            io_channel_id: 1003,
            user_channel_id: 1001,
            share_id: 1,
            enable_server_pointer: true,
            pointer_software_rendering: false,
        }
        .build();
        let mut image = DecodedImage::new(PixelFormat::RgbA32, 1, 1);
        let mut output = WriteBuf::new();
        let mut bulk_decompressor = None;

        let mut truncated_outer = ironrdp_core::encode_vec(&FastPathHeader::new(
            ironrdp_pdu::fast_path::EncryptionFlags::empty(),
            1 /* update header */ + 2 /* update data length */ + 3_200, /* truncated update data */
        ))
        .unwrap();
        truncated_outer.push(UpdateCode::ColorPointer.as_u8());
        truncated_outer.extend_from_slice(&48_788u16.to_le_bytes());
        truncated_outer.resize(truncated_outer.len() + 3_200, 0);

        assert!(
            processor
                .process(&mut image, &truncated_outer, &mut output, &mut bulk_decompressor)
                .is_ok()
        );
        assert!(!processor.take_bitmap_recovery_request());

        let truncated_pointer = ironrdp_core::encode_vec(&FastPathUpdatePdu {
            fragmentation: Fragmentation::Single,
            update_code: UpdateCode::ColorPointer,
            compression_flags: None,
            compression_type: None,
            data: &[],
        })
        .unwrap();
        let mut truncated_inner = ironrdp_core::encode_vec(&FastPathHeader::new(
            ironrdp_pdu::fast_path::EncryptionFlags::empty(),
            truncated_pointer.len(),
        ))
        .unwrap();
        truncated_inner.extend_from_slice(&truncated_pointer);

        assert!(
            processor
                .process(&mut image, &truncated_inner, &mut output, &mut bulk_decompressor)
                .is_ok()
        );
        assert!(!processor.take_bitmap_recovery_request());
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

    #[test]
    fn malformed_color_pointer_uses_default_and_evicts_cached_shape() {
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
        let pointer = ColorPointerAttribute {
            cache_index: 0,
            hot_spot: Point16 { x: 0, y: 0 },
            width: 1,
            height: 1,
            xor_mask: &[],
            and_mask: &[],
        };

        let updates = processor
            .process_pointer_update(&mut image, PointerUpdateData::Color(pointer))
            .expect("malformed pointer must not fail the session");

        assert!(matches!(updates.as_slice(), [UpdateKind::PointerDefault]));
        assert!(!processor.pointer_cache.is_cached(0));
    }
}
#[cfg(test)]
mod compression_tests {
    use super::*;

    #[test]
    fn fragmented_updates_reassemble_with_initial_attributes() {
        let mut complete_data = CompleteData::new();
        let first_attributes = FragmentAttributes {
            fragmentation: Fragmentation::First,
            update_code: UpdateCode::Bitmap,
            compression_flags: Some(CompressionFlags::COMPRESSED),
            compression_type: Some(ironrdp_pdu::rdp::client_info::CompressionType::K64),
        };
        let last_attributes = FragmentAttributes {
            fragmentation: Fragmentation::Last,
            compression_type: Some(ironrdp_pdu::rdp::client_info::CompressionType::K8),
            ..first_attributes
        };

        assert_eq!(
            complete_data
                .process_data(b"first", Fragmentation::First, first_attributes)
                .expect("first fragment should be accepted"),
            None
        );
        assert_eq!(
            complete_data
                .process_data(b"last", Fragmentation::Last, last_attributes)
                .expect("last fragment should be accepted"),
            Some(FragmentedUpdate {
                data: b"firstlast".to_vec(),
                attributes: first_attributes,
            })
        );
    }

    #[test]
    fn fragmented_updates_reject_inconsistent_metadata() {
        let mut complete_data = CompleteData::new();
        let first_attributes = FragmentAttributes {
            fragmentation: Fragmentation::First,
            update_code: UpdateCode::Bitmap,
            compression_flags: Some(CompressionFlags::COMPRESSED),
            compression_type: Some(ironrdp_pdu::rdp::client_info::CompressionType::K64),
        };
        let invalid_attributes = FragmentAttributes {
            fragmentation: Fragmentation::Last,
            update_code: UpdateCode::Palette,
            ..first_attributes
        };

        assert!(
            complete_data
                .process_data(b"first", Fragmentation::First, first_attributes)
                .expect("first fragment should be accepted")
                .is_none()
        );
        assert!(
            complete_data
                .process_data(b"last", Fragmentation::Last, invalid_attributes)
                .is_err()
        );
    }

    #[test]
    fn bulk_decompression_failure_retains_only_bounded_metadata() {
        let attributes = FragmentAttributes {
            fragmentation: Fragmentation::First,
            update_code: UpdateCode::Bitmap,
            compression_flags: Some(CompressionFlags::COMPRESSED | CompressionFlags::FLUSHED),
            compression_type: Some(ironrdp_pdu::rdp::client_info::CompressionType::Rdp6),
        };

        let failure = FastPathBulkDecompressionFailure::new(
            attributes,
            1_024,
            &BulkError::InvalidCompressedData("details must not escape"),
        );

        assert_eq!(
            failure.compression_flags(),
            CompressionFlags::COMPRESSED.bits() | CompressionFlags::FLUSHED.bits()
        );
        assert_eq!(
            failure.compression_type(),
            Some(ironrdp_pdu::rdp::client_info::CompressionType::Rdp6.as_u8())
        );
        assert_eq!(failure.update_code(), UpdateCode::Bitmap.as_u8());
        assert_eq!(failure.fragmentation(), Fragmentation::First.as_u8());
        assert_eq!(failure.payload_length(), 1_024);
        assert_eq!(failure.error_kind(), BulkDecompressionErrorKind::InvalidCompressedData);
    }
}
