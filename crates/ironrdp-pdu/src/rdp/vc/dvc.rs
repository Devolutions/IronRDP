use bit_field::BitField;
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::cursor::{ReadCursor, WriteCursor};
use crate::{decode_cursor, PduDecode, PduEncode, PduResult};

#[cfg(test)]
mod tests;

pub mod display;
pub mod gfx;

mod capabilities;
mod close;
mod create;
mod data;
mod data_first;

pub use self::capabilities::{CapabilitiesRequestPdu, CapabilitiesResponsePdu, CapsVersion};
pub use self::close::ClosePdu;
pub use self::create::{CreateRequestPdu, CreateResponsePdu, DVC_CREATION_STATUS_NO_LISTENER, DVC_CREATION_STATUS_OK};
pub use self::data::DataPdu;
pub use self::data_first::DataFirstPdu;

const HEADER_SIZE: usize = 1;
const PDU_WITH_DATA_MAX_SIZE: usize = 1600;

const UNUSED_U8: u8 = 0;

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum PduType {
    Create = 0x01,
    DataFirst = 0x02,
    Data = 0x03,
    Close = 0x04,
    Capabilities = 0x05,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerPdu {
    CapabilitiesRequest(CapabilitiesRequestPdu),
    CreateRequest(CreateRequestPdu),
    DataFirst(DataFirstPdu),
    Data(DataPdu),
    CloseRequest(ClosePdu),
}

impl ServerPdu {
    const NAME: &'static str = "DvcServerPdu";
}

impl PduEncode for ServerPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            ServerPdu::CapabilitiesRequest(caps_request) => caps_request.encode(dst)?,
            ServerPdu::CreateRequest(create_request) => create_request.encode(dst)?,
            ServerPdu::DataFirst(data_first) => data_first.encode(dst)?,
            ServerPdu::Data(data) => data.encode(dst)?,
            ServerPdu::CloseRequest(close_request) => close_request.encode(dst)?,
        };

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            ServerPdu::CapabilitiesRequest(caps_request) => caps_request.size(),
            ServerPdu::CreateRequest(create_request) => create_request.size(),
            ServerPdu::DataFirst(data_first) => data_first.size(),
            ServerPdu::Data(data) => data.size(),
            ServerPdu::CloseRequest(close_request) => close_request.size(),
        }
    }
}

impl ServerPdu {
    pub fn decode(src: &mut ReadCursor<'_>, mut dvc_data_size: usize) -> PduResult<Self> {
        let dvc_header: Header = decode_cursor(src)?;
        let channel_id_type = FieldType::from_u8(dvc_header.channel_id_type)
            .ok_or_else(|| invalid_message_err!("DvcHeader", "invalid channel ID type"))?;

        dvc_data_size -= HEADER_SIZE;

        let res = match dvc_header.pdu_type {
            PduType::Capabilities => ServerPdu::CapabilitiesRequest(CapabilitiesRequestPdu::decode(src)?),
            PduType::Create => ServerPdu::CreateRequest(CreateRequestPdu::decode(src, channel_id_type, dvc_data_size)?),
            PduType::DataFirst => {
                let data_length_type = FieldType::from_u8(dvc_header.pdu_dependent)
                    .ok_or_else(|| invalid_message_err!("DvcHeader", "data length type"))?;

                ServerPdu::DataFirst(DataFirstPdu::decode(
                    src,
                    channel_id_type,
                    data_length_type,
                    dvc_data_size,
                )?)
            }
            PduType::Data => ServerPdu::Data(DataPdu::decode(src, channel_id_type, dvc_data_size)?),
            PduType::Close => ServerPdu::CloseRequest(ClosePdu::decode(src, channel_id_type)?),
        };

        Ok(res)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientPdu {
    CapabilitiesResponse(CapabilitiesResponsePdu),
    CreateResponse(CreateResponsePdu),
    DataFirst(DataFirstPdu),
    Data(DataPdu),
    CloseResponse(ClosePdu),
}

impl ClientPdu {
    const NAME: &'static str = "ClientPdu";

    pub fn decode(src: &mut ReadCursor<'_>, mut dvc_data_size: usize) -> PduResult<Self> {
        let dvc_header = Header::decode(src)?;
        let channel_id_type = FieldType::from_u8(dvc_header.channel_id_type)
            .ok_or_else(|| invalid_message_err!("DvcHeader", "invalid channel ID type"))?;

        dvc_data_size -= HEADER_SIZE;

        let res = match dvc_header.pdu_type {
            PduType::Capabilities => ClientPdu::CapabilitiesResponse(CapabilitiesResponsePdu::decode(src)?),
            PduType::Create => ClientPdu::CreateResponse(CreateResponsePdu::decode(src, channel_id_type)?),
            PduType::DataFirst => {
                let data_length_type = FieldType::from_u8(dvc_header.pdu_dependent)
                    .ok_or_else(|| invalid_message_err!("DvcHeader", "data length type"))?;

                ClientPdu::DataFirst(DataFirstPdu::decode(
                    src,
                    channel_id_type,
                    data_length_type,
                    dvc_data_size,
                )?)
            }
            PduType::Data => ClientPdu::Data(DataPdu::decode(src, channel_id_type, dvc_data_size)?),
            PduType::Close => ClientPdu::CloseResponse(ClosePdu::decode(src, channel_id_type)?),
        };

        Ok(res)
    }
}

impl PduEncode for ClientPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            ClientPdu::CapabilitiesResponse(caps_request) => caps_request.encode(dst)?,
            ClientPdu::CreateResponse(create_request) => create_request.encode(dst)?,
            ClientPdu::DataFirst(data_first) => data_first.encode(dst)?,
            ClientPdu::Data(data) => data.encode(dst)?,
            ClientPdu::CloseResponse(close_response) => close_response.encode(dst)?,
        };

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        match self {
            ClientPdu::CapabilitiesResponse(caps_request) => caps_request.size(),
            ClientPdu::CreateResponse(create_request) => create_request.size(),
            ClientPdu::DataFirst(data_first) => data_first.size(),
            ClientPdu::Data(data) => data.size(),
            ClientPdu::CloseResponse(close_response) => close_response.size(),
        }
    }
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum FieldType {
    U8 = 0x00,
    U16 = 0x01,
    U32 = 0x02,
}

impl FieldType {
    pub fn read_according_to_type(self, src: &mut ReadCursor<'_>) -> PduResult<u32> {
        ensure_size!(in: src, size: self.size());

        let value = match self {
            FieldType::U8 => u32::from(src.read_u8()),
            FieldType::U16 => u32::from(src.read_u16()),
            FieldType::U32 => src.read_u32(),
        };

        Ok(value)
    }

    pub fn write_according_to_type(self, dst: &mut WriteCursor<'_>, value: u32) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        match self {
            FieldType::U8 => dst.write_u8(value as u8),
            FieldType::U16 => dst.write_u16(value as u16),
            FieldType::U32 => dst.write_u32(value),
        };

        Ok(())
    }

    pub fn size(self) -> usize {
        match self {
            FieldType::U8 => 1,
            FieldType::U16 => 2,
            FieldType::U32 => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Header {
    channel_id_type: u8, // 2 bit
    pdu_dependent: u8,   // 2 bit
    pdu_type: PduType,   // 4 bit
}

impl Header {
    const NAME: &'static str = "DvcHeader";

    const FIXED_PART_SIZE: usize = 1 /* header */;
}

impl PduEncode for Header {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_fixed_part_size!(in: dst);

        let mut dvc_header: u8 = 0;
        dvc_header.set_bits(0..2, self.channel_id_type);
        dvc_header.set_bits(2..4, self.pdu_dependent);
        dvc_header.set_bits(4..8, self.pdu_type.to_u8().unwrap());
        dst.write_u8(dvc_header);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> PduDecode<'de> for Header {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let dvc_header = src.read_u8();
        let channel_id_type = dvc_header.get_bits(0..2);
        let pdu_dependent = dvc_header.get_bits(2..4);
        let pdu_type = PduType::from_u8(dvc_header.get_bits(4..8))
            .ok_or_else(|| invalid_message_err!("DvcHeader", "invalid Cmd"))?;

        Ok(Self {
            channel_id_type,
            pdu_dependent,
            pdu_type,
        })
    }
}
