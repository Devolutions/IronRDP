//! H.264 decoders shared by the graphics pipeline ([MS-RDPEGFX]) and video optimized remoting
//! ([MS-RDPEVOR]).
//!
//! [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/
//! [MS-RDPEVOR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/

use core::fmt;
use std::sync::Arc;

use ironrdp_egfx::decode::H264Decoder;
#[cfg(any(feature = "openh264", windows))]
use tracing::warn;

/// Creates an H.264 decoder for the graphics pipeline and for each video presentation.
///
/// The graphics pipeline takes YUV planes from decoders that return them
/// ([`H264Decoder::supports_yuv420`]) and converts them with its own color transform, which
/// AVC444 requires, so a decoder converting RGBA with the MS-RDPEVOR matrix serves both.
#[derive(Clone)]
pub struct H264DecoderFactory(Arc<dyn Fn() -> Option<Box<dyn H264Decoder>> + Send + Sync>);

impl H264DecoderFactory {
    pub fn new(factory: impl Fn() -> Option<Box<dyn H264Decoder>> + Send + Sync + 'static) -> Self {
        Self(Arc::new(factory))
    }

    /// The default decoder: OpenH264 with the `openh264` feature, otherwise Media Foundation's on
    /// Windows, and none elsewhere.
    pub fn platform_default() -> Option<Self> {
        #[cfg(feature = "openh264")]
        return Some(Self::openh264());
        #[cfg(all(windows, not(feature = "openh264")))]
        return Some(Self::media_foundation());
        #[cfg(not(any(feature = "openh264", windows)))]
        return None;
    }

    /// Software decoder built from the bundled OpenH264 sources.
    #[cfg(feature = "openh264")]
    pub fn openh264() -> Self {
        use ironrdp_egfx::decode::{OpenH264Decoder, YuvMatrix};

        // MS-RDPEVOR video is converted from RGB with BT.601 (2.2.1.6).
        Self::new(|| match OpenH264Decoder::new() {
            Ok(decoder) => {
                let decoder: Box<dyn H264Decoder> = Box::new(decoder.with_yuv_matrix(YuvMatrix::Bt601Limited));
                Some(decoder)
            }
            Err(error) => {
                warn!(%error, "Unable to create the OpenH264 decoder");
                None
            }
        })
    }

    /// Decoder transform shipped with Windows, through Media Foundation.
    #[cfg(windows)]
    pub fn media_foundation() -> Self {
        use ironrdp_egfx_native::{MediaFoundationH264Decoder, YuvMatrix};

        // MS-RDPEVOR video is converted from RGB with BT.601 (2.2.1.6).
        Self::new(|| match MediaFoundationH264Decoder::new(YuvMatrix::Bt601Limited) {
            Ok(decoder) => {
                let decoder: Box<dyn H264Decoder> = Box::new(decoder);
                Some(decoder)
            }
            Err(error) => {
                warn!(%error, "Unable to create the Media Foundation H.264 decoder");
                None
            }
        })
    }

    /// Creates a decoder, or `None` when the backend is unavailable.
    pub fn create(&self) -> Option<Box<dyn H264Decoder>> {
        (self.0)()
    }
}

impl fmt::Debug for H264DecoderFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("H264DecoderFactory")
    }
}
