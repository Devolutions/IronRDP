use ironrdp::connector::{self, sspi, ConnectorErrorKind};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum RemoteDesktopErrorKind {
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
pub struct RemoteDesktopError {
    kind: RemoteDesktopErrorKind,
    source: anyhow::Error,
}

impl RemoteDesktopError {
    pub fn with_kind(mut self, kind: RemoteDesktopErrorKind) -> Self {
        self.kind = kind;
        self
    }
}

#[wasm_bindgen]
impl RemoteDesktopError {
    pub fn backtrace(&self) -> String {
        format!("{:?}", self.source)
    }

    pub fn kind(&self) -> RemoteDesktopErrorKind {
        self.kind
    }
}

impl From<connector::ConnectorError> for RemoteDesktopError {
    fn from(e: connector::ConnectorError) -> Self {
        use sspi::credssp::NStatusCode;

        let kind = match e.kind {
            ConnectorErrorKind::Credssp(sspi::Error {
                nstatus: Some(NStatusCode::WRONG_PASSWORD),
                ..
            }) => RemoteDesktopErrorKind::WrongPassword,
            ConnectorErrorKind::Credssp(sspi::Error {
                nstatus: Some(NStatusCode::LOGON_FAILURE),
                ..
            }) => RemoteDesktopErrorKind::LogonFailure,
            ConnectorErrorKind::AccessDenied => RemoteDesktopErrorKind::AccessDenied,
            _ => RemoteDesktopErrorKind::General,
        };

        Self {
            kind,
            source: anyhow::Error::new(e),
        }
    }
}

impl From<ironrdp::session::SessionError> for RemoteDesktopError {
    fn from(e: ironrdp::session::SessionError) -> Self {
        Self {
            kind: RemoteDesktopErrorKind::General,
            source: anyhow::Error::new(e),
        }
    }
}

impl From<anyhow::Error> for RemoteDesktopError {
    fn from(e: anyhow::Error) -> Self {
        Self {
            kind: RemoteDesktopErrorKind::General,
            source: e,
        }
    }
}
