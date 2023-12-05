use std::borrow::Cow;

use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::utils::{read_string_from_cursor, write_string_to_cursor, CharacterSet};
use ironrdp_pdu::{
    cast_int, ensure_size, impl_pdu_borrowing, invalid_message_err, IntoOwnedPdu, PduDecode, PduEncode, PduResult,
};

use crate::pdu::PartialHeader;

/// Represents `CLIPRDR_TEMP_DIRECTORY`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientTemporaryDirectory<'a> {
    path_buffer: Cow<'a, [u8]>,
}

impl_pdu_borrowing!(ClientTemporaryDirectory<'_>, OwnedClientTemporaryDirectory);

impl IntoOwnedPdu for ClientTemporaryDirectory<'_> {
    type Owned = OwnedClientTemporaryDirectory;

    fn into_owned_pdu(self) -> Self::Owned {
        OwnedClientTemporaryDirectory {
            path_buffer: Cow::Owned(self.path_buffer.into_owned()),
        }
    }
}

impl ClientTemporaryDirectory<'_> {
    const PATH_BUFFER_SIZE: usize = 520;

    const NAME: &'static str = "CLIPRDR_TEMP_DIRECTORY";
    const INNER_SIZE: usize = Self::PATH_BUFFER_SIZE;

    /// Creates new `ClientTemporaryDirectory` and encodes given path to UTF-16 representation.
    pub fn new(path: &str) -> PduResult<Self> {
        let mut buffer = vec![0x00; Self::PATH_BUFFER_SIZE];

        {
            let mut cursor = WriteCursor::new(&mut buffer);
            write_string_to_cursor(&mut cursor, path, CharacterSet::Unicode, true)?;
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
}

impl PduEncode for ClientTemporaryDirectory<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = PartialHeader::new(cast_int!("dataLen", Self::INNER_SIZE)?);
        header.encode(dst)?;

        ensure_size!(in: dst, size: Self::INNER_SIZE);
        dst.write_slice(&self.path_buffer);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE + Self::INNER_SIZE
    }
}

impl<'de> PduDecode<'de> for ClientTemporaryDirectory<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let _header = PartialHeader::decode(src)?;

        ensure_size!(in: src, size: Self::INNER_SIZE);
        let buffer = src.read_slice(Self::PATH_BUFFER_SIZE);

        Ok(Self {
            path_buffer: Cow::Borrowed(buffer),
        })
    }
}
