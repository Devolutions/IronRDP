use crate::pdu::{ClipboardFormat, ClipboardFormatId};

use tracing::error;
use winapi::um::{errhandlingapi::GetLastError, winuser::RegisterClipboardFormatW};

use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct RemoteClipboardFormatRegistry {
    remote_to_local: HashMap<ClipboardFormatId, ClipboardFormatId>,
    local_to_remote: HashMap<ClipboardFormatId, ClipboardFormatId>,
}

impl RemoteClipboardFormatRegistry {
    /// Clear current format mapping, as per RDP spec, format mapping is reset on every
    /// received `CLIPRDR_FORMAT_LIST` message.
    pub fn clear(&mut self) {
        self.remote_to_local.clear();
        self.local_to_remote.clear();
    }

    /// Registers remote clipboard format on local machine. Registered format ids could differ
    /// from remote format ids, so we need to keep track of them based on their names. Standard
    /// formats such as `CF_TEXT` have fixed ids, which are same on all machines, the retuned
    /// id value will be same as remote format id.
    ///
    /// E.g.: Format with name `Custom` was registered as `0xC001`, on the remote, but on the local
    /// machine it was registered as `0xC002`. When we receive format list from the remote, we need to
    /// get the local format id for `Custom` format, and save format mapping to bi-directional map.
    /// When the local machine requests data for `0xC002` format, we will find its mapping for
    /// remote format id `0xC001` and send data for it.
    ///
    /// Returns local format id for the remote format id.
    /// If the format is unknown or not supported on the local machine, returns `None`.
    pub fn register(&mut self, remote_format: &ClipboardFormat) -> Option<ClipboardFormatId> {
        if remote_format.id().is_standard() {
            // Standard formats such as `CF_TEXT` have fixed ids, which are same on all machines.
            return Some(remote_format.id());
        }

        if remote_format.id().is_private() {
            // Private app-specific formats sould not be transferred between machines
            return None;
        }

        if !remote_format.id().is_registrered() {
            // Unknown format range, we could skip it
            return None;
        }

        // Try to register format on the local machine

        let format_name = remote_format.name()?;

        // Make null-terminated UTF-16 format representation
        let format_name_utf16 = format_name
            .value()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();

        let mapped_format_id = ClipboardFormatId::new(unsafe { RegisterClipboardFormatW(format_name_utf16.as_ptr()) });

        if mapped_format_id.value() == 0 {
            let error = unsafe { GetLastError() };
            error!(
                "Failed to register clipboard format `{}`, Error code: {}",
                format_name.value(),
                error
            );
            // Error is not critical, format could be skipped
            return None;
        }

        // save mapping for future use
        self.remote_to_local.insert(remote_format.id(), mapped_format_id);
        self.local_to_remote.insert(mapped_format_id, remote_format.id());

        // We either registered new format or found previously registered one
        Some(mapped_format_id)
    }

    pub fn local_to_remote(&self, local_format: ClipboardFormatId) -> Option<ClipboardFormatId> {
        if local_format.is_standard() {
            return Some(local_format);
        }

        self.local_to_remote.get(&local_format).copied()
    }
}
