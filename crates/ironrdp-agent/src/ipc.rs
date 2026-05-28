//! Binary IPC framing for ironrdp-agent.
//!
//! Wire format: one frame is a `u32` big-endian length followed by an encoded
//! [`Request`] or [`Response`] PDU. Each PDU starts with a `u8` tag identifying
//! the variant, followed by a `u32` request id (for correlation), followed by
//! the variant payload.

use core::time::Duration;

use anyhow::Context as _;
use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const MAX_FRAME_LEN: u32 = 32 * 1024 * 1024;

// --- helpers -----------------------------------------------------------------

fn write_string(dst: &mut WriteCursor<'_>, value: &str) -> EncodeResult<()> {
    ensure_size!(in: dst, size: 4 + value.len());
    dst.write_u32_be(u32::try_from(value.len()).map_err(|_| {
        ironrdp_core::invalid_field_err::<ironrdp_core::EncodeError>("string", "length", "exceeds u32::MAX")
    })?);
    dst.write_slice(value.as_bytes());
    Ok(())
}

fn string_size(value: &str) -> usize {
    4 + value.len()
}

fn read_string(src: &mut ReadCursor<'_>) -> DecodeResult<String> {
    ensure_size!(in: src, size: 4);
    let len = usize::try_from(src.read_u32_be()).map_err(|_| {
        ironrdp_core::invalid_field_err::<ironrdp_core::DecodeError>("string", "length", "exceeds usize")
    })?;
    ensure_size!(in: src, size: len);
    let bytes = src.read_slice(len);
    String::from_utf8(bytes.to_vec())
        .map_err(|_| ironrdp_core::invalid_field_err::<ironrdp_core::DecodeError>("string", "utf8", "invalid utf-8"))
}

fn write_opt_string(dst: &mut WriteCursor<'_>, value: Option<&str>) -> EncodeResult<()> {
    ensure_size!(in: dst, size: 1);
    if let Some(v) = value {
        dst.write_u8(1);
        write_string(dst, v)
    } else {
        dst.write_u8(0);
        Ok(())
    }
}

fn opt_string_size(value: Option<&str>) -> usize {
    1 + value.map_or(0, string_size)
}

fn read_opt_string(src: &mut ReadCursor<'_>) -> DecodeResult<Option<String>> {
    ensure_size!(in: src, size: 1);
    match src.read_u8() {
        0 => Ok(None),
        1 => Ok(Some(read_string(src)?)),
        _ => Err(ironrdp_core::invalid_field_err::<ironrdp_core::DecodeError>(
            "option",
            "tag",
            "invalid tag",
        )),
    }
}

fn write_bytes(dst: &mut WriteCursor<'_>, bytes: &[u8]) -> EncodeResult<()> {
    ensure_size!(in: dst, size: 4 + bytes.len());
    dst.write_u32_be(u32::try_from(bytes.len()).map_err(|_| {
        ironrdp_core::invalid_field_err::<ironrdp_core::EncodeError>("bytes", "length", "exceeds u32::MAX")
    })?);
    dst.write_slice(bytes);
    Ok(())
}

fn bytes_size(bytes: &[u8]) -> usize {
    4 + bytes.len()
}

fn read_bytes(src: &mut ReadCursor<'_>) -> DecodeResult<Vec<u8>> {
    ensure_size!(in: src, size: 4);
    let len = usize::try_from(src.read_u32_be()).map_err(|_| {
        ironrdp_core::invalid_field_err::<ironrdp_core::DecodeError>("bytes", "length", "exceeds usize")
    })?;
    ensure_size!(in: src, size: len);
    Ok(src.read_slice(len).to_vec())
}

// --- mouse / keyboard --------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MouseButton {
    Left = 0,
    Middle = 1,
    Right = 2,
    X1 = 3,
    X2 = 4,
}

impl MouseButton {
    fn to_u8(self) -> u8 {
        match self {
            Self::Left => 0,
            Self::Middle => 1,
            Self::Right => 2,
            Self::X1 => 3,
            Self::X2 => 4,
        }
    }

    fn from_u8(v: u8) -> DecodeResult<Self> {
        Ok(match v {
            0 => Self::Left,
            1 => Self::Middle,
            2 => Self::Right,
            3 => Self::X1,
            4 => Self::X2,
            _other => {
                return Err(ironrdp_core::invalid_field_err::<ironrdp_core::DecodeError>(
                    "MouseButton",
                    "tag",
                    "unknown tag",
                ));
            }
        })
    }
}

impl From<MouseButton> for ironrdp::input::MouseButton {
    fn from(b: MouseButton) -> Self {
        match b {
            MouseButton::Left => Self::Left,
            MouseButton::Middle => Self::Middle,
            MouseButton::Right => Self::Right,
            MouseButton::X1 => Self::X1,
            MouseButton::X2 => Self::X2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Move {
        x: u16,
        y: u16,
    },
    Click {
        button: MouseButton,
        x: Option<u16>,
        y: Option<u16>,
    },
    Down {
        button: MouseButton,
    },
    Up {
        button: MouseButton,
    },
    Wheel {
        units: i16,
        horizontal: bool,
    },
    Position,
}

impl MouseAction {
    fn size(&self) -> usize {
        1 + match self {
            Self::Move { .. } => 4,
            Self::Click { .. } => 1 + 1 + 2 + 1 + 2, // button + opt x + opt y (1+2 each)
            Self::Down { .. } | Self::Up { .. } => 1,
            Self::Wheel { .. } => 2 + 1,
            Self::Position => 0,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: 1);
        match self {
            Self::Move { x, y } => {
                dst.write_u8(0);
                ensure_size!(in: dst, size: 4);
                dst.write_u16_be(*x);
                dst.write_u16_be(*y);
            }
            Self::Click { button, x, y } => {
                dst.write_u8(1);
                ensure_size!(in: dst, size: 1 + 3 + 3);
                dst.write_u8(button.to_u8());
                dst.write_u8(if x.is_some() { 1 } else { 0 });
                dst.write_u16_be(x.unwrap_or(0));
                dst.write_u8(if y.is_some() { 1 } else { 0 });
                dst.write_u16_be(y.unwrap_or(0));
            }
            Self::Down { button } => {
                dst.write_u8(2);
                ensure_size!(in: dst, size: 1);
                dst.write_u8(button.to_u8());
            }
            Self::Up { button } => {
                dst.write_u8(3);
                ensure_size!(in: dst, size: 1);
                dst.write_u8(button.to_u8());
            }
            Self::Wheel { units, horizontal } => {
                dst.write_u8(4);
                ensure_size!(in: dst, size: 3);
                dst.write_i16_be(*units);
                dst.write_u8(u8::from(*horizontal));
            }
            Self::Position => {
                dst.write_u8(5);
            }
        }
        Ok(())
    }

    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        Ok(match src.read_u8() {
            0 => {
                ensure_size!(in: src, size: 4);
                Self::Move {
                    x: src.read_u16_be(),
                    y: src.read_u16_be(),
                }
            }
            1 => {
                ensure_size!(in: src, size: 1 + 3 + 3);
                let button = MouseButton::from_u8(src.read_u8())?;
                let x_present = src.read_u8() != 0;
                let x_val = src.read_u16_be();
                let y_present = src.read_u8() != 0;
                let y_val = src.read_u16_be();
                Self::Click {
                    button,
                    x: x_present.then_some(x_val),
                    y: y_present.then_some(y_val),
                }
            }
            2 => {
                ensure_size!(in: src, size: 1);
                Self::Down {
                    button: MouseButton::from_u8(src.read_u8())?,
                }
            }
            3 => {
                ensure_size!(in: src, size: 1);
                Self::Up {
                    button: MouseButton::from_u8(src.read_u8())?,
                }
            }
            4 => {
                ensure_size!(in: src, size: 3);
                Self::Wheel {
                    units: src.read_i16_be(),
                    horizontal: src.read_u8() != 0,
                }
            }
            5 => Self::Position,
            _other => {
                return Err(ironrdp_core::invalid_field_err::<ironrdp_core::DecodeError>(
                    "MouseAction",
                    "tag",
                    "unknown tag",
                ));
            }
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyboardAction {
    Key { scancode: u16, release: bool },
    Text { text: String },
    Shortcut { scancodes: Vec<u16> },
    ReleaseAll,
}

impl KeyboardAction {
    fn size(&self) -> usize {
        1 + match self {
            Self::Key { .. } => 2 + 1,
            Self::Text { text } => string_size(text),
            Self::Shortcut { scancodes } => 4 + scancodes.len() * 2,
            Self::ReleaseAll => 0,
        }
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: 1);
        match self {
            Self::Key { scancode, release } => {
                dst.write_u8(0);
                ensure_size!(in: dst, size: 3);
                dst.write_u16_be(*scancode);
                dst.write_u8(u8::from(*release));
            }
            Self::Text { text } => {
                dst.write_u8(1);
                write_string(dst, text)?;
            }
            Self::Shortcut { scancodes } => {
                dst.write_u8(2);
                ensure_size!(in: dst, size: 4 + scancodes.len() * 2);
                dst.write_u32_be(u32::try_from(scancodes.len()).map_err(|_| {
                    ironrdp_core::invalid_field_err::<ironrdp_core::EncodeError>(
                        "Shortcut",
                        "length",
                        "exceeds u32::MAX",
                    )
                })?);
                for s in scancodes {
                    dst.write_u16_be(*s);
                }
            }
            Self::ReleaseAll => {
                dst.write_u8(3);
            }
        }
        Ok(())
    }

    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        Ok(match src.read_u8() {
            0 => {
                ensure_size!(in: src, size: 3);
                Self::Key {
                    scancode: src.read_u16_be(),
                    release: src.read_u8() != 0,
                }
            }
            1 => Self::Text {
                text: read_string(src)?,
            },
            2 => {
                ensure_size!(in: src, size: 4);
                let n = usize::try_from(src.read_u32_be()).map_err(|_| {
                    ironrdp_core::invalid_field_err::<ironrdp_core::DecodeError>("Shortcut", "length", "exceeds usize")
                })?;
                ensure_size!(in: src, size: n * 2);
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(src.read_u16_be());
                }
                Self::Shortcut { scancodes: v }
            }
            3 => Self::ReleaseAll,
            _other => {
                return Err(ironrdp_core::invalid_field_err::<ironrdp_core::DecodeError>(
                    "KeyboardAction",
                    "tag",
                    "unknown tag",
                ));
            }
        })
    }
}

// --- Request -----------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum Request {
    Health,
    Connect {
        rdp_content: String,
        label: Option<String>,
    },
    Sessions,
    Status {
        session_id: Option<String>,
    },
    Disconnect {
        session_id: String,
    },
    Mouse {
        session_id: String,
        action: MouseAction,
    },
    Keyboard {
        session_id: String,
        action: KeyboardAction,
    },
    Resize {
        session_id: String,
        width: u16,
        height: u16,
        scale: u32,
    },
    WaitFrame {
        session_id: String,
        timeout_ms: u64,
        after_frame: Option<u64>,
    },
    Screenshot {
        session_id: String,
    },
    MousePosition {
        session_id: String,
    },
    DumpProperties {
        session_id: String,
    },
    SetProperty {
        session_id: String,
        key: String,
        value: String,
    },
}

impl Request {
    fn tag(&self) -> u8 {
        match self {
            Self::Health => 0,
            Self::Connect { .. } => 1,
            Self::Sessions => 2,
            Self::Status { .. } => 3,
            Self::Disconnect { .. } => 4,
            Self::Mouse { .. } => 5,
            Self::Keyboard { .. } => 6,
            Self::Resize { .. } => 7,
            Self::WaitFrame { .. } => 8,
            Self::Screenshot { .. } => 9,
            Self::MousePosition { .. } => 10,
            Self::DumpProperties { .. } => 11,
            Self::SetProperty { .. } => 12,
        }
    }
}

impl Encode for Request {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: 1);
        dst.write_u8(self.tag());
        match self {
            Self::Health | Self::Sessions => Ok(()),
            Self::Connect { rdp_content, label } => {
                write_string(dst, rdp_content)?;
                write_opt_string(dst, label.as_deref())
            }
            Self::Status { session_id } => write_opt_string(dst, session_id.as_deref()),
            Self::Disconnect { session_id }
            | Self::Screenshot { session_id }
            | Self::MousePosition { session_id }
            | Self::DumpProperties { session_id } => write_string(dst, session_id),
            Self::Mouse { session_id, action } => {
                write_string(dst, session_id)?;
                action.encode(dst)
            }
            Self::Keyboard { session_id, action } => {
                write_string(dst, session_id)?;
                action.encode(dst)
            }
            Self::Resize {
                session_id,
                width,
                height,
                scale,
            } => {
                write_string(dst, session_id)?;
                ensure_size!(in: dst, size: 8);
                dst.write_u16_be(*width);
                dst.write_u16_be(*height);
                dst.write_u32_be(*scale);
                Ok(())
            }
            Self::WaitFrame {
                session_id,
                timeout_ms,
                after_frame,
            } => {
                write_string(dst, session_id)?;
                ensure_size!(in: dst, size: 8 + 1);
                dst.write_u64_be(*timeout_ms);
                if let Some(af) = after_frame {
                    dst.write_u8(1);
                    ensure_size!(in: dst, size: 8);
                    dst.write_u64_be(*af);
                } else {
                    dst.write_u8(0);
                }
                Ok(())
            }
            Self::SetProperty { session_id, key, value } => {
                write_string(dst, session_id)?;
                write_string(dst, key)?;
                write_string(dst, value)
            }
        }
    }

    fn name(&self) -> &'static str {
        "AgentRequest"
    }

    fn size(&self) -> usize {
        1 + match self {
            Self::Health | Self::Sessions => 0,
            Self::Connect { rdp_content, label } => string_size(rdp_content) + opt_string_size(label.as_deref()),
            Self::Status { session_id } => opt_string_size(session_id.as_deref()),
            Self::Disconnect { session_id }
            | Self::Screenshot { session_id }
            | Self::MousePosition { session_id }
            | Self::DumpProperties { session_id } => string_size(session_id),
            Self::Mouse { session_id, action } => string_size(session_id) + action.size(),
            Self::Keyboard { session_id, action } => string_size(session_id) + action.size(),
            Self::Resize { session_id, .. } => string_size(session_id) + 8,
            Self::WaitFrame {
                session_id,
                after_frame,
                ..
            } => string_size(session_id) + 8 + 1 + if after_frame.is_some() { 8 } else { 0 },
            Self::SetProperty { session_id, key, value } => {
                string_size(session_id) + string_size(key) + string_size(value)
            }
        }
    }
}

impl<'de> Decode<'de> for Request {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        Ok(match src.read_u8() {
            0 => Self::Health,
            1 => Self::Connect {
                rdp_content: read_string(src)?,
                label: read_opt_string(src)?,
            },
            2 => Self::Sessions,
            3 => Self::Status {
                session_id: read_opt_string(src)?,
            },
            4 => Self::Disconnect {
                session_id: read_string(src)?,
            },
            5 => Self::Mouse {
                session_id: read_string(src)?,
                action: MouseAction::decode(src)?,
            },
            6 => Self::Keyboard {
                session_id: read_string(src)?,
                action: KeyboardAction::decode(src)?,
            },
            7 => {
                let session_id = read_string(src)?;
                ensure_size!(in: src, size: 8);
                Self::Resize {
                    session_id,
                    width: src.read_u16_be(),
                    height: src.read_u16_be(),
                    scale: src.read_u32_be(),
                }
            }
            8 => {
                let session_id = read_string(src)?;
                ensure_size!(in: src, size: 9);
                let timeout_ms = src.read_u64_be();
                let after_frame = if src.read_u8() != 0 {
                    ensure_size!(in: src, size: 8);
                    Some(src.read_u64_be())
                } else {
                    None
                };
                Self::WaitFrame {
                    session_id,
                    timeout_ms,
                    after_frame,
                }
            }
            9 => Self::Screenshot {
                session_id: read_string(src)?,
            },
            10 => Self::MousePosition {
                session_id: read_string(src)?,
            },
            11 => Self::DumpProperties {
                session_id: read_string(src)?,
            },
            12 => Self::SetProperty {
                session_id: read_string(src)?,
                key: read_string(src)?,
                value: read_string(src)?,
            },
            _other => {
                return Err(ironrdp_core::invalid_field_err::<ironrdp_core::DecodeError>(
                    "Request",
                    "tag",
                    "unknown tag",
                ));
            }
        })
    }
}

// --- session summary ---------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SessionStatus {
    Connecting = 0,
    Connected = 1,
    Failed = 2,
    Disconnected = 3,
}

impl SessionStatus {
    fn to_u8(self) -> u8 {
        match self {
            Self::Connecting => 0,
            Self::Connected => 1,
            Self::Failed => 2,
            Self::Disconnected => 3,
        }
    }

    fn from_u8(v: u8) -> DecodeResult<Self> {
        Ok(match v {
            0 => Self::Connecting,
            1 => Self::Connected,
            2 => Self::Failed,
            3 => Self::Disconnected,
            _other => {
                return Err(ironrdp_core::invalid_field_err::<ironrdp_core::DecodeError>(
                    "SessionStatus",
                    "tag",
                    "unknown tag",
                ));
            }
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Failed => "failed",
            Self::Disconnected => "disconnected",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub session_id: String,
    pub label: Option<String>,
    pub status: SessionStatus,
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub frame_sequence: u64,
    pub mouse_x: u16,
    pub mouse_y: u16,
    pub last_error: Option<String>,
}

impl SessionSummary {
    fn size(&self) -> usize {
        string_size(&self.session_id)
            + opt_string_size(self.label.as_deref())
            + 1
            + 1 + 2 // width opt
            + 1 + 2
            + 8 + 2 + 2
            + opt_string_size(self.last_error.as_deref())
    }

    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        write_string(dst, &self.session_id)?;
        write_opt_string(dst, self.label.as_deref())?;
        ensure_size!(in: dst, size: 1 + 3 + 3 + 8 + 4);
        dst.write_u8(self.status.to_u8());
        dst.write_u8(if self.width.is_some() { 1 } else { 0 });
        dst.write_u16_be(self.width.unwrap_or(0));
        dst.write_u8(if self.height.is_some() { 1 } else { 0 });
        dst.write_u16_be(self.height.unwrap_or(0));
        dst.write_u64_be(self.frame_sequence);
        dst.write_u16_be(self.mouse_x);
        dst.write_u16_be(self.mouse_y);
        write_opt_string(dst, self.last_error.as_deref())
    }

    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let session_id = read_string(src)?;
        let label = read_opt_string(src)?;
        ensure_size!(in: src, size: 1 + 3 + 3 + 8 + 4);
        let status = SessionStatus::from_u8(src.read_u8())?;
        let w_present = src.read_u8() != 0;
        let w = src.read_u16_be();
        let h_present = src.read_u8() != 0;
        let h = src.read_u16_be();
        let frame_sequence = src.read_u64_be();
        let mouse_x = src.read_u16_be();
        let mouse_y = src.read_u16_be();
        let last_error = read_opt_string(src)?;
        Ok(Self {
            session_id,
            label,
            status,
            width: w_present.then_some(w),
            height: h_present.then_some(h),
            frame_sequence,
            mouse_x,
            mouse_y,
            last_error,
        })
    }
}

#[derive(Clone, Debug)]
pub struct PropertyEntry {
    pub key: String,
    pub value: String,
    pub description: String,
}

// --- Response ----------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum Response {
    Ok,
    Error { message: String },
    Health,
    Connect { session_id: String },
    Sessions { sessions: Vec<SessionSummary> },
    Status { summary: SessionSummary },
    MousePosition { x: u16, y: u16 },
    Screenshot { png: Vec<u8> },
    Properties { entries: Vec<PropertyEntry> },
}

impl Response {
    fn tag(&self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::Error { .. } => 1,
            Self::Health => 2,
            Self::Connect { .. } => 3,
            Self::Sessions { .. } => 4,
            Self::Status { .. } => 5,
            Self::MousePosition { .. } => 6,
            Self::Screenshot { .. } => 7,
            Self::Properties { .. } => 8,
        }
    }
}

impl Encode for Response {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: 1);
        dst.write_u8(self.tag());
        match self {
            Self::Ok | Self::Health => Ok(()),
            Self::Error { message } => write_string(dst, message),
            Self::Connect { session_id } => write_string(dst, session_id),
            Self::Sessions { sessions } => {
                ensure_size!(in: dst, size: 4);
                dst.write_u32_be(u32::try_from(sessions.len()).map_err(|_| {
                    ironrdp_core::invalid_field_err::<ironrdp_core::EncodeError>("Sessions", "len", "exceeds u32::MAX")
                })?);
                for s in sessions {
                    s.encode(dst)?;
                }
                Ok(())
            }
            Self::Status { summary } => summary.encode(dst),
            Self::MousePosition { x, y } => {
                ensure_size!(in: dst, size: 4);
                dst.write_u16_be(*x);
                dst.write_u16_be(*y);
                Ok(())
            }
            Self::Screenshot { png } => write_bytes(dst, png),
            Self::Properties { entries } => {
                ensure_size!(in: dst, size: 4);
                dst.write_u32_be(u32::try_from(entries.len()).map_err(|_| {
                    ironrdp_core::invalid_field_err::<ironrdp_core::EncodeError>(
                        "Properties",
                        "len",
                        "exceeds u32::MAX",
                    )
                })?);
                for e in entries {
                    write_string(dst, &e.key)?;
                    write_string(dst, &e.value)?;
                    write_string(dst, &e.description)?;
                }
                Ok(())
            }
        }
    }

    fn name(&self) -> &'static str {
        "AgentResponse"
    }

    fn size(&self) -> usize {
        1 + match self {
            Self::Ok | Self::Health => 0,
            Self::Error { message } => string_size(message),
            Self::Connect { session_id } => string_size(session_id),
            Self::Sessions { sessions } => 4 + sessions.iter().map(SessionSummary::size).sum::<usize>(),
            Self::Status { summary } => summary.size(),
            Self::MousePosition { .. } => 4,
            Self::Screenshot { png } => bytes_size(png),
            Self::Properties { entries } => {
                4 + entries
                    .iter()
                    .map(|e| string_size(&e.key) + string_size(&e.value) + string_size(&e.description))
                    .sum::<usize>()
            }
        }
    }
}

impl<'de> Decode<'de> for Response {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        Ok(match src.read_u8() {
            0 => Self::Ok,
            1 => Self::Error {
                message: read_string(src)?,
            },
            2 => Self::Health,
            3 => Self::Connect {
                session_id: read_string(src)?,
            },
            4 => {
                ensure_size!(in: src, size: 4);
                let n = usize::try_from(src.read_u32_be()).map_err(|_| {
                    ironrdp_core::invalid_field_err::<ironrdp_core::DecodeError>("Sessions", "len", "exceeds usize")
                })?;
                let mut sessions = Vec::with_capacity(n);
                for _ in 0..n {
                    sessions.push(SessionSummary::decode(src)?);
                }
                Self::Sessions { sessions }
            }
            5 => Self::Status {
                summary: SessionSummary::decode(src)?,
            },
            6 => {
                ensure_size!(in: src, size: 4);
                Self::MousePosition {
                    x: src.read_u16_be(),
                    y: src.read_u16_be(),
                }
            }
            7 => Self::Screenshot { png: read_bytes(src)? },
            8 => {
                ensure_size!(in: src, size: 4);
                let n = usize::try_from(src.read_u32_be()).map_err(|_| {
                    ironrdp_core::invalid_field_err::<ironrdp_core::DecodeError>("Properties", "len", "exceeds usize")
                })?;
                let mut entries = Vec::with_capacity(n);
                for _ in 0..n {
                    entries.push(PropertyEntry {
                        key: read_string(src)?,
                        value: read_string(src)?,
                        description: read_string(src)?,
                    });
                }
                Self::Properties { entries }
            }
            _other => {
                return Err(ironrdp_core::invalid_field_err::<ironrdp_core::DecodeError>(
                    "Response",
                    "tag",
                    "unknown tag",
                ));
            }
        })
    }
}

// --- framing -----------------------------------------------------------------

pub async fn write_frame<W, P>(writer: &mut W, pdu: &P) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
    P: Encode,
{
    let body = ironrdp_core::encode_vec(pdu).context("encode PDU")?;
    let len = u32::try_from(body.len()).context("frame too large")?;
    writer
        .write_all(&len.to_be_bytes())
        .await
        .context("write frame length")?;
    writer.write_all(&body).await.context("write frame body")?;
    writer.flush().await.context("flush frame")?;
    Ok(())
}

pub async fn read_frame<R, P>(reader: &mut R) -> anyhow::Result<P>
where
    R: tokio::io::AsyncRead + Unpin,
    for<'de> P: Decode<'de>,
{
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await.context("read frame length")?;
    let len = u32::from_be_bytes(len_buf);
    anyhow::ensure!(len <= MAX_FRAME_LEN, "frame too large: {len}");
    let mut body = vec![0u8; usize::try_from(len).context("frame length exceeds usize")?];
    reader.read_exact(&mut body).await.context("read frame body")?;
    let mut cursor = ReadCursor::new(&body);
    let pdu = P::decode(&mut cursor).context("decode PDU")?;
    Ok(pdu)
}

pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip_request(req: Request) {
        let bytes = ironrdp_core::encode_vec(&req).expect("encode");
        let mut cursor = ReadCursor::new(&bytes);
        let decoded = Request::decode(&mut cursor).expect("decode");
        let bytes2 = ironrdp_core::encode_vec(&decoded).expect("encode2");
        assert_eq!(bytes, bytes2, "round-trip mismatch for {req:?}");
    }

    fn round_trip_response(resp: Response) {
        let bytes = ironrdp_core::encode_vec(&resp).expect("encode");
        let mut cursor = ReadCursor::new(&bytes);
        let decoded = Response::decode(&mut cursor).expect("decode");
        let bytes2 = ironrdp_core::encode_vec(&decoded).expect("encode2");
        assert_eq!(bytes, bytes2);
    }

    #[test]
    fn request_round_trips() {
        round_trip_request(Request::Health);
        round_trip_request(Request::Sessions);
        round_trip_request(Request::Connect {
            rdp_content: "full address:s:host\nusername:s:bob".to_owned(),
            label: Some("primary".to_owned()),
        });
        round_trip_request(Request::Status { session_id: None });
        round_trip_request(Request::Status {
            session_id: Some("abc".to_owned()),
        });
        round_trip_request(Request::Disconnect {
            session_id: "abc".to_owned(),
        });
        round_trip_request(Request::Mouse {
            session_id: "s".to_owned(),
            action: MouseAction::Move { x: 10, y: 20 },
        });
        round_trip_request(Request::Mouse {
            session_id: "s".to_owned(),
            action: MouseAction::Click {
                button: MouseButton::Left,
                x: Some(1),
                y: None,
            },
        });
        round_trip_request(Request::Keyboard {
            session_id: "s".to_owned(),
            action: KeyboardAction::Text {
                text: "hello".to_owned(),
            },
        });
        round_trip_request(Request::Keyboard {
            session_id: "s".to_owned(),
            action: KeyboardAction::Shortcut {
                scancodes: vec![1, 2, 3],
            },
        });
        round_trip_request(Request::Keyboard {
            session_id: "s".to_owned(),
            action: KeyboardAction::ReleaseAll,
        });
        round_trip_request(Request::Resize {
            session_id: "s".to_owned(),
            width: 1920,
            height: 1080,
            scale: 100,
        });
        round_trip_request(Request::WaitFrame {
            session_id: "s".to_owned(),
            timeout_ms: 5000,
            after_frame: Some(10),
        });
        round_trip_request(Request::WaitFrame {
            session_id: "s".to_owned(),
            timeout_ms: 1000,
            after_frame: None,
        });
        round_trip_request(Request::Screenshot {
            session_id: "s".to_owned(),
        });
        round_trip_request(Request::MousePosition {
            session_id: "s".to_owned(),
        });
        round_trip_request(Request::DumpProperties {
            session_id: "s".to_owned(),
        });
        round_trip_request(Request::SetProperty {
            session_id: "s".to_owned(),
            key: "desktopwidth".to_owned(),
            value: "1024".to_owned(),
        });
    }

    #[test]
    fn response_round_trips() {
        round_trip_response(Response::Ok);
        round_trip_response(Response::Health);
        round_trip_response(Response::Error {
            message: "boom".to_owned(),
        });
        round_trip_response(Response::Connect {
            session_id: "abc".to_owned(),
        });
        round_trip_response(Response::Sessions {
            sessions: vec![SessionSummary {
                session_id: "abc".to_owned(),
                label: Some("primary".to_owned()),
                status: SessionStatus::Connected,
                width: Some(1024),
                height: Some(768),
                frame_sequence: 42,
                mouse_x: 100,
                mouse_y: 200,
                last_error: None,
            }],
        });
        round_trip_response(Response::MousePosition { x: 1, y: 2 });
        round_trip_response(Response::Screenshot { png: vec![1, 2, 3, 4] });
        round_trip_response(Response::Properties {
            entries: vec![PropertyEntry {
                key: "k".to_owned(),
                value: "v".to_owned(),
                description: "d".to_owned(),
            }],
        });
    }
}
