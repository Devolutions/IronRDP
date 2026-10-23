//! Raw HTTP/1 framing for legacy RPC-over-HTTP gateway channels.

use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, ReadHalf, WriteHalf};

use crate::{Error, GwErrorKind};

const MAX_RESPONSE_HEADERS: usize = 16 * 1024;
const MAX_AUTH_RESPONSE_BODY: u32 = 16 * 1024;
const MAX_COOKIES: usize = 16;
const MAX_COOKIE_HEADER: usize = 4 * 1024;

/// The TS Proxy RPC interface UUID advertised in `Pragma: ResourceTypeUuid`.
///
/// This lets the RPC proxy route the channel to the gateway service
/// ([MS-TSGU] 1.9.1).
const TSPROXY_RESOURCE_TYPE_UUID: &str = "44e265dd-7daf-42cd-8560-3cdb6e7a2729";

const MIN_IN_CONTENT_LENGTH: u32 = 128 * 1024;
const MAX_IN_CONTENT_LENGTH: u32 = 2 * 1024 * 1024 * 1024;

/// A parsed HTTP/1 response head from an RPCH proxy.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RpchResponseHead {
    pub(crate) status: u16,
    pub(crate) reason: String,
    pub(crate) content_length: Option<u32>,
    pub(crate) content_type: Option<String>,
    pub(crate) www_authenticate: Vec<String>,
    /// `name=value` pairs from `Set-Cookie` headers, without attributes.
    pub(crate) set_cookies: Vec<String>,
}

impl RpchResponseHead {
    /// Returns the RPC-Error code carried by an RPC proxy error response.
    ///
    /// [MS-RPCH] 2.1.2.1.3 and 3.2.2.5.11.
    pub(crate) fn rpc_error(&self) -> Option<u32> {
        let code = self.reason.strip_prefix("RPC Error: ")?;
        let code = code.split_once(',').map_or(code, |(code, _)| code).trim();
        u32::from_str_radix(code, 16).ok()
    }
}

/// Cookies a proxy or load balancer set on earlier channel responses.
///
/// Every later IN, OUT, and successor channel request returns them, so a load balancer keeps routing
/// the virtual connection to the same RPC proxy.
/// Cookie attributes such as domain and expiry are not interpreted; the jar lives for one virtual connection
/// to one gateway endpoint.
#[derive(Clone, Debug, Default)]
pub(crate) struct RpchCookieJar {
    cookies: Vec<(String, String)>,
}

impl RpchCookieJar {
    /// Records `Set-Cookie` pairs from a response, replacing values of cookies with the same name.
    pub(crate) fn update(&mut self, set_cookies: &[String]) -> Result<(), Error> {
        for cookie in set_cookies {
            let (name, value) = cookie
                .split_once('=')
                .ok_or_else(|| Error::new("malformed rpch Set-Cookie", GwErrorKind::Decode))?;
            match self.cookies.iter_mut().find(|(existing, _)| existing == name) {
                Some((_, existing)) => value.clone_into(existing),
                None => self.cookies.push((name.to_owned(), value.to_owned())),
            }
        }
        if self.cookies.len() > MAX_COOKIES || self.header().is_some_and(|header| header.len() > MAX_COOKIE_HEADER) {
            return Err(Error::new("rpch cookies exceed limit", GwErrorKind::Decode));
        }
        Ok(())
    }

    /// Returns the `Cookie` request header value, if any cookie was set.
    pub(crate) fn header(&self) -> Option<String> {
        if self.cookies.is_empty() {
            return None;
        }
        let pairs: Vec<String> = self
            .cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        Some(pairs.join("; "))
    }
}

/// Parameters for an RPCH IN or OUT channel request head.
pub(crate) struct RpchRequestHead<'a> {
    pub(crate) method: &'a str,
    pub(crate) host: &'a str,
    pub(crate) target: &'a str,
    pub(crate) content_length: u32,
    pub(crate) authorization: Option<&'a str>,
    pub(crate) cookie: Option<&'a str>,
    /// The HTTP-level session identifier for either channel.
    ///
    /// See [MS-RPCH] 2.1.2.1.1 and 2.1.2.1.2.
    pub(crate) session_id: Option<&'a str>,
    pub(crate) expect_continue: bool,
}

/// An RPCH IN request whose streaming body is authorized after authentication.
///
/// Intermediate authentication legs send `Content-Length: 0`, because proxies may wait for a
/// reserved body before answering with the next challenge.
/// The leg that reserves the full body uses `Expect: 100-continue`, so no RPCH stream bytes are
/// sent until the proxy accepts the request.
pub(crate) struct RpchInRequest<S> {
    stream: S,
    host: String,
    target: String,
    content_length: u32,
    session_id: Option<String>,
    cookie: Option<String>,
    remaining: u32,
    body_reserved: bool,
    body_authorized: bool,
}

impl<S> RpchInRequest<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Opens the IN channel without sending its streaming body.
    ///
    /// `reserve_body` selects between an empty authentication leg and the full streaming request.
    #[expect(
        clippy::too_many_arguments,
        reason = "each argument is one IN channel request header input"
    )]
    pub(crate) async fn open(
        stream: S,
        host: &str,
        target: &str,
        content_length: u32,
        session_id: Option<&str>,
        cookie: Option<String>,
        authorization: Option<&str>,
        reserve_body: bool,
    ) -> Result<Self, Error> {
        let request = Self {
            stream,
            host: host.to_owned(),
            target: target.to_owned(),
            content_length,
            session_id: session_id.map(str::to_owned),
            cookie,
            remaining: 0,
            body_reserved: false,
            body_authorized: false,
        };
        request.retry(authorization, reserve_body).await
    }

    /// Returns whether the current request head reserves the streaming body.
    pub(crate) const fn body_reserved(&self) -> bool {
        self.body_reserved
    }

    /// Reads the response to the current request head.
    ///
    /// Bounded authentication challenge bodies are drained before a retry can reuse the connection.
    pub(crate) async fn receive_response(&mut self) -> Result<RpchResponseHead, Error> {
        let response = read_rpch_response_head(&mut self.stream).await?;
        // Challenges and final responses to empty legs are complete; drain them so the connection can be reused.
        if response.status == 401 || (!self.body_reserved && response.status >= 200) {
            let length = response.content_length.ok_or_else(|| {
                Error::new(
                    "rpch authentication response missing content length",
                    GwErrorKind::Decode,
                )
            })?;
            drain_body(&mut self.stream, length).await?;
        } else if response.status == 100 {
            if !self.body_reserved {
                return Err(Error::new(
                    "rpch IN continue without reserved body",
                    GwErrorKind::Decode,
                ));
            }
            self.body_authorized = true;
        }
        Ok(response)
    }

    /// Sets the `Cookie` header value sent with later request heads.
    pub(crate) fn set_cookie(&mut self, cookie: Option<String>) {
        self.cookie = cookie;
    }

    /// Sends the next request head after an authentication response.
    ///
    /// The request body remains blocked until the proxy returns `100 Continue` to a reserving head.
    pub(crate) async fn retry(mut self, authorization: Option<&str>, reserve_body: bool) -> Result<Self, Error> {
        if self.body_authorized {
            return Err(Error::new("rpch IN retry after body", GwErrorKind::Connect));
        }

        let content_length = if reserve_body { self.content_length } else { 0 };
        write_rpch_request_head(
            &mut self.stream,
            RpchRequestHead {
                method: "RPC_IN_DATA",
                host: &self.host,
                target: &self.target,
                content_length,
                authorization,
                cookie: self.cookie.as_deref(),
                session_id: self.session_id.as_deref(),
                expect_continue: reserve_body,
            },
        )
        .await?;
        self.body_reserved = reserve_body;
        self.remaining = content_length;

        Ok(self)
    }
}

impl<S> RpchInRequest<S>
where
    S: AsyncRead + AsyncWrite,
{
    /// Splits an accepted IN request into its body writer and the read half of the connection.
    ///
    /// The proxy writes nothing on an accepted IN channel, so any bytes on the read half are an error response.
    ///
    /// [MS-RPCH] 2.1.2.1.3 and 3.2.2.5.11.
    pub(crate) fn split(self) -> Result<(RpchInRequest<WriteHalf<S>>, ReadHalf<S>), Error> {
        if !self.body_authorized {
            return Err(Error::new("rpch IN split before authentication", GwErrorKind::Connect));
        }
        let (read, write) = tokio::io::split(self.stream);
        Ok((
            RpchInRequest {
                stream: write,
                host: self.host,
                target: self.target,
                content_length: self.content_length,
                session_id: self.session_id,
                cookie: self.cookie,
                remaining: self.remaining,
                body_reserved: self.body_reserved,
                body_authorized: self.body_authorized,
            },
            read,
        ))
    }
}

impl<S> RpchInRequest<S>
where
    S: AsyncWrite + Unpin,
{
    /// Writes bytes into the committed streaming body.
    pub(crate) async fn write_body(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if !self.body_authorized {
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

/// Writes an RPCH IN or OUT channel request head without committing any body bytes.
///
/// Callers that set `expect_continue` must wait for `100 Continue` before
/// writing an IN channel body.
/// Callers are responsible for writing exactly `content_length` body bytes.
pub(crate) async fn write_rpch_request_head<S>(stream: &mut S, head: RpchRequestHead<'_>) -> Result<(), Error>
where
    S: AsyncWrite + Unpin,
{
    let session_id = validate_request_head(&head)?;

    let mut request = format!(
        "{} /rpc/rpcproxy.dll?{} HTTP/1.1\r\n\
         Accept: application/rpc\r\n\
         Cache-Control: no-cache\r\n\
         Connection: Keep-Alive\r\n\
         Content-Length: {}\r\n\
         Host: {}\r\n\
         Pragma: No-cache\r\n\
         Pragma: ResourceTypeUuid={TSPROXY_RESOURCE_TYPE_UUID}\r\n\
         User-Agent: MSRPC\r\n",
        head.method, head.target, head.content_length, head.host
    );
    if let Some(session_id) = session_id {
        request.push_str("Pragma: SessionId=");
        request.push_str(&session_id.to_string());
        request.push_str("\r\n");
    }
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

fn validate_request_head(head: &RpchRequestHead<'_>) -> Result<Option<uuid::Uuid>, Error> {
    if !matches!(head.method, "RPC_IN_DATA" | "RPC_OUT_DATA") {
        return Err(Error::new("invalid RPCH request method", GwErrorKind::Encode));
    }
    if head.expect_continue && head.method != "RPC_IN_DATA" {
        return Err(Error::new("invalid RPCH expect-continue method", GwErrorKind::Encode));
    }
    if !is_valid_header_value(head.host)
        || !is_valid_target(head.target)
        || !head.authorization.is_none_or(is_valid_header_value)
        || !head.cookie.is_none_or(is_valid_header_value)
    {
        return Err(Error::new("invalid RPCH request header value", GwErrorKind::Encode));
    }
    let session_id = head
        .session_id
        .map(uuid::Uuid::parse_str)
        .transpose()
        .map_err(|_| Error::new("invalid RPCH session ID", GwErrorKind::Encode))?;

    match head.method {
        "RPC_IN_DATA"
            if head.content_length != 0
                && !(MIN_IN_CONTENT_LENGTH..=MAX_IN_CONTENT_LENGTH).contains(&head.content_length) =>
        {
            Err(Error::new("invalid RPCH IN content length", GwErrorKind::Encode))
        }
        "RPC_OUT_DATA" if !matches!(head.content_length, 76 | 120) => {
            #[cfg(test)]
            if head.content_length == 0 {
                return Ok(session_id);
            }

            Err(Error::new("invalid RPCH OUT content length", GwErrorKind::Encode))
        }
        _ => Ok(session_id),
    }
}

fn is_valid_header_value(value: &str) -> bool {
    !value.is_empty() && !value.bytes().any(|byte| byte.is_ascii_control())
}

pub(crate) fn is_valid_target(target: &str) -> bool {
    let Some((server, port)) = target.rsplit_once(':') else {
        return false;
    };

    is_valid_header_value(server)
        && server.len() < 1024
        && server
            .bytes()
            .all(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'/' | b'?' | b'#' | b':'))
        && (1..=6).contains(&port.len())
        && port.bytes().all(|byte| byte.is_ascii_digit())
        && port.parse::<u16>().is_ok_and(|port| port != 0)
}

/// Drains exactly `length` body bytes after an authentication response.
pub(crate) async fn drain_body<S>(stream: &mut S, length: u32) -> Result<(), Error>
where
    S: AsyncRead + Unpin,
{
    if length > MAX_AUTH_RESPONSE_BODY {
        return Err(Error::new(
            "rpch authentication response body exceeds limit",
            GwErrorKind::Decode,
        ));
    }

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
    let (version, rest) = status_line
        .split_once(' ')
        .ok_or_else(|| Error::new("missing rpch response status", GwErrorKind::Decode))?;
    if !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err(Error::new("invalid rpch HTTP version", GwErrorKind::Decode));
    }
    let (status, reason) = rest.split_once(' ').unwrap_or((rest, ""));
    let status: u16 = status
        .parse()
        .map_err(|error| custom_err!("parse rpch response status", error))?;
    if !(100..=599).contains(&status) {
        return Err(Error::new("invalid rpch response status", GwErrorKind::Decode));
    }

    let mut content_length = None;
    let mut content_type = None;
    let mut www_authenticate = Vec::new();
    let mut set_cookies = Vec::new();
    let mut has_transfer_encoding = false;
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
        } else if name.eq_ignore_ascii_case("Set-Cookie") {
            let pair = value.split_once(';').map_or(value, |(pair, _)| pair).trim();
            if !pair.contains('=') || !is_valid_header_value(pair) || pair.contains(';') {
                return Err(Error::new("malformed rpch Set-Cookie", GwErrorKind::Decode));
            }
            set_cookies.push(pair.to_owned());
        } else if name.eq_ignore_ascii_case("Transfer-Encoding") {
            has_transfer_encoding = true;
        }
    }

    if has_transfer_encoding {
        return Err(Error::new(
            "unsupported RPCH response transfer encoding",
            GwErrorKind::Decode,
        ));
    }

    Ok(RpchResponseHead {
        status,
        reason: reason.trim().to_owned(),
        content_length,
        content_type,
        www_authenticate,
        set_cookies,
    })
}
