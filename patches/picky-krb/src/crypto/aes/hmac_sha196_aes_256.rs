use crate::crypto::{Checksum, ChecksumSuite, KerberosCryptoResult};

use super::{AesSize, checksum_sha_aes};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HmacSha196Aes256;

impl Checksum for HmacSha196Aes256 {
    fn checksum_type(&self) -> ChecksumSuite {
        ChecksumSuite::HmacSha196Aes256
    }

    fn checksum(&self, key: &[u8], key_usage: i32, payload: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
        checksum_sha_aes(key, key_usage, payload, &AesSize::Aes256)
    }
}
