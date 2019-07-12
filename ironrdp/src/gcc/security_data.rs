#[cfg(test)]
pub mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::PduParsing;

const CLIENT_ENCRYPTION_METHODS_SIZE: usize = 4;
const CLIENT_EXT_ENCRYPTION_METHODS_SIZE: usize = 4;

const SERVER_ENCRYPTION_METHOD_SIZE: usize = 4;
const SERVER_ENCRYPTION_LEVEL_SIZE: usize = 4;
const SERVER_RANDOM_LEN_SIZE: usize = 4;
const SERVER_CERT_LEN_SIZE: usize = 4;
const SERVER_RANDOM_LEN: usize = 0x20;

#[derive(Debug, Clone, PartialEq)]
pub struct ClientSecurityData {
    pub encryption_methods: EncryptionMethod,
    pub ext_encryption_methods: u32,
}

impl PduParsing for ClientSecurityData {
    type Error = SecurityDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let encryption_methods = EncryptionMethod::from_bits(buffer.read_u32::<LittleEndian>()?)
            .ok_or(SecurityDataError::InvalidEncryptionMethod)?;
        let ext_encryption_methods = buffer.read_u32::<LittleEndian>()?;

        Ok(Self {
            encryption_methods,
            ext_encryption_methods,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.encryption_methods.bits())?;
        buffer.write_u32::<LittleEndian>(self.ext_encryption_methods)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        CLIENT_ENCRYPTION_METHODS_SIZE + CLIENT_EXT_ENCRYPTION_METHODS_SIZE
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerSecurityData {
    pub encryption_method: EncryptionMethod,
    pub encryption_level: EncryptionLevel,
    pub server_random: Option<[u8; SERVER_RANDOM_LEN]>,
    pub server_cert: Vec<u8>,
}

impl PduParsing for ServerSecurityData {
    type Error = SecurityDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let encryption_method = EncryptionMethod::from_bits(buffer.read_u32::<LittleEndian>()?)
            .ok_or(SecurityDataError::InvalidEncryptionMethod)?;
        let encryption_level =
            EncryptionLevel::from_u32(buffer.read_u32::<LittleEndian>()?).unwrap();

        let (server_random, server_cert) =
            if encryption_method.is_empty() && encryption_level == EncryptionLevel::None {
                (None, Vec::new())
            } else {
                let server_random_len = buffer.read_u32::<LittleEndian>()?;
                if server_random_len != SERVER_RANDOM_LEN as u32 {
                    return Err(SecurityDataError::InvalidServerRandomLen);
                }

                let server_cert_len = buffer.read_u32::<LittleEndian>()?;

                let mut server_random = [0; SERVER_RANDOM_LEN];
                buffer.read_exact(&mut server_random)?;

                let mut server_cert = vec![0; server_cert_len as usize];
                buffer.read_exact(&mut server_cert)?;

                (Some(server_random), server_cert)
            };

        Ok(Self {
            encryption_method,
            encryption_level,
            server_random,
            server_cert,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        buffer.write_u32::<LittleEndian>(self.encryption_method.bits())?;
        buffer.write_u32::<LittleEndian>(self.encryption_level.to_u32().unwrap())?;

        if self.encryption_method.is_empty() && self.encryption_level == EncryptionLevel::None {
            if self.server_random.is_some() || !self.server_cert.is_empty() {
                Err(SecurityDataError::InvalidInput(String::from(
                    "An encryption method and encryption level is none, but the server random or certificate is not empty",
                )))
            } else {
                Ok(())
            }
        } else {
            let server_random_len = match self.server_random {
                Some(ref server_random) => server_random.len(),
                None => 0,
            };
            buffer.write_u32::<LittleEndian>(server_random_len as u32)?;
            buffer.write_u32::<LittleEndian>(self.server_cert.len() as u32)?;

            if let Some(ref server_random) = self.server_random {
                buffer.write_all(server_random.as_ref())?;
            }
            buffer.write_all(self.server_cert.as_ref())?;

            Ok(())
        }
    }

    fn buffer_length(&self) -> usize {
        let mut size = SERVER_ENCRYPTION_METHOD_SIZE + SERVER_ENCRYPTION_LEVEL_SIZE;

        if let Some(ref server_random) = self.server_random {
            size += SERVER_RANDOM_LEN_SIZE + server_random.len();
        }
        if !self.server_cert.is_empty() {
            size += SERVER_CERT_LEN_SIZE + self.server_cert.len();
        }

        size
    }
}

bitflags! {
    pub struct EncryptionMethod: u32 {
        const BIT_40 = 0x0000_0001;
        const BIT_128 = 0x0000_0002;
        const BIT_56 = 0x0000_0008;
        const FIPS = 0x0000_0010;
    }
}

#[derive(Debug, Clone, PartialEq, FromPrimitive, ToPrimitive)]
pub enum EncryptionLevel {
    None = 0,
    Low = 1,
    ClientCompatible = 2,
    High = 3,
    Fips = 4,
}

#[derive(Debug, Fail)]
pub enum SecurityDataError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "Invalid encryption methods field")]
    InvalidEncryptionMethod,
    #[fail(display = "Invalid server random length field")]
    InvalidServerRandomLen,
    #[fail(display = "Invalid input: {}", _0)]
    InvalidInput(String),
}

impl_from_error!(io::Error, SecurityDataError, SecurityDataError::IOError);
