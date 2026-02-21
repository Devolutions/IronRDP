use aes::cipher::{Array, KeyIvInit};
use des::TdesEde3;
use des::cipher::BlockModeDecrypt;
use des::cipher::block_padding::NoPadding;

use crate::crypto::common::hmac_sha1;
use crate::crypto::utils::{usage_ke, usage_ki};
use crate::crypto::{DecryptWithoutChecksum, KerberosCryptoError, KerberosCryptoResult};

use super::{DES3_BLOCK_SIZE, DES3_KEY_SIZE, DES3_MAC_SIZE, derive_key};

type DesCbcCipher = cbc::Decryptor<TdesEde3>;

//= [Cryptosystem Profile Based on Simplified Profile](https://datatracker.ietf.org/doc/html/rfc3961#section-5.3) =//
pub fn decrypt_message(key: &[u8], key_usage: i32, cipher_data: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
    let decryption_result = decrypt_message_no_checksum(key, key_usage, cipher_data)?;
    let calculated_hmac = hmac_sha1(
        &decryption_result.ki,
        &[
            decryption_result.confounder.as_slice(),
            decryption_result.plaintext.as_slice(),
        ]
        .concat(),
        DES3_MAC_SIZE,
    );

    // if (H1 != HMAC(Ki, P1)[1..h])
    if calculated_hmac != decryption_result.checksum {
        return Err(KerberosCryptoError::IntegrityCheck);
    }

    Ok(decryption_result.plaintext)
}

/// Returns (Plaintext, conf, H1, Ki)
pub fn decrypt_message_no_checksum(
    key: &[u8],
    key_usage: i32,
    cipher_data: &[u8],
) -> KerberosCryptoResult<DecryptWithoutChecksum> {
    if key.len() != DES3_KEY_SIZE {
        return Err(KerberosCryptoError::KeyLength(key.len(), DES3_KEY_SIZE));
    }

    // (C1,H1) = ciphertext
    let (cipher_data, checksum) = cipher_data.split_at(cipher_data.len() - DES3_MAC_SIZE);

    let ke = derive_key(key, &usage_ke(key_usage))?;
    // (P1, newIV) = D(Ke, C1, oldstate.ivec)
    let plaintext = decrypt_des(&ke, cipher_data)?;

    let ki = derive_key(key, &usage_ki(key_usage))?;

    // [0..DES3_BLOCK_SIZE] = the first block is random confounder bytes.
    let (confounder, plaintext) = plaintext.split_at(DES3_BLOCK_SIZE);

    Ok(DecryptWithoutChecksum {
        plaintext: plaintext.to_vec(),
        confounder: confounder.to_vec(),
        checksum: checksum.to_vec(),
        ki,
    })
}

pub fn decrypt_des(key: &[u8], payload: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
    if key.len() != DES3_KEY_SIZE {
        return Err(KerberosCryptoError::KeyLength(key.len(), DES3_KEY_SIZE));
    }

    let mut payload = payload.to_vec();

    // RFC 3961: initial cipher state      All bits zero
    let iv = [0_u8; DES3_BLOCK_SIZE];
    let key = Array::try_from(key)?;
    let cipher = DesCbcCipher::new(&key, &iv.into());

    cipher.decrypt_padded_inout::<NoPadding>(payload.as_mut_slice().into())?;

    Ok(payload)
}
