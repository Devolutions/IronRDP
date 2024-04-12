#[cfg(test)]
mod test;

use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use super::{BlobHeader, BlobType, LicenseHeader, BLOB_LENGTH_SIZE, BLOB_TYPE_SIZE};
use crate::{
    cursor::{ReadCursor, WriteCursor},
    rdp::server_license::PreambleType,
    PduDecode, PduEncode, PduResult,
};

const ERROR_CODE_SIZE: usize = 4;
const STATE_TRANSITION_SIZE: usize = 4;

/// [2.2.1.12.1.3] Licensing Error Message (LICENSE_ERROR_MESSAGE)
///
/// [2.2.1.12.1.3]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/f18b6c9f-f3d8-4a0e-8398-f9b153233dca
#[derive(Debug, PartialEq, Eq)]
pub struct LicensingErrorMessage {
    pub error_code: LicenseErrorCode,
    pub state_transition: LicensingStateTransition,
    pub error_info: Vec<u8>,
}

impl LicensingErrorMessage {
    const NAME: &'static str = "LicensingErrorMessage";

    const FIXED_PART_SIZE: usize = ERROR_CODE_SIZE + STATE_TRANSITION_SIZE;
}

impl PduEncode for LicensingErrorMessage {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.error_code.to_u32().unwrap());
        dst.write_u32(self.state_transition.to_u32().unwrap());

        BlobHeader::new(BlobType::ERROR, self.error_info.len()).encode(dst)?;
        dst.write_slice(&self.error_info);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.error_info.len() + BLOB_LENGTH_SIZE + BLOB_TYPE_SIZE
    }
}

impl LicensingErrorMessage {
    pub fn decode(license_header: LicenseHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        if license_header.preamble_message_type != PreambleType::ErrorAlert {
            return Err(invalid_message_err!("preambleMessageType", "unexpected preamble type"));
        }

        ensure_fixed_part_size!(in: src);
        let error_code = LicenseErrorCode::from_u32(src.read_u32())
            .ok_or_else(|| invalid_message_err!("errorCode", "invalid error code"))?;
        let state_transition = LicensingStateTransition::from_u32(src.read_u32())
            .ok_or_else(|| invalid_message_err!("stateTransition", "invalid state transition"))?;

        let error_info_blob = BlobHeader::decode(src)?;
        if error_info_blob.length != 0 && error_info_blob.blob_type != BlobType::ERROR {
            return Err(invalid_message_err!("blobType", "invalid blob type"));
        }

        let error_info = vec![0u8; error_info_blob.length];

        Ok(Self {
            error_code,
            state_transition,
            error_info,
        })
    }
}

#[derive(Debug, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum LicenseErrorCode {
    InvalidServerCertificate = 0x01,
    NoLicense = 0x02,
    InvalidMac = 0x03,
    InvalidScope = 0x4,
    NoLicenseServer = 0x06,
    StatusValidClient = 0x07,
    InvalidClient = 0x08,
    InvalidProductId = 0x0b,
    InvalidMessageLen = 0x0c,
}

#[derive(Debug, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum LicensingStateTransition {
    TotalAbort = 1,
    NoTransition = 2,
    ResetPhaseToStart = 3,
    ResendLastMessage = 4,
}
