use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_pdu::{PduDecode, PduEncode, PduError, PduResult};

/// Error or status severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NowSeverity {
    /// Informative status
    ///
    /// NOW-PROTO: NOW_SEVERITY_INFO
    Info = 0,
    /// Warning status
    ///
    /// NOW-PROTO: NOW_SEVERITY_WARN
    Warn = 1,
    /// Error status (recoverable)
    ///
    /// NOW-PROTO: NOW_SEVERITY_ERROR
    Error = 2,
    /// Error status (non-recoverable)
    ///
    /// NOW-PROTO: NOW_SEVERITY_FATAL
    Fatal = 3,
}

impl TryFrom<u8> for NowSeverity {
    type Error = PduError;

    fn try_from(value: u8) -> PduResult<Self> {
        match value {
            0 => Ok(Self::Info),
            1 => Ok(Self::Warn),
            2 => Ok(Self::Error),
            3 => Ok(Self::Fatal),
            _ => Err(invalid_message_err!("severity", "invalid value")),
        }
    }
}

/// Error or status code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NowStatusCode(pub u16);

impl NowStatusCode {
    /// NOW-PROTO: NOW_CODE_SUCCESS
    pub const SUCCESS: Self = Self(0x0000);
    /// NOW-PROTO: NOW_CODE_FAILURE
    pub const FAILURE: Self = Self(0xFFFF);
    /// NOW-PROTO: NOW_CODE_FILE_NOT_FOUND
    pub const FILE_NOT_FOUND: Self = Self(0x0002);
    /// NOW-PROTO: NOW_CODE_ACCESS_DENIED
    pub const ACCESS_DENIED: Self = Self(0x0005);
    /// NOW-PROTO: NOW_CODE_BAD_FORMAT
    pub const BAD_FORMAT: Self = Self(0x000B);
}
/// A status code, with a structure similar to HRESULT.
///
/// NOW-PROTO: NOW_STATUS
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowStatus {
    severity: NowSeverity,
    kind: u8,
    code: NowStatusCode,
}

impl NowStatus {
    const NAME: &'static str = "NOW_STATUS";
    const FIXED_PART_SIZE: usize = 4;

    pub fn new(severity: NowSeverity, code: NowStatusCode) -> Self {
        Self {
            severity,
            kind: 0,
            code,
        }
    }

    pub fn with_kind(self, kind: u8) -> PduResult<Self> {
        if kind > 0x0F {
            return Err(invalid_message_err!("type", "status type is too large"));
        }

        Ok(Self { kind, ..self })
    }

    pub fn severity(&self) -> NowSeverity {
        self.severity
    }

    pub fn kind(&self) -> u8 {
        self.kind
    }

    pub fn code(&self) -> NowStatusCode {
        self.code
    }
}

impl PduEncode for NowStatus {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        // Y, Z, class fields are reserved and must be set to 0.
        let header_byte = (self.severity as u8) << 6;

        dst.write_u8(header_byte);
        dst.write_u8(self.kind);
        dst.write_u16(self.code.0);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl PduDecode<'_> for NowStatus {
    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let header_byte = src.read_u8();
        let severity = (header_byte >> 6) & 0x03;
        let kind = src.read_u8();
        let code = src.read_u16();

        Ok(NowStatus {
            severity: NowSeverity::try_from(severity)?,
            kind,
            code: NowStatusCode(code),
        })
    }
}
