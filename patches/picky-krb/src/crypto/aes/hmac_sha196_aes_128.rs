use crate::crypto::{Checksum, ChecksumSuite, KerberosCryptoResult};

use super::{AesSize, checksum_sha_aes};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HmacSha196Aes128;

impl Checksum for HmacSha196Aes128 {
    fn checksum_type(&self) -> ChecksumSuite {
        ChecksumSuite::HmacSha196Aes128
    }

    fn checksum(&self, key: &[u8], key_usage: i32, payload: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
        checksum_sha_aes(key, key_usage, payload, &AesSize::Aes128)
    }
}
