use crate::crypto::common::hmac_sha1;
use crate::crypto::utils::{usage_ke, usage_ki};
use crate::crypto::{DecryptWithoutChecksum, KerberosCryptoError, KerberosCryptoResult};
use aes::cipher::{Array, KeyIvInit};
use aes::{Aes128, Aes256};
use cbc::cipher::BlockModeDecrypt;
use cbc::cipher::block_padding::NoPadding;

use super::key_derivation::derive_key;
use super::{AES_BLOCK_SIZE, AES_MAC_SIZE, AesSize, swap_two_last_blocks};

pub type Aes256CbcDecryptor = cbc::Decryptor<Aes256>;
pub type Aes128CbcDecryptor = cbc::Decryptor<Aes128>;

//= [Cryptosystem Profile Based on Simplified Profile](https://datatracker.ietf.org/doc/html/rfc3961#section-5.3) =//
pub fn decrypt_message(
    key: &[u8],
    key_usage: i32,
    cipher_data: &[u8],
    aes_size: &AesSize,
) -> KerberosCryptoResult<Vec<u8>> {
    let decryption_result = decrypt_message_no_checksum(key, key_usage, cipher_data, aes_size)?;
    let calculated_checksum = hmac_sha1(
        &decryption_result.ki,
        &[
            decryption_result.confounder.as_slice(),
            decryption_result.plaintext.as_slice(),
        ]
        .concat(),
        AES_MAC_SIZE,
    );

    // if (H1 != HMAC(Ki, P1)[1..h])
    if calculated_checksum != decryption_result.checksum {
        return Err(KerberosCryptoError::IntegrityCheck);
    }

    Ok(decryption_result.plaintext)
}

/// Returns (Plaintext, conf, H1, Ki)
pub fn decrypt_message_no_checksum(
    key: &[u8],
    key_usage: i32,
    cipher_data: &[u8],
    aes_size: &AesSize,
) -> KerberosCryptoResult<DecryptWithoutChecksum> {
    if cipher_data.len() < AES_BLOCK_SIZE + AES_MAC_SIZE {
        return Err(KerberosCryptoError::CipherLength(
            cipher_data.len(),
            AES_BLOCK_SIZE + AES_MAC_SIZE,
        ));
    }

    // (C1,H1) = ciphertext
    let (cipher_data, checksum) = cipher_data.split_at(cipher_data.len() - AES_MAC_SIZE);

    let ke = derive_key(key, &usage_ke(key_usage), aes_size)?;
    // (P1, newIV) = D(Ke, C1, oldstate.ivec)
    let plaintext = decrypt_aes_cts(&ke, cipher_data, aes_size)?;

    let ki = derive_key(key, &usage_ki(key_usage), aes_size)?;

    // [0..AES_BLOCK_SIZE] = the first block is a random confounder bytes.
    let (confounder, plaintext) = plaintext.split_at(AES_BLOCK_SIZE);

    Ok(DecryptWithoutChecksum {
        plaintext: plaintext.to_vec(),
        confounder: confounder.to_vec(),
        checksum: checksum.to_vec(),
        ki,
    })
}

pub fn decrypt_aes_cbc(key: &[u8], cipher_data: &[u8], aes_size: &AesSize) -> KerberosCryptoResult<Vec<u8>> {
    let mut cipher_data = cipher_data.to_vec();

    let iv = [0; AES_BLOCK_SIZE];

    match aes_size {
        AesSize::Aes256 => {
            let key = Array::try_from(key)?;
            let cipher = Aes256CbcDecryptor::new(&key, &iv.into());
            cipher.decrypt_padded_inout::<NoPadding>(cipher_data.as_mut_slice().into())?;
        }
        AesSize::Aes128 => {
            let key = Array::try_from(key)?;
            let cipher = Aes128CbcDecryptor::new(&key, &iv.into());
            cipher.decrypt_padded_inout::<NoPadding>(cipher_data.as_mut_slice().into())?;
        }
    }

    Ok(cipher_data)
}

//= [CTS using CBC](https://en.wikipedia.org/wiki/Ciphertext_stealing#CBC_ciphertext_stealing_decryption_using_a_standard_CBC_interface) =//
pub fn decrypt_aes_cts(key: &[u8], cipher_data: &[u8], aes_size: &AesSize) -> KerberosCryptoResult<Vec<u8>> {
    if cipher_data.len() == AES_BLOCK_SIZE {
        return decrypt_aes_cbc(key, cipher_data, aes_size);
    }

    let pad_length = (AES_BLOCK_SIZE - (cipher_data.len() % AES_BLOCK_SIZE)) % AES_BLOCK_SIZE;

    let mut cipher = cipher_data.to_vec();

    if pad_length != 0 {
        // decrypt Cn-1 with iv = 0.
        let start = cipher.len() + pad_length - AES_BLOCK_SIZE * 2;

        let dn = decrypt_aes_cbc(key, &cipher[start..start + AES_BLOCK_SIZE], aes_size)?;

        let dn_len = dn.len();
        cipher.extend_from_slice(&dn[dn_len - pad_length..]);
    }

    if cipher.len() >= 2 * AES_BLOCK_SIZE {
        swap_two_last_blocks(&mut cipher)?;
    }

    let mut plaintext = decrypt_aes_cbc(key, &cipher, aes_size)?;

    plaintext.resize(cipher.len() - pad_length, 0);

    Ok(plaintext)
}
