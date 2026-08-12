//! Zstd decompression backend for the QOIZ bitmap codec.
//!
//! Two backends are available:
//!
//! - [`zrip`], a pure Rust zstd implementation, used by default.
//! - `zstd-safe`, bindings to the reference C implementation, used when the
//!   `qoiz-zstd` feature is enabled.
//!
//! Both consume standard zstd frames, so the choice of backend is invisible on
//! the wire and either side of a connection may use either one.

/// Streaming decompressor for the QOIZ codec.
///
/// Must be fed every update of a session, in order: an update is decoded
/// against the history of the ones before it.
pub(crate) struct Decompressor(Box<imp::Decompressor>);

impl Decompressor {
    pub(crate) fn new() -> Self {
        Self(Box::new(imp::Decompressor::new()))
    }

    /// Decompresses the payload of a single surface command.
    pub(crate) fn decompress(&mut self, input: &[u8]) -> std::io::Result<Vec<u8>> {
        self.0.decompress(input)
    }
}

impl core::fmt::Debug for Decompressor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Decompressor").field("backend", &imp::NAME).finish()
    }
}

#[cfg(not(feature = "qoiz-zstd"))]
mod imp {
    use std::io::Read as _;
    use std::sync::{Arc, Mutex};

    pub(super) const NAME: &str = "zrip";

    /// Size of the chunks pulled out of the decoder at a time.
    const READ_CHUNK: usize = 16 * 1024;

    /// Source feeding one update at a time to the decoder.
    #[derive(Clone, Debug, Default)]
    struct SharedSource(Arc<Mutex<Vec<u8>>>);

    impl std::io::Read for SharedSource {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let mut source = self.0.lock().map_err(|_| std::io::Error::other("poisoned source"))?;
            let len = buf.len().min(source.len());
            buf[..len].copy_from_slice(&source[..len]);
            source.drain(..len);
            Ok(len)
        }
    }

    pub(super) struct Decompressor {
        decoder: zrip::FrameDecoder<SharedSource>,
        source: SharedSource,
    }

    impl Decompressor {
        pub(super) fn new() -> Self {
            let source = SharedSource::default();

            Self {
                decoder: zrip::FrameDecoder::new(source.clone()),
                source,
            }
        }

        pub(super) fn decompress(&mut self, input: &[u8]) -> std::io::Result<Vec<u8>> {
            {
                let mut source = self
                    .source
                    .0
                    .lock()
                    .map_err(|_| std::io::Error::other("poisoned source"))?;
                source.extend_from_slice(input);
            }

            let mut data = Vec::new();
            let mut buf = [0; READ_CHUNK];

            loop {
                match self.decoder.read(&mut buf) {
                    Ok(0) => break,
                    Ok(len) => data.extend_from_slice(&buf[..len]),
                    // The server flushes after every update, so an update always
                    // ends on a block boundary. EOF means that this update is fully
                    // decoded, and the decoder picks the stream back up when the next one arrives.
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(e) => return Err(e),
                }
            }

            Ok(data)
        }
    }
}

#[cfg(feature = "qoiz-zstd")]
mod imp {
    // Cargo features are additive, so `qoiz` keeps pulling zrip in even when
    // this backend is the one selected. Nothing links it in that case.
    use zrip as _;

    pub(super) const NAME: &str = "zstd-safe";

    pub(super) struct Decompressor {
        zdctx: zstd_safe::DCtx<'static>,
    }

    impl Decompressor {
        pub(super) fn new() -> Self {
            Self {
                zdctx: zstd_safe::DCtx::default(),
            }
        }

        pub(super) fn decompress(&mut self, input: &[u8]) -> std::io::Result<Vec<u8>> {
            let mut source = zstd_safe::InBuffer::around(input);
            let mut data = vec![0; input.len() * 4];
            let mut pos = 0;

            loop {
                let mut output = zstd_safe::OutBuffer::around_pos(data.as_mut_slice(), pos);
                self.zdctx
                    .decompress_stream(&mut output, &mut source)
                    .map_err(zstd_safe::get_error_name)
                    .map_err(std::io::Error::other)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Payload of the update at index `i` in the streams below.
    fn payload(i: u8) -> Vec<u8> {
        // `j / 17` stays under 121, so the conversion never truncates.
        (0..2048u32)
            .map(|j| u8::try_from(j / 17).expect("fits in u8").wrapping_add(i))
            .collect()
    }

    /// The updates encoded by both streams below. The third repeats the first,
    /// so decoding it correctly requires the history of the earlier updates.
    fn payloads() -> Vec<Vec<u8>> {
        vec![payload(0), payload(1), payload(0), payload(2)]
    }

    // The two streams below contain four updates of a single zstd stream
    // as produced by each of the two backends.
    //
    // Decoding both of them here proves that each backend can decode streams
    // produced by the other.

    const ZSTD_SAFE_STREAM: &[&[u8]] = &[
        &[
            0x28, 0xb5, 0x2f, 0xfd, 0x00, 0x88, 0x44, 0x04, 0x00, 0x12, 0x88, 0x1d, 0x07, 0x10, 0x10, 0xdf, 0x30, 0xb3,
            0x7f, 0x0a, 0xff, 0xff, 0xff, 0xff, 0x01, 0x00, 0x75, 0xfa, 0x5c, 0x1e, 0x87, 0xbf, 0xdd, 0x6d, 0xf6, 0x5a,
            0x9d, 0x46, 0x9f, 0xcd, 0x65, 0xf2, 0x58, 0x1c, 0x06, 0x7f, 0xbd, 0x5d, 0xee, 0x56, 0x9b, 0xc5, 0x5e, 0xad,
            0x55, 0xea, 0x54, 0x1a, 0x85, 0x3e, 0x9d, 0x4d, 0xe6, 0x52, 0x99, 0x44, 0x1e, 0x8d, 0x45, 0xe2, 0x50, 0x18,
            0x04, 0xfe, 0x7c, 0x3d, 0xde, 0x4e, 0x97, 0xc3, 0xdd, 0x6c, 0x35, 0xda, 0x4c, 0x16, 0x83, 0xbd, 0x5c, 0x2d,
            0xd6, 0x4a, 0x95, 0x42, 0x9d, 0x4c, 0x25, 0xd2, 0x48, 0x14, 0x02, 0x7d, 0x3c, 0x1d, 0xce, 0x46, 0x93, 0xc1,
            0x5c, 0x2c, 0x15, 0xca, 0x44, 0x12, 0x81, 0x3c, 0x1c, 0x0d, 0xc6, 0x42, 0x91, 0x40, 0x1c, 0x0c, 0x05, 0xc2,
            0x40, 0x10, 0xd8, 0xf7, 0x78, 0x98, 0x10, 0xf0, 0x07, 0x00, 0x10, 0x7e, 0xc5, 0x0f, 0xd5, 0xda, 0x38, 0x48,
            0x01,
        ],
        &[
            0x94, 0x00, 0x00, 0x40, 0x79, 0x79, 0x79, 0x79, 0x79, 0x79, 0x79, 0x79, 0x02, 0x00, 0xb8, 0x02, 0xd8, 0x97,
            0xff, 0xcf, 0x40,
        ],
        &[0x44, 0x00, 0x00, 0x00, 0x01, 0x00, 0xfd, 0x0f, 0xc0, 0x7f, 0x80],
        &[
            0x94, 0x00, 0x00, 0x40, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x02, 0x00, 0xb8, 0x02, 0xd8, 0x97,
            0xff, 0x0f, 0x81,
        ],
    ];

    const ZRIP_STREAM: &[&[u8]] = &[
        &[
            0x28, 0xb5, 0x2f, 0xfd, 0x04, 0x48, 0x44, 0x04, 0x00, 0x04, 0x08, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,
            0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18,
            0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a,
            0x2b, 0x2c, 0x2d, 0x2e, 0x2f, 0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c,
            0x3d, 0x3e, 0x3f, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4a, 0x4b, 0x4c, 0x4d, 0x4e,
            0x4f, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x60,
            0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6a, 0x6b, 0x6c, 0x6d, 0x6e, 0x6f, 0x70, 0x71, 0x72,
            0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x78, 0x78, 0x78, 0x78, 0x78, 0x78, 0x78, 0x78, 0x54, 0x01, 0x00, 0x0d,
            0x01,
        ],
        &[
            0x9c, 0x00, 0x00, 0x48, 0x78, 0x79, 0x79, 0x79, 0x79, 0x79, 0x79, 0x79, 0x79, 0x02, 0x00, 0x2e, 0x21, 0xec,
            0xcb, 0xff, 0x67, 0x20,
        ],
        &[0x44, 0x00, 0x00, 0x00, 0x01, 0x00, 0xfd, 0x0f, 0xc0, 0x7f, 0x80],
        &[
            0x9c, 0x00, 0x00, 0x48, 0x79, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x7a, 0x02, 0x00, 0x2f, 0x21, 0xec,
            0xcb, 0xff, 0x87, 0x40,
        ],
    ];

    fn assert_decodes(stream: &[&[u8]]) {
        let mut decompressor = Decompressor::new();

        for (update, expected) in stream.iter().zip(payloads()) {
            let decompressed = decompressor.decompress(update).unwrap();
            assert_eq!(decompressed, expected);
        }
    }

    #[test]
    fn decompresses_stream_produced_by_zstd_safe() {
        assert_decodes(ZSTD_SAFE_STREAM);
    }

    #[test]
    fn decompresses_stream_produced_by_zrip() {
        assert_decodes(ZRIP_STREAM);
    }

    /// An update is only decodable because the ones before it were fed in
    /// order: starting mid-stream must not silently produce garbage.
    #[test]
    fn rejects_stream_joined_late() {
        let mut decompressor = Decompressor::new();

        let result = decompressor.decompress(ZRIP_STREAM[1]);

        assert!(result.is_err(), "expected an error, got {result:?}");
    }
}
