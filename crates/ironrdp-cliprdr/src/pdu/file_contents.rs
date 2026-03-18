use std::borrow::Cow;

use bitflags::bitflags;
use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, IntoOwned, ReadCursor, WriteCursor, cast_int, ensure_size,
    invalid_field_err,
};
use ironrdp_pdu::impl_pdu_borrowing;
use ironrdp_pdu::utils::{combine_u64, split_u64};

use crate::pdu::{ClipboardPduFlags, PartialHeader};

bitflags! {
    /// Represents `dwFlags` field of `CLIPRDR_FILECONTENTS_REQUEST` structure.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct FileContentsFlags: u32 {
        /// A request for the size of the file identified by the lindex field. The size MUST be
        /// returned as a 64-bit, unsigned integer. The cbRequested field MUST be set to
        /// 0x00000008 and both the nPositionLow and nPositionHigh fields MUST be
        /// set to 0x00000000.
        const SIZE = 0x0000_0001;
        /// A request for a byte range from the file identified by the lindex field. The data
        /// to be retrieved is extracted starting from the offset given by the nPositionLow
        /// and nPositionHigh fields. The maximum number of bytes to extract is specified
        /// by the cbRequested field.
        const RANGE = 0x0000_0002;

        const _ = !0;
    }
}

impl FileContentsFlags {
    /// [MS-RDPECLIP] 2.2.5.3 - Validates that flags are spec-compliant
    ///
    /// Per spec requirements:
    /// - Exactly one of SIZE or RANGE must be set
    /// - SIZE and RANGE flags MUST NOT be set simultaneously
    pub fn validate(self) -> Result<(), &'static str> {
        let size_set = self.contains(FileContentsFlags::SIZE);
        let range_set = self.contains(FileContentsFlags::RANGE);

        match (size_set, range_set) {
            (true, true) => Err("SIZE and RANGE flags are mutually exclusive per MS-RDPECLIP 2.2.5.3"),
            (false, false) => Err("exactly one of SIZE or RANGE must be set"),
            _ => Ok(()),
        }
    }
}

/// Represents `CLIPRDR_FILECONTENTS_RESPONSE`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileContentsResponse<'a> {
    is_error: bool,
    stream_id: u32,
    data: Cow<'a, [u8]>,
}

impl_pdu_borrowing!(FileContentsResponse<'_>, OwnedFileContentsResponse);

impl IntoOwned for FileContentsResponse<'_> {
    type Owned = OwnedFileContentsResponse;

    fn into_owned(self) -> Self::Owned {
        OwnedFileContentsResponse {
            is_error: self.is_error,
            stream_id: self.stream_id,
            data: Cow::Owned(self.data.into_owned()),
        }
    }
}

impl<'a> FileContentsResponse<'a> {
    const NAME: &'static str = "CLIPRDR_FILECONTENTS_RESPONSE";
    const FIXED_PART_SIZE: usize = 4 /* streamId */;

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

    pub fn is_error(&self) -> bool {
        self.is_error
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// [MS-RDPECLIP] 2.2.5.4 - Read data as u64 size value
    ///
    /// Per spec, SIZE responses MUST contain exactly 8 bytes (64-bit unsigned integer).
    ///
    /// # Errors
    ///
    /// Returns an error if the data length is not exactly 8 bytes.
    ///
    /// # Panics
    ///
    /// Should not panic - the try_into conversion is guaranteed to succeed after length validation.
    pub fn data_as_size(&self) -> DecodeResult<u64> {
        if self.data.len() != 8 {
            return Err(invalid_field_err!(
                "requestedFileContentsData",
                "SIZE response must be exactly 8 bytes per MS-RDPECLIP 2.2.5.4"
            ));
        }

        // Per length check above, this conversion is infallible.
        let chunk: [u8; 8] = self
            .data
            .as_ref()
            .try_into()
            .map_err(|_| invalid_field_err!("requestedFileContentsData", "SIZE response data is not 8 bytes"))?;
        Ok(u64::from_le_bytes(chunk))
    }
}

impl Encode for FileContentsResponse<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let flags = if self.is_error {
            ClipboardPduFlags::RESPONSE_FAIL
        } else {
            ClipboardPduFlags::RESPONSE_OK
        };

        let header = PartialHeader::new_with_flags(cast_int!("dataLen", self.inner_size())?, flags);
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

impl<'de> Decode<'de> for FileContentsResponse<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let header = PartialHeader::decode(src)?;

        let is_error = header.message_flags.contains(ClipboardPduFlags::RESPONSE_FAIL);

        ensure_size!(in: src, size: header.data_length());

        if header.data_length() < Self::FIXED_PART_SIZE {
            return Err(invalid_field_err!("requestedFileContentsData", "invalid data size"));
        };

        let data_size = header.data_length() - Self::FIXED_PART_SIZE;

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
    /// Per [MS-RDPECLIP] 2.2.5.3, lindex is a signed 32-bit integer.
    /// Negative values are invalid and rejected during decode.
    pub index: i32,
    pub flags: FileContentsFlags,
    pub position: u64,
    pub requested_size: u32,
    pub data_id: Option<u32>,
}

impl FileContentsRequest {
    const NAME: &'static str = "CLIPRDR_FILECONTENTS_REQUEST";
    const FIXED_PART_SIZE: usize = 4 /* streamId */ + 4 /* lindex */ + 4 /* dwFlags */ + 8 /* nPositionLow + nPositionHigh */ + 4 /* cbRequested */;

    fn inner_size(&self) -> usize {
        let data_id_size = match self.data_id {
            Some(_) => 4,
            None => 0,
        };

        Self::FIXED_PART_SIZE + data_id_size
    }
}

impl Encode for FileContentsRequest {
    /// Encodes the request into the wire format.
    ///
    /// Note: this does not enforce the spec constraints from [MS-RDPECLIP] 2.2.5.3
    /// (e.g., that SIZE requests have `cbRequested = 8` and `position = 0`).
    /// Callers that build these PDUs are responsible for setting fields correctly;
    /// use [`FileContentsFlags::validate`] to check flag consistency.
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        let header = PartialHeader::new(cast_int!("dataLen", self.inner_size())?);
        header.encode(dst)?;

        ensure_size!(in: dst, size: self.inner_size());

        dst.write_u32(self.stream_id);
        dst.write_i32(self.index);
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

impl<'de> Decode<'de> for FileContentsRequest {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let header = PartialHeader::decode(src)?;

        let read_data_id = header.data_length() > Self::FIXED_PART_SIZE;

        let mut expected_size = Self::FIXED_PART_SIZE;
        if read_data_id {
            expected_size += 4;
        }

        ensure_size!(in: src, size: expected_size);

        let stream_id = src.read_u32();
        let index = src.read_i32();
        let flags = FileContentsFlags::from_bits_retain(src.read_u32());
        let position_lo = src.read_u32();
        let position_hi = src.read_u32();
        let position = combine_u64(position_lo, position_hi);
        let requested_size = src.read_u32();
        let data_id = if read_data_id { Some(src.read_u32()) } else { None };

        // [MS-RDPECLIP] 2.2.5.3 - Validate lindex is non-negative
        if index < 0 {
            return Err(invalid_field_err!(
                "lindex",
                "file index must be non-negative per MS-RDPECLIP 2.2.5.3"
            ));
        }

        // [MS-RDPECLIP] 2.2.5.3 - Validate flags are spec-compliant
        flags.validate().map_err(|e| invalid_field_err!("dwFlags", e))?;

        // [MS-RDPECLIP] 2.2.5.3 - Validate SIZE request constraints
        if flags.contains(FileContentsFlags::SIZE) {
            if requested_size != 8 {
                return Err(invalid_field_err!(
                    "cbRequested",
                    "SIZE request must have cbRequested=8 per MS-RDPECLIP 2.2.5.3"
                ));
            }
            if position != 0 {
                return Err(invalid_field_err!(
                    "position",
                    "SIZE request must have position=0 per MS-RDPECLIP 2.2.5.3"
                ));
            }
        }

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
