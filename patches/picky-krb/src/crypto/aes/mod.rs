pub(crate) mod aes128_cts_hmac_sha1_96;
pub(crate) mod aes256_cts_hmac_sha1_96;
pub(crate) mod decrypt;
pub(crate) mod encrypt;
pub(crate) mod hmac_sha196_aes_128;
pub(crate) mod hmac_sha196_aes_256;
mod key_derivation;

use super::common::hmac_sha1;
use super::utils::usage_kc;
use super::{KerberosCryptoError, KerberosCryptoResult};

/// [Kerberos Algorithm Profile Parameters](https://www.rfc-editor.org/rfc/rfc3962.html#section-6)
/// cipher block size 16 octets
pub const AES_BLOCK_SIZE: usize = 16;
/// [Kerberos Algorithm Profile Parameters](https://www.rfc-editor.org/rfc/rfc3962.html#section-6)
/// HMAC output size = 12 octets
pub const AES_MAC_SIZE: usize = 12;

/// [Assigned Numbers](https://www.rfc-editor.org/rfc/rfc3962.html#section-7)
pub const AES128_KEY_SIZE: usize = 128 / 8;
pub const AES256_KEY_SIZE: usize = 256 / 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AesSize {
    Aes256,
    Aes128,
}

impl AesSize {
    pub fn key_length(&self) -> usize {
        match self {
            AesSize::Aes256 => AES256_KEY_SIZE,
            AesSize::Aes128 => AES128_KEY_SIZE,
        }
    }

    pub fn block_bit_len(&self) -> usize {
        match self {
            AesSize::Aes256 => AES_BLOCK_SIZE * 8,
            AesSize::Aes128 => AES_BLOCK_SIZE * 8,
        }
    }

    pub fn seed_bit_len(&self) -> usize {
        self.key_length() * 8
    }
}

pub fn swap_two_last_blocks(data: &mut [u8]) -> KerberosCryptoResult<()> {
    if data.len() < AES_BLOCK_SIZE * 2 {
        return Err(KerberosCryptoError::CipherLength(data.len(), AES_BLOCK_SIZE * 2));
    }

    let len = data.len();

    for i in 0..AES_BLOCK_SIZE {
        data.swap(i + len - 2 * AES_BLOCK_SIZE, i + len - AES_BLOCK_SIZE)
    }

    Ok(())
}

pub fn checksum_sha_aes(
    key: &[u8],
    key_usage: i32,
    payload: &[u8],
    aes_size: &AesSize,
) -> KerberosCryptoResult<Vec<u8>> {
    let kc = derive_key(key, &usage_kc(key_usage), aes_size)?;

    Ok(hmac_sha1(&kc, payload, AES_MAC_SIZE))
}

pub use aes128_cts_hmac_sha1_96::Aes128CtsHmacSha196;
pub use aes256_cts_hmac_sha1_96::Aes256CtsHmacSha196;
pub use hmac_sha196_aes_128::HmacSha196Aes128;
pub use hmac_sha196_aes_256::HmacSha196Aes256;
pub use key_derivation::{derive_key, derive_key_from_password};
