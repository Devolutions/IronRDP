use crate::geometry::InclusiveRectangle;
use crate::{PduDecode, PduEncode, PduResult};
use ironrdp_core::{ReadCursor, WriteCursor};

#[repr(u8)]
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum AllowDisplayUpdatesType {
    SuppressDisplayUpdates = 0x00,
    AllowDisplayUpdates = 0x01,
}

impl AllowDisplayUpdatesType {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::SuppressDisplayUpdates),
            0x01 => Some(Self::AllowDisplayUpdates),
            _ => None,
        }
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// [2.2.11.3.1] Suppress Output PDU Data (TS_SUPPRESS_OUTPUT_PDU)
///
/// The Suppress Output PDU is sent by the client to toggle all display updates
/// from the server. This packet does not end the session or socket connection.
/// Typically, a client sends this packet when its window is either minimized or
/// restored. Server support for this PDU is indicated in the General Capability
/// Set [2.2.7.1.1].
///
/// [2.2.11.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/0be71491-0b01-402c-947d-080706ccf91b
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuppressOutputPdu {
    pub desktop_rect: Option<InclusiveRectangle>,
}

impl SuppressOutputPdu {
    const NAME: &'static str = "SuppressOutputPdu";

    const FIXED_PART_SIZE: usize = 1 /* allowDisplayUpdates */ + 3 /* pad */;
}

impl PduEncode for SuppressOutputPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());

        let allow_display_updates = if self.desktop_rect.is_some() {
            AllowDisplayUpdatesType::AllowDisplayUpdates
        } else {
            AllowDisplayUpdatesType::SuppressDisplayUpdates
        };

        dst.write_u8(allow_display_updates.as_u8());
        write_padding!(dst, 3);
        if let Some(rect) = &self.desktop_rect {
            rect.encode(dst)?;
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.desktop_rect.as_ref().map_or(0, |r| r.size())
        // desktopRect
    }
}

impl<'de> PduDecode<'de> for SuppressOutputPdu {
    fn decode(src: &mut ReadCursor<'de>) -> PduResult<Self> {
        ensure_fixed_part_size!(in: src);

        let allow_display_updates = AllowDisplayUpdatesType::from_u8(src.read_u8())
            .ok_or_else(|| invalid_field_err!("allowDisplayUpdates", "invalid display update type"))?;
        read_padding!(src, 3);
        let desktop_rect = if allow_display_updates == AllowDisplayUpdatesType::AllowDisplayUpdates {
            Some(InclusiveRectangle::decode(src)?)
        } else {
            None
        };
        Ok(Self { desktop_rect })
    }
}
