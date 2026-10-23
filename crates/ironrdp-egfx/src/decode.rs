//! Codec decoder traits for client-side EGFX processing
//!
//! This module provides pluggable decoder traits that allow consumers
//! to bring their own codec implementations (e.g., openh264, ffmpeg,
//! hardware decoders). The traits are designed for core tier: no I/O,
//! `Send` only. They are intended for use in `std` environments;
//! `no_std` + `alloc` support is not currently guaranteed.
//!
//! # Protocol Context
//!
//! H.264 data arrives inside [RFX_AVC420_BITMAP_STREAM][1] payloads
//! within `RDPGFX_WIRE_TO_SURFACE_PDU_1` messages. The specification
//! defines it as an Annex B byte stream (start code prefix), which is what
//! servers built on FreeRDP's server library, such as GNOME Remote Desktop,
//! send. Earlier versions of this crate documented AVC format (4-byte
//! big-endian length prefix per NAL unit) instead, so decoders should accept
//! both.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/5f12c20e-2ea1-4ad1-a2a0-019ee3893731

use core::fmt;

// ============================================================================
// Decoded Frame
// ============================================================================

/// Decoded bitmap frame from an H.264 decoder
///
/// Contains RGBA pixel data for a decoded H.264 frame.
/// The pixel data is in RGBA format (4 bytes per pixel),
/// row-major, top-to-bottom, left-to-right.
#[derive(Clone)]
#[non_exhaustive]
pub struct DecodedFrame {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

impl DecodedFrame {
    #[expect(
        clippy::as_conversions,
        reason = "usize to u64 is lossless on all supported platforms (32/64-bit)"
    )]
    pub fn new(data: Vec<u8>, width: u32, height: u32) -> Self {
        debug_assert_eq!(
            data.len() as u64,
            u64::from(width).saturating_mul(u64::from(height)).saturating_mul(4),
            "DecodedFrame buffer must be RGBA8888 (width * height * 4 bytes)",
        );
        Self { data, width, height }
    }

    /// RGBA pixel data (4 bytes per pixel).
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Frame width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Frame height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Consume the frame and return the owned RGBA buffer.
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }
}

impl fmt::Debug for DecodedFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecodedFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("data_len", &self.data.len())
            .finish()
    }
}

// ============================================================================
// Decoded YUV 4:2:0 Picture
// ============================================================================

/// Planes of a decoded YUV 4:2:0 picture
///
/// The chroma planes are half the luma plane's width and height, rounded up.
/// AVC444 ([MS-RDPEGFX] 3.3.8.3.2, "YUV420p Stream Combination for YUV444 mode")
/// recombines two such pictures, so it needs the planes rather than RGBA.
///
/// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
#[derive(Clone)]
#[non_exhaustive]
pub struct Yuv420Frame {
    /// INVARIANT: at least `luma_stride * height` bytes, and `width <= luma_stride`.
    y: Vec<u8>,
    /// INVARIANT: at least `chroma_stride * height.div_ceil(2)` bytes, and `width.div_ceil(2) <= chroma_stride`.
    u: Vec<u8>,
    /// INVARIANT: same size as `u`.
    v: Vec<u8>,
    luma_stride: usize,
    chroma_stride: usize,
    width: u16,
    height: u16,
}

impl Yuv420Frame {
    /// Wraps decoded planes with `luma_stride`-byte luma rows and `chroma_stride`-byte chroma rows.
    ///
    /// Fails when a stride is narrower than its plane or a plane is shorter than its rows.
    pub fn new(
        width: u16,
        height: u16,
        y: Vec<u8>,
        luma_stride: usize,
        u: Vec<u8>,
        v: Vec<u8>,
        chroma_stride: usize,
    ) -> DecoderResult<Self> {
        let chroma_width = usize::from(width.div_ceil(2));
        let chroma_height = usize::from(height.div_ceil(2));
        let luma_size = luma_stride.checked_mul(usize::from(height));
        let chroma_size = chroma_stride.checked_mul(chroma_height);
        let fits = |plane: &[u8], size: Option<usize>| size.is_some_and(|size| size <= plane.len());
        if !(usize::from(width) <= luma_stride && chroma_width <= chroma_stride) {
            return Err(DecoderError::msg("YUV 4:2:0 stride narrower than its plane"));
        }
        if !(fits(&y, luma_size) && fits(&u, chroma_size) && fits(&v, chroma_size)) {
            return Err(DecoderError::msg("YUV 4:2:0 plane shorter than its rows"));
        }
        Ok(Self {
            y,
            u,
            v,
            luma_stride,
            chroma_stride,
            width,
            height,
        })
    }

    /// Picture width in pixels.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Picture height in pixels.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Luma row `row`, `width` bytes long, or `None` past the last row.
    pub fn luma_row(&self, row: usize) -> Option<&[u8]> {
        plane_row(
            &self.y,
            self.luma_stride,
            usize::from(self.width),
            usize::from(self.height),
            row,
        )
    }

    /// U row `row`, `width / 2` bytes long (rounded up), or `None` past the last row.
    pub fn u_row(&self, row: usize) -> Option<&[u8]> {
        plane_row(
            &self.u,
            self.chroma_stride,
            usize::from(self.width.div_ceil(2)),
            usize::from(self.height.div_ceil(2)),
            row,
        )
    }

    /// V row `row`, `width / 2` bytes long (rounded up), or `None` past the last row.
    pub fn v_row(&self, row: usize) -> Option<&[u8]> {
        plane_row(
            &self.v,
            self.chroma_stride,
            usize::from(self.width.div_ceil(2)),
            usize::from(self.height.div_ceil(2)),
            row,
        )
    }

    /// Converts the picture to RGBA with the full-range BT.709 transform of MS-RDPEGFX AVC420
    /// streams ([MS-RDPEGFX] 3.3.8.3.1, "Color Conversion").
    ///
    /// [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/da5c75f9-cd99-450c-98c4-014a496942b0
    pub fn to_rgba(&self) -> DecodedFrame {
        let (width, height) = (usize::from(self.width), usize::from(self.height));
        let mut rgba = Vec::with_capacity(width * height * 4);
        for row in 0..height {
            let (Some(y), Some(u), Some(v)) = (self.luma_row(row), self.u_row(row / 2), self.v_row(row / 2)) else {
                break;
            };
            for (col, &y) in y.iter().enumerate() {
                rgba.extend_from_slice(&crate::avc444::yuv_to_rgba(y, u[col / 2], v[col / 2]));
            }
        }
        DecodedFrame::new(rgba, u32::from(self.width), u32::from(self.height))
    }
}

impl fmt::Debug for Yuv420Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Yuv420Frame")
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

fn plane_row(plane: &[u8], stride: usize, width: usize, height: usize, row: usize) -> Option<&[u8]> {
    if height <= row {
        return None;
    }
    plane.get(row * stride..)?.get(..width)
}

// ============================================================================
// Decoder Error
// ============================================================================

/// Error type for decoder operations
#[derive(Debug)]
#[non_exhaustive]
pub struct DecoderError {
    context: String,
    source: Option<Box<dyn core::error::Error + Send + Sync>>,
}

impl DecoderError {
    /// Create a decoder error with a source error
    pub fn new(context: impl Into<String>, source: impl core::error::Error + Send + Sync + 'static) -> Self {
        Self {
            context: context.into(),
            source: Some(Box::new(source)),
        }
    }

    /// Create a decoder error with only a message
    pub fn msg(context: impl Into<String>) -> Self {
        Self {
            context: context.into(),
            source: None,
        }
    }
}

impl fmt::Display for DecoderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "decoder error: {}", self.context)?;
        if let Some(ref source) = self.source {
            write!(f, ": {source}")?;
        }
        Ok(())
    }
}

impl core::error::Error for DecoderError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        self.source.as_deref().map(|e| {
            let err: &(dyn core::error::Error + 'static) = e;
            err
        })
    }
}

/// Result type for decoder operations
pub type DecoderResult<T> = Result<T, DecoderError>;

// ============================================================================
// H.264 Decoder Trait
// ============================================================================

/// Trait for H.264 (AVC) decoders
///
/// Implement this trait to provide H.264 decode capability to the
/// EGFX client. The decoder receives the H.264 data from
/// `RFX_AVC420_BITMAP_STREAM` payloads: an Annex B byte stream per the
/// specification, or AVC-format NAL units (4-byte BE length prefix) from
/// servers that followed this crate's earlier documentation.
///
/// # Thread Safety
///
/// Implementations must be `Send` to work with the DVC framework.
///
/// # Example
///
/// ```ignore
/// use ironrdp_egfx::decode::{H264Decoder, DecodedFrame, DecoderResult};
///
/// struct MyH264Decoder { /* ... */ }
///
/// impl H264Decoder for MyH264Decoder {
///     fn decode(&mut self, data: &[u8]) -> DecoderResult<DecodedFrame> {
///         // Decode H.264 NAL units to RGBA
///         todo!()
///     }
/// }
/// ```
pub trait H264Decoder: Send {
    /// Decode H.264 NAL units, as an Annex B byte stream or in AVC format
    /// (4-byte BE length prefix), into an RGBA bitmap.
    ///
    /// Frame dimensions may exceed the destination rectangle due to
    /// macroblock alignment (16x16). The caller crops to fit.
    fn decode(&mut self, data: &[u8]) -> DecoderResult<DecodedFrame>;

    /// Decode H.264 NAL units like [`H264Decoder::decode`], into the planes of the picture.
    ///
    /// The graphics pipeline converts these itself with the color transform MS-RDPEGFX
    /// specifies, and AVC444 recombines two of them, so a decoder implementing this serves
    /// every AVC mode regardless of the matrix [`H264Decoder::decode`] converts with.
    /// Implementations must also return `true` from [`H264Decoder::supports_yuv420`].
    ///
    /// The returned picture must be the one `data` carries, not an earlier one held back:
    /// AVC444 sends its two views as consecutive pictures of one stream and pairs each
    /// call's result with the sub-stream it passed in.
    fn decode_yuv420(&mut self, data: &[u8]) -> DecoderResult<Yuv420Frame> {
        let _ = data;
        Err(DecoderError::msg("this decoder does not return YUV 4:2:0 planes"))
    }

    /// Whether [`H264Decoder::decode_yuv420`] is implemented.
    ///
    /// The graphics pipeline only advertises AVC444 for decoders that return `true`.
    fn supports_yuv420(&self) -> bool {
        false
    }

    /// Finishes starting the decoder, and reports whether it can decode.
    ///
    /// The graphics pipeline calls this when its channel opens, before it advertises its
    /// capabilities and before any other method, and advertises no AVC capability set when
    /// it returns `false`. A decoder that starts in the background, so creating it does not
    /// delay the connection, waits for that here.
    fn wait_until_ready(&mut self) -> bool {
        true
    }

    /// Reset the decoder state
    ///
    /// Called when surfaces are reset (e.g., on `ResetGraphics`).
    /// The decoder should drop any internal state and prepare for
    /// a new stream.
    fn reset(&mut self) {
        // Default: no-op
    }
}

// ============================================================================
// OpenH264 Implementation
// ============================================================================

/// Matrix and range used to convert decoded YUV pictures to RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YuvMatrix {
    /// ITU-R BT.601 with limited range, as specified for MS-RDPEVOR video.
    Bt601Limited,
    /// ITU-R BT.709 with full range, as specified for MS-RDPEGFX AVC420 streams.
    Bt709Full,
}

#[cfg(feature = "openh264")]
mod openh264_impl {
    use openh264::formats::YUVSource;
    use tracing::warn;

    use super::{DecodedFrame, DecoderError, DecoderResult, H264Decoder, Yuv420Frame, YuvMatrix};

    /// H.264 decoder backed by Cisco's OpenH264 library
    ///
    /// This decoder passes Annex B input to OpenH264 as-is and converts
    /// AVC-format NAL units to Annex B first (OpenH264 only reads Annex B),
    /// decodes to YUV420p, then converts to RGBA for the client pipeline.
    ///
    /// # Feature Gates
    ///
    /// Two construction paths are available depending on the feature flags:
    ///
    /// - `openh264-bundled`: compiles OpenH264 from source at build time.
    ///   Use [`OpenH264Decoder::new()`] to construct.
    ///
    /// - `openh264-libloading`: loads a prebuilt Cisco OpenH264 binary at
    ///   runtime. Use [`OpenH264Decoder::from_library_path()`] to construct.
    ///   The library is verified against known Cisco release hashes.
    pub struct OpenH264Decoder {
        decoder: openh264::decoder::Decoder,
        annex_b_buffer: Vec<u8>,
        matrix: YuvMatrix,
    }

    impl OpenH264Decoder {
        /// Create a decoder using the bundled (source-compiled) OpenH264 library
        ///
        /// This compiles OpenH264 C code at build time. The resulting binary
        /// has no patent coverage from Cisco's license agreement.
        #[cfg(feature = "openh264-bundled")]
        pub fn new() -> DecoderResult<Self> {
            let decoder = openh264::decoder::Decoder::new()
                .map_err(|e| DecoderError::new("failed to create OpenH264 decoder", e))?;

            Ok(Self {
                decoder,
                annex_b_buffer: Vec::new(),
                matrix: YuvMatrix::Bt709Full,
            })
        }

        /// Converts pictures with `matrix` instead of the BT.709 full range of MS-RDPEGFX.
        #[must_use]
        pub fn with_yuv_matrix(mut self, matrix: YuvMatrix) -> Self {
            self.matrix = matrix;
            self
        }

        /// Create a decoder using a dynamically loaded OpenH264 library
        ///
        /// `library_path` should point to a Cisco OpenH264 prebuilt binary,
        /// which is verified against known Cisco release hashes before loading.
        /// Cisco's prebuilt binaries carry patent coverage under their license.
        #[cfg(feature = "openh264-libloading")]
        pub fn from_library_path(library_path: &std::path::Path) -> DecoderResult<Self> {
            let api = openh264::OpenH264API::from_blob_path(library_path)
                .map_err(|e| DecoderError::new("failed to load OpenH264 library", e))?;
            let decoder = openh264::decoder::Decoder::with_api_config(api, Default::default())
                .map_err(|e| DecoderError::new("failed to create OpenH264 decoder", e))?;

            Ok(Self {
                decoder,
                annex_b_buffer: Vec::new(),
                matrix: YuvMatrix::Bt709Full,
            })
        }

        /// Decodes `data`, an Annex B byte stream or AVC-format NAL units, to OpenH264's picture.
        fn decode_picture(&mut self, data: &[u8]) -> DecoderResult<openh264::decoder::DecodedYUV<'_>> {
            let annex_b = if crate::pdu::is_avc_format(data) {
                crate::pdu::avc_to_annex_b_into(data, &mut self.annex_b_buffer);
                &self.annex_b_buffer
            } else {
                data
            };

            self.decoder
                .decode(annex_b)
                .map_err(|e| DecoderError::new("OpenH264 decode failed", e))?
                .ok_or_else(|| DecoderError::msg("OpenH264 returned no picture"))
        }
    }

    impl H264Decoder for OpenH264Decoder {
        fn decode(&mut self, data: &[u8]) -> DecoderResult<DecodedFrame> {
            let matrix = self.matrix;
            let yuv = self.decode_picture(data)?;

            let (width, height) = YUVSource::dimensions(&yuv);
            let (y_stride, u_stride, v_stride) = YUVSource::strides(&yuv);

            #[expect(
                clippy::as_conversions,
                clippy::cast_possible_truncation,
                reason = "H.264 frame dimensions are always within u32 range"
            )]
            let (w32, h32) = (width as u32, height as u32);
            #[expect(
                clippy::as_conversions,
                clippy::cast_possible_truncation,
                reason = "strides are bounded by frame dimensions, which fit in u32"
            )]
            let [y_stride, u_stride, v_stride] = [y_stride as u32, u_stride as u32, v_stride as u32];

            let rgba_stride = w32
                .checked_mul(4)
                .ok_or_else(|| DecoderError::msg("frame dimensions too large for RGBA allocation"))?;
            let rgba_size = width
                .checked_mul(height)
                .and_then(|s| s.checked_mul(4))
                .ok_or_else(|| DecoderError::msg("frame dimensions too large for RGBA allocation"))?;
            let mut rgba = vec![0u8; rgba_size];

            // MS-RDPEGFX AVC420 streams carry full-range BT.709 YUV420 ([MS-RDPEGFX] 3.3.8.3.1),
            // and MS-RDPEVOR video limited-range BT.601, so the matrix is chosen by the caller
            // rather than taken from openh264's `write_rgba8`.
            let (range, standard) = match matrix {
                YuvMatrix::Bt601Limited => (yuv::YuvRange::Limited, yuv::YuvStandardMatrix::Bt601),
                YuvMatrix::Bt709Full => (yuv::YuvRange::Full, yuv::YuvStandardMatrix::Bt709),
            };
            let planar = yuv::YuvPlanarImage {
                y_plane: yuv.y(),
                y_stride,
                u_plane: yuv.u(),
                u_stride,
                v_plane: yuv.v(),
                v_stride,
                width: w32,
                height: h32,
            };
            yuv::yuv420_to_rgba(&planar, &mut rgba, rgba_stride, range, standard)
                .map_err(|e| DecoderError::new("failed to convert YUV420 to RGBA", e))?;

            Ok(DecodedFrame::new(rgba, w32, h32))
        }

        fn decode_yuv420(&mut self, data: &[u8]) -> DecoderResult<Yuv420Frame> {
            let yuv = self.decode_picture(data)?;

            let (width, height) = YUVSource::dimensions(&yuv);
            let (y_stride, u_stride, v_stride) = YUVSource::strides(&yuv);
            let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
                return Err(DecoderError::msg("OpenH264 picture dimensions exceed 65535"));
            };
            if u_stride != v_stride {
                return Err(DecoderError::msg("OpenH264 chroma planes have different strides"));
            }
            Yuv420Frame::new(
                width,
                height,
                yuv.y().to_vec(),
                y_stride,
                yuv.u().to_vec(),
                yuv.v().to_vec(),
                u_stride,
            )
        }

        fn supports_yuv420(&self) -> bool {
            true
        }

        fn reset(&mut self) {
            // Recreate decoder from source when available
            #[cfg(feature = "openh264-bundled")]
            match openh264::decoder::Decoder::new() {
                Ok(new_decoder) => self.decoder = new_decoder,
                Err(e) => warn!("Failed to reset OpenH264 decoder, reusing existing state: {e}"),
            }
            // In libloading-only mode, we don't have the library path stored,
            // so we can't recreate. The existing decoder handles new SPS/PPS
            // transparently when the next I-frame arrives.
        }
    }
}

#[cfg(feature = "openh264")]
pub use openh264_impl::OpenH264Decoder;
