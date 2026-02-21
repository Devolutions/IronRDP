//! [Encryption and Checksum Specifications for Kerberos 5](https://datatracker.ietf.org/doc/html/rfc3961)
//! This RFC contains key derivation algorithm for triple DES
//!
//! [DES-Based Encryption and Checksum Types](https://datatracker.ietf.org/doc/html/rfc3961#section-6.2)
//! This sections contains explanation and pseudo code of the implemented functions

use crate::crypto::nfold::n_fold;
use crate::crypto::{KERBEROS, KerberosCryptoError, KerberosCryptoResult};

use super::encrypt::encrypt_des;
use super::{DES3_BLOCK_SIZE, DES3_KEY_SIZE, DES3_SEED_LEN};

// weak keys from https://nvlpubs.nist.gov/nistpubs/Legacy/SP/nistspecialpublication800-67r1.pdf
const WEAK_KEYS: [[u8; 8]; 4] = [
    [0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01],
    [0xFE, 0xFE, 0xFE, 0xFE, 0xFE, 0xFE, 0xFE, 0xFE],
    [0xE0, 0xE0, 0xE0, 0xE0, 0xF1, 0xF1, 0xF1, 0xF1],
    [0x1F, 0x1F, 0x1F, 0x1F, 0x0E, 0x0E, 0x0E, 0x0E],
];

const SEMI_WEAK_KEYS: [[u8; 8]; 12] = [
    [0x01, 0x1F, 0x01, 0x1F, 0x01, 0x0E, 0x01, 0x0E],
    [0x1F, 0x01, 0x1F, 0x01, 0x0E, 0x01, 0x0E, 0x01],
    [0x01, 0xE0, 0x01, 0xE0, 0x01, 0xF1, 0x01, 0xF1],
    [0xE0, 0x01, 0xE0, 0x01, 0xF1, 0x01, 0xF1, 0x01],
    [0x01, 0xFE, 0x01, 0xFE, 0x01, 0xFE, 0x01, 0xFE],
    [0xFE, 0x01, 0xFE, 0x01, 0xFE, 0x01, 0xFE, 0x01],
    [0x1F, 0xE0, 0x1F, 0xE0, 0x0E, 0xF1, 0x0E, 0xF1],
    [0xE0, 0x1F, 0xE0, 0x1F, 0xF1, 0x0E, 0xF1, 0x0E],
    [0x1F, 0xFE, 0x1F, 0xFE, 0x0E, 0xFE, 0x0E, 0xFE],
    [0xFE, 0x1F, 0xFE, 0x1F, 0xFE, 0x0E, 0xFE, 0x0E],
    [0xE0, 0xFE, 0xE0, 0xFE, 0xF1, 0xFE, 0xF1, 0xFE],
    [0xFE, 0xE0, 0xFE, 0xE0, 0xFE, 0xF1, 0xFE, 0xF1],
];

pub fn derive_key(key: &[u8], well_known: &[u8]) -> KerberosCryptoResult<Vec<u8>> {
    if key.len() != DES3_KEY_SIZE {
        return Err(KerberosCryptoError::KeyLength(key.len(), DES3_KEY_SIZE));
    }

    let mut n_fold_usage = n_fold(well_known, DES3_BLOCK_SIZE * 8);

    let key_len = DES3_SEED_LEN;
    let mut out = Vec::with_capacity(key_len);

    while out.len() < key_len {
        n_fold_usage = encrypt_des(key, &n_fold_usage)?;
        out.extend_from_slice(&n_fold_usage);
    }

    Ok(random_to_key(&out))
}

fn fix_weak_key(mut key: Vec<u8>) -> Vec<u8> {
    if weak(&key) {
        key[7] ^= 0xF0;
    }

    key
}

fn weak(key: &[u8]) -> bool {
    for weak_key in WEAK_KEYS {
        if weak_key == key {
            return true;
        }
    }

    for weak_key in SEMI_WEAK_KEYS {
        if weak_key == key {
            return true;
        }
    }

    false
}

fn calc_even_parity(mut b: u8) -> (u8, u8) {
    let lowestbit = b & 0x01;
    let mut c = 0;

    for p in 1..8 {
        let v = b & (1 << p);
        if v != 0 {
            c += 1;
        }
    }

    if c % 2 == 0 {
        b |= 1;
    } else {
        b &= !1;
    }

    (lowestbit, b)
}

fn stretch_56_bits(key: &[u8]) -> Vec<u8> {
    let mut d = key.to_vec();

    let mut lb: u8 = 0;

    for (i, value) in d.iter_mut().enumerate() {
        let (bv, nb) = calc_even_parity(*value);
        *value = nb;
        if bv != 0 {
            lb |= 1 << (i + 1);
        } else {
            lb &= !(1 << (i + 1));
        }
    }

    let (_, lb) = calc_even_parity(lb);
    d.push(lb);

    d
}

pub fn random_to_key(key: &[u8]) -> Vec<u8> {
    let mut r = fix_weak_key(stretch_56_bits(&key[0..7]));

    let r2 = fix_weak_key(stretch_56_bits(&key[7..14]));
    r.extend_from_slice(&r2);

    let r3 = fix_weak_key(stretch_56_bits(&key[14..21]));
    r.extend_from_slice(&r3);

    r
}

//= [Triple DES Key Production](https://datatracker.ietf.org/doc/html/rfc3961#section-6.3.1) =//
pub fn derive_key_from_password<P: AsRef<[u8]>, S: AsRef<[u8]>>(password: P, salt: S) -> KerberosCryptoResult<Vec<u8>> {
    let mut secret = password.as_ref().to_vec();
    secret.extend_from_slice(salt.as_ref());

    let temp_key = random_to_key(&n_fold(&secret, DES3_SEED_LEN * 8));

    derive_key(&temp_key, KERBEROS)
}

#[cfg(test)]
mod tests {
    use crate::crypto::des::derive_key_from_password;

    #[test]
    fn test_derive_key_from_password() {
        let password = "5hYYSAfFJp";
        let salt = "EXAMPLE.COMtest1";

        let key = derive_key_from_password(password, salt).unwrap();

        assert_eq!(
            &[
                115, 248, 21, 32, 230, 42, 157, 138, 158, 254, 157, 145, 13, 110, 64, 107, 173, 206, 247, 93, 55, 146,
                167, 138
            ],
            key.as_slice()
        );
    }
}
