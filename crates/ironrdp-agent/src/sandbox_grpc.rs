//! Pure-Rust client for WindowsSandboxServer `sandboxserver.SandboxCore`.
//!
//! Speaks unary gRPC over HTTP/2 on `\\.\pipe\wsandbox\<md5(user SID) as .NET Guid>`.
//! No .NET runtime or helper process is required.

use core::time::Duration;

use anyhow::{Context as _, bail};
use bytes::{BufMut as _, Bytes, BytesMut};
use http::{Method, Request, StatusCode};
use md5::{Digest as _, Md5};
use tokio::net::windows::named_pipe::ClientOptions;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::core::PWSTR;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const PIPE_BUSY_RETRY: Duration = Duration::from_millis(50);
const RPC_TIMEOUT: Duration = Duration::from_secs(15);
/// `ERROR_PIPE_BUSY` — another client still holds the pipe instance.
const ERROR_PIPE_BUSY: i32 = 231;

/// Resolve the per-user WindowsSandboxServer gRPC pipe path.
pub(crate) fn grpc_pipe_path() -> anyhow::Result<String> {
    let sid = current_user_sid_string()?;
    let digest = Md5::digest(sid.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest);
    let guid = guid_string_from_dotnet_bytes(&bytes);
    Ok(format!(r"\\.\pipe\wsandbox\{guid}"))
}

/// `EnumerateSandboxVMs` → sandbox id list.
pub(crate) fn enumerate_sandbox_vms() -> anyhow::Result<Vec<String>> {
    let payload = block_on(unary("EnumerateSandboxVMs", &[]))?;
    let (hr, ids, _) = parse_reply(&payload, ParseMode::Ids)?;
    ensure_ok(hr, "EnumerateSandboxVMs")?;
    Ok(ids)
}

/// `GetRdpClientConfig` → serialized RdpClientConfig XML.
pub(crate) fn get_rdp_client_config_xml(sandbox_id: &str) -> anyhow::Result<String> {
    let req = encode_string_field(1, sandbox_id);
    let payload = block_on(unary("GetRdpClientConfig", &req))?;
    let (hr, _, cfg) = parse_reply(&payload, ParseMode::Config)?;
    ensure_ok(hr, "GetRdpClientConfig")?;
    if cfg.is_empty() {
        bail!("empty RdpClientConfig from WindowsSandboxServer");
    }
    Ok(cfg)
}

/// `ShutdownSandbox`.
pub(crate) fn shutdown_sandbox(sandbox_id: &str) -> anyhow::Result<()> {
    let req = encode_string_field(1, sandbox_id);
    let payload = block_on(unary("ShutdownSandbox", &req))?;
    let (hr, _, _) = parse_reply(&payload, ParseMode::Config)?;
    ensure_ok(hr, "ShutdownSandbox")?;
    Ok(())
}

fn ensure_ok(hr: i32, method: &str) -> anyhow::Result<()> {
    if hr != 0 {
        bail!("{method} failed with hresult=0x{hr:08X}");
    }
    Ok(())
}

fn block_on<F, T>(fut: F) -> anyhow::Result<T>
where
    F: Future<Output = anyhow::Result<T>>,
{
    // Never call `Handle::block_on` on the outer `#[tokio::main]` runtime — that deadlocks.
    // Always drive the gRPC work on a nested current-thread runtime; when already inside
    // Tokio, park the worker with `block_in_place` first.
    let run = || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("create tokio runtime for Windows Sandbox gRPC")?;
        rt.block_on(fut)
    };

    match tokio::runtime::Handle::try_current() {
        Ok(_) => tokio::task::block_in_place(run),
        Err(_) => run(),
    }
}

async fn unary(method: &str, request_proto: &[u8]) -> anyhow::Result<Vec<u8>> {
    let path = grpc_pipe_path()?;
    let stream = open_pipe(&path).await?;

    let (sender, conn) = h2::client::Builder::new()
        .handshake(stream)
        .await
        .context("HTTP/2 handshake on WindowsSandboxServer pipe")?;
    let mut sender = sender;

    let conn_task = tokio::spawn(async move {
        let _ = conn.await;
    });

    let uri = format!("http://localhost/sandboxserver.SandboxCore/{method}");
    let request = Request::builder()
        .version(http::Version::HTTP_2)
        .method(Method::POST)
        .uri(&uri)
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .header("user-agent", "ironrdp-agent/grpc")
        .body(())
        .context("build gRPC request")?;

    let (response_future, mut send_stream) = sender.send_request(request, false).context("send gRPC headers")?;

    let mut frame = BytesMut::with_capacity(5 + request_proto.len());
    frame.put_u8(0); // not compressed
    frame.put_u32(u32::try_from(request_proto.len()).context("gRPC request too large")?);
    frame.extend_from_slice(request_proto);
    send_stream
        .send_data(frame.freeze(), true)
        .context("send gRPC request body")?;

    let response = tokio::time::timeout(RPC_TIMEOUT, response_future)
        .await
        .context("timeout waiting for gRPC response headers")?
        .context("await gRPC response headers")?;
    if response.status() != StatusCode::OK {
        bail!("WindowsSandboxServer HTTP status {} for {method}", response.status());
    }

    let mut body = response.into_body();
    let mut data = BytesMut::new();
    loop {
        let next = tokio::time::timeout(RPC_TIMEOUT, body.data())
            .await
            .context("timeout reading gRPC response body")?;
        match next {
            Some(chunk) => {
                let chunk = chunk.context("read gRPC response body")?;
                let len = chunk.len();
                data.extend_from_slice(&chunk);
                let _ = body.flow_control().release_capacity(len);
            }
            None => break,
        }
    }

    if let Some(trailers) = tokio::time::timeout(RPC_TIMEOUT, body.trailers())
        .await
        .context("timeout reading gRPC trailers")?
        .context("read gRPC trailers")?
    {
        if let Some(status) = trailers.get("grpc-status") {
            let status = status.to_str().unwrap_or("");
            if status != "0" {
                let message = trailers.get("grpc-message").and_then(|v| v.to_str().ok()).unwrap_or("");
                bail!("gRPC {method} status={status} {message}");
            }
        }
    }

    // Drop streams before joining so the server can finish the connection cleanly.
    drop(send_stream);
    drop(sender);
    let _ = tokio::time::timeout(Duration::from_secs(2), conn_task).await;

    decode_grpc_message(data.freeze())
}

async fn open_pipe(path: &str) -> anyhow::Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
    loop {
        match ClientOptions::new().open(path) {
            Ok(stream) => return Ok(stream),
            Err(err) if err.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(err).context(format!(
                        "open WindowsSandboxServer pipe {path} (still busy after timeout)"
                    ));
                }
                tokio::time::sleep(PIPE_BUSY_RETRY).await;
            }
            Err(err) => {
                return Err(err).context(format!(
                    "open WindowsSandboxServer pipe {path} (is Windows Sandbox / WindowsSandboxServer running?)"
                ));
            }
        }
    }
}

fn decode_grpc_message(data: Bytes) -> anyhow::Result<Vec<u8>> {
    if data.len() < 5 {
        bail!(
            "truncated gRPC response ({} bytes); expected length-prefixed message",
            data.len()
        );
    }
    if data[0] != 0 {
        bail!("compressed gRPC responses are not supported");
    }
    let len =
        usize::try_from(u32::from_be_bytes(data[1..5].try_into().expect("5 bytes"))).context("gRPC message length")?;
    let body = data
        .get(5..5 + len)
        .with_context(|| format!("gRPC message claims {len} bytes, have {}", data.len().saturating_sub(5)))?;
    Ok(body.to_vec())
}

#[derive(Clone, Copy)]
enum ParseMode {
    Ids,
    Config,
}

/// Parse SandboxCore reply messages (`hresult` sfixed32 field 1, string field 2).
fn parse_reply(data: &[u8], mode: ParseMode) -> anyhow::Result<(i32, Vec<String>, String)> {
    let mut i = 0;
    let mut hr = 0i32;
    let mut ids = Vec::new();
    let mut cfg = String::new();

    while i < data.len() {
        let (tag, ni) = read_varint(data, i)?;
        i = ni;
        let field = tag >> 3;
        let wire = tag & 7;

        match (field, wire) {
            (1, 5) => {
                // sfixed32
                if i + 4 > data.len() {
                    bail!("truncated sfixed32 hresult");
                }
                hr = i32::from_le_bytes(data[i..i + 4].try_into().expect("4 bytes"));
                i += 4;
            }
            (2, 2) => {
                let (len, ni) = read_varint(data, i)?;
                i = ni;
                let len = usize::try_from(len).context("string field length")?;
                if i + len > data.len() {
                    bail!("truncated protobuf string field");
                }
                let s = core::str::from_utf8(&data[i..i + len]).context("protobuf string utf-8")?;
                i += len;
                match mode {
                    ParseMode::Ids => ids.push(s.to_owned()),
                    ParseMode::Config => cfg = s.to_owned(),
                }
            }
            (_, 0) => {
                let (_, ni) = read_varint(data, i)?;
                i = ni;
            }
            (_, 1) => {
                i = i.checked_add(8).context("skip fixed64")?;
            }
            (_, 2) => {
                let (len, ni) = read_varint(data, i)?;
                i = ni
                    .checked_add(usize::try_from(len).context("skip len")?)
                    .context("skip len")?;
            }
            (_, 5) => {
                i = i.checked_add(4).context("skip fixed32")?;
            }
            _ => bail!("unsupported protobuf wire type {wire} for field {field}"),
        }
    }

    Ok((hr, ids, cfg))
}

fn encode_string_field(field_number: u32, value: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + value.len());
    write_varint(u64::from((field_number << 3) | 2), &mut out);
    write_varint(u64::try_from(value.len()).expect("usize fits u64"), &mut out);
    out.extend_from_slice(value.as_bytes());
    out
}

fn write_varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let mut byte = u8::try_from(value & 0x7f).expect("7-bit fits u8");
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
            out.push(byte);
        } else {
            out.push(byte);
            break;
        }
    }
}

fn read_varint(data: &[u8], mut i: usize) -> anyhow::Result<(u64, usize)> {
    let mut result = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *data.get(i).context("truncated protobuf varint")?;
        i += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((result, i));
        }
        shift += 7;
        if shift > 63 {
            bail!("protobuf varint too long");
        }
    }
}

/// Match .NET `new Guid(md5Bytes).ToString()` layout (mixed endian).
fn guid_string_from_dotnet_bytes(b: &[u8; 16]) -> String {
    let d1 = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let d2 = u16::from_le_bytes([b[4], b[5]]);
    let d3 = u16::from_le_bytes([b[6], b[7]]);
    format!(
        "{d1:08x}-{d2:04x}-{d3:04x}-{b8:02x}{b9:02x}-{b10:02x}{b11:02x}{b12:02x}{b13:02x}{b14:02x}{b15:02x}",
        b8 = b[8],
        b9 = b[9],
        b10 = b[10],
        b11 = b[11],
        b12 = b[12],
        b13 = b[13],
        b14 = b[14],
        b15 = b[15],
    )
}

fn current_user_sid_string() -> anyhow::Result<String> {
    let mut token = HANDLE::default();
    // SAFETY: pseudo-handle for the calling process; never closed.
    let process = unsafe { GetCurrentProcess() };
    // SAFETY: `token` out-param is written only on success.
    unsafe {
        OpenProcessToken(process, TOKEN_QUERY, &mut token).context("OpenProcessToken for current user SID")?;
    }

    let mut len = 0u32;
    // SAFETY: probe call with null buffer; only `len` is written.
    unsafe {
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
    }
    if len == 0 {
        // SAFETY: `token` owned exclusively by us from `OpenProcessToken`.
        unsafe {
            let _ = CloseHandle(token);
        }
        bail!("GetTokenInformation returned zero length for TokenUser");
    }

    let buf_len = usize::try_from(len).context("TokenUser buffer length")?;
    let mut buf = vec![0u8; buf_len];
    // SAFETY: buffer is at least `len` bytes; API writes TokenUser into it.
    let info_result = unsafe { GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len) };
    if let Err(err) = info_result {
        // SAFETY: close owned token on error path.
        unsafe {
            let _ = CloseHandle(token);
        }
        return Err(err).context("GetTokenInformation(TokenUser)");
    }

    // SAFETY: buffer was filled by GetTokenInformation with a TOKEN_USER.
    let token_user = unsafe { buf.as_ptr().cast::<TOKEN_USER>().read_unaligned() };
    let mut sid_raw = PWSTR::null();
    // SAFETY: SID pointer from TOKEN_USER is valid for the lifetime of `buf`.
    let sid_result = unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_raw) };
    // SAFETY: close owned token after SID conversion.
    unsafe {
        let _ = CloseHandle(token);
    }
    sid_result.context("ConvertSidToStringSidW")?;

    // SAFETY: `sid_raw` is a valid NUL-terminated wide string from ConvertSidToStringSidW.
    let sid = unsafe { sid_raw.to_string() }.context("SID UTF-16 to string")?;
    // SAFETY: string was allocated by ConvertSidToStringSidW; free with LocalFree.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sid_raw.as_ptr().cast())));
    }
    Ok(sid)
}

/// Pull a local-name element value from RdpClientConfig XML (namespace-tolerant).
pub(crate) fn xml_local_value(xml: &str, local_name: &str) -> String {
    let bytes = xml.as_bytes();
    let name = local_name.as_bytes();
    let mut i = 0;
    while i + name.len() + 2 < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        if j < bytes.len() && bytes[j] == b'/' {
            i += 1;
            continue;
        }
        let name_start = j;
        while j < bytes.len() && !matches!(bytes[j], b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/') {
            j += 1;
        }
        let token = &bytes[name_start..j];
        let local = match token.iter().rposition(|&c| c == b':') {
            Some(p) => &token[p + 1..],
            None => token,
        };
        if local != name {
            i += 1;
            continue;
        }
        while j < bytes.len() && bytes[j] != b'>' {
            j += 1;
        }
        if j >= bytes.len() {
            break;
        }
        if j > 0 && bytes[j - 1] == b'/' {
            return String::new();
        }
        let content_start = j + 1;
        // Unprefixed close tag: </LocalName>
        let end_marker = {
            let mut m = Vec::with_capacity(local_name.len() + 3);
            m.extend_from_slice(b"</");
            m.extend_from_slice(name);
            m.push(b'>');
            m
        };
        if let Some(rel) = find_bytes(&bytes[content_start..], &end_marker) {
            return bytes_to_trimmed_string(&bytes[content_start..content_start + rel]);
        }
        // Prefixed close tag: </ns:LocalName>
        let close_suffix = {
            let mut m = Vec::with_capacity(local_name.len() + 1);
            m.extend_from_slice(name);
            m.push(b'>');
            m
        };
        if let Some(rel) = find_bytes(&bytes[content_start..], &close_suffix) {
            let abs = content_start + rel;
            if let Some(lt) = rfind_bytes(&bytes[..abs], b"</") {
                if lt + 2 <= abs && lt >= content_start.saturating_sub(1) {
                    return bytes_to_trimmed_string(&bytes[content_start..lt]);
                }
            }
        }
        i += 1;
    }
    String::new()
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn rfind_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).rposition(|w| w == needle)
}

fn bytes_to_trimmed_string(bytes: &[u8]) -> String {
    let s = core::str::from_utf8(bytes).unwrap_or("");
    s.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guid_matches_dotnet_md5_layout() {
        // Live sample: MD5(SID) → 4b6a999d-8a64-9e3b-dc96-aefb3c47388b
        let md5 = [
            0x9d, 0x99, 0x6a, 0x4b, 0x64, 0x8a, 0x3b, 0x9e, 0xdc, 0x96, 0xae, 0xfb, 0x3c, 0x47, 0x38, 0x8b,
        ];
        assert_eq!(
            guid_string_from_dotnet_bytes(&md5),
            "4b6a999d-8a64-9e3b-dc96-aefb3c47388b"
        );
    }

    #[test]
    fn xml_local_value_reads_elements() {
        let xml = "<?xml version=\"1.0\"?><RdpClientConfig xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><VMId>606cf61b-dd6c-4d4c-8700-4af99a73f7ab</VMId><Username>WDAGUtilityAccount</Username><Password>secret</Password><RdpTransport>NamedPipe</RdpTransport><ClipboardRedirection>true</ClipboardRedirection></RdpClientConfig>";
        assert_eq!(xml_local_value(xml, "VMId"), "606cf61b-dd6c-4d4c-8700-4af99a73f7ab");
        assert_eq!(xml_local_value(xml, "Username"), "WDAGUtilityAccount");
        assert_eq!(xml_local_value(xml, "Password"), "secret");
        assert_eq!(xml_local_value(xml, "RdpTransport"), "NamedPipe");
        assert_eq!(xml_local_value(xml, "ClipboardRedirection"), "true");
    }

    #[test]
    fn parse_config_reply_sfixed32_and_string() {
        // field1 sfixed32=0 (tag 13), field2 string "hi" (tag 18)
        let mut msg = vec![13, 0, 0, 0, 0, 18];
        write_varint(2, &mut msg);
        msg.extend_from_slice(b"hi");
        let (hr, _, cfg) = parse_reply(&msg, ParseMode::Config).unwrap();
        assert_eq!(hr, 0);
        assert_eq!(cfg, "hi");
    }
}
