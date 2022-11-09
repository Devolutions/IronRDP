use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use md5::Digest;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};
use ring::digest;

use crate::rdp::{BasicSecurityHeader, BasicSecurityHeaderFlags, BASIC_SECURITY_HEADER_SIZE};
use crate::{impl_from_error, PduParsing};

#[cfg(test)]
pub mod test;

mod client_new_license_request;
mod client_platform_challenge_response;
mod licensing_error_message;
mod server_license_request;
mod server_platform_challenge;
mod server_upgrade_license;

pub use self::client_new_license_request::{ClientNewLicenseRequest, PLATFORM_ID};
pub use self::client_platform_challenge_response::ClientPlatformChallengeResponse;
pub use self::licensing_error_message::{LicenseErrorCode, LicensingErrorMessage, LicensingStateTransition};
pub use self::server_license_request::{InitialMessageType, InitialServerLicenseMessage, ServerLicenseRequest};
pub use self::server_platform_challenge::ServerPlatformChallenge;
pub use self::server_upgrade_license::ServerUpgradeLicense;

pub const PREAMBLE_SIZE: usize = 4;
pub const PREMASTER_SECRET_SIZE: usize = 48;
pub const RANDOM_NUMBER_SIZE: usize = 32;

const PROTOCOL_VERSION_MASK: u8 = 0x0F;

const BLOB_TYPE_SIZE: usize = 2;
const BLOB_LENGTH_SIZE: usize = 2;

const UTF8_NULL_TERMINATOR_SIZE: usize = 1;
const UTF16_NULL_TERMINATOR_SIZE: usize = 2;

const KEY_EXCHANGE_ALGORITHM_RSA: u32 = 1;

const MAC_SIZE: usize = 16;

#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct LicenseEncryptionData {
    pub premaster_secret: Vec<u8>,
    pub mac_salt_key: Vec<u8>,
    pub license_key: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct LicenseHeader {
    pub security_header: BasicSecurityHeader,
    pub preamble_message_type: PreambleType,
    pub preamble_flags: PreambleFlags,
    pub preamble_version: PreambleVersion,
    pub preamble_message_size: u16,
}

impl PduParsing for LicenseHeader {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let security_header = BasicSecurityHeader::from_buffer(&mut stream).map_err(|err| {
            ServerLicenseError::IOError(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unable to read License Header from buffer. Error: {}", err),
            ))
        })?;

        if !security_header.flags.contains(BasicSecurityHeaderFlags::LICENSE_PKT) {
            return Err(ServerLicenseError::InvalidSecurityFlags);
        }

        let preamble_message_type =
            PreambleType::from_u8(stream.read_u8()?).ok_or(ServerLicenseError::InvalidLicenseType)?;

        let flags_with_version = stream.read_u8()?;
        let preamble_message_size = stream.read_u16::<LittleEndian>()?;

        let preamble_flags = PreambleFlags::from_bits(flags_with_version & !PROTOCOL_VERSION_MASK)
            .ok_or_else(|| ServerLicenseError::InvalidPreamble(String::from("Got invalid flags field")))?;

        let preamble_version = PreambleVersion::from_u8((flags_with_version & PROTOCOL_VERSION_MASK) as u8)
            .ok_or_else(|| {
                ServerLicenseError::InvalidPreamble(String::from("Got invalid version in the flags field"))
            })?;

        Ok(Self {
            security_header,
            preamble_message_type,
            preamble_flags,
            preamble_version,
            preamble_message_size,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        self.security_header.to_buffer(&mut stream).map_err(|err| {
            ServerLicenseError::IOError(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unable to write License Header to buffer. Error: {}", err),
            ))
        })?;

        let flags_with_version = self.preamble_flags.bits() | self.preamble_version.to_u8().unwrap();

        stream.write_u8(self.preamble_message_type.to_u8().unwrap())?;
        stream.write_u8(flags_with_version)?;
        stream.write_u16::<LittleEndian>(self.preamble_message_size)?; // msg size

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        PREAMBLE_SIZE + BASIC_SECURITY_HEADER_SIZE
    }
}

#[repr(u8)]
#[derive(Debug, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum PreambleType {
    LicenseRequest = 0x01,
    PlatformChallenge = 0x02,
    NewLicense = 0x03,
    UpgradeLicense = 0x04,
    LicenseInfo = 0x12,
    NewLicenseRequest = 0x13,
    PlatformChallengeResponse = 0x15,
    ErrorAlert = 0xff,
}

bitflags! {
    pub struct PreambleFlags: u8 {
        const EXTENDED_ERROR_MSG_SUPPORTED = 0x80;
    }
}

#[derive(Debug, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum PreambleVersion {
    V2 = 2, // RDP 4.0
    V3 = 3, // RDP 5.0, 5.1, 5.2, 6.0, 6.1, 7.0, 7.1, 8.0, 8.1, 10.0, 10.1, 10.2, 10.3, 10.4, and 10.5
}

#[derive(Debug, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum BlobType {
    Any = 0x00,
    Data = 0x01,
    Random = 0x02,
    Certificate = 0x03,
    Error = 0x04,
    RsaKey = 0x06,
    EncryptedData = 0x09,
    RsaSignature = 0x08,
    KeyExchangeAlgorithm = 0x0d,
    Scope = 0x0e,
    ClientUserName = 0x0f,
    ClientMachineNameBlob = 0x10,
}

#[derive(Debug, Fail)]
pub enum ServerLicenseError {
    #[fail(display = "IO error: {}", _0)]
    IOError(#[fail(cause)] io::Error),
    #[fail(display = "UTF-8 error: {}", _0)]
    Utf8Error(#[fail(cause)] std::string::FromUtf8Error),
    #[fail(display = "Invalid preamble field: {}", _0)]
    InvalidPreamble(String),
    #[fail(display = "Invalid preamble message type field")]
    InvalidLicenseType,
    #[fail(display = "Invalid error code field")]
    InvalidErrorCode,
    #[fail(display = "Invalid state transition field")]
    InvalidStateTransition,
    #[fail(display = "Invalid blob type field")]
    InvalidBlobType,
    #[fail(display = "Unable to generate random number {}", _0)]
    RandomNumberGenerationError(String),
    #[fail(display = "Unable to retrieve public key from the certificate")]
    UnableToGetPublicKey,
    #[fail(display = "Unable to encrypt RSA public key")]
    RsaKeyEncryptionError,
    #[fail(display = "Invalid License Request key exchange algorithm value")]
    InvalidKeyExchangeValue,
    #[fail(display = "MAC checksum generated over decrypted data does not match the server's checksum")]
    InvalidMacData,
    #[fail(display = "Invalid platform challenge response data version")]
    InvalidChallengeResponseDataVersion,
    #[fail(display = "Invalid platform challenge response data client type")]
    InvalidChallengeResponseDataClientType,
    #[fail(display = "Invalid platform challenge response data license detail level")]
    InvalidChallengeResponseDataLicenseDetail,
    #[fail(display = "Invalid x509 certificate")]
    InvalidX509Certificate,
    #[fail(display = "Invalid certificate version")]
    InvalidCertificateVersion,
    #[fail(display = "Invalid x509 certificates amount")]
    InvalidX509CertificatesAmount,
    #[fail(display = "Invalid proprietary certificate signature algorithm ID")]
    InvalidPropCertSignatureAlgorithmId,
    #[fail(display = "Invalid proprietary certificate key algorithm ID")]
    InvalidPropCertKeyAlgorithmId,
    #[fail(display = "Invalid RSA public key magic")]
    InvalidRsaPublicKeyMagic,
    #[fail(display = "Invalid RSA public key length")]
    InvalidRsaPublicKeyLength,
    #[fail(display = "Invalid RSA public key data length")]
    InvalidRsaPublicKeyDataLength,
    #[fail(display = "Invalid License Header security flags")]
    InvalidSecurityFlags,
    #[fail(display = "The server returned unexpected error")]
    UnexpectedError(LicensingErrorMessage),
    #[fail(display = "Got unexpected license message")]
    UnexpectedLicenseMessage,
    #[fail(display = "The server has returned an unexpected error")]
    UnexpectedServerError(LicensingErrorMessage),
    #[fail(display = "The server has returned STATUS_VALID_CLIENT unexpectedly")]
    UnexpectedValidClientError(LicensingErrorMessage),
    #[fail(display = "Invalid Key Exchange List field")]
    InvalidKeyExchangeAlgorithm,
    #[fail(display = "Received invalid company name length (Product Information): {}", _0)]
    InvalidCompanyNameLength(u32),
    #[fail(display = "Received invalid product ID length (Product Information): {}", _0)]
    InvalidProductIdLength(u32),
    #[fail(display = "Received invalid scope count field: {}", _0)]
    InvalidScopeCount(u32),
    #[fail(display = "Received invalid sertificate length: {}", _0)]
    InvalidCertificateLength(u32),
}

impl_from_error!(io::Error, ServerLicenseError, ServerLicenseError::IOError);

pub struct BlobHeader {
    pub blob_type: BlobType,
    pub length: usize,
}

impl BlobHeader {
    pub fn new(blob_type: BlobType, length: usize) -> Self {
        Self { blob_type, length }
    }

    pub fn read_from_buffer(
        required_blob_type: BlobType,
        mut stream: impl io::Read,
    ) -> Result<Self, ServerLicenseError> {
        let blob_type = stream.read_u16::<LittleEndian>()?;
        let blob_type = BlobType::from_u16(blob_type).ok_or(ServerLicenseError::InvalidBlobType)?;

        if blob_type != required_blob_type {
            return Err(ServerLicenseError::InvalidBlobType);
        }

        let length = stream.read_u16::<LittleEndian>()? as usize;

        Ok(Self { blob_type, length })
    }

    pub fn read_any_blob_from_buffer(mut stream: impl io::Read) -> Result<Self, ServerLicenseError> {
        let _blob_type = stream.read_u16::<LittleEndian>()?;
        let length = stream.read_u16::<LittleEndian>()? as usize;

        Ok(Self {
            blob_type: BlobType::Any,
            length,
        })
    }

    pub fn write_to_buffer(&self, mut stream: impl io::Write) -> Result<(), ServerLicenseError> {
        stream.write_u16::<LittleEndian>(self.blob_type.to_u16().unwrap())?;
        stream.write_u16::<LittleEndian>(self.length as u16)?;

        Ok(())
    }
}

impl_from_error!(
    std::string::FromUtf8Error,
    ServerLicenseError,
    ServerLicenseError::Utf8Error
);

fn compute_mac_data(mac_salt_key: &[u8], data: &[u8]) -> Vec<u8> {
    let data_len_buffer = (data.len() as u32).to_le_bytes();

    let pad_one: [u8; 40] = [0x36; 40];

    let sha_result = digest::digest(
        &digest::SHA1_FOR_LEGACY_USE_ONLY,
        [mac_salt_key, pad_one.as_ref(), data_len_buffer.as_ref(), data]
            .concat()
            .as_slice(),
    );

    let pad_two: [u8; 48] = [0x5c; 48];

    let mut md5 = md5::Md5::new();
    md5.update(
        [mac_salt_key, pad_two.as_ref(), sha_result.as_ref()]
            .concat()
            .as_slice(),
    );

    md5.finalize().to_vec()
}

fn read_license_header(
    required_preamble_message_type: PreambleType,
    mut stream: impl io::Read,
) -> Result<LicenseHeader, ServerLicenseError> {
    let license_header = LicenseHeader::from_buffer(&mut stream)?;

    if license_header.preamble_message_type != required_preamble_message_type {
        if license_header.preamble_message_type == PreambleType::ErrorAlert {
            let license_error = LicensingErrorMessage::from_buffer(&mut stream)?;

            if license_error.error_code == LicenseErrorCode::StatusValidClient
                && license_error.state_transition == LicensingStateTransition::NoTransition
            {
                return Err(ServerLicenseError::UnexpectedValidClientError(license_error));
            } else {
                return Err(ServerLicenseError::UnexpectedServerError(license_error));
            }
        } else {
            return Err(ServerLicenseError::InvalidPreamble(format!(
                "Got {:?} but expected {:?}",
                license_header.preamble_message_type, required_preamble_message_type
            )));
        }
    }

    Ok(license_header)
}
