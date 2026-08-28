use core::convert::Infallible;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

use futures_util::{SinkExt as _, StreamExt as _};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::header::{HeaderName, HeaderValue};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use ironrdp_mstsgu::test_support::GatewayTransport;
use ironrdp_mstsgu::{GwClient, GwConnectTarget};
use ironrdp_tls::{CertificateValidation, CertificateValidationCallback};
use tokio::sync::oneshot;
use tokio_native_tls::TlsAcceptor;
use tokio_native_tls::native_tls::{Identity, TlsAcceptor as NativeTlsAcceptor};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;

type TestBody = BoxBody<Bytes, Infallible>;

const PACKET: &[u8] = &[
    0x0D, 0x00, // type: keepalive
    0x00, 0x00, // reserved
    0x08, 0x00, 0x00, 0x00, // length
];

#[tokio::test]
async fn strict_policy_rejects_self_signed_gateway_certificate() {
    let (listener, acceptor) = tls_listener().await;
    let target = gateway_target(listener.local_addr().expect("gateway listener address"));
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept strict TLS client");
        let _ = acceptor.accept(stream).await;
    });

    let error = match GatewayTransport::connect_tls(&target, CertificateValidation::Strict, None).await {
        Ok(_) => panic!("strict validation must reject the self-signed gateway certificate"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("tls connect"));

    server.await.expect("TLS server task");
}

#[tokio::test]
async fn native_tls_rejects_gateway_certificate_callback() {
    let (listener, acceptor) = tls_listener().await;
    let target = gateway_target(listener.local_addr().expect("gateway listener address"));
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept callback TLS client");
        assert!(acceptor.accept(stream).await.is_err());
    });
    let callback: CertificateValidationCallback = Arc::new(|_, _, _| true);

    let error = match GatewayTransport::connect_tls(&target, CertificateValidation::Strict, Some(callback)).await {
        Ok(_) => panic!("native TLS must reject certificate callbacks"),
        Err(error) => error,
    };
    assert!(
        format!("{error:?}").contains("certificate validation callbacks require the rustls backend"),
        "{error:?}"
    );

    server.await.expect("TLS server task");
}

#[tokio::test]
async fn dangerous_policy_with_gateway_callback_is_rejected() {
    let target = gateway_target("127.0.0.1:1".parse().expect("socket address"));
    let callback: CertificateValidationCallback = Arc::new(|_, _, _| true);

    let error = match GwClient::connect_with_certificate_validation(
        &target,
        "test-client",
        CertificateValidation::DangerouslyAcceptInvalidCertificate,
        Some(callback),
    )
    .await
    {
        Ok(_) => panic!("dangerous policy and callback must be rejected"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("invalid certificate validation configuration")
    );
}

#[tokio::test]
async fn switching_protocols_keeps_websocket_transport() {
    let (out_client, out_server) = tokio::io::duplex(4096);
    let (in_client, _in_server) = tokio::io::duplex(4096);
    let (upgrade_tx, upgrade_rx) = oneshot::channel();

    let server = tokio::spawn(async move {
        let upgrade_tx = Arc::new(Mutex::new(Some(upgrade_tx)));
        let service = service_fn(move |mut request: Request<hyper::body::Incoming>| {
            let upgrade = hyper::upgrade::on(&mut request);
            let upgrade_tx = Arc::clone(&upgrade_tx);
            async move {
                assert_eq!(request.method(), "RDG_OUT_DATA");
                assert!(request.headers().get("authorization").is_none());
                upgrade_tx
                    .lock()
                    .expect("upgrade sender lock")
                    .take()
                    .expect("one websocket request")
                    .send(upgrade)
                    .expect("send upgrade");

                let mut response = Response::new(Full::new(Bytes::new()).boxed());
                *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
                response
                    .headers_mut()
                    .insert("connection", "Upgrade".parse().expect("header"));
                response
                    .headers_mut()
                    .insert("upgrade", "websocket".parse().expect("header"));
                Ok::<_, Infallible>(response)
            }
        });
        http1::Builder::new()
            .serve_connection(TokioIo::new(out_server), service)
            .with_upgrades()
            .await
            .expect("serve websocket");
    });

    let mut transport = GatewayTransport::connect(out_client, in_client).await.expect("connect");
    let upgraded = upgrade_rx.await.expect("receive upgrade").await.expect("upgrade");
    let mut websocket = WebSocketStream::<TokioIo<hyper::upgrade::Upgraded>>::from_raw_socket(
        TokioIo::new(upgraded),
        Role::Server,
        None,
    )
    .await;

    websocket
        .send(Message::Binary(Bytes::from_static(b"gateway packet")))
        .await
        .expect("send websocket packet");
    assert_eq!(
        transport.read_packet().await.expect("read websocket packet"),
        Some(Bytes::from_static(b"gateway packet"))
    );

    transport
        .send_packet(b"client packet")
        .await
        .expect("send websocket packet");
    assert_eq!(
        websocket
            .next()
            .await
            .expect("websocket message")
            .expect("websocket result")
            .into_data(),
        Bytes::from_static(b"client packet")
    );

    transport.close().await.expect("close websocket");
    server.await.expect("server task");
}

#[tokio::test]
async fn dual_http_authenticates_replays_cookies_and_transfers_packets() {
    let (out_client, out_server) = tokio::io::duplex(4096);
    let (in_client, in_server) = tokio::io::duplex(4096);
    let (in_packet_tx, in_packet_rx) = oneshot::channel();
    let out_step = Arc::new(AtomicUsize::new(0));
    let in_step = Arc::new(AtomicUsize::new(0));

    let out_server = tokio::spawn({
        let out_step = Arc::clone(&out_step);
        async move {
            let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                let out_step = Arc::clone(&out_step);
                async move {
                    assert_eq!(request.method(), "RDG_OUT_DATA");
                    assert_eq!(request.headers()["rdg-connection-id"].as_bytes().first(), Some(&b'{'));
                    match out_step.fetch_add(1, Ordering::SeqCst) {
                        0 => {
                            assert!(request.headers().get("authorization").is_none());
                            Ok::<_, Infallible>(response(
                                StatusCode::UNAUTHORIZED,
                                &[
                                    ("www-authenticate", "Basic realm=\"RDG\""),
                                    ("set-cookie", "route=out; Path=/"),
                                ],
                                Bytes::new(),
                            ))
                        }
                        1 => {
                            assert_eq!(request.headers()["authorization"], "Basic dXNlcjpwYXNz");
                            assert_eq!(request.headers()["cookie"], "route=out");
                            Ok(response(
                                StatusCode::OK,
                                &[("set-cookie", "route=out-auth; Path=/")],
                                Bytes::from([b"0123456789".as_slice(), PACKET].concat()),
                            ))
                        }
                        _ => panic!("unexpected OUT request"),
                    }
                }
            });
            http1::Builder::new()
                .serve_connection(TokioIo::new(out_server), service)
                .await
                .expect("serve OUT");
        }
    });

    let in_server = tokio::spawn({
        let in_step = Arc::clone(&in_step);
        async move {
            let in_packet_tx = Arc::new(Mutex::new(Some(in_packet_tx)));
            let service = service_fn(move |mut request: Request<hyper::body::Incoming>| {
                let in_step = Arc::clone(&in_step);
                let in_packet_tx = Arc::clone(&in_packet_tx);
                async move {
                    assert_eq!(request.method(), "RDG_IN_DATA");
                    match in_step.fetch_add(1, Ordering::SeqCst) {
                        0 => {
                            assert!(request.headers().get("authorization").is_none());
                            assert_eq!(request.headers()["cookie"], "route=out-auth");
                            Ok::<_, Infallible>(response(
                                StatusCode::UNAUTHORIZED,
                                &[
                                    ("www-authenticate", "Basic realm=\"RDG\""),
                                    ("set-cookie", "session=in-auth; Path=/"),
                                ],
                                Bytes::new(),
                            ))
                        }
                        1 => {
                            assert_eq!(request.headers()["authorization"], "Basic dXNlcjpwYXNz");
                            assert_eq!(request.headers()["cookie"], "route=out-auth; session=in-auth");
                            Ok(response(StatusCode::OK, &[], Bytes::new()))
                        }
                        2 => {
                            let in_packet_tx = in_packet_tx
                                .lock()
                                .expect("IN data response sender lock")
                                .take()
                                .expect("IN data response sender");
                            assert!(request.headers().get("authorization").is_none());
                            assert_eq!(request.headers()["content-type"], "application/octet-stream");
                            assert_eq!(request.headers()["transfer-encoding"], "chunked");
                            assert_eq!(request.headers()["cookie"], "route=out-auth; session=in-auth");
                            let frame = request
                                .body_mut()
                                .frame()
                                .await
                                .expect("IN request frame")
                                .expect("IN request frame result")
                                .into_data()
                                .expect("IN request data frame");
                            in_packet_tx.send(frame).expect("record IN packet");
                            Ok(response(StatusCode::OK, &[], Bytes::new()))
                        }
                        _ => panic!("unexpected IN request"),
                    }
                }
            });
            http1::Builder::new()
                .serve_connection(TokioIo::new(in_server), service)
                .await
                .expect("serve IN");
        }
    });

    let mut transport = GatewayTransport::connect(out_client, in_client).await.expect("connect");
    transport.send_packet(PACKET).await.expect("send IN packet");
    assert_eq!(
        in_packet_rx.await.expect("recorded IN packet"),
        Bytes::from_static(PACKET)
    );
    assert_eq!(
        transport.read_packet().await.expect("read OUT packet"),
        Some(Bytes::from_static(PACKET))
    );
    transport.close().await.expect("close dual HTTP");

    out_server.await.expect("OUT task");
    in_server.await.expect("IN task");
    assert_eq!(out_step.load(Ordering::SeqCst), 2);
    assert_eq!(in_step.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn dual_http_reports_in_authentication_rejection() {
    let (out_client, out_server) = tokio::io::duplex(4096);
    let (in_client, in_server) = tokio::io::duplex(4096);

    let out_server = tokio::spawn(async move {
        let steps = Arc::new(AtomicUsize::new(0));
        let service = service_fn(move |request: Request<hyper::body::Incoming>| {
            let steps = Arc::clone(&steps);
            async move {
                match steps.fetch_add(1, Ordering::SeqCst) {
                    0 => {
                        assert!(request.headers().get("authorization").is_none());
                        Ok::<_, Infallible>(response(
                            StatusCode::UNAUTHORIZED,
                            &[("www-authenticate", "Basic realm=\"RDG\"")],
                            Bytes::new(),
                        ))
                    }
                    1 => Ok(response(StatusCode::OK, &[], Bytes::from_static(b"0123456789"))),
                    _ => panic!("unexpected OUT request"),
                }
            }
        });
        http1::Builder::new()
            .serve_connection(TokioIo::new(out_server), service)
            .await
            .expect("serve OUT");
    });

    let in_server = tokio::spawn(async move {
        let steps = Arc::new(AtomicUsize::new(0));
        let service = service_fn(move |request: Request<hyper::body::Incoming>| {
            let steps = Arc::clone(&steps);
            async move {
                assert_eq!(request.method(), "RDG_IN_DATA");
                match steps.fetch_add(1, Ordering::SeqCst) {
                    0 => Ok::<_, Infallible>(response(
                        StatusCode::UNAUTHORIZED,
                        &[("www-authenticate", "Basic realm=\"RDG\"")],
                        Bytes::new(),
                    )),
                    1 => {
                        assert_eq!(request.headers()["authorization"], "Basic dXNlcjpwYXNz");
                        Ok(response(StatusCode::FORBIDDEN, &[], Bytes::new()))
                    }
                    _ => panic!("unexpected IN request"),
                }
            }
        });
        http1::Builder::new()
            .serve_connection(TokioIo::new(in_server), service)
            .await
            .expect("serve IN");
    });

    let error = match GatewayTransport::connect(out_client, in_client).await {
        Ok(_) => panic!("IN authentication must fail"),
        Err(error) => error,
    };
    assert!(error.contains("rdg in data"));
    assert!(error.contains("unexpected http status 403"));

    out_server.await.expect("OUT task");
    in_server.await.expect("IN task");
}

#[tokio::test]
async fn dual_http_close_reports_final_in_status() {
    let (out_client, out_server) = tokio::io::duplex(4096);
    let (in_client, in_server) = tokio::io::duplex(4096);

    let out_server = tokio::spawn(async move {
        let steps = Arc::new(AtomicUsize::new(0));
        let service = service_fn(move |request: Request<hyper::body::Incoming>| {
            let steps = Arc::clone(&steps);
            async move {
                assert_eq!(request.method(), "RDG_OUT_DATA");
                match steps.fetch_add(1, Ordering::SeqCst) {
                    0 => Ok::<_, Infallible>(response(
                        StatusCode::UNAUTHORIZED,
                        &[("www-authenticate", "Basic realm=\"RDG\"")],
                        Bytes::new(),
                    )),
                    1 => Ok(response(StatusCode::OK, &[], Bytes::from_static(b"0123456789"))),
                    _ => panic!("unexpected OUT request"),
                }
            }
        });
        http1::Builder::new()
            .serve_connection(TokioIo::new(out_server), service)
            .await
            .expect("serve OUT");
    });

    let in_server = tokio::spawn(async move {
        let steps = Arc::new(AtomicUsize::new(0));
        let service = service_fn(move |request: Request<hyper::body::Incoming>| {
            let steps = Arc::clone(&steps);
            async move {
                match steps.fetch_add(1, Ordering::SeqCst) {
                    0 => Ok::<_, Infallible>(response(
                        StatusCode::UNAUTHORIZED,
                        &[("www-authenticate", "Basic realm=\"RDG\"")],
                        Bytes::new(),
                    )),
                    1 => Ok(response(StatusCode::OK, &[], Bytes::new())),
                    2 => {
                        request.into_body().collect().await.expect("drain IN request");
                        Ok(response(StatusCode::FORBIDDEN, &[], Bytes::new()))
                    }
                    _ => panic!("unexpected IN request"),
                }
            }
        });
        http1::Builder::new()
            .serve_connection(TokioIo::new(in_server), service)
            .await
            .expect("serve IN");
    });

    let mut transport = GatewayTransport::connect(out_client, in_client).await.expect("connect");
    let error = transport.close().await.expect_err("final IN status must fail close");
    assert!(error.contains("rdg in data"));
    assert!(error.contains("unexpected http status 403"));

    out_server.await.expect("OUT task");
    in_server.await.expect("IN task");
}

fn response(status: StatusCode, headers: &[(&str, &str)], body: Bytes) -> Response<TestBody> {
    let mut response = Response::new(Full::new(body).boxed());
    *response.status_mut() = status;
    for (name, value) in headers {
        response.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes()).expect("response header name"),
            HeaderValue::from_str(value).expect("response header value"),
        );
    }
    response
}

async fn tls_listener() -> (tokio::net::TcpListener, TlsAcceptor) {
    let identity = Identity::from_pkcs8(
        include_bytes!("../../../ironrdp-tls/tests/certs/server-cert.pem"),
        include_bytes!("../../../ironrdp-tls/tests/certs/server-key.pem"),
    )
    .expect("create TLS identity");
    let acceptor = TlsAcceptor::from(NativeTlsAcceptor::new(identity).expect("create TLS acceptor"));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind TLS gateway listener");
    (listener, acceptor)
}

fn gateway_target(address: core::net::SocketAddr) -> GwConnectTarget {
    GwConnectTarget {
        gw_endpoint: format!("localhost:{}", address.port()),
        gw_user: "user".to_owned(),
        gw_pass: "pass".to_owned(),
        smart_card: None,
        server: "server.test".to_owned(),
    }
}
