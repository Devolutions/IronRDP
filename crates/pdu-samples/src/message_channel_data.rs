use ironrdp_pdu::gcc::ServerMessageChannelData;

pub const SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER: [u8; 2] = [0xf0, 0x03];

pub const SERVER_GCC_MESSAGE_CHANNEL_BLOCK: ServerMessageChannelData = ServerMessageChannelData {
    mcs_message_channel_id: 0x03f0,
};
