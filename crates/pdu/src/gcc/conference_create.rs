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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConferenceCreateRequest {
    pub gcc_blocks: ClientGccBlocks,
}

impl PduParsing for ConferenceCreateRequest {
    type Error = GccError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        // ConnectData

        // ConnectData::Key: select object (0) of type OBJECT_IDENTIFIER
        if per::legacy::read_choice(&mut stream)? != OBJECT_IDENTIFIER_KEY {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Got unexpected ConnectData Key",
            )));
        }
        // ConnectData::Key: value (OBJECT_IDENTIFIER)
        if per::legacy::read_object_id(&mut stream)? != CONFERENCE_REQUEST_OBJECT_ID {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Got unexpected ConnectData key value",
            )));
        }

        // ConnectData::connectPDU: length
        let _length = per::legacy::read_length(&mut stream)?;
        // ConnectGCCPDU (CHOICE): Select conferenceCreateRequest (0) of type ConferenceCreateRequest
        if per::legacy::read_choice(&mut stream)? != CONNECT_GCC_PDU_CONFERENCE_REQUEST_CHOICE {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Got invalid ConnectGCCPDU choice (expected ConferenceCreateRequest)",
            )));
        }
        // ConferenceCreateRequest::Selection: select optional userData from ConferenceCreateRequest
        if per::legacy::read_selection(&mut stream)? != CONFERENCE_REQUEST_USER_DATA_SELECTION {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Got invalid ConferenceCreateRequest selection (expected UserData)",
            )));
        }
        // ConferenceCreateRequest::ConferenceName
        per::legacy::read_numeric_string(&mut stream, 1)?;
        // padding
        per::legacy::read_padding(&mut stream, 1)?;

        // UserData (SET OF SEQUENCE)
        // one set of UserData
        if per::legacy::read_number_of_sets(&mut stream)? != USER_DATA_NUMBER_OF_SETS {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Got invalid ConferenceCreateRequest number of sets (expected 1)",
            )));
        }
        // select h221NonStandard
        if per::legacy::read_choice(&mut stream)? != USER_DATA_H221_NON_STANDARD_CHOICE {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Expected UserData H221NonStandard choice",
            )));
        }
        // h221NonStandard: client-to-server H.221 key, "Duca"
        if per::legacy::read_octet_string(&mut stream, H221_NON_STANDARD_MIN_LENGTH)?
            != CONFERENCE_REQUEST_CLIENT_TO_SERVER_H221_NON_STANDARD
        {
            return Err(GccError::InvalidConferenceCreateRequest(String::from(
                "Got invalid H221NonStandard client-to-server key",
            )));
        }
        // H221NonStandardIdentifier (octet string)
        let (_gcc_blocks_buffer_length, _) = per::legacy::read_length(&mut stream)?;
        let gcc_blocks = ClientGccBlocks::from_buffer(&mut stream)?;

        Ok(Self { gcc_blocks })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let gcc_blocks_buffer_length = self.gcc_blocks.buffer_length();

        // ConnectData::Key: select type OBJECT_IDENTIFIER
        per::legacy::write_choice(&mut stream, OBJECT_IDENTIFIER_KEY)?;
        // ConnectData::Key: value
        per::legacy::write_object_id(&mut stream, CONFERENCE_REQUEST_OBJECT_ID)?;

        // ConnectData::connectPDU: length
        per::legacy::write_length(
            &mut stream,
            gcc_blocks_buffer_length as u16 + CONFERENCE_REQUEST_CONNECT_PDU_SIZE,
        )?;
        // ConnectGCCPDU (CHOICE): Select conferenceCreateRequest (0) of type ConferenceCreateRequest
        per::legacy::write_choice(&mut stream, CONNECT_GCC_PDU_CONFERENCE_REQUEST_CHOICE)?;
        // ConferenceCreateRequest::Selection: select optional userData from ConferenceCreateRequest
        per::legacy::write_selection(&mut stream, CONFERENCE_REQUEST_USER_DATA_SELECTION)?;
        // ConferenceCreateRequest::ConferenceName
        per::legacy::write_numeric_string(&mut stream, CONFERENCE_NAME, 1)?;
        per::legacy::write_padding(&mut stream, 1)?;
        // UserData (SET OF SEQUENCE)
        // one set of UserData
        per::legacy::write_number_of_sets(&mut stream, USER_DATA_NUMBER_OF_SETS)?;
        // select h221NonStandard
        per::legacy::write_choice(&mut stream, USER_DATA_H221_NON_STANDARD_CHOICE)?;
        // h221NonStandard: client-to-server H.221 key, "Duca"
        per::legacy::write_octet_string(
            &mut stream,
            CONFERENCE_REQUEST_CLIENT_TO_SERVER_H221_NON_STANDARD,
            H221_NON_STANDARD_MIN_LENGTH,
        )?;
        // H221NonStandardIdentifier (octet string)
        per::legacy::write_length(&mut stream, gcc_blocks_buffer_length as u16)?;
        self.gcc_blocks.to_buffer(&mut stream)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let gcc_blocks_buffer_length = self.gcc_blocks.buffer_length() as u16;
        per::CHOICE_SIZE
            + CONFERENCE_REQUEST_OBJECT_ID.len()
            + per::sizeof_length(CONFERENCE_REQUEST_CONNECT_PDU_SIZE + gcc_blocks_buffer_length)
            + CONFERENCE_REQUEST_CONNECT_PDU_SIZE as usize
            + per::sizeof_length(gcc_blocks_buffer_length)
            + gcc_blocks_buffer_length as usize
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConferenceCreateResponse {
    pub user_id: u16,
    pub gcc_blocks: ServerGccBlocks,
}

impl PduParsing for ConferenceCreateResponse {
    type Error = GccError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        // ConnectData::Key: select type OBJECT_IDENTIFIER
        if per::legacy::read_choice(&mut stream)? != OBJECT_IDENTIFIER_KEY {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got unexpected ConnectData Key",
            )));
        }
        // ConnectData::Key: value
        if per::legacy::read_object_id(&mut stream)? != CONFERENCE_REQUEST_OBJECT_ID {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got invalid ConnectData value",
            )));
        };
        // ConnectData::connectPDU: length (MUST be ignored by the client according to [MS-RDPBCGR])
        let _length = per::legacy::read_length(&mut stream)?;
        // ConnectGCCPDU (CHOICE): Select conferenceCreateResponse (1) of type ConferenceCreateResponse
        if per::legacy::read_choice(&mut stream)? != CONNECT_GCC_PDU_CONFERENCE_RESPONSE_CHOICE {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got unexpected ConnectGCCPDU choice",
            )));
        }
        // ConferenceCreateResponse::nodeID (UserID)
        let user_id = per::legacy::read_u16(&mut stream, CONFERENCE_REQUEST_U16_MIN)?;
        // ConferenceCreateResponse::tag (INTEGER)
        if per::legacy::read_u32(&mut stream)? != CONFERENCE_RESPONSE_TAG {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got unexpected ConferenceCreateResponse tag",
            )));
        }
        // ConferenceCreateResponse::result (ENUMERATED)
        if per::legacy::read_enum(&mut stream, mcs::RESULT_ENUM_LENGTH)? != CONFERENCE_RESPONSE_RESULT {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got invalid ConferenceCreateResponse result",
            )));
        }
        if per::legacy::read_number_of_sets(&mut stream)? != USER_DATA_NUMBER_OF_SETS {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got invalid ConferenceCreateResponse number of sets (expected 1)",
            )));
        }
        // select h221NonStandard
        if per::legacy::read_choice(&mut stream)? != USER_DATA_H221_NON_STANDARD_CHOICE {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got unexpected UserData choice (expected H221NonStandard)",
            )));
        }
        // h221NonStandard, server-to-client H.221 key, "McDn"
        if per::legacy::read_octet_string(&mut stream, H221_NON_STANDARD_MIN_LENGTH)?
            != CONFERENCE_REQUEST_SERVER_TO_CLIENT_H221_NON_STANDARD
        {
            return Err(GccError::InvalidConferenceCreateResponse(String::from(
                "Got invalid H221NonStandard server-to-client key",
            )));
        }
        let (_gcc_blocks_buffer_length, _) = per::legacy::read_length(&mut stream)?;
        let gcc_blocks = ServerGccBlocks::from_buffer(&mut stream)?;

        Ok(Self { user_id, gcc_blocks })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        let gcc_blocks_buffer_length = self.gcc_blocks.buffer_length();

        // ConnectData::Key: select type OBJECT_IDENTIFIER
        per::legacy::write_choice(&mut stream, OBJECT_IDENTIFIER_KEY)?;
        // ConnectData::Key: value
        per::legacy::write_object_id(&mut stream, CONFERENCE_REQUEST_OBJECT_ID)?;

        // ConnectData::connectPDU: length (MUST be ignored by the client according to [MS-RDPBCGR])
        per::legacy::write_length(
            &mut stream,
            gcc_blocks_buffer_length as u16 + CONFERENCE_RESPONSE_CONNECT_PDU_SIZE,
        )?;
        // ConnectGCCPDU (CHOICE): Select conferenceCreateResponse (1) of type ConferenceCreateResponse
        per::legacy::write_choice(&mut stream, CONNECT_GCC_PDU_CONFERENCE_RESPONSE_CHOICE)?;
        // ConferenceCreateResponse::nodeID (UserID)
        per::legacy::write_u16(&mut stream, self.user_id, CONFERENCE_REQUEST_U16_MIN)?;
        // ConferenceCreateResponse::tag (INTEGER)
        per::legacy::write_u32(&mut stream, CONFERENCE_RESPONSE_TAG)?;
        // ConferenceCreateResponse::result (ENUMERATED)
        per::legacy::write_enum(&mut stream, CONFERENCE_RESPONSE_RESULT)?;
        per::legacy::write_number_of_sets(&mut stream, USER_DATA_NUMBER_OF_SETS)?;
        // select h221NonStandard
        per::legacy::write_choice(&mut stream, USER_DATA_H221_NON_STANDARD_CHOICE)?;
        // h221NonStandard, server-to-client H.221 key, "McDn"
        per::legacy::write_octet_string(
            &mut stream,
            CONFERENCE_REQUEST_SERVER_TO_CLIENT_H221_NON_STANDARD,
            H221_NON_STANDARD_MIN_LENGTH,
        )?;
        // H221NonStandardIdentifier (octet string)
        per::legacy::write_length(&mut stream, gcc_blocks_buffer_length as u16)?;
        self.gcc_blocks.to_buffer(&mut stream)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let gcc_blocks_buffer_length = self.gcc_blocks.buffer_length() as u16;
        per::CHOICE_SIZE
            + CONFERENCE_REQUEST_OBJECT_ID.len()
            + per::sizeof_length(CONFERENCE_RESPONSE_CONNECT_PDU_SIZE + gcc_blocks_buffer_length)
            + CONFERENCE_RESPONSE_CONNECT_PDU_SIZE as usize
            + per::sizeof_length(gcc_blocks_buffer_length)
            + gcc_blocks_buffer_length as usize
    }
}
