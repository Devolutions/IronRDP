use std::path::{Path, PathBuf};
use std::sync::OnceLock;

mod gateway;
mod gateway_rpch;
mod keylog;

fn capture_replay_binary() -> &'static Path {
    static CAPTURE_REPLAY_BINARY: OnceLock<PathBuf> = OnceLock::new();

    CAPTURE_REPLAY_BINARY
        .get_or_init(|| super::workspace_binary("ironrdp-capture-replay", "ironrdp-capture-replay"))
        .as_path()
}
