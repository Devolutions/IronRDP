use std::io;

use bitflags::bitflags;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};
use thiserror::Error;

use crate::{PduDecode, PduEncode, PduResult};
use ironrdp_core::{ReadCursor, WriteCursor};

const CLIENT_ENCRYPTION_METHODS_SIZE: usize = 4;
const CLIENT_EXT_ENCRYPTION_METHODS_SIZE: usize = 4;

const SERVER_ENCRYPTION_METHOD_SIZE: usize = 4;
const SERVER_ENCRYPTION_LEVEL_SIZE: usize = 4;
const SERVER_RANDOM_LEN_SIZE: usize = 4;
const SERVER_CERT_LEN_SIZE: usize = 4;
const SERVER_RANDOM_LEN: usize = 0x20;
const MAX_SERVER_CERT_LEN: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSecurityData {
    pub encryption_methods: EncryptionMethod,
    pub ext_encryption_methods: u32,
}

impl ClientSecurityData {
    const NAME: &'static str = "ClientSecurityData";

    const FIXED_PART_SIZE: usize = CLIENT_ENCRYPTION_METHODS_SIZE + CLIENT_EXT_ENCRYPTION_METHODS_SIZE;

    pub fn no_security() -> Self {
        Self {
            encryption_methods: EncryptionMethod::empty(),
            ext_encryption_methods: 0,
        }
    }
}

impl PduEncode for ClientSecurityData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.encryption_methods.bits());
        dst.write_u32(self.ext_encryption_methods);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for ClientSecurityData {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let encryption_methods = EncryptionMethod::from_bits(src.read_u32())
            .ok_or_else(|| invalid_message_err!("encryptionMethods", "invalid encryption methods"))?;
        let ext_encryption_methods = src.read_u32();

        Ok(Self {
            encryption_methods,
            ext_encryption_methods,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSecurityData {
    pub encryption_method: EncryptionMethod,
    pub encryption_level: EncryptionLevel,
    pub server_random: Option<[u8; SERVER_RANDOM_LEN]>,
    pub server_cert: Vec<u8>,
}

impl ServerSecurityData {
    const NAME: &'static str = "ServerSecurityData";

    const FIXED_PART_SIZE: usize = SERVER_ENCRYPTION_METHOD_SIZE + SERVER_ENCRYPTION_LEVEL_SIZE;

    pub fn no_security() -> Self {
        Self {
            encryption_method: EncryptionMethod::empty(),
            encryption_level: EncryptionLevel::None,
            server_random: None,
            server_cert: Vec::new(),
        }
    }
}

impl PduEncode for ServerSecurityData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.encryption_method.bits());
        dst.write_u32(self.encryption_level.to_u32().unwrap());

        if self.encryption_method.is_empty() && self.encryption_level == EncryptionLevel::None {
            if self.server_random.is_some() || !self.server_cert.is_empty() {
                Err(invalid_message_err!("serverRandom", "An encryption method and encryption level is none, but the server random or certificate is not empty"))
            } else {
                Ok(())
            }
        } else {
            let server_random_len = match self.server_random {
                Some(ref server_random) => server_random.len(),
                None => 0,
            };
            dst.write_u32(cast_length!("serverRandomLen", server_random_len)?);
            dst.write_u32(cast_length!("serverCertLen", self.server_cert.len())?);

            if let Some(ref server_random) = self.server_random {
                dst.write_slice(server_random.as_ref());
            }
            dst.write_slice(self.server_cert.as_ref());

            Ok(())
        }
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let mut size = Self::FIXED_PART_SIZE;

        if let Some(ref server_random) = self.server_random {
            size += SERVER_RANDOM_LEN_SIZE + server_random.len();
        }
        if !self.server_cert.is_empty() {
            size += SERVER_CERT_LEN_SIZE + self.server_cert.len();
        }

        size
    }
}

impl<'de> PduDecode<'de> for ServerSecurityData {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let encryption_method = EncryptionMethod::from_bits(src.read_u32())
            .ok_or_else(|| invalid_message_err!("encryptionMethod", "invalid encryption method"))?;
        let encryption_level = EncryptionLevel::from_u32(src.read_u32())
            .ok_or_else(|| invalid_message_err!("encryptionLevel", "invalid encryption level"))?;

        let (server_random, server_cert) = if encryption_method.is_empty() && encryption_level == EncryptionLevel::None
        {
            (None, Vec::new())
        } else {
            ensure_size!(in: src, size: 4 + 4);

            let server_random_len: usize = cast_length!("serverRandomLen", src.read_u32())?;
            if server_random_len != SERVER_RANDOM_LEN {
                return Err(invalid_message_err!("serverRandomLen", "Invalid server random length"));
            }

            let server_cert_len = cast_length!("serverCertLen", src.read_u32())?;

            if server_cert_len > MAX_SERVER_CERT_LEN {
                return Err(invalid_message_err!(
                    "serverCetLen",
                    "Invalid server certificate length"
                ));
            }

            ensure_size!(in: src, size: SERVER_RANDOM_LEN);
            let server_random = src.read_array();

            ensure_size!(in: src, size: server_cert_len);
            let server_cert = src.read_slice(server_cert_len);

            (Some(server_random), server_cert.into())
        };

        Ok(Self {
            encryption_method,
            encryption_level,
            server_random,
            server_cert,
        })
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct EncryptionMethod: u32 {
        const BIT_40 = 0x0000_0001;
        const BIT_128 = 0x0000_0002;
        const BIT_56 = 0x0000_0008;
        const FIPS = 0x0000_0010;
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum EncryptionLevel {
    None = 0,
    Low = 1,
    ClientCompatible = 2,
    High = 3,
    Fips = 4,
}

#[derive(Debug, Error)]
pub enum SecurityDataError {
    #[error("IO error")]
    IOError(#[from] io::Error),
    #[error("invalid encryption methods field")]
    InvalidEncryptionMethod,
    #[error("invalid encryption level field")]
    InvalidEncryptionLevel,
    #[error("invalid server random length field: {0}")]
    InvalidServerRandomLen(u32),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("invalid server certificate length: {0}")]
    InvalidServerCertificateLen(u32),
}
