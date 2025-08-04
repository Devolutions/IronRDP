use iron_remote_desktop::IronErrorKind;
use ironrdp::connector::{self, sspi, ConnectorErrorKind};

pub(crate) struct IronError {
    kind: IronErrorKind,
    source: anyhow::Error,
}

impl core::fmt::Debug for IronError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IronError").field("source", &self.source).finish()
    }
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
