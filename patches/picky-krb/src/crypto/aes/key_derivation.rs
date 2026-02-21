use pbkdf2::pbkdf2_hmac;
use sha1::Sha1;

use crate::crypto::nfold::n_fold;
use crate::crypto::{KERBEROS, KerberosCryptoResult};

use super::AesSize;
use super::encrypt::encrypt_aes_cbc;

/// https://www.rfc-editor.org/rfc/rfc3962.html#section-6
/// Default iteration count (rounds) for pbkdf2 function:
/// default string-to-key parameters        00 00 10 00
const AES_ITERATION_COUNT: u32 = 0x1000;

/// https://www.rfc-editor.org/rfc/rfc3961.html#section-3
/// random-to-key (bitstring[K])->(protocol-key)
///
/// For AES Encryption, the random-to-key function is the identity function
/// https://www.rfc-editor.org/rfc/rfc3962.html#section-6
/// random-to-key function: identity function
pub fn random_to_key(data: Vec<u8>) -> Vec<u8> {
    data
}

pub fn derive_key(key: &[u8], well_known: &[u8], aes_size: &AesSize) -> KerberosCryptoResult<Vec<u8>> {
    let mut n_fold_usage = n_fold(well_known, aes_size.block_bit_len());

    let key_len = aes_size.key_length();
    let mut out = Vec::with_capacity(key_len);

    while out.len() < key_len {
        n_fold_usage = encrypt_aes_cbc(key, &n_fold_usage, aes_size)?;
        out.extend_from_slice(&n_fold_usage);
    }

    Ok(out)
}

pub fn derive_key_from_password<P: AsRef<[u8]>, S: AsRef<[u8]>>(
    password: P,
    salt: S,
    aes_size: &AesSize,
) -> KerberosCryptoResult<Vec<u8>> {
    let mut tmp = vec![0; aes_size.key_length()];

    pbkdf2_hmac::<Sha1>(password.as_ref(), salt.as_ref(), AES_ITERATION_COUNT, &mut tmp);

    let temp_key = random_to_key(tmp);

    derive_key(&temp_key, KERBEROS, aes_size)
}

#[cfg(test)]
mod tests {
    use crate::crypto::aes::AesSize;

    use super::derive_key_from_password;

    #[test]
    fn aes256_derive_key_from_password() {
        let password = "5hYYSAfFJp";
        let salt = "EXAMPLE.COMtest1";

        let key = derive_key_from_password(password, salt, &AesSize::Aes256).unwrap();

        assert_eq!(
            &[
                218, 222, 209, 204, 21, 174, 23, 222, 170, 99, 164, 144, 247, 103, 137, 68, 117, 143, 59, 37, 90, 84,
                37, 105, 203, 32, 235, 167, 97, 238, 171, 172
            ],
            key.as_slice()
        );
    }

    #[test]
    fn aes128_derive_key_from_password() {
        let password = "5hYYSAfFJp";
        let salt = "EXAMPLE.COMtest1";

        let key = derive_key_from_password(password, salt, &AesSize::Aes128).unwrap();

        assert_eq!(
            &[187, 67, 208, 2, 227, 119, 67, 22, 18, 86, 174, 201, 6, 129, 207, 220],
            key.as_slice()
        );
    }
}
