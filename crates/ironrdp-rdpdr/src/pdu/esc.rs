//! PDUs for [\[MS-RDPESC\]: Remote Desktop Protocol: Smart Card Virtual Channel Extension]
//!
//! [\[MS-RDPESC\]: Remote Desktop Protocol: Smart Card Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/0428ca28-b4dc-46a3-97c3-01887fa44a90

use super::efs::IoctlCode;
use ironrdp_pdu::{
    cursor::{ReadCursor, WriteCursor},
    ensure_size, invalid_message_err, PduError, PduResult,
};
use tracing::error;

/// From [3.1.4 Message Processing Events and Sequencing Rules]
///
/// [3.1.4 Message Processing Events and Sequencing Rules]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/60d5977d-0017-4c90-ab0c-f34bf44a74a5
#[derive(Debug)]
#[repr(u32)]
pub enum ScardIoctlCode {
    /// SCARD_IOCTL_ESTABLISHCONTEXT
    EstablishContext = 0x00090014,
    /// SCARD_IOCTL_RELEASECONTEXT
    ReleaseContext = 0x00090018,
    /// SCARD_IOCTL_ISVALIDCONTEXT
    IsValidContext = 0x0009001C,
    /// SCARD_IOCTL_LISTREADERGROUPSA
    ListReaderGroupsA = 0x00090020,
    /// SCARD_IOCTL_LISTREADERGROUPSW
    ListReaderGroupsW = 0x00090024,
    /// SCARD_IOCTL_LISTREADERSA
    ListReadersA = 0x00090028,
    /// SCARD_IOCTL_LISTREADERSW
    ListReadersW = 0x0009002C,
    /// SCARD_IOCTL_INTRODUCEREADERGROUPA
    IntroduceReaderGroupA = 0x00090050,
    /// SCARD_IOCTL_INTRODUCEREADERGROUPW
    IntroduceReaderGroupW = 0x00090054,
    /// SCARD_IOCTL_FORGETREADERGROUPA
    ForgetReaderGroupA = 0x00090058,
    /// SCARD_IOCTL_FORGETREADERGROUPW
    ForgetReaderGroupW = 0x0009005C,
    /// SCARD_IOCTL_INTRODUCEREADERA
    IntroduceReaderA = 0x00090060,
    /// SCARD_IOCTL_INTRODUCEREADERW
    IntroduceReaderW = 0x00090064,
    /// SCARD_IOCTL_FORGETREADERA
    ForgetReaderA = 0x00090068,
    /// SCARD_IOCTL_FORGETREADERW
    ForgetReaderW = 0x0009006C,
    /// SCARD_IOCTL_ADDREADERTOGROUPA
    AddReaderToGroupA = 0x00090070,
    /// SCARD_IOCTL_ADDREADERTOGROUPW
    AddReaderToGroupW = 0x00090074,
    /// SCARD_IOCTL_REMOVEREADERFROMGROUPA
    RemoveReaderFromGroupA = 0x00090078,
    /// SCARD_IOCTL_REMOVEREADERFROMGROUPW
    RemoveReaderFromGroupW = 0x0009007C,
    /// SCARD_IOCTL_LOCATECARDSA
    LocateCardsA = 0x00090098,
    /// SCARD_IOCTL_LOCATECARDSW
    LocateCardsW = 0x0009009C,
    /// SCARD_IOCTL_GETSTATUSCHANGEA
    GetStatusChangeA = 0x000900A0,
    /// SCARD_IOCTL_GETSTATUSCHANGEW
    GetStatusChangeW = 0x000900A4,
    /// SCARD_IOCTL_CANCEL
    Cancel = 0x000900A8,
    /// SCARD_IOCTL_CONNECTA
    ConnectA = 0x000900AC,
    /// SCARD_IOCTL_CONNECTW
    ConnectW = 0x000900B0,
    /// SCARD_IOCTL_RECONNECT
    Reconnect = 0x000900B4,
    /// SCARD_IOCTL_DISCONNECT
    Disconnect = 0x000900B8,
    /// SCARD_IOCTL_BEGINTRANSACTION
    BeginTransaction = 0x000900BC,
    /// SCARD_IOCTL_ENDTRANSACTION
    EndTransaction = 0x000900C0,
    /// SCARD_IOCTL_STATE
    State = 0x000900C4,
    /// SCARD_IOCTL_STATUSA
    StatusA = 0x000900C8,
    /// SCARD_IOCTL_STATUSW
    StatusW = 0x000900CC,
    /// SCARD_IOCTL_TRANSMIT
    Transmit = 0x000900D0,
    /// SCARD_IOCTL_CONTROL
    Control = 0x000900D4,
    /// SCARD_IOCTL_GETATTRIB
    GetAttrib = 0x000900D8,
    /// SCARD_IOCTL_SETATTRIB
    SetAttrib = 0x000900DC,
    /// SCARD_IOCTL_ACCESSSTARTEDEVENT
    AccessStartedEvent = 0x000900E0,
    /// SCARD_IOCTL_RELEASETARTEDEVENT
    ReleaseTartedEvent = 0x000900E4,
    /// SCARD_IOCTL_LOCATECARDSBYATRA
    LocateCardsByAtrA = 0x000900E8,
    /// SCARD_IOCTL_LOCATECARDSBYATRW
    LocateCardsByAtrW = 0x000900EC,
    /// SCARD_IOCTL_READCACHEA
    ReadCacheA = 0x000900F0,
    /// SCARD_IOCTL_READCACHEW
    ReadCacheW = 0x000900F4,
    /// SCARD_IOCTL_WRITECACHEA
    WriteCacheA = 0x000900F8,
    /// SCARD_IOCTL_WRITECACHEW
    WriteCacheW = 0x000900FC,
    /// SCARD_IOCTL_GETTRANSMITCOUNT
    GetTransmitCount = 0x00090100,
    /// SCARD_IOCTL_GETREADERICON
    GetReaderIcon = 0x00090104,
    /// SCARD_IOCTL_GETDEVICETYPEID
    GetDeviceTypeId = 0x00090108,
}

impl TryFrom<u32> for ScardIoctlCode {
    type Error = PduError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x00090014 => Ok(ScardIoctlCode::EstablishContext),
            0x00090018 => Ok(ScardIoctlCode::ReleaseContext),
            0x0009001C => Ok(ScardIoctlCode::IsValidContext),
            0x00090020 => Ok(ScardIoctlCode::ListReaderGroupsA),
            0x00090024 => Ok(ScardIoctlCode::ListReaderGroupsW),
            0x00090028 => Ok(ScardIoctlCode::ListReadersA),
            0x0009002C => Ok(ScardIoctlCode::ListReadersW),
            0x00090050 => Ok(ScardIoctlCode::IntroduceReaderGroupA),
            0x00090054 => Ok(ScardIoctlCode::IntroduceReaderGroupW),
            0x00090058 => Ok(ScardIoctlCode::ForgetReaderGroupA),
            0x0009005C => Ok(ScardIoctlCode::ForgetReaderGroupW),
            0x00090060 => Ok(ScardIoctlCode::IntroduceReaderA),
            0x00090064 => Ok(ScardIoctlCode::IntroduceReaderW),
            0x00090068 => Ok(ScardIoctlCode::ForgetReaderA),
            0x0009006C => Ok(ScardIoctlCode::ForgetReaderW),
            0x00090070 => Ok(ScardIoctlCode::AddReaderToGroupA),
            0x00090074 => Ok(ScardIoctlCode::AddReaderToGroupW),
            0x00090078 => Ok(ScardIoctlCode::RemoveReaderFromGroupA),
            0x0009007C => Ok(ScardIoctlCode::RemoveReaderFromGroupW),
            0x00090098 => Ok(ScardIoctlCode::LocateCardsA),
            0x0009009C => Ok(ScardIoctlCode::LocateCardsW),
            0x000900A0 => Ok(ScardIoctlCode::GetStatusChangeA),
            0x000900A4 => Ok(ScardIoctlCode::GetStatusChangeW),
            0x000900A8 => Ok(ScardIoctlCode::Cancel),
            0x000900AC => Ok(ScardIoctlCode::ConnectA),
            0x000900B0 => Ok(ScardIoctlCode::ConnectW),
            0x000900B4 => Ok(ScardIoctlCode::Reconnect),
            0x000900B8 => Ok(ScardIoctlCode::Disconnect),
            0x000900BC => Ok(ScardIoctlCode::BeginTransaction),
            0x000900C0 => Ok(ScardIoctlCode::EndTransaction),
            0x000900C4 => Ok(ScardIoctlCode::State),
            0x000900C8 => Ok(ScardIoctlCode::StatusA),
            0x000900CC => Ok(ScardIoctlCode::StatusW),
            0x000900D0 => Ok(ScardIoctlCode::Transmit),
            0x000900D4 => Ok(ScardIoctlCode::Control),
            0x000900D8 => Ok(ScardIoctlCode::GetAttrib),
            0x000900DC => Ok(ScardIoctlCode::SetAttrib),
            0x000900E0 => Ok(ScardIoctlCode::AccessStartedEvent),
            0x000900E4 => Ok(ScardIoctlCode::ReleaseTartedEvent),
            0x000900E8 => Ok(ScardIoctlCode::LocateCardsByAtrA),
            0x000900EC => Ok(ScardIoctlCode::LocateCardsByAtrW),
            0x000900F0 => Ok(ScardIoctlCode::ReadCacheA),
            0x000900F4 => Ok(ScardIoctlCode::ReadCacheW),
            0x000900F8 => Ok(ScardIoctlCode::WriteCacheA),
            0x000900FC => Ok(ScardIoctlCode::WriteCacheW),
            0x00090100 => Ok(ScardIoctlCode::GetTransmitCount),
            0x00090104 => Ok(ScardIoctlCode::GetReaderIcon),
            0x00090108 => Ok(ScardIoctlCode::GetDeviceTypeId),
            _ => {
                error!("Unsupported ScardIoctlCode: 0x{:08x}", value);
                Err(invalid_message_err!("try_from", "ScardIoctlCode", "unsupported value"))
            }
        }
    }
}

/// Allow [`ScardIoctlCode`] to be used as an [`IoctlCode`].
impl IoctlCode for ScardIoctlCode {}

/// [2.2.2.30 ScardAccessStartedEvent_Call]
///
/// [2.2.2.30 ScardAccessStartedEvent_Call]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/c5ab8dd0-4914-4355-960c-0a527971ea69
#[derive(Debug)]
pub struct ScardAccessStartedEventCall {
    _unused: u32,
}

impl ScardAccessStartedEventCall {
    pub fn decode(payload: &mut ReadCursor<'_>) -> PduResult<Self> {
        Ok(Self {
            _unused: payload.read_u32(),
        })
    }
}

/// [2.2.3.3 Long_Return]
///
/// [2.2.3.3 Long_Return]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/e77a1365-2379-4037-99c4-d30d14ba10fc
#[derive(Debug)]
pub struct LongReturn {
    pub return_code: ReturnCode,
}

impl LongReturn {
    const NAME: &'static str = "Long_Return";
}

impl rpce::HeaderlessEncode for LongReturn {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.return_code as u32);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        4 // ReturnCode
    }
}

/// [2.2.8 Return Code]
///
/// [2.2.8 Return Code]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/9861f8da-76fe-41e6-847e-40c9aa35df8d
#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub enum ReturnCode {
    /// SCARD_S_SUCCESS
    Success = 0x00000000,
    /// SCARD_F_INTERNAL_ERROR
    InternalError = 0x80100001,
    /// SCARD_E_CANCELLED
    Cancelled = 0x80100002,
    /// SCARD_E_INVALID_HANDLE
    InvalidHandle = 0x80100003,
    /// SCARD_E_INVALID_PARAMETER
    InvalidParameter = 0x80100004,
    /// SCARD_E_INVALID_TARGET
    InvalidTarget = 0x80100005,
    /// SCARD_E_NO_MEMORY
    NoMemory = 0x80100006,
    /// SCARD_F_WAITED_TOO_LONG
    WaitedTooLong = 0x80100007,
    /// SCARD_E_INSUFFICIENT_BUFFER
    InsufficientBuffer = 0x80100008,
    /// SCARD_E_UNKNOWN_READER
    UnknownReader = 0x80100009,
    /// SCARD_E_TIMEOUT
    Timeout = 0x8010000A,
    /// SCARD_E_SHARING_VIOLATION
    SharingViolation = 0x8010000B,
    /// SCARD_E_NO_SMARTCARD
    NoSmartcard = 0x8010000C,
    /// SCARD_E_UNKNOWN_CARD
    UnknownCard = 0x8010000D,
    /// SCARD_E_CANT_DISPOSE
    CantDispose = 0x8010000E,
    /// SCARD_E_PROTO_MISMATCH
    ProtoMismatch = 0x8010000F,
    /// SCARD_E_NOT_READY
    NotReady = 0x80100010,
    /// SCARD_E_INVALID_VALUE
    InvalidValue = 0x80100011,
    /// SCARD_E_SYSTEM_CANCELLED
    SystemCancelled = 0x80100012,
    /// SCARD_F_COMM_ERROR
    CommError = 0x80100013,
    /// SCARD_F_UNKNOWN_ERROR
    UnknownError = 0x80100014,
    /// SCARD_E_INVALID_ATR
    InvalidAtr = 0x80100015,
    /// SCARD_E_NOT_TRANSACTED
    NotTransacted = 0x80100016,
    /// SCARD_E_READER_UNAVAILABLE
    ReaderUnavailable = 0x80100017,
    /// SCARD_P_SHUTDOWN
    Shutdown = 0x80100018,
    /// SCARD_E_PCI_TOO_SMALL
    PciTooSmall = 0x80100019,
    /// SCARD_E_ICC_INSTALLATION
    IccInstallation = 0x80100020,
    /// SCARD_E_ICC_CREATEORDER
    IccCreateorder = 0x80100021,
    /// SCARD_E_UNSUPPORTED_FEATURE
    UnsupportedFeature = 0x80100022,
    /// SCARD_E_DIR_NOT_FOUND
    DirNotFound = 0x80100023,
    /// SCARD_E_FILE_NOT_FOUND
    FileNotFound = 0x80100024,
    /// SCARD_E_NO_DIR
    NoDir = 0x80100025,
    /// SCARD_E_READER_UNSUPPORTED
    ReaderUnsupported = 0x8010001A,
    /// SCARD_E_DUPLICATE_READER
    DuplicateReader = 0x8010001B,
    /// SCARD_E_CARD_UNSUPPORTED
    CardUnsupported = 0x8010001C,
    /// SCARD_E_NO_SERVICE
    NoService = 0x8010001D,
    /// SCARD_E_SERVICE_STOPPED
    ServiceStopped = 0x8010001E,
    /// SCARD_E_UNEXPECTED
    Unexpected = 0x8010001F,
    /// SCARD_E_NO_FILE
    NoFile = 0x80100026,
    /// SCARD_E_NO_ACCESS
    NoAccess = 0x80100027,
    /// SCARD_E_WRITE_TOO_MANY
    WriteTooMany = 0x80100028,
    /// SCARD_E_BAD_SEEK
    BadSeek = 0x80100029,
    /// SCARD_E_INVALID_CHV
    InvalidChv = 0x8010002A,
    /// SCARD_E_UNKNOWN_RES_MSG
    UnknownResMsg = 0x8010002B,
    /// SCARD_E_NO_SUCH_CERTIFICATE
    NoSuchCertificate = 0x8010002C,
    /// SCARD_E_CERTIFICATE_UNAVAILABLE
    CertificateUnavailable = 0x8010002D,
    /// SCARD_E_NO_READERS_AVAILABLE
    NoReadersAvailable = 0x8010002E,
    /// SCARD_E_COMM_DATA_LOST
    CommDataLost = 0x8010002F,
    /// SCARD_E_NO_KEY_CONTAINER
    NoKeyContainer = 0x80100030,
    /// SCARD_E_SERVER_TOO_BUSY
    ServerTooBusy = 0x80100031,
    /// SCARD_E_PIN_CACHE_EXPIRED
    PinCacheExpired = 0x80100032,
    /// SCARD_E_NO_PIN_CACHE
    NoPinCache = 0x80100033,
    /// SCARD_E_READ_ONLY_CARD
    ReadOnlyCard = 0x80100034,
    /// SCARD_W_UNSUPPORTED_CARD
    UnsupportedCard = 0x80100065,
    /// SCARD_W_UNRESPONSIVE_CARD
    UnresponsiveCard = 0x80100066,
    /// SCARD_W_UNPOWERED_CARD
    UnpoweredCard = 0x80100067,
    /// SCARD_W_RESET_CARD
    ResetCard = 0x80100068,
    /// SCARD_W_REMOVED_CARD
    RemovedCard = 0x80100069,
    /// SCARD_W_SECURITY_VIOLATION
    SecurityViolation = 0x8010006A,
    /// SCARD_W_WRONG_CHV
    WrongChv = 0x8010006B,
    /// SCARD_W_CHV_BLOCKED
    ChvBlocked = 0x8010006C,
    /// SCARD_W_EOF
    Eof = 0x8010006D,
    /// SCARD_W_CANCELLED_BY_USER
    CancelledByUser = 0x8010006E,
    /// SCARD_W_CARD_NOT_AUTHENTICATED
    CardNotAuthenticated = 0x8010006F,
    /// SCARD_W_CACHE_ITEM_NOT_FOUND
    CacheItemNotFound = 0x80100070,
    /// SCARD_W_CACHE_ITEM_STALE
    CacheItemStale = 0x80100071,
    /// SCARD_W_CACHE_ITEM_TOO_BIG
    CacheItemTooBig = 0x80100072,
}

pub mod rpce {
    //! PDUs for [\[MS-RPCE\]: Remote Procedure Call Protocol Extensions] as required by [MS-RDPESC].
    //!
    //! [\[MS-RPCE\]: Remote Procedure Call Protocol Extensions]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c15

    use ironrdp_pdu::{cursor::WriteCursor, ensure_size, invalid_message_err, PduError, PduResult};

    /// Trait for types that can be encoded into an [MS-RPCE] message.
    ///
    /// Implementers should typically avoid implementing this trait directly
    /// and instead implement [`HeaderlessEncode`].
    pub trait Encode: Send + std::fmt::Debug {
        /// Encodes this RPCE PDU in-place using the provided `WriteCursor`.
        fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()>;

        /// Returns the name associated with this RPCE PDU.
        fn name(&self) -> &'static str;

        /// Computes the size in bytes for this RPCE PDU.
        fn size(&self) -> usize;
    }

    /// Trait for types that can be encoded into an [MS-RPCE] message.
    ///
    /// Implementers should typically implement this trait instead of [`Encode`].
    pub trait HeaderlessEncode: Send + std::fmt::Debug {
        /// Encodes the instance into a buffer sans its [`RpceStreamHeader`] and [`RpceTypeHeader`].
        fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()>;
        /// Returns the name associated with this RPCE PDU.
        fn name(&self) -> &'static str;
        /// Returns the size of the instance sans its [`RpceStreamHeader`] and [`RpceTypeHeader`].
        fn size(&self) -> usize;
    }

    impl<T: HeaderlessEncode> Encode for T {
        fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
            ensure_size!(ctx: self.name(), in: dst, size: self.size());
            let stream_header = StreamHeader::default();
            let type_header = TypeHeader::new(self.size() as u32);

            stream_header.encode(dst)?;
            type_header.encode(dst)?;
            HeaderlessEncode::encode(self, dst)?;

            // Pad response to be 8-byte aligned.
            let padding_size = padding_size(self);
            if padding_size > 0 {
                dst.write_slice(&vec![0; padding_size]);
            }

            Ok(())
        }

        fn name(&self) -> &'static str {
            HeaderlessEncode::name(self)
        }

        fn size(&self) -> usize {
            StreamHeader::size() + TypeHeader::size() + HeaderlessEncode::size(self) + padding_size(self)
        }
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
                filler: 0xcccccccc,
            }
        }
    }

    impl StreamHeader {
        const NAME: &'static str = "RpceStreamHeader";

        fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
            ensure_size!(in: dst, size: Self::size());
            dst.write_u8(self.version);
            dst.write_u8(self.endianness as u8);
            dst.write_u16(self.common_header_length);
            dst.write_u32(self.filler);
            Ok(())
        }

        fn size() -> usize {
            8
        }
    }

    #[derive(Debug, Clone, Copy)]
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

        pub fn new(object_buffer_length: u32) -> Self {
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
    }

    impl TypeHeader {
        fn size() -> usize {
            8
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
