#![allow(unused_crate_dependencies)]

use core::fmt;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

type Error = ironrdp_error::Error<GwErrorKind>;

#[derive(Debug)]
enum GwErrorKind {
    Connect,
    PacketEof,
    Custom,
    Encode,
    Decode,
}

trait GwErrorExt {
    fn custom<E>(context: &'static str, error: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static;
}

impl GwErrorExt for Error {
    fn custom<E>(context: &'static str, error: E) -> Self
    where
        E: core::error::Error + Sync + Send + 'static,
    {
        Self::new(context, GwErrorKind::Custom).with_source(error)
    }
}

impl fmt::Display for GwErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Connect => "connection error",
            Self::PacketEof => "packet EOF",
            Self::Custom => "custom",
            Self::Encode => "encode",
            Self::Decode => "decode",
        })
    }
}

impl core::error::Error for GwErrorKind {}

macro_rules! custom_err {
    ( $context:expr, $source:expr $(,)? ) => {{ <$crate::Error as $crate::GwErrorExt>::custom($context, $source) }};
}

#[path = "../src/rpc_transport.rs"]
mod rpc_transport;

use rpc_transport::{RpchInRequest, RpchRequestHead, drain_body, read_rpch_response_head, write_rpch_request_head};

#[tokio::test]
async fn request_head_contains_rpch_routing_and_authentication_fields() {
    let (mut client, mut server) = tokio::io::duplex(4096);
    let write = tokio::spawn(async move {
        write_rpch_request_head(
            &mut client,
            RpchRequestHead {
                method: "RPC_OUT_DATA",
                host: "gateway.example",
                target: "target.example:3389",
                content_length: 42,
                authorization: Some("Negotiate token"),
                cookie: Some("session=value"),
                session_id: Some("{12345678-1234-1234-1234-123456789abc}"),
                expect_continue: true,
            },
        )
        .await
        .expect("write request head");
    });

    let request = read_head(&mut server).await;
    write.await.expect("join request writer");

    assert_eq!(
        request,
        concat!(
            "RPC_OUT_DATA /rpc/rpcproxy.dll?target.example:3389 HTTP/1.1\r\n",
            "Accept: application/rpc\r\n",
            "Cache-Control: no-cache\r\n",
            "Connection: Keep-Alive\r\n",
            "Content-Length: 42\r\n",
            "Host: gateway.example\r\n",
            "Pragma: ResourceTypeUuid=44e265dd-7daf-42cd-8560-3cdb6e7a2729, ",
            "SessionId={12345678-1234-1234-1234-123456789abc}\r\n",
            "User-Agent: MSRPC\r\n",
            "Expect: 100-continue\r\n",
            "Authorization: Negotiate token\r\n",
            "Cookie: session=value\r\n",
            "\r\n",
        )
    );
}

#[tokio::test]
async fn response_head_preserves_authentication_challenges_and_body() {
    let (mut client, mut server) = tokio::io::duplex(4096);
    let server_task = tokio::spawn(async move {
        server
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\n\
                  Content-Length: 4\r\n\
                  Content-Type: text/html\r\n\
                  WWW-Authenticate: NTLM\r\n\
                  WWW-Authenticate: Negotiate\r\n\
                  \r\n\
                  deny",
            )
            .await
            .expect("write response");
    });

    let response = read_rpch_response_head(&mut client).await.expect("read response head");
    assert_eq!(response.status, 401);
    assert_eq!(response.content_length, Some(4));
    assert_eq!(response.content_type.as_deref(), Some("text/html"));
    assert_eq!(response.www_authenticate, ["NTLM", "Negotiate"]);

    let mut body = [0; 4];
    client.read_exact(&mut body).await.expect("read response body");
    assert_eq!(body, *b"deny");
    server_task.await.expect("join response writer");
}

#[tokio::test]
async fn response_head_rejects_oversized_headers() {
    let (mut client, mut server) = tokio::io::duplex(20 * 1024);
    let server_task = tokio::spawn(async move {
        server
            .write_all(&[b'x'; 16 * 1024])
            .await
            .expect("write oversized response");
    });

    let error = read_rpch_response_head(&mut client)
        .await
        .expect_err("reject oversized response head");
    assert_eq!(error.kind().to_string(), "decode");
    server_task.await.expect("join response writer");
}

#[tokio::test]
async fn drain_body_preserves_following_response_bytes() {
    let (mut client, mut server) = tokio::io::duplex(4096);
    let server_task = tokio::spawn(async move {
        server
            .write_all(b"denyHTTP/1.1 200 OK\r\n\r\n")
            .await
            .expect("write response body and head");
    });

    drain_body(&mut client, 4).await.expect("drain body");
    let response = read_rpch_response_head(&mut client)
        .await
        .expect("read following response");
    assert_eq!(response.status, 200);
    server_task.await.expect("join response writer");
}

#[tokio::test]
async fn in_request_commits_body_only_after_authenticated_retry() {
    let (client, mut server) = tokio::io::duplex(4096);
    let server_task = tokio::spawn(async move {
        let probe = read_head(&mut server).await;
        assert!(probe.contains("Content-Length: 0\r\n"), "probe: {probe}");

        server
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\n\
                  Content-Length: 4\r\n\
                  WWW-Authenticate: NTLM\r\n\
                  \r\n\
                  deny",
            )
            .await
            .expect("write authentication response");

        let retry = read_head(&mut server).await;
        assert!(retry.contains("Content-Length: 4\r\n"), "retry: {retry}");
        assert!(retry.contains("Authorization: NTLM token\r\n"), "retry: {retry}");

        let mut body = [0; 4];
        server.read_exact(&mut body).await.expect("read committed body");
        assert_eq!(body, *b"B1!!");
    });

    let mut request = RpchInRequest::open(client, "gateway.example", "target.example:3389", None)
        .await
        .expect("open authentication probe");
    assert!(request.write_body(b"B1!!").await.is_err());

    let response = request.receive_response().await.expect("read authentication response");
    assert_eq!(response.status, 401);

    let mut request = request
        .retry(Some("NTLM token"), 4)
        .await
        .expect("write authenticated retry");
    request.write_body(b"B1!!").await.expect("write body");
    request.flush().await.expect("flush body");
    assert_eq!(request.remaining(), 0);

    server_task.await.expect("join request server");
}

async fn read_head(stream: &mut tokio::io::DuplexStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read_exact(&mut byte).await.expect("read head byte");
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            return String::from_utf8(head).expect("decode request head");
        }
        assert!(head.len() <= 16 * 1024, "request head exceeds limit");
    }
}
