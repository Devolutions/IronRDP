//! Codec round-trip tests for the `ironrdp-agent` IPC and wire protocols.
//!
//! These exercise the crate's private wire format through its public (and `internal`-feature)
//! API. They live here, in the shared test suite, rather than inside `ironrdp-agent` itself, per
//! the workspace convention of keeping unit tests for protocol codecs in `ironrdp-testsuite-extra`.

use core::fmt::Debug;

use ironrdp_agent::ipc::{
    ConnState, KeyFilter, Payload, PropValue, PropertyDump, PropertyEntry, Request, Response, StatusInfo,
};
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
    props.insert("FullAddress", "host.example:3389");
    props.insert("Username", "operator");

    let mut props2 = PropertySet::new();
    props2.insert("FullAddress", "host.example:3389");

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
                    key: "FullAddress".to_owned(),
                    value: PropValue::Str("host.example:3389".to_owned()),
                },
                PropertyEntry {
                    key: "ServerPort".to_owned(),
                    value: PropValue::Int(3389),
                },
            ],
        })),
        Response::Ok(Payload::Logs(vec!["line one".to_owned(), "line two".to_owned()])),
        Response::Ok(Payload::Screenshot {
            width: 800,
            height: 600,
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
    original.insert("FullAddress", "host.example:3389");
    original.insert("ServerPort", 3389i64);
    original.insert("Username", "operator");
    original.insert("ScreenModeId", 2i64);

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
