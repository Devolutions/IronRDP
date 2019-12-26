mod channels;
mod connection;

pub use self::{
    channels::{
        ChannelIdentificators, DynamicVirtualChannelTransport, StaticVirtualChannelTransport,
    },
    connection::{
        connect, EarlyUserAuthResult, ShareControlHeaderTransport, ShareDataHeaderTransport,
        TsRequestTransport,
    },
};

use std::{io, mem};

use bytes::BytesMut;
use ironrdp::PduParsing;

use crate::{RdpError, RdpResult};

pub trait Encoder {
    type Item;
    type Error;

    fn encode(&mut self, item: Self::Item, stream: impl io::Write) -> Result<(), Self::Error>;
}

pub trait Decoder {
    type Item;
    type Error;

    fn decode(&mut self, stream: impl io::Read) -> Result<Self::Item, Self::Error>;
}

#[derive(Default, Copy, Clone, Debug)]
pub struct DataTransport;

impl Encoder for DataTransport {
    type Item = BytesMut;
    type Error = RdpError;

    fn encode(&mut self, data: Self::Item, mut stream: impl io::Write) -> RdpResult<()> {
        ironrdp::Data::new(data.len()).to_buffer(&mut stream)?;
        stream.write_all(data.as_ref())?;
        stream.flush()?;

        Ok(())
    }
}

impl Decoder for DataTransport {
    type Item = BytesMut;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let data_pdu = ironrdp::Data::from_buffer(&mut stream)?;

        let mut data = BytesMut::with_capacity(data_pdu.data_length);
        data.resize(data_pdu.data_length, 0x00);
        stream.read_exact(&mut data)?;

        Ok(data)
    }
}

#[derive(Copy, Clone, Debug)]
pub struct McsTransport(DataTransport);

impl McsTransport {
    pub fn new(transport: DataTransport) -> Self {
        Self(transport)
    }
}

impl Encoder for McsTransport {
    type Item = ironrdp::McsPdu;
    type Error = RdpError;

    fn encode(&mut self, mcs_pdu: Self::Item, mut stream: impl io::Write) -> RdpResult<()> {
        let mut mcs_pdu_buf = BytesMut::with_capacity(mcs_pdu.buffer_length() as usize);
        mcs_pdu_buf.resize(mcs_pdu.buffer_length(), 0x00);
        mcs_pdu
            .to_buffer(mcs_pdu_buf.as_mut())
            .map_err(RdpError::McsError)?;

        self.0.encode(mcs_pdu_buf, &mut stream)
    }
}

impl Decoder for McsTransport {
    type Item = ironrdp::McsPdu;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let data = self.0.decode(&mut stream)?;

        let mcs_pdu = ironrdp::McsPdu::from_buffer(data.as_ref()).map_err(RdpError::McsError)?;

        Ok(mcs_pdu)
    }
}

#[derive(Clone, Debug)]
pub struct SendDataContextTransport {
    mcs_transport: McsTransport,
    state: SendDataContextTransportState,
    channel_ids: ChannelIdentificators,
    send_data_context_pdu: Vec<u8>,
}

impl SendDataContextTransport {
    pub fn new(mcs_transport: McsTransport, initiator_id: u16, channel_id: u16) -> Self {
        Self {
            mcs_transport,
            channel_ids: ChannelIdentificators {
                initiator_id,
                channel_id,
            },
            state: SendDataContextTransportState::ToDecode,
            send_data_context_pdu: Vec::new(),
        }
    }

    pub fn set_channel_ids(&mut self, channel_ids: ChannelIdentificators) {
        self.channel_ids = channel_ids;
    }

    pub fn set_decoded_context(
        &mut self,
        channel_ids: ChannelIdentificators,
        send_data_context_pdu: Vec<u8>,
    ) {
        self.set_channel_ids(channel_ids);

        self.send_data_context_pdu = send_data_context_pdu;
        self.state = SendDataContextTransportState::Decoded;
    }
}

impl Default for SendDataContextTransport {
    fn default() -> Self {
        Self {
            mcs_transport: McsTransport::new(DataTransport),
            channel_ids: ChannelIdentificators {
                initiator_id: 0,
                channel_id: 0,
            },
            state: SendDataContextTransportState::ToDecode,
            send_data_context_pdu: Vec::new(),
        }
    }
}

impl Encoder for SendDataContextTransport {
    type Item = Vec<u8>;
    type Error = RdpError;

    fn encode(
        &mut self,
        send_data_context_pdu: Self::Item,
        mut stream: impl io::Write,
    ) -> RdpResult<()> {
        let send_data_context = ironrdp::mcs::SendDataContext::new(
            send_data_context_pdu,
            self.channel_ids.initiator_id,
            self.channel_ids.channel_id,
        );

        let send_data_request = ironrdp::McsPdu::SendDataRequest(send_data_context);
        self.mcs_transport.encode(send_data_request, &mut stream)
    }
}

impl Decoder for SendDataContextTransport {
    type Item = (ChannelIdentificators, Vec<u8>);
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        match self.state {
            SendDataContextTransportState::ToDecode => {
                let mcs_pdu = self.mcs_transport.decode(&mut stream)?;

                match mcs_pdu {
                    ironrdp::McsPdu::SendDataIndication(send_data_context) => Ok((
                        ChannelIdentificators {
                            initiator_id: send_data_context.initiator_id,
                            channel_id: send_data_context.channel_id,
                        },
                        send_data_context.pdu,
                    )),
                    ironrdp::McsPdu::DisconnectProviderUltimatum(disconnect_reason) => {
                        Err(RdpError::UnexpectedDisconnection(format!(
                            "Server disconnection reason - {:?}",
                            disconnect_reason
                        )))
                    }
                    _ => Err(RdpError::UnexpectedPdu(format!(
                        "Expected Send Data Context PDU, got {:?}",
                        mcs_pdu.as_short_name()
                    ))),
                }
            }
            SendDataContextTransportState::Decoded => Ok((
                self.channel_ids,
                mem::replace(&mut self.send_data_context_pdu, Vec::new()),
            )),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum SendDataContextTransportState {
    ToDecode,
    Decoded,
}
