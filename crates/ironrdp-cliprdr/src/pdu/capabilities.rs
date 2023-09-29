use bitflags::bitflags;
use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::{
    cast_int, cast_length, ensure_fixed_part_size, ensure_size, invalid_message_err, read_padding, write_padding,
    PduDecode, PduEncode, PduResult,
};

use crate::pdu::PartialHeader;

/// Represents `CLIPRDR_CAPS`
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Capabilities {
    pub capabilities: Vec<CapabilitySet>,
}

impl Capabilities {
    const NAME: &str = "CLIPRDR_CAPS";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u16>() * 2;

    fn inner_size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.capabilities.iter().map(|c| c.size()).sum::<usize>()
    }

    pub fn new(version: ClipboardProtocolVersion, general_flags: ClipboardGeneralCapabilityFlags) -> Self {
        let capabilities = vec![CapabilitySet::General(GeneralCapabilitySet { version, general_flags })];

        Self { capabilities }
    }

    pub fn flags(&self) -> ClipboardGeneralCapabilityFlags {
        // There is only one capability set in the capabilities field in current CLIPRDR version
        self.capabilities
            .first()
            .map(|set| set.general().general_flags)
            .unwrap_or_else(ClipboardGeneralCapabilityFlags::empty)
    }

    pub fn version(&self) -> ClipboardProtocolVersion {
        self.capabilities
            .first()
            .map(|set| set.general().version)
            .unwrap_or(ClipboardProtocolVersion::V1)
    }

    pub fn downgrade(&mut self, server_caps: &Self) {
        let client_flags = self.flags();
        let server_flags = self.flags();

        let flags = client_flags & server_flags;
        let version = self.version().downgrade(server_caps.version());

        self.capabilities = vec![CapabilitySet::General(GeneralCapabilitySet {
            version,
            general_flags: flags,
        })];
    }
}

impl PduEncode for Capabilities {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let header = PartialHeader::new(cast_int!("dataLen", self.inner_size())?);
        header.encode(dst)?;

        ensure_size!(in: dst, size: self.inner_size());

        dst.write_u16(cast_length!(Self::NAME, "cCapabilitiesSets", self.capabilities.len())?);
        write_padding!(dst, 2);

        for capability in &self.capabilities {
            capability.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.inner_size() + PartialHeader::SIZE
    }
}

impl<'de> PduDecode<'de> for Capabilities {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let _header = PartialHeader::decode(src)?;

        ensure_fixed_part_size!(in: src);
        let capabilities_count = src.read_u16();
        read_padding!(src, 2);

        let mut capabilities = Vec::with_capacity(usize::from(capabilities_count));

        for _ in 0..capabilities_count {
            let caps = CapabilitySet::decode(src)?;
            capabilities.push(caps);
        }

        Ok(Self { capabilities })
    }
}

/// Represents `CLIPRDR_CAPS_SET`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitySet {
    General(GeneralCapabilitySet),
}

impl CapabilitySet {
    const NAME: &str = "CLIPRDR_CAPS_SET";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u16>() * 2;

    const CAPSTYPE_GENERAL: u16 = 0x0001;

    pub fn general(&self) -> &GeneralCapabilitySet {
        match self {
            Self::General(value) => value,
        }
    }
}

impl From<GeneralCapabilitySet> for CapabilitySet {
    fn from(value: GeneralCapabilitySet) -> Self {
        Self::General(value)
    }
}

impl PduEncode for CapabilitySet {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let (caps, length) = match self {
            Self::General(value) => {
                let length = value.size() + Self::FIXED_PART_SIZE;
                (value, length)
            }
        };

        ensure_size!(in: dst, size: length);
        dst.write_u16(Self::CAPSTYPE_GENERAL);
        dst.write_u16(cast_int!("lengthCapability", length)?);
        caps.encode(dst)
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let variable_size = match self {
            Self::General(value) => value.size(),
        };

        Self::FIXED_PART_SIZE + variable_size
    }
}

impl<'de> PduDecode<'de> for CapabilitySet {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let caps_type = src.read_u16();
        let _length = src.read_u16();

        match caps_type {
            Self::CAPSTYPE_GENERAL => {
                let general = GeneralCapabilitySet::decode(src)?;
                Ok(Self::General(general))
            }
            _ => Err(invalid_message_err!(
                "capabilitySetType",
                "invalid clipboard capability set type"
            )),
        }
    }
}

/// Represents `CLIPRDR_GENERAL_CAPABILITY` without header
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneralCapabilitySet {
    pub version: ClipboardProtocolVersion,
    pub general_flags: ClipboardGeneralCapabilityFlags,
}

impl GeneralCapabilitySet {
    const NAME: &str = "CLIPRDR_GENERAL_CAPABILITY";
    const FIXED_PART_SIZE: usize = std::mem::size_of::<u32>() * 2;
}

impl PduEncode for GeneralCapabilitySet {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u32(self.version.into());
        dst.write_u32(self.general_flags.bits());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for GeneralCapabilitySet {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let version: ClipboardProtocolVersion = src.read_u32().try_into()?;
        let general_flags = ClipboardGeneralCapabilityFlags::from_bits_truncate(src.read_u32());

        Ok(Self { version, general_flags })
    }
}

/// Specifies the `Remote Desktop Protocol: Clipboard Virtual Channel Extension` version number.
/// This field is for informational purposes and MUST NOT be used to make protocol capability
/// decisions. The actual features supported are specified via [`ClipboardGeneralCapabilityFlags`]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardProtocolVersion {
    V1,
    V2,
}

impl ClipboardProtocolVersion {
    const VERSION_VALUE_V1: u32 = 0x0000_0001;
    const VERSION_VALUE_V2: u32 = 0x0000_0002;

    const NAME: &str = "CLIPRDR_CAPS_VERSION";

    #[must_use]
    pub fn downgrade(self, other: Self) -> Self {
        if self != other {
            return Self::V1;
        }
        self
    }
}

impl From<ClipboardProtocolVersion> for u32 {
    fn from(version: ClipboardProtocolVersion) -> Self {
        match version {
            ClipboardProtocolVersion::V1 => ClipboardProtocolVersion::VERSION_VALUE_V1,
            ClipboardProtocolVersion::V2 => ClipboardProtocolVersion::VERSION_VALUE_V2,
        }
    }
}

impl TryFrom<u32> for ClipboardProtocolVersion {
    type Error = ironrdp_pdu::PduError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            Self::VERSION_VALUE_V1 => Ok(Self::V1),
            Self::VERSION_VALUE_V2 => Ok(Self::V2),
            _ => Err(invalid_message_err!(
                "version",
                "Invalid clipboard capabilities version"
            )),
        }
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ClipboardGeneralCapabilityFlags: u32 {
        /// The Long Format Name variant of the Format List PDU is supported
        /// for exchanging updated format names. If this flag is not set, the
        /// Short Format Name variant MUST be used. If this flag is set by both
        /// protocol endpoints, then the Long Format Name variant MUST be
        /// used.
        const USE_LONG_FORMAT_NAMES = 0x0000_0002;
        /// File copy and paste using stream-based operations are supported
        /// using the File Contents Request PDU and File Contents Response
        /// PDU.
        const STREAM_FILECLIP_ENABLED = 0x0000_0004;
        /// Indicates that any description of files to copy and paste MUST NOT
        /// include the source path of the files.
        const FILECLIP_NO_FILE_PATHS = 0x0000_0008;
        /// Locking and unlocking of File Stream data on the clipboard is
        /// supported using the Lock Clipboard Data PDU and Unlock Clipboard
        /// Data PDU.
        const CAN_LOCK_CLIPDATA = 0x0000_0010;
        /// Indicates support for transferring files that are larger than
        /// 4,294,967,295 bytes in size. If this flag is not set, then only files of
        /// size less than or equal to 4,294,967,295 bytes can be exchanged
        /// using the File Contents Request PDU and File Contents
        /// Response PDU.
        const HUGE_FILE_SUPPORT_ENABLED = 0x0000_0020;
    }
}
