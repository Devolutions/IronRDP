#[cfg(test)]
mod test;

use std::io;

use sspi::ber;

use super::{McsError, RESULT_ENUM_LENGTH};
use crate::{
    gcc::{
        conference_create::{ConferenceCreateRequest, ConferenceCreateResponse},
        Channel,
    },
    PduParsing,
};

const MCS_TYPE_CONNECT_INITIAL: u8 = 0x65;
const MCS_TYPE_CONNECT_RESPONSE: u8 = 0x66;

#[derive(Clone, Debug, PartialEq)]
pub struct ConnectInitial {
    pub conference_create_request: ConferenceCreateRequest,
    calling_domain_selector: Vec<u8>,
    called_domain_selector: Vec<u8>,
    upward_flag: bool,
    target_parameters: DomainParameters,
    min_parameters: DomainParameters,
    max_parameters: DomainParameters,
}

impl ConnectInitial {
    pub fn channel_names(&self) -> Vec<Channel> {
        self.conference_create_request.gcc_blocks.channel_names()
    }

    fn fields_buffer_ber_length(&self) -> u16 {
        ber::sizeof_octet_string(self.calling_domain_selector.len() as u16)
            + ber::sizeof_octet_string(self.called_domain_selector.len() as u16)
            + ber::SIZEOF_BOOL
            + (self.target_parameters.buffer_length()
                + self.min_parameters.buffer_length()
                + self.max_parameters.buffer_length()) as u16
            + ber::sizeof_octet_string(self.conference_create_request.buffer_length() as u16)
    }
}

impl PduParsing for ConnectInitial {
    type Error = McsError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, McsError> {
        ber::read_application_tag(&mut stream, MCS_TYPE_CONNECT_INITIAL)?;
        let calling_domain_selector = ber::read_octet_string(&mut stream)?;
        let called_domain_selector = ber::read_octet_string(&mut stream)?;
        let upward_flag = ber::read_bool(&mut stream)?;
        let target_parameters = DomainParameters::from_buffer(&mut stream)?;
        let min_parameters = DomainParameters::from_buffer(&mut stream)?;
        let max_parameters = DomainParameters::from_buffer(&mut stream)?;
        let _user_data_buffer_length = ber::read_octet_string_tag(&mut stream)?;
        let conference_create_request = ConferenceCreateRequest::from_buffer(&mut stream)?;

        Ok(Self {
            conference_create_request,
            calling_domain_selector,
            called_domain_selector,
            upward_flag,
            target_parameters,
            min_parameters,
            max_parameters,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), McsError> {
        ber::write_application_tag(
            &mut stream,
            MCS_TYPE_CONNECT_INITIAL,
            self.fields_buffer_ber_length(),
        )?;
        ber::write_octet_string(&mut stream, self.calling_domain_selector.as_ref())?;
        ber::write_octet_string(&mut stream, self.called_domain_selector.as_ref())?;
        ber::write_bool(&mut stream, self.upward_flag)?;
        self.target_parameters.to_buffer(&mut stream)?;
        self.min_parameters.to_buffer(&mut stream)?;
        self.max_parameters.to_buffer(&mut stream)?;
        ber::write_octet_string_tag(
            &mut stream,
            self.conference_create_request.buffer_length() as u16,
        )?;
        self.conference_create_request.to_buffer(&mut stream)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let fields_buffer_ber_length = self.fields_buffer_ber_length();

        (fields_buffer_ber_length
            + ber::sizeof_application_tag(MCS_TYPE_CONNECT_INITIAL, fields_buffer_ber_length))
            as usize
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConnectResponse {
    pub conference_create_response: ConferenceCreateResponse,
    called_connect_id: u32,
    domain_parameters: DomainParameters,
}

impl ConnectResponse {
    pub fn channel_ids(&self) -> Vec<u16> {
        self.conference_create_response.gcc_blocks.channel_ids()
    }
    pub fn global_channel_id(&self) -> u16 {
        self.conference_create_response
            .gcc_blocks
            .global_channel_id()
    }

    fn fields_buffer_ber_length(&self) -> u16 {
        ber::SIZEOF_ENUMERATED
            + ber::sizeof_integer(self.called_connect_id)
            + self.domain_parameters.buffer_length() as u16
            + ber::sizeof_octet_string(self.conference_create_response.buffer_length() as u16)
    }
}

impl PduParsing for ConnectResponse {
    type Error = McsError;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, McsError> {
        ber::read_application_tag(&mut stream, MCS_TYPE_CONNECT_RESPONSE)?;
        ber::read_enumerated(&mut stream, RESULT_ENUM_LENGTH)?;
        let called_connect_id = ber::read_integer(&mut stream)? as u32;
        let domain_parameters = DomainParameters::from_buffer(&mut stream)?;
        let _user_data_buffer_length = ber::read_octet_string_tag(&mut stream)?;
        let conference_create_response = ConferenceCreateResponse::from_buffer(&mut stream)?;

        Ok(Self {
            called_connect_id,
            domain_parameters,
            conference_create_response,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), McsError> {
        ber::write_application_tag(
            &mut stream,
            MCS_TYPE_CONNECT_RESPONSE,
            self.fields_buffer_ber_length(),
        )?;
        ber::write_enumerated(&mut stream, 0)?;
        ber::write_integer(&mut stream, self.called_connect_id)?;
        self.domain_parameters.to_buffer(&mut stream)?;
        ber::write_octet_string_tag(
            &mut stream,
            self.conference_create_response.buffer_length() as u16,
        )?;
        self.conference_create_response.to_buffer(&mut stream)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let fields_buffer_ber_length = self.fields_buffer_ber_length();

        (fields_buffer_ber_length
            + ber::sizeof_application_tag(MCS_TYPE_CONNECT_RESPONSE, fields_buffer_ber_length))
            as usize
    }
}

#[derive(Clone, Debug, PartialEq)]
struct DomainParameters {
    max_channel_ids: u32,
    max_user_ids: u32,
    max_token_ids: u32,
    num_priorities: u32,
    min_throughput: u32,
    max_height: u32,
    max_mcs_pdu_size: u32,
    protocol_version: u32,
}

impl DomainParameters {
    fn fields_buffer_ber_length(&self) -> u16 {
        ber::sizeof_integer(self.max_channel_ids)
            + ber::sizeof_integer(self.max_user_ids)
            + ber::sizeof_integer(self.max_token_ids)
            + ber::sizeof_integer(self.num_priorities)
            + ber::sizeof_integer(self.min_throughput)
            + ber::sizeof_integer(self.max_height)
            + ber::sizeof_integer(self.max_mcs_pdu_size)
            + ber::sizeof_integer(self.protocol_version)
    }
}

impl PduParsing for DomainParameters {
    type Error = io::Error;

    fn from_buffer(mut stream: impl io::Read) -> io::Result<Self> {
        ber::read_sequence_tag(&mut stream)?;
        let max_channel_ids = ber::read_integer(&mut stream)? as u32;
        let max_user_ids = ber::read_integer(&mut stream)? as u32;
        let max_token_ids = ber::read_integer(&mut stream)? as u32;
        let num_priorities = ber::read_integer(&mut stream)? as u32;
        let min_throughput = ber::read_integer(&mut stream)? as u32;
        let max_height = ber::read_integer(&mut stream)? as u32;
        let max_mcs_pdu_size = ber::read_integer(&mut stream)? as u32;
        let protocol_version = ber::read_integer(&mut stream)? as u32;

        Ok(Self {
            max_channel_ids,
            max_user_ids,
            max_token_ids,
            num_priorities,
            min_throughput,
            max_height,
            max_mcs_pdu_size,
            protocol_version,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> io::Result<()> {
        ber::write_sequence_tag(&mut stream, self.fields_buffer_ber_length())?;
        ber::write_integer(&mut stream, self.max_channel_ids)?;
        ber::write_integer(&mut stream, self.max_user_ids)?;
        ber::write_integer(&mut stream, self.max_token_ids)?;
        ber::write_integer(&mut stream, self.num_priorities)?;
        ber::write_integer(&mut stream, self.min_throughput)?;
        ber::write_integer(&mut stream, self.max_height)?;
        ber::write_integer(&mut stream, self.max_mcs_pdu_size)?;
        ber::write_integer(&mut stream, self.protocol_version)?;

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let fields_buffer_ber_length = self.fields_buffer_ber_length();

        (fields_buffer_ber_length + ber::sizeof_sequence_tag(fields_buffer_ber_length)) as usize
    }
}
