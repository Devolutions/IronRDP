//! Zstd decompression for the QOIZ bitmap codec.

use crate::{SessionResult, reason_err};

/// Streaming decompressor for QOIZ updates.
///
/// The server compresses all QOIZ updates of a session as a single zstd stream and flushes it after each update.
/// Feed every update in order, because an update can reference data from earlier ones.
#[cfg_attr(feature = "__test", visibility::make(pub))]
#[derive(Default)]
pub(crate) struct Decompressor {
    zdctx: zstd_safe::DCtx<'static>,
}

impl Decompressor {
    /// Decompresses the payload of a single QOIZ update.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn decompress(&mut self, compressed: &[u8]) -> SessionResult<Vec<u8>> {
        let mut input = zstd_safe::InBuffer::around(compressed);
        // Never start empty, or the buffer could not grow and the loop would never end.
        let mut data = vec![0; compressed.len().max(1) * 4];
        let mut pos = 0;

        loop {
            let mut output = zstd_safe::OutBuffer::around_pos(data.as_mut_slice(), pos);
            self.zdctx
                .decompress_stream(&mut output, &mut input)
                .map_err(zstd_safe::get_error_name)
                .map_err(|e| reason_err!("zstd", "{}", e))?;
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
