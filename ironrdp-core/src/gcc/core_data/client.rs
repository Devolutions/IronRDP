#[cfg(test)]
pub mod test;

use std::io;

use bitflags::bitflags;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use num_derive::{FromPrimitive, ToPrimitive};
use num_traits::{FromPrimitive, ToPrimitive};
use tap::Pipe as _;

use super::{CoreDataError, RdpVersion, VERSION_SIZE};
use crate::{connection_initiation, try_read_optional, try_write_optional, utils, PduParsing};

pub const IME_FILE_NAME_SIZE: usize = 64;

const DESKTOP_WIDTH_SIZE: usize = 2;
const DESKTOP_HEIGHT_SIZE: usize = 2;
const COLOR_DEPTH_SIZE: usize = 2;
const SEC_ACCESS_SEQUENCE_SIZE: usize = 2;
const KEYBOARD_LAYOUT_SIZE: usize = 4;
const CLIENT_BUILD_SIZE: usize = 4;
const CLIENT_NAME_SIZE: usize = 32;
const KEYBOARD_TYPE_SIZE: usize = 4;
const KEYBOARD_SUB_TYPE_SIZE: usize = 4;
const KEYBOARD_FUNCTIONAL_KEYS_COUNT_SIZE: usize = 4;

const POST_BETA_COLOR_DEPTH_SIZE: usize = 2;
const CLIENT_PRODUCT_ID_SIZE: usize = 2;
const SERIAL_NUMBER_SIZE: usize = 4;
const HIGH_COLOR_DEPTH_SIZE: usize = 2;
const SUPPORTED_COLOR_DEPTHS_SIZE: usize = 2;
const EARLY_CAPABILITY_FLAGS_SIZE: usize = 2;
const DIG_PRODUCT_ID_SIZE: usize = 64;
const CONNECTION_TYPE_SIZE: usize = 1;
const PADDING_SIZE: usize = 1;
const SERVER_SELECTED_PROTOCOL_SIZE: usize = 4;
const DESKTOP_PHYSICAL_WIDTH_SIZE: usize = 4;
const DESKTOP_PHYSICAL_HEIGHT_SIZE: usize = 4;
const DESKTOP_ORIENTATION_SIZE: usize = 2;
const DESKTOP_SCALE_FACTOR_SIZE: usize = 4;
const DEVICE_SCALE_FACTOR_SIZE: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientCoreData {
    pub version: RdpVersion,
    pub desktop_width: u16,
    pub desktop_height: u16,
    pub color_depth: ColorDepth,
    pub sec_access_sequence: SecureAccessSequence,
    pub keyboard_layout: u32,
    pub client_build: u32,
    pub client_name: String,
    pub keyboard_type: KeyboardType,
    pub keyboard_subtype: u32,
    pub keyboard_functional_keys_count: u32,
    pub ime_file_name: String,
    pub optional_data: ClientCoreOptionalData,
}

impl ClientCoreData {
    pub fn client_color_depth(&self) -> ClientColorDepth {
        if let Some(high_color_depth) = self.optional_data.high_color_depth {
            if let Some(early_capability_flags) = self.optional_data.early_capability_flags {
                if early_capability_flags.contains(ClientEarlyCapabilityFlags::WANT_32_BPP_SESSION) {
                    ClientColorDepth::Bpp32
                } else {
                    From::from(high_color_depth)
                }
            } else {
                From::from(high_color_depth)
            }
        } else if let Some(post_beta_color_depth) = self.optional_data.post_beta_color_depth {
            From::from(post_beta_color_depth)
        } else {
            From::from(self.color_depth)
        }
    }
}

impl PduParsing for ClientCoreData {
    type Error = CoreDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let version = buffer.read_u32::<LittleEndian>()?.pipe(RdpVersion);
        let desktop_width = buffer.read_u16::<LittleEndian>()?;
        let desktop_height = buffer.read_u16::<LittleEndian>()?;
        let color_depth = buffer
            .read_u16::<LittleEndian>()?
            .pipe(ColorDepth::from_u16)
            .ok_or(CoreDataError::InvalidColorDepth)?;
        let sec_access_sequence = buffer
            .read_u16::<LittleEndian>()?
            .pipe(SecureAccessSequence::from_u16)
            .ok_or(CoreDataError::InvalidSecureAccessSequence)?;
        let keyboard_layout = buffer.read_u32::<LittleEndian>()?;
        let client_build = buffer.read_u32::<LittleEndian>()?;

        let mut client_name_buffer = [0; CLIENT_NAME_SIZE];
        buffer.read_exact(&mut client_name_buffer)?;
        let client_name = utils::from_utf16_bytes(client_name_buffer.as_ref())
            .trim_end_matches('\u{0}')
            .into();

        let keyboard_type = buffer
            .read_u32::<LittleEndian>()?
            .pipe(KeyboardType::from_u32)
            .ok_or(CoreDataError::InvalidKeyboardType)?;
        let keyboard_subtype = buffer.read_u32::<LittleEndian>()?;
        let keyboard_functional_keys_count = buffer.read_u32::<LittleEndian>()?;

        let mut ime_file_name_buffer = [0; IME_FILE_NAME_SIZE];
        buffer.read_exact(&mut ime_file_name_buffer)?;
        let ime_file_name = utils::from_utf16_bytes(ime_file_name_buffer.as_ref())
            .trim_end_matches('\u{0}')
            .into();

        let optional_data = ClientCoreOptionalData::from_buffer(&mut buffer)?;

        Ok(Self {
            version,
            desktop_width,
            desktop_height,
            color_depth,
            sec_access_sequence,
            keyboard_layout,
            client_build,
            client_name,
            keyboard_type,
            keyboard_subtype,
            keyboard_functional_keys_count,
            ime_file_name,
            optional_data,
        })
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        let mut client_name_buffer = utils::to_utf16_bytes(self.client_name.as_ref());
        client_name_buffer.resize(CLIENT_NAME_SIZE - 2, 0);
        let mut ime_file_name_buffer = utils::to_utf16_bytes(self.ime_file_name.as_ref());
        ime_file_name_buffer.resize(IME_FILE_NAME_SIZE - 2, 0);

        buffer.write_u32::<LittleEndian>(self.version.0)?;
        buffer.write_u16::<LittleEndian>(self.desktop_width)?;
        buffer.write_u16::<LittleEndian>(self.desktop_height)?;
        buffer.write_u16::<LittleEndian>(self.color_depth.to_u16().unwrap())?;
        buffer.write_u16::<LittleEndian>(self.sec_access_sequence.to_u16().unwrap())?;
        buffer.write_u32::<LittleEndian>(self.keyboard_layout)?;
        buffer.write_u32::<LittleEndian>(self.client_build)?;
        buffer.write_all(client_name_buffer.as_ref())?;
        buffer.write_u16::<LittleEndian>(0)?; // client name UTF-16 null terminator
        buffer.write_u32::<LittleEndian>(self.keyboard_type.to_u32().unwrap())?;
        buffer.write_u32::<LittleEndian>(self.keyboard_subtype)?;
        buffer.write_u32::<LittleEndian>(self.keyboard_functional_keys_count)?;
        buffer.write_all(ime_file_name_buffer.as_ref())?;
        buffer.write_u16::<LittleEndian>(0)?; // ime file name UTF-16 null terminator

        self.optional_data.to_buffer(&mut buffer)
    }

    fn buffer_length(&self) -> usize {
        VERSION_SIZE
            + DESKTOP_WIDTH_SIZE
            + DESKTOP_HEIGHT_SIZE
            + COLOR_DEPTH_SIZE
            + SEC_ACCESS_SEQUENCE_SIZE
            + KEYBOARD_LAYOUT_SIZE
            + CLIENT_BUILD_SIZE
            + CLIENT_NAME_SIZE
            + KEYBOARD_TYPE_SIZE
            + KEYBOARD_SUB_TYPE_SIZE
            + KEYBOARD_FUNCTIONAL_KEYS_COUNT_SIZE
            + IME_FILE_NAME_SIZE
            + self.optional_data.buffer_length()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientCoreOptionalData {
    pub post_beta_color_depth: Option<ColorDepth>,
    pub client_product_id: Option<u16>,
    pub serial_number: Option<u32>,
    pub high_color_depth: Option<HighColorDepth>,
    pub supported_color_depths: Option<SupportedColorDepths>,
    pub early_capability_flags: Option<ClientEarlyCapabilityFlags>,
    pub dig_product_id: Option<String>,
    pub connection_type: Option<ConnectionType>,
    pub server_selected_protocol: Option<connection_initiation::SecurityProtocol>,
    pub desktop_physical_width: Option<u32>,
    pub desktop_physical_height: Option<u32>,
    pub desktop_orientation: Option<u16>,
    pub desktop_scale_factor: Option<u32>,
    pub device_scale_factor: Option<u32>,
}

impl PduParsing for ClientCoreOptionalData {
    type Error = CoreDataError;

    fn from_buffer(mut buffer: impl io::Read) -> Result<Self, Self::Error> {
        let mut optional_data = Self::default();

        optional_data.post_beta_color_depth = Some(
            ColorDepth::from_u16(try_read_optional!(buffer.read_u16::<LittleEndian>(), optional_data))
                .ok_or(CoreDataError::InvalidPostBetaColorDepth)?,
        );

        optional_data.client_product_id = Some(try_read_optional!(buffer.read_u16::<LittleEndian>(), optional_data));
        optional_data.serial_number = Some(try_read_optional!(buffer.read_u32::<LittleEndian>(), optional_data));

        optional_data.high_color_depth = Some(
            HighColorDepth::from_u16(try_read_optional!(buffer.read_u16::<LittleEndian>(), optional_data))
                .ok_or(CoreDataError::InvalidHighColorDepth)?,
        );

        optional_data.supported_color_depths = Some(
            SupportedColorDepths::from_bits(try_read_optional!(buffer.read_u16::<LittleEndian>(), optional_data))
                .ok_or(CoreDataError::InvalidSupportedColorDepths)?,
        );

        optional_data.early_capability_flags = Some(
            ClientEarlyCapabilityFlags::from_bits(try_read_optional!(buffer.read_u16::<LittleEndian>(), optional_data))
                .ok_or(CoreDataError::InvalidEarlyCapabilityFlags)?,
        );

        let mut dig_product_id_buffer = [0; DIG_PRODUCT_ID_SIZE];
        try_read_optional!(buffer.read_exact(&mut dig_product_id_buffer), optional_data);
        optional_data.dig_product_id = Some(
            utils::from_utf16_bytes(dig_product_id_buffer.as_ref())
                .trim_end_matches('\u{0}')
                .into(),
        );

        optional_data.connection_type = Some(
            ConnectionType::from_u8(try_read_optional!(buffer.read_u8(), optional_data))
                .ok_or(CoreDataError::InvalidConnectionType)?,
        );

        try_read_optional!(buffer.read_u8(), optional_data); // pad1octet

        optional_data.server_selected_protocol = Some(
            connection_initiation::SecurityProtocol::from_bits(try_read_optional!(
                buffer.read_u32::<LittleEndian>(),
                optional_data
            ))
            .ok_or(CoreDataError::InvalidServerSecurityProtocol)?,
        );

        optional_data.desktop_physical_width =
            Some(try_read_optional!(buffer.read_u32::<LittleEndian>(), optional_data));
        // physical height must be present, if the physical width is present
        optional_data.desktop_physical_height = Some(buffer.read_u32::<LittleEndian>()?);

        optional_data.desktop_orientation = Some(try_read_optional!(buffer.read_u16::<LittleEndian>(), optional_data));
        optional_data.desktop_scale_factor = Some(try_read_optional!(buffer.read_u32::<LittleEndian>(), optional_data));
        // device scale factor must be present, if the desktop scale factor is present
        optional_data.device_scale_factor = Some(buffer.read_u32::<LittleEndian>()?);

        Ok(optional_data)
    }

    fn to_buffer(&self, mut buffer: impl io::Write) -> Result<(), Self::Error> {
        try_write_optional!(self.post_beta_color_depth, |value: &ColorDepth| {
            buffer.write_u16::<LittleEndian>(value.to_u16().unwrap())
        });

        try_write_optional!(self.client_product_id, |value: &u16| buffer
            .write_u16::<LittleEndian>(*value));

        try_write_optional!(self.serial_number, |value: &u32| buffer
            .write_u32::<LittleEndian>(*value));

        try_write_optional!(self.high_color_depth, |value: &HighColorDepth| buffer
            .write_u16::<LittleEndian>(value.to_u16().unwrap()));

        try_write_optional!(self.supported_color_depths, |value: &SupportedColorDepths| buffer
            .write_u16::<LittleEndian>(
            value.bits()
        ));

        try_write_optional!(self.early_capability_flags, |value: &ClientEarlyCapabilityFlags| buffer
            .write_u16::<LittleEndian>(value.bits()));

        try_write_optional!(self.dig_product_id, |value: &str| {
            let mut dig_product_id_buffer = utils::to_utf16_bytes(value);
            dig_product_id_buffer.resize(DIG_PRODUCT_ID_SIZE - 2, 0);
            dig_product_id_buffer.extend_from_slice([0; 2].as_ref()); // UTF-16 null terminator

            buffer.write_all(dig_product_id_buffer.as_ref())
        });

        try_write_optional!(self.connection_type, |value: &ConnectionType| buffer
            .write_u8(value.to_u8().unwrap()));

        buffer.write_u8(0)?; // pad1octet

        try_write_optional!(
            self.server_selected_protocol,
            |value: &connection_initiation::SecurityProtocol| { buffer.write_u32::<LittleEndian>(value.bits()) }
        );

        try_write_optional!(self.desktop_physical_width, |value: &u32| buffer
            .write_u32::<LittleEndian>(*value));

        try_write_optional!(self.desktop_physical_height, |value: &u32| buffer
            .write_u32::<LittleEndian>(*value));

        try_write_optional!(self.desktop_orientation, |value: &u16| buffer
            .write_u16::<LittleEndian>(*value));

        try_write_optional!(self.desktop_scale_factor, |value: &u32| buffer
            .write_u32::<LittleEndian>(*value));

        try_write_optional!(self.device_scale_factor, |value: &u32| buffer
            .write_u32::<LittleEndian>(*value));

        Ok(())
    }

    fn buffer_length(&self) -> usize {
        let mut size = 0;

        if self.post_beta_color_depth.is_some() {
            size += POST_BETA_COLOR_DEPTH_SIZE;
        }
        if self.client_product_id.is_some() {
            size += CLIENT_PRODUCT_ID_SIZE;
        }
        if self.serial_number.is_some() {
            size += SERIAL_NUMBER_SIZE;
        }
        if self.high_color_depth.is_some() {
            size += HIGH_COLOR_DEPTH_SIZE;
        }
        if self.supported_color_depths.is_some() {
            size += SUPPORTED_COLOR_DEPTHS_SIZE;
        }
        if self.early_capability_flags.is_some() {
            size += EARLY_CAPABILITY_FLAGS_SIZE;
        }
        if self.dig_product_id.is_some() {
            size += DIG_PRODUCT_ID_SIZE;
        }
        if self.connection_type.is_some() {
            size += CONNECTION_TYPE_SIZE + PADDING_SIZE;
        }
        if self.server_selected_protocol.is_some() {
            size += SERVER_SELECTED_PROTOCOL_SIZE;
        }
        if self.desktop_physical_width.is_some() {
            size += DESKTOP_PHYSICAL_WIDTH_SIZE;
        }
        if self.desktop_physical_height.is_some() {
            size += DESKTOP_PHYSICAL_HEIGHT_SIZE;
        }
        if self.desktop_orientation.is_some() {
            size += DESKTOP_ORIENTATION_SIZE;
        }
        if self.desktop_scale_factor.is_some() {
            size += DESKTOP_SCALE_FACTOR_SIZE;
        }
        if self.device_scale_factor.is_some() {
            size += DEVICE_SCALE_FACTOR_SIZE;
        }

        size
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ClientColorDepth {
    Bpp4,
    Bpp8,
    Rgb555Bpp16,
    Rgb565Bpp16,
    Bpp24,
    Bpp32,
}

impl From<ColorDepth> for ClientColorDepth {
    fn from(color_depth: ColorDepth) -> Self {
        match color_depth {
            ColorDepth::Bpp4 => ClientColorDepth::Bpp4,
            ColorDepth::Bpp8 => ClientColorDepth::Bpp8,
            ColorDepth::Rgb555Bpp16 => ClientColorDepth::Rgb555Bpp16,
            ColorDepth::Rgb565Bpp16 => ClientColorDepth::Rgb565Bpp16,
            ColorDepth::Bpp24 => ClientColorDepth::Bpp24,
        }
    }
}

impl From<HighColorDepth> for ClientColorDepth {
    fn from(color_depth: HighColorDepth) -> Self {
        match color_depth {
            HighColorDepth::Bpp4 => ClientColorDepth::Bpp4,
            HighColorDepth::Bpp8 => ClientColorDepth::Bpp8,
            HighColorDepth::Rgb555Bpp16 => ClientColorDepth::Rgb555Bpp16,
            HighColorDepth::Rgb565Bpp16 => ClientColorDepth::Rgb565Bpp16,
            HighColorDepth::Bpp24 => ClientColorDepth::Bpp24,
        }
    }
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum ColorDepth {
    Bpp4 = 0xCA00,
    Bpp8 = 0xCA01,
    Rgb555Bpp16 = 0xCA02,
    Rgb565Bpp16 = 0xCA03,
    Bpp24 = 0xCA04,
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, FromPrimitive, ToPrimitive, Eq, Ord, PartialEq, PartialOrd)]
pub enum HighColorDepth {
    Bpp4 = 0x0004,
    Bpp8 = 0x0008,
    Rgb555Bpp16 = 0x000F,
    Rgb565Bpp16 = 0x0010,
    Bpp24 = 0x0018,
}

#[repr(u16)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum SecureAccessSequence {
    Del = 0xAA03,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum KeyboardType {
    IbmPcXt = 1,
    OlivettiIco = 2,
    IbmPcAt = 3,
    IbmEnhanced = 4,
    Nokia1050 = 5,
    Nokia9140 = 6,
    Japanese = 7,
}

#[repr(u8)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, FromPrimitive, ToPrimitive)]
pub enum ConnectionType {
    NotUsed = 0, // not used as ClientEarlyCapabilityFlags::VALID_CONNECTION_TYPE not set
    Modem = 1,
    BroadbandLow = 2,
    Satellite = 3,
    BroadbandHigh = 4,
    Wan = 5,
    Lan = 6,
    Autodetect = 7,
}

bitflags! {
    pub struct SupportedColorDepths: u16 {
        const BPP24 = 1;
        const BPP16 = 2;
        const BPP15 = 4;
        const BPP32 = 8;
    }
}

bitflags! {
    pub struct ClientEarlyCapabilityFlags: u16 {
        const SUPPORT_ERR_INFO_PDU = 0x0001;
        const WANT_32_BPP_SESSION = 0x0002;
        const SUPPORT_STATUS_INFO_PDU = 0x0004;
        const STRONG_ASYMMETRIC_KEYS = 0x0008;
        const UNUSED = 0x0010;
        const VALID_CONNECTION_TYPE = 0x0020;
        const SUPPORT_MONITOR_LAYOUT_PDU = 0x0040;
        const SUPPORT_NET_CHAR_AUTODETECT = 0x0080;
        const SUPPORT_DYN_VC_GFX_PROTOCOL =0x0100;
        const SUPPORT_DYNAMIC_TIME_ZONE = 0x0200;
        const SUPPORT_HEART_BEAT_PDU = 0x0400;
    }
}
