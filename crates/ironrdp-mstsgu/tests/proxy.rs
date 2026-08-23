#![allow(unused_crate_dependencies)]

use core::convert::Infallible;
use std::ffi::OsString;

use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use ironrdp_mstsgu::GwConnectTarget;
use ironrdp_mstsgu::test_support::{
    GatewayTransport, gateway_endpoint_is_valid, proxy_debug, proxy_summary, proxy_uses_basic_authorization,
    validate_proxy_response,
};
use ironrdp_tls::CertificateValidation;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_native_tls::TlsAcceptor;
use tokio_native_tls::native_tls::{Identity, TlsAcceptor as NativeTlsAcceptor};

type TestBody = BoxBody<Bytes, Infallible>;

static ENVIRONMENT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn parses_supported_proxy_urls_without_exposing_credentials() {
    assert_eq!(
        proxy_summary(
            "gateway.example.test",
            Some("http://proxy.example.test:8080"),
            None,
            None,
            None
        )
        .expect("parse HTTP proxy"),
        Some("http://proxy.example.test:8080".to_owned())
    );
    assert_eq!(
        proxy_summary(
            "gateway.example.test",
            Some("https://proxy.example.test"),
            None,
            None,
            None
        )
        .expect("parse HTTPS proxy"),
        Some("https://proxy.example.test:443".to_owned())
    );
    assert_eq!(
        proxy_summary(
            "gateway.example.test",
            Some("socks5://proxy.example.test"),
            None,
            None,
            None
        )
        .expect("parse SOCKS5 proxy"),
        Some("socks5://proxy.example.test:1080".to_owned())
    );
    assert_eq!(
        proxy_summary(
            "gateway.example.test",
            Some("socks5h://proxy.example.test"),
            None,
            None,
            None
        )
        .expect("parse SOCKS5H proxy"),
        Some("socks5h://proxy.example.test:1080".to_owned())
    );

    let credential = String::from_utf8(vec![115, 101, 99, 114, 101, 116]).expect("credential text");
    let proxy_url = format!("http://user:{credential}@proxy.example.test");
    assert!(proxy_uses_basic_authorization(&proxy_url).expect("build proxy authorization"));
    let debug = proxy_debug(&proxy_url).expect("format proxy configuration");
    if debug.contains(&credential) {
        panic!("proxy configuration debug output must redact credentials");
    }
    assert!(debug.contains("<redacted>"));
}

#[test]
fn rejects_unsupported_or_malformed_proxy_urls() {
    for proxy in [
        "ftp://proxy.example.test",
        "http://proxy.example.test/path",
        "http://proxy.example.test?query",
        "http://proxy.example.test:99999",
        "http://user@proxy.example.test",
        "http://:password@proxy.example.test",
    ] {
        assert!(proxy_summary("gateway.example.test", Some(proxy), None, None, None).is_err());
    }
    assert!(!gateway_endpoint_is_valid("gateway.example.test\r\nX-Injected: 443"));
}

#[test]
fn honors_proxy_and_no_proxy_precedence() {
    assert_eq!(
        proxy_summary(
            "gateway.example.test",
            Some("http://upper.example.test"),
            Some("http://lower.example.test"),
            None,
            None,
        )
        .expect("select preferred proxy"),
        Some("http://upper.example.test:80".to_owned())
    );
    assert_eq!(
        proxy_summary(
            "api.example.test",
            Some("http://proxy.example.test"),
            None,
            Some(".example.test"),
            Some("*"),
        )
        .expect("match uppercase no proxy"),
        None
    );
    assert_eq!(
        proxy_summary(
            "gateway.example.test",
            Some("http://proxy.example.test"),
            None,
            Some("gateway.example.test:8443"),
            None,
        )
        .expect("ignore a no-proxy entry for another port"),
        Some("http://proxy.example.test:80".to_owned())
    );
    assert_eq!(
        proxy_summary(
            "gateway.example.test",
            Some("http://proxy.example.test"),
            None,
            None,
            Some("example.test"),
        )
        .expect("match bare suffix"),
        None
    );
    assert_eq!(
        proxy_summary(
            "10.0.0.1",
            Some("http://proxy.example.test"),
            None,
            Some("10.0.0.0/8"),
            None,
        )
        .expect("ignore CIDR range"),
        Some("http://proxy.example.test:80".to_owned())
    );
    assert_eq!(
        proxy_summary(
            "10.0.0.1",
            Some("http://proxy.example.test"),
            None,
            Some("10.0.0.1"),
            None,
        )
        .expect("match exact IP address"),
        None
    );
    assert_eq!(
        proxy_summary(
            "gateway.example.test",
            Some("http://proxy.example.test"),
            None,
            Some("gateway.example.test:443"),
            None,
        )
        .expect("match exact host and port"),
        None
    );
    assert_eq!(
        proxy_summary("gateway.example.test", None, None, None, None).expect("select direct connection"),
        None
    );
}

#[tokio::test]
async fn validates_bounded_http_connect_responses() {
    validate_proxy_response(b"HTTP/1.1 204 No Content\r\nProxy-Agent: test\r\n\r\n")
        .await
        .expect("accept successful CONNECT response");

    let error = validate_proxy_response(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
        .await
        .expect_err("reject unsuccessful CONNECT response");
    assert!(error.contains("unexpected http status 407"));

    let oversized_header = vec![b'a'; 16 * 1024];
    let header_prefix: &[u8] = b"HTTP/1.1 200 OK\r\nX: ";
    let header_suffix: &[u8] = b"\r\n\r\n";
    let oversized = [header_prefix, oversized_header.as_slice(), header_suffix].concat();
    let error = validate_proxy_response(&oversized)
        .await
        .expect_err("reject oversized CONNECT response");
    assert!(error.contains("proxy connect response too large"));
}

#[tokio::test(flavor = "current_thread")]
async fn http_proxy_tunnels_both_dual_http_legs() {
    let _environment_lock = ENVIRONMENT_LOCK.lock().await;
    let (gateway_listener, gateway_acceptor) = tls_listener().await;
    let gateway_addr = gateway_listener.local_addr().expect("gateway listener address");
    let gateway = tokio::spawn(serve_dual_http_gateway(gateway_listener, gateway_acceptor));
    let proxy_listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind HTTP proxy listener");
    let proxy_port = proxy_listener.local_addr().expect("proxy listener address").port();
    let proxy = tokio::spawn(serve_http_proxy(proxy_listener, gateway_addr));
    let _environment = ProxyEnvironment::set(&format!("http://user:password@127.0.0.1:{proxy_port}"));

    let target = gateway_target(gateway_addr);
    let mut transport = GatewayTransport::connect_tls(
        &target,
        CertificateValidation::DangerouslyAcceptInvalidCertificate,
        None,
    )
    .await
    .expect("connect through HTTP proxy");
    transport.close().await.expect("close dual HTTP transport");
    drop(transport);

    gateway.await.expect("gateway task");
    proxy.await.expect("proxy task");
}

#[tokio::test(flavor = "current_thread")]
async fn socks_proxies_tunnel_both_dual_http_legs() {
    let _environment_lock = ENVIRONMENT_LOCK.lock().await;
    for (scheme, remote_dns) in [("socks5", false), ("socks5h", true)] {
        let (gateway_listener, gateway_acceptor) = tls_listener().await;
        let gateway_addr = gateway_listener.local_addr().expect("gateway listener address");
        let gateway = tokio::spawn(serve_dual_http_gateway(gateway_listener, gateway_acceptor));
        let proxy_listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind SOCKS proxy listener");
        let proxy_port = proxy_listener.local_addr().expect("proxy listener address").port();
        let proxy = tokio::spawn(serve_socks_proxy(proxy_listener, gateway_addr, remote_dns));
        let proxy_url = format!("{scheme}://user:password@127.0.0.1:{proxy_port}");
        let _environment = ProxyEnvironment::set(&proxy_url);

        let target = gateway_target(gateway_addr);
        let mut transport = GatewayTransport::connect_tls(
            &target,
            CertificateValidation::DangerouslyAcceptInvalidCertificate,
            None,
        )
        .await
        .expect("connect through SOCKS proxy");
        transport.close().await.expect("close dual HTTP transport");
        drop(transport);

        gateway.await.expect("gateway task");
        proxy.await.expect("proxy task");
    }
}

struct ProxyEnvironment {
    previous: Vec<(&'static str, Option<OsString>)>,
}

impl ProxyEnvironment {
    fn set(proxy: &str) -> Self {
        let mut previous = Vec::new();
        for variable in ["HTTPS_PROXY", "https_proxy", "NO_PROXY", "no_proxy"] {
            previous.push((variable, std::env::var_os(variable)));
        }
        // SAFETY: the test serializes environment mutation and no spawned task reads these variables.
        unsafe {
            std::env::set_var("HTTPS_PROXY", proxy);
        }
        // SAFETY: the test serializes environment mutation and no spawned task reads these variables.
        unsafe {
            std::env::set_var("NO_PROXY", "");
        }
        Self { previous }
    }
}

impl Drop for ProxyEnvironment {
    fn drop(&mut self) {
        for (variable, value) in &self.previous {
            if let Some(value) = value {
                // SAFETY: the test serializes environment mutation and has completed all proxy connections.
                unsafe {
                    std::env::set_var(variable, value);
                }
            } else {
                // SAFETY: the test serializes environment mutation and has completed all proxy connections.
                unsafe {
                    std::env::remove_var(variable);
                }
            }
        }
    }
}

async fn tls_listener() -> (TcpListener, TlsAcceptor) {
    let identity = Identity::from_pkcs8(
        include_bytes!("../../ironrdp-tls/tests/certs/server-cert.pem"),
        include_bytes!("../../ironrdp-tls/tests/certs/server-key.pem"),
    )
    .expect("create TLS identity");
    let acceptor = TlsAcceptor::from(NativeTlsAcceptor::new(identity).expect("create TLS acceptor"));
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind TLS gateway listener");
    (listener, acceptor)
}

async fn serve_dual_http_gateway(listener: TcpListener, acceptor: TlsAcceptor) {
    for connection in 0..2 {
        let (stream, _) = listener.accept().await.expect("accept gateway client");
        let stream = acceptor.accept(stream).await.expect("accept gateway TLS client");
        let service = service_fn(move |request: Request<hyper::body::Incoming>| async move {
            if connection == 0 {
                assert_eq!(request.method(), "RDG_OUT_DATA");
                let mut response = response(StatusCode::OK, Bytes::from_static(b"0123456789"));
                response.headers_mut().insert(
                    hyper::header::CONNECTION,
                    hyper::header::HeaderValue::from_static("close"),
                );
                return Ok::<_, Infallible>(response);
            }

            assert_eq!(request.method(), "RDG_IN_DATA");
            request.into_body().collect().await.expect("read RDG IN request");
            Ok(response(StatusCode::OK, Bytes::new()))
        });
        http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await
            .expect("serve gateway connection");
    }
}

async fn serve_http_proxy(listener: TcpListener, gateway_addr: core::net::SocketAddr) {
    let mut connections = Vec::new();
    for _ in 0..2 {
        let (stream, _) = listener.accept().await.expect("accept proxy client");
        connections.push(tokio::spawn(serve_http_proxy_connection(stream, gateway_addr)));
    }
    for connection in connections {
        connection.await.expect("proxy connection task");
    }
}

async fn serve_http_proxy_connection(mut stream: TcpStream, gateway_addr: core::net::SocketAddr) {
    let request = read_http_headers(&mut stream).await;
    assert!(request.starts_with(b"CONNECT localhost:"));
    let authorization = b"Proxy-Authorization: Basic dXNlcjpwYXNzd29yZA==\r\n";
    assert!(
        request
            .windows(authorization.len())
            .any(|header| header == authorization)
    );
    stream
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await
        .expect("send CONNECT response");

    let mut gateway = TcpStream::connect(gateway_addr).await.expect("connect gateway");
    tokio::io::copy_bidirectional(&mut stream, &mut gateway)
        .await
        .expect("forward proxy tunnel");
}

async fn serve_socks_proxy(listener: TcpListener, gateway_addr: core::net::SocketAddr, remote_dns: bool) {
    let mut connections = Vec::new();
    for _ in 0..2 {
        let (stream, _) = listener.accept().await.expect("accept proxy client");
        connections.push(tokio::spawn(serve_socks_proxy_connection(
            stream,
            gateway_addr,
            remote_dns,
        )));
    }
    for connection in connections {
        connection.await.expect("proxy connection task");
    }
}

async fn serve_socks_proxy_connection(mut stream: TcpStream, gateway_addr: core::net::SocketAddr, remote_dns: bool) {
    assert_eq!(stream.read_u8().await.expect("read SOCKS version"), 5);
    let methods_len = usize::from(stream.read_u8().await.expect("read SOCKS methods length"));
    let mut methods = vec![0; methods_len];
    stream.read_exact(&mut methods).await.expect("read SOCKS methods");
    assert!(methods.contains(&2));
    stream
        .write_all(&[5, 2])
        .await
        .expect("select SOCKS password authentication");

    assert_eq!(stream.read_u8().await.expect("read SOCKS auth version"), 1);
    let username_len = usize::from(stream.read_u8().await.expect("read SOCKS username length"));
    let mut username = vec![0; username_len];
    stream.read_exact(&mut username).await.expect("read SOCKS username");
    let password_len = usize::from(stream.read_u8().await.expect("read SOCKS password length"));
    let mut password = vec![0; password_len];
    stream.read_exact(&mut password).await.expect("read SOCKS password");
    assert!(username.as_slice() == b"user");
    assert!(password.as_slice() == b"password");
    stream.write_all(&[1, 0]).await.expect("accept SOCKS credentials");

    assert_eq!(stream.read_u8().await.expect("read SOCKS request version"), 5);
    assert_eq!(stream.read_u8().await.expect("read SOCKS command"), 1);
    assert_eq!(stream.read_u8().await.expect("read SOCKS reserved byte"), 0);
    let address_type = stream.read_u8().await.expect("read SOCKS address type");
    assert_eq!(address_type == 3, remote_dns);
    assert!([1, 3, 4].contains(&address_type), "unexpected SOCKS address type");
    if address_type == 1 {
        let mut address = [0; 4];
        stream.read_exact(&mut address).await.expect("read SOCKS IPv4 target");
    } else if address_type == 3 {
        let length = usize::from(stream.read_u8().await.expect("read SOCKS domain length"));
        let mut domain = vec![0; length];
        stream.read_exact(&mut domain).await.expect("read SOCKS domain target");
    } else {
        let mut address = [0; 16];
        stream.read_exact(&mut address).await.expect("read SOCKS IPv6 target");
    }
    let mut port = [0; 2];
    stream.read_exact(&mut port).await.expect("read SOCKS target port");
    stream
        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
        .await
        .expect("send SOCKS success response");

    let mut gateway = TcpStream::connect(gateway_addr).await.expect("connect gateway");
    tokio::io::copy_bidirectional(&mut stream, &mut gateway)
        .await
        .expect("forward proxy tunnel");
}

async fn read_http_headers(stream: &mut TcpStream) -> Vec<u8> {
    let mut headers = Vec::new();
    while !headers.ends_with(b"\r\n\r\n") {
        headers.push(stream.read_u8().await.expect("read proxy request"));
    }
    headers
}

fn response(status: StatusCode, body: Bytes) -> Response<TestBody> {
    let mut response = Response::new(Full::new(body).boxed());
    *response.status_mut() = status;
    response
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
