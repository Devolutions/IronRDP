use std::collections::BTreeMap;

use cef::{ImplBrowser as _, ImplBrowserHost as _};
use ironrdp_server::{KeyboardEvent, MouseButton, MouseEvent};

use crate::model::{Action, Key, KeyResult, Model, Modifiers};
use crate::native::Engine;
use crate::platform::Platform;

fn cef_flag(flag: cef::sys::cef_event_flags_t) -> u32 {
    // CEF binds this mask as signed on Windows and unsigned on Unix.
    u32::from_ne_bytes(flag.0.to_ne_bytes())
}

#[derive(Default)]
pub struct Input {
    pub modifiers: Modifiers,
    position: (i32, i32),
    keys: BTreeMap<u16, (i32, cef::KeyEvent)>,
    buttons: BTreeMap<u8, i32>,
}

impl Input {
    fn flags(&self) -> u32 {
        let mut flags = 0;
        if self.modifiers.shift {
            flags |= cef_flag(cef::sys::cef_event_flags_t::EVENTFLAG_SHIFT_DOWN);
        }
        if self.modifiers.ctrl {
            flags |= cef_flag(cef::sys::cef_event_flags_t::EVENTFLAG_CONTROL_DOWN);
        }
        if self.modifiers.alt {
            flags |= cef_flag(cef::sys::cef_event_flags_t::EVENTFLAG_ALT_DOWN);
        }
        if self.modifiers.meta {
            flags |= cef_flag(cef::sys::cef_event_flags_t::EVENTFLAG_COMMAND_DOWN);
        }
        for &button in self.buttons.keys() {
            flags |= cef_flag(match button {
                0 => cef::sys::cef_event_flags_t::EVENTFLAG_LEFT_MOUSE_BUTTON,
                1 => cef::sys::cef_event_flags_t::EVENTFLAG_MIDDLE_MOUSE_BUTTON,
                _ => cef::sys::cef_event_flags_t::EVENTFLAG_RIGHT_MOUSE_BUTTON,
            });
        }
        flags
    }

    pub fn release_content(&mut self, engine: &Engine) {
        for (scan, (id, mut event)) in std::mem::take(&mut self.keys) {
            event.type_ = cef::KeyEventType::KEYUP;
            event.modifiers = 0;
            event.is_system_key = i32::from(Platform::current().is_alt_system_key(scan & 0xff == 0x38));
            if let Some(host) = engine.browser(id).and_then(|browser| browser.host()) {
                host.send_key_event(Some(&event));
            }
        }
        for (button, id) in std::mem::take(&mut self.buttons) {
            if let Some(host) = engine.browser(id).and_then(|browser| browser.host()) {
                host.send_mouse_click_event(Some(&self.mouse_event()), cef_button(button), 1, 1);
            }
        }
    }

    pub fn keyboard(&mut self, model: &mut Model, engine: &Engine, event: KeyboardEvent) -> Option<Action> {
        match event {
            KeyboardEvent::UnicodePressed(unit) => {
                if !model.text_utf16(unit) {
                    if let Some(host) = engine.browser(model.active_id()).and_then(|browser| browser.host()) {
                        host.send_key_event(Some(&cef::KeyEvent {
                            type_: cef::KeyEventType::CHAR,
                            character: unit,
                            unmodified_character: unit,
                            windows_key_code: i32::from(unit),
                            modifiers: self.flags(),
                            ..Default::default()
                        }));
                    }
                }
                None
            }
            KeyboardEvent::UnicodeReleased(_) | KeyboardEvent::Synchronize(_) => None,
            KeyboardEvent::Pressed { code, extended } => self.scancode(model, engine, code, extended, true),
            KeyboardEvent::Released { code, extended } => self.scancode(model, engine, code, extended, false),
        }
    }

    fn scancode(
        &mut self,
        model: &mut Model,
        engine: &Engine,
        code: u8,
        extended: bool,
        pressed: bool,
    ) -> Option<Action> {
        match code {
            0x1d => self.modifiers.ctrl = pressed,
            0x2a | 0x36 => self.modifiers.shift = pressed,
            0x38 => self.modifiers.alt = pressed,
            0x5b | 0x5c => self.modifiers.meta = pressed,
            _ => {}
        }
        let scan = u16::from(code) | if extended { 0x100 } else { 0 };
        if !pressed {
            if let Some((tab, mut event)) = self.keys.remove(&scan) {
                event.type_ = cef::KeyEventType::KEYUP;
                event.modifiers = self.flags();
                event.is_system_key =
                    i32::from(Platform::current().is_alt_system_key(self.modifiers.alt || code == 0x38));
                if let Some(host) = engine.browser(tab).and_then(|browser| browser.host()) {
                    host.send_key_event(Some(&event));
                }
            }
            return None;
        }
        let Some((virtual_key, native, character, key)) = mapping(code, self.modifiers.shift) else {
            tracing::warn!(code, extended, "Unsupported keyboard scancode");
            return None;
        };
        let Some(native) = Platform::current().native_key_code(code, extended, native) else {
            tracing::warn!(code, extended, "Unsupported platform keyboard scancode");
            return None;
        };
        if let Some(key) = key {
            match model.key(key, self.modifiers) {
                KeyResult::Action(action) => {
                    self.release_content(engine);
                    return Some(action);
                }
                KeyResult::Consumed => {
                    self.release_content(engine);
                    if let Some(host) = engine.browser(model.active_id()).and_then(|browser| browser.host()) {
                        host.set_focus(i32::from(!model.address_focused()));
                    }
                    return None;
                }
                KeyResult::Unhandled => {}
            }
        }
        if model.address_focused() {
            if character != 0 && !self.modifiers.ctrl && !self.modifiers.alt && !self.modifiers.meta {
                model.text_utf16(character);
            }
            return None;
        }
        let tab = model.active_id();
        if let Some(host) = engine.browser(tab).and_then(|browser| browser.host()) {
            let mut modifiers = self.flags();
            // UNTESTED: Consecutive RDP key-downs must retain CEF's repeat flag for web pages.
            // The agent inserts a key-up between repeated presses, so check this with a raw-repeat RDP client when changing input.
            if self.keys.contains_key(&scan) {
                modifiers |= cef_flag(cef::sys::cef_event_flags_t::EVENTFLAG_IS_REPEAT);
            }
            let event = cef::KeyEvent {
                type_: cef::KeyEventType::RAWKEYDOWN,
                windows_key_code: virtual_key,
                native_key_code: native,
                character,
                unmodified_character: character,
                modifiers,
                is_system_key: i32::from(Platform::current().is_alt_system_key(self.modifiers.alt)),
                ..Default::default()
            };
            host.send_key_event(Some(&event));
            self.keys.insert(scan, (tab, event.clone()));
            // macOS Chromium binds editing commands to Command, but RDP clients send Control.
            #[cfg(target_os = "macos")]
            if self.modifiers.ctrl && !self.modifiers.alt && !self.modifiers.meta {
                use cef::{ImplBrowser as _, ImplFrame as _};

                if let Some(frame) = engine.browser(tab).and_then(|browser| browser.focused_frame()) {
                    match code {
                        0x1e => frame.select_all(),
                        0x2d => frame.cut(),
                        0x2e => frame.copy(),
                        0x2f => frame.paste(),
                        _ => {}
                    }
                }
            }
            if character != 0 && !self.modifiers.ctrl && !self.modifiers.alt && !self.modifiers.meta {
                let mut event = event;
                event.type_ = cef::KeyEventType::CHAR;
                event.windows_key_code = Platform::current().character_key_code(event.windows_key_code, character);
                host.send_key_event(Some(&event));
            }
        }
        None
    }

    fn mouse_event(&self) -> cef::MouseEvent {
        cef::MouseEvent {
            x: self.position.0,
            y: self.position.1 - i32::try_from(crate::model::CHROME_HEIGHT).expect("chrome height"),
            modifiers: self.flags(),
        }
    }

    pub fn mouse(&mut self, model: &mut Model, engine: &Engine, event: MouseEvent) -> Option<Action> {
        match event {
            MouseEvent::Move { x, y } => self.position = (i32::from(x), i32::from(y)),
            MouseEvent::RelMove { x, y } => {
                self.position.0 = self.position.0.saturating_add(x);
                self.position.1 = self.position.1.saturating_add(y);
            }
            MouseEvent::Button { x, y, button, pressed } => {
                self.position = (i32::from(x), i32::from(y));
                return self.button(model, engine, button, pressed);
            }
            MouseEvent::ButtonRel { x, y, button, pressed } => {
                self.position.0 = self.position.0.saturating_add(x);
                self.position.1 = self.position.1.saturating_add(y);
                return self.button(model, engine, button, pressed);
            }
            MouseEvent::VerticalScroll { value } => {
                self.wheel(model, engine, 0, i32::from(value));
                return None;
            }
            MouseEvent::HorizontalScroll { value } => {
                self.wheel(model, engine, i32::from(value), 0);
                return None;
            }
            MouseEvent::Scroll { x, y } => {
                self.wheel(model, engine, x, y);
                return None;
            }
            _ => {
                tracing::warn!("Unsupported mouse event");
                return None;
            }
        }
        if let Some(host) = engine.browser(model.active_id()).and_then(|browser| browser.host()) {
            let leave = model.content_point(self.position.0, self.position.1).is_none();
            host.send_mouse_move_event(Some(&self.mouse_event()), i32::from(leave));
        }
        None
    }

    fn wheel(&self, model: &Model, engine: &Engine, x: i32, y: i32) {
        if model.content_point(self.position.0, self.position.1).is_some() {
            if let Some(host) = engine.browser(model.active_id()).and_then(|browser| browser.host()) {
                host.send_mouse_wheel_event(Some(&self.mouse_event()), x, y);
            }
        }
    }

    fn button(&mut self, model: &mut Model, engine: &Engine, button: MouseButton, pressed: bool) -> Option<Action> {
        if pressed && matches!(button, MouseButton::Left) {
            let action = model.click(self.position.0, self.position.1);
            if action.is_some() || model.content_point(self.position.0, self.position.1).is_none() {
                self.release_content(engine);
                if let Some(host) = engine.browser(model.active_id()).and_then(|browser| browser.host()) {
                    host.set_focus(i32::from(!model.address_focused()));
                }
                return action;
            }
        }
        let index = match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            MouseButton::X1 if pressed => return Some(Action::Back { id: model.active_id() }),
            MouseButton::X2 if pressed => return Some(Action::Forward { id: model.active_id() }),
            _ => return None,
        };
        let tab = if pressed {
            model.content_point(self.position.0, self.position.1)?;
            let tab = model.active_id();
            self.buttons.insert(index, tab);
            tab
        } else {
            self.buttons.remove(&index)?
        };
        if let Some(host) = engine.browser(tab).and_then(|browser| browser.host()) {
            host.set_focus(1);
            host.send_mouse_click_event(Some(&self.mouse_event()), cef_button(index), i32::from(!pressed), 1);
        }
        None
    }
}

fn cef_button(index: u8) -> cef::MouseButtonType {
    match index {
        0 => cef::MouseButtonType::LEFT,
        1 => cef::MouseButtonType::MIDDLE,
        _ => cef::MouseButtonType::RIGHT,
    }
}

fn mapping(code: u8, shift: bool) -> Option<(i32, i32, u16, Option<Key>)> {
    let special = match code {
        0x01 => Some((27, 53, 27, Key::Escape)),
        0x0e => Some((8, 51, 8, Key::Backspace)),
        0x0f => Some((9, 48, 9, Key::Tab)),
        0x1c => Some((13, 36, 13, Key::Enter)),
        0x47 => Some((36, 115, 0, Key::Home)),
        0x4f => Some((35, 119, 0, Key::End)),
        0x4b => Some((37, 123, 0, Key::Left)),
        0x4d => Some((39, 124, 0, Key::Right)),
        0x53 => Some((46, 117, 0, Key::Delete)),
        0x3f => Some((116, 96, 0, Key::F5)),
        _ => None,
    };
    if let Some((vk, native, ch, key)) = special {
        return Some((vk, native, ch, Some(key)));
    }
    let (vk, native) = match code {
        0x1d => (17, 59),
        0x2a => (16, 56),
        0x36 => (16, 60),
        0x38 => (18, 58),
        0x5b | 0x5c => (91, 55),
        0x48 => (38, 126),
        0x50 => (40, 125),
        0x49 => (33, 116),
        0x51 => (34, 121),
        _ => (0, 0),
    };
    if vk != 0 {
        return Some((vk, native, 0, None));
    }
    let (plain, upper, native) = match code {
        0x1e => ('a', 'A', 0),
        0x30 => ('b', 'B', 11),
        0x2e => ('c', 'C', 8),
        0x20 => ('d', 'D', 2),
        0x12 => ('e', 'E', 14),
        0x21 => ('f', 'F', 3),
        0x22 => ('g', 'G', 5),
        0x23 => ('h', 'H', 4),
        0x17 => ('i', 'I', 34),
        0x24 => ('j', 'J', 38),
        0x25 => ('k', 'K', 40),
        0x26 => ('l', 'L', 37),
        0x32 => ('m', 'M', 46),
        0x31 => ('n', 'N', 45),
        0x18 => ('o', 'O', 31),
        0x19 => ('p', 'P', 35),
        0x10 => ('q', 'Q', 12),
        0x13 => ('r', 'R', 15),
        0x1f => ('s', 'S', 1),
        0x14 => ('t', 'T', 17),
        0x16 => ('u', 'U', 32),
        0x2f => ('v', 'V', 9),
        0x11 => ('w', 'W', 13),
        0x2d => ('x', 'X', 7),
        0x15 => ('y', 'Y', 16),
        0x2c => ('z', 'Z', 6),
        0x02 => ('1', '!', 18),
        0x03 => ('2', '@', 19),
        0x04 => ('3', '#', 20),
        0x05 => ('4', '$', 21),
        0x06 => ('5', '%', 23),
        0x07 => ('6', '^', 22),
        0x08 => ('7', '&', 26),
        0x09 => ('8', '*', 28),
        0x0a => ('9', '(', 25),
        0x0b => ('0', ')', 29),
        0x39 => (' ', ' ', 49),
        0x0c => ('-', '_', 27),
        0x0d => ('=', '+', 24),
        0x1a => ('[', '{', 33),
        0x1b => (']', '}', 30),
        0x27 => (';', ':', 41),
        0x28 => ('\'', '"', 39),
        0x29 => ('`', '~', 50),
        0x2b => ('\\', '|', 42),
        0x33 => (',', '<', 43),
        0x34 => ('.', '>', 47),
        0x35 => ('/', '?', 44),
        _ => return None,
    };
    let character = if shift { upper } else { plain };
    let virtual_key = match code {
        0x0c => 0xbd,
        0x0d => 0xbb,
        0x1a => 0xdb,
        0x1b => 0xdd,
        0x27 => 0xba,
        0x28 => 0xde,
        0x29 => 0xc0,
        0x2b => 0xdc,
        0x33 => 0xbc,
        0x34 => 0xbe,
        0x35 => 0xbf,
        _ => i32::try_from(u32::from(plain.to_ascii_uppercase())).expect("ASCII key"),
    };
    Some((virtual_key, native, character as u16, Some(Key::Character(plain))))
}
