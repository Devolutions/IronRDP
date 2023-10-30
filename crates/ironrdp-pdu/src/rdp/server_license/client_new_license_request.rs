#[cfg(test)]
mod tests;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use md5::Digest;

use super::{
    BasicSecurityHeader, BasicSecurityHeaderFlags, BlobHeader, BlobType, LicenseEncryptionData, LicenseHeader,
    PreambleFlags, PreambleType, PreambleVersion, ServerLicenseError, ServerLicenseRequest, KEY_EXCHANGE_ALGORITHM_RSA,
    PREAMBLE_SIZE, RANDOM_NUMBER_SIZE, UTF8_NULL_TERMINATOR_SIZE,
};
use crate::crypto::rsa::encrypt_with_public_key;
use crate::utils::{self, CharacterSet};
use crate::PduParsing;

const LICENSE_REQUEST_STATIC_FIELDS_SIZE: usize = 20;

pub const PLATFORM_ID: u32 = ClientOsType::NT_POST_52.bits() | Isv::MICROSOFT.bits();

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ClientOsType: u32 {
        const NT_351 = 0x100_0000;
        const NT_40 = 0x200_0000;
        const NT_50 = 0x300_0000;
        const NT_POST_52 = 0x400_0000;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct Isv: u32 {
        const MICROSOFT = 0x10000;
        const CITRIX = 0x20000;
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ClientNewLicenseRequest {
    pub license_header: LicenseHeader,
    pub client_random: Vec<u8>,
    pub encrypted_premaster_secret: Vec<u8>,
    pub client_username: String,
    pub client_machine_name: String,
}

impl ClientNewLicenseRequest {
    pub fn from_server_license_request(
        license_request: &ServerLicenseRequest,
        client_random: &[u8],
        premaster_secret: &[u8],
        client_username: &str,
        client_machine_name: &str,
    ) -> Result<(Self, LicenseEncryptionData), ServerLicenseError> {
        let public_key = license_request.get_public_key()?
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData,
                "attempted to retrieve the server public key from a server license request message that does not have a certificate"))?;

        let encrypted_premaster_secret = encrypt_with_public_key(premaster_secret, &public_key)?;

        let master_secret = compute_master_secret(
            premaster_secret,
            client_random,
            license_request.server_random.as_slice(),
        );
        let session_key_blob = compute_session_key_blob(
            master_secret.as_slice(),
            client_random,
            license_request.server_random.as_slice(),
        );
        let mac_salt_key = &session_key_blob[..16];

        let mut md5 = md5::Md5::new();
        md5.update(
            [
                &session_key_blob[16..32],
                client_random,
                license_request.server_random.as_slice(),
            ]
            .concat()
            .as_slice(),
        );
        let license_key = md5.finalize().to_vec();

        let license_header = LicenseHeader {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::LICENSE_PKT,
            },
            preamble_message_type: PreambleType::NewLicenseRequest,
            preamble_flags: PreambleFlags::empty(),
            preamble_version: PreambleVersion::V3,
            preamble_message_size: (RANDOM_NUMBER_SIZE
                + PREAMBLE_SIZE
                + LICENSE_REQUEST_STATIC_FIELDS_SIZE
                + encrypted_premaster_secret.len()
                + client_machine_name.len()
                + UTF8_NULL_TERMINATOR_SIZE
                + client_username.len()
                + UTF8_NULL_TERMINATOR_SIZE) as u16,
        };

        Ok((
            Self {
                license_header,
                client_random: Vec::from(client_random),
                encrypted_premaster_secret,
                client_username: client_username.to_owned(),
                client_machine_name: client_machine_name.to_owned(),
            },
            LicenseEncryptionData {
                premaster_secret: Vec::from(premaster_secret),
                mac_salt_key: Vec::from(mac_salt_key),
                license_key,
            },
        ))
    }
}

impl PduParsing for ClientNewLicenseRequest {
    type Error = ServerLicenseError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let license_header = LicenseHeader::from_buffer(&mut stream)?;
        if license_header.preamble_message_type != PreambleType::NewLicenseRequest {
            return Err(ServerLicenseError::InvalidPreamble(format!(
                "Got {:?} but expected {:?}",
                license_header.preamble_message_type,
                PreambleType::NewLicenseRequest
            )));
        }

        let key_exchange_algorithm = stream.read_u32::<LittleEndian>()?;
        if key_exchange_algorithm != KEY_EXCHANGE_ALGORITHM_RSA {
            return Err(ServerLicenseError::InvalidKeyExchangeValue);
        }

        let _platform_id = stream.read_u32::<LittleEndian>()?;

        let mut client_random = vec![0u8; RANDOM_NUMBER_SIZE];
        stream.read_exact(&mut client_random)?;

        let premaster_secret_blob_header = BlobHeader::read_from_buffer(BlobType::Random, &mut stream)?;
        let mut encrypted_premaster_secret = vec![0u8; premaster_secret_blob_header.length];
        stream.read_exact(&mut encrypted_premaster_secret)?;

        let username_blob_header = BlobHeader::read_from_buffer(BlobType::ClientUserName, &mut stream)?;
        let client_username =
            utils::read_string_from_stream(&mut stream, username_blob_header.length, CharacterSet::Ansi, false)?;

        let machine_name_blob = BlobHeader::read_from_buffer(BlobType::ClientMachineNameBlob, &mut stream)?;
        let client_machine_name =
            utils::read_string_from_stream(&mut stream, machine_name_blob.length, CharacterSet::Ansi, false)?;

        Ok(Self {
            license_header,
            client_random,
            encrypted_premaster_secret,
            client_username,
            client_machine_name,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        self.license_header.to_buffer(&mut stream)?;

        stream.write_u32::<LittleEndian>(KEY_EXCHANGE_ALGORITHM_RSA)?;
        stream.write_u32::<LittleEndian>(PLATFORM_ID)?;
        stream.write_all(&self.client_random)?;

        BlobHeader::new(BlobType::Random, self.encrypted_premaster_secret.len()).write_to_buffer(&mut stream)?;
        stream.write_all(&self.encrypted_premaster_secret)?;

        BlobHeader::new(
            BlobType::ClientUserName,
            self.client_username.len() + UTF8_NULL_TERMINATOR_SIZE,
        )
        .write_to_buffer(&mut stream)?;
        utils::write_string_with_null_terminator(&mut stream, &self.client_username, CharacterSet::Ansi)?;

        BlobHeader::new(
            BlobType::ClientMachineNameBlob,
            self.client_machine_name.len() + UTF8_NULL_TERMINATOR_SIZE,
        )
        .write_to_buffer(&mut stream)?;
        utils::write_string_with_null_terminator(&mut stream, &self.client_machine_name, CharacterSet::Ansi)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        self.license_header.buffer_length()
            + LICENSE_REQUEST_STATIC_FIELDS_SIZE
            + RANDOM_NUMBER_SIZE
            + self.encrypted_premaster_secret.len()
            + self.client_machine_name.len()
            + UTF8_NULL_TERMINATOR_SIZE
            + self.client_username.len()
            + UTF8_NULL_TERMINATOR_SIZE
    }
}

fn salted_hash(salt: &[u8], salt_first: &[u8], salt_second: &[u8], input: &[u8]) -> Vec<u8> {
    let mut hasher = sha1::Sha1::new();
    hasher.update([input, salt, salt_first, salt_second].concat().as_slice());
    let sha_result = hasher.finalize();

    let mut md5 = md5::Md5::new();
    md5.update([salt, sha_result.as_ref()].concat().as_slice());

    md5.finalize().to_vec()
}

// According to https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpele/88061224-4a2f-4a28-a52e-e896b75ed2d3
fn compute_master_secret(premaster_secret: &[u8], client_random: &[u8], server_random: &[u8]) -> Vec<u8> {
    [
        salted_hash(premaster_secret, client_random, server_random, b"A"),
        salted_hash(premaster_secret, client_random, server_random, b"BB"),
        salted_hash(premaster_secret, client_random, server_random, b"CCC"),
    ]
    .concat()
}

// According to https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpele/88061224-4a2f-4a28-a52e-e896b75ed2d3
fn compute_session_key_blob(master_secret: &[u8], client_random: &[u8], server_random: &[u8]) -> Vec<u8> {
    [
        salted_hash(master_secret, server_random, client_random, b"A"),
        salted_hash(master_secret, server_random, client_random, b"BB"),
        salted_hash(master_secret, server_random, client_random, b"CCC"),
    ]
    .concat()
}
