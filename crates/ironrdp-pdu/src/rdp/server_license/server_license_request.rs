pub mod cert;

#[cfg(test)]
mod tests;

use cert::{CertificateType, ProprietaryCertificate, X509CertificateChain};

use super::{
    BasicSecurityHeader, BasicSecurityHeaderFlags, BlobHeader, BlobType, LicenseErrorCode, LicenseHeader,
    LicensingErrorMessage, LicensingStateTransition, PreambleFlags, PreambleType, PreambleVersion, ServerLicenseError,
    BLOB_LENGTH_SIZE, BLOB_TYPE_SIZE, KEY_EXCHANGE_ALGORITHM_RSA, PREAMBLE_SIZE, RANDOM_NUMBER_SIZE,
    UTF16_NULL_TERMINATOR_SIZE, UTF8_NULL_TERMINATOR_SIZE,
};
use crate::{
    cursor::{ReadCursor, WriteCursor},
    utils, PduDecode, PduEncode, PduResult,
};

const CERT_VERSION_FIELD_SIZE: usize = 4;
const KEY_EXCHANGE_FIELD_SIZE: usize = 4;
const SCOPE_ARRAY_SIZE_FIELD_SIZE: usize = 4;
const PRODUCT_INFO_STATIC_FIELDS_SIZE: usize = 12;
const CERT_CHAIN_VERSION_MASK: u32 = 0x7FFF_FFFF;
const CERT_CHAIN_ISSUED_MASK: u32 = 0x8000_0000;
const MAX_SCOPE_COUNT: u32 = 256;
const MAX_COMPANY_NAME_LEN: usize = 1024;
const MAX_PRODUCT_ID_LEN: usize = 1024;

const RSA_EXCHANGE_ALGORITHM: u32 = 1;

#[derive(Debug, PartialEq, Eq)]
pub enum InitialMessageType {
    LicenseRequest(ServerLicenseRequest),
    StatusValidClient(LicensingErrorMessage),
}

// FIXME(#269): this is a helper structure which tries to detect if a
// SERVER_LICENSE_REQUEST PDU is received from the server, or if a
// STATUS_VALID_CLIENT error code is received instead (no need to negotiate
// a license). I think this could be refactored into a more generic struct / enum,
// without trying to be too smart by, e.g., returning errors when a LICENSE_ERROR_MESSAGE
// is received depending on the error code. Parsing code should lend the data received
// from the network without making too much decisions.

/// Either a SERVER_LICENSE_REQUEST or a LICENSE_ERROR_MESSAGE with the STATUS_VALID_CLIENT code
#[derive(Debug, PartialEq, Eq)]
pub struct InitialServerLicenseMessage {
    pub license_header: LicenseHeader,
    pub message_type: InitialMessageType,
}

impl InitialServerLicenseMessage {
    const NAME: &'static str = "InitialServerLicenseMessage";

    pub fn new_status_valid_client_message() -> Self {
        let valid_client_message = LicensingErrorMessage {
            error_code: LicenseErrorCode::StatusValidClient,
            state_transition: LicensingStateTransition::NoTransition,
            error_info: Vec::new(),
        };

        Self {
            license_header: LicenseHeader {
                security_header: BasicSecurityHeader {
                    flags: BasicSecurityHeaderFlags::LICENSE_PKT,
                },
                preamble_message_type: PreambleType::ErrorAlert,
                preamble_flags: PreambleFlags::empty(),
                preamble_version: PreambleVersion::V3,
                preamble_message_size: (PREAMBLE_SIZE + valid_client_message.size()) as u16,
            },
            message_type: InitialMessageType::StatusValidClient(valid_client_message),
        }
    }
}

impl PduEncode for InitialServerLicenseMessage {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        self.license_header.encode(dst)?;

        match &self.message_type {
            InitialMessageType::LicenseRequest(license_request) => {
                license_request.encode(dst)?;
            }
            InitialMessageType::StatusValidClient(valid_client) => {
                valid_client.encode(dst)?;
            }
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        self.license_header.size()
            + match &self.message_type {
                InitialMessageType::LicenseRequest(license_request) => license_request.size(),
                InitialMessageType::StatusValidClient(valid_client) => valid_client.size(),
            }
    }
}

impl<'de> PduDecode<'de> for InitialServerLicenseMessage {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let license_header = LicenseHeader::decode(src)?;

        match license_header.preamble_message_type {
            PreambleType::LicenseRequest => {
                let license_request = ServerLicenseRequest::decode(src)?;

                Ok(Self {
                    license_header,
                    message_type: InitialMessageType::LicenseRequest(license_request),
                })
            }
            PreambleType::ErrorAlert => {
                let license_error = LicensingErrorMessage::decode(src)?;

                if license_error.error_code == LicenseErrorCode::StatusValidClient
                    && license_error.state_transition == LicensingStateTransition::NoTransition
                {
                    Ok(Self {
                        license_header,
                        message_type: InitialMessageType::StatusValidClient(license_error),
                    })
                } else {
                    Err(invalid_message_err!("errorCode", "unexpected error"))
                }
            }
            _ => Err(invalid_message_err!("preambleMessageType", "invalid preamble")),
        }
    }
}

// impl_pdu_parsing_max!(InitialServerLicenseMessage);

/// [2.2.2.1] Server License Request (SERVER_LICENSE_REQUEST)
///
/// [2.2.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpele/e17772e9-9642-4bb6-a2bc-82875dd6da7c
#[derive(Debug, PartialEq, Eq)]
pub struct ServerLicenseRequest {
    pub server_random: Vec<u8>,
    pub product_info: ProductInfo,
    pub server_certificate: Option<ServerCertificate>,
    pub scope_list: Vec<Scope>,
}

impl ServerLicenseRequest {
    const NAME: &'static str = "ServerLicenseRequest";

    pub fn get_public_key(&self) -> Result<Option<Vec<u8>>, ServerLicenseError> {
        self.server_certificate.as_ref().map(|c| c.get_public_key()).transpose()
    }
}

impl PduEncode for ServerLicenseRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_slice(&self.server_random);
        self.product_info.encode(dst)?;

        BlobHeader::new(BlobType::KeyExchangeAlgorithm, KEY_EXCHANGE_FIELD_SIZE).encode(dst)?;
        dst.write_u32(KEY_EXCHANGE_ALGORITHM_RSA);

        let cert_size = self.server_certificate.as_ref().map(|v| v.size()).unwrap_or(0);
        BlobHeader::new(BlobType::Certificate, cert_size).encode(dst)?;

        if let Some(cert) = &self.server_certificate {
            cert.encode(dst)?;
        }

        dst.write_u32(cast_length!("listLen", self.scope_list.len())?);

        for scope in self.scope_list.iter() {
            scope.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        RANDOM_NUMBER_SIZE
            + self.product_info.size()
            + BLOB_LENGTH_SIZE * 2 // KeyExchangeBlob + CertificateBlob
            + BLOB_TYPE_SIZE * 2 // KeyExchangeBlob + CertificateBlob
            + KEY_EXCHANGE_FIELD_SIZE
            + self.server_certificate.as_ref().map(|c| c.size()).unwrap_or(0)
            + SCOPE_ARRAY_SIZE_FIELD_SIZE
            + self.scope_list.iter().map(|s| s.size()).sum::<usize>()
    }
}

impl<'de> PduDecode<'de> for ServerLicenseRequest {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_size!(in: src, size: RANDOM_NUMBER_SIZE);
        let server_random = src.read_slice(RANDOM_NUMBER_SIZE).into();

        let product_info = ProductInfo::decode(src)?;

        let key_exchange_algorithm_blob = BlobHeader::decode(src)?;
        if key_exchange_algorithm_blob.blob_type != BlobType::KeyExchangeAlgorithm {
            return Err(invalid_message_err!("blobType", "invalid blob type"));
        }

        ensure_size!(in: src, size: 4);
        let key_exchange_algorithm = src.read_u32();
        if key_exchange_algorithm != RSA_EXCHANGE_ALGORITHM {
            return Err(invalid_message_err!("keyAlgo", "invalid key exchange algorithm"));
        }

        let cert_blob = BlobHeader::decode(src)?;
        if cert_blob.blob_type != BlobType::Certificate {
            return Err(invalid_message_err!("blobType", "invalid blob type"));
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
            return Err(invalid_message_err!("scopeCount", "invalid scope count"));
        }

        let mut scope_list = Vec::with_capacity(scope_count as usize);

        for _ in 0..scope_count {
            scope_list.push(Scope::decode(src)?);
        }

        Ok(Self {
            server_random,
            product_info,
            server_certificate,
            scope_list,
        })
    }
}

impl_pdu_parsing_max!(ServerLicenseRequest);

#[derive(Debug, PartialEq, Eq)]
pub struct Scope(pub String);

impl Scope {
    const NAME: &'static str = "Scope";

    const FIXED_PART_SIZE: usize = BLOB_TYPE_SIZE + BLOB_LENGTH_SIZE;
}

impl PduEncode for Scope {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        let data_size = self.0.len() + UTF8_NULL_TERMINATOR_SIZE;
        BlobHeader::new(BlobType::Scope, data_size).encode(dst)?;
        dst.write_slice(self.0.as_bytes());
        dst.write_u8(0); // null terminator

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.0.len() + UTF8_NULL_TERMINATOR_SIZE
    }
}

impl<'de> PduDecode<'de> for Scope {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        let blob_header = BlobHeader::decode(src)?;
        if blob_header.blob_type != BlobType::Scope {
            return Err(invalid_message_err!("blobType", "invalid blob type"));
        }
        if blob_header.length < UTF8_NULL_TERMINATOR_SIZE {
            return Err(invalid_message_err!("blobLen", "blob too small"));
        }
        ensure_size!(in: src, size: blob_header.length);
        let mut blob_data = src.read_slice(blob_header.length).to_vec();
        blob_data.resize(blob_data.len() - UTF8_NULL_TERMINATOR_SIZE, 0);

        if let Ok(data) = std::str::from_utf8(&blob_data) {
            Ok(Self(String::from(data)))
        } else {
            Err(invalid_message_err!("scope", "scope is not utf8"))
        }
    }
}

impl_pdu_parsing!(Scope);

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
                    modulus: pkcs1::UintRef::new(&certificate.public_key.modulus).unwrap(),
                    public_exponent: pkcs1::UintRef::new(&public_exponent).unwrap(),
                };

                let public_key = pkcs1::der::Encode::to_der(&rsa_public_key).unwrap();

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

impl PduEncode for ServerCertificate {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
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

impl<'de> PduDecode<'de> for ServerCertificate {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let cert_version = src.read_u32();

        let issued_permanently = cert_version & CERT_CHAIN_ISSUED_MASK == CERT_CHAIN_ISSUED_MASK;

        let certificate = match cert_version & CERT_CHAIN_VERSION_MASK {
            1 => CertificateType::Proprietary(ProprietaryCertificate::decode(src)?),
            2 => CertificateType::X509(X509CertificateChain::decode(src)?),
            _ => return Err(invalid_message_err!("certVersion", "invalid certificate version")),
        };

        Ok(Self {
            issued_permanently,
            certificate,
        })
    }
}

impl_pdu_parsing!(ServerCertificate);

#[derive(Debug, PartialEq, Eq)]
pub struct ProductInfo {
    pub version: u32,
    pub company_name: String,
    pub product_id: String,
}

impl ProductInfo {
    const NAME: &'static str = "ProductInfo";

    const FIXED_PART_SIZE: usize = PRODUCT_INFO_STATIC_FIELDS_SIZE;
}

impl PduEncode for ProductInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.version);

        let mut company_name = utils::to_utf16_bytes(&self.company_name);
        company_name.resize(company_name.len() + 2, 0);

        dst.write_u32(cast_length!("companyLen", company_name.len())?);
        dst.write_slice(&company_name);

        let mut product_id = utils::to_utf16_bytes(&self.product_id);
        product_id.resize(product_id.len() + 2, 0);

        dst.write_u32(cast_length!("produceLen", product_id.len())?);
        dst.write_slice(&product_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let company_name_utf_16 = utils::to_utf16_bytes(&self.company_name);
        let product_id_utf_16 = utils::to_utf16_bytes(&self.product_id);

        Self::FIXED_PART_SIZE
            + company_name_utf_16.len()
            + UTF16_NULL_TERMINATOR_SIZE
            + product_id_utf_16.len()
            + UTF16_NULL_TERMINATOR_SIZE
    }
}

impl<'de> PduDecode<'de> for ProductInfo {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let version = src.read_u32();

        let company_name_len = cast_length!("companyLen", src.read_u32())?;
        if !(2..=MAX_COMPANY_NAME_LEN).contains(&company_name_len) {
            return Err(invalid_message_err!("companyLen", "invalid company name length"));
        }

        ensure_size!(in: src, size: company_name_len);
        let mut company_name = src.read_slice(company_name_len).to_vec();
        company_name.resize(company_name_len - 2, 0);
        let company_name = utils::from_utf16_bytes(company_name.as_slice());

        ensure_size!(in: src, size: 4);
        let product_id_len = cast_length!("productIdLen", src.read_u32())?;
        if !(2..=MAX_PRODUCT_ID_LEN).contains(&product_id_len) {
            return Err(invalid_message_err!("productIdLen", "invalid produce ID length"));
        }

        ensure_size!(in: src, size: product_id_len);
        let mut product_id = src.read_slice(product_id_len).to_vec();
        product_id.resize(product_id_len - 2, 0);
        let product_id = utils::from_utf16_bytes(product_id.as_slice());

        Ok(Self {
            version,
            company_name,
            product_id,
        })
    }
}

impl_pdu_parsing_max!(ProductInfo);
