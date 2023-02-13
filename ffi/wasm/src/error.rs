use wasm_bindgen::prelude::*;

#[wasm_bindgen]
#[derive(Clone, Copy)]
pub enum IronRdpErrorKind {
    InvalidCredentials,
    General,
}

#[wasm_bindgen]
pub struct IronRdpError {
    pub kind: IronRdpErrorKind,
    source: anyhow::Error,
}

#[wasm_bindgen]
impl IronRdpError {
    pub fn stacktrace(&self) -> String {
        format!("{:?}", self.source)
    }
}

impl From<ironrdp_session::RdpError> for IronRdpError {
    fn from(e: ironrdp_session::RdpError) -> Self {
        let kind = match &e {
            ironrdp_session::RdpError::CredSsp(e) => match e.error_type {
                // NOTE: this is a quick & dirty solution, needs a LOT of refinements
                sspi::ErrorKind::LogonDenied => IronRdpErrorKind::InvalidCredentials,
                sspi::ErrorKind::UnknownCredentials => IronRdpErrorKind::InvalidCredentials,
                sspi::ErrorKind::NoCredentials => IronRdpErrorKind::InvalidCredentials,
                sspi::ErrorKind::IncompleteCredentials => IronRdpErrorKind::InvalidCredentials,

                _ => IronRdpErrorKind::General,
            },
            _ => IronRdpErrorKind::General,
        };

        Self {
            kind,
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
