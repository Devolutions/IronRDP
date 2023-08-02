pub mod bitmap;

use ironrdp_pdu::{
    cursor::WriteCursor,
    fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode},
    PduEncode,
};

use self::bitmap::BitmapEncoder;

use super::BitmapUpdate;

const MAX_FASTPATH_UPDATE_SIZE: usize = 16_374;

pub struct UpdateEncoder {
    buffer: Vec<u8>,
    bitmap: BitmapEncoder,
}

impl UpdateEncoder {
    pub fn new() -> Self {
        Self {
            buffer: vec![0; 8192 * 8192],
            bitmap: BitmapEncoder::new(),
        }
    }

    pub fn bitmap(&mut self, bitmap: BitmapUpdate, output: &mut [u8]) -> Option<usize> {
        let len = self.bitmap.encode(&bitmap, self.buffer.as_mut_slice()).unwrap();
        UpdateFragmenter::new(UpdateCode::Bitmap).fragment_update(&self.buffer[..len], output)
    }
}

struct UpdateFragmenter {
    code: UpdateCode,
}

impl UpdateFragmenter {
    fn new(code: UpdateCode) -> Self {
        Self { code }
    }

    fn fragment_update(&self, src: &[u8], dst: &mut [u8]) -> Option<usize> {
        let mut cursor = WriteCursor::new(dst);

        if src.len() < MAX_FASTPATH_UPDATE_SIZE {
            self.fastpath_update(Fragmentation::Single, src, &mut cursor);
        } else {
            let mut iter = src.chunks(MAX_FASTPATH_UPDATE_SIZE).peekable();

            let chunk = iter.next()?;
            self.fastpath_update(Fragmentation::First, chunk, &mut cursor);

            while let Some(chunk) = iter.next() {
                let frag = if iter.peek().is_none() {
                    Fragmentation::Last
                } else {
                    Fragmentation::Next
                };

                self.fastpath_update(frag, chunk, &mut cursor);
            }
        }

        Some(cursor.pos())
    }

    fn fastpath_update(&self, frag: Fragmentation, data: &[u8], cursor: &mut WriteCursor) {
        let update = FastPathUpdatePdu {
            fragmentation: frag,
            update_code: self.code,
            compression_flags: None,
            compression_type: None,
            data,
        };

        let header = FastPathHeader {
            flags: EncryptionFlags::empty(),
            data_length: update.size(),
            forced_long_length: false,
        };

        header.encode(cursor).unwrap();
        update.encode(cursor).unwrap();
    }
}
