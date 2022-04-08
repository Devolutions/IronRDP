pub mod cert;
#[cfg(test)]
pub mod test;

use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use x509_parser::parse_x509_certificate;

use cert::{CertificateType, ProprietaryCertificate, X509CertificateChain};

use super::{
    BasicSecurityHeader, BasicSecurityHeaderFlags, BlobHeader, BlobType, LicenseErrorCode,
    LicenseHeader, LicensingErrorMessage, LicensingStateTransition, PreambleFlags, PreambleType,
    PreambleVersion, ServerLicenseError, BLOB_LENGTH_SIZE, BLOB_TYPE_SIZE,
    KEY_EXCHANGE_ALGORITHM_RSA, PREAMBLE_SIZE, RANDOM_NUMBER_SIZE, UTF16_NULL_TERMINATOR_SIZE,
    UTF8_NULL_TERMINATOR_SIZE,
};
use crate::{utils, PduParsing};

const CERT_VERSION_FIELD_SIZE: usize = 4;
const KEY_EXCHANGE_FIELD_SIZE: usize = 4;
const SCOPE_ARRAY_SIZE_FIELD_SIZE: usize = 4;
const PRODUCT_INFO_STATIC_FIELDS_SIZE: usize = 12;
const CERT_CHAIN_VERSION_MASK: u32 = 0x7FFF_FFFF;
const CERT_CHAIN_ISSUED_MASK: u32 = 0x8000_0000;
const MAX_SCOPE_COUNT: u32 = 256;
const MAX_COMPANY_NAME_LEN: u32 = 1024;
const MAX_PRODUCT_ID_LEN: u32 = 1024;

const RSA_EXCHANGE_ALGORITHM: u32 = 1;

#[derive(Debug, PartialEq)]
pub enum InitialMessageType {
    LicenseRequest(ServerLicenseRequest),
    StatusValidClient(LicensingErrorMessage),
}

#[derive(Debug, PartialEq)]
pub struct InitialServerLicenseMessage {
    pub license_header: LicenseHeader,
    pub message_type: InitialMessageType,
}

impl InitialServerLicenseMessage {
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
                preamble_message_size: (PREAMBLE_SIZE + valid_client_message.buffer_length())
                    as u16,
            },
            message_type: InitialMessageType::StatusValidClient(valid_client_message),
        }
    }
}

impl PduParsing for InitialServerLicenseMessage {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let license_header = LicenseHeader::from_buffer(&mut stream)?;
        if license_header.preamble_message_type != PreambleType::LicenseRequest
            && license_header.preamble_message_type != PreambleType::ErrorAlert
        {
            return Err(ServerLicenseError::InvalidPreamble(format!(
                "Got {:?} but expected {:?} or {:?}",
                license_header.preamble_message_type,
                PreambleType::LicenseRequest,
                PreambleType::ErrorAlert
            )));
        }

        match license_header.preamble_message_type {
            PreambleType::LicenseRequest => {
                let license_request = ServerLicenseRequest::from_buffer(&mut stream)?;

                Ok(Self {
                    license_header,
                    message_type: InitialMessageType::LicenseRequest(license_request),
                })
            }
            PreambleType::ErrorAlert => {
                let license_error = LicensingErrorMessage::from_buffer(&mut stream)?;

                if license_error.error_code == LicenseErrorCode::StatusValidClient
                    && license_error.state_transition == LicensingStateTransition::NoTransition
                {
                    Ok(Self {
                        license_header,
                        message_type: InitialMessageType::StatusValidClient(license_error),
                    })
                } else {
                    Err(ServerLicenseError::UnexpectedError(license_error))
                }
            }
            _ => Err(ServerLicenseError::UnexpectedLicenseMessage),
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        self.license_header.to_buffer(&mut stream)?;

        match &self.message_type {
            InitialMessageType::LicenseRequest(license_request) => {
                license_request.to_buffer(&mut stream)?;
            }
            InitialMessageType::StatusValidClient(valid_client) => {
                valid_client.to_buffer(&mut stream)?;
            }
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        self.license_header.buffer_length()
            + match &self.message_type {
                InitialMessageType::LicenseRequest(license_request) => {
                    license_request.buffer_length()
                }
                InitialMessageType::StatusValidClient(valid_client) => valid_client.buffer_length(),
            }
    }
}

#[derive(Debug, PartialEq)]
pub struct ServerLicenseRequest {
    pub server_random: Vec<u8>,
    pub product_info: ProductInfo,
    pub server_certificate: Option<ServerCertificate>,
    pub scope_list: Vec<Scope>,
}

impl ServerLicenseRequest {
    pub fn get_public_key(&self) -> Result<Option<Vec<u8>>, ServerLicenseError> {
        self.server_certificate
            .as_ref()
            .map(|c| c.get_public_key())
            .transpose()
    }
}

impl PduParsing for ServerLicenseRequest {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let mut server_random = vec![0u8; RANDOM_NUMBER_SIZE];
        stream.read_exact(&mut server_random)?;

        let product_info = ProductInfo::from_buffer(&mut stream)?;

        let _key_exchange_algorithm_blob =
            BlobHeader::read_from_buffer(BlobType::KeyExchangeAlgorithm, &mut stream)?;

        let key_exchange_algorithm = stream.read_u32::<LittleEndian>()?;
        if key_exchange_algorithm != RSA_EXCHANGE_ALGORITHM {
            return Err(ServerLicenseError::InvalidKeyExchangeAlgorithm);
        }

        let cert_blob = BlobHeader::read_from_buffer(BlobType::Certificate, &mut stream)?;

        // The terminal server can choose not to send the certificate by setting the wblobLen field in the Licensing Binary BLOB structure to 0
        let server_certificate = if cert_blob.length != 0 {
            Some(ServerCertificate::from_buffer(&mut stream)?)
        } else {
            None
        };

        let scope_count = stream.read_u32::<LittleEndian>()?;
        if scope_count > MAX_SCOPE_COUNT {
            return Err(ServerLicenseError::InvalidScopeCount(scope_count));
        }

        let mut scope_list = Vec::with_capacity(scope_count as usize);

        for _ in 0..scope_count {
            scope_list.push(Scope::from_buffer(&mut stream)?);
        }

        Ok(Self {
            server_random,
            product_info,
            server_certificate,
            scope_list,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_all(&self.server_random)?;
        self.product_info.to_buffer(&mut stream)?;

        BlobHeader::new(BlobType::KeyExchangeAlgorithm, KEY_EXCHANGE_FIELD_SIZE)
            .write_to_buffer(&mut stream)?;
        stream.write_u32::<LittleEndian>(KEY_EXCHANGE_ALGORITHM_RSA)?;

        let cert_size = self
            .server_certificate
            .as_ref()
            .map(|v| v.buffer_length())
            .unwrap_or(0);
        BlobHeader::new(BlobType::Certificate, cert_size).write_to_buffer(&mut stream)?;

        if let Some(cert) = &self.server_certificate {
            cert.to_buffer(&mut stream)?;
        }

        stream.write_u32::<LittleEndian>(self.scope_list.len() as u32)?;

        for scope in self.scope_list.iter() {
            scope.to_buffer(&mut stream)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        RANDOM_NUMBER_SIZE
            + self.product_info.buffer_length()
            + BLOB_LENGTH_SIZE * 2 // KeyExchangeBlob + CertificateBlob
            + BLOB_TYPE_SIZE * 2 // KeyExchangeBlob + CertificateBlob
            + KEY_EXCHANGE_FIELD_SIZE
            + self.server_certificate.as_ref().map(|c| c.buffer_length()).unwrap_or(0)
            + SCOPE_ARRAY_SIZE_FIELD_SIZE
            + self.scope_list.iter().map(|s| s.buffer_length()).sum::<usize>()
    }
}

#[derive(Debug, PartialEq)]
pub struct Scope(pub String);

impl PduParsing for Scope {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let blob_header = BlobHeader::read_from_buffer(BlobType::Scope, &mut stream)?;
        let mut blob_data = vec![0u8; blob_header.length];
        stream.read_exact(&mut blob_data)?;
        blob_data.resize(blob_data.len() - UTF8_NULL_TERMINATOR_SIZE, 0);

        if let Ok(data) = std::str::from_utf8(&blob_data) {
            Ok(Self(String::from(data)))
        } else {
            Err(ServerLicenseError::IOError(io::Error::new(
                io::ErrorKind::InvalidData,
                "scope is not utf8",
            )))
        }
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let data_size = self.0.len() + UTF8_NULL_TERMINATOR_SIZE;
        BlobHeader::new(BlobType::Scope, data_size).write_to_buffer(&mut stream)?;
        stream.write_all(self.0.as_bytes())?;
        stream.write_u8(0)?; // null terminator

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        BLOB_TYPE_SIZE + BLOB_LENGTH_SIZE + self.0.len() + UTF8_NULL_TERMINATOR_SIZE
    }
}

#[derive(Debug, PartialEq)]
pub struct ServerCertificate {
    pub issued_permanently: bool,
    pub certificate: CertificateType,
}

impl ServerCertificate {
    pub fn get_public_key(&self) -> Result<Vec<u8>, ServerLicenseError> {
        match &self.certificate {
            CertificateType::Proprietary(certificate) => {
                let mut public_key_buffer =
                    Vec::with_capacity(certificate.public_key.buffer_length());
                certificate.public_key.to_buffer(&mut public_key_buffer)?;
                Ok(public_key_buffer)
            }
            CertificateType::X509(certificate) => {
                if let Ok((_, tbs)) = parse_x509_certificate(
                    &certificate.certificate_array[certificate.certificate_array.len() - 1]
                        .as_slice(),
                ) {
                    Ok(Vec::from(
                        tbs.tbs_certificate.subject_pki.subject_public_key.data,
                    ))
                } else {
                    Err(ServerLicenseError::InvalidX509Certificate)
                }
            }
        }
    }
}

impl PduParsing for ServerCertificate {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let cert_version = stream.read_u32::<LittleEndian>()?;

        let issued_permanently = cert_version & CERT_CHAIN_ISSUED_MASK == CERT_CHAIN_ISSUED_MASK;

        let certificate = match cert_version & CERT_CHAIN_VERSION_MASK {
            1 => CertificateType::Proprietary(ProprietaryCertificate::from_buffer(&mut stream)?),
            2 => CertificateType::X509(X509CertificateChain::from_buffer(&mut stream)?),
            _ => return Err(ServerLicenseError::InvalidCertificateVersion),
        };

        Ok(Self {
            issued_permanently,
            certificate,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let cert_version: u32 = match self.certificate {
            CertificateType::Proprietary(_) => 1,
            CertificateType::X509(_) => 2,
        };
        let mask = if self.issued_permanently {
            CERT_CHAIN_ISSUED_MASK
        } else {
            0
        };

        stream.write_u32::<LittleEndian>(cert_version | mask)?;

        match &self.certificate {
            CertificateType::Proprietary(cert) => cert.to_buffer(&mut stream)?,
            CertificateType::X509(cert) => cert.to_buffer(&mut stream)?,
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let certificate_size = match &self.certificate {
            CertificateType::Proprietary(cert) => cert.buffer_length(),
            CertificateType::X509(cert) => cert.buffer_length(),
        };

        CERT_VERSION_FIELD_SIZE + certificate_size
    }
}

#[derive(Debug, PartialEq)]
pub struct ProductInfo {
    pub version: u32,
    pub company_name: String,
    pub product_id: String,
}

impl PduParsing for ProductInfo {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let version = stream.read_u32::<LittleEndian>()?;

        let company_name_len = stream.read_u32::<LittleEndian>()?;
        if company_name_len < 2 || company_name_len > MAX_COMPANY_NAME_LEN {
            return Err(ServerLicenseError::InvalidCompanyNameLength(
                company_name_len,
            ));
        }

        let mut company_name = vec![0u8; company_name_len as usize];
        stream.read_exact(&mut company_name)?;

        company_name.resize((company_name_len - 2) as usize, 0);
        let company_name = utils::bytes_to_utf16_string(&company_name.as_slice());

        let product_id_len = stream.read_u32::<LittleEndian>()?;
        if product_id_len < 2 || product_id_len > MAX_PRODUCT_ID_LEN {
            return Err(ServerLicenseError::InvalidProductIdLength(product_id_len));
        }

        let mut product_id = vec![0u8; product_id_len as usize];
        stream.read_exact(&mut product_id)?;

        product_id.resize((product_id_len - 2) as usize, 0);
        let product_id = utils::bytes_to_utf16_string(&product_id.as_slice());

        Ok(Self {
            version,
            company_name,
            product_id,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.version)?;

        let mut company_name = utils::string_to_utf16(&self.company_name);
        company_name.resize(company_name.len() + 2, 0);

        stream.write_u32::<LittleEndian>(company_name.len() as u32)?;
        stream.write_all(&company_name)?;

        let mut product_id = utils::string_to_utf16(&self.product_id);
        product_id.resize(product_id.len() + 2, 0);

        stream.write_u32::<LittleEndian>(product_id.len() as u32)?;
        stream.write_all(&product_id)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let company_name_utf_16 = utils::string_to_utf16(&self.company_name);
        let product_id_utf_16 = utils::string_to_utf16(&self.product_id);

        PRODUCT_INFO_STATIC_FIELDS_SIZE
            + company_name_utf_16.len()
            + UTF16_NULL_TERMINATOR_SIZE
            + product_id_utf_16.len()
            + UTF16_NULL_TERMINATOR_SIZE
    }
}
