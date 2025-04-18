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
}
