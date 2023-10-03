//! PDUs for [\[MS-RDPESC\]: Remote Desktop Protocol: Smart Card Virtual Channel Extension]
//!
//! [\[MS-RDPESC\]: Remote Desktop Protocol: Smart Card Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/0428ca28-b4dc-46a3-97c3-01887fa44a90

use super::efs::IoCtlCode;
use ironrdp_pdu::{
    cursor::{ReadCursor, WriteCursor},
    ensure_size, invalid_message_err, PduError, PduResult,
};

/// From [3.1.4 Message Processing Events and Sequencing Rules]
///
/// [3.1.4 Message Processing Events and Sequencing Rules]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpesc/60d5977d-0017-4c90-ab0c-f34bf44a74a5
#[derive(Debug)]
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
    pub fn decode(payload: &mut ReadCursor<'_>) -> PduResult<Self> {
        ironrdp_pdu::read_padding!(payload, 4); // Unused (4 bytes)
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

impl From<ReturnCode> for u32 {
    fn from(val: ReturnCode) -> Self {
        val as u32
    }
}

pub mod rpce {
    //! PDUs for [\[MS-RPCE\]: Remote Procedure Call Protocol Extensions] as required by [MS-RDPESC].
    //!
    //! [\[MS-RPCE\]: Remote Procedure Call Protocol Extensions]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c15

    use ironrdp_pdu::{cursor::WriteCursor, ensure_size, invalid_message_err, PduEncode, PduError, PduResult};

    /// Wrapper struct for [MS-RPCE] PDUs that allows for a common [`PduEncode`] and [`Encode`] implementation.
    ///
    /// Structs which are meant to be encoded into an [MS-RPCE] message should typically implement [`HeaderlessEncode`],
    /// and their `new` function should return a [`Pdu`] wrapping the underlying struct.
    ///
    /// ```rust
    /// #[derive(Debug)]
    /// pub struct SomeRpcePdu {
    ///     example_field: u32,
    /// }
    ///
    /// impl SomeRpcePdu {
    ///     /// `new` returns a `Pdu` wrapping the underlying struct.
    ///     pub fn new(example_field: u32) -> rpce::Pdu<Self> {
    ///         rpce::Pdu(Self { example_field })
    ///     }
    /// }
    ///
    /// /// The underlying struct should implement `HeaderlessEncode`.
    /// impl rpce::HeaderlessEncode for SomeRpcePdu {
    ///     fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
    ///         ensure_size!(in: dst, size: self.size());
    ///         dst.write_u32(self.return_code.into());
    ///         Ok(())
    ///     }
    ///
    ///     fn name(&self) -> &'static str {
    ///         "SomeRpcePdu"
    ///     }
    ///
    ///     fn size(&self) -> usize {
    ///         4 // u32 == 4 bytes
    ///     }
    /// }
    /// ```
    ///
    /// See [`super::LongReturn`] for a live example.
    #[derive(Debug)]
    pub struct Pdu<T: HeaderlessEncode>(pub T);

    impl<T: HeaderlessEncode> PduEncode for Pdu<T> {
        fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
            ensure_size!(ctx: self.name(), in: dst, size: self.size());
            let stream_header = StreamHeader::default();
            let type_header = TypeHeader::new(self.size() as u32);

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
    pub trait Encode: PduEncode + Send + std::fmt::Debug {}

    /// Trait for types that can be encoded into an [MS-RPCE] message.
    ///
    /// Implementers should typically implement this trait instead of [`Encode`].
    pub trait HeaderlessEncode: Send + std::fmt::Debug {
        /// Encodes the instance into a buffer sans its [`StreamHeader`] and [`TypeHeader`].
        fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()>;
        /// Returns the name associated with this RPCE PDU.
        fn name(&self) -> &'static str;
        /// Returns the size of the instance sans its [`StreamHeader`] and [`TypeHeader`].
        fn size(&self) -> usize;
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
