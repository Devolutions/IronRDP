//! PDUs for [\[MS-RDPESC\]: Remote Desktop Protocol: Smart Card Virtual Channel Extension]
//!
//! [\[MS-RDPESC\]: Remote Desktop Protocol: Smart Card Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/0428ca28-b4dc-46a3-97c3-01887fa44a90

pub mod ndr;
pub mod rpce;

use bitflags::bitflags;
use ironrdp_core::{
    DecodeError, DecodeResult, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_size, invalid_field_err,
    other_err,
};
use ironrdp_pdu::utils::{
    CharacterSet, encoded_multistring_len, read_multistring_from_cursor, write_multistring_to_cursor,
};
use tracing::{error, warn};

use super::efs::IoCtlCode;
use crate::pdu::esc::ndr::{Decode as _, Encode as _};

/// [2.2.2] TS Server-Generated Structures
///
/// [2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/f4ca3b61-b49c-463c-8932-2cf82fb7ec7a
#[derive(Debug, PartialEq, Clone)]
pub enum ScardCall {
    AccessStartedEventCall(ScardAccessStartedEventCall),
    EstablishContextCall(EstablishContextCall),
    ListReaderGroupsCall(ListReaderGroupsCall),
    ListReadersCall(ListReadersCall),
    GetStatusChangeCall(GetStatusChangeCall),
    LocateCardsCall(LocateCardsCall),
    LocateCardsByAtrCall(LocateCardsByAtrCall),
    ConnectCall(ConnectCall),
    ReconnectCall(ReconnectCall),
    HCardAndDispositionCall(HCardAndDispositionCall),
    TransmitCall(TransmitCall),
    StatusCall(StatusCall),
    StateCall(StateCall),
    ControlCall(ControlCall),
    GetAttribCall(GetAttribCall),
    SetAttribCall(SetAttribCall),
    GetTransmitCountCall(GetTransmitCountCall),
    ContextCall(ContextCall),
    GetDeviceTypeIdCall(GetDeviceTypeIdCall),
    ReadCacheCall(ReadCacheCall),
    WriteCacheCall(WriteCacheCall),
    GetReaderIconCall(GetReaderIconCall),
    ContextAndStringCall(ContextAndStringCall),
    ContextAndTwoStringCall(ContextAndTwoStringCall),
    Unsupported,
}

impl ScardCall {
    pub fn decode(io_ctl_code: ScardIoCtlCode, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        match io_ctl_code {
            ScardIoCtlCode::AccessStartedEvent => Ok(ScardCall::AccessStartedEventCall(
                ScardAccessStartedEventCall::decode(src)?,
            )),
            ScardIoCtlCode::EstablishContext => Ok(ScardCall::EstablishContextCall(EstablishContextCall::decode(src)?)),
            ScardIoCtlCode::ListReaderGroupsW | ScardIoCtlCode::ListReaderGroupsA => {
                Ok(ScardCall::ListReaderGroupsCall(ListReaderGroupsCall::decode(src)?))
            }
            ScardIoCtlCode::ListReadersW => Ok(ScardCall::ListReadersCall(ListReadersCall::decode(
                src,
                Some(CharacterSet::Unicode),
            )?)),
            ScardIoCtlCode::ListReadersA => Ok(ScardCall::ListReadersCall(ListReadersCall::decode(
                src,
                Some(CharacterSet::Ansi),
            )?)),
            ScardIoCtlCode::GetStatusChangeW => Ok(ScardCall::GetStatusChangeCall(GetStatusChangeCall::decode(
                src,
                Some(CharacterSet::Unicode),
            )?)),
            ScardIoCtlCode::GetStatusChangeA => Ok(ScardCall::GetStatusChangeCall(GetStatusChangeCall::decode(
                src,
                Some(CharacterSet::Ansi),
            )?)),
            ScardIoCtlCode::LocateCardsW => Ok(ScardCall::LocateCardsCall(LocateCardsCall::decode(
                src,
                Some(CharacterSet::Unicode),
            )?)),
            ScardIoCtlCode::LocateCardsA => Ok(ScardCall::LocateCardsCall(LocateCardsCall::decode(
                src,
                Some(CharacterSet::Ansi),
            )?)),
            ScardIoCtlCode::LocateCardsByAtrW => Ok(ScardCall::LocateCardsByAtrCall(LocateCardsByAtrCall::decode(
                src,
                Some(CharacterSet::Unicode),
            )?)),
            ScardIoCtlCode::LocateCardsByAtrA => Ok(ScardCall::LocateCardsByAtrCall(LocateCardsByAtrCall::decode(
                src,
                Some(CharacterSet::Ansi),
            )?)),
            ScardIoCtlCode::ConnectW => Ok(ScardCall::ConnectCall(ConnectCall::decode(
                src,
                Some(CharacterSet::Unicode),
            )?)),
            ScardIoCtlCode::ConnectA => Ok(ScardCall::ConnectCall(ConnectCall::decode(
                src,
                Some(CharacterSet::Ansi),
            )?)),
            ScardIoCtlCode::Reconnect => Ok(ScardCall::ReconnectCall(ReconnectCall::decode(src)?)),
            ScardIoCtlCode::BeginTransaction => Ok(ScardCall::HCardAndDispositionCall(
                HCardAndDispositionCall::decode(src)?,
            )),
            ScardIoCtlCode::Transmit => Ok(ScardCall::TransmitCall(TransmitCall::decode(src)?)),
            ScardIoCtlCode::StatusW | ScardIoCtlCode::StatusA => Ok(ScardCall::StatusCall(StatusCall::decode(src)?)),
            ScardIoCtlCode::State => Ok(ScardCall::StateCall(StateCall::decode(src)?)),
            ScardIoCtlCode::Control => Ok(ScardCall::ControlCall(ControlCall::decode(src)?)),
            ScardIoCtlCode::GetAttrib => Ok(ScardCall::GetAttribCall(GetAttribCall::decode(src)?)),
            ScardIoCtlCode::SetAttrib => Ok(ScardCall::SetAttribCall(SetAttribCall::decode(src)?)),
            ScardIoCtlCode::GetTransmitCount => Ok(ScardCall::GetTransmitCountCall(GetTransmitCountCall::decode(src)?)),
            ScardIoCtlCode::ReleaseContext => Ok(ScardCall::ContextCall(ContextCall::decode(src)?)),
            ScardIoCtlCode::EndTransaction => Ok(ScardCall::HCardAndDispositionCall(HCardAndDispositionCall::decode(
                src,
            )?)),
            ScardIoCtlCode::Disconnect => Ok(ScardCall::HCardAndDispositionCall(HCardAndDispositionCall::decode(
                src,
            )?)),
            ScardIoCtlCode::Cancel => Ok(ScardCall::ContextCall(ContextCall::decode(src)?)),
            ScardIoCtlCode::IsValidContext => Ok(ScardCall::ContextCall(ContextCall::decode(src)?)),
            ScardIoCtlCode::GetDeviceTypeId => Ok(ScardCall::GetDeviceTypeIdCall(GetDeviceTypeIdCall::decode(src)?)),
            ScardIoCtlCode::ReadCacheW => Ok(ScardCall::ReadCacheCall(ReadCacheCall::decode(
                src,
                Some(CharacterSet::Unicode),
            )?)),
            ScardIoCtlCode::ReadCacheA => Ok(ScardCall::ReadCacheCall(ReadCacheCall::decode(
                src,
                Some(CharacterSet::Ansi),
            )?)),
            ScardIoCtlCode::WriteCacheW => Ok(ScardCall::WriteCacheCall(WriteCacheCall::decode(
                src,
                Some(CharacterSet::Unicode),
            )?)),
            ScardIoCtlCode::WriteCacheA => Ok(ScardCall::WriteCacheCall(WriteCacheCall::decode(
                src,
                Some(CharacterSet::Ansi),
            )?)),
            ScardIoCtlCode::GetReaderIcon => Ok(ScardCall::GetReaderIconCall(GetReaderIconCall::decode(src)?)),
            ScardIoCtlCode::IntroduceReaderGroupW
            | ScardIoCtlCode::ForgetReaderGroupW
            | ScardIoCtlCode::ForgetReaderW => Ok(ScardCall::ContextAndStringCall(ContextAndStringCall::decode(
                src,
                Some(CharacterSet::Unicode),
            )?)),
            ScardIoCtlCode::IntroduceReaderGroupA
            | ScardIoCtlCode::ForgetReaderGroupA
            | ScardIoCtlCode::ForgetReaderA => Ok(ScardCall::ContextAndStringCall(ContextAndStringCall::decode(
                src,
                Some(CharacterSet::Ansi),
            )?)),
            ScardIoCtlCode::IntroduceReaderW
            | ScardIoCtlCode::AddReaderToGroupW
            | ScardIoCtlCode::RemoveReaderFromGroupW => Ok(ScardCall::ContextAndTwoStringCall(
                ContextAndTwoStringCall::decode(src, Some(CharacterSet::Unicode))?,
            )),
            ScardIoCtlCode::IntroduceReaderA
            | ScardIoCtlCode::AddReaderToGroupA
            | ScardIoCtlCode::RemoveReaderFromGroupA => Ok(ScardCall::ContextAndTwoStringCall(
                ContextAndTwoStringCall::decode(src, Some(CharacterSet::Ansi))?,
            )),
            _ => {
                warn!(?io_ctl_code, "Unsupported ScardIoCtlCode");
                // TODO: maybe this should be an error
                Ok(Self::Unsupported)
            }
        }
    }
}

/// MS-RDPESC `cbContext` / `cbHandle` maximum (`range(0,16)`).
const SCARD_OPAQUE_MAX_LEN: u32 = 16;

fn scard_wire_len(length: u32) -> DecodeResult<u8> {
    if length > SCARD_OPAQUE_MAX_LEN {
        return Err(invalid_field_err!(
            "decode_ptr",
            "cbContext/cbHandle exceeds MS-RDPESC range 0..=16"
        ));
    }
    // INVARIANT: length <= 16 fits in u8.
    Ok(u8::try_from(length).expect("length <= 16 fits in u8"))
}

/// [2.2.1.1] REDIR_SCARDCONTEXT
///
/// Opaque client-owned context bytes. MS-RDPESC allows `cbContext` in `0..=16`;
/// Windows clients commonly emit 4 (x86) or 8 (x64) native `SCARDCONTEXT` bytes.
///
/// [2.2.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/060abee1-e520-4149-9ef7-ce79eb500a59
#[derive(Debug, PartialEq, Eq, Hash, Copy, Clone)]
pub struct ScardContext {
    /// Number of significant bytes in [`Self::bytes`] (`0..=16`).
    len: u8,
    /// Opaque context; only the first `len` bytes are on the wire.
    bytes: [u8; 16],
}

impl ScardContext {
    /// Creates a 4-byte context value (legacy synthetic-id encoding).
    pub fn new(value: u32) -> Self {
        let mut bytes = [0u8; 16];
        bytes[..4].copy_from_slice(&value.to_le_bytes());
        Self { len: 4, bytes }
    }

    /// Creates a context from opaque wire bytes (`0..=16`).
    pub fn from_opaque(opaque: &[u8]) -> DecodeResult<Self> {
        let len = scard_wire_len(u32::try_from(opaque.len()).unwrap_or(u32::MAX))?;
        let mut bytes = [0u8; 16];
        bytes[..opaque.len()].copy_from_slice(opaque);
        Ok(Self { len, bytes })
    }

    /// Wire length of `pbContext`.
    pub fn len(self) -> u8 {
        self.len
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Significant opaque bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    /// First four little-endian bytes as `u32` (0 when empty).
    pub fn value(self) -> u32 {
        if self.len == 0 {
            return 0;
        }
        let mut tmp = [0u8; 4];
        let n = usize::from(self.len).min(4);
        tmp[..n].copy_from_slice(&self.bytes[..n]);
        u32::from_le_bytes(tmp)
    }

    /// Creates a context from a native WinSCard `SCARDCONTEXT` (4 bytes on x86, 8 on x64).
    ///
    /// # Panics
    ///
    /// Panics if `size_of::<usize>()` does not fit in `u8` (never on supported targets).
    pub fn from_native(native: usize) -> Self {
        let le = native.to_le_bytes();
        let mut bytes = [0u8; 16];
        bytes[..le.len()].copy_from_slice(&le);
        Self {
            // INVARIANT: `size_of::<usize>()` is 4 or 8, both fit in `u8`.
            len: u8::try_from(le.len()).expect("usize byte length fits in u8"),
            bytes,
        }
    }

    /// Native WinSCard `SCARDCONTEXT` value (0 when empty).
    pub fn native(self) -> usize {
        let mut tmp = [0u8; size_of::<usize>()];
        let n = usize::from(self.len).min(size_of::<usize>());
        tmp[..n].copy_from_slice(&self.bytes[..n]);
        usize::from_le_bytes(tmp)
    }
}

impl ndr::Encode for ScardContext {
    fn encode_ptr(&self, index: &mut u32, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        // Empty context: cbContext=0 and NULL pointer (8 zero bytes), matching FreeRDP/mstscax.
        if self.len == 0 {
            ensure_size!(in: dst, size: ndr::ptr_size(true));
            dst.write_u32(0);
            dst.write_u32(0);
            return Ok(());
        }
        ndr::encode_ptr(Some(u32::from(self.len)), index, dst)
    }

    fn encode_value(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if self.len == 0 {
            return Ok(());
        }
        ensure_size!(in: dst, size: self.size_value());
        dst.write_u32(u32::from(self.len));
        dst.write_slice(self.as_bytes());
        Ok(())
    }

    fn size_ptr(&self) -> usize {
        ndr::ptr_size(true)
    }

    fn size_value(&self) -> usize {
        if self.len == 0 {
            0
        } else {
            4 /* length */ + usize::from(self.len) /* pbContext */
        }
    }
}

impl ndr::Decode for ScardContext {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> DecodeResult<Self>
    where
        Self: Sized,
    {
        ensure_size!(in: src, size: size_of::<u32>());
        let length = src.read_u32();
        let len = match scard_wire_len(length) {
            Ok(len) => len,
            Err(err) => {
                error!(?length, "Unsupported value length in ScardContext");
                return Err(err);
            }
        };

        let ptr = ndr::decode_ptr(src, index)?;
        if (length == 0) != (ptr == 0) {
            return Err(invalid_field_err!(
                "decode_ptr",
                "ScardContext cbContext/pbContext inconsistency"
            ));
        }
        Ok(Self { len, bytes: [0; 16] })
    }

    fn decode_value(&mut self, src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<()> {
        expect_no_charset(charset)?;
        if self.len == 0 {
            return Ok(());
        }
        ensure_size!(in: src, size: size_of::<u32>());
        let length = src.read_u32();
        if length != u32::from(self.len) {
            error!(?length, expected = self.len, "ScardContext length mismatch");
            return Err(invalid_field_err!("decode_value", "ScardContext length mismatch"));
        }
        let n = usize::from(self.len);
        ensure_size!(in: src, size: n);
        self.bytes[..n].copy_from_slice(src.read_slice(n));
        Ok(())
    }
}

/// [2.2.1.7] ReaderStateW
///
/// [2.2.1.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/0ba03cd2-bed0-495b-adbe-3d2cde61980c
#[derive(Debug, PartialEq, Clone)]
pub struct ReaderState {
    pub reader: String,
    pub common: ReaderStateCommonCall,
}

impl ndr::Decode for ReaderState {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> DecodeResult<Self> {
        let _reader_ptr = ndr::decode_ptr(src, index)?;
        let common = ReaderStateCommonCall::decode(src)?;
        Ok(Self {
            reader: String::new(),
            common,
        })
    }

    fn decode_value(&mut self, src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<()> {
        let charset = expect_charset(charset)?;
        self.reader = ndr::read_string_from_cursor(src, charset)?;
        Ok(())
    }
}

/// From [3.1.4] Message Processing Events and Sequencing Rules
///
/// [3.1.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/60d5977d-0017-4c90-ab0c-f34bf44a74a5
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u32)]
pub enum ScardIoCtlCode {
    /// SCARD_IOCTL_ESTABLISHCONTEXT
    EstablishContext = 0x0009_0014,
    /// SCARD_IOCTL_RELEASECONTEXT
    ReleaseContext = 0x0009_0018,
    /// SCARD_IOCTL_ISVALIDCONTEXT
    IsValidContext = 0x0009_001C,
    /// SCARD_IOCTL_LISTREADERGROUPSA
    ListReaderGroupsA = 0x0009_0020,
    /// SCARD_IOCTL_LISTREADERGROUPSW
    ListReaderGroupsW = 0x0009_0024,
    /// SCARD_IOCTL_LISTREADERSA
    ListReadersA = 0x0009_0028,
    /// SCARD_IOCTL_LISTREADERSW
    ListReadersW = 0x0009_002C,
    /// SCARD_IOCTL_INTRODUCEREADERGROUPA
    IntroduceReaderGroupA = 0x0009_0050,
    /// SCARD_IOCTL_INTRODUCEREADERGROUPW
    IntroduceReaderGroupW = 0x0009_0054,
    /// SCARD_IOCTL_FORGETREADERGROUPA
    ForgetReaderGroupA = 0x0009_0058,
    /// SCARD_IOCTL_FORGETREADERGROUPW
    ForgetReaderGroupW = 0x0009_005C,
    /// SCARD_IOCTL_INTRODUCEREADERA
    IntroduceReaderA = 0x0009_0060,
    /// SCARD_IOCTL_INTRODUCEREADERW
    IntroduceReaderW = 0x0009_0064,
    /// SCARD_IOCTL_FORGETREADERA
    ForgetReaderA = 0x0009_0068,
    /// SCARD_IOCTL_FORGETREADERW
    ForgetReaderW = 0x0009_006C,
    /// SCARD_IOCTL_ADDREADERTOGROUPA
    AddReaderToGroupA = 0x0009_0070,
    /// SCARD_IOCTL_ADDREADERTOGROUPW
    AddReaderToGroupW = 0x0009_0074,
    /// SCARD_IOCTL_REMOVEREADERFROMGROUPA
    RemoveReaderFromGroupA = 0x0009_0078,
    /// SCARD_IOCTL_REMOVEREADERFROMGROUPW
    RemoveReaderFromGroupW = 0x0009_007C,
    /// SCARD_IOCTL_LOCATECARDSA
    LocateCardsA = 0x0009_0098,
    /// SCARD_IOCTL_LOCATECARDSW
    LocateCardsW = 0x0009_009C,
    /// SCARD_IOCTL_GETSTATUSCHANGEA
    GetStatusChangeA = 0x0009_00A0,
    /// SCARD_IOCTL_GETSTATUSCHANGEW
    GetStatusChangeW = 0x0009_00A4,
    /// SCARD_IOCTL_CANCEL
    Cancel = 0x0009_00A8,
    /// SCARD_IOCTL_CONNECTA
    ConnectA = 0x0009_00AC,
    /// SCARD_IOCTL_CONNECTW
    ConnectW = 0x0009_00B0,
    /// SCARD_IOCTL_RECONNECT
    Reconnect = 0x0009_00B4,
    /// SCARD_IOCTL_DISCONNECT
    Disconnect = 0x0009_00B8,
    /// SCARD_IOCTL_BEGINTRANSACTION
    BeginTransaction = 0x0009_00BC,
    /// SCARD_IOCTL_ENDTRANSACTION
    EndTransaction = 0x0009_00C0,
    /// SCARD_IOCTL_STATE
    State = 0x0009_00C4,
    /// SCARD_IOCTL_STATUSA
    StatusA = 0x0009_00C8,
    /// SCARD_IOCTL_STATUSW
    StatusW = 0x0009_00CC,
    /// SCARD_IOCTL_TRANSMIT
    Transmit = 0x0009_00D0,
    /// SCARD_IOCTL_CONTROL
    Control = 0x0009_00D4,
    /// SCARD_IOCTL_GETATTRIB
    GetAttrib = 0x0009_00D8,
    /// SCARD_IOCTL_SETATTRIB
    SetAttrib = 0x0009_00DC,
    /// SCARD_IOCTL_ACCESSSTARTEDEVENT
    AccessStartedEvent = 0x0009_00E0,
    /// SCARD_IOCTL_RELEASETARTEDEVENT
    ReleaseTartedEvent = 0x0009_00E4,
    /// SCARD_IOCTL_LOCATECARDSBYATRA
    LocateCardsByAtrA = 0x0009_00E8,
    /// SCARD_IOCTL_LOCATECARDSBYATRW
    LocateCardsByAtrW = 0x0009_00EC,
    /// SCARD_IOCTL_READCACHEA
    ReadCacheA = 0x0009_00F0,
    /// SCARD_IOCTL_READCACHEW
    ReadCacheW = 0x0009_00F4,
    /// SCARD_IOCTL_WRITECACHEA
    WriteCacheA = 0x0009_00F8,
    /// SCARD_IOCTL_WRITECACHEW
    WriteCacheW = 0x0009_00FC,
    /// SCARD_IOCTL_GETTRANSMITCOUNT
    GetTransmitCount = 0x0009_0100,
    /// SCARD_IOCTL_GETREADERICON
    GetReaderIcon = 0x0009_0104,
    /// SCARD_IOCTL_GETDEVICETYPEID
    GetDeviceTypeId = 0x0009_0108,
}

impl TryFrom<u32> for ScardIoCtlCode {
    type Error = DecodeError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x0009_0014 => Ok(ScardIoCtlCode::EstablishContext),
            0x0009_0018 => Ok(ScardIoCtlCode::ReleaseContext),
            0x0009_001C => Ok(ScardIoCtlCode::IsValidContext),
            0x0009_0020 => Ok(ScardIoCtlCode::ListReaderGroupsA),
            0x0009_0024 => Ok(ScardIoCtlCode::ListReaderGroupsW),
            0x0009_0028 => Ok(ScardIoCtlCode::ListReadersA),
            0x0009_002C => Ok(ScardIoCtlCode::ListReadersW),
            0x0009_0050 => Ok(ScardIoCtlCode::IntroduceReaderGroupA),
            0x0009_0054 => Ok(ScardIoCtlCode::IntroduceReaderGroupW),
            0x0009_0058 => Ok(ScardIoCtlCode::ForgetReaderGroupA),
            0x0009_005C => Ok(ScardIoCtlCode::ForgetReaderGroupW),
            0x0009_0060 => Ok(ScardIoCtlCode::IntroduceReaderA),
            0x0009_0064 => Ok(ScardIoCtlCode::IntroduceReaderW),
            0x0009_0068 => Ok(ScardIoCtlCode::ForgetReaderA),
            0x0009_006C => Ok(ScardIoCtlCode::ForgetReaderW),
            0x0009_0070 => Ok(ScardIoCtlCode::AddReaderToGroupA),
            0x0009_0074 => Ok(ScardIoCtlCode::AddReaderToGroupW),
            0x0009_0078 => Ok(ScardIoCtlCode::RemoveReaderFromGroupA),
            0x0009_007C => Ok(ScardIoCtlCode::RemoveReaderFromGroupW),
            0x0009_0098 => Ok(ScardIoCtlCode::LocateCardsA),
            0x0009_009C => Ok(ScardIoCtlCode::LocateCardsW),
            0x0009_00A0 => Ok(ScardIoCtlCode::GetStatusChangeA),
            0x0009_00A4 => Ok(ScardIoCtlCode::GetStatusChangeW),
            0x0009_00A8 => Ok(ScardIoCtlCode::Cancel),
            0x0009_00AC => Ok(ScardIoCtlCode::ConnectA),
            0x0009_00B0 => Ok(ScardIoCtlCode::ConnectW),
            0x0009_00B4 => Ok(ScardIoCtlCode::Reconnect),
            0x0009_00B8 => Ok(ScardIoCtlCode::Disconnect),
            0x0009_00BC => Ok(ScardIoCtlCode::BeginTransaction),
            0x0009_00C0 => Ok(ScardIoCtlCode::EndTransaction),
            0x0009_00C4 => Ok(ScardIoCtlCode::State),
            0x0009_00C8 => Ok(ScardIoCtlCode::StatusA),
            0x0009_00CC => Ok(ScardIoCtlCode::StatusW),
            0x0009_00D0 => Ok(ScardIoCtlCode::Transmit),
            0x0009_00D4 => Ok(ScardIoCtlCode::Control),
            0x0009_00D8 => Ok(ScardIoCtlCode::GetAttrib),
            0x0009_00DC => Ok(ScardIoCtlCode::SetAttrib),
            0x0009_00E0 => Ok(ScardIoCtlCode::AccessStartedEvent),
            0x0009_00E4 => Ok(ScardIoCtlCode::ReleaseTartedEvent),
            0x0009_00E8 => Ok(ScardIoCtlCode::LocateCardsByAtrA),
            0x0009_00EC => Ok(ScardIoCtlCode::LocateCardsByAtrW),
            0x0009_00F0 => Ok(ScardIoCtlCode::ReadCacheA),
            0x0009_00F4 => Ok(ScardIoCtlCode::ReadCacheW),
            0x0009_00F8 => Ok(ScardIoCtlCode::WriteCacheA),
            0x0009_00FC => Ok(ScardIoCtlCode::WriteCacheW),
            0x0009_0100 => Ok(ScardIoCtlCode::GetTransmitCount),
            0x0009_0104 => Ok(ScardIoCtlCode::GetReaderIcon),
            0x0009_0108 => Ok(ScardIoCtlCode::GetDeviceTypeId),
            _ => {
                error!("Unsupported ScardIoCtlCode: 0x{:08x}", value);
                Err(invalid_field_err!("try_from", "ScardIoCtlCode", "unsupported value"))
            }
        }
    }
}

/// Allow [`ScardIoCtlCode`] to be used as an [`IoCtlCode`].
impl IoCtlCode for ScardIoCtlCode {}

/// [2.2.2.30] ScardAccessStartedEvent_Call
///
/// [2.2.2.30]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/c5ab8dd0-4914-4355-960c-0a527971ea69
#[derive(Debug, PartialEq, Clone)]
pub struct ScardAccessStartedEventCall;

impl ScardAccessStartedEventCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ironrdp_pdu::read_padding!(src, 4); // Unused (4 bytes)
        Ok(Self)
    }
}

/// [2.2.3.3] Long_Return
///
/// [2.2.3.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/e77a1365-2379-4037-99c4-d30d14ba10fc
#[derive(Debug, PartialEq, Clone)]
pub struct LongReturn {
    return_code: ReturnCode,
}

impl LongReturn {
    const NAME: &'static str = "Long_Return";

    pub fn new(return_code: ReturnCode) -> rpce::Pdu<Self> {
        rpce::Pdu(Self { return_code })
    }
}

impl rpce::HeaderlessEncode for LongReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size()
    }
}

/// [2.2.8] Return Code
///
/// [2.2.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/9861f8da-76fe-41e6-847e-40c9aa35df8d
#[derive(Debug, PartialEq, Clone, Copy)]
#[repr(u32)]
pub enum ReturnCode {
    /// SCARD_S_SUCCESS
    Success = 0x0000_0000,
    /// SCARD_F_INTERNAL_ERROR
    InternalError = 0x8010_0001,
    /// SCARD_E_CANCELLED
    Cancelled = 0x8010_0002,
    /// SCARD_E_INVALID_HANDLE
    InvalidHandle = 0x8010_0003,
    /// SCARD_E_INVALID_PARAMETER
    InvalidParameter = 0x8010_0004,
    /// SCARD_E_INVALID_TARGET
    InvalidTarget = 0x8010_0005,
    /// SCARD_E_NO_MEMORY
    NoMemory = 0x8010_0006,
    /// SCARD_F_WAITED_TOO_LONG
    WaitedTooLong = 0x8010_0007,
    /// SCARD_E_INSUFFICIENT_BUFFER
    InsufficientBuffer = 0x8010_0008,
    /// SCARD_E_UNKNOWN_READER
    UnknownReader = 0x8010_0009,
    /// SCARD_E_TIMEOUT
    Timeout = 0x8010_000A,
    /// SCARD_E_SHARING_VIOLATION
    SharingViolation = 0x8010_000B,
    /// SCARD_E_NO_SMARTCARD
    NoSmartcard = 0x8010_000C,
    /// SCARD_E_UNKNOWN_CARD
    UnknownCard = 0x8010_000D,
    /// SCARD_E_CANT_DISPOSE
    CantDispose = 0x8010_000E,
    /// SCARD_E_PROTO_MISMATCH
    ProtoMismatch = 0x8010_000F,
    /// SCARD_E_NOT_READY
    NotReady = 0x8010_0010,
    /// SCARD_E_INVALID_VALUE
    InvalidValue = 0x8010_0011,
    /// SCARD_E_SYSTEM_CANCELLED
    SystemCancelled = 0x8010_0012,
    /// SCARD_F_COMM_ERROR
    CommError = 0x8010_0013,
    /// SCARD_F_UNKNOWN_ERROR
    UnknownError = 0x8010_0014,
    /// SCARD_E_INVALID_ATR
    InvalidAtr = 0x8010_0015,
    /// SCARD_E_NOT_TRANSACTED
    NotTransacted = 0x8010_0016,
    /// SCARD_E_READER_UNAVAILABLE
    ReaderUnavailable = 0x8010_0017,
    /// SCARD_P_SHUTDOWN
    Shutdown = 0x8010_0018,
    /// SCARD_E_PCI_TOO_SMALL
    PciTooSmall = 0x8010_0019,
    /// SCARD_E_ICC_INSTALLATION
    IccInstallation = 0x8010_0020,
    /// SCARD_E_ICC_CREATEORDER
    IccCreateorder = 0x8010_0021,
    /// SCARD_E_UNSUPPORTED_FEATURE
    UnsupportedFeature = 0x8010_0022,
    /// SCARD_E_DIR_NOT_FOUND
    DirNotFound = 0x8010_0023,
    /// SCARD_E_FILE_NOT_FOUND
    FileNotFound = 0x8010_0024,
    /// SCARD_E_NO_DIR
    NoDir = 0x8010_0025,
    /// SCARD_E_READER_UNSUPPORTED
    ReaderUnsupported = 0x8010_001A,
    /// SCARD_E_DUPLICATE_READER
    DuplicateReader = 0x8010_001B,
    /// SCARD_E_CARD_UNSUPPORTED
    CardUnsupported = 0x8010_001C,
    /// SCARD_E_NO_SERVICE
    NoService = 0x8010_001D,
    /// SCARD_E_SERVICE_STOPPED
    ServiceStopped = 0x8010_001E,
    /// SCARD_E_UNEXPECTED
    Unexpected = 0x8010_001F,
    /// SCARD_E_NO_FILE
    NoFile = 0x8010_0026,
    /// SCARD_E_NO_ACCESS
    NoAccess = 0x8010_0027,
    /// SCARD_E_WRITE_TOO_MANY
    WriteTooMany = 0x8010_0028,
    /// SCARD_E_BAD_SEEK
    BadSeek = 0x8010_0029,
    /// SCARD_E_INVALID_CHV
    InvalidChv = 0x8010_002A,
    /// SCARD_E_UNKNOWN_RES_MSG
    UnknownResMsg = 0x8010_002B,
    /// SCARD_E_NO_SUCH_CERTIFICATE
    NoSuchCertificate = 0x8010_002C,
    /// SCARD_E_CERTIFICATE_UNAVAILABLE
    CertificateUnavailable = 0x8010_002D,
    /// SCARD_E_NO_READERS_AVAILABLE
    NoReadersAvailable = 0x8010_002E,
    /// SCARD_E_COMM_DATA_LOST
    CommDataLost = 0x8010_002F,
    /// SCARD_E_NO_KEY_CONTAINER
    NoKeyContainer = 0x8010_0030,
    /// SCARD_E_SERVER_TOO_BUSY
    ServerTooBusy = 0x8010_0031,
    /// SCARD_E_PIN_CACHE_EXPIRED
    PinCacheExpired = 0x8010_0032,
    /// SCARD_E_NO_PIN_CACHE
    NoPinCache = 0x8010_0033,
    /// SCARD_E_READ_ONLY_CARD
    ReadOnlyCard = 0x8010_0034,
    /// SCARD_W_UNSUPPORTED_CARD
    UnsupportedCard = 0x8010_0065,
    /// SCARD_W_UNRESPONSIVE_CARD
    UnresponsiveCard = 0x8010_0066,
    /// SCARD_W_UNPOWERED_CARD
    UnpoweredCard = 0x8010_0067,
    /// SCARD_W_RESET_CARD
    ResetCard = 0x8010_0068,
    /// SCARD_W_REMOVED_CARD
    RemovedCard = 0x8010_0069,
    /// SCARD_W_SECURITY_VIOLATION
    SecurityViolation = 0x8010_006A,
    /// SCARD_W_WRONG_CHV
    WrongChv = 0x8010_006B,
    /// SCARD_W_CHV_BLOCKED
    ChvBlocked = 0x8010_006C,
    /// SCARD_W_EOF
    Eof = 0x8010_006D,
    /// SCARD_W_CANCELLED_BY_USER
    CancelledByUser = 0x8010_006E,
    /// SCARD_W_CARD_NOT_AUTHENTICATED
    CardNotAuthenticated = 0x8010_006F,
    /// SCARD_W_CACHE_ITEM_NOT_FOUND
    CacheItemNotFound = 0x8010_0070,
    /// SCARD_W_CACHE_ITEM_STALE
    CacheItemStale = 0x8010_0071,
    /// SCARD_W_CACHE_ITEM_TOO_BIG
    CacheItemTooBig = 0x8010_0072,
}

impl ReturnCode {
    pub fn size(&self) -> usize {
        size_of::<u32>()
    }
}

impl From<ReturnCode> for u32 {
    #[expect(
        clippy::as_conversions,
        reason = "guarantees discriminant layout, and as is the only way to cast enum -> primitive"
    )]
    fn from(val: ReturnCode) -> Self {
        val as u32
    }
}

/// [2.2.2.1] EstablishContext_Call
///
/// [2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/b990635a-7637-464a-8923-361ed3e3d67a
#[derive(Debug, PartialEq, Clone)]
pub struct EstablishContextCall {
    pub scope: Scope,
}

impl EstablishContextCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }

    fn size() -> usize {
        size_of::<u32>()
    }
}

impl rpce::HeaderlessDecode for EstablishContextCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        ensure_size!(in: src, size: Self::size());
        let scope = Scope::try_from(src.read_u32())?;
        Ok(Self { scope })
    }
}

#[derive(Debug, PartialEq, Clone, Copy)]
#[repr(u32)]
pub enum Scope {
    User = 0x0000_0000,
    Terminal = 0x0000_0001,
    System = 0x0000_0002,
}

impl Scope {
    pub fn size(&self) -> usize {
        size_of::<u32>()
    }
}

impl TryFrom<u32> for Scope {
    type Error = DecodeError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x0000_0000 => Ok(Scope::User),
            0x0000_0001 => Ok(Scope::Terminal),
            0x0000_0002 => Ok(Scope::System),
            _ => {
                error!("Unsupported Scope: 0x{:08x}", value);
                Err(invalid_field_err!("try_from", "Scope", "unsupported value"))
            }
        }
    }
}

/// [2.2.3.2] EstablishContext_Return
///
/// [2.2.3.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/9135d95f-3740-411b-bdca-34ac7571fddc
#[derive(Debug, PartialEq, Clone)]
pub struct EstablishContextReturn {
    return_code: ReturnCode,
    context: ScardContext,
}

impl EstablishContextReturn {
    const NAME: &'static str = "EstablishContext_Return";

    pub fn new(return_code: ReturnCode, context: ScardContext) -> rpce::Pdu<Self> {
        rpce::Pdu(Self { return_code, context })
    }
}

impl rpce::HeaderlessEncode for EstablishContextReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        let mut index = 0;
        self.context.encode_ptr(&mut index, dst)?;
        self.context.encode_value(dst)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size() + self.context.size()
    }
}

/// [2.2.2.4] ListReaders_Call
///
/// [2.2.2.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/be2f46a5-77fb-40bf-839c-aed45f0a26d7
#[derive(Debug, PartialEq, Clone)]
pub struct ListReadersCall {
    pub context: ScardContext,
    pub groups_ptr_length: u32,
    pub groups_length: u32,
    pub groups_ptr: u32,
    pub groups: Vec<String>,
    pub readers_is_null: bool, // u32
    pub readers_size: u32,
}

impl ListReadersCall {
    pub fn decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, charset)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for ListReadersCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        let charset = expect_charset(charset)?;
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;

        ensure_size!(in: src, size: size_of::<u32>());
        let groups_ptr_length = src.read_u32();

        let groups_ptr = ndr::decode_ptr(src, &mut index)?;

        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let readers_is_null = (src.read_u32()) == 0x0000_0001;
        let readers_size = src.read_u32();

        context.decode_value(src, None)?;

        if groups_ptr == 0 {
            return Ok(Self {
                context,
                groups_ptr_length,
                groups_ptr,
                groups_length: 0,
                groups: Vec::new(),
                readers_is_null,
                readers_size,
            });
        }

        ensure_size!(in: src, size: size_of::<u32>());
        let groups_length = src.read_u32();
        if groups_length != groups_ptr_length {
            return Err(invalid_field_err!(
                "decode",
                "mismatched reader groups length in NDR pointer and value"
            ));
        }

        let groups = read_multistring_from_cursor(src, charset)?;

        Ok(Self {
            context,
            groups_ptr_length,
            groups_ptr,
            groups_length,
            groups,
            readers_is_null,
            readers_size,
        })
    }
}

/// [2.2.3.4] ListReaderGroups_Return and ListReaders_Return
///
/// [2.2.3.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/6630bb5b-fc0e-4141-8b53-263225c7628d
#[derive(Debug, PartialEq, Clone)]
pub struct ListReadersReturn {
    pub return_code: ReturnCode,
    pub encoding: CharacterSet,
    /// Wire `cBytes` (byte length of multistring when present).
    pub c_bytes: u32,
    /// `None` => NULL `msz` (length-only / insufficient-buffer probe).
    pub readers: Option<Vec<String>>,
}

impl ListReadersReturn {
    const NAME: &'static str = "ListReaders_Return";

    pub fn new(return_code: ReturnCode, readers: Vec<String>, encoding: CharacterSet) -> rpce::Pdu<Self> {
        let c_bytes = u32::try_from(encoded_multistring_len(&readers, encoding)).unwrap_or(u32::MAX);
        rpce::Pdu(Self {
            return_code,
            encoding,
            c_bytes,
            readers: Some(readers),
        })
    }

    pub fn probe(return_code: ReturnCode, c_bytes: u32, encoding: CharacterSet) -> rpce::Pdu<Self> {
        rpce::Pdu(Self {
            return_code,
            encoding,
            c_bytes,
            readers: None,
        })
    }
}

impl rpce::HeaderlessEncode for ListReadersReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        match &self.readers {
            Some(readers) => {
                let mut index = 0;
                ndr::encode_ptr(Some(self.c_bytes), &mut index, dst)?;
                dst.write_u32(self.c_bytes);
                write_multistring_to_cursor(dst, readers, self.encoding)?;
            }
            None => {
                dst.write_u32(self.c_bytes);
                dst.write_u32(0);
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size()
            + match &self.readers {
                Some(readers) => ndr::ptr_size(true) + 4 + encoded_multistring_len(readers, self.encoding),
                None => 8,
            }
    }
}

/// [2.2.2.12] GetStatusChangeW_Call
///
/// [2.2.2.12]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/af357ce8-63ee-4577-b6bf-c6f5ca68d754
#[derive(Debug, PartialEq, Clone)]
pub struct GetStatusChangeCall {
    pub context: ScardContext,
    pub timeout: u32,
    pub states_ptr_length: u32,
    pub states_ptr: u32,
    pub states_length: u32,
    pub states: Vec<ReaderState>,
}

impl GetStatusChangeCall {
    pub fn decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, charset)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for GetStatusChangeCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;

        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let timeout = src.read_u32();
        let states_ptr_length = src.read_u32();

        let states_ptr = ndr::decode_ptr(src, &mut index)?;

        context.decode_value(src, None)?;

        ensure_size!(in: src, size: size_of::<u32>());
        let states_length = src.read_u32();

        let mut states = Vec::new();
        for _ in 0..states_length {
            let state = ReaderState::decode_ptr(src, &mut index)?;
            states.push(state);
        }
        for state in states.iter_mut() {
            state.decode_value(src, charset)?;
        }

        Ok(Self {
            context,
            timeout,
            states_ptr_length,
            states_ptr,
            states_length,
            states,
        })
    }
}

/// [2.2.1.5] ReaderState_Common_Call
///
/// [2.2.1.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/a71e63ba-e58f-487c-a5d2-5a3e48856594
#[derive(Debug, PartialEq, Clone)]
pub struct ReaderStateCommonCall {
    pub current_state: CardStateFlags,
    pub event_state: CardStateFlags,
    pub atr_length: u32,
    pub atr: [u8; 36],
}

impl ReaderStateCommonCall {
    const FIXED_PART_SIZE: usize = size_of::<u32>() * 3 /* dwCurrentState, dwEventState, cbAtr */ + 36 /* rgbAtr */;

    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::FIXED_PART_SIZE);
        let current_state = CardStateFlags::from_bits_retain(src.read_u32());
        let event_state = CardStateFlags::from_bits_retain(src.read_u32());
        let atr_length = src.read_u32();
        let atr = src.read_array::<36>();

        Ok(Self {
            current_state,
            event_state,
            atr_length,
            atr,
        })
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        dst.write_u32(self.current_state.bits());
        dst.write_u32(self.event_state.bits());
        dst.write_u32(self.atr_length);
        dst.write_slice(&self.atr);
        Ok(())
    }

    fn size() -> usize {
        Self::FIXED_PART_SIZE
    }
}

bitflags! {
    #[derive(Debug, PartialEq, Clone, Copy)]
    pub struct CardStateFlags: u32 {
        const SCARD_STATE_UNAWARE = 0x0000_0000;
        const SCARD_STATE_IGNORE = 0x0000_0001;
        const SCARD_STATE_CHANGED = 0x0000_0002;
        const SCARD_STATE_UNKNOWN = 0x0000_0004;
        const SCARD_STATE_UNAVAILABLE = 0x0000_0008;
        const SCARD_STATE_EMPTY = 0x0000_0010;
        const SCARD_STATE_PRESENT = 0x0000_0020;
        const SCARD_STATE_ATRMATCH = 0x0000_0040;
        const SCARD_STATE_EXCLUSIVE = 0x0000_0080;
        const SCARD_STATE_INUSE = 0x0000_0100;
        const SCARD_STATE_MUTE = 0x0000_0200;
        const SCARD_STATE_UNPOWERED = 0x0000_0400;

        const _ = !0;
    }
}

/// [2.2.1.4] LocateCards_ATRMask
///
/// [2.2.1.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/479fe1cf-eaf0-4d51-8964-d4195a61f573
#[derive(Debug, PartialEq, Clone)]
pub struct LocateCardsAtrMask {
    pub atr_length: u32,
    pub atr: [u8; 36],
    pub mask: [u8; 36],
}

impl LocateCardsAtrMask {
    const FIXED_PART_SIZE: usize = 4 /* cbAtr */ + 36 /* rgbAtr */ + 36 /* rgbMask */;

    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::FIXED_PART_SIZE);
        let atr_length = src.read_u32();
        // MS-RDPESC: cbAtr range(0,36)
        if atr_length > 36 {
            return Err(invalid_field_err!("decode", "LocateCards_ATRMask cbAtr out of range"));
        }
        let atr = src.read_array::<36>();
        let mask = src.read_array::<36>();
        Ok(Self { atr_length, atr, mask })
    }
}

/// [2.2.2.9] LocateCardsA_Call / [2.2.2.10] LocateCardsW_Call
///
/// [2.2.2.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/c6b49a98-99e6-43c0-af63-56e4918814f3
/// [2.2.2.10]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/c40fb671-6a50-4ae1-b75d-f44b25612eb2
#[derive(Debug, PartialEq, Clone)]
pub struct LocateCardsCall {
    pub context: ScardContext,
    pub cards_ptr_length: u32,
    pub cards_ptr: u32,
    pub cards_length: u32,
    pub cards: Vec<String>,
    pub states_ptr_length: u32,
    pub states_ptr: u32,
    pub states_length: u32,
    pub states: Vec<ReaderState>,
}

impl LocateCardsCall {
    pub fn decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, charset)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for LocateCardsCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        let charset = expect_charset(charset)?;
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;

        ensure_size!(in: src, size: size_of::<u32>());
        let cards_ptr_length = src.read_u32();
        let cards_ptr = ndr::decode_ptr(src, &mut index)?;

        ensure_size!(in: src, size: size_of::<u32>());
        let states_ptr_length = src.read_u32();
        let states_ptr = ndr::decode_ptr(src, &mut index)?;

        context.decode_value(src, None)?;

        let (cards_length, cards) = if cards_ptr == 0 {
            (0, Vec::new())
        } else {
            ensure_size!(in: src, size: size_of::<u32>());
            let cards_length = src.read_u32();
            if cards_length != cards_ptr_length {
                return Err(invalid_field_err!(
                    "decode",
                    "mismatched cards length in NDR pointer and value"
                ));
            }
            // MS-RDPESC: cBytes range(0, 65536)
            if cards_length > 65_536 {
                return Err(invalid_field_err!("decode", "LocateCards cBytes out of range"));
            }
            let cards_len: usize = cast_length!("LocateCardsCall", "cards_length", cards_length)?;
            ensure_size!(in: src, size: cards_len);
            // Limit the multistring parser to the declared byte count so it cannot
            // consume subsequent NDR fields (for example reader states).
            let mut cards_src = ReadCursor::new(src.read_slice(cards_len));
            let cards = read_multistring_from_cursor(&mut cards_src, charset)?;
            // Pad only when another NDR field follows; stub tail is RPCE packing.
            if states_ptr != 0 {
                ndr::skip_pad(src)?;
            }
            (cards_length, cards)
        };

        let (states_length, states) = if states_ptr == 0 {
            (0, Vec::new())
        } else {
            ensure_size!(in: src, size: size_of::<u32>());
            let states_length = src.read_u32();
            // MS-RDPESC: cReaders range(0,10)
            if states_length > 10 {
                return Err(invalid_field_err!("decode", "LocateCards cReaders out of range"));
            }
            if states_length != states_ptr_length {
                return Err(invalid_field_err!(
                    "decode",
                    "mismatched states length in NDR pointer and value"
                ));
            }
            let mut states = Vec::new();
            for _ in 0..states_length {
                states.push(ReaderState::decode_ptr(src, &mut index)?);
            }
            for state in states.iter_mut() {
                state.decode_value(src, Some(charset))?;
            }
            (states_length, states)
        };

        Ok(Self {
            context,
            cards_ptr_length,
            cards_ptr,
            cards_length,
            cards,
            states_ptr_length,
            states_ptr,
            states_length,
            states,
        })
    }
}

/// [2.2.2.23] LocateCardsByATRA_Call / [2.2.2.24] LocateCardsByATRW_Call
///
/// [2.2.2.23]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/100a5cc6-cb6a-4f90-b0e4-659b872c26d5
/// [2.2.2.24]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/c934cc70-c1c9-4193-8b1b-038d8055c000
#[derive(Debug, PartialEq, Clone)]
pub struct LocateCardsByAtrCall {
    pub context: ScardContext,
    pub atr_masks_ptr_length: u32,
    pub atr_masks_ptr: u32,
    pub atr_masks_length: u32,
    pub atr_masks: Vec<LocateCardsAtrMask>,
    pub states_ptr_length: u32,
    pub states_ptr: u32,
    pub states_length: u32,
    pub states: Vec<ReaderState>,
}

impl LocateCardsByAtrCall {
    pub fn decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, charset)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for LocateCardsByAtrCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;

        ensure_size!(in: src, size: size_of::<u32>());
        let atr_masks_ptr_length = src.read_u32();
        let atr_masks_ptr = ndr::decode_ptr(src, &mut index)?;

        ensure_size!(in: src, size: size_of::<u32>());
        let states_ptr_length = src.read_u32();
        let states_ptr = ndr::decode_ptr(src, &mut index)?;

        context.decode_value(src, None)?;

        let (atr_masks_length, atr_masks) = if atr_masks_ptr == 0 {
            (0, Vec::new())
        } else {
            ensure_size!(in: src, size: size_of::<u32>());
            let atr_masks_length = src.read_u32();
            if atr_masks_length != atr_masks_ptr_length {
                return Err(invalid_field_err!(
                    "decode",
                    "mismatched ATR mask length in NDR pointer and value"
                ));
            }
            // MS-RDPESC: cAtrs range(0,1000)
            if atr_masks_length > 1000 {
                return Err(invalid_field_err!("decode", "LocateCardsByATR cAtrs out of range"));
            }
            let mut atr_masks = Vec::new();
            for _ in 0..atr_masks_length {
                atr_masks.push(LocateCardsAtrMask::decode(src)?);
            }
            (atr_masks_length, atr_masks)
        };

        let (states_length, states) = if states_ptr == 0 {
            (0, Vec::new())
        } else {
            ensure_size!(in: src, size: size_of::<u32>());
            let states_length = src.read_u32();
            // MS-RDPESC: cReaders range(0,10)
            if states_length > 10 {
                return Err(invalid_field_err!("decode", "LocateCardsByATR cReaders out of range"));
            }
            if states_length != states_ptr_length {
                return Err(invalid_field_err!(
                    "decode",
                    "mismatched states length in NDR pointer and value"
                ));
            }
            let mut states = Vec::new();
            for _ in 0..states_length {
                states.push(ReaderState::decode_ptr(src, &mut index)?);
            }
            for state in states.iter_mut() {
                state.decode_value(src, charset)?;
            }
            (states_length, states)
        };

        Ok(Self {
            context,
            atr_masks_ptr_length,
            atr_masks_ptr,
            atr_masks_length,
            atr_masks,
            states_ptr_length,
            states_ptr,
            states_length,
            states,
        })
    }
}

/// [2.2.3.5] LocateCards_Return and GetStatusChange_Return
///
/// [2.2.3.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/7b73e0c2-e0fc-46b1-9b03-50684ad2beba
#[derive(Debug, PartialEq, Clone)]
pub struct GetStatusChangeReturn {
    pub return_code: ReturnCode,
    pub reader_states: Vec<ReaderStateCommonCall>,
}

impl GetStatusChangeReturn {
    const NAME: &'static str = "GetStatusChange_Return";

    pub fn new(return_code: ReturnCode, reader_states: Vec<ReaderStateCommonCall>) -> rpce::Pdu<Self> {
        rpce::Pdu(Self {
            return_code,
            reader_states,
        })
    }
}

impl rpce::HeaderlessEncode for GetStatusChangeReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        let reader_states_len = cast_length!("GetStatusChangeReturn", "reader_states", self.reader_states.len())?;
        let mut index = 0;
        ndr::encode_ptr(Some(reader_states_len), &mut index, dst)?;
        dst.write_u32(reader_states_len);
        for reader_state in &self.reader_states {
            reader_state.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size() // dst.write_u32(self.return_code.into());
        + ndr::ptr_size(true) // ndr::encode_ptr(Some(reader_states_len), &mut index, dst)?;
        + 4 // dst.write_u32(reader_states_len);
        + self.reader_states.iter().map(|_s| ReaderStateCommonCall::size()).sum::<usize>()
    }
}

/// [2.2.2.14] ConnectW_Call
///
/// [2.2.2.14]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/fd06f6a0-a9ea-478c-9b5e-470fd9cde5a6
#[derive(Debug, PartialEq, Clone)]
pub struct ConnectCall {
    pub reader: String,
    pub common: ConnectCommon,
}

impl ConnectCall {
    pub fn decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, charset)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for ConnectCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        let charset = expect_charset(charset)?;
        let mut index = 0;
        let _reader_ptr = ndr::decode_ptr(src, &mut index)?;
        let mut common = ConnectCommon::decode_ptr(src, &mut index)?;
        let reader = ndr::read_string_from_cursor(src, charset)?;
        common.decode_value(src, None)?;
        Ok(Self { reader, common })
    }
}

/// [2.2.1.3] Connect_Common
///
/// [2.2.1.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/32752f32-4410-4682-b9fc-9096674b52de
#[derive(Debug, PartialEq, Clone)]
pub struct ConnectCommon {
    pub context: ScardContext,
    pub share_mode: u32,
    pub preferred_protocols: CardProtocol,
}

impl ndr::Decode for ConnectCommon {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> DecodeResult<Self>
    where
        Self: Sized,
    {
        let context = ScardContext::decode_ptr(src, index)?;
        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let share_mode = src.read_u32();
        let preferred_protocols = CardProtocol::from_bits_retain(src.read_u32());
        Ok(Self {
            context,
            share_mode,
            preferred_protocols,
        })
    }

    fn decode_value(&mut self, src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<()> {
        expect_no_charset(charset)?;
        self.context.decode_value(src, None)
    }
}

bitflags! {
    /// [2.2.5] Protocol Identifier
    ///
    /// [2.2.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/41673567-2710-4e86-be87-7b6f46fe10af
    #[derive(Debug, PartialEq, Clone)]
    pub struct CardProtocol: u32 {
        const SCARD_PROTOCOL_UNDEFINED = 0x0000_0000;
        const SCARD_PROTOCOL_T0 = 0x0000_0001;
        const SCARD_PROTOCOL_T1 = 0x0000_0002;
        const SCARD_PROTOCOL_TX = 0x0000_0003;
        const SCARD_PROTOCOL_RAW = 0x0001_0000;
        const SCARD_PROTOCOL_DEFAULT = 0x8000_0000;
        const SCARD_PROTOCOL_OPTIMAL = 0x0000_0000;

        const _ = !0;
    }
}

/// [2.2.1.2] REDIR_SCARDHANDLE
///
/// Reader handle associated with a [`ScardContext`]. MS-RDPESC allows `cbHandle`
/// in `0..=16`; Windows clients commonly emit 4 or 8 native `SCARDHANDLE` bytes.
///
/// [2.2.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/b6276356-7c5f-4d3e-be92-a6c85e58d008
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct ScardHandle {
    context: ScardContext,
    /// Number of significant bytes in [`Self::bytes`] (`0..=16`).
    len: u8,
    /// Opaque handle; only the first `len` bytes are on the wire.
    bytes: [u8; 16],
}

impl ScardHandle {
    /// Creates a 4-byte handle value (legacy synthetic-id encoding).
    pub fn new(context: ScardContext, value: u32) -> Self {
        let mut bytes = [0u8; 16];
        bytes[..4].copy_from_slice(&value.to_le_bytes());
        Self { context, len: 4, bytes }
    }

    /// Creates a handle from opaque wire bytes (`0..=16`).
    pub fn from_opaque(context: ScardContext, opaque: &[u8]) -> DecodeResult<Self> {
        let len = scard_wire_len(u32::try_from(opaque.len()).unwrap_or(u32::MAX))?;
        let mut bytes = [0u8; 16];
        bytes[..opaque.len()].copy_from_slice(opaque);
        Ok(Self { context, len, bytes })
    }

    /// Context that owns this handle.
    pub fn context(self) -> ScardContext {
        self.context
    }

    /// Wire length of `pbHandle`.
    pub fn len(self) -> u8 {
        self.len
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Significant opaque bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..usize::from(self.len)]
    }

    /// First four little-endian bytes as `u32` (0 when empty).
    pub fn value(self) -> u32 {
        if self.len == 0 {
            return 0;
        }
        let mut tmp = [0u8; 4];
        let n = usize::from(self.len).min(4);
        tmp[..n].copy_from_slice(&self.bytes[..n]);
        u32::from_le_bytes(tmp)
    }

    /// Creates a handle from a native WinSCard `SCARDHANDLE` (4 bytes on x86, 8 on x64).
    ///
    /// # Panics
    ///
    /// Panics if `size_of::<usize>()` does not fit in `u8` (never on supported targets).
    pub fn from_native(context: ScardContext, native: usize) -> Self {
        let le = native.to_le_bytes();
        let mut bytes = [0u8; 16];
        bytes[..le.len()].copy_from_slice(&le);
        Self {
            context,
            // INVARIANT: `size_of::<usize>()` is 4 or 8, both fit in `u8`.
            len: u8::try_from(le.len()).expect("usize byte length fits in u8"),
            bytes,
        }
    }

    /// Native WinSCard `SCARDHANDLE` value (0 when empty).
    pub fn native(self) -> usize {
        let mut tmp = [0u8; size_of::<usize>()];
        let n = usize::from(self.len).min(size_of::<usize>());
        tmp[..n].copy_from_slice(&self.bytes[..n]);
        usize::from_le_bytes(tmp)
    }
}

impl ndr::Decode for ScardHandle {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> DecodeResult<Self>
    where
        Self: Sized,
    {
        let context = ScardContext::decode_ptr(src, index)?;
        ensure_size!(ctx: "ScardHandle::decode_ptr", in: src, size: size_of::<u32>());
        let length = src.read_u32();
        let len = match scard_wire_len(length) {
            Ok(len) => len,
            Err(err) => {
                error!(?length, "Unsupported value length in ScardHandle");
                return Err(err);
            }
        };
        let ptr = ndr::decode_ptr(src, index)?;
        if (length == 0) != (ptr == 0) {
            return Err(invalid_field_err!(
                "decode_ptr",
                "ScardHandle cbHandle/pbHandle inconsistency"
            ));
        }
        Ok(Self {
            context,
            len,
            bytes: [0; 16],
        })
    }

    fn decode_value(&mut self, src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<()> {
        expect_no_charset(charset)?;
        self.context.decode_value(src, None)?;
        if self.len == 0 {
            return Ok(());
        }
        ensure_size!(in: src, size: size_of::<u32>());
        let length = src.read_u32();
        if length != u32::from(self.len) {
            error!(?length, expected = self.len, "ScardHandle length mismatch");
            return Err(invalid_field_err!("decode_value", "ScardHandle length mismatch"));
        }
        let n = usize::from(self.len);
        ensure_size!(in: src, size: n);
        self.bytes[..n].copy_from_slice(src.read_slice(n));
        Ok(())
    }
}

impl ndr::Encode for ScardHandle {
    fn encode_ptr(&self, index: &mut u32, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        self.context.encode_ptr(index, dst)?;
        if self.len == 0 {
            ensure_size!(in: dst, size: ndr::ptr_size(true));
            dst.write_u32(0);
            dst.write_u32(0);
            return Ok(());
        }
        ndr::encode_ptr(Some(u32::from(self.len)), index, dst)?;
        Ok(())
    }

    fn encode_value(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size_value());
        self.context.encode_value(dst)?;
        if self.len == 0 {
            return Ok(());
        }
        dst.write_u32(u32::from(self.len));
        dst.write_slice(self.as_bytes());
        Ok(())
    }

    fn size_ptr(&self) -> usize {
        self.context.size_ptr() + ndr::ptr_size(true)
    }

    fn size_value(&self) -> usize {
        let handle_value = if self.len == 0 {
            0
        } else {
            4 /* length */ + usize::from(self.len) /* pbHandle */
        };
        self.context.size_value() + handle_value
    }
}

/// [2.2.3.8] Connect_Return
///
/// [2.2.3.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/ad9fbc8e-0963-44ac-8d71-38021685790c
#[derive(Debug, PartialEq, Clone)]
pub struct ConnectReturn {
    pub return_code: ReturnCode,
    pub handle: ScardHandle,
    pub active_protocol: CardProtocol,
}

impl ConnectReturn {
    const NAME: &'static str = "Connect_Return";

    pub fn new(return_code: ReturnCode, handle: ScardHandle, active_protocol: CardProtocol) -> rpce::Pdu<Self> {
        rpce::Pdu(Self {
            return_code,
            handle,
            active_protocol,
        })
    }
}

impl rpce::HeaderlessEncode for ConnectReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        let mut index = 0;
        self.handle.encode_ptr(&mut index, dst)?;
        dst.write_u32(self.active_protocol.bits());
        self.handle.encode_value(dst)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size() + self.handle.size() + 4 /* dwActiveProtocol */
    }
}

/// [2.2.2.16] HCardAndDisposition_Call
///
/// [2.2.2.16]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/f15ae865-9e99-4c5b-bb43-15a6b4885bd0
#[derive(Debug, PartialEq, Clone)]
pub struct HCardAndDispositionCall {
    pub handle: ScardHandle,
    pub disposition: u32,
}

impl HCardAndDispositionCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for HCardAndDispositionCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        let mut index = 0;
        let mut handle = ScardHandle::decode_ptr(src, &mut index)?;
        ensure_size!(in: src, size: size_of::<u32>());
        let disposition = src.read_u32();
        handle.decode_value(src, None)?;
        Ok(Self { handle, disposition })
    }
}

/// [2.2.2.19] Transmit_Call
///
/// [2.2.2.19]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/e3861cfa-e61b-4d64-b19d-f6b31e076beb
#[derive(Debug, PartialEq, Clone)]
pub struct TransmitCall {
    pub handle: ScardHandle,
    pub send_pci: SCardIORequest,
    pub send_length: u32,
    pub send_buffer: Vec<u8>,
    pub recv_pci: Option<SCardIORequest>,
    pub recv_buffer_is_null: bool,
    pub recv_length: u32,
}

impl TransmitCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for TransmitCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        let mut index = 0;
        let mut handle = ScardHandle::decode_ptr(src, &mut index)?;
        let mut send_pci = SCardIORequest::decode_ptr(src, &mut index)?;
        ensure_size!(in: src, size: size_of::<u32>());
        let _send_length = src.read_u32();
        let _send_buffer_ptr = ndr::decode_ptr(src, &mut index)?;
        let recv_pci_ptr = ndr::decode_ptr(src, &mut index)?;
        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let recv_buffer_is_null = src.read_u32() == 1;
        let recv_length = src.read_u32();

        handle.decode_value(src, None)?;
        send_pci.decode_value(src, None)?;

        ensure_size!(in: src, size: size_of::<u32>());
        let send_length = src.read_u32();
        let send_length_usize: usize = cast_length!("TransmitCall", "send_length", send_length)?;
        ensure_size!(in: src, size: send_length_usize);
        let send_buffer = src.read_slice(send_length_usize).to_vec();

        let recv_pci = if recv_pci_ptr != 0 {
            let mut recv_pci = SCardIORequest::decode_ptr(src, &mut index)?;
            recv_pci.decode_value(src, None)?;
            Some(recv_pci)
        } else {
            None
        };

        Ok(Self {
            handle,
            send_pci,
            send_length,
            send_buffer,
            recv_pci,
            recv_buffer_is_null,
            recv_length,
        })
    }
}

/// [2.2.1.8] SCardIO_Request
///
/// [2.2.1.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/f6e15da8-5bc0-4ef6-b28a-ce88e8415621
#[derive(Debug, PartialEq, Clone)]
pub struct SCardIORequest {
    pub protocol: CardProtocol,
    pub extra_bytes_length: usize,
    pub extra_bytes: Vec<u8>,
}

impl ndr::Decode for SCardIORequest {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> DecodeResult<Self>
    where
        Self: Sized,
    {
        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let protocol = CardProtocol::from_bits_retain(src.read_u32());
        let extra_bytes_length = cast_length!("SCardIORequest", "extra_bytes_length", src.read_u32())?;
        let _extra_bytes_ptr = ndr::decode_ptr(src, index)?;
        let extra_bytes = Vec::new();
        Ok(Self {
            protocol,
            extra_bytes_length,
            extra_bytes,
        })
    }

    fn decode_value(&mut self, src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<()> {
        expect_no_charset(charset)?;
        ensure_size!(in: src, size: self.extra_bytes_length);
        self.extra_bytes = src.read_slice(self.extra_bytes_length).to_vec();
        Ok(())
    }
}

impl ndr::Encode for SCardIORequest {
    fn encode_ptr(&self, index: &mut u32, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size_ptr());

        let extra_bytes_length = cast_length!("SCardIORequest", "extra_bytes_length", self.extra_bytes_length)?;

        dst.write_u32(self.protocol.bits());
        ndr::encode_ptr(Some(extra_bytes_length), index, dst)
    }

    fn encode_value(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size_value());
        dst.write_slice(&self.extra_bytes);
        Ok(())
    }

    fn size_ptr(&self) -> usize {
        4 /* dwProtocol */ + ndr::ptr_size(true)
    }

    fn size_value(&self) -> usize {
        self.extra_bytes_length
    }
}

/// [2.2.3.11] Transmit_Return
///
/// [2.2.3.11]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/252cffd0-58b8-434d-9e1b-0d547544fb0f
#[derive(Debug, PartialEq, Clone)]
pub struct TransmitReturn {
    pub return_code: ReturnCode,
    pub recv_pci: Option<SCardIORequest>,
    /// `None` => NULL `pbRecvBuffer`; `recv_len` supplies `cbRecvLength`.
    pub recv_buffer: Option<Vec<u8>>,
    pub recv_len: u32,
}

impl TransmitReturn {
    const NAME: &'static str = "Transmit_Return";

    pub fn new(return_code: ReturnCode, recv_pci: Option<SCardIORequest>, recv_buffer: Vec<u8>) -> rpce::Pdu<Self> {
        let recv_len = u32::try_from(recv_buffer.len()).unwrap_or(u32::MAX);
        rpce::Pdu(Self {
            return_code,
            recv_pci,
            recv_buffer: Some(recv_buffer),
            recv_len,
        })
    }

    pub fn recv_probe(return_code: ReturnCode, recv_pci: Option<SCardIORequest>, recv_len: u32) -> rpce::Pdu<Self> {
        rpce::Pdu(Self {
            return_code,
            recv_pci,
            recv_buffer: None,
            recv_len,
        })
    }
}

impl rpce::HeaderlessEncode for TransmitReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        let mut index = 0;
        if let Some(recv_pci) = &self.recv_pci {
            recv_pci.encode_ptr(&mut index, dst)?;
            recv_pci.encode_value(dst)?;
        } else {
            dst.write_u32(0);
        }
        match &self.recv_buffer {
            Some(buf) => {
                let n: u32 = cast_length!("TransmitReturn", "recv_buffer_len", buf.len())?;
                ndr::encode_ptr(Some(n), &mut index, dst)?;
                dst.write_u32(n);
                dst.write_slice(buf);
            }
            None => {
                dst.write_u32(self.recv_len);
                dst.write_u32(0);
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size()
            + self.recv_pci.as_ref().map_or(4, SCardIORequest::size)
            + match &self.recv_buffer {
                Some(buf) => ndr::ptr_size(true) + 4 + buf.len(),
                None => 8,
            }
    }
}

/// [2.2.2.18] Status_Call
///
/// [2.2.2.18]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/f1139aed-e578-47f3-a800-f36b56c80500
#[derive(Debug, PartialEq, Clone)]
pub struct StatusCall {
    pub handle: ScardHandle,
    pub reader_names_is_null: bool,
    pub reader_length: u32,
    pub atr_length: u32,
}

impl StatusCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for StatusCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        let mut index = 0;
        let mut handle = ScardHandle::decode_ptr(src, &mut index)?;
        ensure_size!(in: src, size: size_of::<u32>() * 3);
        let reader_names_is_null = src.read_u32() == 1;
        let reader_length = src.read_u32();
        let atr_length = src.read_u32();
        handle.decode_value(src, None)?;
        Ok(Self {
            handle,
            reader_names_is_null,
            reader_length,
            atr_length,
        })
    }
}

/// [2.2.3.10] Status_Return
///
/// [2.2.3.10]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/987c1358-ad6b-4c8e-88e1-06210c28a66f
#[derive(Debug, PartialEq, Clone)]
pub struct StatusReturn {
    pub return_code: ReturnCode,
    /// `None` => NULL reader-name multistring (probe / insufficient buffer).
    pub reader_names: Option<Vec<String>>,
    pub reader_c_bytes: u32,
    pub state: CardState,
    pub protocol: CardProtocol,
    pub atr: [u8; 32],
    pub atr_length: u32,
    pub encoding: CharacterSet,
}

impl StatusReturn {
    const NAME: &'static str = "Status_Return";

    pub fn new(
        return_code: ReturnCode,
        reader_names: Vec<String>,
        state: CardState,
        protocol: CardProtocol,
        atr: [u8; 32],
        atr_length: u32,
        encoding: CharacterSet,
    ) -> rpce::Pdu<Self> {
        let reader_c_bytes = u32::try_from(encoded_multistring_len(&reader_names, encoding)).unwrap_or(u32::MAX);
        rpce::Pdu(Self {
            return_code,
            reader_names: Some(reader_names),
            reader_c_bytes,
            state,
            protocol,
            atr,
            atr_length,
            encoding,
        })
    }

    pub fn names_probe(
        return_code: ReturnCode,
        reader_c_bytes: u32,
        state: CardState,
        protocol: CardProtocol,
        atr: [u8; 32],
        atr_length: u32,
        encoding: CharacterSet,
    ) -> rpce::Pdu<Self> {
        rpce::Pdu(Self {
            return_code,
            reader_names: None,
            reader_c_bytes,
            state,
            protocol,
            atr,
            atr_length,
            encoding,
        })
    }
}

impl rpce::HeaderlessEncode for StatusReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        match &self.reader_names {
            Some(names) => {
                let mut index = 0;
                ndr::encode_ptr(Some(self.reader_c_bytes), &mut index, dst)?;
                dst.write_u32(self.state.into());
                dst.write_u32(self.protocol.bits());
                dst.write_slice(&self.atr);
                dst.write_u32(self.atr_length);
                dst.write_u32(self.reader_c_bytes);
                write_multistring_to_cursor(dst, names, self.encoding)?;
            }
            None => {
                dst.write_u32(self.reader_c_bytes);
                dst.write_u32(0);
                dst.write_u32(self.state.into());
                dst.write_u32(self.protocol.bits());
                dst.write_slice(&self.atr);
                dst.write_u32(self.atr_length);
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        // return + (names ptr/len) + state + protocol + atr[32] + atr_len [+ names bytes]
        4 + 8
            + 4
            + 4
            + 32
            + 4
            + self
                .reader_names
                .as_ref()
                .map_or(0, |n| encoded_multistring_len(n, self.encoding))
    }
}

/// [2.2.4] Card/Reader State
///
/// [2.2.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/264bc504-1195-43ff-a057-3d86a02c5d9c
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum CardState {
    /// SCARD_UNKNOWN
    Unknown = 0x0000_0000,
    /// SCARD_ABSENT
    Absent = 0x0000_0001,
    /// SCARD_PRESENT
    Present = 0x0000_0002,
    /// SCARD_SWALLOWED
    Swallowed = 0x0000_0003,
    /// SCARD_POWERED
    Powered = 0x0000_0004,
    /// SCARD_NEGOTIABLE
    Negotiable = 0x0000_0005,
    /// SCARD_SPECIFICMODE
    SpecificMode = 0x0000_0006,
}

impl From<CardState> for u32 {
    #[expect(
        clippy::as_conversions,
        reason = "guarantees discriminant layout, and as is the only way to cast enum -> primitive"
    )]
    fn from(val: CardState) -> Self {
        val as u32
    }
}

/// [2.2.2.2] Context_Call
///
/// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/b11d26d9-c3d5-4e96-8d9f-aba35cded852
#[derive(Debug, PartialEq, Clone)]
pub struct ContextCall {
    pub context: ScardContext,
}

impl ContextCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for ContextCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;
        context.decode_value(src, None)?;
        Ok(Self { context })
    }
}

/// [2.2.2.32] GetDeviceTypeId_Call
///
/// [2.2.2.32]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/b5e18874-c42d-42ea-b1b1-3fd86a8a95f1
#[derive(Debug, PartialEq, Clone)]
pub struct GetDeviceTypeIdCall {
    pub context: ScardContext,
    pub reader_ptr: u32,
    pub reader_name: String,
}

impl GetDeviceTypeIdCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for GetDeviceTypeIdCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;
        let reader_ptr = ndr::decode_ptr(src, &mut index)?;
        context.decode_value(src, None)?;
        let reader_name = ndr::read_string_from_cursor(src, CharacterSet::Unicode)?;
        Ok(Self {
            context,
            reader_ptr,
            reader_name,
        })
    }
}

/// [2.2.3.15] GetDeviceTypeId_Return
///
/// [2.2.3.15]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/fed90d29-c41f-490a-86e9-7e88e42656b2
#[derive(Debug, PartialEq, Clone)]
pub struct GetDeviceTypeIdReturn {
    pub return_code: ReturnCode,
    pub device_type_id: u32,
}

impl GetDeviceTypeIdReturn {
    const NAME: &'static str = "GetDeviceTypeId_Return";

    pub fn new(return_code: ReturnCode, device_type_id: u32) -> rpce::Pdu<Self> {
        rpce::Pdu(Self {
            return_code,
            device_type_id,
        })
    }
}

impl rpce::HeaderlessEncode for GetDeviceTypeIdReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        dst.write_u32(self.device_type_id);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size() // dst.write_u32(self.return_code.into());
        + size_of::<u32>() // dst.write_u32(self.device_type_id);
    }
}

/// [2.2.2.26] ReadCacheW_Call
///
/// [2.2.2.26]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/f45705cf-9299-4802-b408-685f02025e6a
#[derive(Debug, PartialEq, Clone)]
pub struct ReadCacheCall {
    pub lookup_name: String,
    pub common: ReadCacheCommon,
}

impl ReadCacheCall {
    pub fn decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, charset)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for ReadCacheCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        let charset = expect_charset(charset)?;
        let mut index = 0;
        let _lookup_name_ptr = ndr::decode_ptr(src, &mut index)?;
        let mut common = ReadCacheCommon::decode_ptr(src, &mut index)?;
        let lookup_name = ndr::read_string_from_cursor(src, charset)?;
        common.decode_value(src, None)?;
        Ok(Self { lookup_name, common })
    }
}

/// [2.2.1.9] ReadCache_Common
///
/// [2.2.1.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/3f9e07fa-66e2-498b-920c-39531709116b
#[derive(Debug, PartialEq, Clone)]
pub struct ReadCacheCommon {
    pub context: ScardContext,
    pub card_uuid: Vec<u8>,
    pub freshness_counter: u32,
    pub data_is_null: bool,
    pub data_len: u32,
}

impl ndr::Decode for ReadCacheCommon {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> DecodeResult<Self>
    where
        Self: Sized,
    {
        let context = ScardContext::decode_ptr(src, index)?;
        let _card_uuid_ptr = ndr::decode_ptr(src, index)?;
        ensure_size!(in: src, size: size_of::<u32>() * 2 + size_of::<i32>());
        let freshness_counter = src.read_u32();
        let data_is_null = src.read_i32() == 1;
        let data_len = src.read_u32();

        Ok(Self {
            context,
            card_uuid: Vec::new(),
            freshness_counter,
            data_is_null,
            data_len,
        })
    }

    fn decode_value(&mut self, src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<()> {
        expect_no_charset(charset)?;
        self.context.decode_value(src, None)?;
        ensure_size!(in: src, size: 16);
        self.card_uuid = src.read_slice(16).to_vec();
        Ok(())
    }
}

/// [2.2.3.1] ReadCache_Return
///
/// [2.2.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/da342355-e37f-485e-a490-3222a97fa356
#[derive(Debug, PartialEq, Clone)]
pub struct ReadCacheReturn {
    pub return_code: ReturnCode,
    pub data: Vec<u8>,
}

impl ReadCacheReturn {
    const NAME: &'static str = "ReadCache_Return";

    pub fn new(return_code: ReturnCode, data: Vec<u8>) -> rpce::Pdu<Self> {
        rpce::Pdu(Self { return_code, data })
    }
}

impl rpce::HeaderlessEncode for ReadCacheReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        let mut index = 0;
        let data_len: u32 = cast_length!("ReadCacheReturn", "data_len", self.data.len())?;
        ndr::encode_ptr(Some(data_len), &mut index, dst)?;
        dst.write_u32(data_len);
        dst.write_slice(&self.data);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size() // dst.write_u32(self.return_code.into());
        + ndr::ptr_size(true) // ndr::encode_ptr(Some(data_len), &mut index, dst)?;
        + size_of::<u32>() // dst.write_u32(data_len);
        + self.data.len() // dst.write_slice(&self.data);
    }
}

/// [2.2.2.28] WriteCacheW_Call
///
/// [2.2.2.28]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/3969bdcd-ecf3-42db-8bc6-2d6f970f9c67
#[derive(Debug, PartialEq, Clone)]
pub struct WriteCacheCall {
    pub lookup_name: String,
    pub common: WriteCacheCommon,
}

impl WriteCacheCall {
    pub fn decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, charset)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for WriteCacheCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        let charset = expect_charset(charset)?;
        let mut index = 0;
        let _lookup_name_ptr = ndr::decode_ptr(src, &mut index)?;
        let mut common = WriteCacheCommon::decode_ptr(src, &mut index)?;
        let lookup_name = ndr::read_string_from_cursor(src, charset)?;
        common.decode_value(src, None)?;
        Ok(Self { lookup_name, common })
    }
}

/// [2.2.1.10] WriteCache_Common
///
/// [2.2.1.10]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/5604251b-9173-457c-9476-57863df9010e
#[derive(Debug, PartialEq, Clone)]
pub struct WriteCacheCommon {
    pub context: ScardContext,
    pub card_uuid: Vec<u8>,
    pub freshness_counter: u32,
    pub data: Vec<u8>,
}

impl ndr::Decode for WriteCacheCommon {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> DecodeResult<Self>
    where
        Self: Sized,
    {
        let context = ScardContext::decode_ptr(src, index)?;
        let _card_uuid_ptr = ndr::decode_ptr(src, index)?;
        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let freshness_counter = src.read_u32();
        let _data_len = src.read_u32();
        let _data_ptr = ndr::decode_ptr(src, index)?;

        Ok(Self {
            context,
            card_uuid: Vec::new(),
            freshness_counter,
            data: Vec::new(),
        })
    }

    fn decode_value(&mut self, src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<()> {
        expect_no_charset(charset)?;
        self.context.decode_value(src, None)?;
        ensure_size!(in: src, size: 16);
        self.card_uuid = src.read_slice(16).to_vec();
        ensure_size!(in: src, size: size_of::<u32>());
        let data_len: usize = cast_length!("WriteCacheCommon", "data_len", src.read_u32())?;
        ensure_size!(in: src, size: data_len);
        self.data = src.read_slice(data_len).to_vec();
        Ok(())
    }
}

/// [2.2.2.5] ContextAndStringA_Call / [2.2.2.6] ContextAndStringW_Call
///
/// Used by IntroduceReaderGroup, ForgetReaderGroup, and ForgetReader.
///
/// [2.2.2.5]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/a130c3df-016c-4ca1-b85e-ad450fa4fe6d
/// [2.2.2.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/a42d07cc-b37b-4fe4-9df9-2ba6320d72fa
#[derive(Debug, PartialEq, Clone)]
pub struct ContextAndStringCall {
    pub context: ScardContext,
    pub sz: String,
}

impl ContextAndStringCall {
    pub fn decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, charset)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for ContextAndStringCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        let charset = expect_charset(charset)?;
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;
        // IDL pointer_default(unique): NULL contributes no referent.
        let sz_ptr = ndr::decode_ptr(src, &mut index)?;
        context.decode_value(src, None)?;
        let sz = if sz_ptr == 0 {
            String::new()
        } else {
            ndr::read_string_from_cursor(src, charset)?
        };
        Ok(Self { context, sz })
    }
}

/// [2.2.2.7] ContextAndTwoStringA_Call / [2.2.2.8] ContextAndTwoStringW_Call
///
/// Used by IntroduceReader, AddReaderToGroup, and RemoveReaderFromGroup.
///
/// [2.2.2.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/9ce7270c-aad5-46f7-8a10-941cb94b57f5
/// [2.2.2.8]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/34bec62b-b75c-4729-adb2-f6033484fe6b
#[derive(Debug, PartialEq, Clone)]
pub struct ContextAndTwoStringCall {
    pub context: ScardContext,
    pub sz1: String,
    pub sz2: String,
}

impl ContextAndTwoStringCall {
    pub fn decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, charset)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for ContextAndTwoStringCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        let charset = expect_charset(charset)?;
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;
        // IDL pointer_default(unique): each [string] may be NULL independently.
        let sz1_ptr = ndr::decode_ptr(src, &mut index)?;
        let sz2_ptr = ndr::decode_ptr(src, &mut index)?;
        context.decode_value(src, None)?;
        let sz1 = if sz1_ptr == 0 {
            String::new()
        } else {
            ndr::read_string_from_cursor(src, charset)?
        };
        let sz2 = if sz2_ptr == 0 {
            String::new()
        } else {
            ndr::read_string_from_cursor(src, charset)?
        };
        Ok(Self { context, sz1, sz2 })
    }
}

/// [2.2.2.31] GetReaderIcon_Call
///
/// [2.2.2.31]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/e6a68d90-697f-4b98-8ad6-f74853d27ccb
#[derive(Debug, PartialEq, Clone)]
pub struct GetReaderIconCall {
    pub context: ScardContext,
    pub reader_name: String,
}

impl GetReaderIconCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for GetReaderIconCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;

        let _reader_ptr = ndr::decode_ptr(src, &mut index)?;

        context.decode_value(src, None)?;
        let reader_name = ndr::read_string_from_cursor(src, CharacterSet::Unicode)?;
        Ok(Self { context, reader_name })
    }
}

/// [2.2.3.14] GetReaderIcon_Return
///
/// [2.2.3.14]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/f011f3d9-e2a4-4c43-a336-4c89ecaa8360
#[derive(Debug, PartialEq, Clone)]
pub struct GetReaderIconReturn {
    pub return_code: ReturnCode,
    pub data: Vec<u8>,
}

impl GetReaderIconReturn {
    const NAME: &'static str = "GetReaderIcon_Return";

    pub fn new(return_code: ReturnCode, data: Vec<u8>) -> rpce::Pdu<Self> {
        rpce::Pdu(Self { return_code, data })
    }
}

impl rpce::HeaderlessEncode for GetReaderIconReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        let data_len: u32 = cast_length!("GetReaderIconReturn", "data_len", self.data.len())?;
        let mut index = 0;
        ndr::encode_ptr(Some(data_len), &mut index, dst)?;
        dst.write_u32(data_len);
        dst.write_slice(&self.data);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        size_of::<u32>() // dst.write_u32(self.return_code.into());
        + ndr::ptr_size(true) // ndr::encode_ptr(Some(data_len), &mut index, dst)?;
        + size_of::<u32>() // dst.write_u32(data_len);
        + self.data.len() // dst.write_slice(&self.data);
    }
}

/// [2.2.2.3] ListReaderGroups_Call
///
/// [2.2.2.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/dde8c811-c258-444f-8028-98311a9e0082
#[derive(Debug, PartialEq, Clone)]
pub struct ListReaderGroupsCall {
    pub context: ScardContext,
    pub groups_is_null: bool,
    pub groups_size: u32,
}

impl ListReaderGroupsCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for ListReaderGroupsCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;
        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let groups_is_null = src.read_u32() == 0x0000_0001;
        let groups_size = src.read_u32();
        context.decode_value(src, None)?;
        Ok(Self {
            context,
            groups_is_null,
            groups_size,
        })
    }
}

/// [2.2.2.15] Reconnect_Call
///
/// [2.2.2.15]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/9c1eca52-3a99-403c-8ac8-6437f246a154
#[derive(Debug, PartialEq, Clone)]
pub struct ReconnectCall {
    pub handle: ScardHandle,
    pub share_mode: u32,
    pub preferred_protocols: CardProtocol,
    pub initialization: u32,
}

impl ReconnectCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for ReconnectCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        let mut index = 0;
        let mut handle = ScardHandle::decode_ptr(src, &mut index)?;
        ensure_size!(in: src, size: size_of::<u32>() * 3);
        let share_mode = src.read_u32();
        let preferred_protocols = CardProtocol::from_bits_retain(src.read_u32());
        let initialization = src.read_u32();
        handle.decode_value(src, None)?;
        Ok(Self {
            handle,
            share_mode,
            preferred_protocols,
            initialization,
        })
    }
}

/// [2.2.3.7] Reconnect_Return
///
/// [2.2.3.7]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/e25a583f-ab82-4ba3-bebf-af656c58e6d8
#[derive(Debug, PartialEq, Clone)]
pub struct ReconnectReturn {
    pub return_code: ReturnCode,
    pub active_protocol: CardProtocol,
}

impl ReconnectReturn {
    const NAME: &'static str = "Reconnect_Return";

    pub fn new(return_code: ReturnCode, active_protocol: CardProtocol) -> rpce::Pdu<Self> {
        rpce::Pdu(Self {
            return_code,
            active_protocol,
        })
    }
}

impl rpce::HeaderlessEncode for ReconnectReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        dst.write_u32(self.active_protocol.bits());
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size() + 4 /* dwActiveProtocol */
    }
}

/// [2.2.2.17] State_Call
///
/// [2.2.2.17]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/ba3b9097-02fe-4f6b-951c-05439a7d9da7
#[derive(Debug, PartialEq, Clone)]
pub struct StateCall {
    pub handle: ScardHandle,
    pub atr_is_null: bool,
    pub atr_length: u32,
}

impl StateCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for StateCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        let mut index = 0;
        let mut handle = ScardHandle::decode_ptr(src, &mut index)?;
        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let atr_is_null = src.read_u32() == 0x0000_0001;
        let atr_length = src.read_u32();
        handle.decode_value(src, None)?;
        Ok(Self {
            handle,
            atr_is_null,
            atr_length,
        })
    }
}

/// [2.2.3.9] State_Return
///
/// [2.2.3.9]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/574e5ec5-96ba-4b11-bfa9-52eb34307356
#[derive(Debug, PartialEq, Clone)]
pub struct StateReturn {
    pub return_code: ReturnCode,
    pub state: CardState,
    pub protocol: CardProtocol,
    /// `None` => NULL `rgAtr`; `atr_len` supplies `cbAtrLen`.
    pub atr: Option<Vec<u8>>,
    pub atr_len: u32,
}

impl StateReturn {
    const NAME: &'static str = "State_Return";

    pub fn new(return_code: ReturnCode, state: CardState, protocol: CardProtocol, atr: Vec<u8>) -> rpce::Pdu<Self> {
        let atr_len = u32::try_from(atr.len()).unwrap_or(u32::MAX);
        rpce::Pdu(Self {
            return_code,
            state,
            protocol,
            atr: Some(atr),
            atr_len,
        })
    }

    pub fn atr_probe(
        return_code: ReturnCode,
        state: CardState,
        protocol: CardProtocol,
        atr_len: u32,
    ) -> rpce::Pdu<Self> {
        rpce::Pdu(Self {
            return_code,
            state,
            protocol,
            atr: None,
            atr_len,
        })
    }
}

impl rpce::HeaderlessEncode for StateReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        dst.write_u32(self.state.into());
        dst.write_u32(self.protocol.bits());
        match &self.atr {
            Some(atr) => {
                if atr.len() > 36 {
                    return Err(invalid_field_err!("encode", "StateReturn cbAtrLen out of range"));
                }
                let atr_len: u32 = cast_length!("StateReturn", "atr_len", atr.len())?;
                let mut index = 0;
                ndr::encode_ptr(Some(atr_len), &mut index, dst)?;
                dst.write_u32(atr_len);
                dst.write_slice(atr);
            }
            None => {
                if self.atr_len > 36 {
                    return Err(invalid_field_err!("encode", "StateReturn cbAtrLen out of range"));
                }
                dst.write_u32(self.atr_len);
                dst.write_u32(0);
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        12 + match &self.atr {
            Some(atr) => ndr::ptr_size(true) + 4 + atr.len(),
            None => 8,
        }
    }
}

/// [2.2.2.20] Control_Call
///
/// [2.2.2.20]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/002fc3a3-2ca2-492e-8463-aba8f3923e48
#[derive(Debug, PartialEq, Clone)]
pub struct ControlCall {
    pub handle: ScardHandle,
    pub control_code: u32,
    pub in_buffer: Vec<u8>,
    pub out_buffer_is_null: bool,
    pub out_buffer_size: u32,
}

impl ControlCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for ControlCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        let mut index = 0;
        let mut handle = ScardHandle::decode_ptr(src, &mut index)?;
        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let control_code = src.read_u32();
        // MS-RDPESC: cbInBufferSize range(0,66560)
        let in_buffer_len = src.read_u32();
        if in_buffer_len > 66_560 {
            return Err(invalid_field_err!("decode", "Control cbInBufferSize out of range"));
        }
        let in_buffer_ptr = ndr::decode_ptr(src, &mut index)?;
        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let out_buffer_is_null = src.read_u32() == 0x0000_0001;
        let out_buffer_size = src.read_u32();
        handle.decode_value(src, None)?;

        let in_buffer = if in_buffer_ptr != 0 {
            ensure_size!(in: src, size: size_of::<u32>());
            let referent_len = src.read_u32();
            if referent_len != in_buffer_len {
                return Err(invalid_field_err!(
                    "decode",
                    "mismatched Control in-buffer length in NDR pointer and value"
                ));
            }
            let in_len: usize = cast_length!("ControlCall", "in_buffer_len", referent_len)?;
            ensure_size!(in: src, size: in_len);
            src.read_slice(in_len).to_vec()
        } else if in_buffer_len != 0 {
            return Err(invalid_field_err!(
                "decode",
                "Control cbInBufferSize/pvInBuffer inconsistency"
            ));
        } else {
            Vec::new()
        };

        Ok(Self {
            handle,
            control_code,
            in_buffer,
            out_buffer_is_null,
            out_buffer_size,
        })
    }
}

/// [2.2.3.6] Control_Return
///
/// [2.2.3.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/e7e854f8-0c5a-4814-bfdd-b72cb8aefe3e
#[derive(Debug, PartialEq, Clone)]
pub struct ControlReturn {
    pub return_code: ReturnCode,
    pub out_buffer: Vec<u8>,
}

impl ControlReturn {
    const NAME: &'static str = "Control_Return";

    pub fn new(return_code: ReturnCode, out_buffer: Vec<u8>) -> rpce::Pdu<Self> {
        rpce::Pdu(Self {
            return_code,
            out_buffer,
        })
    }
}

impl rpce::HeaderlessEncode for ControlReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        // MS-RDPESC: cbOutBufferSize range(0,66560)
        if self.out_buffer.len() > 66_560 {
            return Err(invalid_field_err!(
                "encode",
                "ControlReturn cbOutBufferSize out of range"
            ));
        }
        let data_len: u32 = cast_length!("ControlReturn", "out_buffer_len", self.out_buffer.len())?;
        let mut index = 0;
        ndr::encode_ptr(Some(data_len), &mut index, dst)?;
        dst.write_u32(data_len);
        dst.write_slice(&self.out_buffer);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size()
            + ndr::ptr_size(true)
            + 4 /* cbOutBufferSize value */
            + self.out_buffer.len() // pvOutBuffer
    }
}

/// [2.2.2.21] GetAttrib_Call
///
/// [2.2.2.21]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/f4e36ff1-e7b3-4046-bddb-cd192a76c7ab
#[derive(Debug, PartialEq, Clone)]
pub struct GetAttribCall {
    pub handle: ScardHandle,
    pub attr_id: u32,
    pub attr_is_null: bool,
    pub attr_length: u32,
}

impl GetAttribCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for GetAttribCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        let mut index = 0;
        let mut handle = ScardHandle::decode_ptr(src, &mut index)?;
        ensure_size!(in: src, size: size_of::<u32>() * 3);
        let attr_id = src.read_u32();
        let attr_is_null = src.read_u32() == 0x0000_0001;
        let attr_length = src.read_u32();
        handle.decode_value(src, None)?;
        Ok(Self {
            handle,
            attr_id,
            attr_is_null,
            attr_length,
        })
    }
}

/// [2.2.3.12] GetAttrib_Return
///
/// [2.2.3.12]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/ab3ac071-3fc5-44e6-9b94-c1eee1168266
#[derive(Debug, PartialEq, Clone)]
pub struct GetAttribReturn {
    pub return_code: ReturnCode,
    pub attr: Vec<u8>,
}

impl GetAttribReturn {
    const NAME: &'static str = "GetAttrib_Return";

    pub fn new(return_code: ReturnCode, attr: Vec<u8>) -> rpce::Pdu<Self> {
        rpce::Pdu(Self { return_code, attr })
    }
}

impl rpce::HeaderlessEncode for GetAttribReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        // MS-RDPESC: cbAttrLen range(0,65536)
        if self.attr.len() > 65_536 {
            return Err(invalid_field_err!("encode", "GetAttribReturn cbAttrLen out of range"));
        }
        let data_len: u32 = cast_length!("GetAttribReturn", "attr_len", self.attr.len())?;
        let mut index = 0;
        ndr::encode_ptr(Some(data_len), &mut index, dst)?;
        dst.write_u32(data_len);
        dst.write_slice(&self.attr);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size()
            + ndr::ptr_size(true)
            + 4 /* cbAttrLen value */
            + self.attr.len() // pbAttr
    }
}

/// [2.2.2.22] SetAttrib_Call
///
/// [2.2.2.22]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/28f8dd60-35b7-45fb-ab75-15bbf81f5d11
#[derive(Debug, PartialEq, Clone)]
pub struct SetAttribCall {
    pub handle: ScardHandle,
    pub attr_id: u32,
    pub attr: Vec<u8>,
}

impl SetAttribCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for SetAttribCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        let mut index = 0;
        let mut handle = ScardHandle::decode_ptr(src, &mut index)?;
        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let attr_id = src.read_u32();
        // MS-RDPESC: cbAttrLen range(0,65536)
        let attr_len = src.read_u32();
        if attr_len > 65_536 {
            return Err(invalid_field_err!("decode", "SetAttrib cbAttrLen out of range"));
        }
        let attr_ptr = ndr::decode_ptr(src, &mut index)?;
        handle.decode_value(src, None)?;

        let attr = if attr_ptr != 0 {
            ensure_size!(in: src, size: size_of::<u32>());
            let referent_len = src.read_u32();
            if referent_len != attr_len {
                return Err(invalid_field_err!(
                    "decode",
                    "mismatched SetAttrib attr length in NDR pointer and value"
                ));
            }
            let attr_len: usize = cast_length!("SetAttribCall", "attr_len", referent_len)?;
            ensure_size!(in: src, size: attr_len);
            src.read_slice(attr_len).to_vec()
        } else if attr_len != 0 {
            return Err(invalid_field_err!("decode", "SetAttrib cbAttrLen/pbAttr inconsistency"));
        } else {
            Vec::new()
        };

        Ok(Self { handle, attr_id, attr })
    }
}

/// [2.2.2.29] GetTransmitCount_Call
///
/// [2.2.2.29]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/f453f1b4-1291-4c69-8e97-f781b2d8e66f
#[derive(Debug, PartialEq, Clone)]
pub struct GetTransmitCountCall {
    pub handle: ScardHandle,
}

impl GetTransmitCountCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src, None)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for GetTransmitCountCall {
    fn headerless_decode(src: &mut ReadCursor<'_>, charset: Option<CharacterSet>) -> DecodeResult<Self> {
        expect_no_charset(charset)?;
        let mut index = 0;
        let mut handle = ScardHandle::decode_ptr(src, &mut index)?;
        handle.decode_value(src, None)?;
        Ok(Self { handle })
    }
}

/// [2.2.3.13] GetTransmitCount_Return
///
/// [2.2.3.13]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/32aea1d7-0edb-4807-bbd1-a6ee1fbb0087
#[derive(Debug, PartialEq, Clone)]
pub struct GetTransmitCountReturn {
    pub return_code: ReturnCode,
    pub transmit_count: u32,
}

impl GetTransmitCountReturn {
    const NAME: &'static str = "GetTransmitCount_Return";

    pub fn new(return_code: ReturnCode, transmit_count: u32) -> rpce::Pdu<Self> {
        rpce::Pdu(Self {
            return_code,
            transmit_count,
        })
    }
}

impl rpce::HeaderlessEncode for GetTransmitCountReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        dst.write_u32(self.transmit_count);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size() + 4 /* cTransmitCount */
    }
}

fn expect_charset(charset: Option<CharacterSet>) -> DecodeResult<CharacterSet> {
    charset.ok_or_else(|| other_err!("internal error: missing character set"))
}

fn expect_no_charset(charset: Option<CharacterSet>) -> DecodeResult<()> {
    if charset.is_some() {
        return Err(other_err!(
            "internal error: character set given where none was expected"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdu::esc::rpce::HeaderlessEncode;

    fn roundtrip_context(ctx: ScardContext) -> ScardContext {
        let mut buf = vec![0u8; ctx.size()];
        let mut index = 0;
        {
            let mut dst = WriteCursor::new(&mut buf);
            ctx.encode_ptr(&mut index, &mut dst).unwrap();
            ctx.encode_value(&mut dst).unwrap();
        }
        index = 0;
        let mut src = ReadCursor::new(&buf);
        let mut decoded = ScardContext::decode_ptr(&mut src, &mut index).unwrap();
        decoded.decode_value(&mut src, None).unwrap();
        decoded
    }

    fn roundtrip_handle(handle: ScardHandle) -> ScardHandle {
        let mut buf = vec![0u8; handle.size()];
        let mut index = 0;
        {
            let mut dst = WriteCursor::new(&mut buf);
            handle.encode_ptr(&mut index, &mut dst).unwrap();
            handle.encode_value(&mut dst).unwrap();
        }
        index = 0;
        let mut src = ReadCursor::new(&buf);
        let mut decoded = ScardHandle::decode_ptr(&mut src, &mut index).unwrap();
        decoded.decode_value(&mut src, None).unwrap();
        decoded
    }

    #[test]
    fn scard_context_and_handle_roundtrip() {
        let c4 = ScardContext::new(0xA1B2_C3D4);
        assert_eq!(roundtrip_context(c4), c4);
        assert_eq!(c4.value(), 0xA1B2_C3D4);
        let opaque = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        ];
        assert_eq!(
            roundtrip_context(ScardContext::from_opaque(&opaque).unwrap()).as_bytes(),
            &opaque
        );
        assert!(ScardContext::from_opaque(&[0; 17]).is_err());
        assert_eq!(roundtrip_context(ScardContext::from_opaque(&[]).unwrap()).len(), 0);
        let ctx = ScardContext::new(0x1111_2222);
        assert_eq!(
            roundtrip_handle(ScardHandle::new(ctx, 0x3333_4444)),
            ScardHandle::new(ctx, 0x3333_4444)
        );
        assert_eq!(
            roundtrip_handle(ScardHandle::from_opaque(ctx, &opaque).unwrap()).as_bytes(),
            &opaque
        );
        assert!(ScardHandle::from_opaque(ctx, &[0; 17]).is_err());
    }

    #[test]
    fn context_and_string_null_unique_ptr() {
        // empty context + NULL sz unique pointer
        let mut pdu = vec![0x01, 0x10, 0x08, 0x00, 0xCC, 0xCC, 0xCC, 0xCC, 12, 0, 0, 0, 0, 0, 0, 0];
        pdu.extend_from_slice(&[0u8; 12]);
        let d = ContextAndStringCall::decode(&mut ReadCursor::new(&pdu), Some(CharacterSet::Unicode)).unwrap();
        assert!(d.sz.is_empty());
    }

    /// `::new` 4-byte EstablishContext/Connect returns stay byte-identical to master.
    #[test]
    fn legacy_four_byte_return_encode_matches_master() {
        fn enc(body: &impl HeaderlessEncode) -> Vec<u8> {
            let mut buf = vec![0u8; HeaderlessEncode::size(body)];
            HeaderlessEncode::encode(body, &mut WriteCursor::new(&mut buf)).unwrap();
            buf
        }
        let est = EstablishContextReturn::new(ReturnCode::Success, ScardContext::new(0xA1B2_C3D4)).into_inner();
        assert_eq!(
            enc(&est),
            [0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 2, 0, 4, 0, 0, 0, 0xD4, 0xC3, 0xB2, 0xA1]
        );
        let h = ScardHandle::new(ScardContext::new(0x1111_2222), 0x3333_4444);
        let conn = ConnectReturn::new(ReturnCode::Success, h, CardProtocol::SCARD_PROTOCOL_T0).into_inner();
        assert_eq!(
            enc(&conn),
            [
                0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 2, 0, 4, 0, 0, 0, 4, 0, 2, 0, 1, 0, 0, 0, 4, 0, 0, 0, 0x22, 0x22, 0x11,
                0x11, 4, 0, 0, 0, 0x44, 0x44, 0x33, 0x33,
            ]
        );
    }

    /// 6-byte Unicode mszCards needs 2-byte NDR pad before reader states.
    #[test]
    fn locate_cards_w_skips_msz_cards_ndr_pad() {
        let cards = [0x41u8, 0x00, 0x00, 0x00, 0x00, 0x00]; // "A\0\0"
        let mut body = Vec::new();
        body.extend_from_slice(&[0u8; 8]); // empty context ptr
        body.extend_from_slice(&6u32.to_le_bytes());
        body.extend_from_slice(&0x0002_0000u32.to_le_bytes());
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&0x0002_0004u32.to_le_bytes());
        body.extend_from_slice(&6u32.to_le_bytes());
        body.extend_from_slice(&cards);
        body.extend_from_slice(&[0u8; 2]); // NDR pad
        body.extend_from_slice(&1u32.to_le_bytes());
        body.extend_from_slice(&0x0002_0008u32.to_le_bytes());
        body.extend_from_slice(&[0u8; 12 + 36]); // ReaderState_Common_Call zeros
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&0u32.to_le_bytes());
        body.extend_from_slice(&2u32.to_le_bytes());
        body.extend_from_slice(&[0x52, 0x00, 0x00, 0x00]); // "R"

        let mut pdu = vec![0x01, 0x10, 0x08, 0x00, 0xCC, 0xCC, 0xCC, 0xCC];
        pdu.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        pdu.extend_from_slice(&0u32.to_le_bytes());
        pdu.extend_from_slice(&body);

        let decoded = LocateCardsCall::decode(&mut ReadCursor::new(&pdu), Some(CharacterSet::Unicode)).unwrap();
        assert_eq!(decoded.cards, vec!["A".to_owned()]);
        assert_eq!(decoded.states[0].reader, "R");
    }
}
