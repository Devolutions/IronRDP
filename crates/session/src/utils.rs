use std::collections::HashMap;
use std::hash::Hash;

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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum CodecId {
    RemoteFx = 0x3,
}

impl CodecId {
    pub const fn from_u8(value: u8) -> Option<Self> {
        if value == 0x3 {
            Some(Self::RemoteFx)
        } else {
            None
        }
    }
}
