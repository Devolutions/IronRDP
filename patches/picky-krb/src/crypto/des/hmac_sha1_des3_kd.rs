use crate::crypto::common::hmac_sha1;
use crate::crypto::utils::usage_ki;
use crate::crypto::{Checksum, ChecksumSuite, KerberosCryptoResult};

use super::{DES3_MAC_SIZE, derive_key};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HmacSha1Des3Kd;

impl Checksum for HmacSha1Des3Kd {
    fn checksum_type(&self) -> ChecksumSuite {
        ChecksumSuite::HmacSha1Des3Kd
    }

    fn checksum(&self, key: &[u8], key_usage: i32, payload: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
        let ki = derive_key(key, &usage_ki(key_usage))?;

        Ok(hmac_sha1(&ki, payload, DES3_MAC_SIZE))
    }
}
