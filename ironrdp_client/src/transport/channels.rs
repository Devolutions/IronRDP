use std::io;

use ironrdp::{rdp::vc, PduParsing};

use super::{DataTransport, Decoder, Encoder, McsTransport, SendDataContextTransport};
use crate::{RdpError, RdpResult};

#[derive(Copy, Clone, Debug)]
pub struct ChannelIdentificators {
    pub initiator_id: u16,
    pub channel_id: u16,
}

#[derive(Copy, Clone, Debug)]
pub struct StaticVirtualChannelTransport {
    channel_ids: ChannelIdentificators,
    send_data_context_transport: SendDataContextTransport,
}

impl StaticVirtualChannelTransport {
    pub fn new() -> Self {
        Self {
            channel_ids: ChannelIdentificators {
                channel_id: 0,
                initiator_id: 0,
            },
            send_data_context_transport: SendDataContextTransport::new(
                McsTransport::new(DataTransport),
                0,
                0,
            ),
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
    ) -> RdpResult<()> {
        let channel_header = vc::ChannelPduHeader {
            total_length: channel_data_buffer.len() as u32,
            flags: vc::ChannelControlFlags::FLAG_FIRST | vc::ChannelControlFlags::FLAG_LAST,
        };

        let mut channel_buffer =
            Vec::with_capacity(channel_header.buffer_length() + channel_data_buffer.len());
        channel_header.to_buffer(&mut channel_buffer)?;
        channel_buffer.append(&mut channel_data_buffer);

        self.send_data_context_transport
            .set_channel_ids(self.channel_ids);

        self.send_data_context_transport
            .encode(channel_buffer, &mut stream)
    }
}

impl Decoder for StaticVirtualChannelTransport {
    type Item = (u16, Vec<u8>);
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let (channel_ids, mut channel_data_buffer) =
            self.send_data_context_transport.decode(&mut stream)?;
        self.channel_ids = channel_ids;
        let channel_header = vc::ChannelPduHeader::from_buffer(channel_data_buffer.as_slice())?;

        channel_data_buffer.drain(..channel_header.buffer_length());
        Ok((channel_ids.channel_id, channel_data_buffer))
    }
}

pub struct DynamicVirtualChannelTransport(StaticVirtualChannelTransport);

impl DynamicVirtualChannelTransport {
    pub fn new(svc_transport: StaticVirtualChannelTransport) -> Self {
        Self(svc_transport)
    }
}

impl Encoder for DynamicVirtualChannelTransport {
    type Item = vc::dvc::ClientPdu;
    type Error = RdpError;

    fn encode(&mut self, dvc_clien_pdu: Self::Item, mut stream: impl io::Write) -> RdpResult<()> {
        let mut dvc_clien_pdu_buf = Vec::with_capacity(dvc_clien_pdu.buffer_length());
        dvc_clien_pdu.to_buffer(&mut dvc_clien_pdu_buf)?;

        self.0.encode(dvc_clien_pdu_buf, &mut stream)
    }
}

impl Decoder for DynamicVirtualChannelTransport {
    type Item = vc::dvc::ServerPdu;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let (_channel_id, channel_data_buffer) = self.0.decode(&mut stream)?;
        let dvc_server_pdu = vc::dvc::ServerPdu::from_buffer(channel_data_buffer.as_slice())?;

        Ok(dvc_server_pdu)
    }
}
