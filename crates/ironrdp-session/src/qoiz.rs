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
    pub(crate) fn new() -> Self {
        Self(backend::Decompressor::default())
    }

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
    use std::io::Read as _;
    use std::sync::{Arc, Mutex};

    pub(super) const NAME: &str = "zrip";

    /// Size of the chunks pulled out of the decoder at a time.
    const READ_CHUNK: usize = 16 * 1024;

    #[derive(Debug, Default)]
    struct SourceBuf {
        data: Vec<u8>,
        pos: usize,
    }

    /// Source feeding one update at a time to the decoder.
    #[derive(Clone, Debug, Default)]
    struct SharedSource(Arc<Mutex<SourceBuf>>);

    impl SharedSource {
        fn push(&self, data: &[u8]) -> SessionResult<()> {
            let mut source = self.0.lock().map_err(|_| reason_err!(NAME, "poisoned source"))?;

            // Drop consumed bytes. It is a no-op when the previous update was fully read.
            let consumed = source.pos;
            source.data.drain(..consumed);
            source.pos = 0;
            source.data.extend_from_slice(data);

            Ok(())
        }
    }

    impl std::io::Read for SharedSource {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let mut source = self.0.lock().map_err(|_| std::io::Error::other("poisoned source"))?;

            let rest = &source.data[source.pos..];
            let len = buf.len().min(rest.len());
            buf[..len].copy_from_slice(&rest[..len]);
            source.pos += len;

            Ok(len)
        }
    }

    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(super) struct Decompressor {
        // The `FrameDecoder` structure is large, so it is boxed to keep this type small.
        decoder: Box<zrip::FrameDecoder<SharedSource>>,
        source: SharedSource,
    }

    impl Decompressor {
        #[cfg_attr(feature = "__test", visibility::make(pub))]
        pub(super) fn decompress(&mut self, input: &[u8]) -> SessionResult<Vec<u8>> {
            self.source.push(input)?;

            let mut data = Vec::new();
            let mut buf = [0; READ_CHUNK];

            loop {
                match self.decoder.read(&mut buf) {
                    Ok(0) => break,
                    Ok(len) => data.extend_from_slice(&buf[..len]),
                    // The server flushes after every update, so an update always
                    // ends on a block boundary. EOF means that this update is fully
                    // decoded, and the decoder picks the stream back up when the next one arrives.
                    Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(error) => return Err(reason_err!(NAME, "{}", error)),
                }
            }

            Ok(data)
        }
    }

    impl Default for Decompressor {
        fn default() -> Self {
            let source = SharedSource::default();

            Self {
                decoder: Box::new(zrip::FrameDecoder::new(source.clone())),
                source,
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
            let mut data = vec![0; input.len() * 4];
            let mut pos = 0;

            loop {
                let mut output = zstd_safe::OutBuffer::around_pos(data.as_mut_slice(), pos);
                self.zdctx
                    .decompress_stream(&mut output, &mut source)
                    .map_err(zstd_safe::get_error_name)
                    .map_err(|error| reason_err!(NAME, "{}", error))?;
                pos = output.pos();
                if pos == output.capacity() {
                    data.resize(data.capacity() * 2, 0);
                } else {
                    break;
                }
            }

            data.truncate(pos);

            Ok(data)
        }
    }
}
