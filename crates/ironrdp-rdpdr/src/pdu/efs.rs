//! PDUs for [\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension]
//!
//! [\[MS-RDPEFS\]: Remote Desktop Protocol: File System Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5

use super::esc::rpce;
use super::{PacketId, SharedHeader};
use bitflags::bitflags;
use core::fmt;
use ironrdp_pdu::cursor::{ReadCursor, WriteCursor};
use ironrdp_pdu::utils::{encoded_str_len, from_utf16_bytes, write_string_to_cursor, CharacterSet};
use ironrdp_pdu::{cast_length, ensure_size, invalid_message_err, read_padding, write_padding, PduError, PduResult};
use std::fmt::Debug;
use std::mem::size_of;

#[derive(Debug, PartialEq)]
pub enum VersionAndIdPduKind {
    /// [2.2.2.2 Server Announce Request (DR_CORE_SERVER_ANNOUNCE_REQ)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/046047aa-62d8-49f9-bf16-7fe41880aaf4)
    ServerAnnounceRequest,
    /// [2.2.2.3 Client Announce Reply (DR_CORE_CLIENT_ANNOUNCE_RSP)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/d6fe6d1b-c145-4a6f-99aa-4fe3cdcea398)
    ClientAnnounceReply,
    /// [2.2.2.6 Server Client ID Confirm (DR_CORE_SERVER_CLIENTID_CONFIRM)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/bbbb9666-6994-4cf6-8e65-0d46eb319c6e)
    ServerClientIdConfirm,
}

impl VersionAndIdPduKind {
    fn name(&self) -> &'static str {
        match self {
            VersionAndIdPduKind::ServerAnnounceRequest => "DR_CORE_SERVER_ANNOUNCE_REQ",
            VersionAndIdPduKind::ClientAnnounceReply => "DR_CORE_CLIENT_ANNOUNCE_RSP",
            VersionAndIdPduKind::ServerClientIdConfirm => "DR_CORE_SERVER_CLIENTID_CONFIRM",
        }
    }
}

/// VersionAndIdPDU is a fixed size structure representing multiple PDUs.
///
/// The kind field is used to determine the actual PDU type, see [`VersionAndIdPduKind`].
#[derive(Debug)]
pub struct VersionAndIdPdu {
    /// This field MUST be set to 0x0001 ([`VERSION_MAJOR`]).
    pub version_major: u16,
    pub version_minor: u16,
    pub client_id: u32,
    pub kind: VersionAndIdPduKind,
}

impl VersionAndIdPdu {
    const FIXED_PART_SIZE: usize = (size_of::<u16>() * 2) + size_of::<u32>();

    pub fn new_client_announce_reply(req: VersionAndIdPdu) -> PduResult<Self> {
        if req.kind != VersionAndIdPduKind::ServerAnnounceRequest {
            return Err(invalid_message_err!(
                "VersionAndIdPdu::new_client_announce_reply",
                "VersionAndIdPduKind",
                "invalid value"
            ));
        }

        Ok(Self {
            version_major: VERSION_MAJOR,
            version_minor: VERSION_MINOR_12,
            client_id: req.client_id,
            kind: VersionAndIdPduKind::ClientAnnounceReply,
        })
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(ctx: self.name(), in: dst, size: Self::FIXED_PART_SIZE);
        dst.write_u16(self.version_major);
        dst.write_u16(self.version_minor);
        dst.write_u32(self.client_id);
        Ok(())
    }

    pub fn decode(header: SharedHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let kind = match header.packet_id {
            PacketId::CoreServerAnnounce => VersionAndIdPduKind::ServerAnnounceRequest,
            PacketId::CoreClientidConfirm => VersionAndIdPduKind::ServerClientIdConfirm,
            _ => {
                return Err(invalid_message_err!(
                    "VersionAndIdPdu::decode",
                    "PacketId",
                    "invalid value"
                ))
            }
        };

        ensure_size!(ctx: kind.name(), in: src, size: Self::FIXED_PART_SIZE);
        let version_major = src.read_u16();
        let version_minor = src.read_u16();
        let client_id = src.read_u32();

        Ok(Self {
            version_major,
            version_minor,
            client_id,
            kind,
        })
    }

    pub fn name(&self) -> &'static str {
        self.kind.name()
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [2.2.2.4 Client Name Request (DR_CORE_CLIENT_NAME_REQ)](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/902497f1-3b1c-4aee-95f8-1668f9b7b7d2)
#[derive(Debug)]
pub enum ClientNameRequest {
    Ascii(String),
    Unicode(String),
}

impl ClientNameRequest {
    const NAME: &str = "DR_CORE_CLIENT_NAME_REQ";
    const FIXED_PART_SIZE: usize = size_of::<u32>() * 3; // unicode_flag + CodePage + ComputerNameLen

    pub fn new(computer_name: String, kind: ClientNameRequestUnicodeFlag) -> Self {
        match kind {
            ClientNameRequestUnicodeFlag::Ascii => ClientNameRequest::Ascii(computer_name),
            ClientNameRequestUnicodeFlag::Unicode => ClientNameRequest::Unicode(computer_name),
        }
    }

    fn unicode_flag(&self) -> ClientNameRequestUnicodeFlag {
        match self {
            ClientNameRequest::Ascii(_) => ClientNameRequestUnicodeFlag::Ascii,
            ClientNameRequest::Unicode(_) => ClientNameRequestUnicodeFlag::Unicode,
        }
    }

    fn computer_name(&self) -> &str {
        match self {
            ClientNameRequest::Ascii(name) => name,
            ClientNameRequest::Unicode(name) => name,
        }
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.unicode_flag().into());
        dst.write_u32(0); // // CodePage (4 bytes): it MUST be set to 0
        dst.write_u32(encoded_str_len(self.computer_name(), self.unicode_flag().into(), true) as u32);
        write_string_to_cursor(dst, self.computer_name(), self.unicode_flag().into(), true)
    }

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + encoded_str_len(self.computer_name(), self.unicode_flag().into(), true)
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy)]
pub enum ClientNameRequestUnicodeFlag {
    Ascii = 0x0,
    Unicode = 0x1,
}

impl From<ClientNameRequestUnicodeFlag> for CharacterSet {
    fn from(val: ClientNameRequestUnicodeFlag) -> Self {
        match val {
            ClientNameRequestUnicodeFlag::Ascii => CharacterSet::Ansi,
            ClientNameRequestUnicodeFlag::Unicode => CharacterSet::Unicode,
        }
    }
}

impl From<ClientNameRequestUnicodeFlag> for u32 {
    fn from(val: ClientNameRequestUnicodeFlag) -> Self {
        val as u32
    }
}

/// [2.2.2.7 Server Core Capability Request (DR_CORE_CAPABILITY_REQ)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/702789c3-b924-4bc2-9280-3221bc7d6797)
/// [2.2.2.8 Client Core Capability Response (DR_CORE_CAPABILITY_RSP)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/f513bf87-cca0-488a-ac5c-18cf18f4a7e1)
#[derive(Debug)]
pub struct CoreCapability {
    pub capabilities: Vec<CapabilityMessage>,
    pub kind: CoreCapabilityKind,
}

impl CoreCapability {
    const FIXED_PART_SIZE: usize = size_of::<u16>() * 2;

    /// Creates a new [`DR_CORE_CAPABILITY_RSP`] with the given `capabilities`.
    ///
    /// [`DR_CORE_CAPABILITY_RSP`]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/f513bf87-cca0-488a-ac5c-18cf18f4a7e1
    pub fn new_response(capabilities: Vec<CapabilityMessage>) -> Self {
        Self {
            capabilities,
            kind: CoreCapabilityKind::ClientCoreCapabilityResponse,
        }
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(ctx: self.name(), in: dst, size: self.size());
        dst.write_u16(cast_length!(
            "CoreCapability",
            "numCapabilities",
            self.capabilities.len()
        )?);
        write_padding!(dst, 2); // 2-bytes padding
        for cap in self.capabilities.iter() {
            cap.encode(dst)?;
        }
        Ok(())
    }

    pub fn decode(header: SharedHeader, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let kind = match header.packet_id {
            PacketId::CoreServerCapability => CoreCapabilityKind::ServerCoreCapabilityRequest,
            PacketId::CoreClientCapability => CoreCapabilityKind::ClientCoreCapabilityResponse,
            _ => {
                return Err(invalid_message_err!(
                    "CoreCapability::decode",
                    "PacketId",
                    "invalid value"
                ))
            }
        };

        ensure_size!(ctx: kind.name(), in: src, size: Self::FIXED_PART_SIZE);

        let num_capabilities = src.read_u16();
        read_padding!(src, 2); // 2-bytes padding
        let mut capabilities = Vec::new();
        for _ in 0..num_capabilities {
            capabilities.push(CapabilityMessage::decode(src)?);
        }

        Ok(Self { capabilities, kind })
    }

    pub fn name(&self) -> &'static str {
        self.kind.name()
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.capabilities.iter().map(|c| c.size()).sum::<usize>()
    }
}

#[derive(Debug, PartialEq)]
pub enum CoreCapabilityKind {
    /// [2.2.2.7 Server Core Capability Request (DR_CORE_CAPABILITY_REQ)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/702789c3-b924-4bc2-9280-3221bc7d6797)
    ServerCoreCapabilityRequest,
    /// [2.2.2.8 Client Core Capability Response (DR_CORE_CAPABILITY_RSP)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/f513bf87-cca0-488a-ac5c-18cf18f4a7e1)
    ClientCoreCapabilityResponse,
}

impl CoreCapabilityKind {
    fn name(&self) -> &'static str {
        match self {
            CoreCapabilityKind::ServerCoreCapabilityRequest => "DR_CORE_CAPABILITY_REQ",
            CoreCapabilityKind::ClientCoreCapabilityResponse => "DR_CORE_CAPABILITY_RSP",
        }
    }
}

#[derive(Debug)]
pub struct Capabilities(Vec<CapabilityMessage>);

impl Capabilities {
    pub fn new() -> Self {
        let mut this = Self(Vec::new());
        this.add_general(0);
        this
    }

    pub fn clone_inner(&mut self) -> Vec<CapabilityMessage> {
        self.0.clone()
    }

    pub fn add_smartcard(&mut self) {
        self.push(CapabilityMessage::new_smartcard());
        self.increment_special_devices();
    }

    pub fn add_drive(&mut self) {
        self.push(CapabilityMessage::new_drive());
    }

    fn add_general(&mut self, special_type_device_cap: u32) {
        self.push(CapabilityMessage::new_general(special_type_device_cap));
    }

    fn push(&mut self, capability: CapabilityMessage) {
        self.0.push(capability);
    }

    fn increment_special_devices(&mut self) {
        let capabilities = &mut self.0;
        for capability in capabilities.iter_mut() {
            match &mut capability.capability_data {
                CapabilityData::General(general_capability) => {
                    general_capability.special_type_device_cap += 1;
                    break;
                }
                _ => continue,
            }
        }
    }
}

impl Default for Capabilities {
    fn default() -> Self {
        Self::new()
    }
}

/// [2.2.1.2.1 Capability Message (CAPABILITY_SET)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/f1b9dd1d-2c37-4aac-9836-4b0df02369ba)
#[derive(Debug, Clone, Copy)]
pub struct CapabilityMessage {
    header: CapabilityHeader,
    capability_data: CapabilityData,
}

impl CapabilityMessage {
    const NAME: &str = "CAPABILITY_SET";
    /// Creates a new [`GENERAL_CAPS_SET`].
    ///
    /// `special_type_device_cap`: A 32-bit unsigned integer that
    /// specifies the number of special devices to be redirected
    /// before the user is logged on. Special devices are those
    /// that are safe and/or required to be redirected before a
    /// user logs on (such as smart cards and serial ports).
    ///
    /// [`GENERAL_CAPS_SET`]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/06c7cb30-303d-4fa2-b396-806df8ac1501
    pub fn new_general(special_type_device_cap: u32) -> Self {
        Self {
            header: CapabilityHeader::new_general(),
            capability_data: CapabilityData::General(GeneralCapabilitySet {
                os_type: 0,
                os_version: 0,
                protocol_major_version: 1,
                protocol_minor_version: VERSION_MINOR_12,
                io_code_1: IoCode1::REQUIRED,
                io_code_2: 0,
                extended_pdu: ExtendedPdu::RDPDR_DEVICE_REMOVE_PDUS | ExtendedPdu::RDPDR_CLIENT_DISPLAY_NAME_PDU,
                extra_flags_1: ExtraFlags1::empty(),
                extra_flags_2: 0,
                special_type_device_cap,
            }),
        }
    }

    /// Creates a new [`SMARTCARD_CAPS_SET`].
    ///
    /// [`SMARTCARD_CAPS_SET`]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/e02de60a-4d32-4dc7-ab17-9d591129eb93
    pub fn new_smartcard() -> Self {
        Self {
            header: CapabilityHeader::new_smartcard(),
            capability_data: CapabilityData::Smartcard,
        }
    }

    /// Creates a new [`DRIVE_CAPS_SET`].
    ///
    /// [`DRIVE_CAPS_SET`]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/4f018cd2-60ba-4c7b-adcf-55bd05cea6f8
    pub fn new_drive() -> Self {
        Self {
            header: CapabilityHeader::new_drive(),
            capability_data: CapabilityData::Drive,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
        self.capability_data.encode(dst)
    }

    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        let header = CapabilityHeader::decode(src)?;
        let capability_data = CapabilityData::decode(src, &header)?;

        Ok(Self {
            header,
            capability_data,
        })
    }

    fn size(&self) -> usize {
        CapabilityHeader::SIZE + self.capability_data.size()
    }
}

/// [2.2.1.2 Capability Header (CAPABILITY_HEADER)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/b3c3304a-2e1b-4667-97e9-3bce49544907)
#[derive(Debug, Clone, Copy)]
struct CapabilityHeader {
    cap_type: CapabilityType,
    length: u16,
    version: u32,
}

impl CapabilityHeader {
    const NAME: &str = "CAPABILITY_HEADER";
    const SIZE: usize = size_of::<u16>() * 2 + size_of::<u32>();

    fn new_general() -> Self {
        Self {
            cap_type: CapabilityType::General,
            length: (Self::SIZE + GeneralCapabilitySet::SIZE) as u16,
            version: GENERAL_CAPABILITY_VERSION_02,
        }
    }

    fn new_smartcard() -> Self {
        Self {
            cap_type: CapabilityType::Smartcard,
            length: Self::SIZE as u16,
            version: SMARTCARD_CAPABILITY_VERSION_01,
        }
    }

    fn new_drive() -> Self {
        Self {
            cap_type: CapabilityType::Drive,
            length: Self::SIZE as u16,
            version: DRIVE_CAPABILITY_VERSION_02,
        }
    }

    fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(in: src, size: Self::SIZE);
        let cap_type: CapabilityType = src.read_u16().try_into()?;
        let length = src.read_u16();
        let version = src.read_u32();

        Ok(Self {
            cap_type,
            length,
            version,
        })
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::SIZE);
        dst.write_u16(self.cap_type.into());
        dst.write_u16(self.length);
        dst.write_u32(self.version);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u16)]
enum CapabilityType {
    /// CAP_GENERAL_TYPE
    General = 0x0001,
    /// CAP_PRINTER_TYPE
    Printer = 0x0002,
    /// CAP_PORT_TYPE
    Port = 0x0003,
    /// CAP_DRIVE_TYPE
    Drive = 0x0004,
    /// CAP_SMARTCARD_TYPE
    Smartcard = 0x0005,
}

impl From<CapabilityType> for u16 {
    fn from(cap_type: CapabilityType) -> Self {
        cap_type as u16
    }
}

/// GENERAL_CAPABILITY_VERSION_02
pub const GENERAL_CAPABILITY_VERSION_02: u32 = 0x0000_0002;
/// SMARTCARD_CAPABILITY_VERSION_01
pub const SMARTCARD_CAPABILITY_VERSION_01: u32 = 0x0000_0001;
/// DRIVE_CAPABILITY_VERSION_02
pub const DRIVE_CAPABILITY_VERSION_02: u32 = 0x0000_0002;

impl TryFrom<u16> for CapabilityType {
    type Error = PduError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x0001 => Ok(CapabilityType::General),
            0x0002 => Ok(CapabilityType::Printer),
            0x0003 => Ok(CapabilityType::Port),
            0x0004 => Ok(CapabilityType::Drive),
            0x0005 => Ok(CapabilityType::Smartcard),
            _ => Err(invalid_message_err!("try_from", "CapabilityType", "invalid value")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum CapabilityData {
    General(GeneralCapabilitySet),
    Printer,
    Port,
    Drive,
    Smartcard,
}

impl CapabilityData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        match self {
            CapabilityData::General(general) => general.encode(dst),
            _ => Ok(()),
        }
    }

    fn decode(src: &mut ReadCursor<'_>, header: &CapabilityHeader) -> PduResult<Self> {
        match header.cap_type {
            CapabilityType::General => Ok(CapabilityData::General(GeneralCapabilitySet::decode(
                src,
                header.version,
            )?)),
            CapabilityType::Printer => Ok(CapabilityData::Printer),
            CapabilityType::Port => Ok(CapabilityData::Port),
            CapabilityType::Drive => Ok(CapabilityData::Drive),
            CapabilityType::Smartcard => Ok(CapabilityData::Smartcard),
        }
    }

    fn size(&self) -> usize {
        match self {
            CapabilityData::General(_) => GeneralCapabilitySet::SIZE,
            CapabilityData::Printer => 0,
            CapabilityData::Port => 0,
            CapabilityData::Drive => 0,
            CapabilityData::Smartcard => 0,
        }
    }
}

/// [2.2.2.7.1 General Capability Set (GENERAL_CAPS_SET)](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/06c7cb30-303d-4fa2-b396-806df8ac1501)
#[derive(Debug, Clone, Copy)]
struct GeneralCapabilitySet {
    /// MUST be ignored.
    os_type: u32,
    /// SHOULD be ignored.
    os_version: u32,
    /// MUST be set to 1.
    protocol_major_version: u16,
    /// MUST be set to one of the values described by the VersionMinor field
    /// of the [Server Client ID Confirm (section 2.2.2.6)] packet.
    ///
    /// [Server Client ID Confirm (section 2.2.2.6)]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/bbbb9666-6994-4cf6-8e65-0d46eb319c6e
    protocol_minor_version: u16,
    /// See [`IoCode1`].
    io_code_1: IoCode1,
    /// MUST be set to 0.
    io_code_2: u32,
    /// See [`ExtendedPdu`].
    extended_pdu: ExtendedPdu,
    /// See [`ExtraFlags1`].
    extra_flags_1: ExtraFlags1,
    /// MUST be set to 0.
    extra_flags_2: u32,
    /// A 32-bit unsigned integer that specifies the number
    /// of special devices to be redirected before the user
    /// is logged on. Special devices are those that are safe
    /// and/or required to be redirected before a user logs
    /// on (such as smart cards and serial ports).
    special_type_device_cap: u32,
}

impl GeneralCapabilitySet {
    const NAME: &str = "GENERAL_CAPS_SET";
    #[allow(clippy::manual_bits)]
    const SIZE: usize = size_of::<u32>() * 8 + size_of::<u16>() * 2;

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::SIZE);
        dst.write_u32(self.os_type);
        dst.write_u32(self.os_version);
        dst.write_u16(self.protocol_major_version);
        dst.write_u16(self.protocol_minor_version);
        dst.write_u32(self.io_code_1.bits());
        dst.write_u32(self.io_code_2);
        dst.write_u32(self.extended_pdu.bits());
        dst.write_u32(self.extra_flags_1.bits());
        dst.write_u32(self.extra_flags_2);
        dst.write_u32(self.special_type_device_cap);
        Ok(())
    }

    fn decode(src: &mut ReadCursor<'_>, version: u32) -> PduResult<Self> {
        ensure_size!(in: src, size: Self::SIZE);
        let os_type = src.read_u32();
        let os_version = src.read_u32();
        let protocol_major_version = src.read_u16();
        let protocol_minor_version = src.read_u16();
        let io_code_1 = IoCode1::from_bits_retain(src.read_u32());
        let io_code_2 = src.read_u32();
        let extended_pdu = ExtendedPdu::from_bits_retain(src.read_u32());
        let extra_flags_1 = ExtraFlags1::from_bits_retain(src.read_u32());
        let extra_flags_2 = src.read_u32();
        let special_type_device_cap = if version == GENERAL_CAPABILITY_VERSION_02 {
            src.read_u32()
        } else {
            0
        };

        Ok(Self {
            os_type,
            os_version,
            protocol_major_version,
            protocol_minor_version,
            io_code_1,
            io_code_2,
            extended_pdu,
            extra_flags_1,
            extra_flags_2,
            special_type_device_cap,
        })
    }
}

bitflags! {
    /// A 32-bit unsigned integer that identifies a bitmask of the supported I/O requests for the given device.
    /// If the bit is set, the I/O request is allowed. The requests are identified by the MajorFunction field
    /// in the Device I/O Request (section 2.2.1.4) header. This field MUST be set to a valid combination of
    /// the following values.
    #[derive(Debug, Clone, Copy)]
    struct IoCode1: u32 {
        /// Unused, always set.
        const RDPDR_IRP_MJ_CREATE = 0x0000_0001;
        /// Unused, always set.
        const RDPDR_IRP_MJ_CLEANUP = 0x0000_0002;
        /// Unused, always set.
        const RDPDR_IRP_MJ_CLOSE = 0x0000_0004;
        /// Unused, always set.
        const RDPDR_IRP_MJ_READ = 0x0000_0008;
        /// Unused, always set.
        const RDPDR_IRP_MJ_WRITE = 0x0000_0010;
        /// Unused, always set.
        const RDPDR_IRP_MJ_FLUSH_BUFFERS = 0x0000_0020;
        /// Unused, always set.
        const RDPDR_IRP_MJ_SHUTDOWN = 0x0000_0040;
        /// Unused, always set.
        const RDPDR_IRP_MJ_DEVICE_CONTROL = 0x0000_0080;
        /// Unused, always set.
        const RDPDR_IRP_MJ_QUERY_VOLUME_INFORMATION = 0x0000_0100;
        /// Unused, always set.
        const RDPDR_IRP_MJ_SET_VOLUME_INFORMATION = 0x0000_0200;
        /// Unused, always set.
        const RDPDR_IRP_MJ_QUERY_INFORMATION = 0x0000_0400;
        /// Unused, always set.
        const RDPDR_IRP_MJ_SET_INFORMATION = 0x0000_0800;
        /// Unused, always set.
        const RDPDR_IRP_MJ_DIRECTORY_CONTROL = 0x0000_1000;
        /// Unused, always set.
        const RDPDR_IRP_MJ_LOCK_CONTROL = 0x0000_2000;
        /// Enable Query Security requests (IRP_MJ_QUERY_SECURITY).
        const RDPDR_IRP_MJ_QUERY_SECURITY = 0x0000_4000;
        /// Enable Set Security requests (IRP_MJ_SET_SECURITY).
        const RDPDR_IRP_MJ_SET_SECURITY = 0x0000_8000;

        /// Combination of all the required bits.
        const REQUIRED = Self::RDPDR_IRP_MJ_CREATE.bits()
            | Self::RDPDR_IRP_MJ_CLEANUP.bits()
            | Self::RDPDR_IRP_MJ_CLOSE.bits()
            | Self::RDPDR_IRP_MJ_READ.bits()
            | Self::RDPDR_IRP_MJ_WRITE.bits()
            | Self::RDPDR_IRP_MJ_FLUSH_BUFFERS.bits()
            | Self::RDPDR_IRP_MJ_SHUTDOWN.bits()
            | Self::RDPDR_IRP_MJ_DEVICE_CONTROL.bits()
            | Self::RDPDR_IRP_MJ_QUERY_VOLUME_INFORMATION.bits()
            | Self::RDPDR_IRP_MJ_SET_VOLUME_INFORMATION.bits()
            | Self::RDPDR_IRP_MJ_QUERY_INFORMATION.bits()
            | Self::RDPDR_IRP_MJ_SET_INFORMATION.bits()
            | Self::RDPDR_IRP_MJ_DIRECTORY_CONTROL.bits()
            | Self::RDPDR_IRP_MJ_LOCK_CONTROL.bits();

    }
}

bitflags! {
    /// A 32-bit unsigned integer that specifies extended PDU flags.
    /// This field MUST be set as a bitmask of the following values.
    #[derive(Debug, Clone, Copy)]
    struct ExtendedPdu: u32 {
        /// Allow the client to send Client Drive Device List Remove packets.
        const RDPDR_DEVICE_REMOVE_PDUS = 0x0000_0001;
        /// Unused, always set.
        const RDPDR_CLIENT_DISPLAY_NAME_PDU = 0x0000_0002;
        /// Allow the server to send a Server User Logged On packet.
        const RDPDR_USER_LOGGEDON_PDU = 0x0000_0004;
    }
}

bitflags! {
    /// A 32-bit unsigned integer that specifies extended flags.
    /// The extraFlags1 field MUST be set as a bitmask of the following value.
    #[derive(Debug, Clone, Copy)]
    struct ExtraFlags1: u32 {
        /// Optionally present only in the Client Core Capability Response.
        /// Allows the server to send multiple simultaneous read or write requests
        /// on the same file from a redirected file system.
        const ENABLE_ASYNCIO = 0x0000_0001;
    }
}

/// From VersionMinor in [Server Client ID Confirm (section 2.2.2.6)], [2.2.2.3 Client Announce Reply (DR_CORE_CLIENT_ANNOUNCE_RSP)]
///
/// VERSION_MINOR_12 is what Teleport has successfully been using.
/// There is a version 13 as well, but it's not clear to me what
/// the difference is.
///
/// [Server Client ID Confirm (section 2.2.2.6)]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/bbbb9666-6994-4cf6-8e65-0d46eb319c6e
/// [2.2.2.3 Client Announce Reply (DR_CORE_CLIENT_ANNOUNCE_RSP)]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/d6fe6d1b-c145-4a6f-99aa-4fe3cdcea398
const VERSION_MINOR_12: u16 = 0x000C;
const VERSION_MAJOR: u16 = 0x0001;

/// [2.2.2.9 Client Device List Announce Request (DR_CORE_DEVICELIST_ANNOUNCE_REQ)], [2.2.3.1 Client Device List Announce (DR_DEVICELIST_ANNOUNCE)]
///
/// [2.2.2.9 Client Device List Announce Request (DR_CORE_DEVICELIST_ANNOUNCE_REQ)]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/10ef9ada-cba2-4384-ab60-7b6290ed4a9a
/// [2.2.3.1 Client Device List Announce (DR_DEVICELIST_ANNOUNCE)]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/d8b2bc1c-0207-4c15-abe3-62eaa2afcaf1
#[derive(Debug)]
pub struct ClientDeviceListAnnounce {
    pub device_list: Vec<DeviceAnnounceHeader>,
}

impl ClientDeviceListAnnounce {
    const FIXED_PART_SIZE: usize = size_of::<u32>(); // DeviceCount

    /// Library users should not typically call this directly, use [`Rdpdr::add_drive`] instead.
    pub(crate) fn new_drive(device_id: u32, name: String) -> Self {
        Self {
            device_list: vec![DeviceAnnounceHeader::new_drive(device_id, name)],
        }
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        dst.write_u32(cast_length!(
            "ClientDeviceListAnnounce",
            "DeviceCount",
            self.device_list.len()
        )?);

        for dev in self.device_list.iter() {
            dev.encode(dst)?;
        }

        Ok(())
    }

    pub fn name(&self) -> &'static str {
        "DR_CORE_DEVICELIST_ANNOUNCE_REQ"
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.device_list.iter().map(|d| d.size()).sum::<usize>()
    }
}

#[derive(Debug)]
pub struct Devices(Vec<DeviceAnnounceHeader>);

impl Devices {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn add_smartcard(&mut self, device_id: u32) {
        self.push(DeviceAnnounceHeader::new_smartcard(device_id));
    }

    pub fn add_drive(&mut self, device_id: u32, name: String) {
        self.push(DeviceAnnounceHeader::new_drive(device_id, name));
    }

    /// Returns the [`DeviceType`] for the given device ID.
    pub fn for_device_type(&self, device_id: u32) -> PduResult<DeviceType> {
        if let Some(device_type) = self.0.iter().find(|d| d.device_id == device_id).map(|d| d.device_type) {
            Ok(device_type)
        } else {
            Err(invalid_message_err!(
                "Devices::for_device_type",
                "device_id",
                "no device with that ID"
            ))
        }
    }

    fn push(&mut self, device: DeviceAnnounceHeader) {
        self.0.push(device);
    }

    pub fn clone_inner(&mut self) -> Vec<DeviceAnnounceHeader> {
        self.0.clone()
    }
}

impl Default for Devices {
    fn default() -> Self {
        Self::new()
    }
}

/// [2.2.1.3 Device Announce Header (DEVICE_ANNOUNCE)]
///
/// [2.2.1.3 Device Announce Header (DEVICE_ANNOUNCE)]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/32e34332-774b-4ead-8c9d-5d64720d6bf9
#[derive(Debug, Clone)]
pub struct DeviceAnnounceHeader {
    device_type: DeviceType,
    device_id: u32,
    preferred_dos_name: PreferredDosName,
    device_data: Vec<u8>,
}

impl DeviceAnnounceHeader {
    const FIXED_PART_SIZE: usize = size_of::<u32>() * 3 + 8; // DeviceType, DeviceId, DeviceDataLength, PreferredDosName

    pub fn new_smartcard(device_id: u32) -> Self {
        Self {
            device_type: DeviceType::Smartcard,
            device_id,
            // This name is a constant defined by the spec.
            preferred_dos_name: PreferredDosName("SCARD".to_owned()),
            device_data: Vec::new(),
        }
    }

    fn new_drive(device_id: u32, name: String) -> Self {
        // The spec says Unicode but empirically this wants null terminated UTF-8.
        let mut device_data = name.into_bytes();
        device_data.push(0u8);

        Self {
            device_type: DeviceType::Filesystem,
            device_id,
            // "The drive name MUST be specified in the PreferredDosName field; however, if the drive name is larger than the allocated size of the PreferredDosName field,
            // then the drive name MUST be truncated to fit. If the client supports DRIVE_CAPABILITY_VERSION_02 in the Drive Capability Set, then the full name MUST also
            // be specified in the DeviceData field, as a null-terminated Unicode string. If the DeviceDataLength field is nonzero, the content of the PreferredDosName field
            // is ignored."
            //
            // Since we do support DRIVE_CAPABILITY_VERSION_02, we'll put the full name in the DeviceData field.
            preferred_dos_name: PreferredDosName("ignored".to_owned()),
            device_data,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        dst.write_u32(self.device_type.into());
        dst.write_u32(self.device_id);
        self.preferred_dos_name.encode(dst)?;
        dst.write_u32(cast_length!(
            "DeviceAnnounceHeader",
            "DeviceDataLength",
            self.device_data.len()
        )?);
        dst.write_slice(&self.device_data);
        Ok(())
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.device_data.len()
    }
}

/// From ["PreferredDosName"](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/32e34332-774b-4ead-8c9d-5d64720d6bf9):
///
/// PreferredDosName (8 bytes): A string of ASCII characters (with a maximum length of eight characters) that represents the name of the device as it appears on the client. This field MUST be null-terminated, so the maximum device name is 7 characters long. The following characters are considered invalid for the PreferredDosName field:
///
/// <, >, ", /, \, |
///
/// If any of these characters are present, the DR_CORE_DEVICE_ANNOUNC_RSP packet for this device (section 2.2.2.1) will be sent with STATUS_ACCESS_DENIED set in the ResultCode field.
///
/// If DeviceType is set to RDPDR_DTYP_SMARTCARD, the PreferredDosName MUST be set to "SCARD".
///
/// Note A column character, ":", is valid only when present at the end of the PreferredDosName field, otherwise it is also considered invalid.
#[derive(Debug, Clone)]
struct PreferredDosName(String);

impl PreferredDosName {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        write_string_to_cursor(dst, &self.format(), CharacterSet::Ansi, false)
    }

    /// Returns the underlying String with a maximum length of 7 characters plus a null terminator.
    fn format(&self) -> String {
        let mut name: &str = &self.0;
        if name.len() > 7 {
            name = &name[..7];
        }
        format!("{name:\x00<8}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u32)]
pub enum DeviceType {
    /// RDPDR_DTYP_SERIAL
    Serial = 0x0000_0001,
    /// RDPDR_DTYP_PARALLEL
    Parallel = 0x0000_0002,
    /// RDPDR_DTYP_PRINT
    Print = 0x0000_0004,
    /// RDPDR_DTYP_FILESYSTEM
    Filesystem = 0x0000_0008,
    /// RDPDR_DTYP_SMARTCARD
    Smartcard = 0x0000_0020,
}

impl From<DeviceType> for u32 {
    fn from(device_type: DeviceType) -> Self {
        device_type as u32
    }
}

impl TryFrom<u32> for DeviceType {
    type Error = PduError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x0000_0001 => Ok(DeviceType::Serial),
            0x0000_0002 => Ok(DeviceType::Parallel),
            0x0000_0004 => Ok(DeviceType::Print),
            0x0000_0008 => Ok(DeviceType::Filesystem),
            0x0000_0020 => Ok(DeviceType::Smartcard),
            _ => Err(invalid_message_err!("try_from", "DeviceType", "invalid value")),
        }
    }
}

/// [2.2.2.1 Server Device Announce Response (DR_CORE_DEVICE_ANNOUNCE_RSP)]
///
/// [2.2.2.1 Server Device Announce Response (DR_CORE_DEVICE_ANNOUNCE_RSP)]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/a4c0b619-6e87-4721-bdc4-5d2db7f485f3
#[derive(Debug)]
pub struct ServerDeviceAnnounceResponse {
    pub device_id: u32,
    pub result_code: NtStatus,
}

impl ServerDeviceAnnounceResponse {
    const NAME: &str = "DR_CORE_DEVICE_ANNOUNCE_RSP";
    const FIXED_PART_SIZE: usize = size_of::<u32>() * 2; // DeviceId, ResultCode

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.device_id);
        dst.write_u32(self.result_code.into());
        Ok(())
    }

    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(ctx: Self::NAME, in: src, size: Self::FIXED_PART_SIZE);
        let device_id = src.read_u32();
        let result_code = NtStatus::from(src.read_u32());

        Ok(Self { device_id, result_code })
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [2.3.1 NTSTATUS Values]
///
/// Windows defines an absolutely massive list of potential NTSTATUS values.
/// This enum includes some basic ones for communicating with the RDP server.
///
/// [2.3.1 NTSTATUS Values]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-erref/596a1078-e883-4972-9bbc-49e60bebca55
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct NtStatus(u32);

impl NtStatus {
    /// STATUS_SUCCESS
    pub const SUCCESS: Self = Self(0x0000_0000);
    /// STATUS_UNSUCCESSFUL
    pub const UNSUCCESSFUL: Self = Self(0xC000_0001);
    /// STATUS_NOT_IMPLEMENTED
    pub const NOT_IMPLEMENTED: Self = Self(0xC000_0002);
    /// STATUS_NO_MORE_FILES
    pub const NO_MORE_FILES: Self = Self(0x8000_0006);
    /// STATUS_OBJECT_NAME_COLLISION
    pub const OBJECT_NAME_COLLISION: Self = Self(0xC000_0035);
    /// STATUS_ACCESS_DENIED
    pub const ACCESS_DENIED: Self = Self(0xC000_0022);
    /// STATUS_NOT_A_DIRECTORY
    pub const NOT_A_DIRECTORY: Self = Self(0xC000_0103);
    /// STATUS_NO_SUCH_FILE
    pub const NO_SUCH_FILE: Self = Self(0xC000_000F);
    /// STATUS_NOT_SUPPORTED
    pub const NOT_SUPPORTED: Self = Self(0xC000_00BB);
    /// STATUS_DIRECTORY_NOT_EMPTY
    pub const DIRECTORY_NOT_EMPTY: Self = Self(0xC000_0101);
}

impl Debug for NtStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            NtStatus::SUCCESS => write!(f, "STATUS_SUCCESS"),
            NtStatus::UNSUCCESSFUL => write!(f, "STATUS_UNSUCCESSFUL"),
            NtStatus::NOT_IMPLEMENTED => write!(f, "STATUS_NOT_IMPLEMENTED"),
            NtStatus::NO_MORE_FILES => write!(f, "STATUS_NO_MORE_FILES"),
            NtStatus::OBJECT_NAME_COLLISION => write!(f, "STATUS_OBJECT_NAME_COLLISION"),
            NtStatus::ACCESS_DENIED => write!(f, "STATUS_ACCESS_DENIED"),
            NtStatus::NOT_A_DIRECTORY => write!(f, "STATUS_NOT_A_DIRECTORY"),
            NtStatus::NO_SUCH_FILE => write!(f, "STATUS_NO_SUCH_FILE"),
            NtStatus::NOT_SUPPORTED => write!(f, "STATUS_NOT_SUPPORTED"),
            NtStatus::DIRECTORY_NOT_EMPTY => write!(f, "STATUS_DIRECTORY_NOT_EMPTY"),
            _ => write!(f, "{:#010X}", self.0),
        }
    }
}

impl From<u32> for NtStatus {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl From<NtStatus> for u32 {
    fn from(status: NtStatus) -> Self {
        status.0
    }
}

/// [2.2.1.4 Device I/O Request (DR_DEVICE_IOREQUEST)]
///
/// [2.2.1.4 Device I/O Request (DR_DEVICE_IOREQUEST)]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/a087ffa8-d0d5-4874-ac7b-0494f63e2d5d
#[derive(Debug, Clone)]
pub struct DeviceIoRequest {
    pub device_id: u32,
    pub file_id: u32,
    pub completion_id: u32,
    pub major_function: MajorFunction,
    pub minor_function: MinorFunction,
}

impl DeviceIoRequest {
    const NAME: &str = "DR_DEVICE_IOREQUEST";
    const FIXED_PART_SIZE: usize = size_of::<u32>() * 5; // DeviceId, FileId, CompletionId, MajorFunction, MinorFunction

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.device_id);
        dst.write_u32(self.file_id);
        dst.write_u32(self.completion_id);
        dst.write_u32(self.major_function.into());
        dst.write_u32(self.minor_function.into());
        Ok(())
    }

    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(ctx: Self::NAME, in: src, size: Self::FIXED_PART_SIZE);
        let device_id = src.read_u32();
        let file_id = src.read_u32();
        let completion_id = src.read_u32();
        let major_function = MajorFunction::try_from(src.read_u32())?;
        let minor_function = src.read_u32();

        // From the spec (linked in [`DeviceIoRequest`] doc comment):
        // "This field [MinorFunction] is valid only when the MajorFunction field
        // is set to IRP_MJ_DIRECTORY_CONTROL. If the MajorFunction field is set
        // to another value, the MinorFunction field value SHOULD be 0x00000000."
        //
        // SHOULD means implementations are not guaranteed to give us 0x00000000,
        // so handle that possibility here.
        let minor_function = if major_function == MajorFunction::DirectoryControl {
            MinorFunction::try_from(minor_function)?
        } else {
            MinorFunction::None
        };

        Ok(Self {
            device_id,
            file_id,
            completion_id,
            major_function,
            minor_function,
        })
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// See [`DeviceIoRequest`].
#[derive(Debug, PartialEq, Clone, Copy)]
#[repr(u32)]
pub enum MajorFunction {
    /// IRP_MJ_CREATE
    Create = 0x0000_0000,
    /// IRP_MJ_CLOSE
    Close = 0x0000_0002,
    /// IRP_MJ_READ
    Read = 0x0000_0003,
    /// IRP_MJ_WRITE
    Write = 0x0000_0004,
    /// IRP_MJ_DEVICE_CONTROL
    DeviceControl = 0x0000_000e,
    /// IRP_MJ_QUERY_VOLUME_INFORMATION
    QueryVolumeInformation = 0x0000_000a,
    /// IRP_MJ_SET_VOLUME_INFORMATION
    SetVolumeInformation = 0x0000_000b,
    /// IRP_MJ_QUERY_INFORMATION
    QueryInformation = 0x0000_0005,
    /// IRP_MJ_SET_INFORMATION
    SetInformation = 0x0000_0006,
    /// IRP_MJ_DIRECTORY_CONTROL
    DirectoryControl = 0x0000_000c,
    /// IRP_MJ_LOCK_CONTROL
    LockControl = 0x0000_0011,
}

impl TryFrom<u32> for MajorFunction {
    type Error = PduError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x0000_0000 => Ok(MajorFunction::Create),
            0x0000_0002 => Ok(MajorFunction::Close),
            0x0000_0003 => Ok(MajorFunction::Read),
            0x0000_0004 => Ok(MajorFunction::Write),
            0x0000_000e => Ok(MajorFunction::DeviceControl),
            0x0000_000a => Ok(MajorFunction::QueryVolumeInformation),
            0x0000_000b => Ok(MajorFunction::SetVolumeInformation),
            0x0000_0005 => Ok(MajorFunction::QueryInformation),
            0x0000_0006 => Ok(MajorFunction::SetInformation),
            0x0000_000c => Ok(MajorFunction::DirectoryControl),
            0x0000_0011 => Ok(MajorFunction::LockControl),
            _ => Err(invalid_message_err!("try_from", "MajorFunction", "unsupported value")),
        }
    }
}

impl From<MajorFunction> for u32 {
    fn from(major_function: MajorFunction) -> Self {
        major_function as u32
    }
}

/// See [`DeviceIoRequest`].
#[derive(Debug, Clone, Copy)]
#[repr(u32)]
pub enum MinorFunction {
    /// "If the MajorFunction field is set to another value, the MinorFunction field value SHOULD be 0x0000_0000."
    None = 0x0000_0000,
    /// IRP_MN_QUERY_DIRECTORY
    QueryDirectory = 0x0000_0001,
    /// IRP_MN_NOTIFY_CHANGE_DIRECTORY
    NotifyChangeDirectory = 0x0000_0002,
}

impl TryFrom<u32> for MinorFunction {
    type Error = PduError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x0000_0001 => Ok(MinorFunction::QueryDirectory),
            0x0000_0002 => Ok(MinorFunction::NotifyChangeDirectory),
            _ => Err(invalid_message_err!("try_from", "MinorFunction", "unsupported value")),
        }
    }
}

impl From<MinorFunction> for u32 {
    fn from(minor_function: MinorFunction) -> Self {
        minor_function as u32
    }
}

/// [2.2.1.4.5 Device Control Request (DR_CONTROL_REQ)]
///
/// [2.2.1.4.5 Device Control Request (DR_CONTROL_REQ)]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/30662c80-ec6e-4ed1-9004-2e6e367bb59f
#[derive(Debug)]
pub struct DeviceControlRequest<T: IoCtlCode> {
    pub header: DeviceIoRequest,
    pub output_buffer_length: u32,
    pub input_buffer_length: u32,
    pub io_control_code: T,
}

impl<T: IoCtlCode> DeviceControlRequest<T>
where
    T::Error: ironrdp_error::Source,
{
    fn headerless_size() -> usize {
        size_of::<u32>() * 3 // OutputBufferLength, InputBufferLength, IoControlCode
    }

    pub fn decode(header: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(ctx: "DeviceControlRequest", in: src, size: Self::headerless_size());
        let output_buffer_length = src.read_u32();
        let input_buffer_length = src.read_u32();
        let io_control_code = T::try_from(src.read_u32()).map_err(|e| {
            error!("Failed to parse IoCtlCode");
            invalid_message_err!("DeviceControlRequest", "IoCtlCode", "invalid IoCtlCode").with_source(e)
        })?;

        // Padding (20 bytes): An array of 20 bytes. Reserved. This field can be set to any value and MUST be ignored.
        read_padding!(src, 20);

        Ok(Self {
            header,
            output_buffer_length,
            input_buffer_length,
            io_control_code,
        })
    }
}

/// A 32-bit unsigned integer. This field is specific to the redirected device.
pub trait IoCtlCode: TryFrom<u32> {}

/// [2.2.1.5.5 Device Control Response (DR_CONTROL_RSP)]
///
/// [2.2.1.5.5 Device Control Response (DR_CONTROL_RSP)]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/a00fbce4-95bb-4e15-8182-be2b5ef9076a
#[derive(Debug)]
pub struct DeviceControlResponse {
    pub device_io_reply: DeviceIoResponse,
    pub output_buffer: Box<dyn rpce::Encode>,
}

impl DeviceControlResponse {
    const NAME: &str = "DR_CONTROL_RSP";

    pub fn new<T: IoCtlCode>(
        req: DeviceControlRequest<T>,
        io_status: NtStatus,
        output_buffer: Box<dyn rpce::Encode>,
    ) -> Self {
        Self {
            device_io_reply: DeviceIoResponse {
                device_id: req.header.device_id,
                completion_id: req.header.completion_id,
                io_status,
            },
            output_buffer,
        }
    }

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.device_io_reply.encode(dst)?;
        dst.write_u32(cast_length!(
            "DeviceControlResponse",
            "OutputBufferLength",
            self.output_buffer.size()
        )?);
        self.output_buffer.encode(dst)?;
        Ok(())
    }

    pub fn size(&self) -> usize {
        // DeviceIoResponse + OutputBufferLength + OutputBuffer
        self.device_io_reply.size() + size_of::<u32>() + self.output_buffer.size()
    }
}

/// [2.2.1.5 Device I/O Response (DR_DEVICE_IOCOMPLETION)]
///
/// [2.2.1.5 Device I/O Response (DR_DEVICE_IOCOMPLETION)]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/1c412a84-0776-4984-b35c-3f0445fcae65
#[derive(Debug)]
pub struct DeviceIoResponse {
    pub device_id: u32,
    pub completion_id: u32,
    pub io_status: NtStatus,
}

impl DeviceIoResponse {
    const NAME: &str = "DR_DEVICE_IOCOMPLETION";
    const FIXED_PART_SIZE: usize = size_of::<u32>() * 3; // DeviceId, CompletionId, IoStatus

    pub fn new(device_io_request: DeviceIoRequest, io_status: NtStatus) -> Self {
        Self {
            device_id: device_io_request.device_id,
            completion_id: device_io_request.completion_id,
            io_status,
        }
    }

    pub fn decode(src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(ctx: "DeviceIoResponse", in: src, size: Self::FIXED_PART_SIZE);
        let device_id = src.read_u32();
        let completion_id = src.read_u32();
        let io_status = NtStatus::from(src.read_u32());

        Ok(Self {
            device_id,
            completion_id,
            io_status,
        })
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.device_id);
        dst.write_u32(self.completion_id);
        dst.write_u32(self.io_status.into());
        Ok(())
    }

    pub fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

#[derive(Debug)]
pub enum FilesystemRequest {
    DeviceCreateRequest(DeviceCreateRequest),
}

impl FilesystemRequest {
    pub fn decode(dev_io_req: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        match dev_io_req.major_function {
            MajorFunction::Create => Ok(Self::DeviceCreateRequest(DeviceCreateRequest::decode(dev_io_req, src)?)),
            MajorFunction::Close => todo!(),
            MajorFunction::Read => todo!(),
            MajorFunction::Write => todo!(),
            MajorFunction::DeviceControl => todo!(),
            MajorFunction::QueryVolumeInformation => todo!(),
            MajorFunction::SetVolumeInformation => todo!(),
            MajorFunction::QueryInformation => todo!(),
            MajorFunction::SetInformation => todo!(),
            MajorFunction::DirectoryControl => todo!(),
            MajorFunction::LockControl => todo!(),
        }
    }
}

/// [2.2.3.3.1 Server Create Drive Request (DR_DRIVE_CREATE_REQ)] and [2.2.1.4.1 Device Create Request (DR_CREATE_REQ)]
///
/// [2.2.3.3.1 Server Create Drive Request (DR_DRIVE_CREATE_REQ)]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/95b16fd0-d530-407c-a310-adedc85e9897
/// [2.2.1.4.1 Device Create Request (DR_CREATE_REQ)]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/5f71f6d2-d9ff-40c2-bdb5-a739447d3c3e
#[derive(Debug)]
pub struct DeviceCreateRequest {
    /// The MajorFunction field in this header MUST be set to IRP_MJ_CREATE.
    pub device_io_request: DeviceIoRequest,
    pub desired_access: DesiredAccess,
    pub allocation_size: u64,
    pub file_attributes: FileAttributes,
    pub shared_access: SharedAccess,
    pub create_disposition: CreateDisposition,
    pub create_options: CreateOptions,
    pub path: String,
}

impl DeviceCreateRequest {
    const FIXED_PART_SIZE: usize = 4  // DesiredAccess
                                 + 8  // AllocationSize
                                 + 4  // FileAttributes
                                 + 4  // SharedAccess
                                 + 4  // CreateDisposition
                                 + 4  // CreateOptions
                                 + 4; // PathLength

    fn decode(dev_io_req: DeviceIoRequest, src: &mut ReadCursor<'_>) -> PduResult<Self> {
        ensure_size!(ctx: "DeviceCreateRequest", in: src, size: Self::FIXED_PART_SIZE);
        let desired_access = DesiredAccess::from_bits_retain(src.read_u32());
        let allocation_size = src.read_u64();
        let file_attributes = FileAttributes::from_bits_retain(src.read_u32());
        let shared_access = SharedAccess::from_bits_retain(src.read_u32());
        let create_disposition = CreateDisposition::from_bits_retain(src.read_u32());
        let create_options = CreateOptions::from_bits_retain(src.read_u32());
        let path_length: usize = cast_length!("DeviceCreateRequest", "path_length", src.read_u32())?;

        ensure_size!(ctx: "DeviceCreateRequest", in: src, size: path_length);
        let path = from_utf16_bytes(src.read_slice(path_length));

        Ok(Self {
            device_io_request: dev_io_req,
            desired_access,
            allocation_size,
            file_attributes,
            shared_access,
            create_disposition,
            create_options,
            path,
        })
    }
}

bitflags! {
    /// DesiredAccess can be interpreted as either
    /// [2.2.13.1.1 File_Pipe_Printer_Access_Mask \[MS-SMB2\]] or [2.2.13.1.2 Directory_Access_Mask \[MS-SMB2\]]
    ///
    /// This implements the combination of the two. For flags where the names and/or functions are distinct between the two,
    /// the names are appended with an "_OR_", and the File_Pipe_Printer_Access_Mask functionality is described on the top line comment,
    /// and the Directory_Access_Mask functionality is described on the bottom (2nd) line comment.
    ///
    /// [2.2.13.1.1 File_Pipe_Printer_Access_Mask \[MS-SMB2\]]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-smb2/77b36d0f-6016-458a-a7a0-0f4a72ae1534
    /// [2.2.13.1.2 Directory_Access_Mask \[MS-SMB2\]]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-smb2/0a5934b1-80f1-4da0-b1bf-5e021c309b71
    #[derive(Debug, Clone)]
    pub struct DesiredAccess: u32 {
        /// This value indicates the right to read data from the file or named pipe.
        ///
        /// This value indicates the right to enumerate the contents of the directory.
        const FILE_READ_DATA_OR_FILE_LIST_DIRECTORY = 0x00000001;
        /// This value indicates the right to write data into the file or named pipe beyond the end of the file.
        ///
        /// This value indicates the right to create a file under the directory.
        const FILE_WRITE_DATA_OR_FILE_ADD_FILE = 0x00000002;
        /// This value indicates the right to append data into the file or named pipe.
        ///
        /// This value indicates the right to add a sub-directory under the directory.
        const FILE_APPEND_DATA_OR_FILE_ADD_SUBDIRECTORY = 0x00000004;
        /// This value indicates the right to read the extended attributes of the file or named pipe.
        const FILE_READ_EA = 0x00000008;
        /// This value indicates the right to write or change the extended attributes to the file or named pipe.
        const FILE_WRITE_EA = 0x00000010;
        /// This value indicates the right to traverse this directory if the server enforces traversal checking.
        const FILE_TRAVERSE = 0x00000020;
        /// This value indicates the right to delete entries within a directory.
        const FILE_DELETE_CHILD = 0x00000040;
        /// This value indicates the right to execute the file/directory.
        const FILE_EXECUTE = 0x00000020;
        /// This value indicates the right to read the attributes of the file/directory.
        const FILE_READ_ATTRIBUTES = 0x00000080;
        /// This value indicates the right to change the attributes of the file/directory.
        const FILE_WRITE_ATTRIBUTES = 0x00000100;
        /// This value indicates the right to delete the file/directory.
        const DELETE = 0x00010000;
        /// This value indicates the right to read the security descriptor for the file/directory or named pipe.
        const READ_CONTROL = 0x00020000;
        /// This value indicates the right to change the discretionary access control list (DACL) in the security descriptor for the file/directory or named pipe. For the DACL data pub structure, see ACL in [MS-DTYP].
        const WRITE_DAC = 0x00040000;
        /// This value indicates the right to change the owner in the security descriptor for the file/directory or named pipe.
        const WRITE_OWNER = 0x00080000;
        /// SMB2 clients set this flag to any value. SMB2 servers SHOULD ignore this flag.
        const SYNCHRONIZE = 0x00100000;
        /// This value indicates the right to read or change the system access control list (SACL) in the security descriptor for the file/directory or named pipe. For the SACL data pub structure, see ACL in [MS-DTYP].
        const ACCESS_SYSTEM_SECURITY = 0x01000000;
        /// This value indicates that the client is requesting an open to the file with the highest level of access the client has on this file. If no access is granted for the client on this file, the server MUST fail the open with STATUS_ACCESS_DENIED.
        const MAXIMUM_ALLOWED = 0x02000000;
        /// This value indicates a request for all the access flags that are previously listed except MAXIMUM_ALLOWED and ACCESS_SYSTEM_SECURITY.
        const GENERIC_ALL = 0x10000000;
        /// This value indicates a request for the following combination of access flags listed above: FILE_READ_ATTRIBUTES| FILE_EXECUTE| SYNCHRONIZE| READ_CONTROL.
        const GENERIC_EXECUTE = 0x20000000;
        /// This value indicates a request for the following combination of access flags listed above: FILE_WRITE_DATA| FILE_APPEND_DATA| FILE_WRITE_ATTRIBUTES| FILE_WRITE_EA| SYNCHRONIZE| READ_CONTROL.
        const GENERIC_WRITE = 0x40000000;
        /// This value indicates a request for the following combination of access flags listed above: FILE_READ_DATA| FILE_READ_ATTRIBUTES| FILE_READ_EA| SYNCHRONIZE| READ_CONTROL.
        const GENERIC_READ = 0x80000000;
    }
}

bitflags! {
    /// [2.6 File Attributes \[MS-FSCC\]]
    ///
    /// [2.6 File Attributes \[MS-FSCC\]]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-fscc/ca28ec38-f155-4768-81d6-4bfeb8586fc9
    #[derive(Debug, Clone)]
    pub struct FileAttributes: u32 {
        const FILE_ATTRIBUTE_READONLY = 0x00000001;
        const FILE_ATTRIBUTE_HIDDEN = 0x00000002;
        const FILE_ATTRIBUTE_SYSTEM = 0x00000004;
        const FILE_ATTRIBUTE_DIRECTORY = 0x00000010;
        const FILE_ATTRIBUTE_ARCHIVE = 0x00000020;
        const FILE_ATTRIBUTE_NORMAL = 0x00000080;
        const FILE_ATTRIBUTE_TEMPORARY = 0x00000100;
        const FILE_ATTRIBUTE_SPARSE_FILE = 0x00000200;
        const FILE_ATTRIBUTE_REPARSE_POINT = 0x00000400;
        const FILE_ATTRIBUTE_COMPRESSED = 0x00000800;
        const FILE_ATTRIBUTE_OFFLINE = 0x00001000;
        const FILE_ATTRIBUTE_NOT_CONTENT_INDEXED = 0x00002000;
        const FILE_ATTRIBUTE_ENCRYPTED = 0x00004000;
        const FILE_ATTRIBUTE_INTEGRITY_STREAM = 0x00008000;
        const FILE_ATTRIBUTE_NO_SCRUB_DATA = 0x00020000;
        const FILE_ATTRIBUTE_RECALL_ON_OPEN = 0x00040000;
        const FILE_ATTRIBUTE_PINNED = 0x00080000;
        const FILE_ATTRIBUTE_UNPINNED = 0x00100000;
        const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS = 0x00400000;
    }
}

bitflags! {
    /// Specified in [2.2.13 SMB2 CREATE Request]
    ///
    /// [2.2.13 SMB2 CREATE Request]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-smb2/e8fb45c1-a03d-44ca-b7ae-47385cfd7997
    #[derive(Debug, Clone)]
    pub struct SharedAccess: u32 {
        const FILE_SHARE_READ = 0x00000001;
        const FILE_SHARE_WRITE = 0x00000002;
        const FILE_SHARE_DELETE = 0x00000004;
    }
}

bitflags! {
    /// Defined in [2.2.13 SMB2 CREATE Request]
    ///
    /// See FreeRDP's [drive_file.c] for context about how these should be interpreted.
    ///
    /// [2.2.13 SMB2 CREATE Request]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-smb2/e8fb45c1-a03d-44ca-b7ae-47385cfd7997
    /// [drive_file.c]: https://github.com/FreeRDP/FreeRDP/blob/511444a65e7aa2f537c5e531fa68157a50c1bd4d/channels/drive/client/drive_file.c#L207
    #[derive(PartialEq, Eq, Debug, Clone)]
    pub struct CreateDisposition: u32 {
        const FILE_SUPERSEDE = 0x00000000;
        const FILE_OPEN = 0x00000001;
        const FILE_CREATE = 0x00000002;
        const FILE_OPEN_IF = 0x00000003;
        const FILE_OVERWRITE = 0x00000004;
        const FILE_OVERWRITE_IF = 0x00000005;
    }
}

bitflags! {
    /// Defined in [2.2.13 SMB2 CREATE Request]
    ///
    /// [2.2.13 SMB2 CREATE Request]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-smb2/e8fb45c1-a03d-44ca-b7ae-47385cfd7997
    #[derive(Debug, Clone)]
    pub struct CreateOptions: u32 {
        const FILE_DIRECTORY_FILE = 0x00000001;
        const FILE_WRITE_THROUGH = 0x00000002;
        const FILE_SEQUENTIAL_ONLY = 0x00000004;
        const FILE_NO_INTERMEDIATE_BUFFERING = 0x00000008;
        const FILE_SYNCHRONOUS_IO_ALERT = 0x00000010;
        const FILE_SYNCHRONOUS_IO_NONALERT = 0x00000020;
        const FILE_NON_DIRECTORY_FILE = 0x00000040;
        const FILE_COMPLETE_IF_OPLOCKED = 0x00000100;
        const FILE_NO_EA_KNOWLEDGE = 0x00000200;
        const FILE_RANDOM_ACCESS = 0x00000800;
        const FILE_DELETE_ON_CLOSE = 0x00001000;
        const FILE_OPEN_BY_FILE_ID = 0x00002000;
        const FILE_OPEN_FOR_BACKUP_INTENT = 0x00004000;
        const FILE_NO_COMPRESSION = 0x00008000;
        const FILE_OPEN_REMOTE_INSTANCE = 0x00000400;
        const FILE_OPEN_REQUIRING_OPLOCK = 0x00010000;
        const FILE_DISALLOW_EXCLUSIVE = 0x00020000;
        const FILE_RESERVE_OPFILTER = 0x00100000;
        const FILE_OPEN_REPARSE_POINT = 0x00200000;
        const FILE_OPEN_NO_RECALL = 0x00400000;
        const FILE_OPEN_FOR_FREE_SPACE_QUERY = 0x00800000;
    }
}

/// [2.2.1.5.1 Device Create Response (DR_CREATE_RSP)]
///
/// [2.2.1.5.1 Device Create Response (DR_CREATE_RSP)]: https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/99e5fca5-b37a-41e4-bc69-8d7da7860f76
#[derive(Debug)]
pub struct DeviceCreateResponse {
    pub device_io_reply: DeviceIoResponse,
    pub file_id: u32,
    pub information: Information,
}

impl DeviceCreateResponse {
    const NAME: &str = "DR_CREATE_RSP";

    pub fn name(&self) -> &'static str {
        Self::NAME
    }

    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.device_io_reply.encode(dst)?;
        dst.write_u32(self.file_id);
        dst.write_u8(self.information.bits());
        Ok(())
    }

    pub fn size(&self) -> usize {
        self.device_io_reply.size() // DeviceIoReply
        + 4 // FileId
        + 1 // Information
    }
}

bitflags! {
    /// Defined in [2.2.1.5.1 Device Create Response (DR_CREATE_RSP)]
    ///
    /// [2.2.1.5.1 Device Create Response (DR_CREATE_RSP)]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/99e5fca5-b37a-41e4-bc69-8d7da7860f76
    #[derive(Debug)]
    pub struct Information: u8 {
        /// A new file was created.
        const FILE_SUPERSEDED = 0x00000000;
        /// An existing file was opened.
        const FILE_OPENED = 0x00000001;
        /// An existing file was overwritten.
        const FILE_OVERWRITTEN = 0x00000003;
    }
}
