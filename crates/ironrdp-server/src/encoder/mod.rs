pub mod bitmap;

use ironrdp_pdu::{
    fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode},
    PduEncode,
};

use self::bitmap::BitmapEncoder;

use super::BitmapUpdate;

const MAX_FASTPATH_UPDATE_SIZE: usize = 16_383;

pub struct UpdateEncoder {
    buffer: Vec<u8>,
    bitmap: BitmapEncoder,
}

impl UpdateEncoder {
    pub fn new() -> Self {
        Self {
            buffer: vec![0; 8192 * 8192],
            bitmap: BitmapEncoder {},
        }
    }

    pub fn bitmap(&mut self, bitmap: BitmapUpdate, output: &mut [u8]) -> Option<usize> {
        let update = self.bitmap.handle(&bitmap)?;
        let len = update.size();
        ironrdp_pdu::encode(&update, self.buffer.as_mut_slice()).unwrap();
        self.fragment_update(len, output)
    }

    fn fragment_update(&mut self, len: usize, mut output: &mut [u8]) -> Option<usize> {
        if len > MAX_FASTPATH_UPDATE_SIZE {
            let mut written = 0;
            let mut iter = self.buffer[..len].chunks(MAX_FASTPATH_UPDATE_SIZE).peekable();

            let chunk = iter.next().unwrap();

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

    fn fastpath_update(&self, frag: Fragmentation, data: &[u8], output: &mut [u8]) -> usize {
        let compression = self.bitmap.compression();

        let update = FastPathUpdatePdu {
            fragmentation: frag,
            update_code: UpdateCode::Bitmap,
            compression_flags: compression.map(|c| c.0),
            compression_type: compression.map(|c| c.1),
            data,
        };

        let header = FastPathHeader {
            flags: EncryptionFlags::empty(),
            data_length: update.size(),
            forced_long_length: false,
        };

        0
    }
}
