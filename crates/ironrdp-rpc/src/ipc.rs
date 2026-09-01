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
use ironrdp_rdpei::pdu::{
    EightByteUnsigned, FourByteSigned, FourByteUnsigned, PenContact, PenContactDataFlags, PenContactFlags, PenEventPdu,
    PenFlags, PenFrame, TouchContact, TouchContactFlags, TouchEventPdu, TouchFrame,
};

use crate::wire::{
    bytes_size, opt_string_size, opt_u16_size, opt_u64_size, propertyset, read_bool, read_bytes, read_char,
    read_mouse_button, read_opt_string, read_opt_u16, read_opt_u64, read_string, string_size, write_bool, write_bytes,
    write_char, write_mouse_button, write_opt_string, write_opt_u16, write_opt_u64, write_string,
};

/// Maximum number of Unicode scalar values accepted in one [`Request::UnicodeText`] request.
///
/// The agent reserves one bounded input-queue entry for each character before submitting any text.
pub const MAX_UNICODE_TEXT_CHARS: usize = 96;

/// Maximum contacts in one MS-RDPEI touch frame accepted over RPC.
pub const MAX_TOUCH_CONTACTS: usize = 10;

/// Maximum touch frames in one [`Request::Touch`] PDU accepted over RPC.
pub const MAX_TOUCH_FRAMES: usize = 64;

/// Maximum contacts in one MS-RDPEI pen frame accepted over RPC.
///
/// Clients currently construct [`ironrdp_rdpei::RdpeiClient::default`], which advertises V200 with
/// empty `CS_READY` feature flags. Until multi-pen negotiation is plumbed through, only a single
/// pen contact with `deviceId == 0` is accepted ([MS-RDPEI] 3.1.1.1).
pub const MAX_PEN_CONTACTS: usize = 1;

/// Maximum pen frames in one [`Request::Pen`] PDU accepted over RPC.
pub const MAX_PEN_FRAMES: usize = 64;

/// Maximum MS-RDPEI pen pressure value ([MS-RDPEI] 2.2.3.7.1.1).
pub const MAX_PEN_PRESSURE: u32 = 1024;

/// Maximum MS-RDPEI pen rotation value in degrees ([MS-RDPEI] 2.2.3.7.1.1).
pub const MAX_PEN_ROTATION: u16 = 359;

/// Maximum absolute MS-RDPEI pen tilt angle in degrees ([MS-RDPEI] 2.2.3.7.1.1).
pub const MAX_PEN_TILT: i16 = 90;

/// Maximum retained RAIL observations, excluding a possible history-gap marker.
pub const MAX_RAIL_RETAINED_EVENTS: usize = 256;

const MAX_RAIL_EVENT_DUMP_EVENTS: usize = MAX_RAIL_RETAINED_EVENTS + 1;

/// One contact sample inside a [`TouchFrameRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchContactRequest {
    pub contact_id: u8,
    pub x: i32,
    pub y: i32,
    /// Raw MS-RDPEI `contactFlags` bits ([MS-RDPEI] 2.2.3.3.1.1).
    pub flags: u16,
}

/// One frame inside a [`Request::Touch`] event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchFrameRequest {
    /// Microseconds since the previous frame (`0` for the first frame of a transaction).
    pub frame_offset: u64,
    pub contacts: Vec<TouchContactRequest>,
}

/// One pen contact sample inside a [`PenFrameRequest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PenContactRequest {
    pub device_id: u8,
    pub x: i32,
    pub y: i32,
    /// Raw MS-RDPEI pen `contactFlags` bits ([MS-RDPEI] 2.2.3.7.1.1).
    pub flags: u16,
    pub pressure: Option<u32>,
    pub rotation: Option<u16>,
    pub tilt_x: Option<i16>,
    pub tilt_y: Option<i16>,
    /// Optional MS-RDPEI `penFlags` bits ([MS-RDPEI] 2.2.3.7.1.1).
    pub pen_flags: Option<u32>,
}

/// One frame inside a [`Request::Pen`] event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PenFrameRequest {
    /// Microseconds since the previous frame (`0` for the first frame of a transaction).
    pub frame_offset: u64,
    pub contacts: Vec<PenContactRequest>,
}

/// Validates and converts an RPC touch request into an MS-RDPEI touch event PDU.
///
/// Rejects empty frames/contacts, count limits, unknown or illegal flag combinations, and
/// values outside the MS-RDPEI varint ranges used on the wire.
pub fn touch_event_from_request(encode_time: u32, frames: Vec<TouchFrameRequest>) -> Result<TouchEventPdu, Response> {
    if frames.is_empty() {
        return Err(Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            "touch event requires at least one frame",
        ));
    }
    if frames.len() > MAX_TOUCH_FRAMES {
        return Err(Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            format!("touch event exceeds the {MAX_TOUCH_FRAMES}-frame limit"),
        ));
    }
    if FourByteUnsigned::new(encode_time).is_err() {
        return Err(Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            "touch encode_time is out of MS-RDPEI range",
        ));
    }

    let mut built_frames = Vec::with_capacity(frames.len());
    for frame in frames {
        if frame.contacts.is_empty() {
            return Err(Response::typed_error(
                AgentErrorCategory::InvalidRequest,
                "touch frame requires at least one contact",
            ));
        }
        if frame.contacts.len() > MAX_TOUCH_CONTACTS {
            return Err(Response::typed_error(
                AgentErrorCategory::InvalidRequest,
                format!("touch frame exceeds the {MAX_TOUCH_CONTACTS}-contact limit"),
            ));
        }
        if EightByteUnsigned::new(frame.frame_offset).is_err() {
            return Err(Response::typed_error(
                AgentErrorCategory::InvalidRequest,
                "touch frame_offset is out of MS-RDPEI range",
            ));
        }

        let mut contacts = Vec::with_capacity(frame.contacts.len());
        for contact in frame.contacts {
            contacts.push(touch_contact_from_request(contact)?);
        }
        built_frames.push(TouchFrame::new(frame.frame_offset, contacts));
    }

    Ok(TouchEventPdu::new(encode_time, built_frames))
}

fn touch_contact_from_request(contact: TouchContactRequest) -> Result<TouchContact, Response> {
    if FourByteSigned::new(contact.x).is_err() || FourByteSigned::new(contact.y).is_err() {
        return Err(Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            "touch contact coordinates are out of MS-RDPEI range",
        ));
    }
    let flags = TouchContactFlags::from_bits(u32::from(contact.flags)).ok_or_else(|| {
        Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            "touch contact flags contain unknown bits",
        )
    })?;
    if !flags.is_legal() {
        return Err(Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            "touch contact flags are not a legal MS-RDPEI combination",
        ));
    }
    Ok(TouchContact::new(contact.contact_id, contact.x, contact.y, flags))
}

/// Validates and converts an RPC pen request into an MS-RDPEI pen event PDU.
///
/// Rejects empty frames/contacts, count limits, nonzero device IDs, a nonzero first-frame offset,
/// unknown or illegal flag combinations, optional-field semantic ranges, and values outside the
/// MS-RDPEI varint ranges used on the wire.
pub fn pen_event_from_request(encode_time: u32, frames: Vec<PenFrameRequest>) -> Result<PenEventPdu, Response> {
    if frames.is_empty() {
        return Err(Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            "pen event requires at least one frame",
        ));
    }
    if frames.len() > MAX_PEN_FRAMES {
        return Err(Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            format!("pen event exceeds the {MAX_PEN_FRAMES}-frame limit"),
        ));
    }
    if FourByteUnsigned::new(encode_time).is_err() {
        return Err(Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            "pen encode_time is out of MS-RDPEI range",
        ));
    }
    if frames[0].frame_offset != 0 {
        return Err(Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            "pen first frame_offset must be zero",
        ));
    }

    let mut built_frames = Vec::with_capacity(frames.len());
    for frame in frames {
        if frame.contacts.is_empty() {
            return Err(Response::typed_error(
                AgentErrorCategory::InvalidRequest,
                "pen frame requires at least one contact",
            ));
        }
        if frame.contacts.len() > MAX_PEN_CONTACTS {
            return Err(Response::typed_error(
                AgentErrorCategory::InvalidRequest,
                format!("pen frame exceeds the {MAX_PEN_CONTACTS}-contact limit"),
            ));
        }
        if EightByteUnsigned::new(frame.frame_offset).is_err() {
            return Err(Response::typed_error(
                AgentErrorCategory::InvalidRequest,
                "pen frame_offset is out of MS-RDPEI range",
            ));
        }

        let mut contacts = Vec::with_capacity(frame.contacts.len());
        for contact in frame.contacts {
            contacts.push(pen_contact_from_request(contact)?);
        }
        built_frames.push(PenFrame::new(frame.frame_offset, contacts));
    }

    Ok(PenEventPdu::new(encode_time, built_frames))
}

fn pen_contact_from_request(contact: PenContactRequest) -> Result<PenContact, Response> {
    if contact.device_id != 0 {
        return Err(Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            "pen device_id must be zero until multi-pen is negotiated",
        ));
    }
    if FourByteSigned::new(contact.x).is_err() || FourByteSigned::new(contact.y).is_err() {
        return Err(Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            "pen contact coordinates are out of MS-RDPEI range",
        ));
    }
    let flags = PenContactFlags::from_bits(u32::from(contact.flags)).ok_or_else(|| {
        Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            "pen contact flags contain unknown bits",
        )
    })?;
    if !flags.is_legal() {
        return Err(Response::typed_error(
            AgentErrorCategory::InvalidRequest,
            "pen contact flags are not a legal MS-RDPEI combination",
        ));
    }

    let mut pen = PenContact::new(contact.device_id, contact.x, contact.y, flags);
    if let Some(pen_flags_bits) = contact.pen_flags {
        let pen_flags = PenFlags::from_bits(pen_flags_bits).ok_or_else(|| {
            Response::typed_error(AgentErrorCategory::InvalidRequest, "pen flags contain unknown bits")
        })?;
        pen = pen.with_pen_flags(pen_flags);
    }
    if let Some(pressure) = contact.pressure {
        if pressure > MAX_PEN_PRESSURE {
            return Err(Response::typed_error(
                AgentErrorCategory::InvalidRequest,
                format!("pen pressure must be in 0..={MAX_PEN_PRESSURE}"),
            ));
        }
        pen = pen.with_pressure(pressure);
    }
    if let Some(rotation) = contact.rotation {
        if rotation > MAX_PEN_ROTATION {
            return Err(Response::typed_error(
                AgentErrorCategory::InvalidRequest,
                format!("pen rotation must be in 0..={MAX_PEN_ROTATION}"),
            ));
        }
        pen = pen.with_rotation(rotation);
    }
    if let Some(tilt_x) = contact.tilt_x {
        if !(-MAX_PEN_TILT..=MAX_PEN_TILT).contains(&tilt_x) {
            return Err(Response::typed_error(
                AgentErrorCategory::InvalidRequest,
                format!("pen tilt_x must be in -{MAX_PEN_TILT}..={MAX_PEN_TILT}"),
            ));
        }
    }
    if let Some(tilt_y) = contact.tilt_y {
        if !(-MAX_PEN_TILT..=MAX_PEN_TILT).contains(&tilt_y) {
            return Err(Response::typed_error(
                AgentErrorCategory::InvalidRequest,
                format!("pen tilt_y must be in -{MAX_PEN_TILT}..={MAX_PEN_TILT}"),
            ));
        }
    }
    match (contact.tilt_x, contact.tilt_y) {
        (Some(tilt_x), Some(tilt_y)) => {
            pen = pen.with_tilt(tilt_x, tilt_y);
        }
        (Some(tilt_x), None) => {
            pen.fields_present.insert(PenContactDataFlags::TILTX_PRESENT);
            pen.fields.tilt_x = Some(tilt_x);
        }
        (None, Some(tilt_y)) => {
            pen.fields_present.insert(PenContactDataFlags::TILTY_PRESENT);
            pen.fields.tilt_y = Some(tilt_y);
        }
        (None, None) => {}
    }
    Ok(pen)
}

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
    /// Capture the most recent frame (cursor composited in) as a PNG.
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
    /// Type bounded Unicode text in ordered FastPath input messages.
    UnicodeText { text: String },
    /// Resize the remote desktop.
    Resize { width: u16, height: u16 },
    /// Query and negotiate the capabilities of the session's NOW endpoint.
    NowCapabilities,
    /// Submit an untracked generic NOW Run request.
    NowRun {
        /// Command line interpreted by the remote NOW agent.
        command: String,
        /// Optional remote working directory.
        directory: Option<String>,
    },
    /// Submit a NOW operation that is tracked by the daemon unless `detached` is set.
    NowExecute(NowExecutionRequest),
    /// Request cancellation of the active tracked NOW operation.
    NowCancel { operation_id: u64 },
    /// List daemon-retained NOW operations.
    NowList,
    /// Inspect a retained NOW operation.
    NowStatus { operation_id: u64 },
    /// Replay retained operation events after `after_sequence` and then follow live output.
    NowAttach {
        operation_id: u64,
        after_sequence: Option<u64>,
    },
    /// Forward a bounded raw stdin chunk to a tracked operation.
    NowStdin {
        operation_id: u64,
        data: Vec<u8>,
        last: bool,
    },
    /// Inspect the local NOW endpoint without exposing protocol internals.
    NowDiagnostics,
    /// Queue one MS-RDPEI touch event PDU (`RDPINPUT_TOUCH_EVENT_PDU`) for the session input loop.
    ///
    /// `Ok` means the request was validated and reserved on the local input queue, not that the
    /// Input DVC delivered it to the host. The session loop may still drop the event when RDPEI is
    /// absent, unready, or suspended. Contacts must use legal flag sets from [MS-RDPEI] 3.1.1.1. At
    /// most [`MAX_TOUCH_FRAMES`] frames and [`MAX_TOUCH_CONTACTS`] contacts per frame are accepted.
    Touch {
        /// Milliseconds elapsed for the oldest frame in this PDU.
        encode_time: u32,
        frames: Vec<TouchFrameRequest>,
    },
    /// Queue one MS-RDPEI pen event PDU (`RDPINPUT_PEN_EVENT_PDU`) for the session input loop.
    ///
    /// `Ok` means the request was validated and reserved on the local input queue, not that the
    /// Input DVC delivered it to the host. The session loop may still drop the event when RDPEI is
    /// absent, unready, suspended, or below the negotiated version that allows pen (V200+). Until
    /// multi-pen is negotiated, only one contact with `deviceId == 0` is accepted.
    Pen {
        encode_time: u32,
        frames: Vec<PenFrameRequest>,
    },
    /// Dismiss a hovering touch contact (`RDPINPUT_DISMISS_HOVERING_TOUCH_CONTACT_PDU`).
    DismissHoveringTouchContact { contact_id: u8 },
    /// Inspect the bounded, session-local RAIL observation ledger.
    RailStatus,
    /// Return RAIL observations after a sequence number.
    RailEvents { after_sequence: Option<u64> },
    /// Wait for a RAIL observation after a sequence number.
    RailWait {
        after_sequence: Option<u64>,
        timeout_ms: u32,
    },
    /// Queue a validated RAIL Execute request.
    RailExecute(RailExecuteRequest),
    /// Return the last text received from the remote clipboard, if any.
    ClipboardGet,
    /// Set the local clipboard text and advertise it to the remote (`CF_UNICODETEXT` only).
    ClipboardSet { text: String },
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
            Self::UnicodeText { text } => f
                .debug_struct("UnicodeText")
                .field("char_count", &text.chars().count())
                .finish(),
            Self::Resize { width, height } => f
                .debug_struct("Resize")
                .field("width", width)
                .field("height", height)
                .finish(),
            Self::NowCapabilities => f.write_str("NowCapabilities"),
            Self::NowRun { command, directory } => f
                .debug_struct("NowRun")
                .field("command_len", &command.len())
                .field("directory", directory)
                .finish(),
            Self::NowExecute(request) => f.debug_tuple("NowExecute").field(request).finish(),
            Self::NowCancel { operation_id } => {
                f.debug_struct("NowCancel").field("operation_id", operation_id).finish()
            }
            Self::NowList => f.write_str("NowList"),
            Self::NowStatus { operation_id } => {
                f.debug_struct("NowStatus").field("operation_id", operation_id).finish()
            }
            Self::NowAttach {
                operation_id,
                after_sequence,
            } => f
                .debug_struct("NowAttach")
                .field("operation_id", operation_id)
                .field("after_sequence", after_sequence)
                .finish(),
            Self::NowStdin {
                operation_id,
                data,
                last,
            } => f
                .debug_struct("NowStdin")
                .field("operation_id", operation_id)
                .field("data_len", &data.len())
                .field("last", last)
                .finish(),
            Self::NowDiagnostics => f.write_str("NowDiagnostics"),
            Self::Touch { encode_time, frames } => f
                .debug_struct("Touch")
                .field("encode_time", encode_time)
                .field("frames", frames)
                .finish(),
            Self::Pen { encode_time, frames } => f
                .debug_struct("Pen")
                .field("encode_time", encode_time)
                .field("frames", frames)
                .finish(),
            Self::DismissHoveringTouchContact { contact_id } => f
                .debug_struct("DismissHoveringTouchContact")
                .field("contact_id", contact_id)
                .finish(),
            Self::RailStatus => f.write_str("RailStatus"),
            Self::RailEvents { after_sequence } => f
                .debug_struct("RailEvents")
                .field("after_sequence", after_sequence)
                .finish(),
            Self::RailWait {
                after_sequence,
                timeout_ms,
            } => f
                .debug_struct("RailWait")
                .field("after_sequence", after_sequence)
                .field("timeout_ms", timeout_ms)
                .finish(),
            Self::RailExecute(request) => f.debug_tuple("RailExecute").field(request).finish(),
            Self::ClipboardGet => f.write_str("ClipboardGet"),
            // Never print clipboard contents.
            Self::ClipboardSet { text } => f.debug_struct("ClipboardSet").field("text_len", &text.len()).finish(),
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
    Err(AgentError),
}

impl Response {
    /// A successful response with no payload.
    pub fn ok() -> Self {
        Self::Ok(Payload::Empty)
    }

    /// A failure response.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Err(AgentError::internal(message))
    }

    /// A typed failure response.
    pub fn typed_error(category: AgentErrorCategory, message: impl Into<String>) -> Self {
        Self::Err(AgentError {
            category,
            message: message.into(),
        })
    }

    /// Whether this is a success response.
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_))
    }
}

/// The success payload carried by [`Response::Ok`].
#[derive(Clone, PartialEq, Eq)]
pub enum Payload {
    /// No data.
    Empty,
    /// Current session status.
    Status(StatusInfo),
    /// A dump of the live property bag.
    Properties(PropertyDump),
    /// Retained log lines.
    Logs(Vec<String>),
    /// The most recent frame encoded as a PNG (cursor included), with its dimensions.
    Screenshot { width: u16, height: u16, png: Vec<u8> },
    /// Negotiated NOW capabilities.
    NowCapabilities(NowCapabilities),
    /// One durable NOW operation.
    NowOperation(OperationInfo),
    /// Retained NOW operations.
    NowOperations(Vec<OperationInfo>),
    /// A sequenced NOW operation event. Streaming requests receive one response per event.
    NowEvent(OperationEvent),
    /// Local NOW endpoint diagnostic state.
    NowDiagnostics(NowDiagnostics),
    /// Session-local RAIL state.
    RailStatus(RailStatusInfo),
    /// Sequenced RAIL observations.
    RailEvents(RailEventDump),
    /// A locally assigned RAIL launch identifier.
    RailLaunch(RailLaunchInfo),
    /// The remote clipboard's last `CF_UNICODETEXT` text, or `None` if unavailable.
    ClipboardText(Option<String>),
}

impl fmt::Debug for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("Empty"),
            Self::Status(status) => f.debug_tuple("Status").field(status).finish(),
            Self::Properties(dump) => f.debug_tuple("Properties").field(dump).finish(),
            Self::Logs(lines) => f.debug_tuple("Logs").field(lines).finish(),
            // Print the PNG byte length rather than the (large, binary) blob.
            Self::Screenshot { width, height, png } => f
                .debug_struct("Screenshot")
                .field("width", width)
                .field("height", height)
                .field("png_len", &png.len())
                .finish(),
            Self::NowCapabilities(capabilities) => f.debug_tuple("NowCapabilities").field(capabilities).finish(),
            Self::NowOperation(operation) => f.debug_tuple("NowOperation").field(operation).finish(),
            Self::NowOperations(operations) => f.debug_tuple("NowOperations").field(operations).finish(),
            Self::NowEvent(event) => f.debug_tuple("NowEvent").field(event).finish(),
            Self::NowDiagnostics(diagnostics) => f.debug_tuple("NowDiagnostics").field(diagnostics).finish(),
            Self::RailStatus(status) => f.debug_tuple("RailStatus").field(status).finish(),
            Self::RailEvents(events) => f.debug_tuple("RailEvents").field(events).finish(),
            Self::RailLaunch(launch) => f.debug_tuple("RailLaunch").field(launch).finish(),
            // Never print clipboard contents.
            Self::ClipboardText(text) => f
                .debug_tuple("ClipboardText")
                .field(&text.as_ref().map(String::len))
                .finish(),
        }
    }
}

/// One locally initiated RAIL launch.
///
/// `launch_id` is assigned by the daemon for audit correlation. It is not a protocol identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailLaunchInfo {
    pub launch_id: u64,
    pub executable: String,
    pub flags: u16,
}

/// A validated RAIL Execute request.
#[derive(Clone, PartialEq, Eq)]
pub struct RailExecuteRequest {
    pub executable: String,
    pub working_directory: String,
    pub arguments: String,
    pub flags: u16,
}

impl fmt::Debug for RailExecuteRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RailExecuteRequest")
            .field("executable_len", &self.executable.len())
            .field("working_directory_len", &self.working_directory.len())
            .field("arguments_len", &self.arguments.len())
            .field("flags", &self.flags)
            .finish()
    }
}

/// Summary of the current RAIL observation generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailStatusInfo {
    pub generation: u64,
    pub next_sequence: u64,
    pub handshake_complete: bool,
    pub desktop_synchronized: bool,
    pub pending_launches: Vec<RailLaunchInfo>,
}

/// Bounded RAIL observation history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailEventDump {
    pub generation: u64,
    pub events: Vec<RailEvent>,
}

/// A sequence-numbered RAIL observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailEvent {
    pub sequence: u64,
    pub kind: RailEventKind,
}

/// One client-validated RAIL observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RailEventKind {
    Handshake {
        handshake_ex_flags: Option<u32>,
        initialization_message_count: u16,
        queued_execute_count: u16,
    },
    DesktopSynchronized {
        released_execute_count: u16,
    },
    PostHandshakeQueueReleased {
        released_execute_count: u16,
    },
    ExecuteQueued(RailLaunchInfo),
    ExecuteResult {
        launch_id: Option<u64>,
        executable: String,
        flags: u16,
        result: u16,
        raw_result: u32,
    },
    /// A locally accepted Execute request could not be processed.
    ExecuteFailed {
        launch_id: Option<u64>,
        executable: String,
        flags: u16,
        reason: RailExecuteFailureReason,
    },
    ApplicationId {
        window_id: u32,
        application_id: String,
        process_id: Option<u32>,
        process_image_name: Option<String>,
    },
    Control {
        kind: String,
    },
    WindowingOrders {
        byte_count: u32,
    },
    /// History before this sequence was evicted from the bounded ledger.
    Gap {
        lost_through: u64,
    },
}

/// Stable, command-free diagnostic for a locally failed RAIL Execute request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailExecuteFailureReason {
    /// The active session did not retain a RAIL static-channel processor.
    RailUnavailable,
    /// The RAIL client rejected the Execute request before sending it.
    QueueRejected,
    /// The active stage could not encode the queued RAIL messages.
    MessageProcessingFailed,
}

impl RailExecuteFailureReason {
    /// Stable lowercase diagnostic text for structured CLI output.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RailUnavailable => "rail_unavailable",
            Self::QueueRejected => "queue_rejected",
            Self::MessageProcessingFailed => "message_processing_failed",
        }
    }

    const fn tag(self) -> u8 {
        match self {
            Self::RailUnavailable => 0,
            Self::QueueRejected => 1,
            Self::MessageProcessingFailed => 2,
        }
    }

    fn from_tag(tag: u8) -> DecodeResult<Self> {
        match tag {
            0 => Ok(Self::RailUnavailable),
            1 => Ok(Self::QueueRejected),
            2 => Ok(Self::MessageProcessingFailed),
            _ => Err(ironrdp_core::invalid_field_err!(
                "RAIL Execute failure",
                "unknown reason"
            )),
        }
    }
}

/// Machine-readable error category. The message is safe for display but must not include request
/// command text or stdin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentErrorCategory {
    /// The caller supplied an invalid request.
    InvalidRequest,
    /// No suitable connected RDP/NOW session exists.
    Unavailable,
    /// A tracked operation conflicts with another active tracked operation.
    Conflict,
    /// The local transport or NOW worker failed.
    Transport,
    /// The remote peer rejected or failed an operation.
    Remote,
    /// An internal daemon operation failed.
    Internal,
}

impl AgentErrorCategory {
    /// Stable lowercase category used by structured CLI output.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Unavailable => "unavailable",
            Self::Conflict => "conflict",
            Self::Transport => "transport",
            Self::Remote => "remote",
            Self::Internal => "internal",
        }
    }
}

/// Typed IPC error response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentError {
    /// Error category for JSON/NDJSON clients.
    pub category: AgentErrorCategory,
    /// Display-safe lowercase error message.
    pub message: String,
}

impl AgentError {
    /// Creates an internal error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            category: AgentErrorCategory::Internal,
            message: message.into(),
        }
    }
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl core::error::Error for AgentError {}

/// NOW execution style intentionally exposed by the agent. Shell is not an option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NowExecutionKind {
    /// Windows CreateProcess.
    Process,
    /// Windows Batch.
    Batch,
    /// Windows PowerShell.
    PowerShell,
    /// PowerShell 7.
    Pwsh,
}

/// Request for a supported NOW tracked or detached execution.
#[derive(Clone, PartialEq, Eq)]
pub struct NowExecutionRequest {
    /// Exposed NOW execution style.
    pub kind: NowExecutionKind,
    /// Program path for Process, or script/command for the other styles.
    pub command: String,
    /// Process command-line parameters.
    pub parameters: Option<String>,
    /// Optional remote working directory.
    pub directory: Option<String>,
    /// Optional initial stdin.
    pub stdin: Option<Vec<u8>>,
    /// Optional command deadline in milliseconds.
    pub timeout_ms: Option<u64>,
    /// Ask the peer to detach. Detached commands have no retained output or result.
    pub detached: bool,
    /// Request PowerShell's `-NoProfile` mode.
    pub no_profile: bool,
    /// Request PowerShell's `-NonInteractive` mode.
    pub non_interactive: bool,
}

impl fmt::Debug for NowExecutionRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NowExecutionRequest")
            .field("kind", &self.kind)
            .field("command_len", &self.command.len())
            .field("parameters_len", &self.parameters.as_ref().map(String::len))
            .field("directory", &self.directory)
            .field("stdin_len", &self.stdin.as_ref().map(Vec::len))
            .field("timeout_ms", &self.timeout_ms)
            .field("detached", &self.detached)
            .field("no_profile", &self.no_profile)
            .field("non_interactive", &self.non_interactive)
            .finish()
    }
}

/// Durable operation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationState {
    /// The remote operation is active.
    Running,
    /// Cancellation has been sent and accepted or is awaiting a terminal result.
    Cancelling,
    /// The remote operation completed normally.
    Completed,
    /// The remote operation was cancelled.
    Cancelled,
    /// The operation failed locally or remotely.
    Failed,
    /// A detached operation was submitted and cannot report further state.
    Detached,
}

/// A stream associated with a raw output chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NowStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}

/// Daemon-retained metadata for one NOW operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationInfo {
    /// Daemon-generated operation identity.
    pub id: u64,
    /// Submitted NOW style.
    pub kind: NowExecutionKind,
    /// Current or terminal state.
    pub state: OperationState,
    /// Whether this was submitted detached.
    pub detached: bool,
    /// Remote exit code, including nonzero values.
    pub exit_code: Option<u32>,
    /// Typed terminal failure, when any.
    pub error: Option<AgentError>,
    /// Bytes of raw stdout/stderr currently retained for this operation.
    pub retained_output_bytes: u64,
    /// Sequence assigned to the next operation event.
    pub next_sequence: u64,
}

/// A replayable NOW operation event.
#[derive(Clone, PartialEq, Eq)]
pub struct OperationEvent {
    /// Daemon-generated operation identity.
    pub operation_id: u64,
    /// Monotonically increasing sequence number scoped to the operation.
    pub sequence: u64,
    /// Event content.
    pub kind: OperationEventKind,
}

impl fmt::Debug for OperationEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OperationEvent")
            .field("operation_id", &self.operation_id)
            .field("sequence", &self.sequence)
            .field("kind", &self.kind)
            .finish()
    }
}

/// Content of a replayable NOW operation event.
#[derive(Clone, PartialEq, Eq)]
pub enum OperationEventKind {
    /// The remote peer started the command.
    Started,
    /// A raw output chunk.
    Output {
        /// Output stream.
        stream: NowStream,
        /// Exact bytes received from NOW.
        data: Vec<u8>,
        /// Whether this is the final chunk on this stream.
        last: bool,
    },
    /// The remote peer accepted a cancellation request.
    CancelAccepted,
    /// The command completed with an exit code.
    Completed { exit_code: u32 },
    /// The command was cancelled.
    Cancelled,
    /// The command terminated with a typed error.
    Failed(AgentError),
}

impl fmt::Debug for OperationEventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Started => f.write_str("Started"),
            Self::Output { stream, data, last } => f
                .debug_struct("Output")
                .field("stream", stream)
                .field("data_len", &data.len())
                .field("last", last)
                .finish(),
            Self::CancelAccepted => f.write_str("CancelAccepted"),
            Self::Completed { exit_code } => f.debug_struct("Completed").field("exit_code", exit_code).finish(),
            Self::Cancelled => f.write_str("Cancelled"),
            Self::Failed(error) => f.debug_tuple("Failed").field(error).finish(),
        }
    }
}

/// Local NOW endpoint diagnostic snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowDiagnostics {
    /// Whether the RDP session has allocated a NOW DVC endpoint.
    pub endpoint_allocated: bool,
    /// Whether a NOW client handle is currently cached.
    pub connected: bool,
    /// Current cached capabilities, if a connection has already been established.
    pub capabilities: Option<NowCapabilities>,
}

/// Negotiated NOW capabilities exposed without leaking the NOW PDU types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NowCapabilities {
    /// NOW protocol major version.
    pub version_major: u16,
    /// NOW protocol minor version.
    pub version_minor: u16,
    /// Negotiated heartbeat in milliseconds, if any.
    pub heartbeat_ms: Option<u64>,
    /// Generic Run support.
    pub run: bool,
    /// CreateProcess support.
    pub process: bool,
    /// Batch support.
    pub batch: bool,
    /// Windows PowerShell support.
    pub powershell: bool,
    /// PowerShell 7 support.
    pub pwsh: bool,
    /// Tracked I/O redirection support.
    pub io_redirection: bool,
    /// Unicode console support.
    pub unicode_console: bool,
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
    /// A graceful disconnect was requested; the engine thread is still shutting down.
    Disconnecting,
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
            Self::Disconnecting => 5,
        }
    }

    fn from_tag(tag: u8) -> DecodeResult<Self> {
        match tag {
            0 => Ok(Self::NoSession),
            1 => Ok(Self::Connecting),
            2 => Ok(Self::Connected),
            3 => Ok(Self::Disconnected),
            4 => Ok(Self::Failed),
            5 => Ok(Self::Disconnecting),
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

// ── Pen contact RPC helpers ─────────────────────────────────────────────────

const PEN_CONTACT_PRESSURE: u8 = 0x01;
const PEN_CONTACT_ROTATION: u8 = 0x02;
const PEN_CONTACT_TILT_X: u8 = 0x04;
const PEN_CONTACT_TILT_Y: u8 = 0x08;
const PEN_CONTACT_PEN_FLAGS: u8 = 0x10;

fn pen_contact_size(contact: &PenContactRequest) -> usize {
    1 /* device_id */ + 4 /* x */ + 4 /* y */ + 2 /* flags */ + 1 /* presence */
        + contact.pressure.map_or(0, |_| 4)
        + contact.rotation.map_or(0, |_| 2)
        + contact.tilt_x.map_or(0, |_| 2)
        + contact.tilt_y.map_or(0, |_| 2)
        + contact.pen_flags.map_or(0, |_| 4)
}

fn write_pen_contact(dst: &mut WriteCursor<'_>, contact: &PenContactRequest) -> EncodeResult<()> {
    dst.write_u8(contact.device_id);
    dst.write_i32(contact.x);
    dst.write_i32(contact.y);
    dst.write_u16(contact.flags);
    let mut presence = 0u8;
    if contact.pressure.is_some() {
        presence |= PEN_CONTACT_PRESSURE;
    }
    if contact.rotation.is_some() {
        presence |= PEN_CONTACT_ROTATION;
    }
    if contact.tilt_x.is_some() {
        presence |= PEN_CONTACT_TILT_X;
    }
    if contact.tilt_y.is_some() {
        presence |= PEN_CONTACT_TILT_Y;
    }
    if contact.pen_flags.is_some() {
        presence |= PEN_CONTACT_PEN_FLAGS;
    }
    dst.write_u8(presence);
    if let Some(pressure) = contact.pressure {
        dst.write_u32(pressure);
    }
    if let Some(rotation) = contact.rotation {
        dst.write_u16(rotation);
    }
    if let Some(tilt_x) = contact.tilt_x {
        dst.write_i16(tilt_x);
    }
    if let Some(tilt_y) = contact.tilt_y {
        dst.write_i16(tilt_y);
    }
    if let Some(pen_flags) = contact.pen_flags {
        dst.write_u32(pen_flags);
    }
    Ok(())
}

fn read_pen_contact(src: &mut ReadCursor<'_>) -> DecodeResult<PenContactRequest> {
    ensure_size!(in: src, size: 12);
    let device_id = src.read_u8();
    let x = src.read_i32();
    let y = src.read_i32();
    let flags = src.read_u16();
    let presence = src.read_u8();
    let pressure = if presence & PEN_CONTACT_PRESSURE != 0 {
        ensure_size!(in: src, size: 4);
        Some(src.read_u32())
    } else {
        None
    };
    let rotation = if presence & PEN_CONTACT_ROTATION != 0 {
        ensure_size!(in: src, size: 2);
        Some(src.read_u16())
    } else {
        None
    };
    let tilt_x = if presence & PEN_CONTACT_TILT_X != 0 {
        ensure_size!(in: src, size: 2);
        Some(src.read_i16())
    } else {
        None
    };
    let tilt_y = if presence & PEN_CONTACT_TILT_Y != 0 {
        ensure_size!(in: src, size: 2);
        Some(src.read_i16())
    } else {
        None
    };
    let pen_flags = if presence & PEN_CONTACT_PEN_FLAGS != 0 {
        ensure_size!(in: src, size: 4);
        Some(src.read_u32())
    } else {
        None
    };
    Ok(PenContactRequest {
        device_id,
        x,
        y,
        flags,
        pressure,
        rotation,
        tilt_x,
        tilt_y,
        pen_flags,
    })
}

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
        "ironrdp_rpc::ipc::KeyFilter"
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
            _ => Err(ironrdp_core::invalid_field_err!("key filter", "unknown tag", in: src)),
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
        "ironrdp_rpc::ipc::PropValue"
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
            _ => Err(ironrdp_core::invalid_field_err!("property value", "unknown tag", in: src)),
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
        "ironrdp_rpc::ipc::PropertyEntry"
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
        let count: u32 = cast_length!("property count", self.entries.len(), in: dst)?;
        dst.write_u32(count);
        for entry in &self.entries {
            entry.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::PropertyDump"
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
        "ironrdp_rpc::ipc::StatusInfo"
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

// ── RAIL audit codec ─────────────────────────────────────────────────────────

impl Encode for RailLaunchInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u64(self.launch_id);
        write_string(dst, &self.executable)?;
        dst.write_u16(self.flags);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::RailLaunchInfo"
    }

    fn size(&self) -> usize {
        8 /* launch_id */ + string_size(&self.executable) + 2 /* flags */
    }
}

impl Decode<'_> for RailLaunchInfo {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 8);
        let launch_id = src.read_u64();
        let executable = read_string(src)?;
        ensure_size!(in: src, size: 2);
        let flags = src.read_u16();
        Ok(Self {
            launch_id,
            executable,
            flags,
        })
    }
}

impl_pdu_pod!(RailLaunchInfo);

impl Encode for RailExecuteRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        write_string(dst, &self.executable)?;
        write_string(dst, &self.working_directory)?;
        write_string(dst, &self.arguments)?;
        dst.write_u16(self.flags);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::RailExecuteRequest"
    }

    fn size(&self) -> usize {
        string_size(&self.executable) + string_size(&self.working_directory) + string_size(&self.arguments) + 2 /* flags */
    }
}

impl Decode<'_> for RailExecuteRequest {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let executable = read_string(src)?;
        let working_directory = read_string(src)?;
        let arguments = read_string(src)?;
        ensure_size!(in: src, size: 2);
        let flags = src.read_u16();
        Ok(Self {
            executable,
            working_directory,
            arguments,
            flags,
        })
    }
}

impl_pdu_pod!(RailExecuteRequest);

impl Encode for RailStatusInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u64(self.generation);
        dst.write_u64(self.next_sequence);
        write_bool(dst, self.handshake_complete)?;
        write_bool(dst, self.desktop_synchronized)?;
        let count: u32 = cast_length!("pending RAIL launch count", self.pending_launches.len())?;
        dst.write_u32(count);
        for launch in &self.pending_launches {
            launch.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::RailStatusInfo"
    }

    fn size(&self) -> usize {
        8 /* generation */
            + 8 /* next_sequence */
            + 1 /* handshake_complete */
            + 1 /* desktop_synchronized */
            + 4 /* pending launch count */
            + self.pending_launches.iter().map(Encode::size).sum::<usize>()
    }
}

impl Decode<'_> for RailStatusInfo {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 16);
        let generation = src.read_u64();
        let next_sequence = src.read_u64();
        let handshake_complete = read_bool(src)?;
        let desktop_synchronized = read_bool(src)?;
        ensure_size!(in: src, size: 4);
        let count = src.read_u32();
        let mut pending_launches = Vec::new();
        for _ in 0..count {
            pending_launches.push(RailLaunchInfo::decode(src)?);
        }
        Ok(Self {
            generation,
            next_sequence,
            handshake_complete,
            desktop_synchronized,
            pending_launches,
        })
    }
}

impl_pdu_pod!(RailStatusInfo);

impl Encode for RailEventKind {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Handshake {
                handshake_ex_flags,
                initialization_message_count,
                queued_execute_count,
            } => {
                dst.write_u8(0);
                match handshake_ex_flags {
                    Some(flags) => {
                        dst.write_u8(1);
                        dst.write_u32(*flags);
                    }
                    None => dst.write_u8(0),
                }
                dst.write_u16(*initialization_message_count);
                dst.write_u16(*queued_execute_count);
            }
            Self::DesktopSynchronized { released_execute_count } => {
                dst.write_u8(1);
                dst.write_u16(*released_execute_count);
            }
            Self::PostHandshakeQueueReleased { released_execute_count } => {
                dst.write_u8(2);
                dst.write_u16(*released_execute_count);
            }
            Self::ExecuteQueued(launch) => {
                dst.write_u8(3);
                launch.encode(dst)?;
            }
            Self::ExecuteResult {
                launch_id,
                executable,
                flags,
                result,
                raw_result,
            } => {
                dst.write_u8(4);
                write_opt_u64(dst, *launch_id)?;
                write_string(dst, executable)?;
                dst.write_u16(*flags);
                dst.write_u16(*result);
                dst.write_u32(*raw_result);
            }
            Self::ExecuteFailed {
                launch_id,
                executable,
                flags,
                reason,
            } => {
                dst.write_u8(9);
                write_opt_u64(dst, *launch_id)?;
                write_string(dst, executable)?;
                dst.write_u16(*flags);
                dst.write_u8(reason.tag());
            }
            Self::ApplicationId {
                window_id,
                application_id,
                process_id,
                process_image_name,
            } => {
                dst.write_u8(5);
                dst.write_u32(*window_id);
                write_string(dst, application_id)?;
                match process_id {
                    Some(process_id) => {
                        dst.write_u8(1);
                        dst.write_u32(*process_id);
                    }
                    None => dst.write_u8(0),
                }
                write_opt_string(dst, process_image_name.as_deref())?;
            }
            Self::Control { kind } => {
                dst.write_u8(6);
                write_string(dst, kind)?;
            }
            Self::WindowingOrders { byte_count } => {
                dst.write_u8(7);
                dst.write_u32(*byte_count);
            }
            Self::Gap { lost_through } => {
                dst.write_u8(8);
                dst.write_u64(*lost_through);
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::RailEventKind"
    }

    fn size(&self) -> usize {
        1 /* tag */
            + match self {
                Self::Handshake {
                    handshake_ex_flags,
                    ..
                } => 1 /* flags presence */ + handshake_ex_flags.map_or(0, |_| 4) + 2 + 2,
                Self::DesktopSynchronized { .. } | Self::PostHandshakeQueueReleased { .. } => 2,
                Self::ExecuteQueued(launch) => launch.size(),
                Self::ExecuteResult {
                    launch_id,
                    executable,
                    ..
                } => opt_u64_size(*launch_id) + string_size(executable) + 2 + 2 + 4,
                Self::ExecuteFailed {
                    launch_id,
                    executable,
                    ..
                } => opt_u64_size(*launch_id) + string_size(executable) + 2 + 1,
                Self::ApplicationId {
                    application_id,
                    process_id,
                    process_image_name,
                    ..
                } => 4 + string_size(application_id) + 1 + process_id.map_or(0, |_| 4) + opt_string_size(process_image_name.as_deref()),
                Self::Control { kind } => string_size(kind),
                Self::WindowingOrders { .. } => 4,
                Self::Gap { .. } => 8,
            }
    }
}

impl Decode<'_> for RailEventKind {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => {
                ensure_size!(in: src, size: 1);
                let handshake_ex_flags = match src.read_u8() {
                    0 => None,
                    1 => {
                        ensure_size!(in: src, size: 4);
                        Some(src.read_u32())
                    }
                    _ => {
                        return Err(ironrdp_core::invalid_field_err!(
                            "RAIL handshake",
                            "invalid flags presence"
                        ));
                    }
                };
                ensure_size!(in: src, size: 4);
                Ok(Self::Handshake {
                    handshake_ex_flags,
                    initialization_message_count: src.read_u16(),
                    queued_execute_count: src.read_u16(),
                })
            }
            1 => {
                ensure_size!(in: src, size: 2);
                Ok(Self::DesktopSynchronized {
                    released_execute_count: src.read_u16(),
                })
            }
            2 => {
                ensure_size!(in: src, size: 2);
                Ok(Self::PostHandshakeQueueReleased {
                    released_execute_count: src.read_u16(),
                })
            }
            3 => Ok(Self::ExecuteQueued(RailLaunchInfo::decode(src)?)),
            4 => {
                let launch_id = read_opt_u64(src)?;
                let executable = read_string(src)?;
                ensure_size!(in: src, size: 8);
                Ok(Self::ExecuteResult {
                    launch_id,
                    executable,
                    flags: src.read_u16(),
                    result: src.read_u16(),
                    raw_result: src.read_u32(),
                })
            }
            9 => {
                let launch_id = read_opt_u64(src)?;
                let executable = read_string(src)?;
                ensure_size!(in: src, size: 3);
                Ok(Self::ExecuteFailed {
                    launch_id,
                    executable,
                    flags: src.read_u16(),
                    reason: RailExecuteFailureReason::from_tag(src.read_u8())?,
                })
            }
            5 => {
                ensure_size!(in: src, size: 4);
                let window_id = src.read_u32();
                let application_id = read_string(src)?;
                ensure_size!(in: src, size: 1);
                let process_id = match src.read_u8() {
                    0 => None,
                    1 => {
                        ensure_size!(in: src, size: 4);
                        Some(src.read_u32())
                    }
                    _ => {
                        return Err(ironrdp_core::invalid_field_err!(
                            "RAIL application ID",
                            "invalid process ID presence"
                        ));
                    }
                };
                let process_image_name = read_opt_string(src)?;
                Ok(Self::ApplicationId {
                    window_id,
                    application_id,
                    process_id,
                    process_image_name,
                })
            }
            6 => Ok(Self::Control {
                kind: read_string(src)?,
            }),
            7 => {
                ensure_size!(in: src, size: 4);
                Ok(Self::WindowingOrders {
                    byte_count: src.read_u32(),
                })
            }
            8 => {
                ensure_size!(in: src, size: 8);
                Ok(Self::Gap {
                    lost_through: src.read_u64(),
                })
            }
            _ => Err(ironrdp_core::invalid_field_err!("RAIL event", "unknown tag")),
        }
    }
}

impl_pdu_pod!(RailEventKind);

impl Encode for RailEvent {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u64(self.sequence);
        self.kind.encode(dst)
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::RailEvent"
    }

    fn size(&self) -> usize {
        8 /* sequence */ + self.kind.size()
    }
}

impl Decode<'_> for RailEvent {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 8);
        Ok(Self {
            sequence: src.read_u64(),
            kind: RailEventKind::decode(src)?,
        })
    }
}

impl_pdu_pod!(RailEvent);

impl Encode for RailEventDump {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if self.events.len() > MAX_RAIL_EVENT_DUMP_EVENTS {
            return Err(ironrdp_core::invalid_field_err!("RAIL events", "count exceeds limit"));
        }
        ensure_size!(in: dst, size: self.size());
        dst.write_u64(self.generation);
        let count: u32 = cast_length!("RAIL event count", self.events.len())?;
        dst.write_u32(count);
        for event in &self.events {
            event.encode(dst)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::RailEventDump"
    }

    fn size(&self) -> usize {
        8 /* generation */ + 4 /* event count */ + self.events.iter().map(Encode::size).sum::<usize>()
    }
}

impl Decode<'_> for RailEventDump {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 12);
        let generation = src.read_u64();
        let count = usize::try_from(src.read_u32())
            .map_err(|_| ironrdp_core::other_err!("RAIL events", "count does not fit in usize"))?;
        if count > MAX_RAIL_EVENT_DUMP_EVENTS {
            return Err(ironrdp_core::invalid_field_err!("RAIL events", "count exceeds limit"));
        }
        let mut events = Vec::with_capacity(count);
        for _ in 0..count {
            events.push(RailEvent::decode(src)?);
        }
        Ok(Self { generation, events })
    }
}

impl_pdu_pod!(RailEventDump);

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
                let count: u32 = cast_length!("log line count", lines.len(), in: dst)?;
                dst.write_u32(count);
                for line in lines {
                    write_string(dst, line)?;
                }
            }
            Self::Screenshot { width, height, png } => {
                dst.write_u8(4);
                dst.write_u16(*width);
                dst.write_u16(*height);
                write_bytes(dst, png)?;
            }
            Self::NowCapabilities(capabilities) => {
                dst.write_u8(5);
                capabilities.encode(dst)?;
            }
            Self::NowOperation(operation) => {
                dst.write_u8(6);
                operation.encode(dst)?;
            }
            Self::NowOperations(operations) => {
                dst.write_u8(7);
                let count: u32 = cast_length!("operation count", operations.len(), in: dst)?;
                dst.write_u32(count);
                for operation in operations {
                    operation.encode(dst)?;
                }
            }
            Self::NowEvent(event) => {
                dst.write_u8(8);
                event.encode(dst)?;
            }
            Self::NowDiagnostics(diagnostics) => {
                dst.write_u8(9);
                diagnostics.encode(dst)?;
            }
            Self::RailStatus(status) => {
                dst.write_u8(10);
                status.encode(dst)?;
            }
            Self::RailEvents(events) => {
                dst.write_u8(11);
                events.encode(dst)?;
            }
            Self::RailLaunch(launch) => {
                dst.write_u8(12);
                launch.encode(dst)?;
            }
            Self::ClipboardText(text) => {
                dst.write_u8(13);
                write_opt_string(dst, text.as_deref())?;
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::Payload"
    }

    fn size(&self) -> usize {
        1 /* tag */
            + match self {
                Self::Empty => 0,
                Self::Status(status) => status.size(),
                Self::Properties(dump) => dump.size(),
                Self::Logs(lines) => 4 + lines.iter().map(|line| string_size(line)).sum::<usize>(),
                Self::Screenshot { png, .. } => 2 /* width */ + 2 /* height */ + bytes_size(png),
                Self::NowCapabilities(capabilities) => capabilities.size(),
                Self::NowOperation(operation) => operation.size(),
                Self::NowOperations(operations) => 4 + operations.iter().map(Encode::size).sum::<usize>(),
                Self::NowEvent(event) => event.size(),
                Self::NowDiagnostics(diagnostics) => diagnostics.size(),
                Self::RailStatus(status) => status.size(),
                Self::RailEvents(events) => events.size(),
                Self::RailLaunch(launch) => launch.size(),
                Self::ClipboardText(text) => opt_string_size(text.as_deref()),
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
                let png = read_bytes(src)?;
                Ok(Self::Screenshot { width, height, png })
            }
            5 => Ok(Self::NowCapabilities(NowCapabilities::decode(src)?)),
            6 => Ok(Self::NowOperation(OperationInfo::decode(src)?)),
            7 => {
                ensure_size!(in: src, size: 4);
                let count = src.read_u32();
                let mut operations = Vec::new();
                for _ in 0..count {
                    operations.push(OperationInfo::decode(src)?);
                }
                Ok(Self::NowOperations(operations))
            }
            8 => Ok(Self::NowEvent(OperationEvent::decode(src)?)),
            9 => Ok(Self::NowDiagnostics(NowDiagnostics::decode(src)?)),
            10 => Ok(Self::RailStatus(RailStatusInfo::decode(src)?)),
            11 => Ok(Self::RailEvents(RailEventDump::decode(src)?)),
            12 => Ok(Self::RailLaunch(RailLaunchInfo::decode(src)?)),
            13 => Ok(Self::ClipboardText(read_opt_string(src)?)),
            _ => Err(ironrdp_core::invalid_field_err!("payload", "unknown tag", in: src)),
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
                message.encode(dst)
            }
        }
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::Response"
    }

    fn size(&self) -> usize {
        1 /* tag */
            + match self {
                Self::Ok(payload) => payload.size(),
                Self::Err(error) => error.size(),
            }
    }
}

impl Decode<'_> for Response {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => Ok(Self::Ok(Payload::decode(src)?)),
            1 => Ok(Self::Err(AgentError::decode(src)?)),
            _ => Err(ironrdp_core::invalid_field_err!("response", "unknown tag", in: src)),
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
            Self::UnicodeText { text } => {
                dst.write_u8(21);
                write_string(dst, text)?;
            }
            Self::Resize { width, height } => {
                dst.write_u8(11);
                dst.write_u16(*width);
                dst.write_u16(*height);
            }
            Self::NowCapabilities => dst.write_u8(12),
            Self::NowRun { command, directory } => {
                dst.write_u8(13);
                write_string(dst, command)?;
                write_opt_string(dst, directory.as_deref())?;
            }
            Self::NowExecute(request) => {
                dst.write_u8(14);
                request.encode(dst)?;
            }
            Self::NowCancel { operation_id } => {
                dst.write_u8(15);
                dst.write_u64(*operation_id);
            }
            Self::NowList => dst.write_u8(16),
            Self::NowStatus { operation_id } => {
                dst.write_u8(17);
                dst.write_u64(*operation_id);
            }
            Self::NowAttach {
                operation_id,
                after_sequence,
            } => {
                dst.write_u8(18);
                dst.write_u64(*operation_id);
                write_opt_u64(dst, *after_sequence)?;
            }
            Self::NowStdin {
                operation_id,
                data,
                last,
            } => {
                dst.write_u8(19);
                dst.write_u64(*operation_id);
                write_bytes(dst, data)?;
                write_bool(dst, *last)?;
            }
            Self::NowDiagnostics => dst.write_u8(20),
            Self::Touch { encode_time, frames } => {
                // Tag 26: free after RAIL tags 22-25 on master.
                dst.write_u8(26);
                dst.write_u32(*encode_time);
                let frame_count: u16 = cast_length!("touch frame count", frames.len())?;
                dst.write_u16(frame_count);
                for frame in frames {
                    dst.write_u64(frame.frame_offset);
                    let contact_count: u16 = cast_length!("touch contact count", frame.contacts.len())?;
                    dst.write_u16(contact_count);
                    for contact in &frame.contacts {
                        dst.write_u8(contact.contact_id);
                        dst.write_i32(contact.x);
                        dst.write_i32(contact.y);
                        dst.write_u16(contact.flags);
                    }
                }
            }
            Self::RailStatus => dst.write_u8(22),
            Self::RailEvents { after_sequence } => {
                dst.write_u8(23);
                write_opt_u64(dst, *after_sequence)?;
            }
            Self::RailExecute(request) => {
                dst.write_u8(24);
                request.encode(dst)?;
            }
            Self::RailWait {
                after_sequence,
                timeout_ms,
            } => {
                dst.write_u8(25);
                write_opt_u64(dst, *after_sequence)?;
                dst.write_u32(*timeout_ms);
            }
            Self::Pen { encode_time, frames } => {
                // Tag 27: after Touch (26) and RAIL (22-25).
                dst.write_u8(27);
                dst.write_u32(*encode_time);
                let frame_count: u16 = cast_length!("pen frame count", frames.len())?;
                dst.write_u16(frame_count);
                for frame in frames {
                    dst.write_u64(frame.frame_offset);
                    let contact_count: u16 = cast_length!("pen contact count", frame.contacts.len())?;
                    dst.write_u16(contact_count);
                    for contact in &frame.contacts {
                        write_pen_contact(dst, contact)?;
                    }
                }
            }
            Self::DismissHoveringTouchContact { contact_id } => {
                // Tag 28: dismiss hovering touch contact.
                dst.write_u8(28);
                dst.write_u8(*contact_id);
            }
            // Tags 29-30: free after DismissHoveringTouchContact (28).
            Self::ClipboardGet => dst.write_u8(29),
            Self::ClipboardSet { text } => {
                dst.write_u8(30);
                write_string(dst, text)?;
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::Request"
    }

    fn size(&self) -> usize {
        1 /* tag */
            + match self {
                Self::Connect { properties, log_directive } => {
                    propertyset::size(properties) + opt_string_size(log_directive.as_deref())
                }
                Self::Disconnect
                | Self::Status
                | Self::Screenshot
                | Self::NowCapabilities
                | Self::NowList
                | Self::NowDiagnostics
                | Self::RailStatus
                | Self::ClipboardGet => 0,
                Self::QueryProps { filter } => 1 /* presence */ + filter.as_ref().map_or(0, Encode::size),
                Self::QueryLogs { substring, last } => {
                    opt_string_size(substring.as_deref()) + 1 /* presence */ + last.map_or(0, |_| 4)
                }
                Self::MouseMove { .. } => 2 /* x */ + 2 /* y */,
                Self::MouseButton { .. } => 1 /* button */ + 1 /* pressed */,
                Self::Wheel { .. } => 2 /* delta */ + 1 /* horizontal */,
                Self::KeyScancode { .. } => 2 /* scancode */ + 1 /* pressed */,
                Self::KeyUnicode { .. } => 4 /* ch */ + 1 /* pressed */,
                Self::Touch { frames, .. } => {
                    4 /* encode_time */ + 2 /* frame_count */
                        + frames.iter().map(|frame| {
                            8 /* frame_offset */ + 2 /* contact_count */
                                + frame.contacts.len()
                                    * (1 /* contact_id */ + 4 /* x */ + 4 /* y */ + 2 /* flags */)
                        }).sum::<usize>()
                }
                Self::UnicodeText { text } => string_size(text),
                Self::Resize { .. } => 2 /* width */ + 2 /* height */,
                Self::NowRun { command, directory } => string_size(command) + opt_string_size(directory.as_deref()),
                Self::NowExecute(request) => request.size(),
                Self::NowCancel { .. } | Self::NowStatus { .. } => 8,
                Self::NowAttach { after_sequence, .. } => 8 /* operation_id */ + opt_u64_size(*after_sequence),
                Self::NowStdin { data, .. } => 8 /* operation_id */ + bytes_size(data) + 1 /* last */,
                Self::RailEvents { after_sequence } => opt_u64_size(*after_sequence),
                Self::RailExecute(request) => request.size(),
                Self::RailWait { after_sequence, .. } => opt_u64_size(*after_sequence) + 4 /* timeout_ms */,
                Self::Pen { frames, .. } => {
                    4 /* encode_time */ + 2 /* frame_count */
                        + frames
                            .iter()
                            .map(|frame| {
                                8 /* frame_offset */ + 2 /* contact_count */
                                    + frame.contacts.iter().map(pen_contact_size).sum::<usize>()
                            })
                            .sum::<usize>()
                }
                Self::DismissHoveringTouchContact { .. } => 1 /* contact_id */,
                Self::ClipboardSet { text } => string_size(text),
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
                    _ => return Err(ironrdp_core::invalid_field_err!("dump filter", "invalid presence flag", in: src)),
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
                    _ => return Err(ironrdp_core::invalid_field_err!("query last", "invalid presence flag", in: src)),
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
            21 => Ok(Self::UnicodeText {
                text: read_string(src)?,
            }),
            11 => {
                ensure_size!(in: src, size: 4);
                let width = src.read_u16();
                let height = src.read_u16();
                Ok(Self::Resize { width, height })
            }
            12 => Ok(Self::NowCapabilities),
            13 => Ok(Self::NowRun {
                command: read_string(src)?,
                directory: read_opt_string(src)?,
            }),
            14 => Ok(Self::NowExecute(NowExecutionRequest::decode(src)?)),
            15 => {
                ensure_size!(in: src, size: 8);
                Ok(Self::NowCancel {
                    operation_id: src.read_u64(),
                })
            }
            16 => Ok(Self::NowList),
            17 => {
                ensure_size!(in: src, size: 8);
                Ok(Self::NowStatus {
                    operation_id: src.read_u64(),
                })
            }
            18 => {
                ensure_size!(in: src, size: 8);
                let operation_id = src.read_u64();
                let after_sequence = read_opt_u64(src)?;
                Ok(Self::NowAttach {
                    operation_id,
                    after_sequence,
                })
            }
            19 => {
                ensure_size!(in: src, size: 8);
                let operation_id = src.read_u64();
                let data = read_bytes(src)?;
                let last = read_bool(src)?;
                Ok(Self::NowStdin {
                    operation_id,
                    data,
                    last,
                })
            }
            20 => Ok(Self::NowDiagnostics),
            26 => {
                ensure_size!(in: src, size: 6);
                let encode_time = src.read_u32();
                let frame_count = usize::from(src.read_u16());
                if frame_count > MAX_TOUCH_FRAMES {
                    return Err(ironrdp_core::invalid_field_err!("touch frames", "too many frames"));
                }
                let mut frames = Vec::with_capacity(frame_count);
                for _ in 0..frame_count {
                    ensure_size!(in: src, size: 10);
                    let frame_offset = src.read_u64();
                    let contact_count = usize::from(src.read_u16());
                    if contact_count > MAX_TOUCH_CONTACTS {
                        return Err(ironrdp_core::invalid_field_err!("touch contacts", "too many contacts"));
                    }
                    let mut contacts = Vec::with_capacity(contact_count);
                    for _ in 0..contact_count {
                        ensure_size!(in: src, size: 11);
                        contacts.push(TouchContactRequest {
                            contact_id: src.read_u8(),
                            x: src.read_i32(),
                            y: src.read_i32(),
                            flags: src.read_u16(),
                        });
                    }
                    frames.push(TouchFrameRequest { frame_offset, contacts });
                }
                Ok(Self::Touch { encode_time, frames })
            }
            27 => {
                ensure_size!(in: src, size: 6);
                let encode_time = src.read_u32();
                let frame_count = usize::from(src.read_u16());
                if frame_count > MAX_PEN_FRAMES {
                    return Err(ironrdp_core::invalid_field_err!("pen frames", "too many frames"));
                }
                let mut frames = Vec::with_capacity(frame_count);
                for _ in 0..frame_count {
                    ensure_size!(in: src, size: 10);
                    let frame_offset = src.read_u64();
                    let contact_count = usize::from(src.read_u16());
                    if contact_count > MAX_PEN_CONTACTS {
                        return Err(ironrdp_core::invalid_field_err!("pen contacts", "too many contacts"));
                    }
                    let mut contacts = Vec::with_capacity(contact_count);
                    for _ in 0..contact_count {
                        contacts.push(read_pen_contact(src)?);
                    }
                    frames.push(PenFrameRequest { frame_offset, contacts });
                }
                Ok(Self::Pen { encode_time, frames })
            }
            28 => {
                ensure_size!(in: src, size: 1);
                Ok(Self::DismissHoveringTouchContact {
                    contact_id: src.read_u8(),
                })
            }
            22 => Ok(Self::RailStatus),
            23 => Ok(Self::RailEvents {
                after_sequence: read_opt_u64(src)?,
            }),
            24 => Ok(Self::RailExecute(RailExecuteRequest::decode(src)?)),
            25 => {
                let after_sequence = read_opt_u64(src)?;
                ensure_size!(in: src, size: 4);
                Ok(Self::RailWait {
                    after_sequence,
                    timeout_ms: src.read_u32(),
                })
            }
            29 => Ok(Self::ClipboardGet),
            30 => Ok(Self::ClipboardSet {
                text: read_string(src)?,
            }),
            _ => Err(ironrdp_core::invalid_field_err!("request", "unknown tag", in: src)),
        }
    }
}

impl_pdu_pod!(Request);

// ── NOW IPC codec ──────────────────────────────────────────────────────────

fn write_error_category(dst: &mut WriteCursor<'_>, category: AgentErrorCategory) {
    dst.write_u8(match category {
        AgentErrorCategory::InvalidRequest => 0,
        AgentErrorCategory::Unavailable => 1,
        AgentErrorCategory::Conflict => 2,
        AgentErrorCategory::Transport => 3,
        AgentErrorCategory::Remote => 4,
        AgentErrorCategory::Internal => 5,
    });
}

fn read_error_category(src: &mut ReadCursor<'_>) -> DecodeResult<AgentErrorCategory> {
    ensure_size!(in: src, size: 1);
    match src.read_u8() {
        0 => Ok(AgentErrorCategory::InvalidRequest),
        1 => Ok(AgentErrorCategory::Unavailable),
        2 => Ok(AgentErrorCategory::Conflict),
        3 => Ok(AgentErrorCategory::Transport),
        4 => Ok(AgentErrorCategory::Remote),
        5 => Ok(AgentErrorCategory::Internal),
        _ => Err(ironrdp_core::invalid_field_err!("agent error category", "unknown tag", in: src)),
    }
}

impl Encode for AgentError {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        write_error_category(dst, self.category);
        write_string(dst, &self.message)
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::AgentError"
    }

    fn size(&self) -> usize {
        1 /* category */ + string_size(&self.message)
    }
}

impl Decode<'_> for AgentError {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        Ok(Self {
            category: read_error_category(src)?,
            message: read_string(src)?,
        })
    }
}

impl_pdu_pod!(AgentError);

fn write_execution_kind(dst: &mut WriteCursor<'_>, kind: NowExecutionKind) {
    dst.write_u8(match kind {
        NowExecutionKind::Process => 0,
        NowExecutionKind::Batch => 1,
        NowExecutionKind::PowerShell => 2,
        NowExecutionKind::Pwsh => 3,
    });
}

fn read_execution_kind(src: &mut ReadCursor<'_>) -> DecodeResult<NowExecutionKind> {
    ensure_size!(in: src, size: 1);
    match src.read_u8() {
        0 => Ok(NowExecutionKind::Process),
        1 => Ok(NowExecutionKind::Batch),
        2 => Ok(NowExecutionKind::PowerShell),
        3 => Ok(NowExecutionKind::Pwsh),
        _ => Err(ironrdp_core::invalid_field_err!("NOW execution kind", "unknown tag", in: src)),
    }
}

impl Encode for NowExecutionRequest {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        write_execution_kind(dst, self.kind);
        write_string(dst, &self.command)?;
        write_opt_string(dst, self.parameters.as_deref())?;
        write_opt_string(dst, self.directory.as_deref())?;
        match &self.stdin {
            Some(data) => {
                dst.write_u8(1);
                write_bytes(dst, data)?;
            }
            None => dst.write_u8(0),
        }
        write_opt_u64(dst, self.timeout_ms)?;
        write_bool(dst, self.detached)?;
        write_bool(dst, self.no_profile)?;
        write_bool(dst, self.non_interactive)
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::NowExecutionRequest"
    }

    fn size(&self) -> usize {
        1 /* kind */
            + string_size(&self.command)
            + opt_string_size(self.parameters.as_deref())
            + opt_string_size(self.directory.as_deref())
            + 1 /* stdin presence */
            + self.stdin.as_ref().map_or(0, |data| bytes_size(data))
            + opt_u64_size(self.timeout_ms)
            + 1 /* detached */
            + 1 /* no_profile */
            + 1 /* non_interactive */
    }
}

impl Decode<'_> for NowExecutionRequest {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let kind = read_execution_kind(src)?;
        let command = read_string(src)?;
        let parameters = read_opt_string(src)?;
        let directory = read_opt_string(src)?;
        ensure_size!(in: src, size: 1);
        let stdin = match src.read_u8() {
            0 => None,
            1 => Some(read_bytes(src)?),
            _ => return Err(ironrdp_core::invalid_field_err!("NOW stdin", "invalid presence flag", in: src)),
        };
        Ok(Self {
            kind,
            command,
            parameters,
            directory,
            stdin,
            timeout_ms: read_opt_u64(src)?,
            detached: read_bool(src)?,
            no_profile: read_bool(src)?,
            non_interactive: read_bool(src)?,
        })
    }
}

impl_pdu_pod!(NowExecutionRequest);

fn write_operation_state(dst: &mut WriteCursor<'_>, state: OperationState) {
    dst.write_u8(match state {
        OperationState::Running => 0,
        OperationState::Cancelling => 1,
        OperationState::Completed => 2,
        OperationState::Cancelled => 3,
        OperationState::Failed => 4,
        OperationState::Detached => 5,
    });
}

fn read_operation_state(src: &mut ReadCursor<'_>) -> DecodeResult<OperationState> {
    ensure_size!(in: src, size: 1);
    match src.read_u8() {
        0 => Ok(OperationState::Running),
        1 => Ok(OperationState::Cancelling),
        2 => Ok(OperationState::Completed),
        3 => Ok(OperationState::Cancelled),
        4 => Ok(OperationState::Failed),
        5 => Ok(OperationState::Detached),
        _ => Err(ironrdp_core::invalid_field_err!("operation state", "unknown tag", in: src)),
    }
}

impl Encode for OperationInfo {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u64(self.id);
        write_execution_kind(dst, self.kind);
        write_operation_state(dst, self.state);
        write_bool(dst, self.detached)?;
        match self.exit_code {
            Some(exit_code) => {
                dst.write_u8(1);
                dst.write_u32(exit_code);
            }
            None => dst.write_u8(0),
        }
        match &self.error {
            Some(error) => {
                dst.write_u8(1);
                error.encode(dst)?;
            }
            None => dst.write_u8(0),
        }
        dst.write_u64(self.retained_output_bytes);
        dst.write_u64(self.next_sequence);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::OperationInfo"
    }

    fn size(&self) -> usize {
        8 /* id */
            + 1 /* kind */
            + 1 /* state */
            + 1 /* detached */
            + 1 /* exit-code presence */
            + self.exit_code.map_or(0, |_| 4)
            + 1 /* error presence */
            + self.error.as_ref().map_or(0, Encode::size)
            + 8 /* retained_output_bytes */
            + 8 /* next_sequence */
    }
}

impl Decode<'_> for OperationInfo {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 8);
        let id = src.read_u64();
        let kind = read_execution_kind(src)?;
        let state = read_operation_state(src)?;
        let detached = read_bool(src)?;
        ensure_size!(in: src, size: 1);
        let exit_code = match src.read_u8() {
            0 => None,
            1 => {
                ensure_size!(in: src, size: 4);
                Some(src.read_u32())
            }
            _ => return Err(ironrdp_core::invalid_field_err!("exit code", "invalid presence flag", in: src)),
        };
        ensure_size!(in: src, size: 1);
        let error = match src.read_u8() {
            0 => None,
            1 => Some(AgentError::decode(src)?),
            _ => {
                return Err(ironrdp_core::invalid_field_err!(
                    "operation error",
                    "invalid presence flag",
                    in: src
                ));
            }
        };
        ensure_size!(in: src, size: 16);
        Ok(Self {
            id,
            kind,
            state,
            detached,
            exit_code,
            error,
            retained_output_bytes: src.read_u64(),
            next_sequence: src.read_u64(),
        })
    }
}

impl_pdu_pod!(OperationInfo);

fn write_stream(dst: &mut WriteCursor<'_>, stream: NowStream) {
    dst.write_u8(match stream {
        NowStream::Stdout => 0,
        NowStream::Stderr => 1,
    });
}

fn read_stream(src: &mut ReadCursor<'_>) -> DecodeResult<NowStream> {
    ensure_size!(in: src, size: 1);
    match src.read_u8() {
        0 => Ok(NowStream::Stdout),
        1 => Ok(NowStream::Stderr),
        _ => Err(ironrdp_core::invalid_field_err!("NOW output stream", "unknown tag", in: src)),
    }
}

impl Encode for OperationEventKind {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        match self {
            Self::Started => dst.write_u8(0),
            Self::Output { stream, data, last } => {
                dst.write_u8(1);
                write_stream(dst, *stream);
                write_bytes(dst, data)?;
                write_bool(dst, *last)?;
            }
            Self::CancelAccepted => dst.write_u8(2),
            Self::Completed { exit_code } => {
                dst.write_u8(3);
                dst.write_u32(*exit_code);
            }
            Self::Cancelled => dst.write_u8(4),
            Self::Failed(error) => {
                dst.write_u8(5);
                error.encode(dst)?;
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::OperationEventKind"
    }

    fn size(&self) -> usize {
        1 /* tag */
            + match self {
                Self::Started | Self::CancelAccepted | Self::Cancelled => 0,
                Self::Output { data, .. } => 1 /* stream */ + bytes_size(data) + 1 /* last */,
                Self::Completed { .. } => 4,
                Self::Failed(error) => error.size(),
            }
    }
}

impl Decode<'_> for OperationEventKind {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 1);
        match src.read_u8() {
            0 => Ok(Self::Started),
            1 => Ok(Self::Output {
                stream: read_stream(src)?,
                data: read_bytes(src)?,
                last: read_bool(src)?,
            }),
            2 => Ok(Self::CancelAccepted),
            3 => {
                ensure_size!(in: src, size: 4);
                Ok(Self::Completed {
                    exit_code: src.read_u32(),
                })
            }
            4 => Ok(Self::Cancelled),
            5 => Ok(Self::Failed(AgentError::decode(src)?)),
            _ => Err(ironrdp_core::invalid_field_err!("operation event", "unknown tag", in: src)),
        }
    }
}

impl_pdu_pod!(OperationEventKind);

impl Encode for OperationEvent {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u64(self.operation_id);
        dst.write_u64(self.sequence);
        self.kind.encode(dst)
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::OperationEvent"
    }

    fn size(&self) -> usize {
        8 /* operation_id */ + 8 /* sequence */ + self.kind.size()
    }
}

impl Decode<'_> for OperationEvent {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 16);
        Ok(Self {
            operation_id: src.read_u64(),
            sequence: src.read_u64(),
            kind: OperationEventKind::decode(src)?,
        })
    }
}

impl_pdu_pod!(OperationEvent);

#[cfg(test)]
mod tests {
    use ironrdp_core::{decode, encode_vec};

    use super::{MAX_RAIL_EVENT_DUMP_EVENTS, RailEvent, RailEventDump, RailEventKind, RailExecuteRequest};

    #[test]
    fn rail_execute_debug_redacts_command_fields() {
        let request = RailExecuteRequest {
            executable: "secret-program.exe".to_owned(),
            working_directory: "C:\\secret-directory".to_owned(),
            arguments: "--token secret-token".to_owned(),
            flags: 0,
        };

        let debug = format!("{request:?}");
        assert!(!debug.contains("secret-program.exe"));
        assert!(!debug.contains("C:\\secret-directory"));
        assert!(!debug.contains("secret-token"));
        assert!(debug.contains("executable_len"));
    }

    #[test]
    fn rail_event_dump_rejects_oversized_counts() {
        let event = RailEvent {
            sequence: 1,
            kind: RailEventKind::Gap { lost_through: 1 },
        };
        let accepted = RailEventDump {
            generation: 1,
            events: vec![event.clone(); MAX_RAIL_EVENT_DUMP_EVENTS],
        };
        let encoded = encode_vec(&accepted).expect("encode the event limit");
        assert_eq!(
            decode::<RailEventDump>(&encoded).expect("decode the event limit"),
            accepted
        );

        let dump = RailEventDump {
            generation: 1,
            events: vec![event; MAX_RAIL_EVENT_DUMP_EVENTS + 1],
        };
        assert!(encode_vec(&dump).is_err());

        let mut bytes = [0; 12];
        bytes[8..]
            .copy_from_slice(&(u32::try_from(MAX_RAIL_EVENT_DUMP_EVENTS + 1).expect("limit fits u32")).to_le_bytes());
        assert!(decode::<RailEventDump>(&bytes).is_err());
    }
}

impl Encode for NowCapabilities {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u16(self.version_major);
        dst.write_u16(self.version_minor);
        write_opt_u64(dst, self.heartbeat_ms)?;
        for value in [
            self.run,
            self.process,
            self.batch,
            self.powershell,
            self.pwsh,
            self.io_redirection,
            self.unicode_console,
        ] {
            write_bool(dst, value)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::NowCapabilities"
    }

    fn size(&self) -> usize {
        2 /* version major */
            + 2 /* version minor */
            + opt_u64_size(self.heartbeat_ms)
            + 7 /* feature flags */
    }
}

impl Decode<'_> for NowCapabilities {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: 4);
        let version_major = src.read_u16();
        let version_minor = src.read_u16();
        Ok(Self {
            version_major,
            version_minor,
            heartbeat_ms: read_opt_u64(src)?,
            run: read_bool(src)?,
            process: read_bool(src)?,
            batch: read_bool(src)?,
            powershell: read_bool(src)?,
            pwsh: read_bool(src)?,
            io_redirection: read_bool(src)?,
            unicode_console: read_bool(src)?,
        })
    }
}

impl_pdu_pod!(NowCapabilities);

impl Encode for NowDiagnostics {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        write_bool(dst, self.endpoint_allocated)?;
        write_bool(dst, self.connected)?;
        match &self.capabilities {
            Some(capabilities) => {
                dst.write_u8(1);
                capabilities.encode(dst)?;
            }
            None => dst.write_u8(0),
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ironrdp_rpc::ipc::NowDiagnostics"
    }

    fn size(&self) -> usize {
        1 /* endpoint_allocated */
            + 1 /* connected */
            + 1 /* capabilities presence */
            + self.capabilities.as_ref().map_or(0, Encode::size)
    }
}

impl Decode<'_> for NowDiagnostics {
    fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        let endpoint_allocated = read_bool(src)?;
        let connected = read_bool(src)?;
        ensure_size!(in: src, size: 1);
        let capabilities = match src.read_u8() {
            0 => None,
            1 => Some(NowCapabilities::decode(src)?),
            _ => {
                return Err(ironrdp_core::invalid_field_err!(
                    "NOW capabilities",
                    "invalid presence flag",
                    in: src
                ));
            }
        };
        Ok(Self {
            endpoint_allocated,
            connected,
            capabilities,
        })
    }
}

impl_pdu_pod!(NowDiagnostics);

#[cfg(test)]
mod rdpei_request_tests {
    use super::{
        MAX_PEN_CONTACTS, MAX_PEN_FRAMES, MAX_PEN_PRESSURE, MAX_PEN_ROTATION, MAX_PEN_TILT, MAX_TOUCH_CONTACTS,
        MAX_TOUCH_FRAMES, PenContactRequest, PenFrameRequest, TouchContactRequest, TouchFrameRequest,
        pen_event_from_request, touch_event_from_request,
    };
    use ironrdp_rdpei::pdu::{EightByteUnsigned, FourByteSigned, FourByteUnsigned, PenContactFlags, TouchContactFlags};

    fn contact(flags: u16) -> TouchContactRequest {
        TouchContactRequest {
            contact_id: 1,
            x: 10,
            y: 20,
            flags,
        }
    }

    fn contact_flags(flags: TouchContactFlags) -> u16 {
        u16::try_from(flags.bits()).expect("touch contact flags fit in u16")
    }

    fn pen_contact(flags: u16) -> PenContactRequest {
        PenContactRequest {
            device_id: 0,
            x: 10,
            y: 20,
            flags,
            pressure: None,
            rotation: None,
            tilt_x: None,
            tilt_y: None,
            pen_flags: None,
        }
    }

    fn pen_contact_flags(flags: PenContactFlags) -> u16 {
        u16::try_from(flags.bits()).expect("pen contact flags fit in u16")
    }

    #[test]
    fn touch_event_accepts_legal_down_and_up() {
        let down = contact_flags(TouchContactFlags::DOWN | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT);
        let up = contact_flags(TouchContactFlags::UP);
        let event = touch_event_from_request(
            0,
            vec![
                TouchFrameRequest {
                    frame_offset: 0,
                    contacts: vec![contact(down)],
                },
                TouchFrameRequest {
                    frame_offset: 16_000,
                    contacts: vec![contact(up)],
                },
            ],
        )
        .expect("legal touch tap frames");
        assert_eq!(event.frames.len(), 2);
        assert_eq!(event.frames[1].frame_offset, 16_000);
    }

    #[test]
    fn touch_event_rejects_empty_frames_and_contacts() {
        assert!(touch_event_from_request(0, Vec::new()).is_err());
        assert!(
            touch_event_from_request(
                0,
                vec![TouchFrameRequest {
                    frame_offset: 0,
                    contacts: Vec::new(),
                }],
            )
            .is_err()
        );
    }

    #[test]
    fn touch_event_rejects_illegal_and_unknown_flags() {
        assert!(
            touch_event_from_request(
                0,
                vec![TouchFrameRequest {
                    frame_offset: 0,
                    contacts: vec![contact(0x0008)], // INRANGE alone
                }],
            )
            .is_err()
        );
        assert!(
            touch_event_from_request(
                0,
                vec![TouchFrameRequest {
                    frame_offset: 0,
                    contacts: vec![contact(0x0040)], // unknown bit
                }],
            )
            .is_err()
        );
        assert!(
            touch_event_from_request(
                0,
                vec![TouchFrameRequest {
                    frame_offset: 0,
                    contacts: vec![contact(0x0001)], // DOWN alone
                }],
            )
            .is_err()
        );
    }

    #[test]
    fn touch_event_rejects_count_and_range_limits() {
        let legal = contact_flags(TouchContactFlags::DOWN | TouchContactFlags::INRANGE | TouchContactFlags::INCONTACT);
        let max_contacts = u8::try_from(MAX_TOUCH_CONTACTS).expect("MAX_TOUCH_CONTACTS fits u8");
        let too_many_contacts = (0..=max_contacts)
            .map(|contact_id| TouchContactRequest {
                contact_id,
                x: 0,
                y: 0,
                flags: legal,
            })
            .collect::<Vec<_>>();
        assert!(
            touch_event_from_request(
                0,
                vec![TouchFrameRequest {
                    frame_offset: 0,
                    contacts: too_many_contacts,
                }],
            )
            .is_err()
        );

        let too_many_frames = core::iter::repeat_with(|| TouchFrameRequest {
            frame_offset: 0,
            contacts: vec![contact(legal)],
        })
        .take(MAX_TOUCH_FRAMES + 1)
        .collect::<Vec<_>>();
        assert!(touch_event_from_request(0, too_many_frames).is_err());

        assert!(
            touch_event_from_request(
                FourByteUnsigned::MAX + 1,
                vec![TouchFrameRequest {
                    frame_offset: 0,
                    contacts: vec![contact(legal)],
                }],
            )
            .is_err()
        );
        assert!(
            touch_event_from_request(
                0,
                vec![TouchFrameRequest {
                    frame_offset: EightByteUnsigned::MAX + 1,
                    contacts: vec![contact(legal)],
                }],
            )
            .is_err()
        );
        assert!(
            touch_event_from_request(
                0,
                vec![TouchFrameRequest {
                    frame_offset: 0,
                    contacts: vec![TouchContactRequest {
                        contact_id: 0,
                        x: FourByteSigned::MAX + 1,
                        y: 0,
                        flags: legal,
                    }],
                }],
            )
            .is_err()
        );
    }

    #[test]
    fn pen_event_accepts_legal_down_and_up() {
        let down = pen_contact_flags(PenContactFlags::DOWN | PenContactFlags::INRANGE | PenContactFlags::INCONTACT);
        let up = pen_contact_flags(PenContactFlags::UP);
        let mut down_contact = pen_contact(down);
        down_contact.pressure = Some(MAX_PEN_PRESSURE);
        down_contact.rotation = Some(MAX_PEN_ROTATION);
        down_contact.tilt_x = Some(-MAX_PEN_TILT);
        down_contact.tilt_y = Some(MAX_PEN_TILT);
        let event = pen_event_from_request(
            0,
            vec![
                PenFrameRequest {
                    frame_offset: 0,
                    contacts: vec![down_contact],
                },
                PenFrameRequest {
                    frame_offset: 16_000,
                    contacts: vec![pen_contact(up)],
                },
            ],
        )
        .expect("legal pen tap frames");
        assert_eq!(event.frames.len(), 2);
        assert_eq!(event.frames[0].frame_offset, 0);
        assert_eq!(event.frames[1].frame_offset, 16_000);
    }

    #[test]
    fn pen_event_rejects_empty_frames_and_contacts() {
        assert!(pen_event_from_request(0, Vec::new()).is_err());
        assert!(
            pen_event_from_request(
                0,
                vec![PenFrameRequest {
                    frame_offset: 0,
                    contacts: Vec::new(),
                }],
            )
            .is_err()
        );
    }

    #[test]
    fn pen_event_rejects_illegal_unknown_flags_and_device_id() {
        assert!(
            pen_event_from_request(
                0,
                vec![PenFrameRequest {
                    frame_offset: 0,
                    contacts: vec![pen_contact(0x0008)], // INRANGE alone
                }],
            )
            .is_err()
        );
        assert!(
            pen_event_from_request(
                0,
                vec![PenFrameRequest {
                    frame_offset: 0,
                    contacts: vec![pen_contact(0x0040)], // unknown bit
                }],
            )
            .is_err()
        );
        let mut nonzero_device = pen_contact(pen_contact_flags(
            PenContactFlags::DOWN | PenContactFlags::INRANGE | PenContactFlags::INCONTACT,
        ));
        nonzero_device.device_id = 1;
        assert!(
            pen_event_from_request(
                0,
                vec![PenFrameRequest {
                    frame_offset: 0,
                    contacts: vec![nonzero_device],
                }],
            )
            .is_err()
        );
    }

    #[test]
    fn pen_event_rejects_count_offset_and_field_ranges() {
        let legal = pen_contact_flags(PenContactFlags::DOWN | PenContactFlags::INRANGE | PenContactFlags::INCONTACT);
        assert_eq!(MAX_PEN_CONTACTS, 1);
        assert!(
            pen_event_from_request(
                0,
                vec![PenFrameRequest {
                    frame_offset: 0,
                    contacts: vec![pen_contact(legal), pen_contact(legal)],
                }],
            )
            .is_err()
        );

        let too_many_frames = core::iter::repeat_with(|| PenFrameRequest {
            frame_offset: 0,
            contacts: vec![pen_contact(legal)],
        })
        .take(MAX_PEN_FRAMES + 1)
        .collect::<Vec<_>>();
        assert!(pen_event_from_request(0, too_many_frames).is_err());

        assert!(
            pen_event_from_request(
                0,
                vec![PenFrameRequest {
                    frame_offset: 1,
                    contacts: vec![pen_contact(legal)],
                }],
            )
            .is_err()
        );

        let mut pressure = pen_contact(legal);
        pressure.pressure = Some(MAX_PEN_PRESSURE + 1);
        assert!(
            pen_event_from_request(
                0,
                vec![PenFrameRequest {
                    frame_offset: 0,
                    contacts: vec![pressure],
                }],
            )
            .is_err()
        );

        let mut rotation = pen_contact(legal);
        rotation.rotation = Some(MAX_PEN_ROTATION + 1);
        assert!(
            pen_event_from_request(
                0,
                vec![PenFrameRequest {
                    frame_offset: 0,
                    contacts: vec![rotation],
                }],
            )
            .is_err()
        );

        let mut tilt_x = pen_contact(legal);
        tilt_x.tilt_x = Some(MAX_PEN_TILT + 1);
        assert!(
            pen_event_from_request(
                0,
                vec![PenFrameRequest {
                    frame_offset: 0,
                    contacts: vec![tilt_x],
                }],
            )
            .is_err()
        );

        let mut tilt_y = pen_contact(legal);
        tilt_y.tilt_y = Some(-MAX_PEN_TILT - 1);
        assert!(
            pen_event_from_request(
                0,
                vec![PenFrameRequest {
                    frame_offset: 0,
                    contacts: vec![tilt_y],
                }],
            )
            .is_err()
        );

        assert!(
            pen_event_from_request(
                FourByteUnsigned::MAX + 1,
                vec![PenFrameRequest {
                    frame_offset: 0,
                    contacts: vec![pen_contact(legal)],
                }],
            )
            .is_err()
        );
        assert!(
            pen_event_from_request(
                0,
                vec![
                    PenFrameRequest {
                        frame_offset: 0,
                        contacts: vec![pen_contact(legal)],
                    },
                    PenFrameRequest {
                        frame_offset: EightByteUnsigned::MAX + 1,
                        contacts: vec![pen_contact(legal)],
                    },
                ],
            )
            .is_err()
        );
        assert!(
            pen_event_from_request(
                0,
                vec![PenFrameRequest {
                    frame_offset: 0,
                    contacts: vec![PenContactRequest {
                        device_id: 0,
                        x: FourByteSigned::MAX + 1,
                        y: 0,
                        flags: legal,
                        pressure: None,
                        rotation: None,
                        tilt_x: None,
                        tilt_y: None,
                        pen_flags: None,
                    }],
                }],
            )
            .is_err()
        );
    }
}
