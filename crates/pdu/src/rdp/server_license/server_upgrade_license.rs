#[cfg(test)]
mod tests;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::{
    read_license_header, BlobHeader, BlobType, LicenseEncryptionData, LicenseHeader, PreambleType, ServerLicenseError,
    BLOB_LENGTH_SIZE, BLOB_TYPE_SIZE, MAC_SIZE, UTF16_NULL_TERMINATOR_SIZE, UTF8_NULL_TERMINATOR_SIZE,
};
use crate::crypto::rc4::Rc4;
use crate::utils::CharacterSet;
use crate::{utils, PduParsing};

const NEW_LICENSE_INFO_STATIC_FIELDS_SIZE: usize = 20;

#[derive(Debug, PartialEq, Eq)]
pub struct ServerUpgradeLicense {
    pub license_header: LicenseHeader,
    pub encrypted_license_info: Vec<u8>,
    pub mac_data: Vec<u8>,
}

impl ServerUpgradeLicense {
    pub fn verify_server_license(&self, encryption_data: &LicenseEncryptionData) -> Result<(), ServerLicenseError> {
        let mut rc4 = Rc4::new(encryption_data.license_key.as_slice());
        let decrypted_license_info = rc4.process(self.encrypted_license_info.as_slice());
        let mac_data =
            super::compute_mac_data(encryption_data.mac_salt_key.as_slice(), decrypted_license_info.as_ref());

        if mac_data != self.mac_data {
            return Err(ServerLicenseError::InvalidMacData);
        }

        Ok(())
    }
}

impl PduParsing for ServerUpgradeLicense {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let license_header = read_license_header(PreambleType::NewLicense, &mut stream)?;

        if license_header.preamble_message_type != PreambleType::UpgradeLicense
            && license_header.preamble_message_type != PreambleType::NewLicense
        {
            return Err(ServerLicenseError::InvalidPreamble(format!(
                "Got {:?} but expected {:?} or {:?}",
                license_header.preamble_message_type,
                PreambleType::UpgradeLicense,
                PreambleType::NewLicense
            )));
        }

        let encrypted_license_info_blob = BlobHeader::read_from_buffer(BlobType::EncryptedData, &mut stream)?;

        let mut encrypted_license_info = vec![0u8; encrypted_license_info_blob.length];
        stream.read_exact(&mut encrypted_license_info)?;

        let mut mac_data = vec![0u8; MAC_SIZE];
        stream.read_exact(&mut mac_data)?;

        Ok(Self {
            license_header,
            encrypted_license_info,
            mac_data,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        self.license_header.to_buffer(&mut stream)?;

        BlobHeader::new(BlobType::EncryptedData, self.encrypted_license_info.len()).write_to_buffer(&mut stream)?;
        stream.write_all(&self.encrypted_license_info)?;

        stream.write_all(&self.mac_data)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        self.license_header.buffer_length()
            + BLOB_LENGTH_SIZE
            + BLOB_TYPE_SIZE
            + self.encrypted_license_info.len()
            + MAC_SIZE
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct NewLicenseInformation {
    pub version: u32,
    pub scope: String,
    pub company_name: String,
    pub product_id: String,
    pub license_info: Vec<u8>,
}

impl PduParsing for NewLicenseInformation {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let version = stream.read_u32::<LittleEndian>()?;

        let scope_len = stream.read_u32::<LittleEndian>()?;
        let scope = utils::read_string(
            &mut stream,
            scope_len as usize - UTF8_NULL_TERMINATOR_SIZE,
            CharacterSet::Ansi,
            true,
        )?;

        let company_name_len = stream.read_u32::<LittleEndian>()?;
        let company_name = utils::read_string(
            &mut stream,
            company_name_len as usize - UTF16_NULL_TERMINATOR_SIZE,
            CharacterSet::Unicode,
            true,
        )?;

        let product_id_len = stream.read_u32::<LittleEndian>()?;
        let product_id = utils::read_string(
            &mut stream,
            product_id_len as usize - UTF16_NULL_TERMINATOR_SIZE,
            CharacterSet::Unicode,
            true,
        )?;

        let license_info_len = stream.read_u32::<LittleEndian>()?;
        let mut license_info = vec![0u8; license_info_len as usize];
        stream.read_exact(&mut license_info)?;

        Ok(Self {
            version,
            scope,
            company_name,
            product_id,
            license_info,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.version)?;

        stream.write_u32::<LittleEndian>((self.scope.len() + UTF8_NULL_TERMINATOR_SIZE) as u32)?;
        utils::write_string_with_null_terminator(&mut stream, &self.scope, CharacterSet::Ansi)?;

        stream.write_u32::<LittleEndian>((self.company_name.len() * 2 + UTF16_NULL_TERMINATOR_SIZE) as u32)?;
        utils::write_string_with_null_terminator(&mut stream, &self.company_name, CharacterSet::Unicode)?;

        stream.write_u32::<LittleEndian>((self.product_id.len() * 2 + UTF16_NULL_TERMINATOR_SIZE) as u32)?;
        utils::write_string_with_null_terminator(&mut stream, &self.product_id, CharacterSet::Unicode)?;

        stream.write_u32::<LittleEndian>(self.license_info.len() as u32)?;
        stream.write_all(self.license_info.as_slice())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        NEW_LICENSE_INFO_STATIC_FIELDS_SIZE + self.scope.len() + UTF8_NULL_TERMINATOR_SIZE
        + self.company_name.len() * 2 // utf16
        + UTF16_NULL_TERMINATOR_SIZE
        + self.product_id.len() * 2 // utf16
        + UTF16_NULL_TERMINATOR_SIZE
        + self.license_info.len()
    }
}
