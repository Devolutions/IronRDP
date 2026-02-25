use crate::{IDDCX_ADAPTER, NTSTATUS, STATUS_NOT_SUPPORTED};

#[repr(C)]
pub struct DISPLAYCONFIG_PATH_INFO {
    _private: [u8; 0],
}

#[repr(C)]
pub struct DISPLAYCONFIG_MODE_INFO {
    _private: [u8; 0],
}

pub fn set_display_config(
    _adapter: IDDCX_ADAPTER,
    _paths: &[DISPLAYCONFIG_PATH_INFO],
    _modes: &[DISPLAYCONFIG_MODE_INFO],
) -> NTSTATUS {
    tracing::info!("IddCxAdapterDisplayConfigUpdate (stub)");
    STATUS_NOT_SUPPORTED
}

pub fn handle_session_transition(_adapter: IDDCX_ADAPTER, is_remote: bool) {
    tracing::info!(is_remote, "remote session transition (stub)");
}
