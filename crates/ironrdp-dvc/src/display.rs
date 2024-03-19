//! Display Control Virtual Channel
//! [[MS-RDPEDISP]]
//!
//! [[MS-RDPEDISP]]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/d2954508-f487-48bc-8731-39743e0854a9
use crate::encode_dvc_messages;
use crate::vec;
use crate::Box;
use crate::DvcClientProcessor;
use crate::DvcMessages;
use crate::DvcPduEncode;
use crate::DvcProcessor;
use crate::PduResult;
use crate::SvcMessage;
use crate::Vec;
use bitflags::bitflags;
use ironrdp_pdu::cast_length;
use ironrdp_pdu::cursor::WriteCursor;
use ironrdp_pdu::ensure_size;
use ironrdp_pdu::other_err;
use ironrdp_pdu::write_buf::WriteBuf;
use ironrdp_pdu::PduEncode;
use ironrdp_pdu::PduParsing;
use ironrdp_svc::impl_as_any;

/// A client for the Display Control Virtual Channel.
pub struct DisplayControlClient {}

impl_as_any!(DisplayControlClient);

impl DvcProcessor for DisplayControlClient {
    fn channel_name(&self) -> &str {
        "Microsoft::Windows::RDS::DisplayControl"
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<DvcMessages> {
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<DvcMessages> {
        // TODO: We can parse the payload here for completeness sake,
        // in practice we don't need to do anything with the payload.
        debug!("Got Display PDU of length: {}", payload.len());
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for DisplayControlClient {}

impl DisplayControlClient {
    pub fn new() -> Self {
        Self {}
    }

    /// Fully encodes a [`MonitorLayoutPdu`] with the given monitors.
    pub fn encode_monitors(&self, channel_id: u32, monitors: Vec<Monitor>) -> PduResult<Vec<SvcMessage>> {
        let mut buf = WriteBuf::new();
        let pdu = MonitorLayoutPdu::new(monitors);
        encode_dvc_messages(channel_id, vec![Box::new(pdu)], None)
    }
}

impl Default for DisplayControlClient {
    fn default() -> Self {
        Self::new()
    }
}

/// [2.2.1.1] DISPLAYCONTROL_HEADER
///
/// [2.2.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/3dceb555-2faf-4596-9e74-62be820df8ba
pub struct Header {
    pdu_type: DisplayControlType,
    length: usize,
}

impl Header {
    pub fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::size());
        dst.write_u32(cast_length!("Type", self.pdu_type)?);
        dst.write_u32(cast_length!("Length", self.length)?);
        Ok(())
    }

    pub fn size() -> usize {
        4 /* pdu_type */ + 4 /* length */
    }
}

/// [2.2.2.2] DISPLAYCONTROL_MONITOR_LAYOUT_PDU
///
/// [2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/22741217-12a0-4fb8-b5a0-df43905aaf06
pub struct MonitorLayoutPdu {
    header: Header,
    pub monitors: Vec<Monitor>,
}

impl MonitorLayoutPdu {
    pub fn new(monitors: Vec<Monitor>) -> Self {
        Self {
            header: Header {
                pdu_type: DisplayControlType::MonitorLayout,
                length: (Header::size() + 4 /* MonitorLayoutSize */ + 4 /* NumMonitors */ + (monitors.len() * Monitor::size())),
            },
            monitors,
        }
    }
}

impl PduEncode for MonitorLayoutPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.header.encode(dst)?;
        dst.write_u32(cast_length!("MonitorLayoutSize", Monitor::size())?);
        dst.write_u32(cast_length!("NumMonitors", self.monitors.len())?);
        for monitor in &self.monitors {
            monitor.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "DISPLAYCONTROL_MONITOR_LAYOUT_PDU"
    }

    fn size(&self) -> usize {
        self.header.length
    }
}

impl DvcPduEncode for MonitorLayoutPdu {}

/// [2.2.2.2.1] DISPLAYCONTROL_MONITOR_LAYOUT_PDU
///
/// [2.2.2.2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpedisp/ea2de591-9203-42cd-9908-be7a55237d1c
pub struct Monitor {
    pub flags: MonitorFlags,
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
    pub physical_width: u32,
    pub physical_height: u32,
    pub orientation: Orientation,
    pub desktop_scale_factor: u32,
    pub device_scale_factor: u32,
}

impl Monitor {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> PduResult<()> {
        ensure_size!(in: dst, size: Self::size());
        dst.write_u32(self.flags.bits());
        dst.write_u32(self.left);
        dst.write_u32(self.top);
        dst.write_u32(self.width);
        dst.write_u32(self.height);
        dst.write_u32(self.physical_width);
        dst.write_u32(self.physical_height);
        dst.write_u32(cast_length!("Orientation", self.orientation)?);
        dst.write_u32(self.desktop_scale_factor);
        dst.write_u32(self.device_scale_factor);
        Ok(())
    }
    fn size() -> usize {
        40
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct MonitorFlags: u32 {
        const PRIMARY = 1;
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Orientation {
    Landscape = 0,
    Portrait = 90,
    LandscapeFlipped = 180,
    PortraitFlipped = 270,
}

impl TryFrom<Orientation> for u32 {
    type Error = core::convert::Infallible;

    fn try_from(value: Orientation) -> Result<Self, Self::Error> {
        Ok(match value {
            Orientation::Landscape => 0,
            Orientation::Portrait => 90,
            Orientation::LandscapeFlipped => 180,
            Orientation::PortraitFlipped => 270,
        })
    }
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DisplayControlType {
    /// DISPLAYCONTROL_PDU_TYPE_CAPS
    Caps = 0x00000005,
    /// DISPLAYCONTROL_PDU_TYPE_MONITOR_LAYOUT
    MonitorLayout = 0x00000002,
}

impl TryFrom<DisplayControlType> for u32 {
    type Error = core::convert::Infallible;

    fn try_from(value: DisplayControlType) -> Result<Self, Self::Error> {
        Ok(match value {
            DisplayControlType::Caps => 0x05,
            DisplayControlType::MonitorLayout => 0x02,
        })
    }
}
