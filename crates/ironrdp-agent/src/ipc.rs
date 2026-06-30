//! Strictly-typed IPC schema (V1) and its binary codec.
//!
//! # Framing
//!
//! Every message is sent length-delimited: a little-endian `u32` byte-count prefix followed by the
//! `Encode`d body. The framing is identical over Unix domain sockets and Windows named pipes (see
//! [`crate::transport`]). Both ends are the same binary at the same version, so there is no version
//! byte and no forward/backward-compatibility handling.
//!
//! # Schema
//!
//! Connection configuration travels as a binary-encoded [`PropertySet`] inside [`Request::Connect`];
//! everything else is a strictly-typed message. See [`Request`]/[`Response`].

use core::fmt;

use ironrdp_core::{Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_size};
use ironrdp_input::MouseButton;
use ironrdp_pdu::impl_pdu_pod;
use ironrdp_propertyset::PropertySet;

use crate::wire::propertyset;
use crate::wire::{
    opt_string_size, opt_u16_size, read_bool, read_char, read_mouse_button, read_opt_string, read_opt_u16, read_string,
    string_size, write_bool, write_char, write_mouse_button, write_opt_string, write_opt_u16, write_string,
};

/// A request sent by the CLI to the daemon.
///
/// `Connect` carries a binary-encoded [`PropertySet`] — never `argv` or CLI strings. Runtime
/// operations are strictly-typed.
#[derive(Clone, PartialEq, Eq)]
pub enum Request {
    /// Start an RDP session from a fully-merged property bag.
    ///
    /// `log_directive`, when set, is a [`tracing`]-style filter directive applied to *this*
    /// session's log capture (e.g. `ironrdp_connector=trace`), layered on top of the default
    /// `DEBUG` level. It lets a caller raise verbosity up-front to troubleshoot a connection.
    Connect {
        properties: PropertySet,
        log_directive: Option<String>,
    },
    /// Tear down the current RDP session (the daemon keeps running).
    Disconnect,
    /// Query the current session status.
    Status,
    /// Query the live session property bag, optionally filtered.
    QueryProps { filter: Option<KeyFilter> },
    /// Return retained log lines, optionally filtered by substring and/or limited to the last `n`.
    QueryLogs {
        substring: Option<String>,
        last: Option<u32>,
    },
    /// Return the dimensions of the most recent frame (minimal for V1).
    Screenshot,
    /// Move the mouse pointer to an absolute position.
    MouseMove { x: u16, y: u16 },
    /// Press or release a mouse button.
    MouseButton { button: MouseButton, pressed: bool },
    /// Rotate the mouse wheel.
    Wheel { delta: i16, horizontal: bool },
    // TODO: questioning whether we need a way to send multiple keys at once, e.g. a small mini
    // format to express in a single command that keys A and B are pressed while key C is released.
    // This could save LLM tokens by collapsing several round-trips into one request.
    /// Press or release a key identified by its RDP scancode.
    KeyScancode { scancode: u16, pressed: bool },
    /// Press or release a key identified by a Unicode character.
    KeyUnicode { ch: char, pressed: bool },
    // TODO: add clipboard support (CLIPRDR), e.g. requests to read the remote clipboard text and to
    // set it, so an LLM can copy/paste to and from the session.
}

// Manual `Debug` so the `Connect` payload's property *values* (which may include a password before
// it reaches `ConfigBuilder::build`) are never printed verbatim; only the keys are shown.
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect {
                properties,
                log_directive,
            } => f
                .debug_struct("Connect")
                .field("properties", &PropertyKeys(properties))
                .field("log_directive", log_directive)
                .finish(),
            Self::Disconnect => f.write_str("Disconnect"),
            Self::Status => f.write_str("Status"),
            Self::QueryProps { filter } => f.debug_struct("QueryProps").field("filter", filter).finish(),
            Self::QueryLogs { substring, last } => f
                .debug_struct("QueryLogs")
                .field("substring", substring)
                .field("last", last)
                .finish(),
            Self::Screenshot => f.write_str("Screenshot"),
            Self::MouseMove { x, y } => f.debug_struct("MouseMove").field("x", x).field("y", y).finish(),
            Self::MouseButton { button, pressed } => f
                .debug_struct("MouseButton")
                .field("button", button)
                .field("pressed", pressed)
                .finish(),
            Self::Wheel { delta, horizontal } => f
                .debug_struct("Wheel")
                .field("delta", delta)
                .field("horizontal", horizontal)
                .finish(),
            Self::KeyScancode { scancode, pressed } => f
                .debug_struct("KeyScancode")
                .field("scancode", scancode)
                .field("pressed", pressed)
                .finish(),
            Self::KeyUnicode { ch, pressed } => f
                .debug_struct("KeyUnicode")
                .field("ch", ch)
                .field("pressed", pressed)
                .finish(),
        }
    }
}

/// A [`PropertySet`] whose `Debug` output lists only the keys, never the (possibly secret) values.
struct PropertyKeys<'a>(&'a PropertySet);

impl fmt::Debug for PropertyKeys<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.0.iter().map(|(key, _)| key)).finish()
    }
}

/// The daemon's reply to a [`Request`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Success, carrying an operation-specific [`Payload`].
    Ok(Payload),
    /// Failure. The message is lowercase with no trailing punctuation.
    Err(String),
}

impl Response {
    /// A successful response with no payload.
    pub fn ok() -> Self {
        Self::Ok(Payload::Empty)
    }

    /// A failure response.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Err(message.into())
    }

    /// Whether this is a success response.
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_))
    }
}

/// The success payload carried by [`Response::Ok`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// No data.
    Empty,
    /// Current session status.
    Status(StatusInfo),
    /// A dump of the live property bag.
    Properties(PropertyDump),
    /// Retained log lines.
    Logs(Vec<String>),
    /// Most recent frame dimensions.
    Screenshot { width: u16, height: u16 },
}

/// Coarse connection state reported by [`Request::Status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    /// No session has been started.
    NoSession,
    /// A session was started and is connecting.
    Connecting,
    /// A session is active (at least one frame received).
    Connected,
    /// A session terminated gracefully.
    Disconnected,
    /// A session failed.
    Failed,
}

impl ConnState {
    fn tag(self) -> u8 {
        match self {
            Self::NoSession => 0,
            Self::Connecting => 1,
            Self::Connected => 2,
            Self::Disconnected => 3,
            Self::Failed => 4,
        }
    }

    fn from_tag(tag: u8) -> DecodeResult<Self> {
        match tag {
            0 => Ok(Self::NoSession),
            1 => Ok(Self::Connecting),
            2 => Ok(Self::Connected),
            3 => Ok(Self::Disconnected),
            4 => Ok(Self::Failed),
            _ => Err(ironrdp_core::invalid_field_err!("connection state", "unknown tag")),
        }
    }
}

/// Status snapshot returned by [`Request::Status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusInfo {
    /// Coarse connection state.
    pub state: ConnState,
    /// RDP target (`host:port`), if a session exists.
    pub destination: Option<String>,
    /// Most recent frame width, if any.
    pub width: Option<u16>,
    /// Most recent frame height, if any.
    pub height: Option<u16>,
    /// Human-readable detail, e.g. the failure reason.
    pub message: Option<String>,
    /// `true` when the daemon was started with preloaded credentials (an operator-provided overlay).
    ///
    /// When set, a caller driving `connect` does not need to supply a password (or other secrets):
    /// the daemon layers the overlay on top of the request before building the configuration.
    pub credentials_loaded: bool,
}

/// A bulk dump of live properties.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyDump {
    /// One entry per property, in key order.
    pub entries: Vec<PropertyEntry>,
}

/// A single dumped property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyEntry {
    /// Property key.
    pub key: String,
    /// Property value.
    pub value: PropValue,
}

/// A dumped property value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropValue {
    /// Integer value.
    Int(i64),
    /// String value.
    Str(String),
}

/// A small key filter for [`Request::QueryProps`]. Matching is case-insensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyFilter {
    /// Match keys containing this substring.
    Substring(String),
    /// Match keys starting with this prefix.
    Prefix(String),
}

impl KeyFilter {
    /// Returns `true` when `key` matches this filter (case-insensitive).
    pub fn matches(&self, key: &str) -> bool {
        let key = key.to_ascii_lowercase();
        match self {
            Self::Substring(needle) => key.contains(&needle.to_ascii_lowercase()),
            Self::Prefix(prefix) => key.starts_with(&prefix.to_ascii_lowercase()),
        }
    }
}

// ── KeyFilter codec ─────────────────────────────────────────────────────────

impl Encode for KeyFilter {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Substring(value) => {
                dst.write_u8(0);
                write_string(dst, value)
            }
            Self::Prefix(value) => {
                dst.write_u8(1);
                write_string(dst, value)
            }
        }
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::KeyFilter"
    }

    fn size(&self) -> usize {
        let value = match self {
            Self::Substring(value) | Self::Prefix(value) => value,
        };
        1 /* tag */ + string_size(value)
    }
}

impl Decode<'_> for KeyFilter {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => Ok(Self::Substring(read_string(src)?)),
            1 => Ok(Self::Prefix(read_string(src)?)),
            _ => Err(ironrdp_core::invalid_field_err!("key filter", "unknown tag")),
        }
    }
}

impl_pdu_pod!(KeyFilter);

// ── PropValue / PropertyEntry / PropertyDump codec ──────────────────────────

impl Encode for PropValue {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Int(value) => {
                dst.write_u8(0);
                dst.write_i64(*value);
            }
            Self::Str(value) => {
                dst.write_u8(1);
                write_string(dst, value)?;
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::PropValue"
    }

    fn size(&self) -> usize {
        1 /* tag */
            + match self {
                Self::Int(_) => 8,
                Self::Str(value) => string_size(value),
            }
    }
}

impl Decode<'_> for PropValue {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => {
                ensure_size!(in: src, size: 8);
                Ok(Self::Int(src.read_i64()))
            }
            1 => Ok(Self::Str(read_string(src)?)),
            _ => Err(ironrdp_core::invalid_field_err!("property value", "unknown tag")),
        }
    }
}

impl_pdu_pod!(PropValue);

impl Encode for PropertyEntry {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        write_string(dst, &self.key)?;
        self.value.encode(dst)
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::PropertyEntry"
    }

    fn size(&self) -> usize {
        string_size(&self.key) + self.value.size()
    }
}

impl Decode<'_> for PropertyEntry {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let key = read_string(src)?;
        let value = PropValue::decode(src)?;
        Ok(Self { key, value })
    }
}

impl_pdu_pod!(PropertyEntry);

impl Encode for PropertyDump {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        let count: u32 = cast_length!("property count", self.entries.len())?;
        dst.write_u32(count);
        for entry in &self.entries {
            entry.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::PropertyDump"
    }

    fn size(&self) -> usize {
        4 /* count */ + self.entries.iter().map(Encode::size).sum::<usize>()
    }
}

impl Decode<'_> for PropertyDump {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4);
        let count = src.read_u32();
        let mut entries = Vec::new();
        for _ in 0..count {
            entries.push(PropertyEntry::decode(src)?);
        }
        Ok(Self { entries })
    }
}

impl_pdu_pod!(PropertyDump);

// ── StatusInfo codec ────────────────────────────────────────────────────────

impl Encode for StatusInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u8(self.state.tag());
        write_opt_string(dst, self.destination.as_deref())?;
        write_opt_u16(dst, self.width)?;
        write_opt_u16(dst, self.height)?;
        write_opt_string(dst, self.message.as_deref())?;
        write_bool(dst, self.credentials_loaded)
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::StatusInfo"
    }

    fn size(&self) -> usize {
        1 /* state */
            + opt_string_size(self.destination.as_deref())
            + opt_u16_size(self.width)
            + opt_u16_size(self.height)
            + opt_string_size(self.message.as_deref())
            + 1 /* credentials_loaded */
    }
}

impl Decode<'_> for StatusInfo {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        let state = ConnState::from_tag(src.read_u8())?;
        let destination = read_opt_string(src)?;
        let width = read_opt_u16(src)?;
        let height = read_opt_u16(src)?;
        let message = read_opt_string(src)?;
        let credentials_loaded = read_bool(src)?;
        Ok(Self {
            state,
            destination,
            width,
            height,
            message,
            credentials_loaded,
        })
    }
}

impl_pdu_pod!(StatusInfo);

// ── Payload codec ───────────────────────────────────────────────────────────

impl Encode for Payload {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Empty => dst.write_u8(0),
            Self::Status(status) => {
                dst.write_u8(1);
                status.encode(dst)?;
            }
            Self::Properties(dump) => {
                dst.write_u8(2);
                dump.encode(dst)?;
            }
            Self::Logs(lines) => {
                dst.write_u8(3);
                let count: u32 = cast_length!("log line count", lines.len())?;
                dst.write_u32(count);
                for line in lines {
                    write_string(dst, line)?;
                }
            }
            Self::Screenshot { width, height } => {
                dst.write_u8(4);
                dst.write_u16(*width);
                dst.write_u16(*height);
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::Payload"
    }

    fn size(&self) -> usize {
        1 /* tag */
            + match self {
                Self::Empty => 0,
                Self::Status(status) => status.size(),
                Self::Properties(dump) => dump.size(),
                Self::Logs(lines) => 4 + lines.iter().map(|line| string_size(line)).sum::<usize>(),
                Self::Screenshot { .. } => 2 /* width */ + 2 /* height */,
            }
    }
}

impl Decode<'_> for Payload {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => Ok(Self::Empty),
            1 => Ok(Self::Status(StatusInfo::decode(src)?)),
            2 => Ok(Self::Properties(PropertyDump::decode(src)?)),
            3 => {
                ensure_size!(in: src, size: 4);
                let count = src.read_u32();
                let mut lines = Vec::new();
                for _ in 0..count {
                    lines.push(read_string(src)?);
                }
                Ok(Self::Logs(lines))
            }
            4 => {
                ensure_size!(in: src, size: 4);
                let width = src.read_u16();
                let height = src.read_u16();
                Ok(Self::Screenshot { width, height })
            }
            _ => Err(ironrdp_core::invalid_field_err!("payload", "unknown tag")),
        }
    }
}

impl_pdu_pod!(Payload);

// ── Response codec ──────────────────────────────────────────────────────────

impl Encode for Response {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Ok(payload) => {
                dst.write_u8(0);
                payload.encode(dst)
            }
            Self::Err(message) => {
                dst.write_u8(1);
                write_string(dst, message)
            }
        }
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::Response"
    }

    fn size(&self) -> usize {
        1 /* tag */
            + match self {
                Self::Ok(payload) => payload.size(),
                Self::Err(message) => string_size(message),
            }
    }
}

impl Decode<'_> for Response {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => Ok(Self::Ok(Payload::decode(src)?)),
            1 => Ok(Self::Err(read_string(src)?)),
            _ => Err(ironrdp_core::invalid_field_err!("response", "unknown tag")),
        }
    }
}

impl_pdu_pod!(Response);

// ── Request codec ───────────────────────────────────────────────────────────

impl Encode for Request {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Connect {
                properties,
                log_directive,
            } => {
                dst.write_u8(0);
                propertyset::write(properties, dst)?;
                write_opt_string(dst, log_directive.as_deref())?;
            }
            Self::Disconnect => dst.write_u8(1),
            Self::Status => dst.write_u8(2),
            Self::QueryProps { filter } => {
                dst.write_u8(3);
                match filter {
                    Some(filter) => {
                        dst.write_u8(1);
                        filter.encode(dst)?;
                    }
                    None => dst.write_u8(0),
                }
            }
            Self::QueryLogs { substring, last } => {
                dst.write_u8(4);
                write_opt_string(dst, substring.as_deref())?;
                match last {
                    Some(last) => {
                        dst.write_u8(1);
                        dst.write_u32(*last);
                    }
                    None => dst.write_u8(0),
                }
            }
            Self::Screenshot => dst.write_u8(5),
            Self::MouseMove { x, y } => {
                dst.write_u8(6);
                dst.write_u16(*x);
                dst.write_u16(*y);
            }
            Self::MouseButton { button, pressed } => {
                dst.write_u8(7);
                write_mouse_button(dst, *button)?;
                write_bool(dst, *pressed)?;
            }
            Self::Wheel { delta, horizontal } => {
                dst.write_u8(8);
                dst.write_i16(*delta);
                write_bool(dst, *horizontal)?;
            }
            Self::KeyScancode { scancode, pressed } => {
                dst.write_u8(9);
                dst.write_u16(*scancode);
                write_bool(dst, *pressed)?;
            }
            Self::KeyUnicode { ch, pressed } => {
                dst.write_u8(10);
                write_char(dst, *ch)?;
                write_bool(dst, *pressed)?;
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_agent::Request"
    }

    fn size(&self) -> usize {
        1 /* tag */
            + match self {
                Self::Connect { properties, log_directive } => {
                    propertyset::size(properties) + opt_string_size(log_directive.as_deref())
                }
                Self::Disconnect | Self::Status | Self::Screenshot => 0,
                Self::QueryProps { filter } => 1 /* presence */ + filter.as_ref().map_or(0, Encode::size),
                Self::QueryLogs { substring, last } => {
                    opt_string_size(substring.as_deref()) + 1 /* presence */ + last.map_or(0, |_| 4)
                }
                Self::MouseMove { .. } => 2 /* x */ + 2 /* y */,
                Self::MouseButton { .. } => 1 /* button */ + 1 /* pressed */,
                Self::Wheel { .. } => 2 /* delta */ + 1 /* horizontal */,
                Self::KeyScancode { .. } => 2 /* scancode */ + 1 /* pressed */,
                Self::KeyUnicode { .. } => 4 /* ch */ + 1 /* pressed */,
            }
    }
}

impl Decode<'_> for Request {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => {
                let mut properties = PropertySet::new();
                propertyset::read(&mut properties, src)?;
                let log_directive = read_opt_string(src)?;
                Ok(Self::Connect {
                    properties,
                    log_directive,
                })
            }
            1 => Ok(Self::Disconnect),
            2 => Ok(Self::Status),
            3 => {
                ensure_size!(in: src, size: 1);
                let filter = match src.read_u8() {
                    0 => None,
                    1 => Some(KeyFilter::decode(src)?),
                    _ => return Err(ironrdp_core::invalid_field_err!("dump filter", "invalid presence flag")),
                };
                Ok(Self::QueryProps { filter })
            }
            4 => {
                let substring = read_opt_string(src)?;
                ensure_size!(in: src, size: 1);
                let last = match src.read_u8() {
                    0 => None,
                    1 => {
                        ensure_size!(in: src, size: 4);
                        Some(src.read_u32())
                    }
                    _ => return Err(ironrdp_core::invalid_field_err!("query last", "invalid presence flag")),
                };
                Ok(Self::QueryLogs { substring, last })
            }
            5 => Ok(Self::Screenshot),
            6 => {
                ensure_size!(in: src, size: 4);
                let x = src.read_u16();
                let y = src.read_u16();
                Ok(Self::MouseMove { x, y })
            }
            7 => {
                let button = read_mouse_button(src)?;
                let pressed = read_bool(src)?;
                Ok(Self::MouseButton { button, pressed })
            }
            8 => {
                ensure_size!(in: src, size: 2);
                let delta = src.read_i16();
                let horizontal = read_bool(src)?;
                Ok(Self::Wheel { delta, horizontal })
            }
            9 => {
                ensure_size!(in: src, size: 2);
                let scancode = src.read_u16();
                let pressed = read_bool(src)?;
                Ok(Self::KeyScancode { scancode, pressed })
            }
            10 => {
                let ch = read_char(src)?;
                let pressed = read_bool(src)?;
                Ok(Self::KeyUnicode { ch, pressed })
            }
            _ => Err(ironrdp_core::invalid_field_err!("request", "unknown tag")),
        }
    }
}

impl_pdu_pod!(Request);
