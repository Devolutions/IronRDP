use std::borrow::Cow;

use bitflags::bitflags;
use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{ensure_fixed_part_size, ensure_size, PduDecode, PduEncode, PduResult};

bitflags! {
    /// Represents `mappingMode` fields of `CLIPRDR_MFPICT` strucutre.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct PackedMetafileMappingMode: u32 {
        /// Each logical unit is mapped to one device pixel. Positive x is to the right; positive
        /// y is down.
        const TEXT = 0x00000001;
        /// Each logical unit is mapped to 0.1 millimeter. Positive x is to the right; positive
        /// y is up.
        const LO_METRIC = 0x00000002;
        /// Each logical unit is mapped to 0.01 millimeter. Positive x is to the right; positive
        /// y is up.
        const HI_METRIC = 0x00000003;
        /// Each logical unit is mapped to 0.01 inch. Positive x is to the right; positive y is up.
        const LO_ENGLISH = 0x00000004;
        /// Each logical unit is mapped to 0.001 inch. Positive x is to the right; positive y is up.
        const HI_ENGLISH = 0x00000005;
        /// Each logical unit is mapped to 1/20 of a printer's point (1/1440 of an inch), also
        /// called a twip. Positive x is to the right; positive y is up.
        const TWIPS = 0x00000006;
        /// Logical units are mapped to arbitrary units with equally scaled axes; one unit along
        /// the x-axis is equal to one unit along the y-axis.
        const ISOTROPIC = 0x00000007;
        /// Logical units are mapped to arbitrary units with arbitrarily scaled axes.
        const ANISOTROPIC = 0x00000008;
    }
}

/// Represents `CLIPRDR_MFPICT`
///
/// NOTE: `PduDecode` implementation will read all remaining data in cursor as metafile contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedMetafile<'a> {
    pub mapping_mode: PackedMetafileMappingMode,
    pub x_ext: u32,
    pub y_ext: u32,
    /// The variable sized contents of the metafile as specified in [MS-WMF] section 2
    data: Cow<'a, [u8]>,
}

impl PackedMetafile<'_> {
    const NAME: &str = "CLIPRDR_MFPICT";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u32>() * 3;

    pub fn new(
        mapping_mode: PackedMetafileMappingMode,
        x_ext: u32,
        y_ext: u32,
        data: impl Into<Cow<'static, [u8]>>,
    ) -> Self {
        Self {
            mapping_mode,
            x_ext,
            y_ext,
            data: data.into(),
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

impl PduEncode for PackedMetafile<'_> {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.mapping_mode.bits());
        dst.write_u32(self.x_ext);
        dst.write_u32(self.y_ext);
        dst.write_slice(&self.data);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.data.len()
    }
}

impl<'de> PduDecode<'de> for PackedMetafile<'de> {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let mapping_mode = PackedMetafileMappingMode::from_bits_truncate(src.read_u32());
        let x_ext = src.read_u32();
        let y_ext = src.read_u32();

        let data_len = src.len();

        let data = src.read_slice(data_len);

        Ok(Self {
            mapping_mode,
            x_ext,
            y_ext,
            data: Cow::Borrowed(data),
        })
    }
}
