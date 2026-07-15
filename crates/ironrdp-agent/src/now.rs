//! NOW execution over the `Devolutions::Now::Agent` dynamic virtual channel.
//!
//! This module wires the in-process [`now_client`] worker to the session's DVC channel without a
//! pipe proxy, mirroring the CLIPRDR boundary-crossing pattern:
//!
//! - **Inbound** (remote → client): [`NowDvcProcessor`] runs on the session thread and forwards raw
//!   bytes over a dedicated unbounded channel of [`NowInbound`] events — the analog of
//!   `ClientClipboardMessageProxy`. It must be unbounded because `DvcProcessor::process` is
//!   synchronous and the NOW byte stream is framed (dropping a chunk would corrupt it).
//! - **Outbound** (client → remote): [`NowDvcStream`] chunks bytes with [`encode_dvc_messages`] and
//!   forwards them as [`RdpInputEvent::SendDvcMessages`] over the daemon's existing input channel.
//!
//! [`NowClient::connect`] drives the resulting byte stream on the daemon runtime.

// The pure helpers are `pub` so the `internal` feature can expose them for unit testing; the module
// itself is only public under that feature (see `lib.rs`), so they are otherwise `pub(crate)`.
#![cfg_attr(not(feature = "internal"), allow(unreachable_pub))]

use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::io;

use ironrdp_client::rdp::RdpInputEvent;
use ironrdp_core::{Encode, EncodeResult, WriteCursor, ensure_size, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcEncode, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::PduResult;
use ironrdp_propertyset::PropertySet;
use ironrdp_svc::ChannelFlags;
use now_client::{NowClient, NowClientConfig, NowClientHandle};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;

use crate::ipc::{NowShell, PropValue};

/// The NOW execution DVC channel name.
pub(crate) const DVC_CHANNEL_NAME: &str = "Devolutions::Now::Agent";

/// The property that toggles the NOW DVC channel. On unless explicitly set to `0`.
pub(crate) const ENABLE_PROPERTY: &str = "ironrdp_now";

/// Whether the NOW DVC channel is enabled for a session. Enabled by default (like the other
/// always-registered DVC channels); disable it by setting the [`ENABLE_PROPERTY`] (`ironrdp_now`)
/// property to `0`.
pub fn is_enabled(properties: &PropertySet) -> bool {
    properties.get::<bool>(ENABLE_PROPERTY).unwrap_or(true)
}

/// How long to wait for the DVC channel to open before failing a NOW request.
pub(crate) const READINESS_TIMEOUT: Duration = Duration::from_secs(30);

/// An event forwarded from the session-thread DVC processor to the daemon-side stream adapter.
pub(crate) enum NowInbound {
    /// The channel opened; carries the server-assigned channel id.
    Started(u32),
    /// Raw inbound bytes from the remote peer.
    Data(Vec<u8>),
    /// The channel closed.
    Closed,
}

/// A DVC message carrying already-serialized NOW bytes verbatim.
struct RawDataDvcMessage(Vec<u8>);

impl Encode for RawDataDvcMessage {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "RawDataDvcMessage"
    }

    fn size(&self) -> usize {
        self.0.len()
    }
}

impl DvcEncode for RawDataDvcMessage {}

/// Session-thread DVC processor for the NOW channel. A thin shim: it forwards inbound bytes to the
/// daemon over [`NowInbound`] and holds no protocol logic (that lives in [`now_client`]).
pub(crate) struct NowDvcProcessor {
    inbound: mpsc::UnboundedSender<NowInbound>,
}

impl NowDvcProcessor {
    pub(crate) fn new(inbound: mpsc::UnboundedSender<NowInbound>) -> Self {
        Self { inbound }
    }
}

impl_as_any!(NowDvcProcessor);

impl DvcProcessor for NowDvcProcessor {
    fn channel_name(&self) -> &str {
        DVC_CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        // A dropped receiver just means no NOW request is (or ever will be) in flight; ignore.
        let _ = self.inbound.send(NowInbound::Started(channel_id));
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let _ = self.inbound.send(NowInbound::Data(payload.to_vec()));
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        let _ = self.inbound.send(NowInbound::Closed);
    }
}

impl DvcClientProcessor for NowDvcProcessor {}

/// Daemon-side duplex byte stream bridging the NOW DVC channel to [`now_client`]. The read half is
/// fed by [`NowInbound::Data`]; the write half emits [`RdpInputEvent::SendDvcMessages`].
struct NowDvcStream {
    inbound: mpsc::UnboundedReceiver<NowInbound>,
    input_tx: mpsc::UnboundedSender<RdpInputEvent>,
    channel_id: u32,
    /// Bytes from the most recent [`NowInbound::Data`] not yet handed to a reader.
    leftover: Vec<u8>,
    /// Read offset into `leftover`.
    pos: usize,
    /// Whether the channel has closed (EOF for the reader).
    closed: bool,
}

impl NowDvcStream {
    fn new(
        inbound: mpsc::UnboundedReceiver<NowInbound>,
        input_tx: mpsc::UnboundedSender<RdpInputEvent>,
        channel_id: u32,
    ) -> Self {
        Self {
            inbound,
            input_tx,
            channel_id,
            leftover: Vec::new(),
            pos: 0,
            closed: false,
        }
    }
}

impl AsyncRead for NowDvcStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if this.pos < this.leftover.len() {
                let available = &this.leftover[this.pos..];
                let n = available.len().min(buf.remaining());
                buf.put_slice(&available[..n]);
                this.pos += n;
                return Poll::Ready(Ok(()));
            }
            if this.closed {
                return Poll::Ready(Ok(())); // EOF
            }
            match this.inbound.poll_recv(cx) {
                Poll::Ready(Some(NowInbound::Data(bytes))) => {
                    this.leftover = bytes;
                    this.pos = 0;
                }
                // A late `Started` cannot occur (the channel opens once); ignore defensively.
                Poll::Ready(Some(NowInbound::Started(_))) => {}
                Poll::Ready(Some(NowInbound::Closed)) | Poll::Ready(None) => {
                    this.closed = true;
                    return Poll::Ready(Ok(()));
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl AsyncWrite for NowDvcStream {
    fn poll_write(self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let message: DvcMessage = Box::new(RawDataDvcMessage(buf.to_vec()));
        let messages = encode_dvc_messages(this.channel_id, vec![message], ChannelFlags::empty())
            .map_err(|error| io::Error::other(format!("encode NOW DVC message: {error}")))?;
        this.input_tx
            .send(RdpInputEvent::SendDvcMessages {
                channel_id: this.channel_id,
                messages,
            })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "RDP session loop is closed"))?;
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Waits for the NOW DVC channel to open, then performs the [`now_client`] handshake over it.
///
/// Runs on the daemon runtime; the returned handle owns a spawned worker that drives the stream.
pub(crate) async fn establish(
    mut inbound: mpsc::UnboundedReceiver<NowInbound>,
    input_tx: mpsc::UnboundedSender<RdpInputEvent>,
) -> anyhow::Result<NowClientHandle> {
    // The processor emits `Started` from `DvcProcessor::start`, before any `Data`, so a single wait
    // for the first event yields the channel id.
    let channel_id = match tokio::time::timeout(READINESS_TIMEOUT, inbound.recv()).await {
        Ok(Some(NowInbound::Started(id))) => id,
        Ok(Some(NowInbound::Data(_))) => anyhow::bail!("NOW channel produced data before opening"),
        Ok(Some(NowInbound::Closed)) | Ok(None) => anyhow::bail!("NOW channel closed before it opened"),
        Err(_) => anyhow::bail!("timed out waiting for the {DVC_CHANNEL_NAME} channel to open"),
    };

    let stream = NowDvcStream::new(inbound, input_tx, channel_id);
    NowClient::connect(stream, NowClientConfig::default())
        .await
        .map_err(|error| anyhow::anyhow!("NOW handshake failed: {error}"))
}

// ── Capabilities model and pure helpers (unit-tested via the `internal` feature) ────────────────

/// A flattened, testable snapshot of the NOW capabilities relevant to this crate.
///
/// Decoupled from [`now_client::NowCapabilities`] (which has no public constructor) so the shell
/// resolution, flattening, and rendering logic can be exercised in the workspace test suite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NowCaps {
    /// Negotiated protocol version as `(major, minor)`.
    pub version: (u16, u16),
    /// Negotiated heartbeat interval in milliseconds, if any.
    pub heartbeat_ms: Option<u64>,
    /// Whether the generic Run style is available.
    pub run: bool,
    /// Whether CreateProcess execution is available.
    pub process: bool,
    /// Whether Batch execution is available.
    pub batch: bool,
    /// Whether Windows PowerShell execution is available.
    pub powershell: bool,
    /// Whether PowerShell 7 execution is available.
    pub pwsh: bool,
    /// Whether tracked I/O redirection is available.
    pub io_redirection: bool,
    /// Whether Unicode-console encoding is available.
    pub unicode_console: bool,
}

impl NowCaps {
    /// Snapshots the negotiated [`now_client::NowCapabilities`].
    pub(crate) fn from_client(caps: &now_client::NowCapabilities) -> Self {
        let version = caps.version();
        Self {
            version: (version.major, version.minor),
            heartbeat_ms: caps
                .heartbeat_interval()
                .map(|interval| u64::try_from(interval.as_millis()).unwrap_or(u64::MAX)),
            run: caps.supports_run(),
            process: caps.supports_process(),
            batch: caps.supports_batch(),
            powershell: caps.supports_win_ps(),
            pwsh: caps.supports_pwsh(),
            io_redirection: caps.supports_io_redirection(),
            unicode_console: caps.supports_unicode_console(),
        }
    }

    fn supports(&self, shell: NowShell) -> bool {
        match shell {
            NowShell::Pwsh => self.pwsh,
            NowShell::Powershell => self.powershell,
            NowShell::Batch => self.batch,
        }
    }
}

/// The default shell, in fixed preference order pwsh → powershell → batch, or `None` when the
/// session offers no shell.
pub fn default_shell(caps: &NowCaps) -> Option<NowShell> {
    if caps.pwsh {
        Some(NowShell::Pwsh)
    } else if caps.powershell {
        Some(NowShell::Powershell)
    } else if caps.batch {
        Some(NowShell::Batch)
    } else {
        None
    }
}

/// Resolves the effective shell: validates an explicit choice against the negotiated capabilities,
/// or picks the default. Returns a caller-facing error message on failure.
pub fn resolve_shell(caps: &NowCaps, requested: Option<NowShell>) -> Result<NowShell, String> {
    match requested {
        Some(shell) if caps.supports(shell) => Ok(shell),
        Some(shell) => {
            let available = available_shells(caps);
            let available = if available.is_empty() {
                "none".to_owned()
            } else {
                available.join(", ")
            };
            Err(format!("shell '{shell}' not available (available: {available})"))
        }
        None => default_shell(caps).ok_or_else(|| "no shell available on this session".to_owned()),
    }
}

fn available_shells(caps: &NowCaps) -> Vec<String> {
    [NowShell::Pwsh, NowShell::Powershell, NowShell::Batch]
        .into_iter()
        .filter(|shell| caps.supports(*shell))
        .map(|shell| shell.to_string())
        .collect()
}

/// Flattens capabilities into `now.`-prefixed property entries. Booleans are `Int` 0/1, the version
/// is a `Str`, `now.heartbeat_ms` is present only when negotiated, and `now.default_shell` reflects
/// [`default_shell`]. This is the single source of truth for both the property bag and the
/// `now capabilities` output.
pub fn flatten_caps(caps: &NowCaps) -> Vec<(String, PropValue)> {
    let mut entries = vec![(
        "now.version".to_owned(),
        PropValue::Str(format!("{}.{}", caps.version.0, caps.version.1)),
    )];
    if let Some(heartbeat_ms) = caps.heartbeat_ms {
        entries.push((
            "now.heartbeat_ms".to_owned(),
            PropValue::Int(i64::try_from(heartbeat_ms).unwrap_or(i64::MAX)),
        ));
    }
    for (key, value) in [
        ("now.run", caps.run),
        ("now.process", caps.process),
        ("now.batch", caps.batch),
        ("now.powershell", caps.powershell),
        ("now.pwsh", caps.pwsh),
        ("now.io_redirection", caps.io_redirection),
        ("now.unicode_console", caps.unicode_console),
    ] {
        entries.push((key.to_owned(), PropValue::Int(i64::from(value))));
    }
    entries.push((
        "now.default_shell".to_owned(),
        PropValue::Str(default_shell(caps).map_or_else(|| "none".to_owned(), |shell| shell.to_string())),
    ));
    entries
}

/// Renders flattened capability entries as `key: value` lines (one per entry, trailing newline
/// each), stripping the `now.` prefix. Booleans render as `true`/`false`; `heartbeat_ms` and the
/// version render as-is.
pub fn render_capabilities(entries: &[(String, PropValue)]) -> String {
    let mut out = String::new();
    for (key, value) in entries {
        let key = key.strip_prefix("now.").unwrap_or(key);
        let rendered = match value {
            // `heartbeat_ms` is the only integer that is a genuine number; the rest are booleans.
            PropValue::Int(n) if key == "heartbeat_ms" => n.to_string(),
            PropValue::Int(n) => if *n != 0 { "true" } else { "false" }.to_owned(),
            PropValue::Str(s) => s.clone(),
        };
        out.push_str(&format!("{key}: {rendered}\n"));
    }
    out
}

/// Maps a remote NOW exit code to a CLI process exit status: 0 → 0, 1–255 → the value, >255 → 255.
pub fn remote_exit_status(exit_code: u32) -> i32 {
    if exit_code == 0 {
        0
    } else if exit_code <= 255 {
        i32::try_from(exit_code).unwrap_or(255)
    } else {
        255
    }
}
