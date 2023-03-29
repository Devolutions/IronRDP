use std::io;

use bytes::{Buf as _, BufMut as _, Bytes, BytesMut};
use ironrdp_pdu::rdp::{vc, SERVER_CHANNEL_ID};
use ironrdp_pdu::PduParsing;

use crate::{ChannelIdentificators, RdpError};

pub(crate) trait Frame {
    fn encode(self, stream: impl io::Write) -> Result<(), RdpError>;

    fn decode(stream: impl io::Read) -> Result<Self, RdpError>
    where
        Self: Sized;
}

pub struct X224Frame<T>(pub T)
where
    T: PduParsing;

impl<T> Frame for X224Frame<T>
where
    T: PduParsing,
    RdpError: From<<T as PduParsing>::Error>,
{
    fn encode(self, mut stream: impl io::Write) -> Result<(), RdpError> {
        ironrdp_pdu::DataHeader::new(self.0.buffer_length())
            .to_buffer(&mut stream)
            .map_err(ironrdp_pdu::RdpError::X224Error)?;
        self.0.to_buffer(&mut stream)?;
        stream.flush()?;
        Ok(())
    }

    fn decode(mut stream: impl io::Read) -> Result<Self, RdpError> {
        let data_header =
            ironrdp_pdu::DataHeader::from_buffer(&mut stream).map_err(ironrdp_pdu::RdpError::X224Error)?;
        let length = data_header.data_length;
        let item = T::from_buffer(&mut stream)?;
        let remaining = length - item.buffer_length();
        if remaining > 0 {
            let mut remaining = vec![0u8; remaining];
            stream.read_exact(&mut remaining)?;
        }
        Ok(Self(item))
    }
}

#[derive(Clone, Debug)]
pub struct McsFrame {
    pub pdu: ironrdp_pdu::McsPdu,
    pub data: Bytes,
}

impl Frame for McsFrame {
    fn encode(self, mut stream: impl io::Write) -> Result<(), RdpError> {
        let data_header = ironrdp_pdu::DataHeader::new(self.pdu.buffer_length() + self.data.len());

        data_header.to_buffer(&mut stream)?;
        self.pdu.to_buffer(&mut stream)?;
        stream.write_all(&self.data)?;

        stream.flush()?;

        Ok(())
    }

    fn decode(mut stream: impl io::Read) -> Result<Self, RdpError> {
        let data_header = ironrdp_pdu::DataHeader::from_buffer(&mut stream)?;

        let mcs_pdu = ironrdp_pdu::McsPdu::from_buffer(&mut stream)?;

        let remaining = data_header.data_length - mcs_pdu.buffer_length();
        let mut data = BytesMut::zeroed(remaining);

        if remaining > 0 {
            stream.read_exact(&mut data)?;
        }

        Ok(Self {
            pdu: mcs_pdu,
            data: data.freeze(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct SendDataInfoFrame {
    pub channel_ids: ChannelIdentificators,
    pub data: Bytes,
}

impl SendDataInfoFrame {
    pub fn new(initiator_id: u16, channel_id: u16, data: Bytes) -> Self {
        Self {
            channel_ids: ChannelIdentificators {
                initiator_id,
                channel_id,
            },
            data,
        }
    }
}

impl Frame for SendDataInfoFrame {
    fn encode(self, stream: impl io::Write) -> Result<(), RdpError> {
        let send_data_context = ironrdp_pdu::mcs::SendDataContext {
            channel_id: self.channel_ids.channel_id,
            initiator_id: self.channel_ids.initiator_id,
            pdu_length: self.data.len(),
        };

        McsFrame {
            pdu: ironrdp_pdu::McsPdu::SendDataRequest(send_data_context),
            data: self.data,
        }
        .encode(stream)
    }

    fn decode(stream: impl io::Read) -> Result<Self, RdpError> {
        let mcs_frame = McsFrame::decode(stream)?;

        match mcs_frame.pdu {
            ironrdp_pdu::McsPdu::SendDataIndication(send_data_context) => Ok(Self {
                channel_ids: ChannelIdentificators {
                    initiator_id: send_data_context.initiator_id,
                    channel_id: send_data_context.channel_id,
                },
                data: mcs_frame.data,
            }),
            ironrdp_pdu::McsPdu::DisconnectProviderUltimatum(disconnect_reason) => Err(
                RdpError::UnexpectedDisconnection(format!("Server disconnection reason - {disconnect_reason:?}")),
            ),
            _ => Err(RdpError::UnexpectedPdu(format!(
                "Expected Send Data Context PDU, got {:?}",
                mcs_frame.pdu.as_short_name()
            ))),
        }
    }
}

pub struct ShareControlFrame {
    pub channel_ids: ChannelIdentificators,
    pub share_id: u32,
    pub pdu_source: u16, // NOTE: looks like this is always equal to channel_ids.initiatior_id
    pub pdu: ironrdp_pdu::ShareControlPdu,
    pub data: Bytes,
}

impl Frame for ShareControlFrame {
    fn encode(self, stream: impl io::Write) -> Result<(), RdpError> {
        let share_control_header = ironrdp_pdu::ShareControlHeader {
            share_control_pdu: self.pdu,
            pdu_source: self.pdu_source,
            share_id: self.share_id,
        };

        let mut buf_writer = BytesMut::with_capacity(share_control_header.buffer_length()).writer();
        share_control_header
            .to_buffer(&mut buf_writer)
            .map_err(RdpError::ShareControlHeader)?;
        let mut buf = buf_writer.into_inner();
        buf.extend_from_slice(&self.data);

        SendDataInfoFrame {
            channel_ids: self.channel_ids,
            data: buf.freeze(),
        }
        .encode(stream)
    }

    fn decode(stream: impl io::Read) -> Result<Self, RdpError> {
        let info_frame = SendDataInfoFrame::decode(stream)?;

        let channel_ids = info_frame.channel_ids;
        let mut remaining_reader = info_frame.data.reader();

        let share_control_header = ironrdp_pdu::ShareControlHeader::from_buffer(&mut remaining_reader)
            .map_err(RdpError::ShareControlHeader)?;
        let pdu = share_control_header.share_control_pdu;
        let pdu_source = share_control_header.pdu_source;
        let share_id = share_control_header.share_id;

        if pdu_source != SERVER_CHANNEL_ID {
            warn!(
                "Invalid Share Control Header pdu source: expected ({}) != actual ({})",
                SERVER_CHANNEL_ID, share_control_header.pdu_source
            );
        }

        let remaining = remaining_reader.into_inner();

        Ok(Self {
            channel_ids,
            share_id,
            pdu_source,
            pdu,
            data: remaining,
        })
    }
}

pub struct ShareDataFrame {
    pub channel_ids: ChannelIdentificators,
    pub share_id: u32,
    pub pdu_source: u16, // NOTE: looks like this is always equal to channel_ids.initiatior_id
    pub pdu: ironrdp_pdu::ShareDataPdu,
}

impl Frame for ShareDataFrame {
    fn encode(self, stream: impl io::Write) -> Result<(), RdpError> {
        let share_data_header = ironrdp_pdu::ShareDataHeader {
            share_data_pdu: self.pdu,
            stream_priority: ironrdp_pdu::rdp::StreamPriority::Medium,
            compression_flags: ironrdp_pdu::rdp::CompressionFlags::empty(),
            compression_type: ironrdp_pdu::rdp::CompressionType::K8, // ignored if CompressionFlags::empty()
        };

        ShareControlFrame {
            channel_ids: self.channel_ids,
            share_id: self.share_id,
            pdu_source: self.pdu_source,
            pdu: ironrdp_pdu::ShareControlPdu::Data(share_data_header),
            data: Bytes::new(),
        }
        .encode(stream)
    }

    fn decode(stream: impl io::Read) -> Result<Self, RdpError> {
        let frame = ShareControlFrame::decode(stream)?;

        if let ironrdp_pdu::ShareControlPdu::Data(share_data_header) = frame.pdu {
            Ok(Self {
                channel_ids: frame.channel_ids,
                share_id: frame.share_id,
                pdu_source: frame.pdu_source,
                pdu: share_data_header.share_data_pdu,
            })
        } else {
            Err(RdpError::UnexpectedPdu(format!(
                "Expected Share Data Header, got: {:?}",
                frame.pdu.as_short_name()
            )))
        }
    }
}

#[derive(Clone, Debug)]
pub struct SendPduDataFrame<T>
where
    T: PduParsing,
{
    pub channel_ids: ChannelIdentificators,
    pub pdu: T,
}

impl<T> Frame for SendPduDataFrame<T>
where
    T: PduParsing,
    RdpError: From<<T as PduParsing>::Error>,
{
    fn encode(self, stream: impl io::Write) -> Result<(), RdpError> {
        let send_data_context = ironrdp_pdu::mcs::SendDataContext {
            channel_id: self.channel_ids.channel_id,
            initiator_id: self.channel_ids.initiator_id,
            pdu_length: self.pdu.buffer_length(),
        };

        let mut buf_writer = BytesMut::with_capacity(self.pdu.buffer_length()).writer();
        self.pdu.to_buffer(&mut buf_writer)?;
        let buf = buf_writer.into_inner();

        McsFrame {
            pdu: ironrdp_pdu::McsPdu::SendDataRequest(send_data_context),
            data: buf.freeze(),
        }
        .encode(stream)
    }

    fn decode(stream: impl io::Read) -> Result<Self, RdpError> {
        let mcs_frame = McsFrame::decode(stream)?;

        let channel_ids = match mcs_frame.pdu {
            ironrdp_pdu::McsPdu::SendDataIndication(send_data_context) => ChannelIdentificators {
                initiator_id: send_data_context.initiator_id,
                channel_id: send_data_context.channel_id,
            },
            ironrdp_pdu::McsPdu::DisconnectProviderUltimatum(disconnect_reason) => {
                return Err(RdpError::UnexpectedDisconnection(format!(
                    "Server disconnection reason - {disconnect_reason:?}"
                )))
            }
            _ => {
                return Err(RdpError::UnexpectedPdu(format!(
                    "Expected Send Data Context PDU, got {:?}",
                    mcs_frame.pdu.as_short_name()
                )))
            }
        };

        let pdu = T::from_buffer(mcs_frame.data.reader())?;

        Ok(Self { channel_ids, pdu })
    }
}

#[derive(Clone, Debug)]
pub struct StaticVirtualChannelFrame {
    pub channel_ids: ChannelIdentificators,
    pub data: Bytes,
}

impl Frame for StaticVirtualChannelFrame {
    fn encode(self, mut stream: impl io::Write) -> Result<(), RdpError> {
        let channel_header = vc::ChannelPduHeader {
            total_length: self.data.len() as u32,
            flags: vc::ChannelControlFlags::FLAG_FIRST | vc::ChannelControlFlags::FLAG_LAST,
        };

        let mut buf_writer = BytesMut::with_capacity(channel_header.buffer_length() + self.data.len()).writer();
        channel_header.to_buffer(&mut buf_writer)?;
        let mut buf = buf_writer.into_inner();
        buf.extend_from_slice(&self.data);

        let info_frame = SendDataInfoFrame {
            channel_ids: self.channel_ids,
            data: buf.freeze(),
        };

        info_frame.encode(&mut stream)
    }

    fn decode(stream: impl io::Read) -> Result<Self, RdpError> {
        let info_frame = SendDataInfoFrame::decode(stream)?;

        let mut remaining_reader = info_frame.data.reader();
        let channel_header = vc::ChannelPduHeader::from_buffer(&mut remaining_reader)?;
        let remaining = remaining_reader.into_inner();
        debug_assert_eq!(remaining.len(), channel_header.total_length as usize);

        Ok(Self {
            channel_ids: info_frame.channel_ids,
            data: remaining,
        })
    }
}

pub struct DynamicVirtualChannelServerFrame {
    pub channel_ids: ChannelIdentificators,
    pub dvc_pdu: vc::dvc::ServerPdu,
    pub extra_data: Bytes,
}

impl DynamicVirtualChannelServerFrame {
    pub fn drdynvc_id(&self) -> u16 {
        self.channel_ids.channel_id
    }
}

impl Frame for DynamicVirtualChannelServerFrame {
    fn encode(self, stream: impl io::Write) -> Result<(), RdpError> {
        let mut buf_writer = BytesMut::with_capacity(self.dvc_pdu.buffer_length() + self.extra_data.len()).writer();
        self.dvc_pdu.to_buffer(&mut buf_writer)?;
        let mut buf = buf_writer.into_inner();
        buf.extend_from_slice(&self.extra_data);

        StaticVirtualChannelFrame {
            channel_ids: self.channel_ids,
            data: buf.freeze(),
        }
        .encode(stream)
    }

    fn decode(stream: impl io::Read) -> Result<Self, RdpError> {
        let channel_frame = StaticVirtualChannelFrame::decode(stream)?;
        let channel_ids = channel_frame.channel_ids;
        let dvc_data_size = channel_frame.data.len();

        let mut remaining_reader = channel_frame.data.reader();
        let dvc_server_pdu = vc::dvc::ServerPdu::from_buffer(&mut remaining_reader, dvc_data_size)?;
        let remaining = remaining_reader.into_inner();

        Ok(Self {
            channel_ids,
            dvc_pdu: dvc_server_pdu,
            extra_data: remaining,
        })
    }
}

pub struct DynamicVirtualChannelClientFrame {
    pub channel_ids: ChannelIdentificators,
    pub dvc_pdu: vc::dvc::ClientPdu,
    pub extra_data: Bytes,
}

impl DynamicVirtualChannelClientFrame {
    pub fn drdynvc_id(&self) -> u16 {
        self.channel_ids.channel_id
    }
}

impl Frame for DynamicVirtualChannelClientFrame {
    fn encode(self, stream: impl io::Write) -> Result<(), RdpError> {
        let mut buf_writer = BytesMut::with_capacity(self.dvc_pdu.buffer_length() + self.extra_data.len()).writer();
        self.dvc_pdu.to_buffer(&mut buf_writer)?;
        let mut buf = buf_writer.into_inner();
        buf.extend_from_slice(&self.extra_data);

        StaticVirtualChannelFrame {
            channel_ids: self.channel_ids,
            data: buf.freeze(),
        }
        .encode(stream)
    }

    fn decode(stream: impl io::Read) -> Result<Self, RdpError> {
        let channel_frame = StaticVirtualChannelFrame::decode(stream)?;
        let channel_ids = channel_frame.channel_ids;
        let dvc_data_size = channel_frame.data.len();

        let mut remaining_reader = channel_frame.data.reader();
        let dvc_server_pdu = vc::dvc::ClientPdu::from_buffer(&mut remaining_reader, dvc_data_size)?;
        let remaining = remaining_reader.into_inner();

        Ok(Self {
            channel_ids,
            dvc_pdu: dvc_server_pdu,
            extra_data: remaining,
        })
    }
}
