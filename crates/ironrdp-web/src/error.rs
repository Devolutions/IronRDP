use iron_remote_desktop::IronErrorKind;
use ironrdp::connector::{self, sspi, ConnectorErrorKind};
use tracing::info;

pub(crate) struct IronError {
    kind: IronErrorKind,
    source: anyhow::Error,
}

impl IronError {
    pub(crate) fn with_kind(mut self, kind: IronErrorKind) -> Self {
        self.kind = kind;
        self
    }
}

impl iron_remote_desktop::IronError for IronError {
    fn backtrace(&self) -> String {
        format!("{:?}", self.source)
    }

    fn kind(&self) -> IronErrorKind {
        self.kind
    }
}

impl From<connector::ConnectorError> for IronError {
    fn from(e: connector::ConnectorError) -> Self {
        use sspi::credssp::NStatusCode;

        let kind = match e.kind {
            ConnectorErrorKind::Credssp(sspi::Error {
                nstatus: Some(NStatusCode::WRONG_PASSWORD),
                ..
            }) => IronErrorKind::WrongPassword,
            ConnectorErrorKind::Credssp(sspi::Error {
                nstatus: Some(NStatusCode::LOGON_FAILURE),
                ..
            }) => IronErrorKind::LogonFailure,
            ConnectorErrorKind::AccessDenied => IronErrorKind::AccessDenied,
            ConnectorErrorKind::NegotiationFailure(code) => {
                use ironrdp::pdu::nego::FailureCode;
                info!("RDP negotiation failure: {} (code: 0x{:08x})", code, u32::from(code));
                match code {
                    FailureCode::SSL_REQUIRED_BY_SERVER => IronErrorKind::SslRequiredByServer,
                    FailureCode::SSL_NOT_ALLOWED_BY_SERVER => IronErrorKind::SslNotAllowedByServer,
                    FailureCode::SSL_CERT_NOT_ON_SERVER => IronErrorKind::SslCertNotOnServer,
                    FailureCode::INCONSISTENT_FLAGS => IronErrorKind::InconsistentFlags,
                    FailureCode::HYBRID_REQUIRED_BY_SERVER => IronErrorKind::HybridRequiredByServer,
                    FailureCode::SSL_WITH_USER_AUTH_REQUIRED_BY_SERVER => {
                        IronErrorKind::SslWithUserAuthRequiredByServer
                    }
                    _ => IronErrorKind::General,
                }
            }
            _ => IronErrorKind::General,
        };

        Self {
            kind,
            source: anyhow::Error::new(e),
        }
    }
}

impl From<ironrdp::session::SessionError> for IronError {
    fn from(e: ironrdp::session::SessionError) -> Self {
        Self {
            kind: IronErrorKind::General,
            source: anyhow::Error::new(e),
        }
    }
}

impl From<anyhow::Error> for IronError {
    fn from(e: anyhow::Error) -> Self {
        Self {
            kind: IronErrorKind::General,
            source: e,
        }
    }
}
