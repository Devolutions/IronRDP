//! Messages specific to [Interface Manipulation][1] interface.
//!
//! MS-RDPEUSB utilizes the same Interface Query and Interface Release messages that are defined in
//! [MS-RDPEXPS][2].
//!
//! [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpeusb/6dd37383-9aed-4f9e-ba74-febe3a21f0f5
//! [2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpexps/ebe401f0-f22e-4de4-9cd3-2a55e5493500

use ironrdp_core::{Decode, Encode, ensure_fixed_part_size, ensure_size, invalid_field_err};

use crate::pdu::header::{FunctionId, MessageId, SharedMsgHeader};

/// [\[MS-RDPEXPS\] 2.2.2.2 Interface Release (IFACE_RELEASE)][1] message.
///
/// One-way message that terminates an interface's lifetime.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpexps/5db96fd4-617f-432f-b4ec-58f75564eb06
#[doc(alias = "IFACE_RELEASE")]
#[derive(Debug, PartialEq)]
pub struct InterfaceRelease {
    pub iface_id: u32,
    pub msg_id: MessageId,
}

impl InterfaceRelease {
    pub const FIXED_PART_SIZE: usize = SharedMsgHeader::SIZE_RSP;

    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.iface_id,
            msg_id: self.msg_id,
            function_id: Some(FunctionId::RIMCALL_RELEASE),
        }
    }

    pub(super) fn from_header(header: SharedMsgHeader) -> Self {
        debug_assert!(header.function_id == Some(FunctionId::RIMCALL_RELEASE));
        Self {
            iface_id: header.iface_id,
            msg_id: header.msg_id,
        }
    }
}

impl Decode<'_> for InterfaceRelease {
    fn decode(src: &mut ironrdp_core::ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        let iface_id = src.read_u32();
        let msg_id = src.read_u32();
        if FunctionId(src.read_u32()) != FunctionId::RIMCALL_RELEASE {
            return Err(invalid_field_err!(
                "SHARED_MSG_HEADER::FunctionId",
                "must be 0x1 (RIMCALL_RELEASE)"
            ));
        }

        Ok(Self { iface_id, msg_id })
    }
}

impl Encode for InterfaceRelease {
    fn encode(&self, dst: &mut ironrdp_core::WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        self.header().encode(dst)
    }

    fn name(&self) -> &'static str {
        "IFACE_RELEASE"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEXPS\] 2.2.2.1.1 Query Interface Request (QI_REQ)][1] message.
///
/// Request a new interface ID. Per [MS-RDPEXPS § 3.1.5.2.1.1] the server MUST NOT send `QI_REQ`;
/// MS-RDPEUSB inherits this restriction. We decode incoming `QI_REQ` for ecosystem tolerance and
/// answer with a failure [`QueryInterfaceFailureResponse`].
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpexps/10757445-d7dd-4602-b75f-772540c01a5d
#[doc(alias = "QI_REQ")]
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct QueryInterfaceRequest {
    pub iface_id: u32,
    pub msg_id: MessageId,
    pub new_interface_guid: u128,
}

impl QueryInterfaceRequest {
    pub const FIXED_PART_SIZE: usize = SharedMsgHeader::SIZE_REQ + 16 /* NewInterfaceGUID */;

    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.iface_id,
            msg_id: self.msg_id,
            function_id: Some(FunctionId::RIMCALL_QUERYINTERFACE),
        }
    }

    pub(super) fn decode(
        src: &mut ironrdp_core::ReadCursor<'_>,
        header: SharedMsgHeader,
    ) -> ironrdp_core::DecodeResult<Self> {
        ensure_size!(in: src, size: 16);
        Ok(Self {
            iface_id: header.iface_id,
            msg_id: header.msg_id,
            new_interface_guid: src.read_u128(),
        })
    }
}

impl Encode for QueryInterfaceRequest {
    fn encode(&self, dst: &mut ironrdp_core::WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        self.header().encode(dst)?;
        ensure_size!(in: dst, size: 16);
        dst.write_u128(self.new_interface_guid);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "QI_REQ"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}

/// [\[MS-RDPEXPS\] 2.2.2.1.2 Query Interface Response (QI_RSP)][1] — **failure** variant.
///
/// Per [MS-RDPEXPS § 3.1.5.2.1.2], on receiving a `QI_REQ` the receiver SHOULD return the failure
/// version of `QI_RSP` — a `QI_RSP` **omitting** the optional `NewInterfaceId` field. The
/// originating side MUST interpret this as "interface not supported".
///
/// We never advertise any negotiable interface, so we always reply with this failure variant;
/// there is no `QueryInterfaceSuccessResponse` type.
///
/// [1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpexps/bcf53670-4db2-450a-b53e-879756ca18a8
#[doc(alias = "QI_RSP")]
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct QueryInterfaceFailureResponse {
    pub iface_id: u32,
    pub msg_id: MessageId,
}

impl QueryInterfaceFailureResponse {
    pub const FIXED_PART_SIZE: usize = SharedMsgHeader::SIZE_RSP;

    pub fn for_request(req: &QueryInterfaceRequest) -> Self {
        Self {
            iface_id: req.iface_id,
            msg_id: req.msg_id,
        }
    }

    pub fn header(&self) -> SharedMsgHeader {
        SharedMsgHeader {
            iface_id: self.iface_id,
            msg_id: self.msg_id,
            function_id: None,
        }
    }
}

impl Encode for QueryInterfaceFailureResponse {
    fn encode(&self, dst: &mut ironrdp_core::WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        // Failure QI_RSP = SHARED_MSG_HEADER only, no body.
        self.header().encode(dst)
    }

    fn name(&self) -> &'static str {
        "QI_RSP"
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE
    }
}
