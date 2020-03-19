use std::{collections::HashMap, hash::Hash, io};

use num_derive::{FromPrimitive, ToPrimitive};
use x509_parser::parse_x509_der;

#[macro_export]
macro_rules! eof_try {
    ($e:expr) => {
        match $e {
            Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Ok(None);
            }
            result => result,
        }
    };
}

#[macro_export]
macro_rules! try_ready {
    ($e:expr) => {
        match $e {
            Ok(Some(v)) => Ok(v),
            Ok(None) => return Ok(None),
            Err(e) => Err(e),
        }
    };
}

pub fn socket_addr_to_string(socket_addr: std::net::SocketAddr) -> String {
    format!("{}:{}", socket_addr.ip(), socket_addr.port())
}

pub fn get_tls_peer_pubkey(cert: Vec<u8>) -> io::Result<Vec<u8>> {
    let res = parse_x509_der(&cert[..])
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid der certificate."))?;
    let public_key = res.1.tbs_certificate.subject_pki.subject_public_key;

    Ok(public_key.data.to_vec())
}

pub fn swap_hashmap_kv<K, V>(hm: HashMap<K, V>) -> HashMap<V, K>
where
    V: Hash + Eq,
{
    let mut result = HashMap::with_capacity(hm.len());
    for (k, v) in hm {
        result.insert(v, k);
    }

    result
}

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum CodecId {
    RemoteFx = 0x3,
}
