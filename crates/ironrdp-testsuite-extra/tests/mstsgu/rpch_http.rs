use core::fmt;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

pub(crate) type Error = ironrdp_error::Error<GwErrorKind>;

#[derive(Debug)]
pub(crate) enum GwErrorKind {
    PacketEof,
    Custom,
    Encode,
    Decode,
}

pub(crate) trait GwErrorExt {
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

#[path = "../../../ironrdp-mstsgu/src/rpc_transport.rs"]
mod rpc_transport;

use rpc_transport::{RpchRequestHead, drain_body, read_rpch_response_head, write_rpch_request_head};

#[tokio::test]
async fn request_head_contains_rpch_routing_and_authentication_fields() {
    let (mut client, mut server) = tokio::io::duplex(4096);
    let write = tokio::spawn(async move {
        write_rpch_request_head(
            &mut client,
            RpchRequestHead {
                method: "RPC_IN_DATA",
                host: "gateway.example",
                target: "target.example:3389",
                content_length: 128 * 1024,
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
            "RPC_IN_DATA /rpc/rpcproxy.dll?target.example:3389 HTTP/1.1\r\n",
            "Accept: application/rpc\r\n",
            "Cache-Control: no-cache\r\n",
            "Connection: Keep-Alive\r\n",
            "Content-Length: 131072\r\n",
            "Host: gateway.example\r\n",
            "Pragma: No-cache\r\n",
            "Pragma: ResourceTypeUuid=44e265dd-7daf-42cd-8560-3cdb6e7a2729\r\n",
            "User-Agent: MSRPC\r\n",
            "Pragma: SessionId=12345678-1234-1234-1234-123456789abc\r\n",
            "Expect: 100-continue\r\n",
            "Authorization: Negotiate token\r\n",
            "Cookie: session=value\r\n",
            "\r\n",
        )
    );
}

#[tokio::test]
async fn response_head_preserves_authentication_challenges_and_body_length() {
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
async fn request_head_rejects_invalid_values_before_writing() {
    for head in [
        RpchRequestHead {
            method: "RPC_IN_DATA",
            host: "gateway.example",
            target: "target.example:3389\r\nX-Injected: value",
            content_length: 128 * 1024,
            authorization: None,
            cookie: None,
            session_id: None,
            expect_continue: true,
        },
        RpchRequestHead {
            method: "RPC_IN_DATA",
            host: "gateway.example",
            target: "target.example:1234567",
            content_length: 128 * 1024,
            authorization: None,
            cookie: None,
            session_id: None,
            expect_continue: true,
        },
        RpchRequestHead {
            method: "RPC_IN_DATA",
            host: "gateway.example",
            target: "2001:db8::1:3389",
            content_length: 128 * 1024,
            authorization: None,
            cookie: None,
            session_id: None,
            expect_continue: true,
        },
        RpchRequestHead {
            method: "RPC_OUT_DATA",
            host: "gateway.example",
            target: "target.example:3389",
            content_length: 76,
            authorization: Some("Negotiate token\r\nX-Injected: value"),
            cookie: None,
            session_id: None,
            expect_continue: false,
        },
        RpchRequestHead {
            method: "RPC_OUT_DATA",
            host: "gateway.example",
            target: "target.example:3389",
            content_length: 76,
            authorization: None,
            cookie: Some("session=value\r\nX-Injected: value"),
            session_id: None,
            expect_continue: false,
        },
        RpchRequestHead {
            method: "RPC_OUT_DATA",
            host: "gateway.example",
            target: "target.example:3389",
            content_length: 76,
            authorization: None,
            cookie: None,
            session_id: Some("not-a-uuid"),
            expect_continue: false,
        },
        RpchRequestHead {
            method: "RPC_OUT_DATA",
            host: "gateway.example\r\nX-Injected: value",
            target: "target.example:3389",
            content_length: 76,
            authorization: None,
            cookie: None,
            session_id: None,
            expect_continue: false,
        },
    ] {
        let (mut client, mut server) = tokio::io::duplex(4096);
        assert!(write_rpch_request_head(&mut client, head).await.is_err());
        drop(client);
        let mut bytes = Vec::new();
        server.read_to_end(&mut bytes).await.expect("read rejected request");
        assert!(bytes.is_empty());
    }
}

#[tokio::test]
async fn request_head_rejects_invalid_rpch_content_lengths() {
    for (method, content_length) in [
        ("RPC_IN_DATA", 128 * 1024 - 1),
        ("RPC_IN_DATA", 2 * 1024 * 1024 * 1024 + 1),
        ("RPC_OUT_DATA", 75),
        ("RPC_OUT_DATA", 77),
    ] {
        let (mut client, _) = tokio::io::duplex(4096);
        assert!(
            write_rpch_request_head(
                &mut client,
                RpchRequestHead {
                    method,
                    host: "gateway.example",
                    target: "target.example:3389",
                    content_length,
                    authorization: None,
                    cookie: None,
                    session_id: None,
                    expect_continue: false,
                },
            )
            .await
            .is_err()
        );
    }
}

#[tokio::test]
async fn request_head_rejects_expect_continue_for_out_channel() {
    let (mut client, _) = tokio::io::duplex(4096);
    assert!(
        write_rpch_request_head(
            &mut client,
            RpchRequestHead {
                method: "RPC_OUT_DATA",
                host: "gateway.example",
                target: "target.example:3389",
                content_length: 76,
                authorization: None,
                cookie: None,
                session_id: None,
                expect_continue: true,
            },
        )
        .await
        .is_err()
    );
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
async fn response_head_rejects_transfer_encoding() {
    let (mut client, mut server) = tokio::io::duplex(4096);
    let server_task = tokio::spawn(async move {
        server
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\n\
                  Transfer-Encoding: chunked\r\n\
                  \r\n",
            )
            .await
            .expect("write chunked response head");
    });

    assert!(read_rpch_response_head(&mut client).await.is_err());
    server_task.await.expect("join response writer");
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
