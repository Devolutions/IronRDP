use super::{ClientGccBlocks, ServerGccBlocks};
use crate::{mcs, per, PduDecode, PduEncode, PduResult};
use ironrdp_core::{ReadCursor, WriteCursor};

const CONFERENCE_REQUEST_OBJECT_ID: [u8; 6] = [0, 0, 20, 124, 0, 1];
const CONFERENCE_REQUEST_CLIENT_TO_SERVER_H221_NON_STANDARD: &[u8; 4] = b"Duca";
const CONFERENCE_REQUEST_SERVER_TO_CLIENT_H221_NON_STANDARD: &[u8; 4] = b"McDn";
const CONFERENCE_REQUEST_U16_MIN: u16 = 1001;

const CONFERENCE_REQUEST_CONNECT_PDU_SIZE: usize = 12;
const CONFERENCE_RESPONSE_CONNECT_PDU_SIZE: usize = 13;
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

impl ConferenceCreateRequest {
    const NAME: &'static str = "ConferenceCreateRequest";
}

impl PduEncode for ConferenceCreateRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in:dst, size: self.size());

        let gcc_blocks_buffer_length = self.gcc_blocks.size();

        // ConnectData::Key: select type OBJECT_IDENTIFIER
        per::write_choice(dst, OBJECT_IDENTIFIER_KEY);
        // ConnectData::Key: value
        per::write_object_id(dst, CONFERENCE_REQUEST_OBJECT_ID);

        // ConnectData::connectPDU: length
        per::write_length(
            dst,
            cast_length!(
                "gccBlocksLen",
                gcc_blocks_buffer_length + CONFERENCE_REQUEST_CONNECT_PDU_SIZE
            )?,
        );
        // ConnectGCCPDU (CHOICE): Select conferenceCreateRequest (0) of type ConferenceCreateRequest
        per::write_choice(dst, CONNECT_GCC_PDU_CONFERENCE_REQUEST_CHOICE);
        // ConferenceCreateRequest::Selection: select optional userData from ConferenceCreateRequest
        per::write_selection(dst, CONFERENCE_REQUEST_USER_DATA_SELECTION);
        // ConferenceCreateRequest::ConferenceName
        per::write_numeric_string(dst, CONFERENCE_NAME, 1).map_err(|e| custom_err!("confName", e))?;
        per::write_padding(dst, 1);
        // UserData (SET OF SEQUENCE)
        // one set of UserData
        per::write_number_of_sets(dst, USER_DATA_NUMBER_OF_SETS);
        // select h221NonStandard
        per::write_choice(dst, USER_DATA_H221_NON_STANDARD_CHOICE);
        // h221NonStandard: client-to-server H.221 key, "Duca"
        per::write_octet_string(
            dst,
            CONFERENCE_REQUEST_CLIENT_TO_SERVER_H221_NON_STANDARD,
            H221_NON_STANDARD_MIN_LENGTH,
        )
        .map_err(|e| custom_err!("client-to-server", e))?;
        // H221NonStandardIdentifier (octet string)
        per::write_length(dst, cast_length!("gccBlocksLen", gcc_blocks_buffer_length)?);
        self.gcc_blocks.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let gcc_blocks_buffer_length = self.gcc_blocks.size();
        per::CHOICE_SIZE
            + CONFERENCE_REQUEST_OBJECT_ID.len()
            + per::sizeof_length(
                cast_length!(
                    "gccBlocksLen",
                    CONFERENCE_REQUEST_CONNECT_PDU_SIZE + gcc_blocks_buffer_length
                )
                .unwrap(),
            )
            + CONFERENCE_REQUEST_CONNECT_PDU_SIZE
            + per::sizeof_length(cast_length!("gccBlocksLen", gcc_blocks_buffer_length).unwrap())
            + gcc_blocks_buffer_length
    }
}

impl<'de> PduDecode<'de> for ConferenceCreateRequest {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        // ConnectData

        // ConnectData::Key: select object (0) of type OBJECT_IDENTIFIER
        ensure_size!(in: src, size: per::CHOICE_SIZE);
        if per::read_choice(src) != OBJECT_IDENTIFIER_KEY {
            return Err(invalid_field_err!("ConnectData::Key", "Got unexpected ConnectData key"));
        }
        // ConnectData::Key: value (OBJECT_IDENTIFIER)
        if per::read_object_id(src).map_err(|e| custom_err!("value", e))? != CONFERENCE_REQUEST_OBJECT_ID {
            return Err(invalid_field_err!(
                "ConnectData::Key",
                "Got unexpected ConnectData key value"
            ));
        }

        // ConnectData::connectPDU: length
        let _length = per::read_length(src).map_err(|e| custom_err!("len", e))?;
        // ConnectGCCPDU (CHOICE): Select conferenceCreateRequest (0) of type ConferenceCreateRequest
        ensure_size!(in: src, size: per::CHOICE_SIZE);
        if per::read_choice(src) != CONNECT_GCC_PDU_CONFERENCE_REQUEST_CHOICE {
            return Err(invalid_field_err!(
                "ConnectData::connectPdu",
                "Got invalid ConnectGCCPDU choice (expected ConferenceCreateRequest)"
            ));
        }
        // ConferenceCreateRequest::Selection: select optional userData from ConferenceCreateRequest
        ensure_size!(in: src, size: per::CHOICE_SIZE);
        if per::read_selection(src) != CONFERENCE_REQUEST_USER_DATA_SELECTION {
            return Err(invalid_field_err!(
                "ConferenceCreateRequest::Selection",
                "Got invalid ConferenceCreateRequest selection (expected UserData)",
            ));
        }
        // ConferenceCreateRequest::ConferenceName
        per::read_numeric_string(src, 1).map_err(|e| custom_err!("confName", e))?;
        // padding
        per::read_padding(src, 1);

        // UserData (SET OF SEQUENCE)
        // one set of UserData
        ensure_size!(in: src, size: per::CHOICE_SIZE);
        if per::read_number_of_sets(src) != USER_DATA_NUMBER_OF_SETS {
            return Err(invalid_field_err!(
                "ConferenceCreateRequest",
                "Got invalid ConferenceCreateRequest number of sets (expected 1)",
            ));
        }
        // select h221NonStandard
        ensure_size!(in: src, size: per::CHOICE_SIZE);
        if per::read_choice(src) != USER_DATA_H221_NON_STANDARD_CHOICE {
            return Err(invalid_field_err!(
                "ConferenceCreateRequest",
                "Expected UserData H221NonStandard choice",
            ));
        }
        // h221NonStandard: client-to-server H.221 key, "Duca"
        if per::read_octet_string(src, H221_NON_STANDARD_MIN_LENGTH).map_err(|e| custom_err!("client-to-server", e))?
            != CONFERENCE_REQUEST_CLIENT_TO_SERVER_H221_NON_STANDARD
        {
            return Err(invalid_field_err!(
                "ConferenceCreateRequest",
                "Got invalid H221NonStandard client-to-server key",
            ));
        }
        // H221NonStandardIdentifier (octet string)
        let (_gcc_blocks_buffer_length, _) = per::read_length(src).map_err(|e| custom_err!("len", e))?;
        let gcc_blocks = ClientGccBlocks::decode(src)?;

        Ok(Self { gcc_blocks })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConferenceCreateResponse {
    pub user_id: u16,
    pub gcc_blocks: ServerGccBlocks,
}

impl ConferenceCreateResponse {
    const NAME: &'static str = "ConferenceCreateResponse";
}

impl PduEncode for ConferenceCreateResponse {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        let gcc_blocks_buffer_length = self.gcc_blocks.size();

        // ConnectData::Key: select type OBJECT_IDENTIFIER
        per::write_choice(dst, OBJECT_IDENTIFIER_KEY);
        // ConnectData::Key: value
        per::write_object_id(dst, CONFERENCE_REQUEST_OBJECT_ID);

        // ConnectData::connectPDU: length (MUST be ignored by the client according to [MS-RDPBCGR])
        per::write_length(
            dst,
            cast_length!(
                "gccBlocksLen",
                gcc_blocks_buffer_length + CONFERENCE_RESPONSE_CONNECT_PDU_SIZE + 1
            )?,
        );
        // ConnectGCCPDU (CHOICE): Select conferenceCreateResponse (1) of type ConferenceCreateResponse
        per::write_choice(dst, CONNECT_GCC_PDU_CONFERENCE_RESPONSE_CHOICE);
        // ConferenceCreateResponse::nodeID (UserID)
        per::write_u16(dst, self.user_id, CONFERENCE_REQUEST_U16_MIN).map_err(|e| custom_err!("userId", e))?;
        // ConferenceCreateResponse::tag (INTEGER)
        per::write_u32(dst, CONFERENCE_RESPONSE_TAG);
        // ConferenceCreateResponse::result (ENUMERATED)
        per::write_enum(dst, CONFERENCE_RESPONSE_RESULT);
        per::write_number_of_sets(dst, USER_DATA_NUMBER_OF_SETS);
        // select h221NonStandard
        per::write_choice(dst, USER_DATA_H221_NON_STANDARD_CHOICE);
        // h221NonStandard, server-to-client H.221 key, "McDn"
        per::write_octet_string(
            dst,
            CONFERENCE_REQUEST_SERVER_TO_CLIENT_H221_NON_STANDARD,
            H221_NON_STANDARD_MIN_LENGTH,
        )
        .map_err(|e| custom_err!("server-to-client", e))?;
        // H221NonStandardIdentifier (octet string)
        per::write_length(dst, gcc_blocks_buffer_length as u16);
        self.gcc_blocks.encode(dst)?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let gcc_blocks_buffer_length = self.gcc_blocks.size();
        per::CHOICE_SIZE
            + CONFERENCE_REQUEST_OBJECT_ID.len()
            + per::sizeof_length(
                cast_length!(
                    "gccBlocksLen",
                    CONFERENCE_RESPONSE_CONNECT_PDU_SIZE + gcc_blocks_buffer_length
                )
                .unwrap(),
            )
            + CONFERENCE_RESPONSE_CONNECT_PDU_SIZE
            + per::sizeof_length(cast_length!("gccBlocksLen", gcc_blocks_buffer_length).unwrap())
            + gcc_blocks_buffer_length
    }
}

impl<'de> PduDecode<'de> for ConferenceCreateResponse {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        // ConnectData::Key: select type OBJECT_IDENTIFIER
        ensure_size!(in: src, size: per::CHOICE_SIZE);
        if per::read_choice(src) != OBJECT_IDENTIFIER_KEY {
            return Err(invalid_field_err!("ConnectData::Key", "Got unexpected ConnectData key"));
        }
        // ConnectData::Key: value
        if per::read_object_id(src).map_err(|e| custom_err!("value", e))? != CONFERENCE_REQUEST_OBJECT_ID {
            return Err(invalid_field_err!(
                "ConnectData::Key",
                "Got unexpected ConnectData key value"
            ));
        };
        // ConnectData::connectPDU: length (MUST be ignored by the client according to [MS-RDPBCGR])
        let _length = per::read_length(src).map_err(|e| custom_err!("len", e))?;
        // ConnectGCCPDU (CHOICE): Select conferenceCreateResponse (1) of type ConferenceCreateResponse
        ensure_size!(in: src, size: per::CHOICE_SIZE);
        if per::read_choice(src) != CONNECT_GCC_PDU_CONFERENCE_RESPONSE_CHOICE {
            return Err(invalid_field_err!(
                "ConnectData::connectPdu",
                "Got invalid ConnectGCCPDU choice (expected ConferenceCreateResponse)"
            ));
        }
        // ConferenceCreateResponse::nodeID (UserID)
        let user_id = per::read_u16(src, CONFERENCE_REQUEST_U16_MIN).map_err(|e| custom_err!("userId", e))?;
        // ConferenceCreateResponse::tag (INTEGER)
        if per::read_u32(src).map_err(|e| custom_err!("tag", e))? != CONFERENCE_RESPONSE_TAG {
            return Err(invalid_field_err!(
                "ConferenceCreateResponse::tag",
                "Got unexpected ConferenceCreateResponse tag",
            ));
        }
        // ConferenceCreateResponse::result (ENUMERATED)
        if per::read_enum(src, mcs::RESULT_ENUM_LENGTH).map_err(|e| custom_err!("result", e))?
            != CONFERENCE_RESPONSE_RESULT
        {
            return Err(invalid_field_err!(
                "ConferenceCreateResponse::result",
                "Got invalid ConferenceCreateResponse result",
            ));
        }
        ensure_size!(in: src, size: per::CHOICE_SIZE);
        if per::read_number_of_sets(src) != USER_DATA_NUMBER_OF_SETS {
            return Err(invalid_field_err!(
                "ConferenceCreateResponse",
                "Got invalid ConferenceCreateResponse number of sets (expected 1)",
            ));
        }
        // select h221NonStandard
        ensure_size!(in: src, size: per::CHOICE_SIZE);
        if per::read_choice(src) != USER_DATA_H221_NON_STANDARD_CHOICE {
            return Err(invalid_field_err!(
                "ConferenceCreateResponse",
                "Expected UserData H221NonStandard choice",
            ));
        }
        // h221NonStandard, server-to-client H.221 key, "McDn"
        if per::read_octet_string(src, H221_NON_STANDARD_MIN_LENGTH).map_err(|e| custom_err!("server-to-client", e))?
            != CONFERENCE_REQUEST_SERVER_TO_CLIENT_H221_NON_STANDARD
        {
            return Err(invalid_field_err!(
                "ConferenceCreateResponse",
                "Got invalid H221NonStandard server-to-client key",
            ));
        }
        let (_gcc_blocks_buffer_length, _) = per::read_length(src).map_err(|e| custom_err!("len", e))?;
        let gcc_blocks = ServerGccBlocks::decode(src)?;

        Ok(Self { user_id, gcc_blocks })
    }
}
