use core::convert::Infallible;
use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use futures_util::StreamExt as _;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use ironrdp_mstsgu::GwConnectTarget;
use ironrdp_mstsgu::test_support::GatewayTransport;
use ironrdp_tls::{CertificateValidation, CertificateValidationCallback};
use tokio::sync::oneshot;
use tokio_native_tls::TlsAcceptor;
use tokio_native_tls::native_tls::{Identity, TlsAcceptor as NativeTlsAcceptor};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::Role;

type TestBody = BoxBody<Bytes, Infallible>;

#[tokio::test]
async fn callback_validates_websocket_gateway_tls() {
    let (listener, acceptor) = tls_listener().await;
    let target = gateway_target(listener.local_addr().expect("gateway listener address"));
    let (upgrade_tx, upgrade_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept WebSocket TLS client");
        let stream = acceptor.accept(stream).await.expect("accept WebSocket TLS client");
        let upgrade_tx = Arc::new(std::sync::Mutex::new(Some(upgrade_tx)));
        let service = service_fn(move |mut request: Request<hyper::body::Incoming>| {
            let upgrade = hyper::upgrade::on(&mut request);
            let upgrade_tx = Arc::clone(&upgrade_tx);
            async move {
                assert_eq!(request.method(), "RDG_OUT_DATA");
                upgrade_tx
                    .lock()
                    .expect("upgrade sender lock")
                    .take()
                    .expect("one WebSocket request")
                    .send(upgrade)
                    .expect("send WebSocket upgrade");

                let mut response = Response::new(Full::new(Bytes::new()).boxed());
                *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
                response
                    .headers_mut()
                    .insert("connection", "Upgrade".parse().expect("connection header"));
                response
                    .headers_mut()
                    .insert("upgrade", "websocket".parse().expect("upgrade header"));
                Ok::<_, Infallible>(response)
            }
        });
        http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .with_upgrades()
            .await
            .expect("serve WebSocket");
    });

    let callback_hits = Arc::new(AtomicUsize::new(0));
    let callback: CertificateValidationCallback = {
        let callback_hits = Arc::clone(&callback_hits);
        let expected_endpoint = target.gw_endpoint.clone();
        Arc::new(move |_, server_name, _| {
            assert_eq!(server_name, expected_endpoint);
            callback_hits.fetch_add(1, Ordering::SeqCst);
            true
        })
    };
    let mut transport = GatewayTransport::connect_tls(&target, CertificateValidation::Strict, Some(callback))
        .await
        .expect("callback accepts WebSocket gateway certificate");
    let upgraded = upgrade_rx
        .await
        .expect("receive WebSocket upgrade")
        .await
        .expect("upgrade");
    let mut websocket = WebSocketStream::<TokioIo<hyper::upgrade::Upgraded>>::from_raw_socket(
        TokioIo::new(upgraded),
        Role::Server,
        None,
    )
    .await;

    transport.close().await.expect("close WebSocket transport");
    assert!(matches!(
        websocket
            .next()
            .await
            .expect("WebSocket close message")
            .expect("WebSocket close result"),
        Message::Close(_)
    ));
    assert_eq!(callback_hits.load(Ordering::SeqCst), 1);
    server.await.expect("WebSocket server task");
}

#[tokio::test]
async fn callback_validates_both_dual_http_gateway_tls_connections() {
    let (listener, acceptor) = tls_listener().await;
    let target = gateway_target(listener.local_addr().expect("gateway listener address"));
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept OUT TLS client");
        let stream = acceptor.accept(stream).await.expect("accept OUT TLS client");
        let out_server = tokio::spawn(async move {
            let service = service_fn(|request: Request<hyper::body::Incoming>| async move {
                assert_eq!(request.method(), "RDG_OUT_DATA");
                Ok::<_, Infallible>(response(StatusCode::OK, Bytes::from_static(b"0123456789")))
            });
            http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
                .expect("serve OUT");
        });

        let (stream, _) = listener.accept().await.expect("accept IN TLS client");
        let stream = acceptor.accept(stream).await.expect("accept IN TLS client");
        let in_server = tokio::spawn(async move {
            let request_count = Arc::new(AtomicUsize::new(0));
            let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                let request_count = Arc::clone(&request_count);
                async move {
                    assert_eq!(request.method(), "RDG_IN_DATA");
                    match request_count.fetch_add(1, Ordering::SeqCst) {
                        0 => Ok::<_, Infallible>(response(StatusCode::OK, Bytes::new())),
                        1 => {
                            request.into_body().collect().await.expect("drain IN request");
                            Ok(response(StatusCode::OK, Bytes::new()))
                        }
                        _ => panic!("unexpected IN request"),
                    }
                }
            });
            http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await
                .expect("serve IN");
        });

        out_server.await.expect("OUT server task");
        in_server.await.expect("IN server task");
    });

    let callback_hits = Arc::new(AtomicUsize::new(0));
    let callback: CertificateValidationCallback = {
        let callback_hits = Arc::clone(&callback_hits);
        let expected_endpoint = target.gw_endpoint.clone();
        Arc::new(move |_, server_name, _| {
            assert_eq!(server_name, expected_endpoint);
            callback_hits.fetch_add(1, Ordering::SeqCst);
            true
        })
    };
    let mut transport = GatewayTransport::connect_tls(&target, CertificateValidation::Strict, Some(callback))
        .await
        .expect("callback accepts dual-HTTP gateway certificates");

    assert_eq!(callback_hits.load(Ordering::SeqCst), 2);
    transport.close().await.expect("close dual-HTTP transport");
    server.await.expect("dual-HTTP server task");
}

fn response(status: StatusCode, body: Bytes) -> Response<TestBody> {
    let mut response = Response::new(Full::new(body).boxed());
    *response.status_mut() = status;
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
