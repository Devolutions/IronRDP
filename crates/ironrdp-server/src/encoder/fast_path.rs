use core::{cmp, fmt};

use ironrdp_pdu::{
    fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode},
    Encode, WriteCursor,
};

// this is the maximum amount of data (not including headers) we can send in a single TS_FP_UPDATE_PDU
const MAX_FASTPATH_UPDATE_SIZE: usize = 16_374;

const FASTPATH_HEADER_SIZE: usize = 6;

pub(crate) struct UpdateFragmenterOwned {
    code: UpdateCode,
    index: usize,
    len: usize,
}

pub(crate) struct UpdateFragmenter<'a> {
    code: UpdateCode,
    index: usize,
    data: &'a [u8],
}

impl fmt::Debug for UpdateFragmenter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpdateFragmenter")
            .field("len", &self.data.len())
            .finish()
    }
}

impl<'a> UpdateFragmenter<'a> {
    pub(crate) fn new(code: UpdateCode, data: &'a [u8]) -> Self {
        Self { code, index: 0, data }
    }

    pub(crate) fn into_owned(self) -> UpdateFragmenterOwned {
        UpdateFragmenterOwned {
            code: self.code,
            index: self.index,
            len: self.data.len(),
        }
    }

    pub(crate) fn from_owned(res: UpdateFragmenterOwned, buffer: &'a [u8]) -> UpdateFragmenter<'a> {
        Self {
            code: res.code,
            index: res.index,
            data: &buffer[0..res.len],
        }
    }

    pub(crate) fn size_hint(&self) -> usize {
        FASTPATH_HEADER_SIZE + cmp::min(self.data.len(), MAX_FASTPATH_UPDATE_SIZE)
    }

    pub(crate) fn next(&mut self, dst: &mut [u8]) -> Option<usize> {
        let (consumed, written) = self.encode_next(dst)?;
        self.data = &self.data[consumed..];
        self.index = self.index.checked_add(1)?;
        Some(written)
    }

    fn encode_next(&mut self, dst: &mut [u8]) -> Option<(usize, usize)> {
        match self.data.len() {
            0 => None,

            1..=MAX_FASTPATH_UPDATE_SIZE => {
                let frag = if self.index > 0 {
                    Fragmentation::Last
                } else {
                    Fragmentation::Single
                };

                self.encode_fastpath(frag, self.data, dst)
                    .map(|written| (self.data.len(), written))
            }

            _ => {
                let frag = if self.index > 0 {
                    Fragmentation::Next
                } else {
                    Fragmentation::First
                };

                self.encode_fastpath(frag, &self.data[..MAX_FASTPATH_UPDATE_SIZE], dst)
                    .map(|written| (MAX_FASTPATH_UPDATE_SIZE, written))
            }
        }
    }

    fn encode_fastpath(&self, frag: Fragmentation, data: &[u8], dst: &mut [u8]) -> Option<usize> {
        let mut cursor = WriteCursor::new(dst);

        let update = FastPathUpdatePdu {
            fragmentation: frag,
            update_code: self.code,
            compression_flags: None,
            compression_type: None,
            data,
        };

        let header = FastPathHeader::new(EncryptionFlags::empty(), update.size());

        header.encode(&mut cursor).ok()?;
        update.encode(&mut cursor).ok()?;

        Some(cursor.pos())
    }
}
