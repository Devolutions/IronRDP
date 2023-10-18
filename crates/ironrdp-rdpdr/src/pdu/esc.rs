//! PDUs for [\[MS-RDPESC\]: Remote Desktop Protocol: Smart Card Virtual Channel Extension]
//!
//! [\[MS-RDPESC\]: Remote Desktop Protocol: Smart Card Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/0428ca28-b4dc-46a3-97c3-01887fa44a90

pub mod ndr;
pub mod rpce;

use super::efs::IoCtlCode;
use crate::pdu::esc::ndr::{Decode as _, Encode as _};
use bitflags::bitflags;
use ironrdp_pdu::{
    cast_length,
    cursor::{ReadCursor, WriteCursor},
    ensure_size, invalid_message_err,
    utils::{encoded_multistring_len, read_multistring_from_cursor, write_multistring_to_cursor, CharacterSet},
    PduDecode, PduError, PduResult,
};
use std::mem::size_of;

/// [2.2.2 TS Server-Generated Structures]
///
/// [2.2.2 TS Server-Generated Structures]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/f4ca3b61-b49c-463c-8932-2cf82fb7ec7a
#[derive(Debug)]
pub enum ScardCall {
    AccessStartedEventCall(ScardAccessStartedEventCall),
    EstablishContextCall(EstablishContextCall),
    ListReadersCall(ListReadersCall),
    GetStatusChangeCall(GetStatusChangeCall),
    ConnectCall(ConnectCall),
    HCardAndDispositionCall(HCardAndDispositionCall),
    TransmitCall(TransmitCall),
    StatusCall(StatusCall),
    ContextCall(ContextCall),
    GetDeviceTypeIdCall(GetDeviceTypeIdCall),
    ReadCacheCall(ReadCacheCall),
    WriteCacheCall(WriteCacheCall),
    Unsupported,
}

impl ScardCall {
    pub fn decode(io_ctl_code: ScardIoCtlCode, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        match io_ctl_code {
            ScardIoCtlCode::AccessStartedEvent => Ok(ScardCall::AccessStartedEventCall(
                ScardAccessStartedEventCall::decode(src)?,
            )),
            ScardIoCtlCode::EstablishContext => Ok(ScardCall::EstablishContextCall(EstablishContextCall::decode(src)?)),
            ScardIoCtlCode::ListReadersW => Ok(ScardCall::ListReadersCall(ListReadersCall::decode(src)?)),
            ScardIoCtlCode::GetStatusChangeW => Ok(ScardCall::GetStatusChangeCall(GetStatusChangeCall::decode(src)?)),
            ScardIoCtlCode::ConnectW => Ok(ScardCall::ConnectCall(ConnectCall::decode(src)?)),
            ScardIoCtlCode::BeginTransaction => Ok(ScardCall::HCardAndDispositionCall(
                HCardAndDispositionCall::decode(src)?,
            )),
            ScardIoCtlCode::Transmit => Ok(ScardCall::TransmitCall(TransmitCall::decode(src)?)),
            ScardIoCtlCode::StatusW | ScardIoCtlCode::StatusA => Ok(ScardCall::StatusCall(StatusCall::decode(src)?)),
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
            ScardIoCtlCode::ReadCacheW => Ok(ScardCall::ReadCacheCall(ReadCacheCall::decode(src)?)),
            ScardIoCtlCode::WriteCacheW => Ok(ScardCall::WriteCacheCall(WriteCacheCall::decode(src)?)),
            _ => {
                warn!(?io_ctl_code, "Unsupported ScardIoCtlCode");
                // TODO: maybe this should be an error
                Ok(Self::Unsupported)
            }
        }
    }
}

/// [2.2.1.1 REDIR_SCARDCONTEXT]
///
/// [2.2.1.1 REDIR_SCARDCONTEXT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/060abee1-e520-4149-9ef7-ce79eb500a59
#[derive(Debug, Copy, Clone)]
pub struct ScardContext {
    /// Shortcut: we always create 4-byte context values.
    /// The spec allows this field to have variable length.
    pub value: u32,
}

impl ScardContext {
    const NAME: &'static str = "REDIR_SCARDCONTEXT";
    /// See [`ScardContext::value`]
    const VALUE_LENGTH: u32 = 4;

    pub fn new(value: u32) -> Self {
        Self { value }
    }
}

impl ndr::Encode for ScardContext {
    fn encode_ptr(&self, index: &mut u32, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ndr::encode_ptr(Some(Self::VALUE_LENGTH), index, dst)
    }

    fn encode_value(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size_value());
        dst.write_u32(Self::VALUE_LENGTH);
        dst.write_u32(self.value);
        Ok(())
    }

    fn size_ptr(&self) -> usize {
        ndr::ptr_size(true)
    }

    fn size_value(&self) -> usize {
        4 /* cbContext */ + 4 /* pbContext */
    }
}

impl ndr::Decode for ScardContext {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> PduResult<Self>
    where
        Self: Sized,
    {
        ensure_size!(in: src, size: size_of::<u32>());
        let length = src.read_u32();
        if length != Self::VALUE_LENGTH {
            error!(?length, "Unsupported value length in ScardContext");
            return Err(invalid_message_err!(
                "decode_ptr",
                "unsupported value length in ScardContext"
            ));
        }

        let _ptr = ndr::decode_ptr(src, index)?;
        Ok(Self { value: 0 })
    }

    fn decode_value(&mut self, src: &mut ReadCursor<'_>) -> PduResult<()> {
        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let length = src.read_u32();
        if length != Self::VALUE_LENGTH {
            error!(?length, "Unsupported value length in ScardContext");
            return Err(invalid_message_err!(
                "decode_value",
                "unsupported value length in ScardContext"
            ));
        }
        self.value = src.read_u32();
        Ok(())
    }
}

/// [2.2.1.7 ReaderStateW]
///
/// [2.2.1.7 ReaderStateW]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/0ba03cd2-bed0-495b-adbe-3d2cde61980c
#[derive(Debug)]
pub struct ReaderState {
    pub reader: String,
    pub common: ReaderStateCommonCall,
}

impl ndr::Decode for ReaderState {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> PduResult<Self> {
        let _reader_ptr = ndr::decode_ptr(src, index)?;
        let common = ReaderStateCommonCall::decode(src)?;
        Ok(Self {
            reader: String::new(),
            common,
        })
    }

    fn decode_value(&mut self, src: &mut ReadCursor<'_>) -> PduResult<()> {
        self.reader = ndr::read_string_from_cursor(src)?;
        Ok(())
    }
}

/// From [3.1.4 Message Processing Events and Sequencing Rules]
///
/// [3.1.4 Message Processing Events and Sequencing Rules]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/60d5977d-0017-4c90-ab0c-f34bf44a74a5
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
    type Error = PduError;

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
                Err(invalid_message_err!("try_from", "ScardIoCtlCode", "unsupported value"))
            }
        }
    }
}

/// Allow [`ScardIoCtlCode`] to be used as an [`IoCtlCode`].
impl IoCtlCode for ScardIoCtlCode {}

/// [2.2.2.30 ScardAccessStartedEvent_Call]
///
/// [2.2.2.30 ScardAccessStartedEvent_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/c5ab8dd0-4914-4355-960c-0a527971ea69
#[derive(Debug)]
pub struct ScardAccessStartedEventCall;

impl ScardAccessStartedEventCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ironrdp_pdu::read_padding!(src, 4); // Unused (4 bytes)
        Ok(Self)
    }
}

/// [2.2.3.3 Long_Return]
///
/// [2.2.3.3 Long_Return]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/e77a1365-2379-4037-99c4-d30d14ba10fc
#[derive(Debug)]
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
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
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

/// [2.2.8 Return Code]
///
/// [2.2.8 Return Code]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/9861f8da-76fe-41e6-847e-40c9aa35df8d
#[derive(Debug, Clone, Copy)]
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
    fn from(val: ReturnCode) -> Self {
        val as u32
    }
}

/// [2.2.2.1 EstablishContext_Call]
///
/// [2.2.2.1 EstablishContext_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/b990635a-7637-464a-8923-361ed3e3d67a
#[derive(Debug)]
pub struct EstablishContextCall {
    pub scope: Scope,
}

impl EstablishContextCall {
    const NAME: &'static str = "EstablishContext_Call";

    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src)?.into_inner())
    }

    fn size() -> usize {
        size_of::<u32>()
    }
}

impl rpce::HeaderlessDecode for EstablishContextCall {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(in: src, size: Self::size());
        let scope = Scope::try_from(src.read_u32())?;
        Ok(Self { scope })
    }
}

#[derive(Debug)]
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
    type Error = PduError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x0000_0000 => Ok(Scope::User),
            0x0000_0001 => Ok(Scope::Terminal),
            0x0000_0002 => Ok(Scope::System),
            _ => {
                error!("Unsupported Scope: 0x{:08x}", value);
                Err(invalid_message_err!("try_from", "Scope", "unsupported value"))
            }
        }
    }
}

/// [2.2.3.2 EstablishContext_Return]
///
/// [2.2.3.2 EstablishContext_Return]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/9135d95f-3740-411b-bdca-34ac7571fddc
#[derive(Debug)]
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
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
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

/// [2.2.2.4 ListReaders_Call]
///
/// [2.2.2.4 ListReaders_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/be2f46a5-77fb-40bf-839c-aed45f0a26d7
#[derive(Debug)]
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
    const NAME: &'static str = "ListReaders_Call";

    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for ListReadersCall {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;

        ensure_size!(in: src, size: size_of::<u32>());
        let groups_ptr_length = src.read_u32();

        let groups_ptr = ndr::decode_ptr(src, &mut index)?;

        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let readers_is_null = (src.read_u32()) == 0x0000_0001;
        let readers_size = src.read_u32();

        context.decode_value(src)?;

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
            return Err(invalid_message_err!(
                "decode",
                "mismatched reader groups length in NDR pointer and value"
            ));
        }

        let groups = read_multistring_from_cursor(src, CharacterSet::Unicode)?;

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

/// [2.2.3.4 ListReaderGroups_Return and ListReaders_Return]
///
/// [2.2.3.4 ListReaderGroups_Return and ListReaders_Return]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/6630bb5b-fc0e-4141-8b53-263225c7628d
#[derive(Debug)]
pub struct ListReadersReturn {
    pub return_code: ReturnCode,
    pub readers: Vec<String>,
}

impl ListReadersReturn {
    const NAME: &'static str = "ListReaders_Return";

    pub fn new(return_code: ReturnCode, readers: Vec<String>) -> rpce::Pdu<Self> {
        rpce::Pdu(Self { return_code, readers })
    }
}

impl rpce::HeaderlessEncode for ListReadersReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        let readers_length: u32 = cast_length!(
            "ListReadersReturn",
            "readers",
            encoded_multistring_len(&self.readers, CharacterSet::Unicode)
        )?;
        let mut index = 0;
        ndr::encode_ptr(Some(readers_length), &mut index, dst)?;
        dst.write_u32(readers_length);
        write_multistring_to_cursor(dst, &self.readers, CharacterSet::Unicode)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size() // dst.write_u32(self.return_code.into());
        + ndr::ptr_size(true) // ndr::encode_ptr(...);
        + 4 // dst.write_u32(readers_length);
        + encoded_multistring_len(&self.readers, CharacterSet::Unicode) // write_multistring_to_cursor(...);
    }
}

/// [2.2.2.12 GetStatusChangeW_Call]
///
/// [2.2.2.12 GetStatusChangeW_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/af357ce8-63ee-4577-b6bf-c6f5ca68d754
#[derive(Debug)]
pub struct GetStatusChangeCall {
    pub context: ScardContext,
    pub timeout: u32,
    pub states_ptr_length: u32,
    pub states_ptr: u32,
    pub states_length: u32,
    pub states: Vec<ReaderState>,
}

impl GetStatusChangeCall {
    const NAME: &'static str = "GetStatusChangeW_Call";

    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for GetStatusChangeCall {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;

        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let timeout = src.read_u32();
        let states_ptr_length = src.read_u32();

        let states_ptr = ndr::decode_ptr(src, &mut index)?;

        context.decode_value(src)?;

        ensure_size!(in: src, size: size_of::<u32>());
        let states_length = src.read_u32();

        let mut states = Vec::new();
        for _ in 0..states_length {
            let state = ReaderState::decode_ptr(src, &mut index)?;
            states.push(state);
        }
        for state in states.iter_mut() {
            state.decode_value(src)?;
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

/// [2.2.1.5 ReaderState_Common_Call]
///
/// [2.2.1.5 ReaderState_Common_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/a71e63ba-e58f-487c-a5d2-5a3e48856594
#[derive(Debug)]
pub struct ReaderStateCommonCall {
    pub current_state: CardStateFlags,
    pub event_state: CardStateFlags,
    pub atr_length: u32,
    pub atr: [u8; 36],
}

impl ReaderStateCommonCall {
    const NAME: &'static str = "ReaderState_Common_Call";
    const FIXED_PART_SIZE: usize = size_of::<u32>() * 3 /* dwCurrentState, dwEventState, cbAtr */ + 36 /* rgbAtr */;

    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
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

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
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
    }
}

/// [2.2.3.5 LocateCards_Return and GetStatusChange_Return]
///
/// [2.2.3.5 LocateCards_Return and GetStatusChange_Return]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/7b73e0c2-e0fc-46b1-9b03-50684ad2beba
#[derive(Debug)]
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
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
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

/// [2.2.2.14 ConnectW_Call]
///
/// [2.2.2.14 ConnectW_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/fd06f6a0-a9ea-478c-9b5e-470fd9cde5a6
#[derive(Debug)]
pub struct ConnectCall {
    pub reader: String,
    pub common: ConnectCommon,
}

impl ConnectCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for ConnectCall {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let mut index = 0;
        let _reader_ptr = ndr::decode_ptr(src, &mut index)?;
        let mut common = ConnectCommon::decode_ptr(src, &mut index)?;
        let reader = ndr::read_string_from_cursor(src)?;
        common.decode_value(src)?;
        Ok(Self { reader, common })
    }
}

/// [2.2.1.3 Connect_Common]
///
/// [2.2.1.3 Connect_Common]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/32752f32-4410-4682-b9fc-9096674b52de
#[derive(Debug)]
pub struct ConnectCommon {
    pub context: ScardContext,
    pub share_mode: u32,
    pub preferred_protocols: CardProtocol,
}

impl ConnectCommon {
    const NAME: &'static str = "Connect_Common";
}

impl ndr::Decode for ConnectCommon {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> PduResult<Self>
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

    fn decode_value(&mut self, src: &mut ReadCursor<'_>) -> PduResult<()> {
        self.context.decode_value(src)
    }
}

bitflags! {
    /// [2.2.5 Protocol Identifier]
    ///
    /// [2.2.5 Protocol Identifier]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/41673567-2710-4e86-be87-7b6f46fe10af
    #[derive(Debug, Clone)]
    pub struct CardProtocol: u32 {
        const SCARD_PROTOCOL_UNDEFINED = 0x0000_0000;
        const SCARD_PROTOCOL_T0 = 0x0000_0001;
        const SCARD_PROTOCOL_T1 = 0x0000_0002;
        const SCARD_PROTOCOL_TX = 0x0000_0003;
        const SCARD_PROTOCOL_RAW = 0x0001_0000;
        const SCARD_PROTOCOL_DEFAULT = 0x8000_0000;
        const SCARD_PROTOCOL_OPTIMAL = 0x0000_0000;
    }
}

/// [2.2.1.2 REDIR_SCARDHANDLE]
///
/// [2.2.1.2 REDIR_SCARDHANDLE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/b6276356-7c5f-4d3e-be92-a6c85e58d008
#[derive(Debug)]
pub struct ScardHandle {
    pub context: ScardContext,
    /// Shortcut: we always create 4-byte handle values.
    /// The spec allows this field to have variable length.
    pub value: u32,
}

impl ScardHandle {
    const NAME: &'static str = "REDIR_SCARDHANDLE";
    /// See [`ScardHandle::value`]
    const VALUE_LENGTH: u32 = 4;

    pub fn new(context: ScardContext, value: u32) -> Self {
        Self { context, value }
    }
}

impl ndr::Decode for ScardHandle {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> PduResult<Self>
    where
        Self: Sized,
    {
        let context = ScardContext::decode_ptr(src, index)?;
        ensure_size!(ctx: "ScardHandle::decode_ptr", in: src, size: size_of::<u32>());
        let length = src.read_u32();
        if length != Self::VALUE_LENGTH {
            error!(?length, "Unsupported value length in ScardHandle");
            return Err(invalid_message_err!(
                "decode_ptr",
                "unsupported value length in ScardHandle"
            ));
        }
        let _ptr = ndr::decode_ptr(src, index)?;
        Ok(Self { context, value: 0 })
    }

    fn decode_value(&mut self, src: &mut ReadCursor<'_>) -> PduResult<()> {
        self.context.decode_value(src)?;
        ensure_size!(in: src, size: size_of::<u32>());
        let length = src.read_u32();
        if length != Self::VALUE_LENGTH {
            error!(?length, "Unsupported value length in ScardHandle");
            return Err(invalid_message_err!(
                "decode_value",
                "unsupported value length in ScardHandle"
            ));
        }
        ensure_size!(in: src, size: size_of::<u32>());
        self.value = src.read_u32();
        Ok(())
    }
}

impl ndr::Encode for ScardHandle {
    fn encode_ptr(&self, index: &mut u32, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        self.context.encode_ptr(index, dst)?;
        ndr::encode_ptr(Some(Self::VALUE_LENGTH), index, dst)?;
        Ok(())
    }

    fn encode_value(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size_value());
        self.context.encode_value(dst)?;
        dst.write_u32(Self::VALUE_LENGTH);
        dst.write_u32(self.value);
        Ok(())
    }

    fn size_ptr(&self) -> usize {
        self.context.size_ptr() + ndr::ptr_size(true)
    }

    fn size_value(&self) -> usize {
        self.context.size_value() + 4 /* cbHandle */ + 4 /* pbHandle */
    }
}

/// [2.2.3.8 Connect_Return]
///
/// [2.2.3.8 Connect_Return]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/ad9fbc8e-0963-44ac-8d71-38021685790c
#[derive(Debug)]
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
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
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

/// [2.2.2.16 HCardAndDisposition_Call]
///
/// [2.2.2.16 HCardAndDisposition_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/f15ae865-9e99-4c5b-bb43-15a6b4885bd0
#[derive(Debug)]
pub struct HCardAndDispositionCall {
    pub handle: ScardHandle,
    pub disposition: u32,
}

impl HCardAndDispositionCall {
    const NAME: &'static str = "HCardAndDisposition_Call";

    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for HCardAndDispositionCall {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let mut index = 0;
        let mut handle = ScardHandle::decode_ptr(src, &mut index)?;
        ensure_size!(in: src, size: size_of::<u32>());
        let disposition = src.read_u32();
        handle.decode_value(src)?;
        Ok(Self { handle, disposition })
    }
}

/// [2.2.2.19 Transmit_Call]
///
/// [2.2.2.19 Transmit_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/e3861cfa-e61b-4d64-b19d-f6b31e076beb
#[derive(Debug)]
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
    const NAME: &'static str = "Transmit_Call";

    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for TransmitCall {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
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

        handle.decode_value(src)?;
        send_pci.decode_value(src)?;

        ensure_size!(in: src, size: size_of::<u32>());
        let send_length = src.read_u32();
        let send_length_usize: usize = cast_length!("TransmitCall", "send_length", send_length)?;
        ensure_size!(in: src, size: send_length_usize);
        let send_buffer = src.read_slice(send_length_usize).to_vec();

        let recv_pci = if recv_pci_ptr != 0 {
            let mut recv_pci = SCardIORequest::decode_ptr(src, &mut index)?;
            recv_pci.decode_value(src)?;
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

/// [2.2.1.8 SCardIO_Request]
///
/// [2.2.1.8 SCardIO_Request]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/f6e15da8-5bc0-4ef6-b28a-ce88e8415621
#[derive(Debug, Clone)]
pub struct SCardIORequest {
    pub protocol: CardProtocol,
    pub extra_bytes_length: u32,
    pub extra_bytes: Vec<u8>,
}

impl SCardIORequest {
    const NAME: &'static str = "SCardIO_Request";
}

impl ndr::Decode for SCardIORequest {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> PduResult<Self>
    where
        Self: Sized,
    {
        ensure_size!(in: src, size: size_of::<u32>() * 2);
        let protocol = CardProtocol::from_bits_retain(src.read_u32());
        let extra_bytes_length = src.read_u32();
        let _extra_bytes_ptr = ndr::decode_ptr(src, index)?;
        let extra_bytes = Vec::new();
        Ok(Self {
            protocol,
            extra_bytes_length,
            extra_bytes,
        })
    }

    fn decode_value(&mut self, src: &mut ReadCursor<'_>) -> PduResult<()> {
        let extra_bytes_length: usize = cast_length!("TransmitCall", "extra_bytes_length", self.extra_bytes_length)?;
        ensure_size!(in: src, size: extra_bytes_length);
        self.extra_bytes = src.read_slice(extra_bytes_length).to_vec();
        Ok(())
    }
}

impl ndr::Encode for SCardIORequest {
    fn encode_ptr(&self, index: &mut u32, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size_ptr());
        dst.write_u32(self.protocol.bits());
        ndr::encode_ptr(Some(self.extra_bytes_length), index, dst)
    }

    fn encode_value(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size_value());
        dst.write_slice(&self.extra_bytes);
        Ok(())
    }

    fn size_ptr(&self) -> usize {
        4 /* dwProtocol */ + ndr::ptr_size(true)
    }

    fn size_value(&self) -> usize {
        self.extra_bytes_length as usize
    }
}

/// [2.2.3.11 Transmit_Return]
///
/// [2.2.3.11 Transmit_Return]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/252cffd0-58b8-434d-9e1b-0d547544fb0f
#[derive(Debug)]
pub struct TransmitReturn {
    pub return_code: ReturnCode,
    pub recv_pci: Option<SCardIORequest>,
    pub recv_buffer: Vec<u8>,
}

impl TransmitReturn {
    const NAME: &'static str = "Transmit_Return";

    pub fn new(return_code: ReturnCode, recv_pci: Option<SCardIORequest>, recv_buffer: Vec<u8>) -> rpce::Pdu<Self> {
        rpce::Pdu(Self {
            return_code,
            recv_pci,
            recv_buffer,
        })
    }
}

impl rpce::HeaderlessEncode for TransmitReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());

        let mut index = 0;
        if let Some(recv_pci) = &self.recv_pci {
            recv_pci.encode_ptr(&mut index, dst)?;
            recv_pci.encode_value(dst)?;
        } else {
            dst.write_u32(0); // null value
        }

        let recv_buffer_len: u32 = cast_length!("TransmitReturn", "recv_buffer_len", self.recv_buffer.len())?;
        ndr::encode_ptr(Some(recv_buffer_len), &mut index, dst)?;
        dst.write_u32(recv_buffer_len);
        dst.write_slice(&self.recv_buffer);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.return_code.size() // dst.write_u32(self.return_code.into());
        + if let Some(recv_pci) = &self.recv_pci {
            recv_pci.size()
        } else {
            4 // null value
        }
        + ndr::ptr_size(true) // ndr::encode_ptr(Some(recv_buffer_len), &mut index, dst)?;
        + 4 // dst.write_u32(recv_buffer_len);
        + self.recv_buffer.len() // dst.write_slice(&self.recv_buffer);
    }
}

/// [2.2.2.18 Status_Call]
///
/// [2.2.2.18 Status_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/f1139aed-e578-47f3-a800-f36b56c80500
#[derive(Debug)]
pub struct StatusCall {
    pub handle: ScardHandle,
    pub reader_names_is_null: bool,
    pub reader_length: u32,
    pub atr_length: u32,
}

impl StatusCall {
    const NAME: &'static str = "Status_Call";

    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for StatusCall {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let mut index = 0;
        let mut handle = ScardHandle::decode_ptr(src, &mut index)?;
        ensure_size!(in: src, size: size_of::<u32>() * 3);
        let reader_names_is_null = src.read_u32() == 1;
        let reader_length = src.read_u32();
        let atr_length = src.read_u32();
        handle.decode_value(src)?;
        Ok(Self {
            handle,
            reader_names_is_null,
            reader_length,
            atr_length,
        })
    }
}

/// [2.2.3.10 Status_Return]
///
/// [2.2.3.10 Status_Return]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/987c1358-ad6b-4c8e-88e1-06210c28a66f
#[derive(Debug)]
pub struct StatusReturn {
    pub return_code: ReturnCode,
    pub reader_names: Vec<String>,
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
        rpce::Pdu(Self {
            return_code,
            reader_names,
            state,
            protocol,
            atr,
            atr_length,
            encoding,
        })
    }
}

impl rpce::HeaderlessEncode for StatusReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code.into());
        let mut index = 0;
        let reader_names_length: u32 = cast_length!(
            "StatusReturn",
            "reader_names_length",
            encoded_multistring_len(&self.reader_names, self.encoding)
        )?;
        ndr::encode_ptr(Some(reader_names_length), &mut index, dst)?;
        dst.write_u32(self.state.into());
        dst.write_u32(self.protocol.bits());
        dst.write_slice(&self.atr);
        dst.write_u32(self.atr_length);
        dst.write_u32(reader_names_length);
        write_multistring_to_cursor(dst, &self.reader_names, self.encoding)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        size_of::<u32>() * 5 // dst.write_u32(self.return_code.into()); dst.write_u32(self.state.into()); dst.write_u32(self.protocol.bits()); dst.write_slice(&self.atr); dst.write_u32(self.atr_length);
        + ndr::ptr_size(true) // ndr::encode_ptr(Some(reader_names_length), &mut index, dst)?;
        + self.atr.len() // dst.write_slice(&self.atr);
        + encoded_multistring_len(&self.reader_names, self.encoding) // write_multistring_to_cursor(dst, &self.reader_names, self.encoding)?;
    }
}

/// [2.2.4 Card/Reader State]
///
/// [2.2.4 Card/Reader State]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/264bc504-1195-43ff-a057-3d86a02c5d9c
#[derive(Debug, Clone, Copy)]
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
    fn from(val: CardState) -> Self {
        val as u32
    }
}

/// [2.2.2.2 Context_Call]
///
/// [2.2.2.2 Context_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/b11d26d9-c3d5-4e96-8d9f-aba35cded852
#[derive(Debug)]
pub struct ContextCall {
    pub context: ScardContext,
}

impl ContextCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for ContextCall {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;
        context.decode_value(src)?;
        Ok(Self { context })
    }
}

/// [2.2.2.32 GetDeviceTypeId_Call]
///
/// [2.2.2.32 GetDeviceTypeId_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/b5e18874-c42d-42ea-b1b1-3fd86a8a95f1
#[derive(Debug)]
pub struct GetDeviceTypeIdCall {
    pub context: ScardContext,
    pub reader_ptr: u32,
    pub reader_name: String,
}

impl GetDeviceTypeIdCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for GetDeviceTypeIdCall {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let mut index = 0;
        let mut context = ScardContext::decode_ptr(src, &mut index)?;
        let reader_ptr = ndr::decode_ptr(src, &mut index)?;
        context.decode_value(src)?;
        let reader_name = ndr::read_string_from_cursor(src)?;
        Ok(Self {
            context,
            reader_ptr,
            reader_name,
        })
    }
}

/// [2.2.3.15 GetDeviceTypeId_Return]
///
/// [2.2.3.15 GetDeviceTypeId_Return]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/fed90d29-c41f-490a-86e9-7e88e42656b2
#[derive(Debug)]
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
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
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

/// [2.2.2.26 ReadCacheW_Call]
///
/// [2.2.2.26 ReadCacheW_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/f45705cf-9299-4802-b408-685f02025e6a
#[derive(Debug)]
pub struct ReadCacheCall {
    pub lookup_name: String,
    pub common: ReadCacheCommon,
}

impl ReadCacheCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for ReadCacheCall {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let mut index = 0;
        let _lookup_name_ptr = ndr::decode_ptr(src, &mut index)?;
        let mut common = ReadCacheCommon::decode_ptr(src, &mut index)?;
        let lookup_name = ndr::read_string_from_cursor(src)?;
        common.decode_value(src)?;
        Ok(Self { lookup_name, common })
    }
}

/// [2.2.1.9 ReadCache_Common]
///
/// [2.2.1.9 ReadCache_Common]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/3f9e07fa-66e2-498b-920c-39531709116b
#[derive(Debug)]
pub struct ReadCacheCommon {
    pub context: ScardContext,
    pub card_uuid: Vec<u8>,
    pub freshness_counter: u32,
    pub data_is_null: bool,
    pub data_len: u32,
}

impl ReadCacheCommon {
    const NAME: &'static str = "ReadCache_Common";
}

impl ndr::Decode for ReadCacheCommon {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> PduResult<Self>
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

    fn decode_value(&mut self, src: &mut ReadCursor<'_>) -> PduResult<()> {
        self.context.decode_value(src)?;
        ensure_size!(in: src, size: 16);
        self.card_uuid = src.read_slice(16).to_vec();
        Ok(())
    }
}

/// [2.2.3.1 ReadCache_Return]
///
/// [2.2.3.1 ReadCache_Return]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/da342355-e37f-485e-a490-3222a97fa356
#[derive(Debug)]
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
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
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

/// [2.2.2.28 WriteCacheW_Call]
///
/// [2.2.2.28 WriteCacheW_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/3969bdcd-ecf3-42db-8bc6-2d6f970f9c67
#[derive(Debug)]
pub struct WriteCacheCall {
    pub lookup_name: String,
    pub common: WriteCacheCommon,
}

impl WriteCacheCall {
    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        Ok(rpce::Pdu::<Self>::decode(src)?.into_inner())
    }
}

impl rpce::HeaderlessDecode for WriteCacheCall {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let mut index = 0;
        let _lookup_name_ptr = ndr::decode_ptr(src, &mut index)?;
        let mut common = WriteCacheCommon::decode_ptr(src, &mut index)?;
        let lookup_name = ndr::read_string_from_cursor(src)?;
        common.decode_value(src)?;
        Ok(Self { lookup_name, common })
    }
}

/// [2.2.1.10 WriteCache_Common]
///
/// [2.2.1.10 WriteCache_Common]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/5604251b-9173-457c-9476-57863df9010e
#[derive(Debug)]
pub struct WriteCacheCommon {
    pub context: ScardContext,
    pub card_uuid: Vec<u8>,
    pub freshness_counter: u32,
    pub data: Vec<u8>,
}

impl WriteCacheCommon {
    const NAME: &'static str = "WriteCache_Common";
}

impl ndr::Decode for WriteCacheCommon {
    fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> PduResult<Self>
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

    fn decode_value(&mut self, src: &mut ReadCursor<'_>) -> PduResult<()> {
        self.context.decode_value(src)?;
        ensure_size!(in: src, size: 16);
        self.card_uuid = src.read_slice(16).to_vec();
        ensure_size!(in: src, size: size_of::<u32>());
        let data_len: usize = cast_length!("WriteCacheCommon", "data_len", src.read_u32())?;
        ensure_size!(in: src, size: data_len);
        self.data = src.read_slice(data_len).to_vec();
        Ok(())
    }
}
