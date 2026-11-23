//! Zstd decompression backend for the QOIZ bitmap codec.
//!
//! Two backends are available:
//!
//! - `zstd-safe`, bindings to the reference C implementation, enabled by the
//!   `qoiz-zstd` feature (and by `qoiz`). It takes precedence when both
//!   backends are enabled.
//! - `zrip`, a pure Rust zstd implementation, enabled by the `qoiz-zrip` feature.
//!
//! Both consume standard zstd frames, so the choice of backend is invisible on
//! the wire and either side of a connection may use either one.

#[cfg(not(feature = "qoiz-zstd"))]
use zrip_backend as backend;
#[cfg(feature = "qoiz-zstd")]
use zstd_safe_backend as backend;

/// Streaming decompressor for the QOIZ codec.
///
/// Must be fed every update of a session, in order: an update is decoded
/// against the history of the ones before it.
pub(crate) type Decompressor = backend::Decompressor;

#[cfg(feature = "qoiz-zrip")]
#[cfg_attr(
    all(feature = "qoiz-zstd", not(feature = "__test")),
    expect(dead_code, reason = "`zstd-safe` takes precedence when both backends are enabled")
)]
#[cfg_attr(feature = "__test", visibility::make(pub(crate)))]
mod zrip_backend {
    use crate::{SessionResult, reason_err};

    #[cfg_attr(feature = "__test", visibility::make(pub))]
    #[derive(Default)]
    pub(crate) struct Decompressor(Box<zrip::push::PushDecoder>);

    impl Decompressor {
        /// Decompresses the payload of a single surface command.
        #[cfg_attr(feature = "__test", visibility::make(pub))]
        pub(crate) fn decompress(&mut self, input: &[u8]) -> SessionResult<Vec<u8>> {
            let mut data = Vec::with_capacity(input.len().saturating_mul(4));

            self.0
                .decode_to_vec(input, &mut data)
                .map_err(|error| reason_err!("qoiz decompress", "zrip: failed to decompress: {}", error))?;

            Ok(data)
        }
    }
}

#[cfg(feature = "qoiz-zstd")]
#[cfg_attr(feature = "__test", visibility::make(pub(crate)))]
mod zstd_safe_backend {
    use crate::{SessionResult, reason_err};

    #[cfg_attr(feature = "__test", visibility::make(pub))]
    #[derive(Default)]
    pub(crate) struct Decompressor {
        zdctx: zstd_safe::DCtx<'static>,
    }

    impl Decompressor {
        /// Decompresses the payload of a single surface command.
        #[cfg_attr(feature = "__test", visibility::make(pub))]
        pub(crate) fn decompress(&mut self, input: &[u8]) -> SessionResult<Vec<u8>> {
            let mut source = zstd_safe::InBuffer::around(input);
            // Never start empty, or the buffer could not grow and the loop would never end.
            let mut data = vec![0; input.len().max(1) * 4];
            let mut pos = 0;

            loop {
                let mut output = zstd_safe::OutBuffer::around_pos(data.as_mut_slice(), pos);
                self.zdctx
                    .decompress_stream(&mut output, &mut source)
                    .map_err(zstd_safe::get_error_name)
                    .map_err(|error| reason_err!("qoiz decompress", "zstd-safe: failed to decompress: {}", error))?;
                pos = output.pos();
                if pos == output.capacity() {
                    data.resize(data.len() * 2, 0);
                } else {
                    break;
                }
            }

            data.truncate(pos);

            Ok(data)
        }
    }
}
