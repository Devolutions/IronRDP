use std::sync::LazyLock;

use ironrdp_pdu::gcc::{ClientClusterData, RedirectionFlags, RedirectionVersion};

pub const CLUSTER_DATA_BUFFER: [u8; 8] = [0x0d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];

pub static CLUSTER_DATA: LazyLock<ClientClusterData> = LazyLock::new(|| ClientClusterData {
    flags: RedirectionFlags::REDIRECTION_SUPPORTED,
    redirection_version: RedirectionVersion::V4,
    redirected_session_id: 0,
});
