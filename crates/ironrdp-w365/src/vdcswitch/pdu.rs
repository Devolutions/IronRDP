//! Virtual desktop control PDUs.
//!
//! No Microsoft Open Specification documents this channel.
//! The layouts follow the Windows App 2.0.1485.0 client, which reads every field little-endian after a common header;
//! [`PROTOCOL.md`] describes each layout and rule.
//!
//! [`PROTOCOL.md`]: https://github.com/Devolutions/IronRDP/blob/master/crates/ironrdp-w365/PROTOCOL.md#virtual-desktop-control

use alloc::string::String;
use core::fmt;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_size, invalid_field_err,
};
use ironrdp_dvc::DvcEncode;

const HEADER_SIZE: usize = 2 /* PduType */ + 4 /* PduLength */;
const VERSION_SIZE: usize = 4 /* Version */;
const HRESULT_SIZE: usize = 4 /* HResult */;
const LOCAL_ACTION_SIZE: usize = 1 /* LocalAction */;

const PDU_TYPE_VERSION_REQUEST: u16 = 0x0001;
const PDU_TYPE_VERSION_RESPONSE: u16 = 0x0002;
const PDU_TYPE_SWITCH_TO_LOCAL_REQUEST: u16 = 0x0003;
const PDU_TYPE_SWITCH_TO_LOCAL_RESPONSE: u16 = 0x0004;
const PDU_TYPE_SWITCH_TO_CLOUD_REQUEST: u16 = 0x0005;
const PDU_TYPE_SWITCH_TO_CLOUD_RESPONSE: u16 = 0x0006;
const PDU_TYPE_PERFORM_LOCAL_ACTION: u16 = 0x0007;

/// Protocol version carried by the version request and response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProtocolVersion(pub u32);

impl ProtocolVersion {
    /// Version 1, which supports switching between the local and cloud desktops.
    pub const V1: Self = Self(0x0001_0000);

    /// Version 2, which adds the perform local action PDU.
    pub const V2: Self = Self(0x0002_0000);

    /// Returns whether a client offering versions 1 through 2 can use this version.
    pub fn is_supported(self) -> bool {
        Self::V1 <= self && self <= Self::V2
    }
}

/// Local action the server asks the client to perform on the physical device.
///
/// Values other than the named constants are preserved so that a client can report them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LocalAction(pub u8);

impl LocalAction {
    /// Open the local Bluetooth settings.
    pub const BLUETOOTH_SETTINGS: Self = Self(0x00);

    /// Open the local sound settings.
    pub const SOUND_SETTINGS: Self = Self(0x01);

    /// Open the local display settings.
    pub const DISPLAY_SETTINGS: Self = Self(0x02);

    /// Open the local sign-in options.
    pub const ACCOUNT_SETTINGS: Self = Self(0x03);

    /// Open the local system settings.
    pub const SYSTEM_SETTINGS: Self = Self(0x04);
}

impl fmt::Debug for LocalAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::BLUETOOTH_SETTINGS => f.write_str("BluetoothSettings"),
            Self::SOUND_SETTINGS => f.write_str("SoundSettings"),
            Self::DISPLAY_SETTINGS => f.write_str("DisplaySettings"),
            Self::ACCOUNT_SETTINGS => f.write_str("AccountSettings"),
            Self::SYSTEM_SETTINGS => f.write_str("SystemSettings"),
            Self(value) => write!(f, "LocalAction({value:#04x})"),
        }
    }
}

/// Launch URI sent in the version request.
///
/// Windows App advertises the local actions it supports through query parameters, such as
/// `ms-remotedesktop-launch:?correlationId=<id>&supportBluetoothSettings=true`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchUri(
    /// INVARIANT: contains no NUL character, so its null-terminated UTF-16 encoding round-trips.
    String,
);

impl LaunchUri {
    pub fn new(uri: String) -> EncodeResult<Self> {
        if uri.contains('\0') {
            return Err(invalid_field_err!("launchUri", "launch URI contains a NUL character"));
        }

        Ok(Self(uri))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Size of the null-terminated UTF-16 encoding.
    fn encoded_size(&self) -> usize {
        (self.0.encode_utf16().count() + 1) * 2
    }
}

/// Version request sent by the client when the channel opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRequestPdu {
    pub min_version: ProtocolVersion,
    pub max_version: ProtocolVersion,
    pub launch_uri: LaunchUri,
}

impl VersionRequestPdu {
    /// Creates the request Windows App sends, offering versions 1 through 2.
    pub fn new(launch_uri: LaunchUri) -> Self {
        Self {
            min_version: ProtocolVersion::V1,
            max_version: ProtocolVersion::V2,
            launch_uri,
        }
    }

    const FIXED_SIZE: usize = 4 /* MinVersion */ + 4 /* MaxVersion */ + 4 /* LaunchUriLength */;

    fn body_size(&self) -> usize {
        Self::FIXED_SIZE + self.launch_uri.encoded_size()
    }

    fn encode_body(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        dst.write_u32(self.min_version.0);
        dst.write_u32(self.max_version.0);
        dst.write_u32(cast_length!("launchUriLength", self.launch_uri.encoded_size())?);
        for unit in self.launch_uri.as_str().encode_utf16() {
            dst.write_u16(unit);
        }
        dst.write_u16(0);
        Ok(())
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::FIXED_SIZE);
        let min_version = ProtocolVersion(src.read_u32());
        let max_version = ProtocolVersion(src.read_u32());
        let uri_size = usize::try_from(src.read_u32()).unwrap_or(usize::MAX);
        if !(2 <= uri_size && uri_size % 2 == 0) {
            return Err(invalid_field_err!(
                "launchUriLength",
                "launch URI length is not a null-terminated UTF-16 size",
                in: src
            ));
        }
        ensure_size!(in: src, size: uri_size);
        let bytes = src.read_slice(uri_size);
        let (text, terminator) = bytes.split_last_chunk::<2>().expect("uri_size is at least 2");
        if *terminator != [0, 0] {
            return Err(invalid_field_err!("launchUri", "launch URI is not null-terminated", in: src));
        }
        let units = text.chunks_exact(2).map(|unit| u16::from_le_bytes([unit[0], unit[1]]));
        let uri = char::decode_utf16(units)
            .collect::<Result<String, _>>()
            .map_err(|_| invalid_field_err!("launchUri", "launch URI is not valid UTF-16", in: src))?;
        if uri.contains('\0') {
            return Err(invalid_field_err!("launchUri", "launch URI contains a NUL character", in: src));
        }
        // The NUL check above upholds the LaunchUri invariant.
        let launch_uri = LaunchUri(uri);

        Ok(Self {
            min_version,
            max_version,
            launch_uri,
        })
    }
}

/// Virtual desktop control PDU.
///
/// Every PDU starts with a 16-bit type and a 32-bit length that counts the whole PDU, header included.
/// See [the virtual desktop control section of `PROTOCOL.md`][pdus].
///
/// [pdus]: https://github.com/Devolutions/IronRDP/blob/master/crates/ironrdp-w365/PROTOCOL.md#virtual-desktop-control
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VdcSwitchPdu {
    /// Client to server: the versions and launch URI the client supports.
    VersionRequest(VersionRequestPdu),
    /// Server to client: the version the server selected.
    VersionResponse { version: ProtocolVersion },
    /// Server to client: switch the user to the local desktop.
    SwitchToLocalRequest,
    /// Client to server: the HRESULT of a switch to the local desktop.
    SwitchToLocalResponse { hresult: u32 },
    /// Client to server: the user switched back to the Cloud PC.
    SwitchToCloudRequest,
    /// Server to client: the HRESULT of a switch to the Cloud PC.
    SwitchToCloudResponse { hresult: u32 },
    /// Server to client: perform a local action on the physical device.
    ///
    /// The server sends it only after it selected version 2 or later.
    PerformLocalAction { action: LocalAction },
}

impl VdcSwitchPdu {
    const NAME: &'static str = "VIRTUALDESKTOPCONTROL_PDU";

    fn pdu_type(&self) -> u16 {
        match self {
            Self::VersionRequest(_) => PDU_TYPE_VERSION_REQUEST,
            Self::VersionResponse { .. } => PDU_TYPE_VERSION_RESPONSE,
            Self::SwitchToLocalRequest => PDU_TYPE_SWITCH_TO_LOCAL_REQUEST,
            Self::SwitchToLocalResponse { .. } => PDU_TYPE_SWITCH_TO_LOCAL_RESPONSE,
            Self::SwitchToCloudRequest => PDU_TYPE_SWITCH_TO_CLOUD_REQUEST,
            Self::SwitchToCloudResponse { .. } => PDU_TYPE_SWITCH_TO_CLOUD_RESPONSE,
            Self::PerformLocalAction { .. } => PDU_TYPE_PERFORM_LOCAL_ACTION,
        }
    }

    fn body_size(&self) -> usize {
        match self {
            Self::VersionRequest(pdu) => pdu.body_size(),
            Self::VersionResponse { .. } => VERSION_SIZE,
            Self::SwitchToLocalRequest | Self::SwitchToCloudRequest => 0,
            Self::SwitchToLocalResponse { .. } | Self::SwitchToCloudResponse { .. } => HRESULT_SIZE,
            Self::PerformLocalAction { .. } => LOCAL_ACTION_SIZE,
        }
    }
}

impl Encode for VdcSwitchPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u16(self.pdu_type());
        dst.write_u32(cast_length!("pduLength", self.size())?);
        match self {
            Self::VersionRequest(pdu) => pdu.encode_body(dst)?,
            Self::VersionResponse { version } => dst.write_u32(version.0),
            Self::SwitchToLocalRequest | Self::SwitchToCloudRequest => {}
            Self::SwitchToLocalResponse { hresult } | Self::SwitchToCloudResponse { hresult } => {
                dst.write_u32(*hresult)
            }
            Self::PerformLocalAction { action } => dst.write_u8(action.0),
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        HEADER_SIZE + self.body_size()
    }
}

impl<'de> Decode<'de> for VdcSwitchPdu {
    /// Decodes one PDU and consumes exactly `pduLength` bytes.
    ///
    /// Like Windows App, the decoder checks each PDU type against a minimum length and ignores
    /// bytes past the fields it knows, so a later protocol version can extend a PDU.
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: HEADER_SIZE);
        let pdu_type = src.read_u16();
        let pdu_length = usize::try_from(src.read_u32()).unwrap_or(usize::MAX);
        if !(HEADER_SIZE <= pdu_length && pdu_length - HEADER_SIZE <= src.len()) {
            return Err(invalid_field_err!(
                "pduLength",
                "virtual desktop control PDU length does not fit its payload",
                in: src
            ));
        }
        let mut body = ReadCursor::new(src.read_slice(pdu_length - HEADER_SIZE));

        let pdu = match pdu_type {
            PDU_TYPE_VERSION_REQUEST => Self::VersionRequest(VersionRequestPdu::decode_body(&mut body)?),
            PDU_TYPE_VERSION_RESPONSE => {
                ensure_size!(in: body, size: VERSION_SIZE);
                Self::VersionResponse {
                    version: ProtocolVersion(body.read_u32()),
                }
            }
            PDU_TYPE_SWITCH_TO_LOCAL_REQUEST => Self::SwitchToLocalRequest,
            PDU_TYPE_SWITCH_TO_LOCAL_RESPONSE => {
                ensure_size!(in: body, size: HRESULT_SIZE);
                Self::SwitchToLocalResponse {
                    hresult: body.read_u32(),
                }
            }
            PDU_TYPE_SWITCH_TO_CLOUD_REQUEST => Self::SwitchToCloudRequest,
            PDU_TYPE_SWITCH_TO_CLOUD_RESPONSE => {
                ensure_size!(in: body, size: HRESULT_SIZE);
                Self::SwitchToCloudResponse {
                    hresult: body.read_u32(),
                }
            }
            PDU_TYPE_PERFORM_LOCAL_ACTION => {
                ensure_size!(in: body, size: LOCAL_ACTION_SIZE);
                Self::PerformLocalAction {
                    action: LocalAction(body.read_u8()),
                }
            }
            _ => {
                return Err(invalid_field_err!(
                    "pduType",
                    "unknown virtual desktop control PDU type",
                    in: src
                ));
            }
        };

        Ok(pdu)
    }
}

impl DvcEncode for VdcSwitchPdu {}
