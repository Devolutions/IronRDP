use std::borrow::Cow;

use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::utils::{read_string_from_cursor, write_string_to_cursor, CharacterSet};
use ironrdp_pdu::{
    cast_int, ensure_fixed_part_size, ensure_size, invalid_message_err, PduDecode, PduEncode, PduResult,
};

use crate::pdu::PartialHeader;

/// Represents `CLIPRDR_TEMP_DIRECTORY`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientTemporaryDirectory<'a> {
    path_buffer: Cow<'a, [u8]>,
}

impl ClientTemporaryDirectory<'_> {
    const PATH_BUFFER_SIZE: usize = 520;

    const NAME: &str = "CLIPRDR_TEMP_DIRECTORY";
    const FIXED_PART_SIZE: usize = Self::PATH_BUFFER_SIZE;

    /// Creates new `ClientTemporaryDirectory` and encodes given path to UTF-16 representation.
    pub fn new(path: String) -> PduResult<Self> {
        let mut buffer = vec![0x00; Self::PATH_BUFFER_SIZE];

        {
            let mut cursor = WriteCursor::new(&mut buffer);
            write_string_to_cursor(&mut cursor, &path, CharacterSet::Unicode, true)?;
        }

        Ok(Self {
            path_buffer: Cow::Owned(buffer),
        })
    }

    /// Returns parsed temporary directory path.
    pub fn temporary_directory_path(&self) -> PduResult<String> {
        let mut cursor = ReadCursor::new(&self.path_buffer);

        read_string_from_cursor(&mut cursor, CharacterSet::Unicode, true)
            .map_err(|_| invalid_message_err!("wszTempDir", "failed to decode temp dir path"))
    }

    fn inner_size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl PduEncode for ClientTemporaryDirectory<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = PartialHeader::new(cast_int!("dataLen", self.inner_size())?);
        header.encode(dst)?;

        ensure_size!(in: dst, size: self.inner_size());
        dst.write_slice(&self.path_buffer);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE + self.inner_size()
    }
}

impl<'de> PduDecode<'de> for ClientTemporaryDirectory<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let _header = PartialHeader::decode(src)?;

        ensure_fixed_part_size!(in: src);
        let buffer = src.read_slice(Self::PATH_BUFFER_SIZE);

        Ok(Self {
            path_buffer: Cow::Borrowed(buffer),
        })
    }
}
