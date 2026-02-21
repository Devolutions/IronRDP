pub(crate) mod decrypt;
pub(crate) mod des3_cbc_sha1_kd;
pub(crate) mod encrypt;
pub(crate) mod hmac_sha1_des3_kd;
mod key_derivation;

/// [Triple-DES Based Encryption](https://datatracker.ietf.org/doc/html/rfc3961#section-6.3)
/// message block size = 8 bytes
pub const DES3_BLOCK_SIZE: usize = 8;
/// [Triple-DES Based Encryption](https://datatracker.ietf.org/doc/html/rfc3961#section-6.3)
/// protocol key format = 24 bytes
pub const DES3_KEY_SIZE: usize = 24;
/// [Triple-DES Based Encryption](https://datatracker.ietf.org/doc/html/rfc3961#section-6.3)
/// HMAC output size = 160 bits
pub const DES3_MAC_SIZE: usize = 20;
/// [Triple-DES Based Encryption](https://datatracker.ietf.org/doc/html/rfc3961#section-6.3)
/// key-generation seed length = 21 bytes
pub const DES3_SEED_LEN: usize = 21;

pub use des3_cbc_sha1_kd::Des3CbcSha1Kd;
pub use hmac_sha1_des3_kd::HmacSha1Des3Kd;
pub use key_derivation::{derive_key, derive_key_from_password};
