#![allow(unused_crate_dependencies)]

use std::sync::{Arc, Mutex};

use core::convert::Infallible;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use ironrdp_mstsgu::GwConnectTarget;
use ironrdp_mstsgu::GwErrorKind;
use ironrdp_mstsgu::test_support::{GatewayTransport, evaluate_consent_message};

type TestBody = BoxBody<Bytes, Infallible>;

fn consent_message(message: &str) -> Vec<u8> {
    message
        .encode_utf16()
        .chain(core::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[test]
fn no_consent_preserves_existing_behavior() {
    let callback_count = Arc::new(Mutex::new(0));
    let callback_count_for_callback = Arc::clone(&callback_count);
    let mut callback = move |_message: &str| {
        *callback_count_for_callback.lock().expect("callback count lock") += 1;
        false
    };

    evaluate_consent_message(&[], Some(&mut callback)).expect("no consent");
    assert_eq!(*callback_count.lock().expect("callback count lock"), 0);
}

#[test]
fn default_accepts_gateway_consent() {
    evaluate_consent_message(&consent_message("Accept"), None).expect("default consent acceptance");
}

#[test]
fn callback_receives_decoded_consent_message_once() {
    let messages = Arc::new(Mutex::new(Vec::new()));
    let callback_messages = Arc::clone(&messages);
    let mut callback = move |message: &str| {
        callback_messages
            .lock()
            .expect("callback messages lock")
            .push(message.to_owned());
        true
    };

    evaluate_consent_message(&consent_message("Accept"), Some(&mut callback)).expect("accepted consent");
    assert_eq!(*messages.lock().expect("callback messages lock"), ["Accept"]);
}

#[test]
fn callback_rejection_returns_consent_declined() {
    let mut callback = |_message: &str| false;
    let error =
        evaluate_consent_message(&consent_message("Accept"), Some(&mut callback)).expect_err("declined consent");

    assert!(matches!(error.kind(), GwErrorKind::ConsentDeclined));
}

#[tokio::test]
async fn callback_rejection_stops_before_tunnel_authorization() {
    let (out_client, out_server) = tokio::io::duplex(4096);
    let (in_client, in_server) = tokio::io::duplex(4096);
    let consent = consent_message("Accept");
    let mut out_response = Vec::from([0; 10]);
    out_response.extend(packet(2, &handshake_response()));
    out_response.extend(packet(5, &tunnel_response(&consent)));

    let out_server = tokio::spawn(async move {
        let service = service_fn(move |request: Request<hyper::body::Incoming>| {
            let out_response = out_response.clone();
            async move {
                assert_eq!(request.method(), "RDG_OUT_DATA");
                Ok::<_, Infallible>(response(StatusCode::OK, Bytes::from(out_response)))
            }
        });
        http1::Builder::new()
            .serve_connection(TokioIo::new(out_server), service)
            .await
            .expect("serve OUT");
    });

    let in_server = tokio::spawn(async move {
        let service = service_fn(move |mut request: Request<hyper::body::Incoming>| async move {
            assert_eq!(request.method(), "RDG_IN_DATA");
            if request.headers().get("content-type").is_none() {
                return Ok::<_, Infallible>(response(StatusCode::OK, Bytes::new()));
            }

            for expected_packet_type in [1, 4] {
                let frame = request
                    .body_mut()
                    .frame()
                    .await
                    .expect("IN request frame")
                    .expect("IN request frame result")
                    .into_data()
                    .expect("IN request data frame");
                assert_eq!(packet_type(&frame), expected_packet_type);
            }

            let next = tokio::time::timeout(core::time::Duration::from_millis(100), request.body_mut().frame()).await;
            assert!(
                !matches!(next, Ok(Some(Ok(_)))),
                "no tunnel authorization or channel packet must follow a declined consent message"
            );
            Ok::<_, Infallible>(response(StatusCode::OK, Bytes::new()))
        });
        http1::Builder::new()
            .serve_connection(TokioIo::new(in_server), service)
            .await
            .expect("serve IN");
    });

    let transport = GatewayTransport::connect(out_client, in_client)
        .await
        .expect("connect transport");
    let mut callback_invoked = false;
    let mut callback = |_message: &str| {
        callback_invoked = true;
        false
    };
    let error = match transport
        .connect_tunnel(test_target(), "client.test", 3389, Some(&mut callback))
        .await
    {
        Ok(_) => panic!("declined consent"),
        Err(error) => error,
    };

    assert!(callback_invoked);
    assert!(matches!(error.kind(), GwErrorKind::ConsentDeclined));
    out_server.await.expect("OUT task");
    in_server.await.expect("IN task");
}

#[test]
fn malformed_consent_payload_is_rejected_before_callback() {
    let callback_count = Arc::new(Mutex::new(0));
    let callback_count_for_callback = Arc::clone(&callback_count);
    let mut callback = move |_message: &str| {
        *callback_count_for_callback.lock().expect("callback count lock") += 1;
        true
    };
    let error = evaluate_consent_message(&[0x41], Some(&mut callback)).expect_err("malformed consent");

    assert!(matches!(error.kind(), GwErrorKind::Decode));
    assert_eq!(*callback_count.lock().expect("callback count lock"), 0);
}

fn test_target() -> GwConnectTarget {
    GwConnectTarget {
        gw_endpoint: "gateway.test:443".to_owned(),
        gw_user: "user".to_owned(),
        gw_pass: "pass".to_owned(),
        smart_card: None,
        server: "server.test".to_owned(),
    }
}

fn packet(packet_type: u16, payload: &[u8]) -> Vec<u8> {
    let length = u32::try_from(8 + payload.len()).expect("packet length");
    let mut packet = Vec::with_capacity(8 + payload.len());
    packet.extend(packet_type.to_le_bytes());
    packet.extend([0; 2]);
    packet.extend(length.to_le_bytes());
    packet.extend(payload);
    packet
}

fn packet_type(packet: &[u8]) -> u16 {
    u16::from_le_bytes([packet[0], packet[1]])
}

fn handshake_response() -> [u8; 10] {
    [0, 0, 0, 0, 1, 0, 0, 0, 1, 0]
}

fn tunnel_response(consent: &[u8]) -> Vec<u8> {
    let consent_length = u16::try_from(consent.len()).expect("consent length");
    let mut response = Vec::with_capacity(12 + consent.len());
    response.extend([0, 0]);
    response.extend([0; 4]);
    response.extend([0x10, 0]);
    response.extend([0; 2]);
    response.extend(consent_length.to_le_bytes());
    response.extend(consent);
    response
}

fn response(status: StatusCode, body: Bytes) -> Response<TestBody> {
    let mut response = Response::new(Full::new(body).boxed());
    *response.status_mut() = status;
    response
}
