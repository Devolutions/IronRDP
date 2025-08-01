use wasm_bindgen::prelude::*;

pub trait IronError {
    fn backtrace(&self) -> String;

    fn kind(&self) -> IronErrorKind;
}

#[derive(Clone, Copy)]
#[wasm_bindgen]
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
    /// Server requires Enhanced RDP Security with TLS or CredSSP
    SslRequiredByServer,
    /// Server only supports Standard RDP Security
    SslNotAllowedByServer,
    /// Server lacks valid authentication certificate
    SslCertNotOnServer,
    /// Inconsistent security protocol flags
    InconsistentFlags,
    /// Server requires Enhanced RDP Security with CredSSP
    HybridRequiredByServer,
    /// Server requires Enhanced RDP Security with TLS and client certificate
    SslWithUserAuthRequiredByServer,
}
