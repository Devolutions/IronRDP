//! Local IPC transport and message framing.
//!
//! The daemon and CLI talk over a platform-native local transport:
//!
//! - **Unix**: a [`tokio::net::UnixListener`]/[`tokio::net::UnixStream`] at
//!   `$XDG_RUNTIME_DIR/ironrdp-agent-<uid>.sock`, falling back to `/tmp/ironrdp-agent-<uid>.sock`
//!   when `XDG_RUNTIME_DIR` is unset.
//! - **Windows**: a named pipe at `\\.\pipe\ironrdp-agent-<user>`, restricted to the current user.
//!
//! Framing is identical on both: a little-endian `u32` byte-count prefix followed by the `Encode`d
//! message body.

use anyhow::{Context as _, bail};
use ironrdp_core::{DecodeOwned, Encode};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};

use crate::ipc::{Request, Response};

/// Upper bound on a single framed message, guarding against absurd length prefixes.
///
/// `pub(crate)` so `ipc` can derive payload-specific limits (e.g. the clipboard image cap) from
/// the actual transport ceiling instead of an unrelated number that happens to also be a size.
pub(crate) const MAX_MESSAGE_LEN: usize = 16 * 1024 * 1024;

/// Writes `message` to `stream`, length-delimited.
pub async fn write_message<S, M>(stream: &mut S, message: &M) -> anyhow::Result<()>
where
    S: AsyncWrite + Unpin,
    M: Encode,
{
    let body = ironrdp_core::encode_vec(message).map_err(|e| anyhow::anyhow!("encode {}: {e}", message.name()))?;
    if MAX_MESSAGE_LEN < body.len() {
        bail!("message length {} exceeds the {MAX_MESSAGE_LEN}-byte limit", body.len());
    }
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
pub async fn read_message<S, M>(stream: &mut S) -> anyhow::Result<M>
where
    S: AsyncRead + Unpin,
    M: DecodeOwned,
{
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await.context("read frame length")?;
    let len = usize::try_from(u32::from_le_bytes(len_buf)).context("frame length does not fit in usize")?;
    if MAX_MESSAGE_LEN < len {
        bail!("frame length {len} exceeds the {MAX_MESSAGE_LEN}-byte limit");
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.context("read frame body")?;
    ironrdp_core::decode_owned(&body).map_err(|e| anyhow::anyhow!("decode: {e}"))
}

/// Opens the endpoint, sends one `request`, and returns the daemon's `Response`.
pub async fn send_request(endpoint: &Endpoint, request: &Request) -> anyhow::Result<Response> {
    let mut stream = open_stream(endpoint, request).await?;
    read_message(&mut stream).await
}

/// Opens an IPC connection, sends `request`, and leaves the stream available for streamed replies.
pub async fn open_stream(endpoint: &Endpoint, request: &Request) -> anyhow::Result<ClientStream> {
    let mut stream = connect(endpoint)
        .await
        .with_context(|| format!("connect to daemon at {endpoint}"))?;
    write_message(&mut stream, request).await?;
    Ok(stream)
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

    /// Returns a per-user endpoint for `name`.
    pub fn default_endpoint_named(name: &str) -> Endpoint {
        // SAFETY: `getuid` has no preconditions and is always safe to call.
        let uid = unsafe { libc::getuid() };
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        Endpoint(dir.join(format!("{name}-{uid}.sock")))
    }

    /// Returns the default per-user endpoint.
    pub fn default_endpoint() -> Endpoint {
        default_endpoint_named("ironrdp-agent")
    }

    /// Resolves an explicit endpoint supplied by a local client.
    pub fn endpoint_from_string(value: String) -> Endpoint {
        Endpoint(PathBuf::from(value))
    }

    /// Removes a stale socket endpoint while refusing to touch non-socket paths.
    pub async fn prepare_endpoint(endpoint: &Endpoint) -> io::Result<()> {
        if !endpoint.0.exists() {
            return Ok(());
        }
        if connect(endpoint).await.is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("an RPC listener already appears to be running at {endpoint}"),
            ));
        }
        use std::os::unix::fs::FileTypeExt as _;
        let metadata = std::fs::symlink_metadata(&endpoint.0)?;
        if !metadata.file_type().is_socket() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("refusing to remove {endpoint}: path exists and is not a socket"),
            ));
        }
        std::fs::remove_file(&endpoint.0)
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
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HLOCAL, LocalFree};
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{
        GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows::core::{PCWSTR, PWSTR};

    /// A resolved IPC endpoint (a named pipe path).
    #[derive(Debug, Clone)]
    pub struct Endpoint(pub String);

    impl core::fmt::Display for Endpoint {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    /// Returns a per-user endpoint for `name`.
    pub fn default_endpoint_named(name: &str) -> Endpoint {
        let user = whoami::username().unwrap_or_else(|_| "user".to_owned());
        Endpoint(format!(r"\\.\pipe\{name}-{user}"))
    }

    /// Returns the default per-user endpoint.
    pub fn default_endpoint() -> Endpoint {
        default_endpoint_named("ironrdp-agent")
    }

    /// Resolves an explicit endpoint supplied by a local client.
    pub fn endpoint_from_string(value: String) -> Endpoint {
        if value.starts_with(r"\\.\pipe\") {
            Endpoint(value)
        } else {
            Endpoint(format!(r"\\.\pipe\{value}"))
        }
    }

    /// Named pipes require no stale-path cleanup.
    pub async fn prepare_endpoint(_endpoint: &Endpoint) -> io::Result<()> {
        Ok(())
    }

    /// Connects to a listening daemon.
    pub async fn connect(endpoint: &Endpoint) -> io::Result<NamedPipeClient> {
        ClientOptions::new().open(&endpoint.0)
    }

    /// Owns a process token handle and closes it on drop.
    struct OwnedTokenHandle(HANDLE);

    impl Drop for OwnedTokenHandle {
        fn drop(&mut self) {
            // SAFETY: `self.0` is a real token handle from `OpenProcessToken`, owned exclusively by us.
            if let Err(e) = unsafe { CloseHandle(self.0) } {
                tracing::warn!(error = %e, "Failed to close process token handle");
            }
        }
    }

    /// Owns a self-relative security descriptor allocated by
    /// `ConvertStringSecurityDescriptorToSecurityDescriptorW` and frees it with `LocalFree` on drop.
    struct OwnedSecurityDescriptor(PSECURITY_DESCRIPTOR);

    impl OwnedSecurityDescriptor {
        /// Builds a security descriptor granting access only to the current user.
        ///
        /// The descriptor is a protected DACL (no inherited ACEs) with a single allow-ACE granting
        /// `GENERIC_ALL` to the user identified by the current process token. This mirrors the Unix
        /// listener's `0o600` owner-only stance: only the user running the daemon may connect to the
        /// named pipe. On shared Windows hosts the pipe namespace is otherwise reachable by every
        /// local user, so this is the last line of defense against a different local user driving the
        /// session (input, screenshots, logs, NOW execution).
        fn for_current_user() -> io::Result<Self> {
            let buffer = current_user_token_buffer()?;
            let sid = sid_from_token_buffer(&buffer);

            let mut sid_wide = PWSTR(core::ptr::null_mut());
            // SAFETY: `sid` is a valid SID from the token; `sid_wide` is a valid out-pointer.
            unsafe { ConvertSidToStringSidW(sid, core::ptr::addr_of_mut!(sid_wide)) }
                .map_err(|e| io::Error::other(format!("convert sid to string: {e}")))?;
            // SAFETY: `sid_wide` points to a null-terminated UTF-16 string allocated by the call above.
            let sid_text =
                unsafe { sid_wide.to_string() }.map_err(|e| io::Error::other(format!("decode sid string: {e}")))?;
            // SAFETY: `sid_wide` was `LocalAlloc`'d by `ConvertSidToStringSidW` and is no longer needed.
            let _ = unsafe { LocalFree(Some(HLOCAL(sid_wide.as_ptr().cast::<core::ffi::c_void>()))) };

            // Build the SDDL: a protected DACL granting GENERIC_ALL to the current user only.
            let sddl_string = format!("D:P(A;;GA;;;{sid_text})");
            let mut sddl_utf16: Vec<u16> = sddl_string.encode_utf16().collect();
            sddl_utf16.push(0u16);
            let sddl_ptr = PCWSTR::from_raw(sddl_utf16.as_ptr());

            let mut security_descriptor = PSECURITY_DESCRIPTOR(core::ptr::null_mut());
            // SAFETY: `sddl_ptr` is a valid null-terminated SDDL string; `SDDL_REVISION_1` is the only
            // supported revision; `security_descriptor` is a valid out-pointer. The returned SD is
            // `LocalAlloc`'d and owned by us.
            unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl_ptr,
                    SDDL_REVISION_1,
                    core::ptr::addr_of_mut!(security_descriptor),
                    None,
                )
            }
            .map_err(|e| io::Error::other(format!("convert sddl to security descriptor: {e}")))?;

            Ok(Self(security_descriptor))
        }

        /// Returns the raw security-descriptor pointer for use as `lpSecurityDescriptor`.
        fn as_ptr(&self) -> *mut core::ffi::c_void {
            self.0.0
        }
    }

    impl Drop for OwnedSecurityDescriptor {
        fn drop(&mut self) {
            // SAFETY: `self.0` is a self-relative SD allocated by
            // `ConvertStringSecurityDescriptorToSecurityDescriptorW` (via `LocalAlloc`) and owned
            // exclusively by this guard.
            let _ = unsafe { LocalFree(Some(HLOCAL(self.0.0))) };
        }
    }

    /// Returns the buffer holding the `TOKEN_USER` of the current process token.
    ///
    /// The user SID is referenced by `TOKEN_USER.User.Sid` and lives inside this buffer, so keep the
    /// buffer alive while using the SID pointer.
    fn current_user_token_buffer() -> io::Result<Vec<u8>> {
        // SAFETY: `GetCurrentProcess` has no preconditions and returns a pseudo-handle that must
        // not be closed.
        let process = unsafe { GetCurrentProcess() };

        let mut token = HANDLE::default();
        // SAFETY: `process` is the valid current-process pseudo-handle; `token` is a valid out-pointer.
        unsafe { OpenProcessToken(process, TOKEN_QUERY, core::ptr::addr_of_mut!(token)) }
            .map_err(|e| io::Error::other(format!("open process token: {e}")))?;
        let token_guard = OwnedTokenHandle(token);
        let token = token_guard.0;

        // First query the required buffer size; the call fails with `ERROR_INSUFFICIENT_BUFFER`.
        let mut len = 0u32;
        // SAFETY: `token` is a valid token handle; a null buffer with zero length is the documented
        // way to retrieve the required size; `len` is a valid out-pointer.
        let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, core::ptr::addr_of_mut!(len)) };

        let mut buffer = vec![0u8; usize::try_from(len).expect("token information length fits in usize")];
        // SAFETY: `token` is valid; `buffer` has `len` bytes as required; `len` is a valid out-pointer.
        unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast::<core::ffi::c_void>()),
                len,
                core::ptr::addr_of_mut!(len),
            )
        }
        .map_err(|e| io::Error::other(format!("query token user: {e}")))?;

        Ok(buffer)
    }

    /// Returns the user SID referenced by a token buffer from [`current_user_token_buffer`].
    fn sid_from_token_buffer(buffer: &[u8]) -> PSID {
        // The buffer is byte-aligned, so read the `TOKEN_USER` header unaligned; the SID it references
        // lives further inside the buffer and stays valid as long as the buffer does.
        // SAFETY: `GetTokenInformation` wrote a valid `TOKEN_USER` at the start of `buffer`.
        let token_user = unsafe { core::ptr::read_unaligned(buffer.as_ptr().cast::<TOKEN_USER>()) };
        token_user.User.Sid
    }

    /// Creates one named-pipe server instance with `security` applied as its security descriptor.
    ///
    /// `CreateNamedPipeW` copies the security descriptor before returning, so `security` may be
    /// reused across instances and need only outlive this synchronous call. `first` sets
    /// `FILE_FLAG_FIRST_PIPE_INSTANCE`, which is only valid for the first instance of a pipe name.
    fn create_server_instance(
        name: &str,
        first: bool,
        security: &OwnedSecurityDescriptor,
    ) -> io::Result<NamedPipeServer> {
        let mut options = ServerOptions::new();
        if first {
            options.first_pipe_instance(true);
        }

        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).expect("security_attributes struct fits in u32"),
            lpSecurityDescriptor: security.as_ptr(),
            ..Default::default()
        };

        let raw = core::ptr::addr_of_mut!(attributes).cast::<core::ffi::c_void>();
        // SAFETY: `attributes` lives for the duration of this synchronous call; `CreateNamedPipeW`
        // copies the security descriptor before returning, so dropping `attributes` afterwards is safe.
        unsafe { options.create_with_security_attributes_raw(name, raw) }
    }

    /// A named-pipe listener.
    ///
    /// It always keeps one ready (unconnected) server instance alive, which is both what serves the
    /// next connection and what upholds the `first_pipe_instance` exclusivity (a pipe with no live
    /// instance would let a second daemon claim the name). Every instance is created with a security
    /// descriptor that restricts access to the current user, mirroring the Unix listener's `0o600`.
    pub struct Listener {
        name: String,
        ready: NamedPipeServer,
        security: OwnedSecurityDescriptor,
    }

    impl Listener {
        /// Creates the first pipe instance, claiming the name exclusively.
        ///
        /// `first_pipe_instance(true)` makes this fail with `ERROR_ACCESS_DENIED` if another daemon
        /// already owns the pipe, so two daemons cannot coexist on the same endpoint.
        pub fn bind(endpoint: &Endpoint) -> io::Result<Self> {
            let security = OwnedSecurityDescriptor::for_current_user()?;
            let ready = create_server_instance(&endpoint.0, true, &security)?;
            Ok(Self {
                name: endpoint.0.clone(),
                ready,
                security,
            })
        }

        /// Waits for the next client to connect to the ready instance, then mints a replacement.
        pub async fn accept(&mut self) -> io::Result<NamedPipeServer> {
            // Connect by reference so a cancelled future leaves `ready` intact (and the pipe alive).
            self.ready.connect().await?;
            // Mint the next listening instance before returning so the pipe is never instance-less.
            // Subsequent instances must omit `first_pipe_instance`, which is only valid on the first.
            let next = create_server_instance(&self.name, false, &self.security)?;
            Ok(core::mem::replace(&mut self.ready, next))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::Foundation::ERROR_SUCCESS;
        use windows::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
        use windows::Win32::Security::{
            ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, DACL_SECURITY_INFORMATION, EqualSid,
            GetAce, GetAclInformation, GetSecurityDescriptorControl, GetSecurityDescriptorDacl, SE_DACL_PROTECTED,
        };

        /// The pipe created by the listener must actually carry the restrictive DACL, not just a
        /// non-null descriptor. This catches three regressions a non-null check would miss: the SDDL
        /// granting `Everyone`, the protected-DACL flag (`P`) being dropped, and `create_server_instance`
        /// failing to apply the descriptor. It queries the live pipe's DACL and asserts it is
        /// protected and contains exactly one allow-ACE granting `GENERIC_ALL` to the current user SID.
        #[tokio::test]
        async fn pipe_dacl_grants_generic_all_only_to_current_user() {
            // Expected SID = current process user.
            let expected_buffer = current_user_token_buffer().expect("get current user token");
            let expected_sid = sid_from_token_buffer(&expected_buffer);

            // Build the descriptor and a real server instance with a unique name.
            let security = OwnedSecurityDescriptor::for_current_user().expect("build security descriptor");
            let name = format!("\\\\.\\pipe\\ironrdp-agent-acl-test-{}", std::process::id());
            let mut name_utf16: Vec<u16> = name.encode_utf16().collect();
            name_utf16.push(0u16);
            let _server = create_server_instance(&name, true, &security).expect("create pipe instance");

            // Query the pipe's actual DACL.
            let mut sd = PSECURITY_DESCRIPTOR(core::ptr::null_mut());
            let mut dacl: *mut ACL = core::ptr::null_mut();
            // SAFETY: `name_utf16` is a valid null-terminated pipe path; we request only the DACL; the
            // out-pointers are valid. The returned SD is `LocalAlloc`'d and owned by us until freed below.
            let result = unsafe {
                GetNamedSecurityInfoW(
                    PCWSTR::from_raw(name_utf16.as_ptr()),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    None,
                    None,
                    Some(core::ptr::addr_of_mut!(dacl)),
                    None,
                    core::ptr::addr_of_mut!(sd),
                )
            };
            assert_eq!(result, ERROR_SUCCESS, "GetNamedSecurityInfoW should succeed");

            // `GetNamedSecurityInfoW` `LocalAlloc`'s the descriptor; reuse the production RAII guard so it
            // is freed on drop at the end of the test even if an assertion below panics.
            let _sd_guard = OwnedSecurityDescriptor(sd);

            // The DACL must be protected (no inherited ACEs from the pipe namespace).
            let mut control: u16 = 0;
            let mut revision: u32 = 0;
            // SAFETY: `sd` is a valid self-relative SD returned by `GetNamedSecurityInfoW`.
            unsafe {
                GetSecurityDescriptorControl(sd, core::ptr::addr_of_mut!(control), core::ptr::addr_of_mut!(revision))
            }
            .expect("get security descriptor control");
            assert!(
                (control & SE_DACL_PROTECTED.0) != 0,
                "DACL must be protected so inherited ACEs cannot widen access"
            );

            // Exactly one ACE must be present.
            let mut dacl_present = windows::core::BOOL::default();
            let mut dacl_ptr: *mut ACL = core::ptr::null_mut();
            let mut defaulted = windows::core::BOOL::default();
            // SAFETY: `sd` is a valid SD; the out-pointers are valid.
            unsafe {
                GetSecurityDescriptorDacl(
                    sd,
                    core::ptr::addr_of_mut!(dacl_present),
                    core::ptr::addr_of_mut!(dacl_ptr),
                    core::ptr::addr_of_mut!(defaulted),
                )
            }
            .expect("get dacl");
            assert!(dacl_present.as_bool(), "DACL must be present");
            assert!(!dacl_ptr.is_null(), "DACL pointer must not be null");

            let mut size_info = ACL_SIZE_INFORMATION::default();
            // SAFETY: `dacl_ptr` is a valid ACL; `size_info` is a valid out-buffer of the right size.
            unsafe {
                GetAclInformation(
                    dacl_ptr.cast_const(),
                    core::ptr::addr_of_mut!(size_info).cast::<core::ffi::c_void>(),
                    u32::try_from(size_of::<ACL_SIZE_INFORMATION>()).expect("acl size information fits in u32"),
                    AclSizeInformation,
                )
            }
            .expect("get acl information");
            assert_eq!(size_info.AceCount, 1, "DACL must contain exactly one ACE");

            // The single ACE must be an allow-ACE granting GENERIC_ALL to the current user SID.
            let mut ace_ptr: *mut core::ffi::c_void = core::ptr::null_mut();
            // SAFETY: `dacl_ptr` is valid and holds one ACE (verified above); `ace_ptr` is a valid out-pointer.
            unsafe { GetAce(dacl_ptr.cast_const(), 0, core::ptr::addr_of_mut!(ace_ptr)) }.expect("get ace");
            assert!(!ace_ptr.is_null(), "ACE pointer must not be null");

            // SAFETY: ACE 0 is an `ACCESS_ALLOWED_ACE` (SDDL "A;;" = allow) placed at DWORD alignment by
            // the ACL; `read_unaligned` avoids any alignment assumption.
            let ace: ACCESS_ALLOWED_ACE = unsafe { core::ptr::read_unaligned(ace_ptr.cast::<ACCESS_ALLOWED_ACE>()) };
            assert_eq!(
                ace.Header.AceType, 0u8, /* ACCESS_ALLOWED_ACE_TYPE */
                "ACE must be an allow ACE"
            );
            // `GENERIC_ALL` (0x10000000) in the SDDL is mapped by the kernel to the file-object-specific
            // `FILE_ALL_ACCESS` (0x001F01FF) when the descriptor is stored on a named pipe, so compare
            // against the mapped value rather than the generic bit.
            assert_eq!(
                ace.Mask, 0x001F01FFu32, /* FILE_ALL_ACCESS */
                "ACE must grant full access (GENERIC_ALL mapped to the pipe object)"
            );

            // The SID starts at `SidStart` and extends past the struct into the ACL buffer, so point into
            // the in-place ACE rather than the local copy.
            // SAFETY: `ace_ptr` is a non-null pointer to a valid `ACCESS_ALLOWED_ACE`; `SidStart` is the
            // first DWORD of the variable-length SID that follows the fixed ACE header.
            let ace_sid = PSID(unsafe { ace_ptr.byte_add(core::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart)) });
            // SAFETY: both `ace_sid` and `expected_sid` are valid SIDs from the token / ACL.
            let sids_equal = unsafe { EqualSid(ace_sid, expected_sid) }.is_ok();
            assert!(sids_equal, "ACE SID must equal the current user SID");
        }
    }
}

pub use imp::{
    Endpoint, Listener, connect, default_endpoint, default_endpoint_named, endpoint_from_string, prepare_endpoint,
};

#[cfg(unix)]
pub type ClientStream = tokio::net::UnixStream;
#[cfg(windows)]
pub type ClientStream = tokio::net::windows::named_pipe::NamedPipeClient;

#[cfg(test)]
mod tests {
    use core::pin::Pin;
    use core::task::{Context, Poll};
    use std::io;

    use tokio::io::AsyncWrite;

    use super::{MAX_MESSAGE_LEN, write_message};

    #[derive(Default)]
    struct RecordingWriter(Vec<u8>);

    impl AsyncWrite for RecordingWriter {
        fn poll_write(mut self: Pin<&mut Self>, _cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
            self.0.extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn write_message_rejects_oversized_messages_before_writing() {
        let message = vec![0; MAX_MESSAGE_LEN + 1];
        let mut stream = RecordingWriter::default();

        let error = write_message(&mut stream, &message)
            .await
            .expect_err("message should exceed frame limit");

        assert_eq!(
            error.to_string(),
            format!(
                "message length {} exceeds the {MAX_MESSAGE_LEN}-byte limit",
                message.len()
            )
        );
        assert!(stream.0.is_empty());
    }
}
