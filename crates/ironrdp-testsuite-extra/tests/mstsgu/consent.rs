use std::sync::{Arc, Mutex};

use core::convert::Infallible;
use futures_util::stream;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt as _, Full, StreamBody};
use hyper::body::{Bytes, Frame};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use ironrdp_mstsgu::test_support::{GatewayTransport, evaluate_consent_message};
use ironrdp_mstsgu::{GwConnectTarget, GwErrorKind, GwExtendedAuthentication, GwSessionAuthentication};
use tokio::io::AsyncReadExt as _;

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
    out_response.extend(packet(2, &handshake_response(0)));
    out_response.extend(packet(5, &tunnel_response(0, &consent)));

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

#[tokio::test]
async fn http_sspi_ntlm_is_selected_and_reused_for_the_in_channel() {
    let (out_client, out_server) = tokio::io::duplex(4096);
    let (in_client, in_server) = tokio::io::duplex(4096);
    let out_requests = Arc::new(Mutex::new(0));
    let out_requests_for_service = Arc::clone(&out_requests);

    let out_server = tokio::spawn(async move {
        let service = service_fn(move |request: Request<hyper::body::Incoming>| {
            let out_requests = Arc::clone(&out_requests_for_service);
            async move {
                assert_eq!(request.method(), "RDG_OUT_DATA");
                let request_number = {
                    let mut out_requests = out_requests.lock().expect("OUT request count lock");
                    let request_number = *out_requests;
                    *out_requests += 1;
                    request_number
                };
                match request_number {
                    0 => {
                        assert!(request.headers().get("authorization").is_none());
                        let mut response = response(StatusCode::UNAUTHORIZED, Bytes::new());
                        response
                            .headers_mut()
                            .insert("www-authenticate", "SSPI_NTLM".parse().expect("SSPI_NTLM header"));
                        Ok::<_, Infallible>(response)
                    }
                    1 => {
                        assert_eq!(
                            request.headers().get("authorization").expect("SSPI_NTLM authorization"),
                            "SSPI_NTLM"
                        );
                        Ok::<_, Infallible>(response(StatusCode::OK, Bytes::from(vec![0; 10])))
                    }
                    _ => panic!("unexpected OUT request"),
                }
            }
        });
        http1::Builder::new()
            .serve_connection(TokioIo::new(out_server), service)
            .await
            .expect("serve OUT");
    });

    let in_authorizations = Arc::new(Mutex::new(Vec::new()));
    let in_authorizations_for_service = Arc::clone(&in_authorizations);
    let in_server = tokio::spawn(async move {
        let service = service_fn(move |request: Request<hyper::body::Incoming>| {
            let in_authorizations = Arc::clone(&in_authorizations_for_service);
            async move {
                assert_eq!(request.method(), "RDG_IN_DATA");
                in_authorizations.lock().expect("IN authorization lock").push(
                    request
                        .headers()
                        .get("authorization")
                        .map(|authorization| authorization.to_str().expect("valid authorization").to_owned()),
                );
                Ok::<_, Infallible>(response(StatusCode::OK, Bytes::new()))
            }
        });
        http1::Builder::new()
            .serve_connection(TokioIo::new(in_server), service)
            .await
            .expect("serve IN");
    });

    let transport = GatewayTransport::connect(out_client, in_client)
        .await
        .expect("connect SSPI_NTLM transport");
    assert_eq!(transport.session_authentication(), GwSessionAuthentication::NtlmSspi);
    drop(transport);

    out_server.await.expect("OUT task");
    in_server.await.expect("IN task");
    assert_eq!(*out_requests.lock().expect("OUT request count lock"), 2);
    assert!(
        in_authorizations
            .lock()
            .expect("IN authorization lock")
            .contains(&Some("SSPI_NTLM".to_owned()))
    );
}

#[tokio::test]
async fn extended_sspi_ntlm_sends_an_initial_token_and_rejects_an_empty_challenge() {
    let (out_client, out_server) = tokio::io::duplex(4096);
    let (in_client, in_server) = tokio::io::duplex(4096);
    let mut out_response = Vec::from([0; 10]);
    out_response.extend(packet(2, &handshake_response_with_extended_auth(0, 0x0004)));
    out_response.extend(packet(3, &extended_auth_response(0, &[])));

    let out_server = tokio::spawn(mock_out_server(out_server, vec![out_response.split_off(10)]));
    let in_server = tokio::spawn(async move {
        let service = service_fn(move |mut request: Request<hyper::body::Incoming>| async move {
            assert_eq!(request.method(), "RDG_IN_DATA");
            if request.headers().get("content-type").is_none() {
                return Ok::<_, Infallible>(response(StatusCode::OK, Bytes::new()));
            }

            let handshake = request
                .body_mut()
                .frame()
                .await
                .expect("handshake frame")
                .expect("handshake frame result")
                .into_data()
                .expect("handshake data frame");
            assert_eq!(packet_type(&handshake), 1);
            assert_eq!(&handshake[12..14], &0x0004u16.to_le_bytes());

            let extended_auth = request
                .body_mut()
                .frame()
                .await
                .expect("extended authentication frame")
                .expect("extended authentication frame result")
                .into_data()
                .expect("extended authentication data frame");
            assert_eq!(packet_type(&extended_auth), 3);
            assert_ne!(&extended_auth[12..14], &0u16.to_le_bytes());
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
    let error = match transport
        .connect_tunnel_with_session_authentication(
            test_target(),
            "client.test",
            3389,
            GwSessionAuthentication::NtlmSspi,
        )
        .await
    {
        Ok(_) => panic!("empty extended authentication challenge"),
        Err(error) => error,
    };

    assert!(matches!(error.kind(), GwErrorKind::Connect));
    out_server.await.expect("OUT task");
    in_server.await.expect("IN task");
}

#[tokio::test]
async fn unsupported_extended_authentication_is_reported_precisely() {
    let (out_client, out_server) = tokio::io::duplex(4096);
    let (in_client, in_server) = tokio::io::duplex(4096);
    let out_server = tokio::spawn(mock_out_server(
        out_server,
        vec![packet(2, &handshake_response_with_extended_auth(0, 0x0001))],
    ));
    let in_server = tokio::spawn(async move {
        let service = service_fn(move |mut request: Request<hyper::body::Incoming>| async move {
            if request.headers().get("content-type").is_none() {
                return Ok::<_, Infallible>(response(StatusCode::OK, Bytes::new()));
            }
            let handshake = request
                .body_mut()
                .frame()
                .await
                .expect("handshake frame")
                .expect("handshake frame result")
                .into_data()
                .expect("handshake data frame");
            assert_eq!(packet_type(&handshake), 1);
            assert_eq!(&handshake[12..14], &0u16.to_le_bytes());
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
    let error = match transport.connect_tunnel(test_target(), "client.test", 3389, None).await {
        Ok(_) => panic!("unsupported extended authentication"),
        Err(error) => error,
    };

    assert!(matches!(
        error.kind(),
        GwErrorKind::UnsupportedExtendedAuthentication(GwExtendedAuthentication::SmartCard)
    ));
    out_server.await.expect("OUT task");
    in_server.await.expect("IN task");
}

#[tokio::test]
async fn control_errors_preserve_gateway_hresult() {
    for (expected_packet_types, error_code, context, label) in [
        (&[1][..], 0x8007_59DA, "Handshake", "E_PROXY_RAP_ACCESSDENIED"),
        (&[1, 4][..], 0x8007_59DD, "Tunnel", "E_PROXY_TS_CONNECTFAILED"),
        (&[1, 4, 6][..], 0x8009_030C, "TunnelAuth", "SEC_E_LOGON_DENIED"),
        (&[1, 4, 6, 8][..], 0x0000_59E8, "ChannelCreate", "E_PROXY_NOTSUPPORTED"),
    ] {
        let (out_client, out_server) = tokio::io::duplex(4096);
        let (in_client, in_server) = tokio::io::duplex(4096);
        let mut out_response = Vec::from([0; 10]);
        out_response.extend(packet(
            2,
            &handshake_response(if expected_packet_types.len() == 1 {
                error_code
            } else {
                0
            }),
        ));
        if 1 < expected_packet_types.len() {
            out_response.extend(packet(
                5,
                &tunnel_response(
                    if expected_packet_types.len() == 2 {
                        error_code
                    } else {
                        0
                    },
                    &[],
                ),
            ));
        }
        if 2 < expected_packet_types.len() {
            out_response.extend(packet(
                7,
                &control_response(if expected_packet_types.len() == 3 {
                    error_code
                } else {
                    0
                }),
            ));
        }
        if 3 < expected_packet_types.len() {
            out_response.extend(packet(9, &control_response(error_code)));
        }

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
            let service = service_fn(move |mut request: Request<hyper::body::Incoming>| {
                let expected_packet_types = expected_packet_types.to_vec();
                async move {
                    assert_eq!(request.method(), "RDG_IN_DATA");
                    if request.headers().get("content-type").is_none() {
                        return Ok::<_, Infallible>(response(StatusCode::OK, Bytes::new()));
                    }

                    for expected_packet_type in expected_packet_types {
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

                    Ok::<_, Infallible>(response(StatusCode::OK, Bytes::new()))
                }
            });
            http1::Builder::new()
                .serve_connection(TokioIo::new(in_server), service)
                .await
                .expect("serve IN");
        });

        let transport = GatewayTransport::connect(out_client, in_client)
            .await
            .expect("connect transport");
        let error = match transport.connect_tunnel(test_target(), "client.test", 3389, None).await {
            Ok(_) => panic!("gateway error"),
            Err(error) => error,
        };

        assert!(matches!(error.kind(), GwErrorKind::GatewayCode(code) if *code == error_code));
        assert_eq!(
            error.to_string(),
            format!("[{context}] gateway error 0x{error_code:08x} ({label})")
        );
        out_server.await.expect("OUT task");
        in_server.await.expect("IN task");
    }
}

#[tokio::test]
async fn reauthentication_uses_a_fresh_tunnel_and_preserves_the_data_transport() {
    let reauth_tunnel_context = 0x1122_3344_5566_7788;
    let (initial_out_client, initial_out_server) = tokio::io::duplex(4096);
    let (initial_in_client, initial_in_server) = tokio::io::duplex(4096);
    let (reauth_out_client, reauth_out_server) = tokio::io::duplex(4096);
    let (reauth_in_client, reauth_in_server) = tokio::io::duplex(4096);
    let (reauth_complete_tx, reauth_complete_rx) = tokio::sync::oneshot::channel();
    let (initial_out_tx, initial_out_rx) = tokio::sync::mpsc::channel(2);

    initial_out_tx
        .send(Bytes::from(out_response([
            packet(2, &handshake_response(0)),
            packet(5, &tunnel_response(0, &[])),
            packet(7, &control_response(0)),
            packet(9, &control_response(0)),
            packet(12, &reauth_message(reauth_tunnel_context)),
        ])))
        .await
        .expect("send initial response");
    let initial_out_server = tokio::spawn(mock_streaming_out_server(initial_out_server, initial_out_rx));
    let initial_in_server = tokio::spawn(mock_in_server(initial_in_server, None, None));
    let reauth_out_server = tokio::spawn(mock_out_server(
        reauth_out_server,
        vec![
            packet(2, &handshake_response(0)),
            packet(5, &tunnel_response(0, &[])),
            packet(7, &control_response(0)),
            packet(9, &control_response(0)),
        ],
    ));
    let reauth_in_server = tokio::spawn(mock_in_server(
        reauth_in_server,
        Some(reauth_tunnel_context),
        Some(reauth_complete_tx),
    ));

    let initial_transport = GatewayTransport::connect(initial_out_client, initial_in_client)
        .await
        .expect("connect initial transport");
    let reauth_transport = GatewayTransport::connect(reauth_out_client, reauth_in_client)
        .await
        .expect("connect reauthentication transport");
    let mut client = initial_transport
        .connect_tunnel_with_reauth(test_target(), "client.test", 3389, reauth_transport)
        .await
        .expect("connect tunnel");

    tokio::time::timeout(core::time::Duration::from_secs(1), reauth_complete_rx)
        .await
        .expect("reauthentication completed")
        .expect("reauthentication completion signal");
    reauth_out_server.await.expect("serve reauthentication OUT");
    initial_out_tx
        .send(Bytes::from(data_packet(b"after reauthentication")))
        .await
        .expect("send application data");
    let mut received = [0; 22];
    tokio::time::timeout(core::time::Duration::from_secs(1), client.read_exact(&mut received))
        .await
        .expect("read application data")
        .expect("application data");
    assert_eq!(&received, b"after reauthentication");

    drop(initial_out_tx);
    drop(client);

    initial_out_server.await.expect("serve initial OUT");
    initial_in_server.await.expect("serve initial IN");
    reauth_in_server.await.expect("serve reauthentication IN");
}

#[tokio::test]
async fn failed_reauthentication_preserves_the_data_transport() {
    let reauth_tunnel_context = 0x8877_6655_4433_2211;
    let (initial_out_client, initial_out_server) = tokio::io::duplex(4096);
    let (initial_in_client, initial_in_server) = tokio::io::duplex(4096);
    let (reauth_out_client, reauth_out_server) = tokio::io::duplex(4096);
    let (reauth_in_client, reauth_in_server) = tokio::io::duplex(4096);
    let (reauth_attempt_tx, reauth_attempt_rx) = tokio::sync::oneshot::channel();
    let (initial_out_tx, initial_out_rx) = tokio::sync::mpsc::channel(2);

    initial_out_tx
        .send(Bytes::from(out_response([
            packet(2, &handshake_response(0)),
            packet(5, &tunnel_response(0, &[])),
            packet(7, &control_response(0)),
            packet(9, &control_response(0)),
            packet(12, &reauth_message(reauth_tunnel_context)),
        ])))
        .await
        .expect("send initial response");
    let initial_out_server = tokio::spawn(mock_streaming_out_server(initial_out_server, initial_out_rx));
    let initial_in_server = tokio::spawn(mock_in_server(initial_in_server, None, None));
    let reauth_out_server = tokio::spawn(mock_out_server(
        reauth_out_server,
        vec![
            packet(2, &handshake_response(0)),
            packet(5, &tunnel_response(0x8007_59DD, &[])),
        ],
    ));
    let reauth_in_server = tokio::spawn(mock_reauth_failure_in_server(
        reauth_in_server,
        reauth_tunnel_context,
        reauth_attempt_tx,
    ));

    let initial_transport = GatewayTransport::connect(initial_out_client, initial_in_client)
        .await
        .expect("connect initial transport");
    let reauth_transport = GatewayTransport::connect(reauth_out_client, reauth_in_client)
        .await
        .expect("connect reauthentication transport");
    let mut client = initial_transport
        .connect_tunnel_with_reauth(test_target(), "client.test", 3389, reauth_transport)
        .await
        .expect("connect tunnel");

    tokio::time::timeout(core::time::Duration::from_secs(1), reauth_attempt_rx)
        .await
        .expect("reauthentication attempt")
        .expect("reauthentication attempt signal");
    reauth_out_server.await.expect("serve reauthentication OUT");
    initial_out_tx
        .send(Bytes::from(data_packet(b"after reauthentication failure")))
        .await
        .expect("send application data");
    let mut received = [0; 30];
    tokio::time::timeout(core::time::Duration::from_secs(1), client.read_exact(&mut received))
        .await
        .expect("read application data")
        .expect("application data");
    assert_eq!(&received, b"after reauthentication failure");

    drop(initial_out_tx);
    drop(client);
    initial_out_server.await.expect("serve initial OUT");
    initial_in_server.await.expect("serve initial IN");
    reauth_in_server.await.expect("serve reauthentication IN");
}

#[tokio::test]
async fn reauthentication_consent_requires_an_owned_callback() {
    let reauth_tunnel_context = 0x0102_0304_0506_0708;
    let consent = consent_message("Accept");
    let callback_count = Arc::new(Mutex::new(0));
    let callback_count_for_callback = Arc::clone(&callback_count);
    let (initial_out_client, initial_out_server) = tokio::io::duplex(4096);
    let (initial_in_client, initial_in_server) = tokio::io::duplex(4096);
    let (reauth_out_client, reauth_out_server) = tokio::io::duplex(4096);
    let (reauth_in_client, reauth_in_server) = tokio::io::duplex(4096);
    let (reauth_attempt_tx, reauth_attempt_rx) = tokio::sync::oneshot::channel();
    let (initial_out_tx, initial_out_rx) = tokio::sync::mpsc::channel(2);

    initial_out_tx
        .send(Bytes::from(out_response([
            packet(2, &handshake_response(0)),
            packet(5, &tunnel_response(0, &consent)),
            packet(7, &control_response(0)),
            packet(9, &control_response(0)),
            packet(12, &reauth_message(reauth_tunnel_context)),
        ])))
        .await
        .expect("send initial response");
    let initial_out_server = tokio::spawn(mock_streaming_out_server(initial_out_server, initial_out_rx));
    let initial_in_server = tokio::spawn(mock_in_server(initial_in_server, None, None));
    let reauth_out_server = tokio::spawn(mock_out_server(
        reauth_out_server,
        vec![
            packet(2, &handshake_response(0)),
            packet(5, &tunnel_response(0, &consent)),
        ],
    ));
    let reauth_in_server = tokio::spawn(mock_reauth_failure_in_server(
        reauth_in_server,
        reauth_tunnel_context,
        reauth_attempt_tx,
    ));

    let initial_transport = GatewayTransport::connect(initial_out_client, initial_in_client)
        .await
        .expect("connect initial transport");
    let reauth_transport = GatewayTransport::connect(reauth_out_client, reauth_in_client)
        .await
        .expect("connect reauthentication transport");
    let mut callback = move |_message: &str| {
        *callback_count_for_callback.lock().expect("callback count lock") += 1;
        true
    };
    let client = initial_transport
        .connect_tunnel_with_reauth_and_consent(
            test_target(),
            "client.test",
            3389,
            reauth_transport,
            Some(&mut callback),
        )
        .await
        .expect("connect tunnel");

    tokio::time::timeout(core::time::Duration::from_secs(1), reauth_attempt_rx)
        .await
        .expect("reauthentication attempt")
        .expect("reauthentication attempt signal");
    reauth_out_server.await.expect("serve reauthentication OUT");
    assert_eq!(*callback_count.lock().expect("callback count lock"), 1);

    drop(initial_out_tx);
    drop(client);
    initial_out_server.await.expect("serve initial OUT");
    initial_in_server.await.expect("serve initial IN");
    reauth_in_server.await.expect("serve reauthentication IN");
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

fn handshake_response(error_code: u32) -> [u8; 10] {
    handshake_response_with_extended_auth(error_code, 0)
}

fn handshake_response_with_extended_auth(error_code: u32, extended_auth: u16) -> [u8; 10] {
    let mut response = [0, 0, 0, 0, 1, 0, 0, 0, 0, 0];
    response[..4].copy_from_slice(&error_code.to_le_bytes());
    response[8..].copy_from_slice(&extended_auth.to_le_bytes());
    response
}

fn tunnel_response(status_code: u32, consent: &[u8]) -> Vec<u8> {
    let consent_length = u16::try_from(consent.len()).expect("consent length");
    let mut response = Vec::with_capacity(12 + consent.len());
    response.extend([0, 0]);
    response.extend(status_code.to_le_bytes());
    response.extend(if consent.is_empty() { [0, 0] } else { [0x10, 0] });
    response.extend([0; 2]);
    if !consent.is_empty() {
        response.extend(consent_length.to_le_bytes());
        response.extend(consent);
    }
    response
}

fn control_response(error_code: u32) -> [u8; 8] {
    let mut response = [0; 8];
    response[..4].copy_from_slice(&error_code.to_le_bytes());
    response
}

fn extended_auth_response(error_code: u32, auth_blob: &[u8]) -> Vec<u8> {
    let mut response = Vec::with_capacity(6 + auth_blob.len());
    response.extend(error_code.to_le_bytes());
    response.extend(
        u16::try_from(auth_blob.len())
            .expect("extended authentication blob length")
            .to_le_bytes(),
    );
    response.extend(auth_blob);
    response
}

fn reauth_message(reauth_tunnel_context: u64) -> [u8; 8] {
    reauth_tunnel_context.to_le_bytes()
}

fn data_packet(data: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(2 + data.len());
    payload.extend(u16::try_from(data.len()).expect("data length").to_le_bytes());
    payload.extend(data);
    packet(10, &payload)
}

fn out_response(packets: impl IntoIterator<Item = Vec<u8>>) -> Vec<u8> {
    let mut response_body = Vec::from([0; 10]);
    for packet in packets {
        response_body.extend(packet);
    }
    response_body
}

async fn mock_out_server(stream: tokio::io::DuplexStream, packets: Vec<Vec<u8>>) {
    let response_body = out_response(packets);
    let service = service_fn(move |request: Request<hyper::body::Incoming>| {
        let response_body = response_body.clone();
        async move {
            assert_eq!(request.method(), "RDG_OUT_DATA");
            Ok::<_, Infallible>(response(StatusCode::OK, Bytes::from(response_body)))
        }
    });
    http1::Builder::new()
        .serve_connection(TokioIo::new(stream), service)
        .await
        .expect("serve OUT");
}

async fn mock_streaming_out_server(stream: tokio::io::DuplexStream, response_body: tokio::sync::mpsc::Receiver<Bytes>) {
    let response_body = Arc::new(Mutex::new(Some(response_body)));
    let service = service_fn(move |request: Request<hyper::body::Incoming>| {
        let response_body = response_body
            .lock()
            .expect("response body lock")
            .take()
            .expect("single OUT request");
        async move {
            assert_eq!(request.method(), "RDG_OUT_DATA");
            Ok::<_, Infallible>(streaming_response(StatusCode::OK, response_body))
        }
    });
    http1::Builder::new()
        .serve_connection(TokioIo::new(stream), service)
        .await
        .expect("serve streaming OUT");
}

async fn mock_reauth_failure_in_server(
    stream: tokio::io::DuplexStream,
    reauth_tunnel_context: u64,
    attempt: tokio::sync::oneshot::Sender<()>,
) {
    let attempt = Arc::new(Mutex::new(Some(attempt)));
    let service = service_fn(move |mut request: Request<hyper::body::Incoming>| {
        let attempt = Arc::clone(&attempt);
        async move {
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
                if expected_packet_type == 4 {
                    assert_eq!(&frame[12..14], &0x2u16.to_le_bytes());
                    assert_eq!(&frame[16..24], &reauth_tunnel_context.to_le_bytes());
                }
            }
            attempt
                .lock()
                .expect("reauthentication attempt lock")
                .take()
                .expect("single reauthentication attempt")
                .send(())
                .expect("send reauthentication attempt");

            let next = tokio::time::timeout(core::time::Duration::from_millis(100), request.body_mut().frame()).await;
            assert!(
                !matches!(next, Ok(Some(Ok(_)))),
                "reauthentication consent rejection or tunnel failure must not authorize or create a channel"
            );
            Ok::<_, Infallible>(response(StatusCode::OK, Bytes::new()))
        }
    });
    http1::Builder::new()
        .serve_connection(TokioIo::new(stream), service)
        .await
        .expect("serve reauthentication IN");
}

async fn mock_in_server(
    stream: tokio::io::DuplexStream,
    reauth_tunnel_context: Option<u64>,
    complete: Option<tokio::sync::oneshot::Sender<()>>,
) {
    let complete = Arc::new(Mutex::new(complete));
    let service = service_fn(move |mut request: Request<hyper::body::Incoming>| {
        let complete = Arc::clone(&complete);
        async move {
            assert_eq!(request.method(), "RDG_IN_DATA");
            if request.headers().get("content-type").is_none() {
                return Ok::<_, Infallible>(response(StatusCode::OK, Bytes::new()));
            }

            for expected_packet_type in [1, 4, 6, 8] {
                let frame = request
                    .body_mut()
                    .frame()
                    .await
                    .expect("IN request frame")
                    .expect("IN request frame result")
                    .into_data()
                    .expect("IN request data frame");
                assert_eq!(packet_type(&frame), expected_packet_type);
                if expected_packet_type == 4 {
                    match reauth_tunnel_context {
                        Some(reauth_tunnel_context) => {
                            assert_eq!(&frame[8..12], &0x14u32.to_le_bytes());
                            assert_eq!(&frame[12..14], &0x2u16.to_le_bytes());
                            assert_eq!(&frame[16..24], &reauth_tunnel_context.to_le_bytes());
                        }
                        None => {
                            assert_eq!(&frame[8..12], &0x14u32.to_le_bytes());
                            assert_eq!(&frame[12..14], &0u16.to_le_bytes());
                        }
                    }
                }
            }

            if let Some(complete) = complete.lock().expect("completion lock").take() {
                complete.send(()).expect("send reauthentication completion");
            }
            Ok::<_, Infallible>(response(StatusCode::OK, Bytes::new()))
        }
    });
    http1::Builder::new()
        .serve_connection(TokioIo::new(stream), service)
        .await
        .expect("serve IN");
}

fn response(status: StatusCode, body: Bytes) -> Response<TestBody> {
    let mut response = Response::new(Full::new(body).boxed());
    *response.status_mut() = status;
    response
}

fn streaming_response(status: StatusCode, mut body: tokio::sync::mpsc::Receiver<Bytes>) -> Response<TestBody> {
    let body = StreamBody::new(stream::poll_fn(move |cx| {
        body.poll_recv(cx)
            .map(|data| data.map(|data| Ok::<_, Infallible>(Frame::data(data))))
    }))
    .boxed();
    let mut response = Response::new(body);
    *response.status_mut() = status;
    response
}
