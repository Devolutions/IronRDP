use ironrdp::connector::{self, sspi, ConnectorErrorKind};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum IronRdpErrorKind {
    /// Catch-all error kind
    General,
    /// Incorrect password used
    WrongPassword,
    /// Unable to login to machine
    LogonFailure,
    /// Insufficient permission, server denied access
    AccessDenied,
    /// Something wrong happened when sending or receiving the RDClenaPath message
    RDCleanPath,
    /// Couldn’t connect to proxy
    ProxyConnect,
}

#[wasm_bindgen]
pub struct IronRdpError {
    kind: IronRdpErrorKind,
    source: anyhow::Error,
}

impl IronRdpError {
    pub fn with_kind(mut self, kind: IronRdpErrorKind) -> Self {
        self.kind = kind;
        self
    }
}

#[wasm_bindgen]
impl IronRdpError {
    pub fn backtrace(&self) -> String {
        format!("{:?}", self.source)
    }

    pub fn kind(&self) -> IronRdpErrorKind {
        self.kind
    }
}

impl From<connector::ConnectorError> for IronRdpError {
    fn from(e: connector::ConnectorError) -> Self {
        use sspi::credssp::NStatusCode;

        let kind = match e.kind {
            ConnectorErrorKind::Credssp(sspi::Error {
                nstatus: Some(NStatusCode::WRONG_PASSWORD),
                ..
            }) => IronRdpErrorKind::WrongPassword,
            ConnectorErrorKind::Credssp(sspi::Error {
                nstatus: Some(NStatusCode::LOGON_FAILURE),
                ..
            }) => IronRdpErrorKind::LogonFailure,
            ConnectorErrorKind::AccessDenied => IronRdpErrorKind::AccessDenied,
            _ => IronRdpErrorKind::General,
        };

        Self {
            kind,
            source: anyhow::Error::new(e),
        }
    }
}

impl From<ironrdp::session::SessionError> for IronRdpError {
    fn from(e: ironrdp::session::SessionError) -> Self {
        Self {
            kind: IronRdpErrorKind::General,
            source: anyhow::Error::new(e),
        }
    }
}

impl From<anyhow::Error> for IronRdpError {
    fn from(e: anyhow::Error) -> Self {
        Self {
            kind: IronRdpErrorKind::General,
            source: e,
        }
    }
}
