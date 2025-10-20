use std::sync::LazyLock;

use ironrdp_pdu::gcc::{MultiTransportChannelData, MultiTransportFlags};

pub const SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER: [u8; 4] = [0x01, 0x03, 0x00, 0x00];

pub static SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK: LazyLock<MultiTransportChannelData> =
    LazyLock::new(|| MultiTransportChannelData {
        flags: MultiTransportFlags::TRANSPORT_TYPE_UDP_FECR
            | MultiTransportFlags::TRANSPORT_TYPE_UDP_PREFERRED
            | MultiTransportFlags::SOFT_SYNC_TCP_TO_UDP,
    });
