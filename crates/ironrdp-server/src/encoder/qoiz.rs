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

const COMPRESSION_LEVEL: i32 = 3;

#[cfg(not(feature = "qoiz-zstd"))]
use zrip_backend as backend;
#[cfg(feature = "qoiz-zstd")]
use zstd_safe_backend as backend;

/// Streaming compressor for the QOIZ codec.
pub(crate) type Compressor = backend::Compressor;

#[cfg(any(not(feature = "qoiz-zstd"), feature = "__test"))]
#[cfg_attr(feature = "__test", visibility::make(pub(crate)))]
mod zrip_backend {
    use crate::encoder::qoiz::COMPRESSION_LEVEL;
    use crate::{ServerError, ServerErrorExt as _, ServerResult};
    use zrip::push::{EndOp, PushEncoder};

    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) struct Compressor(Box<PushEncoder>);

    impl Compressor {
        #[cfg_attr(feature = "__test", visibility::make(pub))]
        pub(crate) fn new() -> ServerResult<Self> {
            let options = zrip::Options::default().ldm(true);
            let encoder = Box::new(PushEncoder::with_options(COMPRESSION_LEVEL, &options).map_err(|error| {
                ServerError::reason("qoiz init", format!("zrip: failed to create an encoder: {error}"))
            })?);

            Ok(Self(encoder))
        }

        #[cfg_attr(feature = "__test", visibility::make(pub))]
        /// Compresses `input` and returns the bytes produced for this update.
        pub(crate) fn compress(&mut self, input: &[u8]) -> ServerResult<Vec<u8>> {
            let mut output = Vec::new();

            self.0
                .encode_to_vec(input, &mut output, EndOp::Flush)
                .map_err(|error| ServerError::reason("qoiz compress", format!("zrip: failed to compress: {error}")))?;

            Ok(output)
        }
    }
}

#[cfg(feature = "qoiz-zstd")]
#[cfg_attr(feature = "__test", visibility::make(pub(crate)))]
mod zstd_safe_backend {
    use crate::encoder::qoiz::COMPRESSION_LEVEL;
    use crate::{ServerError, ServerErrorExt as _, ServerResult};

    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) struct Compressor(zstd_safe::CCtx<'static>);

    impl Compressor {
        #[cfg_attr(feature = "__test", visibility::make(pub))]
        pub(crate) fn new() -> ServerResult<Self> {
            let mut zctxt = zstd_safe::CCtx::default();

            zctxt
                .set_parameter(zstd_safe::CParameter::CompressionLevel(COMPRESSION_LEVEL))
                .map_err(|code| {
                    ServerError::reason(
                        "qoiz init",
                        format!(
                            "zstd-safe: failed to set compression level: {}",
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
                            "zstd-safe: failed to enable long distance matching: {}",
                            zstd_safe::get_error_name(code)
                        ),
                    )
                })?;

            Ok(Self(zctxt))
        }

        #[cfg_attr(feature = "__test", visibility::make(pub))]
        /// Compresses `input` and returns the bytes produced for this update.
        pub(crate) fn compress(&mut self, input: &[u8]) -> ServerResult<Vec<u8>> {
            let mut inb = zstd_safe::InBuffer::around(input);
            let mut data = vec![0; input.len()];
            let mut outb;
            let mut pos = 0;

            loop {
                outb = zstd_safe::OutBuffer::around_pos(data.as_mut_slice(), pos);
                let res = self
                    .0
                    .compress_stream2(
                        &mut outb,
                        &mut inb,
                        zstd_safe::zstd_sys::ZSTD_EndDirective::ZSTD_e_flush,
                    )
                    .map_err(|code| {
                        ServerError::reason(
                            "qoiz compress",
                            format!("zstd-safe: failed to compress: {}", zstd_safe::get_error_name(code)),
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
