pub mod bitmap;

use ironrdp_pdu::{
    fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdate, FastPathUpdatePdu, Fragmentation, UpdateCode},
    rdp::{client_info::CompressionType, headers::CompressionFlags},
    PduBufferParsing, PduParsing,
};

use super::BitmapUpdate;

const MAX_FASTPATH_UPDATE_SIZE: usize = 16_383;

pub trait UpdateHandler {
    fn handle<'a>(&mut self, bitmap: &'a BitmapUpdate) -> Option<FastPathUpdate<'a>>;
    fn compression(&self) -> Option<(CompressionFlags, CompressionType)>;
}

pub struct UpdateEncoder<H: UpdateHandler> {
    buffer: Vec<u8>,
    handler: H,
}

impl<H: UpdateHandler> UpdateEncoder<H> {
    pub fn new(handler: H) -> Self {
        Self {
            buffer: vec![0; 8192 * 8192],
            handler,
        }
    }

    pub fn encode(&mut self, bitmap: BitmapUpdate, output: &mut [u8]) -> Option<usize> {
        let update = self.handler.handle(&bitmap)?;
        let len = update.buffer_length();
        update.to_buffer_consume(&mut self.buffer.as_mut_slice()).unwrap();
        self.bitmap_update(len, output)
    }

    fn bitmap_update(&mut self, len: usize, mut output: &mut [u8]) -> Option<usize> {
        if len > MAX_FASTPATH_UPDATE_SIZE {
            let mut written = 0;
            let mut iter = self.buffer[..len].chunks(MAX_FASTPATH_UPDATE_SIZE).peekable();

            let chunk = iter.next().unwrap();

            // TODO: there has to be a better way
            let size = self.fastpath_update(Fragmentation::First, chunk, output);
            output = &mut output[size..];
            written += size;

            while let Some(chunk) = iter.next() {
                let frag = if iter.peek().is_none() {
                    Fragmentation::Last
                } else {
                    Fragmentation::Next
                };

                let size = self.fastpath_update(frag, chunk, output);
                output = &mut output[size..];
                written += size;
            }

            Some(written)
        } else {
            Some(self.fastpath_update(Fragmentation::Single, &self.buffer[..len], output))
        }
    }

    fn fastpath_update(&self, frag: Fragmentation, data: &[u8], mut output: &mut [u8]) -> usize {
        let compression = self.handler.compression();

        let update = FastPathUpdatePdu {
            fragmentation: frag,
            update_code: UpdateCode::Bitmap,
            compression_flags: compression.map(|c| c.0),
            compression_type: compression.map(|c| c.1),
            data,
        };

        let header = FastPathHeader {
            flags: EncryptionFlags::empty(),
            data_length: update.buffer_length(),
            forced_long_length: false,
        };

        header.to_buffer(&mut output).unwrap();
        update.to_buffer_consume(&mut output).unwrap();
        header.buffer_length() + update.buffer_length()
    }
}
