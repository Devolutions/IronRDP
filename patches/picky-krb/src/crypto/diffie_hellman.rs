use crypto_bigint::modular::{BoxedMontyForm, BoxedMontyParams};
use crypto_bigint::{BoxedUint, Odd, RandomBits, Resize};
use rand::TryCryptoRng;
use sha1::{Digest, Sha1};
use thiserror::Error;

use crate::crypto::Cipher;

#[derive(Error, Debug)]
pub enum DiffieHellmanError {
    #[error("Invalid bit len: {0}")]
    BitLen(String),
    #[error("Invalid data len: expected at least {0} but got {1}.")]
    DataLen(usize, usize),
    #[error("modulus is not odd")]
    ModulusIsNotOdd,
}

pub type DiffieHellmanResult<T> = Result<T, DiffieHellmanError>;

/// [Using Diffie-Hellman Key Exchange](https://www.rfc-editor.org/rfc/rfc4556.html#section-3.2.3.1)
/// K-truncate truncates its input to the first K bits
fn k_truncate(k: usize, mut data: Vec<u8>) -> DiffieHellmanResult<Vec<u8>> {
    if k % 8 != 0 {
        return Err(DiffieHellmanError::BitLen(format!(
            "Seed bit len must be a multiple of 8. Got: {}",
            k
        )));
    }

    let bytes_len = k / 8;

    if bytes_len > data.len() {
        return Err(DiffieHellmanError::DataLen(bytes_len, data.len()));
    }

    data.resize(bytes_len, 0);

    Ok(data)
}

/// [Using Diffie-Hellman Key Exchange](https://www.rfc-editor.org/rfc/rfc4556.html#section-3.2.3.1)
/// octetstring2key(x) == random-to-key(K-truncate(
///                          SHA1(0x00 | x) |
///                          SHA1(0x01 | x) |
///                          SHA1(0x02 | x) |
///                          ...
///                          ))
fn octet_string_to_key(x: &[u8], cipher: &dyn Cipher) -> DiffieHellmanResult<Vec<u8>> {
    let seed_len = cipher.seed_bit_len() / 8;

    let mut key = Vec::new();

    let mut i = 0;
    while key.len() < seed_len {
        let mut data = vec![i];
        data.extend_from_slice(x);

        let mut sha1 = Sha1::new();
        sha1.update(data);

        key.extend_from_slice(sha1.finalize().as_slice());
        i += 1;
    }

    Ok(cipher.random_to_key(k_truncate(seed_len * 8, key)?))
}

pub struct DhNonce<'a> {
    pub client_nonce: &'a [u8],
    pub server_nonce: &'a [u8],
}

/// [Using Diffie-Hellman Key Exchange](https://www.rfc-editor.org/rfc/rfc4556.html#section-3.2.3.1)
/// let n_c be the clientDHNonce and n_k be the serverDHNonce; otherwise, let both n_c and n_k be empty octet strings.
/// k = octetstring2key(DHSharedSecret | n_c | n_k)
pub fn generate_key_from_shared_secret(
    mut dh_shared_secret: Vec<u8>,
    nonce: Option<DhNonce>,
    cipher: &dyn Cipher,
) -> DiffieHellmanResult<Vec<u8>> {
    if let Some(DhNonce {
        client_nonce,
        server_nonce,
    }) = nonce
    {
        dh_shared_secret.extend_from_slice(client_nonce);
        dh_shared_secret.extend_from_slice(server_nonce);
    }

    octet_string_to_key(&dh_shared_secret, cipher)
}

/// [Using Diffie-Hellman Key Exchange](https://www.rfc-editor.org/rfc/rfc4556.html#section-3.2.3.1)
/// let DHSharedSecret be the shared secret value. DHSharedSecret is the value ZZ
///
/// [Generation of ZZ](https://www.rfc-editor.org/rfc/rfc2631#section-2.1.1)
/// ZZ = g ^ (xb * xa) mod p
/// ZZ = (yb ^ xa)  mod p  = (ya ^ xb)  mod p
/// where ^ denotes exponentiation
fn generate_dh_shared_secret(public_key: &[u8], private_key: &[u8], p: &[u8]) -> DiffieHellmanResult<Vec<u8>> {
    let public_key = BoxedUint::from_be_slice_vartime(public_key);
    let private_key = BoxedUint::from_be_slice_vartime(private_key);
    let p = Odd::new(BoxedUint::from_be_slice_vartime(p))
        .into_option()
        .ok_or(DiffieHellmanError::ModulusIsNotOdd)?;
    let p = BoxedMontyParams::new_vartime(p);

    // ZZ = (public_key ^ private_key) mod p
    let out = pow_mod_params(&public_key, &private_key, &p);
    Ok(out.to_be_bytes().to_vec())
}

//= [Using Diffie-Hellman Key Exchange](https://www.rfc-editor.org/rfc/rfc4556.html#section-3.2.3.1) =//
pub fn generate_key(
    public_key: &[u8],
    private_key: &[u8],
    modulus: &[u8],
    nonce: Option<DhNonce>,
    cipher: &dyn Cipher,
) -> DiffieHellmanResult<Vec<u8>> {
    let dh_shared_secret = generate_dh_shared_secret(public_key, private_key, modulus)?;
    generate_key_from_shared_secret(dh_shared_secret, nonce, cipher)
}

/// [Key and Parameter Requirements](https://www.rfc-editor.org/rfc/rfc2631#section-2.2)
/// X9.42 requires that the private key x be in the interval [2, (q - 2)]
pub fn generate_private_key<R: TryCryptoRng>(q: &[u8], rng: &mut R) -> Result<Vec<u8>, R::Error> {
    let q = BoxedUint::from_be_slice_vartime(q);
    let low_bound = BoxedUint::from_be_slice_vartime(&[2]);
    let high_bound = q - 1_u32;

    let min_bits = low_bound.bits();
    let max_bits = high_bound.bits();
    loop {
        let bit_length = rng.try_next_u32()? % (max_bits - min_bits) + min_bits;
        let x = BoxedUint::random_bits(rng, bit_length);

        if (&low_bound..&high_bound).contains(&&x) {
            return Ok(x.to_be_bytes().into_vec());
        }
    }
}

/// [Key and Parameter Requirements](https://www.rfc-editor.org/rfc/rfc2631#section-2.2)
/// y is then computed by calculating g^x mod p.
pub fn compute_public_key(private_key: &[u8], modulus: &[u8], base: &[u8]) -> DiffieHellmanResult<Vec<u8>> {
    generate_dh_shared_secret(base, private_key, modulus)
}

// Copied from `rsa` crate: https://github.com/RustCrypto/RSA/blob/eb1cca7b7ea42445dc874c1c1ce38873e4adade7/src/algorithms/rsa.rs#L232-L241
fn pow_mod_params(base: &BoxedUint, exp: &BoxedUint, n_params: &BoxedMontyParams) -> BoxedUint {
    let base = reduce_vartime(base, n_params);
    base.pow(exp).retrieve()
}

fn reduce_vartime(n: &BoxedUint, p: &BoxedMontyParams) -> BoxedMontyForm {
    let modulus = p.modulus().as_nz_ref().clone();
    let n_reduced = n.rem_vartime(&modulus).resize_unchecked(p.bits_precision());
    BoxedMontyForm::new(n_reduced, p.clone())
}
