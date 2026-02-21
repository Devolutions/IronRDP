use rand::prelude::StdRng;
use rand::{RngCore, SeedableRng};

use crate::crypto::common::hmac_sha1;
use crate::crypto::utils::usage_ki;
use crate::crypto::{
    ChecksumSuite, Cipher, CipherSuite, DecryptWithoutChecksum, EncryptWithoutChecksum, KerberosCryptoError,
    KerberosCryptoResult,
};

use super::decrypt::{decrypt_message, decrypt_message_no_checksum};
use super::encrypt::{encrypt_message, encrypt_message_no_checksum};
use super::key_derivation::random_to_key;
use super::{AES_BLOCK_SIZE, AES_MAC_SIZE, AES256_KEY_SIZE, AesSize, derive_key, derive_key_from_password};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Aes256CtsHmacSha196;

impl Aes256CtsHmacSha196 {
    pub fn new() -> Self {
        Self
    }
}

impl Cipher for Aes256CtsHmacSha196 {
    fn key_size(&self) -> usize {
        AES256_KEY_SIZE
    }

    fn cipher_type(&self) -> CipherSuite {
        CipherSuite::Aes256CtsHmacSha196
    }

    fn checksum_type(&self) -> ChecksumSuite {
        ChecksumSuite::HmacSha196Aes256
    }

    fn encrypt(&self, key: &[u8], key_usage: i32, payload: &[u8]) -> Result<Vec<u8>, KerberosCryptoError> {
        let mut confounder = [0; AES_BLOCK_SIZE];
        StdRng::from_os_rng().fill_bytes(&mut confounder);
        encrypt_message(key, key_usage, payload, &AesSize::Aes256, confounder)
    }

    fn encrypt_no_checksum(
        &self,
        key: &[u8],
        key_usage: i32,
        payload: &[u8],
    ) -> KerberosCryptoResult<EncryptWithoutChecksum> {
        let mut confounder = [0; AES_BLOCK_SIZE];
        StdRng::from_os_rng().fill_bytes(&mut confounder);
        encrypt_message_no_checksum(key, key_usage, payload, &AesSize::Aes256, confounder)
    }

    fn decrypt(&self, key: &[u8], key_usage: i32, cipher_data: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
        decrypt_message(key, key_usage, cipher_data, &AesSize::Aes256)
    }

    fn decrypt_no_checksum(
        &self,
        key: &[u8],
        key_usage: i32,
        cipher_data: &[u8],
    ) -> KerberosCryptoResult<DecryptWithoutChecksum> {
        decrypt_message_no_checksum(key, key_usage, cipher_data, &AesSize::Aes256)
    }

    fn encryption_checksum(&self, key: &[u8], key_usage: i32, payload: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
        let ki = derive_key(key, &usage_ki(key_usage), &AesSize::Aes256)?;

        Ok(hmac_sha1(&ki, payload, AES_MAC_SIZE))
    }

    fn generate_key_from_password(&self, password: &[u8], salt: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
        derive_key_from_password(password, salt, &AesSize::Aes256)
    }

    fn seed_bit_len(&self) -> usize {
        self.key_size() * 8
    }

    fn random_to_key(&self, key: Vec<u8>) -> Vec<u8> {
        random_to_key(key)
    }
}

#[cfg(test)]
mod tests {
    use crate::crypto::aes::decrypt::{decrypt_message, decrypt_message_no_checksum};
    use crate::crypto::aes::encrypt::{encrypt_message, encrypt_message_no_checksum};
    use crate::crypto::aes::{AES_MAC_SIZE, AesSize};
    use crate::crypto::common::hmac_sha1;
    use crate::crypto::{DecryptWithoutChecksum, EncryptWithoutChecksum};

    fn encrypt(plaintext: &[u8]) -> Vec<u8> {
        let key = [
            22, 151, 234, 93, 29, 64, 176, 109, 232, 140, 95, 54, 168, 107, 20, 251, 155, 71, 70, 148, 50, 145, 49,
            157, 182, 139, 235, 19, 11, 199, 3, 135,
        ];

        encrypt_message(
            &key,
            5,
            plaintext,
            &AesSize::Aes256,
            [
                161, 52, 157, 33, 238, 232, 185, 93, 167, 130, 91, 180, 167, 165, 224, 78,
            ],
        )
        .unwrap()
    }

    fn encrypt_no_checksum(plaintext: &[u8]) -> EncryptWithoutChecksum {
        let key = [
            22, 151, 234, 93, 29, 64, 176, 109, 232, 140, 95, 54, 168, 107, 20, 251, 155, 71, 70, 148, 50, 145, 49,
            157, 182, 139, 235, 19, 11, 199, 3, 135,
        ];

        encrypt_message_no_checksum(
            &key,
            5,
            plaintext,
            &AesSize::Aes256,
            [
                161, 52, 157, 33, 238, 232, 185, 93, 167, 130, 91, 180, 167, 165, 224, 78,
            ],
        )
        .unwrap()
    }

    fn decrypt(payload: &[u8]) -> Vec<u8> {
        let key = [
            22, 151, 234, 93, 29, 64, 176, 109, 232, 140, 95, 54, 168, 107, 20, 251, 155, 71, 70, 148, 50, 145, 49,
            157, 182, 139, 235, 19, 11, 199, 3, 135,
        ];

        decrypt_message(&key, 5, payload, &AesSize::Aes256).unwrap()
    }

    fn decrypt_no_checksum(payload: &[u8]) -> DecryptWithoutChecksum {
        let key = [
            22, 151, 234, 93, 29, 64, 176, 109, 232, 140, 95, 54, 168, 107, 20, 251, 155, 71, 70, 148, 50, 145, 49,
            157, 182, 139, 235, 19, 11, 199, 3, 135,
        ];

        decrypt_message_no_checksum(&key, 5, payload, &AesSize::Aes256).unwrap()
    }

    #[test]
    fn encrypt_half() {
        // incomplete block
        let plaintext = [97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95];

        assert_eq!(
            &[
                153, 67, 25, 51, 230, 39, 92, 105, 17, 234, 98, 208, 165, 181, 181, 225, 214, 122, 109, 174, 37, 138,
                242, 223, 137, 137, 242, 184, 235, 239, 155, 12, 185, 70, 139, 212, 37, 35, 90
            ],
            encrypt(&plaintext).as_slice()
        );
    }

    #[test]
    fn encrypt_one() {
        // one block
        let plaintext = [97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95];

        assert_eq!(
            &[
                10, 164, 28, 60, 222, 116, 184, 67, 131, 207, 244, 3, 10, 249, 22, 244, 214, 122, 109, 174, 37, 138,
                242, 223, 137, 137, 242, 93, 162, 124, 121, 114, 0, 1, 133, 19, 130, 154, 121, 77, 48, 11, 189, 137
            ],
            encrypt(&plaintext).as_slice()
        );
    }

    #[test]
    fn encrypt_one_and_half() {
        // one block + incomplete block
        let plaintext = [
            97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54,
        ];

        assert_eq!(
            &[
                214, 122, 109, 174, 37, 138, 242, 223, 137, 137, 242, 93, 162, 124, 121, 114, 161, 144, 68, 138, 219,
                96, 18, 26, 10, 139, 245, 156, 28, 218, 173, 28, 10, 164, 28, 60, 222, 116, 184, 96, 153, 3, 46, 220,
                113, 173, 31, 154, 73, 236, 25
            ],
            encrypt(&plaintext).as_slice()
        );
    }

    #[test]
    fn encrypt_two() {
        // two blocks
        let plaintext = [
            97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 5, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54, 107,
            101, 121, 95, 100, 101, 114, 105, 118,
        ];

        assert_eq!(
            &[
                214, 122, 109, 174, 37, 138, 242, 223, 137, 137, 242, 93, 162, 124, 121, 114, 214, 57, 118, 48, 238,
                82, 92, 83, 182, 254, 200, 38, 71, 6, 142, 72, 115, 214, 107, 193, 38, 10, 184, 156, 34, 121, 228, 100,
                13, 228, 159, 52, 191, 126, 65, 159, 253, 157, 62, 9, 125, 106, 82, 136
            ],
            encrypt(&plaintext).as_slice()
        );
    }

    #[test]
    fn encrypt_two_and_half() {
        // two blocks + incomplete block
        let plaintext = [
            97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54, 107,
            101, 121, 95, 100, 101, 114, 105, 118, 97, 116, 105, 111, 110, 46, 114,
        ];

        assert_eq!(
            &[
                214, 122, 109, 174, 37, 138, 242, 223, 137, 137, 242, 93, 162, 124, 121, 114, 10, 164, 28, 60, 222,
                116, 184, 67, 131, 207, 244, 3, 10, 249, 22, 244, 64, 87, 14, 62, 62, 12, 77, 137, 200, 194, 20, 216,
                149, 179, 128, 92, 156, 39, 25, 101, 126, 251, 45, 121, 20, 103, 36, 246, 54, 67, 200, 167, 244, 214,
                209,
            ],
            encrypt(&plaintext).as_slice()
        );
    }

    #[test]
    fn encrypt_three() {
        // three blocks
        let plaintext = [
            97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54, 46,
            107, 101, 121, 95, 100, 101, 114, 105, 118, 97, 116, 105, 111, 110, 46, 114, 115, 46, 99, 114, 121, 112,
            116, 111,
        ];

        assert_eq!(
            &[
                214, 122, 109, 174, 37, 138, 242, 223, 137, 137, 242, 93, 162, 124, 121, 114, 10, 164, 28, 60, 222,
                116, 184, 67, 131, 207, 244, 3, 10, 249, 22, 244, 35, 238, 183, 171, 208, 35, 185, 212, 190, 49, 9, 49,
                122, 105, 47, 155, 81, 226, 246, 250, 147, 120, 239, 83, 65, 157, 252, 73, 142, 130, 107, 70, 233, 12,
                140, 124, 156, 243, 171, 176, 162, 128, 119, 189
            ],
            encrypt(&plaintext).as_slice()
        );
    }

    #[test]
    fn three_and_half() {
        // three blocks + incomplete block
        let plaintext = [
            97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54, 46,
            107, 101, 121, 95, 100, 101, 114, 105, 118, 97, 116, 105, 111, 110, 46, 114, 115, 46, 99, 114, 121, 112,
            116, 111, 46, 114, 115, 46, 112, 105, 99, 107, 121, 45, 114, 115, 46,
        ];

        assert_eq!(
            &[
                214, 122, 109, 174, 37, 138, 242, 223, 137, 137, 242, 93, 162, 124, 121, 114, 10, 164, 28, 60, 222,
                116, 184, 67, 131, 207, 244, 3, 10, 249, 22, 244, 81, 226, 246, 250, 147, 120, 239, 83, 65, 157, 252,
                73, 142, 130, 107, 70, 54, 89, 220, 119, 43, 138, 67, 4, 82, 98, 225, 84, 221, 24, 143, 47, 35, 238,
                183, 171, 208, 35, 185, 212, 190, 49, 9, 49, 122, 221, 131, 75, 188, 8, 114, 203, 108, 140, 156, 131,
                175
            ],
            encrypt(&plaintext).as_slice()
        );
    }

    #[test]
    fn decrypt_half() {
        // incomplete block
        let payload = [
            153, 67, 25, 51, 230, 39, 92, 105, 17, 234, 98, 208, 165, 181, 181, 225, 214, 122, 109, 174, 37, 138, 242,
            223, 137, 137, 242, 184, 235, 239, 155, 12, 185, 70, 139, 212, 37, 35, 90,
        ];

        assert_eq!(
            &[97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95,],
            decrypt(&payload).as_slice()
        );
    }

    #[test]
    fn decrypt_one() {
        // one block
        let payload = [
            10, 164, 28, 60, 222, 116, 184, 67, 131, 207, 244, 3, 10, 249, 22, 244, 214, 122, 109, 174, 37, 138, 242,
            223, 137, 137, 242, 93, 162, 124, 121, 114, 0, 1, 133, 19, 130, 154, 121, 77, 48, 11, 189, 137,
        ];

        assert_eq!(
            &[97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95,],
            decrypt(&payload).as_slice()
        );
    }

    #[test]
    fn decrypt_one_and_half() {
        // one block + incomplete block
        let payload = [
            214, 122, 109, 174, 37, 138, 242, 223, 137, 137, 242, 93, 162, 124, 121, 114, 161, 144, 68, 138, 219, 96,
            18, 26, 10, 139, 245, 156, 28, 218, 173, 28, 10, 164, 28, 60, 222, 116, 184, 96, 153, 3, 46, 220, 113, 173,
            31, 154, 73, 236, 25,
        ];

        assert_eq!(
            &[
                97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54,
            ],
            decrypt(&payload).as_slice()
        );
    }

    #[test]
    fn decrypt_two() {
        // two blocks
        let payload = [
            214, 122, 109, 174, 37, 138, 242, 223, 137, 137, 242, 93, 162, 124, 121, 114, 214, 57, 118, 48, 238, 82,
            92, 83, 182, 254, 200, 38, 71, 6, 142, 72, 115, 214, 107, 193, 38, 10, 184, 156, 34, 121, 228, 100, 13,
            228, 159, 52, 191, 126, 65, 159, 253, 157, 62, 9, 125, 106, 82, 136,
        ];

        assert_eq!(
            &[
                97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 5, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54, 107,
                101, 121, 95, 100, 101, 114, 105, 118
            ],
            decrypt(&payload).as_slice()
        );
    }

    #[test]
    fn decrypt_two_and_half() {
        // two blocks + incomplete block
        let payload = [
            214, 122, 109, 174, 37, 138, 242, 223, 137, 137, 242, 93, 162, 124, 121, 114, 10, 164, 28, 60, 222, 116,
            184, 67, 131, 207, 244, 3, 10, 249, 22, 244, 64, 87, 14, 62, 62, 12, 77, 137, 200, 194, 20, 216, 149, 179,
            128, 92, 156, 39, 25, 101, 126, 251, 45, 121, 20, 103, 36, 246, 54, 67, 200, 167, 244, 214, 209,
        ];

        assert_eq!(
            &[
                97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54,
                107, 101, 121, 95, 100, 101, 114, 105, 118, 97, 116, 105, 111, 110, 46, 114,
            ],
            decrypt(&payload).as_slice()
        );
    }

    #[test]
    fn decrypt_three() {
        // three blocks
        let payload = [
            214, 122, 109, 174, 37, 138, 242, 223, 137, 137, 242, 93, 162, 124, 121, 114, 10, 164, 28, 60, 222, 116,
            184, 67, 131, 207, 244, 3, 10, 249, 22, 244, 35, 238, 183, 171, 208, 35, 185, 212, 190, 49, 9, 49, 122,
            105, 47, 155, 81, 226, 246, 250, 147, 120, 239, 83, 65, 157, 252, 73, 142, 130, 107, 70, 233, 12, 140, 124,
            156, 243, 171, 176, 162, 128, 119, 189,
        ];

        assert_eq!(
            &[
                97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54, 46,
                107, 101, 121, 95, 100, 101, 114, 105, 118, 97, 116, 105, 111, 110, 46, 114, 115, 46, 99, 114, 121,
                112, 116, 111,
            ],
            decrypt(&payload).as_slice()
        );
    }

    #[test]
    fn decrypt_three_and_half() {
        // three blocks + incomplete block
        let payload = [
            214, 122, 109, 174, 37, 138, 242, 223, 137, 137, 242, 93, 162, 124, 121, 114, 10, 164, 28, 60, 222, 116,
            184, 67, 131, 207, 244, 3, 10, 249, 22, 244, 81, 226, 246, 250, 147, 120, 239, 83, 65, 157, 252, 73, 142,
            130, 107, 70, 54, 89, 220, 119, 43, 138, 67, 4, 82, 98, 225, 84, 221, 24, 143, 47, 35, 238, 183, 171, 208,
            35, 185, 212, 190, 49, 9, 49, 122, 221, 131, 75, 188, 8, 114, 203, 108, 140, 156, 131, 175,
        ];

        assert_eq!(
            &[
                97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54, 46,
                107, 101, 121, 95, 100, 101, 114, 105, 118, 97, 116, 105, 111, 110, 46, 114, 115, 46, 99, 114, 121,
                112, 116, 111, 46, 114, 115, 46, 112, 105, 99, 107, 121, 45, 114, 115, 46
            ],
            decrypt(&payload).as_slice()
        );
    }

    #[test]
    fn encrypt_decrypt_no_checksum() {
        // three blocks
        let plaintext = [
            97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54, 46,
            107, 101, 121, 95, 100, 101, 114, 105, 118, 97, 116, 105, 111, 110, 46, 114, 115, 46, 99, 114, 121, 112,
            116, 111,
        ];

        let expected_encrypted = &[
            214, 122, 109, 174, 37, 138, 242, 223, 137, 137, 242, 93, 162, 124, 121, 114, 10, 164, 28, 60, 222, 116,
            184, 67, 131, 207, 244, 3, 10, 249, 22, 244, 35, 238, 183, 171, 208, 35, 185, 212, 190, 49, 9, 49, 122,
            105, 47, 155, 81, 226, 246, 250, 147, 120, 239, 83, 65, 157, 252, 73, 142, 130, 107, 70, 233, 12, 140, 124,
            156, 243, 171, 176, 162, 128, 119, 189,
        ];

        assert_eq!(expected_encrypted, encrypt(&plaintext).as_slice());

        let expected_encrypted_no_checksum = &expected_encrypted[0..expected_encrypted.len() - AES_MAC_SIZE];

        let encryption_result = encrypt_no_checksum(&plaintext);
        assert_eq!(expected_encrypted_no_checksum, encryption_result.encrypted);

        // prepare for checksum calculation
        let mut conf_and_plaintext = encryption_result.confounder.clone();
        conf_and_plaintext.extend_from_slice(&plaintext);

        // verify that the same checksum is generated
        assert_eq!(
            hmac_sha1(&encryption_result.ki, &conf_and_plaintext, AES_MAC_SIZE),
            expected_encrypted[expected_encrypted.len() - AES_MAC_SIZE..]
        );

        // verify that concatenating encrypted data and checksum gives expected result
        let mut encrypted_with_checksum = encryption_result.encrypted;
        encrypted_with_checksum.extend(&hmac_sha1(&encryption_result.ki, &conf_and_plaintext, AES_MAC_SIZE));
        assert_eq!(encrypted_with_checksum, expected_encrypted);

        // verify that decrypt functions produce the same result
        let decryption_result = decrypt_no_checksum(expected_encrypted);
        assert_eq!(decrypt(expected_encrypted), decryption_result.plaintext);

        assert_eq!(decryption_result.confounder, encryption_result.confounder);

        assert_eq!(
            decryption_result.checksum,
            expected_encrypted[expected_encrypted.len() - AES_MAC_SIZE..]
        );

        // generate checksum and validate it against the actual
        let mut decrypted_confounder_with_plaintext = decryption_result.confounder.clone();
        decrypted_confounder_with_plaintext.extend(decryption_result.plaintext);
        assert_eq!(
            hmac_sha1(
                &decryption_result.ki,
                &decrypted_confounder_with_plaintext,
                AES_MAC_SIZE
            ),
            decryption_result.checksum
        );
    }
}
