use std::io;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Fail;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{
    impl_from_error,
    utils::{self, SplitTo},
    PduBufferParsing,
};

const PRECONNECTION_PDU_V1_SIZE: usize = 16;

#[derive(Debug, Clone, PartialEq)]
pub struct PreconnectionPdu {
    pub id: u32,
    pub payload: Option<String>,
}

impl PreconnectionPdu {}

impl PduBufferParsing<'_> for PreconnectionPdu {
    type Error = PreconnectionPduError;

    fn from_buffer_consume(buffer: &mut &[u8]) -> Result<Self, Self::Error> {
        if buffer.len() < PRECONNECTION_PDU_V1_SIZE {
            return Err(PreconnectionPduError::IoError(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "More data required to parse Preconnection PDU header",
            )));
        }

        let size = buffer.read_u32::<LittleEndian>()? as usize;

        if (size % 2 != 0) || (size < PRECONNECTION_PDU_V1_SIZE) {
            return Err(PreconnectionPduError::InvalidHeader);
        }

        buffer.read_u32::<LittleEndian>()?; // flags
        let version = buffer.read_u32::<LittleEndian>()?;
        let version =
            Version::from_u32(version).ok_or(PreconnectionPduError::UnexpectedVersion(version))?;

        let id = buffer.read_u32::<LittleEndian>()?;

        let remaining_size = size - PRECONNECTION_PDU_V1_SIZE;

        if buffer.len() < remaining_size {
            return Err(PreconnectionPduError::IoError(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "More data required to parse Preconnection PDU payload",
            )));
        }

        let mut buffer = buffer.split_to(remaining_size);

        let payload = match version {
            Version::V1 => None,
            Version::V2 => {
                let size = buffer.read_u16::<LittleEndian>()? as usize;
                if buffer.len() < size * 2 {
                    return Err(PreconnectionPduError::InvalidDataLength {
                        expected: size * 2,
                        actual: buffer.len(),
                    });
                }

                let payload_bytes = buffer.split_to(size * 2);
                let payload = utils::bytes_to_utf16_string(payload_bytes)
                    .trim_end_matches('\0')
                    .into();

                Some(payload)
            }
        };

        Ok(Self { id, payload })
    }

    fn to_buffer_consume(&self, buffer: &mut &mut [u8]) -> Result<(), Self::Error> {
        let size = self.buffer_length();

        buffer.write_u32::<LittleEndian>(size as u32)?;
        buffer.write_u32::<LittleEndian>(0)?; // flags
        buffer.write_u32::<LittleEndian>(Version::from(self).to_u32().unwrap())?;
        buffer.write_u32::<LittleEndian>(self.id)?;

        if let Some(ref payload) = self.payload {
            buffer.write_u16::<LittleEndian>(payload.len() as u16 + 1)?; // + null terminator
            utils::write_string_with_null_terminator(
                buffer,
                payload.as_str(),
                utils::CharacterSet::Unicode,
            )?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        PRECONNECTION_PDU_V1_SIZE
            + self
                .payload
                .as_ref()
                .map(|p| 2 + (p.len() + 1) * 2)
                .unwrap_or(0)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, FromPrimitive, ToPrimitive)]
enum Version {
    V1 = 0x1,
    V2 = 0x2,
}

impl From<&PreconnectionPdu> for Version {
    fn from(p: &PreconnectionPdu) -> Self {
        if p.payload.is_some() {
            Self::V2
        } else {
            Self::V1
        }
    }
}

#[derive(Debug, Fail)]
pub enum PreconnectionPduError {
    #[fail(display = "IO error: {}", _0)]
    IoError(#[fail(cause)] io::Error),
    #[fail(display = "Provided data is not an valid preconnection Pdu")]
    InvalidHeader,
    #[fail(display = "Unexpected version: {}", _0)]
    UnexpectedVersion(u32),
}

impl_from_error!(
    io::Error,
    PreconnectionPduError,
    PreconnectionPduError::IoError
);

#[cfg(test)]
mod tests {
    use lazy_static::lazy_static;

    use super::*;

    const PRECONNECTION_PDU_V1_EMPTY_SIZE_BUFFER: [u8; 16] = [
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::cbSize = 0x00 = 0 bytes
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Flags = 0
        0x01, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Version = 1
        0xeb, 0x99, 0xc6, 0xee, // -> RDP_PRECONNECTION_PDU_V1::Id = 0xEEC699EB = 4005992939
    ];
    const PRECONNECTION_PDU_V1_LARGE_DATA_LENGTH_BUFFER: [u8; 16] = [
        0xff, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::cbSize = 0xff
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Flags = 0
        0x01, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Version = 1
        0xeb, 0x99, 0xc6, 0xee, // -> RDP_PRECONNECTION_PDU_V1::Id = 0xEEC699EB = 4005992939
    ];
    const PRECONNECTION_PDU_V1_BUFFER: [u8; 16] = [
        0x10, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::cbSize = 0x10 = 16 bytes
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Flags = 0
        0x01, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Version = 1
        0xeb, 0x99, 0xc6, 0xee, // -> RDP_PRECONNECTION_PDU_V1::Id = 0xEEC699EB = 4005992939
    ];
    const PRECONNECTION_PDU_V2_LARGE_PAYLOAD_SIZE_BUFFER: [u8; 32] = [
        0x20, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::cbSize = 0x20 = 32 bytes
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Flags = 0
        0x02, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Version = 2
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Id = 0
        0xff, 0x00, //       -> RDP_PRECONNECTION_PDU_V2::cchPCB = 0xff
        0x54, 0x00, 0x65, 0x00, 0x73, 0x00, 0x74, 0x00, 0x56, 0x00, 0x4d, 0x00, 0x00,
        0x00, // -> RDP_PRECONNECTION_PDU_V2::wszPCB -> "TestVM" (including null terminator)
    ];
    const PRECONNECTION_PDU_V2_BUFFER: [u8; 32] = [
        0x20, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::cbSize = 0x20 = 32 bytes
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Flags = 0
        0x02, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Version = 2
        0x00, 0x00, 0x00, 0x00, // -> RDP_PRECONNECTION_PDU_V1::Id = 0
        0x07, 0x00, //       -> RDP_PRECONNECTION_PDU_V2::cchPCB = 0x7 = 7 characters
        0x54, 0x00, 0x65, 0x00, 0x73, 0x00, 0x74, 0x00, 0x56, 0x00, 0x4d, 0x00, 0x00,
        0x00, // -> RDP_PRECONNECTION_PDU_V2::wszPCB -> "TestVM" (including null terminator)
    ];

    const PRECONNECTION_PDU_V1: PreconnectionPdu = PreconnectionPdu {
        id: 4_005_992_939,
        payload: None,
    };

    lazy_static! {
        static ref PRECONNECTION_PDU_V2: PreconnectionPdu = PreconnectionPdu {
            id: 0,
            payload: Some(String::from("TestVM")),
        };
    }

    #[test]
    fn from_buffer_for_preconnection_pdu_returns_error_on_empty_size() {
        assert!(
            PreconnectionPdu::from_buffer(PRECONNECTION_PDU_V1_EMPTY_SIZE_BUFFER.as_ref()).is_err()
        );
    }

    #[test]
    fn from_buffer_for_preconnection_pdu_returns_error_on_data_length_greater_then_available_data()
    {
        assert!(PreconnectionPdu::from_buffer(
            PRECONNECTION_PDU_V1_LARGE_DATA_LENGTH_BUFFER.as_ref()
        )
        .is_err());
    }

    #[test]
    fn from_buffer_correctly_parses_preconnection_pdu_v1() {
        assert_eq!(
            PRECONNECTION_PDU_V1,
            PreconnectionPdu::from_buffer(PRECONNECTION_PDU_V1_BUFFER.as_ref()).unwrap()
        );
    }

    #[test]
    fn to_buffer_correctly_serializes_preconnection_pdu_v1() {
        let expected = PRECONNECTION_PDU_V1_BUFFER.as_ref();
        let mut buffer = vec![0; expected.len()];

        PRECONNECTION_PDU_V1
            .to_buffer_consume(&mut buffer.as_mut_slice())
            .unwrap();
        assert_eq!(expected, buffer.as_slice());
    }

    #[test]
    fn buffer_length_is_correct_for_preconnection_pdu_v1() {
        assert_eq!(
            PRECONNECTION_PDU_V1_BUFFER.len(),
            PRECONNECTION_PDU_V1.buffer_length()
        );
    }

    #[test]
    fn from_buffer_for_preconnection_pdu_v2_returns_error_on_payload_size_greater_then_available_data(
    ) {
        assert!(PreconnectionPdu::from_buffer(
            PRECONNECTION_PDU_V2_LARGE_PAYLOAD_SIZE_BUFFER.as_ref()
        )
        .is_err());
    }

    #[test]
    fn from_buffer_correctly_parses_preconnection_pdu_v2() {
        assert_eq!(
            *PRECONNECTION_PDU_V2,
            PreconnectionPdu::from_buffer(PRECONNECTION_PDU_V2_BUFFER.as_ref()).unwrap()
        );
    }

    #[test]
    fn to_buffer_correctly_serializes_preconnection_pdu_v2() {
        let expected = PRECONNECTION_PDU_V2_BUFFER.as_ref();
        let mut buffer = vec![0; expected.len()];

        PRECONNECTION_PDU_V2
            .to_buffer_consume(&mut buffer.as_mut_slice())
            .unwrap();
        assert_eq!(expected, buffer.as_slice());
    }

    #[test]
    fn buffer_length_is_correct_for_preconnection_pdu_v2() {
        assert_eq!(
            PRECONNECTION_PDU_V2_BUFFER.len(),
            PRECONNECTION_PDU_V2.buffer_length()
        );
    }
}
