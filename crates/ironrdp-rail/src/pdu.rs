//! [MS-RDPERP] 2.2.2 RAIL static-channel PDUs.
//!
//! The codec does not couple decoding to session state.
//! Consumers can classify each [`RailPdu`] and validate its expected direction before sending it.
//!
//! [MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp/

#[cfg(not(feature = "std"))]
use alloc::{borrow::ToOwned as _, string::String, vec::Vec};
use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, invalid_field_err};
#[cfg(feature = "std")]
use ironrdp_svc::SvcEncode;

const HEADER_SIZE: usize = 2 /* orderType */ + 2 /* orderLength */;
const MAX_PDU_SIZE: usize = 65_535;
const MAX_EXECUTABLE_BYTES: usize = 520;
const MAX_ARGUMENT_BYTES: usize = 16_000;
const FIXED_APPLICATION_ID_BYTES: usize = 520;

macro_rules! ensure_remaining {
    ($available:expr, $needed:expr, $name:literal) => {
        if $available < $needed {
            return Err(invalid_field_err!($name, "not enough bytes"));
        }
    };
}

macro_rules! impl_wire_value {
    ($type:ty => $wire:ty { $($variant:path => $value:expr),+ $(,)? }) => {
        impl From<$type> for $wire {
            fn from(value: $type) -> Self {
                match value {
                    $($variant => $value),+
                }
            }
        }
    };
}

/// A RAIL order type from a [`RailPduHeader`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum RailOrderType {
    Execute = 0x0001,
    Activate = 0x0002,
    SystemParameters = 0x0003,
    SystemCommand = 0x0004,
    Handshake = 0x0005,
    NotifyEvent = 0x0006,
    WindowMove = 0x0008,
    LocalMoveSize = 0x0009,
    MinMaxInfo = 0x000A,
    ClientStatus = 0x000B,
    SystemMenu = 0x000C,
    LanguageBarInfo = 0x000D,
    GetApplicationIdRequest = 0x000E,
    GetApplicationIdResponse = 0x000F,
    TaskbarInfo = 0x0010,
    LanguageImeInfo = 0x0011,
    CompartmentInfo = 0x0012,
    HandshakeEx = 0x0013,
    ZOrderSync = 0x0014,
    Cloak = 0x0015,
    PowerDisplayRequest = 0x0016,
    SnapArrange = 0x0017,
    GetApplicationIdResponseEx = 0x0018,
    TextScaleInfo = 0x0019,
    CaretBlinkInfo = 0x001A,
    ExecuteResult = 0x0080,
}

impl TryFrom<u16> for RailOrderType {
    type Error = u16;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        let order = match value {
            0x0001 => Self::Execute,
            0x0002 => Self::Activate,
            0x0003 => Self::SystemParameters,
            0x0004 => Self::SystemCommand,
            0x0005 => Self::Handshake,
            0x0006 => Self::NotifyEvent,
            0x0008 => Self::WindowMove,
            0x0009 => Self::LocalMoveSize,
            0x000A => Self::MinMaxInfo,
            0x000B => Self::ClientStatus,
            0x000C => Self::SystemMenu,
            0x000D => Self::LanguageBarInfo,
            0x000E => Self::GetApplicationIdRequest,
            0x000F => Self::GetApplicationIdResponse,
            0x0010 => Self::TaskbarInfo,
            0x0011 => Self::LanguageImeInfo,
            0x0012 => Self::CompartmentInfo,
            0x0013 => Self::HandshakeEx,
            0x0014 => Self::ZOrderSync,
            0x0015 => Self::Cloak,
            0x0016 => Self::PowerDisplayRequest,
            0x0017 => Self::SnapArrange,
            0x0018 => Self::GetApplicationIdResponseEx,
            0x0019 => Self::TextScaleInfo,
            0x001A => Self::CaretBlinkInfo,
            0x0080 => Self::ExecuteResult,
            _ => return Err(value),
        };

        Ok(order)
    }
}

/// Common RAIL header ([MS-RDPERP] 2.2.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailPduHeader {
    pub order_type: RailOrderType,
    pub order_length: u16,
}

impl RailPduHeader {
    /// Creates and validates a common RAIL header.
    pub fn new(order_type: RailOrderType, order_length: u16) -> EncodeResult<Self> {
        if usize::from(order_length) < HEADER_SIZE {
            return Err(invalid_field_err!(
                "orderLength",
                "RAIL PDU length is smaller than its header"
            ));
        }

        Ok(Self {
            order_type,
            order_length,
        })
    }
}

impl Encode for RailPduHeader {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if usize::from(self.order_length) < HEADER_SIZE {
            return Err(invalid_field_err!(
                "orderLength",
                "RAIL PDU length is smaller than its header"
            ));
        }
        ensure_remaining!(dst.len(), HEADER_SIZE, "TS_RAIL_PDU_HEADER");
        dst.write_u16(u16::from(self.order_type));
        dst.write_u16(self.order_length);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_RAIL_PDU_HEADER"
    }

    fn size(&self) -> usize {
        HEADER_SIZE
    }
}

impl<'de> Decode<'de> for RailPduHeader {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_remaining!(src.len(), HEADER_SIZE, "TS_RAIL_PDU_HEADER");
        let raw_order_type = src.read_u16();
        let order_type = RailOrderType::try_from(raw_order_type)
            .map_err(|_| invalid_field_err!("orderType", "unknown RAIL order type"))?;
        let order_length = src.read_u16();

        if usize::from(order_length) < HEADER_SIZE {
            return Err(invalid_field_err!(
                "orderLength",
                "RAIL PDU length is smaller than its header"
            ));
        }

        Ok(Self {
            order_type,
            order_length,
        })
    }
}

/// A window-move rectangle in virtual screen coordinates ([MS-RDPERP] 2.2.2.7.4/2.2.2.7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rectangle {
    pub left: i16,
    pub top: i16,
    pub right: i16,
    pub bottom: i16,
}

impl Rectangle {
    fn encode(self, dst: &mut WriteCursor<'_>) {
        dst.write_i16(self.left);
        dst.write_i16(self.top);
        dst.write_i16(self.right);
        dst.write_i16(self.bottom);
    }

    fn decode(src: &mut ReadCursor<'_>) -> Self {
        Self {
            left: src.read_i16(),
            top: src.read_i16(),
            right: src.read_i16(),
            bottom: src.read_i16(),
        }
    }
}

/// A `TS_RECTANGLE_16` system-parameter rectangle ([MS-RDPERP] 2.2.1.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemParameterRectangle {
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
}

impl SystemParameterRectangle {
    const SIZE: usize = 2 /* left */ + 2 /* top */ + 2 /* right */ + 2 /* bottom */;

    fn encode(self, dst: &mut WriteCursor<'_>) {
        dst.write_u16(self.left);
        dst.write_u16(self.top);
        dst.write_u16(self.right);
        dst.write_u16(self.bottom);
    }

    fn decode(src: &mut ReadCursor<'_>) -> Self {
        Self {
            left: src.read_u16(),
            top: src.read_u16(),
            right: src.read_u16(),
            bottom: src.read_u16(),
        }
    }
}

/// A GUID encoded with the RAIL wire layout ([MS-RDPERP] 2.2.2.10.1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailGuid(pub [u8; 16]);

impl RailGuid {
    pub const NULL: Self = Self([0; 16]);

    fn is_null(self) -> bool {
        self == Self::NULL
    }

    fn encode(self, dst: &mut WriteCursor<'_>) {
        dst.write_slice(&self.0);
    }

    fn decode(src: &mut ReadCursor<'_>) -> Self {
        Self(src.read_array())
    }
}

/// The permitted direction for a RAIL order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailPduDirection {
    ClientToServer,
    ServerToClient,
}

/// Full message set on the RAIL static channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RailPdu {
    Handshake(HandshakePdu),
    ClientStatus(ClientStatusPdu),
    HandshakeEx(HandshakeExPdu),
    Execute(ExecutePdu),
    ExecuteResult(ExecuteResultPdu),
    ClientSystemParameters(ClientSystemParametersPdu),
    ServerSystemParameters(ServerSystemParametersPdu),
    Activate(ActivatePdu),
    SystemMenu(SystemMenuPdu),
    SystemCommand(SystemCommandPdu),
    NotifyEvent(NotifyEventPdu),
    GetApplicationIdRequest(GetApplicationIdRequestPdu),
    MinMaxInfo(MinMaxInfoPdu),
    LocalMoveSize(LocalMoveSizePdu),
    WindowMove(WindowMovePdu),
    SnapArrange(SnapArrangePdu),
    GetApplicationIdResponse(GetApplicationIdResponsePdu),
    GetApplicationIdResponseEx(GetApplicationIdResponseExPdu),
    LanguageBarInfo(LanguageBarInfoPdu),
    LanguageImeInfo(LanguageImeInfoPdu),
    CompartmentInfo(CompartmentInfoPdu),
    ZOrderSync(ZOrderSyncPdu),
    Cloak(CloakPdu),
    PowerDisplayRequest(PowerDisplayRequestPdu),
    TaskbarInfo(TaskbarInfoPdu),
    TextScaleInfo(TextScaleInfoPdu),
    CaretBlinkInfo(CaretBlinkInfoPdu),
}

impl RailPdu {
    /// Returns whether this PDU is valid from the server to the client.
    pub const fn is_server_to_client(&self) -> bool {
        matches!(
            self,
            Self::Handshake(_)
                | Self::HandshakeEx(_)
                | Self::ExecuteResult(_)
                | Self::ServerSystemParameters(_)
                | Self::MinMaxInfo(_)
                | Self::LocalMoveSize(_)
                | Self::GetApplicationIdResponse(_)
                | Self::GetApplicationIdResponseEx(_)
                | Self::LanguageBarInfo(_)
                | Self::CompartmentInfo(_)
                | Self::ZOrderSync(_)
                | Self::Cloak(_)
                | Self::PowerDisplayRequest(_)
                | Self::TaskbarInfo(_)
        )
    }

    /// Returns whether this PDU is valid from the client to the server.
    pub const fn is_client_to_server(&self) -> bool {
        matches!(
            self,
            Self::Handshake(_)
                | Self::ClientStatus(_)
                | Self::Execute(_)
                | Self::ClientSystemParameters(_)
                | Self::Activate(_)
                | Self::SystemMenu(_)
                | Self::SystemCommand(_)
                | Self::NotifyEvent(_)
                | Self::GetApplicationIdRequest(_)
                | Self::WindowMove(_)
                | Self::SnapArrange(_)
                | Self::LanguageBarInfo(_)
                | Self::LanguageImeInfo(_)
                | Self::CompartmentInfo(_)
                | Self::Cloak(_)
                | Self::TextScaleInfo(_)
                | Self::CaretBlinkInfo(_)
        )
    }

    /// Validates this PDU against an expected channel direction.
    pub fn validate_direction(&self, expected: RailPduDirection) -> EncodeResult<()> {
        let valid = match expected {
            RailPduDirection::ClientToServer => self.is_client_to_server(),
            RailPduDirection::ServerToClient => self.is_server_to_client(),
        };

        if !valid {
            return Err(invalid_field_err!(
                "orderType",
                "RAIL order is invalid for this channel direction"
            ));
        }

        Ok(())
    }

    /// Returns the order type associated with this PDU.
    pub const fn order_type(&self) -> RailOrderType {
        match self {
            Self::Handshake(_) => RailOrderType::Handshake,
            Self::ClientStatus(_) => RailOrderType::ClientStatus,
            Self::HandshakeEx(_) => RailOrderType::HandshakeEx,
            Self::Execute(_) => RailOrderType::Execute,
            Self::ExecuteResult(_) => RailOrderType::ExecuteResult,
            Self::ClientSystemParameters(_) | Self::ServerSystemParameters(_) => RailOrderType::SystemParameters,
            Self::Activate(_) => RailOrderType::Activate,
            Self::SystemMenu(_) => RailOrderType::SystemMenu,
            Self::SystemCommand(_) => RailOrderType::SystemCommand,
            Self::NotifyEvent(_) => RailOrderType::NotifyEvent,
            Self::GetApplicationIdRequest(_) => RailOrderType::GetApplicationIdRequest,
            Self::MinMaxInfo(_) => RailOrderType::MinMaxInfo,
            Self::LocalMoveSize(_) => RailOrderType::LocalMoveSize,
            Self::WindowMove(_) => RailOrderType::WindowMove,
            Self::SnapArrange(_) => RailOrderType::SnapArrange,
            Self::GetApplicationIdResponse(_) => RailOrderType::GetApplicationIdResponse,
            Self::GetApplicationIdResponseEx(_) => RailOrderType::GetApplicationIdResponseEx,
            Self::LanguageBarInfo(_) => RailOrderType::LanguageBarInfo,
            Self::LanguageImeInfo(_) => RailOrderType::LanguageImeInfo,
            Self::CompartmentInfo(_) => RailOrderType::CompartmentInfo,
            Self::ZOrderSync(_) => RailOrderType::ZOrderSync,
            Self::Cloak(_) => RailOrderType::Cloak,
            Self::PowerDisplayRequest(_) => RailOrderType::PowerDisplayRequest,
            Self::TaskbarInfo(_) => RailOrderType::TaskbarInfo,
            Self::TextScaleInfo(_) => RailOrderType::TextScaleInfo,
            Self::CaretBlinkInfo(_) => RailOrderType::CaretBlinkInfo,
        }
    }

    fn body_size(&self) -> usize {
        match self {
            Self::Handshake(_) => 4,
            Self::ClientStatus(_) => 4,
            Self::HandshakeEx(_) => 8,
            Self::Execute(pdu) => {
                8 + utf16_bytes(&pdu.executable) + utf16_bytes(&pdu.working_directory) + utf16_bytes(&pdu.arguments)
            }
            Self::ExecuteResult(pdu) => 12 + utf16_bytes(&pdu.executable),
            Self::ClientSystemParameters(pdu) => 4 + pdu.parameter.size(),
            Self::ServerSystemParameters(_) => 5,
            Self::Activate(_) => 5,
            Self::SystemMenu(_) => 8,
            Self::SystemCommand(_) => 6,
            Self::NotifyEvent(_) => 12,
            Self::GetApplicationIdRequest(_) => 4,
            Self::MinMaxInfo(_) => 20,
            Self::LocalMoveSize(_) => 12,
            Self::WindowMove(_) | Self::SnapArrange(_) => 12,
            Self::GetApplicationIdResponse(_) => 4 + FIXED_APPLICATION_ID_BYTES,
            Self::GetApplicationIdResponseEx(_) => 4 + FIXED_APPLICATION_ID_BYTES + 4 + FIXED_APPLICATION_ID_BYTES,
            Self::LanguageBarInfo(_) => 4,
            Self::LanguageImeInfo(_) => 4 + 2 + 16 + 16 + 4,
            Self::CompartmentInfo(_) => 16,
            Self::ZOrderSync(_) => 4,
            Self::Cloak(_) => 5,
            Self::PowerDisplayRequest(_) => 4,
            Self::TaskbarInfo(_) => 12,
            Self::TextScaleInfo(_) | Self::CaretBlinkInfo(_) => 4,
        }
    }

    /// Validates this PDU before it is queued for transport.
    pub fn validate(&self) -> EncodeResult<()> {
        let body_size = self.body_size();
        pdu_length(body_size)?;

        match self {
            Self::Handshake(_) | Self::MinMaxInfo(_) | Self::SystemMenu(_) | Self::Activate(_) => Ok(()),
            Self::ClientStatus(pdu) => validate_bits(pdu.flags, ClientStatusPdu::VALID_FLAGS, "Flags"),
            Self::HandshakeEx(pdu) => validate_bits(pdu.flags, HandshakeExPdu::VALID_FLAGS, "railHandshakeFlags"),
            Self::Execute(pdu) => pdu.validate(),
            Self::ExecuteResult(pdu) => pdu.validate(),
            Self::ClientSystemParameters(pdu) => pdu.parameter.validate(),
            Self::ServerSystemParameters(_) => Ok(()),
            Self::SystemCommand(_) | Self::NotifyEvent(_) | Self::GetApplicationIdRequest(_) => Ok(()),
            Self::LocalMoveSize(_) | Self::WindowMove(_) | Self::SnapArrange(_) => Ok(()),
            Self::GetApplicationIdResponse(pdu) => validate_fixed_utf16(&pdu.application_id, "ApplicationId"),
            Self::GetApplicationIdResponseEx(pdu) => {
                validate_fixed_utf16(&pdu.application_id, "ApplicationId")?;
                validate_fixed_utf16(&pdu.process_image_name, "ProcessImageName")
            }
            Self::LanguageBarInfo(pdu) => pdu.validate(),
            Self::LanguageImeInfo(pdu) => pdu.validate(),
            Self::CompartmentInfo(pdu) => pdu.validate(),
            Self::ZOrderSync(_) => Ok(()),
            Self::Cloak(_) | Self::PowerDisplayRequest(_) => Ok(()),
            Self::TaskbarInfo(_) => Ok(()),
            Self::TextScaleInfo(pdu) => {
                if !(100..=225).contains(&pdu.text_scale_factor) {
                    return Err(invalid_field_err!(
                        "TextScaleFactor",
                        "text scale factor must be between 100 and 225"
                    ));
                }
                Ok(())
            }
            Self::CaretBlinkInfo(_) => Ok(()),
        }
    }

    fn encode_body(&self, dst: &mut WriteCursor<'_>) {
        match self {
            Self::Handshake(pdu) => dst.write_u32(pdu.build_number),
            Self::ClientStatus(pdu) => dst.write_u32(pdu.flags),
            Self::HandshakeEx(pdu) => {
                dst.write_u32(pdu.build_number);
                dst.write_u32(pdu.flags);
            }
            Self::Execute(pdu) => {
                dst.write_u16(pdu.flags);
                dst.write_u16(narrow_u16(utf16_bytes(&pdu.executable)));
                dst.write_u16(narrow_u16(utf16_bytes(&pdu.working_directory)));
                dst.write_u16(narrow_u16(utf16_bytes(&pdu.arguments)));
                write_utf16(dst, &pdu.executable);
                write_utf16(dst, &pdu.working_directory);
                write_utf16(dst, &pdu.arguments);
            }
            Self::ExecuteResult(pdu) => {
                dst.write_u16(pdu.flags);
                dst.write_u16(u16::from(pdu.result));
                dst.write_u32(pdu.raw_result);
                dst.write_u16(0);
                dst.write_u16(narrow_u16(utf16_bytes(&pdu.executable)));
                write_utf16(dst, &pdu.executable);
            }
            Self::ClientSystemParameters(pdu) => {
                dst.write_u32(pdu.parameter.kind());
                pdu.parameter.encode(dst);
            }
            Self::ServerSystemParameters(pdu) => {
                dst.write_u32(u32::from(pdu.parameter));
                dst.write_u8(u8::from(pdu.enabled));
            }
            Self::Activate(pdu) => {
                dst.write_u32(pdu.window_id);
                dst.write_u8(u8::from(pdu.enabled));
            }
            Self::SystemMenu(pdu) => {
                dst.write_u32(pdu.window_id);
                dst.write_i16(pdu.left);
                dst.write_i16(pdu.top);
            }
            Self::SystemCommand(pdu) => {
                dst.write_u32(pdu.window_id);
                dst.write_u16(u16::from(pdu.command));
            }
            Self::NotifyEvent(pdu) => {
                dst.write_u32(pdu.window_id);
                dst.write_u32(pdu.notify_icon_id);
                dst.write_u32(u32::from(pdu.message));
            }
            Self::GetApplicationIdRequest(pdu) => dst.write_u32(pdu.window_id),
            Self::MinMaxInfo(pdu) => {
                dst.write_u32(pdu.window_id);
                for value in pdu.values {
                    dst.write_i16(value);
                }
            }
            Self::LocalMoveSize(pdu) => {
                dst.write_u32(pdu.window_id);
                dst.write_u16(u16::from(pdu.is_start));
                dst.write_u16(u16::from(pdu.move_size_type));
                dst.write_i16(pdu.x);
                dst.write_i16(pdu.y);
            }
            Self::WindowMove(pdu) => {
                dst.write_u32(pdu.window_id);
                pdu.rectangle.encode(dst);
            }
            Self::SnapArrange(pdu) => {
                dst.write_u32(pdu.window_id);
                pdu.rectangle.encode(dst);
            }
            Self::GetApplicationIdResponse(pdu) => {
                dst.write_u32(pdu.window_id);
                write_fixed_utf16(dst, &pdu.application_id);
            }
            Self::GetApplicationIdResponseEx(pdu) => {
                dst.write_u32(pdu.window_id);
                write_fixed_utf16(dst, &pdu.application_id);
                dst.write_u32(pdu.process_id);
                write_fixed_utf16(dst, &pdu.process_image_name);
            }
            Self::LanguageBarInfo(pdu) => dst.write_u32(pdu.status),
            Self::LanguageImeInfo(pdu) => {
                dst.write_u32(u32::from(pdu.profile_type));
                dst.write_u16(pdu.language_id);
                pdu.language_profile_clsid.encode(dst);
                pdu.profile_guid.encode(dst);
                dst.write_u32(pdu.keyboard_layout);
            }
            Self::CompartmentInfo(pdu) => {
                dst.write_u32(u32::from(pdu.ime_state));
                dst.write_u32(pdu.ime_conversion_mode);
                dst.write_u32(pdu.ime_sentence_mode);
                dst.write_u32(u32::from(pdu.kana_mode));
            }
            Self::ZOrderSync(pdu) => dst.write_u32(pdu.window_id_marker),
            Self::Cloak(pdu) => {
                dst.write_u32(pdu.window_id);
                dst.write_u8(u8::from(pdu.cloaked));
            }
            Self::PowerDisplayRequest(pdu) => dst.write_u32(u32::from(pdu.active)),
            Self::TaskbarInfo(pdu) => {
                dst.write_u32(u32::from(pdu.message));
                dst.write_u32(pdu.window_id_tab);
                dst.write_u32(pdu.body);
            }
            Self::TextScaleInfo(pdu) => dst.write_u32(pdu.text_scale_factor),
            Self::CaretBlinkInfo(pdu) => dst.write_u32(pdu.caret_blink_rate),
        }
    }
}

impl Encode for RailPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        self.validate()?;
        let body_size = self.body_size();
        ensure_remaining!(dst.len(), HEADER_SIZE + body_size, "RAIL PDU");
        RailPduHeader::new(self.order_type(), pdu_length(body_size)?)?.encode(dst)?;
        self.encode_body(dst);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "TS_RAIL_PDU"
    }

    fn size(&self) -> usize {
        HEADER_SIZE.saturating_add(self.body_size()).min(MAX_PDU_SIZE)
    }
}

#[cfg(feature = "std")]
impl SvcEncode for RailPdu {}

impl<'de> Decode<'de> for RailPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let full_length = src.len();
        let header = RailPduHeader::decode(src)?;
        if usize::from(header.order_length) != full_length {
            return Err(invalid_field_err!(
                "orderLength",
                "RAIL PDU length does not match the payload size"
            ));
        }

        let mut body = ReadCursor::new(src.read_remaining());
        let pdu = decode_body(header.order_type, &mut body)?;
        if !body.is_empty() {
            return Err(invalid_field_err!("orderLength", "trailing bytes after RAIL PDU body"));
        }
        Ok(pdu)
    }
}

/// Handshake PDU ([MS-RDPERP] 2.2.2.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandshakePdu {
    pub build_number: u32,
}

/// Client Information PDU ([MS-RDPERP] 2.2.2.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientStatusPdu {
    pub flags: u32,
}

impl ClientStatusPdu {
    pub const ALLOW_LOCAL_MOVE_SIZE: u32 = 0x0000_0001;
    pub const AUTO_RECONNECT: u32 = 0x0000_0002;
    pub const Z_ORDER_SYNC: u32 = 0x0000_0004;
    pub const WINDOW_RESIZE_MARGIN: u32 = 0x0000_0010;
    pub const HIGH_DPI_ICONS: u32 = 0x0000_0020;
    pub const APPBAR_REMOTING: u32 = 0x0000_0040;
    pub const POWER_DISPLAY_REQUEST: u32 = 0x0000_0080;
    pub const BIDIRECTIONAL_CLOAK: u32 = 0x0000_0200;
    pub const SUPPRESS_ICON_ORDERS: u32 = 0x0000_0400;
    pub const VALID_FLAGS: u32 = 0x0000_06F7;
}

/// HandshakeEx PDU ([MS-RDPERP] 2.2.2.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandshakeExPdu {
    pub build_number: u32,
    pub flags: u32,
}

impl HandshakeExPdu {
    pub const HIDEF: u32 = 0x0000_0001;
    pub const EXTENDED_SPI: u32 = 0x0000_0002;
    pub const SNAP_ARRANGE: u32 = 0x0000_0004;
    pub const TEXT_SCALE: u32 = 0x0000_0008;
    pub const CARET_BLINK: u32 = 0x0000_0010;
    pub const EXTENDED_SPI_2: u32 = 0x0000_0020;
    pub const EXTENDED_SPI_3: u32 = 0x0000_0040;
    const VALID_FLAGS: u32 = 0x0000_007F;
}

/// Client Execute PDU ([MS-RDPERP] 2.2.2.3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutePdu {
    pub flags: u16,
    pub executable: String,
    pub working_directory: String,
    pub arguments: String,
}

impl ExecutePdu {
    pub const EXPAND_WORKING_DIRECTORY: u16 = 0x0001;
    pub const TRANSLATE_FILES: u16 = 0x0002;
    pub const FILE: u16 = 0x0004;
    pub const EXPAND_ARGUMENTS: u16 = 0x0008;
    pub const APP_USER_MODEL_ID: u16 = 0x0010;
    const VALID_FLAGS: u16 = 0x001F;

    fn validate(&self) -> EncodeResult<()> {
        validate_bits(u32::from(self.flags), u32::from(Self::VALID_FLAGS), "Flags")?;
        if self.flags & Self::TRANSLATE_FILES != 0 && self.flags & Self::FILE == 0 {
            return Err(invalid_field_err!("Flags", "TRANSLATE_FILES requires FILE"));
        }
        validate_non_terminated_utf16(&self.executable, "ExeOrFile", 1, MAX_EXECUTABLE_BYTES)?;
        validate_non_terminated_utf16(&self.working_directory, "WorkingDir", 0, MAX_EXECUTABLE_BYTES)?;
        validate_non_terminated_utf16(&self.arguments, "Arguments", 0, MAX_ARGUMENT_BYTES)
    }
}

/// Execute result code ([MS-RDPERP] 2.2.2.3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum ExecuteResult {
    Ok = 0x0000,
    HookNotLoaded = 0x0001,
    DecodeFailed = 0x0002,
    NotInAllowlist = 0x0003,
    FileNotFound = 0x0005,
    Fail = 0x0006,
    SessionLocked = 0x0007,
}

impl TryFrom<u16> for ExecuteResult {
    type Error = u16;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x0000 => Ok(Self::Ok),
            0x0001 => Ok(Self::HookNotLoaded),
            0x0002 => Ok(Self::DecodeFailed),
            0x0003 => Ok(Self::NotInAllowlist),
            0x0005 => Ok(Self::FileNotFound),
            0x0006 => Ok(Self::Fail),
            0x0007 => Ok(Self::SessionLocked),
            _ => Err(value),
        }
    }
}

/// Server Execute Result PDU ([MS-RDPERP] 2.2.2.3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteResultPdu {
    pub flags: u16,
    pub result: ExecuteResult,
    pub raw_result: u32,
    pub executable: String,
}

impl ExecuteResultPdu {
    fn validate(&self) -> EncodeResult<()> {
        validate_bits(u32::from(self.flags), u32::from(ExecutePdu::VALID_FLAGS), "Flags")?;
        if self.flags & ExecutePdu::TRANSLATE_FILES != 0 && self.flags & ExecutePdu::FILE == 0 {
            return Err(invalid_field_err!("Flags", "TRANSLATE_FILES requires FILE"));
        }
        validate_non_terminated_utf16(&self.executable, "ExeOrFile", 1, MAX_EXECUTABLE_BYTES)
    }
}

/// Client System Parameters Update PDU ([MS-RDPERP] 2.2.2.4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSystemParametersPdu {
    pub parameter: ClientSystemParameter,
}

/// A client system-parameter value and its typed body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientSystemParameter {
    FullWindowDrag(bool),
    KeyboardCues(bool),
    KeyboardPreference(bool),
    WorkArea(SystemParameterRectangle),
    DisplayChange(SystemParameterRectangle),
    MouseButtonSwap(bool),
    TaskbarPosition(SystemParameterRectangle),
    HighContrast(HighContrast),
    CaretWidth(u32),
    StickyKeys(StickyKeys),
    ToggleKeys(ToggleKeys),
    FilterKeys(FilterKeys),
    DisplayAnimationsEnabled(bool),
    DisplayAdvancedEffectsEnabled(bool),
    DisplayAutoHideScrollbars(bool),
    DisplayMessageDuration(u32),
    ClosedCaptionFontColor(u8),
    ClosedCaptionFontOpacity(u8),
    ClosedCaptionFontSize(u8),
    ClosedCaptionFontStyle(u8),
    ClosedCaptionFontEdgeEffect(u8),
    ClosedCaptionBackgroundColor(u8),
    ClosedCaptionBackgroundOpacity(u8),
    ClosedCaptionRegionColor(u8),
    ClosedCaptionRegionOpacity(u8),
    AccentColor(AccentColor),
    SystemUsesLightTheme(bool),
    AppsUseLightTheme(bool),
}

impl ClientSystemParameter {
    fn kind(&self) -> u32 {
        match self {
            Self::FullWindowDrag(_) => 0x0000_0025,
            Self::KeyboardCues(_) => 0x0000_100B,
            Self::KeyboardPreference(_) => 0x0000_0045,
            Self::WorkArea(_) => 0x0000_002F,
            Self::DisplayChange(_) => 0x0000_F001,
            Self::MouseButtonSwap(_) => 0x0000_0021,
            Self::TaskbarPosition(_) => 0x0000_F000,
            Self::HighContrast(_) => 0x0000_0043,
            Self::CaretWidth(_) => 0x0000_2007,
            Self::StickyKeys(_) => 0x0000_003B,
            Self::ToggleKeys(_) => 0x0000_0035,
            Self::FilterKeys(_) => 0x0000_0033,
            Self::DisplayAnimationsEnabled(_) => 0x0000_F002,
            Self::DisplayAdvancedEffectsEnabled(_) => 0x0000_F003,
            Self::DisplayAutoHideScrollbars(_) => 0x0000_F004,
            Self::DisplayMessageDuration(_) => 0x0000_F005,
            Self::ClosedCaptionFontColor(_) => 0x0000_F006,
            Self::ClosedCaptionFontOpacity(_) => 0x0000_F007,
            Self::ClosedCaptionFontSize(_) => 0x0000_F008,
            Self::ClosedCaptionFontStyle(_) => 0x0000_F009,
            Self::ClosedCaptionFontEdgeEffect(_) => 0x0000_F00A,
            Self::ClosedCaptionBackgroundColor(_) => 0x0000_F00B,
            Self::ClosedCaptionBackgroundOpacity(_) => 0x0000_F00C,
            Self::ClosedCaptionRegionColor(_) => 0x0000_F00D,
            Self::ClosedCaptionRegionOpacity(_) => 0x0000_F00E,
            Self::AccentColor(_) => 0x0000_F00F,
            Self::SystemUsesLightTheme(_) => 0x0000_F010,
            Self::AppsUseLightTheme(_) => 0x0000_F011,
        }
    }

    fn size(&self) -> usize {
        match self {
            Self::FullWindowDrag(_)
            | Self::KeyboardCues(_)
            | Self::KeyboardPreference(_)
            | Self::MouseButtonSwap(_)
            | Self::DisplayAnimationsEnabled(_)
            | Self::DisplayAdvancedEffectsEnabled(_)
            | Self::DisplayAutoHideScrollbars(_)
            | Self::ClosedCaptionFontColor(_)
            | Self::ClosedCaptionFontOpacity(_)
            | Self::ClosedCaptionFontSize(_)
            | Self::ClosedCaptionFontStyle(_)
            | Self::ClosedCaptionFontEdgeEffect(_)
            | Self::ClosedCaptionBackgroundColor(_)
            | Self::ClosedCaptionBackgroundOpacity(_)
            | Self::ClosedCaptionRegionColor(_)
            | Self::ClosedCaptionRegionOpacity(_) => 1,
            Self::WorkArea(_) | Self::DisplayChange(_) | Self::TaskbarPosition(_) => SystemParameterRectangle::SIZE,
            Self::HighContrast(value) => value.size(),
            Self::CaretWidth(_)
            | Self::StickyKeys(_)
            | Self::ToggleKeys(_)
            | Self::DisplayMessageDuration(_)
            | Self::SystemUsesLightTheme(_)
            | Self::AppsUseLightTheme(_) => 4,
            Self::FilterKeys(_) => FilterKeys::SIZE,
            Self::AccentColor(value) => value.size(),
        }
    }

    fn validate(&self) -> EncodeResult<()> {
        match self {
            Self::HighContrast(value) => value.validate(),
            Self::CaretWidth(value) if *value == 0 => {
                Err(invalid_field_err!("CaretWidth", "caret width must be nonzero"))
            }
            Self::StickyKeys(value) => value.validate(),
            Self::ToggleKeys(value) => value.validate(),
            Self::FilterKeys(value) => value.validate(),
            Self::AccentColor(value) => value.validate(),
            _ => Ok(()),
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) {
        match self {
            Self::FullWindowDrag(value)
            | Self::KeyboardCues(value)
            | Self::KeyboardPreference(value)
            | Self::MouseButtonSwap(value)
            | Self::DisplayAnimationsEnabled(value)
            | Self::DisplayAdvancedEffectsEnabled(value)
            | Self::DisplayAutoHideScrollbars(value) => dst.write_u8(u8::from(*value)),
            Self::WorkArea(value) | Self::DisplayChange(value) | Self::TaskbarPosition(value) => value.encode(dst),
            Self::HighContrast(value) => value.encode(dst),
            Self::CaretWidth(value) | Self::DisplayMessageDuration(value) => dst.write_u32(*value),
            Self::StickyKeys(value) => dst.write_u32(value.flags),
            Self::ToggleKeys(value) => dst.write_u32(value.flags),
            Self::FilterKeys(value) => value.encode(dst),
            Self::ClosedCaptionFontColor(value)
            | Self::ClosedCaptionFontOpacity(value)
            | Self::ClosedCaptionFontSize(value)
            | Self::ClosedCaptionFontStyle(value)
            | Self::ClosedCaptionFontEdgeEffect(value)
            | Self::ClosedCaptionBackgroundColor(value)
            | Self::ClosedCaptionBackgroundOpacity(value)
            | Self::ClosedCaptionRegionColor(value)
            | Self::ClosedCaptionRegionOpacity(value) => dst.write_u8(*value),
            Self::AccentColor(value) => value.encode(dst),
            Self::SystemUsesLightTheme(value) | Self::AppsUseLightTheme(value) => dst.write_u32(u32::from(*value)),
        }
    }
}

/// TS_HIGHCONTRAST ([MS-RDPERP] 2.2.2.4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighContrast {
    pub flags: u32,
    pub color_scheme: String,
}

impl HighContrast {
    const VALID_FLAGS: u32 = 0x0000_001F;

    fn size(&self) -> usize {
        4 + 4 + utf16_bytes(&self.color_scheme).saturating_add(2)
    }

    fn validate(&self) -> EncodeResult<()> {
        validate_bits(self.flags, Self::VALID_FLAGS, "HighContrast.Flags")?;
        validate_null_terminated_utf16(&self.color_scheme, "ColorScheme", MAX_PDU_SIZE - 8)
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) {
        dst.write_u32(self.flags);
        dst.write_u32(narrow_u32(utf16_bytes(&self.color_scheme) + 2));
        write_utf16(dst, &self.color_scheme);
        dst.write_u16(0);
    }
}

/// TS_FILTERKEYS ([MS-RDPERP] 2.2.2.4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilterKeys {
    pub flags: u32,
    pub wait_time: u32,
    pub delay_time: u32,
    pub repeat_time: u32,
    pub bounce_time: u32,
}

impl FilterKeys {
    const SIZE: usize = 20;
    const VALID_FLAGS: u32 = 0x0000_007F;

    fn validate(self) -> EncodeResult<()> {
        validate_bits(self.flags, Self::VALID_FLAGS, "FilterKeys.Flags")
    }

    fn encode(self, dst: &mut WriteCursor<'_>) {
        dst.write_u32(self.flags);
        dst.write_u32(self.wait_time);
        dst.write_u32(self.delay_time);
        dst.write_u32(self.repeat_time);
        dst.write_u32(self.bounce_time);
    }
}

/// TS_TOGGLEKEYS ([MS-RDPERP] 2.2.2.4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToggleKeys {
    pub flags: u32,
}

impl ToggleKeys {
    const VALID_FLAGS: u32 = 0x0000_001F;

    fn validate(self) -> EncodeResult<()> {
        validate_bits(self.flags, Self::VALID_FLAGS, "ToggleKeys.Flags")
    }
}

/// TS_STICKYKEYS ([MS-RDPERP] 2.2.2.4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StickyKeys {
    pub flags: u32,
}

impl StickyKeys {
    const VALID_FLAGS: u32 = 0xFFFF_01FF;

    fn validate(self) -> EncodeResult<()> {
        validate_bits(self.flags, Self::VALID_FLAGS, "StickyKeys.Flags")
    }
}

/// TS_ACCENTCOLOR ([MS-RDPERP] 2.2.2.4.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccentColor {
    pub fields_valid_flags: u32,
    pub accent_color: u32,
    pub colorization_color: u32,
    pub colorization_color_balance: u32,
    pub colorization_afterglow: u32,
    pub colorization_afterglow_balance: u32,
    pub colorization_blur_balance: u32,
    pub colorization_glass_attribute: u32,
    pub color_prevalence: u32,
    pub enable_window_colorization: u32,
    pub accent_color_menu: u32,
    pub start_color_menu: u32,
    /// 32-bit little-endian color entries.
    pub accent_palette: Vec<u32>,
}

impl AccentColor {
    const FIXED_SIZE: usize = 12 * 4;
    const VALID_FLAGS: u32 = 0x0000_0BFF;

    fn size(&self) -> usize {
        Self::FIXED_SIZE + 4 /* AccentPaletteLength */ + self.accent_palette.len().saturating_mul(4)
    }

    fn validate(&self) -> EncodeResult<()> {
        validate_bits(self.fields_valid_flags, Self::VALID_FLAGS, "FieldsValidFlags")?;
        let palette_length = self
            .accent_palette
            .len()
            .checked_mul(4)
            .ok_or_else(|| invalid_field_err!("AccentPaletteLength", "accent palette length overflow"))?;
        if palette_length > MAX_PDU_SIZE - Self::FIXED_SIZE - HEADER_SIZE - 4 {
            return Err(invalid_field_err!("AccentPaletteLength", "accent palette is too large"));
        }
        Ok(())
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) {
        for value in [
            self.fields_valid_flags,
            self.accent_color,
            self.colorization_color,
            self.colorization_color_balance,
            self.colorization_afterglow,
            self.colorization_afterglow_balance,
            self.colorization_blur_balance,
            self.colorization_glass_attribute,
            self.color_prevalence,
            self.enable_window_colorization,
            self.accent_color_menu,
            self.start_color_menu,
        ] {
            dst.write_u32(value);
        }
        dst.write_u32(narrow_u32(palette_byte_length(&self.accent_palette)));
        for color in &self.accent_palette {
            dst.write_u32(*color);
        }
    }
}

/// Server System Parameters Update PDU ([MS-RDPERP] 2.2.2.5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerSystemParametersPdu {
    pub parameter: ServerSystemParameter,
    pub enabled: bool,
}

/// Server-controlled system parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ServerSystemParameter {
    ScreenSaverActive = 0x0000_0011,
    ScreenSaverSecure = 0x0000_0077,
}

impl TryFrom<u32> for ServerSystemParameter {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x0000_0011 => Ok(Self::ScreenSaverActive),
            0x0000_0077 => Ok(Self::ScreenSaverSecure),
            _ => Err(value),
        }
    }
}

/// Client Activate PDU ([MS-RDPERP] 2.2.2.6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivatePdu {
    pub window_id: u32,
    pub enabled: bool,
}

/// Client System Menu PDU ([MS-RDPERP] 2.2.2.6.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemMenuPdu {
    pub window_id: u32,
    pub left: i16,
    pub top: i16,
}

/// System command ([MS-RDPERP] 2.2.2.6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum SystemCommand {
    Size = 0xF000,
    Move = 0xF010,
    Minimize = 0xF020,
    Maximize = 0xF030,
    Close = 0xF060,
    KeyMenu = 0xF100,
    Restore = 0xF120,
    Default = 0xF160,
}

impl TryFrom<u16> for SystemCommand {
    type Error = u16;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0xF000 => Ok(Self::Size),
            0xF010 => Ok(Self::Move),
            0xF020 => Ok(Self::Minimize),
            0xF030 => Ok(Self::Maximize),
            0xF060 => Ok(Self::Close),
            0xF100 => Ok(Self::KeyMenu),
            0xF120 => Ok(Self::Restore),
            0xF160 => Ok(Self::Default),
            _ => Err(value),
        }
    }
}

/// Client System Command PDU ([MS-RDPERP] 2.2.2.6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemCommandPdu {
    pub window_id: u32,
    pub command: SystemCommand,
}

/// Notification icon event message ([MS-RDPERP] 2.2.2.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NotifyEventMessage {
    LeftButtonDown = 0x0000_0201,
    LeftButtonUp = 0x0000_0202,
    LeftButtonDoubleClick = 0x0000_0203,
    RightButtonDown = 0x0000_0204,
    RightButtonUp = 0x0000_0205,
    RightButtonDoubleClick = 0x0000_0206,
    ContextMenu = 0x0000_007B,
    Select = 0x0000_0400,
    KeySelect = 0x0000_0401,
    BalloonShow = 0x0000_0402,
    BalloonHide = 0x0000_0403,
    BalloonTimeout = 0x0000_0404,
    BalloonUserClick = 0x0000_0405,
}

impl TryFrom<u32> for NotifyEventMessage {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x0000_0201 => Ok(Self::LeftButtonDown),
            0x0000_0202 => Ok(Self::LeftButtonUp),
            0x0000_0203 => Ok(Self::LeftButtonDoubleClick),
            0x0000_0204 => Ok(Self::RightButtonDown),
            0x0000_0205 => Ok(Self::RightButtonUp),
            0x0000_0206 => Ok(Self::RightButtonDoubleClick),
            0x0000_007B => Ok(Self::ContextMenu),
            0x0000_0400 => Ok(Self::Select),
            0x0000_0401 => Ok(Self::KeySelect),
            0x0000_0402 => Ok(Self::BalloonShow),
            0x0000_0403 => Ok(Self::BalloonHide),
            0x0000_0404 => Ok(Self::BalloonTimeout),
            0x0000_0405 => Ok(Self::BalloonUserClick),
            _ => Err(value),
        }
    }
}

/// Client Notify Event PDU ([MS-RDPERP] 2.2.2.6.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotifyEventPdu {
    pub window_id: u32,
    pub notify_icon_id: u32,
    pub message: NotifyEventMessage,
}

/// Client Get Application ID PDU ([MS-RDPERP] 2.2.2.6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetApplicationIdRequestPdu {
    pub window_id: u32,
}

/// Server Min Max Info PDU ([MS-RDPERP] 2.2.2.7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinMaxInfoPdu {
    pub window_id: u32,
    /// Max width, max height, max X, max Y, min track width, min track height,
    /// max track width, and max track height.
    pub values: [i16; 8],
}

/// A move/size operation ([MS-RDPERP] 2.2.2.7.2 and 2.2.2.7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum MoveSizeType {
    Left = 0x0001,
    Right = 0x0002,
    Top = 0x0003,
    TopLeft = 0x0004,
    TopRight = 0x0005,
    Bottom = 0x0006,
    BottomLeft = 0x0007,
    BottomRight = 0x0008,
    Move = 0x0009,
    KeyMove = 0x000A,
    KeySize = 0x000B,
}

impl TryFrom<u16> for MoveSizeType {
    type Error = u16;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0x0001 => Ok(Self::Left),
            0x0002 => Ok(Self::Right),
            0x0003 => Ok(Self::Top),
            0x0004 => Ok(Self::TopLeft),
            0x0005 => Ok(Self::TopRight),
            0x0006 => Ok(Self::Bottom),
            0x0007 => Ok(Self::BottomLeft),
            0x0008 => Ok(Self::BottomRight),
            0x0009 => Ok(Self::Move),
            0x000A => Ok(Self::KeyMove),
            0x000B => Ok(Self::KeySize),
            _ => Err(value),
        }
    }
}

/// Server Move/Size Start or End PDU ([MS-RDPERP] 2.2.2.7.2/2.2.2.7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalMoveSizePdu {
    pub window_id: u32,
    /// `true` is a start PDU; `false` is an end PDU.
    pub is_start: bool,
    pub move_size_type: MoveSizeType,
    /// Start: pointer position/offset. End: final top-left X.
    pub x: i16,
    /// Start: pointer position/offset. End: final top-left Y.
    pub y: i16,
}

/// Client Window Move PDU ([MS-RDPERP] 2.2.2.7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowMovePdu {
    pub window_id: u32,
    pub rectangle: Rectangle,
}

/// Client Window Snap PDU ([MS-RDPERP] 2.2.2.7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapArrangePdu {
    pub window_id: u32,
    pub rectangle: Rectangle,
}

/// Server Get Application ID Response PDU ([MS-RDPERP] 2.2.2.8.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetApplicationIdResponsePdu {
    pub window_id: u32,
    pub application_id: String,
}

/// Server Get Application ID Extended Response PDU ([MS-RDPERP] 2.2.2.8.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetApplicationIdResponseExPdu {
    pub window_id: u32,
    pub application_id: String,
    pub process_id: u32,
    pub process_image_name: String,
}

/// Language Bar Information PDU ([MS-RDPERP] 2.2.2.9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageBarInfoPdu {
    pub status: u32,
}

impl LanguageBarInfoPdu {
    const PRIMARY_STATES: u32 = 0x0000_080F;
    const TRANSPARENCY_STATES: u32 = 0x0000_0070;
    const LABEL_STATES: u32 = 0x0000_0180;
    const MINIMIZED_ICON_STATES: u32 = 0x0000_0600;
    const VALID_FLAGS: u32 = 0x0000_0FFF;

    fn validate(self) -> EncodeResult<()> {
        validate_bits(self.status, Self::VALID_FLAGS, "LanguageBarStatus")?;
        if has_multiple_bits(self.status & Self::PRIMARY_STATES) {
            return Err(invalid_field_err!(
                "LanguageBarStatus",
                "language bar primary state flags are mutually exclusive"
            ));
        }
        if has_multiple_bits(self.status & Self::TRANSPARENCY_STATES) {
            return Err(invalid_field_err!(
                "LanguageBarStatus",
                "language bar transparency flags are mutually exclusive"
            ));
        }
        if has_multiple_bits(self.status & Self::LABEL_STATES)
            || has_multiple_bits(self.status & Self::MINIMIZED_ICON_STATES)
        {
            return Err(invalid_field_err!(
                "LanguageBarStatus",
                "language bar display flags are mutually exclusive"
            ));
        }
        Ok(())
    }
}

/// Language profile type ([MS-RDPERP] 2.2.2.10.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum LanguageProfileType {
    InputProcessor = 0x0000_0001,
    KeyboardLayout = 0x0000_0002,
}

impl TryFrom<u32> for LanguageProfileType {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0x0000_0001 => Ok(Self::InputProcessor),
            0x0000_0002 => Ok(Self::KeyboardLayout),
            _ => Err(value),
        }
    }
}

/// Language Profile Information PDU ([MS-RDPERP] 2.2.2.10.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageImeInfoPdu {
    pub profile_type: LanguageProfileType,
    pub language_id: u16,
    pub language_profile_clsid: RailGuid,
    pub profile_guid: RailGuid,
    pub keyboard_layout: u32,
}

impl LanguageImeInfoPdu {
    fn validate(self) -> EncodeResult<()> {
        if self.profile_type == LanguageProfileType::KeyboardLayout
            && (!self.language_profile_clsid.is_null() || !self.profile_guid.is_null())
        {
            return Err(invalid_field_err!(
                "LanguageProfileCLSID",
                "keyboard layouts require null language profile GUIDs"
            ));
        }
        Ok(())
    }
}

/// IME open/closed state ([MS-RDPERP] 2.2.2.10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ImeState {
    Closed = 0,
    Open = 1,
}

impl TryFrom<u32> for ImeState {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Closed),
            1 => Ok(Self::Open),
            _ => Err(value),
        }
    }
}

/// Japanese KANA input state ([MS-RDPERP] 2.2.2.10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum KanaMode {
    Off = 0,
    On = 1,
}

impl TryFrom<u32> for KanaMode {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Off),
            1 => Ok(Self::On),
            _ => Err(value),
        }
    }
}

/// Compartment Status Information PDU ([MS-RDPERP] 2.2.2.10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompartmentInfoPdu {
    pub ime_state: ImeState,
    pub ime_conversion_mode: u32,
    pub ime_sentence_mode: u32,
    pub kana_mode: KanaMode,
}

impl CompartmentInfoPdu {
    const VALID_CONVERSION_MODE: u32 = 0x0000_0FFB;
    const VALID_SENTENCE_MODE: u32 = 0x0000_001F;

    fn validate(self) -> EncodeResult<()> {
        validate_bits(self.ime_conversion_mode, Self::VALID_CONVERSION_MODE, "ImeConvMode")?;
        validate_bits(self.ime_sentence_mode, Self::VALID_SENTENCE_MODE, "ImeSentenceMode")
    }
}

/// Server Z-Order Sync Information PDU ([MS-RDPERP] 2.2.2.11.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZOrderSyncPdu {
    pub window_id_marker: u32,
}

/// Window Cloak State Change PDU ([MS-RDPERP] 2.2.2.12.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloakPdu {
    pub window_id: u32,
    pub cloaked: bool,
}

/// Power Display Request PDU ([MS-RDPERP] 2.2.2.13.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PowerDisplayRequestPdu {
    pub active: bool,
}

/// Taskbar tab operation ([MS-RDPERP] 2.2.2.14.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum TaskbarMessage {
    Register = 1,
    Unregister = 2,
    Order = 3,
    Active = 4,
    Properties = 5,
}

impl TryFrom<u32> for TaskbarMessage {
    type Error = u32;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Register),
            2 => Ok(Self::Unregister),
            3 => Ok(Self::Order),
            4 => Ok(Self::Active),
            5 => Ok(Self::Properties),
            _ => Err(value),
        }
    }
}

/// Taskbar Tab Info PDU ([MS-RDPERP] 2.2.2.14.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskbarInfoPdu {
    pub message: TaskbarMessage,
    pub window_id_tab: u32,
    pub body: u32,
}

/// Text Scale Information PDU ([MS-RDPERP] 2.2.2.15.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextScaleInfoPdu {
    pub text_scale_factor: u32,
}

/// Caret Blink Information PDU ([MS-RDPERP] 2.2.2.15.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaretBlinkInfoPdu {
    pub caret_blink_rate: u32,
}

impl_wire_value!(RailOrderType => u16 {
    RailOrderType::Execute => 0x0001,
    RailOrderType::Activate => 0x0002,
    RailOrderType::SystemParameters => 0x0003,
    RailOrderType::SystemCommand => 0x0004,
    RailOrderType::Handshake => 0x0005,
    RailOrderType::NotifyEvent => 0x0006,
    RailOrderType::WindowMove => 0x0008,
    RailOrderType::LocalMoveSize => 0x0009,
    RailOrderType::MinMaxInfo => 0x000A,
    RailOrderType::ClientStatus => 0x000B,
    RailOrderType::SystemMenu => 0x000C,
    RailOrderType::LanguageBarInfo => 0x000D,
    RailOrderType::GetApplicationIdRequest => 0x000E,
    RailOrderType::GetApplicationIdResponse => 0x000F,
    RailOrderType::TaskbarInfo => 0x0010,
    RailOrderType::LanguageImeInfo => 0x0011,
    RailOrderType::CompartmentInfo => 0x0012,
    RailOrderType::HandshakeEx => 0x0013,
    RailOrderType::ZOrderSync => 0x0014,
    RailOrderType::Cloak => 0x0015,
    RailOrderType::PowerDisplayRequest => 0x0016,
    RailOrderType::SnapArrange => 0x0017,
    RailOrderType::GetApplicationIdResponseEx => 0x0018,
    RailOrderType::TextScaleInfo => 0x0019,
    RailOrderType::CaretBlinkInfo => 0x001A,
    RailOrderType::ExecuteResult => 0x0080,
});

impl_wire_value!(ExecuteResult => u16 {
    ExecuteResult::Ok => 0x0000,
    ExecuteResult::HookNotLoaded => 0x0001,
    ExecuteResult::DecodeFailed => 0x0002,
    ExecuteResult::NotInAllowlist => 0x0003,
    ExecuteResult::FileNotFound => 0x0005,
    ExecuteResult::Fail => 0x0006,
    ExecuteResult::SessionLocked => 0x0007,
});

impl_wire_value!(ServerSystemParameter => u32 {
    ServerSystemParameter::ScreenSaverActive => 0x0000_0011,
    ServerSystemParameter::ScreenSaverSecure => 0x0000_0077,
});

impl_wire_value!(SystemCommand => u16 {
    SystemCommand::Size => 0xF000,
    SystemCommand::Move => 0xF010,
    SystemCommand::Minimize => 0xF020,
    SystemCommand::Maximize => 0xF030,
    SystemCommand::Close => 0xF060,
    SystemCommand::KeyMenu => 0xF100,
    SystemCommand::Restore => 0xF120,
    SystemCommand::Default => 0xF160,
});

impl_wire_value!(NotifyEventMessage => u32 {
    NotifyEventMessage::LeftButtonDown => 0x0000_0201,
    NotifyEventMessage::LeftButtonUp => 0x0000_0202,
    NotifyEventMessage::LeftButtonDoubleClick => 0x0000_0203,
    NotifyEventMessage::RightButtonDown => 0x0000_0204,
    NotifyEventMessage::RightButtonUp => 0x0000_0205,
    NotifyEventMessage::RightButtonDoubleClick => 0x0000_0206,
    NotifyEventMessage::ContextMenu => 0x0000_007B,
    NotifyEventMessage::Select => 0x0000_0400,
    NotifyEventMessage::KeySelect => 0x0000_0401,
    NotifyEventMessage::BalloonShow => 0x0000_0402,
    NotifyEventMessage::BalloonHide => 0x0000_0403,
    NotifyEventMessage::BalloonTimeout => 0x0000_0404,
    NotifyEventMessage::BalloonUserClick => 0x0000_0405,
});

impl_wire_value!(MoveSizeType => u16 {
    MoveSizeType::Left => 0x0001,
    MoveSizeType::Right => 0x0002,
    MoveSizeType::Top => 0x0003,
    MoveSizeType::TopLeft => 0x0004,
    MoveSizeType::TopRight => 0x0005,
    MoveSizeType::Bottom => 0x0006,
    MoveSizeType::BottomLeft => 0x0007,
    MoveSizeType::BottomRight => 0x0008,
    MoveSizeType::Move => 0x0009,
    MoveSizeType::KeyMove => 0x000A,
    MoveSizeType::KeySize => 0x000B,
});

impl_wire_value!(LanguageProfileType => u32 {
    LanguageProfileType::InputProcessor => 0x0000_0001,
    LanguageProfileType::KeyboardLayout => 0x0000_0002,
});

impl_wire_value!(ImeState => u32 {
    ImeState::Closed => 0,
    ImeState::Open => 1,
});

impl_wire_value!(KanaMode => u32 {
    KanaMode::Off => 0,
    KanaMode::On => 1,
});

impl_wire_value!(TaskbarMessage => u32 {
    TaskbarMessage::Register => 1,
    TaskbarMessage::Unregister => 2,
    TaskbarMessage::Order => 3,
    TaskbarMessage::Active => 4,
    TaskbarMessage::Properties => 5,
});

fn pdu_length(body_size: usize) -> EncodeResult<u16> {
    let total = HEADER_SIZE
        .checked_add(body_size)
        .ok_or_else(|| invalid_field_err!("orderLength", "RAIL PDU length overflow"))?;
    u16::try_from(total).map_err(|_| invalid_field_err!("orderLength", "RAIL PDU is larger than 65535 bytes"))
}

fn utf16_bytes(value: &str) -> usize {
    value.encode_utf16().count().saturating_mul(2)
}

fn narrow_u16(value: usize) -> u16 {
    u16::try_from(value).expect("RAIL PDU validation bounds UTF-16 lengths to u16")
}

fn narrow_u32(value: usize) -> u32 {
    u32::try_from(value).expect("RAIL PDU validation bounds variable lengths to u32")
}

fn palette_byte_length(palette: &[u32]) -> usize {
    palette
        .len()
        .checked_mul(4)
        .expect("RAIL PDU validation bounds accent palette byte length")
}

fn validate_bits(value: u32, valid: u32, field: &'static str) -> EncodeResult<()> {
    if value & !valid != 0 {
        return Err(invalid_field_err!(field, "contains undefined flag bits"));
    }
    Ok(())
}

fn validate_non_terminated_utf16(
    value: &str,
    field: &'static str,
    minimum_length: usize,
    maximum_length: usize,
) -> EncodeResult<()> {
    if value.contains('\0') {
        return Err(invalid_field_err!(field, "must not contain a null terminator"));
    }
    let length = utf16_bytes(value);
    if length < minimum_length || length > maximum_length {
        return Err(invalid_field_err!(
            field,
            "UTF-16 byte length is outside the permitted range"
        ));
    }
    if !length.is_multiple_of(2) {
        return Err(invalid_field_err!(field, "UTF-16 byte length is not even"));
    }
    Ok(())
}

fn validate_null_terminated_utf16(value: &str, field: &'static str, maximum_length: usize) -> EncodeResult<()> {
    validate_non_terminated_utf16(value, field, 0, maximum_length.saturating_sub(2))?;
    Ok(())
}

fn validate_fixed_utf16(value: &str, field: &'static str) -> EncodeResult<()> {
    validate_non_terminated_utf16(value, field, 0, FIXED_APPLICATION_ID_BYTES - 2)
}

fn write_utf16(dst: &mut WriteCursor<'_>, value: &str) {
    for character in value.encode_utf16() {
        dst.write_u16(character);
    }
}

fn write_fixed_utf16(dst: &mut WriteCursor<'_>, value: &str) {
    let used = utf16_bytes(value) + 2;
    write_utf16(dst, value);
    dst.write_u16(0);
    for _ in used..FIXED_APPLICATION_ID_BYTES {
        dst.write_u8(0);
    }
}

fn expect_exact(src: &ReadCursor<'_>, expected: usize, name: &'static str) -> DecodeResult<()> {
    if src.len() != expected {
        return Err(invalid_field_err!(name, "unexpected body length"));
    }
    Ok(())
}

fn validate_bits_decode(value: u32, valid: u32, field: &'static str) -> DecodeResult<()> {
    if value & !valid != 0 {
        return Err(invalid_field_err!(field, "contains undefined flag bits"));
    }
    Ok(())
}

fn decode_utf16(src: &mut ReadCursor<'_>, byte_length: usize, field: &'static str) -> DecodeResult<String> {
    if !byte_length.is_multiple_of(2) {
        return Err(invalid_field_err!(field, "UTF-16 byte length is not even"));
    }
    ensure_remaining!(src.len(), byte_length, "UTF-16 string");
    let bytes = src.read_slice(byte_length);
    let values = bytes
        .chunks_exact(2)
        .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&values).map_err(|_| invalid_field_err!(field, "contains invalid UTF-16"))
}

fn decode_non_terminated_utf16(
    src: &mut ReadCursor<'_>,
    byte_length: usize,
    field: &'static str,
    minimum_length: usize,
    maximum_length: usize,
) -> DecodeResult<String> {
    if byte_length < minimum_length || byte_length > maximum_length {
        return Err(invalid_field_err!(
            field,
            "UTF-16 byte length is outside the permitted range"
        ));
    }
    let value = decode_utf16(src, byte_length, field)?;
    if value.contains('\0') {
        return Err(invalid_field_err!(field, "must not contain a null terminator"));
    }
    Ok(value)
}

fn decode_null_terminated_utf16(
    src: &mut ReadCursor<'_>,
    byte_length: usize,
    field: &'static str,
) -> DecodeResult<String> {
    if byte_length < 2 {
        return Err(invalid_field_err!(field, "null-terminated string is empty"));
    }
    let value = decode_utf16(src, byte_length, field)?;
    let value = value
        .strip_suffix('\0')
        .ok_or_else(|| invalid_field_err!(field, "string is not null terminated"))?;
    if value.contains('\0') {
        return Err(invalid_field_err!(field, "string contains an embedded null"));
    }
    Ok(value.to_owned())
}

fn decode_fixed_utf16(src: &mut ReadCursor<'_>, field: &'static str) -> DecodeResult<String> {
    ensure_remaining!(src.len(), FIXED_APPLICATION_ID_BYTES, "fixed UTF-16 string");
    let bytes = src.read_slice(FIXED_APPLICATION_ID_BYTES);
    let values = bytes
        .chunks_exact(2)
        .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
        .collect::<Vec<_>>();
    let terminator = values
        .iter()
        .position(|value| *value == 0)
        .ok_or_else(|| invalid_field_err!(field, "fixed UTF-16 string is not null terminated"))?;
    String::from_utf16(&values[..terminator]).map_err(|_| invalid_field_err!(field, "contains invalid UTF-16"))
}

fn decode_bool_u8(src: &mut ReadCursor<'_>) -> bool {
    src.read_u8() != 0
}

fn decode_strict_bool_u8(src: &mut ReadCursor<'_>, field: &'static str) -> DecodeResult<bool> {
    match src.read_u8() {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid_field_err!(field, "must be zero or one")),
    }
}

fn decode_strict_bool_u32(src: &mut ReadCursor<'_>, field: &'static str) -> DecodeResult<bool> {
    match src.read_u32() {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid_field_err!(field, "must be zero or one")),
    }
}

fn decode_rectangle(src: &mut ReadCursor<'_>) -> Rectangle {
    Rectangle::decode(src)
}

fn decode_system_parameter_rectangle(src: &mut ReadCursor<'_>) -> SystemParameterRectangle {
    SystemParameterRectangle::decode(src)
}

fn decode_client_system_parameter(kind: u32, src: &mut ReadCursor<'_>) -> DecodeResult<ClientSystemParameter> {
    let parameter = match kind {
        0x0000_0025 => {
            expect_exact(src, 1, "SPI_SETDRAGFULLWINDOWS")?;
            ClientSystemParameter::FullWindowDrag(decode_bool_u8(src))
        }
        0x0000_100B => {
            expect_exact(src, 1, "SPI_SETKEYBOARDCUES")?;
            ClientSystemParameter::KeyboardCues(decode_bool_u8(src))
        }
        0x0000_0045 => {
            expect_exact(src, 1, "SPI_SETKEYBOARDPREF")?;
            ClientSystemParameter::KeyboardPreference(decode_bool_u8(src))
        }
        0x0000_002F => {
            expect_exact(src, SystemParameterRectangle::SIZE, "SPI_SETWORKAREA")?;
            ClientSystemParameter::WorkArea(decode_system_parameter_rectangle(src))
        }
        0x0000_F001 => {
            expect_exact(src, SystemParameterRectangle::SIZE, "RAIL_SPI_DISPLAYCHANGE")?;
            ClientSystemParameter::DisplayChange(decode_system_parameter_rectangle(src))
        }
        0x0000_0021 => {
            expect_exact(src, 1, "SPI_SETMOUSEBUTTONSWAP")?;
            ClientSystemParameter::MouseButtonSwap(decode_bool_u8(src))
        }
        0x0000_F000 => {
            expect_exact(src, SystemParameterRectangle::SIZE, "RAIL_SPI_TASKBARPOS")?;
            ClientSystemParameter::TaskbarPosition(decode_system_parameter_rectangle(src))
        }
        0x0000_0043 => ClientSystemParameter::HighContrast(decode_high_contrast(src)?),
        0x0000_2007 => {
            expect_exact(src, 4, "SPI_SETCARETWIDTH")?;
            let width = src.read_u32();
            if width == 0 {
                return Err(invalid_field_err!("CaretWidth", "caret width must be nonzero"));
            }
            ClientSystemParameter::CaretWidth(width)
        }
        0x0000_003B => {
            expect_exact(src, 4, "SPI_SETSTICKYKEYS")?;
            let flags = src.read_u32();
            validate_bits_decode(flags, StickyKeys::VALID_FLAGS, "StickyKeys.Flags")?;
            ClientSystemParameter::StickyKeys(StickyKeys { flags })
        }
        0x0000_0035 => {
            expect_exact(src, 4, "SPI_SETTOGGLEKEYS")?;
            let flags = src.read_u32();
            validate_bits_decode(flags, ToggleKeys::VALID_FLAGS, "ToggleKeys.Flags")?;
            ClientSystemParameter::ToggleKeys(ToggleKeys { flags })
        }
        0x0000_0033 => ClientSystemParameter::FilterKeys(decode_filter_keys(src)?),
        0x0000_F002 => {
            expect_exact(src, 1, "RAIL_SPI_DISPLAY_ANIMATIONS_ENABLED")?;
            ClientSystemParameter::DisplayAnimationsEnabled(decode_bool_u8(src))
        }
        0x0000_F003 => {
            expect_exact(src, 1, "RAIL_SPI_DISPLAY_ADVANCED_EFFECTS_ENABLED")?;
            ClientSystemParameter::DisplayAdvancedEffectsEnabled(decode_bool_u8(src))
        }
        0x0000_F004 => {
            expect_exact(src, 1, "RAIL_SPI_DISPLAY_AUTO_HIDE_SCROLLBARS")?;
            ClientSystemParameter::DisplayAutoHideScrollbars(decode_bool_u8(src))
        }
        0x0000_F005 => {
            expect_exact(src, 4, "RAIL_SPI_DISPLAY_MESSAGE_DURATION")?;
            ClientSystemParameter::DisplayMessageDuration(src.read_u32())
        }
        0x0000_F006 => decode_caption_byte(src, kind, ClientSystemParameter::ClosedCaptionFontColor)?,
        0x0000_F007 => decode_caption_byte(src, kind, ClientSystemParameter::ClosedCaptionFontOpacity)?,
        0x0000_F008 => decode_caption_byte(src, kind, ClientSystemParameter::ClosedCaptionFontSize)?,
        0x0000_F009 => decode_caption_byte(src, kind, ClientSystemParameter::ClosedCaptionFontStyle)?,
        0x0000_F00A => decode_caption_byte(src, kind, ClientSystemParameter::ClosedCaptionFontEdgeEffect)?,
        0x0000_F00B => decode_caption_byte(src, kind, ClientSystemParameter::ClosedCaptionBackgroundColor)?,
        0x0000_F00C => decode_caption_byte(src, kind, ClientSystemParameter::ClosedCaptionBackgroundOpacity)?,
        0x0000_F00D => decode_caption_byte(src, kind, ClientSystemParameter::ClosedCaptionRegionColor)?,
        0x0000_F00E => decode_caption_byte(src, kind, ClientSystemParameter::ClosedCaptionRegionOpacity)?,
        0x0000_F00F => ClientSystemParameter::AccentColor(decode_accent_color(src)?),
        0x0000_F010 => {
            expect_exact(src, 4, "RAIL_SPI_SYSTEM_USES_LIGHT_THEME")?;
            ClientSystemParameter::SystemUsesLightTheme(decode_strict_bool_u32(src, "SystemUsesLightTheme")?)
        }
        0x0000_F011 => {
            expect_exact(src, 4, "RAIL_SPI_APPS_USE_LIGHT_THEME")?;
            ClientSystemParameter::AppsUseLightTheme(decode_strict_bool_u32(src, "AppsUseLightTheme")?)
        }
        _ => return Err(invalid_field_err!("SystemParam", "unknown client system parameter")),
    };
    Ok(parameter)
}

fn decode_caption_byte(
    src: &mut ReadCursor<'_>,
    _kind: u32,
    value: fn(u8) -> ClientSystemParameter,
) -> DecodeResult<ClientSystemParameter> {
    expect_exact(src, 1, "closed caption parameter")?;
    Ok(value(src.read_u8()))
}

fn decode_high_contrast(src: &mut ReadCursor<'_>) -> DecodeResult<HighContrast> {
    ensure_remaining!(src.len(), 8, "TS_HIGHCONTRAST");
    let flags = src.read_u32();
    validate_bits_decode(flags, HighContrast::VALID_FLAGS, "HighContrast.Flags")?;
    let color_scheme_length = usize::try_from(src.read_u32())
        .map_err(|_| invalid_field_err!("ColorSchemeLength", "does not fit in memory"))?;
    if color_scheme_length != src.len() {
        return Err(invalid_field_err!(
            "ColorSchemeLength",
            "does not match the remaining body size"
        ));
    }
    let color_scheme = decode_null_terminated_utf16(src, color_scheme_length, "ColorScheme")?;
    Ok(HighContrast { flags, color_scheme })
}

fn decode_filter_keys(src: &mut ReadCursor<'_>) -> DecodeResult<FilterKeys> {
    expect_exact(src, FilterKeys::SIZE, "TS_FILTERKEYS")?;
    let flags = src.read_u32();
    validate_bits_decode(flags, FilterKeys::VALID_FLAGS, "FilterKeys.Flags")?;
    Ok(FilterKeys {
        flags,
        wait_time: src.read_u32(),
        delay_time: src.read_u32(),
        repeat_time: src.read_u32(),
        bounce_time: src.read_u32(),
    })
}

fn decode_accent_color(src: &mut ReadCursor<'_>) -> DecodeResult<AccentColor> {
    ensure_remaining!(src.len(), AccentColor::FIXED_SIZE + 4, "TS_ACCENTCOLOR");
    let fields_valid_flags = src.read_u32();
    validate_bits_decode(fields_valid_flags, AccentColor::VALID_FLAGS, "FieldsValidFlags")?;
    let accent_color = src.read_u32();
    let colorization_color = src.read_u32();
    let colorization_color_balance = src.read_u32();
    let colorization_afterglow = src.read_u32();
    let colorization_afterglow_balance = src.read_u32();
    let colorization_blur_balance = src.read_u32();
    let colorization_glass_attribute = src.read_u32();
    let color_prevalence = src.read_u32();
    let enable_window_colorization = src.read_u32();
    let accent_color_menu = src.read_u32();
    let start_color_menu = src.read_u32();
    let palette_length = usize::try_from(src.read_u32())
        .map_err(|_| invalid_field_err!("AccentPaletteLength", "does not fit in memory"))?;
    if palette_length != src.len() || !palette_length.is_multiple_of(4) {
        return Err(invalid_field_err!(
            "AccentPaletteLength",
            "must match the remaining body size and be a multiple of four"
        ));
    }
    let mut accent_palette = Vec::with_capacity(palette_length / 4);
    while !src.is_empty() {
        accent_palette.push(src.read_u32());
    }
    Ok(AccentColor {
        fields_valid_flags,
        accent_color,
        colorization_color,
        colorization_color_balance,
        colorization_afterglow,
        colorization_afterglow_balance,
        colorization_blur_balance,
        colorization_glass_attribute,
        color_prevalence,
        enable_window_colorization,
        accent_color_menu,
        start_color_menu,
        accent_palette,
    })
}

fn decode_body(kind: RailOrderType, src: &mut ReadCursor<'_>) -> DecodeResult<RailPdu> {
    let pdu = match kind {
        RailOrderType::Handshake => {
            expect_exact(src, 4, "TS_RAIL_ORDER_HANDSHAKE")?;
            RailPdu::Handshake(HandshakePdu {
                build_number: src.read_u32(),
            })
        }
        RailOrderType::ClientStatus => {
            expect_exact(src, 4, "TS_RAIL_ORDER_CLIENTSTATUS")?;
            let flags = src.read_u32();
            validate_bits_decode(flags, ClientStatusPdu::VALID_FLAGS, "Flags")?;
            RailPdu::ClientStatus(ClientStatusPdu { flags })
        }
        RailOrderType::HandshakeEx => {
            expect_exact(src, 8, "TS_RAIL_ORDER_HANDSHAKE_EX")?;
            let build_number = src.read_u32();
            let flags = src.read_u32();
            validate_bits_decode(flags, HandshakeExPdu::VALID_FLAGS, "railHandshakeFlags")?;
            RailPdu::HandshakeEx(HandshakeExPdu { build_number, flags })
        }
        RailOrderType::Execute => RailPdu::Execute(decode_execute(src)?),
        RailOrderType::ExecuteResult => RailPdu::ExecuteResult(decode_execute_result(src)?),
        RailOrderType::SystemParameters => decode_system_parameters(src)?,
        RailOrderType::Activate => {
            expect_exact(src, 5, "TS_RAIL_ORDER_ACTIVATE")?;
            RailPdu::Activate(ActivatePdu {
                window_id: src.read_u32(),
                enabled: decode_bool_u8(src),
            })
        }
        RailOrderType::SystemMenu => {
            expect_exact(src, 8, "TS_RAIL_ORDER_SYSMENU")?;
            RailPdu::SystemMenu(SystemMenuPdu {
                window_id: src.read_u32(),
                left: src.read_i16(),
                top: src.read_i16(),
            })
        }
        RailOrderType::SystemCommand => {
            expect_exact(src, 6, "TS_RAIL_ORDER_SYSCOMMAND")?;
            let window_id = src.read_u32();
            let command = SystemCommand::try_from(src.read_u16())
                .map_err(|_| invalid_field_err!("Command", "unknown system command"))?;
            RailPdu::SystemCommand(SystemCommandPdu { window_id, command })
        }
        RailOrderType::NotifyEvent => {
            expect_exact(src, 12, "TS_RAIL_ORDER_NOTIFY_EVENT")?;
            let window_id = src.read_u32();
            let notify_icon_id = src.read_u32();
            let message = NotifyEventMessage::try_from(src.read_u32())
                .map_err(|_| invalid_field_err!("Message", "unknown notification event"))?;
            RailPdu::NotifyEvent(NotifyEventPdu {
                window_id,
                notify_icon_id,
                message,
            })
        }
        RailOrderType::GetApplicationIdRequest => {
            expect_exact(src, 4, "TS_RAIL_ORDER_GET_APPID_REQ")?;
            RailPdu::GetApplicationIdRequest(GetApplicationIdRequestPdu {
                window_id: src.read_u32(),
            })
        }
        RailOrderType::MinMaxInfo => {
            expect_exact(src, 20, "TS_RAIL_ORDER_MINMAXINFO")?;
            let window_id = src.read_u32();
            let mut values = [0; 8];
            for value in &mut values {
                *value = src.read_i16();
            }
            RailPdu::MinMaxInfo(MinMaxInfoPdu { window_id, values })
        }
        RailOrderType::LocalMoveSize => {
            expect_exact(src, 12, "TS_RAIL_ORDER_LOCALMOVESIZE")?;
            let window_id = src.read_u32();
            let is_start = src.read_u16() != 0;
            let move_size_type = MoveSizeType::try_from(src.read_u16())
                .map_err(|_| invalid_field_err!("MoveSizeType", "unknown move/size type"))?;
            let x = src.read_i16();
            let y = src.read_i16();
            RailPdu::LocalMoveSize(LocalMoveSizePdu {
                window_id,
                is_start,
                move_size_type,
                x,
                y,
            })
        }
        RailOrderType::WindowMove => RailPdu::WindowMove(decode_window_move(src)?),
        RailOrderType::SnapArrange => {
            let pdu = decode_window_move(src)?;
            RailPdu::SnapArrange(SnapArrangePdu {
                window_id: pdu.window_id,
                rectangle: pdu.rectangle,
            })
        }
        RailOrderType::GetApplicationIdResponse => {
            expect_exact(src, 4 + FIXED_APPLICATION_ID_BYTES, "TS_RAIL_ORDER_GET_APPID_RESP")?;
            RailPdu::GetApplicationIdResponse(GetApplicationIdResponsePdu {
                window_id: src.read_u32(),
                application_id: decode_fixed_utf16(src, "ApplicationId")?,
            })
        }
        RailOrderType::GetApplicationIdResponseEx => {
            expect_exact(
                src,
                4 + FIXED_APPLICATION_ID_BYTES + 4 + FIXED_APPLICATION_ID_BYTES,
                "TS_RAIL_ORDER_GET_APPID_RESP_EX",
            )?;
            let window_id = src.read_u32();
            let application_id = decode_fixed_utf16(src, "ApplicationId")?;
            let process_id = src.read_u32();
            let process_image_name = decode_fixed_utf16(src, "ProcessImageName")?;
            RailPdu::GetApplicationIdResponseEx(GetApplicationIdResponseExPdu {
                window_id,
                application_id,
                process_id,
                process_image_name,
            })
        }
        RailOrderType::LanguageBarInfo => {
            expect_exact(src, 4, "TS_RAIL_ORDER_LANGBARINFO")?;
            let status = src.read_u32();
            validate_language_bar_status(status)?;
            RailPdu::LanguageBarInfo(LanguageBarInfoPdu { status })
        }
        RailOrderType::LanguageImeInfo => {
            expect_exact(src, 42, "TS_RAIL_ORDER_LANGUAGEIMEINFO")?;
            let profile_type = LanguageProfileType::try_from(src.read_u32())
                .map_err(|_| invalid_field_err!("ProfileType", "unknown language profile type"))?;
            let language_id = src.read_u16();
            let language_profile_clsid = RailGuid::decode(src);
            let profile_guid = RailGuid::decode(src);
            let keyboard_layout = src.read_u32();
            let pdu = LanguageImeInfoPdu {
                profile_type,
                language_id,
                language_profile_clsid,
                profile_guid,
                keyboard_layout,
            };
            validate_language_ime(pdu)?;
            RailPdu::LanguageImeInfo(pdu)
        }
        RailOrderType::CompartmentInfo => {
            expect_exact(src, 16, "TS_RAIL_ORDER_COMPARTMENTINFO")?;
            let ime_state = ImeState::try_from(src.read_u32())
                .map_err(|_| invalid_field_err!("ImeState", "must be zero or one"))?;
            let ime_conversion_mode = src.read_u32();
            let ime_sentence_mode = src.read_u32();
            let kana_mode = KanaMode::try_from(src.read_u32())
                .map_err(|_| invalid_field_err!("KANAMode", "must be zero or one"))?;
            validate_bits_decode(
                ime_conversion_mode,
                CompartmentInfoPdu::VALID_CONVERSION_MODE,
                "ImeConvMode",
            )?;
            validate_bits_decode(
                ime_sentence_mode,
                CompartmentInfoPdu::VALID_SENTENCE_MODE,
                "ImeSentenceMode",
            )?;
            RailPdu::CompartmentInfo(CompartmentInfoPdu {
                ime_state,
                ime_conversion_mode,
                ime_sentence_mode,
                kana_mode,
            })
        }
        RailOrderType::ZOrderSync => {
            expect_exact(src, 4, "TS_RAIL_ORDER_ZORDER_SYNC")?;
            RailPdu::ZOrderSync(ZOrderSyncPdu {
                window_id_marker: src.read_u32(),
            })
        }
        RailOrderType::Cloak => {
            expect_exact(src, 5, "TS_RAIL_ORDER_CLOAK")?;
            let window_id = src.read_u32();
            let cloaked = decode_strict_bool_u8(src, "Cloaked")?;
            RailPdu::Cloak(CloakPdu { window_id, cloaked })
        }
        RailOrderType::PowerDisplayRequest => {
            expect_exact(src, 4, "TS_RAIL_ORDER_POWER_DISPLAY_REQUEST")?;
            RailPdu::PowerDisplayRequest(PowerDisplayRequestPdu {
                active: decode_strict_bool_u32(src, "Active")?,
            })
        }
        RailOrderType::TaskbarInfo => {
            expect_exact(src, 12, "TS_RAIL_ORDER_TASKBARINFO")?;
            let message = TaskbarMessage::try_from(src.read_u32())
                .map_err(|_| invalid_field_err!("TaskbarMessage", "unknown taskbar message"))?;
            let window_id_tab = src.read_u32();
            let body = src.read_u32();
            RailPdu::TaskbarInfo(TaskbarInfoPdu {
                message,
                window_id_tab,
                body,
            })
        }
        RailOrderType::TextScaleInfo => {
            expect_exact(src, 4, "TS_RAIL_ORDER_TEXTSCALEINFO")?;
            let text_scale_factor = src.read_u32();
            if !(100..=225).contains(&text_scale_factor) {
                return Err(invalid_field_err!("TextScaleFactor", "must be between 100 and 225"));
            }
            RailPdu::TextScaleInfo(TextScaleInfoPdu { text_scale_factor })
        }
        RailOrderType::CaretBlinkInfo => {
            expect_exact(src, 4, "TS_RAIL_ORDER_CARETBLINKINFO")?;
            RailPdu::CaretBlinkInfo(CaretBlinkInfoPdu {
                caret_blink_rate: src.read_u32(),
            })
        }
    };
    Ok(pdu)
}

fn decode_execute(src: &mut ReadCursor<'_>) -> DecodeResult<ExecutePdu> {
    ensure_remaining!(src.len(), 8, "TS_RAIL_ORDER_EXEC");
    let flags = src.read_u16();
    validate_bits_decode(u32::from(flags), u32::from(ExecutePdu::VALID_FLAGS), "Flags")?;
    if flags & ExecutePdu::TRANSLATE_FILES != 0 && flags & ExecutePdu::FILE == 0 {
        return Err(invalid_field_err!("Flags", "TRANSLATE_FILES requires FILE"));
    }
    let executable_length = usize::from(src.read_u16());
    let working_directory_length = usize::from(src.read_u16());
    let arguments_length = usize::from(src.read_u16());
    let total_length = executable_length
        .checked_add(working_directory_length)
        .and_then(|length| length.checked_add(arguments_length))
        .ok_or_else(|| invalid_field_err!("string lengths", "overflow"))?;
    if total_length != src.len() {
        return Err(invalid_field_err!("string lengths", "do not match the PDU body length"));
    }
    let executable = decode_non_terminated_utf16(src, executable_length, "ExeOrFile", 1, MAX_EXECUTABLE_BYTES)?;
    let working_directory =
        decode_non_terminated_utf16(src, working_directory_length, "WorkingDir", 0, MAX_EXECUTABLE_BYTES)?;
    let arguments = decode_non_terminated_utf16(src, arguments_length, "Arguments", 0, MAX_ARGUMENT_BYTES)?;
    Ok(ExecutePdu {
        flags,
        executable,
        working_directory,
        arguments,
    })
}

fn decode_execute_result(src: &mut ReadCursor<'_>) -> DecodeResult<ExecuteResultPdu> {
    ensure_remaining!(src.len(), 12, "TS_RAIL_ORDER_EXEC_RESULT");
    let flags = src.read_u16();
    validate_bits_decode(u32::from(flags), u32::from(ExecutePdu::VALID_FLAGS), "Flags")?;
    if flags & ExecutePdu::TRANSLATE_FILES != 0 && flags & ExecutePdu::FILE == 0 {
        return Err(invalid_field_err!("Flags", "TRANSLATE_FILES requires FILE"));
    }
    let result = ExecuteResult::try_from(src.read_u16())
        .map_err(|_| invalid_field_err!("ExecResult", "unknown execute result"))?;
    let raw_result = src.read_u32();
    let _padding = src.read_u16();
    let executable_length = usize::from(src.read_u16());
    if executable_length != src.len() {
        return Err(invalid_field_err!(
            "ExeOrFileLength",
            "does not match the remaining PDU body"
        ));
    }
    let executable = decode_non_terminated_utf16(src, executable_length, "ExeOrFile", 1, MAX_EXECUTABLE_BYTES)?;
    Ok(ExecuteResultPdu {
        flags,
        result,
        raw_result,
        executable,
    })
}

fn decode_system_parameters(src: &mut ReadCursor<'_>) -> DecodeResult<RailPdu> {
    ensure_remaining!(src.len(), 4, "TS_RAIL_ORDER_SYSPARAM");
    let kind = src.read_u32();
    if let Ok(parameter) = ServerSystemParameter::try_from(kind) {
        expect_exact(src, 1, "server system parameter")?;
        return Ok(RailPdu::ServerSystemParameters(ServerSystemParametersPdu {
            parameter,
            enabled: decode_bool_u8(src),
        }));
    }

    Ok(RailPdu::ClientSystemParameters(ClientSystemParametersPdu {
        parameter: decode_client_system_parameter(kind, src)?,
    }))
}

fn decode_window_move(src: &mut ReadCursor<'_>) -> DecodeResult<WindowMovePdu> {
    expect_exact(src, 12, "window move body")?;
    Ok(WindowMovePdu {
        window_id: src.read_u32(),
        rectangle: decode_rectangle(src),
    })
}

fn validate_language_bar_status(status: u32) -> DecodeResult<()> {
    validate_bits_decode(status, LanguageBarInfoPdu::VALID_FLAGS, "LanguageBarStatus")?;
    if has_multiple_bits(status & LanguageBarInfoPdu::PRIMARY_STATES) {
        return Err(invalid_field_err!(
            "LanguageBarStatus",
            "language bar primary state flags are mutually exclusive"
        ));
    }
    if has_multiple_bits(status & LanguageBarInfoPdu::TRANSPARENCY_STATES) {
        return Err(invalid_field_err!(
            "LanguageBarStatus",
            "language bar transparency flags are mutually exclusive"
        ));
    }
    if has_multiple_bits(status & LanguageBarInfoPdu::LABEL_STATES)
        || has_multiple_bits(status & LanguageBarInfoPdu::MINIMIZED_ICON_STATES)
    {
        return Err(invalid_field_err!(
            "LanguageBarStatus",
            "language bar display flags are mutually exclusive"
        ));
    }
    Ok(())
}

fn validate_language_ime(pdu: LanguageImeInfoPdu) -> DecodeResult<()> {
    if pdu.profile_type == LanguageProfileType::KeyboardLayout
        && (!pdu.language_profile_clsid.is_null() || !pdu.profile_guid.is_null())
    {
        return Err(invalid_field_err!(
            "LanguageProfileCLSID",
            "keyboard layouts require null language profile GUIDs"
        ));
    }
    Ok(())
}

fn has_multiple_bits(value: u32) -> bool {
    value != 0 && !value.is_power_of_two()
}

#[cfg(test)]
mod tests {
    use ironrdp_core::{decode, encode_vec};

    use super::*;

    fn assert_round_trip(pdu: RailPdu) {
        let encoded = encode_vec(&pdu).expect("PDU encodes");
        let decoded: RailPdu = decode(&encoded).expect("PDU decodes");
        assert_eq!(decoded, pdu);
    }

    fn sample_accent_color() -> AccentColor {
        AccentColor {
            fields_valid_flags: 0x0000_0BFF,
            accent_color: 1,
            colorization_color: 2,
            colorization_color_balance: 3,
            colorization_afterglow: 4,
            colorization_afterglow_balance: 5,
            colorization_blur_balance: 6,
            colorization_glass_attribute: 7,
            color_prevalence: 8,
            enable_window_colorization: 9,
            accent_color_menu: 10,
            start_color_menu: 11,
            accent_palette: vec![1],
        }
    }

    #[test]
    fn round_trips_every_order_type() {
        let rectangle = Rectangle {
            left: -1,
            top: 2,
            right: 300,
            bottom: 400,
        };
        let pdus = vec![
            RailPdu::Handshake(HandshakePdu { build_number: 1 }),
            RailPdu::ClientStatus(ClientStatusPdu {
                flags: ClientStatusPdu::ALLOW_LOCAL_MOVE_SIZE,
            }),
            RailPdu::HandshakeEx(HandshakeExPdu {
                build_number: 2,
                flags: HandshakeExPdu::SNAP_ARRANGE,
            }),
            RailPdu::Execute(ExecutePdu {
                flags: ExecutePdu::FILE,
                executable: "app.exe".into(),
                working_directory: "C:\\work".into(),
                arguments: "--test".into(),
            }),
            RailPdu::ExecuteResult(ExecuteResultPdu {
                flags: ExecutePdu::FILE,
                result: ExecuteResult::Ok,
                raw_result: 0,
                executable: "app.exe".into(),
            }),
            RailPdu::ClientSystemParameters(ClientSystemParametersPdu {
                parameter: ClientSystemParameter::HighContrast(HighContrast {
                    flags: 1,
                    color_scheme: "High Contrast".into(),
                }),
            }),
            RailPdu::ServerSystemParameters(ServerSystemParametersPdu {
                parameter: ServerSystemParameter::ScreenSaverActive,
                enabled: true,
            }),
            RailPdu::Activate(ActivatePdu {
                window_id: 1,
                enabled: true,
            }),
            RailPdu::SystemMenu(SystemMenuPdu {
                window_id: 1,
                left: 10,
                top: 20,
            }),
            RailPdu::SystemCommand(SystemCommandPdu {
                window_id: 1,
                command: SystemCommand::Maximize,
            }),
            RailPdu::NotifyEvent(NotifyEventPdu {
                window_id: 1,
                notify_icon_id: 2,
                message: NotifyEventMessage::LeftButtonUp,
            }),
            RailPdu::GetApplicationIdRequest(GetApplicationIdRequestPdu { window_id: 1 }),
            RailPdu::MinMaxInfo(MinMaxInfoPdu {
                window_id: 1,
                values: [1, 2, 3, 4, 5, 6, 7, 8],
            }),
            RailPdu::LocalMoveSize(LocalMoveSizePdu {
                window_id: 1,
                is_start: true,
                move_size_type: MoveSizeType::Move,
                x: 5,
                y: 6,
            }),
            RailPdu::WindowMove(WindowMovePdu {
                window_id: 1,
                rectangle,
            }),
            RailPdu::SnapArrange(SnapArrangePdu {
                window_id: 1,
                rectangle,
            }),
            RailPdu::GetApplicationIdResponse(GetApplicationIdResponsePdu {
                window_id: 1,
                application_id: "com.example.app".into(),
            }),
            RailPdu::GetApplicationIdResponseEx(GetApplicationIdResponseExPdu {
                window_id: 1,
                application_id: "com.example.app".into(),
                process_id: 42,
                process_image_name: "app.exe".into(),
            }),
            RailPdu::LanguageBarInfo(LanguageBarInfoPdu { status: 1 }),
            RailPdu::LanguageImeInfo(LanguageImeInfoPdu {
                profile_type: LanguageProfileType::KeyboardLayout,
                language_id: 0x0409,
                language_profile_clsid: RailGuid::NULL,
                profile_guid: RailGuid::NULL,
                keyboard_layout: 0x0001_0409,
            }),
            RailPdu::CompartmentInfo(CompartmentInfoPdu {
                ime_state: ImeState::Open,
                ime_conversion_mode: 1,
                ime_sentence_mode: 4,
                kana_mode: KanaMode::On,
            }),
            RailPdu::ZOrderSync(ZOrderSyncPdu { window_id_marker: 1 }),
            RailPdu::Cloak(CloakPdu {
                window_id: 1,
                cloaked: true,
            }),
            RailPdu::PowerDisplayRequest(PowerDisplayRequestPdu { active: true }),
            RailPdu::TaskbarInfo(TaskbarInfoPdu {
                message: TaskbarMessage::Register,
                window_id_tab: 1,
                body: 2,
            }),
            RailPdu::TextScaleInfo(TextScaleInfoPdu { text_scale_factor: 125 }),
            RailPdu::CaretBlinkInfo(CaretBlinkInfoPdu {
                caret_blink_rate: u32::MAX,
            }),
        ];

        for pdu in pdus {
            assert_round_trip(pdu);
        }
    }

    #[test]
    fn round_trips_every_client_system_parameter_body() {
        let rectangle = SystemParameterRectangle {
            left: 1,
            top: 2,
            right: 3,
            bottom: 4,
        };
        let parameters = vec![
            ClientSystemParameter::FullWindowDrag(true),
            ClientSystemParameter::KeyboardCues(true),
            ClientSystemParameter::KeyboardPreference(true),
            ClientSystemParameter::WorkArea(rectangle),
            ClientSystemParameter::DisplayChange(rectangle),
            ClientSystemParameter::MouseButtonSwap(true),
            ClientSystemParameter::TaskbarPosition(rectangle),
            ClientSystemParameter::HighContrast(HighContrast {
                flags: 1,
                color_scheme: "contrast".into(),
            }),
            ClientSystemParameter::CaretWidth(1),
            ClientSystemParameter::StickyKeys(StickyKeys { flags: 1 }),
            ClientSystemParameter::ToggleKeys(ToggleKeys { flags: 1 }),
            ClientSystemParameter::FilterKeys(FilterKeys {
                flags: 1,
                wait_time: 1,
                delay_time: 2,
                repeat_time: 3,
                bounce_time: 4,
            }),
            ClientSystemParameter::DisplayAnimationsEnabled(true),
            ClientSystemParameter::DisplayAdvancedEffectsEnabled(true),
            ClientSystemParameter::DisplayAutoHideScrollbars(true),
            ClientSystemParameter::DisplayMessageDuration(5),
            ClientSystemParameter::ClosedCaptionFontColor(1),
            ClientSystemParameter::ClosedCaptionFontOpacity(1),
            ClientSystemParameter::ClosedCaptionFontSize(1),
            ClientSystemParameter::ClosedCaptionFontStyle(1),
            ClientSystemParameter::ClosedCaptionFontEdgeEffect(1),
            ClientSystemParameter::ClosedCaptionBackgroundColor(1),
            ClientSystemParameter::ClosedCaptionBackgroundOpacity(1),
            ClientSystemParameter::ClosedCaptionRegionColor(1),
            ClientSystemParameter::ClosedCaptionRegionOpacity(1),
            ClientSystemParameter::AccentColor(sample_accent_color()),
            ClientSystemParameter::SystemUsesLightTheme(true),
            ClientSystemParameter::AppsUseLightTheme(false),
        ];

        for parameter in parameters {
            assert_round_trip(RailPdu::ClientSystemParameters(ClientSystemParametersPdu { parameter }));
        }
    }

    #[test]
    fn rejects_invalid_header_and_length_fields() {
        assert!(decode::<RailPduHeader>(&[0x05, 0x00, 0x03, 0x00]).is_err());
        assert!(decode::<RailPdu>(&[0xFF, 0xFF, 0x04, 0x00]).is_err());
        assert!(decode::<RailPdu>(&[0x05, 0x00, 0x09, 0x00, 1, 0, 0, 0]).is_err());

        // An Execute PDU with an odd ExeOrFileLength is invalid UTF-16.
        assert!(decode::<RailPdu>(&[0x01, 0x00, 0x0D, 0x00, 0, 0, 1, 0, 0, 0, 0, 0, 0]).is_err());
        assert!(decode::<RailPdu>(&[0x15, 0x00, 0x09, 0x00, 0, 0, 0, 0, 2]).is_err());
    }

    #[test]
    fn accepts_nonzero_move_size_start() {
        let pdu = RailPdu::LocalMoveSize(LocalMoveSizePdu {
            window_id: 1,
            is_start: true,
            move_size_type: MoveSizeType::Move,
            x: 5,
            y: 6,
        });
        let mut encoded = encode_vec(&pdu).unwrap();
        encoded[8] = 2;

        assert_eq!(decode::<RailPdu>(&encoded).unwrap(), pdu);
    }

    #[test]
    fn rejects_invalid_outbound_values() {
        let invalid_execute = RailPdu::Execute(ExecutePdu {
            flags: ExecutePdu::TRANSLATE_FILES,
            executable: "bad\0value".into(),
            working_directory: String::new(),
            arguments: String::new(),
        });
        assert!(encode_vec(&invalid_execute).is_err());

        let invalid_text_scale = RailPdu::TextScaleInfo(TextScaleInfoPdu { text_scale_factor: 99 });
        assert!(encode_vec(&invalid_text_scale).is_err());
    }

    #[test]
    fn text_scale_and_caret_blink_are_client_to_server_only() {
        assert!(!RailPdu::TextScaleInfo(TextScaleInfoPdu { text_scale_factor: 125 }).is_server_to_client());
        assert!(!RailPdu::CaretBlinkInfo(CaretBlinkInfoPdu { caret_blink_rate: 530 }).is_server_to_client());
    }

    #[test]
    fn validates_pdu_direction() {
        let handshake = RailPdu::Handshake(HandshakePdu { build_number: 1 });
        assert!(handshake.validate_direction(RailPduDirection::ServerToClient).is_ok());
        assert!(handshake.validate_direction(RailPduDirection::ClientToServer).is_ok());

        let handshake_ex = RailPdu::HandshakeEx(HandshakeExPdu {
            build_number: 1,
            flags: HandshakeExPdu::HIDEF,
        });
        assert!(
            handshake_ex
                .validate_direction(RailPduDirection::ServerToClient)
                .is_ok()
        );
        assert!(
            handshake_ex
                .validate_direction(RailPduDirection::ClientToServer)
                .is_err()
        );

        let language_bar = RailPdu::LanguageBarInfo(LanguageBarInfoPdu { status: 1 });
        assert!(language_bar.is_server_to_client());
        assert!(language_bar.is_client_to_server());

        let language_ime = RailPdu::LanguageImeInfo(LanguageImeInfoPdu {
            profile_type: LanguageProfileType::KeyboardLayout,
            language_id: 0x0409,
            language_profile_clsid: RailGuid::NULL,
            profile_guid: RailGuid::NULL,
            keyboard_layout: 0x0001_0409,
        });
        assert!(!language_ime.is_server_to_client());
        assert!(language_ime.is_client_to_server());

        let compartment = RailPdu::CompartmentInfo(CompartmentInfoPdu {
            ime_state: ImeState::Open,
            ime_conversion_mode: 1,
            ime_sentence_mode: 4,
            kana_mode: KanaMode::On,
        });
        assert!(compartment.is_server_to_client());
        assert!(compartment.is_client_to_server());

        let cloak = RailPdu::Cloak(CloakPdu {
            window_id: 1,
            cloaked: true,
        });
        assert!(cloak.is_server_to_client());
        assert!(cloak.is_client_to_server());
    }
}
