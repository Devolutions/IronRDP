//! Message codecs for the [MS-RDPADRV] audio level and drive letter channels.
//!
//! Every message starts with a 32-bit `eEvent` type.
//! The two channels number their messages independently, so each channel has its own PDU type.
//!
//! [MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/

use alloc::string::String;
use alloc::vec::Vec;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_size, invalid_field_err,
};
use ironrdp_dvc::DvcEncode;

const EVENT_SIZE: usize = 4 /* eEvent */;

const SAE_STARTED: u32 = 0x0000_0001;
const SAE_VOLUME_CHANGE: u32 = 0x0000_0002;
const SAE_REMOTE_CONNECT: u32 = 0x0000_0003;

const SADLE_STARTED: u32 = 0x0000_0001;
const SADLE_SERIALIZED_CACHE: u32 = 0x0000_0002;

const E_RENDER: u32 = 0x0000_0000;
const E_CAPTURE: u32 = 0x0000_0001;

const NAME_MARKER: u32 = 0x1818_1818;
const VALUE_MARKER: u32 = 0x2727_2727;

/// Registry value type of a 32-bit little-endian number, the type the server stores drive letters as.
pub const REG_DWORD: u32 = 4;

/// Message on the `WMSAud` audio level channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AudioLevelPdu {
    /// Server to client: a new session needs the client's stored volume levels.
    ///
    /// [MS-RDPADRV] 2.2.1 "SAE_Started".
    ///
    /// [MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/
    Started,
    /// Either direction: the volume level of one data flow.
    ///
    /// [MS-RDPADRV] 2.2.2 "SAE_VolumeChange".
    ///
    /// [MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/
    VolumeChange(VolumeChange),
    /// Server to client: a reconnected session needs the client's stored volume levels.
    ///
    /// [MS-RDPADRV] 2.2.3 "SAE_RemoteConnect".
    ///
    /// [MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/
    RemoteConnect,
}

impl AudioLevelPdu {
    const NAME: &'static str = "AudioLevelPdu";
}

impl Encode for AudioLevelPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Started => dst.write_u32(SAE_STARTED),
            Self::VolumeChange(change) => {
                dst.write_u32(SAE_VOLUME_CHANGE);
                change.encode(dst)?;
            }
            Self::RemoteConnect => dst.write_u32(SAE_REMOTE_CONNECT),
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            Self::Started | Self::RemoteConnect => EVENT_SIZE,
            Self::VolumeChange(change) => EVENT_SIZE + change.size(),
        }
    }
}

impl<'de> Decode<'de> for AudioLevelPdu {
    /// Decodes one message; bytes past the fields of its type are ignored.
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(ctx: Self::NAME, in: src, size: EVENT_SIZE);
        match src.read_u32() {
            SAE_STARTED => Ok(Self::Started),
            SAE_VOLUME_CHANGE => Ok(Self::VolumeChange(VolumeChange::decode(src)?)),
            SAE_REMOTE_CONNECT => Ok(Self::RemoteConnect),
            _ => Err(invalid_field_err!(Self::NAME, "eEvent", "unknown audio level event", in: src)),
        }
    }
}

impl DvcEncode for AudioLevelPdu {}

/// Audio endpoint data flow a volume level applies to, from `EDataFlow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataFlow {
    /// Playback.
    Render,
    /// Recording.
    Capture,
}

impl DataFlow {
    /// Both data flows, in the order a client reports them ([MS-RDPADRV] 4.1).
    pub const ALL: [Self; 2] = [Self::Render, Self::Capture];

    fn as_u32(self) -> u32 {
        match self {
            Self::Render => E_RENDER,
            Self::Capture => E_CAPTURE,
        }
    }
}

/// Body of an [`AudioLevelPdu::VolumeChange`] message.
///
/// [MS-RDPADRV] 2.2.2 "SAE_VolumeChange".
///
/// [MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolumeChange {
    pub data_flow: DataFlow,
    /// Master volume level, normalized to 0.0 (minimum) through 1.0 (maximum) on an audio-tapered curve.
    ///
    /// The codec carries the value as received and does not check the range.
    pub volume: f32,
    /// Whether the data flow is muted.
    pub muted: bool,
}

impl VolumeChange {
    const NAME: &'static str = "SAE_VolumeChange";

    const FIXED_PART_SIZE: usize = 4 /* eDataFlow */ + 4 /* lVolume */ + 4 /* fMuted */;
}

impl Encode for VolumeChange {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: Self::FIXED_PART_SIZE);
        dst.write_u32(self.data_flow.as_u32());
        dst.write_u32(self.volume.to_bits());
        dst.write_u32(u32::from(self.muted));
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for VolumeChange {
    /// Decodes the fields that follow `eEvent`.
    ///
    /// Any nonzero `fMuted` reads as muted, like a Windows `BOOL`.
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(ctx: Self::NAME, in: src, size: Self::FIXED_PART_SIZE);
        let data_flow = match src.read_u32() {
            E_RENDER => DataFlow::Render,
            E_CAPTURE => DataFlow::Capture,
            _ => return Err(invalid_field_err!(Self::NAME, "eDataFlow", "unknown data flow", in: src)),
        };
        let volume = f32::from_bits(src.read_u32());
        let muted = src.read_u32() != 0;
        Ok(Self {
            data_flow,
            volume,
            muted,
        })
    }
}

/// Message on the `WMSDL` drive letter channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveLetterPdu {
    /// Server to client: the session needs the client's stored drive letter mappings.
    ///
    /// [MS-RDPADRV] 2.2.4 "SADLE_Started".
    ///
    /// [MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/
    Started,
    /// Either direction: the session's drive letter mappings.
    ///
    /// [MS-RDPADRV] 2.2.5 "SADLE_SerializedCache".
    ///
    /// [MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/
    SerializedCache(SerializedCache),
}

impl DriveLetterPdu {
    const NAME: &'static str = "DriveLetterPdu";
}

impl Encode for DriveLetterPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Started => dst.write_u32(SADLE_STARTED),
            Self::SerializedCache(cache) => {
                dst.write_u32(SADLE_SERIALIZED_CACHE);
                cache.encode(dst)?;
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            Self::Started => EVENT_SIZE,
            Self::SerializedCache(cache) => EVENT_SIZE + cache.size(),
        }
    }
}

impl<'de> Decode<'de> for DriveLetterPdu {
    /// Decodes one message; bytes past the fields of its type are ignored.
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(ctx: Self::NAME, in: src, size: EVENT_SIZE);
        match src.read_u32() {
            SADLE_STARTED => Ok(Self::Started),
            SADLE_SERIALIZED_CACHE => Ok(Self::SerializedCache(SerializedCache::decode(src)?)),
            _ => Err(invalid_field_err!(Self::NAME, "eEvent", "unknown drive letter event", in: src)),
        }
    }
}

impl DvcEncode for DriveLetterPdu {}

/// Body of a [`DriveLetterPdu::SerializedCache`] message: the registry values of the session's drive letter cache.
///
/// [MS-RDPADRV] 2.2.5 "SADLE_SerializedCache".
///
/// [MS-RDPADRV]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpadrv/
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SerializedCache {
    pub entries: Vec<CacheEntry>,
}

impl SerializedCache {
    const NAME: &'static str = "SADLE_SerializedCache";

    const FIXED_PART_SIZE: usize = 4 /* cbMessageData */ + 4 /* cbNameValueData */ + 4 /* cNameValuePairs */;

    fn name_value_data_size(&self) -> usize {
        self.entries.iter().map(CacheEntry::size).sum()
    }
}

impl Encode for SerializedCache {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        let data_size: u32 = cast_length!(Self::NAME, "cbNameValueData", self.name_value_data_size())?;
        dst.write_u32(data_size);
        dst.write_u32(data_size);
        dst.write_u32(cast_length!(Self::NAME, "cNameValuePairs", self.entries.len())?);
        for entry in &self.entries {
            entry.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.name_value_data_size()
    }
}

impl<'de> Decode<'de> for SerializedCache {
    /// Decodes the fields that follow `eEvent`.
    ///
    /// `cbNameValueData` bounds the name-value pairs, and bytes it counts past the last pair are the unused tail the specification allows.
    /// `cbMessageData` and `cbNameValueData` must be equal, as the specification requires of the sender.
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(ctx: Self::NAME, in: src, size: Self::FIXED_PART_SIZE);
        let message_data_size = src.read_u32();
        let name_value_data_size = src.read_u32();
        let pair_count = usize::try_from(src.read_u32()).unwrap_or(usize::MAX);
        if message_data_size != name_value_data_size {
            return Err(invalid_field_err!(
                Self::NAME,
                "cbNameValueData",
                "name-value data size differs from message data size",
                in: src
            ));
        }
        let name_value_data_size = usize::try_from(name_value_data_size).unwrap_or(usize::MAX);
        ensure_size!(ctx: Self::NAME, in: src, size: name_value_data_size);
        let mut data = ReadCursor::new(src.read_slice(name_value_data_size));

        // INVARIANT: every pair consumes bytes or fails, so a forged pair count cannot loop past the data.
        let entries = core::iter::repeat_with(|| CacheEntry::decode(&mut data))
            .take(pair_count)
            .collect::<DecodeResult<Vec<_>>>()?;
        Ok(Self { entries })
    }
}

/// One registry value of the drive letter cache: a `NAME_DATA` structure followed by a `VALUE_DATA` structure.
///
/// The server stores each mapping as a [`REG_DWORD`] value, whose name identifies the storage device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    /// Registry value name, `szName`, including any terminating null character the sender wrote.
    pub name: String,
    /// Registry value type, such as [`REG_DWORD`].
    pub value_type: u32,
    /// Registry value data, `rgValue`.
    pub value: Vec<u8>,
}

impl CacheEntry {
    const NAME: &'static str = "CacheEntry";

    const NAME_DATA_FIXED_PART_SIZE: usize = 4 /* Name marker */ + 4 /* cchName */;
    const VALUE_DATA_FIXED_PART_SIZE: usize = 4 /* Value marker */ + 4 /* Value type */ + 4 /* cbValue */;

    /// Creates a [`REG_DWORD`] entry.
    pub fn dword(name: String, value: u32) -> Self {
        Self {
            name,
            value_type: REG_DWORD,
            value: value.to_le_bytes().to_vec(),
        }
    }

    /// Returns the value of a [`REG_DWORD`] entry, or `None` for any other type or a value that is not 4 bytes long.
    pub fn as_dword(&self) -> Option<u32> {
        if self.value_type != REG_DWORD {
            return None;
        }
        let bytes: [u8; 4] = self.value.as_slice().try_into().ok()?;
        Some(u32::from_le_bytes(bytes))
    }

    fn name_size(&self) -> usize {
        self.name.encode_utf16().count() * 2 // UTF-16 code units of 2 bytes each
    }

    /// Returns the size of `szName` in bytes, given the `cchName` value and the bytes that follow it.
    ///
    /// The specification gives `cchName` both as a length in bytes and as a count of UTF-16 code units.
    /// The byte reading, the field's own definition, wins whenever `VALUE_DATA` starts where it puts the end of the name;
    /// otherwise the code unit reading is used if `VALUE_DATA` starts where that one puts it.
    fn name_size_from(name_length: usize, rest: &[u8]) -> usize {
        let value_marker_at = |offset: usize| {
            rest.get(offset..)
                .and_then(|tail| tail.get(..4))
                .is_some_and(|marker| marker == VALUE_MARKER.to_le_bytes())
        };
        match name_length.checked_mul(2) {
            Some(code_units_size) if !value_marker_at(name_length) && value_marker_at(code_units_size) => {
                code_units_size
            }
            _ => name_length,
        }
    }
}

impl Encode for CacheEntry {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(NAME_MARKER);
        dst.write_u32(cast_length!(Self::NAME, "cchName", self.name_size())?);
        for code_unit in self.name.encode_utf16() {
            dst.write_u16(code_unit);
        }
        dst.write_u32(VALUE_MARKER);
        dst.write_u32(self.value_type);
        dst.write_u32(cast_length!(Self::NAME, "cbValue", self.value.len())?);
        dst.write_slice(&self.value);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::NAME_DATA_FIXED_PART_SIZE // Name marker + cchName
            + self.name_size() // szName
            + Self::VALUE_DATA_FIXED_PART_SIZE // Value marker + Value type + cbValue
            + self.value.len() // rgValue
    }
}

impl<'de> Decode<'de> for CacheEntry {
    /// Decodes one `NAME_DATA` and `VALUE_DATA` pair.
    ///
    /// The specification gives `cchName` both as a length in bytes and as a count of UTF-16 code units.
    /// This codec writes it in bytes, the field's own definition, and reads either: see [`Self::name_size_from`].
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(ctx: Self::NAME, in: src, size: Self::NAME_DATA_FIXED_PART_SIZE);
        if src.read_u32() != NAME_MARKER {
            return Err(invalid_field_err!(Self::NAME, "Name marker", "invalid name marker", in: src));
        }
        let name_length = usize::try_from(src.read_u32()).unwrap_or(usize::MAX);
        let name_size = Self::name_size_from(name_length, src.remaining());
        if name_size % 2 != 0 {
            return Err(invalid_field_err!(Self::NAME, "cchName", "odd UTF-16 name length", in: src));
        }
        ensure_size!(ctx: Self::NAME, in: src, size: name_size);
        let name_bytes = src.read_slice(name_size);
        let name = char::decode_utf16(
            name_bytes
                .chunks_exact(2)
                .map(|code_unit| u16::from_le_bytes([code_unit[0], code_unit[1]])),
        )
        .collect::<Result<String, _>>()
        .map_err(|_| invalid_field_err!(Self::NAME, "szName", "invalid UTF-16 name", in: src))?;

        ensure_size!(ctx: Self::NAME, in: src, size: Self::VALUE_DATA_FIXED_PART_SIZE);
        if src.read_u32() != VALUE_MARKER {
            return Err(invalid_field_err!(Self::NAME, "Value marker", "invalid value marker", in: src));
        }
        let value_type = src.read_u32();
        let value_size = usize::try_from(src.read_u32()).unwrap_or(usize::MAX);
        ensure_size!(ctx: Self::NAME, in: src, size: value_size);
        let value = src.read_slice(value_size).to_vec();

        Ok(Self {
            name,
            value_type,
            value,
        })
    }
}
