use std::io;

use ironrdp::{rdp::vc, PduParsing};

use super::{Decoder, Encoder, SendDataContextTransport};
use crate::RdpError;

#[derive(Copy, Clone, Debug)]
pub struct ChannelIdentificators {
    pub initiator_id: u16,
    pub channel_id: u16,
}

#[derive(Clone, Debug)]
pub struct StaticVirtualChannelTransport {
    channel_ids: ChannelIdentificators,
    transport: SendDataContextTransport,
}

impl StaticVirtualChannelTransport {
    pub fn new(transport: SendDataContextTransport) -> Self {
        Self {
            channel_ids: ChannelIdentificators {
                channel_id: 0,
                initiator_id: 0,
            },
            transport,
        }
    }
}

impl Encoder for StaticVirtualChannelTransport {
    type Item = Vec<u8>;
    type Error = RdpError;

    fn encode(
        &mut self,
        mut channel_data_buffer: Self::Item,
        mut stream: impl io::Write,
    ) -> Result<(), RdpError> {
        let channel_header = vc::ChannelPduHeader {
            total_length: channel_data_buffer.len() as u32,
            flags: vc::ChannelControlFlags::FLAG_FIRST | vc::ChannelControlFlags::FLAG_LAST,
        };

        let mut channel_buffer =
            Vec::with_capacity(channel_header.buffer_length() + channel_data_buffer.len());
        channel_header.to_buffer(&mut channel_buffer)?;
        channel_buffer.append(&mut channel_data_buffer);

        self.transport.set_channel_ids(self.channel_ids);
        self.transport.encode(channel_buffer, &mut stream)
    }
}

impl Decoder for StaticVirtualChannelTransport {
    type Item = (u16, usize);
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> Result<Self::Item, RdpError> {
        let channel_ids = self.transport.decode(&mut stream)?;
        self.channel_ids = channel_ids;
        let channel_header = vc::ChannelPduHeader::from_buffer(&mut stream)?;

        Ok((channel_ids.channel_id, channel_header.total_length as usize))
    }
}

pub struct DynamicVirtualChannelTransport {
    transport: StaticVirtualChannelTransport,
    drdynvc_id: u16,
}

impl DynamicVirtualChannelTransport {
    pub fn new(transport: StaticVirtualChannelTransport, drdynvc_id: u16) -> Self {
        Self {
            transport,
            drdynvc_id,
        }
    }

    pub fn prepare_data_to_encode(
        dvc_pdu: vc::dvc::ClientPdu,
        extra_data: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, RdpError> {
        let mut full_data_buff = Vec::with_capacity(dvc_pdu.buffer_length());
        dvc_pdu.to_buffer(&mut full_data_buff)?;

        if let Some(mut extra_data) = extra_data {
            full_data_buff.append(&mut extra_data);
        }

        Ok(full_data_buff)
    }
}

impl Encoder for DynamicVirtualChannelTransport {
    type Item = Vec<u8>;
    type Error = RdpError;

    fn encode(
        &mut self,
        client_pdu_buff: Self::Item,
        mut stream: impl io::Write,
    ) -> Result<(), RdpError> {
        self.transport.encode(client_pdu_buff, &mut stream)
    }
}

impl Decoder for DynamicVirtualChannelTransport {
    type Item = vc::dvc::ServerPdu;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> Result<Self::Item, RdpError> {
        let (channel_id, dvc_data_size) = self.transport.decode(&mut stream)?;
        if self.drdynvc_id != channel_id {
            return Err(RdpError::InvalidChannelIdError(format!(
                "Expected drdynvc {} ID, got: {} ID",
                self.drdynvc_id, channel_id,
            )));
        }

        let dvc_server_pdu = vc::dvc::ServerPdu::from_buffer(&mut stream, dvc_data_size)?;

        Ok(dvc_server_pdu)
    }
}
