use std::collections::BTreeMap;

use cef::{ImplBrowser as _, ImplBrowserHost as _};
use ironrdp_server::{
    KeyboardEvent, MouseButton, MouseEvent, PenContact, PenContactFlags, PenEventPdu, PenFlags, TouchContact,
    TouchContactFlags, TouchEventPdu,
};

use crate::model::{Action, CHROME_HEIGHT, Key, KeyResult, Model, Modifiers};
use crate::native::Engine;
use crate::platform::Platform;

const MAX_CONCURRENT_TOUCHES: usize = 16;
const MAX_CONCURRENT_PENS: usize = 5;
const PEN_ID_OFFSET: i32 = 256;

fn cef_flag(flag: cef::sys::cef_event_flags_t) -> u32 {
    // CEF binds this mask as signed on Windows and unsigned on Unix.
    u32::from_ne_bytes(flag.0.to_ne_bytes())
}

#[derive(Default)]
pub struct Input {
    pub modifiers: Modifiers,
    position: Option<(i32, i32)>,
    keys: BTreeMap<u16, (i32, cef::KeyEvent)>,
    buttons: BTreeMap<u8, i32>,
    touches: BTreeMap<u8, (i32, cef::TouchEvent)>,
    pens: BTreeMap<u8, (i32, cef::TouchEvent)>,
}

impl Input {
    pub fn position(&self) -> Option<(i32, i32)> {
        self.position
    }

    pub fn clear_position(&mut self) {
        self.position = None;
    }

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
        self.cancel_touches(engine);
    }

    pub fn cancel_touches(&mut self, engine: &Engine) {
        for (_, (id, mut event)) in std::mem::take(&mut self.touches)
            .into_iter()
            .chain(std::mem::take(&mut self.pens))
        {
            event.type_ = cef::TouchEventType::CANCELLED;
            event.modifiers = 0;
            if let Some(host) = engine.browser(id).and_then(|browser| browser.host()) {
                host.send_touch_event(Some(&event));
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

    fn coords(&self) -> (i32, i32) {
        self.position
            .unwrap_or((0, i32::try_from(crate::model::CHROME_HEIGHT).expect("chrome height")))
    }

    fn mouse_event(&self) -> cef::MouseEvent {
        let (x, y) = self.coords();
        cef::MouseEvent {
            x,
            y: y - i32::try_from(crate::model::CHROME_HEIGHT).expect("chrome height"),
            modifiers: self.flags(),
        }
    }

    pub fn mouse(&mut self, model: &mut Model, engine: &Engine, event: MouseEvent) -> Option<Action> {
        match event {
            MouseEvent::Move { x, y } => self.position = Some((i32::from(x), i32::from(y))),
            MouseEvent::RelMove { x, y } => {
                let (cx, cy) = self.coords();
                self.position = Some((cx.saturating_add(x), cy.saturating_add(y)));
            }
            MouseEvent::Button { x, y, button, pressed } => {
                self.position = Some((i32::from(x), i32::from(y)));
                return self.button(model, engine, button, pressed);
            }
            MouseEvent::ButtonRel { x, y, button, pressed } => {
                let (cx, cy) = self.coords();
                self.position = Some((cx.saturating_add(x), cy.saturating_add(y)));
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
        let (px, py) = self.coords();
        if let Some(host) = engine.browser(model.active_id()).and_then(|browser| browser.host()) {
            let leave = model.content_point(px, py).is_none();
            host.send_mouse_move_event(Some(&self.mouse_event()), i32::from(leave));
        }
        None
    }

    fn wheel(&self, model: &Model, engine: &Engine, x: i32, y: i32) {
        let (px, py) = self.coords();
        if model.content_point(px, py).is_some()
            && let Some(host) = engine.browser(model.active_id()).and_then(|browser| browser.host())
        {
            host.send_mouse_wheel_event(Some(&self.mouse_event()), x, y);
        }
    }

    fn button(&mut self, model: &mut Model, engine: &Engine, button: MouseButton, pressed: bool) -> Option<Action> {
        let (px, py) = self.coords();
        if pressed && matches!(button, MouseButton::Left) {
            let action = model.click(px, py);
            if action.is_some() || model.content_point(px, py).is_none() {
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
            model.content_point(px, py)?;
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

    pub fn touch(&mut self, model: &mut Model, engine: &Engine, pdu: TouchEventPdu) -> Option<Action> {
        let mut pending_action = None;
        for frame in pdu.frames {
            for contact in frame.contacts {
                let mut emitted = Vec::new();
                if let Some(action) = self.apply_touch_contact(model, &contact, &mut emitted) {
                    self.release_content(engine);
                    if let Some(host) = engine.browser(model.active_id()).and_then(|browser| browser.host()) {
                        host.set_focus(i32::from(!model.address_focused()));
                    }
                    pending_action = Some(action);
                    break;
                }
                for (tab, focus, event) in emitted {
                    if let Some(host) = engine.browser(tab).and_then(|browser| browser.host()) {
                        if focus {
                            host.set_focus(1);
                        }
                        host.send_touch_event(Some(&event));
                    }
                }
            }
            if pending_action.is_some() {
                break;
            }
        }
        pending_action
    }

    fn apply_touch_contact(
        &mut self,
        model: &mut Model,
        contact: &TouchContact,
        emitted: &mut Vec<(i32, bool, cef::TouchEvent)>,
    ) -> Option<Action> {
        let flags = contact.contact_flags;
        let modifiers = self.flags();
        if flags.contains(TouchContactFlags::CANCELED) {
            if let Some((tab, mut prev)) = self.touches.remove(&contact.contact_id) {
                prev.type_ = cef::TouchEventType::CANCELLED;
                prev.modifiers = modifiers;
                emitted.push((tab, false, prev));
            }
            return None;
        }
        if flags.contains(TouchContactFlags::UP) {
            if let Some((tab, _)) = self.touches.remove(&contact.contact_id) {
                emitted.push((
                    tab,
                    false,
                    cef_touch_event(contact, cef::TouchEventType::RELEASED, modifiers),
                ));
            }
            return None;
        }
        if flags.contains(TouchContactFlags::DOWN) && flags.contains(TouchContactFlags::INCONTACT) {
            if self.touches.is_empty() && model.content_point(contact.x, contact.y).is_none() {
                return model.click(contact.x, contact.y);
            }
            model.content_point(contact.x, contact.y)?;
            if let Some((old_tab, mut prev)) = self.touches.remove(&contact.contact_id) {
                prev.type_ = cef::TouchEventType::CANCELLED;
                prev.modifiers = modifiers;
                emitted.push((old_tab, false, prev));
            }
            if self.touches.len() >= MAX_CONCURRENT_TOUCHES {
                return None;
            }
            let tab = model.active_id();
            let event = cef_touch_event(contact, cef::TouchEventType::PRESSED, modifiers);
            self.touches.insert(contact.contact_id, (tab, event.clone()));
            emitted.push((tab, true, event));
            return None;
        }
        if flags.contains(TouchContactFlags::UPDATE)
            && flags.contains(TouchContactFlags::INCONTACT)
            && let Some((tab, prev)) = self.touches.get_mut(&contact.contact_id)
        {
            let event = cef_touch_event(contact, cef::TouchEventType::MOVED, modifiers);
            *prev = event.clone();
            emitted.push((*tab, false, event));
        }
        None
    }

    pub fn pen(&mut self, model: &mut Model, engine: &Engine, pdu: PenEventPdu) -> Option<Action> {
        let mut pending_action = None;
        for frame in pdu.frames {
            for contact in frame.contacts {
                let mut emitted = Vec::new();
                if let Some(action) = self.apply_pen_contact(model, &contact, &mut emitted) {
                    self.release_content(engine);
                    if let Some(host) = engine.browser(model.active_id()).and_then(|browser| browser.host()) {
                        host.set_focus(i32::from(!model.address_focused()));
                    }
                    pending_action = Some(action);
                    break;
                }
                for (tab, focus, event) in emitted {
                    if let Some(host) = engine.browser(tab).and_then(|browser| browser.host()) {
                        if focus {
                            host.set_focus(1);
                        }
                        host.send_touch_event(Some(&event));
                    }
                }
            }
            if pending_action.is_some() {
                break;
            }
        }
        pending_action
    }

    fn apply_pen_contact(
        &mut self,
        model: &mut Model,
        contact: &PenContact,
        emitted: &mut Vec<(i32, bool, cef::TouchEvent)>,
    ) -> Option<Action> {
        let flags = contact.contact_flags;
        let modifiers = self.flags();
        if flags.contains(PenContactFlags::CANCELED) {
            if let Some((tab, mut prev)) = self.pens.remove(&contact.device_id) {
                prev.type_ = cef::TouchEventType::CANCELLED;
                prev.modifiers = modifiers;
                emitted.push((tab, false, prev));
            }
            return None;
        }
        if flags.contains(PenContactFlags::UP) {
            if let Some((tab, prev)) = self.pens.remove(&contact.device_id) {
                emitted.push((
                    tab,
                    false,
                    cef_pen_event(
                        contact,
                        cef::TouchEventType::RELEASED,
                        modifiers,
                        Some(prev.pointer_type),
                    ),
                ));
            }
            return None;
        }
        if flags.contains(PenContactFlags::DOWN) && flags.contains(PenContactFlags::INCONTACT) {
            if self.touches.is_empty() && self.pens.is_empty() && model.content_point(contact.x, contact.y).is_none() {
                return model.click(contact.x, contact.y);
            }
            model.content_point(contact.x, contact.y)?;
            if let Some((old_tab, mut prev)) = self.pens.remove(&contact.device_id) {
                prev.type_ = cef::TouchEventType::CANCELLED;
                prev.modifiers = modifiers;
                emitted.push((old_tab, false, prev));
            }
            if self.pens.len() + self.touches.len() >= MAX_CONCURRENT_TOUCHES || self.pens.len() >= MAX_CONCURRENT_PENS
            {
                return None;
            }
            let tab = model.active_id();
            let event = cef_pen_event(contact, cef::TouchEventType::PRESSED, modifiers, None);
            self.pens.insert(contact.device_id, (tab, event.clone()));
            emitted.push((tab, true, event));
            return None;
        }
        if flags.contains(PenContactFlags::UPDATE)
            && flags.contains(PenContactFlags::INCONTACT)
            && let Some((tab, prev)) = self.pens.get_mut(&contact.device_id)
        {
            let event = cef_pen_event(contact, cef::TouchEventType::MOVED, modifiers, Some(prev.pointer_type));
            *prev = event.clone();
            emitted.push((*tab, false, event));
        }
        None
    }
}

#[expect(
    clippy::as_conversions,
    reason = "CEF touch coordinates, radii, angles, and normalized pressure use f32"
)]
fn cef_touch_event(contact: &TouchContact, type_: cef::TouchEventType, modifiers: u32) -> cef::TouchEvent {
    let chrome_y = i32::try_from(CHROME_HEIGHT).expect("chrome height");
    let (radius_x, radius_y) = match (
        contact.fields.contact_rect_left,
        contact.fields.contact_rect_top,
        contact.fields.contact_rect_right,
        contact.fields.contact_rect_bottom,
    ) {
        (Some(left), Some(top), Some(right), Some(bottom)) => (
            ((i32::from(right) - i32::from(left)).max(0) as f32) * 0.5,
            ((i32::from(bottom) - i32::from(top)).max(0) as f32) * 0.5,
        ),
        _ => (0.0, 0.0),
    };
    let rotation_angle = contact
        .fields
        .orientation
        .map_or(0.0, |degrees| ((degrees % 360) as f32).to_radians());
    let pressure = contact
        .fields
        .pressure
        .map_or(0.0, |value| (value.min(1024) as f32) / 1024.0);
    cef::TouchEvent {
        id: i32::from(contact.contact_id),
        x: contact.x as f32,
        y: contact.y.saturating_sub(chrome_y) as f32,
        radius_x,
        radius_y,
        rotation_angle,
        pressure,
        type_,
        modifiers,
        pointer_type: cef::PointerType::TOUCH,
    }
}

#[expect(
    clippy::as_conversions,
    reason = "CEF pen coordinates, angles, and normalized pressure use f32"
)]
fn cef_pen_event(
    contact: &PenContact,
    type_: cef::TouchEventType,
    modifiers: u32,
    fallback_pointer: Option<cef::PointerType>,
) -> cef::TouchEvent {
    let chrome_y = i32::try_from(CHROME_HEIGHT).expect("chrome height");
    let pointer_type = fallback_pointer.unwrap_or_else(|| match contact.fields.pen_flags {
        Some(flags) if flags.intersects(PenFlags::INVERTED | PenFlags::ERASER_PRESSED) => cef::PointerType::ERASER,
        _ => cef::PointerType::PEN,
    });
    let rotation_angle = contact
        .fields
        .rotation
        .map_or(0.0, |degrees| (f32::from(degrees % 360)).to_radians());
    let pressure = contact
        .fields
        .pressure
        .map_or(0.0, |value| (value.min(1024) as f32) / 1024.0);
    cef::TouchEvent {
        id: PEN_ID_OFFSET + i32::from(contact.device_id),
        x: contact.x as f32,
        y: contact.y.saturating_sub(chrome_y) as f32,
        radius_x: 0.0,
        radius_y: 0.0,
        rotation_angle,
        pressure,
        type_,
        modifiers,
        pointer_type,
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
        0x52 => (45, 114),
        0x3b => (112, 122),
        0x3c => (113, 120),
        0x3d => (114, 99),
        0x3e => (115, 118),
        0x40 => (117, 97),
        0x41 => (118, 98),
        0x42 => (119, 100),
        0x43 => (120, 101),
        0x44 => (121, 109),
        0x57 => (122, 103),
        0x58 => (123, 111),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_touch_lifecycle_maps_geometry_orientation_and_pressure() {
        // oracle: spec MS-RDPEI §2.2.3.3.1.1; mutant omitting CHROME_HEIGHT offset, degree-to-radian conversion, or [0,1] pressure scaling.
        let mut model = Model::new(800, 600).unwrap();
        let mut input = Input::default();
        let mut emitted = Vec::new();
        let down = TouchContact::new(
            3,
            120,
            100,
            TouchContactFlags::DOWN | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT,
        )
        .with_contact_rect(-4, -6, 4, 6)
        .with_orientation(90)
        .with_pressure(512);
        assert!(input.apply_touch_contact(&mut model, &down, &mut emitted).is_none());
        assert_eq!(emitted.len(), 1);
        let (tab, focus, pressed) = &emitted[0];
        assert_eq!(*tab, model.active_id());
        assert!(*focus);
        assert_eq!(pressed.id, 3);
        assert_eq!(pressed.x, 120.0);
        assert_eq!(pressed.y, (100 - i32::try_from(CHROME_HEIGHT).unwrap()) as f32);
        assert_eq!(pressed.radius_x, 4.0);
        assert_eq!(pressed.radius_y, 6.0);
        assert!((pressed.rotation_angle - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((pressed.pressure - 0.5).abs() < 1e-5);
        assert_eq!(pressed.type_, cef::TouchEventType::PRESSED);
        assert_eq!(pressed.pointer_type, cef::PointerType::TOUCH);

        emitted.clear();
        let moved = TouchContact::new(
            3,
            140,
            20,
            TouchContactFlags::UPDATE | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT,
        );
        assert!(input.apply_touch_contact(&mut model, &moved, &mut emitted).is_none());
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].2.type_, cef::TouchEventType::MOVED);
        assert_eq!(emitted[0].2.x, 140.0);
        assert_eq!(emitted[0].2.y, (20 - i32::try_from(CHROME_HEIGHT).unwrap()) as f32);

        emitted.clear();
        let up = TouchContact::new(3, 140, 20, TouchContactFlags::UP);
        assert!(input.apply_touch_contact(&mut model, &up, &mut emitted).is_none());
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].2.type_, cef::TouchEventType::RELEASED);
        assert!(input.touches.is_empty());
    }

    #[test]
    fn duplicate_down_cancels_previous_contact_and_enforces_sixteen_touch_limit() {
        // oracle: CEF _cef_touch_event_t contract requires PRESSED -> MOVED* -> RELEASED|CANCELLED and caps concurrent touches at 16.
        let mut model = Model::new(800, 600).unwrap();
        let mut input = Input::default();
        let flags = TouchContactFlags::DOWN | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT;
        for id in 0..18u8 {
            let mut emitted = Vec::new();
            let contact = TouchContact::new(id, 100, 100, flags);
            assert!(input.apply_touch_contact(&mut model, &contact, &mut emitted).is_none());
            assert_eq!(emitted.len(), usize::from(id < 16));
        }
        assert_eq!(input.touches.len(), 16);

        let mut emitted = Vec::new();
        let duplicate = TouchContact::new(0, 110, 110, flags);
        assert!(
            input
                .apply_touch_contact(&mut model, &duplicate, &mut emitted)
                .is_none()
        );
        assert_eq!(emitted.len(), 2);
        assert_eq!(emitted[0].2.type_, cef::TouchEventType::CANCELLED);
        assert_eq!(emitted[1].2.type_, cef::TouchEventType::PRESSED);
    }

    #[test]
    fn toolbar_tap_invokes_chrome_action_without_tracking_content_touch() {
        // oracle: mutant dropping toolbar taps instead of triggering chrome navigation actions.
        let mut model = Model::new(800, 600).unwrap();
        let tab = model.new_tab("https://example.test").unwrap();
        let mut input = Input::default();
        let mut emitted = Vec::new();
        let reload_tap = TouchContact::new(
            1,
            80,
            56,
            TouchContactFlags::DOWN | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT,
        );
        let action = input.apply_touch_contact(&mut model, &reload_tap, &mut emitted);
        assert_eq!(action, Some(Action::Reload { id: tab }));
        assert!(emitted.is_empty());
        assert!(input.touches.is_empty());
    }

    #[test]
    fn pen_contacts_map_stylus_and_eraser_without_colliding_with_finger_ids() {
        // oracle: spec MS-RDPEI §2.2.3.7.1.1; mutant colliding pen device_id 0 with touch contact_id 0 or ignoring INVERTED/ERASER_PRESSED.
        let mut model = Model::new(800, 600).unwrap();
        model.new_tab("https://example.test").unwrap();
        let mut input = Input::default();
        let mut emitted = Vec::new();
        let down = PenContact::new(
            0,
            200,
            188,
            PenContactFlags::DOWN | PenContactFlags::INRANGE | PenContactFlags::INCONTACT,
        )
        .with_pen_flags(PenFlags::INVERTED)
        .with_pressure(256)
        .with_rotation(180);
        assert!(input.apply_pen_contact(&mut model, &down, &mut emitted).is_none());
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].2.id, PEN_ID_OFFSET);
        assert_eq!(emitted[0].2.pointer_type, cef::PointerType::ERASER);
        assert!((emitted[0].2.pressure - 0.25).abs() < 1e-5);
        assert!((emitted[0].2.rotation_angle - std::f32::consts::PI).abs() < 1e-5);

        emitted.clear();
        let up = PenContact::new(0, 200, 188, PenContactFlags::UP).with_pen_flags(PenFlags::empty());
        assert!(input.apply_pen_contact(&mut model, &up, &mut emitted).is_none());
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].2.type_, cef::TouchEventType::RELEASED);
        assert_eq!(emitted[0].2.pointer_type, cef::PointerType::ERASER);
        assert!(input.pens.is_empty());
    }
}
