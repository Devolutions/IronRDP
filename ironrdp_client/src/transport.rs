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

use std::io;

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
    type Item = usize;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        let data_pdu = ironrdp::Data::from_buffer(&mut stream)?;

        Ok(data_pdu.data_length)
    }
}

#[derive(Copy, Clone, Debug)]
pub struct McsTransport(DataTransport);

impl McsTransport {
    pub fn new(transport: DataTransport) -> Self {
        Self(transport)
    }

    pub fn prepare_data_to_encode(
        mcs_pdu: ironrdp::McsPdu,
        extra_data: Option<Vec<u8>>,
    ) -> RdpResult<BytesMut> {
        let mut mcs_pdu_buff = BytesMut::with_capacity(mcs_pdu.buffer_length());
        mcs_pdu_buff.resize(mcs_pdu.buffer_length(), 0x00);
        mcs_pdu
            .to_buffer(mcs_pdu_buff.as_mut())
            .map_err(RdpError::McsError)?;

        if let Some(data) = extra_data {
            mcs_pdu_buff.extend_from_slice(&data);
        }

        Ok(mcs_pdu_buff)
    }
}

impl Encoder for McsTransport {
    type Item = BytesMut;
    type Error = RdpError;

    fn encode(&mut self, mcs_pdu_buff: Self::Item, mut stream: impl io::Write) -> RdpResult<()> {
        self.0.encode(mcs_pdu_buff, &mut stream)
    }
}

impl Decoder for McsTransport {
    type Item = ironrdp::McsPdu;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        self.0.decode(&mut stream)?;
        let mcs_pdu = ironrdp::McsPdu::from_buffer(&mut stream).map_err(RdpError::McsError)?;

        Ok(mcs_pdu)
    }
}

#[derive(Clone, Debug)]
pub struct SendDataContextTransport {
    mcs_transport: McsTransport,
    state: SendDataContextTransportState,
    channel_ids: ChannelIdentificators,
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
        }
    }

    pub fn set_channel_ids(&mut self, channel_ids: ChannelIdentificators) {
        self.channel_ids = channel_ids;
    }

    pub fn set_decoded_context(&mut self, channel_ids: ChannelIdentificators) {
        self.set_channel_ids(channel_ids);
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
        let send_data_context = ironrdp::mcs::SendDataContext {
            channel_id: self.channel_ids.channel_id,
            initiator_id: self.channel_ids.initiator_id,
            pdu_length: send_data_context_pdu.len(),
        };

        self.mcs_transport.encode(
            McsTransport::prepare_data_to_encode(
                ironrdp::McsPdu::SendDataRequest(send_data_context),
                Some(send_data_context_pdu),
            )?,
            &mut stream,
        )
    }
}

impl Decoder for SendDataContextTransport {
    type Item = ChannelIdentificators;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> RdpResult<Self::Item> {
        match self.state {
            SendDataContextTransportState::ToDecode => {
                let mcs_pdu = self.mcs_transport.decode(&mut stream)?;

                match mcs_pdu {
                    ironrdp::McsPdu::SendDataIndication(send_data_context) => {
                        Ok(ChannelIdentificators {
                            initiator_id: send_data_context.initiator_id,
                            channel_id: send_data_context.channel_id,
                        })
                    }
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
            SendDataContextTransportState::Decoded => Ok(self.channel_ids),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum SendDataContextTransportState {
    ToDecode,
    Decoded,
}
