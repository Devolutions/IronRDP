//! Local IPC transport and message framing.
//!
//! The daemon and CLI talk over a platform-native local transport:
//!
//! - **Unix**: a [`tokio::net::UnixListener`]/[`tokio::net::UnixStream`] at
//!   `$XDG_RUNTIME_DIR/ironrdp-agent-<uid>.sock`, falling back to `/tmp/ironrdp-agent-<uid>.sock`
//!   when `XDG_RUNTIME_DIR` is unset.
//! - **Windows**: a named pipe at `\\.\pipe\ironrdp-agent-<user>`.
//!
//! Framing is identical on both: a little-endian `u32` byte-count prefix followed by the `Encode`d
//! message body.

use anyhow::{Context as _, bail};
use ironrdp_core::{DecodeOwned, Encode};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::ipc::{Request, Response};

/// Upper bound on a single framed message, guarding against absurd length prefixes.
const MAX_MESSAGE_LEN: usize = 16 * 1024 * 1024;

/// Writes `message` to `stream`, length-delimited.
pub(crate) async fn write_message<S, M>(stream: &mut S, message: &M) -> anyhow::Result<()>
where
    S: AsyncWrite + Unpin,
    M: Encode,
{
    let body = ironrdp_core::encode_vec(message).map_err(|e| anyhow::anyhow!("encode {}: {e}", message.name()))?;
    let len = u32::try_from(body.len()).context("message too large to frame")?;
    stream
        .write_all(&len.to_le_bytes())
        .await
        .context("write frame length")?;
    stream.write_all(&body).await.context("write frame body")?;
    stream.flush().await.context("flush frame")?;
    Ok(())
}

/// Reads a single length-delimited message from `stream`.
pub(crate) async fn read_message<S, M>(stream: &mut S) -> anyhow::Result<M>
where
    S: AsyncRead + Unpin,
    M: DecodeOwned,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.context("read frame length")?;
    let len = usize::try_from(u32::from_le_bytes(len_buf)).expect("u32 fits in usize on supported platforms");
    if MAX_MESSAGE_LEN < len {
        bail!("frame length {len} exceeds the {MAX_MESSAGE_LEN}-byte limit");
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.context("read frame body")?;
    ironrdp_core::decode_owned(&body).map_err(|e| anyhow::anyhow!("decode: {e}"))
}

/// Opens the endpoint, sends one `request`, and returns the daemon's `Response`.
pub async fn send_request(endpoint: &Endpoint, request: &Request) -> anyhow::Result<Response> {
    let mut stream = connect(endpoint)
        .await
        .with_context(|| format!("connect to daemon at {endpoint}"))?;
    write_message(&mut stream, request).await?;
    read_message(&mut stream).await
}

#[cfg(unix)]
mod imp {
    use std::io;
    use std::path::PathBuf;

    use tokio::net::{UnixListener, UnixStream};

    /// A resolved IPC endpoint (a Unix domain socket path).
    #[derive(Debug, Clone)]
    pub struct Endpoint(pub PathBuf);

    impl core::fmt::Display for Endpoint {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "{}", self.0.display())
        }
    }

    /// Returns the default per-user endpoint.
    pub fn default_endpoint() -> Endpoint {
        // SAFETY: `getuid` has no preconditions and is always safe to call.
        let uid = unsafe { libc::getuid() };
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Endpoint(dir.join(format!("ironrdp-agent-{uid}.sock")))
    }

    /// Connects to a listening daemon.
    pub async fn connect(endpoint: &Endpoint) -> io::Result<UnixStream> {
        UnixStream::connect(&endpoint.0).await
    }

    /// A bound listener that accepts client connections.
    pub struct Listener {
        inner: UnixListener,
        path: PathBuf,
    }

    impl Listener {
        /// Binds the listener at `endpoint`.
        pub fn bind(endpoint: &Endpoint) -> io::Result<Self> {
            let inner = UnixListener::bind(&endpoint.0)?;
            // Restrict the socket to the owner. The fallback directory is world-writable `/tmp`, so
            // without this any local user could connect and drive the session (input, screenshots,
            // logs). Fail loudly rather than serve on a world-accessible endpoint.
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&endpoint.0, std::fs::Permissions::from_mode(0o600))?;
            Ok(Self {
                inner,
                path: endpoint.0.clone(),
            })
        }

        /// Accepts the next client connection.
        pub async fn accept(&mut self) -> io::Result<UnixStream> {
            let (stream, _addr) = self.inner.accept().await?;
            Ok(stream)
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            // Best-effort removal of the socket file on shutdown (named pipes need no cleanup).
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::io;

    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions};

    /// A resolved IPC endpoint (a named pipe path).
    #[derive(Debug, Clone)]
    pub struct Endpoint(pub String);

    impl core::fmt::Display for Endpoint {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    /// Returns the default per-user endpoint.
    pub fn default_endpoint() -> Endpoint {
        let user = whoami::username().unwrap_or_else(|_| "user".to_owned());
        Endpoint(format!(r"\\.\pipe\ironrdp-agent-{user}"))
    }

    /// Connects to a listening daemon.
    pub async fn connect(endpoint: &Endpoint) -> io::Result<NamedPipeClient> {
        ClientOptions::new().open(&endpoint.0)
    }

    /// A named-pipe listener.
    ///
    /// It always keeps one ready (unconnected) server instance alive, which is both what serves the
    /// next connection and what upholds the `first_pipe_instance` exclusivity (a pipe with no live
    /// instance would let a second daemon claim the name).
    pub struct Listener {
        name: String,
        ready: NamedPipeServer,
    }

    impl Listener {
        /// Creates the first pipe instance, claiming the name exclusively.
        ///
        /// `first_pipe_instance(true)` makes this fail with `ERROR_ACCESS_DENIED` if another daemon
        /// already owns the pipe, so two daemons cannot coexist on the same endpoint.
        pub fn bind(endpoint: &Endpoint) -> io::Result<Self> {
            let ready = ServerOptions::new().first_pipe_instance(true).create(&endpoint.0)?;
            Ok(Self {
                name: endpoint.0.clone(),
                ready,
            })
        }

        /// Waits for the next client to connect to the ready instance, then mints a replacement.
        pub async fn accept(&mut self) -> io::Result<NamedPipeServer> {
            // Connect by reference so a cancelled future leaves `ready` intact (and the pipe alive).
            self.ready.connect().await?;
            // Mint the next listening instance before returning so the pipe is never instance-less.
            // Subsequent instances must omit `first_pipe_instance`, which is only valid on the first.
            let next = ServerOptions::new().create(&self.name)?;
            Ok(core::mem::replace(&mut self.ready, next))
        }
    }
}

pub use imp::{Endpoint, Listener, connect, default_endpoint};
