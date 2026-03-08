// Use the `windows` PE subsystem so that this binary can be spawned via
// CreateProcessAsUserW into another session without STATUS_DLL_INIT_FAILED
// (0xC0000142). Console-subsystem binaries require CSRSS interaction during
// DLL initialization, which fails when the target session's CSRSS is not
// reachable (e.g. winlogon desktop of a partially-initialized session).
// The binary runs as a service/scheduled task with redirected stdout/stderr,
// so the lack of an auto-allocated console is harmless.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(not(windows))]
fn main() {
    eprintln!("ironrdp-termsrv is only supported on windows");
}

#[cfg(windows)]
mod windows_main {
    use core::ffi::c_void;
    use core::net::{Ipv4Addr, SocketAddr};
    use core::num::{NonZeroI32, NonZeroU16, NonZeroUsize};
    use core::ptr::null_mut;
    use core::sync::atomic::{fence, AtomicBool, AtomicU64, Ordering};
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::io;
    use std::io::Write as _;
    use std::sync::{Arc, Mutex as StdMutex, OnceLock};
    use std::time::Instant;

    use anyhow::{anyhow, Context as _};
    use ironrdp_server::tokio_rustls::{rustls, TlsAcceptor};
    use ironrdp_server::{
        BitmapUpdate, Credentials, DesktopSize, DisplayUpdate, KeyboardEvent, MouseEvent, PixelFormat, RdpServer,
        RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler,
    };
    use ironrdp_wtsprotocol_ipc::{
        default_pipe_name, pipe_path, resolve_pipe_name_from_env, ProviderCommand, ServiceEvent, DEFAULT_MAX_FRAME_SIZE,
    };
    use rustls_cng::signer::CngSigningKey;
    use rustls_cng::store::{CertStore, CertStoreType};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::windows::named_pipe;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::{mpsc, watch, Mutex};
    use tokio::task::JoinHandle;
    use tokio::time::{sleep, timeout, Duration};
    use tracing::{debug, error, info, warn};
    use tracing_subscriber::EnvFilter;
    use windows::core::{w, BOOL, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{
        GetLastError, LocalFree, SetLastError, ERROR_BAD_LENGTH, ERROR_NO_MORE_FILES, ERROR_NOT_ALL_ASSIGNED, HANDLE,
        HLOCAL, LUID, WAIT_OBJECT_0, WAIT_TIMEOUT, WIN32_ERROR,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT, DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
    };
    use windows::Win32::Security::Cryptography::{
        CertAddCertificateContextToStore, CertCloseStore, CertCreateSelfSignCertificate, CertFindCertificateInStore,
        CertFreeCertificateContext, CertOpenStore, CertStrToNameW, NCryptCreatePersistedKey, NCryptFinalizeKey,
        NCryptFreeObject, NCryptOpenStorageProvider, NCryptSetProperty, BCRYPT_RSA_ALGORITHM, CERT_CONTEXT,
        CERT_CREATE_SELFSIGN_FLAGS, CERT_FIND_SUBJECT_STR_W, CERT_NCRYPT_KEY_SPEC, CERT_OPEN_STORE_FLAGS,
        CERT_QUERY_ENCODING_TYPE, CERT_STORE_ADD_REPLACE_EXISTING, CERT_STORE_PROV_SYSTEM_W,
        CERT_SYSTEM_STORE_LOCAL_MACHINE, CERT_X500_NAME_STR, CRYPT_INTEGER_BLOB, CRYPT_KEY_PROV_INFO, HCERTSTORE,
        MS_KEY_STORAGE_PROVIDER, NCRYPT_ALLOW_EXPORT_FLAG, NCRYPT_ALLOW_PLAINTEXT_EXPORT_FLAG,
        NCRYPT_EXPORT_POLICY_PROPERTY, NCRYPT_FLAGS, NCRYPT_HANDLE, NCRYPT_LENGTH_PROPERTY, NCRYPT_MACHINE_KEY_FLAG,
        NCRYPT_PROV_HANDLE, PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };
    use windows::Win32::System::Memory::{
        CreateFileMappingW, MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ, FILE_MAP_WRITE,
        PAGE_READWRITE,
    };
    use windows::Win32::System::RemoteDesktop::{
        WTSEnumerateSessionsW, WTSFreeMemory, WTSGetActiveConsoleSessionId, WTSQueryUserToken, WTS_SESSION_INFOW,
    };
    use windows::Win32::System::Threading::{
        CreateEventW, CreateProcessAsUserW, GetCurrentProcess, GetCurrentProcessId, OpenEventW, OpenProcess,
        OpenProcessToken, SetEvent, TerminateProcess, WaitForSingleObject, CREATE_NO_WINDOW, EVENT_MODIFY_STATE,
        PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
        KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
        MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, VIRTUAL_KEY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SetCursorPos, SM_CXSCREEN, SM_CYSCREEN};

    use windows::Win32::Security::{
        AdjustTokenPrivileges, DuplicateTokenEx, GetTokenInformation, LookupPrivilegeValueW, RevertToSelf,
        SecurityImpersonation, SetTokenInformation, TokenPrimary, TokenSessionId, LUID_AND_ATTRIBUTES,
        SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY,
        TOKEN_DUPLICATE, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };

    use windows::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

    use windows::Win32::System::RemoteDesktop::{WTSEnumerateProcessesW, WTS_PROCESS_INFOW};

    #[link(name = "sas")]
    unsafe extern "system" {
        // Win32 BOOL is a 32-bit signed integer.
        fn SendSAS(as_user: i32) -> i32;
    }

    const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
    const CONTROL_PIPE_SERVER_INSTANCES: usize = 64;
    const CONTROL_PIPE_IDLE_TIMEOUT: Duration = Duration::from_secs(2);
    const LISTEN_ADDR_ENV: &str = "IRONRDP_WTS_LISTEN_ADDR";
    const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:4489";
    const CAPTURE_INTERVAL: Duration = Duration::from_millis(100);
    const LOGON_READINESS_PROBE_INTERVAL: Duration = Duration::from_secs(2);
    const WAITING_FOR_USER_LOGON_DIAGNOSTIC_INTERVAL: Duration = Duration::from_secs(5);
    const SHELL_BOOTSTRAP_GRACE: Duration = Duration::from_secs(0);
    const SHELL_BOOTSTRAP_RETRY_INTERVAL: Duration = Duration::from_secs(1);
    const AUTO_SEND_SAS_RETRY_INTERVAL: Duration = Duration::from_secs(3);
    const FIRST_FRAME_BLANK_GRACE: Duration = Duration::from_secs(1);
    const FIRST_FRAME_BLANK_MAX_FRAMES: u32 = 8;
    const PERSISTENT_BLANK_RESTART_GRACE: Duration = Duration::from_secs(3);
    const PERSISTENT_BLANK_RESTART_MIN_FRAMES: u32 = 20;
    const CAPTURE_HELPER_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
    const CAPTURE_HELPER_RETRY_DELAY: Duration = Duration::from_secs(5);
    const CAPTURE_IPC_ENV: &str = "IRONRDP_WTS_CAPTURE_IPC";
    const CAPTURE_SESSION_ID_ENV: &str = "IRONRDP_WTS_CAPTURE_SESSION_ID";
    const DUMP_BITMAP_UPDATES_DIR_ENV: &str = "IRONRDP_WTS_DUMP_BITMAP_UPDATES_DIR";
    const AUTO_LISTEN_ENV: &str = "IRONRDP_WTS_AUTO_LISTEN";
    const AUTO_LISTEN_NAME_ENV: &str = "IRONRDP_WTS_AUTO_LISTENER_NAME";
    const AUTO_SEND_SAS_ENV: &str = "IRONRDP_WTS_AUTO_SEND_SAS";
    const TLS_CERT_SUBJECT_FIND: &str = "IronRDP TermSrv";
    const TLS_KEY_NAME: &str = "IronRdpTermSrvTlsKey";
    const RDP_USERNAME_ENV: &str = "IRONRDP_RDP_USERNAME";
    const RDP_PASSWORD_ENV: &str = "IRONRDP_RDP_PASSWORD";
    const RDP_DOMAIN_ENV: &str = "IRONRDP_RDP_DOMAIN";
    const WTS_LOGON_USERNAME_ENV: &str = "IRONRDP_WTS_LOGON_USERNAME";
    const WTS_LOGON_PASSWORD_ENV: &str = "IRONRDP_WTS_LOGON_PASSWORD";
    const WTS_LOGON_DOMAIN_ENV: &str = "IRONRDP_WTS_LOGON_DOMAIN";

    fn control_pipe_security_attributes() -> anyhow::Result<(SECURITY_ATTRIBUTES, PSECURITY_DESCRIPTOR)> {
        // Allow TermService (NetworkService) to connect to the control pipe.
        //
        // NOTE: we also include Everyone (WD) as a diagnostic escape hatch so we can
        // confirm that the DACL is actually being applied and unblock integration on
        // newer Windows builds where TermService may use additional restricted SIDs.
        //
        // - SY: LocalSystem
        // - BA: Builtin Administrators
        // - NS: NetworkService
        // - WD: Everyone
        let sddl = w!("D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;NS)(A;;GA;;;WD)");

        let mut sd = PSECURITY_DESCRIPTOR::default();
        let mut sd_len = 0u32;

        // SAFETY: `sddl` is a NUL-terminated wide string literal. `sd` receives a valid pointer
        // which must be freed with LocalFree.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, SDDL_REVISION_1, &mut sd, Some(&mut sd_len))
        }
        .map_err(|error| anyhow!("ConvertStringSecurityDescriptorToSecurityDescriptorW failed: {error}"))?;

        let attrs = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
                .map_err(|_| anyhow!("SECURITY_ATTRIBUTES size overflow"))?,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: BOOL(0),
        };

        Ok((attrs, sd))
    }

    const INPUT_FRAME_MAGIC: [u8; 4] = *b"IRIN";
    const INPUT_FRAME_VERSION: u16 = 1;
    const INPUT_FRAME_HEADER_LEN: usize = 8;

    const INPUT_MSG_SCANCODE_KEY: u8 = 1;
    const INPUT_MSG_UNICODE_KEY: u8 = 2;
    const INPUT_MSG_MOUSE_MOVE_ABS: u8 = 10;
    const INPUT_MSG_MOUSE_MOVE_REL: u8 = 11;
    const INPUT_MSG_MOUSE_BUTTON: u8 = 12;
    const INPUT_MSG_MOUSE_WHEEL: u8 = 13;
    const INPUT_MSG_MOUSE_HWHEEL: u8 = 14;

    const INPUT_KEY_FLAG_RELEASE: u8 = 0b0000_0001;
    const INPUT_KEY_FLAG_EXTENDED: u8 = 0b0000_0010;

    const INPUT_MOUSE_BUTTON_LEFT: u8 = 1;
    const INPUT_MOUSE_BUTTON_RIGHT: u8 = 2;
    const INPUT_MOUSE_BUTTON_MIDDLE: u8 = 3;
    const INPUT_MOUSE_BUTTON_X1: u8 = 4;
    const INPUT_MOUSE_BUTTON_X2: u8 = 5;

    const INPUT_MOUSE_BUTTON_DOWN: u8 = 1;
    const INPUT_MOUSE_BUTTON_UP: u8 = 0;

    const WHEEL_DELTA_I32: i32 = 120;

    #[derive(Debug, Clone)]
    struct InputPacket {
        bytes: Vec<u8>,
    }

    fn make_input_packet(msg_type: u8, payload: &[u8]) -> Option<InputPacket> {
        let payload_len = u8::try_from(payload.len()).ok()?;
        let mut bytes = Vec::with_capacity(INPUT_FRAME_HEADER_LEN + payload.len());
        bytes.extend_from_slice(&INPUT_FRAME_MAGIC);
        bytes.extend_from_slice(&INPUT_FRAME_VERSION.to_le_bytes());
        bytes.push(msg_type);
        bytes.push(payload_len);
        bytes.extend_from_slice(payload);
        Some(InputPacket { bytes })
    }

    #[derive(Clone)]
    struct TermSrvInputHandler {
        connection_id: u32,
        tx: mpsc::UnboundedSender<InputPacket>,
    }

    impl TermSrvInputHandler {
        fn new(connection_id: u32, tx: mpsc::UnboundedSender<InputPacket>) -> Self {
            Self { connection_id, tx }
        }

        fn send(&self, packet: Option<InputPacket>) {
            let Some(packet) = packet else {
                return;
            };

            if self.tx.send(packet).is_err() {
                warn!(
                    connection_id = self.connection_id,
                    "Input channel closed; dropping input event"
                );
            }
        }

        fn send_scancode(&self, code: u8, extended: bool, released: bool) {
            let mut flags = 0u8;
            if released {
                flags |= INPUT_KEY_FLAG_RELEASE;
            }
            if extended {
                flags |= INPUT_KEY_FLAG_EXTENDED;
            }

            self.send(make_input_packet(INPUT_MSG_SCANCODE_KEY, &[flags, code]));
        }

        fn send_unicode(&self, ch: u16, released: bool) {
            let flags = if released { INPUT_KEY_FLAG_RELEASE } else { 0 };
            let mut payload = [0u8; 3];
            payload[0] = flags;
            payload[1..3].copy_from_slice(&ch.to_le_bytes());
            self.send(make_input_packet(INPUT_MSG_UNICODE_KEY, &payload));
        }

        fn send_mouse_move_abs(&self, x: u16, y: u16) {
            let mut payload = [0u8; 4];
            payload[0..2].copy_from_slice(&x.to_le_bytes());
            payload[2..4].copy_from_slice(&y.to_le_bytes());
            self.send(make_input_packet(INPUT_MSG_MOUSE_MOVE_ABS, &payload));
        }

        fn send_mouse_move_rel(&self, dx: i32, dy: i32) {
            let mut payload = [0u8; 8];
            payload[0..4].copy_from_slice(&dx.to_le_bytes());
            payload[4..8].copy_from_slice(&dy.to_le_bytes());
            self.send(make_input_packet(INPUT_MSG_MOUSE_MOVE_REL, &payload));
        }

        fn send_mouse_button(&self, button: u8, down: bool) {
            let state = if down {
                INPUT_MOUSE_BUTTON_DOWN
            } else {
                INPUT_MOUSE_BUTTON_UP
            };
            self.send(make_input_packet(INPUT_MSG_MOUSE_BUTTON, &[button, state]));
        }

        fn send_mouse_wheel(&self, delta: i32) {
            self.send(make_input_packet(INPUT_MSG_MOUSE_WHEEL, &delta.to_le_bytes()));
        }

        fn send_mouse_hwheel(&self, delta: i32) {
            self.send(make_input_packet(INPUT_MSG_MOUSE_HWHEEL, &delta.to_le_bytes()));
        }
    }

    impl RdpServerInputHandler for TermSrvInputHandler {
        fn keyboard(&mut self, event: KeyboardEvent) {
            match event {
                KeyboardEvent::Pressed { code, extended } => self.send_scancode(code, extended, false),
                KeyboardEvent::Released { code, extended } => self.send_scancode(code, extended, true),
                KeyboardEvent::UnicodePressed(ch) => self.send_unicode(ch, false),
                KeyboardEvent::UnicodeReleased(ch) => self.send_unicode(ch, true),
                KeyboardEvent::Synchronize(_flags) => {
                    // Best-effort: ignore synchronize toggles for now.
                }
            }
        }

        fn mouse(&mut self, event: MouseEvent) {
            match event {
                MouseEvent::Move { x, y } => self.send_mouse_move_abs(x, y),
                MouseEvent::RelMove { x, y } => self.send_mouse_move_rel(x, y),
                MouseEvent::LeftPressed => self.send_mouse_button(INPUT_MOUSE_BUTTON_LEFT, true),
                MouseEvent::LeftReleased => self.send_mouse_button(INPUT_MOUSE_BUTTON_LEFT, false),
                MouseEvent::RightPressed => self.send_mouse_button(INPUT_MOUSE_BUTTON_RIGHT, true),
                MouseEvent::RightReleased => self.send_mouse_button(INPUT_MOUSE_BUTTON_RIGHT, false),
                MouseEvent::MiddlePressed => self.send_mouse_button(INPUT_MOUSE_BUTTON_MIDDLE, true),
                MouseEvent::MiddleReleased => self.send_mouse_button(INPUT_MOUSE_BUTTON_MIDDLE, false),
                MouseEvent::Button4Pressed => self.send_mouse_button(INPUT_MOUSE_BUTTON_X1, true),
                MouseEvent::Button4Released => self.send_mouse_button(INPUT_MOUSE_BUTTON_X1, false),
                MouseEvent::Button5Pressed => self.send_mouse_button(INPUT_MOUSE_BUTTON_X2, true),
                MouseEvent::Button5Released => self.send_mouse_button(INPUT_MOUSE_BUTTON_X2, false),
                MouseEvent::VerticalScroll { value } => {
                    let delta = i32::from(value).saturating_mul(WHEEL_DELTA_I32);
                    self.send_mouse_wheel(delta);
                }
                MouseEvent::Scroll { x, y } => {
                    if y != 0 {
                        self.send_mouse_wheel(y);
                    }
                    if x != 0 {
                        self.send_mouse_hwheel(x);
                    }
                }
            }
        }
    }

    async fn run_input_spooler(
        connection_id: u32,
        stream_slot: Arc<Mutex<Option<TcpStream>>>,
        mut rx: mpsc::UnboundedReceiver<InputPacket>,
    ) {
        let mut forwarded_any = false;

        while let Some(packet) = rx.recv().await {
            let mut guard = stream_slot.lock().await;
            let Some(stream) = guard.as_mut() else {
                continue;
            };

            if let Err(error) = stream.write_all(&packet.bytes).await {
                warn!(
                    connection_id,
                    error = %format!("{error:#}"),
                    "Failed to write input packet to helper; disabling input until helper reconnects"
                );
                *guard = None;
            } else if !forwarded_any {
                forwarded_any = true;
                info!(connection_id, "Forwarding input events to capture helper");
            }
        }
    }

    struct GdiDisplay {
        connection_id: u32,
        desktop_size: DesktopSize,
        input_stream_slot: Arc<Mutex<Option<TcpStream>>>,
        connection_session_ids: Arc<StdMutex<HashMap<u32, u32>>>,
        credentials_slot: Arc<Mutex<Option<StoredCredentials>>>,
        provider_mode: bool,
    }

    impl GdiDisplay {
        fn new(
            connection_id: u32,
            input_stream_slot: Arc<Mutex<Option<TcpStream>>>,
            connection_session_ids: Arc<StdMutex<HashMap<u32, u32>>>,
            credentials_slot: Arc<Mutex<Option<StoredCredentials>>>,
            provider_mode: bool,
        ) -> anyhow::Result<Self> {
            let desktop_size = desktop_size_from_gdi().context("failed to query desktop size")?;

            info!(
                width = desktop_size.width,
                height = desktop_size.height,
                "Initialized GDI display source"
            );

            Ok(Self {
                connection_id,
                desktop_size,
                input_stream_slot,
                connection_session_ids,
                credentials_slot,
                provider_mode,
            })
        }
    }

    #[async_trait::async_trait]
    impl RdpServerDisplay for GdiDisplay {
        async fn size(&mut self) -> DesktopSize {
            self.desktop_size
        }

        async fn request_initial_size(&mut self, client_size: DesktopSize) -> DesktopSize {
            info!(
                client_width = client_size.width,
                client_height = client_size.height,
                server_width = self.desktop_size.width,
                server_height = self.desktop_size.height,
                "Received initial client desktop size request"
            );
            self.desktop_size
        }

        async fn updates(&mut self) -> anyhow::Result<Box<dyn RdpServerDisplayUpdates>> {
            Ok(Box::new(
                GdiDisplayUpdates::new(
                    self.connection_id,
                    self.desktop_size,
                    Arc::clone(&self.input_stream_slot),
                    Arc::clone(&self.connection_session_ids),
                    Arc::clone(&self.credentials_slot),
                    self.provider_mode,
                )
                .context("failed to initialize GDI display updates")?,
            ))
        }
    }

    /// In Provider mode, how long to wait for the WTS provider DLL to send `SetCaptureSessionId`
    /// before logging and continuing to wait. We intentionally do not fall back to a guessed
    /// session in provider mode to avoid pinning capture to prelogon desktops.
    const PROVIDER_SESSION_ID_WAIT_TIMEOUT: Duration = Duration::from_secs(30);

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CaptureOutputSource {
        HelperFrame,
        HelperPreEncodedFrame,
        GdiFrame,
        GdiFrameAfterHelperError,
        GdiFrameAfterHelperTimeout,
        CachedFrameAfterHelperError,
        CachedFrameAfterHelperTimeout,
        CachedFrameAfterGdiError,
        SyntheticTestPattern,
    }

    impl CaptureOutputSource {
        fn as_str(self) -> &'static str {
            match self {
                Self::HelperFrame => "helper_frame",
                Self::HelperPreEncodedFrame => "helper_preencoded_frame",
                Self::GdiFrame => "gdi_frame",
                Self::GdiFrameAfterHelperError => "gdi_frame_after_helper_error",
                Self::GdiFrameAfterHelperTimeout => "gdi_frame_after_helper_timeout",
                Self::CachedFrameAfterHelperError => "cached_frame_after_helper_error",
                Self::CachedFrameAfterHelperTimeout => "cached_frame_after_helper_timeout",
                Self::CachedFrameAfterGdiError => "cached_frame_after_gdi_error",
                Self::SyntheticTestPattern => "synthetic_test_pattern",
            }
        }
    }

    struct GdiDisplayUpdates {
        connection_id: u32,
        desktop_size: DesktopSize,
        input_stream_slot: Arc<Mutex<Option<TcpStream>>>,
        connection_session_ids: Arc<StdMutex<HashMap<u32, u32>>>,
        credentials_slot: Arc<Mutex<Option<StoredCredentials>>>,
        capture: Option<CaptureClient>,
        next_helper_attempt_at: Instant,
        sent_first_frame: bool,
        warned_blank_capture: bool,
        initial_blank_since: Option<Instant>,
        initial_blank_frames: u32,
        persistent_blank_since: Option<Instant>,
        persistent_blank_frames: u32,
        capture_restarted_for_blank: bool,
        last_bitmap: Option<BitmapUpdate>,
        helper_frames_received: u64,
        helper_timeouts: u64,
        /// True once at least one capture helper process has started successfully.
        /// Used to disable prelogon fallback token paths on later restart attempts.
        helper_started_once: bool,
        /// Session id used for the last logon/shell readiness probe.
        logon_readiness_session_id: Option<u32>,
        /// Result of the last throttled logon/shell readiness probe.
        logon_readiness_ready: bool,
        /// Next time we may probe logon/shell readiness.
        next_logon_readiness_probe_at: Instant,
        /// Set the first time we observe `session_id_override` but `WTSQueryUserToken` has not yet
        /// succeeded.  We delay spawning the capture helper until the user is logged in (or until
        /// we decide to fall back to a non-user token).
        waiting_for_user_login_since: Option<Instant>,
        /// Next time we may emit an automatic SAS while waiting for a real interactive token.
        next_auto_send_sas_at: Instant,
        /// Next time we may emit a detailed session-state snapshot while waiting for logon.
        next_waiting_for_user_login_diagnostic_at: Instant,
        /// Set to `true` after we have restarted the capture helper once the user has logged in.
        /// Prevents repeated restarts if `WTSQueryUserToken` briefly flaps.
        capture_restarted_for_logon: bool,
        /// First time we observed a valid user token for the provider-selected session while
        /// explorer was still missing.
        waiting_for_shell_ready_since: Option<Instant>,
        /// Next time we may attempt a best-effort shell bootstrap in provider mode.
        next_shell_bootstrap_attempt_at: Instant,
        /// The `session_id_override` value that was in effect when the capture helper was last
        /// started.
        ///
        /// - `None`: capture has not started yet
        /// - `Some(None)`: capture started without a WTS override (guessed session)
        /// - `Some(Some(id))`: capture started with an explicit WTS session id override
        ///
        /// Used to detect when the provider DLL sets (or changes) a session after capture already
        /// started with a guessed or different session.
        capture_started_with_session_override: Option<Option<u32>>,
        /// When `true` (Provider mode), hold off starting the capture helper until the WTS provider
        /// DLL sends `SetCaptureSessionId` so we don't waste frames on the wrong (guessed) session.
        /// Falls through after `PROVIDER_SESSION_ID_WAIT_TIMEOUT` to avoid blocking forever.
        provider_mode: bool,
        /// Deadline for the provider-session-ID wait.  Initialized lazily on first poll.
        wait_for_session_id_until: Option<Instant>,
        /// Last emitted capture output source marker for this connection.
        last_capture_output_source: Option<CaptureOutputSource>,
    }

    impl GdiDisplayUpdates {
        fn new(
            connection_id: u32,
            size: DesktopSize,
            input_stream_slot: Arc<Mutex<Option<TcpStream>>>,
            connection_session_ids: Arc<StdMutex<HashMap<u32, u32>>>,
            credentials_slot: Arc<Mutex<Option<StoredCredentials>>>,
            provider_mode: bool,
        ) -> anyhow::Result<Self> {
            let _ = desktop_size_nonzero(size)?;

            Ok(Self {
                connection_id,
                desktop_size: size,
                input_stream_slot,
                connection_session_ids,
                credentials_slot,
                capture: None,
                next_helper_attempt_at: Instant::now(),
                sent_first_frame: false,
                warned_blank_capture: false,
                initial_blank_since: None,
                initial_blank_frames: 0,
                persistent_blank_since: None,
                persistent_blank_frames: 0,
                capture_restarted_for_blank: false,
                last_bitmap: None,
                helper_frames_received: 0,
                helper_timeouts: 0,
                helper_started_once: false,
                logon_readiness_session_id: None,
                logon_readiness_ready: false,
                next_logon_readiness_probe_at: Instant::now(),
                waiting_for_user_login_since: None,
                next_auto_send_sas_at: Instant::now(),
                next_waiting_for_user_login_diagnostic_at: Instant::now(),
                capture_restarted_for_logon: false,
                waiting_for_shell_ready_since: None,
                next_shell_bootstrap_attempt_at: Instant::now(),
                capture_started_with_session_override: None,
                provider_mode,
                wait_for_session_id_until: None,
                last_capture_output_source: None,
            })
        }

        fn log_capture_output_source_transition(&mut self, source: CaptureOutputSource) {
            if self.last_capture_output_source == Some(source) {
                return;
            }

            info!(
                connection_id = self.connection_id,
                source = source.as_str(),
                "SESSION_PROOF_TERMSRV_CAPTURE_OUTPUT_SOURCE"
            );

            self.last_capture_output_source = Some(source);
        }
    }

    fn is_probably_blank_bgra32(data: &[u8]) -> bool {
        if data.is_empty() {
            return true;
        }

        // Sample the buffer to avoid scanning multi-megabyte frames on every tick.
        // If all sampled B/G/R bytes are zero, it's almost certainly a blocked/blank capture.
        let samples = 2048usize;
        let step = (data.len() / samples).max(4);
        let mut i = 0usize;
        while i + 2 < data.len() {
            if data[i] != 0 || data[i + 1] != 0 || data[i + 2] != 0 {
                return false;
            }
            i = i.saturating_add(step);
        }
        true
    }

    impl Drop for GdiDisplayUpdates {
        fn drop(&mut self) {
            if self.helper_frames_received > 0 || self.helper_timeouts > 0 {
                info!(
                    connection_id = self.connection_id,
                    helper_frames_received = self.helper_frames_received,
                    helper_timeouts = self.helper_timeouts,
                    "Display updates stopping"
                );
            }

            if let Some(capture) = self.capture.take() {
                capture.terminate();
            }
        }
    }

    #[async_trait::async_trait]
    impl RdpServerDisplayUpdates for GdiDisplayUpdates {
        async fn next_update(&mut self) -> anyhow::Result<Option<DisplayUpdate>> {
            // We loop here so that "no first frame yet" paths (capture helper timeout/error before
            // the first frame arrives) can retry without returning Ok(None) — returning Ok(None)
            // would cause the IronRDP server to disconnect the client immediately.
            loop {
                if self.sent_first_frame && self.capture.is_none() {
                    sleep(CAPTURE_INTERVAL).await;
                }

                let mut session_id_override = self
                    .connection_session_ids
                    .lock()
                    .ok()
                    .and_then(|guard| guard.get(&self.connection_id).copied());

                // In Provider mode, the WTS provider DLL will call SetCaptureSessionId from
                // ConnectNotify (~1-2 s after TCP accept) to tell us the correct TermService session.
                // Spin here until the session ID arrives so we never start the capture helper with
                // a wrong (guessed) session. We must NOT return Ok(None) because the IronRDP server
                // interprets None as "disconnect".
                if self.provider_mode && self.capture.is_none() && session_id_override.is_none() {
                    let deadline = self
                        .wait_for_session_id_until
                        .get_or_insert_with(|| Instant::now() + PROVIDER_SESSION_ID_WAIT_TIMEOUT);

                    while Instant::now() < *deadline {
                        sleep(Duration::from_millis(100)).await;
                        session_id_override = self
                            .connection_session_ids
                            .lock()
                            .ok()
                            .and_then(|guard| guard.get(&self.connection_id).copied());
                        if session_id_override.is_some() {
                            break;
                        }
                    }

                    if session_id_override.is_none() {
                        info!(
                            connection_id = self.connection_id,
                            "Still waiting for provider SetCaptureSessionId; keeping capture helper stopped"
                        );

                        *deadline = Instant::now() + PROVIDER_SESSION_ID_WAIT_TIMEOUT;
                        continue;
                    }
                }

                // In provider mode (IRONRDP_WTS_PROVIDER=1), note when the WTS provider assigns
                // a session so we can restart capture with the correct session and user token.
                // The session_override_arrived / user_logged_in restart logic below handles the
                // transition from the initial fallback capture to the real user desktop.

                // Restart the capture helper if a better session or user token has become available:
                //   - The WTS provider set session_id_override AFTER capture already started with a
                //     guessed session from resolve_capture_session_id.  Restart to target the correct
                //     TermService session.
                //   - The user has now logged into the TermService session.  Restart to pick up
                //     the real user desktop instead of the pre-login fallback (winlogon) desktop.
                //     We detect this via throttled WTSQueryUserToken probing.
                let session_override_changed = self
                    .capture_started_with_session_override
                    .is_some_and(|started_with| started_with != session_id_override);

                let should_probe_logon_readiness = self.logon_readiness_session_id != session_id_override
                    || Instant::now() >= self.next_logon_readiness_probe_at;

                if should_probe_logon_readiness {
                    let user_token_available = session_id_override.is_some_and(session_has_user_token);
                    let explorer_ready = session_id_override.is_some_and(session_has_explorer_token);
                    let interactive_shell_ready = user_token_available || explorer_ready;

                    self.logon_readiness_ready = interactive_shell_ready;
                    self.logon_readiness_session_id = session_id_override;
                    self.next_logon_readiness_probe_at = Instant::now() + LOGON_READINESS_PROBE_INTERVAL;

                    if self.provider_mode {
                        match session_id_override {
                            Some(_session_id) if explorer_ready || !user_token_available => {
                                self.waiting_for_shell_ready_since = None;
                            }
                            Some(session_id) => {
                                let now = Instant::now();
                                let waiting_since = self.waiting_for_shell_ready_since.get_or_insert(now);

                                if now.saturating_duration_since(*waiting_since) >= SHELL_BOOTSTRAP_GRACE
                                    && now >= self.next_shell_bootstrap_attempt_at
                                {
                                    self.next_shell_bootstrap_attempt_at = now + SHELL_BOOTSTRAP_RETRY_INTERVAL;

                                    info!(
                                        connection_id = self.connection_id,
                                        session_id,
                                        user_token_available,
                                        "SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_ATTEMPT"
                                    );

                                    let bootstrap_task = tokio::task::spawn_blocking(move || {
                                        try_start_explorer_process(session_id, false)
                                    });

                                    match timeout(Duration::from_secs(3), bootstrap_task).await {
                                        Ok(Ok(Ok(pid))) => {
                                            info!(
                                                connection_id = self.connection_id,
                                                session_id,
                                                explorer_pid = pid,
                                                "SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_SUCCESS"
                                            );

                                            if let Some(capture) = self.capture.take() {
                                                capture.terminate();

                                                info!(
                                                    connection_id = self.connection_id,
                                                    session_id,
                                                    explorer_pid = pid,
                                                    "Restarting capture helper after shell bootstrap success"
                                                );

                                                self.next_helper_attempt_at = Instant::now();
                                                self.warned_blank_capture = false;
                                                self.initial_blank_since = None;
                                                self.initial_blank_frames = 0;
                                                self.persistent_blank_since = None;
                                                self.persistent_blank_frames = 0;
                                                self.capture_restarted_for_blank = false;
                                                self.sent_first_frame = false;
                                                self.last_bitmap = None;
                                                self.helper_frames_received = 0;
                                                self.helper_timeouts = 0;
                                            }
                                        }
                                        Ok(Ok(Err(error))) => {
                                            warn!(
                                                connection_id = self.connection_id,
                                                session_id,
                                                error = %format!("{error:#}"),
                                                "SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_ERROR"
                                            );
                                        }
                                        Ok(Err(join_error)) => {
                                            warn!(
                                                connection_id = self.connection_id,
                                                session_id,
                                                error = %join_error,
                                                "SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_ERROR"
                                            );
                                        }
                                        Err(_) => {
                                            warn!(
                                                connection_id = self.connection_id,
                                                session_id,
                                                error = "bootstrap attempt timed out",
                                                "SESSION_PROOF_TERMSRV_SHELL_BOOTSTRAP_ERROR"
                                            );
                                        }
                                    }
                                }
                            }
                            _ => {
                                self.waiting_for_shell_ready_since = None;
                            }
                        }
                    }
                }

                let should_restart_for_logon = self.logon_readiness_ready && !self.capture_restarted_for_logon;
                let should_restart = self.capture.is_some() && (session_override_changed || should_restart_for_logon);

                if should_restart {
                    let session_id = session_id_override.unwrap_or(0);
                    if should_restart_for_logon {
                        info!(
                            connection_id = self.connection_id,
                            session_id, "User has logged in; restarting capture helper to pick up real user desktop"
                        );
                        self.capture_restarted_for_logon = true;
                    } else {
                        info!(
                            connection_id = self.connection_id,
                            session_id,
                            "WTS session assigned after capture started; restarting capture helper on correct session"
                        );
                    }

                    if let Some(capture) = self.capture.take() {
                        capture.terminate();
                    }

                    self.next_helper_attempt_at = Instant::now();
                    self.warned_blank_capture = false;
                    self.initial_blank_since = None;
                    self.initial_blank_frames = 0;
                    self.persistent_blank_since = None;
                    self.persistent_blank_frames = 0;
                    self.capture_restarted_for_blank = false;
                    self.sent_first_frame = false;
                    self.last_bitmap = None;
                    self.helper_frames_received = 0;
                    self.helper_timeouts = 0;
                    self.logon_readiness_ready = false;
                    self.logon_readiness_session_id = None;
                    self.next_logon_readiness_probe_at = Instant::now();
                    self.waiting_for_shell_ready_since = None;
                    self.next_shell_bootstrap_attempt_at = Instant::now();
                    self.next_auto_send_sas_at = Instant::now();
                    self.capture_started_with_session_override = None;
                    self.waiting_for_user_login_since = None;
                }

                if self.capture.is_none() && Instant::now() >= self.next_helper_attempt_at {
                    // In provider mode, wait for a real interactive user token before starting the
                    // helper. Launching the first helper on a prelogon token pins capture to the
                    // wrong desktop and can fail process startup during session bring-up.
                    if let Some(session_id) = session_id_override {
                        let launch_user_token_available = session_has_user_token(session_id);
                        let launch_explorer_ready = session_has_explorer_token(session_id);
                        if launch_user_token_available || launch_explorer_ready {
                            self.waiting_for_user_login_since = None;
                            self.next_auto_send_sas_at = Instant::now();
                            self.next_waiting_for_user_login_diagnostic_at = Instant::now();
                        } else if self.provider_mode {
                            let now = Instant::now();
                            let waiting_since = self.waiting_for_user_login_since.get_or_insert(now);

                            if *waiting_since == now {
                                info!(
                                    connection_id = self.connection_id,
                                    session_id,
                                    "Waiting for interactive user token before starting capture helper"
                                );
                                info!(
                                    connection_id = self.connection_id,
                                    session_id,
                                    "SESSION_PROOF_TERMSRV_WAITING_FOR_USER_LOGON"
                                );
                            }

                            if now >= self.next_waiting_for_user_login_diagnostic_at {
                                self.next_waiting_for_user_login_diagnostic_at =
                                    now + WAITING_FOR_USER_LOGON_DIAGNOSTIC_INTERVAL;

                                info!(
                                    connection_id = self.connection_id,
                                    session_id,
                                    snapshot = %session_selection_snapshot(),
                                    "SESSION_PROOF_TERMSRV_WAITING_FOR_USER_LOGON_STATE"
                                );
                            }

                            if auto_send_sas_enabled() && now >= self.next_auto_send_sas_at {
                                self.next_auto_send_sas_at = now + AUTO_SEND_SAS_RETRY_INTERVAL;

                                if try_send_sas("provider_wait_for_user_logon") {
                                    info!(
                                        connection_id = self.connection_id,
                                        session_id,
                                        "SESSION_PROOF_TERMSRV_SAS_SENT"
                                    );
                                }
                            }

                            sleep(CAPTURE_INTERVAL).await;
                            continue;
                        }
                    }

                    let captured_credentials = { self.credentials_slot.lock().await.clone() };
                    let allow_prelogon_fallback = !self.provider_mode && !self.helper_started_once;

                    let launch_user_token_available = session_id_override.is_some_and(session_has_user_token);
                    let launch_explorer_ready = session_id_override.is_some_and(session_has_explorer_token);
                    info!(
                        connection_id = self.connection_id,
                        session_id = session_id_override.unwrap_or(0),
                        has_session_override = session_id_override.is_some(),
                        launch_user_token_available,
                        launch_explorer_ready,
                        allow_prelogon_fallback,
                        "Capture helper launch attempt"
                    );

                    match CaptureClient::start(
                        self.connection_id,
                        self.desktop_size,
                        Arc::clone(&self.input_stream_slot),
                        session_id_override,
                        captured_credentials,
                        allow_prelogon_fallback,
                    )
                    .await
                    {
                        Ok(capture) => {
                            info!(
                                connection_id = self.connection_id,
                                helper_pid = capture.pid(),
                                "Started interactive capture helper"
                            );
                            info!(
                                connection_id = self.connection_id,
                                session_id = session_id_override.unwrap_or(0),
                                has_session_override = session_id_override.is_some(),
                                helper_pid = capture.pid(),
                                "SESSION_PROOF_TERMSRV_CAPTURE_HELPER_STARTED"
                            );
                            self.initial_blank_since = None;
                            self.initial_blank_frames = 0;
                            self.persistent_blank_since = None;
                            self.persistent_blank_frames = 0;
                            self.helper_started_once = true;
                            self.capture_started_with_session_override = Some(session_id_override);
                            self.capture = Some(capture);
                        }
                        Err(error) => {
                            warn!(
                                connection_id = self.connection_id,
                                error = %format!("{error:#}"),
                                "Failed to start interactive capture helper; falling back to in-process GDI"
                            );
                            warn!(
                                connection_id = self.connection_id,
                                error = %format!("{error:#}"),
                                "SESSION_PROOF_TERMSRV_CAPTURE_HELPER_FALLBACK_GDI"
                            );
                            self.next_helper_attempt_at = Instant::now() + CAPTURE_HELPER_RETRY_DELAY;
                        }
                    }
                }

                let (captured, capture_output_source) = if let Some(capture) = &mut self.capture {
                    let read_timeout = if self.helper_frames_received == 0 {
                        Duration::from_secs(1)
                    } else {
                        CAPTURE_INTERVAL
                    };

                    match timeout(read_timeout, capture.read_frame()).await {
                        Ok(Ok(frame)) => {
                            self.helper_frames_received = self.helper_frames_received.saturating_add(1);
                            (frame, CaptureOutputSource::HelperFrame)
                        }
                        Ok(Err(error)) => {
                            warn!(
                                connection_id = self.connection_id,
                                error = %format!("{error:#}"),
                                "Interactive capture helper failed"
                            );
                            let capture = self.capture.take();
                            if let Some(capture) = capture {
                                capture.terminate();
                            }
                            self.next_helper_attempt_at = Instant::now() + CAPTURE_HELPER_RETRY_DELAY;

                            if let Some(bitmap) = self.last_bitmap.clone() {
                                (
                                    CapturedFrame::Raw(bitmap),
                                    CaptureOutputSource::CachedFrameAfterHelperError,
                                )
                            } else {
                                match capture_bitmap_update(self.desktop_size) {
                                    Ok(bitmap) => {
                                        (CapturedFrame::Raw(bitmap), CaptureOutputSource::GdiFrameAfterHelperError)
                                    }
                                    Err(capture_error) => {
                                        warn!(
                                            connection_id = self.connection_id,
                                            error = %format!("{capture_error:#}"),
                                            "Capture helper failed before first frame and GDI fallback failed"
                                        );
                                        sleep(CAPTURE_INTERVAL).await;
                                        continue;
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            self.helper_timeouts = self.helper_timeouts.saturating_add(1);

                            if self.helper_frames_received == 0 && self.helper_timeouts >= 3 {
                                warn!(
                                    connection_id = self.connection_id,
                                    helper_timeouts = self.helper_timeouts,
                                    "Capture helper timed out before first frame; restarting helper"
                                );

                                if let Some(capture) = self.capture.take() {
                                    capture.terminate();
                                }

                                self.next_helper_attempt_at = Instant::now();
                                self.helper_timeouts = 0;
                                sleep(CAPTURE_INTERVAL).await;
                                continue;
                            }

                            sleep(CAPTURE_INTERVAL).await;

                            if let Some(bitmap) = self.last_bitmap.clone() {
                                (
                                    CapturedFrame::Raw(bitmap),
                                    CaptureOutputSource::CachedFrameAfterHelperTimeout,
                                )
                            } else {
                                match capture_bitmap_update(self.desktop_size) {
                                    Ok(bitmap) => {
                                        (CapturedFrame::Raw(bitmap), CaptureOutputSource::GdiFrameAfterHelperTimeout)
                                    }
                                    Err(capture_error) => {
                                        warn!(
                                            connection_id = self.connection_id,
                                            error = %format!("{capture_error:#}"),
                                            "Capture helper timed out before first frame and GDI fallback failed"
                                        );
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                } else {
                    match capture_bitmap_update(self.desktop_size) {
                        Ok(bitmap) => (CapturedFrame::Raw(bitmap), CaptureOutputSource::GdiFrame),
                        Err(error) => {
                            if let Some(bitmap) = self.last_bitmap.clone() {
                                warn!(
                                    error = %format!("{error:#}"),
                                    "GDI capture failed; re-sending last bitmap"
                                );
                                (
                                    CapturedFrame::Raw(bitmap),
                                    CaptureOutputSource::CachedFrameAfterGdiError,
                                )
                            } else if self.provider_mode {
                                warn!(
                                    error = %format!("{error:#}"),
                                    "GDI capture failed with no cached frame in provider mode; waiting for helper retry"
                                );
                                sleep(CAPTURE_INTERVAL).await;
                                continue;
                            } else {
                                warn!(
                                    error = %format!("{error:#}"),
                                    "GDI capture failed; sending synthetic test pattern"
                                );
                                let bitmap = fallback_bitmap_update(self.desktop_size)
                                    .context("failed to generate fallback bitmap update")?;
                                (CapturedFrame::Raw(bitmap), CaptureOutputSource::SyntheticTestPattern)
                            }
                        }
                    }
                };

                self.log_capture_output_source_transition(capture_output_source);

                match captured {
                    CapturedFrame::PreEncoded(surface) => {
                        self.log_capture_output_source_transition(CaptureOutputSource::HelperPreEncodedFrame);
                        self.initial_blank_since = None;
                        self.initial_blank_frames = 0;
                        self.sent_first_frame = true;
                        return Ok(Some(DisplayUpdate::PreEncodedSurface(surface)));
                    }
                    CapturedFrame::Raw(bitmap) => {
                        let is_blank = is_probably_blank_bgra32(bitmap.data.as_ref());
                        let now = Instant::now();

                        if !self.sent_first_frame && is_blank {
                            let initial_blank_since = self.initial_blank_since.get_or_insert(now);
                            self.initial_blank_frames = self.initial_blank_frames.saturating_add(1);

                            let blank_elapsed = now.saturating_duration_since(*initial_blank_since);
                            let still_in_grace = blank_elapsed < FIRST_FRAME_BLANK_GRACE
                                && self.initial_blank_frames < FIRST_FRAME_BLANK_MAX_FRAMES;

                            if still_in_grace {
                                if !self.warned_blank_capture {
                                    self.warned_blank_capture = true;
                                    debug!(
                                        connection_id = self.connection_id,
                                        "Captured blank frame before first meaningful update; waiting for initialized desktop"
                                    );
                                }

                                sleep(CAPTURE_INTERVAL).await;
                                continue;
                            }

                            info!(
                                connection_id = self.connection_id,
                                blank_frames = self.initial_blank_frames,
                                blank_elapsed_ms = blank_elapsed.as_millis(),
                                "Sending blank first frame after startup grace period"
                            );
                        }

                        if !self.sent_first_frame {
                            self.initial_blank_since = None;
                            self.initial_blank_frames = 0;
                            self.persistent_blank_since = None;
                            self.persistent_blank_frames = 0;
                        }

                        if is_blank && !self.warned_blank_capture {
                            self.warned_blank_capture = true;
                            debug!(
                                connection_id = self.connection_id,
                                "Captured blank frame; sending as-is after first meaningful update"
                            );
                        }

                        if self.sent_first_frame && self.capture.is_some() && is_blank {
                            let blank_since = self.persistent_blank_since.get_or_insert(now);
                            self.persistent_blank_frames = self.persistent_blank_frames.saturating_add(1);

                            let blank_elapsed = now.saturating_duration_since(*blank_since);
                            let should_restart_for_blank = !self.capture_restarted_for_blank
                                && self.logon_readiness_ready
                                && blank_elapsed >= PERSISTENT_BLANK_RESTART_GRACE
                                && self.persistent_blank_frames >= PERSISTENT_BLANK_RESTART_MIN_FRAMES;

                            if should_restart_for_blank {
                                info!(
                                    connection_id = self.connection_id,
                                    blank_frames = self.persistent_blank_frames,
                                    blank_elapsed_ms = blank_elapsed.as_millis(),
                                    "Persistent blank capture detected; restarting capture helper once"
                                );

                                if let Some(capture) = self.capture.take() {
                                    capture.terminate();
                                }

                                self.capture_restarted_for_blank = true;
                                self.next_helper_attempt_at = Instant::now();
                                self.warned_blank_capture = false;
                                self.initial_blank_since = None;
                                self.initial_blank_frames = 0;
                                self.persistent_blank_since = None;
                                self.persistent_blank_frames = 0;
                                self.sent_first_frame = false;
                                self.last_bitmap = None;
                                self.helper_frames_received = 0;
                                self.helper_timeouts = 0;

                                sleep(CAPTURE_INTERVAL).await;
                                continue;
                            }
                        } else if !is_blank {
                            self.persistent_blank_since = None;
                            self.persistent_blank_frames = 0;
                        }

                        self.sent_first_frame = true;
                        self.last_bitmap = Some(bitmap.clone());
                        return Ok(Some(DisplayUpdate::Bitmap(bitmap)));
                    }
                }
            } // end loop
        }
    }

    #[derive(Clone, Copy)]
    struct SendHandle(HANDLE);

    // SAFETY: Windows kernel object handles can be sent and used across threads.
    unsafe impl Send for SendHandle {}
    // SAFETY: Windows kernel object handles can be shared across threads.
    unsafe impl Sync for SendHandle {}

    #[derive(Clone, Copy)]
    struct SendMappedView(windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS);

    // SAFETY: this wraps a process-local mapped view pointer; we only use it while the mapping is alive.
    unsafe impl Send for SendMappedView {}
    // SAFETY: access is coordinated by &mut self on the owning client; sharing the address is fine.
    unsafe impl Sync for SendMappedView {}

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum CaptureIpc {
        Tcp,
        SharedMem,
    }

    fn capture_ipc_from_env() -> CaptureIpc {
        let configured = std::env::var(CAPTURE_IPC_ENV)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "tcp".to_owned());

        match configured.to_ascii_lowercase().as_str() {
            "tcp" => CaptureIpc::Tcp,
            "shm" | "sharedmem" | "shared-memory" => CaptureIpc::SharedMem,
            _ => CaptureIpc::Tcp,
        }
    }

    enum CapturedFrame {
        Raw(BitmapUpdate),
        PreEncoded(ironrdp_server::PreEncodedSurface),
    }

    enum CaptureClient {
        Tcp(HelperCaptureClient),
        SharedMem(SharedMemCaptureClient),
    }

    impl CaptureClient {
        async fn start(
            connection_id: u32,
            desktop_size: DesktopSize,
            input_stream_slot: Arc<Mutex<Option<TcpStream>>>,
            session_id_override: Option<u32>,
            credentials: Option<StoredCredentials>,
            allow_prelogon_fallback: bool,
        ) -> anyhow::Result<Self> {
            match capture_ipc_from_env() {
                CaptureIpc::Tcp => Ok(Self::Tcp(
                    HelperCaptureClient::start(
                        connection_id,
                        input_stream_slot,
                        session_id_override,
                        credentials,
                        allow_prelogon_fallback,
                    )
                    .await?,
                )),
                CaptureIpc::SharedMem => {
                    match SharedMemCaptureClient::start(
                        connection_id,
                        desktop_size,
                        Arc::clone(&input_stream_slot),
                        session_id_override,
                        credentials.clone(),
                        allow_prelogon_fallback,
                    )
                    .await
                    {
                        Ok(client) => Ok(Self::SharedMem(client)),
                        Err(error) => {
                            warn!(
                                connection_id,
                                error = %format!("{error:#}"),
                                "Shared-memory capture IPC failed; falling back to TCP"
                            );
                            Ok(Self::Tcp(
                                HelperCaptureClient::start(
                                    connection_id,
                                    input_stream_slot,
                                    session_id_override,
                                    credentials,
                                    allow_prelogon_fallback,
                                )
                                .await?,
                            ))
                        }
                    }
                }
            }
        }

        fn pid(&self) -> u32 {
            match self {
                Self::Tcp(client) => client.pid(),
                Self::SharedMem(client) => client.pid(),
            }
        }

        fn terminate(self) {
            match self {
                Self::Tcp(client) => client.terminate(),
                Self::SharedMem(client) => client.terminate(),
            }
        }

        async fn read_frame(&mut self) -> anyhow::Result<CapturedFrame> {
            match self {
                Self::Tcp(client) => client.read_frame().await,
                Self::SharedMem(client) => client.read_frame().await.map(CapturedFrame::Raw),
            }
        }
    }

    struct HelperCaptureClient {
        helper_pid: u32,
        helper_process: SendHandle,
        input_stream_slot: Arc<Mutex<Option<TcpStream>>>,
        stream: TcpStream,
    }

    impl HelperCaptureClient {
        async fn start(
            connection_id: u32,
            input_stream_slot: Arc<Mutex<Option<TcpStream>>>,
            session_id_override: Option<u32>,
            credentials: Option<StoredCredentials>,
            allow_prelogon_fallback: bool,
        ) -> anyhow::Result<Self> {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .context("failed to bind local capture helper listener")?;

            let input_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .context("failed to bind local capture helper input listener")?;

            let local_addr = listener
                .local_addr()
                .context("failed to query local helper listener address")?;

            let input_addr = input_listener
                .local_addr()
                .context("failed to query local helper input listener address")?;

            info!(connection_id, input_addr = %input_addr, "Capture helper input listener bound");

            let helper = spawn_capture_helper_process_tcp(
                local_addr,
                input_addr,
                true,
                session_id_override,
                credentials,
                allow_prelogon_fallback,
            )
            .with_context(|| format!("failed to spawn capture helper for connection {connection_id}"))?;

            info!(
                connection_id,
                helper_pid = helper.pid,
                "Waiting for capture helper TCP connection"
            );

            let (stream, _peer) = match timeout(CAPTURE_HELPER_CONNECT_TIMEOUT, listener.accept()).await {
                Ok(Ok(pair)) => {
                    info!(connection_id, helper_pid = helper.pid, "Capture helper capture stream connected");
                    pair
                }
                Ok(Err(accept_err)) => {
                    // Check if the helper process has exited
                    let mut exit_code = 0u32;
                    let exited = unsafe {
                        windows::Win32::System::Threading::GetExitCodeProcess(helper.process.0, &mut exit_code)
                    };
                    warn!(
                        connection_id,
                        helper_pid = helper.pid,
                        exit_code,
                        exited_ok = exited.is_ok(),
                        still_active = (exit_code == 259), // STILL_ACTIVE
                        error = %accept_err,
                        "Capture helper: accept failed"
                    );
                    return Err(accept_err).context("failed to accept capture helper connection");
                }
                Err(_timeout) => {
                    // Check if the helper process has exited
                    let mut exit_code = 0u32;
                    let exited = unsafe {
                        windows::Win32::System::Threading::GetExitCodeProcess(helper.process.0, &mut exit_code)
                    };
                    warn!(
                        connection_id,
                        helper_pid = helper.pid,
                        exit_code,
                        exited_ok = exited.is_ok(),
                        still_active = (exit_code == 259), // STILL_ACTIVE
                        "Capture helper did not connect within timeout (process exit check)"
                    );
                    return Err(anyhow!("capture helper did not connect within timeout (pid={}, exit_code={exit_code}, still_active={})", helper.pid, exit_code == 259));
                }
            };

            let (input_stream, _peer) = timeout(CAPTURE_HELPER_CONNECT_TIMEOUT, input_listener.accept())
                .await
                .map_err(|_| anyhow!("capture helper input did not connect within timeout"))?
                .context("failed to accept capture helper input connection")?;

            info!(connection_id, "Capture helper input channel connected");

            {
                let mut guard = input_stream_slot.lock().await;
                *guard = Some(input_stream);
            }

            Ok(Self {
                helper_pid: helper.pid,
                helper_process: helper.process,
                input_stream_slot,
                stream,
            })
        }

        fn pid(&self) -> u32 {
            self.helper_pid
        }

        fn terminate(self) {
            if let Ok(mut guard) = self.input_stream_slot.try_lock() {
                *guard = None;
            }

            // SAFETY: handle was returned by CreateProcessAsUserW.
            unsafe {
                let _ = TerminateProcess(self.helper_process.0, 1);
            }

            // SAFETY: handle was returned by CreateProcessAsUserW.
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.helper_process.0);
            }
        }

        async fn read_frame(&mut self) -> anyhow::Result<CapturedFrame> {
            read_capture_frame(&mut self.stream).await
        }
    }

    const SHM_FB_MAGIC: [u8; 4] = *b"IRFB";
    const SHM_FB_VERSION: u32 = 1;
    const SHM_FB_HEADER_LEN: usize = 64;
    const SHM_FB_SLOTS: usize = 2;

    const SHM_OFF_MAGIC: usize = 0;
    const SHM_OFF_VERSION: usize = 4;
    const SHM_OFF_WIDTH: usize = 8;
    const SHM_OFF_HEIGHT: usize = 10;
    const SHM_OFF_STRIDE: usize = 12;
    const SHM_OFF_SLOT_LEN: usize = 16;
    const SHM_OFF_SLOTS: usize = 20;
    const SHM_OFF_PUBLISHED_SLOT: usize = 24;
    const SHM_OFF_PAYLOAD_LEN: usize = 28;
    const SHM_OFF_SEQ: usize = 32;

    unsafe fn shm_read_u16(view: *const u8, offset: usize, view_len: usize) -> anyhow::Result<u16> {
        let end = offset
            .checked_add(2)
            .ok_or_else(|| anyhow!("shared memory read overflow"))?;
        if end > view_len {
            return Err(anyhow!("shared memory read out of bounds"));
        }
        // SAFETY: bounds checked above; caller guarantees `view` is valid for `view_len` bytes.
        let ptr = unsafe { view.add(offset) };
        // SAFETY: ptr points inside the mapped view; unaligned read is permitted.
        let value = unsafe { core::ptr::read_unaligned(ptr.cast::<u16>()) };
        Ok(u16::from_le(value))
    }

    unsafe fn shm_read_u32(view: *const u8, offset: usize, view_len: usize) -> anyhow::Result<u32> {
        let end = offset
            .checked_add(4)
            .ok_or_else(|| anyhow!("shared memory read overflow"))?;
        if end > view_len {
            return Err(anyhow!("shared memory read out of bounds"));
        }
        // SAFETY: bounds checked above; caller guarantees `view` is valid for `view_len` bytes.
        let ptr = unsafe { view.add(offset) };
        // SAFETY: ptr points inside the mapped view; unaligned read is permitted.
        let value = unsafe { core::ptr::read_unaligned(ptr.cast::<u32>()) };
        Ok(u32::from_le(value))
    }

    unsafe fn shm_read_u64(view: *const u8, offset: usize, view_len: usize) -> anyhow::Result<u64> {
        let end = offset
            .checked_add(8)
            .ok_or_else(|| anyhow!("shared memory read overflow"))?;
        if end > view_len {
            return Err(anyhow!("shared memory read out of bounds"));
        }
        // SAFETY: bounds checked above; caller guarantees `view` is valid for `view_len` bytes.
        let ptr = unsafe { view.add(offset) };
        // SAFETY: ptr points inside the mapped view; unaligned read is permitted.
        let value = unsafe { core::ptr::read_unaligned(ptr.cast::<u64>()) };
        Ok(u64::from_le(value))
    }

    unsafe fn shm_write_u16(view: *mut u8, offset: usize, value: u16, view_len: usize) -> anyhow::Result<()> {
        let end = offset
            .checked_add(2)
            .ok_or_else(|| anyhow!("shared memory write overflow"))?;
        if end > view_len {
            return Err(anyhow!("shared memory write out of bounds"));
        }
        // SAFETY: bounds checked above; caller guarantees `view` is valid for `view_len` bytes.
        let ptr = unsafe { view.add(offset) };
        // SAFETY: ptr points inside the mapped view; unaligned write is permitted.
        unsafe { core::ptr::write_unaligned(ptr.cast::<u16>(), value.to_le()) };
        Ok(())
    }

    unsafe fn shm_write_u32(view: *mut u8, offset: usize, value: u32, view_len: usize) -> anyhow::Result<()> {
        let end = offset
            .checked_add(4)
            .ok_or_else(|| anyhow!("shared memory write overflow"))?;
        if end > view_len {
            return Err(anyhow!("shared memory write out of bounds"));
        }
        // SAFETY: bounds checked above; caller guarantees `view` is valid for `view_len` bytes.
        let ptr = unsafe { view.add(offset) };
        // SAFETY: ptr points inside the mapped view; unaligned write is permitted.
        unsafe { core::ptr::write_unaligned(ptr.cast::<u32>(), value.to_le()) };
        Ok(())
    }

    unsafe fn shm_write_u64(view: *mut u8, offset: usize, value: u64, view_len: usize) -> anyhow::Result<()> {
        let end = offset
            .checked_add(8)
            .ok_or_else(|| anyhow!("shared memory write overflow"))?;
        if end > view_len {
            return Err(anyhow!("shared memory write out of bounds"));
        }
        // SAFETY: bounds checked above; caller guarantees `view` is valid for `view_len` bytes.
        let ptr = unsafe { view.add(offset) };
        // SAFETY: ptr points inside the mapped view; unaligned write is permitted.
        unsafe { core::ptr::write_unaligned(ptr.cast::<u64>(), value.to_le()) };
        Ok(())
    }

    unsafe fn shm_init_header(
        view: *mut u8,
        width: NonZeroU16,
        height: NonZeroU16,
        stride: NonZeroUsize,
        slot_len: usize,
    ) -> anyhow::Result<()> {
        let view_len = SHM_FB_HEADER_LEN;
        // SAFETY: header is at least SHM_FB_HEADER_LEN bytes.
        let magic_dst = unsafe { view.add(SHM_OFF_MAGIC) };
        // SAFETY: magic_dst points within the header and is valid for 4 bytes.
        unsafe { core::ptr::copy_nonoverlapping(SHM_FB_MAGIC.as_ptr(), magic_dst, 4) };
        // SAFETY: view points to a mapped header region with validated bounds.
        unsafe { shm_write_u32(view, SHM_OFF_VERSION, SHM_FB_VERSION, view_len) }?;
        // SAFETY: view points to a mapped header region with validated bounds.
        unsafe { shm_write_u16(view, SHM_OFF_WIDTH, width.get(), view_len) }?;
        // SAFETY: view points to a mapped header region with validated bounds.
        unsafe { shm_write_u16(view, SHM_OFF_HEIGHT, height.get(), view_len) }?;
        // SAFETY: view points to a mapped header region with validated bounds.
        unsafe {
            shm_write_u32(
                view,
                SHM_OFF_STRIDE,
                u32::try_from(stride.get()).map_err(|_| anyhow!("stride out of range"))?,
                view_len,
            )
        }?;
        // SAFETY: view points to a mapped header region with validated bounds.
        unsafe {
            shm_write_u32(
                view,
                SHM_OFF_SLOT_LEN,
                u32::try_from(slot_len).map_err(|_| anyhow!("slot length out of range"))?,
                view_len,
            )
        }?;
        // SAFETY: view points to a mapped header region with validated bounds.
        unsafe { shm_write_u32(view, SHM_OFF_SLOTS, u32::try_from(SHM_FB_SLOTS).unwrap_or(2), view_len) }?;
        // SAFETY: view points to a mapped header region with validated bounds.
        unsafe { shm_write_u32(view, SHM_OFF_PUBLISHED_SLOT, 0, view_len) }?;
        // SAFETY: view points to a mapped header region with validated bounds.
        unsafe { shm_write_u32(view, SHM_OFF_PAYLOAD_LEN, 0, view_len) }?;
        // SAFETY: view points to a mapped header region with validated bounds.
        unsafe { shm_write_u64(view, SHM_OFF_SEQ, 0, view_len) }?;
        Ok(())
    }

    unsafe fn shm_read_published_meta(view: *const u8, view_len: usize) -> anyhow::Result<(u64, u32, u32)> {
        if view_len < SHM_FB_HEADER_LEN {
            return Err(anyhow!("shared memory view too small"));
        }

        // SAFETY: header is at least SHM_FB_HEADER_LEN bytes.
        let magic_ptr = unsafe { view.add(SHM_OFF_MAGIC) };
        // SAFETY: magic_ptr points within the mapped header and is valid for 4 bytes.
        let magic = unsafe { core::slice::from_raw_parts(magic_ptr, 4) };
        if magic != SHM_FB_MAGIC {
            return Err(anyhow!("shared memory magic mismatch"));
        }

        // SAFETY: caller guarantees `view` is a valid mapping and header bounds were checked.
        let version = unsafe { shm_read_u32(view, SHM_OFF_VERSION, SHM_FB_HEADER_LEN) }?;
        if version != SHM_FB_VERSION {
            return Err(anyhow!("unsupported shared memory version: {version}"));
        }

        // SAFETY: caller guarantees `view` is a valid mapping and header bounds were checked.
        let slots = unsafe { shm_read_u32(view, SHM_OFF_SLOTS, SHM_FB_HEADER_LEN) }?;
        if usize::try_from(slots).ok() != Some(SHM_FB_SLOTS) {
            return Err(anyhow!("unexpected shared memory slot count: {slots}"));
        }

        // SAFETY: caller guarantees `view` is a valid mapping and header bounds were checked.
        let seq = unsafe { shm_read_u64(view, SHM_OFF_SEQ, SHM_FB_HEADER_LEN) }?;
        // SAFETY: caller guarantees `view` is a valid mapping and header bounds were checked.
        let published_slot = unsafe { shm_read_u32(view, SHM_OFF_PUBLISHED_SLOT, SHM_FB_HEADER_LEN) }?;
        // SAFETY: caller guarantees `view` is a valid mapping and header bounds were checked.
        let payload_len = unsafe { shm_read_u32(view, SHM_OFF_PAYLOAD_LEN, SHM_FB_HEADER_LEN) }?;
        Ok((seq, published_slot, payload_len))
    }

    unsafe fn shm_read_layout(
        view: *const u8,
        view_len: usize,
    ) -> anyhow::Result<(NonZeroU16, NonZeroU16, NonZeroUsize, usize)> {
        if view_len < SHM_FB_HEADER_LEN {
            return Err(anyhow!("shared memory view too small"));
        }

        // SAFETY: caller guarantees `view` is a valid mapping and header bounds were checked.
        let width = NonZeroU16::new(unsafe { shm_read_u16(view, SHM_OFF_WIDTH, SHM_FB_HEADER_LEN) }?)
            .ok_or_else(|| anyhow!("shared memory width is zero"))?;
        // SAFETY: caller guarantees `view` is a valid mapping and header bounds were checked.
        let height = NonZeroU16::new(unsafe { shm_read_u16(view, SHM_OFF_HEIGHT, SHM_FB_HEADER_LEN) }?)
            .ok_or_else(|| anyhow!("shared memory height is zero"))?;
        // SAFETY: caller guarantees `view` is a valid mapping and header bounds were checked.
        let stride_u32 = unsafe { shm_read_u32(view, SHM_OFF_STRIDE, SHM_FB_HEADER_LEN) }?;
        let stride_usize = usize::try_from(stride_u32).map_err(|_| anyhow!("shared memory stride out of range"))?;
        let stride = NonZeroUsize::new(stride_usize).ok_or_else(|| anyhow!("shared memory stride is zero"))?;
        // SAFETY: caller guarantees `view` is a valid mapping and header bounds were checked.
        let slot_len_u32 = unsafe { shm_read_u32(view, SHM_OFF_SLOT_LEN, SHM_FB_HEADER_LEN) }?;
        let slot_len = usize::try_from(slot_len_u32).map_err(|_| anyhow!("shared memory slot length out of range"))?;
        Ok((width, height, stride, slot_len))
    }

    unsafe fn shm_publish_frame(
        view: *mut u8,
        view_len: usize,
        slot_idx: usize,
        slot_len: usize,
        seq: u64,
        payload: &[u8],
    ) -> anyhow::Result<()> {
        if slot_idx >= SHM_FB_SLOTS {
            return Err(anyhow!("slot index out of range: {slot_idx}"));
        }
        if payload.len() != slot_len {
            return Err(anyhow!(
                "payload length mismatch: got {}, expected {slot_len}",
                payload.len()
            ));
        }

        let slot_offset = SHM_FB_HEADER_LEN + slot_idx * slot_len;
        let end = slot_offset
            .checked_add(slot_len)
            .ok_or_else(|| anyhow!("shared memory slot overflow"))?;
        if end > view_len {
            return Err(anyhow!("shared memory slot out of bounds"));
        }

        // SAFETY: bounds checked above; caller guarantees `view` is valid for `view_len` bytes.
        let slot_ptr = unsafe { view.add(slot_offset) };
        // SAFETY: slot_ptr points within the mapped view and is valid for slot_len bytes.
        unsafe { core::ptr::copy_nonoverlapping(payload.as_ptr(), slot_ptr, slot_len) };
        fence(Ordering::SeqCst);

        // SAFETY: header is within the mapped view.
        unsafe {
            shm_write_u32(
                view,
                SHM_OFF_PUBLISHED_SLOT,
                u32::try_from(slot_idx).unwrap_or(0),
                SHM_FB_HEADER_LEN,
            )
        }?;
        // SAFETY: header is within the mapped view.
        unsafe {
            shm_write_u32(
                view,
                SHM_OFF_PAYLOAD_LEN,
                u32::try_from(slot_len).map_err(|_| anyhow!("payload length out of range"))?,
                SHM_FB_HEADER_LEN,
            )
        }?;
        // SAFETY: header is within the mapped view.
        unsafe { shm_write_u64(view, SHM_OFF_SEQ, seq, SHM_FB_HEADER_LEN) }?;
        fence(Ordering::SeqCst);
        Ok(())
    }

    struct SharedMemCaptureClient {
        helper_pid: u32,
        helper_process: SendHandle,
        input_stream_slot: Arc<Mutex<Option<TcpStream>>>,
        mapping: SendHandle,
        frame_ready_event: SendHandle,
        view: SendMappedView,
        view_len: usize,
        width: NonZeroU16,
        height: NonZeroU16,
        stride: NonZeroUsize,
        slot_len: usize,
        last_seq: u64,
    }

    // SAFETY: the mapped view address points to a process-local memory-mapped region.
    // It is valid to access from any thread as long as the mapping stays alive, and we
    // keep it alive for the lifetime of this client.
    unsafe impl Send for SharedMemCaptureClient {}
    // SAFETY: the view is only mutated via &mut self methods (single writer per instance).
    unsafe impl Sync for SharedMemCaptureClient {}

    impl SharedMemCaptureClient {
        async fn start(
            connection_id: u32,
            desktop_size: DesktopSize,
            input_stream_slot: Arc<Mutex<Option<TcpStream>>>,
            session_id_override: Option<u32>,
            credentials: Option<StoredCredentials>,
            allow_prelogon_fallback: bool,
        ) -> anyhow::Result<Self> {
            let (width, height) = desktop_size_nonzero(desktop_size)?;
            let width_usize = NonZeroUsize::from(width).get();
            let height_usize = NonZeroUsize::from(height).get();

            let stride_usize = width_usize
                .checked_mul(4)
                .ok_or_else(|| anyhow!("frame stride overflow"))?;
            let stride = NonZeroUsize::new(stride_usize).ok_or_else(|| anyhow!("frame stride is zero"))?;

            let slot_len = stride
                .get()
                .checked_mul(height_usize)
                .ok_or_else(|| anyhow!("frame buffer length overflow"))?;

            let view_len = SHM_FB_HEADER_LEN
                .checked_add(
                    slot_len
                        .checked_mul(SHM_FB_SLOTS)
                        .ok_or_else(|| anyhow!("frame buffer length overflow"))?,
                )
                .ok_or_else(|| anyhow!("frame buffer length overflow"))?;

            let view_len_u32 = u32::try_from(view_len).map_err(|_| anyhow!("frame buffer too large"))?;

            let pid = std::process::id();
            let map_name = format!("Global\\IronRdpTermSrvFb_{pid}_{connection_id}");
            let event_name = format!("Global\\IronRdpTermSrvFb_{pid}_{connection_id}_Ready");

            let map_name_w: Vec<u16> = map_name.encode_utf16().chain(Some(0)).collect();
            let event_name_w: Vec<u16> = event_name.encode_utf16().chain(Some(0)).collect();

            let input_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .context("failed to bind local capture helper input listener")?;

            let input_addr = input_listener
                .local_addr()
                .context("failed to query local helper input listener address")?;

            info!(connection_id, input_addr = %input_addr, "Capture helper input listener bound");

            // SAFETY: CreateFileMappingW creates (or opens) a named mapping object.
            let mapping = unsafe {
                CreateFileMappingW(
                    windows::Win32::Foundation::INVALID_HANDLE_VALUE,
                    None,
                    PAGE_READWRITE,
                    0,
                    view_len_u32,
                    PCWSTR(map_name_w.as_ptr()),
                )
            }
            .map_err(|error| anyhow!("CreateFileMappingW failed: {error}"))
            .context("CreateFileMappingW failed")?;

            let mapping = SendHandle(mapping);

            // SAFETY: MapViewOfFile maps the mapping into this process address space.
            let view = unsafe { MapViewOfFile(mapping.0, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, view_len) };
            let view = SendMappedView(view);
            if view.0.Value.is_null() {
                // SAFETY: mapping handle is owned by us.
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(mapping.0);
                }
                return Err(anyhow!("MapViewOfFile returned null"));
            }

            // SAFETY: we mapped at least SHM_FB_HEADER_LEN bytes.
            unsafe { shm_init_header(view.0.Value.cast::<u8>(), width, height, stride, slot_len)? };

            // SAFETY: CreateEventW creates (or opens) a named auto-reset event.
            let frame_ready_event = unsafe { CreateEventW(None, false, false, PCWSTR(event_name_w.as_ptr())) }
                .map_err(|error| anyhow!("CreateEventW failed: {error}"))
                .context("CreateEventW failed")?;

            let frame_ready_event = SendHandle(frame_ready_event);

            let helper = spawn_capture_helper_process_shared_mem(
                &map_name,
                &event_name,
                input_addr,
                session_id_override,
                credentials,
                allow_prelogon_fallback,
            )
            .with_context(|| format!("failed to spawn shared-memory capture helper for connection {connection_id}"))?;

            let (input_stream, _peer) = timeout(CAPTURE_HELPER_CONNECT_TIMEOUT, input_listener.accept())
                .await
                .map_err(|_| anyhow!("capture helper input did not connect within timeout"))?
                .context("failed to accept capture helper input connection")?;

            info!(connection_id, "Capture helper input channel connected");

            {
                let mut guard = input_stream_slot.lock().await;
                *guard = Some(input_stream);
            }

            Ok(Self {
                helper_pid: helper.pid,
                helper_process: helper.process,
                input_stream_slot,
                mapping,
                frame_ready_event,
                view,
                view_len,
                width,
                height,
                stride,
                slot_len,
                last_seq: 0,
            })
        }

        fn pid(&self) -> u32 {
            self.helper_pid
        }

        fn terminate(self) {
            if let Ok(mut guard) = self.input_stream_slot.try_lock() {
                *guard = None;
            }

            // SAFETY: handle was returned by CreateProcessAsUserW.
            unsafe {
                let _ = TerminateProcess(self.helper_process.0, 1);
            }

            // SAFETY: handle was returned by CreateProcessAsUserW.
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.helper_process.0);
            }

            // SAFETY: view was mapped by MapViewOfFile.
            unsafe {
                let _ = UnmapViewOfFile(self.view.0);
            }

            // SAFETY: mapping/event handles are owned by us.
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.mapping.0);
            }

            // SAFETY: mapping/event handles are owned by us.
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.frame_ready_event.0);
            }
        }

        async fn read_frame(&mut self) -> anyhow::Result<BitmapUpdate> {
            loop {
                // SAFETY: waiting on a valid event handle with zero timeout is safe.
                let wait = unsafe { WaitForSingleObject(self.frame_ready_event.0, 0) };
                if wait == WAIT_TIMEOUT {
                    sleep(Duration::from_millis(1)).await;
                    continue;
                }

                if wait != WAIT_OBJECT_0 {
                    return Err(anyhow!("WaitForSingleObject failed: {wait:?}"));
                }

                let view_ptr = self.view.0.Value.cast::<u8>();

                // SAFETY: `view_ptr` points to a memory-mapped view that is valid for `self.view_len` bytes.
                let (seq, published_slot, payload_len) = unsafe { shm_read_published_meta(view_ptr, self.view_len)? };
                if seq == 0 || seq == self.last_seq {
                    continue;
                }

                let slot = usize::try_from(published_slot).map_err(|_| anyhow!("published slot out of range"))?;
                if slot >= SHM_FB_SLOTS {
                    return Err(anyhow!("published slot out of range: {slot}"));
                }

                let payload_len = usize::try_from(payload_len).map_err(|_| anyhow!("payload length out of range"))?;
                if payload_len != self.slot_len {
                    return Err(anyhow!(
                        "unexpected payload length from shared memory: got {payload_len}, expected {}",
                        self.slot_len
                    ));
                }

                let mut data = Vec::new();
                if let Err(error) = data.try_reserve(payload_len) {
                    return Err(anyhow!(
                        "failed to allocate shared memory payload buffer ({payload_len} bytes): {error}"
                    ));
                }
                data.resize(payload_len, 0);

                let slot_offset = SHM_FB_HEADER_LEN + slot * self.slot_len;
                let end = slot_offset + payload_len;
                if end > self.view_len {
                    return Err(anyhow!("shared memory slot out of bounds"));
                }

                // SAFETY: slot bounds are checked above and `view_ptr` is valid for `self.view_len` bytes.
                let slot_ptr = unsafe { view_ptr.add(slot_offset) };
                // SAFETY: slot_ptr points within the mapped view and is valid for payload_len bytes.
                unsafe { core::ptr::copy_nonoverlapping(slot_ptr, data.as_mut_ptr(), payload_len) };

                fence(Ordering::SeqCst);

                // SAFETY: `view_ptr` points to a memory-mapped view that is valid for `self.view_len` bytes.
                let (seq_after, _, _) = unsafe { shm_read_published_meta(view_ptr, self.view_len)? };
                if seq_after != seq {
                    continue;
                }

                self.last_seq = seq;

                return Ok(BitmapUpdate {
                    x: 0,
                    y: 0,
                    width: self.width,
                    height: self.height,
                    format: PixelFormat::BgrA32,
                    data: data.into(),
                    stride: self.stride,
                });
            }
        }
    }

    struct SpawnedProcess {
        pid: u32,
        process: SendHandle,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum HelperDesktop {
        Default,
        Winlogon,
    }

    impl HelperDesktop {
        fn as_lpdesktop(self) -> &'static str {
            match self {
                Self::Default => "winsta0\\default",
                Self::Winlogon => "winsta0\\winlogon",
            }
        }
    }

    struct AcquiredSessionToken {
        token: HANDLE,
        desktop: HelperDesktop,
    }

    static SESSION_KEEPALIVE_STARTED: OnceLock<StdMutex<HashSet<u32>>> = OnceLock::new();

    fn keepalive_started_set() -> &'static StdMutex<HashSet<u32>> {
        SESSION_KEEPALIVE_STARTED.get_or_init(|| StdMutex::new(HashSet::new()))
    }

    fn close_handle_best_effort(handle: HANDLE) {
        if handle.is_invalid() {
            return;
        }

        // SAFETY: handle is either valid or invalid; CloseHandle is safe to call.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(handle);
        }
    }

    fn spawn_session_keepalive_process(session_id: u32, user_token: HANDLE) -> anyhow::Result<u32> {
        // Keep the session alive by running a long-lived, no-window process in the session.
        // If TermService tears down sessions with no processes after disconnect, this helps
        // make `irdp-tcp#N` behave more like a normal RDP user session.
        let exe_path = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
        let args = format!(
            "\"{exe_path}\" -NoProfile -NonInteractive -WindowStyle Hidden -Command \"Start-Sleep -Seconds 86400\""
        );

        let app_name: Vec<u16> = exe_path.encode_utf16().chain(Some(0)).collect();
        let mut cmd_line: Vec<u16> = args.encode_utf16().chain(Some(0)).collect();

        // Avoid specifying lpDesktop here; the process is only a keepalive and should not depend
        // on WinSta0/desktop ACLs (pre-logon sessions often only expose winlogon).
        let startup_info = STARTUPINFOW {
            cb: u32::try_from(size_of::<STARTUPINFOW>()).map_err(|_| anyhow!("STARTUPINFOW size overflow"))?,
            ..Default::default()
        };

        let mut process_info = PROCESS_INFORMATION::default();

        // SAFETY:
        // - token handle is valid primary token
        // - app/cmd buffers are NUL-terminated and live for the call
        // - process_info/startup_info are valid out-pointers
        let create_ok = unsafe {
            CreateProcessAsUserW(
                Some(user_token),
                PCWSTR(app_name.as_ptr()),
                Some(PWSTR(cmd_line.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_NO_WINDOW,
                None,
                None,
                &startup_info,
                &mut process_info,
            )
        };

        close_handle_best_effort(user_token);

        create_ok
            .ok()
            .with_context(|| format!("CreateProcessAsUserW keepalive failed (session_id={session_id})"))?;

        close_handle_best_effort(process_info.hThread);
        close_handle_best_effort(process_info.hProcess);

        Ok(process_info.dwProcessId)
    }

    fn ensure_session_keepalive_started(session_id: u32, user_token: HANDLE) -> anyhow::Result<()> {
        let should_start = {
            let mut guard = keepalive_started_set()
                .lock()
                .map_err(|_| anyhow!("keepalive session set lock poisoned"))?;
            if guard.contains(&session_id) {
                false
            } else {
                guard.insert(session_id);
                true
            }
        };

        if !should_start {
            close_handle_best_effort(user_token);
            return Ok(());
        }

        match spawn_session_keepalive_process(session_id, user_token) {
            Ok(pid) => {
                info!(session_id, keepalive_pid = pid, "Started session keepalive process");
                Ok(())
            }
            Err(error) => {
                // Allow retries on a subsequent connection attempt.
                if let Ok(mut guard) = keepalive_started_set().lock() {
                    guard.remove(&session_id);
                }
                Err(error)
            }
        }
    }

    fn spawn_capture_helper_process_tcp(
        connect_addr: SocketAddr,
        input_connect_addr: SocketAddr,
        rfx_encode: bool,
        session_id_override: Option<u32>,
        credentials: Option<StoredCredentials>,
        allow_prelogon_fallback: bool,
    ) -> anyhow::Result<SpawnedProcess> {
        let rfx_flag = if rfx_encode { " --rfx-encode" } else { "" };
        spawn_capture_helper_process_with_args(
            &format!("--connect {connect_addr} --input-connect {input_connect_addr}{rfx_flag}"),
            session_id_override,
            credentials,
            allow_prelogon_fallback,
        )
    }

    fn spawn_capture_helper_process_shared_mem(
        map_name: &str,
        event_name: &str,
        input_connect_addr: SocketAddr,
        session_id_override: Option<u32>,
        credentials: Option<StoredCredentials>,
        allow_prelogon_fallback: bool,
    ) -> anyhow::Result<SpawnedProcess> {
        spawn_capture_helper_process_with_args(
            &format!("--shm-map \"{map_name}\" --shm-event \"{event_name}\" --input-connect {input_connect_addr}"),
            session_id_override,
            credentials,
            allow_prelogon_fallback,
        )
    }

    fn spawn_capture_helper_process_with_args(
        extra_args: &str,
        session_id_override: Option<u32>,
        credentials: Option<StoredCredentials>,
        allow_prelogon_fallback: bool,
    ) -> anyhow::Result<SpawnedProcess> {
        let session_id = match session_id_override {
            Some(id) => {
                info!(session_id = id, "Using WTS-notified capture session");
                id
            }
            None => resolve_capture_session_id().context("failed to resolve capture session id")?,
        };
        info!(session_id, "Selected capture session");

        let acquired = acquire_session_token(session_id, credentials.as_ref(), allow_prelogon_fallback, true)
            .context("failed to acquire a token for the capture session")?;
        let user_token = acquired.token;

        let exe_path = std::env::current_exe().context("failed to resolve current executable path")?;
        let exe_path_str = exe_path
            .to_str()
            .ok_or_else(|| anyhow!("current executable path is not valid unicode"))?;

        let args = format!("\"{exe_path_str}\" --capture-helper {extra_args}");

        let app_name: Vec<u16> = exe_path_str.encode_utf16().chain(Some(0)).collect();
        let desktop = acquired.desktop.as_lpdesktop();
        let mut cmd_line: Vec<u16> = args.encode_utf16().chain(Some(0)).collect();
        let mut desktop_w: Vec<u16> = desktop.encode_utf16().chain(Some(0)).collect();

        let startup_info = STARTUPINFOW {
            cb: u32::try_from(size_of::<STARTUPINFOW>()).map_err(|_| anyhow!("STARTUPINFOW size overflow"))?,
            lpDesktop: PWSTR(desktop_w.as_mut_ptr()),
            ..Default::default()
        };

        let mut process_info = PROCESS_INFORMATION::default();

        // SAFETY:
        // - token handle is valid on success from WTSQueryUserToken
        // - app/cmd/desktop buffers are nul-terminated and live for the call
        // - process_info/startup_info are valid out-pointers
        //
        // NOTE: CREATE_NO_WINDOW is safe here because the binary uses the `windows`
        // PE subsystem (#![windows_subsystem = "windows"]), so the DLL loader does
        // not attempt to create a console via CSRSS during process initialization.
        let create_ok = unsafe {
            CreateProcessAsUserW(
                Some(user_token),
                PCWSTR(app_name.as_ptr()),
                Some(PWSTR(cmd_line.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_NO_WINDOW,
                None,
                None,
                &startup_info,
                &mut process_info,
            )
        };

        // SAFETY: close token handle from WTSQueryUserToken.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(user_token);
        }

        create_ok.ok().context("CreateProcessAsUserW failed")?;

        let helper_session_id = process_id_to_session_id(process_info.dwProcessId)
            .ok_or_else(|| anyhow!("failed to resolve capture helper session id for pid {}", process_info.dwProcessId))?;

        info!(
            helper_pid = process_info.dwProcessId,
            requested_session_id = session_id,
            helper_session_id,
            desktop,
            "Capture helper process created"
        );

        if helper_session_id != session_id {
            // SAFETY: handles were returned by CreateProcessAsUserW and are still owned here.
            unsafe {
                let _ = TerminateProcess(process_info.hProcess, 1);
                let _ = windows::Win32::Foundation::CloseHandle(process_info.hThread);
                let _ = windows::Win32::Foundation::CloseHandle(process_info.hProcess);
            }

            return Err(anyhow!(
                "capture helper launched in unexpected session {helper_session_id} (expected {session_id})"
            ));
        }

        // SAFETY: close thread handle we don't need.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(process_info.hThread);
        }

        Ok(SpawnedProcess {
            pid: process_info.dwProcessId,
            process: SendHandle(process_info.hProcess),
        })
    }

    fn try_start_explorer_process(session_id: u32, allow_prelogon_fallback: bool) -> anyhow::Result<u32> {
        let acquired = acquire_session_token(session_id, None, allow_prelogon_fallback, false)
            .context("failed to acquire a user token for explorer bootstrap")?;
        let user_token = acquired.token;

        let exe_path = r"C:\Windows\explorer.exe";
        let args = format!("\"{exe_path}\"");

        let app_name: Vec<u16> = exe_path.encode_utf16().chain(Some(0)).collect();
        let mut cmd_line: Vec<u16> = args.encode_utf16().chain(Some(0)).collect();
        let mut desktop_w: Vec<u16> = acquired.desktop.as_lpdesktop().encode_utf16().chain(Some(0)).collect();

        let startup_info = STARTUPINFOW {
            cb: u32::try_from(size_of::<STARTUPINFOW>()).map_err(|_| anyhow!("STARTUPINFOW size overflow"))?,
            lpDesktop: PWSTR(desktop_w.as_mut_ptr()),
            ..Default::default()
        };

        let mut process_info = PROCESS_INFORMATION::default();

        // SAFETY:
        // - user token handle is valid on successful acquisition
        // - app/cmd/desktop buffers are NUL-terminated and live for the call
        // - startup/process structures are valid out-parameters
        let create_ok = unsafe {
            CreateProcessAsUserW(
                Some(user_token),
                PCWSTR(app_name.as_ptr()),
                Some(PWSTR(cmd_line.as_mut_ptr())),
                None,
                None,
                false,
                windows::Win32::System::Threading::PROCESS_CREATION_FLAGS(0),
                None,
                None,
                &startup_info,
                &mut process_info,
            )
        };

        // SAFETY: close token handle acquired for this launch attempt.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(user_token);
        }

        create_ok.ok().context("CreateProcessAsUserW(explorer) failed")?;

        // SAFETY: close thread/process handles from successful process creation.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(process_info.hThread);
            let _ = windows::Win32::Foundation::CloseHandle(process_info.hProcess);
        }

        Ok(process_info.dwProcessId)
    }

    fn session_has_user_token(session_id: u32) -> bool {
        if let Err(error) = enable_privilege(w!("SeTcbPrivilege")) {
            debug!(
                session_id,
                error = %format!("{error:#}"),
                "Failed to enable SeTcbPrivilege before WTSQueryUserToken"
            );
        }

        let mut token = HANDLE::default();
        // SAFETY: `WTSQueryUserToken` writes a token handle into `token` on success.
        let res = unsafe { WTSQueryUserToken(session_id, &mut token) };
        if res.is_ok() {
            // SAFETY: close token handle from WTSQueryUserToken.
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(token);
            }
            true
        } else {
            if let Err(error) = &res {
                info!(
                    session_id,
                    error = %error,
                    error_code = %error.code(),
                    "WTSQueryUserToken indicates no interactive user token for session"
                );
            }
            false
        }
    }

    fn session_has_process(session_id: u32, process_name: &str) -> bool {
        let mut process_info_ptr: *mut WTS_PROCESS_INFOW = null_mut();
        let mut process_count = 0u32;

        // SAFETY: WTSEnumerateProcessesW writes a buffer pointer/count pair on success.
        let enumerate_result = unsafe { WTSEnumerateProcessesW(None, 0, 1, &mut process_info_ptr, &mut process_count) };
        if enumerate_result.is_err() || process_info_ptr.is_null() || process_count == 0 {
            return false;
        }

        let mut found = false;

        if let Ok(process_count_usize) = usize::try_from(process_count) {
            // SAFETY: `process_info_ptr` references `process_count_usize` entries on success.
            let processes = unsafe { core::slice::from_raw_parts(process_info_ptr, process_count_usize) };

            for entry in processes {
                if entry.SessionId != session_id {
                    continue;
                }

                // SAFETY: pProcessName is a NUL-terminated string owned by the WTS buffer.
                let name = unsafe { PCWSTR(entry.pProcessName.0).to_string() }.unwrap_or_default();
                if name.eq_ignore_ascii_case(process_name) {
                    found = true;
                    break;
                }
            }
        }

        // SAFETY: free memory allocated by WTSEnumerateProcessesW.
        unsafe {
            WTSFreeMemory(process_info_ptr.cast());
        }

        found
    }

    fn canonicalize_enumerated_session_id(session_id: u32) -> u32 {
        if session_id > u32::from(u16::MAX) {
            let low_word = session_id & u32::from(u16::MAX);
            if low_word != 0 && low_word != u32::from(u16::MAX) {
                return low_word;
            }

            return u32::MAX;
        }

        session_id
    }

    fn session_selection_snapshot() -> String {
        // SAFETY: safe to call and returns a process-global session id value.
        let console_session = unsafe { WTSGetActiveConsoleSessionId() };
        let console = if console_session == u32::MAX {
            "none".to_owned()
        } else {
            console_session.to_string()
        };

        let mut sessions_ptr: *mut WTS_SESSION_INFOW = null_mut();
        let mut session_count = 0u32;

        // SAFETY: WTSEnumerateSessionsW writes a pointer/count pair on success.
        let enumerate_result = unsafe { WTSEnumerateSessionsW(None, 0, 1, &mut sessions_ptr, &mut session_count) };
        if enumerate_result.is_err() || sessions_ptr.is_null() || session_count == 0 {
            return format!("console={console} sessions=none");
        }

        let mut rows = Vec::new();

        if let Ok(session_count_usize) = usize::try_from(session_count) {
            // SAFETY: `sessions_ptr` references `session_count_usize` entries on success.
            let sessions = unsafe { core::slice::from_raw_parts(sessions_ptr, session_count_usize) };
            let mut seen = HashSet::new();

            for entry in sessions {
                let raw = entry.SessionId;
                if raw == u32::MAX {
                    continue;
                }

                let canonical = canonicalize_enumerated_session_id(raw);
                if !seen.insert((raw, canonical)) {
                    continue;
                }

                let has_token = canonical != u32::MAX && session_has_user_token(canonical);
                let has_explorer = canonical != u32::MAX && session_has_process(canonical, "explorer.exe");
                let has_winlogon = canonical != u32::MAX && session_has_process(canonical, "winlogon.exe");
                let has_logonui = canonical != u32::MAX && session_has_process(canonical, "LogonUI.exe");

                rows.push(format!(
                    "{raw}->{canonical}:token={has_token},explorer={has_explorer},winlogon={has_winlogon},logonui={has_logonui}",
                ));
            }
        }

        // SAFETY: free memory allocated by WTSEnumerateSessionsW.
        unsafe {
            WTSFreeMemory(sessions_ptr.cast());
        }

        if rows.is_empty() {
            format!("console={console} sessions=empty")
        } else {
            format!("console={console} sessions=[{}]", rows.join("; "))
        }
    }

    fn session_has_explorer_token(session_id: u32) -> bool {
        match token_from_session_process_with_retries(session_id, "explorer.exe", 1) {
            Ok(token) => {
                close_handle_best_effort(token);
                true
            }
            Err(_) => false,
        }
    }

    fn resolve_capture_session_id() -> anyhow::Result<u32> {
        if let Ok(configured) = std::env::var(CAPTURE_SESSION_ID_ENV) {
            let configured = configured.trim();
            if !configured.is_empty() {
                let session_id: u32 = configured
                    .parse()
                    .with_context(|| format!("failed to parse {CAPTURE_SESSION_ID_ENV} as u32: {configured}"))?;
                return Ok(session_id);
            }
        }

        // SAFETY: safe to call and returns a process-global session id value.
        let console_session = unsafe { WTSGetActiveConsoleSessionId() };
        if console_session != u32::MAX && session_has_user_token(console_session) {
            return Ok(console_session);
        }

        let mut sessions_ptr: *mut WTS_SESSION_INFOW = null_mut();
        let mut session_count = 0u32;

        // SAFETY: WTSEnumerateSessionsW writes a buffer pointer into `sessions_ptr` on success.
        let res = unsafe { WTSEnumerateSessionsW(None, 0, 1, &mut sessions_ptr, &mut session_count) };
        if res.is_err() || sessions_ptr.is_null() || session_count == 0 {
            if console_session == u32::MAX {
                return Err(anyhow!("no active console session"));
            }
            return Ok(console_session);
        }

        let session_count_usize = usize::try_from(session_count).map_err(|_| anyhow!("session count overflow"))?;

        // SAFETY: WTSEnumerateSessionsW returned a valid buffer for `session_count_usize` entries.
        let sessions = unsafe { core::slice::from_raw_parts(sessions_ptr, session_count_usize) };

        let mut candidates: Vec<u32> = sessions.iter().map(|s| s.SessionId).collect();

        // SAFETY: free buffer allocated by WTSEnumerateSessionsW.
        unsafe {
            WTSFreeMemory(sessions_ptr.cast());
        }

        candidates.sort_unstable();

        for session_id in candidates {
            if session_id == u32::MAX {
                continue;
            }
            if session_has_user_token(session_id) {
                return Ok(session_id);
            }
        }

        if console_session == u32::MAX {
            Err(anyhow!("no suitable capture session found"))
        } else {
            Ok(console_session)
        }
    }

    fn acquire_session_token(
        session_id: u32,
        credentials: Option<&StoredCredentials>,
        allow_prelogon_fallback: bool,
        allow_service_token_fallback: bool,
    ) -> anyhow::Result<AcquiredSessionToken> {
        let process_lookup_retries = if allow_service_token_fallback && allow_prelogon_fallback {
            8
        } else {
            1
        };

        if allow_service_token_fallback {
            // Prefer a token from explorer first when present (interactive desktop), even when
            // prelogon fallback is disabled. This is the strongest signal that the remote user
            // shell exists, and it remains usable on hosts where WTSQueryUserToken keeps failing.
            match token_from_session_process_with_retries(session_id, "explorer.exe", process_lookup_retries) {
                Ok(token) => {
                    debug!(session_id, "Using explorer.exe token (default desktop)");
                    return finalize_acquired_session_token(
                        session_id,
                        token,
                        HelperDesktop::Default,
                        "explorer.exe",
                    );
                }
                Err(error) => {
                    info!(
                        session_id,
                        error = %format!("{error:#}"),
                        "explorer.exe token unavailable for capture session"
                    );
                }
            }
        }

        let mut token = HANDLE::default();
        let mut wts_result = Err(windows::core::Error::empty());

        if allow_service_token_fallback {
            if let Err(error) = enable_privilege(w!("SeTcbPrivilege")) {
                info!(
                    session_id,
                    error = %format!("{error:#}"),
                    "Failed to enable SeTcbPrivilege before WTSQueryUserToken"
                );
            }

            // SAFETY: `WTSQueryUserToken` writes a token handle into `token` on success.
            wts_result = unsafe { WTSQueryUserToken(session_id, &mut token) };

            if wts_result.is_ok() {
                // If the user is logged in, we can start the keepalive using the real session token
                // instead of manufacturing an interactive logon via LogonUserW.
                if let Ok(dup_for_keepalive) = duplicate_primary_token(token) {
                    if let Err(error) = ensure_session_keepalive_started(session_id, dup_for_keepalive) {
                        warn!(
                            session_id,
                            error = %format!("{error:#}"),
                            "Failed to start session keepalive process"
                        );
                    }
                }

                return finalize_acquired_session_token(
                    session_id,
                    token,
                    HelperDesktop::Default,
                    "wts_query_user_token",
                );
            }
        }

        if !allow_prelogon_fallback {
            let wts_error = wts_result.err().unwrap_or_else(windows::core::Error::empty);
            info!(
                session_id,
                error = %wts_error,
                error_code = %wts_error.code(),
                "WTSQueryUserToken unavailable and prelogon token fallback is disabled after initial helper start"
            );
            return Err(anyhow!("prelogon token fallback disabled after initial helper start"));
        }

        match token_from_session_process_with_retries(session_id, "winlogon.exe", process_lookup_retries) {
            Ok(token) => {
                info!(
                    session_id,
                    "Using winlogon.exe token (winlogon desktop \u{2014} pre-login)"
                );
                return finalize_acquired_session_token(
                    session_id,
                    token,
                    HelperDesktop::Winlogon,
                    "winlogon.exe",
                );
            }
            Err(error) => {
                info!(
                    session_id,
                    error = %format!("{error:#}"),
                    "winlogon.exe token unavailable for capture session"
                );
            }
        }

        if allow_service_token_fallback {
            match token_from_session_process_with_retries(session_id, "csrss.exe", process_lookup_retries) {
                Ok(token) => {
                    info!(
                        session_id,
                        "Using csrss.exe token (fallback for session-bound helper startup)"
                    );
                    return finalize_acquired_session_token(
                        session_id,
                        token,
                        HelperDesktop::Default,
                        "csrss.exe",
                    );
                }
                Err(error) => {
                    info!(
                        session_id,
                        error = %format!("{error:#}"),
                        "csrss.exe token unavailable for capture session"
                    );
                }
            }
        }

        let wts_error = wts_result.err().unwrap_or_else(windows::core::Error::empty);

        if !allow_service_token_fallback {
            info!(
                session_id,
                error = %wts_error,
                error_code = %wts_error.code(),
                "interactive session token unavailable and service-token fallback is disabled"
            );
            return Err(anyhow!(
                "interactive session token unavailable and service-token fallback is disabled"
            ));
        }

        warn!(
            session_id,
            error = %wts_error,
            error_code = %wts_error.code(),
            "WTSQueryUserToken failed; attempting to spawn helper with a session-adjusted service token"
        );

        // NOTE: We intentionally do not fall back to LogonUserW here.
        // This avoids generating misleading Security 4624 LogonType=2 events from ironrdp-termsrv.
        let _ = credentials;

        debug!(session_id, "Using duplicated service token for capture");
        let token = match duplicate_self_token_for_session(session_id) {
            Ok(token) => token,
            Err(error) => {
                warn!(
                    session_id,
                    error = %format!("{error:#}"),
                    "Session-adjusted service token acquisition failed"
                );
                return Err(error);
            }
        };

        finalize_acquired_session_token(
            session_id,
            token,
            HelperDesktop::Default,
            "service_token_fallback",
        )
    }

    fn duplicate_primary_token(token: HANDLE) -> anyhow::Result<HANDLE> {
        let mut primary_token = HANDLE::default();

        // SAFETY: DuplicateTokenEx writes a new token handle into `primary_token` on success.
        unsafe {
            DuplicateTokenEx(
                token,
                TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_QUERY,
                None,
                SecurityImpersonation,
                TokenPrimary,
                &mut primary_token,
            )
        }
        .map_err(|error| anyhow!("DuplicateTokenEx(session token) failed: {error}"))
        .context("DuplicateTokenEx(session token) failed")?;

        Ok(primary_token)
    }

    fn query_token_session_id(token: HANDLE) -> anyhow::Result<u32> {
        let mut session_id = 0u32;
        let mut return_length = 0u32;

        // SAFETY: `token` is expected to be a valid token handle and `session_id` is a writable output buffer.
        unsafe {
            GetTokenInformation(
                token,
                TokenSessionId,
                Some(core::ptr::addr_of_mut!(session_id).cast()),
                u32::try_from(size_of::<u32>()).map_err(|_| anyhow!("TokenSessionId size overflow"))?,
                &mut return_length,
            )
        }
        .map_err(|error| anyhow!("GetTokenInformation(TokenSessionId) failed: {error}"))
        .context("GetTokenInformation(TokenSessionId) failed")?;

        Ok(session_id)
    }

    fn ensure_token_session_id(token: HANDLE, session_id: u32, source: &str) -> anyhow::Result<u32> {
        let actual_session_id = query_token_session_id(token)?;
        if actual_session_id == session_id {
            debug!(
                requested_session_id = session_id,
                token_session_id = actual_session_id,
                source,
                "Token already targets requested session"
            );
            return Ok(actual_session_id);
        }

        warn!(
            requested_session_id = session_id,
            token_session_id = actual_session_id,
            source,
            "Retargeting token to requested session"
        );

        enable_privilege(w!("SeTcbPrivilege"))
            .context("failed to enable SeTcbPrivilege before token session retarget")?;

        let session_id_ptr = core::ptr::addr_of!(session_id).cast::<c_void>();

        // SAFETY: `token` is a valid primary token handle with TOKEN_ADJUST_SESSIONID and `session_id_ptr` is valid.
        unsafe {
            SetTokenInformation(
                token,
                TokenSessionId,
                session_id_ptr,
                u32::try_from(size_of::<u32>()).map_err(|_| anyhow!("TokenSessionId size overflow"))?,
            )
        }
        .map_err(|error| anyhow!("SetTokenInformation(TokenSessionId) failed: {error}"))
        .context("SetTokenInformation(TokenSessionId) failed")?;

        let updated_session_id = query_token_session_id(token)?;
        if updated_session_id != session_id {
            return Err(anyhow!(
                "token session id mismatch after retarget (expected {session_id}, got {updated_session_id})"
            ));
        }

        Ok(updated_session_id)
    }

    fn finalize_acquired_session_token(
        session_id: u32,
        token: HANDLE,
        desktop: HelperDesktop,
        source: &'static str,
    ) -> anyhow::Result<AcquiredSessionToken> {
        let token_session_id = ensure_token_session_id(token, session_id, source)?;
        info!(
            requested_session_id = session_id,
            token_session_id,
            source,
            desktop = desktop.as_lpdesktop(),
            "Acquired capture-session token"
        );

        Ok(AcquiredSessionToken { token, desktop })
    }

    fn token_from_session_process_with_retries(
        session_id: u32,
        process_name: &str,
        enumerate_process_retries: u32,
    ) -> anyhow::Result<HANDLE> {
        let mut process_info_ptr: *mut WTS_PROCESS_INFOW = null_mut();
        let mut process_count = 0u32;

        let mut enumerate_success = false;
        let mut enumerate_error: Option<anyhow::Error> = None;

        let enumerate_process_retries = enumerate_process_retries.max(1);

        if enumerate_process_retries == 1 {
            let pid = find_session_process_pid_toolhelp(session_id, process_name)?;
            return duplicate_primary_token_from_process(pid, process_name);
        }

        for attempt in 0..enumerate_process_retries {
            process_info_ptr = null_mut();
            process_count = 0;

            // SAFETY: WTSEnumerateProcessesW writes a buffer pointer into `process_info_ptr` on success.
            let enumerate_result = unsafe { WTSEnumerateProcessesW(None, 0, 1, &mut process_info_ptr, &mut process_count) };

            match enumerate_result {
                Ok(()) => {
                    enumerate_success = true;
                    break;
                }
                Err(error) => {
                    if error.code() == windows::core::HRESULT::from_win32(ERROR_BAD_LENGTH.0)
                        && attempt + 1 < enumerate_process_retries
                    {
                        warn!(
                            session_id,
                            process_name,
                            attempt = attempt + 1,
                            "WTSEnumerateProcessesW returned ERROR_BAD_LENGTH; retrying"
                        );
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    }

                    enumerate_error = Some(anyhow!("WTSEnumerateProcessesW failed: {error}"));
                    break;
                }
            }
        }

        let mut found_pid: Option<u32> = None;

        if enumerate_success {
            struct ProcessListGuard(*mut WTS_PROCESS_INFOW);
            impl Drop for ProcessListGuard {
                fn drop(&mut self) {
                    if !self.0.is_null() {
                        // SAFETY: pointer was allocated by WTSEnumerateProcessesW and must be freed with WTSFreeMemory.
                        unsafe { WTSFreeMemory(self.0.cast()) };
                    }
                }
            }

            let _guard = ProcessListGuard(process_info_ptr);

            if process_info_ptr.is_null() {
                return Err(anyhow!("WTSEnumerateProcessesW returned a null process list pointer"));
            }

            let process_count_usize = usize::try_from(process_count)
                .map_err(|_| anyhow!("WTSEnumerateProcessesW returned too many process entries: {process_count}"))?;

            // SAFETY: WTSEnumerateProcessesW returned `process_count` entries at `process_info_ptr`.
            let processes = unsafe { core::slice::from_raw_parts(process_info_ptr, process_count_usize) };

            for entry in processes {
                if entry.SessionId != session_id {
                    continue;
                }

                // SAFETY: pProcessName is a nul-terminated wide string pointer returned by WTSEnumerateProcessesW.
                let name = unsafe { PCWSTR(entry.pProcessName.0).to_string() }.unwrap_or_default();
                if name.eq_ignore_ascii_case(process_name) {
                    found_pid = Some(entry.ProcessId);
                    break;
                }
            }
        }

        let pid = if let Some(pid) = found_pid {
            pid
        } else {
            find_session_process_pid_toolhelp(session_id, process_name).map_err(|toolhelp_error| {
                if let Some(enumerate_error) = &enumerate_error {
                    anyhow!(
                        "{process_name} lookup failed via WTSEnumerateProcessesW ({enumerate_error:#}) and Toolhelp fallback ({toolhelp_error:#})"
                    )
                } else {
                    toolhelp_error
                }
            })?
        };

        duplicate_primary_token_from_process(pid, process_name)
    }

    fn duplicate_primary_token_from_process(process_id: u32, process_name: &str) -> anyhow::Result<HANDLE> {
        // SAFETY: OpenProcess returns a handle for the specified PID when permitted.
        let process_handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }
            .map_err(|error| anyhow!("OpenProcess failed: {error}"))
            .context("OpenProcess failed")?;

        struct HandleGuard(HANDLE);
        impl Drop for HandleGuard {
            fn drop(&mut self) {
                // SAFETY: handle is either valid or null; CloseHandle is safe to call.
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(self.0);
                }
            }
        }

        let _process_guard = HandleGuard(process_handle);

        let mut process_token = HANDLE::default();

        // SAFETY: OpenProcessToken writes a token handle into `process_token` on success.
        unsafe {
            OpenProcessToken(
                process_handle,
                TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_QUERY,
                &mut process_token,
            )
        }
        .map_err(|error| anyhow!("OpenProcessToken({process_name}) failed: {error}"))
        .context("OpenProcessToken failed")?;

        let _token_guard = HandleGuard(process_token);

        let mut primary_token = HANDLE::default();

        // SAFETY: DuplicateTokenEx writes a new token handle into `primary_token` on success.
        unsafe {
            DuplicateTokenEx(
                process_token,
                TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_QUERY | TOKEN_ADJUST_SESSIONID,
                None,
                SecurityImpersonation,
                TokenPrimary,
                &mut primary_token,
            )
        }
        .map_err(|error| anyhow!("DuplicateTokenEx({process_name}) failed: {error}"))
        .context("DuplicateTokenEx failed")?;

        Ok(primary_token)
    }

    fn duplicate_self_token_for_session(session_id: u32) -> anyhow::Result<HANDLE> {
        // Avoid privilege checks unexpectedly being evaluated against an impersonation token.
        // SAFETY: RevertToSelf has no parameters and only affects the current thread token.
        unsafe {
            RevertToSelf()
                .map_err(|error| anyhow!("RevertToSelf failed: {error}"))
                .context("RevertToSelf failed")?;
        }

        let mut process_token = HANDLE::default();

        // SAFETY: `GetCurrentProcess` is safe to call.
        let current_process = unsafe { GetCurrentProcess() };

        // SAFETY: `OpenProcessToken` writes a token handle into `process_token` on success.
        let open_result = unsafe {
            OpenProcessToken(
                current_process,
                TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_QUERY | TOKEN_ADJUST_SESSIONID,
                &mut process_token,
            )
        };

        open_result
            .map_err(|error| anyhow!("OpenProcessToken failed: {error}"))
            .context("OpenProcessToken failed")?;

        let mut primary_token = HANDLE::default();

        // SAFETY: `DuplicateTokenEx` writes a new token handle into `primary_token` on success.
        let duplicate_result = unsafe {
            DuplicateTokenEx(
                process_token,
                TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_QUERY | TOKEN_ADJUST_SESSIONID,
                None,
                SecurityImpersonation,
                TokenPrimary,
                &mut primary_token,
            )
        };

        duplicate_result
            .map_err(|error| anyhow!("DuplicateTokenEx failed: {error}"))
            .context("DuplicateTokenEx failed")?;

        // SAFETY: close the original process token.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(process_token);
        }

        let session_id_ptr = core::ptr::addr_of!(session_id).cast::<c_void>();

        enable_privilege(w!("SeTcbPrivilege"))
            .context("failed to enable SeTcbPrivilege (hint: run ironrdp-termsrv as LocalSystem)")?;

        // SAFETY: SetTokenInformation expects a pointer to a u32 session id.
        let set_result = unsafe {
            SetTokenInformation(
                primary_token,
                TokenSessionId,
                session_id_ptr,
                u32::try_from(size_of::<u32>()).map_err(|_| anyhow!("TokenSessionId size overflow"))?,
            )
        };

        set_result
            .map_err(|error| anyhow!("SetTokenInformation(TokenSessionId) failed: {error}"))
            .context("SetTokenInformation(TokenSessionId) failed")?;

        Ok(primary_token)
    }

    fn enable_privilege(privilege_name: PCWSTR) -> anyhow::Result<()> {
        let mut process_token = HANDLE::default();

        // SAFETY: `GetCurrentProcess` is safe to call.
        let current_process = unsafe { GetCurrentProcess() };

        // SAFETY: OpenProcessToken writes a token handle into `process_token` on success.
        let open_result = unsafe {
            OpenProcessToken(
                current_process,
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut process_token,
            )
        };

        open_result
            .map_err(|error| anyhow!("OpenProcessToken failed: {error}"))
            .context("OpenProcessToken failed")?;

        let mut luid = LUID::default();

        // SAFETY: LookupPrivilegeValueW writes a LUID into `luid` on success.
        let lookup_result = unsafe { LookupPrivilegeValueW(None, privilege_name, &mut luid) };

        lookup_result
            .map_err(|error| anyhow!("LookupPrivilegeValueW failed: {error}"))
            .context("LookupPrivilegeValueW failed")?;

        let token_privileges = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };

        // SAFETY: reset last-error so we can detect ERROR_NOT_ALL_ASSIGNED.
        unsafe { SetLastError(WIN32_ERROR(0)) };

        // SAFETY: AdjustTokenPrivileges is passed a valid token and TOKEN_PRIVILEGES buffer.
        let adjust_result =
            unsafe { AdjustTokenPrivileges(process_token, false, Some(&token_privileges), 0, None, None) };

        // SAFETY: GetLastError is safe to call and returns the last error for the calling thread.
        let last_error = unsafe { GetLastError() };

        // SAFETY: close the process token.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(process_token);
        }

        adjust_result
            .map_err(|error| anyhow!("AdjustTokenPrivileges failed: {error}"))
            .context("AdjustTokenPrivileges failed")?;

        if last_error == ERROR_NOT_ALL_ASSIGNED {
            return Err(anyhow!("required privilege not held"));
        }

        Ok(())
    }

    const CAPTURE_FRAME_MAGIC: [u8; 4] = *b"IRDP";
    const CAPTURE_FRAME_HEADER_LEN: usize = 24;
    const CAPTURE_FRAME_RESYNC_LIMIT: usize = 1024 * 1024;

    async fn read_capture_frame(stream: &mut TcpStream) -> anyhow::Result<CapturedFrame> {
        let mut header = [0u8; CAPTURE_FRAME_HEADER_LEN];
        stream
            .read_exact(&mut header[0..4])
            .await
            .context("failed to read capture frame magic")?;

        let mut discarded = 0usize;
        while header[0..4] != CAPTURE_FRAME_MAGIC {
            if discarded >= CAPTURE_FRAME_RESYNC_LIMIT {
                return Err(anyhow!(
                    "capture stream desynchronized (discarded {discarded} bytes without finding magic); got {:02X?}",
                    &header[0..4]
                ));
            }

            let mut next = [0u8; 1];
            stream
                .read_exact(&mut next)
                .await
                .context("failed to read capture stream resync byte")?;

            header[0] = header[1];
            header[1] = header[2];
            header[2] = header[3];
            header[3] = next[0];
            discarded += 1;
        }

        stream
            .read_exact(&mut header[4..])
            .await
            .context("failed to read capture frame header")?;

        let version = u16::from_le_bytes([header[4], header[5]]);

        let width_u16 = u16::from_le_bytes([header[6], header[7]]);
        let height_u16 = u16::from_le_bytes([header[8], header[9]]);
        let payload_len = u32::from_le_bytes([header[20], header[21], header[22], header[23]]);
        let payload_len_usize =
            usize::try_from(payload_len).map_err(|_| anyhow!("capture payload length out of range"))?;

        match version {
            1 => {
                let stride_u32 = u32::from_le_bytes([header[10], header[11], header[12], header[13]]);
                let format = header[14];
                if format != 0 {
                    return Err(anyhow!("unsupported capture pixel format: {format}"));
                }

                let width = NonZeroU16::new(width_u16).ok_or_else(|| anyhow!("capture frame width is zero"))?;
                let height = NonZeroU16::new(height_u16).ok_or_else(|| anyhow!("capture frame height is zero"))?;
                let stride_usize =
                    usize::try_from(stride_u32).map_err(|_| anyhow!("capture frame stride out of range"))?;
                let stride = NonZeroUsize::new(stride_usize).ok_or_else(|| anyhow!("capture frame stride is zero"))?;

                let expected = stride
                    .get()
                    .checked_mul(NonZeroUsize::from(height).get())
                    .ok_or_else(|| anyhow!("capture payload length overflow"))?;

                if payload_len_usize != expected {
                    return Err(anyhow!(
                        "capture payload length mismatch (got {payload_len_usize}, expected {expected})"
                    ));
                }

                let mut payload = Vec::new();
                if let Err(error) = payload.try_reserve(payload_len_usize) {
                    return Err(anyhow!(
                        "failed to allocate capture frame payload buffer ({payload_len_usize} bytes): {error}"
                    ));
                }
                payload.resize(payload_len_usize, 0);
                stream
                    .read_exact(&mut payload)
                    .await
                    .context("failed to read capture frame payload")?;

                Ok(CapturedFrame::Raw(BitmapUpdate {
                    x: 0,
                    y: 0,
                    width,
                    height,
                    format: PixelFormat::BgrA32,
                    data: payload.into(),
                    stride,
                }))
            }
            2 => {
                let codec_id = header[14];

                let mut payload = Vec::new();
                if let Err(error) = payload.try_reserve(payload_len_usize) {
                    return Err(anyhow!(
                        "failed to allocate pre-encoded frame payload buffer ({payload_len_usize} bytes): {error}"
                    ));
                }
                payload.resize(payload_len_usize, 0);
                stream
                    .read_exact(&mut payload)
                    .await
                    .context("failed to read pre-encoded frame payload")?;

                Ok(CapturedFrame::PreEncoded(ironrdp_server::PreEncodedSurface {
                    codec_id,
                    width: width_u16,
                    height: height_u16,
                    data: payload.into(),
                }))
            }
            _ => Err(anyhow!("unsupported capture frame version: {version}")),
        }
    }

    async fn write_capture_frame(stream: &mut TcpStream, bitmap: &BitmapUpdate) -> anyhow::Result<()> {
        let width_u16 = bitmap.width.get();
        let height_u16 = bitmap.height.get();
        let stride_u32 = u32::try_from(bitmap.stride.get()).map_err(|_| anyhow!("stride out of range"))?;
        let payload_len_u32 = u32::try_from(bitmap.data.len()).map_err(|_| anyhow!("payload too large"))?;

        let mut header = [0u8; CAPTURE_FRAME_HEADER_LEN];
        header[0..4].copy_from_slice(&CAPTURE_FRAME_MAGIC);
        header[4..6].copy_from_slice(&1u16.to_le_bytes());
        header[6..8].copy_from_slice(&width_u16.to_le_bytes());
        header[8..10].copy_from_slice(&height_u16.to_le_bytes());
        header[10..14].copy_from_slice(&stride_u32.to_le_bytes());
        header[14] = 0; // BgrA32
        header[20..24].copy_from_slice(&payload_len_u32.to_le_bytes());

        stream
            .write_all(&header)
            .await
            .context("failed to write capture header")?;
        stream
            .write_all(bitmap.data.as_ref())
            .await
            .context("failed to write capture payload")?;

        Ok(())
    }

    async fn write_capture_frame_preencoded(
        stream: &mut TcpStream,
        width: u16,
        height: u16,
        codec_id: u8,
        data: &[u8],
    ) -> anyhow::Result<()> {
        let payload_len_u32 = u32::try_from(data.len()).map_err(|_| anyhow!("pre-encoded payload too large"))?;

        let mut header = [0u8; CAPTURE_FRAME_HEADER_LEN];
        header[0..4].copy_from_slice(&CAPTURE_FRAME_MAGIC);
        header[4..6].copy_from_slice(&2u16.to_le_bytes()); // version 2 = pre-encoded
        header[6..8].copy_from_slice(&width.to_le_bytes());
        header[8..10].copy_from_slice(&height.to_le_bytes());
        header[14] = codec_id;
        header[20..24].copy_from_slice(&payload_len_u32.to_le_bytes());

        stream
            .write_all(&header)
            .await
            .context("failed to write pre-encoded capture header")?;
        stream
            .write_all(data)
            .await
            .context("failed to write pre-encoded capture payload")?;

        Ok(())
    }

    static BITMAP_DUMP_DIR: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    static BITMAP_DUMP_SEQ: AtomicU64 = AtomicU64::new(0);
    static BITMAP_DUMP_ENABLED_LOGGED: AtomicBool = AtomicBool::new(false);
    static BITMAP_DUMP_ERROR_LOGGED: AtomicBool = AtomicBool::new(false);
    static BITMAP_DUMP_LAST_MS: AtomicU64 = AtomicU64::new(0);
    static BITMAP_DUMP_COUNT: AtomicU64 = AtomicU64::new(0);

    const BITMAP_DUMP_INTERVAL_MS: u64 = 5_000;
    const BITMAP_DUMP_MAX_COUNT: u64 = 30;

    fn process_id_to_session_id(pid: u32) -> Option<u32> {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn ProcessIdToSessionId(process_id: u32, session_id: *mut u32) -> BOOL;
        }

        let mut session_id = 0u32;

        // SAFETY: `ProcessIdToSessionId` writes to `session_id` on success.
        let ok = unsafe { ProcessIdToSessionId(pid, &mut session_id) };
        if ok.as_bool() {
            Some(session_id)
        } else {
            None
        }
    }

    fn current_process_session_id() -> Option<u32> {
        let pid = unsafe { GetCurrentProcessId() };
        process_id_to_session_id(pid)
    }

    fn now_unix_ms_best_effort() -> Option<u64> {
        let now = std::time::SystemTime::now();
        let dur = now.duration_since(std::time::UNIX_EPOCH).ok()?;
        Some(dur.as_millis().min(u128::from(u64::MAX)) as u64)
    }

    fn bitmap_dump_dir() -> Option<&'static std::path::PathBuf> {
        BITMAP_DUMP_DIR
            .get_or_init(|| {
                let raw = std::env::var_os(DUMP_BITMAP_UPDATES_DIR_ENV)?;
                let raw = raw.to_string_lossy();
                let trimmed = raw.trim();

                let dir = if trimmed.is_empty()
                    || trimmed.eq_ignore_ascii_case("1")
                    || trimmed.eq_ignore_ascii_case("true")
                    || trimmed.eq_ignore_ascii_case("yes")
                {
                    std::env::temp_dir().join("ironrdp-wts-bitmap-updates")
                } else {
                    std::path::PathBuf::from(trimmed)
                };

                Some(dir)
            })
            .as_ref()
    }

    fn maybe_dump_bitmap_update_bgra32(width: NonZeroU16, height: NonZeroU16, stride: NonZeroUsize, data: &[u8]) {
        let Some(dir) = bitmap_dump_dir() else {
            return;
        };

        if !BITMAP_DUMP_ENABLED_LOGGED.swap(true, Ordering::Relaxed) {
            info!(
                dir = %dir.display(),
                interval_ms = BITMAP_DUMP_INTERVAL_MS,
                max_count = BITMAP_DUMP_MAX_COUNT,
                "Bitmap dumping enabled (unset IRONRDP_WTS_DUMP_BITMAP_UPDATES_DIR to disable)"
            );
        }

        // Avoid generating unbounded huge BMP files (a full desktop frame can be multiple MB).
        // This is only a diagnostics mechanism; rate-limit and cap the number of dumps.
        if let Some(now_ms) = now_unix_ms_best_effort() {
            let last_ms = BITMAP_DUMP_LAST_MS.load(Ordering::Relaxed);
            if last_ms != 0 && now_ms.saturating_sub(last_ms) < BITMAP_DUMP_INTERVAL_MS {
                return;
            }

            let count = BITMAP_DUMP_COUNT.load(Ordering::Relaxed);
            if count >= BITMAP_DUMP_MAX_COUNT {
                return;
            }

            BITMAP_DUMP_LAST_MS.store(now_ms, Ordering::Relaxed);
            BITMAP_DUMP_COUNT.fetch_add(1, Ordering::Relaxed);
        }

        if let Err(error) = dump_bitmap_update_bgra32_impl(dir, width, height, stride, data) {
            if !BITMAP_DUMP_ERROR_LOGGED.swap(true, Ordering::Relaxed) {
                warn!(error = %format!("{error:#}"), "Failed to dump captured bitmap");
            }
        }
    }

    fn dump_bitmap_update_bgra32_impl(
        dir: &std::path::Path,
        width: NonZeroU16,
        height: NonZeroU16,
        stride: NonZeroUsize,
        data: &[u8],
    ) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create bitmap dump directory: {}", dir.display()))?;

        let expected = stride
            .get()
            .checked_mul(NonZeroUsize::from(height).get())
            .ok_or_else(|| anyhow!("bitmap dump length overflow"))?;

        if data.len() != expected {
            return Err(anyhow!(
                "bitmap dump length mismatch (got {}, expected {})",
                data.len(),
                expected
            ));
        }

        let pixels_len_u32 = u32::try_from(expected).map_err(|_| anyhow!("bitmap dump payload too large"))?;
        let header_len_u32 = 14u32 + 40u32;
        let file_len_u32 = header_len_u32
            .checked_add(pixels_len_u32)
            .ok_or_else(|| anyhow!("bitmap dump file length overflow"))?;

        let width_i32 = NonZeroI32::from(width).get();
        let height_i32 = NonZeroI32::from(height)
            .get()
            .checked_neg()
            .ok_or_else(|| anyhow!("bitmap dump height overflow"))?;

        let pid = std::process::id();
        let session_id = current_process_session_id().unwrap_or(0);
        let seq = BITMAP_DUMP_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
        let path = dir.join(format!("bitmap-update-s{session_id}-p{pid}-{seq:08}.bmp"));

        let mut header = [0u8; 54];
        header[0..2].copy_from_slice(b"BM");
        header[2..6].copy_from_slice(&file_len_u32.to_le_bytes());
        header[10..14].copy_from_slice(&header_len_u32.to_le_bytes());

        header[14..18].copy_from_slice(&40u32.to_le_bytes());
        header[18..22].copy_from_slice(&width_i32.to_le_bytes());
        header[22..26].copy_from_slice(&height_i32.to_le_bytes());
        header[26..28].copy_from_slice(&1u16.to_le_bytes());
        header[28..30].copy_from_slice(&32u16.to_le_bytes());
        header[30..34].copy_from_slice(&0u32.to_le_bytes());
        header[34..38].copy_from_slice(&pixels_len_u32.to_le_bytes());

        let mut file = std::fs::File::create(&path)
            .with_context(|| format!("failed to create bitmap dump file: {}", path.display()))?;
        file.write_all(&header)
            .with_context(|| format!("failed to write bitmap dump header: {}", path.display()))?;
        file.write_all(data)
            .with_context(|| format!("failed to write bitmap dump pixels: {}", path.display()))?;

        Ok(())
    }

    fn capture_bitmap_update(size: DesktopSize) -> anyhow::Result<BitmapUpdate> {
        let (width, height) = desktop_size_nonzero(size)?;
        let width_i32 = i32::from(NonZeroI32::from(width));
        let height_i32 = i32::from(NonZeroI32::from(height));
        let top_down_height = height_i32
            .checked_neg()
            .ok_or_else(|| anyhow!("desktop height overflow while creating top-down bitmap"))?;

        let stride = NonZeroUsize::from(width)
            .checked_mul(NonZeroUsize::new(4).ok_or_else(|| anyhow!("invalid bytes-per-pixel value"))?)
            .ok_or_else(|| anyhow!("frame stride overflow"))?;
        let frame_len = stride
            .get()
            .checked_mul(NonZeroUsize::from(height).get())
            .ok_or_else(|| anyhow!("frame buffer length overflow"))?;

        // SAFETY: `GetDC(None)` is safe to call and returns the DC for the entire screen.
        let screen_dc = unsafe { GetDC(None) };
        if screen_dc.0.is_null() {
            return Err(anyhow!("GetDC returned a null screen device context"));
        }

        // SAFETY: `screen_dc` is a valid display DC obtained above.
        let memory_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
        if memory_dc.0.is_null() {
            // SAFETY: `screen_dc` was acquired with `GetDC` and must be released.
            let _ = unsafe { ReleaseDC(None, screen_dc) };
            return Err(anyhow!("CreateCompatibleDC returned a null memory device context"));
        }

        let mut bitmap_info = BITMAPINFO::default();
        bitmap_info.bmiHeader.biSize =
            u32::try_from(size_of::<BITMAPINFOHEADER>()).map_err(|_| anyhow!("BITMAPINFOHEADER size overflow"))?;
        bitmap_info.bmiHeader.biWidth = width_i32;
        bitmap_info.bmiHeader.biHeight = top_down_height;
        bitmap_info.bmiHeader.biPlanes = 1;
        bitmap_info.bmiHeader.biBitCount = 32;
        bitmap_info.bmiHeader.biCompression = BI_RGB.0;

        let mut bits_ptr: *mut c_void = null_mut();

        // SAFETY: `screen_dc` and `bitmap_info` are valid, and we pass a valid out-pointer for bits.
        let bitmap = unsafe { CreateDIBSection(Some(screen_dc), &bitmap_info, DIB_RGB_COLORS, &mut bits_ptr, None, 0) }
            .map_err(|error| anyhow!("CreateDIBSection failed: {error}"))?;

        if bitmap.0.is_null() {
            // SAFETY: `memory_dc` and `screen_dc` are valid handles created above.
            let _ = unsafe { DeleteDC(memory_dc) };
            // SAFETY: `screen_dc` was acquired with `GetDC` and must be released.
            let _ = unsafe { ReleaseDC(None, screen_dc) };
            return Err(anyhow!("CreateDIBSection returned a null bitmap handle"));
        }

        // SAFETY: `memory_dc` is valid and `bitmap` is a valid bitmap handle.
        let previous_bitmap = unsafe { SelectObject(memory_dc, HGDIOBJ(bitmap.0)) };
        if previous_bitmap.0.is_null() {
            // SAFETY: `bitmap`, `memory_dc`, and `screen_dc` were created above and must be released.
            let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
            // SAFETY: `memory_dc` is a valid memory DC created above.
            let _ = unsafe { DeleteDC(memory_dc) };
            // SAFETY: `screen_dc` was acquired with `GetDC` and must be released.
            let _ = unsafe { ReleaseDC(None, screen_dc) };
            return Err(anyhow!("SelectObject failed for capture bitmap"));
        }

        // SAFETY: all DC handles are valid and dimensions are taken from initialized state.
        // CAPTUREBLT requests that layered windows and DWM-composited surfaces be included.
        let bitblt_result = unsafe {
            BitBlt(
                memory_dc,
                0,
                0,
                width_i32,
                height_i32,
                Some(screen_dc),
                0,
                0,
                SRCCOPY | CAPTUREBLT,
            )
        };

        let mut data = Vec::new();
        if bitblt_result.is_ok() {
            if bits_ptr.is_null() {
                // SAFETY: clean up GDI objects created above.
                let _ = unsafe { SelectObject(memory_dc, previous_bitmap) };
                // SAFETY: clean up GDI objects created above.
                let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
                // SAFETY: clean up GDI objects created above.
                let _ = unsafe { DeleteDC(memory_dc) };
                // SAFETY: clean up GDI objects created above.
                let _ = unsafe { ReleaseDC(None, screen_dc) };
                return Err(anyhow!("CreateDIBSection returned a null bitmap data pointer"));
            }

            if let Err(error) = data.try_reserve(frame_len) {
                // SAFETY: clean up GDI objects created above.
                let _ = unsafe { SelectObject(memory_dc, previous_bitmap) };
                // SAFETY: clean up GDI objects created above.
                let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
                // SAFETY: clean up GDI objects created above.
                let _ = unsafe { DeleteDC(memory_dc) };
                // SAFETY: clean up GDI objects created above.
                let _ = unsafe { ReleaseDC(None, screen_dc) };
                return Err(anyhow!("failed to allocate frame buffer ({frame_len} bytes): {error}"));
            }

            data.resize(frame_len, 0);

            // SAFETY: source and destination buffers are valid for `frame_len` bytes and do not overlap.
            unsafe {
                core::ptr::copy_nonoverlapping(bits_ptr.cast::<u8>(), data.as_mut_ptr(), frame_len);
            }
        }

        // SAFETY: restore previous bitmap before releasing the DC and objects.
        let _ = unsafe { SelectObject(memory_dc, previous_bitmap) };
        // SAFETY: `bitmap` was created with `CreateDIBSection`.
        let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
        // SAFETY: `memory_dc` was created with `CreateCompatibleDC`.
        let _ = unsafe { DeleteDC(memory_dc) };
        // SAFETY: `screen_dc` was acquired with `GetDC`.
        let _ = unsafe { ReleaseDC(None, screen_dc) };

        bitblt_result.map_err(|error| anyhow!("BitBlt failed while capturing desktop frame: {error}"))?;

        // Optional diagnostics: dump the raw (pre-RemoteFX encode) BGRA32 frame to disk.
        maybe_dump_bitmap_update_bgra32(width, height, stride, &data);

        Ok(BitmapUpdate {
            x: 0,
            y: 0,
            width,
            height,
            format: PixelFormat::BgrA32,
            data: data.into(),
            stride,
        })
    }

    fn fallback_bitmap_update(size: DesktopSize) -> anyhow::Result<BitmapUpdate> {
        let (width, height) = desktop_size_nonzero(size)?;

        let stride = NonZeroUsize::from(width)
            .checked_mul(NonZeroUsize::new(4).ok_or_else(|| anyhow!("invalid bytes-per-pixel value"))?)
            .ok_or_else(|| anyhow!("frame stride overflow"))?;
        let frame_len = stride
            .get()
            .checked_mul(NonZeroUsize::from(height).get())
            .ok_or_else(|| anyhow!("frame buffer length overflow"))?;

        let width_usize = NonZeroUsize::from(width).get();
        let height_usize = NonZeroUsize::from(height).get();
        let stride_usize = stride.get();

        let mut data = Vec::new();
        if let Err(error) = data.try_reserve(frame_len) {
            return Err(anyhow!(
                "failed to allocate fallback frame buffer ({frame_len} bytes): {error}"
            ));
        }
        data.resize(frame_len, 0);
        let modulus = usize::from(u8::MAX) + 1;

        for y in 0..height_usize {
            let row = &mut data[(y * stride_usize)..((y + 1) * stride_usize)];
            let g = u8::try_from(y % modulus).unwrap_or(0).wrapping_mul(3);
            for x in 0..width_usize {
                let offset = x * 4;
                let b = u8::try_from(x % modulus).unwrap_or(0).wrapping_mul(5);
                row[offset] = b;
                row[offset + 1] = g;
                row[offset + 2] = 0x80;
                row[offset + 3] = 0xFF;
            }
        }

        Ok(BitmapUpdate {
            x: 0,
            y: 0,
            width,
            height,
            format: PixelFormat::BgrA32,
            data: data.into(),
            stride,
        })
    }

    fn desktop_size_nonzero(size: DesktopSize) -> anyhow::Result<(NonZeroU16, NonZeroU16)> {
        let width = NonZeroU16::new(size.width).ok_or_else(|| anyhow!("desktop width must be non-zero"))?;
        let height = NonZeroU16::new(size.height).ok_or_else(|| anyhow!("desktop height must be non-zero"))?;

        Ok((width, height))
    }

    fn desktop_size_from_gdi() -> anyhow::Result<DesktopSize> {
        // SAFETY: `GetSystemMetrics` is safe to call for these index constants.
        let width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        // SAFETY: `GetSystemMetrics` is safe to call for these index constants.
        let height = unsafe { GetSystemMetrics(SM_CYSCREEN) };

        let width_u16 = u16::try_from(width).map_err(|_| anyhow!("screen width out of range: {width}"))?;
        let height_u16 = u16::try_from(height).map_err(|_| anyhow!("screen height out of range: {height}"))?;

        if width_u16 == 0 || height_u16 == 0 {
            return Err(anyhow!(
                "screen metrics returned zero-sized desktop ({width_u16}x{height_u16})"
            ));
        }

        Ok(DesktopSize {
            width: width_u16,
            height: height_u16,
        })
    }

    /// Plaintext credentials captured from the CredSSP/NLA handshake.
    ///
    /// The connection task stores these here after `run_connection` returns the
    /// `AuthIdentity`, so the IPC handler can serve them to the WTS provider DLL
    /// when it calls `GetConnectionCredentials`.
    #[derive(Debug, Clone)]
    struct StoredCredentials {
        username: String,
        domain: String,
        password: String,
    }

    struct AcceptedSocket {
        listener_name: String,
        peer_addr: Option<String>,
        stream: TcpStream,
    }

    #[derive(Debug)]
    struct PendingIncoming {
        listener_name: String,
        connection_id: u32,
        peer_addr: Option<String>,
    }

    #[derive(Debug)]
    struct PendingBroken {
        listener_name: String,
        connection_id: u32,
        reason: String,
    }

    #[derive(Debug)]
    struct BrokenNotification {
        listener_name: String,
        connection_id: u32,
        reason: String,
    }

    struct ConnectionEntry {
        listener_name: String,
        peer_addr: Option<String>,
        stream: Option<TcpStream>,
        session_task: Option<JoinHandle<()>>,
        /// Credentials captured from the CredSSP handshake.
        ///
        /// `None` until the NLA handshake completes; queried by `GetConnectionCredentials`.
        credentials: Arc<Mutex<Option<StoredCredentials>>>,
    }

    struct ManagedListener {
        join_handle: JoinHandle<()>,
    }

    #[derive(Clone)]
    struct ControlPlane {
        state: Arc<Mutex<ServiceState>>,
        pending_wakeup_tx: watch::Sender<u64>,
        pending_seq: Arc<AtomicU64>,
    }

    impl ControlPlane {
        fn new(state: ServiceState) -> Self {
            let (pending_wakeup_tx, _pending_wakeup_rx) = watch::channel(0u64);

            Self {
                state: Arc::new(Mutex::new(state)),
                pending_wakeup_tx,
                pending_seq: Arc::new(AtomicU64::new(0)),
            }
        }

        fn notify_pending_changed(&self) {
            let next = self.pending_seq.fetch_add(1, Ordering::Relaxed).saturating_add(1);
            let _ = self.pending_wakeup_tx.send(next);
        }

        async fn handle_command(
            &self,
            command: ProviderCommand,
            pending_wakeup_rx: &mut watch::Receiver<u64>,
        ) -> ServiceEvent {
            match command {
                ProviderCommand::StartListen { listener_name } => self.start_listen(listener_name).await,
                ProviderCommand::StopListen { listener_name } => self.stop_listen(listener_name).await,
                ProviderCommand::WaitForIncoming {
                    listener_name,
                    timeout_ms,
                } => {
                    self.wait_for_incoming(listener_name, timeout_ms, pending_wakeup_rx)
                        .await
                }
                ProviderCommand::AcceptConnection { connection_id } => self.accept_connection(connection_id).await,
                ProviderCommand::CloseConnection { connection_id } => self.close_connection(connection_id).await,
                ProviderCommand::GetConnectionCredentials { connection_id } => {
                    self.get_connection_credentials(connection_id).await
                }
                ProviderCommand::SetCaptureSessionId {
                    connection_id,
                    session_id,
                } => self.set_capture_session_id(connection_id, session_id).await,
                ProviderCommand::NotifyIddDriverLoaded { session_id } => {
                    let mut guard = self.state.lock().await;
                    guard.notify_idd_driver_loaded(session_id)
                }
            }
        }

        async fn start_listen(&self, listener_name: String) -> ServiceEvent {
            {
                let guard = self.state.lock().await;
                if guard.listeners.contains_key(&listener_name) {
                    return ServiceEvent::ListenerStarted { listener_name };
                }
            }

            let (bind_addr, accept_tx) = {
                let guard = self.state.lock().await;
                (guard.bind_addr, guard.accepted_tx.clone())
            };

            let listener = match TcpListener::bind(bind_addr).await {
                Ok(listener) => listener,
                Err(error) => {
                    return ServiceEvent::Error {
                        message: format!("failed to bind listener socket: {error}"),
                    };
                }
            };

            let listener_name_for_task = listener_name.clone();
            let join_handle = tokio::spawn(async move {
                loop {
                    match listener.accept().await {
                        Ok((stream, peer_addr)) => {
                            let accepted = AcceptedSocket {
                                listener_name: listener_name_for_task.clone(),
                                peer_addr: Some(peer_addr.to_string()),
                                stream,
                            };

                            if accept_tx.send(accepted).is_err() {
                                return;
                            }
                        }
                        Err(error) => {
                            warn!(%error, listener_name = %listener_name_for_task, "Listener accept loop failed");
                            return;
                        }
                    }
                }
            });

            {
                let mut guard = self.state.lock().await;
                if guard.listeners.contains_key(&listener_name) {
                    join_handle.abort();
                    return ServiceEvent::ListenerStarted { listener_name };
                }

                guard
                    .listeners
                    .insert(listener_name.clone(), ManagedListener { join_handle });
            }

            info!(%listener_name, bind_addr = %bind_addr, "Started control-plane listener task");

            ServiceEvent::ListenerStarted { listener_name }
        }

        async fn stop_listen(&self, listener_name: String) -> ServiceEvent {
            let connection_ids_to_close = {
                let mut guard = self.state.lock().await;

                if let Some(listener) = guard.listeners.remove(&listener_name) {
                    listener.join_handle.abort();
                }

                guard
                    .pending_incoming
                    .retain(|pending| pending.listener_name != listener_name);

                guard
                    .connections
                    .iter()
                    .filter_map(|(connection_id, connection)| {
                        if connection.listener_name == listener_name {
                            Some(*connection_id)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<u32>>()
            };

            for connection_id in connection_ids_to_close {
                let _ = self.close_connection(connection_id).await;
            }

            info!(%listener_name, "Stopped control-plane listener task");

            ServiceEvent::ListenerStopped { listener_name }
        }

        async fn wait_for_incoming(
            &self,
            listener_name: String,
            timeout_ms: u32,
            pending_wakeup_rx: &mut watch::Receiver<u64>,
        ) -> ServiceEvent {
            // Auto-start listener on first wait_for_incoming.
            {
                let guard = self.state.lock().await;
                if !guard.listeners.contains_key(&listener_name) {
                    drop(guard);
                    let event = self.start_listen(listener_name.clone()).await;
                    match &event {
                        ServiceEvent::ListenerStarted { .. } => {}
                        _ => return event,
                    }
                }
            }

            // Fast-path: return immediately if we already have something queued.
            {
                let mut guard = self.state.lock().await;
                if let Some(event) = guard.pop_pending_for_listener(&listener_name) {
                    return event;
                }
            }

            let timeout_duration = Duration::from_millis(u64::from(timeout_ms));
            match timeout(timeout_duration, pending_wakeup_rx.changed()).await {
                Ok(Ok(())) => {
                    let mut guard = self.state.lock().await;
                    if let Some(event) = guard.pop_pending_for_listener(&listener_name) {
                        event
                    } else {
                        ServiceEvent::NoIncoming
                    }
                }
                Ok(Err(_)) | Err(_) => ServiceEvent::NoIncoming,
            }
        }

        async fn accept_connection(&self, connection_id: u32) -> ServiceEvent {
            let mut guard = self.state.lock().await;
            guard.accept_connection(connection_id)
        }

        async fn get_connection_credentials(&self, connection_id: u32) -> ServiceEvent {
            let entry = {
                let guard = self.state.lock().await;
                guard
                    .connections
                    .get(&connection_id)
                    .map(|e| Arc::clone(&e.credentials))
            };

            let Some(credentials) = entry else {
                return ServiceEvent::NoCredentials { connection_id };
            };

            let guard = credentials.lock().await;
            match &*guard {
                Some(creds) => ServiceEvent::ConnectionCredentials {
                    connection_id,
                    username: creds.username.clone(),
                    domain: creds.domain.clone(),
                    password: creds.password.clone(),
                },
                None => ServiceEvent::NoCredentials { connection_id },
            }
        }

        async fn set_capture_session_id(&self, connection_id: u32, session_id: u32) -> ServiceEvent {
            let mut guard = self.state.lock().await;
            guard.set_capture_session_id(connection_id, session_id)
        }

        async fn close_connection(&self, connection_id: u32) -> ServiceEvent {
            let mut guard = self.state.lock().await;
            guard.close_connection(connection_id)
        }
    }

    fn find_session_process_pid_toolhelp(session_id: u32, process_name: &str) -> anyhow::Result<u32> {
        // SAFETY: CreateToolhelp32Snapshot returns a process snapshot handle on success.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|error| anyhow!("CreateToolhelp32Snapshot failed: {error}"))
            .context("CreateToolhelp32Snapshot failed")?;

        struct SnapshotGuard(HANDLE);
        impl Drop for SnapshotGuard {
            fn drop(&mut self) {
                close_handle_best_effort(self.0);
            }
        }

        let _snapshot_guard = SnapshotGuard(snapshot);

        let mut entry = PROCESSENTRY32W {
            dwSize: u32::try_from(size_of::<PROCESSENTRY32W>()).map_err(|_| anyhow!("PROCESSENTRY32W size overflow"))?,
            ..Default::default()
        };

        // SAFETY: snapshot is valid and entry points to writable PROCESSENTRY32W with initialized dwSize.
        unsafe { Process32FirstW(snapshot, &mut entry) }
            .map_err(|error| anyhow!("Process32FirstW failed: {error}"))
            .context("Process32FirstW failed")?;

        loop {
            let pid = entry.th32ProcessID;
            if pid != 0 && process_id_to_session_id(pid) == Some(session_id) {
                let process_name_len = entry
                    .szExeFile
                    .iter()
                    .position(|&code_unit| code_unit == 0)
                    .unwrap_or(entry.szExeFile.len());
                let exe_name = String::from_utf16_lossy(&entry.szExeFile[..process_name_len]);

                if exe_name.eq_ignore_ascii_case(process_name) {
                    return Ok(pid);
                }
            }

            // SAFETY: snapshot is valid and entry remains a valid output buffer between iterations.
            match unsafe { Process32NextW(snapshot, &mut entry) } {
                Ok(()) => {}
                Err(error) => {
                    if error.code() == windows::core::HRESULT::from_win32(ERROR_NO_MORE_FILES.0) {
                        break;
                    }

                    return Err(anyhow!("Process32NextW failed: {error}")).context("Process32NextW failed");
                }
            }
        }

        Err(anyhow!("{process_name} not found in session {session_id}"))
    }

    struct ServiceState {
        bind_addr: SocketAddr,
        listeners: HashMap<String, ManagedListener>,
        pending_incoming: VecDeque<PendingIncoming>,
        pending_broken: VecDeque<PendingBroken>,
        connections: HashMap<u32, ConnectionEntry>,
        next_connection_id: u32,
        accepted_tx: mpsc::UnboundedSender<AcceptedSocket>,
        broken_tx: mpsc::UnboundedSender<BrokenNotification>,
        /// Session ID per connection, set when the provider receives NotifySessionId from WTS.
        connection_session_ids: Arc<StdMutex<HashMap<u32, u32>>>,
        /// Session IDs for which the provider has signaled `NotifyIddDriverLoaded`.
        idd_loaded_sessions: HashSet<u32>,
    }

    impl ServiceState {
        fn new(
            bind_addr: SocketAddr,
        ) -> (
            Self,
            mpsc::UnboundedReceiver<AcceptedSocket>,
            mpsc::UnboundedReceiver<BrokenNotification>,
        ) {
            let (accepted_tx, accepted_rx) = mpsc::unbounded_channel();
            let (broken_tx, broken_rx) = mpsc::unbounded_channel();

            let state = Self {
                bind_addr,
                listeners: HashMap::new(),
                pending_incoming: VecDeque::new(),
                pending_broken: VecDeque::new(),
                connections: HashMap::new(),
                next_connection_id: 1,
                accepted_tx,
                broken_tx,
                connection_session_ids: Arc::new(StdMutex::new(HashMap::new())),
                idd_loaded_sessions: HashSet::new(),
            };

            (state, accepted_rx, broken_rx)
        }

        fn set_capture_session_id(&mut self, connection_id: u32, session_id: u32) -> ServiceEvent {
            let idd_loaded_for_session = self.idd_loaded_sessions.contains(&session_id);

            if let Ok(mut guard) = self.connection_session_ids.lock() {
                guard.insert(connection_id, session_id);
                info!(connection_id, session_id, "Set capture session id for connection");
                info!(
                    connection_id,
                    session_id,
                    "SESSION_PROOF_TERMSRV_SET_CAPTURE_SESSION_ID_APPLIED"
                );
                info!(
                    connection_id,
                    session_id,
                    idd_loaded_for_session,
                    "SESSION_PROOF_TERMSRV_IDD_SESSION_BIND"
                );

                if idd_loaded_for_session {
                    info!(
                        connection_id,
                        session_id,
                        "SESSION_PROOF_TERMSRV_IDD_READY_FOR_CAPTURE"
                    );
                }
            }
            ServiceEvent::Ack
        }

        fn notify_idd_driver_loaded(&mut self, session_id: u32) -> ServiceEvent {
            let inserted = self.idd_loaded_sessions.insert(session_id);

            info!(session_id, inserted, "WDDM IDD driver loaded (notification)");
            info!(session_id, inserted, "SESSION_PROOF_TERMSRV_IDD_DRIVER_LOADED");

            if let Ok(guard) = self.connection_session_ids.lock() {
                for (connection_id, mapped_session_id) in guard.iter() {
                    if *mapped_session_id != session_id {
                        continue;
                    }

                    info!(
                        connection_id = *connection_id,
                        session_id,
                        "SESSION_PROOF_TERMSRV_IDD_READY_FOR_CAPTURE"
                    );
                }
            }

            ServiceEvent::Ack
        }

        fn accept_connection(&mut self, connection_id: u32) -> ServiceEvent {
            let known_ids: Vec<u32> = self.connections.keys().copied().collect();
            info!(
                connection_id,
                ?known_ids,
                total_connections = self.connections.len(),
                "accept_connection ENTRY"
            );
            let connection = match self.connections.get_mut(&connection_id) {
                Some(connection) => connection,
                None => {
                    warn!(connection_id, ?known_ids, "accept_connection: MISS");
                    return ServiceEvent::Error {
                        message: format!("MISS connection id={connection_id} known={known_ids:?}"),
                    };
                }
            };

            if connection.session_task.is_some() {
                return ServiceEvent::ConnectionReady { connection_id };
            }

            let Some(stream) = connection.stream.take() else {
                return ServiceEvent::Error {
                    message: format!("connection stream already consumed: {connection_id}"),
                };
            };

            let peer_addr = connection.peer_addr.clone();
            let credentials_slot = Arc::clone(&connection.credentials);
            let connection_session_ids = Arc::clone(&self.connection_session_ids);
            let listener_name_for_task = connection.listener_name.clone();
            let broken_tx = self.broken_tx.clone();

            let session_task = tokio::task::spawn_local(async move {
                let result = run_ironrdp_connection(
                    connection_id,
                    peer_addr.as_deref(),
                    stream,
                    credentials_slot,
                    connection_session_ids,
                    true, // provider_mode: WTS provider DLL will send SetCaptureSessionId
                )
                .await;

                let reason = match &result {
                    Ok(()) => "connection closed".to_owned(),
                    Err(error) => format!("{error:#}"),
                };

                let _ = broken_tx.send(BrokenNotification {
                    listener_name: listener_name_for_task,
                    connection_id,
                    reason,
                });

                if let Err(error) = result {
                    warn!(error = %format!("{error:#}"), connection_id, "IronRDP connection task failed");
                }
            });

            connection.session_task = Some(session_task);

            ServiceEvent::ConnectionReady { connection_id }
        }

        fn close_connection(&mut self, connection_id: u32) -> ServiceEvent {
            if let Ok(mut guard) = self.connection_session_ids.lock() {
                guard.remove(&connection_id);
            }
            if let Some(entry) = self.connections.remove(&connection_id) {
                if let Some(session_task) = entry.session_task {
                    session_task.abort();
                }

                info!(
                    connection_id,
                    listener_name = %entry.listener_name,
                    peer_addr = ?entry.peer_addr,
                    "Closed connection entry"
                );
            }

            ServiceEvent::Ack
        }

        fn pop_pending_for_listener(&mut self, listener_name: &str) -> Option<ServiceEvent> {
            if let Some(index) = self
                .pending_broken
                .iter()
                .position(|pending| pending.listener_name == listener_name)
            {
                let pending = self.pending_broken.remove(index)?;
                return Some(ServiceEvent::ConnectionBroken {
                    connection_id: pending.connection_id,
                    reason: pending.reason,
                });
            }

            let index = self
                .pending_incoming
                .iter()
                .position(|pending| pending.listener_name == listener_name)?;

            let pending = self.pending_incoming.remove(index)?;

            Some(ServiceEvent::IncomingConnection {
                listener_name: pending.listener_name,
                connection_id: pending.connection_id,
                peer_addr: pending.peer_addr,
            })
        }

        fn register_accepted(&mut self, accepted: AcceptedSocket) {
            let connection_id = self.next_connection_id;
            self.next_connection_id = self.next_connection_id.saturating_add(1);

            let listener_name = accepted.listener_name;
            let peer_addr = accepted.peer_addr;

            info!(
                connection_id,
                %listener_name,
                peer_addr = ?peer_addr,
                "Registered incoming TCP connection"
            );

            // Start processing the TCP stream immediately so the wire-level handshake behaves
            // like a real RDP server (responding to the ConnectionRequest without waiting for
            // TermService IPC round-trips). TermService will still call AcceptConnection /
            // NotifySessionId / etc, but those callbacks should not gate initial protocol IO.
            let credentials = Arc::new(Mutex::new(None));
            let credentials_slot = Arc::clone(&credentials);
            let connection_session_ids = Arc::clone(&self.connection_session_ids);
            let peer_addr_for_task = peer_addr.clone();
            let listener_name_for_task = listener_name.clone();
            let broken_tx = self.broken_tx.clone();

            let stream = accepted.stream;
            let session_task = tokio::task::spawn_local(async move {
                let result = run_ironrdp_connection(
                    connection_id,
                    peer_addr_for_task.as_deref(),
                    stream,
                    credentials_slot,
                    connection_session_ids,
                    true, // provider_mode: WTS provider DLL will send SetCaptureSessionId
                )
                .await;

                let reason = match &result {
                    Ok(()) => "connection closed".to_owned(),
                    Err(error) => format!("{error:#}"),
                };

                let _ = broken_tx.send(BrokenNotification {
                    listener_name: listener_name_for_task,
                    connection_id,
                    reason,
                });

                if let Err(error) = result {
                    warn!(
                        error = %format!("{error:#}"),
                        connection_id,
                        "IronRDP connection task failed"
                    );
                }
            });

            self.connections.insert(
                connection_id,
                ConnectionEntry {
                    listener_name: listener_name.clone(),
                    peer_addr: peer_addr.clone(),
                    stream: None,
                    session_task: Some(session_task),
                    credentials,
                },
            );

            self.pending_incoming.push_back(PendingIncoming {
                listener_name,
                connection_id,
                peer_addr,
            });
        }
    }

    async fn run_ironrdp_connection(
        connection_id: u32,
        peer_addr: Option<&str>,
        stream: TcpStream,
        credentials_slot: Arc<Mutex<Option<StoredCredentials>>>,
        connection_session_ids: Arc<StdMutex<HashMap<u32, u32>>>,
        provider_mode: bool,
    ) -> anyhow::Result<()> {
        info!(connection_id, peer_addr = ?peer_addr, "Starting IronRDP session task");

        let input_stream_slot: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
        let (input_tx, input_rx) = mpsc::unbounded_channel::<InputPacket>();
        let input_task = tokio::task::spawn_local(run_input_spooler(
            connection_id,
            Arc::clone(&input_stream_slot),
            input_rx,
        ));

        let display = GdiDisplay::new(
            connection_id,
            Arc::clone(&input_stream_slot),
            connection_session_ids,
            Arc::clone(&credentials_slot),
            provider_mode,
        )
        .context("failed to initialize GDI display handler")?;
        let (tls_acceptor, tls_pub_key) = make_tls_acceptor().context("failed to initialize TLS acceptor")?;

        let input_handler = TermSrvInputHandler::new(connection_id, input_tx);

        let mut server = {
            let builder = RdpServer::builder().with_addr(([127, 0, 0, 1], 0));
            let builder = builder.with_hybrid(tls_acceptor, tls_pub_key);

            let rfx_only_codecs =
                ironrdp_pdu::rdp::capability_sets::server_codecs_capabilities(&["remotefx:on", "qoi:off", "qoiz:off"])
                    .expect("valid codec config");

            builder
                .with_input_handler(input_handler)
                .with_display_handler(display)
                .with_bitmap_codecs(rfx_only_codecs)
                .build()
        };

        if provider_mode {
            let expected_credentials = resolve_rdp_credentials_from_env()?;

            if let Some(credentials) = expected_credentials {
                info!(
                    username = %credentials.username,
                    domain = ?credentials.domain,
                    "Configured expected RDP credentials for provider-mode CredSSP"
                );
                server.set_credentials(Some(credentials));
            } else {
                warn!(
                    username_env = %RDP_USERNAME_ENV,
                    password_env = %RDP_PASSWORD_ENV,
                    domain_env = %RDP_DOMAIN_ENV,
                    "RDP credentials are not configured; provider-mode CredSSP handshake may fail"
                );
            }

            server.set_allow_unverified_credentials(true);

            let credentials_slot = Arc::clone(&credentials_slot);
            server.set_client_info_credentials_sink(move |creds| {
                let username = creds.username;
                let domain = creds.domain.unwrap_or_default();
                let password = creds.password;

                if username.is_empty() || password.is_empty() {
                    info!(
                        connection_id,
                        username = %username,
                        domain = %domain,
                        "Received ClientInfo credentials without username/password; ignoring"
                    );
                    return;
                }

                info!(
                    connection_id,
                    username = %username,
                    domain = %domain,
                    "Captured ClientInfo credentials; storing for WTS provider"
                );

                let credentials_slot = Arc::clone(&credentials_slot);
                tokio::task::spawn_local(async move {
                    let mut guard = credentials_slot.lock().await;
                    if guard.is_none() {
                        *guard = Some(StoredCredentials {
                            username,
                            domain,
                            password,
                        });
                    }
                });
            });
        } else {
            let expected_credentials = resolve_rdp_credentials_from_env()?;

            if let Some(credentials) = expected_credentials {
                info!(username = %credentials.username, domain = ?credentials.domain, "Configured expected RDP credentials");
                server.set_credentials(Some(credentials));
            } else {
                warn!(
                    username_env = %RDP_USERNAME_ENV,
                    password_env = %RDP_PASSWORD_ENV,
                    domain_env = %RDP_DOMAIN_ENV,
                    "RDP credentials are not configured; standard security connections will be rejected"
                );
            }
        }

        let pending = server
            .run_connection_handshake(stream)
            .await
            .with_context(|| format!("failed handshake for connection {connection_id}"))?;

        // Store CredSSP credentials immediately so the WTS provider DLL can
        // retrieve them via `GetConnectionCredentials` before the display loop
        // blocks.
        if let Some(identity) = pending.captured_identity() {
            let username = identity.username.account_name().to_owned();
            let domain = identity.username.domain_name().unwrap_or("").to_owned();
            let password = identity.password.as_ref().to_owned();

            info!(
                connection_id,
                username = %username,
                domain = %domain,
                "Captured CredSSP credentials; storing for WTS provider"
            );

            let mut guard = credentials_slot.lock().await;
            if guard.is_none() {
                *guard = Some(StoredCredentials {
                    username,
                    domain,
                    password,
                });
            }
        }

        server
            .run_connection_session(pending)
            .await
            .with_context(|| format!("failed to run IronRDP session for connection {connection_id}"))?;

        input_task.abort();

        info!(connection_id, peer_addr = ?peer_addr, "IronRDP session task finished");
        Ok(())
    }

    fn resolve_rdp_credentials_from_env() -> anyhow::Result<Option<Credentials>> {
        let username = std::env::var(RDP_USERNAME_ENV)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        let password = std::env::var(RDP_PASSWORD_ENV)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        let domain = std::env::var(RDP_DOMAIN_ENV)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        match (username, password) {
            (None, None) => Ok(None),
            (Some(_), None) | (None, Some(_)) => Err(anyhow!(
                "both {RDP_USERNAME_ENV} and {RDP_PASSWORD_ENV} must be set together"
            )),
            (Some(username), Some(password)) => Ok(Some(Credentials {
                username,
                password,
                domain,
            })),
        }
    }

    fn resolve_wts_logon_credentials_from_env() -> anyhow::Result<Option<StoredCredentials>> {
        let username = std::env::var(WTS_LOGON_USERNAME_ENV)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        let password = std::env::var(WTS_LOGON_PASSWORD_ENV)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        let domain = std::env::var(WTS_LOGON_DOMAIN_ENV)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_default();

        match (username, password) {
            (None, None) => Ok(None),
            (Some(_), None) | (None, Some(_)) => Err(anyhow!(
                "both {WTS_LOGON_USERNAME_ENV} and {WTS_LOGON_PASSWORD_ENV} must be set together"
            )),
            (Some(username), Some(password)) => Ok(Some(StoredCredentials {
                username,
                domain,
                password,
            })),
        }
    }

    fn make_tls_acceptor() -> anyhow::Result<(TlsAcceptor, Vec<u8>)> {
        let subject_name = std::env::var("IRONRDP_TLS_CERT_SUBJECT")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| TLS_CERT_SUBJECT_FIND.to_owned());

        ensure_windows_tls_cert_in_machine_store(&subject_name)
            .with_context(|| format!("failed to ensure a self-signed certificate exists for `{subject_name}`"))?;

        let resolver = WindowsStoreCertResolver {
            subject_name,
            store_name: "My".to_owned(),
        };

        let certified_key = resolver
            .resolve_once()
            .context("resolve TLS certificate from Windows certificate store")?;

        let pub_key = {
            use x509_cert::der::Decode as _;

            let cert = certified_key
                .cert
                .first()
                .ok_or_else(|| anyhow!("TLS certificate chain is empty"))?;
            let cert = x509_cert::Certificate::from_der(cert).map_err(|source| anyhow!(source))?;

            cert.tbs_certificate
                .subject_public_key_info
                .subject_public_key
                .as_bytes()
                .ok_or_else(|| anyhow!("subject public key BIT STRING is not aligned"))?
                .to_owned()
        };

        let resolver = StaticCertifiedKeyResolver {
            certified_key: Arc::clone(&certified_key),
        };

        let mut server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(resolver));

        // This adds support for the SSLKEYLOGFILE env variable (https://wiki.wireshark.org/TLS#using-the-pre-master-secret)
        server_config.key_log = Arc::new(rustls::KeyLogFile::new());

        Ok((TlsAcceptor::from(Arc::new(server_config)), pub_key))
    }

    #[derive(Debug)]
    struct StaticCertifiedKeyResolver {
        certified_key: Arc<rustls::sign::CertifiedKey>,
    }

    impl rustls::server::ResolvesServerCert for StaticCertifiedKeyResolver {
        fn resolve(&self, _client_hello: rustls::server::ClientHello<'_>) -> Option<Arc<rustls::sign::CertifiedKey>> {
            Some(Arc::clone(&self.certified_key))
        }
    }

    #[derive(Debug)]
    struct WindowsStoreCertResolver {
        subject_name: String,
        store_name: String,
    }

    impl WindowsStoreCertResolver {
        fn resolve_once(&self) -> anyhow::Result<Arc<rustls::sign::CertifiedKey>> {
            let store = CertStore::open(CertStoreType::LocalMachine, &self.store_name)
                .context("open Windows certificate store")?;

            let mut contexts = store
                .find_by_subject_str(&self.subject_name)
                .with_context(|| format!("find certificate with subject `{}`", self.subject_name))?;

            let ctx = contexts
                .pop()
                .ok_or_else(|| anyhow!("no certificate found in Windows store"))?;

            let key_handle = ctx.acquire_key(true).context("acquire private key for certificate")?;
            let key = CngSigningKey::new(key_handle).context("wrap CNG signing key")?;

            let chain = ctx.as_chain_der().context("certificate chain is not available")?;

            let certs = chain.into_iter().map(rustls::pki_types::CertificateDer::from).collect();

            Ok(Arc::new(rustls::sign::CertifiedKey {
                cert: certs,
                key: Arc::new(key),
                ocsp: None,
            }))
        }
    }

    impl rustls::server::ResolvesServerCert for WindowsStoreCertResolver {
        fn resolve(&self, _client_hello: rustls::server::ClientHello<'_>) -> Option<Arc<rustls::sign::CertifiedKey>> {
            match self.resolve_once() {
                Ok(key) => Some(key),
                Err(error) => {
                    error!(%error, subject_name = %self.subject_name, "Failed to resolve TLS certificate from Windows certificate store");
                    None
                }
            }
        }
    }

    fn ensure_windows_tls_cert_in_machine_store(subject_name: &str) -> anyhow::Result<()> {
        // First check using wincrypt directly (fast path, no extra processing).
        if wincrypt_find_cert_by_subject(subject_name)? {
            return Ok(());
        }

        info!(%subject_name, "TLS certificate not found in machine store; generating a self-signed certificate");
        wincrypt_create_self_signed_machine_cert(subject_name).context("create self-signed machine certificate")?;
        Ok(())
    }

    fn wincrypt_open_local_machine_my() -> anyhow::Result<HCERTSTORE> {
        // SAFETY: `w!("MY")` is a valid null-terminated wide string.
        let store = unsafe {
            CertOpenStore(
                CERT_STORE_PROV_SYSTEM_W,
                CERT_QUERY_ENCODING_TYPE(0),
                None,
                CERT_OPEN_STORE_FLAGS(CERT_SYSTEM_STORE_LOCAL_MACHINE),
                Some(w!("MY").as_ptr().cast()),
            )
        }
        .context("CertOpenStore(LocalMachine\\My) failed")?;

        Ok(store)
    }

    fn wincrypt_find_cert_by_subject(subject_name: &str) -> anyhow::Result<bool> {
        let store = wincrypt_open_local_machine_my()?;

        let subject_wide: Vec<u16> = subject_name.encode_utf16().chain(Some(0)).collect();

        // SAFETY: `store` is a valid store handle.
        let found: *const CERT_CONTEXT = unsafe {
            CertFindCertificateInStore(
                store,
                X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
                0,
                CERT_FIND_SUBJECT_STR_W,
                Some(subject_wide.as_ptr().cast()),
                None,
            )
        };

        let exists = !found.is_null();
        if exists {
            // SAFETY: `found` was returned by WinCrypto and must be freed.
            unsafe {
                let _ = CertFreeCertificateContext(Some(found));
            };
        }

        // SAFETY: `store` is owned by us.
        let _ = unsafe { CertCloseStore(Some(store), 0) };

        Ok(exists)
    }

    fn wincrypt_create_self_signed_machine_cert(subject_name: &str) -> anyhow::Result<()> {
        let store = wincrypt_open_local_machine_my()?;

        // Create/overwrite a persisted machine key.
        let mut provider: NCRYPT_PROV_HANDLE = NCRYPT_PROV_HANDLE::default();
        // SAFETY: `provider` is a valid out-pointer and `MS_KEY_STORAGE_PROVIDER` is a valid null-terminated string.
        unsafe { NCryptOpenStorageProvider(&mut provider, MS_KEY_STORAGE_PROVIDER, 0) }
            .context("NCryptOpenStorageProvider failed")?;

        let key_name_wide: Vec<u16> = TLS_KEY_NAME.encode_utf16().chain(Some(0)).collect();
        let mut key = windows::Win32::Security::Cryptography::NCRYPT_KEY_HANDLE::default();

        // SAFETY: all pointers are valid for the duration of the call; `key` is a valid out-handle.
        unsafe {
            NCryptCreatePersistedKey(
                provider,
                &mut key,
                BCRYPT_RSA_ALGORITHM,
                PCWSTR(key_name_wide.as_ptr()),
                windows::Win32::Security::Cryptography::CERT_KEY_SPEC(0),
                NCRYPT_MACHINE_KEY_FLAG,
            )
        }
        .context("NCryptCreatePersistedKey failed")?;

        let key_len: u32 = 2048;
        // SAFETY: `key` is a valid key handle and the property buffer is valid for the call.
        unsafe {
            NCryptSetProperty(
                NCRYPT_HANDLE::from(key),
                NCRYPT_LENGTH_PROPERTY,
                &key_len.to_le_bytes(),
                NCRYPT_FLAGS(0),
            )
        }
        .context("NCryptSetProperty(NCRYPT_LENGTH_PROPERTY) failed")?;

        // Allow export (helps tooling) but we still use CNG directly at runtime.
        let export_policy: u32 = NCRYPT_ALLOW_EXPORT_FLAG | NCRYPT_ALLOW_PLAINTEXT_EXPORT_FLAG;
        // SAFETY: `key` is a valid key handle and the property buffer is valid for the call.
        unsafe {
            NCryptSetProperty(
                NCRYPT_HANDLE::from(key),
                NCRYPT_EXPORT_POLICY_PROPERTY,
                &export_policy.to_le_bytes(),
                NCRYPT_FLAGS(0),
            )
        }
        .context("NCryptSetProperty(NCRYPT_EXPORT_POLICY_PROPERTY) failed")?;

        // SAFETY: `key` is a valid key handle created above.
        unsafe { NCryptFinalizeKey(key, NCRYPT_FLAGS(0)) }.context("NCryptFinalizeKey failed")?;

        let subject_x500 = format!("CN={subject_name}");
        let subject_wide: Vec<u16> = subject_x500.encode_utf16().chain(Some(0)).collect();

        // Encode X.500 name.
        let mut required = 0u32;
        // SAFETY: the output length pointer is valid, and `subject_wide` is a valid null-terminated string.
        unsafe {
            CertStrToNameW(
                X509_ASN_ENCODING,
                PCWSTR(subject_wide.as_ptr()),
                CERT_X500_NAME_STR,
                None,
                None,
                &mut required,
                None,
            )
        }
        .context("CertStrToNameW (query size) failed")?;

        let required_usize = usize::try_from(required).map_err(|_| anyhow!("encoded subject name too large"))?;
        let mut encoded_name = vec![0u8; required_usize];
        // SAFETY: `encoded_name` is a valid output buffer sized using the previous call.
        unsafe {
            CertStrToNameW(
                X509_ASN_ENCODING,
                PCWSTR(subject_wide.as_ptr()),
                CERT_X500_NAME_STR,
                None,
                Some(encoded_name.as_mut_ptr()),
                &mut required,
                None,
            )
        }
        .context("CertStrToNameW (encode) failed")?;

        let name_blob = CRYPT_INTEGER_BLOB {
            cbData: required,
            pbData: encoded_name.as_mut_ptr(),
        };

        let prov_name_wide: Vec<u16> = "Microsoft Software Key Storage Provider"
            .encode_utf16()
            .chain(Some(0))
            .collect();

        let prov_info = CRYPT_KEY_PROV_INFO {
            pwszContainerName: PWSTR(key_name_wide.as_ptr().cast_mut()),
            pwszProvName: PWSTR(prov_name_wide.as_ptr().cast_mut()),
            dwProvType: 0,
            dwFlags: windows::Win32::Security::Cryptography::CRYPT_MACHINE_KEYSET,
            cProvParam: 0,
            rgProvParam: null_mut(),
            dwKeySpec: CERT_NCRYPT_KEY_SPEC.0,
        };

        // SAFETY: name/prov info buffers outlive the call.
        let cert_ctx = unsafe {
            CertCreateSelfSignCertificate(
                None,
                &name_blob,
                CERT_CREATE_SELFSIGN_FLAGS(0),
                Some(&prov_info),
                None,
                None,
                None,
                None,
            )
        };

        if cert_ctx.is_null() {
            // SAFETY: release key/provider handles.
            let _ = unsafe { NCryptFreeObject(key.into()) };
            // SAFETY: `provider` is a valid provider handle created above.
            let _ = unsafe { NCryptFreeObject(provider.into()) };
            anyhow::bail!("CertCreateSelfSignCertificate returned null");
        }

        // SAFETY: store and cert_ctx are valid.
        unsafe { CertAddCertificateContextToStore(Some(store), cert_ctx, CERT_STORE_ADD_REPLACE_EXISTING, None) }
            .context("CertAddCertificateContextToStore failed")?;

        // SAFETY: `cert_ctx` was returned by WinCrypto and must be freed.
        unsafe {
            let _ = CertFreeCertificateContext(Some(cert_ctx.cast_const()));
        };

        // SAFETY: `store` is owned by us.
        let _ = unsafe { CertCloseStore(Some(store), 0) };

        // SAFETY: release key/provider handles.
        let _ = unsafe { NCryptFreeObject(key.into()) };
        // SAFETY: `provider` is a valid provider handle created above.
        let _ = unsafe { NCryptFreeObject(provider.into()) };

        Ok(())
    }

    async fn run() -> anyhow::Result<()> {
        // Check capture helper mode BEFORE init_tracing, because when spawned via
        // CreateProcessAsUserW with CREATE_NO_WINDOW and bInheritHandles=false,
        // stderr may be invalid and the tracing subscriber could panic on first write.
        let capture_helper_mode = parse_capture_helper_mode()?;
        let is_capture_helper = capture_helper_mode.is_some();

        if is_capture_helper {
            // In capture helper mode, redirect tracing to a per-PID log file so we
            // get diagnostics even when stderr is unavailable (spawned with no console).
            let pid = std::process::id();
            let diag_dir = "C:\\IronRDPDeploy\\logs";
            let _ = std::fs::create_dir_all(diag_dir);
            let diag_path = format!("{diag_dir}\\capture-helper-{pid}.log");
            match std::fs::File::create(&diag_path) {
                Ok(file) => {
                    let env_filter = EnvFilter::builder()
                        .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
                        .with_env_var("IRONRDP_LOG")
                        .from_env_lossy();

                    let _ = tracing_subscriber::fmt()
                        .with_env_filter(env_filter)
                        .with_target(true)
                        .with_ansi(false)
                        .with_writer(std::sync::Mutex::new(file))
                        .compact()
                        .try_init();
                }
                Err(_) => {
                    // Fall back to stderr-based tracing if file creation fails.
                    let _ = init_tracing();
                }
            }
        } else {
            init_tracing()?;
        }

        if let Some(mode) = capture_helper_mode {
            match mode {
                CaptureHelperMode::Tcp {
                    connect_addr,
                    input_connect_addr,
                    rfx_encode,
                } => {
                    info!(
                        connect_addr = %connect_addr,
                        input_connect_addr = %input_connect_addr,
                        rfx_encode,
                        "Starting capture helper mode (tcp)"
                    );
                    return run_capture_helper_tcp(connect_addr, input_connect_addr, rfx_encode).await;
                }
                CaptureHelperMode::SharedMem {
                    map_name,
                    event_name,
                    input_connect_addr,
                } => {
                    info!(
                        map_name = %map_name,
                        event_name = %event_name,
                        input_connect_addr = %input_connect_addr,
                        "Starting capture helper mode (sharedmem)"
                    );
                    return run_capture_helper_shared_mem(&map_name, &event_name, input_connect_addr).await;
                }
            }
        }

        let pipe_name = resolve_pipe_name_from_env().unwrap_or_else(default_pipe_name);
        let bind_addr = resolve_bind_addr()?;

        if auto_listen_enabled() {
            let listener_name = std::env::var(AUTO_LISTEN_NAME_ENV)
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "IRDP-Tcp".to_owned());

            info!(
                %listener_name,
                bind_addr = %bind_addr,
                "Starting termsrv standalone listener (auto-listen)"
            );

            return run_standalone_listener(bind_addr).await;
        }

        let instance_id = std::process::id();
        info!(pipe = %pipe_name, instance_id, "Starting termsrv control loop");
        info!(bind_addr = %bind_addr, "Configured service listener bind address");

        let (state, mut accepted_rx, mut broken_rx) = ServiceState::new(bind_addr);
        let control_plane = ControlPlane::new(state);

        // Drain accepted TCP connections continuously so `WaitForIncoming` never needs to hold
        // the global state lock across a timed wait.
        {
            let control_plane = control_plane.clone();
            tokio::task::spawn_local(async move {
                while let Some(accepted) = accepted_rx.recv().await {
                    {
                        let mut guard = control_plane.state.lock().await;
                        guard.register_accepted(accepted);
                    }
                    control_plane.notify_pending_changed();
                }

                warn!("Accepted TCP drain task ended; no further incoming connections will be registered");
            });
        }

        {
            let control_plane = control_plane.clone();
            tokio::task::spawn_local(async move {
                while let Some(notification) = broken_rx.recv().await {
                    let connection_id = notification.connection_id;
                    {
                        let mut guard = control_plane.state.lock().await;
                        guard.pending_broken.push_back(PendingBroken {
                            listener_name: notification.listener_name,
                            connection_id,
                            reason: notification.reason,
                        });
                        let _ = guard.close_connection(connection_id);
                    }

                    control_plane.notify_pending_changed();
                }
            });
        }

        let full_pipe_name = pipe_path(&pipe_name);
        let empty_disconnects = Arc::new(AtomicU64::new(0));

        // Multiple pipe server instances reduce `ERROR_PIPE_BUSY` under concurrent provider calls.
        // Each instance independently serves one client connection at a time.
        for _ in 0..CONTROL_PIPE_SERVER_INSTANCES {
            let control_plane = control_plane.clone();
            let full_pipe_name = full_pipe_name.clone();
            let empty_disconnects = Arc::clone(&empty_disconnects);
            tokio::task::spawn_local(async move {
                run_control_pipe_instance_loop(&full_pipe_name, control_plane, empty_disconnects).await;
            });
        }

        #[expect(clippy::infinite_loop, reason = "service runs indefinitely")]
        loop {
            sleep(Duration::from_secs(3600)).await;
        }
    }

    fn auto_listen_enabled() -> bool {
        let configured = std::env::var(AUTO_LISTEN_ENV)
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .unwrap_or_default();

        matches!(configured.as_str(), "1" | "true" | "yes" | "on")
    }

    async fn run_standalone_listener(bind_addr: SocketAddr) -> anyhow::Result<()> {
        let listener = TcpListener::bind(bind_addr)
            .await
            .context("failed to bind standalone listener socket")?;

        let mut next_connection_id: u32 = 1;

        loop {
            let (stream, peer_addr) = listener.accept().await.context("standalone listener accept failed")?;

            let connection_id = next_connection_id;
            next_connection_id = next_connection_id.saturating_add(1);

            let peer_addr = peer_addr.to_string();

            info!(connection_id, peer_addr = %peer_addr, "Client accepted");

            tokio::task::spawn_local(async move {
                // In standalone mode there is no WTS provider to query credentials; use a
                // throw-away slot (credentials won't be read by anyone).
                let credentials_slot = Arc::new(Mutex::new(None));
                let connection_session_ids = Arc::new(StdMutex::new(HashMap::new()));
                if let Err(error) = run_ironrdp_connection(
                    connection_id,
                    Some(&peer_addr),
                    stream,
                    credentials_slot,
                    connection_session_ids,
                    false, // provider_mode: standalone mode, session ID resolved immediately
                )
                .await
                {
                    warn!(
                        error = %format!("{error:#}"),
                        connection_id,
                        peer_addr = %peer_addr,
                        "IronRDP connection task failed"
                    );
                }
            });
        }
    }

    fn resolve_bind_addr() -> anyhow::Result<SocketAddr> {
        let configured = std::env::var(LISTEN_ADDR_ENV)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_LISTEN_ADDR.to_owned());

        configured
            .parse()
            .with_context(|| format!("failed to parse {LISTEN_ADDR_ENV} as socket address: {configured}"))
    }

    fn init_tracing() -> anyhow::Result<()> {
        let env_filter = EnvFilter::builder()
            .with_default_directive(tracing::level_filters::LevelFilter::INFO.into())
            .with_env_var("IRONRDP_LOG")
            .from_env_lossy();

        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(true)
            .compact()
            .try_init()
            .map_err(|error| anyhow::anyhow!("failed to initialize tracing subscriber: {error}"))
    }

    enum CaptureHelperMode {
        Tcp {
            connect_addr: SocketAddr,
            input_connect_addr: SocketAddr,
            rfx_encode: bool,
        },
        SharedMem {
            map_name: String,
            event_name: String,
            input_connect_addr: SocketAddr,
        },
    }

    fn parse_capture_helper_mode() -> anyhow::Result<Option<CaptureHelperMode>> {
        let mut args = std::env::args().skip(1);

        let mut capture_helper = false;
        let mut connect: Option<SocketAddr> = None;
        let mut input_connect: Option<SocketAddr> = None;
        let mut map_name: Option<String> = None;
        let mut event_name: Option<String> = None;
        let mut rfx_encode = false;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--capture-helper" => {
                    capture_helper = true;
                }
                "--rfx-encode" => {
                    rfx_encode = true;
                }
                "--connect" => {
                    let Some(value) = args.next() else {
                        return Err(anyhow!("--connect requires a value"));
                    };
                    connect = Some(
                        value
                            .parse()
                            .with_context(|| format!("failed to parse --connect address: {value}"))?,
                    );
                }
                "--input-connect" => {
                    let Some(value) = args.next() else {
                        return Err(anyhow!("--input-connect requires a value"));
                    };
                    input_connect = Some(
                        value
                            .parse()
                            .with_context(|| format!("failed to parse --input-connect address: {value}"))?,
                    );
                }
                "--shm-map" => {
                    let Some(value) = args.next() else {
                        return Err(anyhow!("--shm-map requires a value"));
                    };
                    map_name = Some(value);
                }
                "--shm-event" => {
                    let Some(value) = args.next() else {
                        return Err(anyhow!("--shm-event requires a value"));
                    };
                    event_name = Some(value);
                }
                _ => {}
            }
        }

        if !capture_helper {
            return Ok(None);
        }

        let input_connect_addr = input_connect.ok_or_else(|| anyhow!("--capture-helper requires --input-connect"))?;

        if let Some(connect) = connect {
            if map_name.is_some() || event_name.is_some() {
                return Err(anyhow!("--connect cannot be combined with --shm-map/--shm-event"));
            }
            return Ok(Some(CaptureHelperMode::Tcp {
                connect_addr: connect,
                input_connect_addr,
                rfx_encode,
            }));
        }

        let map_name =
            map_name.ok_or_else(|| anyhow!("--capture-helper requires --connect or --shm-map/--shm-event"))?;
        let event_name = event_name.ok_or_else(|| anyhow!("--capture-helper requires --shm-event"))?;
        Ok(Some(CaptureHelperMode::SharedMem {
            map_name,
            event_name,
            input_connect_addr,
        }))
    }

    #[derive(Debug, Clone, Copy)]
    enum HelperInputEvent {
        ScancodeKey { code: u8, extended: bool, released: bool },
        UnicodeKey { ch: u16, released: bool },
        MouseMoveAbs { x: u16, y: u16 },
        MouseMoveRel { dx: i32, dy: i32 },
        MouseButton { button: u8, down: bool },
        MouseWheel { delta: i32 },
        MouseHWheel { delta: i32 },
    }

    async fn read_helper_input_event(stream: &mut TcpStream) -> anyhow::Result<HelperInputEvent> {
        let mut header = [0u8; INPUT_FRAME_HEADER_LEN];
        stream
            .read_exact(&mut header)
            .await
            .context("failed to read input header")?;

        if header[0..4] != INPUT_FRAME_MAGIC {
            return Err(anyhow!("input stream magic mismatch"));
        }

        let version = u16::from_le_bytes([header[4], header[5]]);
        if version != INPUT_FRAME_VERSION {
            return Err(anyhow!("unsupported input stream version: {version}"));
        }

        let msg_type = header[6];
        let payload_len = usize::from(header[7]);

        let mut payload = vec![0u8; payload_len];
        stream
            .read_exact(&mut payload)
            .await
            .context("failed to read input payload")?;

        match msg_type {
            INPUT_MSG_SCANCODE_KEY => {
                if payload.len() != 2 {
                    return Err(anyhow!("invalid scancode key payload length"));
                }
                let flags = payload[0];
                let code = payload[1];
                Ok(HelperInputEvent::ScancodeKey {
                    code,
                    extended: (flags & INPUT_KEY_FLAG_EXTENDED) != 0,
                    released: (flags & INPUT_KEY_FLAG_RELEASE) != 0,
                })
            }
            INPUT_MSG_UNICODE_KEY => {
                if payload.len() != 3 {
                    return Err(anyhow!("invalid unicode key payload length"));
                }
                let flags = payload[0];
                let ch = u16::from_le_bytes([payload[1], payload[2]]);
                Ok(HelperInputEvent::UnicodeKey {
                    ch,
                    released: (flags & INPUT_KEY_FLAG_RELEASE) != 0,
                })
            }
            INPUT_MSG_MOUSE_MOVE_ABS => {
                if payload.len() != 4 {
                    return Err(anyhow!("invalid mouse move payload length"));
                }
                let x = u16::from_le_bytes([payload[0], payload[1]]);
                let y = u16::from_le_bytes([payload[2], payload[3]]);
                Ok(HelperInputEvent::MouseMoveAbs { x, y })
            }
            INPUT_MSG_MOUSE_MOVE_REL => {
                if payload.len() != 8 {
                    return Err(anyhow!("invalid relative mouse move payload length"));
                }
                let dx = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let dy = i32::from_le_bytes([payload[4], payload[5], payload[6], payload[7]]);
                Ok(HelperInputEvent::MouseMoveRel { dx, dy })
            }
            INPUT_MSG_MOUSE_BUTTON => {
                if payload.len() != 2 {
                    return Err(anyhow!("invalid mouse button payload length"));
                }
                let button = payload[0];
                let down = payload[1] == INPUT_MOUSE_BUTTON_DOWN;
                Ok(HelperInputEvent::MouseButton { button, down })
            }
            INPUT_MSG_MOUSE_WHEEL => {
                if payload.len() != 4 {
                    return Err(anyhow!("invalid mouse wheel payload length"));
                }
                let delta = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(HelperInputEvent::MouseWheel { delta })
            }
            INPUT_MSG_MOUSE_HWHEEL => {
                if payload.len() != 4 {
                    return Err(anyhow!("invalid mouse hwheel payload length"));
                }
                let delta = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(HelperInputEvent::MouseHWheel { delta })
            }
            _ => Err(anyhow!("unknown input message type: {msg_type}")),
        }
    }

    fn inject_helper_input_event(event: HelperInputEvent) -> anyhow::Result<()> {
        match event {
            HelperInputEvent::MouseMoveAbs { x, y } => {
                // SAFETY: SetCursorPos is safe to call.
                unsafe {
                    let _ = SetCursorPos(i32::from(x), i32::from(y));
                }
                Ok(())
            }
            HelperInputEvent::MouseMoveRel { dx, dy } => {
                let input = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx,
                            dy,
                            mouseData: 0,
                            dwFlags: MOUSEEVENTF_MOVE,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };

                // SAFETY: INPUT is fully initialized and we pass the correct struct size.
                let sent = unsafe { SendInput(&[input], i32::try_from(size_of::<INPUT>()).unwrap_or(0)) };
                if sent != 1 {
                    warn!(sent, "SendInput did not inject relative mouse move");
                }
                Ok(())
            }
            HelperInputEvent::MouseButton { button, down } => {
                let (flags, mouse_data) = match (button, down) {
                    (INPUT_MOUSE_BUTTON_LEFT, true) => (MOUSEEVENTF_LEFTDOWN, 0u32),
                    (INPUT_MOUSE_BUTTON_LEFT, false) => (MOUSEEVENTF_LEFTUP, 0u32),
                    (INPUT_MOUSE_BUTTON_RIGHT, true) => (MOUSEEVENTF_RIGHTDOWN, 0u32),
                    (INPUT_MOUSE_BUTTON_RIGHT, false) => (MOUSEEVENTF_RIGHTUP, 0u32),
                    (INPUT_MOUSE_BUTTON_MIDDLE, true) => (MOUSEEVENTF_MIDDLEDOWN, 0u32),
                    (INPUT_MOUSE_BUTTON_MIDDLE, false) => (MOUSEEVENTF_MIDDLEUP, 0u32),
                    (INPUT_MOUSE_BUTTON_X1, true) => (MOUSEEVENTF_XDOWN, 1u32),
                    (INPUT_MOUSE_BUTTON_X1, false) => (MOUSEEVENTF_XUP, 1u32),
                    (INPUT_MOUSE_BUTTON_X2, true) => (MOUSEEVENTF_XDOWN, 2u32),
                    (INPUT_MOUSE_BUTTON_X2, false) => (MOUSEEVENTF_XUP, 2u32),
                    _ => return Ok(()),
                };

                let input = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: 0,
                            dy: 0,
                            mouseData: mouse_data,
                            dwFlags: flags,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };

                // SAFETY: INPUT is fully initialized and we pass the correct struct size.
                let sent = unsafe { SendInput(&[input], i32::try_from(size_of::<INPUT>()).unwrap_or(0)) };
                if sent != 1 {
                    warn!(sent, "SendInput did not inject mouse button event");
                }
                Ok(())
            }
            HelperInputEvent::MouseWheel { delta } => {
                let input = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: 0,
                            dy: 0,
                            mouseData: u32::from_le_bytes(delta.to_le_bytes()),
                            dwFlags: MOUSEEVENTF_WHEEL,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };

                // SAFETY: INPUT is fully initialized and we pass the correct struct size.
                let sent = unsafe { SendInput(&[input], i32::try_from(size_of::<INPUT>()).unwrap_or(0)) };
                if sent != 1 {
                    warn!(sent, "SendInput did not inject mouse wheel event");
                }
                Ok(())
            }
            HelperInputEvent::MouseHWheel { delta } => {
                let input = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: 0,
                            dy: 0,
                            mouseData: u32::from_le_bytes(delta.to_le_bytes()),
                            dwFlags: MOUSEEVENTF_HWHEEL,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };

                // SAFETY: INPUT is fully initialized and we pass the correct struct size.
                let sent = unsafe { SendInput(&[input], i32::try_from(size_of::<INPUT>()).unwrap_or(0)) };
                if sent != 1 {
                    warn!(sent, "SendInput did not inject horizontal mouse wheel event");
                }
                Ok(())
            }
            HelperInputEvent::ScancodeKey {
                code,
                extended,
                released,
            } => {
                let mut flags = KEYEVENTF_SCANCODE;
                if extended {
                    flags |= KEYEVENTF_EXTENDEDKEY;
                }
                if released {
                    flags |= KEYEVENTF_KEYUP;
                }

                let input = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0),
                            wScan: u16::from(code),
                            dwFlags: flags,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };

                // SAFETY: INPUT is fully initialized and we pass the correct struct size.
                let sent = unsafe { SendInput(&[input], i32::try_from(size_of::<INPUT>()).unwrap_or(0)) };
                if sent != 1 {
                    warn!(sent, "SendInput did not inject scancode keyboard event");
                }
                Ok(())
            }
            HelperInputEvent::UnicodeKey { ch, released } => {
                let mut flags = KEYEVENTF_UNICODE;
                if released {
                    flags |= KEYEVENTF_KEYUP;
                }

                let input = INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(0),
                            wScan: ch,
                            dwFlags: flags,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                };

                // SAFETY: INPUT is fully initialized and we pass the correct struct size.
                let sent = unsafe { SendInput(&[input], i32::try_from(size_of::<INPUT>()).unwrap_or(0)) };
                if sent != 1 {
                    warn!(sent, "SendInput did not inject unicode keyboard event");
                }
                Ok(())
            }
        }
    }

    async fn run_input_injector(mut stream: TcpStream) -> anyhow::Result<()> {
        // Track modifier state so we can translate Ctrl+Alt+End into a real SAS.
        // This is necessary because Winlogon often ignores normal injected key sequences.
        let mut ctrl_down = false;
        let mut alt_down = false;

        loop {
            let event = read_helper_input_event(&mut stream).await?;

            if let HelperInputEvent::ScancodeKey { code, released, .. } = event {
                // Set-1 scancodes used by mstsc for modifiers.
                const SCANCODE_CTRL: u8 = 0x1D;
                const SCANCODE_ALT: u8 = 0x38;
                const SCANCODE_END: u8 = 0x4F;
                const SCANCODE_DEL: u8 = 0x53;

                match code {
                    SCANCODE_CTRL => ctrl_down = !released,
                    SCANCODE_ALT => alt_down = !released,
                    SCANCODE_END | SCANCODE_DEL if !released && ctrl_down && alt_down => {
                        if try_send_sas("input_ctrl_alt_end") {
                            continue;
                        }
                    }
                    _ => {}
                }
            }

            if let Err(error) = inject_helper_input_event(event) {
                warn!(error = %format!("{error:#}"), "Failed to inject input event");
            }
        }
    }

    fn auto_send_sas_enabled() -> bool {
        let configured = std::env::var(AUTO_SEND_SAS_ENV)
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .unwrap_or_default();

        matches!(configured.as_str(), "1" | "true" | "yes" | "on")
    }

    fn try_send_sas(source: &'static str) -> bool {
        let ok = unsafe { SendSAS(0) };
        if ok != 0 {
            info!(source = %source, "Generated Secure Attention Sequence (SAS)");
            true
        } else {
            let err = unsafe { GetLastError() };
            warn!(error = ?err, source = %source, "SendSAS failed");
            false
        }
    }

    async fn run_capture_helper_tcp(
        connect_addr: SocketAddr,
        input_connect_addr: SocketAddr,
        rfx_encode: bool,
    ) -> anyhow::Result<()> {
        info!(
            pid = std::process::id(),
            session_id = unsafe { windows::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId() },
            connect_addr = %connect_addr,
            input_connect_addr = %input_connect_addr,
            "Capture helper TCP: starting connection"
        );

        let mut stream = match TcpStream::connect(connect_addr).await {
            Ok(stream) => {
                info!(connect_addr = %connect_addr, "Capture helper: connected to capture consumer");
                stream
            }
            Err(error) => {
                error!(
                    connect_addr = %connect_addr,
                    error = %error,
                    "Capture helper: failed to connect to capture consumer"
                );
                return Err(error).with_context(|| format!("failed to connect to capture consumer at {connect_addr}"));
            }
        };

        let input_stream = match TcpStream::connect(input_connect_addr).await {
            Ok(stream) => {
                info!(input_connect_addr = %input_connect_addr, "Capture helper: connected to input consumer");
                stream
            }
            Err(error) => {
                error!(
                    input_connect_addr = %input_connect_addr,
                    error = %error,
                    "Capture helper: failed to connect to input consumer"
                );
                return Err(error).with_context(|| format!("failed to connect to input consumer at {input_connect_addr}"));
            }
        };

        tokio::spawn(async move {
            if let Err(error) = run_input_injector(input_stream).await {
                warn!(error = %format!("{error:#}"), "Input injector task stopped");
            }
        });

        if auto_send_sas_enabled() {
            for attempt in 1..=5 {
                if try_send_sas("capture_helper_startup") {
                    break;
                }

                if attempt < 5 {
                    sleep(Duration::from_millis(500)).await;
                }
            }
        }

        let desktop_size = desktop_size_from_gdi().context("failed to query desktop size")?;
        info!(
            width = desktop_size.width,
            height = desktop_size.height,
            rfx_encode,
            "Initialized capture helper desktop size"
        );

        let mut rfx_encoder = if rfx_encode {
            use ironrdp_pdu::rdp::capability_sets::EntropyBits;
            Some(ironrdp_server::encoder::rfx::RfxEncoder::new(EntropyBits::Rlgr3))
        } else {
            None
        };

        let rfx_codec_id: u8 = 3; // CODEC_ID_REMOTEFX
        let mut rfx_first_frame = true;

        loop {
            match capture_bitmap_update(desktop_size) {
                Ok(bitmap) => {
                    if let Some(encoder) = rfx_encoder.as_mut() {
                        if is_probably_blank_bgra32(bitmap.data.as_ref()) {
                            write_capture_frame(&mut stream, &bitmap).await?;
                        } else {
                            let ds = if rfx_first_frame {
                                rfx_first_frame = false;
                                Some(desktop_size)
                            } else {
                                None
                            };

                            let mut buf = vec![0u8; bitmap.data.len()];
                            let encoded_len = loop {
                                match encoder.encode(&bitmap, &mut buf, ds) {
                                    Ok(len) => break len,
                                    Err(e) => match e.kind() {
                                        ironrdp_core::EncodeErrorKind::NotEnoughBytes { .. } => {
                                            buf.resize(buf.len() * 2, 0);
                                        }
                                        _ => return Err(anyhow::anyhow!("RemoteFX encode error: {e}")),
                                    },
                                }
                            };

                            write_capture_frame_preencoded(
                                &mut stream,
                                desktop_size.width,
                                desktop_size.height,
                                rfx_codec_id,
                                &buf[..encoded_len],
                            )
                            .await?;
                        }
                    } else {
                        write_capture_frame(&mut stream, &bitmap).await?;
                    }
                    sleep(CAPTURE_INTERVAL).await;
                }
                Err(error) => {
                    warn!(
                        error = %format!("{error:#}"),
                        "Capture helper failed to capture frame"
                    );
                    let bitmap = fallback_bitmap_update(desktop_size)
                        .context("failed to generate fallback bitmap in capture helper")?;
                    maybe_dump_bitmap_update_bgra32(bitmap.width, bitmap.height, bitmap.stride, bitmap.data.as_ref());
                    write_capture_frame(&mut stream, &bitmap).await?;
                    sleep(CAPTURE_INTERVAL).await;
                }
            }
        }
    }

    async fn run_capture_helper_shared_mem(
        map_name: &str,
        event_name: &str,
        input_connect_addr: SocketAddr,
    ) -> anyhow::Result<()> {
        let map_name_w: Vec<u16> = map_name.encode_utf16().chain(Some(0)).collect();
        let event_name_w: Vec<u16> = event_name.encode_utf16().chain(Some(0)).collect();

        // SAFETY: opens the named mapping created by the service.
        let mapping = unsafe { OpenFileMappingW(FILE_MAP_WRITE.0, false, PCWSTR(map_name_w.as_ptr())) }
            .map_err(|error| anyhow!("OpenFileMappingW failed: {error}"))
            .context("OpenFileMappingW failed")?;

        struct HandleGuard(HANDLE);
        impl Drop for HandleGuard {
            fn drop(&mut self) {
                // SAFETY: handle is owned by this guard.
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(self.0);
                }
            }
        }

        let _mapping_guard = HandleGuard(mapping);

        // SAFETY: map the whole view; size 0 maps the entire mapping.
        let view_ptr = unsafe { MapViewOfFile(mapping, FILE_MAP_WRITE, 0, 0, 0) };
        if view_ptr.Value.is_null() {
            return Err(anyhow!("MapViewOfFile returned null"));
        }

        let view = view_ptr.Value.cast::<u8>();

        // SAFETY: opens the named event created by the service.
        let frame_ready_event = unsafe { OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(event_name_w.as_ptr())) }
            .map_err(|error| anyhow!("OpenEventW failed: {error}"))
            .context("OpenEventW failed")?;

        let _event_guard = HandleGuard(frame_ready_event);

        let input_stream = TcpStream::connect(input_connect_addr)
            .await
            .with_context(|| format!("failed to connect to input consumer at {input_connect_addr}"))?;

        tokio::spawn(async move {
            if let Err(error) = run_input_injector(input_stream).await {
                warn!(error = %format!("{error:#}"), "Input injector task stopped");
            }
        });

        if auto_send_sas_enabled() {
            for attempt in 1..=5 {
                if try_send_sas("capture_helper_startup") {
                    break;
                }

                if attempt < 5 {
                    sleep(Duration::from_millis(500)).await;
                }
            }
        }

        // SAFETY: mapping has at least the header.
        let (width, height, _stride, slot_len) = unsafe { shm_read_layout(view, SHM_FB_HEADER_LEN)? };
        let view_len = SHM_FB_HEADER_LEN
            .checked_add(
                slot_len
                    .checked_mul(SHM_FB_SLOTS)
                    .ok_or_else(|| anyhow!("frame buffer length overflow"))?,
            )
            .ok_or_else(|| anyhow!("frame buffer length overflow"))?;
        let desktop_size = DesktopSize {
            width: width.get(),
            height: height.get(),
        };
        info!(
            width = desktop_size.width,
            height = desktop_size.height,
            "Initialized shared-memory capture helper desktop size"
        );

        let mut seq: u64 = 0;
        let mut slot_idx: usize = 0;

        loop {
            match capture_bitmap_update(desktop_size) {
                Ok(bitmap) => {
                    seq = seq.wrapping_add(1);
                    slot_idx = (slot_idx + 1) % SHM_FB_SLOTS;

                    // SAFETY: view pointer is valid; slot_len and slot_idx are checked by shm_publish_frame.
                    unsafe {
                        shm_publish_frame(view, view_len, slot_idx, slot_len, seq, bitmap.data.as_ref())?;
                    }

                    // SAFETY: event handle is valid.
                    unsafe { SetEvent(frame_ready_event) }
                        .map_err(|error| anyhow!("SetEvent failed: {error}"))
                        .context("SetEvent failed")?;

                    // Important: yield/throttle so the input injector task can run.
                    sleep(CAPTURE_INTERVAL).await;
                }
                Err(error) => {
                    warn!(
                        error = %format!("{error:#}"),
                        "Capture helper failed to capture frame"
                    );
                    let bitmap = fallback_bitmap_update(desktop_size)
                        .context("failed to generate fallback bitmap in shared-memory capture helper")?;
                    maybe_dump_bitmap_update_bgra32(bitmap.width, bitmap.height, bitmap.stride, bitmap.data.as_ref());

                    seq = seq.wrapping_add(1);
                    slot_idx = (slot_idx + 1) % SHM_FB_SLOTS;

                    unsafe {
                        shm_publish_frame(view, view_len, slot_idx, slot_len, seq, bitmap.data.as_ref())?;
                    }

                    unsafe { SetEvent(frame_ready_event) }
                        .map_err(|set_event_error| anyhow!("SetEvent failed: {set_event_error}"))
                        .context("SetEvent failed")?;
                    sleep(CAPTURE_INTERVAL).await;
                }
            }
        }
    }

    #[expect(clippy::infinite_loop, reason = "pipe server instances run indefinitely")]
    async fn run_control_pipe_instance_loop(
        full_pipe_name: &str,
        control_plane: ControlPlane,
        empty_disconnects: Arc<AtomicU64>,
    ) {
        loop {
            let mut opts = named_pipe::ServerOptions::new();
            opts.access_inbound(true)
                .access_outbound(true)
                .in_buffer_size(PIPE_BUFFER_SIZE)
                .out_buffer_size(PIPE_BUFFER_SIZE)
                .pipe_mode(named_pipe::PipeMode::Byte);

            let (mut attrs, sd) = match control_pipe_security_attributes() {
                Ok(values) => values,
                Err(error) => {
                    warn!(%error, "Failed to build control pipe security attributes; retrying");
                    sleep(Duration::from_millis(200)).await;
                    continue;
                }
            };

            // Create the pipe with an explicit DACL so TermService can open it.
            // SAFETY: attrs is a valid SECURITY_ATTRIBUTES for the duration of the call.
            let mut server = match unsafe {
                opts.create_with_security_attributes_raw(
                    full_pipe_name,
                    core::ptr::from_mut(&mut attrs).cast::<c_void>(),
                )
            } {
                Ok(server) => server,
                Err(error) => {
                    // SAFETY: frees the buffer allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
                    unsafe {
                        let _ = LocalFree(Some(HLOCAL(sd.0)));
                    }
                    warn!(%error, pipe = %full_pipe_name, "Failed to create control pipe server instance; retrying");
                    sleep(Duration::from_millis(200)).await;
                    continue;
                }
            };

            // SAFETY: frees the buffer allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
            unsafe {
                let _ = LocalFree(Some(HLOCAL(sd.0)));
            }

            if let Err(error) = server.connect().await {
                warn!(%error, pipe = %full_pipe_name, "Failed to accept control connection; retrying");
                sleep(Duration::from_millis(200)).await;
                continue;
            }

            let mut pending_wakeup_rx = control_plane.pending_wakeup_tx.subscribe();

            match handle_client(&mut server, &control_plane, &mut pending_wakeup_rx).await {
                Ok(0) => {
                    let n = empty_disconnects.fetch_add(1, Ordering::Relaxed).saturating_add(1);
                    if n == 1 || (n <= 10 && n % 5 == 0) || n % 100_000 == 0 {
                        info!(empty_disconnects = n, "Client disconnected without sending commands");
                    }
                }
                Ok(_) => {
                    empty_disconnects.store(0, Ordering::Relaxed);
                }
                Err(error) => {
                    warn!(%error, pipe = %full_pipe_name, "Control connection handler returned error");
                    empty_disconnects.store(0, Ordering::Relaxed);
                }
            }
        }
    }

    async fn handle_client(
        pipe: &mut named_pipe::NamedPipeServer,
        control_plane: &ControlPlane,
        pending_wakeup_rx: &mut watch::Receiver<u64>,
    ) -> anyhow::Result<u32> {
        let mut commands_processed: u32 = 0;

        loop {
            let command = match timeout(CONTROL_PIPE_IDLE_TIMEOUT, read_command(pipe)).await {
                Ok(Ok(Some(command))) => command,
                Ok(Ok(None)) => return Ok(commands_processed),
                Ok(Err(error)) => return Err(error).context("failed to read provider command"),
                Err(_) => {
                    debug!(
                        commands_processed,
                        "Control pipe client idle; closing connection to free pipe instance"
                    );
                    return Ok(commands_processed);
                }
            };

            commands_processed += 1;
            info!(command = ?command, seq = commands_processed, "Processing pipe command");

            let event = control_plane.handle_command(command, pending_wakeup_rx).await;

            info!(
                event_type = event_kind(&event),
                seq = commands_processed,
                "Sending pipe response"
            );
            write_event(pipe, &event).await?;
        }
    }

    async fn read_command(pipe: &mut named_pipe::NamedPipeServer) -> io::Result<Option<ProviderCommand>> {
        let payload = match read_frame(pipe).await {
            Ok(payload) => payload,
            Err(error) if is_disconnect_error(&error) => return Ok(None),
            Err(error) => return Err(error),
        };

        let command = serde_json::from_slice(&payload)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("failed to parse command: {error}")))?;

        Ok(Some(command))
    }

    async fn write_event(pipe: &mut named_pipe::NamedPipeServer, event: &ServiceEvent) -> io::Result<()> {
        let payload = serde_json::to_vec(event).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("failed to serialize event: {error}"),
            )
        })?;

        write_frame(pipe, &payload).await
    }

    async fn read_frame(pipe: &mut named_pipe::NamedPipeServer) -> io::Result<Vec<u8>> {
        let mut len_buf = [0u8; 4];
        pipe.read_exact(&mut len_buf).await?;

        let frame_len_u32 = u32::from_le_bytes(len_buf);
        let frame_len = usize::try_from(frame_len_u32)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame length does not fit in usize"))?;

        if frame_len > DEFAULT_MAX_FRAME_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame length exceeds maximum size",
            ));
        }

        let mut payload = vec![0u8; frame_len];
        pipe.read_exact(&mut payload).await?;

        Ok(payload)
    }

    async fn write_frame(pipe: &mut named_pipe::NamedPipeServer, payload: &[u8]) -> io::Result<()> {
        let payload_len = u32::try_from(payload.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;

        pipe.write_all(&payload_len.to_le_bytes()).await?;
        pipe.write_all(payload).await
    }

    fn event_kind(event: &ServiceEvent) -> &'static str {
        match event {
            ServiceEvent::Ack => "ack",
            ServiceEvent::ListenerStarted { .. } => "listener_started",
            ServiceEvent::ListenerStopped { .. } => "listener_stopped",
            ServiceEvent::IncomingConnection { .. } => "incoming_connection",
            ServiceEvent::NoIncoming => "no_incoming",
            ServiceEvent::ConnectionReady { .. } => "connection_ready",
            ServiceEvent::ConnectionBroken { .. } => "connection_broken",
            ServiceEvent::ConnectionCredentials { .. } => "connection_credentials",
            ServiceEvent::NoCredentials { .. } => "no_credentials",
            ServiceEvent::Error { .. } => "error",
        }
    }

    fn is_disconnect_error(error: &io::Error) -> bool {
        matches!(error.kind(), io::ErrorKind::BrokenPipe | io::ErrorKind::UnexpectedEof)
    }

    #[tokio::main(flavor = "current_thread")]
    async fn main_impl() {
        let local_set = tokio::task::LocalSet::new();

        local_set
            .run_until(async {
                if let Err(error) = run().await {
                    error!(%error, "TermSrv service failed");
                }
            })
            .await;
    }

    pub(crate) fn main() {
        // Ultra-early diagnostic for capture helper debugging.
        // When spawned via CreateProcessAsUserW with CREATE_NO_WINDOW, the process
        // may crash silently before Tokio initializes. Write a breadcrumb file before
        // anything else so we know the process at least started.
        let args: Vec<String> = std::env::args().collect();
        if args.iter().any(|a| a == "--capture-helper") {
            let pid = std::process::id();
            let diag = format!(
                "Capture helper process started\nPID: {pid}\nArgs: {args:?}\nTime: {:?}\n",
                std::time::SystemTime::now()
            );
            let _ = std::fs::create_dir_all("C:\\IronRDPDeploy\\logs");
            let _ = std::fs::write(
                format!("C:\\IronRDPDeploy\\logs\\capture-helper-early-{pid}.txt"),
                &diag,
            );
        }

        main_impl();
    }
}

#[cfg(windows)]
fn main() {
    windows_main::main();
}
