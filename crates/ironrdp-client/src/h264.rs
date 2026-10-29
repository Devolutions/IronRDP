//! H.264 decoders shared by the graphics pipeline ([MS-RDPEGFX]) and video optimized remoting
//! ([MS-RDPEVOR]).
//!
//! [MS-RDPEGFX]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/
//! [MS-RDPEVOR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpevor/

use core::fmt;
use core::time::Duration;
use std::sync::{Arc, mpsc};

use ironrdp_egfx::decode::{DecodedFrame, DecoderError, DecoderResult, H264Decoder, Yuv420Frame};
use tracing::warn;

/// Longest time [`H264Decoder::wait_until_ready`] waits for a decoder started in the background.
///
/// Longer than the Media Foundation decoder's own start limit, so it only bounds factories
/// that never return.
const BACKGROUND_START_TIMEOUT: Duration = Duration::from_secs(15);

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
        Self::new(|| {
            create_or_warn("OpenH264", || {
                OpenH264Decoder::new().map(|decoder| decoder.with_yuv_matrix(YuvMatrix::Bt601Limited))
            })
        })
    }

    /// Decoder transform shipped with Windows, through Media Foundation.
    #[cfg(windows)]
    pub fn media_foundation() -> Self {
        use ironrdp_egfx_native::{MediaFoundationH264Decoder, YuvMatrix};

        // MS-RDPEVOR video is converted from RGB with BT.601 (2.2.1.6).
        Self::new(|| {
            create_or_warn("Media Foundation", || {
                MediaFoundationH264Decoder::new(YuvMatrix::Bt601Limited)
            })
        })
    }

    /// Creates a decoder, or `None` when the backend is unavailable.
    pub fn create(&self) -> Option<Box<dyn H264Decoder>> {
        (self.0)()
    }

    /// Starts creating a decoder on another thread and returns at once.
    ///
    /// The returned decoder finishes starting in [`H264Decoder::wait_until_ready`], which
    /// reports `false` when the backend is unavailable, so the graphics pipeline can create it
    /// before connecting without delaying the connection.
    pub fn create_in_background(&self) -> Box<dyn H264Decoder> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let factory = self.clone();
        let started = std::thread::Builder::new()
            .name("h264-decoder-start".to_owned())
            .spawn(move || {
                let _ = sender.send(factory.create());
            });
        let state = match started {
            Ok(_) => BackgroundState::Starting(receiver),
            Err(error) => {
                warn!(%error, "Unable to start creating the H.264 decoder");
                BackgroundState::Unavailable
            }
        };
        Box::new(BackgroundDecoder { state })
    }
}

/// A decoder another thread is still creating.
struct BackgroundDecoder {
    state: BackgroundState,
}

enum BackgroundState {
    Starting(mpsc::Receiver<Option<Box<dyn H264Decoder>>>),
    Ready(Box<dyn H264Decoder>),
    Unavailable,
}

impl BackgroundDecoder {
    fn ready(&mut self) -> DecoderResult<&mut Box<dyn H264Decoder>> {
        match &mut self.state {
            BackgroundState::Ready(decoder) => Ok(decoder),
            _ => Err(DecoderError::msg("the H.264 decoder is not ready")),
        }
    }
}

impl H264Decoder for BackgroundDecoder {
    fn decode(&mut self, data: &[u8]) -> DecoderResult<DecodedFrame> {
        self.ready()?.decode(data)
    }

    fn decode_yuv420(&mut self, data: &[u8]) -> DecoderResult<Yuv420Frame> {
        self.ready()?.decode_yuv420(data)
    }

    fn supports_yuv420(&self) -> bool {
        matches!(&self.state, BackgroundState::Ready(decoder) if decoder.supports_yuv420())
    }

    fn wait_until_ready(&mut self) -> bool {
        if let BackgroundState::Starting(receiver) = &self.state {
            self.state = match receiver.recv_timeout(BACKGROUND_START_TIMEOUT) {
                Ok(Some(mut decoder)) => {
                    if decoder.wait_until_ready() {
                        BackgroundState::Ready(decoder)
                    } else {
                        BackgroundState::Unavailable
                    }
                }
                Ok(_) => BackgroundState::Unavailable,
                Err(_) => {
                    warn!("The H.264 decoder did not start in time");
                    BackgroundState::Unavailable
                }
            };
        }
        matches!(self.state, BackgroundState::Ready(_))
    }

    fn reset(&mut self) {
        if let BackgroundState::Ready(decoder) = &mut self.state {
            decoder.reset();
        }
    }
}

/// Boxes the decoder `create` returns, or logs why `backend` is unavailable.
#[cfg(any(feature = "openh264", windows))]
fn create_or_warn<D: H264Decoder + 'static>(
    backend: &str,
    create: impl FnOnce() -> DecoderResult<D>,
) -> Option<Box<dyn H264Decoder>> {
    match create() {
        Ok(decoder) => Some(Box::new(decoder)),
        Err(error) => {
            warn!(%error, backend, "Unable to create the H.264 decoder");
            None
        }
    }
}

impl fmt::Debug for H264DecoderFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("H264DecoderFactory")
    }
}
