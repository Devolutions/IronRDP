use std::collections::HashMap;

use ironrdp_cliprdr::pdu::{ClipboardFormat, ClipboardFormatId};
use tracing::error;
use windows::core::PCWSTR;
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;

use crate::windows::utils::get_last_winapi_error;

#[derive(Debug, Default)]
pub(crate) struct RemoteClipboardFormatRegistry {
    remote_to_local: HashMap<ClipboardFormatId, ClipboardFormatId>,
    local_to_remote: HashMap<ClipboardFormatId, ClipboardFormatId>,
}

impl RemoteClipboardFormatRegistry {
    /// Clear current format mapping, as per RDP spec, format mapping is reset on every
    /// received `CLIPRDR_FORMAT_LIST` message.
    pub(crate) fn clear(&mut self) {
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
    pub(crate) fn register(&mut self, remote_format: &ClipboardFormat) -> Option<ClipboardFormatId> {
        if remote_format.id().is_standard() {
            // Standard formats such as `CF_TEXT` have fixed ids, which are same on all machines.
            return Some(remote_format.id());
        }

        if is_private_format_id(remote_format.id()) {
            // Private app-specific formats should not be normally transferred between machines
            return None;
        }

        if !remote_format.id().is_registered() {
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

        let format_name_pcwstr = PCWSTR::from_raw(format_name_utf16.as_ptr());

        // SAFETY: `RegisterClipboardFormatW` is always safe to call.
        let raw_format_id = unsafe { RegisterClipboardFormatW(format_name_pcwstr) };

        let mapped_format_id = ClipboardFormatId::new(raw_format_id);

        if mapped_format_id.value() == 0 {
            let error_code = get_last_winapi_error().0;
            error!(
                "Failed to register clipboard format `{}`, Error code: {}",
                format_name.value(),
                error_code
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

    pub(crate) fn local_to_remote(&self, local_format: ClipboardFormatId) -> Option<ClipboardFormatId> {
        if local_format.is_standard() {
            return Some(local_format);
        }

        self.local_to_remote.get(&local_format).copied()
    }
}

fn is_private_format_id(format: ClipboardFormatId) -> bool {
    // Private Windows format ranges which should not be transferred between machines
    const CF_PRIVATEFIRST: u32 = 0x0200;
    const CF_PRIVATELAST: u32 = 0x02FF;

    const CF_GDIOBJFIRST: u32 = 0x0300;
    const CF_GDIOBJLAST: u32 = 0x03FF;

    let id = format.value();

    let private_range = (CF_PRIVATEFIRST..=CF_PRIVATELAST).contains(&id);
    let gdi_range = (CF_GDIOBJFIRST..=CF_GDIOBJLAST).contains(&id);

    private_range || gdi_range
}
