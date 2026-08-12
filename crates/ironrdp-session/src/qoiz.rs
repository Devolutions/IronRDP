//! Zstd decompression backend for the QOIZ bitmap codec.
//!
//! Two backends are available:
//!
//! - `zrip`, a pure Rust zstd implementation, used by default.
//! - `zstd-safe`, bindings to the reference C implementation, used when the
//!   `qoiz-zstd` feature is enabled.
//!
//! Both consume standard zstd frames, so the choice of backend is invisible on
//! the wire and either side of a connection may use either one.

use crate::SessionResult;

#[cfg(not(feature = "qoiz-zstd"))]
use zrip_backend as backend;
#[cfg(feature = "qoiz-zstd")]
use zstd_safe_backend as backend;

/// Streaming decompressor for the QOIZ codec.
///
/// Must be fed every update of a session, in order: an update is decoded
/// against the history of the ones before it.
#[derive(Default)]
pub(crate) struct Decompressor(backend::Decompressor);

impl Decompressor {
    /// Decompresses the payload of a single surface command.
    pub(crate) fn decompress(&mut self, input: &[u8]) -> SessionResult<Vec<u8>> {
        self.0.decompress(input)
    }
}

impl core::fmt::Debug for Decompressor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Decompressor").field("backend", &backend::NAME).finish()
    }
}

#[cfg_attr(all(feature = "qoiz-zstd", not(feature = "__test")), expect(dead_code))]
#[cfg_attr(feature = "__test", visibility::make(pub(crate)))]
mod zrip_backend {
    use crate::{SessionResult, reason_err};

    pub(super) const NAME: &str = "zrip";

    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(super) struct Decompressor {
        decoder: zrip::push::PushDecoder,
    }

    impl Decompressor {
        #[cfg_attr(feature = "__test", visibility::make(pub))]
        pub(super) fn decompress(&mut self, input: &[u8]) -> SessionResult<Vec<u8>> {
            let mut data = Vec::with_capacity(input.len().saturating_mul(4));

            self.decoder
                .decode_to_vec(input, &mut data)
                .map_err(|error| reason_err!(NAME, "{}", error))?;

            Ok(data)
        }
    }

    impl Default for Decompressor {
        fn default() -> Self {
            Self {
                decoder: zrip::push::PushDecoder::new(),
            }
        }
    }
}

#[cfg(feature = "qoiz-zstd")]
#[cfg_attr(feature = "__test", visibility::make(pub(crate)))]
mod zstd_safe_backend {
    use crate::{SessionResult, reason_err};

    pub(super) const NAME: &str = "zstd-safe";

    #[cfg_attr(feature = "__test", visibility::make(pub))]
    #[derive(Default)]
    pub(super) struct Decompressor {
        zdctx: zstd_safe::DCtx<'static>,
    }

    impl Decompressor {
        #[cfg_attr(feature = "__test", visibility::make(pub))]
        pub(super) fn decompress(&mut self, input: &[u8]) -> SessionResult<Vec<u8>> {
            let mut source = zstd_safe::InBuffer::around(input);
            // Never start empty, or the buffer could not grow and the loop would never end.
            let mut data = vec![0; input.len().max(1) * 4];
            let mut pos = 0;

            loop {
                let mut output = zstd_safe::OutBuffer::around_pos(data.as_mut_slice(), pos);
                self.zdctx
                    .decompress_stream(&mut output, &mut source)
                    .map_err(zstd_safe::get_error_name)
                    .map_err(|error| reason_err!(NAME, "{}", error))?;
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
