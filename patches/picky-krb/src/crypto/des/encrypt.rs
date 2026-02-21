use crate::crypto::common::hmac_sha1;
use crate::crypto::utils::{usage_ke, usage_ki};
use crate::crypto::{EncryptWithoutChecksum, KerberosCryptoError, KerberosCryptoResult};
use aes::cipher::{Array, KeyIvInit};
use des::TdesEde3;
use des::cipher::BlockModeEncrypt;
use des::cipher::block_padding::NoPadding;
use inout::InOutBufReserved;

use super::{DES3_BLOCK_SIZE, DES3_KEY_SIZE, DES3_MAC_SIZE, derive_key};

type DesCbcCipher = cbc::Encryptor<TdesEde3>;

//= [Cryptosystem Profile Based on Simplified Profile](https://datatracker.ietf.org/doc/html/rfc3961#section-5.3) =//
pub fn encrypt_message(
    key: &[u8],
    key_usage: i32,
    payload: &[u8],
    // conf = Random string of length c
    confounder: [u8; DES3_BLOCK_SIZE],
) -> KerberosCryptoResult<Vec<u8>> {
    let mut encryption_result = encrypt_message_no_checksum(key, key_usage, payload, confounder)?;
    // prepare for checksum generation
    let mut data_to_encrypt = vec![0; DES3_BLOCK_SIZE + payload.len()];

    let (confounder_buf, payload_buf) = data_to_encrypt.split_at_mut(DES3_BLOCK_SIZE);
    confounder_buf.copy_from_slice(&confounder);
    payload_buf.copy_from_slice(payload);

    let pad_len = (DES3_BLOCK_SIZE - (data_to_encrypt.len() % DES3_BLOCK_SIZE)) % DES3_BLOCK_SIZE;
    // pad
    data_to_encrypt.resize(data_to_encrypt.len() + pad_len, 0);

    let hmac = hmac_sha1(&encryption_result.ki, &data_to_encrypt, DES3_MAC_SIZE);

    // ciphertext =  C1 | H1[1..h]
    encryption_result.encrypted.extend_from_slice(&hmac);

    Ok(encryption_result.encrypted)
}

// Returns (C1, conf, Ki)
pub fn encrypt_message_no_checksum(
    key: &[u8],
    key_usage: i32,
    payload: &[u8],
    // conf = Random string of length c
    confounder: [u8; DES3_BLOCK_SIZE],
) -> KerberosCryptoResult<EncryptWithoutChecksum> {
    if key.len() != DES3_KEY_SIZE {
        return Err(KerberosCryptoError::KeyLength(key.len(), DES3_KEY_SIZE));
    }

    let mut data_to_encrypt = vec![0; DES3_BLOCK_SIZE + payload.len()];

    let (confounder_buf, payload_buf) = data_to_encrypt.split_at_mut(DES3_BLOCK_SIZE);
    confounder_buf.copy_from_slice(&confounder);
    payload_buf.copy_from_slice(payload);

    let pad_len = (DES3_BLOCK_SIZE - (data_to_encrypt.len() % DES3_BLOCK_SIZE)) % DES3_BLOCK_SIZE;
    // pad
    data_to_encrypt.resize(data_to_encrypt.len() + pad_len, 0);

    let ke = derive_key(key, &usage_ke(key_usage))?;
    // (C1, newIV) = E(Ke, conf | plaintext | pad, oldstate.ivec)
    let encrypted = encrypt_des(&ke, &data_to_encrypt)?;

    let ki = derive_key(key, &usage_ki(key_usage))?;

    Ok(EncryptWithoutChecksum {
        encrypted,
        confounder: confounder.to_vec(),
        ki,
    })
}

pub fn encrypt_des(key: &[u8], payload: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
    if key.len() != DES3_KEY_SIZE {
        return Err(KerberosCryptoError::KeyLength(key.len(), DES3_KEY_SIZE));
    }

    let pad_length = (DES3_BLOCK_SIZE - (payload.len() % DES3_BLOCK_SIZE)) % DES3_BLOCK_SIZE;

    let mut payload = payload.to_vec();
    payload.resize(payload.len() + pad_length, 0);

    let payload_len = payload.len();

    payload.extend_from_slice(&[0; DES3_BLOCK_SIZE]);

    // RFC 3961: initial cipher state. All bits zero
    let iv = [0_u8; DES3_BLOCK_SIZE];

    let key = Array::try_from(key)?;
    let ct = DesCbcCipher::new(&key, &iv.into());

    let inout = InOutBufReserved::from_mut_slice(&mut payload, payload_len)?;
    ct.encrypt_padded_inout::<NoPadding>(inout)?;

    payload.resize(payload_len, 0);

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::encrypt_des;

    #[test]
    fn test_encrypt_des() {
        let key = &[
            78, 101, 119, 84, 114, 105, 112, 108, 101, 68, 69, 83, 67, 105, 112, 104, 101, 114, 40, 107, 101, 121, 41,
            46,
        ];
        let payload = &[
            115, 114, 99, 47, 99, 114, 121, 112, 116, 111, 47, 100, 101, 115, 47, 100, 101, 115, 51, 95, 99, 98, 99,
            95, 115, 104, 97, 49, 95, 107, 100, 46, 114, 115,
        ];

        let cipher = encrypt_des(key, payload).unwrap();

        assert_eq!(
            &[
                87, 99, 22, 0, 235, 138, 12, 253, 230, 59, 41, 113, 167, 76, 242, 13, 165, 158, 210, 120, 86, 75, 221,
                202, 86, 77, 170, 9, 146, 89, 112, 88, 71, 246, 188, 99, 190, 8, 2, 57
            ],
            cipher.as_slice()
        );
    }

    #[test]
    fn test_encrypt_des_3() {
        let payload = &[254, 157, 144, 13, 111, 64, 173, 206];
        let key = &[
            100, 234, 37, 148, 191, 233, 42, 16, 104, 233, 26, 155, 127, 110, 98, 200, 104, 196, 248, 253, 35, 227, 26,
            167,
        ];

        let cipher = encrypt_des(key, payload).unwrap();

        assert_eq!(&[247, 92, 54, 146, 167, 87, 189, 111], cipher.as_slice());
    }

    #[test]
    fn test_encrypt_des_2() {
        let payload = &[115, 248, 21, 32, 230, 42, 157, 159];
        let key = &[
            100, 234, 37, 148, 191, 233, 42, 16, 104, 233, 26, 155, 127, 110, 98, 200, 104, 196, 248, 253, 35, 227, 26,
            167,
        ];

        let cipher = encrypt_des(key, payload).unwrap();

        assert_eq!(&[254, 157, 144, 13, 111, 64, 173, 206], cipher.as_slice());
    }
}
