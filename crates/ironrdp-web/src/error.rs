use iron_remote_desktop::{IronError, IronErrorKind};
use ironrdp::connector::{self, sspi, ConnectorErrorKind};

pub(crate) struct RdpIronError {
    kind: IronErrorKind,
    source: anyhow::Error,
}

impl RdpIronError {
    pub(crate) fn with_kind(mut self, kind: IronErrorKind) -> Self {
        self.kind = kind;
        self
    }
}

impl IronError for RdpIronError {
    fn backtrace(&self) -> String {
        format!("{:?}", self.source)
    }

    fn kind(&self) -> IronErrorKind {
        self.kind
    }
}

impl From<connector::ConnectorError> for RdpIronError {
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

impl From<ironrdp::session::SessionError> for RdpIronError {
    fn from(e: ironrdp::session::SessionError) -> Self {
        Self {
            kind: IronErrorKind::General,
            source: anyhow::Error::new(e),
        }
    }
}

impl From<anyhow::Error> for RdpIronError {
    fn from(e: anyhow::Error) -> Self {
        Self {
            kind: IronErrorKind::General,
            source: e,
        }
    }
}
