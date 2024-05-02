#[cfg(test)]
mod tests;

use super::{
    BlobHeader, BlobType, LicenseEncryptionData, LicenseHeader, PreambleType, ServerLicenseError, BLOB_LENGTH_SIZE,
    BLOB_TYPE_SIZE, MAC_SIZE, UTF16_NULL_TERMINATOR_SIZE, UTF8_NULL_TERMINATOR_SIZE,
};
use crate::crypto::rc4::Rc4;
use crate::cursor::{ReadCursor, WriteCursor};
use crate::utils::CharacterSet;
use crate::{utils, PduDecode, PduEncode, PduResult};

const NEW_LICENSE_INFO_STATIC_FIELDS_SIZE: usize = 20;

/// [2.2.2.6] Server Upgrade License (SERVER_UPGRADE_LICENSE)
///
/// [2.2.2.6]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpele/e8339fbd-1fe3-42c2-a599-27c04407166d
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

impl ServerUpgradeLicense {
    const NAME: &'static str = "ServerUpgradeLicense";
}

impl ServerUpgradeLicense {
    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.license_header.encode(dst)?;
        BlobHeader::new(BlobType::ENCRYPTED_DATA, self.encrypted_license_info.len()).encode(dst)?;
        dst.write_slice(&self.encrypted_license_info);
        dst.write_slice(&self.mac_data);

        Ok(())
    }

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn size(&self) -> usize {
        self.license_header.size() + BLOB_LENGTH_SIZE + BLOB_TYPE_SIZE + self.encrypted_license_info.len() + MAC_SIZE
    }
}

impl ServerUpgradeLicense {
    pub fn decode(license_header: LicenseHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        if license_header.preamble_message_type != PreambleType::UpgradeLicense
            && license_header.preamble_message_type != PreambleType::NewLicense
        {
            return Err(invalid_message_err!(
                "preambleType",
                "got unexpected message preamble type"
            ));
        }

        let encrypted_license_info_blob = BlobHeader::decode(src)?;
        if encrypted_license_info_blob.blob_type != BlobType::ENCRYPTED_DATA {
            return Err(invalid_message_err!("blobType", "unexpected blob type"));
        }

        ensure_size!(in: src, size: encrypted_license_info_blob.length + MAC_SIZE);
        let encrypted_license_info = src.read_slice(encrypted_license_info_blob.length).into();
        let mac_data = src.read_slice(MAC_SIZE).into();

        Ok(Self {
            license_header,
            encrypted_license_info,
            mac_data,
        })
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

impl NewLicenseInformation {
    const NAME: &'static str = "NewLicenseInformation";

    const FIXED_PART_SIZE: usize = NEW_LICENSE_INFO_STATIC_FIELDS_SIZE;
}

impl PduEncode for NewLicenseInformation {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.version);

        dst.write_u32(cast_length!("scopeLen", self.scope.len() + UTF8_NULL_TERMINATOR_SIZE)?);
        utils::write_string_to_cursor(dst, &self.scope, CharacterSet::Ansi, true)?;

        dst.write_u32(cast_length!(
            "companyLen",
            self.company_name.len() * 2 + UTF16_NULL_TERMINATOR_SIZE
        )?);
        utils::write_string_to_cursor(dst, &self.company_name, CharacterSet::Unicode, true)?;

        dst.write_u32(cast_length!(
            "produceIdLen",
            self.product_id.len() * 2 + UTF16_NULL_TERMINATOR_SIZE
        )?);
        utils::write_string_to_cursor(dst, &self.product_id, CharacterSet::Unicode, true)?;

        dst.write_u32(cast_length!("licenseInfoLen", self.license_info.len())?);
        dst.write_slice(self.license_info.as_slice());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            + self.scope.len() + UTF8_NULL_TERMINATOR_SIZE
            + self.company_name.len() * 2 // utf16
            + UTF16_NULL_TERMINATOR_SIZE
            + self.product_id.len() * 2 // utf16
            + UTF16_NULL_TERMINATOR_SIZE
            + self.license_info.len()
    }
}

impl<'de> PduDecode<'de> for NewLicenseInformation {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let version = src.read_u32();

        let scope_len: usize = cast_length!("scopeLen", src.read_u32())?;
        ensure_size!(in: src, size: scope_len);
        let scope = utils::decode_string(src.read_slice(scope_len), CharacterSet::Ansi, true)?;

        let company_name_len: usize = cast_length!("companyLen", src.read_u32())?;
        ensure_size!(in: src, size: company_name_len);
        let company_name = utils::decode_string(src.read_slice(company_name_len), CharacterSet::Unicode, true)?;

        let product_id_len: usize = cast_length!("productIdLen", src.read_u32())?;
        ensure_size!(in: src, size: product_id_len);
        let product_id = utils::decode_string(src.read_slice(product_id_len), CharacterSet::Unicode, true)?;

        let license_info_len = cast_length!("licenseInfoLen", src.read_u32())?;
        ensure_size!(in: src, size: license_info_len);
        let license_info = src.read_slice(license_info_len).into();

        Ok(Self {
            version,
            scope,
            company_name,
            product_id,
            license_info,
        })
    }
}
