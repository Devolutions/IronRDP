use crate::{DynamicChannelId, Vec};
use ironrdp_pdu::{cast_length, cursor::WriteCursor, ensure_size, PduEncode, PduResult};
use ironrdp_svc::SvcPduEncode;

// TODO: The rest of the PDU's currently in `ironrdp-pdu/src/rdp/vc/dvc.rs` should ultimately be moved here.
#[derive(Debug)]
pub enum DrdynvcPdu {
    CapabilitiesResponse(CapabilitiesResponsePdu),
    CreateResponse(CreateResponsePdu),
    DataFirst(DataFirstPdu),
    Data(DataPdu),
    Close(ClosePdu),
}

impl PduEncode for DrdynvcPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            DrdynvcPdu::DataFirst(pdu) => pdu.encode(dst),
            DrdynvcPdu::Data(pdu) => pdu.encode(dst),
            DrdynvcPdu::CreateResponse(pdu) => pdu.encode(dst),
            DrdynvcPdu::Close(pdu) => pdu.encode(dst),
            DrdynvcPdu::CapabilitiesResponse(pdu) => pdu.encode(dst),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            DrdynvcPdu::DataFirst(pdu) => pdu.name(),
            DrdynvcPdu::Data(pdu) => pdu.name(),
            DrdynvcPdu::CreateResponse(pdu) => pdu.name(),
            DrdynvcPdu::Close(pdu) => pdu.name(),
            DrdynvcPdu::CapabilitiesResponse(pdu) => pdu.name(),
        }
    }

    fn size(&self) -> usize {
        match self {
            DrdynvcPdu::DataFirst(pdu) => pdu.size(),
            DrdynvcPdu::Data(pdu) => pdu.size(),
            DrdynvcPdu::CreateResponse(pdu) => pdu.size(),
            DrdynvcPdu::Close(pdu) => pdu.size(),
            DrdynvcPdu::CapabilitiesResponse(pdu) => pdu.size(),
        }
    }
}

/// Dynamic virtual channel PDU's are sent over a static virtual channel, so they are `SvcPduEncode`.
impl SvcPduEncode for DrdynvcPdu {}

/// [2.2] Message Syntax
///
/// [2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/0b07a750-bf51-4042-bcf2-a991b6729d6e
#[derive(Debug)]
struct Header {
    cb_id: FieldType, // 2 bit
    sp: FieldType,    // 2 bit; meaning depends on the cmd field
    cmd: Cmd,         // 4 bit
}

impl Header {
    /// Create a new `Header` with the given `cb_id_val`, `sp_val`, and `cmd`.
    ///
    /// If `cb_id_val` or `sp_val` is not relevant for a given `cmd`, it should be set to 0 respectively.
    fn new(cb_id_val: u32, sp_val: u32, cmd: Cmd) -> Self {
        Self {
            cb_id: FieldType::for_val(cb_id_val),
            sp: FieldType::for_val(sp_val),
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
#[derive(Debug, Copy, Clone)]
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
#[derive(Debug)]
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
            header: Header::new(channel_id, total_length.into(), Cmd::DataFirst),
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
#[derive(Debug, Copy, Clone)]
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

    fn for_val(value: u32) -> Self {
        if value <= u8::MAX as u32 {
            FieldType::U8
        } else if value <= u16::MAX as u32 {
            FieldType::U16
        } else {
            FieldType::U32
        }
    }
}

/// 2.2.3.2 DVC Data PDU (DYNVC_DATA)
///
/// [2.2.3.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/15b59886-db44-47f1-8da3-47c8fcd82803
#[derive(Debug)]
pub struct DataPdu {
    header: Header,
    channel_id: DynamicChannelId,
    data: Vec<u8>,
}

impl DataPdu {
    pub fn new(channel_id: DynamicChannelId, data: Vec<u8>) -> Self {
        Self {
            header: Header::new(channel_id, 0, Cmd::Data),
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

/// 2.2.2.2 DVC Create Response PDU (DYNVC_CREATE_RSP)
///
/// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/8f284ea3-54f3-4c24-8168-8a001c63b581
#[derive(Debug)]
pub struct CreateResponsePdu {
    header: Header,
    channel_id: DynamicChannelId,
    creation_status: CreationStatus,
}

impl CreateResponsePdu {
    pub fn new(channel_id: DynamicChannelId, creation_status: CreationStatus) -> Self {
        Self {
            header: Header::new(channel_id, 0, Cmd::Create),
            channel_id,
            creation_status,
        }
    }

    fn name(&self) -> &'static str {
        "DYNVC_CREATE_RSP"
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
        self.header.cb_id.encode(self.channel_id, dst)?;
        self.creation_status.encode(dst)?;
        Ok(())
    }

    fn size(&self) -> usize {
        Header::size() +
        self.header.cb_id.size_of_val() + // ChannelId
        CreationStatus::size() // CreationStatus
    }
}

#[derive(Debug)]
pub struct CreationStatus(u32);

impl CreationStatus {
    pub const OK: Self = Self(0x00000000);
    pub const NO_LISTENER: Self = Self(0xC0000001);

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::size());
        dst.write_u32(self.0);
        Ok(())
    }

    fn size() -> usize {
        4
    }
}

/// 2.2.4 Closing a DVC (DYNVC_CLOSE)
///
/// [2.2.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/c02dfd21-ccbc-4254-985b-3ef6dd115dec
#[derive(Debug)]
pub struct ClosePdu {
    header: Header,
    channel_id: DynamicChannelId,
}

impl ClosePdu {
    pub fn new(channel_id: DynamicChannelId) -> Self {
        Self {
            header: Header::new(channel_id, 0, Cmd::Close),
            channel_id,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
        self.header.cb_id.encode(self.channel_id, dst)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "DYNVC_CLOSE"
    }

    fn size(&self) -> usize {
        Header::size() + self.header.cb_id.size_of_val()
    }
}

/// 2.2.1.2 DVC Capabilities Response PDU (DYNVC_CAPS_RSP)
///
/// [2.2.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedyc/d45cb2a6-e7bd-453e-8603-9c57600e24ce
#[derive(Debug)]
pub struct CapabilitiesResponsePdu {
    header: Header,
    version: CapsVersion,
}

impl CapabilitiesResponsePdu {
    pub fn new(version: CapsVersion) -> Self {
        Self {
            header: Header::new(0, 0, Cmd::Capability),
            version,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
        dst.write_u8(0x00); // Pad, MUST be 0x00
        self.version.encode(dst)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "DYNVC_CAPS_RSP"
    }

    fn size(&self) -> usize {
        Header::size() + 1 /* Pad */ + CapsVersion::size()
    }
}

#[repr(u16)]
#[derive(Debug, Copy, Clone)]
pub enum CapsVersion {
    V1 = 0x0001,
    V2 = 0x0002,
    V3 = 0x0003,
}

impl CapsVersion {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::size());
        dst.write_u16(*self as u16);
        Ok(())
    }

    fn size() -> usize {
        2
    }
}
