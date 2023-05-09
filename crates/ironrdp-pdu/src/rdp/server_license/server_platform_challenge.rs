#[cfg(test)]
mod test;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::{
    read_license_header, BlobHeader, BlobType, LicenseHeader, PreambleType, ServerLicenseError, BLOB_LENGTH_SIZE,
    BLOB_TYPE_SIZE, MAC_SIZE,
};
use crate::PduParsing;

const CONNECT_FLAGS_FIELD_SIZE: usize = 4;

#[derive(Debug, PartialEq, Eq)]
pub struct ServerPlatformChallenge {
    pub license_header: LicenseHeader,
    pub encrypted_platform_challenge: Vec<u8>,
    pub mac_data: Vec<u8>,
}

impl PduParsing for ServerPlatformChallenge {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let license_header = read_license_header(PreambleType::PlatformChallenge, &mut stream)?;

        let _connect_flags = stream.read_u32::<LittleEndian>()?;

        let blob_header = BlobHeader::read_any_blob_from_buffer(&mut stream)?;

        let mut encrypted_platform_challenge = vec![0u8; blob_header.length];
        stream.read_exact(&mut encrypted_platform_challenge)?;

        let mut mac_data = vec![0u8; MAC_SIZE];
        stream.read_exact(&mut mac_data)?;

        Ok(Self {
            license_header,
            encrypted_platform_challenge,
            mac_data,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        self.license_header.to_buffer(&mut stream)?;

        stream.write_u32::<LittleEndian>(0)?; // connect_flags, ignored

        BlobHeader::new(BlobType::Any, self.encrypted_platform_challenge.len()).write_to_buffer(&mut stream)?;
        stream.write_all(&self.encrypted_platform_challenge)?;

        stream.write_all(&self.mac_data)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        self.license_header.buffer_length()
            + CONNECT_FLAGS_FIELD_SIZE
            + MAC_SIZE
            + BLOB_LENGTH_SIZE
            + BLOB_TYPE_SIZE
            + self.encrypted_platform_challenge.len()
    }
}
