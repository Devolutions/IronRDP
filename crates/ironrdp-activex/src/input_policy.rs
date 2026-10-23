//! Keyboard hotkeys, focus release and local input policy of the ActiveX renderer.
//!
//! Defaults, bindings and gates follow mstscax: its property table supplies the default virtual-key
//! codes, `CTSInput::IHCheckForHotkey` translates the Alt replacement hotkeys, and
//! `CTSInput::IHPostMessageToMainWindow` handles full screen, focus release and the relative mouse toggle.

use ironrdp_pdu::input::fast_path::FastPathInputEvent;
use ironrdp_pdu::input::mouse_rel::{MouseRelPdu, PointerRelFlags};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VK_ADD, VK_DELETE, VK_DOWN, VK_END, VK_HOME, VK_INSERT, VK_LEFT, VK_NEXT, VK_OEM_5, VK_PRIOR, VK_RIGHT,
    VK_SUBTRACT, VK_UP,
};
use windows::Win32::UI::Input::{
    GetRawInputData, HRAWINPUT, MOUSE_MOVE_ABSOLUTE, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RID_INPUT,
    RIDEV_CAPTUREMOUSE, RIDEV_NOLEGACY, RIDEV_REMOVE, RIM_TYPEMOUSE, RegisterRawInputDevices,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ClipCursor, RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_BUTTON_4_UP, RI_MOUSE_BUTTON_5_DOWN, RI_MOUSE_BUTTON_5_UP,
    RI_MOUSE_HWHEEL, RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP, RI_MOUSE_MIDDLE_BUTTON_DOWN,
    RI_MOUSE_MIDDLE_BUTTON_UP, RI_MOUSE_RIGHT_BUTTON_DOWN, RI_MOUSE_RIGHT_BUTTON_UP, RI_MOUSE_WHEEL, SetCursorPos,
    ShowCursor, WM_SYSCHAR,
};

use super::*;

/// `IMsTscAxEvents::OnFocusReleased`; its argument is the release direction.
const DISPID_ON_FOCUS_RELEASED: i32 = 23;
/// `IMsTscAxEvents::OnMouseInputModeChanged(VARIANT_BOOL fMouseModeRelative)`.
const DISPID_ON_MOUSE_INPUT_MODE_CHANGED: i32 = 26;

/// `RNS_US_SAS_DEL_ALT_CTRL`, the only secure attention sequence the hotkey translation sends.
pub(super) const SAS_SEQUENCE_CTRL_ALT_DEL: i32 = 0xAA03;

/// mstscax clamps `SuperPanAccelerationFactor` to this value; it is also the factor's default.
const DEFAULT_SUPER_PAN_ACCELERATION_FACTOR: u32 = 1;

const LEFT_CONTROL: Scancode = Scancode::from_u8(false, 0x1d);
const RIGHT_CONTROL: Scancode = Scancode::from_u8(true, 0x1d);
const LEFT_ALT: Scancode = Scancode::from_u8(false, 0x38);
const RIGHT_ALT: Scancode = Scancode::from_u8(true, 0x38);
const LEFT_SHIFT: Scancode = Scancode::from_u8(false, 0x2a);
const ESCAPE: Scancode = Scancode::from_u8(false, 0x01);
const TAB: Scancode = Scancode::from_u8(false, 0x0f);
const SPACE: Scancode = Scancode::from_u8(false, 0x39);
const DELETE: Scancode = Scancode::from_u8(true, 0x53);
/// mstscax injects Print Screen as the Alt+SysRq scancode, with and without Alt.
const SYSTEM_REQUEST: Scancode = Scancode::from_u8(false, 0x54);

/// The virtual-key codes of the `IMsRdpClientAdvancedSettings` hotkeys.
///
/// Full screen, Ctrl+Alt+Del and focus release combine the key with Ctrl+Alt; the others replace an
/// Alt combination that Windows keeps for itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Hotkeys {
    pub(super) full_screen: u32,
    pub(super) ctrl_esc: u32,
    pub(super) alt_esc: u32,
    pub(super) alt_tab: u32,
    pub(super) alt_shift_tab: u32,
    pub(super) alt_space: u32,
    pub(super) ctrl_alt_del: u32,
    pub(super) focus_release_left: u32,
    pub(super) focus_release_right: u32,
    pub(super) focus_release_up: u32,
    pub(super) focus_release_down: u32,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            full_screen: u32::from(VK_CANCEL.0),
            ctrl_esc: u32::from(VK_HOME.0),
            alt_esc: u32::from(VK_INSERT.0),
            alt_tab: u32::from(VK_PRIOR.0),
            alt_shift_tab: u32::from(VK_NEXT.0),
            alt_space: u32::from(VK_DELETE.0),
            ctrl_alt_del: u32::from(VK_END.0),
            focus_release_left: u32::from(VK_LEFT.0),
            focus_release_right: u32::from(VK_RIGHT.0),
            focus_release_up: u32::from(VK_UP.0),
            focus_release_down: u32::from(VK_DOWN.0),
        }
    }
}

/// Host-configured local keyboard and mouse policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct InputPolicy {
    pub(super) hotkeys: Hotkeys,
    /// `AcceleratorPassthrough`: Alt key messages go to the session instead of the container.
    pub(super) accelerator_passthrough: bool,
    /// `AllowAxToContainerEvents`: the focus-release hotkeys raise `OnFocusReleased`.
    pub(super) focus_release_events: bool,
    /// `DisableFullScreenHotkey`.
    pub(super) full_screen_hotkey_disabled: bool,
    /// `AllowSasSequenceTransmission`: the Ctrl+Alt+Del hotkey sends Ctrl+Alt+Del.
    pub(super) sas_transmission: bool,
    /// `AllowRelativeMouseMode`: Ctrl+Alt+\ switches the mouse to relative pointer events.
    pub(super) relative_mouse_allowed: bool,
    /// `IgnoreClientSideMouseInput`.
    pub(super) ignore_client_mouse_input: bool,
}

impl Default for InputPolicy {
    fn default() -> Self {
        Self {
            hotkeys: Hotkeys::default(),
            accelerator_passthrough: true,
            focus_release_events: false,
            full_screen_hotkey_disabled: false,
            sas_transmission: true,
            relative_mouse_allowed: false,
            ignore_client_mouse_input: false,
        }
    }
}

/// A key press as the hotkey translation sees it.
#[derive(Clone, Copy, Debug)]
pub(super) struct HotkeyInput {
    pub(super) virtual_key: u16,
    /// The key comes from the navigation cluster rather than the numeric keypad.
    pub(super) extended: bool,
    pub(super) control: bool,
    pub(super) alt: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FocusReleaseDirection {
    Left,
    Right,
    Up,
    Down,
}

impl FocusReleaseDirection {
    /// The `OnFocusReleased` argument mstscax passes for this direction.
    pub(super) fn event_argument(self) -> i32 {
        match self {
            Self::Left => -1,
            Self::Right => 1,
            Self::Up => 2,
            Self::Down => -2,
        }
    }
}

/// A key combination the control sends in place of a hotkey.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyInjection {
    /// Alt+Home: releases Alt and sends Ctrl+Esc.
    CtrlEsc,
    /// Alt+Insert: sends Esc while Alt stays down.
    AltEsc,
    /// Alt+Page Up: sends Tab while Alt stays down.
    AltTab,
    /// Alt+Page Down: sends Shift+Tab while Alt stays down.
    AltShiftTab,
    /// Alt+Delete: sends Space while Alt stays down.
    AltSpace,
    /// Ctrl+Alt+End: sends Delete while Ctrl+Alt stay down.
    CtrlAltDel,
    /// Ctrl+Alt+Minus on the keypad: releases Ctrl and sends Alt+Print Screen.
    AltPrintScreen,
    /// Ctrl+Alt+Plus on the keypad: releases Ctrl and Alt and sends Print Screen.
    PrintScreen,
}

impl KeyInjection {
    /// The key operations to apply, given which keys the server currently sees as pressed.
    ///
    /// Modifiers released for the injection come back down afterwards, as in mstscax, except Alt
    /// for Ctrl+Esc, which stays up until the user presses it again.
    pub(super) fn operations(self, is_pressed: impl Fn(Scancode) -> bool) -> Vec<Operation> {
        let tap = |key| [Operation::KeyPressed(key), Operation::KeyReleased(key)];
        let held = |keys: &[Scancode]| keys.iter().copied().filter(|key| is_pressed(*key)).collect::<Vec<_>>();
        let around = |modifiers: Vec<Scancode>, inner: &[Operation]| {
            modifiers
                .iter()
                .map(|key| Operation::KeyReleased(*key))
                .chain(inner.iter().cloned())
                .chain(modifiers.iter().rev().map(|key| Operation::KeyPressed(*key)))
                .collect()
        };

        match self {
            Self::CtrlEsc => [LEFT_ALT, RIGHT_ALT]
                .into_iter()
                .filter(|key| is_pressed(*key))
                .map(Operation::KeyReleased)
                .chain([Operation::KeyPressed(LEFT_CONTROL)])
                .chain(tap(ESCAPE))
                .chain([Operation::KeyReleased(LEFT_CONTROL)])
                .collect(),
            Self::AltEsc => tap(ESCAPE).to_vec(),
            Self::AltTab => tap(TAB).to_vec(),
            Self::AltShiftTab => [Operation::KeyPressed(LEFT_SHIFT)]
                .into_iter()
                .chain(tap(TAB))
                .chain([Operation::KeyReleased(LEFT_SHIFT)])
                .collect(),
            Self::AltSpace => tap(SPACE).to_vec(),
            Self::CtrlAltDel => tap(DELETE).to_vec(),
            Self::AltPrintScreen => around(held(&[LEFT_CONTROL, RIGHT_CONTROL]), &tap(SYSTEM_REQUEST)),
            Self::PrintScreen => around(
                held(&[LEFT_CONTROL, RIGHT_CONTROL, LEFT_ALT, RIGHT_ALT]),
                &tap(SYSTEM_REQUEST),
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HotkeyAction {
    ToggleFullScreen,
    ReleaseFocus(FocusReleaseDirection),
    ToggleRelativeMouse,
    Inject(KeyInjection),
}

/// Maps a key press to the hotkey it triggers, if any.
///
/// mstscax turns the Alt replacements off while its low-level keyboard hook sends the real Alt
/// combinations to the server. IronRDP installs no such hook, so the replacements apply in every
/// `KeyboardHookMode`.
pub(super) fn translate_hotkey(policy: &InputPolicy, key: HotkeyInput) -> Option<HotkeyAction> {
    let hotkeys = &policy.hotkeys;
    let virtual_key = u32::from(key.virtual_key);
    let control_alt = key.control && key.alt;

    if control_alt && !policy.full_screen_hotkey_disabled {
        // Ctrl+Break reports VK_CANCEL, but keyboards without a Break key send Pause instead.
        let pause_alias = hotkeys.full_screen == u32::from(VK_CANCEL.0) && virtual_key == u32::from(VK_PAUSE.0);
        if virtual_key == hotkeys.full_screen || pause_alias {
            return Some(HotkeyAction::ToggleFullScreen);
        }
    }

    if control_alt && policy.focus_release_events {
        let direction = [
            (hotkeys.focus_release_left, FocusReleaseDirection::Left),
            (hotkeys.focus_release_right, FocusReleaseDirection::Right),
            (hotkeys.focus_release_up, FocusReleaseDirection::Up),
            (hotkeys.focus_release_down, FocusReleaseDirection::Down),
        ]
        .into_iter()
        .find_map(|(hotkey, direction)| (hotkey == virtual_key).then_some(direction));
        if let Some(direction) = direction {
            return Some(HotkeyAction::ReleaseFocus(direction));
        }
    }

    if control_alt && policy.relative_mouse_allowed && virtual_key == u32::from(VK_OEM_5.0) {
        return Some(HotkeyAction::ToggleRelativeMouse);
    }

    if !key.alt {
        return None;
    }

    if key.extended {
        if key.control && policy.sas_transmission && virtual_key == hotkeys.ctrl_alt_del {
            return Some(HotkeyAction::Inject(KeyInjection::CtrlAltDel));
        }
        let injection = if virtual_key == hotkeys.ctrl_esc {
            Some(KeyInjection::CtrlEsc)
        } else if virtual_key == hotkeys.alt_esc && !key.control {
            Some(KeyInjection::AltEsc)
        } else if virtual_key == hotkeys.alt_tab {
            Some(KeyInjection::AltTab)
        } else if virtual_key == hotkeys.alt_shift_tab {
            Some(KeyInjection::AltShiftTab)
        } else if virtual_key == hotkeys.alt_space {
            Some(KeyInjection::AltSpace)
        } else {
            None
        };
        if let Some(injection) = injection {
            return Some(HotkeyAction::Inject(injection));
        }
    }

    if key.control && key.virtual_key == VK_SUBTRACT.0 {
        return Some(HotkeyAction::Inject(KeyInjection::AltPrintScreen));
    }
    if key.control && key.virtual_key == VK_ADD.0 {
        return Some(HotkeyAction::Inject(KeyInjection::PrintScreen));
    }
    None
}

/// Relative mouse state for the current connection.
#[derive(Default)]
pub(super) struct RelativeMouse {
    /// The server advertised `INPUT_FLAG_MOUSE_RELATIVE` for the current activation.
    server_supported: bool,
    capture: Option<RelativeMouseCapture>,
}

struct RelativeMouseCapture {
    restore_position: Option<POINT>,
    /// Buttons the server saw go down through relative events and has not seen released.
    pressed: PointerRelFlags,
}

/// Raw mouse button transitions, in the order relative events report them.
const RAW_BUTTONS: [(u32, u32, PointerRelFlags); 5] = [
    (
        RI_MOUSE_LEFT_BUTTON_DOWN,
        RI_MOUSE_LEFT_BUTTON_UP,
        PointerRelFlags::BUTTON1,
    ),
    (
        RI_MOUSE_RIGHT_BUTTON_DOWN,
        RI_MOUSE_RIGHT_BUTTON_UP,
        PointerRelFlags::BUTTON2,
    ),
    (
        RI_MOUSE_MIDDLE_BUTTON_DOWN,
        RI_MOUSE_MIDDLE_BUTTON_UP,
        PointerRelFlags::BUTTON3,
    ),
    (RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_BUTTON_4_UP, PointerRelFlags::XBUTTON1),
    (RI_MOUSE_BUTTON_5_DOWN, RI_MOUSE_BUTTON_5_UP, PointerRelFlags::XBUTTON2),
];

fn relative_event(flags: PointerRelFlags, x_delta: i16, y_delta: i16) -> FastPathInputEvent {
    FastPathInputEvent::MouseEventRel(MouseRelPdu {
        flags,
        x_delta,
        y_delta,
    })
}

/// Splits a raw mouse movement into deltas that each fit the 16-bit wire fields.
fn relative_moves(mut x: i32, mut y: i32) -> Vec<FastPathInputEvent> {
    let mut events = Vec::new();
    while x != 0 || y != 0 {
        let x_delta = x.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
        let y_delta = y.clamp(i32::from(i16::MIN), i32::from(i16::MAX));
        events.push(relative_event(PointerRelFlags::MOVE, x_delta as i16, y_delta as i16));
        x -= x_delta;
        y -= y_delta;
    }
    events
}

/// Translates one raw mouse report into relative pointer events, tracking which buttons are down.
fn relative_button_events(button_flags: u32, pressed: &mut PointerRelFlags) -> Vec<FastPathInputEvent> {
    let mut events = Vec::new();
    for (down, up, button) in RAW_BUTTONS {
        if button_flags & down != 0 && !pressed.contains(button) {
            pressed.insert(button);
            events.push(relative_event(PointerRelFlags::DOWN | button, 0, 0));
        }
        if button_flags & up != 0 && pressed.contains(button) {
            pressed.remove(button);
            events.push(relative_event(button, 0, 0));
        }
    }
    events
}

fn relative_releases(pressed: PointerRelFlags) -> Vec<FastPathInputEvent> {
    RAW_BUTTONS
        .into_iter()
        .filter(|(_, _, button)| pressed.contains(*button))
        .map(|(_, _, button)| relative_event(button, 0, 0))
        .collect()
}

fn register_raw_mouse(window: HWND, flags: windows::Win32::UI::Input::RAWINPUTDEVICE_FLAGS) -> Result<()> {
    let device = RAWINPUTDEVICE {
        // Generic Desktop page, Mouse usage.
        usUsagePage: 0x01,
        usUsage: 0x02,
        dwFlags: flags,
        hwndTarget: window,
    };
    unsafe { RegisterRawInputDevices(&[device], size_of::<RAWINPUTDEVICE>() as u32) }
}

/// Runs the hotkey bound to a key press. Returns `true` when the key press was consumed.
pub(super) fn handle_hotkey(control: &Control, window: HWND, key: HotkeyInput) -> bool {
    let policy = control.compatibility.borrow().input_policy;
    let Some(action) = translate_hotkey(&policy, key) else {
        return false;
    };

    match action {
        HotkeyAction::ToggleFullScreen => {
            trace_host_call("IMsTscAx::FullScreenHotKey");
            control.request_fullscreen_toggle();
            true
        }
        HotkeyAction::ReleaseFocus(direction) => {
            // As in mstscax, the container decides where focus goes; the event only reports the request.
            trace_host_call("IMsTscAx::FocusReleaseHotKey");
            control.fire_event(DISPID_ON_FOCUS_RELEASED, &[direction.event_argument()]);
            true
        }
        HotkeyAction::ToggleRelativeMouse => toggle_relative_mouse(control, window),
        HotkeyAction::Inject(injection) => {
            trace_host_call("IMsTscAx::InputHotKey");
            let operations = {
                let input_database = control.input_database.borrow();
                injection.operations(|key| input_database.is_key_pressed(key))
            };
            control.apply_input(operations);
            true
        }
    }
}

/// Whether `IOleInPlaceActiveObject::TranslateAccelerator` offers a key message to the container.
///
/// With `AcceleratorPassthrough`, Alt key combinations belong to the session, as in mstscax.
pub(super) fn offers_accelerator_to_container(policy: &InputPolicy, message: u32) -> bool {
    !(policy.accelerator_passthrough && matches!(message, WM_SYSKEYDOWN | WM_SYSKEYUP | WM_SYSCHAR))
}

/// Reads the modifier state for a `WM_KEYDOWN` or `WM_SYSKEYDOWN` and runs its hotkey.
pub(super) fn handle_hotkey_message(control: &Control, window: HWND, wparam: WPARAM, lparam: LPARAM) -> bool {
    let key = HotkeyInput {
        virtual_key: wparam.0 as u16,
        extended: lparam.0 as u32 & 0x0100_0000 != 0,
        control: unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0,
        alt: unsafe { GetKeyState(i32::from(VK_MENU.0)) } < 0,
    };
    handle_hotkey(control, window, key)
}

/// Whether local mouse messages reach the session.
pub(super) fn local_mouse_input_enabled(control: &Control) -> bool {
    let compatibility = control.compatibility.borrow();
    compatibility.enable_mouse && !compatibility.input_policy.ignore_client_mouse_input
}

/// Records the server's Input Capability Set flags for the current activation.
pub(super) fn set_server_input_flags(control: &Control, flags: InputFlags) {
    let supported = flags.contains(InputFlags::MOUSE_RELATIVE);
    control.relative_mouse.borrow_mut().server_supported = supported;
    if !supported {
        release_relative_mouse(control);
    }
}

fn toggle_relative_mouse(control: &Control, window: HWND) -> bool {
    if control.relative_mouse.borrow().capture.is_some() {
        release_relative_mouse(control);
        return true;
    }

    let available = control.relative_mouse.borrow().server_supported
        && control.state.get() == ConnectionState::Connected
        && !control.compatibility.borrow().remote_program_mode
        && control.input_sender.borrow().is_some();
    if !available || window.0.is_null() {
        // mstscax sends the key to the server when relative mode is unavailable.
        return false;
    }

    // The pointer's absolute position stops tracking the local cursor, so drop pressed buttons first.
    control.release_input();
    if let Err(error) = register_raw_mouse(window, RIDEV_NOLEGACY | RIDEV_CAPTUREMOUSE) {
        tracing::debug!(?error, "Unable to capture the mouse for relative pointer input");
        return false;
    }

    let mut restore_position = POINT::default();
    let restore_position = unsafe { GetCursorPos(&mut restore_position) }
        .is_ok()
        .then_some(restore_position);
    let mut window_rect = RECT::default();
    if unsafe { GetWindowRect(window, &mut window_rect) }.is_ok()
        && let Err(error) = unsafe { ClipCursor(Some(&window_rect)) }
    {
        tracing::debug!(?error, "Unable to confine the cursor during relative pointer input");
    }
    // UNTESTED: this hide is undone by exactly one `ShowCursor(true)` in `release_relative_mouse`, and every
    // path that ends the capture goes through that function. The display counter belongs to the host's
    // UI thread, so an unbalanced call leaves the host's cursor hidden. Check both functions together.
    unsafe { ShowCursor(false) };
    trace_host_call("IMsTscAx::RelativeMouseCaptured");
    control.relative_mouse.borrow_mut().capture = Some(RelativeMouseCapture {
        restore_position,
        pressed: PointerRelFlags::empty(),
    });
    control.fire_event_values(DISPID_ON_MOUSE_INPUT_MODE_CHANGED, vec![VariantValue::Bool(true)]);
    true
}

/// Ends relative pointer input and gives the local cursor back.
pub(super) fn release_relative_mouse(control: &Control) {
    let Some(capture) = control.relative_mouse.borrow_mut().capture.take() else {
        return;
    };

    send_relative_events(control, relative_releases(capture.pressed));
    // UNTESTED: removing the registration restores legacy mouse messages for the whole host process, and
    // releasing the clip and restoring the position give the user their cursor back where it was. Verify
    // manually with a server that advertises relative mouse support.
    if let Err(error) = register_raw_mouse(HWND(ptr::null_mut()), RIDEV_REMOVE) {
        tracing::debug!(?error, "Unable to release the relative pointer capture");
    }
    unsafe {
        let _ = ClipCursor(None);
        ShowCursor(true);
        if let Some(position) = capture.restore_position {
            let _ = SetCursorPos(position.x, position.y);
        }
    }
    trace_host_call("IMsTscAx::RelativeMouseReleased");
    // mstscax reports mode changes of a live session, not the teardown of one.
    if control.state.get() == ConnectionState::Connected {
        control.fire_event_values(DISPID_ON_MOUSE_INPUT_MODE_CHANGED, vec![VariantValue::Bool(false)]);
    }
}

/// Handles `WM_INPUT` while relative pointer input is active.
pub(super) fn handle_raw_input(control: &Control, lparam: LPARAM) {
    if control.relative_mouse.borrow().capture.is_none() || !local_mouse_input_enabled(control) {
        return;
    }

    let mut input = RAWINPUT::default();
    let mut size = size_of::<RAWINPUT>() as u32;
    let copied = unsafe {
        GetRawInputData(
            HRAWINPUT(lparam.0 as *mut c_void),
            RID_INPUT,
            Some(ptr::from_mut(&mut input).cast()),
            &mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if copied == u32::MAX || copied == 0 || input.header.dwType != RIM_TYPEMOUSE.0 {
        return;
    }

    let mouse = unsafe { input.data.mouse };
    let (button_flags, button_data) = unsafe {
        (
            u32::from(mouse.Anonymous.Anonymous.usButtonFlags),
            mouse.Anonymous.Anonymous.usButtonData as i16,
        )
    };

    let mut events = Vec::new();
    // Absolute devices (pens, remote sessions) have no relative motion to report.
    if mouse.usFlags.0 & MOUSE_MOVE_ABSOLUTE.0 == 0 {
        events.extend(relative_moves(mouse.lLastX, mouse.lLastY));
    }
    if let Some(capture) = control.relative_mouse.borrow_mut().capture.as_mut() {
        events.extend(relative_button_events(button_flags, &mut capture.pressed));
    }
    send_relative_events(control, events);

    // A raw mouse report carries one wheel delta, vertical or horizontal, in `usButtonData`.
    if button_flags & (RI_MOUSE_WHEEL | RI_MOUSE_HWHEEL) != 0 {
        control.apply_input(wheel_operations(button_flags & RI_MOUSE_WHEEL != 0, button_data));
    }
}

fn send_relative_events(control: &Control, events: Vec<FastPathInputEvent>) {
    if events.is_empty() {
        return;
    }
    if let Some(sender) = control.input_sender.borrow().as_ref()
        && let Err(error) = sender.try_send(RdpInputEvent::FastPath(events.into_iter().collect()))
    {
        tracing::warn!(%error, "Unable to enqueue ActiveX relative pointer input for the RDP session");
    }
}

fn input_policy_mut<R>(this: *mut c_void, update: impl FnOnce(&mut InputPolicy) -> R) -> R {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    let mut settings = object.settings.borrow_mut();
    update(&mut settings.input_policy)
}

fn input_policy(this: *mut c_void) -> InputPolicy {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    object.settings.borrow().input_policy
}

/// mstscax rejects these properties with `E_FAIL` once a connection starts; it reads them when the
/// connection's input handler starts.
fn input_property_mutable(this: *mut c_void) -> bool {
    let object = unsafe { &*(this.cast::<AdvancedSettingsObject>()) };
    !object.settings.borrow().connection_settings_sealed
}

macro_rules! hotkey_accessors {
    ($(($put:ident, $get:ident, $field:ident)),+ $(,)?) => {
        $(
            pub(super) unsafe extern "system" fn $put(this: *mut c_void, value: i32) -> HRESULT {
                if !input_property_mutable(this) {
                    return E_FAIL;
                }
                // mstscax stores any ULONG; keys outside the virtual-key range never match.
                input_policy_mut(this, |policy| policy.hotkeys.$field = value as u32);
                S_OK
            }

            pub(super) unsafe extern "system" fn $get(this: *mut c_void, value: *mut i32) -> HRESULT {
                write_out(value, input_policy(this).hotkeys.$field as i32).map_or_else(|error| error.code(), |_| S_OK)
            }
        )+
    };
}

hotkey_accessors!(
    (
        advanced_put_hotkey_full_screen,
        advanced_get_hotkey_full_screen,
        full_screen
    ),
    (advanced_put_hotkey_ctrl_esc, advanced_get_hotkey_ctrl_esc, ctrl_esc),
    (advanced_put_hotkey_alt_esc, advanced_get_hotkey_alt_esc, alt_esc),
    (advanced_put_hotkey_alt_tab, advanced_get_hotkey_alt_tab, alt_tab),
    (
        advanced_put_hotkey_alt_shift_tab,
        advanced_get_hotkey_alt_shift_tab,
        alt_shift_tab
    ),
    (advanced_put_hotkey_alt_space, advanced_get_hotkey_alt_space, alt_space),
    (
        advanced_put_hotkey_ctrl_alt_del,
        advanced_get_hotkey_ctrl_alt_del,
        ctrl_alt_del
    ),
    (
        advanced_put_hotkey_focus_release_left,
        advanced_get_hotkey_focus_release_left,
        focus_release_left
    ),
    (
        advanced_put_hotkey_focus_release_right,
        advanced_get_hotkey_focus_release_right,
        focus_release_right
    ),
);

pub(super) unsafe extern "system" fn advanced_put_accelerator_passthrough(this: *mut c_void, value: i32) -> HRESULT {
    if !input_property_mutable(this) {
        return E_FAIL;
    }
    input_policy_mut(this, |policy| policy.accelerator_passthrough = value != 0);
    S_OK
}

pub(super) unsafe extern "system" fn advanced_get_accelerator_passthrough(
    this: *mut c_void,
    value: *mut i32,
) -> HRESULT {
    write_out(value, i32::from(input_policy(this).accelerator_passthrough)).map_or_else(|error| error.code(), |_| S_OK)
}

pub(super) unsafe extern "system" fn advanced_put_sas_sequence(_this: *mut c_void, value: i32) -> HRESULT {
    // The Ctrl+Alt+Del hotkey always sends Ctrl+Alt+Del; no other legacy sequence exists in IronRDP.
    if value == SAS_SEQUENCE_CTRL_ALT_DEL {
        S_OK
    } else {
        trace_host_call("E_NOTIMPL:AdvancedSettings::SasSequence");
        E_NOTIMPL
    }
}

pub(super) unsafe extern "system" fn advanced_get_sas_sequence(_this: *mut c_void, value: *mut i32) -> HRESULT {
    write_out(value, SAS_SEQUENCE_CTRL_ALT_DEL).map_or_else(|error| error.code(), |_| S_OK)
}

pub(super) unsafe extern "system" fn advanced_put_disable_ctrl_alt_del(_this: *mut c_void, value: i32) -> HRESULT {
    // IronRDP's Client Info PDU always sets INFO_DISABLECTRLALTDEL, the mstscax default.
    if value != 0 {
        S_OK
    } else {
        trace_host_call("E_NOTIMPL:AdvancedSettings::DisableCtrlAltDel");
        E_NOTIMPL
    }
}

pub(super) unsafe extern "system" fn advanced_get_disable_ctrl_alt_del(_this: *mut c_void, value: *mut i32) -> HRESULT {
    write_out(value, 1).map_or_else(|error| error.code(), |_| S_OK)
}

pub(super) unsafe extern "system" fn advanced_put_double_click_detect(_this: *mut c_void, value: i32) -> HRESULT {
    // IronRDP's Client Info PDU never sets INFO_DOUBLECLICKDETECT, the mstscax default.
    if value == 0 {
        S_OK
    } else {
        trace_host_call("E_NOTIMPL:AdvancedSettings::DoubleClickDetect");
        E_NOTIMPL
    }
}

pub(super) unsafe extern "system" fn advanced_get_double_click_detect(_this: *mut c_void, value: *mut i32) -> HRESULT {
    write_out(value, 0).map_or_else(|error| error.code(), |_| S_OK)
}

pub(super) unsafe extern "system" fn advanced_put_relative_mouse_mode(_this: *mut c_void, value: i16) -> HRESULT {
    // mstscax uses this to accept the server's undocumented input mode change PDU, which IronRDP does
    // not decode. Host-toggled relative input is `AllowRelativeMouseMode`.
    if value == VARIANT_FALSE.0 {
        S_OK
    } else {
        trace_host_call("E_NOTIMPL:AdvancedSettings::RelativeMouseMode");
        E_NOTIMPL
    }
}

pub(super) unsafe extern "system" fn advanced_get_relative_mouse_mode(_this: *mut c_void, value: *mut i16) -> HRESULT {
    write_out(value, VARIANT_FALSE.0).map_or_else(|error| error.code(), |_| S_OK)
}

pub(super) unsafe extern "system" fn advanced_put_enable_super_pan(_this: *mut c_void, value: i16) -> HRESULT {
    // The renderer scales or centers the remote desktop and never pans it.
    if value == VARIANT_FALSE.0 {
        S_OK
    } else {
        trace_host_call("E_NOTIMPL:AdvancedSettings::EnableSuperPan");
        E_NOTIMPL
    }
}

pub(super) unsafe extern "system" fn advanced_get_enable_super_pan(_this: *mut c_void, value: *mut i16) -> HRESULT {
    write_out(value, VARIANT_FALSE.0).map_or_else(|error| error.code(), |_| S_OK)
}

pub(super) unsafe extern "system" fn advanced_put_super_pan_acceleration_factor(
    _this: *mut c_void,
    _value: u32,
) -> HRESULT {
    trace_host_call("E_NOTIMPL:AdvancedSettings::SuperPanAccelerationFactor");
    E_NOTIMPL
}

pub(super) unsafe extern "system" fn advanced_get_super_pan_acceleration_factor(
    _this: *mut c_void,
    value: *mut u32,
) -> HRESULT {
    if let Err(error) = write_out(value, DEFAULT_SUPER_PAN_ACCELERATION_FACTOR) {
        return error.code();
    }
    trace_host_call("E_NOTIMPL:AdvancedSettings::SuperPanAccelerationFactor");
    E_NOTIMPL
}

/// The `IMsRdpExtendedSettings` properties that configure local input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputExtendedProperty {
    AllowAxToContainerEvents,
    AllowRelativeMouseMode,
    AllowSasSequenceTransmission,
    DisableFullScreenHotkey,
    IgnoreClientSideMouseInput,
    IgnoreServerGeneratedMouseMoves,
}

impl InputExtendedProperty {
    fn from_name(name: &str) -> Option<Self> {
        [
            ("AllowAxToContainerEvents", Self::AllowAxToContainerEvents),
            ("AllowRelativeMouseMode", Self::AllowRelativeMouseMode),
            ("AllowSasSequenceTransmission", Self::AllowSasSequenceTransmission),
            ("DisableFullScreenHotkey", Self::DisableFullScreenHotkey),
            ("IgnoreClientSideMouseInput", Self::IgnoreClientSideMouseInput),
            ("IgnoreServerGeneratedMouseMoves", Self::IgnoreServerGeneratedMouseMoves),
        ]
        .into_iter()
        .find_map(|(candidate, property)| name.eq_ignore_ascii_case(candidate).then_some(property))
    }

    fn get(self, policy: &InputPolicy) -> bool {
        match self {
            Self::AllowAxToContainerEvents => policy.focus_release_events,
            Self::AllowRelativeMouseMode => policy.relative_mouse_allowed,
            Self::AllowSasSequenceTransmission => policy.sas_transmission,
            Self::DisableFullScreenHotkey => policy.full_screen_hotkey_disabled,
            Self::IgnoreClientSideMouseInput => policy.ignore_client_mouse_input,
            // The control never moves the local cursor for server pointer position updates.
            Self::IgnoreServerGeneratedMouseMoves => true,
        }
    }

    fn put(self, policy: &mut InputPolicy, value: bool) -> Result<()> {
        match self {
            Self::AllowAxToContainerEvents => policy.focus_release_events = value,
            Self::AllowRelativeMouseMode => policy.relative_mouse_allowed = value,
            Self::AllowSasSequenceTransmission => policy.sas_transmission = value,
            Self::DisableFullScreenHotkey => policy.full_screen_hotkey_disabled = value,
            Self::IgnoreClientSideMouseInput => policy.ignore_client_mouse_input = value,
            Self::IgnoreServerGeneratedMouseMoves if value => {}
            Self::IgnoreServerGeneratedMouseMoves => return Err(Error::from_hresult(E_NOTIMPL)),
        }
        Ok(())
    }
}

/// Handles `IMsRdpExtendedSettings::put_Property` for the input properties; `None` for other names.
pub(super) fn put_input_extended_property(control: &Control, name: &str, value: *mut VARIANT) -> Option<Result<()>> {
    let property = InputExtendedProperty::from_name(name)?;
    if value.is_null() {
        return Some(Err(Error::from_hresult(E_POINTER)));
    }
    let result = variant_bool(unsafe { &*value }, ptr::null_mut()).and_then(|value| {
        property.put(&mut control.compatibility.borrow_mut().input_policy, value)?;
        if property == InputExtendedProperty::AllowRelativeMouseMode && !value {
            release_relative_mouse(control);
        }
        Ok(())
    });
    trace_host_call(if result.is_ok() {
        "IMsRdpExtendedSettings::put_InputPolicy"
    } else {
        "E_FAIL:IMsRdpExtendedSettings::put_InputPolicy"
    });
    Some(result)
}

/// Handles `IMsRdpExtendedSettings::get_Property` for the input properties; `None` for other names.
pub(super) fn get_input_extended_property(control: &Control, name: &str, value: *mut VARIANT) -> Option<Result<()>> {
    let property = InputExtendedProperty::from_name(name)?;
    let current = property.get(&control.compatibility.borrow().input_policy);
    trace_host_call("IMsRdpExtendedSettings::get_InputPolicy");
    Some(write_out(value, variant_bool_value(current)))
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY;

    use super::*;

    fn key(virtual_key: VIRTUAL_KEY, extended: bool, control: bool, alt: bool) -> HotkeyInput {
        HotkeyInput {
            virtual_key: virtual_key.0,
            extended,
            control,
            alt,
        }
    }

    fn translate(policy: &InputPolicy, input: HotkeyInput) -> Option<HotkeyAction> {
        translate_hotkey(policy, input)
    }

    #[test]
    fn hotkey_defaults_match_the_mstscax_property_table() {
        // oracle: reference mstscax.dll 10.0.26100.9711 AdvancedSettings property table defaults
        let hotkeys = Hotkeys::default();
        assert_eq!(
            [
                hotkeys.full_screen,
                hotkeys.ctrl_esc,
                hotkeys.alt_esc,
                hotkeys.alt_tab,
                hotkeys.alt_shift_tab,
                hotkeys.alt_space,
                hotkeys.ctrl_alt_del,
                hotkeys.focus_release_left,
                hotkeys.focus_release_right,
                hotkeys.focus_release_up,
                hotkeys.focus_release_down,
            ],
            [0x03, 0x24, 0x2d, 0x21, 0x22, 0x2e, 0x23, 0x25, 0x27, 0x26, 0x28]
        );
        let policy = InputPolicy::default();
        assert!(policy.accelerator_passthrough);
        assert!(!policy.focus_release_events);
        assert!(policy.sas_transmission);
        assert!(!policy.relative_mouse_allowed);
    }

    #[test]
    fn full_screen_hotkey_requires_control_alt_and_can_be_disabled() {
        let mut policy = InputPolicy::default();
        let toggle = Some(HotkeyAction::ToggleFullScreen);
        assert_eq!(translate(&policy, key(VK_CANCEL, false, true, true)), toggle);
        assert_eq!(translate(&policy, key(VK_PAUSE, false, true, true)), toggle);
        assert_eq!(translate(&policy, key(VK_CANCEL, false, false, true)), None);
        assert_eq!(translate(&policy, key(VK_CANCEL, false, true, false)), None);

        policy.hotkeys.full_screen = u32::from(VIRTUAL_KEY(0x46).0);
        assert_eq!(translate(&policy, key(VIRTUAL_KEY(0x46), false, true, true)), toggle);
        assert_eq!(translate(&policy, key(VK_PAUSE, false, true, true)), None);

        policy.full_screen_hotkey_disabled = true;
        assert_eq!(translate(&policy, key(VIRTUAL_KEY(0x46), false, true, true)), None);
    }

    #[test]
    fn focus_release_hotkeys_require_the_container_events_opt_in() {
        let mut policy = InputPolicy::default();
        assert_eq!(translate(&policy, key(VK_LEFT, true, true, true)), None);

        policy.focus_release_events = true;
        for (virtual_key, direction, argument) in [
            (VK_LEFT, FocusReleaseDirection::Left, -1),
            (VK_RIGHT, FocusReleaseDirection::Right, 1),
            (VK_UP, FocusReleaseDirection::Up, 2),
            (VK_DOWN, FocusReleaseDirection::Down, -2),
        ] {
            assert_eq!(
                translate(&policy, key(virtual_key, true, true, true)),
                Some(HotkeyAction::ReleaseFocus(direction))
            );
            assert_eq!(direction.event_argument(), argument);
            assert_eq!(translate(&policy, key(virtual_key, true, false, true)), None);
        }
    }

    #[test]
    fn alt_replacement_hotkeys_need_the_navigation_cluster_key() {
        let policy = InputPolicy::default();
        for (virtual_key, injection) in [
            (VK_HOME, KeyInjection::CtrlEsc),
            (VK_INSERT, KeyInjection::AltEsc),
            (VK_PRIOR, KeyInjection::AltTab),
            (VK_NEXT, KeyInjection::AltShiftTab),
            (VK_DELETE, KeyInjection::AltSpace),
        ] {
            assert_eq!(
                translate(&policy, key(virtual_key, true, false, true)),
                Some(HotkeyAction::Inject(injection))
            );
            assert_eq!(translate(&policy, key(virtual_key, false, false, true)), None);
            assert_eq!(translate(&policy, key(virtual_key, true, false, false)), None);
        }
        // Ctrl+Alt+Insert is not Alt+Esc.
        assert_eq!(translate(&policy, key(VK_INSERT, true, true, true)), None);
    }

    #[test]
    fn ctrl_alt_end_sends_ctrl_alt_del_only_when_allowed() {
        let mut policy = InputPolicy::default();
        let ctrl_alt_del = Some(HotkeyAction::Inject(KeyInjection::CtrlAltDel));
        assert_eq!(translate(&policy, key(VK_END, true, true, true)), ctrl_alt_del);
        assert_eq!(translate(&policy, key(VK_END, false, true, true)), None);
        assert_eq!(translate(&policy, key(VK_END, true, false, true)), None);

        policy.sas_transmission = false;
        assert_eq!(translate(&policy, key(VK_END, true, true, true)), None);
    }

    #[test]
    fn keypad_minus_and_plus_send_print_screen() {
        let policy = InputPolicy::default();
        assert_eq!(
            translate(&policy, key(VK_SUBTRACT, false, true, true)),
            Some(HotkeyAction::Inject(KeyInjection::AltPrintScreen))
        );
        assert_eq!(
            translate(&policy, key(VK_ADD, false, true, true)),
            Some(HotkeyAction::Inject(KeyInjection::PrintScreen))
        );
        assert_eq!(translate(&policy, key(VK_ADD, false, false, true)), None);
    }

    #[test]
    fn relative_mouse_toggle_requires_the_host_opt_in() {
        let mut policy = InputPolicy::default();
        assert_eq!(translate(&policy, key(VK_OEM_5, false, true, true)), None);
        policy.relative_mouse_allowed = true;
        assert_eq!(
            translate(&policy, key(VK_OEM_5, false, true, true)),
            Some(HotkeyAction::ToggleRelativeMouse)
        );
    }

    #[test]
    fn injections_release_and_restore_only_pressed_modifiers() {
        let pressed = |keys: &'static [Scancode]| move |key: Scancode| keys.contains(&key);

        assert_eq!(
            format!("{:?}", KeyInjection::CtrlEsc.operations(pressed(&[LEFT_ALT]))),
            format!(
                "{:?}",
                [
                    Operation::KeyReleased(LEFT_ALT),
                    Operation::KeyPressed(LEFT_CONTROL),
                    Operation::KeyPressed(ESCAPE),
                    Operation::KeyReleased(ESCAPE),
                    Operation::KeyReleased(LEFT_CONTROL),
                ]
            )
        );
        assert_eq!(
            format!(
                "{:?}",
                KeyInjection::PrintScreen.operations(pressed(&[RIGHT_CONTROL, LEFT_ALT]))
            ),
            format!(
                "{:?}",
                [
                    Operation::KeyReleased(RIGHT_CONTROL),
                    Operation::KeyReleased(LEFT_ALT),
                    Operation::KeyPressed(SYSTEM_REQUEST),
                    Operation::KeyReleased(SYSTEM_REQUEST),
                    Operation::KeyPressed(LEFT_ALT),
                    Operation::KeyPressed(RIGHT_CONTROL),
                ]
            )
        );
        assert_eq!(
            format!("{:?}", KeyInjection::AltShiftTab.operations(pressed(&[]))),
            format!(
                "{:?}",
                [
                    Operation::KeyPressed(LEFT_SHIFT),
                    Operation::KeyPressed(TAB),
                    Operation::KeyReleased(TAB),
                    Operation::KeyReleased(LEFT_SHIFT),
                ]
            )
        );
    }

    #[test]
    fn hotkeys_send_their_replacement_and_swallow_the_key() {
        let control = Control::new();
        control.apply_input([Operation::KeyPressed(LEFT_ALT)]);

        assert!(handle_hotkey(
            &control,
            HWND::default(),
            key(VK_PRIOR, true, false, true)
        ));
        let database = control.input_database.borrow();
        assert!(database.is_key_pressed(LEFT_ALT));
        assert!(!database.is_key_pressed(TAB));
        assert!(!database.is_key_pressed(Scancode::from_u8(true, 0x49)));
        drop(database);

        assert!(handle_hotkey(
            &control,
            HWND::default(),
            key(VK_HOME, true, false, true)
        ));
        assert!(!control.input_database.borrow().is_key_pressed(LEFT_ALT));

        assert!(!handle_hotkey(
            &control,
            HWND::default(),
            key(VIRTUAL_KEY(0x41), false, false, true)
        ));
    }

    #[test]
    fn relative_mouse_toggle_is_declined_without_server_support() {
        let control = Control::new();
        control.compatibility.borrow_mut().input_policy.relative_mouse_allowed = true;
        assert!(!handle_hotkey(
            &control,
            HWND::default(),
            key(VK_OEM_5, false, true, true)
        ));

        set_server_input_flags(&control, InputFlags::MOUSE_RELATIVE);
        // Still declined: no session is connected.
        assert!(!handle_hotkey(
            &control,
            HWND::default(),
            key(VK_OEM_5, false, true, true)
        ));
    }

    #[test]
    fn relative_buttons_report_each_transition_once() {
        let mut pressed = PointerRelFlags::empty();
        let events = relative_button_events(RI_MOUSE_LEFT_BUTTON_DOWN | RI_MOUSE_BUTTON_4_DOWN, &mut pressed);
        assert_eq!(
            events,
            [
                relative_event(PointerRelFlags::DOWN | PointerRelFlags::BUTTON1, 0, 0),
                relative_event(PointerRelFlags::DOWN | PointerRelFlags::XBUTTON1, 0, 0),
            ]
        );
        assert!(relative_button_events(RI_MOUSE_LEFT_BUTTON_DOWN, &mut pressed).is_empty());
        assert_eq!(
            relative_button_events(RI_MOUSE_LEFT_BUTTON_UP | RI_MOUSE_RIGHT_BUTTON_UP, &mut pressed),
            [relative_event(PointerRelFlags::BUTTON1, 0, 0)]
        );
        assert_eq!(
            relative_releases(pressed),
            [relative_event(PointerRelFlags::XBUTTON1, 0, 0)]
        );
    }

    #[test]
    fn large_relative_moves_split_into_wire_sized_deltas() {
        assert!(relative_moves(0, 0).is_empty());
        assert_eq!(relative_moves(5, -7), [relative_event(PointerRelFlags::MOVE, 5, -7)]);
        assert_eq!(
            relative_moves(40_000, -1),
            [
                relative_event(PointerRelFlags::MOVE, i16::MAX, -1),
                relative_event(PointerRelFlags::MOVE, (40_000 - i32::from(i16::MAX)) as i16, 0),
            ]
        );
    }

    #[test]
    fn extended_input_properties_round_trip() {
        let control = Control::new();
        for name in [
            "AllowAxToContainerEvents",
            "AllowRelativeMouseMode",
            "AllowSasSequenceTransmission",
            "DisableFullScreenHotkey",
            "IgnoreClientSideMouseInput",
        ] {
            for expected in [true, false] {
                let mut value = variant_bool_value(expected);
                put_input_extended_property(&control, name, &mut value)
                    .expect("input property")
                    .expect("valid boolean");
                let mut read = VARIANT::default();
                get_input_extended_property(&control, name, &mut read)
                    .expect("input property")
                    .expect("readable");
                assert_eq!(
                    variant_bool(&read, ptr::null_mut()).expect("boolean"),
                    expected,
                    "{name}"
                );
            }
        }

        let mut value = variant_bool_value(false);
        let error = put_input_extended_property(&control, "IgnoreServerGeneratedMouseMoves", &mut value)
            .expect("input property")
            .expect_err("server pointer moves are never applied");
        assert_eq!(error.code(), E_NOTIMPL);
        let mut value = variant_i32(1);
        let error = put_input_extended_property(&control, "allowrelativemousemode", &mut value)
            .expect("names are case-insensitive")
            .expect_err("the property is a VARIANT_BOOL");
        assert_eq!(error.code(), DISP_E_TYPEMISMATCH);
        assert!(put_input_extended_property(&control, "ZoomLevel", &mut value).is_none());
    }

    fn advanced_settings_object(settings: &Rc<RefCell<CompatibilitySettings>>) -> AdvancedSettingsObject {
        CompatibilitySettingsObject {
            vtable: advanced_vtable(),
            references: AtomicU32::new(1),
            settings: Rc::clone(settings),
            native_mstsc_credential_bridge: None,
            remote_program_bridge: None,
            server_object: false,
        }
    }

    #[test]
    fn hotkey_settings_round_trip_until_a_connection_starts() {
        let settings = Rc::new(RefCell::new(CompatibilitySettings::default()));
        let mut object = advanced_settings_object(&settings);
        let this = (&mut object as *mut AdvancedSettingsObject).cast::<c_void>();
        type Put = unsafe extern "system" fn(*mut c_void, i32) -> HRESULT;
        type Get = unsafe extern "system" fn(*mut c_void, *mut i32) -> HRESULT;
        // oracle: reference mstscax.dll 10.0.26100.9711 AdvancedSettings property table defaults
        let accessors: [(&str, Put, Get, i32, i32); 10] = [
            (
                "HotKeyFullScreen",
                advanced_put_hotkey_full_screen,
                advanced_get_hotkey_full_screen,
                0x03,
                0x70,
            ),
            (
                "HotKeyCtrlEsc",
                advanced_put_hotkey_ctrl_esc,
                advanced_get_hotkey_ctrl_esc,
                0x24,
                0x71,
            ),
            (
                "HotKeyAltEsc",
                advanced_put_hotkey_alt_esc,
                advanced_get_hotkey_alt_esc,
                0x2d,
                0x72,
            ),
            (
                "HotKeyAltTab",
                advanced_put_hotkey_alt_tab,
                advanced_get_hotkey_alt_tab,
                0x21,
                0x73,
            ),
            (
                "HotKeyAltShiftTab",
                advanced_put_hotkey_alt_shift_tab,
                advanced_get_hotkey_alt_shift_tab,
                0x22,
                0x74,
            ),
            (
                "HotKeyAltSpace",
                advanced_put_hotkey_alt_space,
                advanced_get_hotkey_alt_space,
                0x2e,
                0x75,
            ),
            (
                "HotKeyCtrlAltDel",
                advanced_put_hotkey_ctrl_alt_del,
                advanced_get_hotkey_ctrl_alt_del,
                0x23,
                0x76,
            ),
            (
                "HotKeyFocusReleaseLeft",
                advanced_put_hotkey_focus_release_left,
                advanced_get_hotkey_focus_release_left,
                0x25,
                0x77,
            ),
            (
                "HotKeyFocusReleaseRight",
                advanced_put_hotkey_focus_release_right,
                advanced_get_hotkey_focus_release_right,
                0x27,
                0x78,
            ),
            (
                "AcceleratorPassthrough",
                advanced_put_accelerator_passthrough,
                advanced_get_accelerator_passthrough,
                1,
                0,
            ),
        ];

        for (name, put, get, default, updated) in accessors {
            let mut value = -1;
            assert_eq!(unsafe { get(this, &mut value) }, S_OK, "get_{name}");
            assert_eq!(value, default, "{name} default");
            assert_eq!(unsafe { put(this, updated) }, S_OK, "put_{name}");
            assert_eq!(unsafe { get(this, &mut value) }, S_OK, "get_{name}");
            assert_eq!(value, updated, "{name} after put");

            settings.borrow_mut().connection_settings_sealed = true;
            assert_eq!(unsafe { put(this, default) }, E_FAIL, "put_{name} after sealing");
            settings.borrow_mut().connection_settings_sealed = false;
        }
        assert_eq!(settings.borrow().input_policy.hotkeys.full_screen, 0x70);
        assert!(!settings.borrow().input_policy.accelerator_passthrough);
    }

    #[test]
    fn unsupported_input_settings_accept_only_the_value_ironrdp_implements() {
        let settings = Rc::new(RefCell::new(CompatibilitySettings::default()));
        let mut object = advanced_settings_object(&settings);
        let this = (&mut object as *mut AdvancedSettingsObject).cast::<c_void>();

        assert_eq!(
            unsafe { advanced_put_sas_sequence(this, SAS_SEQUENCE_CTRL_ALT_DEL) },
            S_OK
        );
        assert_eq!(unsafe { advanced_put_sas_sequence(this, 0) }, E_NOTIMPL);
        assert_eq!(unsafe { advanced_put_disable_ctrl_alt_del(this, 1) }, S_OK);
        assert_eq!(unsafe { advanced_put_disable_ctrl_alt_del(this, 0) }, E_NOTIMPL);
        assert_eq!(unsafe { advanced_put_double_click_detect(this, 0) }, S_OK);
        assert_eq!(unsafe { advanced_put_double_click_detect(this, 1) }, E_NOTIMPL);
        assert_eq!(unsafe { advanced_put_relative_mouse_mode(this, VARIANT_FALSE.0) }, S_OK);
        assert_eq!(
            unsafe { advanced_put_relative_mouse_mode(this, VARIANT_TRUE.0) },
            E_NOTIMPL
        );
        assert_eq!(unsafe { advanced_put_enable_super_pan(this, VARIANT_FALSE.0) }, S_OK);
        assert_eq!(
            unsafe { advanced_put_enable_super_pan(this, VARIANT_TRUE.0) },
            E_NOTIMPL
        );
        assert_eq!(
            unsafe { advanced_put_super_pan_acceleration_factor(this, 2) },
            E_NOTIMPL
        );

        let mut value = 0;
        assert_eq!(unsafe { advanced_get_sas_sequence(this, &mut value) }, S_OK);
        assert_eq!(value, SAS_SEQUENCE_CTRL_ALT_DEL);
        assert_eq!(unsafe { advanced_get_disable_ctrl_alt_del(this, &mut value) }, S_OK);
        assert_eq!(value, 1);
        assert_eq!(unsafe { advanced_get_double_click_detect(this, &mut value) }, S_OK);
        assert_eq!(value, 0);
        let mut flag = VARIANT_TRUE.0;
        assert_eq!(unsafe { advanced_get_relative_mouse_mode(this, &mut flag) }, S_OK);
        assert_eq!(flag, VARIANT_FALSE.0);
        let mut factor = 0;
        assert_eq!(
            unsafe { advanced_get_super_pan_acceleration_factor(this, &mut factor) },
            E_NOTIMPL
        );
        assert_eq!(factor, DEFAULT_SUPER_PAN_ACCELERATION_FACTOR);
    }

    #[implement(IDispatch)]
    struct FocusReleasedSink {
        seen: Arc<Mutex<Vec<(i32, i32)>>>,
    }

    impl IDispatch_Impl for FocusReleasedSink_Impl {
        fn GetTypeInfoCount(&self) -> Result<u32> {
            Ok(0)
        }

        fn GetTypeInfo(&self, _itinfo: u32, _lcid: u32) -> Result<ITypeInfo> {
            Err(Error::from_hresult(E_NOTIMPL))
        }

        fn GetIDsOfNames(
            &self,
            _riid: *const GUID,
            _names: *const PCWSTR,
            _count: u32,
            _lcid: u32,
            _dispids: *mut i32,
        ) -> Result<()> {
            Err(Error::from_hresult(DISP_E_UNKNOWNNAME))
        }

        fn Invoke(
            &self,
            dispid: i32,
            _riid: *const GUID,
            _lcid: u32,
            _flags: DISPATCH_FLAGS,
            params: *const DISPPARAMS,
            _result: *mut VARIANT,
            _exception: *mut EXCEPINFO,
            _argument_error: *mut u32,
        ) -> Result<()> {
            let params = unsafe { params.as_ref() }.ok_or_else(|| Error::from_hresult(E_POINTER))?;
            assert_eq!(params.cArgs, 1);
            let argument = variant_i32_value(unsafe { &*params.rgvarg }, ptr::null_mut())?;
            self.seen.lock().expect("events are available").push((dispid, argument));
            Ok(())
        }
    }

    #[test]
    fn focus_release_hotkey_raises_on_focus_released_with_its_direction() {
        let control = Control::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let dispatch: IDispatch = FocusReleasedSink {
            seen: Arc::clone(&seen),
        }
        .into();
        control.sinks.borrow_mut().insert(1, EventSink { cookie: 1, dispatch });

        assert!(!handle_hotkey(
            &control,
            HWND::default(),
            key(VK_RIGHT, true, true, true)
        ));
        control.compatibility.borrow_mut().input_policy.focus_release_events = true;
        assert!(handle_hotkey(
            &control,
            HWND::default(),
            key(VK_RIGHT, true, true, true)
        ));
        assert!(handle_hotkey(&control, HWND::default(), key(VK_DOWN, true, true, true)));

        assert_eq!(
            *seen.lock().expect("events are available"),
            [(DISPID_ON_FOCUS_RELEASED, 1), (DISPID_ON_FOCUS_RELEASED, -2)]
        );
    }

    #[test]
    fn accelerator_passthrough_keeps_alt_messages_for_the_session() {
        let mut policy = InputPolicy::default();
        for message in [WM_SYSKEYDOWN, WM_SYSKEYUP, WM_SYSCHAR] {
            assert!(!offers_accelerator_to_container(&policy, message));
        }
        assert!(offers_accelerator_to_container(&policy, WM_KEYDOWN));

        policy.accelerator_passthrough = false;
        assert!(offers_accelerator_to_container(&policy, WM_SYSKEYDOWN));
    }

    #[test]
    fn ignored_client_mouse_input_stops_local_mouse_messages() {
        let control = Control::new();
        assert!(local_mouse_input_enabled(&control));
        control
            .compatibility
            .borrow_mut()
            .input_policy
            .ignore_client_mouse_input = true;
        assert!(!local_mouse_input_enabled(&control));
    }

    #[test]
    fn wheel_deltas_split_into_fast_path_sized_rotations() {
        let rotations = |delta| {
            wheel_operations(true, delta)
                .into_iter()
                .map(|operation| match operation {
                    Operation::WheelRotations(rotation) => rotation.rotation_units,
                    other => panic!("unexpected operation {other:?}"),
                })
                .collect::<Vec<_>>()
        };
        assert!(rotations(0).is_empty());
        assert_eq!(rotations(120), [120]);
        assert_eq!(rotations(600), [255, 255, 90]);
        assert_eq!(rotations(-600), [-256, -256, -88]);
    }

    proptest::proptest! {
        /// The emitted deltas each fit the 16-bit wire fields and add up to the raw movement.
        #[test]
        fn relative_moves_split_without_losing_movement(x in -5_000_000i32..=5_000_000, y in -5_000_000i32..=5_000_000) {
            let (mut total_x, mut total_y) = (0i64, 0i64);
            for event in relative_moves(x, y) {
                let FastPathInputEvent::MouseEventRel(pdu) = event else {
                    proptest::prop_assert!(false, "unexpected event {event:?}");
                    unreachable!();
                };
                proptest::prop_assert_eq!(pdu.flags, PointerRelFlags::MOVE);
                proptest::prop_assert!(pdu.x_delta != 0 || pdu.y_delta != 0, "empty delta");
                total_x += i64::from(pdu.x_delta);
                total_y += i64::from(pdu.y_delta);
            }
            proptest::prop_assert_eq!((total_x, total_y), (i64::from(x), i64::from(y)));
        }
    }
}
