//! Codec round-trip tests for the `ironrdp-agent` IPC and wire protocols.
//!
//! These exercise the crate's private wire format through its public (and `internal`-feature)
//! API. They live here, in the shared test suite, rather than inside `ironrdp-agent` itself, per
//! the workspace convention of keeping unit tests for protocol codecs in `ironrdp-testsuite-extra`.

use core::fmt::Debug;

use ironrdp_agent::ipc::{
    ConnState, KeyFilter, NowShell, NowTerminal, Payload, PropValue, PropertyDump, PropertyEntry, Request, Response,
    StatusInfo,
};
use ironrdp_agent::now::{self, NowCaps};
use ironrdp_agent::wire;
use ironrdp_core::{Decode, DecodeOwned, Encode, decode, decode_owned, encode_vec};
use ironrdp_input::MouseButton;
use ironrdp_propertyset::PropertySet;

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
        Request::NowCapabilities,
        Request::NowExec {
            shell: Some(NowShell::Pwsh),
            command: "Get-Process".to_owned(),
            profile: false,
            interactive: false,
            directory: Some("C:\\Temp".to_owned()),
            stdin: Some(vec![1, 2, 3, 4]),
            timeout_secs: Some(30),
        },
        Request::NowExec {
            shell: None,
            command: "echo hi".to_owned(),
            profile: true,
            interactive: true,
            directory: None,
            stdin: None,
            timeout_secs: None,
        },
        Request::NowProcess {
            filename: "notepad.exe".to_owned(),
            parameters: Some("C:\\file.txt".to_owned()),
            directory: None,
            stdin: None,
            timeout_secs: Some(5),
        },
        Request::NowProcess {
            filename: "cmd.exe".to_owned(),
            parameters: None,
            directory: Some("C:\\".to_owned()),
            stdin: Some(vec![0xFF, 0x00]),
            timeout_secs: None,
        },
    ];

    for request in &requests {
        round_trip(request);
    }

    for shell in [NowShell::Pwsh, NowShell::Powershell, NowShell::Batch] {
        round_trip(&shell);
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
        Response::Ok(Payload::NowCapabilities(vec![
            ("now.version".to_owned(), PropValue::Str("1.6".to_owned())),
            ("now.pwsh".to_owned(), PropValue::Int(1)),
            ("now.default_shell".to_owned(), PropValue::Str("pwsh".to_owned())),
        ])),
        Response::Ok(Payload::NowOutput {
            stdout: b"hello\n".to_vec(),
            stderr: Vec::new(),
            exit_code: 0,
            terminal: NowTerminal::Completed,
        }),
        Response::Ok(Payload::NowOutput {
            stdout: Vec::new(),
            stderr: b"boom".to_vec(),
            exit_code: 3,
            terminal: NowTerminal::Cancelled,
        }),
        Response::Ok(Payload::Empty),
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
fn now_request_debug_redacts_command_parameters_and_stdin() {
    let exec = Request::NowExec {
        shell: Some(NowShell::Pwsh),
        command: "SUPER-SECRET-COMMAND".to_owned(),
        profile: false,
        interactive: false,
        directory: None,
        stdin: Some(b"SUPER-SECRET-STDIN".to_vec()),
        timeout_secs: None,
    };
    let rendered = format!("{exec:?}");
    assert!(!rendered.contains("SUPER-SECRET-COMMAND"), "command leaked: {rendered}");
    assert!(!rendered.contains("SUPER-SECRET-STDIN"), "stdin leaked: {rendered}");

    let process = Request::NowProcess {
        filename: "notepad.exe".to_owned(),
        parameters: Some("SUPER-SECRET-PARAMS".to_owned()),
        directory: None,
        stdin: Some(b"SUPER-SECRET-STDIN".to_vec()),
        timeout_secs: None,
    };
    let rendered = format!("{process:?}");
    assert!(
        !rendered.contains("SUPER-SECRET-PARAMS"),
        "parameters leaked: {rendered}"
    );
    assert!(!rendered.contains("SUPER-SECRET-STDIN"), "stdin leaked: {rendered}");
}

#[test]
fn now_remote_exit_status_maps_codes() {
    assert_eq!(now::remote_exit_status(0), 0);
    assert_eq!(now::remote_exit_status(1), 1);
    assert_eq!(now::remote_exit_status(255), 255);
    assert_eq!(now::remote_exit_status(256), 255);
    assert_eq!(now::remote_exit_status(1000), 255);
}

fn test_caps(pwsh: bool, powershell: bool, batch: bool) -> NowCaps {
    NowCaps {
        version: (1, 6),
        heartbeat_ms: Some(60_000),
        run: true,
        process: true,
        batch,
        powershell,
        pwsh,
        io_redirection: true,
        unicode_console: true,
    }
}

#[test]
fn now_default_shell_prefers_pwsh_then_powershell_then_batch() {
    assert_eq!(now::default_shell(&test_caps(true, true, true)), Some(NowShell::Pwsh));
    assert_eq!(
        now::default_shell(&test_caps(false, true, true)),
        Some(NowShell::Powershell)
    );
    assert_eq!(
        now::default_shell(&test_caps(false, false, true)),
        Some(NowShell::Batch)
    );
    assert_eq!(now::default_shell(&test_caps(false, false, false)), None);
}

#[test]
fn now_resolve_shell_validates_and_defaults() {
    // Auto-resolution follows the preference order.
    assert_eq!(
        now::resolve_shell(&test_caps(false, true, true), None),
        Ok(NowShell::Powershell)
    );
    // An available explicit choice is accepted.
    assert_eq!(
        now::resolve_shell(&test_caps(false, true, true), Some(NowShell::Batch)),
        Ok(NowShell::Batch)
    );
    // An unavailable explicit choice is rejected with the available list.
    let error = now::resolve_shell(&test_caps(false, true, true), Some(NowShell::Pwsh)).unwrap_err();
    assert!(error.contains("pwsh"), "message: {error}");
    assert!(error.contains("powershell"), "message: {error}");
    assert!(error.contains("batch"), "message: {error}");
    // No shell available at all.
    let error = now::resolve_shell(&test_caps(false, false, false), None).unwrap_err();
    assert!(error.contains("no shell available"), "message: {error}");
}

#[test]
fn now_flatten_caps_produces_expected_entries() {
    let entries = now::flatten_caps(&test_caps(true, false, true));
    let get = |key: &str| entries.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());

    assert_eq!(get("now.version"), Some(PropValue::Str("1.6".to_owned())));
    assert_eq!(get("now.heartbeat_ms"), Some(PropValue::Int(60_000)));
    assert_eq!(get("now.pwsh"), Some(PropValue::Int(1)));
    assert_eq!(get("now.powershell"), Some(PropValue::Int(0)));
    assert_eq!(get("now.batch"), Some(PropValue::Int(1)));
    assert_eq!(get("now.default_shell"), Some(PropValue::Str("pwsh".to_owned())));

    // `heartbeat_ms` is omitted when no heartbeat was negotiated.
    let mut caps = test_caps(false, false, false);
    caps.heartbeat_ms = None;
    let entries = now::flatten_caps(&caps);
    assert!(entries.iter().all(|(key, _)| key != "now.heartbeat_ms"));
    let default_shell = entries
        .iter()
        .find(|(k, _)| k == "now.default_shell")
        .map(|(_, v)| v.clone());
    assert_eq!(default_shell, Some(PropValue::Str("none".to_owned())));
}

#[test]
fn now_render_capabilities_formats_key_value_lines() {
    let rendered = now::render_capabilities(&now::flatten_caps(&test_caps(true, false, true)));
    assert!(rendered.contains("version: 1.6\n"), "rendered: {rendered}");
    assert!(rendered.contains("heartbeat_ms: 60000\n"), "rendered: {rendered}");
    assert!(rendered.contains("pwsh: true\n"), "rendered: {rendered}");
    assert!(rendered.contains("powershell: false\n"), "rendered: {rendered}");
    assert!(rendered.contains("default_shell: pwsh\n"), "rendered: {rendered}");
}
