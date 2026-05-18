//! Packets sent to the client as part of [`TransferInRequest`] and [`TransferOutRequest`] messages
//! when the server receives a URB request from its system.
//!
//! A [`TsUrb`] packet is sent as part of a [`TransferInRequest`] or [`TransferOutRequest`].

use alloc::vec::Vec;

use ironrdp_core::{
    Decode as _, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_fixed_part_size, ensure_size,
    invalid_field_err, other_err, read_padding, write_padding,
};

use crate::pdu::usb_dev::ts_urb::utils::{SetupPacket, TsUrbHeader, TsUsbdInterfaceInfo, UrbFunction, UsbConfigDesc};
#[cfg(doc)]
use crate::pdu::usb_dev::{TransferInRequest, TransferOutRequest};
use crate::pdu::utils::{ConfigHandle, FrameNumber, PipeHandle, USBD_TRANSFER_DIRECTION_IN, UsbdIsoPacketDesc};

pub mod utils;

macro_rules! ensure_transfer_flag {
    ($direction:expr, $transfer_flags:expr, $ts_urb_name:expr) => {
        let flag_in = ($transfer_flags & USBD_TRANSFER_DIRECTION_IN) == USBD_TRANSFER_DIRECTION_IN;
        let transfer_in = matches!($direction, TransferDirection::In);
        if transfer_in && !flag_in {
            return Err(invalid_field_err!(
                concat!("TRANSFER_IN_REQUEST::TsUrb: ", $ts_urb_name, "::TransferFlags"),
                "does not contain USBD_TRANSFER_DIRECTION_IN"
            ));
        } else if !transfer_in && flag_in {
            return Err(invalid_field_err!(
                concat!("TRANSFER_OUT_REQUEST::TsUrb: ", $ts_urb_name, "::TransferFlags"),
                "contains USBD_TRANSFER_DIRECTION_IN"
            ));
        }
    };
}

macro_rules! ctl_desc_func_err {
    (OUT, $func:expr) => {{
        invalid_field_err!(
            "TRANSFER_OUT_REQUEST::TsUrb: TS_URB_CONTROL_DESCRIPTOR_REQUEST::TS_URB_HEADER::URB Function",
            concat!("is ", $func, " (only used with TRANSFER_IN_REQUEST)")
        )
    }};
    (IN, $func:expr) => {{
        invalid_field_err!(
            "TRANSFER_IN_REQUEST::TsUrb: TS_URB_CONTROL_DESCRIPTOR_REQUEST::TS_URB_HEADER::URB Function",
            concat!("is ", $func, " (only used with TRANSFER_OUT_REQUEST)")
        )
    }};
}

/// Enumeration of all the [\[MS-RDPEUSB\] 2.2.9 TS_URB Structures][1].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/eed35296-3ca1-4271-bd0a-597138131b47
#[non_exhaustive]
#[doc(alias = "TS_URB")]
#[derive(Debug, PartialEq, Clone)]
pub enum TsUrb {
    SelectConfig(TsUrbSelectConfig),
    SelectIface(TsUrbSelectInterface),
    PipeReq(TsUrbPipeRequest),
    GetCurFrameNum(TsUrbGetCurrFrameNum),
    CtlTransfer(TsUrbControlTransfer),
    BulkInterruptTransfer(TsUrbBulkOrInterruptTransfer),
    IsochTransfer(TsUrbIsochTransfer),
    CtlDescReq(TsUrbControlDescRequest),
    CtlFeatReq(TsUrbControlFeatRequest),
    CtlGetStatus(TsUrbControlGetStatusRequest),
    VendorClassReq(TsUrbControlVendorClassRequest),
    CtlGetConfig(TsUrbControlGetConfigRequest),
    CtlGetIface(TsUrbControlGetInterfaceRequest),
    OsFeatDescReq(TsUrbOsFeatDescRequest),
    CtlTransferEx(TsUrbControlTransferEx),
}

impl TsUrb {
    pub(crate) fn decode(src: &mut ReadCursor<'_>, direction: TransferDirection) -> DecodeResult<Self> {
        use UrbFunction::*;

        let ts_urb_size = src.read_u16(/* TS_URB_HEADER::Size */);

        let header = TsUrbHeader::decode(&mut ReadCursor::new(src.read_slice(TsUrbHeader::FIXED_PART_SIZE)))?;

        if matches!(direction, TransferDirection::In) {
            if header.no_ack {
                return Err(invalid_field_err!(
                    "TRANSFER_IN_REQUEST::TsUrb::TS_URB_HEADER::NoAck",
                    "is non-zero: NoAck MUST be set to zero for TRANSFER_IN_REQUEST"
                ));
            }
            match header.func {
                SetDescriptorToDevice => return Err(ctl_desc_func_err!(IN, "URB_FUNCTION_SET_DESCRIPTOR_TO_DEVICE")),
                SetDescriptorToEndpoint => {
                    return Err(ctl_desc_func_err!(IN, "URB_FUNCTION_SET_DESCRIPTOR_TO_ENDPOINT"));
                }
                SetDescriptorToInterface => {
                    return Err(ctl_desc_func_err!(IN, "URB_FUNCTION_SET_DESCRIPTOR_TO_INTERFACE"));
                }
                _ => (),
            }
        }

        // Weed out all URBs that are only used with TRANSFER_IN_REQUEST
        if matches!(direction, TransferDirection::Out) {
            macro_rules! invalid_tsurb_err {
                ($reflected_ts_urb:expr) => {{
                    invalid_field_err!(
                        "TRANSFER_OUT_REQUEST::TsUrb::TS_URB_HEADER::URB_Function",
                        concat!(
                            "URB Function reflects that TsUrb is ",
                            $reflected_ts_urb,
                            " (only used with TRANSFER_IN_REQUEST)"
                        )
                    )
                }};
            }

            match header.func {
                SelectConfiguration => return Err(invalid_tsurb_err!("TS_URB_SELECT_CONFIGURATION")),
                SelectInterface => return Err(invalid_tsurb_err!("TS_URB_SELECT_CONFIGURATION")),
                AbortPipe | SyncResetPipeAndClearStall | SyncResetPipe | SyncClearStall | CloseStaticStreams => {
                    return Err(invalid_tsurb_err!("TS_URB_PIPE_REQUEST"));
                }
                GetCurrentFrameNumber => return Err(invalid_tsurb_err!("TS_URB_GET_CURRENT_FRAME_NUMBER")),

                GetDescriptorFromDevice => {
                    return Err(ctl_desc_func_err!(OUT, "URB_FUNCTION_GET_DESCRIPTOR_FROM_DEVICE"));
                }
                GetDescriptorFromEndpoint => {
                    return Err(ctl_desc_func_err!(OUT, "URB_FUNCTION_GET_DESCRIPTOR_FROM_ENDPOINT"));
                }
                GetDescriptorFromInterface => {
                    return Err(ctl_desc_func_err!(OUT, "URB_FUNCTION_GET_DESCRIPTOR_FROM_INTERFACE"));
                }
                #[expect(unused_parens)]
                (SetFeatureToDevice | SetFeatureToInterface | SetFeatureToEndpoint | SetFeatureToOther)
                | (ClearFeatureToDevice | ClearFeatureToInterface | ClearFeatureToEndpoint | ClearFeatureToOther) => {
                    return Err(invalid_tsurb_err!("TS_URB_CONTROL_FEATURE_REQUEST"));
                }
                GetStatusFromDevice | GetStatusFromInterface | GetStatusFromEndpoint | GetStatusFromOther => {
                    return Err(invalid_tsurb_err!("TS_URB_CONTROL_GET_STATUS_REQUEST"));
                }
                GetConfiguration => return Err(invalid_tsurb_err!("TS_URB_CONTROL_GET_CONFIGURATION_REQUEST")),
                GetInterface => return Err(invalid_tsurb_err!("TS_URB_CONTROL_GET_INTERFACE_REQUEST")),
                GetMsFeatureDescriptor => return Err(invalid_tsurb_err!("TS_URB_OS_FEATURE_DESCRIPTOR_REQUEST")),
                _ => (),
            }
        }

        let mut src = ReadCursor::new(
            src.read_slice(usize::from(ts_urb_size) - size_of::<u16>(/* ts_urb_size */) - header.size()),
        );

        let ts_urb = match header.func {
            SelectConfiguration => Self::SelectConfig(TsUrbSelectConfig::decode(&mut src, header)?),

            SelectInterface => Self::SelectIface(TsUrbSelectInterface::decode(&mut src, header)?),

            AbortPipe | SyncResetPipeAndClearStall | SyncResetPipe | SyncClearStall | CloseStaticStreams => {
                Self::PipeReq(TsUrbPipeRequest::decode(&mut src, header)?)
            }
            GetCurrentFrameNumber => Self::GetCurFrameNum(TsUrbGetCurrFrameNum::decode(&mut src, header)?),
            ControlTransfer => {
                let urb = TsUrbControlTransfer::decode(&mut src, header)?;
                ensure_transfer_flag!(direction, urb.transfer_flags, "TS_URB_CONTROL_TRANSFER");
                Self::CtlTransfer(urb)
            }
            ControlTransferEx => {
                let urb = TsUrbControlTransferEx::decode(&mut src, header)?;
                ensure_transfer_flag!(direction, urb.transfer_flags, "TS_URB_CONTROL_TRANSFER_EX");
                Self::CtlTransferEx(urb)
            }
            BulkOrInterruptTransfer | BulkOrInterruptTransferUsingChainedMdl => {
                let urb = TsUrbBulkOrInterruptTransfer::decode(&mut src, header)?;
                ensure_transfer_flag!(direction, urb.transfer_flags, "TS_URB_BULK_OR_INTERRUPT_TRANSFER");
                Self::BulkInterruptTransfer(urb)
            }
            IsochTransfer | IsochTransferUsingChainedMdl => {
                let urb = TsUrbIsochTransfer::decode(&mut src, header)?;
                ensure_transfer_flag!(direction, urb.transfer_flags, "TS_URB_ISOCH_TRANSFER");
                Self::IsochTransfer(urb)
            }
            GetDescriptorFromDevice | GetDescriptorFromEndpoint | GetDescriptorFromInterface => {
                Self::CtlDescReq(TsUrbControlDescRequest::decode(&mut src, header)?)
            }
            SetDescriptorToDevice | SetDescriptorToEndpoint | SetDescriptorToInterface => {
                Self::CtlDescReq(TsUrbControlDescRequest::decode(&mut src, header)?)
            }
            #[expect(unused_parens)]
            (SetFeatureToDevice | SetFeatureToInterface | SetFeatureToEndpoint | SetFeatureToOther)
            | (ClearFeatureToDevice | ClearFeatureToInterface | ClearFeatureToEndpoint | ClearFeatureToOther) => {
                Self::CtlFeatReq(TsUrbControlFeatRequest::decode(&mut src, header)?)
            }
            GetStatusFromDevice | GetStatusFromInterface | GetStatusFromEndpoint | GetStatusFromOther => {
                Self::CtlGetStatus(TsUrbControlGetStatusRequest::decode(&mut src, header)?)
            }
            #[expect(unused_parens)]
            (VendorDevice | VendorInterface | VendorEndpoint | VendorOther)
            | (ClassDevice | ClassInterface | ClassEndpoint | ClassOther) => {
                let urb = TsUrbControlVendorClassRequest::decode(&mut src, header)?;
                ensure_transfer_flag!(direction, urb.transfer_flags, "TS_URB_CONTROL_VENDOR_OR_CLASS_REQUEST");
                Self::VendorClassReq(urb)
            }
            GetConfiguration => Self::CtlGetConfig(TsUrbControlGetConfigRequest::decode(&mut src, header)?),

            GetInterface => Self::CtlGetIface(TsUrbControlGetInterfaceRequest::decode(&mut src, header)?),

            GetMsFeatureDescriptor => Self::OsFeatDescReq(TsUrbOsFeatDescRequest::decode(&mut src, header)?),
        };

        Ok(ts_urb)
    }

    pub(crate) fn encode(&self, dst: &mut WriteCursor<'_>, direction: TransferDirection) -> EncodeResult<()> {
        use TsUrb::*;

        if matches!(direction, TransferDirection::In) {
            if let CtlDescReq(ctl_desc_req) = self {
                match ctl_desc_req.header.func {
                    UrbFunction::SetDescriptorToDevice => {
                        return Err(ctl_desc_func_err!(IN, "URB_FUNCTION_SET_DESCRIPTOR_TO_DEVICE"));
                    }
                    UrbFunction::SetDescriptorToEndpoint => {
                        return Err(ctl_desc_func_err!(IN, "URB_FUNCTION_SET_DESCRIPTOR_TO_ENDPOINT"));
                    }
                    UrbFunction::SetDescriptorToInterface => {
                        return Err(ctl_desc_func_err!(IN, "URB_FUNCTION_SET_DESCRIPTOR_TO_INTERFACE"));
                    }
                    _ => (),
                }
            }
        }

        // Weed out all URBs that are only used with TRANSFER_IN_REQUEST
        if matches!(direction, TransferDirection::Out) {
            macro_rules! invalid_tsurb_err {
                ($reflected_ts_urb:expr) => {{
                    invalid_field_err!(
                        "TRANSFER_OUT_REQUEST::TsUrb",
                        concat!("is ", $reflected_ts_urb, " (only used with TRANSFER_IN_REQUEST)")
                    )
                }};
            }

            match self {
                SelectConfig(_) => return Err(invalid_tsurb_err!("TS_URB_SELECT_CONFIGURATION")),
                SelectIface(_) => return Err(invalid_tsurb_err!("TS_URB_SELECT_CONFIGURATION")),
                PipeReq(_) => return Err(invalid_tsurb_err!("TS_URB_PIPE_REQUEST")),
                GetCurFrameNum(_) => return Err(invalid_tsurb_err!("TS_URB_GET_CURRENT_FRAME_NUMBER")),
                CtlDescReq(ctl_desc_req) => match ctl_desc_req.header.func {
                    UrbFunction::GetDescriptorFromDevice => {
                        return Err(ctl_desc_func_err!(OUT, "URB_FUNCTION_GET_DESCRIPTOR_FROM_DEVICE"));
                    }
                    UrbFunction::GetDescriptorFromEndpoint => {
                        return Err(ctl_desc_func_err!(OUT, "URB_FUNCTION_GET_DESCRIPTOR_FROM_ENDPOINT"));
                    }
                    UrbFunction::GetDescriptorFromInterface => {
                        return Err(ctl_desc_func_err!(OUT, "URB_FUNCTION_GET_DESCRIPTOR_FROM_INTERFACE"));
                    }
                    _ => (),
                },
                CtlFeatReq(_) => return Err(invalid_tsurb_err!("TS_URB_CONTROL_FEATURE_REQUEST")),
                CtlGetStatus(_) => return Err(invalid_tsurb_err!("TS_URB_CONTROL_GET_STATUS_REQUEST")),
                CtlGetConfig(_) => return Err(invalid_tsurb_err!("TS_URB_CONTROL_GET_CONFIGURATION_REQUEST")),
                CtlGetIface(_) => return Err(invalid_tsurb_err!("TS_URB_CONTROL_GET_INTERFACE_REQUEST")),
                OsFeatDescReq(_) => return Err(invalid_tsurb_err!("TS_URB_OS_FEATURE_DESCRIPTOR_REQUEST")),
                _ => (),
            }
        }

        macro_rules! ensure_no_ack {
            ($direction:expr, $no_ack:expr, $ts_urb_name:expr) => {{
                if matches!($direction, TransferDirection::In) && $no_ack {
                    return Err(invalid_field_err!(
                        concat!(
                            "TRANSFER_IN_REQUEST::TsUrb: ",
                            $ts_urb_name,
                            "::TS_URB_HEADER::NoAck"
                        ),
                        "is non-zero: NoAck MUST be set to zero for TRANSFER_IN_REQUEST"
                    ));
                }
            }};
        }

        match self {
            SelectConfig(urb) => urb.encode(dst),
            SelectIface(urb) => urb.encode(dst),
            PipeReq(urb) => urb.encode(dst),
            GetCurFrameNum(urb) => urb.encode(dst),
            CtlTransfer(urb) => {
                ensure_no_ack!(direction, urb.header.no_ack, "TS_URB_CONTROL_TRANSFER");
                ensure_transfer_flag!(direction, urb.transfer_flags, "TS_URB_CONTROL_TRANSFER");
                urb.encode(dst)
            }
            BulkInterruptTransfer(urb) => {
                ensure_no_ack!(direction, urb.header.no_ack, "TS_URB_BULK_OR_INTERRUPT_TRANSFER");
                ensure_transfer_flag!(direction, urb.transfer_flags, "TS_URB_BULK_OR_INTERRUPT_TRANSFER");
                urb.encode(dst)
            }
            IsochTransfer(urb) => {
                ensure_no_ack!(direction, urb.header.no_ack, "TS_URB_ISOCH_TRANSFER");
                ensure_transfer_flag!(direction, urb.transfer_flags, "TS_URB_ISOCH_TRANSFER");
                urb.encode(dst)
            }
            CtlDescReq(urb) => urb.encode(dst),
            CtlFeatReq(urb) => urb.encode(dst),
            CtlGetStatus(urb) => urb.encode(dst),
            VendorClassReq(urb) => {
                ensure_no_ack!(direction, urb.header.no_ack, "TS_URB_CONTROL_VENDOR_OR_CLASS_REQUEST");
                ensure_transfer_flag!(direction, urb.transfer_flags, "TS_URB_CONTROL_VENDOR_OR_CLASS_REQUEST");
                urb.encode(dst)
            }
            CtlGetConfig(urb) => urb.encode(dst),
            CtlGetIface(urb) => urb.encode(dst),
            OsFeatDescReq(urb) => urb.encode(dst),
            CtlTransferEx(urb) => {
                ensure_no_ack!(direction, urb.header.no_ack, "TS_URB_CONTROL_TRANSFER_EX");
                ensure_transfer_flag!(direction, urb.transfer_flags, "TS_URB_CONTROL_TRANSFER_EX");
                urb.encode(dst)
            }
        }
    }

    pub fn name(&self) -> &'static str {
        "TS_URB"
    }

    pub fn size(&self) -> usize {
        use TsUrb::*;

        match self {
            SelectConfig(urb) => urb.size(),
            SelectIface(urb) => urb.size(),
            PipeReq(urb) => urb.size(),
            GetCurFrameNum(urb) => urb.size(),
            CtlTransfer(urb) => urb.size(),
            BulkInterruptTransfer(urb) => urb.size(),
            IsochTransfer(urb) => urb.size(),
            CtlDescReq(urb) => urb.size(),
            CtlFeatReq(urb) => urb.size(),
            CtlGetStatus(urb) => urb.size(),
            VendorClassReq(urb) => urb.size(),
            CtlGetConfig(urb) => urb.size(),
            CtlGetIface(urb) => urb.size(),
            OsFeatDescReq(urb) => urb.size(),
            CtlTransferEx(urb) => urb.size(),
        }
    }
}

#[repr(u8)]
#[derive(PartialEq, Clone, Copy)]
pub(crate) enum TransferDirection {
    Out = 0x0,
    In = 0x1,
}

macro_rules! encode_ts_urb_size {
    ($dst:expr, $size:expr) => { {
            let size = u16::try_from($size).map_err(|e| other_err!(source: e))?;
            $dst.write_u16(size);
        }
    };
}

/// [\[MS-RDPEUSB\] 2.2.9.2 TS_URB_SELECT_CONFIGURATION][1] packet.
///
/// This packet represents [`URB_SELECT_CONFIGURATION`][2], and is sent using [`TransferInRequest`]
/// (with `output_buffer_size` set to `0`).
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/196e83a1-9bfd-45fb-97cc-a27c6a0c74ee
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_select_configuration
#[doc(alias = "TS_URB_SELECT_CONFIGURATION")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbSelectConfig {
    pub header: TsUrbHeader,
    pub usbd_ifaces: Vec<TsUsbdInterfaceInfo>,
    pub desc: Option<UsbConfigDesc>,
}

impl TsUrbSelectConfig {
    pub fn decode(src: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        let desc = src.read_u8(/* ConfigurationDescriptorIsValid */) != 0;

        ensure_size!(in: src, size: const { 3 * size_of::<u8>() });
        read_padding!(src, 3);

        let usbd_ifaces = {
            ensure_size!(in: src, size: const { size_of::<u32>() });
            let num_usbd_ifaces = src.read_u32(/* NumInterfaces */);

            let mut usbd_ifaces = Vec::new();
            for _ in 0..num_usbd_ifaces {
                usbd_ifaces.push(TsUsbdInterfaceInfo::decode(src)?)
            }
            usbd_ifaces
        };

        let desc = if desc { Some(UsbConfigDesc::decode(src)?) } else { None };

        Ok(Self {
            header,
            usbd_ifaces,
            desc,
        })
    }
}

impl Encode for TsUrbSelectConfig {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if !matches!(self.header.func, UrbFunction::SelectConfiguration) {
            return Err(invalid_field_err!(
                "TS_URB_SELECT_CONFIGURATION::TS_URB_HEADER::URB_Function",
                "is not URB_FUNCTION_SELECT_CONFIGURATION"
            ));
        }
        if self.header.no_ack {
            return Err(invalid_field_err!(
                "TS_URB_SELECT_CONFIGURATION::TS_URB_HEADER::URB_Function::NoAck",
                "is non-zero"
            ));
        }
        ensure_size!(in: dst, size: self.size());
        encode_ts_urb_size!(dst, self.size());
        self.header.encode(dst)?;

        // ConfigurationDescriptorIsValid
        dst.write_u8(self.desc.is_some().into());

        write_padding!(dst, 3);

        // NumInterfaces
        dst.write_u32(
            self.usbd_ifaces
                .len()
                .try_into()
                .expect("max 255 since bNumInterfaces is 1 byte"),
        );

        // TS_USBD_INTERFACE_INFORMATION
        for usbd_iface in &self.usbd_ifaces {
            usbd_iface.encode(dst)?;
        }

        if let Some(config_desc) = self.desc.as_ref() {
            config_desc.encode(dst)?; // USB_CONFIGURATION_DESCRIPTOR
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_URB_SELECT_CONFIGURATION"
    }

    fn size(&self) -> usize {
        size_of::<u16>(/* TS_URB_HEADER::Size */)
            + TsUrbHeader::FIXED_PART_SIZE
            + const {
                size_of::<u8>(/* ConfigurationDescriptorIsValid */)
                    + (3 * size_of::<u8>()/* Padding */)
                    + size_of::<u32>(/* NumInterfaces */)
            }
            + self.usbd_ifaces.iter().map(Encode::size).sum::<usize>()
            + self.desc.as_ref().map(Encode::size).unwrap_or_default()
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.3 TS_URB_SELECT_INTERFACE][1] packet.
///
/// This packet represents [`URB_SELECT_INTERFACE`][2], and is sent using [`TransferInRequest`]
/// (with `output_buffer_size` set to `0`).
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/36c33bed-8ce1-43b6-9ccd-030884a030c9
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_select_interface
#[doc(alias = "TS_URB_SELECT_INTERFACE")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbSelectInterface {
    pub header: TsUrbHeader,
    pub config_handle: ConfigHandle,
    pub usbd_iface: TsUsbdInterfaceInfo,
}

impl TsUrbSelectInterface {
    pub fn decode(src: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: const {
            size_of::<u32>(/* ConfigurationHandle */)
        });

        let config_handle = src.read_u32();

        let usbd_iface = TsUsbdInterfaceInfo::decode(src)?;

        Ok(Self {
            header,
            config_handle,
            usbd_iface,
        })
    }
}

impl Encode for TsUrbSelectInterface {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if !matches!(self.header.func, UrbFunction::SelectInterface) {
            return Err(invalid_field_err!(
                "TS_URB_SELECT_INTERFACE::TS_URB_HEADER::URB_Function",
                "is not URB_FUNCTION_SELECT_INTERFACE"
            ));
        }
        if self.header.no_ack {
            return Err(invalid_field_err!(
                "TS_URB_SELECT_INTERFACE::TS_URB_HEADER::URB_Function::NoAck",
                "is non-zero"
            ));
        }

        ensure_size!(in: dst, size: self.size());
        encode_ts_urb_size!(dst, self.size());
        self.header.encode(dst)?;
        dst.write_u32(self.config_handle);
        self.usbd_iface.encode(dst)
    }

    fn name(&self) -> &'static str {
        "TS_URB_SELECT_INTERFACE"
    }

    fn size(&self) -> usize {
        size_of::<u16>(/* TS_URB_HEADER::Size */)
            + TsUrbHeader::FIXED_PART_SIZE
            + const {
                size_of::<u32>(/* ConfigurationHandle */)
            }
            + self.usbd_iface.size()
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.4 TS_URB_PIPE_REQUEST][1] packet.
///
/// This packet represents [`URB_PIPE_REQUEST`][2], and is sent using [`TransferInRequest`] (with
/// `output_buffer_size` set to `0`).
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/dcba564e-de14-4d60-82ac-a0fe7a52b312
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_pipe_request
#[doc(alias = "TS_URB_PIPE_REQUEST")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbPipeRequest {
    pub header: TsUrbHeader,
    pub pipe_handle: PipeHandle,
}

impl TsUrbPipeRequest {
    pub const FIXED_PART_SIZE: usize =
        size_of::<u16>(/* TS_URB_HEADER::Size */) + TsUrbHeader::FIXED_PART_SIZE + size_of::<u32>(/* PipeHandle */);

    pub fn decode(src: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: const { size_of::<ConfigHandle>(/* PipeHandle */) });

        let pipe_handle = src.read_u32();

        Ok(Self { header, pipe_handle })
    }
}

impl Encode for TsUrbPipeRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        use UrbFunction::*;
        if !matches!(
            self.header.func,
            AbortPipe | SyncResetPipeAndClearStall | SyncResetPipe | SyncClearStall | CloseStaticStreams
        ) {
            return Err(invalid_field_err!(
                "TS_URB_PIPE_REQUEST::TS_URB_HEADER::URB_Function",
                "is not one of: \
                    URB_FUNCTION_ABORT_PIPE, \
                    URB_FUNCTION_SYNC_RESET_PIPE_AND_CLEAR_STALL, \
                    URB_FUNCTION_SYNC_RESET_PIPE, \
                    URB_FUNCTION_SYNC_CLEAR_STALL, \
                    URB_FUNCTION_CLOSE_STATIC_STREAMS"
            ));
        }
        if self.header.no_ack {
            return Err(invalid_field_err!(
                "TS_URB_PIPE_REQUEST::TS_URB_HEADER::URB_Function::NoAck",
                "is non-zero"
            ));
        }

        ensure_fixed_part_size!(in: dst);
        encode_ts_urb_size!(dst, self.size());
        self.header.encode(dst)?;
        dst.write_u32(self.pipe_handle);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_URB_PIPE_REQUEST"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.5 TS_URB_GET_CURRENT_FRAME_NUMBER][1] packet.
///
/// This packet represents [`URB_GET_CURRENT_FRAME_NUMBER`][2], and is sent using
/// [`TransferInRequest`] (with `output_buffer_size` set to `0`).
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/4985b1dc-5bd9-4988-97a6-063969dc26b4
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_get_current_frame_number
#[doc(alias = "TS_URB_GET_CURRENT_FRAME_NUMBER")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbGetCurrFrameNum {
    pub header: TsUrbHeader,
}

impl TsUrbGetCurrFrameNum {
    pub const FIXED_PART_SIZE: usize = size_of::<u16>(/* TS_URB_HEADER::Size */) + TsUrbHeader::FIXED_PART_SIZE;

    #[inline]
    pub fn decode(_: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        Ok(Self { header })
    }
}

impl Encode for TsUrbGetCurrFrameNum {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if !matches!(self.header.func, UrbFunction::GetCurrentFrameNumber) {
            return Err(invalid_field_err!(
                "TS_URB_GET_CURRENT_FRAME_NUMBER::TS_URB_HEADER::URB_Function",
                "is not URB_FUNCTION_GET_CURRENT_FRAME_NUMBER"
            ));
        }
        if self.header.no_ack {
            return Err(invalid_field_err!(
                "TS_URB_GET_CURRENT_FRAME_NUMBER::TS_URB_HEADER::URB_Function::NoAck",
                "is non-zero"
            ));
        }
        encode_ts_urb_size!(dst, self.size());
        self.header.encode(dst)
    }

    fn name(&self) -> &'static str {
        "TS_URB_GET_CURRENT_FRAME_NUMBER"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.6 TS_URB_CONTROL_TRANSFER][1] packet.
///
/// This packet represents [`URB_CONTROL_TRANSFER`][2]. Transfer flags MUST contain
/// `USBD_TRANSFER_DIRECTION_IN` to send using [`TransferInRequest`]; MUST not contain
/// `USBD_TRANSFER_DIRECTION_IN` to send using [`TransferOutRequest`].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/859aefe0-0209-4d31-af7c-7a1bf1c7e49a
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_control_transfer
#[doc(alias = "TS_URB_CONTROL_TRANSFER")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbControlTransfer {
    pub header: TsUrbHeader,
    pub pipe: PipeHandle,
    pub transfer_flags: u32,
    pub setup_packet: SetupPacket,
}

impl TsUrbControlTransfer {
    pub const PAYLOAD_SIZE: usize =
        size_of::<u32>(/* PipeHandle */) + size_of::<u32>(/* TransferFlags */) + SetupPacket::FIXED_PART_SIZE;

    pub const FIXED_PART_SIZE: usize =
        size_of::<u16>(/* TS_URB_HEADER::Size */) + TsUrbHeader::FIXED_PART_SIZE + Self::PAYLOAD_SIZE;

    pub fn decode(src: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let pipe_handle = src.read_u32();
        let transfer_flags = src.read_u32();
        let setup_packet = SetupPacket::decode(src)?;

        Ok(Self {
            header,
            pipe: pipe_handle,
            transfer_flags,
            setup_packet,
        })
    }
}

impl Encode for TsUrbControlTransfer {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if !matches!(self.header.func, UrbFunction::ControlTransfer) {
            return Err(invalid_field_err!(
                "TS_URB_CONTROL_TRANSFER::TS_URB_HEADER::URB_Function",
                "is not URB_FUNCTION_CONTROL_TRANSFER"
            ));
        }
        ensure_fixed_part_size!(in: dst);
        encode_ts_urb_size!(dst, self.size());
        self.header.encode(dst)?;
        dst.write_u32(self.pipe);
        dst.write_u32(self.transfer_flags);
        self.setup_packet.encode(dst)
    }

    fn name(&self) -> &'static str {
        "TS_URB_CONTROL_TRANSFER"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.7 TS_URB_BULK_OR_INTERRUPT_TRANSFER][1] packet.
///
/// This packet represents [`URB_BULK_OR_INTERRUPT_TRANSFER`][2]. Transfer flags MUST contain
/// `USBD_TRANSFER_DIRECTION_IN` to send using [`TransferInRequest`]; MUST not contain
/// `USBD_TRANSFER_DIRECTION_IN` to send using [`TransferOutRequest`].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/8c06982e-3a7b-4a27-a554-5f7f9d3f210a
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_bulk_or_interrupt_transfer
#[doc(alias = "TS_URB_BULK_OR_INTERRUPT_TRANSFER")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbBulkOrInterruptTransfer {
    pub header: TsUrbHeader,
    pub pipe_handle: PipeHandle,
    pub transfer_flags: u32,
}

impl TsUrbBulkOrInterruptTransfer {
    pub const PAYLOAD_SIZE: usize = size_of::<u32>(/* PipeHandle */) + size_of::<u32>(/* TransferFlags */);

    pub const FIXED_PART_SIZE: usize =
        size_of::<u16>(/* TS_URB_HEADER::Size */) + TsUrbHeader::FIXED_PART_SIZE + Self::PAYLOAD_SIZE;

    pub fn decode(src: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let pipe_handle = src.read_u32();
        let transfer_flags = src.read_u32();

        Ok(Self {
            header,
            pipe_handle,
            transfer_flags,
        })
    }
}

impl Encode for TsUrbBulkOrInterruptTransfer {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if !matches!(
            self.header.func,
            UrbFunction::BulkOrInterruptTransfer | UrbFunction::BulkOrInterruptTransferUsingChainedMdl
        ) {
            return Err(invalid_field_err!(
                "TS_URB_BULK_OR_INTERRUPT_TRANSFER::TS_URB_HEADER::URB_Function",
                "is not one of: URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER, URB_FUNCTION_BULK_OR_INTERRUPT_TRANSFER_USING_CHAINED_MDL"
            ));
        }
        ensure_fixed_part_size!(in: dst);
        encode_ts_urb_size!(dst, self.size());

        self.header.encode(dst)?;
        dst.write_u32(self.pipe_handle);
        dst.write_u32(self.transfer_flags);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_URB_BULK_OR_INTERRUPT_TRANSFER"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.8 TS_URB_ISOCH_TRANSFER][1] packet.
///
/// This packet represents [`URB_ISOCH_TRANSFER`][2]. Transfer flags MUST contain
/// `USBD_TRANSFER_DIRECTION_IN` to send using [`TransferInRequest`]; MUST not contain
/// `USBD_TRANSFER_DIRECTION_IN` to send using [`TransferOutRequest`].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/6ded5444-daaf-4a59-96bd-c1a3c6468a82
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_isoch_transfer
#[doc(alias = "TS_URB_ISOCH_TRANSFER")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbIsochTransfer {
    pub header: TsUrbHeader,
    pub pipe_handle: PipeHandle,
    pub transfer_flags: u32,
    pub start_frame: FrameNumber,
    // /// Unused.
    pub error_count: u32,
    // pub iso_packet: Vec<UsbdIsoPacketDesc>,
    pub iso_packet_offsets: Vec<usize>,
}

impl TsUrbIsochTransfer {
    pub fn decode(src: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 20);

        let pipe_handle = src.read_u32();
        let transfer_flags = src.read_u32();
        let start_frame = src.read_u32();
        let number_of_packets = src.read_u32();
        let error_count = src.read_u32();
        // src.advance(4); // ErrorCount

        #[expect(clippy::map_with_unused_argument_over_ranges)]
        let iso_packet_offsets = (0..number_of_packets)
            .map(|_| {
                UsbdIsoPacketDesc::decode(src)
                    .and_then(|iso| usize::try_from(iso.offset).map_err(|e| other_err!(source: e)))
            })
            .collect::<Result<Vec<usize>, _>>()?;

        Ok(Self {
            header,
            pipe_handle,
            transfer_flags,
            start_frame,
            error_count,
            // iso_packet,
            iso_packet_offsets,
        })
    }
}

impl Encode for TsUrbIsochTransfer {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        use UrbFunction::{IsochTransfer, IsochTransferUsingChainedMdl};
        if !matches!(self.header.func, IsochTransfer | IsochTransferUsingChainedMdl) {
            return Err(invalid_field_err!(
                "TS_URB_ISOCH_TRANSFER::TS_URB_HEADER::URB_Function",
                "is not one of: URB_FUNCTION_ISOCH_TRANSFER, URB_FUNCTION_ISOCH_TRANSFER_USING_CHAINED_MDL"
            ));
        }
        ensure_size!(in: dst, size: self.size());
        encode_ts_urb_size!(dst, self.size());

        self.header.encode(dst)?;
        dst.write_u32(self.pipe_handle);
        dst.write_u32(self.transfer_flags);
        dst.write_u32(self.start_frame);
        // dst.write_u32(self.iso_packet.len().try_into().map_err(|_| {
        //     invalid_field_err!(
        //         "TS_URB_ISOCH_TRANSFER::IsoPacket",
        //         "too many packets: count exceeded field NumberOfPackets (4 bytes)"
        //     )
        // })?);
        dst.write_u32(self.iso_packet_offsets.len().try_into().map_err(|_| {
            invalid_field_err!(
                "TS_URB_ISOCH_TRANSFER::IsoPacket",
                "too many packets: count exceeded field NumberOfPackets (4 bytes)"
            )
        })?);
        dst.write_u32(self.error_count);
        // self.iso_packet.iter().try_for_each(|packet| packet.encode(dst))?;
        self.iso_packet_offsets.iter().try_for_each(|offset| {
            u32::try_from(*offset)
                .map_err(|e| other_err!(source: e))
                .and_then(|offset| {
                    UsbdIsoPacketDesc {
                        offset,
                        length: 0,
                        status: 0,
                    }
                    .encode(dst)
                })
            // u32::try_from(*offset)
            //     .map(|offset| dst.write_u32(offset))
            //     .map_err(|e| other_err!(source: e))
        })?;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_URB_ISOCH_TRANSFER"
    }

    fn size(&self) -> usize {
        size_of::<u16>(/* TS_URB_HEADER::Size */)
            + TsUrbHeader::FIXED_PART_SIZE
            + const {
                size_of::<PipeHandle>()
                    + size_of::<u32>(/* TransferFlags */)
                    + size_of::<u32>(/* StartFrame */)
                    + size_of::<u32>(/* NumberOfPackets */)
                    + size_of::<u32>(/* ErrorCount */)
            }
            // + self.iso_packet.len() * UsbdIsoPacketDesc::FIXED_PART_SIZE
            + self.iso_packet_offsets.len() * UsbdIsoPacketDesc::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.9 TS_URB_CONTROL_DESCRIPTOR_REQUEST][1] packet.
///
/// This packet represents [`URB_CONTROL_DESCRIPTOR_REQUEST`][2], and is sent using
/// [`TransferInRequest`] if URB Function in header is one of
/// [`UrbFunction::GetDescriptorFromDevice`], [`UrbFunction::GetDescriptorFromEndpoint`] or
/// [`UrbFunction::GetDescriptorFromInterface`]; otherwise sent using  [`TransferOutRequest`] if
/// URB Function in header is one of [`UrbFunction::SetDescriptorToDevice`],
/// [`UrbFunction::SetDescriptorToEndpoint`] or [`UrbFunction::SetDescriptorToInterface`].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/c6096d89-01e6-40e1-b1c7-9327487c5fff
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_control_descriptor_request
#[doc(alias = "TS_URB_CONTROL_DESCRIPTOR_REQUEST")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbControlDescRequest {
    pub header: TsUrbHeader,
    pub index: u8,
    pub desc_type: u8,
    pub lang_id: u16,
}

impl TsUrbControlDescRequest {
    pub const PAYLOAD_SIZE: usize =
        size_of::<u8>(/* Index */) + size_of::<u8>(/* DescriptorType */) + size_of::<u16>(/* LanguageId */);

    pub const FIXED_PART_SIZE: usize =
        size_of::<u16>(/* TS_URB_HEADER::Size */) + TsUrbHeader::FIXED_PART_SIZE + Self::PAYLOAD_SIZE;

    pub fn decode(src: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let index = src.read_u8();
        let desc_type = src.read_u8();
        let lang_id = src.read_u16();

        Ok(Self {
            header,
            index,
            desc_type,
            lang_id,
        })
    }
}

impl Encode for TsUrbControlDescRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        use UrbFunction::*;
        #[expect(unused_parens)]
        if !matches!(
            self.header.func,
            (GetDescriptorFromDevice | GetDescriptorFromEndpoint | GetDescriptorFromInterface)
                | (SetDescriptorToDevice | SetDescriptorToEndpoint | SetDescriptorToInterface)
        ) {
            return Err(invalid_field_err!(
                "TS_URB_CONTROL_DESCRIPTOR_REQUEST::TS_URB_HEADER::URB_Function",
                "is not one of: \
                    URB_FUNCTION_GET_DESCRIPTOR_FROM_DEVICE, \
                    URB_FUNCTION_GET_DESCRIPTOR_FROM_ENDPOINT, \
                    URB_FUNCTION_GET_DESCRIPTOR_FROM_INTERFACE, \
                    URB_FUNCTION_SET_DESCRIPTOR_TO_DEVICE, \
                    URB_FUNCTION_SET_DESCRIPTOR_TO_ENDPOINT, \
                    URB_FUNCTION_SET_DESCRIPTOR_TO_INTERFACE"
            ));
        }
        ensure_fixed_part_size!(in: dst);

        encode_ts_urb_size!(dst, self.size());
        self.header.encode(dst)?;
        dst.write_u8(self.index);
        dst.write_u8(self.desc_type);
        dst.write_u16(self.lang_id);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_URB_CONTROL_DESCRIPTOR_REQUEST"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.10 TS_URB_CONTROL_FEATURE_REQUEST][1] packet.
///
/// This packet represents [`URB_CONTROL_FEATURE_REQUEST`][2], and is sent using [`TransferInRequest`]
/// (with `output_buffer_size` set to `0`).
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/2ff4f3f0-1205-400d-8e01-88e931855b7a
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_control_feature_request
#[doc(alias = "TS_URB_CONTROL_FEATURE_REQUEST")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbControlFeatRequest {
    pub header: TsUrbHeader,
    pub feat_selector: u16,
    pub index: u16,
}

impl TsUrbControlFeatRequest {
    pub const PAYLOAD_SIZE: usize = size_of::<u16>(/* FeatureSelector */) + size_of::<u16>(/* Index */);

    pub const FIXED_PART_SIZE: usize =
        size_of::<u16>(/* TS_URB_HEADER::Size */) + TsUrbHeader::FIXED_PART_SIZE + Self::PAYLOAD_SIZE;

    pub fn decode(src: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let feat_selector = src.read_u16();
        let index = src.read_u16();

        Ok(Self {
            header,
            feat_selector,
            index,
        })
    }
}

impl Encode for TsUrbControlFeatRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        use UrbFunction::*;

        #[expect(unused_parens)]
        if !matches!(
            self.header.func,
            (SetFeatureToDevice | SetFeatureToInterface | SetFeatureToEndpoint | SetFeatureToOther)
                | (ClearFeatureToDevice | ClearFeatureToInterface | ClearFeatureToEndpoint | ClearFeatureToOther)
        ) {
            return Err(invalid_field_err!(
                "TS_URB_CONTROL_FEATURE_REQUEST::TS_URB_HEADER::URB_Function",
                "is not one of: \
                    URB_FUNCTION_SET_FEATURE_TO_DEVICE, \
                    URB_FUNCTION_SET_FEATURE_TO_INTERFACE, \
                    URB_FUNCTION_SET_FEATURE_TO_ENDPOINT, \
                    URB_FUNCTION_SET_FEATURE_TO_OTHER, \
                    URB_FUNCTION_CLEAR_FEATURE_TO_DEVICE, \
                    URB_FUNCTION_CLEAR_FEATURE_TO_INTERFACE, \
                    URB_FUNCTION_CLEAR_FEATURE_TO_ENDPOINT, \
                    URB_FUNCTION_CLEAR_FEATURE_TO_OTHER"
            ));
        }
        if self.header.no_ack {
            return Err(invalid_field_err!(
                "TS_URB_CONTROL_FEATURE_REQUEST::TS_URB_HEADER::URB_Function::NoAck",
                "is non-zero"
            ));
        }
        ensure_fixed_part_size!(in: dst);

        encode_ts_urb_size!(dst, self.size());
        self.header.encode(dst)?;
        dst.write_u16(self.feat_selector);
        dst.write_u16(self.index);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_URB_CONTROL_FEATURE_REQUEST"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.11 TS_URB_CONTROL_GET_STATUS_REQUEST][1] packet.
///
/// This packet represents [`URB_CONTROL_GET_STATUS_REQUEST`][2], and is sent using
/// [`TransferInRequest`] (with `output_buffer_size` set to `2`).
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/f2a82d78-14f9-426e-826c-13844f1c93b6
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_control_get_status_request
#[doc(alias = "TS_URB_CONTROL_GET_STATUS_REQUEST")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbControlGetStatusRequest {
    pub header: TsUrbHeader,
    pub index: u16,
}

impl TsUrbControlGetStatusRequest {
    pub const PAYLOAD_SIZE: usize = size_of::<u16>(/* Index */) + size_of::<u16>(/* Padding */);

    pub const FIXED_PART_SIZE: usize =
        size_of::<u16>(/* TS_URB_HEADER::Size */) + TsUrbHeader::FIXED_PART_SIZE + Self::PAYLOAD_SIZE;

    pub fn decode(src: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let index = src.read_u16();
        read_padding!(src, 2);

        Ok(Self { header, index })
    }
}

impl Encode for TsUrbControlGetStatusRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        use UrbFunction::*;

        if !matches!(
            self.header.func,
            GetStatusFromDevice | GetStatusFromInterface | GetStatusFromEndpoint | GetStatusFromOther
        ) {
            return Err(invalid_field_err!(
                "TS_URB_CONTROL_GET_STATUS_REQUEST::TS_URB_HEADER::URB_Function",
                "is not one of: \
                    URB_FUNCTION_GET_STATUS_FROM_DEVICE, \
                    URB_FUNCTION_GET_STATUS_FROM_INTERFACE, \
                    URB_FUNCTION_GET_STATUS_FROM_ENDPOINT, \
                    URB_FUNCTION_GET_STATUS_FROM_OTHER"
            ));
        }
        if self.header.no_ack {
            return Err(invalid_field_err!(
                "TS_URB_CONTROL_GET_STATUS_REQUEST::TS_URB_HEADER::URB_Function::NoAck",
                "is non-zero"
            ));
        }
        ensure_fixed_part_size!(in: dst);

        encode_ts_urb_size!(dst, self.size());
        self.header.encode(dst)?;
        dst.write_u16(self.index);
        write_padding!(dst, 2);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_URB_CONTROL_GET_STATUS_REQUEST"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.12 TS_URB_CONTROL_VENDOR_OR_CLASS_REQUEST][1] packet.
///
/// This packet represents [`URB_CONTROL_VENDOR_OR_CLASS_REQUEST`][2]. Transfer flags MUST contain
/// `USBD_TRANSFER_DIRECTION_IN` to send using [`TransferInRequest`]; MUST not contain
/// `USBD_TRANSFER_DIRECTION_IN` to send using [`TransferOutRequest`].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/b97c5a08-5c42-4c13-bc32-e3e29cb0d3d3
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_control_vendor_or_class_request
#[doc(alias = "TS_URB_CONTROL_VENDOR_OR_CLASS_REQUEST")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbControlVendorClassRequest {
    pub header: TsUrbHeader,
    pub transfer_flags: u32,
    pub request: u8,
    pub value: u16,
    pub index: u16,
}

impl TsUrbControlVendorClassRequest {
    pub const PAYLOAD_SIZE: usize = size_of::<u32>()
        + size_of::<u8>(/* RequestTypeReservedBits */)
        + size_of::<u8>(/* Request */)
        + size_of::<u16>(/* Value */)
        + size_of::<u16>(/* Index */)
        + size_of::<u16>(/* Padding */);

    pub const FIXED_PART_SIZE: usize =
        size_of::<u16>(/* TS_URB_HEADER::Size */) + TsUrbHeader::FIXED_PART_SIZE + Self::PAYLOAD_SIZE;

    pub fn decode(src: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let transfer_flags: u32 = src.read_u32();
        src.advance(1); // RequestTypeReservedBits
        let request = src.read_u8();
        let value: u16 = src.read_u16();
        let index: u16 = src.read_u16();
        read_padding!(src, 2);

        Ok(Self {
            header,
            transfer_flags,
            request,
            value,
            index,
        })
    }
}

impl Encode for TsUrbControlVendorClassRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        use UrbFunction::*;

        #[expect(unused_parens)]
        if !matches!(
            self.header.func,
            (VendorDevice | VendorInterface | VendorEndpoint | VendorOther)
                | (ClassDevice | ClassInterface | ClassEndpoint | ClassOther)
        ) {
            return Err(invalid_field_err!(
                "TS_URB_CONTROL_VENDOR_OR_CLASS_REQUEST::TS_URB_HEADER::URB_Function",
                "is not one of: \
                    URB_FUNCTION_VENDOR_DEVICE, \
                    URB_FUNCTION_VENDOR_INTERFACE, \
                    URB_FUNCTION_VENDOR_ENDPOINT, \
                    URB_FUNCTION_VENDOR_OTHER, \
                    URB_FUNCTION_CLASS_DEVICE, \
                    URB_FUNCTION_CLASS_INTERFACE, \
                    URB_FUNCTION_CLASS_ENDPOINT, \
                    URB_FUNCTION_CLASS_OTHER"
            ));
        }
        ensure_fixed_part_size!(in: dst);

        encode_ts_urb_size!(dst, self.size());
        self.header.encode(dst)?;
        dst.write_u32(self.transfer_flags);
        write_padding!(dst, 1); // RequestTypeReservedBits
        dst.write_u8(self.request);
        dst.write_u16(self.value);
        dst.write_u16(self.index);
        write_padding!(dst, 2);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_URB_CONTROL_VENDOR_OR_CLASS_REQUEST"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.13 TS_URB_CONTROL_GET_CONFIGURATION_REQUEST][1] packet.
///
/// This packet represents [`URB_CONTROL_GET_CONFIGURATION_REQUEST`][2], and is sent using
/// [`TransferInRequest`] (with `output_buffer_size` set to `1`).
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/974dabf5-82c2-4f80-a460-9a5f0ac4ede5
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_control_get_configuration_request
#[doc(alias = "TS_URB_CONTROL_GET_CONFIGURATION_REQUEST")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbControlGetConfigRequest {
    pub header: TsUrbHeader,
}

impl TsUrbControlGetConfigRequest {
    pub const FIXED_PART_SIZE: usize = size_of::<u16>(/* TS_URB_HEADER::Size */) + TsUrbHeader::FIXED_PART_SIZE;

    #[inline]
    pub fn decode(_: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        Ok(Self { header })
    }
}

impl Encode for TsUrbControlGetConfigRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if !matches!(self.header.func, UrbFunction::GetConfiguration) {
            return Err(invalid_field_err!(
                "TS_URB_CONTROL_GET_CONFIGURATION_REQUEST::TS_URB_HEADER::URB_Function",
                "is not URB_FUNCTION_GET_CONFIGURATION"
            ));
        }
        if self.header.no_ack {
            return Err(invalid_field_err!(
                "TS_URB_CONTROL_GET_CONFIGURATION_REQUEST::TS_URB_HEADER::URB_Function::NoAck",
                "is non-zero"
            ));
        }
        ensure_fixed_part_size!(in: dst);
        encode_ts_urb_size!(dst, self.size());
        self.header.encode(dst)
    }

    fn name(&self) -> &'static str {
        "TS_URB_CONTROL_GET_CONFIGURATION_REQUEST"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.14 TS_URB_CONTROL_GET_INTERFACE_REQUEST][1] packet.
///
/// This packet represents [`URB_CONTROL_GET_INTERFACE_REQUEST`][2], and is sent using
/// [`TransferInRequest`] (with `output_buffer_size` set to `1`).
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/71d890f0-ec15-4b03-83e2-09fa096bc4e2
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_control_get_interface_request
#[doc(alias = "TS_URB_CONTROL_GET_INTERFACE_REQUEST")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbControlGetInterfaceRequest {
    pub header: TsUrbHeader,
    pub interface: u16,
}

impl TsUrbControlGetInterfaceRequest {
    pub const PAYLOAD_SIZE: usize = size_of::<u16>(/* Interface */) + size_of::<u16>(/* Padding */);

    pub const FIXED_PART_SIZE: usize =
        size_of::<u16>(/* TS_URB_HEADER::Size */) + TsUrbHeader::FIXED_PART_SIZE + Self::PAYLOAD_SIZE;

    pub fn decode(src: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);
        let interface = src.read_u16();
        read_padding!(src, 2);

        Ok(Self { header, interface })
    }
}

impl Encode for TsUrbControlGetInterfaceRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if !matches!(self.header.func, UrbFunction::GetInterface) {
            return Err(invalid_field_err!(
                "TS_URB_CONTROL_GET_INTERFACE_REQUEST::TS_URB_HEADER::URB_Function",
                "is not URB_FUNCTION_GET_INTERFACE"
            ));
        }
        if self.header.no_ack {
            return Err(invalid_field_err!(
                "TS_URB_CONTROL_GET_INTERFACE_REQUEST::TS_URB_HEADER::URB_Function::NoAck",
                "is non-zero"
            ));
        }
        ensure_fixed_part_size!(in: dst);
        encode_ts_urb_size!(dst, self.size());
        self.header.encode(dst)?;
        dst.write_u16(self.interface);
        write_padding!(dst, 2);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_URB_CONTROL_GET_INTERFACE_REQUEST"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.15 TS_URB_OS_FEATURE_DESCRIPTOR_REQUEST][1] packet.
///
/// This packet represents [`URB_OS_FEATURE_DESCRIPTOR_REQUEST`][2], and is sent using
/// [`TransferInRequest`].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/9f6c44ac-5f8e-4c03-95da-fac88d33d91d
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_os_feature_descriptor_request
#[doc(alias = "TS_URB_OS_FEATURE_DESCRIPTOR_REQUEST")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbOsFeatDescRequest {
    pub header: TsUrbHeader,
    pub recipient: u8,
    pub interface_number: u8,
    pub ms_feat_desc_index: u16,
}

impl TsUrbOsFeatDescRequest {
    pub const PAYLOAD_SIZE: usize = size_of::<u8>(/* Recipient + Padding1 */)
        + size_of::<u8>(/* InterfaceNumber */)
        + size_of::<u8>(/* MS_PageIndex */)
        + size_of::<u16>(/* MS_FeatureDescriptorIndex */)
        + (3 * size_of::<u8>()/* Padding2 */);

    pub const FIXED_PART_SIZE: usize =
        size_of::<u16>(/* TS_URB_HEADER::Size */) + TsUrbHeader::FIXED_PART_SIZE + Self::PAYLOAD_SIZE;

    pub fn decode(src: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let recipient = src.read_u8() & 0x1F;
        let interface_number = src.read_u8();
        if src.read_u8(/* MS_PageIndex */) != 0 {
            return Err(invalid_field_err!(
                "TRANSFER_IN_REQUEST::TsUrb: TS_URB_OS_FEATURE_DESCRIPTOR_REQUEST::MS_PageIndex",
                "should be: 0x0"
            ));
        }
        let ms_feat_desc_index = src.read_u16();
        read_padding!(src, 3);

        Ok(Self {
            header,
            recipient,
            interface_number,
            ms_feat_desc_index,
        })
    }
}

impl Encode for TsUrbOsFeatDescRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if !matches!(self.header.func, UrbFunction::GetMsFeatureDescriptor) {
            return Err(invalid_field_err!(
                "TS_URB_OS_FEATURE_DESCRIPTOR_REQUEST::TS_URB_HEADER::URB_Function",
                "is not URB_FUNCTION_GET_MS_FEATURE_DESCRIPTOR"
            ));
        }
        if self.header.no_ack {
            return Err(invalid_field_err!(
                "TS_URB_OS_FEATURE_DESCRIPTOR_REQUEST::TS_URB_HEADER::URB_Function::NoAck",
                "is non-zero"
            ));
        }
        ensure_fixed_part_size!(in: dst);
        encode_ts_urb_size!(dst, self.size());
        self.header.encode(dst)?;
        dst.write_u8(self.recipient & 0x1F);
        dst.write_u8(self.interface_number);
        dst.write_u8(0x0); // MS_PageIndex
        dst.write_u16(self.ms_feat_desc_index);
        write_padding!(dst, 3);

        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_URB_OS_FEATURE_DESCRIPTOR_REQUEST"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEUSB\] 2.2.9.16 TS_URB_CONTROL_TRANSFER_EX][1] packet.
///
/// This packet represents [`URB_CONTROL_TRANSFER_EX`][2]. Transfer flags MUST contain
/// `USBD_TRANSFER_DIRECTION_IN` to send using [`TransferInRequest`]; MUST not contain
/// `USBD_TRANSFER_DIRECTION_IN` to send using [`TransferOutRequest`].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/94be9864-e0f4-4485-b5a0-8df1984dcab8
/// [2]: https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/usb/ns-usb-_urb_control_transfer_ex
#[doc(alias = "TS_URB_CONTROL_TRANSFER_EX")]
#[derive(Debug, PartialEq, Clone)]
pub struct TsUrbControlTransferEx {
    pub header: TsUrbHeader,
    pub pipe: PipeHandle,
    pub transfer_flags: u32,
    pub timeout: u32,
    pub setup_packet: SetupPacket,
}

impl TsUrbControlTransferEx {
    pub const PAYLOAD_SIZE: usize = size_of::<PipeHandle>()
        + size_of::<u32>(/* TransferFlags */)
        + size_of::<u32>(/* Timeout */)
        + SetupPacket::FIXED_PART_SIZE;

    pub const FIXED_PART_SIZE: usize =
        size_of::<u16>(/* TS_URB_HEADER::Size */) + TsUrbHeader::FIXED_PART_SIZE + Self::PAYLOAD_SIZE;

    pub fn decode(src: &mut ReadCursor<'_>, header: TsUrbHeader) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::PAYLOAD_SIZE);

        let pipe_handle = src.read_u32();
        let transfer_flags = src.read_u32();
        let timeout = src.read_u32();
        let setup_packet = SetupPacket::decode(src)?;

        Ok(Self {
            header,
            pipe: pipe_handle,
            transfer_flags,
            timeout,
            setup_packet,
        })
    }
}

impl Encode for TsUrbControlTransferEx {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if !matches!(self.header.func, UrbFunction::ControlTransferEx) {
            return Err(invalid_field_err!(
                "TS_URB_CONTROL_TRANSFER_EX::TS_URB_HEADER::URB_Function",
                "is not URB_FUNCTION_CONTROL_TRANSFER_EX"
            ));
        }
        ensure_fixed_part_size!(in: dst);
        encode_ts_urb_size!(dst, self.size());
        self.header.encode(dst)?;
        dst.write_u32(self.pipe);
        dst.write_u32(self.transfer_flags);
        dst.write_u32(self.timeout);
        self.setup_packet.encode(dst)
    }

    fn name(&self) -> &'static str {
        "TS_URB_CONTROL_TRANSFER_EX"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use TransferDirection::*;
    use utils::TsUsbdPipeInfo;

    use super::*;
    use crate::pdu::utils::{
        RequestIdTransferInOut, USBD_DEFAULT_PIPE_TRANSFER, USBD_START_ISO_TRANSFER_ASAP, USBD_TRANSFER_DIRECTION_OUT,
    };

    fn round_trip(en: &TsUrb, direction: TransferDirection) -> TsUrb {
        let mut buf = vec![0; en.size()];
        en.encode(&mut WriteCursor::new(&mut buf), direction).unwrap();
        TsUrb::decode(&mut ReadCursor::new(&buf), direction).unwrap()
    }

    #[test]
    fn select_config_in() {
        let en = TsUrb::SelectConfig(TsUrbSelectConfig {
            header: TsUrbHeader {
                func: UrbFunction::SelectConfiguration,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            usbd_ifaces: vec![
                TsUsbdInterfaceInfo {
                    interface_number: 1,
                    alternate_setting: 1,
                    ts_usbd_pipe_info: vec![
                        TsUsbdPipeInfo {
                            max_packet_size: 12,
                            max_transfer_size: 34,
                            pipe_flags: 0,
                        },
                        TsUsbdPipeInfo {
                            max_packet_size: 56,
                            max_transfer_size: 78,
                            pipe_flags: 1,
                        },
                    ],
                },
                TsUsbdInterfaceInfo {
                    interface_number: 1,
                    alternate_setting: 2,
                    ts_usbd_pipe_info: vec![
                        TsUsbdPipeInfo {
                            max_packet_size: 13,
                            max_transfer_size: 35,
                            pipe_flags: 0,
                        },
                        TsUsbdPipeInfo {
                            max_packet_size: 57,
                            max_transfer_size: 79,
                            pipe_flags: 1,
                        },
                    ],
                },
            ],
            desc: Some(UsbConfigDesc {
                length: 1,
                descriptor_type: 2,
                total_length: 3,
                num_interfaces: 4,
                configuration_value: 5,
                configuration: 6,
                attributes: 7,
                max_power: 8,
            }),
        });
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn select_interface_in() {
        let en = TsUrb::SelectIface(TsUrbSelectInterface {
            header: TsUrbHeader {
                func: UrbFunction::SelectInterface,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            config_handle: 4,
            usbd_iface: TsUsbdInterfaceInfo {
                interface_number: 1,
                alternate_setting: 2,
                ts_usbd_pipe_info: vec![
                    TsUsbdPipeInfo {
                        max_packet_size: 13,
                        max_transfer_size: 35,
                        pipe_flags: 0,
                    },
                    TsUsbdPipeInfo {
                        max_packet_size: 57,
                        max_transfer_size: 79,
                        pipe_flags: 1,
                    },
                ],
            },
        });
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn pipe_req_in() {
        let mut ts_urb = TsUrbPipeRequest {
            header: TsUrbHeader {
                func: UrbFunction::AbortPipe,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe_handle: 213,
        };

        let en = TsUrb::PipeReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::SyncResetPipeAndClearStall;
        let en = TsUrb::PipeReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::SyncResetPipe;
        let en = TsUrb::PipeReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::SyncClearStall;
        let en = TsUrb::PipeReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::CloseStaticStreams;
        let en = TsUrb::PipeReq(ts_urb);
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn frame_num_in() {
        let en = TsUrb::GetCurFrameNum(TsUrbGetCurrFrameNum {
            header: TsUrbHeader {
                func: UrbFunction::GetCurrentFrameNumber,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
        });
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn control_transfer_in() {
        let en = TsUrb::CtlTransfer(TsUrbControlTransfer {
            header: TsUrbHeader {
                func: UrbFunction::ControlTransfer,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe: 235,
            transfer_flags: USBD_TRANSFER_DIRECTION_IN,
            // We only care about transfer direction for tests (bmRequestType D7)
            setup_packet: SetupPacket {
                request_type: 1 << 7,
                request: 23,
                value: 76,
                index: 12,
                length: 34,
            },
        });
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn control_transfer_out() {
        let mut ts_urb = TsUrbControlTransfer {
            header: TsUrbHeader {
                func: UrbFunction::ControlTransfer,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe: 235,
            transfer_flags: USBD_TRANSFER_DIRECTION_OUT,
            // We only care about transfer direction for tests (bmRequestType D7)
            setup_packet: SetupPacket {
                request_type: 0,
                request: 23,
                value: 76,
                index: 12,
                length: 34,
            },
        };

        let en = TsUrb::CtlTransfer(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.pipe = 0;
        ts_urb.transfer_flags = USBD_TRANSFER_DIRECTION_OUT | USBD_DEFAULT_PIPE_TRANSFER;
        ts_urb.header.no_ack = true;

        let en = TsUrb::CtlTransfer(ts_urb);
        let de = round_trip(&en, Out);
        assert_eq!(en, de);
    }

    #[test]
    fn bulk_or_interrupt_transfer_in() {
        let mut ts_urb = TsUrbBulkOrInterruptTransfer {
            header: TsUrbHeader {
                func: UrbFunction::BulkOrInterruptTransfer,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe_handle: 13,
            transfer_flags: USBD_TRANSFER_DIRECTION_IN,
        };

        let en = TsUrb::BulkInterruptTransfer(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::BulkOrInterruptTransferUsingChainedMdl;
        ts_urb.pipe_handle = 23;

        let en = TsUrb::BulkInterruptTransfer(ts_urb);
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn bulk_or_interrupt_transfer_out() {
        let mut ts_urb = TsUrbBulkOrInterruptTransfer {
            header: TsUrbHeader {
                func: UrbFunction::BulkOrInterruptTransfer,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe_handle: 13,
            transfer_flags: USBD_TRANSFER_DIRECTION_OUT,
        };

        let en = TsUrb::BulkInterruptTransfer(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::BulkOrInterruptTransferUsingChainedMdl;
        let en = TsUrb::BulkInterruptTransfer(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.no_ack = true;

        ts_urb.header.func = UrbFunction::BulkOrInterruptTransfer;
        let en = TsUrb::BulkInterruptTransfer(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::BulkOrInterruptTransferUsingChainedMdl;
        let en = TsUrb::BulkInterruptTransfer(ts_urb);
        let de = round_trip(&en, Out);
        assert_eq!(en, de);
    }

    #[test]
    fn isoch_transfer_in() {
        let mut ts_urb = TsUrbIsochTransfer {
            header: TsUrbHeader {
                func: UrbFunction::IsochTransfer,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe_handle: 23,
            transfer_flags: USBD_TRANSFER_DIRECTION_IN | USBD_START_ISO_TRANSFER_ASAP,
            start_frame: 0,
            error_count: 0,
            iso_packet_offsets: vec![0, 1, 2],
        };

        let en = TsUrb::IsochTransfer(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::IsochTransferUsingChainedMdl;
        let en = TsUrb::IsochTransfer(ts_urb);
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn isoch_transfer_out() {
        let mut ts_urb = TsUrbIsochTransfer {
            header: TsUrbHeader {
                func: UrbFunction::IsochTransfer,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe_handle: 23,
            transfer_flags: USBD_TRANSFER_DIRECTION_OUT | USBD_START_ISO_TRANSFER_ASAP,
            start_frame: 0,
            error_count: 0,
            iso_packet_offsets: vec![0, 1, 2],
        };

        let en = TsUrb::IsochTransfer(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::IsochTransferUsingChainedMdl;
        let en = TsUrb::IsochTransfer(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.no_ack = true;

        ts_urb.header.func = UrbFunction::IsochTransfer;
        let en = TsUrb::IsochTransfer(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::IsochTransferUsingChainedMdl;
        let en = TsUrb::IsochTransfer(ts_urb);
        let de = round_trip(&en, Out);
        assert_eq!(en, de);
    }

    #[test]
    fn control_desc_req_in() {
        let mut ts_urb = TsUrbControlDescRequest {
            header: TsUrbHeader {
                func: UrbFunction::GetDescriptorFromDevice,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            index: 2,
            desc_type: 3,
            lang_id: 4,
        };
        let en = TsUrb::CtlDescReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::GetDescriptorFromEndpoint;
        let en = TsUrb::CtlDescReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::GetDescriptorFromInterface;
        let en = TsUrb::CtlDescReq(ts_urb);
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn control_desc_req_out() {
        let mut ts_urb = TsUrbControlDescRequest {
            header: TsUrbHeader {
                func: UrbFunction::SetDescriptorToDevice,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            index: 2,
            desc_type: 3,
            lang_id: 4,
        };
        let en = TsUrb::CtlDescReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::SetDescriptorToEndpoint;
        let en = TsUrb::CtlDescReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::SetDescriptorToInterface;
        let en = TsUrb::CtlDescReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.no_ack = !ts_urb.header.no_ack;

        ts_urb.header.func = UrbFunction::SetDescriptorToDevice;
        let en = TsUrb::CtlDescReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::SetDescriptorToEndpoint;
        let en = TsUrb::CtlDescReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::SetDescriptorToInterface;
        let en = TsUrb::CtlDescReq(ts_urb);
        let de = round_trip(&en, Out);
        assert_eq!(en, de);
    }

    #[test]
    fn control_feat_req_in() {
        let mut ts_urb = TsUrbControlFeatRequest {
            header: TsUrbHeader {
                func: UrbFunction::SetFeatureToDevice,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            feat_selector: 1,
            index: 2,
        };
        let en = TsUrb::CtlFeatReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::SetFeatureToInterface;
        let en = TsUrb::CtlFeatReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::SetFeatureToEndpoint;
        let en = TsUrb::CtlFeatReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::SetFeatureToOther;
        let en = TsUrb::CtlFeatReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClearFeatureToDevice;
        let en = TsUrb::CtlFeatReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClearFeatureToInterface;
        let en = TsUrb::CtlFeatReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClearFeatureToEndpoint;
        let en = TsUrb::CtlFeatReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClearFeatureToOther;
        let en = TsUrb::CtlFeatReq(ts_urb);
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn control_get_status_req_in() {
        let mut ts_urb = TsUrbControlGetStatusRequest {
            header: TsUrbHeader {
                func: UrbFunction::GetStatusFromDevice,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            index: 234,
        };
        let en = TsUrb::CtlGetStatus(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::GetStatusFromInterface;
        let en = TsUrb::CtlGetStatus(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::GetStatusFromEndpoint;
        let en = TsUrb::CtlGetStatus(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::GetStatusFromOther;
        let en = TsUrb::CtlGetStatus(ts_urb);
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn control_vendor_or_class_req_in() {
        let mut ts_urb = TsUrbControlVendorClassRequest {
            header: TsUrbHeader {
                func: UrbFunction::VendorDevice,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            transfer_flags: USBD_TRANSFER_DIRECTION_IN,
            request: 1,
            value: 2,
            index: 3,
        };
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::VendorInterface;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::VendorEndpoint;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::VendorOther;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClassDevice;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClassInterface;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClassEndpoint;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClassOther;
        let en = TsUrb::VendorClassReq(ts_urb);
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn control_vendor_or_class_req_out() {
        let mut ts_urb = TsUrbControlVendorClassRequest {
            header: TsUrbHeader {
                func: UrbFunction::VendorDevice,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            transfer_flags: USBD_TRANSFER_DIRECTION_OUT,
            request: 10,
            value: 11,
            index: 12,
        };
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::VendorInterface;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::VendorEndpoint;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::VendorOther;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClassDevice;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClassInterface;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClassEndpoint;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClassOther;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.no_ack = !ts_urb.header.no_ack;

        ts_urb.header.func = UrbFunction::VendorDevice;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::VendorInterface;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::VendorEndpoint;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::VendorOther;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClassDevice;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClassInterface;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClassEndpoint;
        let en = TsUrb::VendorClassReq(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.header.func = UrbFunction::ClassOther;
        let en = TsUrb::VendorClassReq(ts_urb);
        let de = round_trip(&en, Out);
        assert_eq!(en, de);
    }

    #[test]
    fn control_get_config_req_in() {
        let en = TsUrb::CtlGetConfig(TsUrbControlGetConfigRequest {
            header: TsUrbHeader {
                func: UrbFunction::GetConfiguration,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
        });
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn control_get_iface_req_in() {
        let en = TsUrb::CtlGetIface(TsUrbControlGetInterfaceRequest {
            header: TsUrbHeader {
                func: UrbFunction::GetInterface,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            interface: 5,
        });
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn os_feat_desc_req_in() {
        let mut ts_urb = TsUrbOsFeatDescRequest {
            header: TsUrbHeader {
                func: UrbFunction::GetMsFeatureDescriptor,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            recipient: 0,
            interface_number: 0,
            ms_feat_desc_index: 213,
        };
        let en = TsUrb::OsFeatDescReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.recipient = 1;
        ts_urb.interface_number = 1;
        let en = TsUrb::OsFeatDescReq(ts_urb.clone());
        let de = round_trip(&en, In);
        assert_eq!(en, de);

        ts_urb.recipient = 2;
        ts_urb.interface_number = 1;
        let en = TsUrb::OsFeatDescReq(ts_urb);
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn control_transfer_ex_in() {
        let en = TsUrb::CtlTransferEx(TsUrbControlTransferEx {
            header: TsUrbHeader {
                func: UrbFunction::ControlTransferEx,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe: 235,
            transfer_flags: USBD_TRANSFER_DIRECTION_IN,
            timeout: 12,
            // We only care about transfer direction for tests (bmRequestType D7)
            setup_packet: SetupPacket {
                request_type: 1 << 7,
                request: 23,
                value: 76,
                index: 12,
                length: 34,
            },
        });
        let de = round_trip(&en, In);
        assert_eq!(en, de);
    }

    #[test]
    fn control_transfer_ex_out() {
        let mut ts_urb = TsUrbControlTransferEx {
            header: TsUrbHeader {
                func: UrbFunction::ControlTransferEx,
                req_id: RequestIdTransferInOut::try_from(3453).unwrap(),
                no_ack: false,
            },
            pipe: 235,
            transfer_flags: USBD_TRANSFER_DIRECTION_OUT,
            timeout: 234,
            // We only care about transfer direction for tests (bmRequestType D7)
            setup_packet: SetupPacket {
                request_type: 0,
                request: 23,
                value: 76,
                index: 12,
                length: 34,
            },
        };

        let en = TsUrb::CtlTransferEx(ts_urb.clone());
        let de = round_trip(&en, Out);
        assert_eq!(en, de);

        ts_urb.pipe = 0;
        ts_urb.transfer_flags = USBD_TRANSFER_DIRECTION_OUT | USBD_DEFAULT_PIPE_TRANSFER;
        ts_urb.header.no_ack = true;

        let en = TsUrb::CtlTransferEx(ts_urb);
        let de = round_trip(&en, Out);
        assert_eq!(en, de);
    }
}
