use ironrdp::connector::{self, sspi, ConnectorErrorKind};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum IronErrorKind {
    /// Catch-all error kind
    General,
    /// Incorrect password used
    WrongPassword,
    /// Unable to login to machine
    LogonFailure,
    /// Insufficient permission, server denied access
    AccessDenied,
    /// Something wrong happened when sending or receiving the RDCleanPath message
    RDCleanPath,
    /// Couldn’t connect to proxy
    ProxyConnect,
}

#[wasm_bindgen]
pub struct IronError {
    kind: IronErrorKind,
    source: anyhow::Error,
}

impl IronError {
    pub fn with_kind(mut self, kind: IronErrorKind) -> Self {
        self.kind = kind;
        self
    }
}

#[wasm_bindgen]
impl IronError {
    pub fn backtrace(&self) -> String {
        format!("{:?}", self.source)
    }

    pub fn kind(&self) -> IronErrorKind {
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
