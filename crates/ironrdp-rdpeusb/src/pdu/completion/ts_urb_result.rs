//! Packets sent as responses to [`TsUrb`]s received from the server as part of
//! [`TransferInRequest`] and [`TransferOutRequest`] messages.
//!
//! The [`TsUrbResult`] packets are sent as part of [`UrbCompletion`] or [`UrbCompletionNoData`].

use alloc::vec::Vec;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, ensure_size,
    invalid_field_err, other_err, read_padding, write_padding,
};

use crate::pdu::utils::{ConfigHandle, FrameNumber, PipeHandle, UsbdIsoPacketDesc, UsbdStatus};
#[cfg(doc)]
use crate::pdu::{
    completion::{UrbCompletion, UrbCompletionNoData},
    header::SharedMsgHeader,
    usb_dev::{
        InternalIoControl, IoControl, RegisterRequestCallback, TransferInRequest, TransferOutRequest,
        ts_urb::{TsUrb, TsUrbGetCurrFrameNum, TsUrbIsochTransfer, TsUrbSelectConfig, TsUrbSelectInterface},
    },
};

/// [\[MS-RDPEUSB\] 2.2.10 TS_URB_RESULT][1] structure.
///
/// Sent in response to the [`TransferInRequest`] and [`TransferOutRequest`] messages, these
/// structures are sent via the [`UrbCompletion`] or [`UrbCompletionNoData`] messages.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/5a797c73-8ea0-46db-901c-cfb56f1a04a0
#[doc(alias = "TS_URB_RESULT")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbResult {
    pub header: TsUrbResultHeader,
    pub payload: TsUrbResultPayload,
}

impl Decode<'_> for TsUrbResult {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: size_of::<u16>(/* TS_URB_RESULT_HEADER::Size */));
        let urb_size: usize = src.read_u16().into();
        let header = TsUrbResultHeader::decode(src)?;
        const ACTUAL_HEADER_SIZE: usize = size_of::<u16>(/* Size */) + TsUrbResultHeader::FIXED_PART_SIZE;
        if urb_size < ACTUAL_HEADER_SIZE {
            return Err(invalid_field_err!("TS_URB_RESULT_HEADER::Size", "is smaller than 8"));
        }
        let payload = TsUrbResultPayload::decode(&mut ReadCursor::new(src.read_slice(urb_size - ACTUAL_HEADER_SIZE)))?;
        Ok(Self { header, payload })
    }
}

impl Encode for TsUrbResult {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u16(self.size().try_into().map_err(|e| other_err!(source: e))?);
        self.header.encode(dst)?;
        self.payload.encode(dst)
    }

    fn name(&self) -> &'static str {
        "TS_URB_RESULT"
    }

    fn size(&self) -> usize {
        size_of::<u16>(/* TS_URB_RESULT_HEADER::Size */) + self.header.size() + self.payload.size()
    }
}

/// [\[MS-RDPEUSB\] 2.2.10.1.1 TS_URB_RESULT_HEADER][1].
///
/// Common header for all [`TsUrbResult`] structures analogous to how [`SharedMsgHeader`] is for
/// all "top-level" *\[MS-RDPEUSB\]* messages.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/9161e272-be27-4184-86c9-4ab1103eec0e
#[doc(alias = "TS_URB_RESULT_HEADER")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbResultHeader {
    pub usbd_status: UsbdStatus,
}

impl TsUrbResultHeader {
    pub const FIXED_PART_SIZE: usize = size_of::<u16>(/* Padding */) + size_of::<u32>(/* UsbdStatus */);
}

impl Decode<'_> for TsUrbResultHeader {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        read_padding!(src, 2);
        let usbd_status = src.read_u32();

        Ok(Self { usbd_status })
    }
}

impl Encode for TsUrbResultHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        write_padding!(dst, 2);
        dst.write_u32(self.usbd_status);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_URB_RESULT_HEADER"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// Extra payload for (any of) the [`TsUrbResult`] structures in addition to the header.
///
/// While encoding, any of the non-[`Raw`][1] variants should be used. On decoding, always gives a
/// [`Raw`][1] variant. The raw bytes will have to be decoded to any of the non-raw variants
/// depending upon the `RequestId` field of the outer [`UrbCompletionNoData`] packet. For a
/// [`UrbCompletion`] packet, the payload can, and should, always be decoded to
/// [`TsUrbIsochTransferResult`]. In the case there's no extra payload, this will decode to a
/// `Raw(vec![])`.
///
/// [1]: TsUrbResultPayload::Raw
//
// The Raw variant exists cause successfully decoding to an actual result variant will require
// request id's for all four variants, passed by the state machine from outside mod pdu. Instead,
// we could return Raw variant by default while decoding (or merely "reading" in this case) and
// using request ID the raw bytes can be synthesized into actual variants.
#[non_exhaustive]
#[doc(alias = "TS_URB_RESULT")]
#[derive(Debug, PartialEq, Clone)]
pub enum TsUrbResultPayload {
    SelectConfig(TsUrbSelectConfigResult),
    SelectIface(TsUrbSelectInterfaceResult),
    FrameNum(TsUrbGetCurrFrameNumResult),
    Isoch(TsUrbIsochTransferResult),
    Raw(Vec<u8>),
}

impl Decode<'_> for TsUrbResultPayload {
    /// Reads all remaining bytes. Decode to the [`Raw`][TsUrbResultPayload::Raw] variant.
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(Self::Raw(src.remaining().into()))
    }
}

impl Encode for TsUrbResultPayload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        match self {
            Self::SelectConfig(select_config_res_payload) => select_config_res_payload.encode(dst),
            Self::SelectIface(select_iface_res_payload) => select_iface_res_payload.encode(dst),
            Self::FrameNum(frame_num_res_payload) => frame_num_res_payload.encode(dst),
            Self::Isoch(isoch_transfer_res_payload) => isoch_transfer_res_payload.encode(dst),
            Self::Raw(bytes) => {
                ensure_size!(in: dst, size: bytes.len());
                dst.write_slice(bytes);
                Ok(())
            }
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::SelectConfig(payload) => payload.name(),
            Self::SelectIface(payload) => payload.name(),
            Self::FrameNum(payload) => payload.name(),
            Self::Isoch(payload) => payload.name(),
            Self::Raw(_) => "TS_URB_RESULT (raw payload)",
        }
    }

    fn size(&self) -> usize {
        match self {
            Self::SelectConfig(payload) => payload.size(),
            Self::SelectIface(payload) => payload.size(),
            Self::FrameNum(payload) => payload.size(),
            Self::Isoch(payload) => payload.size(),
            Self::Raw(bytes) => bytes.len(),
        }
    }
}

/// Payload for the [\[MS-RDPEUSB\] 2.2.10.2 TS_URB_SELECT_CONFIGURATION_RESULT][1] packet.
///
/// Represents the result of [`TransferInRequest`] with [`TsUrbSelectConfig`]. This packet is sent
/// via the [`UrbCompletionNoData`] message.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/d79d7cd5-1294-4529-9849-c27436a399bc
#[doc(alias = "TS_URB_SELECT_CONFIGURATION_RESULT")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbSelectConfigResult {
    pub config_handle: ConfigHandle,
    pub interface: Vec<TsUsbdInterfaceInfoResult>,
}

impl TsUrbSelectConfigResult {
    pub const FIXED_PART_SIZE: usize = size_of::<u32>(/* ConfigurationHandle */) + size_of::<u32>(/* NumInterfaces */);

    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let config_handle = src.read_u32();
        let num_interfaces = src.read_u32();
        #[expect(clippy::map_with_unused_argument_over_ranges)]
        let interface = (0..num_interfaces)
            .map(|_| TsUsbdInterfaceInfoResult::decode(src))
            .collect::<Result<Vec<TsUsbdInterfaceInfoResult>, _>>()?;

        Ok(Self {
            config_handle,
            interface,
        })
    }
}

impl Encode for TsUrbSelectConfigResult {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u32(self.config_handle);
        dst.write_u32(self.interface.len().try_into().map_err(|_| {
            invalid_field_err!(
                "TS_URB_SELECT_CONFIGURATION_RESULT::Interface",
                "too many interfaces / alternate settings; count exceeded field NumInterfaces (4 bytes)"
            )
        })?);
        self.interface
            .iter()
            .try_for_each(|interface_result| interface_result.encode(dst))
    }

    fn name(&self) -> &'static str {
        "TS_URB_SELECT_CONFIGURATION_RESULT"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.interface.iter().map(Encode::size).sum::<usize>()
    }
}

/// Payload for the [\[MS-RDPEUSB\] 2.2.10.3 TS_URB_SELECT_INTERFACE_RESULT][1] packet.
///
/// Represents the result of [`TransferInRequest`] with [`TsUrbSelectInterface`]. This packet is
/// sent via the [`UrbCompletionNoData`] message.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/1831dd13-c367-4eff-a256-79c1acfeac17
#[doc(alias = "TS_URB_SELECT_INTERFACE_RESULT")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbSelectInterfaceResult {
    pub interface: TsUsbdInterfaceInfoResult,
}

impl TsUrbSelectInterfaceResult {
    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        TsUsbdInterfaceInfoResult::decode(src).map(|interface| Self { interface })
    }
}

impl Encode for TsUrbSelectInterfaceResult {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        self.interface.encode(dst)
    }

    fn name(&self) -> &'static str {
        "TS_URB_SELECT_INTERFACE_RESULT"
    }

    fn size(&self) -> usize {
        self.interface.size()
    }
}

/// Payload for the [\[MS-RDPEUSB\] 2.2.10.4 TS_URB_GET_CURRENT_FRAME_NUMBER_RESULT][1] packet.
///
/// Represents the result of [`TransferInRequest`] with [`TsUrbGetCurrFrameNum`]. This packet is
/// sent via the [`UrbCompletionNoData`] message.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/5cd61d0c-3adb-4009-afbc-1550bc54ac2b
#[doc(alias = "TS_URB_GET_CURRENT_FRAME_NUMBER_RESULT")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbGetCurrFrameNumResult {
    pub frame_number: FrameNumber,
}

impl TsUrbGetCurrFrameNumResult {
    pub const FIXED_PART_SIZE: usize = size_of::<u32>(/* FrameNumber */);

    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let frame_number = src.read_u32();
        Ok(Self { frame_number })
    }
}

impl Encode for TsUrbGetCurrFrameNumResult {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);
        dst.write_u32(self.frame_number);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_URB_GET_CURRENT_FRAME_NUMBER_RESULT"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// Payload for the [\[MS-RDPEUSB\] 2.2.10.5 TS_URB_ISOCH_TRANSFER_RESULT][1] packet.
///
/// Represents the result of [`TransferInRequest`] or [`TransferOutRequest`] with
/// [`TsUrbIsochTransfer`]. This packet is sent via the [`UrbCompletion`] message if there is data
/// to be sent back, or [`UrbCompletionNoData`] message if there is no data to send back.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/6f072673-52b8-4750-ac91-9d2313f13b17
#[doc(alias = "TS_URB_ISOCH_TRANSFER_RESULT")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbIsochTransferResult {
    pub start_frame: FrameNumber,
    // Only used for URB_COMPLETION_NO_DATA in response to TRANSFER_OUT_REQUEST
    pub error_count: u32,
    pub iso_packet: Vec<UsbdIsoPacketDesc>,
}

impl TsUrbIsochTransferResult {
    pub const FIXED_PART_SIZE: usize =
        size_of::<u32>(/* StartFrame */) + size_of::<u32>(/* NumberOfPackets */) + size_of::<u32>(/* ErrorCount */);

    pub fn count_error(iso_packets: &[UsbdIsoPacketDesc]) -> usize {
        iso_packets.iter().filter(|iso| iso.status < 0).count()
    }

    pub fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let start_frame = src.read_u32();
        let number_of_packets = src.read_u32();
        let error_count = src.read_u32();
        #[expect(clippy::map_with_unused_argument_over_ranges)]
        let iso_packet = (0..number_of_packets)
            .map(|_| UsbdIsoPacketDesc::decode(src))
            .collect::<Result<Vec<UsbdIsoPacketDesc>, _>>()?;

        Ok(Self {
            start_frame,
            error_count,
            iso_packet,
        })
    }
}

impl Encode for TsUrbIsochTransferResult {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u32(self.start_frame);
        dst.write_u32(self.iso_packet.len().try_into().map_err(|_| {
            invalid_field_err!(
                "TS_URB_ISOCH_TRANSFER_RESULT::IsoPacket",
                "too many packets: count exceeded field NumberOfPackets (4 bytes)"
            )
        })?);
        dst.write_u32(Self::count_error(&self.iso_packet).try_into().map_err(|_| {
            invalid_field_err!(
                "TS_URB_ISOCH_TRANSFER_RESULT::IsoPacket",
                "too many failed transfers: count exceeded field ErrorCount (4 bytes)"
            )
        })?);
        self.iso_packet.iter().try_for_each(|iso| iso.encode(dst))
    }

    fn name(&self) -> &'static str {
        "TS_URB_ISOCH_TRANSFER_RESULT"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.iso_packet.len() * UsbdIsoPacketDesc::FIXED_PART_SIZE
    }
}

/// The [\[MS-RDPEUSB\] 2.2.10.1.2 TS_USBD_INTERFACE_INFORMATION_RESULT][1] structure.
///
/// Based on the [`USBD_INTERFACE_INFORMATION`][2] structure.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/b27abecf-2827-453a-a885-94dc3198e6d5
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_usbd_interface_information
#[doc(alias = "TS_USBD_INTERFACE_INFORMATION_RESULT")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUsbdInterfaceInfoResult {
    pub interface_number: u8,
    pub alternate_setting: u8,
    pub class: u8,
    pub sub_class: u8,
    pub protocol: u8,
    pub interface_handle: u32,
    pub pipes: Vec<TsUsbdPipeInfoResult>,
}

impl TsUsbdInterfaceInfoResult {
    pub const FIXED_PART_SIZE: usize = size_of::<u16>(/* Length */)
        + size_of::<u8>(/* InterfaceNumber */)
        + size_of::<u8>(/* AlternateSetting */)
        + size_of::<u8>(/* Class */)
        + size_of::<u8>(/* SubClass */)
        + size_of::<u8>(/* Protocol */)
        + size_of::<u8>(/* Padding */)
        + size_of::<u32>(/* InterfaceHandle */)
        + size_of::<u32>(/* NumberOfPipes */);

    /// # Panics
    ///
    /// If *(number-of-pipes * 20) + 16* is greater than `u16::MAX`.
    #[inline]
    pub fn length(&self) -> u16 {
        (Self::FIXED_PART_SIZE + self.pipes.len() * TsUsbdPipeInfoResult::FIXED_PART_SIZE)
            .try_into()
            .expect("Max: 16 + 30 * 20 = 616")
    }
}

impl Decode<'_> for TsUsbdInterfaceInfoResult {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let length @ 16.. = src.read_u16() else {
            return Err(invalid_field_err!(
                "TS_USBD_INTERFACE_INFORMATION_RESULT::Length",
                "is less than min reqd value of 16"
            ));
        };
        let mut src = ReadCursor::new(src.read_slice(usize::from(length) - 2));
        let interface_number = src.read_u8();
        let alternate_setting = src.read_u8();
        let class = src.read_u8();
        let sub_class = src.read_u8();
        let protocol = src.read_u8();
        read_padding(&mut src, 1);
        let interface_handle = src.read_u32();
        #[expect(clippy::map_with_unused_argument_over_ranges)]
        let pipes = (0..src.read_u32())
            .map(|_| TsUsbdPipeInfoResult::decode(&mut src))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            interface_number,
            alternate_setting,
            class,
            sub_class,
            protocol,
            interface_handle,
            pipes,
        })
    }
}

impl Encode for TsUsbdInterfaceInfoResult {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(self.length());
        dst.write_u8(self.interface_number);
        dst.write_u8(self.alternate_setting);
        dst.write_u8(self.class);
        dst.write_u8(self.sub_class);
        dst.write_u8(self.protocol);
        write_padding!(dst, 1);
        dst.write_u32(self.interface_handle);
        dst.write_u32(self.pipes.len().try_into().map_err(|e| other_err!(source: e))?);
        self.pipes.iter().try_for_each(|pipe| pipe.encode(dst))
    }

    fn name(&self) -> &'static str {
        "TS_USBD_INTERFACE_INFORMATION_RESULT"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.pipes.len() * TsUsbdPipeInfoResult::FIXED_PART_SIZE
    }
}

/// The [\[MS-RDPEUSB\] 2.2.10.1.3 TS_USBD_PIPE_INFORMATION_RESULT][2] structure.
///
/// Based on the [`USBD_PIPE_INFORMATION`][1] structure.
///
/// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_usbd_pipe_information
/// [2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/b27abecf-2827-453a-a885-94dc3198e6d5
#[doc(alias = "TS_USBD_PIPE_INFORMATION_RESULT")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUsbdPipeInfoResult {
    pub max_packet_size: u16,
    pub endpoint_address: u8,
    pub interval: u8,
    pub pipe_type: UsbdPipeType,
    pub pipe_handle: PipeHandle,
    pub max_transfer_size: u32,
    pub pipe_flags: u32,
}

impl TsUsbdPipeInfoResult {
    pub const FIXED_PART_SIZE: usize = size_of::<u16>(/* MaximumPacketSize */)
        + size_of::<u8>(/* EndpointAddress */)
        + size_of::<u8>(/* Interval */)
        + size_of::<u32>(/* PipeType */)
        + size_of::<PipeHandle>(/* PipeHandle */)
        + size_of::<u32>(/* MaximumTransferSize */)
        + size_of::<u32>(/* PipeFlags */);
}

impl Decode<'_> for TsUsbdPipeInfoResult {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let max_packet_size = src.read_u16();
        let endpoint_address = src.read_u8();
        let interval = src.read_u8();
        let pipe_type = match src.read_u32() {
            0 => UsbdPipeType::Control,
            1 => UsbdPipeType::Isochronous,
            2 => UsbdPipeType::Bulk,
            3 => UsbdPipeType::Interrupt,
            _ => {
                return Err(invalid_field_err!(
                    "TS_USBD_PIPE_INFORMATION_RESULT::PipeType",
                    "is not one of: \
                        0x0 (UsbdPipeTypeControl),\
                        0x1 (UsbdPipeTypeIsochronous),\
                        0x2 (UsbdPipeTypeBulk),\
                        0x3 (UsbdPipeTypeInterrupt)"
                ));
            }
        };
        let pipe_handle = src.read_u32();
        let max_transfer_size = src.read_u32();
        let pipe_flags = src.read_u32();

        Ok(Self {
            max_packet_size,
            endpoint_address,
            interval,
            pipe_type,
            pipe_handle,
            max_transfer_size,
            pipe_flags,
        })
    }
}

impl Encode for TsUsbdPipeInfoResult {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_fixed_part_size!(in: dst);

        dst.write_u16(self.max_packet_size);
        dst.write_u8(self.endpoint_address);
        dst.write_u8(self.interval);
        #[expect(clippy::as_conversions)]
        dst.write_u32(self.pipe_type as u32);
        dst.write_u32(self.pipe_handle);
        dst.write_u32(self.max_transfer_size);
        dst.write_u32(self.pipe_flags);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_USBD_PIPE_INFORMATION_RESULT"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// The [`USBD_PIPE_TYPE`][1] enumeration indicating the type of pipe.
///
/// [1]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ne-usb-_usbd_pipe_type
#[repr(u32)]
#[doc(alias = "USBD_PIPE_TYPE")]
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum UsbdPipeType {
    /// Indicates that the pipe is a control pipe.
    Control = 0x0,
    /// Indicates that the pipe is an isochronous transfer pipe.
    Isochronous = 0x1,
    /// Indicates that the pipe is a bulk transfer pipe.
    Bulk = 0x2,
    /// Indicates that the pipe is an interrupt pipe.
    Interrupt = 0x3,
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    extern crate std;

    #[test]
    fn header() {
        let en = TsUrbResultHeader { usbd_status: 234 };
        let mut buf = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUrbResultHeader::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en, de);
    }

    #[test]
    fn ts_usbd_pipe_info_result() {
        let mut buf = vec![0; TsUsbdPipeInfoResult::FIXED_PART_SIZE];
        let mut en = TsUsbdPipeInfoResult {
            max_packet_size: 1,
            endpoint_address: 2,
            interval: 3,
            pipe_type: UsbdPipeType::Control,
            pipe_handle: 4,
            max_transfer_size: 5,
            pipe_flags: 6,
        };
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUsbdPipeInfoResult::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en, de);

        en.pipe_type = UsbdPipeType::Isochronous;
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUsbdPipeInfoResult::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en, de);

        en.pipe_type = UsbdPipeType::Bulk;
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUsbdPipeInfoResult::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en, de);

        en.pipe_type = UsbdPipeType::Interrupt;
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUsbdPipeInfoResult::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en, de);
    }

    #[test]
    fn ts_usbd_interface_info_result() {
        let en = TsUsbdInterfaceInfoResult {
            interface_number: 0,
            alternate_setting: 1,
            class: 2,
            sub_class: 3,
            protocol: 4,
            interface_handle: 5,
            pipes: vec![
                TsUsbdPipeInfoResult {
                    max_packet_size: 6,
                    endpoint_address: 7,
                    interval: 8,
                    pipe_type: UsbdPipeType::Control,
                    pipe_handle: 9,
                    max_transfer_size: 10,
                    pipe_flags: 11,
                },
                TsUsbdPipeInfoResult {
                    max_packet_size: 12,
                    endpoint_address: 13,
                    interval: 14,
                    pipe_type: UsbdPipeType::Isochronous,
                    pipe_handle: 15,
                    max_transfer_size: 16,
                    pipe_flags: 17,
                },
                TsUsbdPipeInfoResult {
                    max_packet_size: 24,
                    endpoint_address: 25,
                    interval: 26,
                    pipe_type: UsbdPipeType::Bulk,
                    pipe_handle: 27,
                    max_transfer_size: 28,
                    pipe_flags: 29,
                },
                TsUsbdPipeInfoResult {
                    max_packet_size: 30,
                    endpoint_address: 31,
                    interval: 32,
                    pipe_type: UsbdPipeType::Interrupt,
                    pipe_handle: 33,
                    max_transfer_size: 34,
                    pipe_flags: 35,
                },
            ],
        };
        let mut buf = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUsbdInterfaceInfoResult::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en, de);
    }

    #[test]
    fn ts_urb_select_config_result_payload() {
        let en = TsUrbSelectConfigResult {
            config_handle: 123,
            interface: vec![
                TsUsbdInterfaceInfoResult {
                    interface_number: 0,
                    alternate_setting: 1,
                    class: 2,
                    sub_class: 3,
                    protocol: 4,
                    interface_handle: 5,
                    pipes: vec![
                        TsUsbdPipeInfoResult {
                            max_packet_size: 6,
                            endpoint_address: 7,
                            interval: 8,
                            pipe_type: UsbdPipeType::Control,
                            pipe_handle: 9,
                            max_transfer_size: 10,
                            pipe_flags: 11,
                        },
                        TsUsbdPipeInfoResult {
                            max_packet_size: 12,
                            endpoint_address: 13,
                            interval: 14,
                            pipe_type: UsbdPipeType::Isochronous,
                            pipe_handle: 15,
                            max_transfer_size: 16,
                            pipe_flags: 17,
                        },
                    ],
                },
                TsUsbdInterfaceInfoResult {
                    interface_number: 18,
                    alternate_setting: 19,
                    class: 20,
                    sub_class: 21,
                    protocol: 22,
                    interface_handle: 23,
                    pipes: vec![
                        TsUsbdPipeInfoResult {
                            max_packet_size: 24,
                            endpoint_address: 25,
                            interval: 26,
                            pipe_type: UsbdPipeType::Bulk,
                            pipe_handle: 27,
                            max_transfer_size: 28,
                            pipe_flags: 29,
                        },
                        TsUsbdPipeInfoResult {
                            max_packet_size: 30,
                            endpoint_address: 31,
                            interval: 32,
                            pipe_type: UsbdPipeType::Interrupt,
                            pipe_handle: 33,
                            max_transfer_size: 34,
                            pipe_flags: 35,
                        },
                    ],
                },
            ],
        };

        let mut buf = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUrbSelectConfigResult::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en, de);

        let en = TsUrbResultPayload::SelectConfig(en);
        let mut buf2 = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf2)).unwrap();
        let de = TsUrbResultPayload::decode(&mut ReadCursor::new(&buf2[..en.size()])).unwrap();
        assert_eq!(de, TsUrbResultPayload::Raw(buf2));
    }

    #[test]
    fn ts_urb_select_interface_result_payload() {
        let en = TsUrbSelectInterfaceResult {
            interface: TsUsbdInterfaceInfoResult {
                interface_number: 0,
                alternate_setting: 1,
                class: 2,
                sub_class: 3,
                protocol: 4,
                interface_handle: 5,
                pipes: vec![
                    TsUsbdPipeInfoResult {
                        max_packet_size: 6,
                        endpoint_address: 7,
                        interval: 8,
                        pipe_type: UsbdPipeType::Control,
                        pipe_handle: 9,
                        max_transfer_size: 10,
                        pipe_flags: 11,
                    },
                    TsUsbdPipeInfoResult {
                        max_packet_size: 12,
                        endpoint_address: 13,
                        interval: 14,
                        pipe_type: UsbdPipeType::Isochronous,
                        pipe_handle: 15,
                        max_transfer_size: 16,
                        pipe_flags: 17,
                    },
                    TsUsbdPipeInfoResult {
                        max_packet_size: 24,
                        endpoint_address: 25,
                        interval: 26,
                        pipe_type: UsbdPipeType::Bulk,
                        pipe_handle: 27,
                        max_transfer_size: 28,
                        pipe_flags: 29,
                    },
                    TsUsbdPipeInfoResult {
                        max_packet_size: 30,
                        endpoint_address: 31,
                        interval: 32,
                        pipe_type: UsbdPipeType::Interrupt,
                        pipe_handle: 33,
                        max_transfer_size: 34,
                        pipe_flags: 35,
                    },
                ],
            },
        };

        let mut buf = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUrbSelectInterfaceResult::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en, de);

        let en = TsUrbResultPayload::SelectIface(en);
        let mut buf2 = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf2)).unwrap();
        let de = TsUrbResultPayload::decode(&mut ReadCursor::new(&buf2[..en.size()])).unwrap();
        assert_eq!(de, TsUrbResultPayload::Raw(buf2));
    }

    #[test]
    fn ts_urb_get_curr_frame_num_result_payload() {
        let en = TsUrbGetCurrFrameNumResult { frame_number: 133 };
        let mut buf = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUrbGetCurrFrameNumResult::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en, de);

        let en = TsUrbResultPayload::FrameNum(en);
        let mut buf2 = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf2)).unwrap();
        let de = TsUrbResultPayload::decode(&mut ReadCursor::new(&buf2[..en.size()])).unwrap();
        assert_eq!(de, TsUrbResultPayload::Raw(buf2));
    }

    #[test]
    fn ts_urb_isoch_transfer_result_payload() {
        let en = TsUrbIsochTransferResult {
            start_frame: 123,
            error_count: 1,
            iso_packet: vec![
                UsbdIsoPacketDesc {
                    offset: 0,
                    length: 1024,
                    status: 0,
                },
                UsbdIsoPacketDesc {
                    offset: 1024,
                    length: 1024,
                    status: -1,
                },
                UsbdIsoPacketDesc {
                    offset: 2048,
                    length: 1024,
                    status: 0,
                },
            ],
        };
        let mut buf = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUrbIsochTransferResult::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en, de);

        let en = TsUrbResultPayload::Isoch(en);
        let mut buf2 = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf2)).unwrap();
        let de = TsUrbResultPayload::decode(&mut ReadCursor::new(&buf2[..en.size()])).unwrap();
        assert_eq!(de, TsUrbResultPayload::Raw(buf2));
    }

    #[test]
    fn ts_urb_result() {
        let mut en = TsUrbResult {
            header: TsUrbResultHeader { usbd_status: 12342 },
            payload: TsUrbResultPayload::Raw(vec![]),
        };

        en.payload = TsUrbResultPayload::SelectConfig(TsUrbSelectConfigResult {
            config_handle: 8976,
            interface: vec![
                TsUsbdInterfaceInfoResult {
                    interface_number: 0,
                    alternate_setting: 1,
                    class: 2,
                    sub_class: 3,
                    protocol: 4,
                    interface_handle: 5,
                    pipes: vec![
                        TsUsbdPipeInfoResult {
                            max_packet_size: 6,
                            endpoint_address: 7,
                            interval: 8,
                            pipe_type: UsbdPipeType::Control,
                            pipe_handle: 9,
                            max_transfer_size: 10,
                            pipe_flags: 11,
                        },
                        TsUsbdPipeInfoResult {
                            max_packet_size: 12,
                            endpoint_address: 13,
                            interval: 14,
                            pipe_type: UsbdPipeType::Isochronous,
                            pipe_handle: 15,
                            max_transfer_size: 16,
                            pipe_flags: 17,
                        },
                    ],
                },
                TsUsbdInterfaceInfoResult {
                    interface_number: 18,
                    alternate_setting: 19,
                    class: 20,
                    sub_class: 21,
                    protocol: 22,
                    interface_handle: 23,
                    pipes: vec![
                        TsUsbdPipeInfoResult {
                            max_packet_size: 24,
                            endpoint_address: 25,
                            interval: 26,
                            pipe_type: UsbdPipeType::Bulk,
                            pipe_handle: 27,
                            max_transfer_size: 28,
                            pipe_flags: 29,
                        },
                        TsUsbdPipeInfoResult {
                            max_packet_size: 30,
                            endpoint_address: 31,
                            interval: 32,
                            pipe_type: UsbdPipeType::Interrupt,
                            pipe_handle: 33,
                            max_transfer_size: 34,
                            pipe_flags: 35,
                        },
                    ],
                },
            ],
        });

        let mut buf = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUrbResult::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en.size(), de.size());
        assert_eq!(en.payload.size(), de.payload.size());
        assert_eq!(en.header, de.header);
        let TsUrbResultPayload::Raw(payload) = de.payload else {
            unreachable!()
        };
        let payload = TsUrbSelectConfigResult::decode(&mut ReadCursor::new(&payload)).unwrap();
        assert_eq!(en.payload, TsUrbResultPayload::SelectConfig(payload));

        let mut buf2 = vec![0; en.payload.size()];
        en.payload.encode(&mut WriteCursor::new(&mut buf2)).unwrap();
        let de = TsUrbResultPayload::decode(&mut ReadCursor::new(&buf2[..en.payload.size()])).unwrap();
        assert_eq!(de, TsUrbResultPayload::Raw(buf2));

        en.payload = TsUrbResultPayload::SelectIface(TsUrbSelectInterfaceResult {
            interface: TsUsbdInterfaceInfoResult {
                interface_number: 0,
                alternate_setting: 1,
                class: 2,
                sub_class: 3,
                protocol: 4,
                interface_handle: 5,
                pipes: vec![
                    TsUsbdPipeInfoResult {
                        max_packet_size: 6,
                        endpoint_address: 7,
                        interval: 8,
                        pipe_type: UsbdPipeType::Control,
                        pipe_handle: 9,
                        max_transfer_size: 10,
                        pipe_flags: 11,
                    },
                    TsUsbdPipeInfoResult {
                        max_packet_size: 12,
                        endpoint_address: 13,
                        interval: 14,
                        pipe_type: UsbdPipeType::Isochronous,
                        pipe_handle: 15,
                        max_transfer_size: 16,
                        pipe_flags: 17,
                    },
                    TsUsbdPipeInfoResult {
                        max_packet_size: 24,
                        endpoint_address: 25,
                        interval: 26,
                        pipe_type: UsbdPipeType::Bulk,
                        pipe_handle: 27,
                        max_transfer_size: 28,
                        pipe_flags: 29,
                    },
                    TsUsbdPipeInfoResult {
                        max_packet_size: 30,
                        endpoint_address: 31,
                        interval: 32,
                        pipe_type: UsbdPipeType::Interrupt,
                        pipe_handle: 33,
                        max_transfer_size: 34,
                        pipe_flags: 35,
                    },
                ],
            },
        });

        let mut buf = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUrbResult::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en.size(), de.size());
        assert_eq!(en.payload.size(), de.payload.size());
        assert_eq!(en.header, de.header);
        let TsUrbResultPayload::Raw(payload) = de.payload else {
            unreachable!()
        };
        let payload = TsUrbSelectInterfaceResult::decode(&mut ReadCursor::new(&payload)).unwrap();
        assert_eq!(en.payload, TsUrbResultPayload::SelectIface(payload));

        let mut buf2 = vec![0; en.payload.size()];
        en.payload.encode(&mut WriteCursor::new(&mut buf2)).unwrap();
        let de = TsUrbResultPayload::decode(&mut ReadCursor::new(&buf2[..en.payload.size()])).unwrap();
        assert_eq!(de, TsUrbResultPayload::Raw(buf2));

        en.payload = TsUrbResultPayload::FrameNum(TsUrbGetCurrFrameNumResult { frame_number: 133 });

        let mut buf = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUrbResult::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en.size(), de.size());
        assert_eq!(en.payload.size(), de.payload.size());
        assert_eq!(en.header, de.header);
        let TsUrbResultPayload::Raw(payload) = de.payload else {
            unreachable!()
        };
        let payload = TsUrbGetCurrFrameNumResult::decode(&mut ReadCursor::new(&payload)).unwrap();
        assert_eq!(en.payload, TsUrbResultPayload::FrameNum(payload));

        let mut buf2 = vec![0; en.payload.size()];
        en.payload.encode(&mut WriteCursor::new(&mut buf2)).unwrap();
        let de = TsUrbResultPayload::decode(&mut ReadCursor::new(&buf2[..en.payload.size()])).unwrap();
        assert_eq!(de, TsUrbResultPayload::Raw(buf2));

        en.payload = TsUrbResultPayload::Isoch(TsUrbIsochTransferResult {
            start_frame: 123,
            error_count: 1,
            iso_packet: vec![
                UsbdIsoPacketDesc {
                    offset: 0,
                    length: 1024,
                    status: 0,
                },
                UsbdIsoPacketDesc {
                    offset: 1024,
                    length: 1024,
                    status: -1,
                },
                UsbdIsoPacketDesc {
                    offset: 2048,
                    length: 1024,
                    status: 0,
                },
            ],
        });

        let mut buf = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf)).unwrap();
        let de = TsUrbResult::decode(&mut ReadCursor::new(&buf)).unwrap();
        assert_eq!(en.size(), de.size());
        assert_eq!(en.payload.size(), de.payload.size());
        assert_eq!(en.header, de.header);
        let TsUrbResultPayload::Raw(payload) = de.payload else {
            unreachable!()
        };
        let payload = TsUrbIsochTransferResult::decode(&mut ReadCursor::new(&payload)).unwrap();
        assert_eq!(en.payload, TsUrbResultPayload::Isoch(payload));

        let mut buf2 = vec![0; en.payload.size()];
        en.payload.encode(&mut WriteCursor::new(&mut buf2)).unwrap();
        let de = TsUrbResultPayload::decode(&mut ReadCursor::new(&buf2[..en.payload.size()])).unwrap();
        assert_eq!(de, TsUrbResultPayload::Raw(buf2));
    }
}
