#![allow(unused_crate_dependencies)]

use ironrdp_mstsgu::test_support::{
    proxy_debug, proxy_summary, proxy_uses_basic_authorization, validate_proxy_response,
};

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
        "http://user@proxy.example.test",
        "http://:password@proxy.example.test",
    ] {
        assert!(proxy_summary("gateway.example.test", Some(proxy), None, None, None).is_err());
    }
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
