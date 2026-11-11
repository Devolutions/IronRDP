use std::io;

use der_parser::parse_der;
use num_bigint::BigUint;

/// The smallest RSA modulus accepted from a server, in bits.
///
/// A modulus of zero would make `modpow` panic.
/// 512 bits is the size of the proprietary certificate key in [4.1.4] Server MCS Connect Response PDU with GCC
/// Conference Create Response, whose [2.2.1.4.3.1.1.1] RSA Public Key (RSA_PUBLIC_KEY) has a `bitlen` of 512, and a
/// modulus of at least 512 bits is larger than the 48-byte premaster secret it encrypts.
///
/// [4.1.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/d23f7725-876c-48d4-9e41-8288896a19d3
/// [2.2.1.4.3.1.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/fe93545c-772a-4ade-9d02-ad1e0d81b6af
const MIN_MODULUS_BITS: u64 = 512;

pub(crate) fn encrypt_with_public_key(message: &[u8], public_key_der: &[u8]) -> io::Result<Vec<u8>> {
    let (_, der_object) = parse_der(public_key_der).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unable to parse public key from DER: {err:?}"),
        )
    })?;

    let der_object_sequence = der_object.as_sequence().map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unable to extract a sequence from the DER object: {err:?}"),
        )
    })?;

    if der_object_sequence.len() != 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "DER object sequence is empty",
        ));
    }

    let n = der_object_sequence[0].as_slice().map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unable to extract a slice from public key modulus sequence: {err:?}"),
        )
    })?;

    let e = der_object_sequence[1].as_slice().map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unable to extract a slice from public key exponent sequence: {err:?}"),
        )
    })?;

    let n = BigUint::from_bytes_be(n);
    if n.bits() < MIN_MODULUS_BITS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("RSA public key modulus is shorter than {MIN_MODULUS_BITS} bits"),
        ));
    }
    let e = BigUint::from_bytes_be(e);
    let m = BigUint::from_bytes_le(message);
    let c = m.modpow(&e, &n);

    let mut result = c.to_bytes_le();
    result.resize(result.len() + 8, 0u8);

    Ok(result)
}
