//! Raw HTTP/1 framing for legacy RPC-over-HTTP gateway channels.

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::{Error, GwErrorKind};

const MAX_RESPONSE_HEADERS: usize = 16 * 1024;

/// The TS Proxy RPC interface UUID advertised in `Pragma: ResourceTypeUuid`.
///
/// This lets the RPC proxy route the channel to the gateway service
/// ([MS-TSGU] 2.2.2.1).
const TSPROXY_RESOURCE_TYPE_UUID: &str = "44e265dd-7daf-42cd-8560-3cdb6e7a2729";

/// A parsed HTTP/1 response head from an RPCH proxy.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RpchResponseHead {
    pub(crate) status: u16,
    pub(crate) content_length: Option<u32>,
    pub(crate) content_type: Option<String>,
    pub(crate) www_authenticate: Vec<String>,
}

/// An RPCH IN request whose streaming body is committed only after authentication.
///
/// Authentication probes carry no body; the authenticated request carries the
/// channel-lifetime content length ([MS-RPCH] 3.2.2.2.1).
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
    /// The HTTP-level session identifier pairing the OUT and IN channels.
    ///
    /// It is sent on the OUT channel only ([MS-RPCH] 2.1.3.1).
    pub(crate) session_id: Option<&'a str>,
    pub(crate) expect_continue: bool,
}

impl<S> RpchInRequest<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Opens the IN channel with an authentication probe and no body.
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

    /// Reads the response to the current request head.
    ///
    /// A 401 body is drained so the connection remains clean for an
    /// authentication retry.
    pub(crate) async fn receive_response(&mut self) -> Result<RpchResponseHead, Error> {
        let response = read_rpch_response_head(&mut self.stream).await?;
        if response.status == 401
            && let Some(length) = response.content_length
        {
            drain_body(&mut self.stream, length).await?;
        }
        Ok(response)
    }

    /// Retries an unauthenticated request head with new authorization.
    ///
    /// A nonzero content length commits the streaming body; a zero length is a
    /// further authentication probe.
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

    /// Writes bytes to an authenticated IN channel body.
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
/// The caller must wait for `100 Continue` before writing an IN channel body
/// so an authentication retry never replays RPC bytes.
pub(crate) async fn write_rpch_request_head<S>(stream: &mut S, head: RpchRequestHead<'_>) -> Result<(), Error>
where
    S: AsyncWrite + Unpin,
{
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

/// Reads one bounded HTTP/1 response head without consuming its body.
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

/// Drains exactly `length` body bytes after an authentication response.
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
