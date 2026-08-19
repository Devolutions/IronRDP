//! Raw HTTP/1 primitives for the non-replayable RPCH IN channel.

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::{Error, GwErrorKind};

const MAX_RESPONSE_HEADERS: usize = 16 * 1024;

/// The TS Proxy RPC interface UUID, advertised in the `Pragma: ResourceTypeUuid` header
/// so the RPC proxy routes the channel to the gateway service ([MS-TSGU] 2.2.2.1).
const TSPROXY_RESOURCE_TYPE_UUID: &str = "44e265dd-7daf-42cd-8560-3cdb6e7a2729";

/// A parsed HTTP/1 response head from the RPCH proxy.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RpchResponseHead {
    pub(crate) status: u16,
    pub(crate) content_length: Option<u32>,
    pub(crate) content_type: Option<String>,
    pub(crate) www_authenticate: Vec<String>,
}

/// An RPCH IN request whose streaming body is committed only once the channel is
/// authenticated. Authentication probes carry no body (`Content-Length: 0`); the final
/// request carries the channel-lifetime length, mirroring mstsc ([MS-RPCH] 3.2.2.2.1).
pub(crate) struct RpchInRequest<S> {
    stream: S,
    host: String,
    target: String,
    remaining: u32,
    body_committed: bool,
}

/// Parameters for an RPCH request head.
pub(crate) struct RpchRequestHead<'a> {
    pub(crate) method: &'a str,
    pub(crate) host: &'a str,
    pub(crate) target: &'a str,
    pub(crate) content_length: u32,
    pub(crate) authorization: Option<&'a str>,
    pub(crate) cookie: Option<&'a str>,
    /// The HTTP-level session identifier pairing the OUT and IN channels. Sent on the
    /// OUT channel only ([MS-RPCH] 2.1.3.1).
    pub(crate) session_id: Option<&'a str>,
    pub(crate) expect_continue: bool,
}

impl<S> RpchInRequest<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Opens the IN channel with an authentication probe (`Content-Length: 0`, no body).
    pub(crate) async fn open(
        mut stream: S,
        host: &str,
        target: &str,
        authorization: Option<&str>,
    ) -> Result<Self, Error> {
        write_rpch_request_head(
            &mut stream,
            RpchRequestHead {
                method: "RPC_IN_DATA",
                host,
                target,
                content_length: 0,
                authorization,
                cookie: None,
                session_id: None,
                expect_continue: false,
            },
        )
        .await?;
        Ok(Self {
            stream,
            host: host.to_owned(),
            target: target.to_owned(),
            remaining: 0,
            body_committed: false,
        })
    }

    /// Reads the response to the current request head. A 401 error body is drained so the
    /// connection stays clean for an authentication retry.
    pub(crate) async fn receive_response(&mut self) -> Result<RpchResponseHead, Error> {
        let response = read_rpch_response_head(&mut self.stream).await?;
        if response.status == 401
            && let Some(length) = response.content_length
        {
            drain_body(&mut self.stream, length).await?;
        }
        Ok(response)
    }

    /// Retries the request head with new authorization after a 401 response. When
    /// `content_length` is non-zero this is the final, authenticated request and the
    /// streaming body may follow; a zero length is another authentication probe.
    pub(crate) async fn retry(mut self, authorization: Option<&str>, content_length: u32) -> Result<Self, Error> {
        if self.body_committed {
            return Err(Error::new("rpch IN retry after body", GwErrorKind::Connect));
        }
        let host = self.host.clone();
        let target = self.target.clone();
        write_rpch_request_head(
            &mut self.stream,
            RpchRequestHead {
                method: "RPC_IN_DATA",
                host: &host,
                target: &target,
                content_length,
                authorization,
                cookie: None,
                session_id: None,
                expect_continue: false,
            },
        )
        .await?;
        self.remaining = content_length;
        self.body_committed = content_length > 0;
        Ok(self)
    }

    pub(crate) async fn write_body(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if !self.body_committed {
            return Err(Error::new("rpch IN body before authentication", GwErrorKind::Connect));
        }
        let length = u32::try_from(bytes.len()).map_err(|_| Error::new("rpch IN body length", GwErrorKind::Encode))?;
        if length > self.remaining {
            return Err(Error::new("rpch IN body exceeds content length", GwErrorKind::Encode));
        }
        self.stream
            .write_all(bytes)
            .await
            .map_err(|error| custom_err!("write rpch IN body", error))?;
        self.remaining -= length;
        Ok(())
    }

    pub(crate) const fn remaining(&self) -> u32 {
        self.remaining
    }

    /// Flushes buffered body bytes to the gateway.
    pub(crate) async fn flush(&mut self) -> Result<(), Error> {
        self.stream
            .flush()
            .await
            .map_err(|error| custom_err!("flush rpch IN body", error))
    }
}

/// Writes an RPCH request head without committing any body bytes.
///
/// The caller must wait for a `100 Continue` response before writing an IN
/// channel body so an authentication retry can never replay RPC bytes.
pub(crate) async fn write_rpch_request_head<S>(stream: &mut S, head: RpchRequestHead<'_>) -> Result<(), Error>
where
    S: AsyncWrite + Unpin,
{
    // The ResourceTypeUuid identifies the TS Proxy RPC interface to the gateway; the
    // SessionId pairs the OUT channel with its IN channel at the HTTP level. Both are
    // required by the RPC proxy ([MS-RPCH] 2.1.3.1, [MS-TSGU] 2.2.2.1).
    let mut pragma = format!("ResourceTypeUuid={TSPROXY_RESOURCE_TYPE_UUID}");
    if let Some(session_id) = head.session_id {
        pragma.push_str(", SessionId=");
        pragma.push_str(session_id);
    }
    let mut request = format!(
        "{} /rpc/rpcproxy.dll?{} HTTP/1.1\r\n\
         Accept: application/rpc\r\n\
         Cache-Control: no-cache\r\n\
         Connection: Keep-Alive\r\n\
         Content-Length: {}\r\n\
         Host: {}\r\n\
         Pragma: {}\r\n\
         User-Agent: MSRPC\r\n",
        head.method, head.target, head.content_length, head.host, pragma
    );
    if head.expect_continue {
        request.push_str("Expect: 100-continue\r\n");
    }
    if let Some(authorization) = head.authorization {
        request.push_str("Authorization: ");
        request.push_str(authorization);
        request.push_str("\r\n");
    }
    if let Some(cookie) = head.cookie {
        request.push_str("Cookie: ");
        request.push_str(cookie);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");

    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| custom_err!("write rpch request headers", error))?;
    stream
        .flush()
        .await
        .map_err(|error| custom_err!("flush rpch request headers", error))
}

/// Reads one bounded HTTP/1 response head without consuming the body. Callers that need to
/// re-use the connection after an error response (authentication retry) drain the body via
/// [`RpchInRequest::receive_gate`]; the OUT-channel success response keeps its body attached
/// because the body is the streaming RPC data.
pub(crate) async fn read_rpch_response_head<S>(stream: &mut S) -> Result<RpchResponseHead, Error>
where
    S: AsyncRead + Unpin,
{
    let mut bytes = Vec::with_capacity(512);
    loop {
        if bytes.len() == MAX_RESPONSE_HEADERS {
            return Err(Error::new("rpch response headers exceed limit", GwErrorKind::Decode));
        }
        let byte = stream
            .read_u8()
            .await
            .map_err(|error| custom_err!("read rpch response headers", error))?;
        bytes.push(byte);
        if bytes.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    parse_rpch_response_head(&bytes)
}

/// Drain exactly `length` body bytes (authentication error pages). Called after an error
/// response head so the connection is clean before the request is retried.
pub(crate) async fn drain_body<S>(stream: &mut S, length: u32) -> Result<(), Error>
where
    S: AsyncRead + Unpin,
{
    let mut remaining = usize::try_from(length).map_err(|_| Error::new("rpch body length", GwErrorKind::Decode))?;
    let mut chunk = [0u8; 4096];
    while remaining > 0 {
        let read_len = remaining.min(chunk.len());
        let read = stream
            .read(&mut chunk[..read_len])
            .await
            .map_err(|error| custom_err!("drain rpch body", error))?;
        if read == 0 {
            return Err(Error::new("rpch body truncated", GwErrorKind::PacketEof));
        }
        remaining -= read;
    }
    Ok(())
}

fn parse_rpch_response_head(bytes: &[u8]) -> Result<RpchResponseHead, Error> {
    let text = core::str::from_utf8(bytes).map_err(|error| custom_err!("decode rpch response headers", error))?;
    let mut lines = text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| Error::new("missing rpch response status", GwErrorKind::Decode))?;
    let mut status = status_line.split_ascii_whitespace();
    let version = status
        .next()
        .ok_or_else(|| Error::new("missing rpch HTTP version", GwErrorKind::Decode))?;
    if !version.starts_with("HTTP/") {
        return Err(Error::new("invalid rpch HTTP version", GwErrorKind::Decode));
    }
    let status = status
        .next()
        .ok_or_else(|| Error::new("missing rpch response status", GwErrorKind::Decode))?
        .parse()
        .map_err(|error| custom_err!("parse rpch response status", error))?;

    let mut content_length = None;
    let mut content_type = None;
    let mut www_authenticate = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| Error::new("malformed rpch response header", GwErrorKind::Decode))?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("Content-Length") {
            if content_length
                .replace(
                    value
                        .parse()
                        .map_err(|error| custom_err!("parse rpch content length", error))?,
                )
                .is_some()
            {
                return Err(Error::new("duplicate rpch content length", GwErrorKind::Decode));
            }
        } else if name.eq_ignore_ascii_case("Content-Type") {
            content_type = Some(value.to_owned());
        } else if name.eq_ignore_ascii_case("WWW-Authenticate") {
            www_authenticate.push(value.to_owned());
        }
    }

    Ok(RpchResponseHead {
        status,
        content_length,
        content_type,
        www_authenticate,
    })
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use super::*;

    #[tokio::test]
    async fn rpch_in_headers_hold_body_for_continue() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let write = tokio::spawn(async move {
            write_rpch_request_head(
                &mut client,
                RpchRequestHead {
                    method: "RPC_IN_DATA",
                    host: "gateway.example",
                    target: "target.example:3389",
                    content_length: 128 * 1024,
                    authorization: Some("NTLM token"),
                    cookie: Some("session=value"),
                    session_id: None,
                    expect_continue: true,
                },
            )
            .await
            .expect("write headers");
        });
        let mut request = Vec::new();
        server.read_to_end(&mut request).await.expect("read request");
        write.await.expect("join");
        let request = String::from_utf8(request).expect("ASCII request");
        assert!(request.contains("Expect: 100-continue\r\n"));
        assert!(request.ends_with("\r\n\r\n"));
        assert!(!request.contains("CONN/B1"));
    }

    #[tokio::test]
    async fn response_head_preserves_authentication_challenges() {
        let (mut client, mut server) = tokio::io::duplex(4096);
        let server = tokio::spawn(async move {
            server
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nWWW-Authenticate: NTLM\r\nWWW-Authenticate: Negotiate\r\n\r\n",
                )
                .await
                .expect("write response");
        });
        let response = read_rpch_response_head(&mut client).await.expect("read response");
        server.await.expect("join");
        assert_eq!(response.status, 401);
        assert_eq!(response.content_length, Some(0));
        assert_eq!(response.www_authenticate, ["NTLM", "Negotiate"]);
    }

    #[tokio::test]
    async fn in_request_commits_body_after_authenticated_retry() {
        let (client, mut server) = tokio::io::duplex(4096);
        let server_task = tokio::spawn(async move {
            // Authentication probe carries no body (Content-Length: 0).
            let head = read_head(&mut server).await;
            assert!(head.contains("Content-Length: 0"), "probe head: {head}");
            server
                .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 4\r\nWWW-Authenticate: NTLM\r\n\r\n")
                .await
                .expect("write 401");
            server.write_all(b"deny").await.expect("write 401 body");
            // The authenticated retry commits the streaming body.
            let head = read_head(&mut server).await;
            assert!(head.contains("Content-Length: 4"), "retry head: {head}");
            assert!(head.contains("Authorization: NTLM token"), "retry head: {head}");
            let mut body = [0; 4];
            server.read_exact(&mut body).await.expect("read body");
            assert_eq!(body, *b"B1!!");
        });

        let mut request = RpchInRequest::open(client, "gateway.example", "target.example:3389", None)
            .await
            .expect("open request");
        // The body cannot be written before the authenticated request.
        assert!(request.write_body(b"B1!!").await.is_err());
        let response = request.receive_response().await.expect("401");
        assert_eq!(response.status, 401);
        let mut request = request.retry(Some("NTLM token"), 4).await.expect("retry");
        request.write_body(b"B1!!").await.expect("write body");
        assert_eq!(request.remaining(), 0);
        server_task.await.expect("join");
    }

    #[tokio::test]
    async fn in_request_drains_401_body_before_retry() {
        let (client, mut server) = tokio::io::duplex(4096);
        let server_task = tokio::spawn(async move {
            let _ = read_head(&mut server).await;
            server
                .write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 4\r\nWWW-Authenticate: NTLM\r\n\r\ndeny")
                .await
                .expect("write 401 with body");
            // After the drained 401, the retried head must parse as a fresh request.
            let head = read_head(&mut server).await;
            assert!(head.starts_with("RPC_IN_DATA "), "retried head: {head}");
        });

        let request = RpchInRequest::open(client, "gateway.example", "target.example:3389", None)
            .await
            .expect("open request");
        let mut request = request;
        let response = request.receive_response().await.expect("401");
        assert_eq!(response.status, 401);
        let _ = request.retry(Some("NTLM token"), 0).await.expect("retry");
        server_task.await.expect("join");
    }

    async fn read_head(stream: &mut tokio::io::DuplexStream) -> String {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            stream.read_exact(&mut byte).await.expect("read head byte");
            head.push(byte[0]);
            if head.ends_with(b"\r\n\r\n") {
                return String::from_utf8(head).expect("utf8 head");
            }
            assert!(head.len() <= 16 * 1024, "head too large");
        }
    }
}
