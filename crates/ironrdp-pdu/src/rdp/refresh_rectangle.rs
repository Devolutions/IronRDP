use std::io;

use byteorder::{ReadBytesExt as _, WriteBytesExt as _};

use crate::{geometry::InclusiveRectangle, PduParsing};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshRectanglePdu {
    areas_to_refresh: Vec<InclusiveRectangle>,
}

/// [2.2.11.2.1] Refresh Rect PDU Data (TS_REFRESH_RECT_PDU)
///
/// The Refresh Rect PDU allows the client to request that the server redraw one
/// or more rectangles of the session screen area. The client can use it to
/// repaint sections of the client window that were obscured by local
/// applications. Server support for this PDU is indicated in the General
/// Capability Set (section [2.2.7.1.1].
///
/// [2.2.11.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/fe04a39d-dc10-489f-bea7-08dad5538547
impl PduParsing for RefreshRectanglePdu {
    type Error = io::Error;

    fn from_buffer(mut stream: impl io::Read) -> Result<Self, Self::Error> {
        let number_of_areas = stream.read_u8()?;
        let _padding = stream.read_u8()?; // padding
        let _padding = stream.read_u8()?; // padding
        let _padding = stream.read_u8()?; // padding
        let areas_to_refresh = (0..number_of_areas)
            .map(|_| InclusiveRectangle::from_buffer(&mut stream))
            .collect::<Result<Vec<_>, Self::Error>>()?;

        Ok(Self { areas_to_refresh })
    }

    fn to_buffer(&self, mut stream: impl io::Write) -> Result<(), Self::Error> {
        stream.write_u8(self.areas_to_refresh.len() as u8)?;
        stream.write_u8(0)?; // padding
        stream.write_u8(0)?; // padding
        stream.write_u8(0)?; // padding
        for rectangle in self.areas_to_refresh.iter() {
            rectangle.to_buffer(&mut stream)?;
        }

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        1 // numberOfAreas
        + 3 // pad3Octets
        + self.areas_to_refresh.iter().map(|r| r.buffer_length()).sum::<usize>()
        // areasToRefresh
    }
}
