use ironrdp_pdu::gcc::{MultiTransportChannelData, MultiTransportFlags};

pub const SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER: [u8; 4] = [0x01, 0x03, 0x00, 0x00];

lazy_static! {
    pub static ref SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK: MultiTransportChannelData = MultiTransportChannelData {
        flags: MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR
            | MultiTransportFlags::TRANSPORT_TYPE_UDP_PREFERRED
            | MultiTransportFlags::SOFT_SYNC_TCP_TO_UDP,
    };
}
