use std::io;

use byteorder::{ReadBytesExt as _, WriteBytesExt as _};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};

use crate::{geometry::InclusiveRectangle, PduParsing};

#[repr(u8)]
#[derive(Debug, PartialEq, Eq, FromPrimitive, ToPrimitive, Clone)]
pub enum AllowDisplayUpdatesType {
    SuppressDisplayUpdates = 0x00,
    AllowDisplayUpdates = 0x01,
}

/// [2.2.11.3.1] Suppress Output PDU Data (TS_SUPPRESS_OUTPUT_PDU)
///
/// The Suppress Output PDU is sent by the client to toggle all display updates
/// from the server. This packet does not end the session or socket connection.
/// Typically, a client sends this packet when its window is either minimized or
/// restored. Server support for this PDU is indicated in the General Capability
/// Set [2.2.7.1.1].
///
/// [2.2.11.3.1] https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/0be71491-0b01-402c-947d-080706ccf91b
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuppressOutputPdu {
    allow_display_updates: AllowDisplayUpdatesType,
    desktop_rect: Option<InclusiveRectangle>,
}

impl PduParsing for SuppressOutputPdu {
    type Error = io::Error;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let allow_display_updates = AllowDisplayUpdatesType::from_u8(stream.read_u8()?)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid display update type"))?;
        let _padding = stream.read_u8()?; // padding
        let _padding = stream.read_u8()?; // padding
        let _padding = stream.read_u8()?; // padding
        let desktop_rect = if allow_display_updates == AllowDisplayUpdatesType::AllowDisplayUpdates {
            Some(InclusiveRectangle::from_buffer(&mut stream)?)
        } else {
            None
        };
        Ok(Self {
            allow_display_updates,
            desktop_rect,
        })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u8(self.allow_display_updates.to_u8().unwrap())?;
        stream.write_u8(0)?; // padding
        stream.write_u8(0)?; // padding
        stream.write_u8(0)?; // padding
        if let Some(rect) = &self.desktop_rect {
            rect.to_buffer(&mut stream)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        1 // allowDisplayUpdates
        + 3 // pad3Octets
        + self.desktop_rect.as_ref().map_or(0, |r| r.buffer_length()) // desktopRect
    }
}
