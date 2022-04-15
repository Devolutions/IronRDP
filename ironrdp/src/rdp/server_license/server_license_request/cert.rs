use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};

use super::{BlobHeader, BlobType, ServerLicenseError, KEY_EXCHANGE_ALGORITHM_RSA};
use crate::PduParsing;

pub const SIGNATURE_ALGORITHM_RSA: u32 = 1;
pub const PROP_CERT_NO_BLOBS_SIZE: usize = 8;
pub const PROP_CERT_BLOBS_HEADERS_SIZE: usize = 8;
pub const X509_CERT_LENGTH_FIELD_SIZE: usize = 4;
pub const X509_CERT_COUNT: usize = 4;
pub const RSA_KEY_PADDING_LENGTH: u32 = 8;
pub const RSA_SENTINEL: u32 = 0x3141_5352;
pub const RSA_KEY_SIZE_WITHOUT_MODULUS: usize = 20;

const MIN_CERTIFICATE_AMOUNT: u32 = 2;
const MAX_CERTIFICATE_AMOUNT: u32 = 200;
const MAX_CERTIFICATE_LEN: u32 = 4096;

#[derive(Debug, PartialEq)]
pub enum CertificateType {
    Proprietary(ProprietaryCertificate),
    X509(X509CertificateChain),
}

#[derive(Debug, PartialEq)]
pub struct X509CertificateChain {
    pub certificate_array: Vec<Vec<u8>>,
}

impl PduParsing for X509CertificateChain {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let certificate_count = stream.read_u32::<LittleEndian>()?;
        if !(MIN_CERTIFICATE_AMOUNT..MAX_CERTIFICATE_AMOUNT).contains(&certificate_count) {
            return Err(ServerLicenseError::InvalidX509CertificatesAmount);
        }

        let certificate_array: Vec<_> = (0..certificate_count)
            .map(|_| {
                let certificate_len = stream.read_u32::<LittleEndian>()?;
                if certificate_len > MAX_CERTIFICATE_LEN {
                    return Err(ServerLicenseError::InvalidCertificateLength(
                        certificate_len,
                    ));
                }

                let mut certificate = vec![0u8; certificate_len as usize];
                stream.read_exact(&mut certificate)?;

                Ok(certificate)
            })
            .collect::<Result<_, Self::Error>>()?;

        let mut padding = vec![0u8; (8 + 4 * certificate_count) as usize]; // MSDN: A byte array of the length 8 + 4*NumCertBlobs
        stream.read_exact(&mut padding)?;

        Ok(Self { certificate_array })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(self.certificate_array.len() as u32)?;

        for certificate in &self.certificate_array {
            stream.write_u32::<LittleEndian>(certificate.len() as u32)?;
            stream.write_all(certificate)?;
        }

        let padding_len = (8 + 4 * self.certificate_array.len()) as usize; // MSDN: A byte array of the length 8 + 4*NumCertBlobs
        let padding = vec![0u8; padding_len];
        stream.write_all(padding.as_slice())?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let certificates_length: usize = self
            .certificate_array
            .iter()
            .map(|certificate| certificate.len() + X509_CERT_LENGTH_FIELD_SIZE)
            .sum();
        let padding: usize = 8 + 4 * self.certificate_array.len();
        X509_CERT_COUNT + certificates_length + padding
    }
}

#[derive(Debug, PartialEq)]
pub struct ProprietaryCertificate {
    pub public_key: RsaPublicKey,
    pub signature: Vec<u8>,
}

impl PduParsing for ProprietaryCertificate {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let signature_algorithm_id = stream.read_u32::<LittleEndian>()?;
        if signature_algorithm_id != SIGNATURE_ALGORITHM_RSA {
            return Err(ServerLicenseError::InvalidPropCertSignatureAlgorithmId);
        }

        let key_algorithm_id = stream.read_u32::<LittleEndian>()?;
        if key_algorithm_id != KEY_EXCHANGE_ALGORITHM_RSA {
            return Err(ServerLicenseError::InvalidPropCertKeyAlgorithmId);
        }

        let _key_blob_header = BlobHeader::read_from_buffer(BlobType::RsaKey, &mut stream)?;
        let public_key = RsaPublicKey::from_buffer(&mut stream)?;

        let sig_blob_header = BlobHeader::read_from_buffer(BlobType::RsaSignature, &mut stream)?;
        let mut signature = vec![0u8; sig_blob_header.length];
        stream.read_exact(&mut signature)?;

        Ok(Self {
            public_key,
            signature,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u32::<LittleEndian>(SIGNATURE_ALGORITHM_RSA as u32)?;
        stream.write_u32::<LittleEndian>(KEY_EXCHANGE_ALGORITHM_RSA as u32)?;

        BlobHeader::new(BlobType::RsaKey, self.public_key.buffer_length())
            .write_to_buffer(&mut stream)?;
        self.public_key.to_buffer(&mut stream)?;

        BlobHeader::new(BlobType::RsaSignature, self.signature.len())
            .write_to_buffer(&mut stream)?;
        stream.write_all(&self.signature)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        PROP_CERT_BLOBS_HEADERS_SIZE
            + PROP_CERT_NO_BLOBS_SIZE
            + self.public_key.buffer_length()
            + self.signature.len()
    }
}

#[derive(PartialEq, Debug, Clone)]
pub struct RsaPublicKey {
    pub public_exponent: u32,
    pub modulus: Vec<u8>,
}

impl PduParsing for RsaPublicKey {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let magic = stream.read_u32::<LittleEndian>()?;
        if magic != RSA_SENTINEL {
            return Err(ServerLicenseError::InvalidRsaPublicKeyMagic);
        }

        let keylen = stream.read_u32::<LittleEndian>()?;

        let bitlen = stream.read_u32::<LittleEndian>()?;
        if keylen != (bitlen / 8) + 8 {
            return Err(ServerLicenseError::InvalidRsaPublicKeyLength);
        }

        let datalen = stream.read_u32::<LittleEndian>()?;
        if datalen != (bitlen / 8) - 1 {
            return Err(ServerLicenseError::InvalidRsaPublicKeyDataLength);
        }

        let public_exponent = stream.read_u32::<LittleEndian>()?;

        let mut modulus = vec![0u8; keylen as usize];
        stream.read_exact(&mut modulus)?;

        Ok(Self {
            public_exponent,
            modulus,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let keylen = self.modulus.len() as u32;
        let bitlen = (keylen - RSA_KEY_PADDING_LENGTH) * 8;
        let datalen = bitlen / 8 - 1;

        stream.write_u32::<LittleEndian>(RSA_SENTINEL)?; // magic
        stream.write_u32::<LittleEndian>(keylen)?;
        stream.write_u32::<LittleEndian>(bitlen)?;
        stream.write_u32::<LittleEndian>(datalen)?;
        stream.write_u32::<LittleEndian>(self.public_exponent)?;
        stream.write_all(&self.modulus)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        RSA_KEY_SIZE_WITHOUT_MODULUS + self.modulus.len()
    }
}
