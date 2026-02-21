use rand::rngs::StdRng;
use rand::{RngCore, SeedableRng};

use crate::crypto::common::hmac_sha1;
use crate::crypto::utils::usage_ki;
use crate::crypto::{
    ChecksumSuite, Cipher, CipherSuite, DecryptWithoutChecksum, EncryptWithoutChecksum, KerberosCryptoResult,
};

use super::decrypt::{decrypt_message, decrypt_message_no_checksum};
use super::encrypt::{encrypt_message, encrypt_message_no_checksum};
use super::key_derivation::random_to_key;
use super::{DES3_BLOCK_SIZE, DES3_KEY_SIZE, DES3_MAC_SIZE, DES3_SEED_LEN, derive_key, derive_key_from_password};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Des3CbcSha1Kd;

impl Des3CbcSha1Kd {
    pub fn new() -> Self {
        Self
    }
}

impl Cipher for Des3CbcSha1Kd {
    fn key_size(&self) -> usize {
        DES3_KEY_SIZE
    }

    fn seed_bit_len(&self) -> usize {
        DES3_SEED_LEN * 8
    }

    fn random_to_key(&self, key: Vec<u8>) -> Vec<u8> {
        random_to_key(&key)
    }

    fn cipher_type(&self) -> CipherSuite {
        CipherSuite::Des3CbcSha1Kd
    }

    fn checksum_type(&self) -> ChecksumSuite {
        ChecksumSuite::HmacSha1Des3Kd
    }

    fn encrypt(&self, key: &[u8], key_usage: i32, payload: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
        let mut cofounder = [0; DES3_BLOCK_SIZE];
        StdRng::from_os_rng().fill_bytes(&mut cofounder);
        encrypt_message(key, key_usage, payload, cofounder)
    }

    fn encrypt_no_checksum(
        &self,
        key: &[u8],
        key_usage: i32,
        payload: &[u8],
    ) -> KerberosCryptoResult<EncryptWithoutChecksum> {
        let mut cofounder = [0; DES3_BLOCK_SIZE];
        StdRng::from_os_rng().fill_bytes(&mut cofounder);
        encrypt_message_no_checksum(key, key_usage, payload, cofounder)
    }

    fn decrypt(&self, key: &[u8], key_usage: i32, cipher_data: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
        decrypt_message(key, key_usage, cipher_data)
    }

    fn decrypt_no_checksum(
        &self,
        key: &[u8],
        key_usage: i32,
        cipher_data: &[u8],
    ) -> KerberosCryptoResult<DecryptWithoutChecksum> {
        decrypt_message_no_checksum(key, key_usage, cipher_data)
    }

    fn encryption_checksum(&self, key: &[u8], key_usage: i32, payload: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
        let ki = derive_key(key, &usage_ki(key_usage))?;

        Ok(hmac_sha1(&ki, payload, DES3_MAC_SIZE))
    }

    fn generate_key_from_password(&self, password: &[u8], salt: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
        derive_key_from_password(password, salt)
    }
}

#[cfg(test)]
mod tests {
    use crate::crypto::common::hmac_sha1;
    use crate::crypto::des::decrypt::{decrypt_message, decrypt_message_no_checksum};
    use crate::crypto::des::encrypt::{encrypt_message, encrypt_message_no_checksum};
    use crate::crypto::des::{DES3_BLOCK_SIZE, DES3_MAC_SIZE};

    #[test]
    fn encrypt() {
        let key = [
            115, 248, 21, 32, 230, 42, 157, 138, 158, 254, 157, 145, 13, 110, 64, 107, 173, 206, 247, 93, 55, 146, 167,
            138,
        ];
        let plaintext = [
            97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54,
        ];
        let confounder = [161, 52, 157, 33, 238, 232, 185, 93];

        let cipher_data = encrypt_message(&key, 5, &plaintext, confounder).unwrap();

        assert_eq!(
            &[
                126, 136, 43, 80, 62, 251, 57, 122, 225, 31, 122, 177, 228, 203, 192, 209, 209, 50, 207, 26, 25, 42,
                111, 102, 243, 28, 130, 32, 30, 129, 155, 136, 93, 10, 246, 56, 89, 215, 120, 254, 207, 136, 121, 74,
                156, 20, 56, 227, 234, 98, 203, 221
            ],
            cipher_data.as_slice()
        );
    }

    #[test]
    fn decrypt() {
        let key = [
            115, 248, 21, 32, 230, 42, 157, 138, 158, 254, 157, 145, 13, 110, 64, 107, 173, 206, 247, 93, 55, 146, 167,
            138,
        ];
        let payload = [
            126, 136, 43, 80, 62, 251, 57, 122, 225, 31, 122, 177, 228, 203, 192, 209, 209, 50, 207, 26, 25, 42, 111,
            102, 243, 28, 130, 32, 30, 129, 155, 136, 93, 10, 246, 56, 89, 215, 120, 254, 207, 136, 121, 74, 156, 20,
            56, 227, 234, 98, 203, 221,
        ];

        let plaintext = decrypt_message(&key, 5, &payload).unwrap();

        assert_eq!(
            &[
                97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54, 0
            ],
            plaintext.as_slice()
        );
    }

    #[test]
    fn encrypt_no_checksum() {
        let key = [
            115, 248, 21, 32, 230, 42, 157, 138, 158, 254, 157, 145, 13, 110, 64, 107, 173, 206, 247, 93, 55, 146, 167,
            138,
        ];
        let plaintext = [
            97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54,
        ];
        let confounder = [161, 52, 157, 33, 238, 232, 185, 93];

        let encryption_result = encrypt_message_no_checksum(&key, 5, &plaintext, confounder).unwrap();

        assert_eq!(confounder, encryption_result.confounder.as_slice());

        let expected_encrypted_data_with_checksum = &[
            126, 136, 43, 80, 62, 251, 57, 122, 225, 31, 122, 177, 228, 203, 192, 209, 209, 50, 207, 26, 25, 42, 111,
            102, 243, 28, 130, 32, 30, 129, 155, 136, 93, 10, 246, 56, 89, 215, 120, 254, 207, 136, 121, 74, 156, 20,
            56, 227, 234, 98, 203, 221,
        ];

        let mut conf_with_plaintext: Vec<u8> = encryption_result.confounder;
        conf_with_plaintext.extend_from_slice(&plaintext);
        let pad_len = (DES3_BLOCK_SIZE - (conf_with_plaintext.len() % DES3_BLOCK_SIZE)) % DES3_BLOCK_SIZE;
        conf_with_plaintext.resize(conf_with_plaintext.len() + pad_len, 0);

        assert_eq!(
            hmac_sha1(&encryption_result.ki, &conf_with_plaintext, DES3_MAC_SIZE),
            expected_encrypted_data_with_checksum[expected_encrypted_data_with_checksum.len() - DES3_MAC_SIZE..]
        );

        let mut cipher_data_with_checksum = encryption_result.encrypted;
        cipher_data_with_checksum.extend(hmac_sha1(&encryption_result.ki, &conf_with_plaintext, DES3_MAC_SIZE));

        assert_eq!(cipher_data_with_checksum, expected_encrypted_data_with_checksum);

        assert_eq!(
            cipher_data_with_checksum,
            encrypt_message(&key, 5, &plaintext, confounder).unwrap()
        );
    }

    #[test]
    fn decrypt_with_checksum() {
        let key = [
            115, 248, 21, 32, 230, 42, 157, 138, 158, 254, 157, 145, 13, 110, 64, 107, 173, 206, 247, 93, 55, 146, 167,
            138,
        ];

        let confounder = [161, 52, 157, 33, 238, 232, 185, 93];

        let encrypted_with_checksum = &[
            126, 136, 43, 80, 62, 251, 57, 122, 225, 31, 122, 177, 228, 203, 192, 209, 209, 50, 207, 26, 25, 42, 111,
            102, 243, 28, 130, 32, 30, 129, 155, 136, 93, 10, 246, 56, 89, 215, 120, 254, 207, 136, 121, 74, 156, 20,
            56, 227, 234, 98, 203, 221,
        ];

        let decryption_result = decrypt_message_no_checksum(&key, 5, encrypted_with_checksum).unwrap();

        assert_eq!(
            decrypt_message(&key, 5, encrypted_with_checksum).unwrap(),
            decryption_result.plaintext
        );

        assert_eq!(confounder, decryption_result.confounder.as_slice());

        let mut conf_with_padded_plaintext = decryption_result.confounder;
        conf_with_padded_plaintext.extend(decryption_result.plaintext);

        assert_eq!(
            hmac_sha1(&decryption_result.ki, &conf_with_padded_plaintext, DES3_MAC_SIZE),
            decryption_result.checksum
        );
    }
}
