#[cfg(test)]
pub mod test;

use std::io;

use super::{ClientGccBlocks, GccError, ServerGccBlocks};
use crate::{mcs, per, PduParsing};

const CONFERENCE_REQUEST_OBJECT_ID: [u8; 6] = [0, 0, 20, 124, 0, 1];
const CONFERENCE_REQUEST_CLIENT_TO_SERVER_H221_NON_STANDARD: &[u8; 4] = b"Duca";
const CONFERENCE_REQUEST_SERVER_TO_CLIENT_H221_NON_STANDARD: &[u8; 4] = b"McDn";
const CONFERENCE_REQUEST_U16_MIN: u16 = 1001;

const CONFERENCE_REQUEST_CONNECT_PDU_SIZE: u16 = 12;
const CONFERENCE_RESPONSE_CONNECT_PDU_SIZE: u16 = 13;
const OBJECT_IDENTIFIER_KEY: u8 = 0;
const CONNECT_GCC_PDU_CONFERENCE_REQUEST_CHOICE: u8 = 0;
const CONNECT_GCC_PDU_CONFERENCE_RESPONSE_CHOICE: u8 = 0x14;
const CONFERENCE_REQUEST_USER_DATA_SELECTION: u8 = 8;
const USER_DATA_NUMBER_OF_SETS: u8 = 1;
const USER_DATA_H221_NON_STANDARD_CHOICE: u8 = 0xc0;
const CONFERENCE_RESPONSE_TAG: u32 = 1;
const CONFERENCE_RESPONSE_RESULT: u8 = 0;
const H221_NON_STANDARD_MIN_LENGTH: usize = 4;
const CONFERENCE_NAME: &[u8] = b"1";

#[derive(Clone, Debug, PartialEq)]
pub struct ConferenceCreateRequest {
    pub gcc_blocks: ClientGccBlocks,
}

impl PduParsing for ConferenceCreateRequest {
    type Error = GccError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        // ConnectData

        // ConnectData::Key: select object (0) of type OBJECT_IDENTIFIER
        if per::read_choice(&mut stream)? != OBJECT_IDENTIFIER_KEY {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Got unexpected ConnectData Key",
            )));
        }
        // ConnectData::Key: value (OBJECT_IDENTIFIER)
        if per::read_object_id(&mut stream)? != CONFERENCE_REQUEST_OBJECT_ID {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Got unexpected ConnectData key value",
            )));
        }

        // ConnectData::connectPDU: length
        let _length = per::read_length(&mut stream)?;
        // ConnectGCCPDU (CHOICE): Select conferenceCreateRequest (0) of type ConferenceCreateRequest
        if per::read_choice(&mut stream)? != CONNECT_GCC_PDU_CONFERENCE_REQUEST_CHOICE {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Got invalid ConnectGCCPDU choice (expected ConferenceCreateRequest)",
            )));
        }
        // ConferenceCreateRequest::Selection: select optional userData from ConferenceCreateRequest
        if per::read_selection(&mut stream)? != CONFERENCE_REQUEST_USER_DATA_SELECTION {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Got invalid ConferenceCreateRequest selection (expected UserData)",
            )));
        }
        // ConferenceCreateRequest::ConferenceName
        per::read_numeric_string(&mut stream, 1)?;
        // padding
        per::read_padding(&mut stream, 1)?;

        // UserData (SET OF SEQUENCE)
        // one set of UserData
        if per::read_number_of_sets(&mut stream)? != USER_DATA_NUMBER_OF_SETS {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Got invalid ConferenceCreateRequest number of sets (expected 1)",
            )));
        }
        // select h221NonStandard
        if per::read_choice(&mut stream)? != USER_DATA_H221_NON_STANDARD_CHOICE {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Expected UserData H221NonStandard choice",
            )));
        }
        // h221NonStandard: client-to-server H.221 key, "Duca"
        if per::read_octet_string(&mut stream, H221_NON_STANDARD_MIN_LENGTH)?
            != CONFERENCE_REQUEST_CLIENT_TO_SERVER_H221_NON_STANDARD
        {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Got invalid H221NonStandard client-to-server key",
            )));
        }
        // H221NonStandardIdentifier (octet string)
        let (_gcc_blocks_buffer_length, _) = per::read_length(&mut stream)?;
        let gcc_blocks = ClientGccBlocks::from_buffer(&mut stream)?;

        Ok(Self { gcc_blocks })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let gcc_blocks_buffer_length = self.gcc_blocks.buffer_length();

        // ConnectData::Key: select type OBJECT_IDENTIFIER
        per::write_choice(&mut stream, OBJECT_IDENTIFIER_KEY)?;
        // ConnectData::Key: value
        per::write_object_id(&mut stream, CONFERENCE_REQUEST_OBJECT_ID)?;

        // ConnectData::connectPDU: length
        per::write_length(
            &mut stream,
            gcc_blocks_buffer_length as u16 + CONFERENCE_REQUEST_CONNECT_PDU_SIZE,
        )?;
        // ConnectGCCPDU (CHOICE): Select conferenceCreateRequest (0) of type ConferenceCreateRequest
        per::write_choice(&mut stream, CONNECT_GCC_PDU_CONFERENCE_REQUEST_CHOICE)?;
        // ConferenceCreateRequest::Selection: select optional userData from ConferenceCreateRequest
        per::write_selection(&mut stream, CONFERENCE_REQUEST_USER_DATA_SELECTION)?;
        // ConferenceCreateRequest::ConferenceName
        per::write_numeric_string(&mut stream, CONFERENCE_NAME, 1)?;
        per::write_padding(&mut stream, 1)?;
        // UserData (SET OF SEQUENCE)
        // one set of UserData
        per::write_number_of_sets(&mut stream, USER_DATA_NUMBER_OF_SETS)?;
        // select h221NonStandard
        per::write_choice(&mut stream, USER_DATA_H221_NON_STANDARD_CHOICE)?;
        // h221NonStandard: client-to-server H.221 key, "Duca"
        per::write_octet_string(
            &mut stream,
            CONFERENCE_REQUEST_CLIENT_TO_SERVER_H221_NON_STANDARD,
            H221_NON_STANDARD_MIN_LENGTH,
        )?;
        // H221NonStandardIdentifier (octet string)
        per::write_length(&mut stream, gcc_blocks_buffer_length as u16)?;
        self.gcc_blocks.to_buffer(&mut stream)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let gcc_blocks_buffer_length = self.gcc_blocks.buffer_length() as u16;
        per::SIZEOF_CHOICE
            + CONFERENCE_REQUEST_OBJECT_ID.len()
            + per::sizeof_length(CONFERENCE_REQUEST_CONNECT_PDU_SIZE + gcc_blocks_buffer_length)
            + CONFERENCE_REQUEST_CONNECT_PDU_SIZE as usize
            + per::sizeof_length(gcc_blocks_buffer_length)
            + gcc_blocks_buffer_length as usize
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConferenceCreateResponse {
    pub user_id: u16,
    pub gcc_blocks: ServerGccBlocks,
}

impl PduParsing for ConferenceCreateResponse {
    type Error = GccError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        // ConnectData::Key: select type OBJECT_IDENTIFIER
        if per::read_choice(&mut stream)? != OBJECT_IDENTIFIER_KEY {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got unexpected ConnectData Key",
            )));
        }
        // ConnectData::Key: value
        if per::read_object_id(&mut stream)? != CONFERENCE_REQUEST_OBJECT_ID {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got invalid ConnectData value",
            )));
        };
        // ConnectData::connectPDU: length (MUST be ignored by the client according to [MS-RDPBCGR])
        let _length = per::read_length(&mut stream)?;
        // ConnectGCCPDU (CHOICE): Select conferenceCreateResponse (1) of type ConferenceCreateResponse
        if per::read_choice(&mut stream)? != CONNECT_GCC_PDU_CONFERENCE_RESPONSE_CHOICE {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got unexpected ConnectGCCPDU choice",
            )));
        }
        // ConferenceCreateResponse::nodeID (UserID)
        let user_id = per::read_u16(&mut stream, CONFERENCE_REQUEST_U16_MIN)?;
        // ConferenceCreateResponse::tag (INTEGER)
        if per::read_u32(&mut stream)? != CONFERENCE_RESPONSE_TAG {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got unexpected ConferenceCreateResponse tag",
            )));
        }
        // ConferenceCreateResponse::result (ENUMERATED)
        if per::read_enum(&mut stream, mcs::RESULT_ENUM_LENGTH)? != CONFERENCE_RESPONSE_RESULT {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got invalid ConferenceCreateResponse result",
            )));
        }
        if per::read_number_of_sets(&mut stream)? != USER_DATA_NUMBER_OF_SETS {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got invalid ConferenceCreateResponse number of sets (expected 1)",
            )));
        }
        // select h221NonStandard
        if per::read_choice(&mut stream)? != USER_DATA_H221_NON_STANDARD_CHOICE {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got unexpected UserData choice (expected H221NonStandard)",
            )));
        }
        // h221NonStandard, server-to-client H.221 key, "McDn"
        if per::read_octet_string(&mut stream, H221_NON_STANDARD_MIN_LENGTH)?
            != CONFERENCE_REQUEST_SERVER_TO_CLIENT_H221_NON_STANDARD
        {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got invalid H221NonStandard server-to-client key",
            )));
        }
        let (_gcc_blocks_buffer_length, _) = per::read_length(&mut stream)?;
        let gcc_blocks = ServerGccBlocks::from_buffer(&mut stream)?;

        Ok(Self { user_id, gcc_blocks })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let gcc_blocks_buffer_length = self.gcc_blocks.buffer_length();

        // ConnectData::Key: select type OBJECT_IDENTIFIER
        per::write_choice(&mut stream, OBJECT_IDENTIFIER_KEY)?;
        // ConnectData::Key: value
        per::write_object_id(&mut stream, CONFERENCE_REQUEST_OBJECT_ID)?;

        // ConnectData::connectPDU: length (MUST be ignored by the client according to [MS-RDPBCGR])
        per::write_length(
            &mut stream,
            gcc_blocks_buffer_length as u16 + CONFERENCE_RESPONSE_CONNECT_PDU_SIZE,
        )?;
        // ConnectGCCPDU (CHOICE): Select conferenceCreateResponse (1) of type ConferenceCreateResponse
        per::write_choice(&mut stream, CONNECT_GCC_PDU_CONFERENCE_RESPONSE_CHOICE)?;
        // ConferenceCreateResponse::nodeID (UserID)
        per::write_u16(&mut stream, self.user_id, CONFERENCE_REQUEST_U16_MIN)?;
        // ConferenceCreateResponse::tag (INTEGER)
        per::write_u32(&mut stream, CONFERENCE_RESPONSE_TAG)?;
        // ConferenceCreateResponse::result (ENUMERATED)
        per::write_enum(&mut stream, CONFERENCE_RESPONSE_RESULT)?;
        per::write_number_of_sets(&mut stream, USER_DATA_NUMBER_OF_SETS)?;
        // select h221NonStandard
        per::write_choice(&mut stream, USER_DATA_H221_NON_STANDARD_CHOICE)?;
        // h221NonStandard, server-to-client H.221 key, "McDn"
        per::write_octet_string(
            &mut stream,
            CONFERENCE_REQUEST_SERVER_TO_CLIENT_H221_NON_STANDARD,
            H221_NON_STANDARD_MIN_LENGTH as usize,
        )?;
        // H221NonStandardIdentifier (octet string)
        per::write_length(&mut stream, gcc_blocks_buffer_length as u16)?;
        self.gcc_blocks.to_buffer(&mut stream)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let gcc_blocks_buffer_length = self.gcc_blocks.buffer_length() as u16;
        per::SIZEOF_CHOICE
            + CONFERENCE_REQUEST_OBJECT_ID.len()
            + per::sizeof_length(CONFERENCE_RESPONSE_CONNECT_PDU_SIZE + gcc_blocks_buffer_length)
            + CONFERENCE_RESPONSE_CONNECT_PDU_SIZE as usize
            + per::sizeof_length(gcc_blocks_buffer_length)
            + gcc_blocks_buffer_length as usize
    }
}
