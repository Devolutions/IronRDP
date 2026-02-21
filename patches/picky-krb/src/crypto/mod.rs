pub mod aes;
mod checksum;
mod cipher;
pub(crate) mod common;
pub mod des;
pub mod diffie_hellman;
pub(crate) mod nfold;
pub(crate) mod utils;

use ::aes::cipher::block_padding::UnpadError;
use ::aes::cipher::inout::PadError;
use thiserror::Error;

/// https://www.rfc-editor.org/rfc/rfc3962.html#section-4
/// the 8-octet ASCII string "kerberos"
pub const KERBEROS: &[u8; 8] = b"kerberos";

#[derive(Error, Debug)]
pub enum KerberosCryptoError {
    #[error("Invalid key length: {0}. Expected: {1}")]
    KeyLength(usize, usize),
    #[error("Invalid cipher length: {0}. Expected at least: {1}")]
    CipherLength(usize, usize),
    #[error("Invalid algorithm identifier: {0}")]
    AlgorithmIdentifier(usize),
    #[error("Invalid algorithm identifier: {0:?}")]
    AlgorithmIdentifierData(Vec<u8>),
    #[error("Bad integrity: calculated hmac is different than provided")]
    IntegrityCheck,
    #[error("Cipher error: {0}")]
    CipherError(String),
    #[error("Padding error: {0:?}")]
    CipherUnpad(UnpadError),
    #[error("Padding error: {0:?}")]
    CipherPad(PadError),
    #[error("Invalid seed bit len: {0}")]
    SeedBitLen(String),
    #[error(transparent)]
    RandError(#[from] rand::rand_core::OsError),
    #[error(transparent)]
    TooSmallBuffer(#[from] inout::OutIsTooSmallError),
    #[error(transparent)]
    ArrayTryFromSliceError(#[from] std::array::TryFromSliceError),
}

impl From<UnpadError> for KerberosCryptoError {
    fn from(err: UnpadError) -> Self {
        Self::CipherUnpad(err)
    }
}

impl From<PadError> for KerberosCryptoError {
    fn from(err: PadError) -> Self {
        Self::CipherPad(err)
    }
}

pub struct DecryptWithoutChecksum {
    pub plaintext: Vec<u8>,
    pub confounder: Vec<u8>,
    pub checksum: Vec<u8>,
    pub ki: Vec<u8>,
}

pub struct EncryptWithoutChecksum {
    pub encrypted: Vec<u8>,
    pub confounder: Vec<u8>,
    pub ki: Vec<u8>,
}

pub type KerberosCryptoResult<T> = Result<T, KerberosCryptoError>;

pub use checksum::{Checksum, ChecksumSuite};
pub use cipher::{Cipher, CipherSuite};
