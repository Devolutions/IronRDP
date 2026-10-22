//! RemoteApp notification area icons ([MS-RDPERP] 2.2.1.3.2 Notification Icon and
//! 2.2.2.6.4 Client Notify Event PDU).
//!
//! Each server notification icon becomes a local `Shell_NotifyIconW` icon owned by a
//! hidden message-only window. Callback messages from the shell are reported back to
//! the server as Client Notify Event PDUs.
//!
//! [MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp

use core::cell::RefCell;
use core::ffi::c_void;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::Mutex;

use ironrdp_client::rail::RailInputEvent;
use ironrdp_client::rdp::RdpInputSender;
use ironrdp_pdu::window::{NotifyIconOrder, NotifyIconState};
use ironrdp_rail::pdu::{NotifyEventMessage, NotifyEventPdu};
use windows::Win32::Foundation::{ERROR_CLASS_DOES_NOT_EXIST, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_STATE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NIS_HIDDEN, NOTIFY_ICON_INFOTIP_FLAGS, NOTIFY_ICON_MESSAGE, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW, HWND_MESSAGE,
    RegisterClassW, SetWindowLongPtrW, UnregisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_NCCREATE, WNDCLASSW,
};
use windows::core::w;
use windows_core::{Error, HRESULT, PCWSTR};

use crate::rail_icons::{OwnedIcon, RailIconCache};

const TRAY_WINDOW_CLASS: PCWSTR = w!("IronRDP.ActiveX.RailTray");
/// Callback message of icons using the callback format before `NOTIFYICON_VERSION_4`.
const TRAY_CALLBACK_MESSAGE: u32 = WM_APP + 0x152;
/// Callback message of icons using the `NOTIFYICON_VERSION_4` format.
///
/// Each format has its own message, because the icon ID sits in `wParam` in one and in
/// `lParam` in the other: decoding a callback with the wrong format could name another icon.
const TRAY_CALLBACK_MESSAGE_V4: u32 = WM_APP + 0x153;
const MAX_TRAY_ICONS: usize = 256;

static TRAY_CLASS_USERS: Mutex<usize> = Mutex::new(0);

/// Decodes a notification icon callback into the icon's local ID and its message.
///
/// With `NOTIFYICON_VERSION_4` the low word of `lParam` is the message and its high
/// word the icon ID; earlier versions pass the ID in `wParam` and the message in `lParam`.
pub(crate) fn tray_callback_event(version: u32, wparam: WPARAM, lparam: LPARAM) -> Option<(u32, NotifyEventMessage)> {
    let lparam = lparam.0 as usize;
    let (id, message) = if NOTIFYICON_VERSION_4 <= version {
        ((lparam >> 16) & 0xffff, lparam & 0xffff)
    } else {
        (wparam.0, lparam & 0xffff_ffff)
    };
    let id = u32::try_from(id).ok()?;
    let message = NotifyEventMessage::try_from(u32::try_from(message).ok()?).ok()?;
    Some((id, message))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ServerIconId {
    window_id: u32,
    notify_icon_id: u32,
}

struct TrayIcon {
    local_id: u32,
    version: u32,
    _icon: Option<Rc<OwnedIcon>>,
}

/// State shared with the message-only window procedure.
///
/// The procedure runs while `RailTray` applies orders (the shell can call back from
/// `Shell_NotifyIconW`), so it keeps its own local-ID index instead of borrowing the tray.
struct TrayContext {
    input_sender: RdpInputSender,
    /// Local icon ID to the server's icon.
    icons: RefCell<BTreeMap<u32, ServerIconId>>,
}

/// The notification icons of one RemoteApp session.
pub(crate) struct RailTray {
    hwnd: HWND,
    context: Box<TrayContext>,
    icons: BTreeMap<ServerIconId, TrayIcon>,
    next_local_id: u32,
}

impl RailTray {
    pub(crate) fn new(input_sender: RdpInputSender) -> windows_core::Result<Self> {
        acquire_tray_class()?;
        let mut context = Box::new(TrayContext {
            input_sender,
            icons: RefCell::new(BTreeMap::new()),
        });
        // SAFETY: the class is registered; `context` outlives the window, which
        // `Drop` destroys before the box is freed.
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                TRAY_WINDOW_CLASS,
                PCWSTR::null(),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                Some((&mut *context as *mut TrayContext).cast::<c_void>().cast_const()),
            )
        };
        match hwnd {
            Ok(hwnd) => Ok(Self {
                hwnd,
                context,
                icons: BTreeMap::new(),
                next_local_id: 1,
            }),
            Err(error) => {
                release_tray_class();
                Err(error)
            }
        }
    }

    /// Applies a Notification Icon order.
    pub(crate) fn apply(&mut self, order: NotifyIconOrder, cache: &mut RailIconCache) {
        let key = ServerIconId {
            window_id: order.window_id,
            notify_icon_id: order.notify_icon_id,
        };
        let Some(state) = order.state else {
            self.remove(key);
            return;
        };

        let icon = state.icon.as_ref().and_then(|source| cache.resolve(source));
        let existing = self.icons.get(&key).map(|icon| (icon.local_id, icon.version));
        let (local_id, version, adding) = match existing {
            // The callback format is fixed when the icon is added.
            Some((local_id, version)) => (local_id, version, false),
            None => {
                if MAX_TRAY_ICONS <= self.icons.len() {
                    tracing::debug!(?key, "Ignoring RAIL notification icon beyond the local limit");
                    return;
                }
                let local_id = self.next_local_id;
                self.next_local_id = self.next_local_id.wrapping_add(1).max(1);
                (local_id, state.version.unwrap_or(0), true)
            }
        };

        let mut data = notify_icon_data(self.hwnd, local_id, &state, icon.as_deref());
        let message = if adding { NIM_ADD } else { NIM_MODIFY };
        if adding {
            data.uFlags |= NIF_MESSAGE;
            data.uCallbackMessage = if NOTIFYICON_VERSION_4 <= version {
                TRAY_CALLBACK_MESSAGE_V4
            } else {
                TRAY_CALLBACK_MESSAGE
            };
        }
        if !notify(message, &data) {
            tracing::debug!(?key, adding, "Unable to update a RAIL notification icon");
            if adding {
                return;
            }
        }
        if adding && version != 0 {
            data.Anonymous.uVersion = version;
            if !notify(NIM_SETVERSION, &data) {
                tracing::debug!(?key, version, "Unable to set the RAIL notification icon version");
            }
        }

        self.context.icons.borrow_mut().insert(local_id, key);
        let previous_icon = self.icons.remove(&key).and_then(|icon| icon._icon);
        self.icons.insert(
            key,
            TrayIcon {
                local_id,
                version,
                // Keep the previous icon when this order did not replace it.
                _icon: icon.or(previous_icon),
            },
        );
    }

    /// Removes every notification icon.
    pub(crate) fn clear(&mut self) {
        for key in self.icons.keys().copied().collect::<Vec<_>>() {
            self.remove(key);
        }
    }

    fn remove(&mut self, key: ServerIconId) {
        let Some(icon) = self.icons.remove(&key) else {
            return;
        };
        self.context.icons.borrow_mut().remove(&icon.local_id);
        let data = NOTIFYICONDATAW {
            cbSize: notify_icon_data_size(),
            hWnd: self.hwnd,
            uID: icon.local_id,
            ..Default::default()
        };
        if !notify(NIM_DELETE, &data) {
            tracing::debug!(?key, "Unable to remove a RAIL notification icon");
        }
    }
}

impl Drop for RailTray {
    fn drop(&mut self) {
        self.clear();
        // SAFETY: the window belongs to this thread and holds a pointer to `context`.
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            if let Err(error) = DestroyWindow(self.hwnd) {
                tracing::debug!(?error, "Unable to destroy the RAIL notification window");
            }
        }
        release_tray_class();
    }
}

fn notify_icon_data_size() -> u32 {
    u32::try_from(size_of::<NOTIFYICONDATAW>()).unwrap_or(u32::MAX)
}

fn notify_icon_data(hwnd: HWND, local_id: u32, state: &NotifyIconState, icon: Option<&OwnedIcon>) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: notify_icon_data_size(),
        hWnd: hwnd,
        uID: local_id,
        ..Default::default()
    };
    if let Some(icon) = icon {
        data.uFlags |= NIF_ICON;
        data.hIcon = icon.handle();
    }
    if let Some(tooltip) = &state.tooltip {
        data.uFlags |= NIF_TIP | NIF_SHOWTIP;
        copy_truncated(&mut data.szTip, tooltip);
    }
    if let Some(info_tip) = &state.info_tip {
        data.uFlags |= NIF_INFO;
        copy_truncated(&mut data.szInfo, &info_tip.text);
        copy_truncated(&mut data.szInfoTitle, &info_tip.title);
        data.Anonymous.uTimeout = info_tip.timeout;
        data.dwInfoFlags = NOTIFY_ICON_INFOTIP_FLAGS(info_tip.info_flags);
    }
    if let Some(icon_state) = state.state {
        data.uFlags |= NIF_STATE;
        data.dwStateMask = NIS_HIDDEN;
        if icon_state & 1 != 0 {
            data.dwState = NIS_HIDDEN;
        }
    }
    data
}

/// Copies `value` into a fixed UTF-16 buffer, truncating it and keeping the terminator.
fn copy_truncated(buffer: &mut [u16], value: &str) {
    let capacity = buffer.len().saturating_sub(1);
    let mut length = 0;
    for (slot, unit) in buffer.iter_mut().zip(value.encode_utf16().take(capacity)) {
        *slot = unit;
        length += 1;
    }
    if let Some(terminator) = buffer.get_mut(length) {
        *terminator = 0;
    }
}

fn notify(message: NOTIFY_ICON_MESSAGE, data: &NOTIFYICONDATAW) -> bool {
    // SAFETY: `data` is a fully initialized NOTIFYICONDATAW with its size set.
    unsafe { Shell_NotifyIconW(message, data) }.as_bool()
}

fn handle_tray_message(context: &TrayContext, message: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
    let version = match message {
        TRAY_CALLBACK_MESSAGE => 0,
        TRAY_CALLBACK_MESSAGE_V4 => NOTIFYICON_VERSION_4,
        _ => return false,
    };
    let Some((server_id, message)) = tray_callback_event(version, wparam, lparam)
        .and_then(|(local_id, message)| Some((context.icons.borrow().get(&local_id).copied()?, message)))
    else {
        return true;
    };
    let event = RailInputEvent::NotifyEvent(NotifyEventPdu {
        window_id: server_id.window_id,
        notify_icon_id: server_id.notify_icon_id,
        message,
    });
    if let Err(error) = context.input_sender.try_send_rail_input(event) {
        tracing::debug!(?error, "Unable to forward a RAIL notification icon event");
    }
    true
}

unsafe extern "system" fn tray_window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match catch_unwind(AssertUnwindSafe(|| unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let context = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const TrayContext;
        if !context.is_null() && handle_tray_message(&*context, message, wparam, lparam) {
            LRESULT(0)
        } else {
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
    })) {
        Ok(result) => result,
        Err(_) => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn acquire_tray_class() -> windows_core::Result<()> {
    let mut users = TRAY_CLASS_USERS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if *users == 0 {
        // SAFETY: querying this process's module handle has no preconditions.
        let instance = unsafe { GetModuleHandleW(None) }?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(tray_window_proc),
            hInstance: HINSTANCE(instance.0),
            lpszClassName: TRAY_WINDOW_CLASS,
            ..Default::default()
        };
        // SAFETY: `class` is fully initialized.
        if unsafe { RegisterClassW(&class) } == 0 {
            // SAFETY: reads the calling thread's last error.
            let error = unsafe { GetLastError() };
            return Err(Error::from_hresult(HRESULT::from_win32(error.0)));
        }
    }
    *users += 1;
    Ok(())
}

fn release_tray_class() {
    let mut users = TRAY_CLASS_USERS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if *users == 0 {
        return;
    }
    *users -= 1;
    if *users != 0 {
        return;
    }
    // SAFETY: no tray window of this class remains.
    let result = unsafe { GetModuleHandleW(None) }
        .and_then(|instance| unsafe { UnregisterClassW(TRAY_WINDOW_CLASS, Some(HINSTANCE(instance.0))) });
    match result {
        Ok(()) => {}
        Err(error) if error.code() == HRESULT::from_win32(ERROR_CLASS_DOES_NOT_EXIST.0) => {}
        Err(error) => tracing::debug!(?error, "Unable to unregister the RAIL notification window class"),
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::WindowsAndMessaging::{WM_CONTEXTMENU, WM_LBUTTONUP};

    use super::*;

    // oracle: reference Win32 Shell_NotifyIconW NOTIFYICON_VERSION_4 callback: LOWORD(lParam) is the event, HIWORD(lParam) the icon ID
    #[test]
    fn version_4_callbacks_carry_the_id_in_the_high_word() {
        let lparam = LPARAM(((7 << 16) | WM_CONTEXTMENU) as isize);

        assert_eq!(
            tray_callback_event(NOTIFYICON_VERSION_4, WPARAM(0x0010_0020), lparam),
            Some((7, NotifyEventMessage::ContextMenu))
        );
    }

    // oracle: reference Win32 Shell_NotifyIconW callback before NOTIFYICON_VERSION_4: wParam is the icon ID, lParam the mouse message
    #[test]
    fn legacy_callbacks_carry_the_id_in_wparam() {
        assert_eq!(
            tray_callback_event(0, WPARAM(9), LPARAM(WM_LBUTTONUP as isize)),
            Some((9, NotifyEventMessage::LeftButtonUp))
        );
    }

    // oracle: spec MS-RDPERP §2.2.2.6.4 NIN_BALLOONUSERCLICK = 0x0405 = WM_USER + 5
    #[test]
    fn balloon_notifications_map_to_rail_messages() {
        // NIN_BALLOONUSERCLICK is WM_USER + 5.
        assert_eq!(
            tray_callback_event(NOTIFYICON_VERSION_4, WPARAM(0), LPARAM((3 << 16) | 0x0405)),
            Some((3, NotifyEventMessage::BalloonUserClick))
        );
    }

    // oracle: spec MS-RDPERP §2.2.2.6.4 lists no WM_MOUSEMOVE (0x0200) message
    #[test]
    fn mouse_moves_are_not_reported() {
        assert_eq!(tray_callback_event(0, WPARAM(1), LPARAM(0x0200)), None);
    }

    // oracle: reference Win32 NOTIFYICONDATAW szTip is a NUL-terminated fixed buffer
    #[test]
    fn long_tooltips_are_truncated_with_a_terminator() {
        let mut buffer = [0xffffu16; 4];
        copy_truncated(&mut buffer, "abcdef");

        assert_eq!(buffer, [u16::from(b'a'), u16::from(b'b'), u16::from(b'c'), 0]);
    }
}
