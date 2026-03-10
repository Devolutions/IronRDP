#[cfg(test)]
mod tests;

use ironrdp_core::{
    Decode, DecodeOwned as _, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length,
    ensure_fixed_part_size, ensure_size, invalid_field_err,
};
use ironrdp_str::ansi;
use ironrdp_str::prefixed::CbU32StringNullIncluded;

use super::{
    BLOB_LENGTH_SIZE, BLOB_TYPE_SIZE, BlobHeader, BlobType, LicenseEncryptionData, LicenseHeader, MAC_SIZE,
    PreambleType, ServerLicenseError,
};
use crate::crypto::rc4::Rc4;

const LICENSE_INFO_STATIC_FIELDS_SIZE: usize = 8; // version(4) + scope_len(4); the rest use decode_owned

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
    const NAME: &'static str = "ServerUpgradeLicense";

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.license_header.encode(dst)?;
        BlobHeader::new(BlobType::ENCRYPTED_DATA, self.encrypted_license_info.len()).encode(dst)?;
        dst.write_slice(&self.encrypted_license_info);
        dst.write_slice(&self.mac_data);

        Ok(())
    }

    pub fn decode(license_header: LicenseHeader, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        if license_header.preamble_message_type != PreambleType::UpgradeLicense
            && license_header.preamble_message_type != PreambleType::NewLicense
        {
            return Err(invalid_field_err!(
                "preambleType",
                "got unexpected message preamble type"
            ));
        }

        let encrypted_license_info_blob = BlobHeader::decode(src)?;
        if encrypted_license_info_blob.blob_type != BlobType::ENCRYPTED_DATA {
            return Err(invalid_field_err!("blobType", "unexpected blob type"));
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

    pub fn verify_server_license(&self, encryption_data: &LicenseEncryptionData) -> Result<(), ServerLicenseError> {
        let decrypted_license_info = self.decrypted_license_info(encryption_data);
        let mac_data =
            super::compute_mac_data(encryption_data.mac_salt_key.as_slice(), decrypted_license_info.as_ref())?;

        if mac_data != self.mac_data {
            return Err(ServerLicenseError::InvalidMacData);
        }

        Ok(())
    }

    pub fn new_license_info(&self, encryption_data: &LicenseEncryptionData) -> DecodeResult<LicenseInformation> {
        let data = self.decrypted_license_info(encryption_data);
        LicenseInformation::decode(&mut ReadCursor::new(&data))
    }

    fn decrypted_license_info(&self, encryption_data: &LicenseEncryptionData) -> Vec<u8> {
        let mut rc4 = Rc4::new(encryption_data.license_key.as_slice());
        rc4.process(self.encrypted_license_info.as_slice())
    }

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn size(&self) -> usize {
        self.license_header.size() + BLOB_LENGTH_SIZE + BLOB_TYPE_SIZE + self.encrypted_license_info.len() + MAC_SIZE
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct LicenseInformation {
    pub version: u32,
    /// Scope identifier (ANSI/ASCII string)
    pub scope: String,
    /// Company name ([MS-RDPELE] §2.2.2.6.1 `pbCompanyName`, UTF-16LE, u32 cb prefix including null)
    pub company_name: CbU32StringNullIncluded,
    /// Product ID ([MS-RDPELE] §2.2.2.6.1 `pbProductId`, UTF-16LE, u32 cb prefix including null)
    pub product_id: CbU32StringNullIncluded,
    pub license_info: Vec<u8>,
}

impl LicenseInformation {
    const NAME: &'static str = "LicenseInformation";

    const FIXED_PART_SIZE: usize = LICENSE_INFO_STATIC_FIELDS_SIZE;
}

impl Encode for LicenseInformation {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.version);

        dst.write_u32(cast_length!("scopeLen", ansi::encoded_ansi_len_with_null(&self.scope))?);
        ansi::write_ansi_with_null(dst, &self.scope)?;

        self.company_name.encode(dst)?;
        self.product_id.encode(dst)?;

        dst.write_u32(cast_length!("licenseInfoLen", self.license_info.len())?);
        dst.write_slice(self.license_info.as_slice());

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        4 // version
            + 4 + ansi::encoded_ansi_len_with_null(&self.scope) // scopeLen(u32) + scope + null
            + self.company_name.size()
            + self.product_id.size()
            + 4 + self.license_info.len() // licenseInfoLen(u32) + data
    }
}

impl<'de> Decode<'de> for LicenseInformation {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let version = src.read_u32();

        let scope_len: usize = cast_length!("scopeLen", src.read_u32())?;
        ensure_size!(in: src, size: scope_len);
        let scope_bytes = src.read_slice(scope_len);
        let scope =
            ansi::decode_ansi(scope_bytes).map_err(|_| invalid_field_err!("scope", "invalid UTF-8 in scope"))?;

        let company_name = CbU32StringNullIncluded::decode_owned(src)?;
        let product_id = CbU32StringNullIncluded::decode_owned(src)?;

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
