use crate::crypto::aes::key_derivation::derive_key;
use crate::crypto::common::hmac_sha1;
use crate::crypto::utils::{usage_ke, usage_ki};
use crate::crypto::{EncryptWithoutChecksum, KerberosCryptoError, KerberosCryptoResult};
use aes::cipher::{Array, KeyIvInit};
use aes::{Aes128, Aes256};
use cbc::Encryptor;
use cbc::cipher::BlockModeEncrypt;
use cbc::cipher::block_padding::NoPadding;
use inout::InOutBufReserved;

use super::{AES_BLOCK_SIZE, AES_MAC_SIZE, AesSize, swap_two_last_blocks};

pub type Aes256CbcEncryptor = Encryptor<Aes256>;
pub type Aes128CbcEncryptor = Encryptor<Aes128>;

//= [Cryptosystem Profile Based on Simplified Profile](https://datatracker.ietf.org/doc/html/rfc3961#section-5.3) =//
pub fn encrypt_message(
    key: &[u8],
    key_usage: i32,
    payload: &[u8],
    aes_size: &AesSize,
    // conf = Random string of length c
    confounder: [u8; AES_BLOCK_SIZE],
) -> KerberosCryptoResult<Vec<u8>> {
    let mut encryption_result = encrypt_message_no_checksum(key, key_usage, payload, aes_size, confounder)?;
    // prepare for checksum generation
    let mut data_to_encrypt = vec![0; AES_BLOCK_SIZE + payload.len()];

    let (confounder_buf, payload_buf) = data_to_encrypt.split_at_mut(AES_BLOCK_SIZE);
    confounder_buf.copy_from_slice(&confounder);
    payload_buf.copy_from_slice(payload);

    // H1 = HMAC(Ki, conf | plaintext | pad)
    let hmac = hmac_sha1(&encryption_result.ki, &data_to_encrypt, AES_MAC_SIZE);

    // ciphertext =  C1 | H1[1..h]
    encryption_result.encrypted.extend_from_slice(&hmac);

    Ok(encryption_result.encrypted)
}

/// Returns (C1, conf, Ki)
pub fn encrypt_message_no_checksum(
    key: &[u8],
    key_usage: i32,
    payload: &[u8],
    aes_size: &AesSize,
    // conf = Random string of length c
    confounder: [u8; AES_BLOCK_SIZE],
) -> KerberosCryptoResult<EncryptWithoutChecksum> {
    if key.len() != aes_size.key_length() {
        return Err(KerberosCryptoError::KeyLength(key.len(), aes_size.key_length()));
    }

    let mut data_to_encrypt = vec![0; AES_BLOCK_SIZE + payload.len()];

    let (confounder_buf, payload_buf) = data_to_encrypt.split_at_mut(AES_BLOCK_SIZE);
    confounder_buf.copy_from_slice(&confounder);
    payload_buf.copy_from_slice(payload);

    let ke = derive_key(key, &usage_ke(key_usage), aes_size)?;
    // (C1, newIV) = E(Ke, conf | plaintext | pad, oldstate.ivec)
    let encrypted = encrypt_aes_cts(&ke, &data_to_encrypt, aes_size)?;

    let ki = derive_key(key, &usage_ki(key_usage), aes_size)?;

    Ok(EncryptWithoutChecksum {
        encrypted,
        confounder: confounder.to_vec(),
        ki,
    })
}

pub fn encrypt_aes_cbc(key: &[u8], plaintext: &[u8], aes_size: &AesSize) -> KerberosCryptoResult<Vec<u8>> {
    let iv = [0; AES_BLOCK_SIZE];

    let mut payload = plaintext.to_vec();
    let payload_len = payload.len();

    match aes_size {
        AesSize::Aes256 => {
            let key = Array::try_from(key)?;
            let cipher = Aes256CbcEncryptor::new(&key, &iv.into());
            let inout = InOutBufReserved::from_mut_slice(&mut payload, payload_len)?;
            cipher.encrypt_padded_inout::<NoPadding>(inout)?;
        }
        AesSize::Aes128 => {
            let key = Array::try_from(key)?;
            let cipher = Aes128CbcEncryptor::new(&key, &iv.into());
            let inout = InOutBufReserved::from_mut_slice(&mut payload, payload_len)?;
            cipher.encrypt_padded_inout::<NoPadding>(inout)?;
        }
    }

    Ok(payload)
}

//= [CTS using CBC](https://en.wikipedia.org/wiki/Ciphertext_stealing#CBC_ciphertext_stealing_encryption_using_a_standard_CBC_interface) =//
pub fn encrypt_aes_cts(key: &[u8], payload: &[u8], aes_size: &AesSize) -> KerberosCryptoResult<Vec<u8>> {
    let pad_length = (AES_BLOCK_SIZE - (payload.len() % AES_BLOCK_SIZE)) % AES_BLOCK_SIZE;

    let mut padded_payload = payload.to_vec();
    padded_payload.resize(padded_payload.len() + pad_length, 0);

    let mut cipher = encrypt_aes_cbc(key, &padded_payload, aes_size)?;

    if cipher.len() <= AES_BLOCK_SIZE {
        return Ok(cipher);
    }

    if cipher.len() >= 2 * AES_BLOCK_SIZE {
        swap_two_last_blocks(&mut cipher)?;
    }

    cipher.resize(payload.len(), 0);

    Ok(cipher)
}
