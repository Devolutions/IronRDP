//! Common utilities needed for all the [\[MS-RDPEUSB\]][1] messages.
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/a1004d0e-99e9-4968-894b-0b924ef2f125

use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size};

#[cfg(doc)]
use crate::pdu::usb_dev::{InternalIoControl, IoControl, TransferInRequest, TransferOutRequest};

pub type ConfigHandle = u32;

pub type PipeHandle = u32;

pub type FrameNumber = u32;

pub type UsbdStatus = u32;

/// An integer value that indicates the result or status of an operation.
///
/// * [MS-ERREF § 2.1 HRESULT][1]
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-erref/0642cb2f-2075-4469-918c-4441e69c548a
pub type HResult = u32;

/// Represents the ID of a request previously sent via [`IoControl`], [`InternalIoControl`],
/// [`TransferInRequest`], or [`TransferOutRequest`] message. Think of this like an "umbrella" type
/// for [`RequestIdIoctl`] and [`RequestIdTsUrb`].
pub type RequestId = u32;

/// Represents a request ID that uniquely identifies an [`IoControl`] or [`InternalIoControl`]
/// message.
pub type RequestIdIoctl = u32;

/// Is set to request data from a device. To transfer data to a device, this flag **MUST** be clear.
pub(crate) const USBD_TRANSFER_DIRECTION_IN: u32 = 0x1;

#[cfg(test)]
pub(crate) const USBD_TRANSFER_DIRECTION_OUT: u32 = 0x0;

#[cfg(test)]
pub(crate) const USBD_DEFAULT_PIPE_TRANSFER: u32 = 0x8;

#[cfg(test)]
pub(crate) const USBD_START_ISO_TRANSFER_ASAP: u32 = 0x4;

/// The maximum number of endpoints EP 1-15 (IN + OUT) excluding EP 0, in a USB device.
/// (see USB2.0 Spec 9.6.6 Endpoint).
pub const MAX_NON_DEFAULT_EP_COUNT: usize = 30;

/// Represents a request ID that uniquely identifies a [`TransferInRequest`] or
/// [`TransferOutRequest`] message. 31 bits.
#[repr(transparent)]
#[derive(Debug, PartialEq, Clone, Copy)]
pub struct RequestIdTransferInOut(u32);

impl TryFrom<u32> for RequestIdTransferInOut {
    type Error = &'static str;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value <= 0x7F_FF_FF_FF {
            Ok(RequestIdTransferInOut(value))
        } else {
            Err("value greater than 31 bits")
        }
    }
}

impl From<RequestIdTransferInOut> for u32 {
    fn from(value: RequestIdTransferInOut) -> Self {
        value.0
    }
}

/// Describes an isochronous transfer packet. See [WDK: `USBD_ISO_PACKET_DESCRIPTOR`][1].
///
/// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_usbd_iso_packet_descriptor
#[doc(alias = "USBD_ISO_PACKET_DESCRIPTOR")]
#[derive(Debug, PartialEq, Clone)]
pub struct UsbdIsoPacketDesc {
    pub offset: u32,
    pub length: u32,
    pub status: i32,
}

impl UsbdIsoPacketDesc {
    pub const FIXED_PART_SIZE: usize =
        size_of::<u32>(/* Offset */) + size_of::<u32>(/* Length */) + size_of::<i32>(/* Status */);
}

impl Encode for UsbdIsoPacketDesc {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.offset);
        dst.write_u32(self.length);
        dst.write_i32(self.status);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "USBD_ISO_PACKET_DESCRIPTOR"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

impl Decode<'_> for UsbdIsoPacketDesc {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let offset = src.read_u32();
        let length = src.read_u32();
        let status = src.read_i32();
        Ok(Self { offset, length, status })
    }
}

#[cfg(test)]
macro_rules! round_trip {
    ($en:expr, $de:ty) => {{
        let mut buf = alloc::vec![0; $en.size()];
        $en.encode(&mut ironrdp_core::WriteCursor::new(&mut buf)).unwrap();
        let mut src = ironrdp_core::ReadCursor::new(&buf);
        $crate::pdu::header::SharedMsgHeader::decode(&mut src)
            .and_then(|header| <$de>::decode(&mut src, header))
            .unwrap()
    }};
}

#[cfg(test)]
pub(crate) use round_trip;
