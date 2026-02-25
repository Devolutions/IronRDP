use crate::{IDDCX_ADAPTER, NTSTATUS, STATUS_SUCCESS};

pub fn set_display_config(
    _adapter: IDDCX_ADAPTER,
    paths: &[crate::adapter::IDDCX_PATH],
) -> NTSTATUS {
    let mut changed_paths = 0u32;
    let mut active_paths = 0u32;

    for path in paths {
        if (path.Flags & 1) != 0 {
            changed_paths = changed_paths.saturating_add(1);
        }

        if (path.Flags & 2) != 0 {
            active_paths = active_paths.saturating_add(1);
        }
    }

    tracing::info!(
        path_count = paths.len(),
        changed_paths,
        active_paths,
        "IddCxAdapterDisplayConfigUpdate applied"
    );
    STATUS_SUCCESS
}

pub fn handle_session_transition(_adapter: IDDCX_ADAPTER, is_remote: bool) {
    tracing::info!(is_remote, "remote session transition (stub)");
}
