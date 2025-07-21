use array_concat::{concat_arrays, concat_arrays_size};
use ironrdp_pdu::gcc::{
    ClientCoreData, ClientCoreOptionalData, ClientEarlyCapabilityFlags, ColorDepth, ConnectionType, HighColorDepth,
    KeyboardType, RdpVersion, SecureAccessSequence, ServerCoreData, ServerCoreOptionalData, ServerEarlyCapabilityFlags,
    SupportedColorDepths,
};
use ironrdp_pdu::nego::SecurityProtocol;
use lazy_static::lazy_static;

pub const CLIENT_CORE_DATA_BUFFER: [u8; 128] = [
    0x04, 0x00, 0x08, 0x00, // version
    0x00, 0x05, // desktop width
    0x00, 0x04, // desktop height
    0x00, 0xca, // color depth
    0x03, 0xaa, // sas sequence
    0x09, 0x04, 0x00, 0x00, // keyboard layout
    0xce, 0x0e, 0x00, 0x00, // client build
    0x45, 0x00, 0x4c, 0x00, 0x54, 0x00, 0x4f, 0x00, 0x4e, 0x00, 0x53, 0x00, 0x2d, 0x00, 0x44, 0x00, 0x45, 0x00, 0x56,
    0x00, 0x32, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // client name
    0x04, 0x00, 0x00, 0x00, // keyboard type
    0x00, 0x00, 0x00, 0x00, // keyboard subtype
    0x0c, 0x00, 0x00, 0x00, // keyboard function key
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // ime file name
];

pub const CLIENT_OPTIONAL_CORE_DATA_TO_HIGH_COLOR_DEPTH_BUFFER: [u8; 8] = [
    0x01, 0xca, // post beta color depth
    0x01, 0x00, // client product id
    0x00, 0x00, 0x00, 0x00, // serial number
];

pub const EARLY_CAPABILITY_FLAGS_START: usize = 4;

pub const EARLY_CAPABILITY_FLAGS_LENGTH: usize = 2;

pub const CLIENT_OPTIONAL_CORE_DATA_FROM_HIGH_COLOR_DEPTH_TO_SERVER_SELECTED_PROTOCOL_BUFFER: [u8; 76] = [
    0x18, 0x00, // high color depth
    0x07, 0x00, // supported color depths
    0x01, 0x00, // early capability flags
    0x36, 0x00, 0x39, 0x00, 0x37, 0x00, 0x31, 0x00, 0x32, 0x00, 0x2d, 0x00, 0x37, 0x00, 0x38, 0x00, 0x33, 0x00, 0x2d,
    0x00, 0x30, 0x00, 0x33, 0x00, 0x35, 0x00, 0x37, 0x00, 0x39, 0x00, 0x37, 0x00, 0x34, 0x00, 0x2d, 0x00, 0x34, 0x00,
    0x32, 0x00, 0x37, 0x00, 0x31, 0x00, 0x34, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // client dig product id
    0x00, // connection type
    0x00, // padding
    0x00, 0x00, 0x00, 0x00, // server selected protocol
];

pub const CLIENT_OPTIONAL_CORE_DATA_FROM_DESKTOP_PHYSICAL_WIDTH_TO_DEVICE_SCALE_FACTOR_BUFFER: [u8; 18] = [
    0x88, 0x13, 0x00, 0x00, // desktop physical width
    0xb8, 0x0b, 0x00, 0x00, //desktop physical height
    0x5a, 0x00, // desktop orientation
    0xc8, 0x00, 0x00, 0x00, // desktop scale factor
    0x8c, 0x00, 0x00, 0x00, // device scale factor
];

lazy_static! {
    pub static ref CLIENT_CORE_DATA_WITHOUT_OPTIONAL_FIELDS: ClientCoreData = ClientCoreData {
        version: RdpVersion::V5_PLUS,
        desktop_width: 1280,
        desktop_height: 1024,
        color_depth: ColorDepth::Bpp4,
        sec_access_sequence: SecureAccessSequence::Del,
        keyboard_layout: 1033,
        client_build: 3790,
        client_name: String::from("ELTONS-DEV2"),
        keyboard_type: KeyboardType::IbmEnhanced,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        ime_file_name: String::new(),
        optional_data: ClientCoreOptionalData::default(),
    };
    pub static ref CLIENT_OPTIONAL_CORE_DATA_TO_HIGH_COLOR_DEPTH: ClientCoreData = {
        let mut data = CLIENT_CORE_DATA_WITHOUT_OPTIONAL_FIELDS.clone();
        data.optional_data.post_beta2_color_depth = Some(ColorDepth::Bpp8);
        data.optional_data.client_product_id = Some(1);
        data.optional_data.serial_number = Some(0);
        data
    };
    pub static ref CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL: ClientCoreData = {
        let mut data = CLIENT_OPTIONAL_CORE_DATA_TO_HIGH_COLOR_DEPTH.clone();
        data.optional_data.high_color_depth = Some(HighColorDepth::Bpp24);
        data.optional_data.supported_color_depths =
            Some(SupportedColorDepths::BPP24 | SupportedColorDepths::BPP16 | SupportedColorDepths::BPP15);
        data.optional_data.early_capability_flags = Some(ClientEarlyCapabilityFlags::SUPPORT_ERR_INFO_PDU);
        data.optional_data.dig_product_id = Some(String::from("69712-783-0357974-42714"));
        data.optional_data.connection_type = Some(ConnectionType::NotUsed);
        data.optional_data.server_selected_protocol = Some(SecurityProtocol::empty());
        data
    };
    pub static ref CLIENT_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS: ClientCoreData = {
        let mut data = CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL.clone();
        data.optional_data.desktop_physical_width = Some(5000);
        data.optional_data.desktop_physical_height = Some(3000);
        data.optional_data.desktop_orientation = Some(90);
        data.optional_data.desktop_scale_factor = Some(200);
        data.optional_data.device_scale_factor = Some(140);
        data
    };
    pub static ref CLIENT_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_WITH_WANT_32_BPP_EARLY_FLAG: ClientCoreData = {
        let mut data = CLIENT_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS.clone();
        data.optional_data.early_capability_flags = Some(ClientEarlyCapabilityFlags::WANT_32_BPP_SESSION);
        data
    };
    pub static ref CLIENT_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_WITH_WANT_32_BPP_EARLY_FLAG_BUFFER: Vec<u8> = {
        let early_capability_flags = ClientEarlyCapabilityFlags::WANT_32_BPP_SESSION.bits().to_le_bytes();

        let mut from_high_color_to_server_protocol =
            CLIENT_OPTIONAL_CORE_DATA_FROM_HIGH_COLOR_DEPTH_TO_SERVER_SELECTED_PROTOCOL_BUFFER;
        from_high_color_to_server_protocol
            [EARLY_CAPABILITY_FLAGS_START..EARLY_CAPABILITY_FLAGS_START + EARLY_CAPABILITY_FLAGS_LENGTH]
            .clone_from_slice(early_capability_flags.as_ref());

        let mut buffer = CLIENT_OPTIONAL_CORE_DATA_TO_HIGH_COLOR_DEPTH_BUFFER_BUFFER.to_vec();
        buffer.extend(from_high_color_to_server_protocol.as_ref());
        buffer.extend(CLIENT_OPTIONAL_CORE_DATA_FROM_DESKTOP_PHYSICAL_WIDTH_TO_DEVICE_SCALE_FACTOR_BUFFER.as_ref());

        buffer
    };
}

pub const CLIENT_OPTIONAL_CORE_DATA_TO_HIGH_COLOR_DEPTH_BUFFER_BUFFER: [u8; concat_arrays_size!(
    CLIENT_CORE_DATA_BUFFER,
    CLIENT_OPTIONAL_CORE_DATA_TO_HIGH_COLOR_DEPTH_BUFFER
)] = concat_arrays!(
    CLIENT_CORE_DATA_BUFFER,
    CLIENT_OPTIONAL_CORE_DATA_TO_HIGH_COLOR_DEPTH_BUFFER
);

pub const CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL_BUFFER: [u8; concat_arrays_size!(
    CLIENT_OPTIONAL_CORE_DATA_TO_HIGH_COLOR_DEPTH_BUFFER_BUFFER,
    CLIENT_OPTIONAL_CORE_DATA_FROM_HIGH_COLOR_DEPTH_TO_SERVER_SELECTED_PROTOCOL_BUFFER
)] = concat_arrays!(
    CLIENT_OPTIONAL_CORE_DATA_TO_HIGH_COLOR_DEPTH_BUFFER_BUFFER,
    CLIENT_OPTIONAL_CORE_DATA_FROM_HIGH_COLOR_DEPTH_TO_SERVER_SELECTED_PROTOCOL_BUFFER
);

pub const CLIENT_OPTIONAL_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_BUFFER: [u8; concat_arrays_size!(
    CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL_BUFFER,
    CLIENT_OPTIONAL_CORE_DATA_FROM_DESKTOP_PHYSICAL_WIDTH_TO_DEVICE_SCALE_FACTOR_BUFFER
)] = concat_arrays!(
    CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL_BUFFER,
    CLIENT_OPTIONAL_CORE_DATA_FROM_DESKTOP_PHYSICAL_WIDTH_TO_DEVICE_SCALE_FACTOR_BUFFER
);

pub const SERVER_CORE_DATA_BUFFER: [u8; 4] = [
    0x04, 0x00, 0x08, 0x00, // version
];

pub const REQUESTED_PROTOCOL_BUFFER: [u8; 4] = [
    0x00, 0x00, 0x00, 0x00, // client requested protocols
];

pub const FLAGS_BUFFER: [u8; 4] = [
    0x01, 0x00, 0x00, 0x00, // early capability flags
];

lazy_static! {
    pub static ref SERVER_CORE_DATA: ServerCoreData = ServerCoreData {
        version: RdpVersion::V5_PLUS,
        optional_data: ServerCoreOptionalData {
            client_requested_protocols: None,
            early_capability_flags: None,
        },
    };
    pub static ref SERVER_CORE_DATA_TO_FLAGS: ServerCoreData = ServerCoreData {
        version: RdpVersion::V5_PLUS,
        optional_data: ServerCoreOptionalData {
            client_requested_protocols: Some(SecurityProtocol::empty()),
            early_capability_flags: None,
        },
    };
    pub static ref SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS: ServerCoreData = ServerCoreData {
        version: RdpVersion::V5_PLUS,
        optional_data: ServerCoreOptionalData {
            client_requested_protocols: Some(SecurityProtocol::empty()),
            early_capability_flags: Some(ServerEarlyCapabilityFlags::EDGE_ACTIONS_SUPPORTED_V1),
        },
    };
}

pub const SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER: [u8; concat_arrays_size!(
    SERVER_CORE_DATA_BUFFER,
    REQUESTED_PROTOCOL_BUFFER
)] = concat_arrays!(SERVER_CORE_DATA_BUFFER, REQUESTED_PROTOCOL_BUFFER);

pub const SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_BUFFER: [u8; concat_arrays_size!(
    SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER,
    FLAGS_BUFFER
)] = concat_arrays!(SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER, FLAGS_BUFFER);
