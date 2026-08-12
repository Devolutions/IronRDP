//! Zstd compression for the QOIZ bitmap codec.
//!
//! Two backends are available:
//!
//! - `zrip`, a pure Rust zstd implementation, used by default.
//! - `zstd-safe`, bindings to the reference C implementation, used when the
//!   `qoiz-zstd` feature is enabled.
//!
//! Both emit standard zstd frames, so the choice of backend is invisible on the
//! wire and either side of a connection may use either one.

use crate::ServerResult;

/// The `zstd-safe` and `zrip` backends use nearly identical parameter sets at compression level 3.
const COMPRESSION_LEVEL: i32 = 3;

#[cfg(not(feature = "qoiz-zstd"))]
use zrip_backend as backend;
#[cfg(feature = "qoiz-zstd")]
use zstd_safe_backend as backend;

/// Streaming compressor for the QOIZ codec.
pub(crate) struct Compressor(backend::Compressor);

impl Compressor {
    pub(crate) fn new() -> ServerResult<Self> {
        Ok(Self(backend::Compressor::new()?))
    }

    /// Compresses `input` and returns the bytes produced for this update.
    pub(crate) fn compress(&mut self, input: &[u8]) -> ServerResult<Vec<u8>> {
        self.0.compress(input)
    }
}

impl core::fmt::Debug for Compressor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Compressor").field("backend", &backend::NAME).finish()
    }
}

// Compiled even when the `qoiz-zstd` feature is enabled, so tests can use both backends.
#[cfg_attr(all(feature = "qoiz-zstd", not(feature = "__test")), expect(dead_code))]
#[cfg_attr(feature = "__test", visibility::make(pub(crate)))]
mod zrip_backend {
    use crate::encoder::qoiz::COMPRESSION_LEVEL;
    use crate::{ServerError, ServerErrorExt as _, ServerResult};
    use std::io::Write as _;
    use std::sync::{Arc, Mutex};

    // Used by the `Debug` impl only when zrip is the active backend.
    #[cfg(not(feature = "qoiz-zstd"))]
    pub(super) const NAME: &str = "zrip";

    /// Sink handing the encoder's output back to the caller.
    ///
    /// [`zrip::FrameEncoder`] owns its writer and exposes no accessor for it, so
    /// the buffer is shared rather than borrowed back.
    #[derive(Clone, Debug, Default)]
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl SharedSink {
        fn take(&self) -> ServerResult<Vec<u8>> {
            let mut sink = self
                .0
                .lock()
                .map_err(|_| ServerError::reason("qoiz compress", "poisoned sink"))?;

            Ok(core::mem::take(&mut *sink))
        }
    }

    impl std::io::Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let mut sink = self.0.lock().map_err(|_| std::io::Error::other("poisoned sink"))?;
            sink.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(super) struct Compressor {
        // The `FrameEncoder` structure is large, so it is boxed to keep this type small.
        encoder: Box<zrip::FrameEncoder<SharedSink>>,
        sink: SharedSink,
    }

    impl Compressor {
        #[cfg_attr(feature = "__test", visibility::make(pub))]
        pub(super) fn new() -> ServerResult<Self> {
            // Long distance matching is what lets an update match against
            // earlier ones. Without it, repeated content is re-emitted in full.
            let options = zrip::Options::default().ldm(true);
            let sink = SharedSink::default();
            let encoder = Box::new(
                zrip::FrameEncoder::with_options(sink.clone(), COMPRESSION_LEVEL, &options).map_err(|error| {
                    ServerError::reason("qoiz init", format!("failed to create an encoder: {error}"))
                })?,
            );

            Ok(Self { encoder, sink })
        }

        #[cfg_attr(feature = "__test", visibility::make(pub))]
        pub(super) fn compress(&mut self, input: &[u8]) -> ServerResult<Vec<u8>> {
            self.encoder.write_all(input).map_err(|error| {
                ServerError::reason(
                    "qoiz compress",
                    format!("failed to write input data to the encoder: {error}"),
                )
            })?;
            self.encoder.flush().map_err(|error| {
                ServerError::reason("qoiz compress", format!("failed to flush the encoder: {error}"))
            })?;

            self.sink.take()
        }
    }
}

#[cfg(feature = "qoiz-zstd")]
#[cfg_attr(feature = "__test", visibility::make(pub(crate)))]
mod zstd_safe_backend {
    use crate::encoder::qoiz::COMPRESSION_LEVEL;
    use crate::{ServerError, ServerErrorExt as _, ServerResult};

    pub(super) const NAME: &str = "zstd-safe";

    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(super) struct Compressor {
        zctxt: zstd_safe::CCtx<'static>,
    }

    impl Compressor {
        #[cfg_attr(feature = "__test", visibility::make(pub))]
        pub(super) fn new() -> ServerResult<Self> {
            let mut zctxt = zstd_safe::CCtx::default();

            zctxt
                .set_parameter(zstd_safe::CParameter::CompressionLevel(COMPRESSION_LEVEL))
                .map_err(|code| {
                    ServerError::reason(
                        "qoiz init",
                        format!(
                            "failed to set zstd compression level: {}",
                            zstd_safe::get_error_name(code)
                        ),
                    )
                })?;
            zctxt
                .set_parameter(zstd_safe::CParameter::EnableLongDistanceMatching(true))
                .map_err(|code| {
                    ServerError::reason(
                        "qoiz init",
                        format!(
                            "failed to set zstd enable long distance matching: {}",
                            zstd_safe::get_error_name(code)
                        ),
                    )
                })?;

            Ok(Self { zctxt })
        }

        #[cfg_attr(feature = "__test", visibility::make(pub))]
        pub(super) fn compress(&mut self, input: &[u8]) -> ServerResult<Vec<u8>> {
            let mut inb = zstd_safe::InBuffer::around(input);
            let mut data = vec![0; input.len()];
            let mut outb;
            let mut pos = 0;

            loop {
                outb = zstd_safe::OutBuffer::around_pos(data.as_mut_slice(), pos);
                let res = self
                    .zctxt
                    .compress_stream2(
                        &mut outb,
                        &mut inb,
                        zstd_safe::zstd_sys::ZSTD_EndDirective::ZSTD_e_flush,
                    )
                    .map_err(|code| {
                        ServerError::reason(
                            "qoiz compress",
                            format!("failed to compress: {}", zstd_safe::get_error_name(code)),
                        )
                    })?;
                if res == 0 {
                    break;
                }
                pos = outb.pos();
                data.resize(data.len() + res, 0);
            }

            let len = outb.pos();
            data.truncate(len);

            Ok(data)
        }
    }
}
