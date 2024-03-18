use crate::{DynamicChannelId, Vec};
use ironrdp_pdu::{cast_length, cursor::WriteCursor, ensure_size, PduEncode, PduResult};
use ironrdp_svc::SvcPduEncode;

// TODO: The rest of the PDU's currently in `ironrdp-pdu/src/rdp/vc/dvc.rs` should ultimately be moved here.
pub enum DrdynvcPdu {
    DataFirst(DataFirstPdu),
    Data(DataPdu),
}

impl PduEncode for DrdynvcPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            DrdynvcPdu::DataFirst(pdu) => pdu.encode(dst),
            DrdynvcPdu::Data(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            DrdynvcPdu::DataFirst(pdu) => pdu.name(),
            DrdynvcPdu::Data(pdu) => pdu.name(),
        }
    }

    fn size(&self) -> usize {
        match self {
            DrdynvcPdu::DataFirst(pdu) => pdu.size(),
            DrdynvcPdu::Data(pdu) => pdu.size(),
        }
    }
}

/// Dynamic virtual channel PDU's are sent over a static virtual channel, so they are `SvcPduEncode`.
impl SvcPduEncode for DrdynvcPdu {}

/// [2.2] Message Syntax
///
/// [2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/0b07a750-bf51-4042-bcf2-a991b6729d6e
struct Header {
    cb_id: FieldType, // 2 bit
    sp: FieldType,    // 2 bit; meaning depends on the cmd field
    cmd: Cmd,         // 4 bit
}

impl Header {
    fn new(cmd: Cmd) -> Self {
        // Always using U32 for cb_id and sp
        // ensures that their respective values
        // always fit.
        Self {
            cb_id: FieldType::U32,
            sp: FieldType::U32,
            cmd,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::size());
        dst.write_u8((self.cmd as u8) << 4 | (self.sp as u8) << 2 | (self.cb_id as u8));
        Ok(())
    }

    fn size() -> usize {
        1
    }
}

/// [2.2] Message Syntax
///
/// [2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/0b07a750-bf51-4042-bcf2-a991b6729d6e
#[repr(u8)]
#[derive(Copy, Clone)]
enum Cmd {
    Create = 0x01,
    DataFirst = 0x02,
    Data = 0x03,
    Close = 0x04,
    Capability = 0x05,
    DataFirstCompressed = 0x06,
    DataCompressed = 0x07,
    SoftSyncRequest = 0x08,
    SoftSyncResponse = 0x09,
}

/// 2.2.3.1 DVC Data First PDU (DYNVC_DATA_FIRST)
///
/// [2.2.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/69377767-56a6-4ab8-996b-7758676e9261
pub struct DataFirstPdu {
    header: Header,
    channel_id: DynamicChannelId,
    /// Length is the *total* length of the data to be sent, including the length
    /// of the data that will be sent by subsequent DVC_DATA PDUs.
    length: u8,
    /// Data is just the data to be sent in this PDU.
    data: Vec<u8>,
}

impl DataFirstPdu {
    /// Create a new `DataFirstPdu` with the given `channel_id`, `length`, and `data`.
    ///
    /// `length` is the *total* length of the data to be sent, including the length
    /// of the data that will be sent by subsequent `DataPdu`s.
    ///
    /// `data` is just the data to be sent in this PDU.
    pub fn new(channel_id: DynamicChannelId, total_length: u8, data: Vec<u8>) -> Self {
        Self {
            header: Header::new(Cmd::DataFirst),
            channel_id,
            length: total_length,
            data,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
        self.header.cb_id.encode(self.channel_id, dst)?;
        self.header
            .sp
            .encode(cast_length!("DataFirstPdu::Length", self.length)?, dst)?;
        dst.write_slice(&self.data);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "DYNVC_DATA_FIRST"
    }

    fn size(&self) -> usize {
        Header::size() +
        self.header.cb_id.size_of_val() + // ChannelId
        self.header.sp.size_of_val() + // Length
        self.data.len() // Data
    }
}

#[repr(u8)]
#[derive(Copy, Clone)]
pub enum FieldType {
    U8 = 0x00,
    U16 = 0x01,
    U32 = 0x02,
}

impl FieldType {
    fn encode(&self, value: u32, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size_of_val());
        match self {
            FieldType::U8 => dst.write_u8(cast_length!("FieldType::encode", value)?),
            FieldType::U16 => dst.write_u16(cast_length!("FieldType::encode", value)?),
            FieldType::U32 => dst.write_u32(value),
        };
        Ok(())
    }

    /// Returns the size of the value in bytes.
    fn size_of_val(&self) -> usize {
        match self {
            FieldType::U8 => 1,
            FieldType::U16 => 2,
            FieldType::U32 => 4,
        }
    }
}

/// 2.2.3.2 DVC Data PDU (DYNVC_DATA)
///
/// [2.2.3.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/15b59886-db44-47f1-8da3-47c8fcd82803
pub struct DataPdu {
    header: Header,
    channel_id: DynamicChannelId,
    data: Vec<u8>,
}

impl DataPdu {
    pub fn new(channel_id: DynamicChannelId, data: Vec<u8>) -> Self {
        Self {
            header: Header::new(Cmd::Data),
            channel_id,
            data,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
        self.header.cb_id.encode(self.channel_id, dst)?;
        dst.write_slice(&self.data);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "DYNVC_DATA"
    }

    fn size(&self) -> usize {
        Header::size() +
        self.header.cb_id.size_of_val() + // ChannelId
        self.data.len() // Data
    }
}
