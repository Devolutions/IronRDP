mod channels;
mod connection;

use ironrdp::rdp::SERVER_CHANNEL_ID;

pub use self::{
    channels::{
        ChannelIdentificators, DynamicVirtualChannelTransport, StaticVirtualChannelTransport,
    },
    connection::{connect, EarlyUserAuthResult, TsRequestTransport},
};

use std::io;

use bytes::BytesMut;
use ironrdp::{PduParsing, RdpPdu};
use log::warn;

use crate::RdpError;

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

#[derive(Copy, Clone, Debug)]
pub struct DataTransport {
    data_length: usize,
    state: TransportState,
}

impl Default for DataTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl DataTransport {
    pub fn new() -> Self {
        Self {
            data_length: 0,
            state: TransportState::ToDecode,
        }
    }

    pub fn set_decoded_context(&mut self, data_length: usize) {
        self.data_length = data_length;
        self.state = TransportState::Decoded;
    }
}

impl Encoder for DataTransport {
    type Item = BytesMut;
    type Error = RdpError;

    fn encode(&mut self, data: Self::Item, mut stream: impl io::Write) -> Result<(), RdpError> {
        ironrdp::Data::new(data.len()).to_buffer(&mut stream)?;
        stream.write_all(data.as_ref())?;
        stream.flush()?;

        Ok(())
    }
}

impl Decoder for DataTransport {
    type Item = usize;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> Result<Self::Item, RdpError> {
        match self.state {
            TransportState::ToDecode => {
                let data_pdu = ironrdp::Data::from_buffer(&mut stream)?;

                Ok(data_pdu.data_length)
            }
            TransportState::Decoded => Ok(self.data_length),
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct McsTransport(pub DataTransport);

impl McsTransport {
    pub fn new(transport: DataTransport) -> Self {
        Self(transport)
    }

    pub fn prepare_data_to_encode(
        mcs_pdu: ironrdp::McsPdu,
        extra_data: Option<Vec<u8>>,
    ) -> Result<BytesMut, RdpError> {
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

    fn encode(
        &mut self,
        mcs_pdu_buff: Self::Item,
        mut stream: impl io::Write,
    ) -> Result<(), RdpError> {
        self.0.encode(mcs_pdu_buff, &mut stream)
    }
}

impl Decoder for McsTransport {
    type Item = ironrdp::McsPdu;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> Result<Self::Item, RdpError> {
        self.0.decode(&mut stream)?;
        let mcs_pdu = ironrdp::McsPdu::from_buffer(&mut stream).map_err(RdpError::McsError)?;

        Ok(mcs_pdu)
    }
}

#[derive(Clone, Debug)]
pub struct SendDataContextTransport {
    pub mcs_transport: McsTransport,
    state: TransportState,
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
            state: TransportState::ToDecode,
        }
    }

    pub fn set_channel_ids(&mut self, channel_ids: ChannelIdentificators) {
        self.channel_ids = channel_ids;
    }

    pub fn set_decoded_context(&mut self, channel_ids: ChannelIdentificators) {
        self.set_channel_ids(channel_ids);
        self.state = TransportState::Decoded;
    }
}

impl Default for SendDataContextTransport {
    fn default() -> Self {
        Self {
            mcs_transport: McsTransport::new(DataTransport::default()),
            channel_ids: ChannelIdentificators {
                initiator_id: 0,
                channel_id: 0,
            },
            state: TransportState::ToDecode,
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
    ) -> Result<(), RdpError> {
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

    fn decode(&mut self, mut stream: impl io::Read) -> Result<Self::Item, RdpError> {
        match self.state {
            TransportState::ToDecode => {
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
            TransportState::Decoded => Ok(self.channel_ids),
        }
    }
}

pub struct ShareControlHeaderTransport {
    global_channel_id: u16,
    share_id: u32,
    pdu_source: u16,
    send_data_context_transport: SendDataContextTransport,
}

impl ShareControlHeaderTransport {
    pub fn new(
        send_data_context_transport: SendDataContextTransport,
        pdu_source: u16,
        global_channel_id: u16,
    ) -> Self {
        Self {
            global_channel_id,
            send_data_context_transport,
            pdu_source,
            share_id: 0,
        }
    }
}

impl Encoder for ShareControlHeaderTransport {
    type Item = ironrdp::ShareControlPdu;
    type Error = RdpError;

    fn encode(
        &mut self,
        share_control_pdu: Self::Item,
        mut stream: impl io::Write,
    ) -> Result<(), RdpError> {
        let share_control_header = ironrdp::ShareControlHeader {
            share_control_pdu,
            pdu_source: self.pdu_source,
            share_id: self.share_id,
        };

        let mut pdu = Vec::with_capacity(share_control_header.buffer_length());
        share_control_header
            .to_buffer(&mut pdu)
            .map_err(RdpError::ShareControlHeaderError)?;

        self.send_data_context_transport.encode(pdu, &mut stream)
    }
}

impl Decoder for ShareControlHeaderTransport {
    type Item = ironrdp::ShareControlPdu;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> Result<Self::Item, RdpError> {
        let channel_ids = self.send_data_context_transport.decode(&mut stream)?;
        if channel_ids.channel_id != self.global_channel_id {
            return Err(RdpError::InvalidResponse(format!(
                "Unexpected Send Data Context channel ID ({})",
                channel_ids.channel_id,
            )));
        }

        let share_control_header = ironrdp::ShareControlHeader::from_buffer(&mut stream)
            .map_err(RdpError::ShareControlHeaderError)?;
        self.share_id = share_control_header.share_id;

        if share_control_header.pdu_source != SERVER_CHANNEL_ID {
            warn!(
                "Invalid Share Control Header pdu source: expected ({}) != actual ({})",
                SERVER_CHANNEL_ID, share_control_header.pdu_source
            );
        }

        Ok(share_control_header.share_control_pdu)
    }
}

pub struct ShareDataHeaderTransport(ShareControlHeaderTransport);

impl ShareDataHeaderTransport {
    pub fn new(transport: ShareControlHeaderTransport) -> Self {
        Self(transport)
    }
}

impl Encoder for ShareDataHeaderTransport {
    type Item = ironrdp::ShareDataPdu;
    type Error = RdpError;

    fn encode(
        &mut self,
        share_data_pdu: Self::Item,
        mut stream: impl io::Write,
    ) -> Result<(), RdpError> {
        let share_data_header = ironrdp::ShareDataHeader {
            share_data_pdu,
            stream_priority: ironrdp::rdp::StreamPriority::Medium,
            compression_flags: ironrdp::rdp::CompressionFlags::empty(),
            compression_type: ironrdp::rdp::CompressionType::K8, // ignored if CompressionFlags::empty()
        };

        self.0.encode(
            ironrdp::ShareControlPdu::Data(share_data_header),
            &mut stream,
        )
    }
}

impl Decoder for ShareDataHeaderTransport {
    type Item = ironrdp::ShareDataPdu;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> Result<Self::Item, RdpError> {
        let share_control_pdu = self.0.decode(&mut stream)?;

        if let ironrdp::ShareControlPdu::Data(share_data_header) = share_control_pdu {
            Ok(share_data_header.share_data_pdu)
        } else {
            Err(RdpError::UnexpectedPdu(format!(
                "Expected Share Data Header, got: {:?}",
                share_control_pdu.as_short_name()
            )))
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum TransportState {
    ToDecode,
    Decoded,
}

#[derive(Debug, Copy, Clone, Default)]
pub struct RdpTransport;

impl Decoder for RdpTransport {
    type Item = RdpPdu;
    type Error = RdpError;

    fn decode(&mut self, mut stream: impl io::Read) -> Result<Self::Item, Self::Error> {
        RdpPdu::from_buffer(&mut stream).map_err(RdpError::from)
    }
}

impl Encoder for RdpTransport {
    type Item = (RdpPdu, BytesMut);
    type Error = RdpError;

    fn encode(
        &mut self,
        (item, data): Self::Item,
        mut stream: impl io::Write,
    ) -> Result<(), Self::Error> {
        match item {
            RdpPdu::X224(data) => {
                data.to_buffer(&mut stream)?;
            }
            RdpPdu::FastPath(fast_path) => {
                fast_path.to_buffer(&mut stream)?;
            }
        }

        stream.write_all(data.as_ref())?;
        stream.flush()?;

        Ok(())
    }
}
