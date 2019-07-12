use std::{error::Error, fmt, io, str};

use serde_derive::{Deserialize, Serialize};

use crate::utils;

pub type SspiResult = std::result::Result<SspiOk, SspiError>;
pub type Result<T> = std::result::Result<T, SspiError>;

pub trait Sspi {
    fn package_type(&self) -> PackageType;
    fn identity(&self) -> Option<CredentialsBuffers>;
    fn update_identity(&mut self, identity: Option<CredentialsBuffers>);
    fn initialize_security_context(
        &mut self,
        input: impl io::Read,
        output: impl io::Write,
    ) -> self::SspiResult;
    fn accept_security_context(
        &mut self,
        input: impl io::Read,
        output: impl io::Write,
    ) -> self::SspiResult;
    fn complete_auth_token(&mut self) -> self::Result<()>;
    fn encrypt_message(&mut self, input: &[u8], message_seq_number: u32) -> self::Result<Vec<u8>>;
    fn decrypt_message(&mut self, input: &[u8], message_seq_number: u32) -> self::Result<Vec<u8>>;
}

pub enum PackageType {
    Ntlm,
}

/// Owns credentials of an identity used in negotiations and communications.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
    pub domain: Option<String>,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct CredentialsBuffers {
    pub user: Vec<u8>,
    pub domain: Vec<u8>,
    pub password: Vec<u8>,
}

impl Credentials {
    pub fn new(username: String, password: String, domain: Option<String>) -> Self {
        Self {
            username,
            password,
            domain,
        }
    }
}

impl CredentialsBuffers {
    pub fn new(user: Vec<u8>, domain: Vec<u8>, password: Vec<u8>) -> Self {
        Self {
            user,
            domain,
            password,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.user.is_empty() || self.password.is_empty()
    }
}

impl From<Credentials> for CredentialsBuffers {
    fn from(credentials: Credentials) -> Self {
        Self {
            user: utils::string_to_utf16(credentials.username.as_str()),
            domain: credentials
                .domain
                .map(|v| utils::string_to_utf16(v.as_str()))
                .unwrap_or_default(),
            password: utils::string_to_utf16(credentials.password.as_str()),
        }
    }
}

impl From<CredentialsBuffers> for Credentials {
    fn from(credentials_buffers: CredentialsBuffers) -> Self {
        Self {
            username: utils::bytes_to_utf16_string(credentials_buffers.user.as_ref()),
            password: utils::bytes_to_utf16_string(credentials_buffers.password.as_ref()),
            domain: if credentials_buffers.domain.is_empty() {
                None
            } else {
                Some(utils::bytes_to_utf16_string(
                    credentials_buffers.domain.as_ref(),
                ))
            },
        }
    }
}

/// The kind of an SSPI related error. Enables to specify the error based on its type.
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum SspiErrorType {
    /// May correspond to any internal error (I/O error, server error, etc.).
    InternalError = 0x8009_0304,
    /// Used in cases when supplied data is missing or invalid.
    InvalidToken = 0x8009_0308,
    /// Used when a required NTLM state does not correspond to the current.
    OutOfSequence = 0x8009_0310,
    /// Used in contexts of supplying invalid credentials.
    MessageAltered = 0x8009_030F,
    TargetUnknown = 0x8009_0303,
}

/// Holds the [`SspiErrorType`](enum.SspiErrorType.html) and the description of the error.
#[derive(Debug, PartialEq)]
pub struct SspiError {
    pub error_type: SspiErrorType,
    pub description: String,
}

#[derive(Debug, PartialEq)]
pub enum SspiOk {
    ContinueNeeded = 0x0009_0312,
    CompleteNeeded = 0x0009_0313,
}

impl SspiError {
    /// Allows to fill a new error easily, supplying it with a coherent description.
    pub fn new(error_type: SspiErrorType, error: String) -> Self {
        Self {
            error_type,
            description: error,
        }
    }
}

impl Error for SspiError {}

impl fmt::Display for SspiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl From<io::Error> for SspiError {
    fn from(err: io::Error) -> Self {
        Self::new(
            SspiErrorType::InternalError,
            format!("IO error: {}", err.to_string()),
        )
    }
}

impl From<rand::Error> for SspiError {
    fn from(err: rand::Error) -> Self {
        Self::new(
            SspiErrorType::InternalError,
            format!("Rand error: {}", err.to_string()),
        )
    }
}

impl From<std::string::FromUtf16Error> for SspiError {
    fn from(err: std::string::FromUtf16Error) -> Self {
        Self::new(
            SspiErrorType::InternalError,
            format!("UTF-16 error: {}", err.to_string()),
        )
    }
}

impl From<SspiError> for io::Error {
    fn from(err: SspiError) -> io::Error {
        io::Error::new(
            io::ErrorKind::Other,
            format!("{:?}: {}", err.error_type, err.description),
        )
    }
}

impl fmt::Display for SspiOk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
