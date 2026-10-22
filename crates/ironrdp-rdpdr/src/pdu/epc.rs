//! PDUs for [\[MS-RDPEPC\]: Remote Desktop Protocol: Print Virtual Channel Extension]
//! that use the `RDPDR_CTYP_PRN` component.
//!
//! Printer device announcements and printer IRPs are built on [MS-RDPEFS] structures and live in
//! [`super::efs`].
//!
//! [\[MS-RDPEPC\]: Remote Desktop Protocol: Print Virtual Channel Extension]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpepc/
//! [MS-RDPEFS]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpefs/34d9de58-b2b5-40b6-b970-f82d4603bdb5

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_fixed_part_size,
    ensure_size, invalid_field_err, unsupported_value_err,
};
use ironrdp_pdu::utils::{CharacterSet, encoded_str_len, from_utf16_bytes, write_string_to_cursor};

use super::{Component, PacketId, SharedHeader};

/// [MS-RDPEPC 2.2.1.1] `RDPDR_ADD_PRINTER_EVENT`.
pub const RDPDR_ADD_PRINTER_EVENT: u32 = 0x0000_0001;
/// [MS-RDPEPC 2.2.1.1] `RDPDR_UPDATE_PRINTER_EVENT`.
pub const RDPDR_UPDATE_PRINTER_EVENT: u32 = 0x0000_0002;
/// [MS-RDPEPC 2.2.1.1] `RDPDR_DELETE_PRINTER_EVENT`.
pub const RDPDR_DELETE_PRINTER_EVENT: u32 = 0x0000_0003;
/// [MS-RDPEPC 2.2.1.1] `RDPDR_RENAME_PRINTER_EVENT`.
pub const RDPDR_RENAME_PRINTER_EVENT: u32 = 0x0000_0004;

/// Server printer cache-data message (`PAKID_PRN_CACHE_DATA`).
///
/// The server sends these when a user changes a redirected queue on the server, so the client can
/// replay the change as `CachedPrinterConfigData` the next time it announces the printer
/// ([MS-RDPEPC 3.1.1.1]). The configuration data is opaque to the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrinterCacheData {
    /// [MS-RDPEPC 2.2.2.3] Add Printer Cachedata (`DR_PRN_ADD_CACHEDATA`).
    Add(AddPrinterCacheData),
    /// [MS-RDPEPC 2.2.2.4] Update Printer Cachedata (`DR_PRN_UPDATE_CACHEDATA`).
    Update(UpdatePrinterCacheData),
    /// [MS-RDPEPC 2.2.2.5] Delete Printer Cachedata (`DR_PRN_DELETE_CACHEDATA`).
    Delete(DeletePrinterCacheData),
    /// [MS-RDPEPC 2.2.2.6] Rename Printer Cachedata (`DR_PRN_RENAME_CACHEDATA`).
    Rename(RenamePrinterCacheData),
}

/// Sent when a queue is created manually on the client's redirected port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddPrinterCacheData {
    /// ASCII port name of at most 8 characters, without NUL padding.
    pub port_dos_name: String,
    /// Ignored on receipt.
    pub pnp_name: String,
    /// Server driver associated with the configuration data.
    pub driver_name: String,
    /// Client printer name the configuration belongs to.
    pub printer_name: String,
    /// Opaque `CachedPrinterConfigData` blob.
    pub config_data: Vec<u8>,
}

/// Sent when the configuration of a redirected queue changes on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePrinterCacheData {
    /// Client printer name the configuration belongs to.
    pub printer_name: String,
    /// Opaque `CachedPrinterConfigData` blob.
    pub config_data: Vec<u8>,
}

/// Sent when a redirected queue is deleted on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletePrinterCacheData {
    /// Client printer name whose cached configuration must be deleted.
    pub printer_name: String,
}

/// Sent when the user renames a redirected queue on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePrinterCacheData {
    /// Printer name before the rename.
    pub old_printer_name: String,
    /// Printer name after the rename.
    pub new_printer_name: String,
}

impl PrinterCacheData {
    const NAME: &'static str = "PrinterCacheData";

    const FIXED_PART_SIZE: usize = SharedHeader::SIZE + 4 /* EventId */;

    const ADD_FIXED_PART_SIZE: usize = 8 /* PortDosName */
        + 4 /* PnPNameLen */
        + 4 /* DriverNameLen */
        + 4 /* PrintNameLen */
        + 4 /* CachedFieldsLen */;

    const UPDATE_FIXED_PART_SIZE: usize = 4 /* PrinterNameLen */ + 4 /* ConfigDataLen */;

    const DELETE_FIXED_PART_SIZE: usize = 4 /* PrinterNameLen */;

    const RENAME_FIXED_PART_SIZE: usize = 4 /* OldPrinterNameLen */ + 4 /* NewPrinterNameLen */;

    /// Decodes the message body that follows an already-decoded `PAKID_PRN_CACHE_DATA` header.
    pub fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(ctx: Self::NAME, in: src, size: 4 /* EventId */);
        let event_id = src.read_u32();

        match event_id {
            RDPDR_ADD_PRINTER_EVENT => {
                ensure_size!(ctx: Self::NAME, in: src, size: Self::ADD_FIXED_PART_SIZE);
                let port_dos_name = src.read_slice(8);
                let port_dos_name = port_dos_name
                    .iter()
                    .take_while(|byte| **byte != 0)
                    .map(|byte| char::from(*byte))
                    .collect::<String>();
                let pnp_name_len = read_length(src, "PnPNameLen")?;
                let driver_name_len = read_length(src, "DriverNameLen")?;
                let print_name_len = read_length(src, "PrintNameLen")?;
                let cached_fields_len = read_length(src, "CachedFieldsLen")?;

                Ok(Self::Add(AddPrinterCacheData {
                    port_dos_name,
                    pnp_name: read_unicode(src, pnp_name_len)?,
                    driver_name: read_unicode(src, driver_name_len)?,
                    printer_name: read_unicode(src, print_name_len)?,
                    config_data: read_bytes(src, cached_fields_len)?,
                }))
            }
            RDPDR_UPDATE_PRINTER_EVENT => {
                ensure_size!(ctx: Self::NAME, in: src, size: Self::UPDATE_FIXED_PART_SIZE);
                let printer_name_len = read_length(src, "PrinterNameLen")?;
                let config_data_len = read_length(src, "ConfigDataLen")?;

                Ok(Self::Update(UpdatePrinterCacheData {
                    printer_name: read_unicode(src, printer_name_len)?,
                    config_data: read_bytes(src, config_data_len)?,
                }))
            }
            RDPDR_DELETE_PRINTER_EVENT => {
                ensure_size!(ctx: Self::NAME, in: src, size: Self::DELETE_FIXED_PART_SIZE);
                let printer_name_len = read_length(src, "PrinterNameLen")?;

                Ok(Self::Delete(DeletePrinterCacheData {
                    printer_name: read_unicode(src, printer_name_len)?,
                }))
            }
            RDPDR_RENAME_PRINTER_EVENT => {
                ensure_size!(ctx: Self::NAME, in: src, size: Self::RENAME_FIXED_PART_SIZE);
                let old_printer_name_len = read_length(src, "OldPrinterNameLen")?;
                let new_printer_name_len = read_length(src, "NewPrinterNameLen")?;

                Ok(Self::Rename(RenamePrinterCacheData {
                    old_printer_name: read_unicode(src, old_printer_name_len)?,
                    new_printer_name: read_unicode(src, new_printer_name_len)?,
                }))
            }
            _ => Err(unsupported_value_err!(
                "EventId",
                format!("unknown printer cache event {event_id:#010x}"),
                in: src
            )),
        }
    }

    /// Returns the client printer name the message refers to.
    ///
    /// A rename refers to the name before the rename.
    pub fn printer_name(&self) -> &str {
        match self {
            Self::Add(pdu) => &pdu.printer_name,
            Self::Update(pdu) => &pdu.printer_name,
            Self::Delete(pdu) => &pdu.printer_name,
            Self::Rename(pdu) => &pdu.old_printer_name,
        }
    }

    fn event_id(&self) -> u32 {
        match self {
            Self::Add(_) => RDPDR_ADD_PRINTER_EVENT,
            Self::Update(_) => RDPDR_UPDATE_PRINTER_EVENT,
            Self::Delete(_) => RDPDR_DELETE_PRINTER_EVENT,
            Self::Rename(_) => RDPDR_RENAME_PRINTER_EVENT,
        }
    }
}

impl Encode for PrinterCacheData {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        SharedHeader {
            component: Component::RdpdrCtypPrn,
            packet_id: PacketId::PrnCacheData,
        }
        .encode(dst)?;
        dst.write_u32(self.event_id());

        match self {
            Self::Add(pdu) => {
                if pdu.port_dos_name.len() > 8 || !pdu.port_dos_name.is_ascii() {
                    return Err(invalid_field_err!(
                        "PortDosName",
                        "must contain at most 8 ASCII characters",
                        in: dst
                    ));
                }
                let mut port_dos_name = [0u8; 8];
                port_dos_name[..pdu.port_dos_name.len()].copy_from_slice(pdu.port_dos_name.as_bytes());
                dst.write_slice(&port_dos_name);
                write_length(dst, "PnPNameLen", optional_unicode_len(&pdu.pnp_name))?;
                write_length(dst, "DriverNameLen", unicode_len(&pdu.driver_name))?;
                write_length(dst, "PrintNameLen", unicode_len(&pdu.printer_name))?;
                write_length(dst, "CachedFieldsLen", pdu.config_data.len())?;
                if !pdu.pnp_name.is_empty() {
                    write_unicode(dst, &pdu.pnp_name)?;
                }
                write_unicode(dst, &pdu.driver_name)?;
                write_unicode(dst, &pdu.printer_name)?;
                dst.write_slice(&pdu.config_data);
            }
            Self::Update(pdu) => {
                write_length(dst, "PrinterNameLen", unicode_len(&pdu.printer_name))?;
                write_length(dst, "ConfigDataLen", pdu.config_data.len())?;
                write_unicode(dst, &pdu.printer_name)?;
                dst.write_slice(&pdu.config_data);
            }
            Self::Delete(pdu) => {
                write_length(dst, "PrinterNameLen", unicode_len(&pdu.printer_name))?;
                write_unicode(dst, &pdu.printer_name)?;
            }
            Self::Rename(pdu) => {
                write_length(dst, "OldPrinterNameLen", unicode_len(&pdu.old_printer_name))?;
                write_length(dst, "NewPrinterNameLen", unicode_len(&pdu.new_printer_name))?;
                write_unicode(dst, &pdu.old_printer_name)?;
                write_unicode(dst, &pdu.new_printer_name)?;
            }
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
            + match self {
                Self::Add(pdu) => {
                    Self::ADD_FIXED_PART_SIZE
                        + optional_unicode_len(&pdu.pnp_name)
                        + unicode_len(&pdu.driver_name)
                        + unicode_len(&pdu.printer_name)
                        + pdu.config_data.len()
                }
                Self::Update(pdu) => {
                    Self::UPDATE_FIXED_PART_SIZE + unicode_len(&pdu.printer_name) + pdu.config_data.len()
                }
                Self::Delete(pdu) => Self::DELETE_FIXED_PART_SIZE + unicode_len(&pdu.printer_name),
                Self::Rename(pdu) => {
                    Self::RENAME_FIXED_PART_SIZE
                        + unicode_len(&pdu.old_printer_name)
                        + unicode_len(&pdu.new_printer_name)
                }
            }
    }
}

impl<'de> Decode<'de> for PrinterCacheData {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let header = SharedHeader::decode(src)?;
        if !matches!(header.component, Component::RdpdrCtypPrn) || header.packet_id != PacketId::PrnCacheData {
            return Err(invalid_field_err!("SharedHeader", "expected PAKID_PRN_CACHE_DATA", in: src));
        }
        Self::decode_body(src)
    }
}

/// [MS-RDPEPC 2.2.2.2] Server Printer Set XPS Mode (`DR_PRN_USING_XPS`).
///
/// The server sends this only for printers announced with `RDPDR_PRINTER_ANNOUNCE_FLAG_XPSFORMAT`,
/// before any write request that carries XPS data ([MS-RDPEPC 3.2.5.1.2]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrinterUsingXps {
    /// Device ID of the printer switched to XPS mode.
    pub printer_id: u32,
    /// Unused; any value is accepted.
    pub flags: u32,
}

impl PrinterUsingXps {
    const NAME: &'static str = "PrinterUsingXps";

    const FIXED_PART_SIZE: usize = SharedHeader::SIZE + 4 /* PrinterId */ + 4 /* Flags */;

    /// Decodes the message body that follows an already-decoded `PAKID_PRN_USING_XPS` header.
    pub fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(ctx: Self::NAME, in: src, size: 4 /* PrinterId */ + 4 /* Flags */);
        Ok(Self {
            printer_id: src.read_u32(),
            flags: src.read_u32(),
        })
    }
}

impl Encode for PrinterUsingXps {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        SharedHeader {
            component: Component::RdpdrCtypPrn,
            packet_id: PacketId::PrnUsingXps,
        }
        .encode(dst)?;
        dst.write_u32(self.printer_id);
        dst.write_u32(self.flags);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl<'de> Decode<'de> for PrinterUsingXps {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let header = SharedHeader::decode(src)?;
        if !matches!(header.component, Component::RdpdrCtypPrn) || header.packet_id != PacketId::PrnUsingXps {
            return Err(invalid_field_err!("SharedHeader", "expected PAKID_PRN_USING_XPS", in: src));
        }
        Self::decode_body(src)
    }
}

fn read_length(src: &mut ReadCursor<'_>, field: &'static str) -> DecodeResult<usize> {
    cast_length!(PrinterCacheData::NAME, field, src.read_u32(), in: src)
}

fn read_bytes(src: &mut ReadCursor<'_>, length: usize) -> DecodeResult<Vec<u8>> {
    ensure_size!(ctx: PrinterCacheData::NAME, in: src, size: length);
    Ok(src.read_slice(length).to_vec())
}

fn read_unicode(src: &mut ReadCursor<'_>, length: usize) -> DecodeResult<String> {
    ensure_size!(ctx: PrinterCacheData::NAME, in: src, size: length);
    if !length.is_multiple_of(2) {
        return Err(invalid_field_err!(
            "PrinterName",
            "Unicode string length must be a multiple of two",
            in: src
        ));
    }
    Ok(from_utf16_bytes(src.read_slice(length))
        .trim_end_matches('\0')
        .to_owned())
}

fn unicode_len(value: &str) -> usize {
    encoded_str_len(value, CharacterSet::Unicode, true)
}

/// Length of a string that is omitted entirely when empty, like `PnPName` (MS-RDPEPC 2.2.2.1).
fn optional_unicode_len(value: &str) -> usize {
    if value.is_empty() { 0 } else { unicode_len(value) }
}

fn write_length(dst: &mut WriteCursor<'_>, field: &'static str, length: usize) -> EncodeResult<()> {
    dst.write_u32(cast_length!(PrinterCacheData::NAME, field, length)?);
    Ok(())
}

fn write_unicode(dst: &mut WriteCursor<'_>, value: &str) -> EncodeResult<()> {
    write_string_to_cursor(dst, value, CharacterSet::Unicode, true)
}
