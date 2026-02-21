use rand::rngs::StdRng;
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
use super::{AES_BLOCK_SIZE, AES_MAC_SIZE, AES128_KEY_SIZE, AesSize, derive_key, derive_key_from_password};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Aes128CtsHmacSha196;

impl Aes128CtsHmacSha196 {
    pub fn new() -> Self {
        Self
    }
}

impl Cipher for Aes128CtsHmacSha196 {
    fn key_size(&self) -> usize {
        AES128_KEY_SIZE
    }

    fn seed_bit_len(&self) -> usize {
        self.key_size() * 8
    }

    fn cipher_type(&self) -> CipherSuite {
        CipherSuite::Aes128CtsHmacSha196
    }

    fn checksum_type(&self) -> ChecksumSuite {
        ChecksumSuite::HmacSha196Aes128
    }

    fn encrypt(&self, key: &[u8], key_usage: i32, payload: &[u8]) -> Result<Vec<u8>, KerberosCryptoError> {
        let mut confounder = [0; AES_BLOCK_SIZE];
        StdRng::from_os_rng().fill_bytes(&mut confounder);

        encrypt_message(key, key_usage, payload, &AesSize::Aes128, confounder)
    }

    fn encrypt_no_checksum(
        &self,
        key: &[u8],
        key_usage: i32,
        payload: &[u8],
    ) -> KerberosCryptoResult<EncryptWithoutChecksum> {
        let mut confounder = [0; AES_BLOCK_SIZE];
        StdRng::from_os_rng().fill_bytes(&mut confounder);

        encrypt_message_no_checksum(key, key_usage, payload, &AesSize::Aes128, confounder)
    }

    fn decrypt(&self, key: &[u8], key_usage: i32, cipher_data: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
        decrypt_message(key, key_usage, cipher_data, &AesSize::Aes128)
    }

    fn decrypt_no_checksum(
        &self,
        key: &[u8],
        key_usage: i32,
        cipher_data: &[u8],
    ) -> KerberosCryptoResult<DecryptWithoutChecksum> {
        decrypt_message_no_checksum(key, key_usage, cipher_data, &AesSize::Aes128)
    }

    fn encryption_checksum(&self, key: &[u8], key_usage: i32, payload: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
        let ki = derive_key(key, &usage_ki(key_usage), &AesSize::Aes128)?;

        Ok(hmac_sha1(&ki, payload, AES_MAC_SIZE))
    }

    fn generate_key_from_password(&self, password: &[u8], salt: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
        derive_key_from_password(password, salt, &AesSize::Aes128)
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
        let key = [199, 196, 22, 102, 68, 93, 58, 102, 147, 19, 119, 57, 30, 138, 63, 230];

        encrypt_message(
            &key,
            5,
            plaintext,
            &AesSize::Aes128,
            [
                161, 52, 157, 33, 238, 232, 185, 93, 167, 130, 91, 180, 167, 165, 224, 78,
            ],
        )
        .unwrap()
    }

    fn encrypt_no_checksum(plaintext: &[u8]) -> EncryptWithoutChecksum {
        let key = [199, 196, 22, 102, 68, 93, 58, 102, 147, 19, 119, 57, 30, 138, 63, 230];

        encrypt_message_no_checksum(
            &key,
            5,
            plaintext,
            &AesSize::Aes128,
            [
                161, 52, 157, 33, 238, 232, 185, 93, 167, 130, 91, 180, 167, 165, 224, 78,
            ],
        )
        .unwrap()
    }

    fn decrypt(payload: &[u8]) -> Vec<u8> {
        let key = [199, 196, 22, 102, 68, 93, 58, 102, 147, 19, 119, 57, 30, 138, 63, 230];

        decrypt_message(&key, 5, payload, &AesSize::Aes128).unwrap()
    }

    fn decrypt_no_checksum(payload: &[u8]) -> DecryptWithoutChecksum {
        let key = [199, 196, 22, 102, 68, 93, 58, 102, 147, 19, 119, 57, 30, 138, 63, 230];

        decrypt_message_no_checksum(&key, 5, payload, &AesSize::Aes128).unwrap()
    }

    #[test]
    fn encrypt_half() {
        // incomplete block
        let plaintext = [97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95];

        assert_eq!(
            &[
                249, 151, 66, 206, 179, 219, 81, 201, 114, 214, 225, 206, 177, 160, 148, 231, 78, 233, 222, 4, 134,
                134, 236, 1, 140, 145, 53, 4, 248, 187, 38, 11, 80, 112, 117, 209, 115, 102, 145
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
                104, 52, 200, 252, 181, 222, 143, 82, 225, 234, 197, 103, 164, 244, 40, 198, 78, 233, 222, 4, 134, 134,
                236, 1, 140, 145, 53, 46, 29, 194, 87, 66, 120, 156, 142, 194, 74, 55, 7, 100, 179, 35, 90, 29
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
                78, 233, 222, 4, 134, 134, 236, 1, 140, 145, 53, 46, 29, 194, 87, 66, 72, 95, 20, 161, 59, 0, 167, 59,
                211, 170, 55, 181, 76, 198, 221, 143, 104, 52, 200, 252, 181, 222, 143, 178, 209, 198, 123, 25, 61,
                158, 185, 147, 98, 229, 18
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
                78, 233, 222, 4, 134, 134, 236, 1, 140, 145, 53, 46, 29, 194, 87, 66, 131, 19, 9, 19, 224, 219, 2, 36,
                61, 133, 217, 189, 236, 6, 107, 91, 52, 151, 188, 239, 255, 95, 48, 147, 254, 188, 16, 132, 202, 235,
                191, 244, 246, 170, 64, 133, 136, 188, 159, 169, 103, 8, 29, 69
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
                78, 233, 222, 4, 134, 134, 236, 1, 140, 145, 53, 46, 29, 194, 87, 66, 104, 52, 200, 252, 181, 222, 143,
                82, 225, 234, 197, 103, 164, 244, 40, 198, 146, 39, 38, 36, 114, 70, 240, 92, 121, 162, 182, 230, 208,
                74, 77, 193, 3, 95, 106, 255, 79, 172, 86, 208, 248, 74, 255, 77, 142, 73, 199, 138, 133, 239, 206
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
                78, 233, 222, 4, 134, 134, 236, 1, 140, 145, 53, 46, 29, 194, 87, 66, 104, 52, 200, 252, 181, 222, 143,
                82, 225, 234, 197, 103, 164, 244, 40, 198, 6, 187, 115, 114, 107, 118, 157, 175, 46, 192, 246, 169,
                229, 49, 110, 150, 233, 162, 172, 7, 161, 45, 150, 89, 88, 51, 29, 171, 216, 205, 143, 58, 133, 85, 45,
                174, 47, 252, 197, 189, 115, 50, 75, 27
            ],
            encrypt(&plaintext).as_slice()
        );
    }

    #[test]
    fn encrypt_three_and_half() {
        // three blocks + incomplete block
        let plaintext = [
            97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54, 46,
            107, 101, 121, 95, 100, 101, 114, 105, 118, 97, 116, 105, 111, 110, 46, 114, 115, 46, 99, 114, 121, 112,
            116, 111, 46, 114, 115, 46, 112, 105, 99, 107, 121, 45, 114, 115, 46,
        ];

        assert_eq!(
            &[
                78, 233, 222, 4, 134, 134, 236, 1, 140, 145, 53, 46, 29, 194, 87, 66, 104, 52, 200, 252, 181, 222, 143,
                82, 225, 234, 197, 103, 164, 244, 40, 198, 233, 162, 172, 7, 161, 45, 150, 89, 88, 51, 29, 171, 216,
                205, 143, 58, 158, 46, 64, 190, 77, 6, 14, 225, 161, 191, 90, 20, 3, 213, 205, 122, 6, 187, 115, 114,
                107, 118, 157, 175, 46, 192, 246, 169, 229, 125, 148, 112, 118, 204, 39, 6, 158, 39, 166, 159, 112
            ],
            encrypt(&plaintext).as_slice()
        );
    }

    #[test]
    fn decrypt_half() {
        // incomplete block
        let payload = [
            249, 151, 66, 206, 179, 219, 81, 201, 114, 214, 225, 206, 177, 160, 148, 231, 78, 233, 222, 4, 134, 134,
            236, 1, 140, 145, 53, 4, 248, 187, 38, 11, 80, 112, 117, 209, 115, 102, 145,
        ];

        assert_eq!(
            &[97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95],
            decrypt(&payload).as_slice()
        );
    }

    #[test]
    fn decrypt_one() {
        // one block
        let payload = [
            104, 52, 200, 252, 181, 222, 143, 82, 225, 234, 197, 103, 164, 244, 40, 198, 78, 233, 222, 4, 134, 134,
            236, 1, 140, 145, 53, 46, 29, 194, 87, 66, 120, 156, 142, 194, 74, 55, 7, 100, 179, 35, 90, 29,
        ];

        assert_eq!(
            &[97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95],
            decrypt(&payload).as_slice()
        );
    }

    #[test]
    fn decrypt_one_and_half() {
        // one block + incomplete block
        let plaintext = [
            78, 233, 222, 4, 134, 134, 236, 1, 140, 145, 53, 46, 29, 194, 87, 66, 72, 95, 20, 161, 59, 0, 167, 59, 211,
            170, 55, 181, 76, 198, 221, 143, 104, 52, 200, 252, 181, 222, 143, 178, 209, 198, 123, 25, 61, 158, 185,
            147, 98, 229, 18,
        ];

        assert_eq!(
            &[
                97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54,
            ],
            decrypt(&plaintext).as_slice()
        );
    }

    #[test]
    fn decrypt_two() {
        // two blocks
        let payload = [
            78, 233, 222, 4, 134, 134, 236, 1, 140, 145, 53, 46, 29, 194, 87, 66, 131, 19, 9, 19, 224, 219, 2, 36, 61,
            133, 217, 189, 236, 6, 107, 91, 52, 151, 188, 239, 255, 95, 48, 147, 254, 188, 16, 132, 202, 235, 191, 244,
            246, 170, 64, 133, 136, 188, 159, 169, 103, 8, 29, 69,
        ];

        assert_eq!(
            &[
                97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 5, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54, 107,
                101, 121, 95, 100, 101, 114, 105, 118,
            ],
            decrypt(&payload).as_slice()
        );
    }

    #[test]
    fn decrypt_two_and_half() {
        // two blocks + incomplete block
        let payload = [
            78, 233, 222, 4, 134, 134, 236, 1, 140, 145, 53, 46, 29, 194, 87, 66, 104, 52, 200, 252, 181, 222, 143, 82,
            225, 234, 197, 103, 164, 244, 40, 198, 146, 39, 38, 36, 114, 70, 240, 92, 121, 162, 182, 230, 208, 74, 77,
            193, 3, 95, 106, 255, 79, 172, 86, 208, 248, 74, 255, 77, 142, 73, 199, 138, 133, 239, 206,
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
            78, 233, 222, 4, 134, 134, 236, 1, 140, 145, 53, 46, 29, 194, 87, 66, 104, 52, 200, 252, 181, 222, 143, 82,
            225, 234, 197, 103, 164, 244, 40, 198, 6, 187, 115, 114, 107, 118, 157, 175, 46, 192, 246, 169, 229, 49,
            110, 150, 233, 162, 172, 7, 161, 45, 150, 89, 88, 51, 29, 171, 216, 205, 143, 58, 133, 85, 45, 174, 47,
            252, 197, 189, 115, 50, 75, 27,
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
            78, 233, 222, 4, 134, 134, 236, 1, 140, 145, 53, 46, 29, 194, 87, 66, 104, 52, 200, 252, 181, 222, 143, 82,
            225, 234, 197, 103, 164, 244, 40, 198, 233, 162, 172, 7, 161, 45, 150, 89, 88, 51, 29, 171, 216, 205, 143,
            58, 158, 46, 64, 190, 77, 6, 14, 225, 161, 191, 90, 20, 3, 213, 205, 122, 6, 187, 115, 114, 107, 118, 157,
            175, 46, 192, 246, 169, 229, 125, 148, 112, 118, 204, 39, 6, 158, 39, 166, 159, 112,
        ];

        assert_eq!(
            &[
                97, 101, 115, 50, 53, 54, 95, 99, 116, 115, 95, 104, 109, 97, 99, 95, 115, 104, 97, 49, 95, 57, 54, 46,
                107, 101, 121, 95, 100, 101, 114, 105, 118, 97, 116, 105, 111, 110, 46, 114, 115, 46, 99, 114, 121,
                112, 116, 111, 46, 114, 115, 46, 112, 105, 99, 107, 121, 45, 114, 115, 46,
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
            78, 233, 222, 4, 134, 134, 236, 1, 140, 145, 53, 46, 29, 194, 87, 66, 104, 52, 200, 252, 181, 222, 143, 82,
            225, 234, 197, 103, 164, 244, 40, 198, 6, 187, 115, 114, 107, 118, 157, 175, 46, 192, 246, 169, 229, 49,
            110, 150, 233, 162, 172, 7, 161, 45, 150, 89, 88, 51, 29, 171, 216, 205, 143, 58, 133, 85, 45, 174, 47,
            252, 197, 189, 115, 50, 75, 27,
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
