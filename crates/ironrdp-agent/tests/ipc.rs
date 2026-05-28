//! IPC framing integration test: round-trip every Request and Response over
//! an in-memory duplex stream and assert the value is preserved end-to-end.

#![expect(unused_crate_dependencies, reason = "agent crate brings in many transitive deps")]

use ironrdp_agent::ipc::{
    KeyboardAction, MouseAction, MouseButton, PropertyEntry, Request, Response, SessionStatus, SessionSummary,
    read_frame, write_frame,
};

async fn roundtrip_request(req: Request) -> Request {
    let (mut a, mut b) = tokio::io::duplex(1024 * 1024);
    let writer = tokio::spawn(async move {
        write_frame(&mut a, &req).await.expect("write");
        req
    });
    let received: Request = read_frame(&mut b).await.expect("read");
    let original = writer.await.expect("writer task");
    // Re-encode for byte equality.
    let a = ironrdp_core::encode_vec(&original).expect("encode a");
    let b = ironrdp_core::encode_vec(&received).expect("encode b");
    assert_eq!(a, b);
    received
}

async fn roundtrip_response(resp: Response) {
    let (mut a, mut b) = tokio::io::duplex(1024 * 1024);
    let original = resp.clone();
    let writer = tokio::spawn(async move {
        write_frame(&mut a, &resp).await.expect("write");
    });
    let received: Response = read_frame(&mut b).await.expect("read");
    writer.await.expect("writer task");
    let a = ironrdp_core::encode_vec(&original).expect("encode a");
    let b = ironrdp_core::encode_vec(&received).expect("encode b");
    assert_eq!(a, b);
}

#[tokio::test]
async fn all_request_variants_round_trip() {
    roundtrip_request(Request::Health).await;
    roundtrip_request(Request::Sessions).await;
    roundtrip_request(Request::Connect {
        rdp_content: "full address:s:host\nusername:s:bob".to_owned(),
        label: Some("primary".to_owned()),
    })
    .await;
    roundtrip_request(Request::Status { session_id: None }).await;
    roundtrip_request(Request::Status {
        session_id: Some("abc".to_owned()),
    })
    .await;
    roundtrip_request(Request::Disconnect {
        session_id: "abc".to_owned(),
    })
    .await;
    roundtrip_request(Request::Mouse {
        session_id: "s".to_owned(),
        action: MouseAction::Move { x: 10, y: 20 },
    })
    .await;
    roundtrip_request(Request::Mouse {
        session_id: "s".to_owned(),
        action: MouseAction::Click {
            button: MouseButton::Right,
            x: Some(1),
            y: Some(2),
        },
    })
    .await;
    roundtrip_request(Request::Mouse {
        session_id: "s".to_owned(),
        action: MouseAction::Wheel {
            units: -3,
            horizontal: true,
        },
    })
    .await;
    roundtrip_request(Request::Keyboard {
        session_id: "s".to_owned(),
        action: KeyboardAction::Text {
            text: "hello".to_owned(),
        },
    })
    .await;
    roundtrip_request(Request::Keyboard {
        session_id: "s".to_owned(),
        action: KeyboardAction::Shortcut {
            scancodes: vec![0x1d, 0x2e],
        },
    })
    .await;
    roundtrip_request(Request::Keyboard {
        session_id: "s".to_owned(),
        action: KeyboardAction::ReleaseAll,
    })
    .await;
    roundtrip_request(Request::Resize {
        session_id: "s".to_owned(),
        width: 1024,
        height: 768,
        scale: 125,
    })
    .await;
    roundtrip_request(Request::WaitFrame {
        session_id: "s".to_owned(),
        timeout_ms: 5000,
        after_frame: Some(42),
    })
    .await;
    roundtrip_request(Request::Screenshot {
        session_id: "s".to_owned(),
    })
    .await;
    roundtrip_request(Request::MousePosition {
        session_id: "s".to_owned(),
    })
    .await;
    roundtrip_request(Request::DumpProperties {
        session_id: "s".to_owned(),
    })
    .await;
    roundtrip_request(Request::SetProperty {
        session_id: "s".to_owned(),
        key: "desktopwidth".to_owned(),
        value: "1024".to_owned(),
    })
    .await;
}

#[tokio::test]
async fn all_response_variants_round_trip() {
    roundtrip_response(Response::Ok).await;
    roundtrip_response(Response::Health).await;
    roundtrip_response(Response::Error {
        message: "boom".to_owned(),
    })
    .await;
    roundtrip_response(Response::Connect {
        session_id: "abc".to_owned(),
    })
    .await;
    roundtrip_response(Response::Sessions {
        sessions: vec![
            SessionSummary {
                session_id: "a".to_owned(),
                label: None,
                status: SessionStatus::Connecting,
                width: None,
                height: None,
                frame_sequence: 0,
                mouse_x: 0,
                mouse_y: 0,
                last_error: None,
            },
            SessionSummary {
                session_id: "b".to_owned(),
                label: Some("p".to_owned()),
                status: SessionStatus::Failed,
                width: Some(1920),
                height: Some(1080),
                frame_sequence: 99,
                mouse_x: 10,
                mouse_y: 20,
                last_error: Some("nope".to_owned()),
            },
        ],
    })
    .await;
    roundtrip_response(Response::Screenshot {
        png: (0..256u32)
            .map(|i| u8::try_from(i & 0xff).expect("masked to 0..=255"))
            .collect(),
    })
    .await;
    roundtrip_response(Response::Properties {
        entries: vec![
            PropertyEntry {
                key: "k1".to_owned(),
                value: "v1".to_owned(),
                description: "d1".to_owned(),
            },
            PropertyEntry {
                key: "agent:state".to_owned(),
                value: "connected".to_owned(),
                description: "Current session state".to_owned(),
            },
        ],
    })
    .await;
}
