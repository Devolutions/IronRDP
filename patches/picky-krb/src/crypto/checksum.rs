use crate::constants::cksum_types::{HMAC_SHA1_96_AES128, HMAC_SHA1_96_AES256, HMAC_SHA1_DES3_KD};

use super::aes::{HmacSha196Aes128, HmacSha196Aes256};
use super::des::HmacSha1Des3Kd;
use super::{KerberosCryptoError, KerberosCryptoResult};

pub trait Checksum {
    fn checksum_type(&self) -> ChecksumSuite;
    fn checksum(&self, key: &[u8], key_usage: i32, payload: &[u8]) -> KerberosCryptoResult<Vec<u8>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChecksumSuite {
    HmacSha196Aes256,
    HmacSha196Aes128,
    HmacSha1Des3Kd,
}

impl ChecksumSuite {
    pub fn hasher(&self) -> Box<dyn Checksum> {
        match self {
            ChecksumSuite::HmacSha196Aes256 => Box::<HmacSha196Aes256>::default(),
            ChecksumSuite::HmacSha196Aes128 => Box::<HmacSha196Aes128>::default(),
            ChecksumSuite::HmacSha1Des3Kd => Box::<HmacSha1Des3Kd>::default(),
        }
    }
}

impl TryFrom<usize> for ChecksumSuite {
    type Error = KerberosCryptoError;

    fn try_from(identifier: usize) -> Result<Self, Self::Error> {
        match identifier {
            HMAC_SHA1_96_AES256 => Ok(Self::HmacSha196Aes256),
            HMAC_SHA1_96_AES128 => Ok(Self::HmacSha196Aes128),
            HMAC_SHA1_DES3_KD => Ok(Self::HmacSha1Des3Kd),
            _ => Err(KerberosCryptoError::AlgorithmIdentifier(identifier)),
        }
    }
}

impl From<&ChecksumSuite> for u32 {
    fn from(checksum_suite: &ChecksumSuite) -> Self {
        match checksum_suite {
            ChecksumSuite::HmacSha196Aes256 => HMAC_SHA1_96_AES256 as u32,
            ChecksumSuite::HmacSha196Aes128 => HMAC_SHA1_96_AES128 as u32,
            ChecksumSuite::HmacSha1Des3Kd => HMAC_SHA1_DES3_KD as u32,
        }
    }
}

impl From<ChecksumSuite> for u32 {
    fn from(checksum_suite: ChecksumSuite) -> Self {
        match checksum_suite {
            ChecksumSuite::HmacSha196Aes256 => HMAC_SHA1_96_AES256 as u32,
            ChecksumSuite::HmacSha196Aes128 => HMAC_SHA1_96_AES128 as u32,
            ChecksumSuite::HmacSha1Des3Kd => HMAC_SHA1_DES3_KD as u32,
        }
    }
}

impl From<ChecksumSuite> for usize {
    fn from(checksum_suite: ChecksumSuite) -> Self {
        match checksum_suite {
            ChecksumSuite::HmacSha196Aes256 => HMAC_SHA1_96_AES256,
            ChecksumSuite::HmacSha196Aes128 => HMAC_SHA1_96_AES128,
            ChecksumSuite::HmacSha1Des3Kd => HMAC_SHA1_DES3_KD,
        }
    }
}

impl From<ChecksumSuite> for u8 {
    fn from(checksum_suite: ChecksumSuite) -> Self {
        match checksum_suite {
            ChecksumSuite::HmacSha196Aes256 => HMAC_SHA1_96_AES256 as u8,
            ChecksumSuite::HmacSha196Aes128 => HMAC_SHA1_96_AES128 as u8,
            ChecksumSuite::HmacSha1Des3Kd => HMAC_SHA1_DES3_KD as u8,
        }
    }
}

impl From<&ChecksumSuite> for u8 {
    fn from(checksum_suite: &ChecksumSuite) -> Self {
        match checksum_suite {
            ChecksumSuite::HmacSha196Aes256 => HMAC_SHA1_96_AES256 as u8,
            ChecksumSuite::HmacSha196Aes128 => HMAC_SHA1_96_AES128 as u8,
            ChecksumSuite::HmacSha1Des3Kd => HMAC_SHA1_DES3_KD as u8,
        }
    }
}
