use crate::constants::etypes::{AES128_CTS_HMAC_SHA1_96, AES256_CTS_HMAC_SHA1_96, DES3_CBC_SHA1_KD};

use super::aes::{Aes128CtsHmacSha196, Aes256CtsHmacSha196};
use super::des::Des3CbcSha1Kd;
use super::{ChecksumSuite, DecryptWithoutChecksum, EncryptWithoutChecksum, KerberosCryptoError, KerberosCryptoResult};

pub trait Cipher {
    fn key_size(&self) -> usize;
    fn seed_bit_len(&self) -> usize;
    fn cipher_type(&self) -> CipherSuite;
    fn checksum_type(&self) -> ChecksumSuite;

    fn encrypt(&self, key: &[u8], key_usage: i32, payload: &[u8]) -> KerberosCryptoResult<Vec<u8>>;
    fn decrypt(&self, key: &[u8], key_usage: i32, cipher_data: &[u8]) -> KerberosCryptoResult<Vec<u8>>;

    fn encrypt_no_checksum(
        &self,
        key: &[u8],
        key_usage: i32,
        payload: &[u8],
    ) -> KerberosCryptoResult<EncryptWithoutChecksum>;
    fn decrypt_no_checksum(
        &self,
        key: &[u8],
        key_usage: i32,
        cipher_data: &[u8],
    ) -> KerberosCryptoResult<DecryptWithoutChecksum>;

    /// Calculates Kerberos checksum over the provided data.
    ///
    /// Note: this method differs from [Checksum::checksum]. Key derivation processes for
    /// encryption checksum and just checksum are different. More details:
    /// * [Encryption and Checksum Specifications for Kerberos 5](https://datatracker.ietf.org/doc/html/rfc3961).
    fn encryption_checksum(&self, key: &[u8], key_usage: i32, payload: &[u8]) -> KerberosCryptoResult<Vec<u8>>;

    fn generate_key_from_password(&self, password: &[u8], salt: &[u8]) -> KerberosCryptoResult<Vec<u8>>;
    fn random_to_key(&self, key: Vec<u8>) -> Vec<u8>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CipherSuite {
    Aes128CtsHmacSha196,
    Aes256CtsHmacSha196,
    Des3CbcSha1Kd,
}

impl CipherSuite {
    pub fn cipher(&self) -> Box<dyn Cipher> {
        match self {
            CipherSuite::Aes256CtsHmacSha196 => Box::new(Aes256CtsHmacSha196::new()),
            CipherSuite::Aes128CtsHmacSha196 => Box::new(Aes128CtsHmacSha196::new()),
            CipherSuite::Des3CbcSha1Kd => Box::new(Des3CbcSha1Kd::new()),
        }
    }
}

impl TryFrom<&[u8]> for CipherSuite {
    type Error = KerberosCryptoError;

    fn try_from(identifier: &[u8]) -> Result<Self, Self::Error> {
        if identifier.len() != 1 {
            return Err(KerberosCryptoError::AlgorithmIdentifierData(identifier.into()));
        }

        match u8::from_be_bytes(identifier.try_into().unwrap()) as usize {
            AES256_CTS_HMAC_SHA1_96 => Ok(Self::Aes256CtsHmacSha196),
            AES128_CTS_HMAC_SHA1_96 => Ok(Self::Aes128CtsHmacSha196),
            DES3_CBC_SHA1_KD => Ok(Self::Des3CbcSha1Kd),
            _ => Err(KerberosCryptoError::AlgorithmIdentifierData(identifier.into())),
        }
    }
}

impl TryFrom<usize> for CipherSuite {
    type Error = KerberosCryptoError;

    fn try_from(identifier: usize) -> Result<Self, Self::Error> {
        match identifier {
            AES256_CTS_HMAC_SHA1_96 => Ok(Self::Aes256CtsHmacSha196),
            AES128_CTS_HMAC_SHA1_96 => Ok(Self::Aes128CtsHmacSha196),
            DES3_CBC_SHA1_KD => Ok(Self::Des3CbcSha1Kd),
            _ => Err(KerberosCryptoError::AlgorithmIdentifier(identifier)),
        }
    }
}

impl From<CipherSuite> for usize {
    fn from(cipher: CipherSuite) -> Self {
        match cipher {
            CipherSuite::Aes256CtsHmacSha196 => AES256_CTS_HMAC_SHA1_96,
            CipherSuite::Aes128CtsHmacSha196 => AES128_CTS_HMAC_SHA1_96,
            CipherSuite::Des3CbcSha1Kd => DES3_CBC_SHA1_KD,
        }
    }
}

impl From<&CipherSuite> for u32 {
    fn from(cipher: &CipherSuite) -> Self {
        match cipher {
            CipherSuite::Aes256CtsHmacSha196 => AES256_CTS_HMAC_SHA1_96 as u32,
            CipherSuite::Aes128CtsHmacSha196 => AES128_CTS_HMAC_SHA1_96 as u32,
            CipherSuite::Des3CbcSha1Kd => DES3_CBC_SHA1_KD as u32,
        }
    }
}

impl From<CipherSuite> for u8 {
    fn from(cipher: CipherSuite) -> Self {
        match cipher {
            CipherSuite::Aes256CtsHmacSha196 => AES256_CTS_HMAC_SHA1_96 as u8,
            CipherSuite::Aes128CtsHmacSha196 => AES128_CTS_HMAC_SHA1_96 as u8,
            CipherSuite::Des3CbcSha1Kd => DES3_CBC_SHA1_KD as u8,
        }
    }
}

impl From<&CipherSuite> for u8 {
    fn from(cipher: &CipherSuite) -> Self {
        match cipher {
            CipherSuite::Aes256CtsHmacSha196 => AES256_CTS_HMAC_SHA1_96 as u8,
            CipherSuite::Aes128CtsHmacSha196 => AES128_CTS_HMAC_SHA1_96 as u8,
            CipherSuite::Des3CbcSha1Kd => DES3_CBC_SHA1_KD as u8,
        }
    }
}
