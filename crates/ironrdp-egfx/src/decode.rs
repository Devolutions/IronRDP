//! Codec decoder traits for client-side EGFX processing
//!
//! This module provides pluggable decoder traits that allow consumers
//! to bring their own codec implementations (e.g., openh264, ffmpeg,
//! hardware decoders). The traits are designed for core tier: no I/O,
//! no `std` dependency, `Send` only.
//!
//! # Protocol Context
//!
//! H.264 data arrives inside [RFX_AVC420_BITMAP_STREAM][1] payloads
//! within `RDPGFX_WIRE_TO_SURFACE_PDU_1` messages. The NAL units
//! are in AVC format (4-byte big-endian length prefix per NAL unit),
//! not Annex B (start code prefix).
//!
//! # OpenH264 Licensing
//!
//! When using the `openh264` feature, the library is loaded dynamically
//! at runtime via `libloading`. For patent compliance, the loaded binary
//! should be Cisco's precompiled OpenH264 release (not compiled from source).
//! See [Cisco's Binary License](https://www.openh264.org/BINARY_LICENSE.txt)
//! for the conditions that must be met:
//!
//! 1. The binary must be downloaded separately to the end user's device
//! 2. The end user must be able to enable, disable, and re-enable it
//! 3. The software must display: "OpenH264 Video Codec provided by Cisco Systems, Inc."
//! 4. All license text must be reproduced in licensing information
//!
//! Compiling OpenH264 from source (the `openh264-source` feature) provides
//! ZERO patent coverage from Cisco. Use only for development and testing.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/d65c3f9c-2088-4302-90c0-53adc0e11a78

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
pub struct DecodedFrame {
    /// RGBA pixel data (4 bytes per pixel)
    pub data: Vec<u8>,
    /// Frame width in pixels
    pub width: u32,
    /// Frame height in pixels
    pub height: u32,
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
// Decoder Error
// ============================================================================

/// Error type for decoder operations
#[derive(Debug)]
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
/// EGFX client. The decoder receives AVC-format NAL units (length-prefixed,
/// not Annex B) from `RFX_AVC420_BITMAP_STREAM` payloads.
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
    /// Decode AVC-format H.264 NAL units (4-byte BE length prefix, not Annex B)
    /// into an RGBA bitmap.
    ///
    /// Frame dimensions may exceed the destination rectangle due to
    /// macroblock alignment (16x16). The caller crops to fit.
    fn decode(&mut self, data: &[u8]) -> DecoderResult<DecodedFrame>;

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
// OpenH264 Attribution
// ============================================================================

/// Cisco attribution notice required when using OpenH264
///
/// Per Cisco's Binary License, software using OpenH264 must display:
/// "OpenH264 Video Codec provided by Cisco Systems, Inc."
///
/// This constant is provided for consumers to include in their
/// About/Licensing UI.
#[cfg(feature = "openh264")]
pub const OPENH264_ATTRIBUTION: &str = "OpenH264 Video Codec provided by Cisco Systems, Inc.";

// ============================================================================
// OpenH264 Implementation (libloading)
// ============================================================================

#[cfg(feature = "openh264")]
mod openh264_impl {
    use super::{DecodedFrame, DecoderError, DecoderResult, H264Decoder};
    use tracing::{info, warn};

    /// Well-known system paths where Cisco's OpenH264 binary may be installed
    const SYSTEM_LIBRARY_PATHS: &[&str] = &[
        // Debian/Ubuntu multiarch
        "/usr/lib/x86_64-linux-gnu/libopenh264.so",
        "/usr/lib/x86_64-linux-gnu/libopenh264.so.7",
        "/usr/lib/x86_64-linux-gnu/libopenh264.so.6",
        "/usr/lib/aarch64-linux-gnu/libopenh264.so",
        "/usr/lib/aarch64-linux-gnu/libopenh264.so.7",
        "/usr/lib/aarch64-linux-gnu/libopenh264.so.6",
        // Fedora/RHEL
        "/usr/lib64/libopenh264.so",
        "/usr/lib64/libopenh264.so.7",
        "/usr/lib64/libopenh264.so.6",
        // Arch/generic
        "/usr/lib/libopenh264.so",
        "/usr/lib/libopenh264.so.7",
        "/usr/lib/libopenh264.so.6",
        // Flatpak runtime extension
        "/usr/lib/extensions/openh264/extra/lib/libopenh264.so",
        // Windows
        #[cfg(target_os = "windows")]
        "openh264-2.5.0-win64.dll",
        // macOS
        #[cfg(target_os = "macos")]
        "/usr/local/lib/libopenh264.dylib",
    ];

    /// Configuration for the OpenH264 decoder
    ///
    /// Controls how and where the OpenH264 binary is loaded from.
    /// The binary must be Cisco's precompiled release for patent compliance.
    #[derive(Default)]
    pub struct OpenH264DecoderConfig {
        /// Explicit path to the OpenH264 shared library.
        /// When set, only this path is tried (no system search).
        pub library_path: Option<String>,
    }

    /// H.264 decoder backed by Cisco's OpenH264 library (loaded dynamically)
    ///
    /// This decoder converts AVC-format NAL units to Annex B format
    /// (as required by OpenH264), decodes to YUV420p, then converts
    /// to RGBA for the client pipeline.
    ///
    /// # Patent Compliance
    ///
    /// The OpenH264 library is loaded at runtime via `libloading`.
    /// For Cisco's patent coverage to apply, the loaded binary must be
    /// Cisco's precompiled release, downloaded separately to the device.
    /// See [`OPENH264_ATTRIBUTION`](super::OPENH264_ATTRIBUTION).
    pub struct OpenH264Decoder {
        decoder: openh264::decoder::Decoder,
        /// Resolved path to the loaded library (for reset/reload)
        loaded_path: Option<String>,
        annex_b_buffer: Vec<u8>,
    }

    impl OpenH264Decoder {
        /// Create a new OpenH264 decoder by loading Cisco's binary
        ///
        /// Searches well-known system paths for the OpenH264 shared library.
        /// Use [`with_config`](Self::with_config) for explicit path control.
        pub fn new() -> DecoderResult<Self> {
            Self::with_config(OpenH264DecoderConfig::default())
        }

        /// Create a new OpenH264 decoder with explicit configuration
        pub fn with_config(config: OpenH264DecoderConfig) -> DecoderResult<Self> {
            let (api, resolved_path) = if let Some(ref path) = config.library_path {
                let api = openh264::OpenH264API::from_blob_path(path)
                    .map_err(|e| DecoderError::new(format!("failed to load OpenH264 from {path}"), e))?;
                (api, Some(path.clone()))
            } else {
                Self::load_from_system_paths()?
            };

            let decoder = openh264::decoder::Decoder::with_api_config(api, Default::default())
                .map_err(|e| DecoderError::new("failed to initialize OpenH264 decoder", e))?;

            info!("OpenH264 Video Codec provided by Cisco Systems, Inc.");

            Ok(Self {
                decoder,
                loaded_path: resolved_path,
                annex_b_buffer: Vec::new(),
            })
        }

        /// Create a decoder using the compiled-in source (development/testing only)
        ///
        /// This bypasses the `libloading` path and uses OpenH264 compiled from
        /// source. Requires the `openh264-source` feature. Provides NO patent
        /// coverage from Cisco — use only for development and testing.
        #[cfg(feature = "openh264-source")]
        pub fn from_source() -> DecoderResult<Self> {
            let api = openh264::OpenH264API::from_source();
            let decoder = openh264::decoder::Decoder::with_api_config(api, Default::default())
                .map_err(|e| DecoderError::new("failed to initialize OpenH264 decoder from source", e))?;

            Ok(Self {
                decoder,
                loaded_path: None,
                annex_b_buffer: Vec::new(),
            })
        }

        /// Try loading from well-known system paths
        fn load_from_system_paths() -> DecoderResult<(openh264::OpenH264API, Option<String>)> {
            for path in SYSTEM_LIBRARY_PATHS {
                match openh264::OpenH264API::from_blob_path(path) {
                    Ok(api) => {
                        info!(%path, "Loaded OpenH264 library");
                        return Ok((api, Some((*path).to_owned())));
                    }
                    Err(_) => continue,
                }
            }

            Err(DecoderError::msg(
                "OpenH264 library not found in system paths. \
                 Install the Cisco OpenH264 binary for H.264 support \
                 (e.g., libopenh264-7 on Debian, openh264 on Fedora)",
            ))
        }

        /// Convert AVC format (4-byte BE length prefix) to Annex B (start codes)
        fn avc_to_annex_b(&mut self, data: &[u8]) {
            self.annex_b_buffer.clear();
            let mut offset = 0;

            while offset + 4 <= data.len() {
                let nal_len = u32::from_be_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]);

                #[expect(clippy::as_conversions, reason = "NAL length from wire format")]
                let nal_len = nal_len as usize;
                offset += 4;

                // Use checked addition to prevent overflow on malicious input
                let Some(end) = offset.checked_add(nal_len) else {
                    warn!(
                        offset,
                        nal_len, "AVC-to-Annex B: NAL length overflow, emitting partial output"
                    );
                    break;
                };
                if end > data.len() {
                    warn!(
                        offset,
                        nal_len,
                        data_len = data.len(),
                        "AVC-to-Annex B: NAL extends beyond buffer, emitting partial output"
                    );
                    break;
                }

                // Annex B start code
                self.annex_b_buffer.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                self.annex_b_buffer.extend_from_slice(&data[offset..offset + nal_len]);
                offset += nal_len;
            }
        }
    }

    impl H264Decoder for OpenH264Decoder {
        fn decode(&mut self, data: &[u8]) -> DecoderResult<DecodedFrame> {
            self.avc_to_annex_b(data);

            let yuv = self
                .decoder
                .decode(&self.annex_b_buffer)
                .map_err(|e| DecoderError::new("OpenH264 decode failed", e))?
                .ok_or_else(|| DecoderError::msg("OpenH264 returned no picture"))?;

            let (width, height) = openh264::formats::YUVSource::dimensions(&yuv);

            #[expect(
                clippy::as_conversions,
                clippy::cast_possible_truncation,
                reason = "H.264 frame dimensions are always within u32 range"
            )]
            let (w32, h32) = (width as u32, height as u32);

            let rgba_len = width
                .checked_mul(height)
                .and_then(|n| n.checked_mul(4))
                .ok_or_else(|| DecoderError::msg("RGBA allocation overflow: frame dimensions too large"))?;
            let mut rgba = vec![0u8; rgba_len];
            yuv.write_rgba8(&mut rgba);

            Ok(DecodedFrame {
                data: rgba,
                width: w32,
                height: h32,
            })
        }

        fn reset(&mut self) {
            // Reload the library and create a fresh decoder
            let api = if let Some(ref path) = self.loaded_path {
                match openh264::OpenH264API::from_blob_path(path) {
                    Ok(api) => api,
                    Err(e) => {
                        warn!("Failed to reload OpenH264 library on reset: {e}");
                        return;
                    }
                }
            } else {
                match Self::load_from_system_paths() {
                    Ok((api, _)) => api,
                    Err(e) => {
                        warn!("Failed to reload OpenH264 library on reset: {e}");
                        return;
                    }
                }
            };

            match openh264::decoder::Decoder::with_api_config(api, Default::default()) {
                Ok(new_decoder) => self.decoder = new_decoder,
                Err(e) => warn!("Failed to reset OpenH264 decoder, reusing existing state: {e}"),
            }
        }
    }
}

#[cfg(feature = "openh264")]
pub use openh264_impl::{OpenH264Decoder, OpenH264DecoderConfig};

// ============================================================================
// OpenH264 Tests (require source feature for encoder access)
// ============================================================================

#[cfg(all(test, feature = "openh264-source"))]
mod openh264_tests {
    use super::{H264Decoder, OpenH264Decoder};

    /// Generate a minimal AVC-format H.264 bitstream by encoding a black 16x16 frame
    ///
    /// Uses the `source` feature (compile-time encoder) to produce test data.
    /// This is only for testing — the `source` feature must not be used in
    /// shipped products (no patent coverage).
    fn generate_test_avc_bitstream() -> Vec<u8> {
        use openh264::encoder::Encoder;
        use openh264::formats::YUVBuffer;

        let mut encoder = Encoder::new().expect("encoder should initialize");

        // Black 16x16 YUV420p frame (all zeros)
        let yuv = YUVBuffer::new(16, 16);
        let bitstream = encoder.encode(&yuv).expect("encode should succeed");
        let annex_b = bitstream.to_vec();

        // Convert Annex B (0x00 0x00 0x00 0x01 | 0x00 0x00 0x01) to AVC (4-byte BE length prefix)
        annex_b_to_avc(&annex_b)
    }

    /// Convert Annex B format NAL units to AVC format (4-byte BE length prefix)
    fn annex_b_to_avc(data: &[u8]) -> Vec<u8> {
        let mut avc = Vec::new();
        let mut i = 0;

        // Find NAL unit boundaries by scanning for start codes
        let mut nal_starts = Vec::new();
        while i < data.len() {
            if i + 3 < data.len() && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
                nal_starts.push(i + 4);
                i += 4;
            } else if i + 2 < data.len() && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
                nal_starts.push(i + 3);
                i += 3;
            } else {
                i += 1;
            }
        }

        for (idx, &start) in nal_starts.iter().enumerate() {
            let end = if idx + 1 < nal_starts.len() {
                // Find the start code before the next NAL
                let next_start = nal_starts[idx + 1];
                // Back up past the start code prefix
                if next_start >= 4
                    && data[next_start - 4] == 0
                    && data[next_start - 3] == 0
                    && data[next_start - 2] == 0
                {
                    next_start - 4
                } else {
                    next_start - 3
                }
            } else {
                data.len()
            };

            let nal_data = &data[start..end];

            #[expect(clippy::as_conversions, reason = "NAL unit length for test data")]
            let len = nal_data.len() as u32;
            avc.extend_from_slice(&len.to_be_bytes());
            avc.extend_from_slice(nal_data);
        }

        avc
    }

    #[test]
    fn test_openh264_decoder_init() {
        let _decoder = OpenH264Decoder::from_source().expect("decoder should initialize");
    }

    #[test]
    fn test_openh264_decode_sps_pps() {
        // Generate a full bitstream (SPS + PPS + IDR) and verify decode succeeds
        // SPS and PPS are always delivered together with the first I-frame
        // in RFX_AVC420_BITMAP_STREAM payloads
        let avc_data = generate_test_avc_bitstream();
        assert!(!avc_data.is_empty(), "encoder should produce output");

        let mut decoder = OpenH264Decoder::from_source().expect("decoder should initialize");
        let frame = decoder.decode(&avc_data).expect("decode should succeed");
        assert!(frame.width >= 16, "decoded width should be at least 16");
        assert!(frame.height >= 16, "decoded height should be at least 16");
    }

    #[test]
    fn test_openh264_decode_iframe() {
        let avc_data = generate_test_avc_bitstream();

        let mut decoder = OpenH264Decoder::from_source().expect("decoder should initialize");
        let frame = decoder.decode(&avc_data).expect("decode should succeed");

        // Verify RGBA output dimensions and data
        assert_eq!(frame.width, 16);
        assert_eq!(frame.height, 16);
        assert_eq!(frame.data.len(), 16 * 16 * 4, "RGBA data should be 16x16x4 bytes");
    }

    #[test]
    fn test_openh264_decoder_reset() {
        let mut decoder = OpenH264Decoder::from_source().expect("decoder should initialize");

        // Decode a frame to populate internal state
        let avc_data = generate_test_avc_bitstream();
        let _ = decoder.decode(&avc_data);

        // Reset should not panic
        decoder.reset();

        // Decoder should still be usable after reset
        let frame = decoder.decode(&avc_data).expect("decode after reset should succeed");
        assert_eq!(frame.width, 16);
        assert_eq!(frame.height, 16);
    }
}
