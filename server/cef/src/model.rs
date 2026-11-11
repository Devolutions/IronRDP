use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use anyhow::{bail, ensure};

pub const INITIAL_WIDTH: u32 = 1280;
pub const INITIAL_HEIGHT: u32 = 800;
pub const MIN_WIDTH: u32 = 320;
pub const MIN_HEIGHT: u32 = 240;
pub const MAX_WIDTH: u32 = 3840;
pub const MAX_HEIGHT: u32 = 2160;
pub const MAX_TABS: usize = 8;
pub const CHROME_HEIGHT: u32 = 88;
pub const TAB_STRIP_HEIGHT: u32 = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Navigate { tab: i32, url: String },
    NewTab,
    CloseTab { id: i32 },
    Activate { id: i32 },
    Back { id: i32 },
    Forward { id: i32 },
    Reload { id: i32 },
    CancelDownloads,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyResult {
    Unhandled,
    Consumed,
    Action(Action),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Character(char),
    Enter,
    Escape,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Tab,
    F5,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

#[derive(Debug)]
pub struct Tab {
    pub id: i32,
    pub title: String,
    pub url: String,
    pub loading: bool,
    pub error: Option<String>,
    pixels: Vec<u8>,
    popup: Option<Popup>,
}

#[derive(Debug)]
struct Popup {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

pub struct Model {
    width: u32,
    height: u32,
    generation: u64,
    tabs: Vec<Tab>,
    active: i32,
    next_id: i32,
    address: String,
    focused: bool,
    cursor: usize,
    anchor: usize,
    pending_surrogate: Option<u16>,
    font: Option<FontArc>,
    download_status: String,
}

impl Model {
    pub fn new(width: u32, height: u32) -> anyhow::Result<Self> {
        validate_dimensions(width, height)?;
        let mut model = Self {
            width,
            height,
            generation: 0,
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            address: String::new(),
            focused: false,
            cursor: 0,
            anchor: 0,
            pending_surrogate: None,
            font: None,
            download_status: String::new(),
        };
        model.new_tab("about:blank")?;
        Ok(model)
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn content_dimensions(&self) -> (u32, u32) {
        (
            self.width,
            self.height - CHROME_HEIGHT - if self.download_status.is_empty() { 0 } else { 32 },
        )
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active_id(&self) -> i32 {
        self.active
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn address_focused(&self) -> bool {
        self.focused
    }

    pub fn set_download_status(&mut self, status: &str) -> anyhow::Result<bool> {
        if self.download_status == status {
            return Ok(false);
        }
        let layout_changed = self.download_status.is_empty() != status.is_empty();
        let generation = self
            .generation
            .checked_add(u64::from(layout_changed))
            .ok_or_else(|| anyhow::anyhow!("resize generation exhausted"))?;
        self.download_status = status.to_owned();
        if layout_changed {
            self.generation = generation;
            self.clear_surfaces();
        }
        Ok(true)
    }

    pub fn set_font(&mut self, bytes: Vec<u8>) -> anyhow::Result<()> {
        self.font = Some(FontArc::try_from_vec(bytes).map_err(|_| anyhow::anyhow!("invalid shell font"))?);
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) -> anyhow::Result<()> {
        validate_dimensions(width, height)?;
        if (width, height) == self.dimensions() {
            return Ok(());
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("resize generation exhausted"))?;
        self.width = width;
        self.height = height;
        self.generation = generation;
        self.clear_surfaces();
        Ok(())
    }

    fn clear_surfaces(&mut self) {
        let (width, height) = self.content_dimensions();
        for tab in &mut self.tabs {
            tab.pixels = vec![255; byte_len(width, height)];
            tab.popup = None;
        }
    }

    pub fn new_tab(&mut self, url: &str) -> anyhow::Result<i32> {
        ensure!(self.tabs.len() < MAX_TABS, "tab limit reached");
        let id = self.next_id;
        let next = id
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("tab identifiers exhausted"))?;
        let (width, height) = self.content_dimensions();
        self.tabs.push(Tab {
            id,
            title: String::new(),
            url: url.to_owned(),
            loading: false,
            error: None,
            pixels: vec![255; byte_len(width, height)],
            popup: None,
        });
        self.next_id = next;
        self.activate(id)?;
        Ok(id)
    }

    pub fn activate(&mut self, id: i32) -> anyhow::Result<()> {
        ensure!(self.tabs.iter().any(|tab| tab.id == id), "unknown tab");
        self.active = id;
        self.blur_address();
        Ok(())
    }

    pub fn close_tab(&mut self, id: i32) -> anyhow::Result<()> {
        let index = self.tab_index(id)?;
        self.tabs.remove(index);
        if self.active == id {
            self.active = self
                .tabs
                .get(index.min(self.tabs.len().saturating_sub(1)))
                .map_or(0, |tab| tab.id);
            self.blur_address();
        }
        Ok(())
    }

    pub fn update_tab(
        &mut self,
        id: i32,
        title: Option<&str>,
        url: Option<&str>,
        loading: Option<bool>,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let index = self.tab_index(id)?;
        let tab = &mut self.tabs[index];
        if let Some(title) = title {
            tab.title = title.to_owned();
        }
        if let Some(url) = url {
            tab.url = url.to_owned();
        }
        if let Some(loading) = loading {
            tab.loading = loading;
        }
        if let Some(error) = error {
            tab.error = Some(error.to_owned());
        }
        if id == self.active && !self.focused {
            self.sync_address();
        }
        Ok(())
    }

    pub fn set_error(&mut self, id: i32, error: Option<&str>) -> anyhow::Result<()> {
        let index = self.tab_index(id)?;
        self.tabs[index].error = error.map(str::to_owned);
        Ok(())
    }

    pub fn paint_view(
        &mut self,
        id: i32,
        generation: u64,
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> anyhow::Result<()> {
        let index = self.tab_index(id)?;
        ensure!(generation == self.generation, "stale paint generation");
        ensure!(
            (width, height) == self.content_dimensions(),
            "view paint dimensions do not match"
        );
        ensure!(pixels.len() == byte_len(width, height), "invalid BGRA buffer length");
        self.tabs[index].pixels.copy_from_slice(pixels);
        Ok(())
    }

    pub fn paint_popup(
        &mut self,
        id: i32,
        generation: u64,
        position: (i32, i32),
        width: u32,
        height: u32,
        pixels: &[u8],
    ) -> anyhow::Result<()> {
        let index = self.tab_index(id)?;
        ensure!(generation == self.generation, "stale popup generation");
        ensure!(
            0 < width && width <= MAX_WIDTH && 0 < height && height <= MAX_HEIGHT,
            "invalid popup dimensions"
        );
        ensure!(
            pixels.len() == byte_len(width, height),
            "invalid popup BGRA buffer length"
        );
        self.tabs[index].popup = Some(Popup {
            x: position.0,
            y: position.1,
            width,
            height,
            pixels: pixels.to_vec(),
        });
        Ok(())
    }

    pub fn hide_popup(&mut self, id: i32) -> anyhow::Result<()> {
        let index = self.tab_index(id)?;
        self.tabs[index].popup = None;
        Ok(())
    }

    pub fn content_point(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        let bottom = self.height - if self.download_status.is_empty() { 0 } else { 32 };
        (0 <= x && x < self.width as i32 && CHROME_HEIGHT as i32 <= y && y < bottom as i32)
            .then(|| (x, y - CHROME_HEIGHT as i32))
    }

    pub fn click(&mut self, x: i32, y: i32) -> Option<Action> {
        if !(0 <= x && x < self.width as i32 && 0 <= y && y < self.height as i32) {
            return None;
        }
        if !self.download_status.is_empty() && self.height as i32 - 32 <= y {
            return (self.width as i32 - 96 <= x).then_some(Action::CancelDownloads);
        }
        if y < TAB_STRIP_HEIGHT as i32 {
            self.blur_address();
            let width = self.tab_width();
            let index = x as usize / width;
            if let Some(tab) = self.tabs.get(index) {
                return Some(if width - 24 <= x as usize % width {
                    Action::CloseTab { id: tab.id }
                } else {
                    Action::Activate { id: tab.id }
                });
            }
            return (x < (width * self.tabs.len() + 32) as i32 && self.tabs.len() < MAX_TABS).then_some(Action::NewTab);
        }
        if y < CHROME_HEIGHT as i32 {
            if 104 <= x && x < self.width as i32 - 8 {
                self.focus_address();
                return None;
            }
            self.blur_address();
            if self.active == 0 {
                return None;
            }
            return match x {
                0..32 => Some(Action::Back { id: self.active }),
                32..64 => Some(Action::Forward { id: self.active }),
                64..96 => Some(Action::Reload { id: self.active }),
                _ => None,
            };
        }
        self.blur_address();
        None
    }

    pub fn text(&mut self, text: &str) -> bool {
        if !self.focused {
            return false;
        }
        self.pending_surrogate = None;
        let clean: String = text.chars().filter(|ch| !ch.is_control()).collect();
        self.replace_selection(&clean);
        true
    }

    pub fn text_utf16(&mut self, unit: u16) -> bool {
        if !self.focused {
            return false;
        }
        if let Some(high) = self.pending_surrogate.take() {
            if (0xdc00..=0xdfff).contains(&unit) {
                let scalar = 0x10000 + ((u32::from(high) - 0xd800) << 10) + u32::from(unit) - 0xdc00;
                let mut bytes = [0; 4];
                self.replace_selection(
                    char::from_u32(scalar)
                        .unwrap_or(char::REPLACEMENT_CHARACTER)
                        .encode_utf8(&mut bytes),
                );
                return true;
            }
            self.replace_selection("\u{fffd}");
        }
        if (0xd800..=0xdbff).contains(&unit) {
            self.pending_surrogate = Some(unit);
        } else {
            let ch = char::from_u32(u32::from(unit)).unwrap_or(char::REPLACEMENT_CHARACTER);
            if !ch.is_control() {
                let mut bytes = [0; 4];
                self.replace_selection(ch.encode_utf8(&mut bytes));
            }
        }
        true
    }

    pub fn key(&mut self, key: Key, modifiers: Modifiers) -> KeyResult {
        let command = modifiers.ctrl || modifiers.meta;
        if command {
            match key {
                Key::Character('l' | 'L') => {
                    self.focus_address();
                    return KeyResult::Consumed;
                }
                Key::Character('t' | 'T') => {
                    return if self.tabs.len() < MAX_TABS {
                        KeyResult::Action(Action::NewTab)
                    } else {
                        KeyResult::Consumed
                    };
                }
                Key::Character('w' | 'W') if self.active != 0 => {
                    return KeyResult::Action(Action::CloseTab { id: self.active });
                }
                Key::Character('r' | 'R') if self.active != 0 => {
                    return KeyResult::Action(Action::Reload { id: self.active });
                }
                Key::Tab if !self.tabs.is_empty() => {
                    let index = self.tabs.iter().position(|tab| tab.id == self.active).unwrap_or(0);
                    let next = if modifiers.shift {
                        (index + self.tabs.len() - 1) % self.tabs.len()
                    } else {
                        (index + 1) % self.tabs.len()
                    };
                    return KeyResult::Action(Action::Activate { id: self.tabs[next].id });
                }
                Key::Character('a' | 'A') if self.focused => {
                    self.anchor = 0;
                    self.cursor = self.address.len();
                    return KeyResult::Consumed;
                }
                _ => {}
            }
        }
        if self.active != 0 {
            match key {
                Key::F5 => return KeyResult::Action(Action::Reload { id: self.active }),
                Key::Left if modifiers.alt => return KeyResult::Action(Action::Back { id: self.active }),
                Key::Right if modifiers.alt => return KeyResult::Action(Action::Forward { id: self.active }),
                _ => {}
            }
        }
        if !self.focused {
            return KeyResult::Unhandled;
        }
        self.pending_surrogate = None;
        match key {
            Key::Enter => {
                let url = self.address.trim().to_owned();
                self.focused = false;
                return if url.is_empty() || self.active == 0 {
                    KeyResult::Consumed
                } else {
                    KeyResult::Action(Action::Navigate { tab: self.active, url })
                };
            }
            Key::Escape | Key::Tab => self.blur_address(),
            Key::Backspace => {
                if self.cursor == self.anchor {
                    self.anchor = self.previous_boundary();
                }
                self.replace_selection("");
            }
            Key::Delete => {
                if self.cursor == self.anchor {
                    self.anchor = self.next_boundary();
                }
                self.replace_selection("");
            }
            Key::Left => {
                self.cursor = if !modifiers.shift && self.cursor != self.anchor {
                    self.cursor.min(self.anchor)
                } else {
                    self.previous_boundary()
                };
                if !modifiers.shift {
                    self.anchor = self.cursor;
                }
            }
            Key::Right => {
                self.cursor = if !modifiers.shift && self.cursor != self.anchor {
                    self.cursor.max(self.anchor)
                } else {
                    self.next_boundary()
                };
                if !modifiers.shift {
                    self.anchor = self.cursor;
                }
            }
            Key::Home => {
                self.cursor = 0;
                if !modifiers.shift {
                    self.anchor = self.cursor;
                }
            }
            Key::End => {
                self.cursor = self.address.len();
                if !modifiers.shift {
                    self.anchor = self.cursor;
                }
            }
            Key::Character(_) => return KeyResult::Unhandled,
            Key::F5 => {}
        }
        KeyResult::Consumed
    }

    pub fn compose(&self) -> Vec<u8> {
        let mut frame = vec![255; byte_len(self.width, self.height)];
        if let Some(tab) = self.tabs.iter().find(|tab| tab.id == self.active) {
            let offset = byte_len(self.width, CHROME_HEIGHT);
            frame[offset..offset + tab.pixels.len()].copy_from_slice(&tab.pixels);
            if let Some(popup) = &tab.popup {
                let left = i64::from(popup.x).max(0);
                let top = i64::from(popup.y).max(0);
                let right = (i64::from(popup.x) + i64::from(popup.width)).min(i64::from(self.width));
                let bottom = (i64::from(popup.y) + i64::from(popup.height)).min(i64::from(self.content_dimensions().1));
                if left < right && top < bottom {
                    for y in top..bottom {
                        let source = (((y - i64::from(popup.y)) * i64::from(popup.width) + left - i64::from(popup.x))
                            * 4) as usize;
                        let target = (((y + i64::from(CHROME_HEIGHT)) * i64::from(self.width) + left) * 4) as usize;
                        let length = ((right - left) * 4) as usize;
                        frame[target..target + length].copy_from_slice(&popup.pixels[source..source + length]);
                    }
                }
            }
        }
        self.draw_chrome(&mut frame);
        if !self.download_status.is_empty() {
            let top = self.height - 32;
            fill_rect(&mut frame, self.width, 0, top, self.width, 32, [225, 225, 225, 255]);
            self.draw_text(
                &mut frame,
                &self.download_status,
                8,
                top + 8,
                self.width - 100,
                [35, 35, 35, 255],
            );
            fill_rect(
                &mut frame,
                self.width,
                self.width - 96,
                top,
                96,
                32,
                [180, 190, 215, 255],
            );
            self.draw_text(
                &mut frame,
                "Cancel all",
                self.width - 88,
                top + 8,
                self.width,
                [35, 35, 35, 255],
            );
        }
        frame
    }

    fn tab_index(&self, id: i32) -> anyhow::Result<usize> {
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == id) {
            return Ok(index);
        }
        bail!("unknown tab")
    }

    fn sync_address(&mut self) {
        self.address = self
            .tabs
            .iter()
            .find(|tab| tab.id == self.active)
            .map_or_else(String::new, |tab| tab.url.clone());
        self.cursor = self.address.len();
        self.anchor = self.cursor;
    }

    fn blur_address(&mut self) {
        self.focused = false;
        self.pending_surrogate = None;
        self.sync_address();
    }

    fn focus_address(&mut self) {
        self.focused = true;
        self.cursor = self.address.len();
        self.anchor = 0;
        self.pending_surrogate = None;
    }

    fn replace_selection(&mut self, text: &str) {
        let start = self.cursor.min(self.anchor);
        let end = self.cursor.max(self.anchor);
        self.address.replace_range(start..end, text);
        self.cursor = start + text.len();
        self.anchor = self.cursor;
    }

    fn previous_boundary(&self) -> usize {
        self.address[..self.cursor]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next_boundary(&self) -> usize {
        self.address[self.cursor..]
            .chars()
            .next()
            .map_or(self.cursor, |ch| self.cursor + ch.len_utf8())
    }

    fn tab_width(&self) -> usize {
        ((self.width as usize - 32) / self.tabs.len().max(1)).min(180)
    }

    fn draw_chrome(&self, frame: &mut [u8]) {
        fill_rect(frame, self.width, 0, 0, self.width, CHROME_HEIGHT, [224, 222, 220, 255]);
        let width = self.tab_width() as u32;
        for (index, tab) in self.tabs.iter().enumerate() {
            let x = index as u32 * width;
            let color = if tab.id == self.active {
                [250, 249, 248, 255]
            } else {
                [200, 198, 196, 255]
            };
            fill_rect(frame, self.width, x + 1, 2, width - 2, 30, color);
            let title = if tab.title.is_empty() { &tab.url } else { &tab.title };
            self.draw_text(frame, title, x + 6, 8, x + width - 25, [40, 40, 40, 255]);
            self.draw_text(frame, "x", x + width - 19, 8, x + width - 3, [40, 40, 40, 255]);
            if tab.loading {
                fill_rect(frame, self.width, x + 2, 29, width - 4, 3, [220, 130, 30, 255]);
            }
        }
        if self.tabs.len() < MAX_TABS {
            self.draw_text(
                frame,
                "+",
                width * self.tabs.len() as u32 + 10,
                7,
                self.width,
                [40, 40, 40, 255],
            );
        }
        for (label, x) in [("<", 12), (">", 44), ("R", 76)] {
            self.draw_text(frame, label, x, 50, x + 20, [40, 40, 40, 255]);
        }
        let border = if self.focused {
            [210, 110, 30, 255]
        } else {
            [170, 170, 170, 255]
        };
        fill_rect(frame, self.width, 103, 42, self.width - 110, 36, border);
        fill_rect(frame, self.width, 105, 44, self.width - 114, 32, [255; 4]);
        let text_color = if self.tabs.iter().any(|tab| tab.id == self.active && tab.error.is_some()) {
            [45, 45, 180, 255]
        } else {
            [35, 35, 35, 255]
        };
        let end = self.width - 14;
        let start = 112;
        let visible_start = if self.focused {
            self.address[..self.cursor]
                .char_indices()
                .rev()
                .take(((end - start) / 9) as usize)
                .last()
                .map_or(self.cursor, |(index, _)| index)
        } else {
            0
        };
        let text = &self.address[visible_start..];
        if self.focused && self.anchor != self.cursor {
            let selection_start = self.cursor.min(self.anchor).max(visible_start);
            let selection_end = self.cursor.max(self.anchor).max(visible_start);
            let x = start + self.text_width(&self.address[visible_start..selection_start]);
            let length = self.text_width(&self.address[selection_start..selection_end]);
            if x < end {
                fill_rect(frame, self.width, x, 47, length.min(end - x), 25, [245, 220, 190, 255]);
            }
        }
        self.draw_text(frame, text, start, 51, end, text_color);
        if self.focused {
            let x = start + self.text_width(&self.address[visible_start..self.cursor]);
            if x < end {
                fill_rect(frame, self.width, x, 48, 1, 22, [30, 30, 30, 255]);
            }
        }
    }

    fn text_width(&self, text: &str) -> u32 {
        self.font.as_ref().map_or(text.chars().count() as u32 * 8, |font| {
            let scaled = font.as_scaled(PxScale::from(16.0));
            text.chars()
                .map(|ch| scaled.h_advance(font.glyph_id(ch)))
                .sum::<f32>()
                .ceil() as u32
        })
    }

    fn draw_text(&self, frame: &mut [u8], text: &str, x: u32, y: u32, right: u32, color: [u8; 4]) {
        let Some(font) = &self.font else { return };
        let scaled = font.as_scaled(PxScale::from(16.0));
        let mut position = point(x as f32, y as f32 + scaled.ascent());
        for ch in text.chars() {
            let mut glyph = scaled.scaled_glyph(ch);
            let advance = scaled.h_advance(glyph.id);
            if right as f32 <= position.x {
                break;
            }
            glyph.position = position;
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, coverage| {
                    let px = bounds.min.x as i32 + gx as i32;
                    let py = bounds.min.y as i32 + gy as i32;
                    if x as i32 <= px && px < right.min(self.width) as i32 && 0 <= py && py < self.height as i32 {
                        let offset = (py as usize * self.width as usize + px as usize) * 4;
                        for channel in 0..3 {
                            frame[offset + channel] = (f32::from(color[channel]) * coverage
                                + f32::from(frame[offset + channel]) * (1.0 - coverage))
                                .round() as u8;
                        }
                        frame[offset + 3] = 255;
                    }
                });
            }
            position.x += advance;
        }
    }
}

fn validate_dimensions(width: u32, height: u32) -> anyhow::Result<()> {
    ensure!(
        (MIN_WIDTH..=MAX_WIDTH).contains(&width) && (MIN_HEIGHT..=MAX_HEIGHT).contains(&height),
        "desktop dimensions outside supported bounds"
    );
    Ok(())
}

fn byte_len(width: u32, height: u32) -> usize {
    width as usize * height as usize * 4
}

fn fill_rect(frame: &mut [u8], stride: u32, x: u32, y: u32, width: u32, height: u32, color: [u8; 4]) {
    for row in y..y + height {
        let start = (row as usize * stride as usize + x as usize) * 4;
        let end = start + width as usize * 4;
        for pixel in frame[start..end].chunks_exact_mut(4) {
            pixel.copy_from_slice(&color);
        }
    }
}
