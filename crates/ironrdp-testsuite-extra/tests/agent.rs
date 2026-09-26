//! Codec round-trip tests for the shared RPC protocol and daemon NOW endpoint.
//!
//! These exercise the reusable protocol and daemon support. They live here, in the shared test
//! suite, rather than inside the owning crates themselves, per
//! the workspace convention of keeping unit tests for protocol codecs in `ironrdp-testsuite-extra`.

use core::fmt::Debug;

use ironrdp_core::{Decode, DecodeOwned, Encode, decode, decode_owned, encode_vec};
use ironrdp_daemon::now::{DVC_CHANNEL_NAME, INITIAL_ENDPOINT_TIMEOUT, NowEndpoint, RECONNECT_ENDPOINT_TIMEOUT};
use ironrdp_input::MouseButton;
use ironrdp_propertyset::PropertySet;
use ironrdp_rpc::ipc::{
    AgentError, AgentErrorCategory, ConnState, KeyFilter, NowCapabilities, NowDiagnostics, NowExecutionKind,
    NowExecutionRequest, NowStream, OperationEvent, OperationEventKind, OperationInfo, OperationState, Payload,
    PropValue, PropertyDump, PropertyEntry, RailEvent, RailEventDump, RailEventKind, RailExecuteFailureReason,
    RailExecuteRequest, RailLaunchInfo, RailStatusInfo, Request, Response, StatusInfo,
};
use ironrdp_rpc::wire;

#[track_caller]
fn round_trip<T>(value: &T)
where
    T: Encode + DecodeOwned + for<'de> Decode<'de> + PartialEq + Debug,
{
    let bytes = encode_vec(value).expect("encode");

    let decoded_owned: T = decode_owned(&bytes).expect("decode_owned");
    assert_eq!(value, &decoded_owned, "decode_owned round-trip mismatch");

    let decoded: T = decode(&bytes).expect("decode");
    assert_eq!(value, &decoded, "decode round-trip mismatch");
}

#[test]
fn request_variants_round_trip() {
    let mut props = PropertySet::new();
    props.insert("full address", "host.example:3389");
    props.insert("username", "operator");

    let mut props2 = PropertySet::new();
    props2.insert("full address", "host.example:3389");

    let requests = [
        Request::Connect {
            properties: props,
            log_directive: None,
        },
        Request::Connect {
            properties: props2,
            log_directive: Some("ironrdp_connector=trace,debug".to_owned()),
        },
        Request::Disconnect,
        Request::Status,
        Request::QueryProps { filter: None },
        Request::QueryProps {
            filter: Some(KeyFilter::Substring("addr".to_owned())),
        },
        Request::QueryProps {
            filter: Some(KeyFilter::Prefix("Full".to_owned())),
        },
        Request::QueryLogs {
            substring: Some("error".to_owned()),
            last: Some(50),
        },
        Request::QueryLogs {
            substring: None,
            last: None,
        },
        Request::Screenshot,
        Request::MouseMove { x: 640, y: 480 },
        Request::MouseButton {
            button: MouseButton::Right,
            pressed: true,
        },
        Request::Wheel {
            delta: -120,
            horizontal: false,
        },
        Request::KeyScancode {
            scancode: 0x1C,
            pressed: false,
        },
        Request::KeyUnicode {
            ch: '\u{00e9}',
            pressed: true,
        },
        Request::UnicodeText {
            text: "Hello, \u{4e16}\u{754c}".to_owned(),
        },
        Request::Touch {
            encode_time: 12,
            frames: vec![ironrdp_rpc::ipc::TouchFrameRequest {
                frame_offset: 0,
                contacts: vec![ironrdp_rpc::ipc::TouchContactRequest {
                    contact_id: 1,
                    x: 100,
                    y: 200,
                    flags: 0x0019, // DOWN | INRANGE | INCONTACT
                }],
            }],
        },
        Request::Pen {
            encode_time: 24,
            frames: vec![ironrdp_rpc::ipc::PenFrameRequest {
                frame_offset: 0,
                contacts: vec![ironrdp_rpc::ipc::PenContactRequest {
                    device_id: 0,
                    x: 300,
                    y: 400,
                    flags: 0x0019, // DOWN | INRANGE | INCONTACT
                    pressure: Some(512),
                    rotation: Some(45),
                    tilt_x: Some(10),
                    tilt_y: Some(-5),
                    pen_flags: None,
                }],
            }],
        },
        Request::DismissHoveringTouchContact { contact_id: 3 },
        Request::NowCapabilities,
        Request::NowRun {
            command: "echo secret".to_owned(),
            directory: Some("C:\\Temp".to_owned()),
        },
        Request::NowExecute(NowExecutionRequest {
            kind: NowExecutionKind::PowerShell,
            command: "$env:SECRET".to_owned(),
            parameters: None,
            directory: None,
            stdin: Some(vec![0, 0xFF]),
            timeout_ms: Some(3_000),
            detached: false,
            no_profile: true,
            non_interactive: true,
        }),
        Request::NowCancel { operation_id: 42 },
        Request::NowList,
        Request::NowStatus { operation_id: 42 },
        Request::NowAttach {
            operation_id: 42,
            after_sequence: Some(7),
        },
        Request::NowStdin {
            operation_id: 42,
            data: vec![0, 0xFF],
            last: true,
        },
        Request::NowDiagnostics,
        Request::RailStatus,
        Request::RailEvents {
            after_sequence: Some(7),
        },
        Request::RailWait {
            after_sequence: Some(7),
            timeout_ms: 1_000,
        },
        Request::RailExecute(RailExecuteRequest {
            executable: "notepad.exe".to_owned(),
            working_directory: "C:\\Temp".to_owned(),
            arguments: "audit.txt".to_owned(),
            flags: 0,
        }),
        Request::ClipboardGet,
        Request::ClipboardSet {
            text: "clipboard text".to_owned(),
        },
        Request::ClipboardGetImage,
        Request::ClipboardSetImage {
            png: vec![0x89, b'P', b'N', b'G', 0, 0xFF],
        },
        Request::ClipboardGetHtml,
        Request::ClipboardSetHtml {
            html: "<b>clipboard html</b>".to_owned(),
        },
    ];

    for request in &requests {
        round_trip(request);
    }
}

#[test]
fn response_variants_round_trip() {
    let responses = [
        Response::ok(),
        Response::error("connection refused"),
        Response::Ok(Payload::Status(StatusInfo {
            state: ConnState::NoSession,
            destination: None,
            width: None,
            height: None,
            message: None,
            credentials_loaded: true,
        })),
        Response::Ok(Payload::Status(StatusInfo {
            state: ConnState::Connected,
            destination: Some("host.example:3389".to_owned()),
            width: Some(1920),
            height: Some(1080),
            message: Some("ok".to_owned()),
            credentials_loaded: false,
        })),
        Response::Ok(Payload::Properties(PropertyDump {
            entries: vec![
                PropertyEntry {
                    key: "full address".to_owned(),
                    value: PropValue::Str("host.example:3389".to_owned()),
                },
                PropertyEntry {
                    key: "server port".to_owned(),
                    value: PropValue::Int(3389),
                },
            ],
        })),
        Response::Ok(Payload::Logs(vec!["line one".to_owned(), "line two".to_owned()])),
        Response::Ok(Payload::Screenshot {
            width: 800,
            height: 600,
            png: vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        }),
        Response::Ok(Payload::Empty),
        Response::Err(AgentError {
            category: AgentErrorCategory::Remote,
            message: "remote command failed".to_owned(),
        }),
        Response::Ok(Payload::NowCapabilities(NowCapabilities {
            version_major: 1,
            version_minor: 4,
            heartbeat_ms: Some(60_000),
            run: true,
            process: true,
            batch: true,
            powershell: true,
            pwsh: true,
            io_redirection: true,
            unicode_console: true,
        })),
        Response::Ok(Payload::NowOperation(OperationInfo {
            id: 7,
            kind: NowExecutionKind::Batch,
            state: OperationState::Completed,
            detached: false,
            exit_code: Some(17),
            error: None,
            retained_output_bytes: 3,
            next_sequence: 2,
        })),
        Response::Ok(Payload::NowOperations(vec![OperationInfo {
            id: 8,
            kind: NowExecutionKind::Process,
            state: OperationState::Failed,
            detached: false,
            exit_code: None,
            error: Some(AgentError {
                category: AgentErrorCategory::Transport,
                message: "now worker closed".to_owned(),
            }),
            retained_output_bytes: 0,
            next_sequence: 1,
        }])),
        Response::Ok(Payload::NowEvent(OperationEvent {
            operation_id: 7,
            sequence: 1,
            kind: OperationEventKind::Output {
                stream: NowStream::Stderr,
                data: vec![0, 0xFF],
                last: true,
            },
        })),
        Response::Ok(Payload::NowDiagnostics(NowDiagnostics {
            endpoint_allocated: true,
            connected: false,
            capabilities: None,
        })),
        Response::Ok(Payload::RailStatus(RailStatusInfo {
            generation: 9,
            next_sequence: 4,
            handshake_complete: true,
            desktop_synchronized: false,
            pending_launches: vec![RailLaunchInfo {
                launch_id: 3,
                executable: "notepad.exe".to_owned(),
                flags: 0,
            }],
        })),
        Response::Ok(Payload::RailEvents(RailEventDump {
            generation: 9,
            events: vec![
                RailEvent {
                    sequence: 1,
                    kind: RailEventKind::Gap { lost_through: 4 },
                },
                RailEvent {
                    sequence: 5,
                    kind: RailEventKind::ExecuteResult {
                        launch_id: Some(3),
                        executable: "notepad.exe".to_owned(),
                        flags: 0,
                        result: 0,
                        raw_result: 0,
                    },
                },
                RailEvent {
                    sequence: 6,
                    kind: RailEventKind::ExecuteFailed {
                        launch_id: Some(3),
                        executable: "notepad.exe".to_owned(),
                        flags: 0,
                        reason: RailExecuteFailureReason::QueueRejected,
                    },
                },
            ],
        })),
        Response::Ok(Payload::RailLaunch(RailLaunchInfo {
            launch_id: 3,
            executable: "notepad.exe".to_owned(),
            flags: 0,
        })),
        Response::Ok(Payload::ClipboardText(None)),
        Response::Ok(Payload::ClipboardText(Some("clipboard text".to_owned()))),
        Response::Ok(Payload::ClipboardImage(None)),
        Response::Ok(Payload::ClipboardImage(Some(vec![0x89, b'P', b'N', b'G', 0, 0xFF]))),
        Response::Ok(Payload::ClipboardHtml(None)),
        Response::Ok(Payload::ClipboardHtml(Some("<b>clipboard html</b>".to_owned()))),
    ];

    for response in &responses {
        round_trip(response);
    }
}

#[test]
fn property_set_wire_round_trips() {
    let mut original = PropertySet::new();
    original.insert("full address", "host.example:3389");
    original.insert("server port", 3389i64);
    original.insert("username", "operator");
    original.insert("screen mode id", 2i64);

    let size = wire::propertyset::size(&original);
    let mut buf = vec![0u8; size];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    wire::propertyset::write(&original, &mut cursor).expect("write");
    assert_eq!(cursor.pos(), size, "written length must match computed size");

    let mut decoded = PropertySet::new();
    let mut read_cursor = ironrdp_core::ReadCursor::new(&buf);
    wire::propertyset::read(&mut decoded, &mut read_cursor).expect("read");

    let mut original_pairs: Vec<_> = original.iter().collect();
    let mut decoded_pairs: Vec<_> = decoded.iter().collect();
    original_pairs.sort_by_key(|(key, _)| *key);
    decoded_pairs.sort_by_key(|(key, _)| *key);
    assert_eq!(original_pairs, decoded_pairs, "property set wire round-trip mismatch");
}

#[test]
fn bytes_wire_round_trips() {
    let original = vec![0x89, b'P', b'N', b'G', 0x00, 0xFF, 0x10, 0x20];

    let size = wire::bytes_size(&original);
    let mut buf = vec![0u8; size];
    let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
    wire::write_bytes(&mut cursor, &original).expect("write_bytes");
    assert_eq!(cursor.pos(), size, "written length must match computed size");

    let mut read_cursor = ironrdp_core::ReadCursor::new(&buf);
    let decoded = wire::read_bytes(&mut read_cursor).expect("read_bytes");
    assert_eq!(original, decoded, "bytes wire round-trip mismatch");
}

#[test]
fn opt_bytes_wire_round_trips() {
    for original in [None, Some(vec![0x89, b'P', b'N', b'G', 0x00, 0xFF])] {
        let size = wire::opt_bytes_size(original.as_deref());
        let mut buf = vec![0u8; size];
        let mut cursor = ironrdp_core::WriteCursor::new(&mut buf);
        wire::write_opt_bytes(&mut cursor, original.as_deref()).expect("write_opt_bytes");
        assert_eq!(cursor.pos(), size, "written length must match computed size");

        let mut read_cursor = ironrdp_core::ReadCursor::new(&buf);
        let decoded = wire::read_opt_bytes(&mut read_cursor).expect("read_opt_bytes");
        assert_eq!(original, decoded, "optional bytes wire round-trip mismatch");
    }
}

#[test]
fn clipboard_debug_redacts_content() {
    let request = Request::ClipboardSet {
        text: "secret-text".to_owned(),
    };
    let debug = format!("{request:?}");
    assert!(!debug.contains("secret-text"));

    let payload = Payload::ClipboardText(Some("secret-text".to_owned()));
    let debug = format!("{payload:?}");
    assert!(!debug.contains("secret-text"));

    let request = Request::ClipboardSetImage {
        png: b"secret-pixels".to_vec(),
    };
    let debug = format!("{request:?}");
    assert!(!debug.contains("secret-pixels"));

    let payload = Payload::ClipboardImage(Some(b"secret-pixels".to_vec()));
    let debug = format!("{payload:?}");
    assert!(!debug.contains("secret-pixels"));

    let request = Request::ClipboardSetHtml {
        html: "<b>secret-markup</b>".to_owned(),
    };
    let debug = format!("{request:?}");
    assert!(!debug.contains("secret-markup"));

    let payload = Payload::ClipboardHtml(Some("<b>secret-markup</b>".to_owned()));
    let debug = format!("{payload:?}");
    assert!(!debug.contains("secret-markup"));
}

#[test]
fn now_request_debug_redacts_command_and_stdin() {
    let request = Request::NowExecute(NowExecutionRequest {
        kind: NowExecutionKind::Batch,
        command: "secret-command".to_owned(),
        parameters: None,
        directory: None,
        stdin: Some(b"secret-stdin".to_vec()),
        timeout_ms: None,
        detached: false,
        no_profile: false,
        non_interactive: false,
    });

    let debug = format!("{request:?}");
    assert!(!debug.contains("secret-command"));
    assert!(!debug.contains("secret-stdin"));
}

#[test]
fn remote_exit_codes_follow_the_cli_contract() {
    assert_eq!(ironrdp_agent::cli::remote_exit_status(0), 0);
    assert_eq!(ironrdp_agent::cli::remote_exit_status(1), 1);
    assert_eq!(ironrdp_agent::cli::remote_exit_status(255), 255);
    assert_eq!(ironrdp_agent::cli::remote_exit_status(256), 255);
    assert_eq!(ironrdp_agent::cli::remote_exit_status(u32::MAX), 255);
}

#[test]
fn now_endpoint_is_per_session_and_uses_documented_deadlines() {
    let first = NowEndpoint::new().expect("endpoint allocation must succeed");
    let second = NowEndpoint::new().expect("endpoint allocation must succeed");

    assert_ne!(first.pipe_name(), second.pipe_name());
    assert_eq!(first.dvc_proxy_info().channel_name, DVC_CHANNEL_NAME);
    assert_eq!(INITIAL_ENDPOINT_TIMEOUT.as_secs(), 30);
    assert_eq!(RECONNECT_ENDPOINT_TIMEOUT.as_secs(), 10);
}
