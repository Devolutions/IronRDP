use ironrdp_pdu::gcc::*;

pub const CLIENT_NETWORK_DATA_WITH_CHANNELS_BUFFER: [u8; 40] = [
    0x03, 0x00, 0x00, 0x00, // channels count
    0x72, 0x64, 0x70, 0x64, 0x72, 0x00, 0x00, 0x00, // channel 1::name
    0x00, 0x00, 0x80, 0x80, // channel 1::options
    0x63, 0x6c, 0x69, 0x70, 0x72, 0x64, 0x72, 0x00, // channel 2::name
    0x00, 0x00, 0xa0, 0xc0, // channel 2::options
    0x72, 0x64, 0x70, 0x73, 0x6e, 0x64, 0x00, 0x00, // channel 3::name
    0x00, 0x00, 0x00, 0xc0, // channel 3::options
];

pub const SERVER_NETWORK_DATA_WITH_CHANNELS_ID_BUFFER: [u8; 12] = [
    0xeb, 0x03, // io channel
    0x03, 0x00, // channels count
    0xec, 0x03, // channel 1::id
    0xed, 0x03, // channel 2::id
    0xee, 0x03, // channel 3::id
    0x00, 0x00, // padding
];

pub const CLIENT_NETWORK_DATA_WITHOUT_CHANNELS_BUFFER: [u8; 4] = [
    0x00, 0x00, 0x00, 0x00, // channels count
];

pub const SERVER_NETWORK_DATA_WITHOUT_CHANNELS_ID_BUFFER: [u8; 4] = [
    0xeb, 0x03, // io channel
    0x00, 0x00, // channels count
];

lazy_static! {
    pub static ref CLIENT_NETWORK_DATA_WITH_CHANNELS: ClientNetworkData = ClientNetworkData {
        channels: vec![
            ChannelDef {
                name: ChannelName::from_utf8("rdpdr").unwrap(),
                options: ChannelOptions::INITIALIZED | ChannelOptions::COMPRESS_RDP,
            },
            ChannelDef {
                name: ChannelName::from_utf8("cliprdr").unwrap(),
                options: ChannelOptions::INITIALIZED
                    | ChannelOptions::COMPRESS_RDP
                    | ChannelOptions::ENCRYPT_RDP
                    | ChannelOptions::SHOW_PROTOCOL,
            },
            ChannelDef {
                name: ChannelName::from_utf8("rdpsnd").unwrap(),
                options: ChannelOptions::INITIALIZED | ChannelOptions::ENCRYPT_RDP,
            },
        ],
    };
    pub static ref SERVER_NETWORK_DATA_WITH_CHANNELS_ID: ServerNetworkData = ServerNetworkData {
        io_channel: 1003,
        channel_ids: vec![1004, 1005, 1006],
    };
    pub static ref CLIENT_NETWORK_DATA_WITHOUT_CHANNELS: ClientNetworkData = ClientNetworkData { channels: Vec::new() };
    pub static ref SERVER_NETWORK_DATA_WITHOUT_CHANNELS_ID: ServerNetworkData = ServerNetworkData {
        io_channel: 1003,
        channel_ids: Vec::new(),
    };
}
