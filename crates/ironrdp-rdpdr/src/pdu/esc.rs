//! PDUs for [\[MS-RDPESC\]: Remote Desktop Protocol: Smart Card Virtual Channel Extension]
//!
//! [\[MS-RDPESC\]: Remote Desktop Protocol: Smart Card Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/0428ca28-b4dc-46a3-97c3-01887fa44a90

use self::ndr::{ReaderState, ScardContext};
use super::efs::IoCtlCode;
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
            _ => {
                warn!(?io_ctl_code, "Unsupported ScardIoCtlCode");
                // TODO: maybe this should be an error
                Ok(Self::Unsupported)
            }
        }
    }
}

/// From [3.1.4 Message Processing Events and Sequencing Rules]
///
/// [3.1.4 Message Processing Events and Sequencing Rules]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/60d5977d-0017-4c90-ab0c-f34bf44a74a5
#[derive(Debug, Clone, Copy)]
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
        let current_state = CardStateFlags::from_bits_truncate(src.read_u32());
        let event_state = CardStateFlags::from_bits_truncate(src.read_u32());
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

    fn size(&self) -> usize {
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
        + self.reader_states.iter().map(|s| s.size()).sum::<usize>()
    }
}

pub mod rpce {
    //! PDUs for [\[MS-RPCE\]: Remote Procedure Call Protocol Extensions] as required by [MS-RDPESC].
    //!
    //! [\[MS-RPCE\]: Remote Procedure Call Protocol Extensions]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c15

    use std::mem::size_of;

    use ironrdp_pdu::{
        cast_length,
        cursor::{ReadCursor, WriteCursor},
        ensure_size, invalid_message_err, PduDecode, PduEncode, PduError, PduResult,
    };

    /// Wrapper struct for [MS-RPCE] PDUs that allows for common [`PduEncode`], [`Encode`], and [`Self::decode`] implementations.
    ///
    /// Structs which are meant to be encoded into an [MS-RPCE] message should typically implement [`HeaderlessEncode`],
    /// and their `new` function should return a [`Pdu`] wrapping the underlying struct.
    ///
    /// ```rust
    /// #[derive(Debug)]
    /// pub struct RpceEncodePdu {
    ///     example_field: u32,
    /// }
    ///
    /// impl RpceEncodePdu {
    ///     /// `new` returns a `Pdu` wrapping the underlying struct.
    ///     pub fn new(example_field: u32) -> rpce::Pdu<Self> {
    ///         rpce::Pdu(Self { example_field })
    ///     }
    /// }
    ///
    /// /// The underlying struct should implement `HeaderlessEncode`.
    /// impl rpce::HeaderlessEncode for RpceEncodePdu {
    ///     fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
    ///         ensure_size!(in: dst, size: self.size());
    ///         dst.write_u32(self.return_code.into());
    ///         Ok(())
    ///     }
    ///
    ///     fn name(&self) -> &'static str {
    ///         "RpceEncodePdu"
    ///     }
    ///
    ///     fn size(&self) -> usize {
    ///         std::mem::size_of<u32>()
    ///     }
    /// }
    /// ```
    ///
    /// See [`super::LongReturn`] for a live example of an encodable PDU.
    ///
    /// Structs which are meant to be decoded from an [MS-RPCE] message should typically implement [`HeaderlessDecode`],
    /// and their `decode` function should return a [`Pdu`] wrapping the underlying struct.
    ///
    /// ```rust
    /// pub struct RpceDecodePdu {
    ///     example_field: u32,
    /// }
    ///
    /// impl RpceDecodePdu {
    ///     /// `decode` returns a `Pdu` wrapping the underlying struct.
    ///     pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<rpce::Pdu<Self>> {
    ///         Ok(rpce::Pdu::<Self>::decode(src)?.into_inner())
    ///     }
    ///
    ///     fn size() -> usize {
    ///         std::mem::size_of<u32>()
    ///     }
    /// }
    ///
    /// /// The underlying struct should implement `HeaderlessDecode`.
    /// impl rpce::HeaderlessDecode for RpceDecodePdu {
    ///    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self>
    ///    where
    ///         Self: Sized,
    ///    {
    ///        ensure_size!(in: src, size: Self::size());
    ///        let example_field = src.read_u32();
    ///        Ok(Self { example_field })
    ///     }
    /// }
    /// ```
    ///
    /// See [`super::EstablishContextCall`] for a live example of a decodable PDU.
    #[derive(Debug)]
    pub struct Pdu<T>(pub T);

    impl<T> Pdu<T> {
        pub fn into_inner(self) -> T {
            self.0
        }

        pub fn into_inner_ref(&self) -> &T {
            &self.0
        }
    }

    impl<T: HeaderlessDecode> PduDecode<'_> for Pdu<T> {
        /// Decodes the instance from a buffer stripping it of its [`StreamHeader`] and [`TypeHeader`].
        fn decode(src: &mut ReadCursor<'_>) -> PduResult<Pdu<T>> {
            // We expect `StreamHeader::decode`, `TypeHeader::decode`, and `T::decode` to each
            // call `ensure_size!` to ensure that the buffer is large enough, so we can safely
            // omit that check here.
            let _stream_header = StreamHeader::decode(src)?;
            let _type_header = TypeHeader::decode(src)?;
            let pdu = T::decode(src)?;
            Ok(Self(pdu))
        }
    }

    impl<T: HeaderlessEncode> PduEncode for Pdu<T> {
        fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
            ensure_size!(ctx: self.name(), in: dst, size: self.size());
            let stream_header = StreamHeader::default();
            let type_header = TypeHeader::new(cast_length!("Pdu<T>", "size", self.size())?);

            stream_header.encode(dst)?;
            type_header.encode(dst)?;
            HeaderlessEncode::encode(&self.0, dst)?;

            // Pad response to be 8-byte aligned.
            let padding_size = padding_size(&self.0);
            if padding_size > 0 {
                dst.write_slice(&vec![0; padding_size]);
            }

            Ok(())
        }

        fn name(&self) -> &'static str {
            self.0.name()
        }

        fn size(&self) -> usize {
            StreamHeader::size() + TypeHeader::size() + HeaderlessEncode::size(&self.0) + padding_size(&self.0)
        }
    }

    impl<T: HeaderlessEncode> Encode for Pdu<T> {}

    /// Trait for types that can be encoded into an [MS-RPCE] message.
    ///
    /// Implementers should typically avoid implementing this trait directly
    /// and instead implement [`HeaderlessEncode`], and wrap it in a [`Pdu`].
    pub trait Encode: PduEncode + Send + Sync + std::fmt::Debug {}

    /// Trait for types that can be encoded into an [MS-RPCE] message.
    ///
    /// Implementers should typically implement this trait instead of [`Encode`].
    pub trait HeaderlessEncode: Send + Sync + std::fmt::Debug {
        /// Encodes the instance into a buffer sans its [`StreamHeader`] and [`TypeHeader`].
        fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()>;
        /// Returns the name associated with this RPCE PDU.
        fn name(&self) -> &'static str;
        /// Returns the size of the instance sans its [`StreamHeader`] and [`TypeHeader`].
        fn size(&self) -> usize;
    }

    /// Trait for types that can be decoded from an [MS-RPCE] message.
    ///
    /// Implementers should typically implement this trait for a given type `T`
    /// and then call [`Pdu::decode`] to decode the instance. See [`Pdu`] for more
    /// details and an example.
    pub trait HeaderlessDecode: Sized {
        /// Decodes the instance from a buffer sans its [`StreamHeader`] and [`TypeHeader`].
        fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self>;
    }

    /// [2.2.6.1 Common Type Header for the Serialization Stream]
    ///
    /// [2.2.6.1 Common Type Header for the Serialization Stream]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/6d75d40e-e2d2-4420-b9e9-8508a726a9ae
    struct StreamHeader {
        version: u8,
        endianness: Endianness,
        common_header_length: u16,
        filler: u32,
    }

    impl Default for StreamHeader {
        fn default() -> Self {
            Self {
                version: 1,
                endianness: Endianness::LittleEndian,
                common_header_length: 8,
                filler: 0xCCCC_CCCC,
            }
        }
    }

    impl StreamHeader {
        const NAME: &'static str = "RpceStreamHeader";

        fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
            ensure_size!(in: dst, size: Self::size());
            dst.write_u8(self.version);
            dst.write_u8(self.endianness.into());
            dst.write_u16(self.common_header_length);
            dst.write_u32(self.filler);
            Ok(())
        }

        fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
            ensure_size!(in: src, size: Self::size());
            let version = src.read_u8();
            let endianness = Endianness::try_from(src.read_u8())?;
            let common_header_length = src.read_u16();
            let filler = src.read_u32();
            if endianness == Endianness::LittleEndian {
                Ok(Self {
                    version,
                    endianness,
                    common_header_length,
                    filler,
                })
            } else {
                Err(invalid_message_err!(
                    "decode",
                    "StreamHeader",
                    "server returned big-endian data, parsing not implemented"
                ))
            }
        }

        fn size() -> usize {
            size_of::<u8>() + size_of::<u8>() + size_of::<u16>() + size_of::<u32>()
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    #[repr(u8)]
    enum Endianness {
        BigEndian = 0x00,
        LittleEndian = 0x10,
    }

    impl TryFrom<u8> for Endianness {
        type Error = PduError;

        fn try_from(value: u8) -> Result<Self, Self::Error> {
            match value {
                0x00 => Ok(Endianness::BigEndian),
                0x10 => Ok(Endianness::LittleEndian),
                _ => Err(invalid_message_err!("try_from", "RpceEndianness", "unsupported value")),
            }
        }
    }

    impl From<Endianness> for u8 {
        fn from(endianness: Endianness) -> Self {
            endianness as u8
        }
    }

    /// [2.2.6.2 Private Header for Constructed Type]
    ///
    /// [2.2.6.2 Private Header for Constructed Type]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/63949ba8-bc88-4c0c-9377-23f14b197827
    #[derive(Debug)]
    struct TypeHeader {
        object_buffer_length: u32,
        filler: u32,
    }

    impl TypeHeader {
        const NAME: &'static str = "RpceTypeHeader";

        fn new(object_buffer_length: u32) -> Self {
            Self {
                object_buffer_length,
                filler: 0,
            }
        }

        fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
            ensure_size!(in: dst, size: Self::size());
            dst.write_u32(self.object_buffer_length);
            dst.write_u32(self.filler);
            Ok(())
        }

        fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
            ensure_size!(in: src, size: Self::size());
            let object_buffer_length = src.read_u32();
            let filler = src.read_u32();

            Ok(Self {
                object_buffer_length,
                filler,
            })
        }
    }

    impl TypeHeader {
        fn size() -> usize {
            size_of::<u32>() * 2
        }
    }

    /// Calculates the padding required for an [MS-RPCE] message
    /// to be 8-byte aligned.
    fn padding_size(pdu: &impl HeaderlessEncode) -> usize {
        let tail = pdu.size() % 8;
        if tail > 0 {
            8 - tail
        } else {
            0
        }
    }
}

pub mod ndr {
    //! Request/response messages are nested structs with fields, encoded as NDR (network data
    //! representation).
    //!
    //! Fixed-size fields are encoded in-line as they appear in the struct.
    //!
    //! Variable-sized fields (strings, byte arrays, sometimes structs) are encoded as pointers:
    //! - in place of the field in the struct, a "pointer" is written
    //! - the pointer value is 0x0002xxxx, where xxxx is an "index" in increments of 4
    //! - for example, first pointer is 0x0002_0000, second is 0x0002_0004, third is 0x0002_0008 etc.
    //! - the actual values are then appended at the end of the message, in the same order as their
    //!   pointers appeared
    //! - in the code below, "*_ptr" is the pointer value and "*_value" the actual data
    //! - note that some fields (like arrays) will have a length prefix before the pointer and also
    //!   before the actual data at the end of the message
    //!
    //! To deal with this, fixed-size structs only have encode/decode methods, while variable-size ones
    //! have encode_ptr/decode_ptr and encode_value/decode_value methods. Messages are parsed linearly,
    //! so decode_ptr/decode_value are called at different stages (same for encoding).
    //!
    //! Most of the above was reverse-engineered from FreeRDP:
    //! https://github.com/FreeRDP/FreeRDP/blob/master/channels/smartcard/client/smartcard_pack.c

    use std::mem::size_of;

    use ironrdp_pdu::{
        cursor::{ReadCursor, WriteCursor},
        ensure_size, invalid_message_err,
        utils::{self, CharacterSet},
        PduResult,
    };

    use super::ReaderStateCommonCall;

    /// [2.2.1.1 REDIR_SCARDCONTEXT]
    ///
    /// [2.2.1.1 REDIR_SCARDCONTEXT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/060abee1-e520-4149-9ef7-ce79eb500a59
    #[derive(Debug)]
    pub struct ScardContext {
        pub length: u32,
        // Shortcut: we always create 4-byte context values.
        // The spec allows this field to have variable length.
        pub value: u32,
    }

    impl ScardContext {
        const NAME: &'static str = "REDIR_SCARDCONTEXT";

        pub fn new(value: u32) -> Self {
            Self {
                length: size_of::<u32>() as u32,
                value,
            }
        }

        pub fn encode_ptr(&self, index: &mut u32, dst: &mut WriteCursor<'_>) -> PduResult<()> {
            encode_ptr(Some(self.length), index, dst)
        }

        pub fn encode_value(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
            dst.write_u32(self.length);
            dst.write_u32(self.value);
            Ok(())
        }

        pub fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> PduResult<Self> {
            ensure_size!(in: src, size: size_of::<u32>());
            let length = src.read_u32();
            let _ptr = decode_ptr(src, index)?;
            Ok(Self { length, value: 0 })
        }

        pub fn decode_value(&mut self, src: &mut ReadCursor<'_>) -> PduResult<()> {
            ensure_size!(in: src, size: size_of::<u32>()*2);
            let length = src.read_u32();
            if length != self.length {
                Err(invalid_message_err!(
                    "decode_value",
                    "mismatched length in ScardContext reference and value"
                ))
            } else {
                self.value = src.read_u32();
                Ok(())
            }
        }

        pub fn size(&self) -> usize {
            ptr_size(true) + size_of::<u32>() * 2
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

    impl ReaderState {
        pub fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> PduResult<Self> {
            let _reader_ptr = decode_ptr(src, index)?;
            let common = ReaderStateCommonCall::decode(src)?;
            Ok(Self {
                reader: String::new(),
                common,
            })
        }

        pub fn decode_value(&mut self, src: &mut ReadCursor<'_>) -> PduResult<()> {
            self.reader = read_string_from_cursor(src)?;
            Ok(())
        }
    }

    pub fn encode_ptr(length: Option<u32>, index: &mut u32, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(ctx: "encode_ptr", in: dst, size: ptr_size(length.is_some()));
        if let Some(length) = length {
            dst.write_u32(length);
        }

        dst.write_u32(0x0002_0000 + *index * 4);
        *index += 1;
        Ok(())
    }

    pub fn decode_ptr(src: &mut ReadCursor<'_>, index: &mut u32) -> PduResult<u32> {
        ensure_size!(ctx: "decode_ptr", in: src, size: size_of::<u32>());
        let ptr = src.read_u32();
        if ptr == 0 {
            // NULL pointer is OK. Don't update index.
            return Ok(ptr);
        }
        let expect_ptr = 0x0002_0000 + *index * 4;
        *index += 1;
        if ptr != expect_ptr {
            Err(invalid_message_err!("decode_ptr", "ptr", "ptr != expect_ptr"))
        } else {
            Ok(ptr)
        }
    }

    pub fn ptr_size(with_length: bool) -> usize {
        if with_length {
            size_of::<u32>() * 2
        } else {
            size_of::<u32>()
        }
    }

    /// A special read_string_from_cursor which reads and ignores the additional length and
    /// offset fields prefixing the string, as well as any extra padding for a 4-byte aligned
    /// NULL-terminated string.
    pub fn read_string_from_cursor(cursor: &mut ReadCursor<'_>) -> PduResult<String> {
        ensure_size!(ctx: "ndr::read_string_from_cursor", in: cursor, size: size_of::<u32>() * 3);
        let length = cursor.read_u32();
        let _offset = cursor.read_u32();
        let _length2 = cursor.read_u32();

        let string = utils::read_string_from_cursor(cursor, CharacterSet::Unicode, true)?;

        // Skip padding for 4-byte aligned NULL-terminated string.
        if length % 2 != 0 {
            ensure_size!(ctx: "ndr::read_string_from_cursor", in: cursor, size: size_of::<u16>());
            let _padding = cursor.read_u16();
        }

        Ok(string)
    }
}
