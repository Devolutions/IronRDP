use core::fmt;

/// Provides user-friendly error messages for RDP negotiation failures
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NegotiationFailure(ironrdp_pdu::nego::FailureCode);

impl NegotiationFailure {
    pub fn code(self) -> ironrdp_pdu::nego::FailureCode {
        self.0
    }
}

impl core::error::Error for NegotiationFailure {}

impl From<ironrdp_pdu::nego::FailureCode> for NegotiationFailure {
    fn from(code: ironrdp_pdu::nego::FailureCode) -> Self {
        Self(code)
    }
}

impl fmt::Display for NegotiationFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ironrdp_pdu::nego::FailureCode;

        match self.0 {
            FailureCode::SSL_REQUIRED_BY_SERVER => {
                write!(f, "server requires Enhanced RDP Security with TLS or CredSSP")
            }
            FailureCode::SSL_NOT_ALLOWED_BY_SERVER => {
                write!(f, "server only supports Standard RDP Security")
            }
            FailureCode::SSL_CERT_NOT_ON_SERVER => {
                write!(f, "server lacks valid authentication certificate")
            }
            FailureCode::INCONSISTENT_FLAGS => {
                write!(f, "inconsistent security protocol flags")
            }
            FailureCode::HYBRID_REQUIRED_BY_SERVER => {
                write!(f, "server requires Enhanced RDP Security with CredSSP")
            }
            FailureCode::SSL_WITH_USER_AUTH_REQUIRED_BY_SERVER => {
                write!(
                    f,
                    "server requires Enhanced RDP Security with TLS and client certificate"
                )
            }
            _ => write!(f, "unknown negotiation failure (code: 0x{:08x})", u32::from(self.0)),
        }
    }
}
