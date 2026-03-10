pub mod cert;

#[cfg(test)]
mod tests;

use cert::{CertificateType, ProprietaryCertificate, X509CertificateChain};
use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_fixed_part_size,
    ensure_size, invalid_field_err,
};
use ironrdp_str::ansi;
use ironrdp_str::prefixed::CbU32StringNullIncluded;

use super::{
    BLOB_LENGTH_SIZE, BLOB_TYPE_SIZE, BlobHeader, BlobType, KEY_EXCHANGE_ALGORITHM_RSA, LicenseHeader, PreambleType,
    RANDOM_NUMBER_SIZE, ServerLicenseError,
};

const CERT_VERSION_FIELD_SIZE: usize = 4;
const KEY_EXCHANGE_FIELD_SIZE: usize = 4;
const SCOPE_ARRAY_SIZE_FIELD_SIZE: usize = 4;
const PRODUCT_INFO_STATIC_FIELDS_SIZE: usize = 4; // version only; company_name and product_id use decode_owned
// [MS-RDPELE] §2.2.2.1.1 does not specify an explicit cap; 1024 bytes is a conservative bound
// matching the pre-migration validation, protecting against pathological allocations.
const MAX_COMPANY_NAME_LEN: usize = 1024;
const MAX_PRODUCT_ID_LEN: usize = 1024;
const CERT_CHAIN_VERSION_MASK: u32 = 0x7FFF_FFFF;
const CERT_CHAIN_ISSUED_MASK: u32 = 0x8000_0000;
const MAX_SCOPE_COUNT: u32 = 256;

const RSA_EXCHANGE_ALGORITHM: u32 = 1;

/// [2.2.2.1] Server License Request (SERVER_LICENSE_REQUEST)
///
/// [2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpele/e17772e9-9642-4bb6-a2bc-82875dd6da7c
#[derive(Debug, PartialEq, Eq)]
pub struct ServerLicenseRequest {
    pub license_header: LicenseHeader,
    pub server_random: Vec<u8>,
    pub product_info: ProductInfo,
    pub server_certificate: Option<ServerCertificate>,
    pub scope_list: Vec<Scope>,
}

impl ServerLicenseRequest {
    const NAME: &'static str = "ServerLicenseRequest";

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        self.license_header.encode(dst)?;

        dst.write_slice(&self.server_random);
        self.product_info.encode(dst)?;

        BlobHeader::new(BlobType::KEY_EXCHANGE_ALGORITHM, KEY_EXCHANGE_FIELD_SIZE).encode(dst)?;
        dst.write_u32(KEY_EXCHANGE_ALGORITHM_RSA);

        let cert_size = self.server_certificate.as_ref().map(|v| v.size()).unwrap_or(0);
        BlobHeader::new(BlobType::CERTIFICATE, cert_size).encode(dst)?;

        if let Some(cert) = &self.server_certificate {
            cert.encode(dst)?;
        }

        dst.write_u32(cast_length!("listLen", self.scope_list.len())?);

        for scope in self.scope_list.iter() {
            scope.encode(dst)?;
        }

        Ok(())
    }

    pub fn decode(license_header: LicenseHeader, src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        if license_header.preamble_message_type != PreambleType::LicenseRequest {
            return Err(invalid_field_err!("preambleMessageType", "unexpected preamble type"));
        }

        ensure_size!(in: src, size: RANDOM_NUMBER_SIZE);
        let server_random = src.read_slice(RANDOM_NUMBER_SIZE).into();

        let product_info = ProductInfo::decode(src)?;

        let key_exchange_algorithm_blob = BlobHeader::decode(src)?;
        if key_exchange_algorithm_blob.blob_type != BlobType::KEY_EXCHANGE_ALGORITHM {
            return Err(invalid_field_err!("blobType", "invalid blob type"));
        }

        ensure_size!(in: src, size: 4);
        let key_exchange_algorithm = src.read_u32();
        if key_exchange_algorithm != RSA_EXCHANGE_ALGORITHM {
            return Err(invalid_field_err!("keyAlgo", "invalid key exchange algorithm"));
        }

        let cert_blob = BlobHeader::decode(src)?;
        if cert_blob.blob_type != BlobType::CERTIFICATE {
            return Err(invalid_field_err!("blobType", "invalid blob type"));
        }

        // The terminal server can choose not to send the certificate by setting the wblobLen field in the Licensing Binary BLOB structure to 0
        let server_certificate = if cert_blob.length != 0 {
            Some(ServerCertificate::decode(src)?)
        } else {
            None
        };

        ensure_size!(in: src, size: 4);
        let scope_count = src.read_u32();
        if scope_count > MAX_SCOPE_COUNT {
            return Err(invalid_field_err!("scopeCount", "invalid scope count"));
        }

        let mut scope_list = Vec::with_capacity(
            #[expect(clippy::missing_panics_doc, reason = "unreachable panic (checked integer underflow)")]
            usize::try_from(scope_count).expect("scope_count is guaranteed to fit into usize due to the prior check"),
        );

        for _ in 0..scope_count {
            scope_list.push(Scope::decode(src)?);
        }

        Ok(Self {
            license_header,
            server_random,
            product_info,
            server_certificate,
            scope_list,
        })
    }

    pub fn get_public_key(&self) -> Result<Option<Vec<u8>>, ServerLicenseError> {
        self.server_certificate.as_ref().map(|c| c.get_public_key()).transpose()
    }

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn size(&self) -> usize {
        self.license_header.size()
            + RANDOM_NUMBER_SIZE
            + self.product_info.size()
            + BLOB_LENGTH_SIZE * 2 // KeyExchangeBlob + CertificateBlob
            + BLOB_TYPE_SIZE * 2 // KeyExchangeBlob + CertificateBlob
            + KEY_EXCHANGE_FIELD_SIZE
            + self.server_certificate.as_ref().map(|c| c.size()).unwrap_or(0)
            + SCOPE_ARRAY_SIZE_FIELD_SIZE
            + self.scope_list.iter().map(|s| s.size()).sum::<usize>()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Scope(pub String);

impl Scope {
    const NAME: &'static str = "Scope";

    const FIXED_PART_SIZE: usize = BLOB_TYPE_SIZE + BLOB_LENGTH_SIZE;
}

impl Encode for Scope {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        BlobHeader::new(BlobType::SCOPE, ansi::encoded_ansi_len_with_null(&self.0)).encode(dst)?;
        ansi::write_ansi_with_null(dst, &self.0)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + ansi::encoded_ansi_len_with_null(&self.0)
    }
}

impl<'de> Decode<'de> for Scope {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let blob_header = BlobHeader::decode(src)?;
        if blob_header.blob_type != BlobType::SCOPE {
            return Err(invalid_field_err!("blobType", "invalid blob type"));
        }
        if blob_header.length < 1 {
            return Err(invalid_field_err!("blobLen", "blob too small"));
        }
        ensure_size!(in: src, size: blob_header.length);
        let blob_data = src.read_slice(blob_header.length);
        ansi::decode_ansi(blob_data)
            .map(Self)
            .map_err(|_| invalid_field_err!("scope", "scope is not utf8"))
    }
}

/// [2.2.1.4.3.1] Server Certificate (SERVER_CERTIFICATE)
///
/// [2.2.1.4.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/54e72cc6-3422-404c-a6b4-2486db125342
#[derive(Debug, PartialEq, Eq)]
pub struct ServerCertificate {
    pub issued_permanently: bool,
    pub certificate: CertificateType,
}

impl ServerCertificate {
    const NAME: &'static str = "ServerCertificate";

    const FIXED_PART_SIZE: usize = CERT_VERSION_FIELD_SIZE;

    pub fn get_public_key(&self) -> Result<Vec<u8>, ServerLicenseError> {
        use x509_cert::der::Decode as _;

        match &self.certificate {
            CertificateType::Proprietary(certificate) => {
                let public_exponent = certificate.public_key.public_exponent.to_le_bytes();

                let rsa_public_key = pkcs1::RsaPublicKey {
                    modulus: pkcs1::UintRef::new(&certificate.public_key.modulus)?,
                    public_exponent: pkcs1::UintRef::new(&public_exponent)?,
                };

                let public_key = pkcs1::der::Encode::to_der(&rsa_public_key)?;

                Ok(public_key)
            }
            CertificateType::X509(certificate) => {
                let cert_der = certificate
                    .certificate_array
                    .last()
                    .ok_or_else(|| ServerLicenseError::InvalidX509CertificatesAmount)?;

                let cert = x509_cert::Certificate::from_der(cert_der).map_err(|source| {
                    ServerLicenseError::InvalidX509Certificate {
                        source,
                        cert_der: cert_der.clone(),
                    }
                })?;

                let public_key = cert
                    .tbs_certificate
                    .subject_public_key_info
                    .subject_public_key
                    .raw_bytes()
                    .to_owned();

                Ok(public_key)
            }
        }
    }
}

impl Encode for ServerCertificate {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        let cert_version: u32 = match self.certificate {
            CertificateType::Proprietary(_) => 1,
            CertificateType::X509(_) => 2,
        };
        let mask = if self.issued_permanently {
            CERT_CHAIN_ISSUED_MASK
        } else {
            0
        };

        dst.write_u32(cert_version | mask);

        match &self.certificate {
            CertificateType::Proprietary(cert) => cert.encode(dst)?,
            CertificateType::X509(cert) => cert.encode(dst)?,
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let certificate_size = match &self.certificate {
            CertificateType::Proprietary(cert) => cert.size(),
            CertificateType::X509(cert) => cert.size(),
        };

        Self::FIXED_PART_SIZE + certificate_size
    }
}

impl<'de> Decode<'de> for ServerCertificate {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let cert_version = src.read_u32();

        let issued_permanently = cert_version & CERT_CHAIN_ISSUED_MASK == CERT_CHAIN_ISSUED_MASK;

        let certificate = match cert_version & CERT_CHAIN_VERSION_MASK {
            1 => CertificateType::Proprietary(ProprietaryCertificate::decode(src)?),
            2 => CertificateType::X509(X509CertificateChain::decode(src)?),
            _ => return Err(invalid_field_err!("certVersion", "invalid certificate version")),
        };

        Ok(Self {
            issued_permanently,
            certificate,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProductInfo {
    pub version: u32,
    /// Company name ([MS-RDPELE] §2.2.2.1.1 `pbCompanyName`, UTF-16LE, u32 cb prefix including null)
    pub company_name: CbU32StringNullIncluded,
    /// Product ID ([MS-RDPELE] §2.2.2.1.1 `pbProductId`, UTF-16LE, u32 cb prefix including null)
    pub product_id: CbU32StringNullIncluded,
}

impl ProductInfo {
    const NAME: &'static str = "ProductInfo";

    const FIXED_PART_SIZE: usize = PRODUCT_INFO_STATIC_FIELDS_SIZE;
}

impl Encode for ProductInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.version);
        self.company_name.encode(dst)?;
        self.product_id.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        4 // version
            + self.company_name.size()
            + self.product_id.size()
    }
}

impl<'de> Decode<'de> for ProductInfo {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let version = src.read_u32();

        let company_name = CbU32StringNullIncluded::decode_owned_max(src, MAX_COMPANY_NAME_LEN)?;
        let product_id = CbU32StringNullIncluded::decode_owned_max(src, MAX_PRODUCT_ID_LEN)?;

        Ok(Self {
            version,
            company_name,
            product_id,
        })
    }
}
