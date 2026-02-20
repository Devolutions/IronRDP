use iron_remote_desktop::{IronErrorKind, RDCleanPathDetails};
use ironrdp::connector::{self, sspi, ConnectorErrorKind};

pub(crate) struct IronError {
    kind: IronErrorKind,
    source: anyhow::Error,
    rdcleanpath_details: Option<RDCleanPathDetails>,
}

impl IronError {
    pub(crate) fn with_kind(mut self, kind: IronErrorKind) -> Self {
        self.kind = kind;
        self
    }

    pub(crate) fn with_rdcleanpath_details(mut self, details: RDCleanPathDetails) -> Self {
        debug_assert!(
            matches!(self.kind, IronErrorKind::RDCleanPath),
            "rdcleanpath_details should only be set for RDCleanPath errors"
        );
        self.rdcleanpath_details = Some(details);
        self
    }
}

impl iron_remote_desktop::IronError for IronError {
    fn backtrace(&self) -> String {
        format!("{:#}", self.source)
    }

    fn kind(&self) -> IronErrorKind {
        self.kind
    }

    fn rdcleanpath_details(&self) -> Option<RDCleanPathDetails> {
        self.rdcleanpath_details
    }
}

impl From<connector::ConnectorError> for IronError {
    fn from(e: connector::ConnectorError) -> Self {
        use sspi::credssp::NStatusCode;

        let kind = match e.kind() {
            ConnectorErrorKind::Credssp(sspi::Error {
                nstatus: Some(NStatusCode::WRONG_PASSWORD),
                ..
            }) => IronErrorKind::WrongPassword,
            ConnectorErrorKind::Credssp(sspi::Error {
                nstatus: Some(NStatusCode::LOGON_FAILURE),
                ..
            }) => IronErrorKind::LogonFailure,
            ConnectorErrorKind::AccessDenied => IronErrorKind::AccessDenied,
            ConnectorErrorKind::Negotiation(_) => IronErrorKind::NegotiationFailure,
            _ => IronErrorKind::General,
        };

        Self {
            kind,
            source: anyhow::Error::new(e),
            rdcleanpath_details: None,
        }
    }
}

impl From<ironrdp::session::SessionError> for IronError {
    fn from(e: ironrdp::session::SessionError) -> Self {
        Self {
            kind: IronErrorKind::General,
            source: anyhow::Error::new(e),
            rdcleanpath_details: None,
        }
    }
}

impl From<anyhow::Error> for IronError {
    fn from(e: anyhow::Error) -> Self {
        Self {
            kind: IronErrorKind::General,
            source: e,
            rdcleanpath_details: None,
        }
    }
}
