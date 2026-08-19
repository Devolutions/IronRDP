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

use anyhow::Result;

/// Streaming compressor for the QOIZ codec.
pub(crate) struct Compressor(Box<imp::Compressor>);

impl Compressor {
    pub(crate) fn new() -> Result<Self> {
        imp::Compressor::new().map(|imp| Self(Box::new(imp)))
    }

    /// Compresses `input` and returns the bytes produced for this update.
    pub(crate) fn compress(&mut self, input: &[u8]) -> Result<Vec<u8>> {
        self.0.compress(input)
    }
}

impl core::fmt::Debug for Compressor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Compressor").field("backend", &imp::NAME).finish()
    }
}

#[cfg(not(feature = "qoiz-zstd"))]
mod imp {
    use std::io::Write as _;
    use std::sync::{Arc, Mutex};

    use anyhow::{Context as _, Result, anyhow};

    pub(super) const NAME: &str = "zrip";

    /// Compression level, on zrip's `-8..=4` scale.
    const COMPRESSION_LEVEL: i32 = 1;

    /// Sink handing the encoder's output back to the caller.
    ///
    /// [`zrip::FrameEncoder`] owns its writer and exposes no accessor for it, so
    /// the buffer is shared rather than borrowed back.
    #[derive(Clone, Debug, Default)]
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

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

    pub(super) struct Compressor {
        encoder: zrip::FrameEncoder<SharedSink>,
        sink: SharedSink,
    }

    impl Compressor {
        pub(super) fn new() -> Result<Self> {
            // Long distance matching is what lets an update match against
            // earlier ones. Without it, repeated content is re-emitted in full.
            let options = zrip::Options::default().ldm(true);
            let sink = SharedSink::default();
            let encoder = zrip::FrameEncoder::with_options(sink.clone(), COMPRESSION_LEVEL, &options)
                .map_err(|e| anyhow!("failed to create zstd encoder: {e}"))?;

            Ok(Self { encoder, sink })
        }

        pub(super) fn compress(&mut self, input: &[u8]) -> Result<Vec<u8>> {
            self.encoder.write_all(input).context("failed to zstd compress")?;
            self.encoder.flush().context("failed to zstd compress")?;

            let mut sink = self
                .sink
                .0
                .lock()
                .map_err(|_| anyhow!("failed to zstd compress: poisoned sink"))?;

            Ok(core::mem::take(&mut *sink))
        }
    }
}

#[cfg(feature = "qoiz-zstd")]
mod imp {
    // Cargo features are additive, so `qoiz` keeps pulling zrip in even when
    // this backend is the one selected. Nothing links it in that case.
    use zrip as _;

    use anyhow::{Result, anyhow};

    pub(super) const NAME: &str = "zstd-safe";

    const COMPRESSION_LEVEL: i32 = 3;

    pub(super) struct Compressor {
        zctxt: zstd_safe::CCtx<'static>,
    }

    impl Compressor {
        pub(super) fn new() -> Result<Self> {
            let mut zctxt = zstd_safe::CCtx::default();

            zctxt
                .set_parameter(zstd_safe::CParameter::CompressionLevel(COMPRESSION_LEVEL))
                .map_err(|code| {
                    anyhow!(
                        "failed to set zstd compression level: {}",
                        zstd_safe::get_error_name(code)
                    )
                })?;
            zctxt
                .set_parameter(zstd_safe::CParameter::EnableLongDistanceMatching(true))
                .map_err(|code| {
                    anyhow!(
                        "failed to set zstd enable long distance matching: {}",
                        zstd_safe::get_error_name(code)
                    )
                })?;

            Ok(Self { zctxt })
        }

        pub(super) fn compress(&mut self, input: &[u8]) -> Result<Vec<u8>> {
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
                    .map_err(|code| anyhow!("failed to Zstd compress: {}", zstd_safe::get_error_name(code)))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Magic bytes every zstd frame starts with.
    const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

    fn payload(i: u8) -> Vec<u8> {
        // `j / 17` stays under 121, so the conversion never truncates.
        (0..2048u32)
            .map(|j| u8::try_from(j / 17).expect("fits in u8").wrapping_add(i))
            .collect()
    }

    #[test]
    fn produces_a_zstd_frame() {
        let mut compressor = Compressor::new().unwrap();

        let compressed = compressor.compress(&payload(0)).unwrap();

        assert_eq!(compressed.get(..4), Some(ZSTD_MAGIC.as_slice()));
    }

    /// Locks in long-distance-matching (LDM) behavior.
    #[test]
    fn reuses_history_across_updates() {
        let mut compressor = Compressor::new().unwrap();

        let first = compressor.compress(&payload(0)).unwrap();
        compressor.compress(&payload(1)).unwrap();
        let repeat = compressor.compress(&payload(0)).unwrap();

        assert!(
            repeat.len() * 4 < first.len(),
            "repeated update did not match against history: {} bytes vs {} bytes",
            repeat.len(),
            first.len()
        );
    }

    #[test]
    fn flushes_every_update() {
        let mut compressor = Compressor::new().unwrap();

        for i in 0..8 {
            let compressed = compressor.compress(&payload(i)).unwrap();

            assert!(!compressed.is_empty(), "update {i} produced no output");
        }
    }
}
