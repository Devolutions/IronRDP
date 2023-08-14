use crate::clipboard::{ClipboardPduFlags, PartialHeader};
use crate::cursor::{ReadCursor, WriteCursor};
use crate::utils::{combine_u64, split_u64};
use crate::{invalid_message_err, PduDecode, PduEncode, PduResult};
use bitflags::bitflags;
use std::borrow::Cow;

bitflags! {
    /// Represents `dwFlags` field of `CLIPRDR_FILECONTENTS_REQUEST` structure.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct FileContentsFlags: u32 {
        /// A request for the size of the file identified by the lindex field. The size MUST be
        /// returned as a 64-bit, unsigned integer. The cbRequested field MUST be set to
        /// 0x00000008 and both the nPositionLow and nPositionHigh fields MUST be
        /// set to 0x00000000.
        const SIZE = 0x00000001;
        /// A request for the data present in the file identified by the lindex field. The data
        /// to be retrieved is extracted starting from the offset given by the nPositionLow
        /// and nPositionHigh fields. The maximum number of bytes to extract is specified
        /// by the cbRequested field.
        const DATA = 0x00000002;
    }
}

/// Represents `CLIPRDR_FILECONTENTS_RESPONSE`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContentsResponse<'a> {
    is_error: bool,
    stream_id: u32,
    data: Cow<'a, [u8]>,
}

impl<'a> FileContentsResponse<'a> {
    const NAME: &str = "CLIPRDR_FILECONTENTS_RESPONSE";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u32>();

    fn inner_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.data.len()
    }

    /// Creates a new `FileContentsResponse` with u64 size value
    pub fn new_size_response(stream_id: u32, size: u64) -> Self {
        Self {
            is_error: false,
            stream_id,
            data: Cow::Owned(size.to_le_bytes().to_vec()),
        }
    }

    /// Creates a new `FileContentsResponse` with file contents value
    pub fn new_data_response(stream_id: u32, data: impl Into<Cow<'a, [u8]>>) -> Self {
        Self {
            is_error: false,
            stream_id,
            data: data.into(),
        }
    }

    /// Creates new `FileContentsResponse` with error
    pub fn new_error(stream_id: u32) -> Self {
        Self {
            is_error: true,
            stream_id,
            data: Cow::Borrowed(&[]),
        }
    }

    pub fn stream_id(&self) -> u32 {
        self.stream_id
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Read data as u64 size value
    pub fn data_as_size(&self) -> PduResult<u64> {
        if self.data.len() != 8 {
            return Err(invalid_message_err!(
                "requestedFileContentsData",
                "Invalid data size for u64 size"
            ));
        }

        Ok(u64::from_le_bytes(self.data.as_ref().try_into().unwrap()))
    }
}

impl PduEncode for FileContentsResponse<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let flags = if self.is_error {
            ClipboardPduFlags::RESPONSE_FAIL
        } else {
            ClipboardPduFlags::RESPONSE_OK
        };

        let header = PartialHeader::new_with_flags(self.inner_size() as u32, flags);
        header.encode(dst)?;

        ensure_size!(in: dst, size: self.inner_size());

        dst.write_u32(self.stream_id);
        dst.write_slice(&self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE + self.inner_size()
    }
}

impl<'de> PduDecode<'de> for FileContentsResponse<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let header = PartialHeader::decode(src)?;

        let is_error = header.message_flags.contains(ClipboardPduFlags::RESPONSE_FAIL);

        ensure_size!(in: src, size: header.inner_data_length());

        let data_size = header.inner_data_length() - Self::FIXED_PART_SIZE;

        let stream_id = src.read_u32();
        let data = src.read_slice(data_size);

        Ok(Self {
            is_error,
            stream_id,
            data: Cow::Borrowed(data),
        })
    }
}

/// Represents `CLIPRDR_FILECONTENTS_REQUEST`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContentsRequest {
    pub stream_id: u32,
    pub index: u32,
    pub flags: FileContentsFlags,
    pub position: u64,
    pub requested_size: u32,
    pub data_id: Option<u32>,
}

impl FileContentsRequest {
    const NAME: &str = "CLIPRDR_FILECONTENTS_REQUEST";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u32>() * 4 + std::mem::size_of::<u64>();

    fn inner_size(&self) -> usize {
        let data_id_size = match self.data_id {
            Some(_) => std::mem::size_of::<u32>(),
            None => 0,
        };

        Self::FIXED_PART_SIZE + data_id_size
    }
}

impl PduEncode for FileContentsRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = PartialHeader::new(self.inner_size() as u32);
        header.encode(dst)?;

        ensure_size!(in: dst, size: self.inner_size());

        dst.write_u32(self.stream_id);
        dst.write_u32(self.index);
        dst.write_u32(self.flags.bits());

        let (position_lo, position_hi) = split_u64(self.position);
        dst.write_u32(position_lo);
        dst.write_u32(position_hi);
        dst.write_u32(self.requested_size);

        if let Some(data_id) = self.data_id {
            dst.write_u32(data_id);
        };

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        PartialHeader::SIZE + self.inner_size()
    }
}

impl<'de> PduDecode<'de> for FileContentsRequest {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let header = PartialHeader::decode(src)?;

        let read_data_id = header.inner_data_length() > Self::FIXED_PART_SIZE;

        let mut expected_size = Self::FIXED_PART_SIZE;
        if read_data_id {
            expected_size += std::mem::size_of::<u32>();
        }

        ensure_size!(in: src, size: expected_size);

        let stream_id = src.read_u32();
        let index = src.read_u32();
        let flags = FileContentsFlags::from_bits_truncate(src.read_u32());
        let position_lo = src.read_u32();
        let position_hi = src.read_u32();
        let position = combine_u64(position_lo, position_hi);
        let requested_size = src.read_u32();
        let data_id = if read_data_id { Some(src.read_u32()) } else { None };

        Ok(Self {
            stream_id,
            index,
            flags,
            position,
            requested_size,
            data_id,
        })
    }
}
