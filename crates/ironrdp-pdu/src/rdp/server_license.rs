use std::io;

use bitflags::bitflags;
use md5::Digest;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};
use thiserror::Error;

use crate::cursor::{ReadCursor, WriteCursor};
use crate::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags, BASIC_SECURITY_HEADER_SIZE};
use crate::{PduDecode, PduEncode, PduError, PduResult};

#[cfg(test)]
mod tests;

mod client_new_license_request;
mod client_platform_challenge_response;
mod licensing_error_message;
mod server_license_request;
mod server_platform_challenge;
mod server_upgrade_license;

pub use self::client_new_license_request::{ClientNewLicenseRequest, PLATFORM_ID};
pub use self::client_platform_challenge_response::ClientPlatformChallengeResponse;
pub use self::licensing_error_message::{LicenseErrorCode, LicensingErrorMessage, LicensingStateTransition};
pub use self::server_license_request::{
    cert, InitialMessageType, InitialServerLicenseMessage, ProductInfo, Scope, ServerCertificate, ServerLicenseRequest,
};
pub use self::server_platform_challenge::ServerPlatformChallenge;
pub use self::server_upgrade_license::{NewLicenseInformation, ServerUpgradeLicense};

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

impl LicenseHeader {
    const NAME: &'static str = "LicenseHeader";

    const FIXED_PART_SIZE: usize = PREAMBLE_SIZE + BASIC_SECURITY_HEADER_SIZE;
}

impl PduEncode for LicenseHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        self.security_header.encode(dst)?;

        let flags_with_version = self.preamble_flags.bits() | self.preamble_version.to_u8().unwrap();

        dst.write_u8(self.preamble_message_type.to_u8().unwrap());
        dst.write_u8(flags_with_version);
        dst.write_u16(self.preamble_message_size); // msg size

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for LicenseHeader {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let security_header = BasicSecurityHeader::decode(src)?;

        if !security_header.flags.contains(BasicSecurityHeaderFlags::LICENSE_PKT) {
            return Err(invalid_message_err!(
                "securityHeaderFlags",
                "invalid security header flags"
            ));
        }

        let preamble_message_type = PreambleType::from_u8(src.read_u8())
            .ok_or_else(|| invalid_message_err!("preambleType", "invalid license type"))?;

        let flags_with_version = src.read_u8();
        let preamble_message_size = src.read_u16();

        let preamble_flags = PreambleFlags::from_bits(flags_with_version & !PROTOCOL_VERSION_MASK)
            .ok_or_else(|| invalid_message_err!("preambleFlags", "Got invalid flags field"))?;

        let preamble_version = PreambleVersion::from_u8(flags_with_version & PROTOCOL_VERSION_MASK)
            .ok_or_else(|| invalid_message_err!("preambleVersion", "Got invalid version in the flags filed"))?;

        Ok(Self {
            security_header,
            preamble_message_type,
            preamble_flags,
            preamble_version,
            preamble_message_size,
        })
    }
}

/// [2.2.1.12.1.1] Licensing Preamble (LICENSE_PREAMBLE)
///
/// [2.2.1.12.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/73170ca2-5f82-4a2d-9d1b-b439f3d8dadc
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
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

#[derive(Debug, Error)]
pub enum ServerLicenseError {
    #[error("IO error: {0}")]
    IOError(#[from] io::Error),
    #[error("UTF-8 error: {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),
    #[error("invalid preamble field: {0}")]
    InvalidPreamble(String),
    #[error("invalid preamble message type field")]
    InvalidLicenseType,
    #[error("invalid error code field")]
    InvalidErrorCode,
    #[error("invalid state transition field")]
    InvalidStateTransition,
    #[error("invalid blob type field")]
    InvalidBlobType,
    #[error("unable to generate random number {0}")]
    RandomNumberGenerationError(String),
    #[error("unable to retrieve public key from the certificate")]
    UnableToGetPublicKey,
    #[error("unable to encrypt RSA public key")]
    RsaKeyEncryptionError,
    #[error("invalid License Request key exchange algorithm value")]
    InvalidKeyExchangeValue,
    #[error("MAC checksum generated over decrypted data does not match the server's checksum")]
    InvalidMacData,
    #[error("invalid platform challenge response data version")]
    InvalidChallengeResponseDataVersion,
    #[error("invalid platform challenge response data client type")]
    InvalidChallengeResponseDataClientType,
    #[error("invalid platform challenge response data license detail level")]
    InvalidChallengeResponseDataLicenseDetail,
    #[error("invalid x509 certificate")]
    InvalidX509Certificate {
        source: x509_cert::der::Error,
        cert_der: Vec<u8>,
    },
    #[error("invalid certificate version")]
    InvalidCertificateVersion,
    #[error("invalid x509 certificates amount")]
    InvalidX509CertificatesAmount,
    #[error("invalid proprietary certificate signature algorithm ID")]
    InvalidPropCertSignatureAlgorithmId,
    #[error("invalid proprietary certificate key algorithm ID")]
    InvalidPropCertKeyAlgorithmId,
    #[error("invalid RSA public key magic")]
    InvalidRsaPublicKeyMagic,
    #[error("invalid RSA public key length")]
    InvalidRsaPublicKeyLength,
    #[error("invalid RSA public key data length")]
    InvalidRsaPublicKeyDataLength,
    #[error("invalid RSA public key bit length")]
    InvalidRsaPublicKeyBitLength,
    #[error("invalid License Header security flags")]
    InvalidSecurityFlags,
    #[error("the server returned unexpected error: {0:?}")]
    UnexpectedError(LicensingErrorMessage),
    #[error("got unexpected license message")]
    UnexpectedLicenseMessage,
    #[error("the server has returned an unexpected error")]
    UnexpectedServerError(LicensingErrorMessage),
    #[error("the server has returned STATUS_VALID_CLIENT (not an error)")]
    ValidClientStatus(LicensingErrorMessage),
    #[error("invalid Key Exchange List field")]
    InvalidKeyExchangeAlgorithm,
    #[error("received invalid company name length (Product Information): {0}")]
    InvalidCompanyNameLength(u32),
    #[error("received invalid product ID length (Product Information): {0}")]
    InvalidProductIdLength(u32),
    #[error("received invalid scope count field: {0}")]
    InvalidScopeCount(u32),
    #[error("received invalid certificate length: {0}")]
    InvalidCertificateLength(u32),
    #[error("blob too small")]
    BlobTooSmall,
    #[error("PDU error: {0}")]
    Pdu(PduError),
}

impl From<PduError> for ServerLicenseError {
    fn from(e: PduError) -> Self {
        Self::Pdu(e)
    }
}

#[cfg(feature = "std")]
impl ironrdp_error::legacy::ErrorContext for ServerLicenseError {
    fn context(&self) -> &'static str {
        "server license"
    }
}

pub struct BlobHeader {
    pub blob_type: BlobType,
    pub length: usize,
}

impl BlobHeader {
    const NAME: &'static str = "BlobHeader";

    const FIXED_PART_SIZE: usize = 2 /* blobType */ + 2 /* len */;

    pub fn new(blob_type: BlobType, length: usize) -> Self {
        Self { blob_type, length }
    }
}

impl PduEncode for BlobHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.blob_type.to_u16().unwrap());
        dst.write_u16(cast_length!("len", self.length)?);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for BlobHeader {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let blob_type = src.read_u16();
        let blob_type =
            BlobType::from_u16(blob_type).ok_or_else(|| invalid_message_err!("blobType", "invalid blob type"))?;

        let length = cast_length!("len", src.read_u16())?;

        Ok(Self { blob_type, length })
    }
}

fn compute_mac_data(mac_salt_key: &[u8], data: &[u8]) -> Vec<u8> {
    let data_len_buffer = (data.len() as u32).to_le_bytes();

    let pad_one: [u8; 40] = [0x36; 40];

    let mut hasher = sha1::Sha1::new();
    hasher.update(
        [mac_salt_key, pad_one.as_ref(), data_len_buffer.as_ref(), data]
            .concat()
            .as_slice(),
    );
    let sha_result = hasher.finalize();

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
    src: &mut ReadCursor<'_>,
) -> Result<LicenseHeader, PduError> {
    let license_header = LicenseHeader::decode(src)?;

    // FIXME(#269): ERROR_ALERT licensing packets should not be returned as error by the parser.
    // Such packets should be handled by the caller, and the caller is responsible for turning
    // those into "Result::Err" if necessary. It should be possible to decode a `LICENSE_ERROR_MESSAGE`
    // structure like any other PDU.
    // Otherwise it requires the caller to match on the error kind in order to check for variants that are
    // not actual errors, it makes the flow of control harder to write correctly and less obvious.
    // See `ConnectionConfirm` from the `nego` module for prior art.

    if license_header.preamble_message_type != required_preamble_message_type {
        if license_header.preamble_message_type == PreambleType::ErrorAlert {
            let license_error = LicensingErrorMessage::decode(src)?;

            if license_error.error_code == LicenseErrorCode::StatusValidClient
                && license_error.state_transition == LicensingStateTransition::NoTransition
            {
                return Err(invalid_message_err!(
                    "preambleType",
                    "the server has returned STATUS_VALID_CLIENT (not an error)"
                ));
            } else {
                return Err(invalid_message_err!("preambleType", "invalid preamble type"));
            }
        } else {
            return Err(invalid_message_err!("preambleType", "got unexptected preamble type"));
        }
    }

    Ok(license_header)
}
