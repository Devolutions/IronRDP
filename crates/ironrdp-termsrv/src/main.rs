// Use the `windows` PE subsystem so that this binary can be spawned via
// CreateProcessAsUserW into another session without STATUS_DLL_INIT_FAILED
// (0xC0000142). Console-subsystem binaries require CSRSS interaction during
// DLL initialization, which fails when the target session's CSRSS is not
// reachable (e.g. winlogon desktop of a partially-initialized session).
// The binary runs as a service/scheduled task with redirected stdout/stderr,
// so the lack of an auto-allocated console is harmless.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod clipboard;

#[cfg(windows)]
mod cursor;

#[cfg(windows)]
mod pointer;

#[cfg(windows)]
mod native;

#[cfg(windows)]
mod native_pipe;

#[cfg(not(windows))]
fn main() {
    eprintln!("ironrdp-termsrv is only supported on windows");
}

#[cfg(windows)]
mod windows_main {
    use core::ffi::c_void;
    use core::net::{Ipv4Addr, SocketAddr};
    use core::num::{NonZeroI32, NonZeroU16, NonZeroU32, NonZeroUsize};
    use core::ptr::null_mut;
    use core::sync::atomic::{AtomicBool, AtomicU64, Ordering, fence};
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::io;
    use std::io::Write as _;
    use std::os::windows::io::AsRawHandle as _;
    use std::sync::{Arc, Mutex as StdMutex, OnceLock};
    use std::time::Instant;

    use anyhow::{Context as _, anyhow};
    use ironrdp_displaycontrol::pdu::DisplayControlMonitorLayout;
    use ironrdp_server::tokio_rustls::{TlsAcceptor, rustls};
    use ironrdp_server::{
        BitmapUpdate, Credentials, CsReadyPdu, DesktopSize, DismissHoveringTouchContactPdu, DisplayContext,
        DisplayUpdate, KeyboardEvent, MouseButton, MouseEvent, PenEventPdu, PixelFormat, RdpInputProtocolVersion,
        RdpServer, RdpServerDisplay, RdpServerDisplayUpdates, RdpServerInputHandler, RdpeiHandler, RdpeiServer,
        RdpeiServerFactory, ScReadyFeatures, ServerError, ServerErrorExt as _, ServerEvent, ServerEventSender,
        ServerResult, TouchEventPdu,
    };
    use ironrdp_wtsprotocol_ipc::display::{DISPLAY_REQUEST_DIRECTORY, DisplaySize};
    use ironrdp_wtsprotocol_ipc::{
        DEFAULT_MAX_FRAME_SIZE, ProviderCommand, ServiceEvent, default_pipe_name, pipe_path, resolve_pipe_name_from_env,
    };
    use rustls_cng::signer::CngSigningKey;
    use rustls_cng::store::{CertStore, CertStoreType};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::windows::named_pipe;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::{Mutex, Notify, Semaphore, mpsc, watch};
    use tokio::task::JoinHandle;
    use tokio::time::{Duration, sleep, timeout};
    use tracing::{debug, error, info, warn};
    use tracing_subscriber::EnvFilter;
    use windows::Win32::Foundation::{
        CloseHandle, ERROR_BAD_LENGTH, ERROR_NO_MORE_FILES, ERROR_NOT_ALL_ASSIGNED, GetLastError, HANDLE, HLOCAL, LUID,
        LocalFree, SetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT, WIN32_ERROR,
    };
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GetDC, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows::Win32::Security::Cryptography::{
        BCRYPT_RSA_ALGORITHM, BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom, CERT_CONTEXT,
        CERT_CREATE_SELFSIGN_FLAGS, CERT_FIND_SUBJECT_STR_W, CERT_NCRYPT_KEY_SPEC, CERT_OPEN_STORE_FLAGS,
        CERT_QUERY_ENCODING_TYPE, CERT_STORE_ADD_REPLACE_EXISTING, CERT_STORE_PROV_SYSTEM_W,
        CERT_SYSTEM_STORE_LOCAL_MACHINE, CERT_X500_NAME_STR, CRYPT_INTEGER_BLOB, CRYPT_KEY_PROV_INFO,
        CertAddCertificateContextToStore, CertCloseStore, CertCreateSelfSignCertificate, CertFindCertificateInStore,
        CertFreeCertificateContext, CertOpenStore, CertStrToNameW, HCERTSTORE, MS_KEY_STORAGE_PROVIDER,
        NCRYPT_ALLOW_EXPORT_FLAG, NCRYPT_ALLOW_PLAINTEXT_EXPORT_FLAG, NCRYPT_EXPORT_POLICY_PROPERTY, NCRYPT_FLAGS,
        NCRYPT_HANDLE, NCRYPT_KEY_HANDLE, NCRYPT_LENGTH_PROPERTY, NCRYPT_MACHINE_KEY_FLAG, NCRYPT_PROV_HANDLE,
        NCryptCreatePersistedKey, NCryptFinalizeKey, NCryptFreeObject, NCryptOpenStorageProvider, NCryptSetProperty,
        PKCS_7_ASN_ENCODING, X509_ASN_ENCODING,
    };
    use windows::Win32::Security::{
        AdjustTokenPrivileges, DuplicateTokenEx, GetTokenInformation, ImpersonateLoggedOnUser, LUID_AND_ATTRIBUTES,
        LookupPrivilegeValueW, PSECURITY_DESCRIPTOR, RevertToSelf, SE_PRIVILEGE_ENABLED, SECURITY_ATTRIBUTES,
        SecurityImpersonation, SetTokenInformation, TOKEN_ADJUST_PRIVILEGES, TOKEN_ADJUST_SESSIONID,
        TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_PRIVILEGES, TOKEN_QUERY, TOKEN_USER, TokenPrimary, TokenSessionId,
        TokenUser,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Memory::{
        CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MapViewOfFile, OpenFileMappingW, PAGE_READWRITE,
        UnmapViewOfFile,
    };
    use windows::Win32::System::Pipes::{GetNamedPipeClientProcessId, ImpersonateNamedPipeClient};
    use windows::Win32::System::RemoteDesktop::{
        WTS_PROCESS_INFOW, WTS_SESSION_INFOW, WTSEnumerateProcessesW, WTSEnumerateSessionsW, WTSFreeMemory,
        WTSGetActiveConsoleSessionId, WTSQueryUserToken,
    };
    use windows::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateEventW, CreateProcessAsUserW, EVENT_MODIFY_STATE,
        GetCurrentProcess, GetCurrentProcessId, GetCurrentThread, OpenEventW, OpenProcess, OpenProcessToken,
        OpenThreadToken, PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW, SetEvent,
        TerminateProcess, WaitForSingleObject,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
        KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN,
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
        MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput, VIRTUAL_KEY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    use windows::core::{BOOL, PCWSTR, PWSTR, w};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryA(lp_lib_file_name: *const u8) -> *mut c_void;
        fn GetProcAddress(h_module: *mut c_void, lp_proc_name: *const u8) -> *mut c_void;
        fn FreeLibrary(h_lib_module: *mut c_void) -> i32;
        fn GetCurrentThreadId() -> u32;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenInputDesktop(dw_flags: u32, f_inherit: i32, dw_desired_access: u32) -> *mut c_void;
        fn GetThreadDesktop(dw_thread_id: u32) -> *mut c_void;
        fn SetThreadDesktop(h_desktop: *mut c_void) -> i32;
        fn CloseDesktop(h_desktop: *mut c_void) -> i32;
        fn GetUserObjectInformationW(
            h_obj: *mut c_void,
            n_index: i32,
            pv_info: *mut c_void,
            n_length: u32,
            lpn_length_needed: *mut u32,
        ) -> i32;
        fn SystemParametersInfoW(ui_action: u32, ui_param: u32, pv_param: *mut c_void, f_win_ini: u32) -> i32;
        fn SetCaretBlinkTime(u_mseconds: u32) -> i32;
        fn LoadKeyboardLayoutW(pwsz_klid: *const u16, flags: u32) -> *mut c_void;
        fn ActivateKeyboardLayout(hkl: *mut c_void, flags: u32) -> *mut c_void;
        fn SendNotifyMessageW(hwnd: usize, msg: u32, wparam: usize, lparam: *mut c_void) -> i32;
    }

    const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
    const CONTROL_PIPE_SERVER_INSTANCES: usize = 64;
    const CONTROL_PIPE_IDLE_TIMEOUT: Duration = Duration::from_secs(2);
    const MAX_CONCURRENT_CONNECTIONS: usize = 64;
    const MAX_PENDING_NOTIFICATIONS: usize = MAX_CONCURRENT_CONNECTIONS * 2;
    const CONNECTION_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
    const LISTEN_ADDR_ENV: &str = "IRONRDP_WTS_LISTEN_ADDR";
    const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:4489";
    const CAPTURE_INTERVAL: Duration = Duration::from_millis(16);
    const LOGON_READINESS_PROBE_INTERVAL: Duration = Duration::from_secs(2);
    const WAITING_FOR_USER_LOGON_DIAGNOSTIC_INTERVAL: Duration = Duration::from_secs(5);
    const AUTO_SEND_SAS_RETRY_INTERVAL: Duration = Duration::from_secs(3);
    const FIRST_FRAME_BLANK_GRACE: Duration = Duration::from_secs(1);
    const FIRST_FRAME_BLANK_MAX_FRAMES: u32 = 8;
    const PERSISTENT_BLANK_RESTART_GRACE: Duration = Duration::from_secs(3);
    const PERSISTENT_BLANK_RESTART_MIN_FRAMES: u32 = 20;
    const CAPTURE_HELPER_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
    const CAPTURE_HELPER_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
    const CAPTURE_HELPER_TOKEN_ENV: &str = "IRONRDP_CAPTURE_HELPER_TOKEN";
    const CAPTURE_HELPER_TOKEN_LEN: usize = 64;
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

    pub(super) fn security_attributes(sddl: PCWSTR) -> anyhow::Result<(SECURITY_ATTRIBUTES, PSECURITY_DESCRIPTOR)> {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        let mut sd_len = 0u32;

        // SAFETY: callers keep the NUL-terminated SDDL string alive for this call. `sd` receives a valid pointer
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

    struct PipeClientImpersonation;

    impl Drop for PipeClientImpersonation {
        fn drop(&mut self) {
            // SAFETY: this guard is created only after impersonating the current thread.
            if let Err(error) = unsafe { RevertToSelf() } {
                warn!(%error, "Failed to revert control-pipe client impersonation");
            }
        }
    }

    struct OwnedHandle(HANDLE);

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: the handle is owned exclusively by this guard.
            if let Err(error) = unsafe { CloseHandle(self.0) } {
                warn!(%error, "Failed to close owned Windows handle");
            }
        }
    }

    fn authenticate_control_pipe_client(pipe: &named_pipe::NamedPipeServer) -> anyhow::Result<()> {
        let pipe_handle = HANDLE(pipe.as_raw_handle());
        let mut client_pid = 0u32;

        // SAFETY: `pipe_handle` is a connected named-pipe server handle.
        unsafe { GetNamedPipeClientProcessId(pipe_handle, &mut client_pid) }
            .context("GetNamedPipeClientProcessId failed")?;

        // SAFETY: `pipe_handle` is a connected named-pipe server handle.
        unsafe { ImpersonateNamedPipeClient(pipe_handle) }.context("ImpersonateNamedPipeClient failed")?;
        let _impersonation = PipeClientImpersonation;

        let mut token = HANDLE::default();
        // SAFETY: GetCurrentThread returns a pseudo-handle that must not be closed.
        let current_thread = unsafe { GetCurrentThread() };
        // SAFETY: the current thread is impersonating the connected client and `token` is a valid out-pointer.
        unsafe { OpenThreadToken(current_thread, TOKEN_QUERY, true, &mut token) }
            .context("OpenThreadToken failed for control-pipe client")?;
        let token = OwnedHandle(token);

        let sid = token_user_sid(token.0).context("query control-pipe client identity")?;
        anyhow::ensure!(
            matches!(sid.as_str(), "S-1-5-18" | "S-1-5-20"),
            "unauthorized control-pipe client pid {client_pid} sid {sid}"
        );

        debug!(client_pid, client_sid = %sid, "Authenticated control-pipe client");
        Ok(())
    }

    fn token_user_sid(token: HANDLE) -> anyhow::Result<String> {
        let mut len = 0u32;
        // SAFETY: querying with no buffer retrieves the required TokenUser buffer length.
        let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut len) };
        anyhow::ensure!(len != 0, "TokenUser length is zero");

        let buffer_len = usize::try_from(len).context("TokenUser length overflow")?;
        let mut buffer = vec![0u8; buffer_len];
        // SAFETY: `buffer` has the size requested by the preceding TokenUser query.
        unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                Some(buffer.as_mut_ptr().cast::<c_void>()),
                len,
                &mut len,
            )
        }
        .context("GetTokenInformation(TokenUser) failed")?;

        // SAFETY: GetTokenInformation wrote a TOKEN_USER header into `buffer`.
        let token_user = unsafe { core::ptr::read_unaligned(buffer.as_ptr().cast::<TOKEN_USER>()) };
        let mut sid_text = PWSTR::null();
        // SAFETY: `token_user.User.Sid` points inside the live TokenUser buffer and `sid_text` is a valid out-pointer.
        unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut sid_text) }
            .context("ConvertSidToStringSidW failed")?;
        // SAFETY: `sid_text` is a NUL-terminated string allocated by ConvertSidToStringSidW.
        let sid = unsafe { sid_text.to_string() }.context("decode token user SID");
        // SAFETY: ConvertSidToStringSidW allocated `sid_text` with LocalAlloc.
        let _ = unsafe { LocalFree(Some(HLOCAL(sid_text.as_ptr().cast::<c_void>()))) };

        sid
    }

    const INPUT_FRAME_MAGIC: [u8; 4] = *b"IRIN";
    const INPUT_FRAME_VERSION: u16 = 2;
    const INPUT_FRAME_HEADER_LEN: usize = 4 /* Magic */ + 2 /* Version */ + 1 /* Type */ + 2 /* Length */;
    // A 255-event fast-path PDU can produce two helper messages per mouse event before the spooler runs.
    const INPUT_QUEUE_CAPACITY: usize = 512;

    const INPUT_MSG_SCANCODE_KEY: u8 = 1;
    const INPUT_MSG_UNICODE_KEY: u8 = 2;
    const INPUT_MSG_MOUSE_MOVE_ABS: u8 = 10;
    const INPUT_MSG_MOUSE_MOVE_REL: u8 = 11;
    const INPUT_MSG_MOUSE_BUTTON: u8 = 12;
    const INPUT_MSG_MOUSE_WHEEL: u8 = 13;
    const INPUT_MSG_MOUSE_HWHEEL: u8 = 14;
    const INPUT_MSG_RDPEI_READY: u8 = 20;
    const INPUT_MSG_TOUCH: u8 = 21;
    const INPUT_MSG_DISMISS_HOVERING: u8 = 22;
    const INPUT_MSG_PEN: u8 = 23;
    const INPUT_MSG_POINTER_RESET: u8 = 24;
    const INPUT_DESKTOP_READY: u8 = 1;

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
        let payload_len = u16::try_from(payload.len()).ok()?;
        let mut bytes = Vec::with_capacity(INPUT_FRAME_HEADER_LEN + payload.len());
        bytes.extend_from_slice(&INPUT_FRAME_MAGIC);
        bytes.extend_from_slice(&INPUT_FRAME_VERSION.to_le_bytes());
        bytes.push(msg_type);
        bytes.extend_from_slice(&payload_len.to_le_bytes());
        bytes.extend_from_slice(payload);
        Some(InputPacket { bytes })
    }

    #[derive(Clone)]
    struct TermSrvInputHandler {
        connection_id: u32,
        tx: mpsc::Sender<InputPacket>,
        server_events: Arc<OnceLock<mpsc::UnboundedSender<ServerEvent>>>,
        failed: Arc<AtomicBool>,
    }

    impl TermSrvInputHandler {
        fn new(connection_id: u32, tx: mpsc::Sender<InputPacket>) -> Self {
            Self {
                connection_id,
                tx,
                server_events: Arc::new(OnceLock::new()),
                failed: Arc::new(AtomicBool::new(false)),
            }
        }

        fn fail(&self, reason: &'static str) {
            if self.failed.swap(true, Ordering::Relaxed) {
                return;
            }
            warn!(
                connection_id = self.connection_id,
                error = reason,
                "Closing connection after input failure"
            );
            if let Some(sender) = self.server_events.get() {
                let _ = sender.send(ServerEvent::Quit(reason.to_owned()));
            }
        }

        fn send(&self, packet: Option<InputPacket>) {
            if self.failed.load(Ordering::Relaxed) {
                return;
            }
            let Some(packet) = packet else {
                self.fail("input packet exceeds the helper frame limit");
                return;
            };

            if self.tx.try_send(packet).is_err() {
                self.fail("input channel is full or closed");
            }
        }

        fn send_rdpei(&self, msg_type: u8, pdu: &dyn ironrdp_core::Encode) {
            if usize::from(u16::MAX) < pdu.size() {
                self.fail("RDPEI input exceeds the helper frame limit");
                return;
            }
            match ironrdp_core::encode_vec(pdu) {
                Ok(payload) => self.send(make_input_packet(msg_type, &payload)),
                Err(error) => {
                    warn!(%error, "Failed to encode RDPEI helper message");
                    self.fail("failed to encode RDPEI input");
                }
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

        fn send_mouse_button(&self, button: MouseButton, down: bool) {
            let button = match button {
                MouseButton::Left => INPUT_MOUSE_BUTTON_LEFT,
                MouseButton::Right => INPUT_MOUSE_BUTTON_RIGHT,
                MouseButton::Middle => INPUT_MOUSE_BUTTON_MIDDLE,
                MouseButton::X1 => INPUT_MOUSE_BUTTON_X1,
                MouseButton::X2 => INPUT_MOUSE_BUTTON_X2,
                _ => {
                    warn!(?button, "Unsupported mouse button");
                    return;
                }
            };
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
                MouseEvent::Button { x, y, button, pressed } => {
                    self.send_mouse_move_abs(x, y);
                    self.send_mouse_button(button, pressed);
                }
                MouseEvent::ButtonRel { x, y, button, pressed } => {
                    self.send_mouse_move_rel(x, y);
                    self.send_mouse_button(button, pressed);
                }
                MouseEvent::VerticalScroll { value } => {
                    let delta = i32::from(value).saturating_mul(WHEEL_DELTA_I32);
                    self.send_mouse_wheel(delta);
                }
                MouseEvent::HorizontalScroll { value } => {
                    let delta = i32::from(value).saturating_mul(WHEEL_DELTA_I32);
                    self.send_mouse_hwheel(delta);
                }
                MouseEvent::Scroll { x, y } => {
                    if y != 0 {
                        self.send_mouse_wheel(y);
                    }
                    if x != 0 {
                        self.send_mouse_hwheel(x);
                    }
                }
                _ => warn!(?event, "Unsupported mouse event"),
            }
        }
    }

    impl ServerEventSender for TermSrvInputHandler {
        fn set_sender(&mut self, sender: mpsc::UnboundedSender<ServerEvent>) {
            if self.server_events.set(sender).is_err() {
                self.fail("input server event sender was already installed");
            }
        }
    }

    impl RdpeiServerFactory for TermSrvInputHandler {
        fn build_server(&self) -> RdpeiServer {
            RdpeiServer::new(Box::new(TermSrvRdpeiHandler { input: self.clone() }))
                .with_protocol_version(RdpInputProtocolVersion::V300)
                .with_supported_features(ScReadyFeatures::MULTIPEN_INJECTION_SUPPORTED)
        }
    }

    struct TermSrvRdpeiHandler {
        input: TermSrvInputHandler,
    }

    impl Drop for TermSrvRdpeiHandler {
        fn drop(&mut self) {
            self.input.send(make_input_packet(INPUT_MSG_POINTER_RESET, &[]));
        }
    }

    impl RdpeiHandler for TermSrvRdpeiHandler {
        fn cs_ready(&mut self, pdu: CsReadyPdu) {
            self.input.send_rdpei(INPUT_MSG_RDPEI_READY, &pdu);
        }

        fn touch(&mut self, pdu: TouchEventPdu) {
            self.input.send_rdpei(INPUT_MSG_TOUCH, &pdu);
        }

        fn pen(&mut self, pdu: PenEventPdu) {
            self.input.send_rdpei(INPUT_MSG_PEN, &pdu);
        }

        fn dismiss_hovering(&mut self, pdu: DismissHoveringTouchContactPdu) {
            self.input.send_rdpei(INPUT_MSG_DISMISS_HOVERING, &pdu);
        }
    }

    struct HelperInputStream {
        stream: TcpStream,
        desktop_ready: bool,
        rdpei_initialized: bool,
        cursor: crate::cursor::Receiver,
    }

    impl HelperInputStream {
        fn new(stream: TcpStream) -> Self {
            Self {
                stream,
                desktop_ready: false,
                rdpei_initialized: false,
                cursor: crate::cursor::Receiver::default(),
            }
        }

        fn poll_cursor(&mut self) -> anyhow::Result<Option<crate::cursor::Snapshot>> {
            if !self.desktop_ready {
                let mut ready = [0];
                match self.stream.try_read(&mut ready) {
                    Ok(1) if ready[0] == INPUT_DESKTOP_READY => self.desktop_ready = true,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                    Err(error) => return Err(error).context("read input desktop readiness"),
                    _ => anyhow::bail!("invalid input desktop readiness"),
                }
            }
            let mut latest = None;
            for _ in 0..16 {
                match self.cursor.poll(&self.stream)? {
                    Some(snapshot) => latest = Some(snapshot),
                    None => break,
                }
            }
            Ok(latest)
        }
    }

    #[derive(Default)]
    struct HelperInputSlot {
        stream: Mutex<Option<HelperInputStream>>,
        available: Notify,
    }

    impl HelperInputSlot {
        async fn attach(&self, stream: TcpStream) {
            *self.stream.lock().await = Some(HelperInputStream::new(stream));
            self.available.notify_one();
        }
    }

    type InputStreamSlot = Arc<HelperInputSlot>;

    async fn run_input_spooler(connection_id: u32, stream_slot: InputStreamSlot, mut rx: mpsc::Receiver<InputPacket>) {
        let mut forwarded_any = false;
        let mut rdpei_ready: Option<InputPacket> = None;

        while let Some(packet) = rx.recv().await {
            if packet.bytes[6] == INPUT_MSG_RDPEI_READY {
                rdpei_ready = Some(packet.clone());
            } else if packet.bytes[6] == INPUT_MSG_POINTER_RESET {
                rdpei_ready = None;
            }
            loop {
                let mut guard = loop {
                    let available = stream_slot.available.notified();
                    let guard = stream_slot.stream.lock().await;
                    if guard.is_some() {
                        break guard;
                    }
                    drop(guard);
                    available.await;
                };
                let stream = guard.as_mut().expect("helper stream checked while locked");
                if !stream.desktop_ready {
                    let readiness = stream.stream.read_u8().await.and_then(|ready| {
                        if ready == INPUT_DESKTOP_READY {
                            Ok(())
                        } else {
                            Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "invalid input desktop readiness",
                            ))
                        }
                    });
                    if let Err(error) = readiness {
                        warn!(connection_id, %error, "Input desktop was not ready; waiting for a replacement helper");
                        *guard = None;
                        continue;
                    }
                    stream.desktop_ready = true;
                }

                let result = async {
                    // CS_READY can precede logon or helper replacement; each new helper needs it before input.
                    if packet.bytes[6] != INPUT_MSG_RDPEI_READY && !stream.rdpei_initialized {
                        if let Some(ready) = &rdpei_ready {
                            stream.stream.write_all(&ready.bytes).await?;
                            stream.rdpei_initialized = true;
                        }
                    }
                    stream.stream.write_all(&packet.bytes).await?;
                    if packet.bytes[6] == INPUT_MSG_RDPEI_READY {
                        stream.rdpei_initialized = true;
                    } else if packet.bytes[6] == INPUT_MSG_POINTER_RESET {
                        stream.rdpei_initialized = false;
                    }
                    io::Result::Ok(())
                }
                .await;
                if let Err(error) = result {
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
                break;
            }
        }
    }

    struct DisplayState {
        size: DesktopSize,
        input_marker: usize,
        requested: Option<DisplaySize>,
        requested_at: Option<Instant>,
        submitted: Option<(u32, DisplaySize)>,
        client_preferences: Option<(u32, u32)>,
    }

    struct StandaloneConnectionObserver(Arc<StdMutex<DisplayState>>);

    impl ironrdp_server::ConnectionHandler for StandaloneConnectionObserver {
        fn on_client_session_config(
            &mut self,
            _flags: ironrdp_pdu::rdp::client_info::ClientInfoFlags,
            performance_flags: ironrdp_pdu::rdp::client_info::PerformanceFlags,
            keyboard_layout: u32,
            _keyboard_type: ironrdp_pdu::gcc::KeyboardType,
            _keyboard_subtype: u32,
            _keyboard_functional_keys_count: u32,
            _ime_file_name: &str,
            _desktop_size: DesktopSize,
        ) -> ServerResult<()> {
            if let Ok(mut state) = self.0.lock() {
                state.client_preferences = Some((performance_flags.bits(), keyboard_layout));
            }
            Ok(())
        }
    }

    struct GdiDisplay {
        connection_id: u32,
        state: Arc<StdMutex<DisplayState>>,
        input_stream_slot: InputStreamSlot,
        connection_session_ids: Arc<StdMutex<HashMap<u32, u32>>>,
        credentials_slot: Arc<Mutex<Option<StoredCredentials>>>,
        native_channels: Option<Arc<crate::native::NativeChannels>>,
    }

    impl GdiDisplay {
        fn new(
            connection_id: u32,
            input_stream_slot: InputStreamSlot,
            connection_session_ids: Arc<StdMutex<HashMap<u32, u32>>>,
            credentials_slot: Arc<Mutex<Option<StoredCredentials>>>,
            native_channels: Option<Arc<crate::native::NativeChannels>>,
        ) -> anyhow::Result<Self> {
            let desktop_size = desktop_size_from_gdi().context("failed to query desktop size")?;
            let input_marker = {
                let mut bytes = 0u32.to_le_bytes();
                // SAFETY: The system RNG fills this live byte array without an algorithm handle.
                let status = unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
                anyhow::ensure!(status.0 >= 0, "generate input marker: {status:?}");
                usize::try_from(u32::from_le_bytes(bytes) | 1)?
            };

            info!(
                width = desktop_size.width,
                height = desktop_size.height,
                "Initialized GDI display source"
            );

            Ok(Self {
                connection_id,
                state: Arc::new(StdMutex::new(DisplayState {
                    size: desktop_size,
                    input_marker,
                    requested: None,
                    requested_at: None,
                    submitted: None,
                    client_preferences: None,
                })),
                input_stream_slot,
                connection_session_ids,
                credentials_slot,
                native_channels,
            })
        }
    }

    #[async_trait::async_trait]
    impl RdpServerDisplay for GdiDisplay {
        async fn size(&mut self) -> DesktopSize {
            self.state.lock().expect("display state poisoned").size
        }

        async fn request_initial_size(&mut self, client_size: DesktopSize) -> DesktopSize {
            let mut state = self.state.lock().expect("display state poisoned");
            if self.native_channels.is_some() && state.requested.is_none() {
                match DisplaySize::new(client_size.width & !1, client_size.height) {
                    Ok(size) => {
                        let applied = DesktopSize {
                            width: size.width(),
                            height: size.height(),
                        };
                        state.requested = Some(size);
                        state.requested_at = Some(Instant::now());
                        state.size = applied;
                    }
                    Err(error) => warn!(%error, "Rejected initial desktop size"),
                }
            }
            state.size
        }

        fn request_layout(&mut self, layout: DisplayControlMonitorLayout) {
            let [monitor] = layout.monitors() else {
                warn!("Only one remote monitor is supported");
                return;
            };
            let (width, height) = monitor.dimensions();
            let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
                warn!("Desktop size exceeds the supported range");
                return;
            };
            match DisplaySize::new(width & !1, height) {
                Ok(size) => {
                    let mut state = self.state.lock().expect("display state poisoned");
                    state.requested = Some(size);
                    state.requested_at = Some(Instant::now());
                }
                Err(error) => warn!(%error, "Rejected desktop size"),
            }
        }

        async fn offers_display_control(&mut self) -> bool {
            self.native_channels.is_some()
        }

        async fn updates(&mut self, _context: DisplayContext) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
            if let Some(prefs) = self
                .native_channels
                .as_ref()
                .and_then(|native| native.remote_app_settings().ok().flatten())
                .map(|settings| (settings.performance_flags, settings.keyboard_layout))
            {
                self.state.lock().expect("display state poisoned").client_preferences = Some(prefs);
            }
            Ok(Box::new(
                GdiDisplayUpdates::new(
                    self.connection_id,
                    Arc::clone(&self.state),
                    self.state.lock().expect("display state poisoned").size,
                    Arc::clone(&self.input_stream_slot),
                    Arc::clone(&self.connection_session_ids),
                    Arc::clone(&self.credentials_slot),
                    self.native_channels.is_some(),
                )
                .map_err(|error| ServerError::io("initialize GDI display updates", io::Error::other(error)))?,
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
        display_state: Arc<StdMutex<DisplayState>>,
        desktop_size: DesktopSize,
        input_stream_slot: InputStreamSlot,
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
        last_cursor: Option<crate::cursor::Snapshot>,
        cursor_updates: VecDeque<DisplayUpdate>,
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
            display_state: Arc<StdMutex<DisplayState>>,
            size: DesktopSize,
            input_stream_slot: InputStreamSlot,
            connection_session_ids: Arc<StdMutex<HashMap<u32, u32>>>,
            credentials_slot: Arc<Mutex<Option<StoredCredentials>>>,
            provider_mode: bool,
        ) -> anyhow::Result<Self> {
            let _ = desktop_size_nonzero(size)?;

            Ok(Self {
                connection_id,
                display_state,
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
                last_cursor: None,
                cursor_updates: VecDeque::new(),
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
                capture_started_with_session_override: None,
                provider_mode,
                wait_for_session_id_until: None,
                last_capture_output_source: None,
            })
        }

        fn accept_resized_frame(&self, actual: DesktopSize) -> bool {
            let mut state = self.display_state.lock().expect("display state poisoned");
            if let Some(requested) = state.requested {
                let target = DesktopSize {
                    width: requested.width(),
                    height: requested.height(),
                };
                if actual != target
                    && state
                        .requested_at
                        .is_some_and(|at| at.elapsed() < Duration::from_secs(3))
                {
                    return false;
                }
            }
            state.size = actual;
            true
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
        async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>> {
            // We loop here so that "no first frame yet" paths (capture helper timeout/error before
            // the first frame arrives) can retry without returning Ok(None) — returning Ok(None)
            // would cause the IronRDP server to disconnect the client immediately.
            loop {
                if let Some(update) = self.cursor_updates.pop_front() {
                    return Ok(Some(update));
                }
                let cursor = {
                    let mut slot = self.input_stream_slot.stream.try_lock().ok();
                    slot.as_mut()
                        .and_then(|slot| slot.as_mut())
                        .map(HelperInputStream::poll_cursor)
                        .transpose()
                };
                match cursor {
                    Ok(Some(Some(snapshot))) => {
                        snapshot.updates(self.last_cursor.as_ref(), &mut self.cursor_updates);
                        self.last_cursor = Some(snapshot);
                        if let Some(update) = self.cursor_updates.pop_front() {
                            return Ok(Some(update));
                        }
                    }
                    Err(error) => {
                        warn!(error = %format!("{error:#}"), "Native cursor stream stopped; restarting capture helper");
                        *self.input_stream_slot.stream.lock().await = None;
                        if let Some(capture) = self.capture.take() {
                            capture.terminate();
                        }
                        self.next_helper_attempt_at = Instant::now();
                        self.last_cursor = None;
                        return Ok(Some(DisplayUpdate::HidePointer));
                    }
                    _ => {}
                }
                if self.sent_first_frame && self.capture.is_none() {
                    sleep(CAPTURE_INTERVAL).await;
                }

                let mut session_id_override = self
                    .connection_session_ids
                    .lock()
                    .ok()
                    .and_then(|guard| guard.get(&self.connection_id).copied());

                if self.provider_mode {
                    if let Some(session_id) = session_id_override {
                        let mut state = self.display_state.lock().expect("display state poisoned");
                        if let Some(size) = state.requested {
                            if state.submitted != Some((session_id, size)) {
                                size.publish(
                                    std::path::Path::new(DISPLAY_REQUEST_DIRECTORY),
                                    session_id,
                                    self.connection_id,
                                )
                                .map_err(|error| ServerError::io("publish session display size", error))?;
                                state.submitted = Some((session_id, size));
                                state.requested_at = Some(Instant::now());
                                info!(
                                    session_id,
                                    width = size.width(),
                                    height = size.height(),
                                    "Requested session display size"
                                );
                            }
                        }
                    }
                }

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
                    let (user_token_available, explorer_ready) = tokio::task::spawn_blocking(move || {
                        (
                            session_id_override.is_some_and(session_has_user_token),
                            session_id_override.is_some_and(session_has_explorer_token),
                        )
                    })
                    .await
                    .unwrap_or((false, false));
                    let interactive_shell_ready = user_token_available || explorer_ready;

                    self.logon_readiness_ready = interactive_shell_ready;
                    self.logon_readiness_session_id = session_id_override;
                    self.next_logon_readiness_probe_at = Instant::now() + LOGON_READINESS_PROBE_INTERVAL;
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
                    self.next_auto_send_sas_at = Instant::now();
                    self.capture_started_with_session_override = None;
                    self.waiting_for_user_login_since = None;
                }

                if self.capture.is_none() && Instant::now() >= self.next_helper_attempt_at {
                    // In provider mode, wait for a real interactive user token before starting the
                    // helper. Launching the first helper on a prelogon token pins capture to the
                    // wrong desktop and can fail process startup during session bring-up.
                    if let Some(session_id) = session_id_override {
                        // WTS queries are RPCs into TermService, which may itself be inside a provider
                        // callback waiting on this process's control pipe. Blocking the runtime thread
                        // here stops the pipe from being served and deadlocks both processes.
                        let (launch_user_token_available, launch_explorer_ready) =
                            tokio::task::spawn_blocking(move || {
                                (
                                    session_has_user_token(session_id),
                                    session_has_explorer_token(session_id),
                                )
                            })
                            .await
                            .unwrap_or((false, false));
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
                                    session_id, "Waiting for interactive user token before starting capture helper"
                                );
                                info!(
                                    connection_id = self.connection_id,
                                    session_id, "SESSION_PROOF_TERMSRV_WAITING_FOR_USER_LOGON"
                                );
                            }

                            if now >= self.next_waiting_for_user_login_diagnostic_at {
                                self.next_waiting_for_user_login_diagnostic_at =
                                    now + WAITING_FOR_USER_LOGON_DIAGNOSTIC_INTERVAL;

                                let snapshot = tokio::task::spawn_blocking(session_selection_snapshot)
                                    .await
                                    .unwrap_or_default();
                                info!(
                                    connection_id = self.connection_id,
                                    session_id,
                                    %snapshot,
                                    "SESSION_PROOF_TERMSRV_WAITING_FOR_USER_LOGON_STATE"
                                );
                            }

                            if auto_send_sas_enabled() && now >= self.next_auto_send_sas_at {
                                self.next_auto_send_sas_at = now + AUTO_SEND_SAS_RETRY_INTERVAL;

                                if tokio::task::spawn_blocking(move || {
                                    try_send_sas(Some(session_id), "provider_wait_for_user_logon")
                                })
                                .await
                                .unwrap_or(false)
                                {
                                    info!(
                                        connection_id = self.connection_id,
                                        session_id, "SESSION_PROOF_TERMSRV_SAS_SENT"
                                    );
                                }
                            }

                            sleep(CAPTURE_INTERVAL).await;
                            continue;
                        }
                    }

                    let captured_credentials = { self.credentials_slot.lock().await.clone() };
                    let allow_prelogon_fallback = !self.provider_mode && !self.helper_started_once;

                    let (launch_user_token_available, launch_explorer_ready) = tokio::task::spawn_blocking(move || {
                        (
                            session_id_override.is_some_and(session_has_user_token),
                            session_id_override.is_some_and(session_has_explorer_token),
                        )
                    })
                    .await
                    .unwrap_or((false, false));
                    info!(
                        connection_id = self.connection_id,
                        session_id = session_id_override.unwrap_or(0),
                        has_session_override = session_id_override.is_some(),
                        launch_user_token_available,
                        launch_explorer_ready,
                        allow_prelogon_fallback,
                        "Capture helper launch attempt"
                    );

                    let (input_marker, client_preferences) = {
                        let state = self.display_state.lock().expect("display state poisoned");
                        (state.input_marker, state.client_preferences)
                    };
                    match CaptureClient::start(
                        self.connection_id,
                        input_marker,
                        self.desktop_size,
                        Arc::clone(&self.input_stream_slot),
                        session_id_override,
                        captured_credentials,
                        allow_prelogon_fallback,
                        client_preferences,
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
                            self.capture_restarted_for_logon = launch_user_token_available || launch_explorer_ready;
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
                                    Ok(bitmap) => (
                                        CapturedFrame::Raw(bitmap),
                                        CaptureOutputSource::GdiFrameAfterHelperError,
                                    ),
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
                            if self.sent_first_frame {
                                continue;
                            }

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
                                    Ok(bitmap) => (
                                        CapturedFrame::Raw(bitmap),
                                        CaptureOutputSource::GdiFrameAfterHelperTimeout,
                                    ),
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
                                let bitmap = fallback_bitmap_update(self.desktop_size).map_err(|error| {
                                    ServerError::io("generate fallback bitmap update", io::Error::other(error))
                                })?;
                                (CapturedFrame::Raw(bitmap), CaptureOutputSource::SyntheticTestPattern)
                            }
                        }
                    }
                };

                self.log_capture_output_source_transition(capture_output_source);

                match captured {
                    CapturedFrame::Size(actual) => {
                        if !self.accept_resized_frame(actual) {
                            sleep(CAPTURE_INTERVAL).await;
                            continue;
                        }
                        return Ok(Some(DisplayUpdate::Resize(actual)));
                    }
                    CapturedFrame::PreEncoded(surface) => {
                        self.log_capture_output_source_transition(CaptureOutputSource::HelperPreEncodedFrame);
                        self.initial_blank_since = None;
                        self.initial_blank_frames = 0;
                        self.sent_first_frame = true;
                        return Ok(Some(DisplayUpdate::PreEncodedSurface(surface)));
                    }
                    CapturedFrame::Raw(bitmap) => {
                        if capture_output_source == CaptureOutputSource::HelperFrame {
                            let actual = DesktopSize {
                                width: bitmap.width.get(),
                                height: bitmap.height.get(),
                            };
                            if actual != self.desktop_size {
                                if !self.accept_resized_frame(actual) {
                                    sleep(CAPTURE_INTERVAL).await;
                                    continue;
                                }
                                return Ok(Some(DisplayUpdate::Resize(actual)));
                            }
                        }
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
                            if blank_elapsed < PERSISTENT_BLANK_RESTART_GRACE {
                                sleep(CAPTURE_INTERVAL).await;
                                continue;
                            }
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

    struct SendHandle(HANDLE);

    // SAFETY: Windows kernel object handles can be sent and used across threads.
    unsafe impl Send for SendHandle {}
    // SAFETY: Windows kernel object handles can be shared across threads.
    unsafe impl Sync for SendHandle {}

    impl Drop for SendHandle {
        fn drop(&mut self) {
            close_handle_best_effort(self.0);
        }
    }

    struct SendMappedView(windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS);

    // SAFETY: this wraps a process-local mapped view pointer; we only use it while the mapping is alive.
    unsafe impl Send for SendMappedView {}
    // SAFETY: access is coordinated by &mut self on the owning client; sharing the address is fine.
    unsafe impl Sync for SendMappedView {}

    impl Drop for SendMappedView {
        fn drop(&mut self) {
            if self.0.Value.is_null() {
                return;
            }

            // SAFETY: this guard exclusively owns the view returned by MapViewOfFile.
            if let Err(error) = unsafe { UnmapViewOfFile(self.0) } {
                warn!(%error, "Failed to unmap shared-memory capture view");
            }
        }
    }

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
            .unwrap_or_else(|| "shm".to_owned());

        match configured.to_ascii_lowercase().as_str() {
            "tcp" => CaptureIpc::Tcp,
            "shm" | "sharedmem" | "shared-memory" => CaptureIpc::SharedMem,
            _ => CaptureIpc::SharedMem,
        }
    }

    pub(super) fn generate_capture_helper_token() -> anyhow::Result<String> {
        let mut bytes = [0u8; CAPTURE_HELPER_TOKEN_LEN / 2];
        // SAFETY: the system-preferred RNG does not require an algorithm handle and `bytes` is writable.
        let status = unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
        anyhow::ensure!(status.0 >= 0, "BCryptGenRandom failed with status {status:?}");

        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut token = String::with_capacity(CAPTURE_HELPER_TOKEN_LEN);
        for byte in bytes {
            token.push(char::from(HEX[usize::from(byte >> 4)]));
            token.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }

        Ok(token)
    }

    fn capture_helper_token_from_env() -> anyhow::Result<String> {
        let token = std::env::var(CAPTURE_HELPER_TOKEN_ENV)
            .with_context(|| format!("capture helper is missing {CAPTURE_HELPER_TOKEN_ENV}"))?;
        anyhow::ensure!(
            token.len() == CAPTURE_HELPER_TOKEN_LEN && token.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "capture helper token has invalid format"
        );
        Ok(token)
    }

    fn child_environment_entries(variables: impl IntoIterator<Item = (String, String)>, token: &str) -> Vec<String> {
        const ALLOWED_VARIABLES: [&str; 9] = [
            "SystemRoot",
            "WINDIR",
            "TEMP",
            "TMP",
            "PATH",
            "IRONRDP_LOG",
            AUTO_SEND_SAS_ENV,
            DUMP_BITMAP_UPDATES_DIR_ENV,
            CAPTURE_IPC_ENV,
        ];

        let mut entries: Vec<String> = variables
            .into_iter()
            .filter(|(name, _)| {
                ALLOWED_VARIABLES
                    .iter()
                    .any(|allowed| name.eq_ignore_ascii_case(allowed))
            })
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        entries.push(format!("{CAPTURE_HELPER_TOKEN_ENV}={token}"));
        entries.sort_unstable_by_key(|entry| entry.to_ascii_uppercase());
        entries
    }

    fn child_environment_with_capture_token(token: &str) -> Vec<u16> {
        let mut block = Vec::new();
        for entry in child_environment_entries(std::env::vars(), token) {
            block.extend(entry.encode_utf16());
            block.push(0);
        }
        block.push(0);
        block
    }

    async fn accept_capture_helper_connection(
        listener: &TcpListener,
        expected_token: &str,
        connection_id: u32,
        channel: &'static str,
    ) -> anyhow::Result<TcpStream> {
        let started = Instant::now();

        loop {
            let remaining = CAPTURE_HELPER_CONNECT_TIMEOUT
                .checked_sub(started.elapsed())
                .ok_or_else(|| anyhow!("capture helper {channel} connection timed out"))?;
            let (mut stream, peer) = timeout(remaining, listener.accept())
                .await
                .map_err(|_| anyhow!("capture helper {channel} connection timed out"))?
                .with_context(|| format!("failed to accept capture helper {channel} connection"))?;

            let mut presented = [0u8; CAPTURE_HELPER_TOKEN_LEN];
            let authenticated = matches!(
                timeout(CAPTURE_HELPER_HANDSHAKE_TIMEOUT, stream.read_exact(&mut presented)).await,
                Ok(Ok(_))
            ) && presented.as_slice() == expected_token.as_bytes();

            if authenticated {
                info!(connection_id, %peer, channel, "Authenticated capture helper connection");
                return Ok(stream);
            }

            warn!(connection_id, %peer, channel, "Rejected unauthenticated capture helper connection");
        }
    }

    async fn authenticate_capture_helper_stream(
        stream: &mut TcpStream,
        token: &str,
        channel: &'static str,
    ) -> anyhow::Result<()> {
        timeout(CAPTURE_HELPER_HANDSHAKE_TIMEOUT, stream.write_all(token.as_bytes()))
            .await
            .map_err(|_| anyhow!("capture helper {channel} authentication timed out"))?
            .with_context(|| format!("failed to authenticate capture helper {channel} stream"))
    }

    enum CapturedFrame {
        Raw(BitmapUpdate),
        PreEncoded(ironrdp_server::PreEncodedSurface),
        Size(DesktopSize),
    }

    enum CaptureClient {
        Tcp(HelperCaptureClient),
        SharedMem(SharedMemCaptureClient),
    }

    impl CaptureClient {
        #[expect(clippy::too_many_arguments, reason = "internal helper startup parameters")]
        async fn start(
            connection_id: u32,
            input_marker: usize,
            desktop_size: DesktopSize,
            input_stream_slot: InputStreamSlot,
            session_id_override: Option<u32>,
            credentials: Option<StoredCredentials>,
            allow_prelogon_fallback: bool,
            client_preferences: Option<(u32, u32)>,
        ) -> anyhow::Result<Self> {
            match capture_ipc_from_env() {
                CaptureIpc::Tcp => Ok(Self::Tcp(
                    HelperCaptureClient::start(
                        connection_id,
                        input_marker,
                        input_stream_slot,
                        session_id_override,
                        credentials,
                        allow_prelogon_fallback,
                        client_preferences,
                    )
                    .await?,
                )),
                CaptureIpc::SharedMem => {
                    match SharedMemCaptureClient::start(
                        connection_id,
                        input_marker,
                        desktop_size,
                        Arc::clone(&input_stream_slot),
                        session_id_override,
                        credentials.clone(),
                        allow_prelogon_fallback,
                        client_preferences,
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
                                    input_marker,
                                    input_stream_slot,
                                    session_id_override,
                                    credentials,
                                    allow_prelogon_fallback,
                                    client_preferences,
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
                Self::SharedMem(client) => client.read_frame().await,
            }
        }
    }

    struct HelperCaptureClient {
        helper_pid: u32,
        helper_process: SendHandle,
        input_stream_slot: InputStreamSlot,
        frame_rx: mpsc::Receiver<anyhow::Result<CapturedFrame>>,
        frame_task: JoinHandle<()>,
    }

    fn spawn_capture_frame_reader(
        mut stream: TcpStream,
    ) -> (mpsc::Receiver<anyhow::Result<CapturedFrame>>, JoinHandle<()>) {
        let (frame_tx, frame_rx) = mpsc::channel(1);
        let frame_task = tokio::task::spawn_local(async move {
            loop {
                let frame = read_capture_frame(&mut stream).await;
                let should_stop = frame.is_err();
                if frame_tx.send(frame).await.is_err() || should_stop {
                    break;
                }
            }
        });
        (frame_rx, frame_task)
    }

    impl HelperCaptureClient {
        async fn start(
            connection_id: u32,
            input_marker: usize,
            input_stream_slot: InputStreamSlot,
            session_id_override: Option<u32>,
            credentials: Option<StoredCredentials>,
            allow_prelogon_fallback: bool,
            client_preferences: Option<(u32, u32)>,
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

            let helper_token = generate_capture_helper_token()?;
            // Token acquisition queries TermService over RPC; keep it off the runtime thread so the
            // control pipe stays responsive (see the logon readiness probe in the capture loop).
            let helper = {
                let helper_token = helper_token.clone();
                tokio::task::spawn_blocking(move || {
                    spawn_capture_helper_process_tcp(
                        local_addr,
                        input_addr,
                        &helper_token,
                        session_id_override,
                        credentials,
                        allow_prelogon_fallback,
                        client_preferences,
                    )
                })
                .await
                .context("capture helper spawn task failed")?
            }
            .with_context(|| format!("failed to spawn capture helper for connection {connection_id}"))?;

            info!(
                connection_id,
                helper_pid = helper.pid,
                "Waiting for capture helper TCP connection"
            );

            let stream = accept_capture_helper_connection(&listener, &helper_token, connection_id, "capture").await?;
            let mut input_stream =
                accept_capture_helper_connection(&input_listener, &helper_token, connection_id, "input").await?;
            input_stream
                .write_u64_le(u64::try_from(input_marker)?)
                .await
                .context("send connection input marker")?;

            info!(connection_id, "Capture helper input channel connected");

            input_stream_slot.attach(input_stream).await;

            let (helper_pid, helper_process) = helper.into_process();
            let (frame_rx, frame_task) = spawn_capture_frame_reader(stream);

            Ok(Self {
                helper_pid,
                helper_process,
                input_stream_slot,
                frame_rx,
                frame_task,
            })
        }

        fn pid(&self) -> u32 {
            self.helper_pid
        }

        fn terminate(self) {
            drop(self);
        }

        async fn read_frame(&mut self) -> anyhow::Result<CapturedFrame> {
            self.frame_rx
                .recv()
                .await
                .ok_or_else(|| anyhow!("capture helper frame reader stopped"))?
        }
    }

    impl Drop for HelperCaptureClient {
        fn drop(&mut self) {
            if let Ok(mut guard) = self.input_stream_slot.stream.try_lock() {
                *guard = None;
            }
            self.frame_task.abort();
            // SAFETY: this client owns the process handle returned by CreateProcessAsUserW.
            let _ = unsafe { TerminateProcess(self.helper_process.0, 1) };
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
    const SHM_OFF_DISPLAY_SIZE: usize = 40;

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
        input_stream_slot: InputStreamSlot,
        _mapping: SendHandle,
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
        #[expect(clippy::too_many_arguments, reason = "internal helper startup parameters")]
        async fn start(
            connection_id: u32,
            input_marker: usize,
            desktop_size: DesktopSize,
            input_stream_slot: InputStreamSlot,
            session_id_override: Option<u32>,
            credentials: Option<StoredCredentials>,
            allow_prelogon_fallback: bool,
            client_preferences: Option<(u32, u32)>,
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
            let generation = generate_capture_helper_token()?;
            let map_name = format!("Global\\IronRdpTermSrvFb_{pid}_{connection_id}_{generation}");
            let event_name = format!("{map_name}_Ready");

            let map_name_w: Vec<u16> = map_name.encode_utf16().chain(Some(0)).collect();
            let event_name_w: Vec<u16> = event_name.encode_utf16().chain(Some(0)).collect();

            let input_listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .context("failed to bind local capture helper input listener")?;

            let input_addr = input_listener
                .local_addr()
                .context("failed to query local helper input listener address")?;

            info!(connection_id, input_addr = %input_addr, "Capture helper input listener bound");

            let helper_token = generate_capture_helper_token()?;
            let (mapping, frame_ready_event, view, helper) = {
                let helper_token = helper_token.clone();
                tokio::task::spawn_blocking(move || {
                    let session_id = match session_id_override {
                        Some(id) => id,
                        None => resolve_capture_session_id()?,
                    };
                    let acquired =
                        acquire_session_token(session_id, credentials.as_ref(), allow_prelogon_fallback, true)?;
                    let sid = token_user_sid(acquired.token)?;
                    // The session user needs access even when its administrator group is filtered by UAC.
                    let sddl: Vec<u16> = format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})S:(ML;;NW;;;ME)")
                        .encode_utf16()
                        .chain(Some(0))
                        .collect();
                    let (attrs, descriptor) = security_attributes(PCWSTR(sddl.as_ptr()))?;
                    let result = (|| {
                        // SAFETY: the descriptor and name remain valid throughout object creation.
                        let mapping = SendHandle(
                            unsafe {
                                CreateFileMappingW(
                                    windows::Win32::Foundation::INVALID_HANDLE_VALUE,
                                    Some(&attrs),
                                    PAGE_READWRITE,
                                    0,
                                    view_len_u32,
                                    PCWSTR(map_name_w.as_ptr()),
                                )
                            }
                            .context("create shared framebuffer mapping")?,
                        );
                        // SAFETY: the mapping has view_len bytes.
                        let view = SendMappedView(unsafe {
                            MapViewOfFile(mapping.0, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, view_len)
                        });
                        anyhow::ensure!(!view.0.Value.is_null(), "MapViewOfFile returned null");
                        // SAFETY: the writable view contains the entire header.
                        unsafe { shm_init_header(view.0.Value.cast::<u8>(), width, height, stride, slot_len)? };
                        // SAFETY: the descriptor and name remain live for this synchronous call.
                        let event = SendHandle(
                            unsafe { CreateEventW(Some(&attrs), false, false, PCWSTR(event_name_w.as_ptr())) }
                                .context("create framebuffer notification event")?,
                        );
                        let pref_args = format_client_preference_args(client_preferences);
                        let helper = spawn_session_helper_process(
                            &format!(
                                "--shm-map \"{map_name}\" --shm-event \"{event_name}\" --input-connect {input_addr}{pref_args}"
                            ),
                            &helper_token,
                            session_id,
                            acquired,
                        )?;
                        Ok::<_, anyhow::Error>((mapping, event, view, helper))
                    })();
                    // SAFETY: both kernel objects copied this LocalAlloc-owned descriptor.
                    let _ = unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
                    result
                })
                .await
                .context("shared-memory capture helper spawn task failed")?
            }
            .with_context(|| format!("failed to spawn shared-memory capture helper for connection {connection_id}"))?;

            let mut input_stream =
                accept_capture_helper_connection(&input_listener, &helper_token, connection_id, "input").await?;
            input_stream
                .write_u64_le(u64::try_from(input_marker)?)
                .await
                .context("send connection input marker")?;

            info!(connection_id, "Capture helper input channel connected");

            input_stream_slot.attach(input_stream).await;

            let (helper_pid, helper_process) = helper.into_process();

            Ok(Self {
                helper_pid,
                helper_process,
                input_stream_slot,
                _mapping: mapping,
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
            drop(self);
        }

        async fn read_frame(&mut self) -> anyhow::Result<CapturedFrame> {
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

                // SAFETY: the aligned size record is inside the mapped header.
                let size = unsafe { shm_read_u32(view_ptr, SHM_OFF_DISPLAY_SIZE, self.view_len)? };
                if size != 0 {
                    let bytes = size.to_le_bytes();
                    let actual = DesktopSize {
                        width: u16::from_le_bytes([bytes[0], bytes[1]]),
                        height: u16::from_le_bytes([bytes[2], bytes[3]]),
                    };
                    DisplaySize::new(actual.width, actual.height).context("invalid shared-memory desktop size")?;
                    if actual.width != self.width.get() || actual.height != self.height.get() {
                        return Ok(CapturedFrame::Size(actual));
                    }
                }

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

                return Ok(CapturedFrame::Raw(BitmapUpdate {
                    x: 0,
                    y: 0,
                    width: self.width,
                    height: self.height,
                    format: PixelFormat::BgrA32,
                    data: data.into(),
                    stride: self.stride,
                }));
            }
        }
    }

    impl Drop for SharedMemCaptureClient {
        fn drop(&mut self) {
            if let Ok(mut guard) = self.input_stream_slot.stream.try_lock() {
                *guard = None;
            }
            // SAFETY: this client owns the process handle returned by CreateProcessAsUserW.
            let _ = unsafe { TerminateProcess(self.helper_process.0, 1) };
        }
    }

    struct SpawnedProcess {
        pid: u32,
        process: Option<SendHandle>,
    }

    impl SpawnedProcess {
        fn into_process(mut self) -> (u32, SendHandle) {
            let process = self.process.take().expect("spawned process handle is present");
            (self.pid, process)
        }
    }

    impl Drop for SpawnedProcess {
        fn drop(&mut self) {
            let Some(process) = self.process.as_ref() else {
                return;
            };

            // SAFETY: this guard owns the process handle and the child has not been handed off.
            let _ = unsafe { TerminateProcess(process.0, 1) };
        }
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

    impl Drop for AcquiredSessionToken {
        fn drop(&mut self) {
            close_handle_best_effort(self.token);
        }
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
            let _ = CloseHandle(handle);
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

    fn format_client_preference_args(client_preferences: Option<(u32, u32)>) -> String {
        match client_preferences {
            Some((performance_flags, keyboard_layout)) => {
                format!(" --performance-flags {performance_flags} --keyboard-layout {keyboard_layout}")
            }
            None => String::new(),
        }
    }

    fn spawn_capture_helper_process_tcp(
        connect_addr: SocketAddr,
        input_connect_addr: SocketAddr,
        helper_token: &str,
        session_id_override: Option<u32>,
        credentials: Option<StoredCredentials>,
        allow_prelogon_fallback: bool,
        client_preferences: Option<(u32, u32)>,
    ) -> anyhow::Result<SpawnedProcess> {
        let pref_args = format_client_preference_args(client_preferences);
        spawn_capture_helper_process_with_args(
            &format!("--connect {connect_addr} --input-connect {input_connect_addr}{pref_args}"),
            helper_token,
            session_id_override,
            credentials,
            allow_prelogon_fallback,
        )
    }

    fn spawn_capture_helper_process_with_args(
        extra_args: &str,
        helper_token: &str,
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
        spawn_session_helper_process(extra_args, helper_token, session_id, acquired)
    }

    fn spawn_session_helper_process(
        extra_args: &str,
        helper_token: &str,
        session_id: u32,
        acquired: AcquiredSessionToken,
    ) -> anyhow::Result<SpawnedProcess> {
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
        let environment = child_environment_with_capture_token(helper_token);

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
                CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
                Some(environment.as_ptr().cast::<c_void>()),
                None,
                &startup_info,
                &mut process_info,
            )
        };

        create_ok.ok().context("CreateProcessAsUserW failed")?;

        let _thread = SendHandle(process_info.hThread);
        let helper = SpawnedProcess {
            pid: process_info.dwProcessId,
            process: Some(SendHandle(process_info.hProcess)),
        };

        let helper_session_id = process_id_to_session_id(helper.pid)
            .ok_or_else(|| anyhow!("failed to resolve capture helper session id for pid {}", helper.pid))?;

        info!(
            helper_pid = helper.pid,
            requested_session_id = session_id,
            helper_session_id,
            desktop,
            "Capture helper process created"
        );

        if helper_session_id != session_id {
            return Err(anyhow!(
                "capture helper launched in unexpected session {helper_session_id} (expected {session_id})"
            ));
        }

        Ok(helper)
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
                let _ = CloseHandle(token);
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
            }
        }
        let user_token_available = wts_result.is_ok();
        let interactive_user_ready = user_token_available || session_has_explorer_token(session_id);
        let target_desktop = if interactive_user_ready || !allow_prelogon_fallback {
            HelperDesktop::Default
        } else {
            HelperDesktop::Winlogon
        };

        if allow_service_token_fallback && (interactive_user_ready || allow_prelogon_fallback) {
            // Prefer a session-bound LocalSystem token (winlogon.exe / csrss.exe / service token)
            // for the display and input helper so WmsgSendMessage(SAS) passes Winlogon's
            // LocalSystem check and OpenInputDesktop/SetThreadDesktop can follow transitions
            // between WinSta0\Default and WinSta0\Winlogon (Ctrl+Alt+Del and UAC secure desktop).
            for source_process in ["winlogon.exe", "csrss.exe"] {
                if let Ok(sys_token) =
                    token_from_session_process_with_retries(session_id, source_process, process_lookup_retries)
                {
                    if user_token_available {
                        close_handle_best_effort(token);
                    }
                    return finalize_acquired_session_token(session_id, sys_token, target_desktop, source_process);
                }
            }
            if let Ok(sys_token) = duplicate_self_token_for_session(session_id) {
                if user_token_available {
                    close_handle_best_effort(token);
                }
                return finalize_acquired_session_token(
                    session_id,
                    sys_token,
                    target_desktop,
                    "service_token_fallback",
                );
            }
        }

        if allow_service_token_fallback {
            match token_from_session_process_with_retries(session_id, "explorer.exe", process_lookup_retries) {
                Ok(explorer_token) => {
                    if user_token_available {
                        close_handle_best_effort(token);
                    }
                    debug!(session_id, "Using explorer.exe token (default desktop)");
                    return finalize_acquired_session_token(
                        session_id,
                        explorer_token,
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

        if user_token_available {
            return finalize_acquired_session_token(session_id, token, HelperDesktop::Default, "wts_query_user_token");
        }

        if !allow_prelogon_fallback && !allow_service_token_fallback {
            let wts_error = wts_result.err().unwrap_or_else(windows::core::Error::empty);
            info!(
                session_id,
                error = %wts_error,
                error_code = %wts_error.code(),
                "WTSQueryUserToken unavailable and prelogon token fallback is disabled after initial helper start"
            );
            return Err(anyhow!("prelogon token fallback disabled after initial helper start"));
        }

        if allow_prelogon_fallback {
            match token_from_session_process_with_retries(session_id, "winlogon.exe", process_lookup_retries) {
                Ok(token) => {
                    info!(
                        session_id,
                        "Using winlogon.exe token (winlogon desktop \u{2014} pre-login)"
                    );
                    return finalize_acquired_session_token(session_id, token, HelperDesktop::Winlogon, "winlogon.exe");
                }
                Err(error) => {
                    info!(
                        session_id,
                        error = %format!("{error:#}"),
                        "winlogon.exe token unavailable for capture session"
                    );
                }
            }
        }

        if allow_service_token_fallback {
            match token_from_session_process_with_retries(session_id, "csrss.exe", process_lookup_retries) {
                Ok(token) => {
                    info!(
                        session_id,
                        "Using csrss.exe token (fallback for session-bound helper startup)"
                    );
                    return finalize_acquired_session_token(session_id, token, HelperDesktop::Default, "csrss.exe");
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

        finalize_acquired_session_token(session_id, token, HelperDesktop::Default, "service_token_fallback")
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
            let enumerate_result =
                unsafe { WTSEnumerateProcessesW(None, 0, 1, &mut process_info_ptr, &mut process_count) };

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
                    let _ = CloseHandle(self.0);
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
            let _ = CloseHandle(process_token);
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
            let _ = CloseHandle(process_token);
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
        if ok.as_bool() { Some(session_id) } else { None }
    }

    fn current_process_session_id() -> Option<u32> {
        // SAFETY: GetCurrentProcessId has no preconditions.
        let pid = unsafe { GetCurrentProcessId() };
        process_id_to_session_id(pid)
    }

    fn now_unix_ms_best_effort() -> Option<u64> {
        let now = std::time::SystemTime::now();
        let dur = now.duration_since(std::time::UNIX_EPOCH).ok()?;
        Some(u64::try_from(dur.as_millis()).unwrap_or(u64::MAX))
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

    struct CachedCaptureSurface {
        width: i32,
        height: i32,
        memory_dc: windows::Win32::Graphics::Gdi::HDC,
        bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
        previous_bitmap: HGDIOBJ,
        bits_ptr: *mut u8,
        frame_len: usize,
    }

    impl Drop for CachedCaptureSurface {
        fn drop(&mut self) {
            // SAFETY: restore previous bitmap before deleting the DIB section and memory DC.
            let _ = unsafe { SelectObject(self.memory_dc, self.previous_bitmap) };
            // SAFETY: `bitmap` was created by CreateDIBSection.
            let _ = unsafe { DeleteObject(HGDIOBJ(self.bitmap.0)) };
            // SAFETY: `memory_dc` was created by CreateCompatibleDC.
            let _ = unsafe { DeleteDC(self.memory_dc) };
        }
    }

    fn capture_bitmap_update(size: DesktopSize) -> anyhow::Result<BitmapUpdate> {
        thread_local! {
            static SURFACE: core::cell::RefCell<Option<CachedCaptureSurface>> = const { core::cell::RefCell::new(None) };
        }

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

        let result = SURFACE.with(|slot| -> anyhow::Result<Vec<u8>> {
            let mut slot = slot.borrow_mut();
            let needs_recreate = slot
                .as_ref()
                .is_none_or(|s| s.width != width_i32 || s.height != height_i32 || s.frame_len != frame_len);
            if needs_recreate {
                *slot = None;
                // SAFETY: `screen_dc` is a valid display DC obtained above.
                let memory_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
                if memory_dc.0.is_null() {
                    return Err(anyhow!("CreateCompatibleDC returned a null memory device context"));
                }

                let mut bitmap_info = BITMAPINFO::default();
                bitmap_info.bmiHeader.biSize = u32::try_from(size_of::<BITMAPINFOHEADER>())
                    .map_err(|_| anyhow!("BITMAPINFOHEADER size overflow"))?;
                bitmap_info.bmiHeader.biWidth = width_i32;
                bitmap_info.bmiHeader.biHeight = top_down_height;
                bitmap_info.bmiHeader.biPlanes = 1;
                bitmap_info.bmiHeader.biBitCount = 32;
                bitmap_info.bmiHeader.biCompression = BI_RGB.0;

                let mut bits_ptr: *mut c_void = null_mut();
                // SAFETY: `screen_dc` and `bitmap_info` are valid, and we pass a valid out-pointer for bits.
                let bitmap =
                    unsafe { CreateDIBSection(Some(screen_dc), &bitmap_info, DIB_RGB_COLORS, &mut bits_ptr, None, 0) }
                        .map_err(|error| {
                            // SAFETY: `memory_dc` was created above and must be deleted on error.
                            let _ = unsafe { DeleteDC(memory_dc) };
                            anyhow!("CreateDIBSection failed: {error}")
                        })?;

                if bitmap.0.is_null() || bits_ptr.is_null() {
                    if !bitmap.0.is_null() {
                        // SAFETY: `bitmap` was created by CreateDIBSection above.
                        let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
                    }
                    // SAFETY: `memory_dc` was created above.
                    let _ = unsafe { DeleteDC(memory_dc) };
                    return Err(anyhow!(
                        "CreateDIBSection returned a null bitmap handle or data pointer"
                    ));
                }

                // SAFETY: `memory_dc` is valid and `bitmap` is a valid bitmap handle.
                let previous_bitmap = unsafe { SelectObject(memory_dc, HGDIOBJ(bitmap.0)) };
                if previous_bitmap.0.is_null() {
                    // SAFETY: `bitmap` was created by CreateDIBSection above.
                    let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
                    // SAFETY: `memory_dc` was created above.
                    let _ = unsafe { DeleteDC(memory_dc) };
                    return Err(anyhow!("SelectObject failed for capture bitmap"));
                }

                *slot = Some(CachedCaptureSurface {
                    width: width_i32,
                    height: height_i32,
                    memory_dc,
                    bitmap,
                    previous_bitmap,
                    bits_ptr: bits_ptr.cast::<u8>(),
                    frame_len,
                });
            }

            let surface = slot.as_ref().expect("surface initialized above");
            // SAFETY: all DC handles are valid and dimensions match the selected DIB section.
            let bitblt_result = unsafe {
                BitBlt(
                    surface.memory_dc,
                    0,
                    0,
                    width_i32,
                    height_i32,
                    Some(screen_dc),
                    0,
                    0,
                    SRCCOPY,
                )
            };
            if let Err(error) = bitblt_result {
                *slot = None;
                return Err(anyhow!("BitBlt failed while capturing desktop frame: {error}"));
            }

            // SAFETY: `bits_ptr` points to the mapped DIB section of `frame_len` bytes populated by BitBlt.
            let slice = unsafe { core::slice::from_raw_parts(surface.bits_ptr, frame_len) };
            Ok(slice.to_vec())
        });

        // SAFETY: `screen_dc` was acquired with `GetDC` and must be released.
        let _ = unsafe { ReleaseDC(None, screen_dc) };

        let data = result?;

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
        native_channels: Arc<crate::native::NativeChannels>,
        listener_name: String,
        peer_addr: Option<String>,
        session_task: Option<JoinHandle<()>>,
        /// Credentials captured from the CredSSP handshake.
        ///
        /// `None` until client-supplied credentials are captured; queried by `GetConnectionCredentials`.
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
                ProviderCommand::GetRemoteAppSettings { connection_id } => {
                    let state = self.state.lock().await;
                    match state.connections.get(&connection_id) {
                        Some(entry) => match entry.native_channels.remote_app_settings() {
                            Ok(settings) => ServiceEvent::RemoteAppSettings {
                                connection_id,
                                settings,
                            },
                            Err(error) => {
                                warn!(connection_id, %error, "Failed to query native RemoteApp settings");
                                ServiceEvent::Error {
                                    message: error.to_string(),
                                }
                            }
                        },
                        None => ServiceEvent::Error {
                            message: "RemoteApp connection is unknown".to_owned(),
                        },
                    }
                }
                ProviderCommand::SetCaptureSessionId {
                    connection_id,
                    session_id,
                } => self.set_capture_session_id(connection_id, session_id).await,
                ProviderCommand::NotifyIddDriverLoaded { session_id } => {
                    let mut guard = self.state.lock().await;
                    guard.notify_idd_driver_loaded(session_id)
                }
                ProviderCommand::OpenVirtualChannel {
                    connection_id,
                    session_id,
                    endpoint_name,
                    static_channel,
                } => {
                    let native = {
                        let state = self.state.lock().await;
                        state
                            .connections
                            .get(&connection_id)
                            .map(|entry| Arc::clone(&entry.native_channels))
                    };
                    let result = async {
                        let native = native.context("native channel connection is unknown")?;
                        if static_channel {
                            native.open_static(session_id, &endpoint_name).await
                        } else {
                            native.open(session_id, &endpoint_name).await
                        }
                    }
                    .await;
                    match result {
                        Ok(pipe_name) => ServiceEvent::VirtualChannelReady { pipe_name },
                        Err(error) => {
                            warn!(connection_id, session_id, %error, "Native channel open rejected");
                            ServiceEvent::Error {
                                message: format!("{error:#}"),
                            }
                        }
                    }
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

                            if accept_tx.send(accepted).await.is_err() {
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
            dwSize: u32::try_from(size_of::<PROCESSENTRY32W>())
                .map_err(|_| anyhow!("PROCESSENTRY32W size overflow"))?,
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
        accepted_tx: mpsc::Sender<AcceptedSocket>,
        broken_tx: mpsc::Sender<BrokenNotification>,
        /// Session ID per connection, set when the provider receives NotifySessionId from WTS.
        connection_session_ids: Arc<StdMutex<HashMap<u32, u32>>>,
        /// Session IDs for which the provider has signaled `NotifyIddDriverLoaded`.
        idd_loaded_sessions: HashSet<u32>,
        session_binding_changed: watch::Sender<()>,
    }

    impl ServiceState {
        fn new(bind_addr: SocketAddr) -> (Self, mpsc::Receiver<AcceptedSocket>, mpsc::Receiver<BrokenNotification>) {
            let (accepted_tx, accepted_rx) = mpsc::channel(MAX_CONCURRENT_CONNECTIONS);
            let (broken_tx, broken_rx) = mpsc::channel(MAX_CONCURRENT_CONNECTIONS);

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
                session_binding_changed: watch::channel(()).0,
            };

            (state, accepted_rx, broken_rx)
        }

        fn allocate_connection_id(&mut self) -> Option<u32> {
            for _ in 0..=self.connections.len() {
                let candidate = self.next_connection_id;
                self.next_connection_id = self.next_connection_id.checked_add(1).unwrap_or(1);
                if !self.connections.contains_key(&candidate) {
                    return Some(candidate);
                }
            }
            None
        }

        fn set_capture_session_id(&mut self, connection_id: u32, session_id: u32) -> ServiceEvent {
            let Some(connection) = self.connections.get(&connection_id) else {
                return ServiceEvent::Error {
                    message: "session binding references an unknown connection".to_owned(),
                };
            };
            connection.native_channels.set_session(session_id);
            let idd_loaded_for_session = self.idd_loaded_sessions.contains(&session_id);

            if let Ok(mut guard) = self.connection_session_ids.lock() {
                if guard.insert(connection_id, session_id) != Some(session_id) {
                    self.session_binding_changed.send_replace(());
                }
                info!(connection_id, session_id, "Set capture session id for connection");
                info!(
                    connection_id,
                    session_id, "SESSION_PROOF_TERMSRV_SET_CAPTURE_SESSION_ID_APPLIED"
                );
                info!(
                    connection_id,
                    session_id, idd_loaded_for_session, "SESSION_PROOF_TERMSRV_IDD_SESSION_BIND"
                );

                if idd_loaded_for_session {
                    info!(connection_id, session_id, "SESSION_PROOF_TERMSRV_IDD_READY_FOR_CAPTURE");
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
                        session_id, "SESSION_PROOF_TERMSRV_IDD_READY_FOR_CAPTURE"
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

            ServiceEvent::Error {
                message: format!("connection session task not started for connection {connection_id}"),
            }
        }

        fn close_connection(&mut self, connection_id: u32) -> ServiceEvent {
            if let Ok(mut guard) = self.connection_session_ids.lock() {
                guard.remove(&connection_id);
                self.session_binding_changed.send_replace(());
            }
            self.pending_incoming
                .retain(|pending| pending.connection_id != connection_id);
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

        fn register_accepted(&mut self, accepted: AcceptedSocket) -> bool {
            if self.connections.len() >= MAX_CONCURRENT_CONNECTIONS {
                warn!(
                    max_connections = MAX_CONCURRENT_CONNECTIONS,
                    peer_addr = ?accepted.peer_addr,
                    "Rejecting connection because the server is at capacity"
                );
                return false;
            }
            let Some(connection_id) = self.allocate_connection_id() else {
                warn!("Rejecting connection because no connection ID is available");
                return false;
            };

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
            let session_binding_changed = self.session_binding_changed.subscribe();
            let peer_addr_for_task = peer_addr.clone();
            let listener_name_for_task = listener_name.clone();
            let broken_tx = self.broken_tx.clone();

            let stream = accepted.stream;
            let native_channels = Arc::new(crate::native::NativeChannels::default());
            let native_for_task = Arc::clone(&native_channels);
            let session_task = tokio::task::spawn_local(async move {
                let result = run_ironrdp_connection(
                    connection_id,
                    peer_addr_for_task.as_deref(),
                    stream,
                    credentials_slot,
                    connection_session_ids,
                    Some(session_binding_changed),
                    Some(native_for_task),
                )
                .await;

                let reason = match &result {
                    Ok(()) => "connection closed".to_owned(),
                    Err(error) => format!("{error:#}"),
                };

                let _ = broken_tx
                    .send(BrokenNotification {
                        listener_name: listener_name_for_task,
                        connection_id,
                        reason,
                    })
                    .await;

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
                    native_channels,
                    listener_name: listener_name.clone(),
                    peer_addr: peer_addr.clone(),
                    session_task: Some(session_task),
                    credentials,
                },
            );

            self.pending_incoming.push_back(PendingIncoming {
                listener_name,
                connection_id,
                peer_addr,
            });
            true
        }
    }

    async fn run_ironrdp_connection(
        connection_id: u32,
        peer_addr: Option<&str>,
        stream: TcpStream,
        credentials_slot: Arc<Mutex<Option<StoredCredentials>>>,
        connection_session_ids: Arc<StdMutex<HashMap<u32, u32>>>,
        mut session_binding_changed: Option<watch::Receiver<()>>,
        native_channels: Option<Arc<crate::native::NativeChannels>>,
    ) -> anyhow::Result<()> {
        let provider_mode = native_channels.is_some();
        let remote_app_enabled = crate::native::remote_app_enabled()?;
        anyhow::ensure!(!remote_app_enabled || provider_mode, "RemoteApp requires provider mode");
        if let Some(native) = &native_channels {
            native.configure_remote_app(remote_app_enabled);
        }
        info!(connection_id, peer_addr = ?peer_addr, "Starting IronRDP session task");

        let input_stream_slot: InputStreamSlot = Arc::default();
        let (input_tx, input_rx) = mpsc::channel::<InputPacket>(INPUT_QUEUE_CAPACITY);
        let input_task = ConnectionTask(tokio::task::spawn_local(run_input_spooler(
            connection_id,
            Arc::clone(&input_stream_slot),
            input_rx,
        )));

        let display = GdiDisplay::new(
            connection_id,
            Arc::clone(&input_stream_slot),
            Arc::clone(&connection_session_ids),
            Arc::clone(&credentials_slot),
            native_channels.clone(),
        )
        .context("failed to initialize GDI display handler")?;
        let display_state = Arc::clone(&display.state);
        let (tls_acceptor, tls_pub_key) = make_tls_acceptor().context("failed to initialize TLS acceptor")?;

        let input_handler = TermSrvInputHandler::new(connection_id, input_tx);
        let (clipboard_factory, clipboard_bridge) = if crate::clipboard::enabled()? {
            let (factory, bridge) = crate::clipboard::ClipboardFactory::new();
            let factory: Box<dyn ironrdp_server::CliprdrServerFactory> = Box::new(factory);
            (Some(factory), Some(bridge))
        } else {
            (None, None)
        };

        let mut server = {
            let builder = RdpServer::builder().with_addr(([127, 0, 0, 1], 0));
            let builder = builder.with_hybrid(tls_acceptor, tls_pub_key);

            let rfx_only_codecs =
                ironrdp_pdu::rdp::capability_sets::server_codecs_capabilities(&["remotefx:on", "qoi:off", "qoiz:off"])
                    .expect("valid codec config");

            let builder = builder
                .with_input_handler(input_handler.clone())
                .with_display_handler(display)
                .with_honor_client_desktop_size(provider_mode.then_some(DesktopSize {
                    width: 3840,
                    height: 2400,
                }))
                .with_bitmap_codecs(rfx_only_codecs)
                .with_remotefx_quant(ironrdp_pdu::codecs::rfx::Quant {
                    ll3: 6,
                    lh3: 6,
                    hl3: 6,
                    hh3: 6,
                    lh2: 6,
                    hl2: 6,
                    hh2: 6,
                    lh1: 6,
                    hl1: 6,
                    hh1: 6,
                })
                .with_cliprdr_factory(clipboard_factory)
                .with_rdpei_factory(Some(Box::new(input_handler)));

            match &native_channels {
                Some(native) => builder
                    .with_static_channel_factory(native.static_channels())
                    .with_remote_app(native.remote_app_capabilities())
                    .with_connection_handler(Some(native.connection_handler()))
                    .build(),
                None => builder
                    .with_connection_handler(Some(Box::new(StandaloneConnectionObserver(Arc::clone(&display_state)))))
                    .build(),
            }
        };

        let _native_guard = native_channels
            .as_ref()
            .map(|native| native.attach(server.event_sender().clone()))
            .transpose()?;

        if provider_mode {
            server.set_credssp_backend(ironrdp_server::ironrdp_acceptor::CredsspBackend::Windows);
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

        let pending = match timeout(CONNECTION_HANDSHAKE_TIMEOUT, server.run_connection_handshake(stream)).await {
            Ok(Ok(pending)) => pending,
            Ok(Err(error)) => {
                return Err(error).with_context(|| format!("failed handshake for connection {connection_id}"));
            }
            Err(_) => return Err(anyhow!("connection {connection_id} handshake timed out")),
        };

        // Store CredSSP credentials immediately so the WTS provider DLL can
        // retrieve them via `GetConnectionCredentials` before the display loop
        // blocks.
        if let Some(identity) = pending.captured_identity() {
            let (username, domain) = match identity.username.parts() {
                ironrdp_server::ironrdp_acceptor::UsernameParts::UserPrincipalName(parts) => {
                    (parts.upn().to_owned(), String::new())
                }
                ironrdp_server::ironrdp_acceptor::UsernameParts::DownLevelLogonName(parts) => (
                    parts.account_name().to_owned(),
                    parts.netbios_domain().unwrap_or("").to_owned(),
                ),
            };
            let password = identity.password.as_ref().to_owned();

            info!(
                connection_id,
                username = %username,
                domain = %domain,
                "Captured CredSSP credentials; storing for WTS provider"
            );

            let mut guard = credentials_slot.lock().await;
            *guard = Some(StoredCredentials {
                username,
                domain,
                password,
            });
        } else if provider_mode {
            return Err(anyhow!(
                "provider-mode connection {connection_id} completed handshake without captured CredSSP credentials"
            ));
        }

        let clipboard_sender = server.event_sender().clone();
        let clipboard_native_channels = native_channels.clone();
        let clipboard_task = clipboard_bridge.map(|clipboard_bridge| {
            ConnectionTask(tokio::task::spawn_local(async move {
                let result = async {
                    let session_id = loop {
                        let assigned = if provider_mode {
                            connection_session_ids
                                .lock()
                                .map_err(|_| anyhow!("session binding lock poisoned"))?
                                .get(&connection_id)
                                .copied()
                        } else {
                            Some(resolve_capture_session_id()?)
                        };
                        if let Some(session_id) = assigned {
                            let ready = tokio::task::spawn_blocking(move || session_has_user_token(session_id))
                                .await
                                .context("query clipboard session readiness")?;
                            if ready {
                                if provider_mode {
                                    let current = connection_session_ids
                                        .lock()
                                        .map_err(|_| anyhow!("session binding lock poisoned"))?
                                        .get(&connection_id)
                                        .copied();
                                    anyhow::ensure!(current == Some(session_id), "clipboard session binding changed");
                                }
                                break session_id;
                            }
                        }
                        sleep(LOGON_READINESS_PROBE_INTERVAL).await;
                    };
                    let client_preferences = clipboard_native_channels
                        .as_ref()
                        .and_then(|native| native.remote_app_settings().ok().flatten())
                        .map(|settings| (settings.performance_flags, settings.keyboard_layout))
                        .or_else(|| display_state.lock().ok().and_then(|state| state.client_preferences));
                    let pref_args = format_client_preference_args(client_preferences);
                    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
                    let addr = listener.local_addr()?;
                    let token = generate_capture_helper_token()?;
                    let helper_token = token.clone();
                    let helper = tokio::task::spawn_blocking(move || {
                        let mut user_token = HANDLE::default();
                        // UNTESTED: Concurrent users must not share clipboard state; verify with distinct logged-on users.
                        // SAFETY: The output is writable; only the assigned session's user token is requested.
                        unsafe { WTSQueryUserToken(session_id, &mut user_token) }
                            .context("acquire clipboard user token")?;
                        let acquired = finalize_acquired_session_token(
                            session_id,
                            user_token,
                            HelperDesktop::Default,
                            "clipboard_user",
                        )?;
                        spawn_session_helper_process(
                            &format!("--clipboard-connect {addr}{pref_args}"),
                            &helper_token,
                            session_id,
                            acquired,
                        )
                    })
                    .await
                    .context("spawn clipboard helper task")??;
                    let stream =
                        accept_capture_helper_connection(&listener, &token, connection_id, "clipboard").await?;
                    info!(
                        connection_id,
                        session_id,
                        helper_pid = helper.pid,
                        "Session clipboard bridge connected"
                    );
                    let result = tokio::select! {
                        biased;
                        result = async {
                            loop {
                                if provider_mode {
                                    let current = connection_session_ids.lock()
                                        .map_err(|_| anyhow!("session binding lock poisoned"))?
                                        .get(&connection_id).copied();
                                    anyhow::ensure!(current == Some(session_id), "clipboard session binding changed");
                                }
                                if let Some(changed) = &mut session_binding_changed {
                                    changed.changed().await.context("session binding source closed")?;
                                } else {
                                    core::future::pending::<()>().await;
                                }
                            }
                        } => result,
                        result = clipboard_bridge.run(stream, clipboard_sender) => result,
                    };
                    drop(helper);
                    result
                }
                .await;
                if let Err(error) = result {
                    warn!(connection_id, %error, "Session clipboard redirection stopped");
                }
            }))
        });

        server
            .run_connection_session(pending)
            .await
            .with_context(|| format!("failed to run IronRDP session for connection {connection_id}"))?;

        drop(input_task);
        drop(clipboard_task);

        info!(connection_id, peer_addr = ?peer_addr, "IronRDP session task finished");
        Ok(())
    }

    struct ConnectionTask(JoinHandle<()>);

    impl Drop for ConnectionTask {
        fn drop(&mut self) {
            self.0.abort();
        }
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

    struct OwnedCertStore(HCERTSTORE);

    impl Drop for OwnedCertStore {
        fn drop(&mut self) {
            // SAFETY: this guard exclusively owns the certificate store.
            let _ = unsafe { CertCloseStore(Some(self.0), 0) };
        }
    }

    struct OwnedCertContext(*const CERT_CONTEXT);

    impl Drop for OwnedCertContext {
        fn drop(&mut self) {
            // SAFETY: this guard exclusively owns the certificate context.
            let _ = unsafe { CertFreeCertificateContext(Some(self.0)) };
        }
    }

    struct OwnedNCryptProvider(NCRYPT_PROV_HANDLE);

    impl Drop for OwnedNCryptProvider {
        fn drop(&mut self) {
            // SAFETY: this guard exclusively owns the CNG provider handle.
            let _ = unsafe { NCryptFreeObject(self.0.into()) };
        }
    }

    struct OwnedNCryptKey(NCRYPT_KEY_HANDLE);

    impl Drop for OwnedNCryptKey {
        fn drop(&mut self) {
            // SAFETY: this guard exclusively owns the CNG key handle.
            let _ = unsafe { NCryptFreeObject(self.0.into()) };
        }
    }

    fn wincrypt_open_local_machine_my() -> anyhow::Result<OwnedCertStore> {
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

        Ok(OwnedCertStore(store))
    }

    fn wincrypt_find_cert_by_subject(subject_name: &str) -> anyhow::Result<bool> {
        let store = wincrypt_open_local_machine_my()?;

        let subject_wide: Vec<u16> = subject_name.encode_utf16().chain(Some(0)).collect();

        // SAFETY: `store` is a valid store handle.
        let found: *const CERT_CONTEXT = unsafe {
            CertFindCertificateInStore(
                store.0,
                X509_ASN_ENCODING | PKCS_7_ASN_ENCODING,
                0,
                CERT_FIND_SUBJECT_STR_W,
                Some(subject_wide.as_ptr().cast()),
                None,
            )
        };

        let exists = !found.is_null();
        let _found = exists.then_some(OwnedCertContext(found));

        Ok(exists)
    }

    fn wincrypt_create_self_signed_machine_cert(subject_name: &str) -> anyhow::Result<()> {
        let store = wincrypt_open_local_machine_my()?;

        // Create/overwrite a persisted machine key.
        let mut provider: NCRYPT_PROV_HANDLE = NCRYPT_PROV_HANDLE::default();
        // SAFETY: `provider` is a valid out-pointer and `MS_KEY_STORAGE_PROVIDER` is a valid null-terminated string.
        unsafe { NCryptOpenStorageProvider(&mut provider, MS_KEY_STORAGE_PROVIDER, 0) }
            .context("NCryptOpenStorageProvider failed")?;
        let provider = OwnedNCryptProvider(provider);

        let key_name_wide: Vec<u16> = TLS_KEY_NAME.encode_utf16().chain(Some(0)).collect();
        let mut key = NCRYPT_KEY_HANDLE::default();

        // SAFETY: all pointers are valid for the duration of the call; `key` is a valid out-handle.
        unsafe {
            NCryptCreatePersistedKey(
                provider.0,
                &mut key,
                BCRYPT_RSA_ALGORITHM,
                PCWSTR(key_name_wide.as_ptr()),
                windows::Win32::Security::Cryptography::CERT_KEY_SPEC(0),
                NCRYPT_MACHINE_KEY_FLAG,
            )
        }
        .context("NCryptCreatePersistedKey failed")?;
        let key = OwnedNCryptKey(key);

        let key_len: u32 = 2048;
        // SAFETY: `key` is a valid key handle and the property buffer is valid for the call.
        unsafe {
            NCryptSetProperty(
                NCRYPT_HANDLE::from(key.0),
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
                NCRYPT_HANDLE::from(key.0),
                NCRYPT_EXPORT_POLICY_PROPERTY,
                &export_policy.to_le_bytes(),
                NCRYPT_FLAGS(0),
            )
        }
        .context("NCryptSetProperty(NCRYPT_EXPORT_POLICY_PROPERTY) failed")?;

        // SAFETY: `key` is a valid key handle created above.
        unsafe { NCryptFinalizeKey(key.0, NCRYPT_FLAGS(0)) }.context("NCryptFinalizeKey failed")?;

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
            anyhow::bail!("CertCreateSelfSignCertificate returned null");
        }
        let cert_ctx = OwnedCertContext(cert_ctx.cast_const());

        // SAFETY: store and cert_ctx are valid.
        unsafe { CertAddCertificateContextToStore(Some(store.0), cert_ctx.0, CERT_STORE_ADD_REPLACE_EXISTING, None) }
            .context("CertAddCertificateContextToStore failed")?;

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

        if let Some(config) = capture_helper_mode {
            let helper_token = capture_helper_token_from_env()?;
            apply_client_session_preferences(config.performance_flags, config.keyboard_layout);
            match config.mode {
                CaptureHelperMode::Tcp {
                    connect_addr,
                    input_connect_addr,
                } => {
                    info!(
                        connect_addr = %connect_addr,
                        input_connect_addr = %input_connect_addr,
                        "Starting capture helper mode (tcp)"
                    );
                    return run_capture_helper_tcp(connect_addr, input_connect_addr, &helper_token).await;
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
                    return run_capture_helper_shared_mem(&map_name, &event_name, input_connect_addr, &helper_token)
                        .await;
                }
                CaptureHelperMode::Clipboard { connect_addr } => {
                    let mut stream = TcpStream::connect(connect_addr)
                        .await
                        .context("connect clipboard helper to service")?;
                    authenticate_capture_helper_stream(&mut stream, &helper_token, "clipboard").await?;
                    return crate::clipboard::run_helper(stream).await;
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
                        if !guard.register_accepted(accepted) {
                            continue;
                        }
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
                        if guard.pending_broken.len() >= MAX_PENDING_NOTIFICATIONS {
                            guard.pending_broken.pop_front();
                            warn!(
                                max_pending = MAX_PENDING_NOTIFICATIONS,
                                "Dropping oldest broken-connection notification"
                            );
                        }
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
        let first_server =
            create_control_pipe_server(&full_pipe_name, true).context("failed to claim control-pipe name")?;

        // Multiple pipe server instances reduce `ERROR_PIPE_BUSY` under concurrent provider calls.
        // Each instance independently serves one client connection at a time.
        {
            let control_plane = control_plane.clone();
            let full_pipe_name = full_pipe_name.clone();
            let empty_disconnects = Arc::clone(&empty_disconnects);
            tokio::task::spawn_local(async move {
                run_control_pipe_instance_loop(&full_pipe_name, control_plane, empty_disconnects, Some(first_server))
                    .await;
            });
        }

        for _ in 1..CONTROL_PIPE_SERVER_INSTANCES {
            let control_plane = control_plane.clone();
            let full_pipe_name = full_pipe_name.clone();
            let empty_disconnects = Arc::clone(&empty_disconnects);
            tokio::task::spawn_local(async move {
                run_control_pipe_instance_loop(&full_pipe_name, control_plane, empty_disconnects, None).await;
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
        let connection_permits = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));

        let mut next_connection_id: u32 = 1;

        loop {
            let permit = Arc::clone(&connection_permits)
                .acquire_owned()
                .await
                .context("standalone connection limiter closed")?;
            let (stream, peer_addr) = listener.accept().await.context("standalone listener accept failed")?;

            let connection_id = next_connection_id;
            next_connection_id = next_connection_id.checked_add(1).unwrap_or(1);

            let peer_addr = peer_addr.to_string();

            info!(connection_id, peer_addr = %peer_addr, "Client accepted");

            tokio::task::spawn_local(async move {
                let _permit = permit;
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
                    None,
                    None,
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

    struct CaptureHelperConfig {
        mode: CaptureHelperMode,
        performance_flags: Option<u32>,
        keyboard_layout: Option<u32>,
    }

    enum CaptureHelperMode {
        Clipboard {
            connect_addr: SocketAddr,
        },
        Tcp {
            connect_addr: SocketAddr,
            input_connect_addr: SocketAddr,
        },
        SharedMem {
            map_name: String,
            event_name: String,
            input_connect_addr: SocketAddr,
        },
    }

    fn parse_capture_helper_mode() -> anyhow::Result<Option<CaptureHelperConfig>> {
        let mut args = std::env::args().skip(1);

        let mut capture_helper = false;
        let mut connect: Option<SocketAddr> = None;
        let mut input_connect: Option<SocketAddr> = None;
        let mut map_name: Option<String> = None;
        let mut event_name: Option<String> = None;
        let mut clipboard_connect: Option<SocketAddr> = None;
        let mut performance_flags: Option<u32> = None;
        let mut keyboard_layout: Option<u32> = None;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--capture-helper" => {
                    capture_helper = true;
                }
                "--clipboard-connect" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow!("--clipboard-connect requires a value"))?;
                    clipboard_connect = Some(value.parse().context("invalid clipboard helper address")?);
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
                "--performance-flags" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow!("--performance-flags requires a value"))?;
                    performance_flags = Some(value.parse().context("invalid --performance-flags value")?);
                }
                "--keyboard-layout" => {
                    let value = args
                        .next()
                        .ok_or_else(|| anyhow!("--keyboard-layout requires a value"))?;
                    keyboard_layout = Some(value.parse().context("invalid --keyboard-layout value")?);
                }
                _ => {}
            }
        }

        if !capture_helper {
            return Ok(None);
        }

        if let Some(connect_addr) = clipboard_connect {
            anyhow::ensure!(
                connect.is_none() && input_connect.is_none() && map_name.is_none() && event_name.is_none(),
                "--clipboard-connect cannot be combined with capture arguments"
            );
            anyhow::ensure!(
                connect_addr.ip().is_loopback(),
                "clipboard helper requires a loopback address"
            );
            return Ok(Some(CaptureHelperConfig {
                mode: CaptureHelperMode::Clipboard { connect_addr },
                performance_flags,
                keyboard_layout,
            }));
        }

        let input_connect_addr = input_connect.ok_or_else(|| anyhow!("--capture-helper requires --input-connect"))?;

        if let Some(connect) = connect {
            if map_name.is_some() || event_name.is_some() {
                return Err(anyhow!("--connect cannot be combined with --shm-map/--shm-event"));
            }
            return Ok(Some(CaptureHelperConfig {
                mode: CaptureHelperMode::Tcp {
                    connect_addr: connect,
                    input_connect_addr,
                },
                performance_flags,
                keyboard_layout,
            }));
        }

        let map_name =
            map_name.ok_or_else(|| anyhow!("--capture-helper requires --connect or --shm-map/--shm-event"))?;
        let event_name = event_name.ok_or_else(|| anyhow!("--capture-helper requires --shm-event"))?;
        Ok(Some(CaptureHelperConfig {
            mode: CaptureHelperMode::SharedMem {
                map_name,
                event_name,
                input_connect_addr,
            },
            performance_flags,
            keyboard_layout,
        }))
    }

    #[derive(Debug, Clone)]
    enum HelperInputEvent {
        ScancodeKey { code: u8, extended: bool, released: bool },
        UnicodeKey { ch: u16, released: bool },
        MouseMoveAbs { x: u16, y: u16 },
        MouseMoveRel { dx: i32, dy: i32 },
        MouseButton { button: u8, down: bool },
        MouseWheel { delta: i32 },
        MouseHWheel { delta: i32 },
        RdpeiReady(CsReadyPdu),
        Touch(TouchEventPdu),
        DismissHovering(DismissHoveringTouchContactPdu),
        Pen(PenEventPdu),
        PointerReset,
    }

    async fn read_helper_input_event(
        stream: &mut (dyn tokio::io::AsyncRead + Unpin + Send),
    ) -> anyhow::Result<HelperInputEvent> {
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
        let payload_len = usize::from(u16::from_le_bytes([header[7], header[8]]));

        let mut payload = vec![0u8; payload_len];
        stream
            .read_exact(&mut payload)
            .await
            .context("failed to read input payload")?;

        match msg_type {
            INPUT_MSG_RDPEI_READY => Ok(HelperInputEvent::RdpeiReady(ironrdp_core::decode(&payload)?)),
            INPUT_MSG_TOUCH => Ok(HelperInputEvent::Touch(ironrdp_core::decode(&payload)?)),
            INPUT_MSG_DISMISS_HOVERING => Ok(HelperInputEvent::DismissHovering(ironrdp_core::decode(&payload)?)),
            INPUT_MSG_PEN => Ok(HelperInputEvent::Pen(ironrdp_core::decode(&payload)?)),
            INPUT_MSG_POINTER_RESET => {
                anyhow::ensure!(payload.is_empty(), "invalid pointer reset payload length");
                Ok(HelperInputEvent::PointerReset)
            }
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

    fn inject_helper_input_event(input_marker: usize, event: HelperInputEvent) -> anyhow::Result<()> {
        match event {
            HelperInputEvent::RdpeiReady(_)
            | HelperInputEvent::Touch(_)
            | HelperInputEvent::DismissHovering(_)
            | HelperInputEvent::Pen(_)
            | HelperInputEvent::PointerReset => {
                anyhow::bail!("RDPEI input requires the session pointer injector");
            }
            HelperInputEvent::MouseMoveAbs { x, y } => {
                let size = desktop_size_from_gdi().context("query mouse coordinate space")?;
                let (width, height) = desktop_size_nonzero(size)?;
                // Pixel centers avoid rounding to an adjacent pixel in the normalized coordinate space.
                let dx = (u32::from(x.min(width.get() - 1)) * 65536 + 32768) / NonZeroU32::from(width);
                let dy = (u32::from(y.min(height.get() - 1)) * 65536 + 32768) / NonZeroU32::from(height);
                let input = INPUT {
                    r#type: INPUT_MOUSE,
                    Anonymous: INPUT_0 {
                        mi: MOUSEINPUT {
                            dx: i32::try_from(dx)?,
                            dy: i32::try_from(dy)?,
                            dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                            dwExtraInfo: input_marker,
                            ..Default::default()
                        },
                    },
                };
                // SAFETY: the input structure is initialized and its size is correct.
                let sent = unsafe { SendInput(&[input], i32::try_from(size_of::<INPUT>())?) };
                anyhow::ensure!(sent == 1, "SendInput did not inject absolute mouse movement");
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
                            dwExtraInfo: input_marker,
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
                            dwExtraInfo: input_marker,
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
                            dwExtraInfo: input_marker,
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
                            dwExtraInfo: input_marker,
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

    async fn run_input_injector(input_marker: usize, mut stream: tokio::net::tcp::OwnedReadHalf) -> anyhow::Result<()> {
        // Track modifier state so we can translate Ctrl+Alt+End into a real SAS.
        // This is necessary because Winlogon often ignores normal injected key sequences.
        let mut ctrl_down = false;
        let mut alt_down = false;
        let mut pointer = crate::pointer::Input::default();

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
                        let _ = inject_helper_input_event(
                            input_marker,
                            HelperInputEvent::ScancodeKey {
                                code: SCANCODE_ALT,
                                extended: false,
                                released: true,
                            },
                        );
                        let _ = inject_helper_input_event(
                            input_marker,
                            HelperInputEvent::ScancodeKey {
                                code: SCANCODE_CTRL,
                                extended: false,
                                released: true,
                            },
                        );
                        ctrl_down = false;
                        alt_down = false;
                        if tokio::task::spawn_blocking(|| try_send_sas(None, "input_ctrl_alt_end"))
                            .await
                            .unwrap_or(false)
                        {
                            continue;
                        }
                    }
                    _ => {}
                }
            }

            let result = match event {
                HelperInputEvent::RdpeiReady(ready) => pointer.initialize(ready),
                HelperInputEvent::Touch(touch) => pointer.touch(touch).await,
                HelperInputEvent::DismissHovering(dismiss) => pointer.dismiss(dismiss.contact_id).await,
                HelperInputEvent::Pen(pen) => pointer.pen(pen).await,
                HelperInputEvent::PointerReset => {
                    pointer = crate::pointer::Input::default();
                    Ok(())
                }
                event => {
                    let first = inject_helper_input_event(input_marker, event.clone());
                    if first.is_err() {
                        if !sync_thread_to_input_desktop() {
                            pointer = crate::pointer::Input::default();
                            let _ = sync_thread_to_input_desktop();
                        }
                        inject_helper_input_event(input_marker, event)
                    } else {
                        first
                    }
                }
            };
            if let Err(error) = result {
                warn!(error = %format!("{error:#}"), "Failed to inject input event");
                pointer.reset();
            }
        }
    }

    fn spi_set_bool_pv(action: u32, enabled: bool) {
        const SPIF_SENDCHANGE: u32 = 0x0002;
        let mut value = i32::from(enabled);
        // SAFETY: `value` is a live `BOOL` out/in buffer for the BOOL-pointer SPI action.
        let _ = unsafe {
            SystemParametersInfoW(
                action,
                0,
                core::ptr::from_mut(&mut value).cast::<c_void>(),
                SPIF_SENDCHANGE,
            )
        };
    }

    fn spi_set_bool_ui(action: u32, enabled: bool) {
        const SPIF_SENDCHANGE: u32 = 0x0002;
        // SAFETY: This SPI action reads `ui_param` as a BOOL and ignores `pv_param`.
        let _ = unsafe { SystemParametersInfoW(action, u32::from(enabled), null_mut(), SPIF_SENDCHANGE) };
    }

    fn impersonate_current_session_user() -> Option<(PipeClientImpersonation, OwnedHandle)> {
        let session_id = current_process_session_id().filter(|&id| id != 0)?;
        let _ = enable_privilege(w!("SeTcbPrivilege"));
        let mut user_token = HANDLE::default();
        // SAFETY: `user_token` is a writable output handle for the current session's user token.
        unsafe { WTSQueryUserToken(session_id, &mut user_token) }.ok()?;
        let token_guard = OwnedHandle(user_token);
        // SAFETY: `user_token` is a valid primary token returned by `WTSQueryUserToken`.
        unsafe { ImpersonateLoggedOnUser(token_guard.0) }.ok()?;
        Some((PipeClientImpersonation, token_guard))
    }

    fn apply_client_session_preferences(performance_flags: Option<u32>, keyboard_layout: Option<u32>) {
        use ironrdp_pdu::rdp::client_info::PerformanceFlags;

        const SPI_SETDESKWALLPAPER: u32 = 0x0014;
        const SPI_SETDRAGFULLWINDOWS: u32 = 0x0025;
        const SPI_SETANIMATION: u32 = 0x0049;
        const SPI_SETFONTSMOOTHING: u32 = 0x004B;
        const SPI_SETDEFAULTINPUTLANG: u32 = 0x005A;
        const SPI_GETDESKWALLPAPER: u32 = 0x0073;
        const SPI_SETMENUANIMATION: u32 = 0x1003;
        const SPI_SETCOMBOBOXANIMATION: u32 = 0x1005;
        const SPI_SETLISTBOXSMOOTHSCROLLING: u32 = 0x1007;
        const SPI_SETSELECTIONFADE: u32 = 0x1015;
        const SPI_SETTOOLTIPANIMATION: u32 = 0x1017;
        const SPI_SETCURSORSHADOW: u32 = 0x101B;
        const SPI_SETDROPSHADOW: u32 = 0x1025;
        const SPI_SETUIEFFECTS: u32 = 0x103F;
        const SPI_SETCLIENTAREAANIMATION: u32 = 0x1043;
        const SPIF_SENDCHANGE: u32 = 0x0002;

        #[repr(C)]
        struct AnimationInfo {
            cb_size: u32,
            i_min_animate: i32,
        }

        let _user_impersonation = impersonate_current_session_user();

        if let Some(bits) = performance_flags {
            let flags = PerformanceFlags::from_bits_retain(bits);
            spi_set_bool_ui(
                SPI_SETDRAGFULLWINDOWS,
                !flags.contains(PerformanceFlags::DISABLE_FULLWINDOWDRAG),
            );

            let menu_animations = !flags.contains(PerformanceFlags::DISABLE_MENUANIMATIONS);
            for action in [
                SPI_SETMENUANIMATION,
                SPI_SETCOMBOBOXANIMATION,
                SPI_SETLISTBOXSMOOTHSCROLLING,
                SPI_SETSELECTIONFADE,
                SPI_SETTOOLTIPANIMATION,
                SPI_SETCLIENTAREAANIMATION,
            ] {
                spi_set_bool_pv(action, menu_animations);
            }

            let mut anim = AnimationInfo {
                cb_size: u32::try_from(size_of::<AnimationInfo>()).unwrap_or(8),
                i_min_animate: i32::from(menu_animations),
            };
            // SAFETY: `anim` is a properly sized `ANIMATIONINFO` struct for `SPI_SETANIMATION`.
            let _ = unsafe {
                SystemParametersInfoW(
                    SPI_SETANIMATION,
                    anim.cb_size,
                    core::ptr::from_mut(&mut anim).cast::<c_void>(),
                    SPIF_SENDCHANGE,
                )
            };

            spi_set_bool_pv(SPI_SETUIEFFECTS, !flags.contains(PerformanceFlags::DISABLE_THEMING));

            let cursor_shadow = !flags.contains(PerformanceFlags::DISABLE_CURSOR_SHADOW);
            spi_set_bool_pv(SPI_SETCURSORSHADOW, cursor_shadow);
            spi_set_bool_pv(SPI_SETDROPSHADOW, cursor_shadow);

            let blink_ms = if flags.contains(PerformanceFlags::DISABLE_CURSORSETTINGS) {
                u32::MAX
            } else {
                530
            };
            // SAFETY: `SetCaretBlinkTime` takes a plain millisecond count with no pointers.
            let _ = unsafe { SetCaretBlinkTime(blink_ms) };

            spi_set_bool_ui(
                SPI_SETFONTSMOOTHING,
                flags.contains(PerformanceFlags::ENABLE_FONT_SMOOTHING),
            );

            if flags.contains(PerformanceFlags::DISABLE_WALLPAPER) {
                let mut empty = [0u16; 1];
                // SAFETY: `empty` is a NUL-terminated UTF-16 string clearing the in-memory wallpaper.
                let _ = unsafe {
                    SystemParametersInfoW(
                        SPI_SETDESKWALLPAPER,
                        0,
                        empty.as_mut_ptr().cast::<c_void>(),
                        SPIF_SENDCHANGE,
                    )
                };
            } else {
                let mut current_wallpaper = [0u16; 260];
                // SAFETY: `current_wallpaper` is a writable UTF-16 buffer of `260` `WCHAR`s.
                let queried = unsafe {
                    SystemParametersInfoW(
                        SPI_GETDESKWALLPAPER,
                        u32::try_from(current_wallpaper.len()).unwrap_or(260),
                        current_wallpaper.as_mut_ptr().cast::<c_void>(),
                        0,
                    )
                };
                if queried != 0 && current_wallpaper[0] == 0 {
                    // SAFETY: A null `pv_param` reloads the impersonated user's configured wallpaper from HKCU.
                    let _ = unsafe { SystemParametersInfoW(SPI_SETDESKWALLPAPER, 0, null_mut(), SPIF_SENDCHANGE) };
                }
            }

            info!(
                performance_flags = format_args!("0x{bits:08x}"),
                "Applied client RDP performance flags"
            );
        }

        if let Some(klid) = keyboard_layout.filter(|&id| id != 0) {
            const KLF_ACTIVATE: u32 = 0x0000_0001;
            const KLF_SUBSTITUTE_OK: u32 = 0x0000_0002;
            const KLF_SETFORPROCESS: u32 = 0x0000_0100;
            const WM_INPUTLANGCHANGEREQUEST: u32 = 0x0050;

            let klid_w: Vec<u16> = format!("{klid:08x}").encode_utf16().chain(Some(0)).collect();
            // SAFETY: `klid_w` is a NUL-terminated 8-digit hex keyboard layout ID.
            let mut hkl =
                unsafe { LoadKeyboardLayoutW(klid_w.as_ptr(), KLF_ACTIVATE | KLF_SUBSTITUTE_OK | KLF_SETFORPROCESS) };
            if !hkl.is_null() {
                // SAFETY: `hkl` is a valid keyboard layout handle returned by `LoadKeyboardLayoutW`.
                let _ = unsafe { ActivateKeyboardLayout(hkl, KLF_SETFORPROCESS) };
                // SAFETY: `hkl` is passed by pointer as required by `SPI_SETDEFAULTINPUTLANG`.
                let _ = unsafe {
                    SystemParametersInfoW(
                        SPI_SETDEFAULTINPUTLANG,
                        0,
                        core::ptr::addr_of_mut!(hkl).cast::<c_void>(),
                        SPIF_SENDCHANGE,
                    )
                };
                // SAFETY: `HWND_BROADCAST` (0xFFFF) broadcasts the non-blocking input-language change request.
                let _ = unsafe { SendNotifyMessageW(0xFFFF, WM_INPUTLANGCHANGEREQUEST, 0, hkl) };
                info!(
                    keyboard_layout = format_args!("0x{klid:08x}"),
                    "Activated client keyboard layout"
                );
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

    fn desktop_object_name(desktop: *mut c_void) -> Option<String> {
        const UOI_NAME: i32 = 2;
        if desktop.is_null() {
            return None;
        }
        let mut buf = [0u16; 128];
        let mut needed = 0u32;
        // SAFETY: `desktop` is a desktop handle and `buf` is writable for `size_of_val(&buf)` bytes.
        let ok = unsafe {
            GetUserObjectInformationW(
                desktop,
                UOI_NAME,
                buf.as_mut_ptr().cast::<c_void>(),
                u32::try_from(size_of_val(&buf)).unwrap_or(0),
                &mut needed,
            )
        };
        if ok == 0 {
            return None;
        }
        let end = buf.iter().position(|&ch| ch == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }

    fn sync_thread_to_input_desktop() -> bool {
        const DESKTOP_ACCESS: u32 = 0x01FF;
        thread_local! {
            static BOUND_DESKTOP: core::cell::Cell<*mut c_void> = const { core::cell::Cell::new(null_mut()) };
        }

        // SAFETY: OpenInputDesktop has no pointer preconditions.
        let input_desktop = unsafe { OpenInputDesktop(0, 0, DESKTOP_ACCESS) };
        if input_desktop.is_null() {
            return false;
        }

        // SAFETY: GetCurrentThreadId has no preconditions.
        let thread_id = unsafe { GetCurrentThreadId() };
        // SAFETY: GetThreadDesktop has no pointer preconditions; the returned handle is owned by
        // the system and must not be closed.
        let current_desktop = unsafe { GetThreadDesktop(thread_id) };
        let current_name = desktop_object_name(current_desktop);
        let input_name = desktop_object_name(input_desktop);

        if current_name
            .as_deref()
            .zip(input_name.as_deref())
            .is_some_and(|(cur, inp)| cur.eq_ignore_ascii_case(inp))
        {
            // SAFETY: `input_desktop` was not assigned to any thread and must be closed.
            let _ = unsafe { CloseDesktop(input_desktop) };
            return true;
        }

        // SAFETY: `input_desktop` is a valid desktop handle returned by OpenInputDesktop.
        let switched = unsafe { SetThreadDesktop(input_desktop) } != 0;
        if switched {
            let prev = BOUND_DESKTOP.with(|slot| slot.replace(input_desktop));
            if !prev.is_null() {
                // SAFETY: `prev` was previously opened by OpenInputDesktop and is no longer the thread desktop.
                let _ = unsafe { CloseDesktop(prev) };
            }
            info!(
                from = ?current_name,
                to = ?input_name,
                "Switched capture helper thread to active input desktop"
            );
            true
        } else {
            // SAFETY: SetThreadDesktop failed, so `input_desktop` is not bound to this thread and must be closed.
            let _ = unsafe { CloseDesktop(input_desktop) };
            false
        }
    }

    fn try_send_sas(session_id: Option<u32>, source: &'static str) -> bool {
        const WMSG_SAS: u32 = 0x208;
        type WmsgSendMessageFn =
            unsafe extern "system" fn(session_id: u32, msg: u32, wparam: u32, response: *mut u32) -> u32;

        let target_session_id = session_id
            .or_else(|| current_process_session_id().filter(|&id| id != 0))
            // SAFETY: WTSGetActiveConsoleSessionId has no preconditions.
            .unwrap_or_else(|| unsafe { WTSGetActiveConsoleSessionId() });

        // SAFETY: The literal is a NUL-terminated DLL name.
        let module = unsafe { LoadLibraryA(c"wmsgapi.dll".as_ptr().cast::<u8>()) };
        if module.is_null() {
            // SAFETY: GetLastError reads the calling thread's last-error value.
            let err = unsafe { GetLastError() };
            warn!(
                error = ?err,
                session_id = target_session_id,
                source = %source,
                "Failed to load wmsgapi.dll for SAS"
            );
            return false;
        }

        // SAFETY: `module` is a loaded module handle and the symbol name is NUL-terminated.
        let proc = unsafe { GetProcAddress(module, c"WmsgSendMessage".as_ptr().cast::<u8>()) };
        let result = if proc.is_null() {
            // SAFETY: GetLastError reads the calling thread's last-error value.
            let err = unsafe { GetLastError() };
            warn!(
                error = ?err,
                session_id = target_session_id,
                source = %source,
                "WmsgSendMessage not found in wmsgapi.dll"
            );
            None
        } else {
            // SAFETY: `WmsgSendMessage` in `wmsgapi.dll` has the 4-argument x64 stdcall/system ABI.
            let send_msg: WmsgSendMessageFn = unsafe { core::mem::transmute(proc) };
            let mut response = 0u32;
            // SAFETY: `response` is a writable `u32` out-parameter for the synchronous LRPC call.
            let status = unsafe { send_msg(target_session_id, WMSG_SAS, 0, &mut response) };
            Some((status, response))
        };
        // SAFETY: `module` was returned by a successful `LoadLibraryA` above.
        let _ = unsafe { FreeLibrary(module) };
        let Some((status, response)) = result else {
            return false;
        };

        if status == 0 && response == 0 {
            info!(
                session_id = target_session_id,
                source = %source,
                "Generated Secure Attention Sequence (SAS)"
            );
            true
        } else {
            warn!(
                session_id = target_session_id,
                status,
                response,
                source = %source,
                "WmsgSendMessage(SAS) failed"
            );
            false
        }
    }

    async fn run_capture_helper_tcp(
        connect_addr: SocketAddr,
        input_connect_addr: SocketAddr,
        helper_token: &str,
    ) -> anyhow::Result<()> {
        info!(
            pid = std::process::id(),
            // SAFETY: WTSGetActiveConsoleSessionId has no input parameters.
            active_console_session_id = unsafe { WTSGetActiveConsoleSessionId() },
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
        authenticate_capture_helper_stream(&mut stream, helper_token, "capture").await?;

        let mut input_stream = match TcpStream::connect(input_connect_addr).await {
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
                return Err(error)
                    .with_context(|| format!("failed to connect to input consumer at {input_connect_addr}"));
            }
        };
        authenticate_capture_helper_stream(&mut input_stream, helper_token, "input").await?;
        let input_marker = usize::try_from(
            input_stream
                .read_u64_le()
                .await
                .context("read connection input marker")?,
        )?;
        anyhow::ensure!(input_marker != 0, "input marker must be nonzero");

        let mut input_stream = Some(input_stream);
        let mut cursor_stream = None;
        let mut last_cursor = None;
        let mut last_frame: Option<BitmapUpdate> = None;
        let mut blank_since: Option<Instant> = None;
        let mut cursor_capture = crate::cursor::Capture::new(input_marker);

        if auto_send_sas_enabled() {
            for attempt in 1..=5 {
                if try_send_sas(None, "capture_helper_startup") {
                    break;
                }

                if attempt < 5 {
                    sleep(Duration::from_millis(500)).await;
                }
            }
        }

        let _ = sync_thread_to_input_desktop();
        let desktop_size = desktop_size_from_gdi().context("failed to query desktop size")?;
        info!(
            width = desktop_size.width,
            height = desktop_size.height,
            "Initialized capture helper desktop size"
        );

        loop {
            let _ = sync_thread_to_input_desktop();
            let desktop_size = desktop_size_from_gdi().context("failed to query current desktop size")?;
            let captured = match capture_bitmap_update(desktop_size) {
                Ok(bitmap) => Ok(bitmap),
                Err(_) if sync_thread_to_input_desktop() => capture_bitmap_update(desktop_size),
                Err(error) => Err(error),
            };
            match captured {
                Ok(bitmap) => {
                    if let Some(input_stream) = input_stream.take() {
                        cursor_stream = Some(enable_helper_input(input_marker, input_stream).await?);
                    }
                    let is_blank = is_probably_blank_bgra32(bitmap.data.as_ref());
                    let suppress_transient_blank = if is_blank && last_frame.is_some() {
                        let start = blank_since.get_or_insert_with(Instant::now);
                        start.elapsed() < FIRST_FRAME_BLANK_GRACE
                    } else {
                        blank_since = None;
                        false
                    };
                    let unchanged = !is_blank
                        && last_frame.as_ref().is_some_and(|prev| {
                            prev.width == bitmap.width
                                && prev.height == bitmap.height
                                && prev.data.as_ref() == bitmap.data.as_ref()
                        });
                    if !unchanged && !suppress_transient_blank {
                        // RemoteFX encoding stays in the server process, where the
                        // client-selected codec ID and entropy coder are available.
                        write_capture_frame(&mut stream, &bitmap).await?;
                        if !is_blank {
                            last_frame = Some(bitmap);
                        }
                    }
                    let cursor = cursor_capture.capture(desktop_size);
                    if last_cursor.as_ref() != Some(&cursor) {
                        cursor
                            .write(cursor_stream.as_mut().expect("input enabled after capture"))
                            .await?;
                        last_cursor = Some(cursor);
                    }
                    sleep(CAPTURE_INTERVAL).await;
                }
                Err(error) => {
                    warn!(
                        error = %format!("{error:#}"),
                        "Capture helper failed to capture frame"
                    );
                    if last_frame.is_none() {
                        let bitmap = fallback_bitmap_update(desktop_size)
                            .context("failed to generate fallback bitmap in capture helper")?;
                        maybe_dump_bitmap_update_bgra32(
                            bitmap.width,
                            bitmap.height,
                            bitmap.stride,
                            bitmap.data.as_ref(),
                        );
                        write_capture_frame(&mut stream, &bitmap).await?;
                    }
                    sleep(CAPTURE_INTERVAL).await;
                }
            }
        }
    }

    async fn run_capture_helper_shared_mem(
        map_name: &str,
        event_name: &str,
        input_connect_addr: SocketAddr,
        helper_token: &str,
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
                    let _ = CloseHandle(self.0);
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

        let mut input_stream = TcpStream::connect(input_connect_addr)
            .await
            .with_context(|| format!("failed to connect to input consumer at {input_connect_addr}"))?;
        authenticate_capture_helper_stream(&mut input_stream, helper_token, "input").await?;
        let input_marker = usize::try_from(
            input_stream
                .read_u64_le()
                .await
                .context("read connection input marker")?,
        )?;
        anyhow::ensure!(input_marker != 0, "input marker must be nonzero");

        let mut input_stream = Some(input_stream);
        let mut cursor_stream = None;
        let mut last_cursor = None;
        let mut last_frame: Option<BitmapUpdate> = None;
        let mut blank_since: Option<Instant> = None;
        let mut cursor_capture = crate::cursor::Capture::new(input_marker);

        if auto_send_sas_enabled() {
            for attempt in 1..=5 {
                if try_send_sas(None, "capture_helper_startup") {
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
            let _ = sync_thread_to_input_desktop();
            let actual = desktop_size_from_gdi().context("failed to query current desktop size")?;
            let size = u32::from(actual.width) | (u32::from(actual.height) << 16);
            // SAFETY: the aligned size record is inside the mapped header.
            unsafe { shm_write_u32(view, SHM_OFF_DISPLAY_SIZE, size, view_len)? };
            if actual != desktop_size {
                // SAFETY: the event is live; the consumer will resize and replace this fixed-size mapping.
                unsafe { SetEvent(frame_ready_event) }.context("notify changed display size")?;
                sleep(CAPTURE_INTERVAL).await;
                continue;
            }
            let captured = match capture_bitmap_update(desktop_size) {
                Ok(bitmap) => Ok(bitmap),
                Err(_) if sync_thread_to_input_desktop() => capture_bitmap_update(desktop_size),
                Err(error) => Err(error),
            };
            match captured {
                Ok(bitmap) => {
                    if let Some(input_stream) = input_stream.take() {
                        cursor_stream = Some(enable_helper_input(input_marker, input_stream).await?);
                    }
                    let is_blank = is_probably_blank_bgra32(bitmap.data.as_ref());
                    let suppress_transient_blank = if is_blank && last_frame.is_some() {
                        let start = blank_since.get_or_insert_with(Instant::now);
                        start.elapsed() < FIRST_FRAME_BLANK_GRACE
                    } else {
                        blank_since = None;
                        false
                    };
                    let unchanged = !is_blank
                        && last_frame.as_ref().is_some_and(|prev| {
                            prev.width == bitmap.width
                                && prev.height == bitmap.height
                                && prev.data.as_ref() == bitmap.data.as_ref()
                        });
                    if !unchanged && !suppress_transient_blank {
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
                        if !is_blank {
                            last_frame = Some(bitmap);
                        }
                    }
                    let cursor = cursor_capture.capture(desktop_size);
                    if last_cursor.as_ref() != Some(&cursor) {
                        cursor
                            .write(cursor_stream.as_mut().expect("input enabled after capture"))
                            .await?;
                        last_cursor = Some(cursor);
                    }

                    // Important: yield/throttle so the input injector task can run.
                    sleep(CAPTURE_INTERVAL).await;
                }
                Err(error) => {
                    warn!(
                        error = %format!("{error:#}"),
                        "Capture helper failed to capture frame"
                    );
                    if last_frame.is_none() {
                        let bitmap = fallback_bitmap_update(desktop_size)
                            .context("failed to generate fallback bitmap in shared-memory capture helper")?;
                        maybe_dump_bitmap_update_bgra32(
                            bitmap.width,
                            bitmap.height,
                            bitmap.stride,
                            bitmap.data.as_ref(),
                        );

                        seq = seq.wrapping_add(1);
                        slot_idx = (slot_idx + 1) % SHM_FB_SLOTS;

                        // SAFETY: `view` is the live mapped framebuffer and all lengths come from its validated header.
                        unsafe { shm_publish_frame(view, view_len, slot_idx, slot_len, seq, bitmap.data.as_ref())? };

                        // SAFETY: `frame_ready_event` is a live event handle opened above.
                        unsafe { SetEvent(frame_ready_event) }
                            .map_err(|set_event_error| anyhow!("SetEvent failed: {set_event_error}"))
                            .context("SetEvent failed")?;
                    }
                    sleep(CAPTURE_INTERVAL).await;
                }
            }
        }
    }

    async fn enable_helper_input(
        input_marker: usize,
        mut stream: TcpStream,
    ) -> anyhow::Result<tokio::net::tcp::OwnedWriteHalf> {
        stream
            .write_u8(INPUT_DESKTOP_READY)
            .await
            .context("signal input desktop readiness")?;
        let (input, cursor) = stream.into_split();
        tokio::task::spawn_local(async move {
            if let Err(error) = run_input_injector(input_marker, input).await {
                warn!(error = %format!("{error:#}"), "Input injector task stopped");
            }
        });
        Ok(cursor)
    }

    fn create_control_pipe_server(
        full_pipe_name: &str,
        first_instance: bool,
    ) -> anyhow::Result<named_pipe::NamedPipeServer> {
        let mut opts = named_pipe::ServerOptions::new();
        opts.access_inbound(true)
            .access_outbound(true)
            .in_buffer_size(PIPE_BUFFER_SIZE)
            .out_buffer_size(PIPE_BUFFER_SIZE)
            .pipe_mode(named_pipe::PipeMode::Byte);
        if first_instance {
            opts.first_pipe_instance(true);
        }

        // Only LocalSystem and NetworkService host the authenticated TermService peer.
        let (mut attrs, sd) = security_attributes(w!("D:P(A;;GA;;;SY)(A;;GA;;;NS)"))?;
        // SAFETY: `attrs` is valid for this synchronous call and CreateNamedPipe copies the descriptor.
        let result = unsafe {
            opts.create_with_security_attributes_raw(full_pipe_name, core::ptr::from_mut(&mut attrs).cast::<c_void>())
        };
        // SAFETY: frees the buffer allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        let _ = unsafe { LocalFree(Some(HLOCAL(sd.0))) };

        result.context("create control-pipe server instance")
    }

    #[expect(clippy::infinite_loop, reason = "pipe server instances run indefinitely")]
    async fn run_control_pipe_instance_loop(
        full_pipe_name: &str,
        control_plane: ControlPlane,
        empty_disconnects: Arc<AtomicU64>,
        mut initial_server: Option<named_pipe::NamedPipeServer>,
    ) {
        loop {
            let mut server = match initial_server.take() {
                Some(server) => server,
                None => match create_control_pipe_server(full_pipe_name, false) {
                    Ok(server) => server,
                    Err(error) => {
                        warn!(%error, pipe = %full_pipe_name, "Failed to create control pipe server instance; retrying");
                        sleep(Duration::from_millis(200)).await;
                        continue;
                    }
                },
            };

            if let Err(error) = server.connect().await {
                warn!(%error, pipe = %full_pipe_name, "Failed to accept control connection; retrying");
                sleep(Duration::from_millis(200)).await;
                continue;
            }

            if let Err(error) = authenticate_control_pipe_client(&server) {
                warn!(%error, pipe = %full_pipe_name, "Rejected unauthorized control-pipe client");
                continue;
            }

            let mut pending_wakeup_rx = control_plane.pending_wakeup_tx.subscribe();

            match handle_client(&mut server, &control_plane, &mut pending_wakeup_rx).await {
                Ok(0) => {
                    let n = empty_disconnects.fetch_add(1, Ordering::Relaxed).saturating_add(1);
                    if n == 1 || (n <= 10 && n.is_multiple_of(5)) || n.is_multiple_of(100_000) {
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
            ServiceEvent::RemoteAppSettings { .. } => "remote_app_settings",
            ServiceEvent::ConnectionBroken { .. } => "connection_broken",
            ServiceEvent::ConnectionCredentials { .. } => "connection_credentials",
            ServiceEvent::NoCredentials { .. } => "no_credentials",
            ServiceEvent::VirtualChannelReady { .. } => "virtual_channel_ready",
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

    #[cfg(test)]
    mod tests {
        use super::*;

        // oracle: mutant input overload must close once, not create an unbounded queue of shutdown events.
        #[test]
        fn input_overload_requests_only_one_disconnect() {
            let (tx, _rx) = mpsc::channel(1);
            let (events, mut received) = mpsc::unbounded_channel();
            let mut input = TermSrvInputHandler::new(7, tx);
            input.set_sender(events);
            input.keyboard(KeyboardEvent::Pressed {
                code: 0x1D,
                extended: false,
            });
            for _ in 0..10 {
                input.keyboard(KeyboardEvent::Released {
                    code: 0x1D,
                    extended: false,
                });
            }
            assert!(matches!(received.try_recv(), Ok(ServerEvent::Quit(_))));
            assert!(received.try_recv().is_err(), "overload queued repeated disconnects");
        }

        // oracle: spec MS-RDPBCGR §2.2.8.1.2 permits 255 events in one fast-path input PDU.
        #[test]
        fn input_queue_accepts_a_complete_fast_path_batch() {
            let (tx, _rx) = mpsc::channel(INPUT_QUEUE_CAPACITY);
            let (events, mut received) = mpsc::unbounded_channel();
            let mut input = TermSrvInputHandler::new(7, tx);
            input.set_sender(events);
            for event in 0..255 {
                input.mouse(MouseEvent::Button {
                    x: 320,
                    y: 240,
                    button: MouseButton::Left,
                    pressed: event % 2 == 0,
                });
            }
            assert!(received.try_recv().is_err(), "one valid input PDU overflowed the queue");
        }

        // oracle: mutant input accepted before helper attachment is discarded.
        #[tokio::test(flavor = "current_thread")]
        async fn input_waits_for_helper_attachment() -> anyhow::Result<()> {
            let slot: InputStreamSlot = Arc::default();
            let (tx, rx) = mpsc::channel(4);
            let mut input = TermSrvInputHandler::new(7, tx);
            let spooler = tokio::spawn(run_input_spooler(7, Arc::clone(&slot), rx));
            input.keyboard(KeyboardEvent::Pressed {
                code: 0x1E,
                extended: false,
            });
            timeout(Duration::from_secs(2), async {
                while input.tx.capacity() != 4 {
                    tokio::task::yield_now().await;
                }
            })
            .await?;

            let (writer, mut reader) = input_socket_pair().await?;
            reader.write_all(&[1]).await?;
            slot.attach(writer).await;
            let event = timeout(Duration::from_secs(2), read_helper_input_event(&mut reader)).await??;
            assert!(matches!(
                event,
                HelperInputEvent::ScancodeKey {
                    code: 0x1E,
                    extended: false,
                    released: false,
                }
            ));
            drop(input);
            timeout(Duration::from_secs(2), spooler).await??;
            Ok(())
        }

        // oracle: mutant cursor reads require an input event, lose fragmented headers, or allocate oversized frames.
        #[tokio::test]
        async fn cursor_metadata_is_bounded_and_available_without_input() -> anyhow::Result<()> {
            let (parent, mut helper) = input_socket_pair().await?;
            helper.write_all(&[1, 1, 0]).await?;
            parent.readable().await?;
            let mut input = HelperInputStream::new(parent);
            assert!(input.poll_cursor()?.is_none());
            assert!(input.desktop_ready);
            helper.write_all(&[0, 0, 0]).await?;
            input.stream.readable().await?;
            assert_eq!(input.poll_cursor()?, Some(crate::cursor::Snapshot::Hidden));
            helper.write_all(&600_000u32.to_le_bytes()).await?;
            input.stream.readable().await?;
            assert!(input.poll_cursor().is_err());
            Ok(())
        }

        // oracle: mutant input is not retained until an attached helper's native desktop becomes accessible.
        #[tokio::test(start_paused = true)]
        async fn input_waits_for_the_native_desktop() -> anyhow::Result<()> {
            let (writer, mut reader) = input_socket_pair().await?;
            let slot: InputStreamSlot = Arc::default();
            slot.attach(writer).await;
            let (tx, rx) = mpsc::channel(4);
            let mut input = TermSrvInputHandler::new(7, tx);
            let spooler = tokio::spawn(run_input_spooler(7, slot, rx));
            input.keyboard(KeyboardEvent::Pressed {
                code: 0x1E,
                extended: false,
            });
            assert!(
                timeout(Duration::from_millis(50), read_helper_input_event(&mut reader))
                    .await
                    .is_err(),
                "input reached a helper before its desktop was accessible"
            );
            tokio::time::advance(Duration::from_secs(60)).await;
            tokio::task::yield_now().await;
            tokio::time::resume();
            reader.write_all(&[1]).await?;
            let event = timeout(Duration::from_secs(2), read_helper_input_event(&mut reader)).await??;
            assert!(matches!(event, HelperInputEvent::ScancodeKey { code: 0x1E, .. }));
            drop(input);
            timeout(Duration::from_secs(2), spooler).await??;
            Ok(())
        }

        // oracle: mutant replacing a helper before desktop readiness discards input not yet delivered.
        #[tokio::test(flavor = "current_thread")]
        async fn input_survives_replacement_before_desktop_readiness() -> anyhow::Result<()> {
            let (writer, reader) = input_socket_pair().await?;
            let slot: InputStreamSlot = Arc::default();
            slot.attach(writer).await;
            let (tx, rx) = mpsc::channel(4);
            let mut input = TermSrvInputHandler::new(7, tx);
            let spooler = tokio::spawn(run_input_spooler(7, Arc::clone(&slot), rx));
            input.keyboard(KeyboardEvent::Pressed {
                code: 0x1E,
                extended: false,
            });
            timeout(Duration::from_secs(2), async {
                while input.tx.capacity() != 4 {
                    tokio::task::yield_now().await;
                }
            })
            .await?;
            drop(reader);
            let (writer, mut replacement) = input_socket_pair().await?;
            replacement.write_all(&[1]).await?;
            timeout(Duration::from_secs(2), slot.attach(writer)).await?;
            let event = timeout(Duration::from_secs(2), read_helper_input_event(&mut replacement)).await??;
            assert!(matches!(event, HelperInputEvent::ScancodeKey { code: 0x1E, .. }));
            drop(input);
            timeout(Duration::from_secs(2), spooler).await??;
            Ok(())
        }

        // oracle: mutant helper replacement loses readiness, truncates large frames, or retains dropped-channel pointers.
        #[tokio::test]
        async fn pointer_input_survives_helper_replacement_and_resets_on_channel_drop() -> anyhow::Result<()> {
            use ironrdp_server::{CsReadyFlags, TouchContact, TouchContactFlags, TouchFrame};

            let (writer, mut reader) = input_socket_pair().await?;
            reader.write_all(&[1]).await?;
            let slot: InputStreamSlot = Arc::default();
            slot.attach(writer).await;
            let (tx, rx) = mpsc::channel(8);
            let mut handler = TermSrvRdpeiHandler {
                input: TermSrvInputHandler::new(7, tx),
            };
            let spooler = tokio::spawn(run_input_spooler(7, Arc::clone(&slot), rx));
            let ready = CsReadyPdu::new(
                CsReadyFlags::ENABLE_MULTIPEN_INJECTION,
                RdpInputProtocolVersion::V300,
                10,
            );
            handler.cs_ready(ready.clone());
            let first = timeout(Duration::from_secs(2), read_helper_input_event(&mut reader)).await??;
            assert!(matches!(first, HelperInputEvent::RdpeiReady(pdu) if pdu == ready));

            let (writer, mut replacement) = input_socket_pair().await?;
            replacement.write_all(&[1]).await?;
            slot.attach(writer).await;
            let event = TouchEventPdu::new(
                40,
                (0..10)
                    .map(|frame| {
                        TouchFrame::new(
                            frame * 100,
                            (0..10)
                                .map(|id| {
                                    TouchContact::new(
                                        id,
                                        300,
                                        400,
                                        TouchContactFlags::UPDATE | TouchContactFlags::INRANGE,
                                    )
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            );
            assert!(255 < ironrdp_core::Encode::size(&event));
            handler.touch(event.clone());
            let ready_again = timeout(Duration::from_secs(2), read_helper_input_event(&mut replacement)).await??;
            assert!(matches!(ready_again, HelperInputEvent::RdpeiReady(pdu) if pdu == ready));
            let touch = timeout(Duration::from_secs(2), read_helper_input_event(&mut replacement)).await??;
            assert!(matches!(touch, HelperInputEvent::Touch(pdu) if pdu == event));
            drop(handler);
            let reset = timeout(Duration::from_secs(2), read_helper_input_event(&mut replacement)).await??;
            assert!(matches!(reset, HelperInputEvent::PointerReset));
            timeout(Duration::from_secs(2), spooler).await??;
            Ok(())
        }

        async fn input_socket_pair() -> io::Result<(TcpStream, TcpStream)> {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
            let writer = TcpStream::connect(listener.local_addr()?).await?;
            let (reader, _) = listener.accept().await?;
            Ok((writer, reader))
        }

        #[test]
        fn helper_environment_excludes_service_credentials() {
            let entries = child_environment_entries(
                [
                    ("SystemRoot".to_owned(), r"C:\Windows".to_owned()),
                    (RDP_USERNAME_ENV.to_owned(), "alice".to_owned()),
                    (RDP_PASSWORD_ENV.to_owned(), "secret".to_owned()),
                    (RDP_DOMAIN_ENV.to_owned(), "CORP".to_owned()),
                    ("UNRELATED_SECRET".to_owned(), "hidden".to_owned()),
                ],
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            );

            assert!(entries.iter().any(|entry| entry == r"SystemRoot=C:\Windows"));
            assert!(entries.iter().any(|entry| entry.starts_with(CAPTURE_HELPER_TOKEN_ENV)));
            assert!(!entries.iter().any(|entry| entry.starts_with(RDP_USERNAME_ENV)));
            assert!(!entries.iter().any(|entry| entry.starts_with(RDP_PASSWORD_ENV)));
            assert!(!entries.iter().any(|entry| entry.starts_with(RDP_DOMAIN_ENV)));
            assert!(!entries.iter().any(|entry| entry.starts_with("UNRELATED_SECRET=")));
        }

        #[test]
        fn connection_ids_wrap_without_using_zero() {
            let bind_addr = "127.0.0.1:4489".parse().expect("valid bind address");
            let (mut state, _accepted_rx, _broken_rx) = ServiceState::new(bind_addr);
            state.next_connection_id = u32::MAX;

            assert_eq!(state.allocate_connection_id(), Some(u32::MAX));
            assert_eq!(state.allocate_connection_id(), Some(1));
        }

        #[test]
        fn closing_connection_removes_pending_incoming_notification() {
            let bind_addr = "127.0.0.1:4489".parse().expect("valid bind address");
            let (mut state, _accepted_rx, _broken_rx) = ServiceState::new(bind_addr);
            state.pending_incoming.push_back(PendingIncoming {
                listener_name: "IRDP-Tcp".to_owned(),
                connection_id: 7,
                peer_addr: None,
            });

            assert_eq!(state.close_connection(7), ServiceEvent::Ack);
            assert!(state.pending_incoming.is_empty());
        }

        // oracle: mutant prevent clipboard helpers from surviving a reassigned or closed session
        #[test]
        fn session_binding_changes_notify_clipboard_immediately() {
            let (mut state, _accepted_rx, _broken_rx) = ServiceState::new("127.0.0.1:4489".parse().unwrap());
            state.connections.insert(
                7,
                ConnectionEntry {
                    native_channels: Arc::new(crate::native::NativeChannels::default()),
                    listener_name: "IRDP-Tcp".to_owned(),
                    peer_addr: None,
                    session_task: None,
                    credentials: Arc::default(),
                },
            );
            let mut changed = state.session_binding_changed.subscribe();
            assert_eq!(state.set_capture_session_id(7, 2), ServiceEvent::Ack);
            assert!(changed.has_changed().unwrap());
            changed.borrow_and_update();
            state.set_capture_session_id(7, 2);
            assert!(!changed.has_changed().unwrap());
            state.set_capture_session_id(7, 3);
            assert!(changed.has_changed().unwrap());
            changed.borrow_and_update();
            state.close_connection(7);
            assert!(changed.has_changed().unwrap());
        }

        #[tokio::test(flavor = "current_thread")]
        async fn capture_reader_survives_receiver_timeout_mid_frame() {
            let local = tokio::task::LocalSet::new();
            local
                .run_until(async {
                    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                        .await
                        .expect("bind loopback listener");
                    let address = listener.local_addr().expect("listener address");
                    let connect = TcpStream::connect(address);
                    let accept = listener.accept();
                    let (connect, accept) = tokio::join!(connect, accept);
                    let mut writer = connect.expect("connect loopback stream");
                    let (reader, _) = accept.expect("accept loopback stream");
                    let (mut frame_rx, frame_task) = spawn_capture_frame_reader(reader);

                    let mut header = [0u8; CAPTURE_FRAME_HEADER_LEN];
                    header[0..4].copy_from_slice(&CAPTURE_FRAME_MAGIC);
                    header[4..6].copy_from_slice(&1u16.to_le_bytes());
                    header[6..8].copy_from_slice(&1u16.to_le_bytes());
                    header[8..10].copy_from_slice(&1u16.to_le_bytes());
                    header[10..14].copy_from_slice(&4u32.to_le_bytes());
                    header[14] = 0;
                    header[20..24].copy_from_slice(&4u32.to_le_bytes());

                    writer.write_all(&header).await.expect("write capture header");
                    writer.write_all(&[1, 2]).await.expect("write partial payload");

                    assert!(
                        timeout(Duration::from_millis(10), frame_rx.recv()).await.is_err(),
                        "partial frame should not be delivered"
                    );

                    writer.write_all(&[3, 4]).await.expect("finish payload");
                    let frame = timeout(Duration::from_secs(1), frame_rx.recv())
                        .await
                        .expect("completed frame timeout")
                        .expect("frame reader still open")
                        .expect("frame should decode");
                    match frame {
                        CapturedFrame::Raw(bitmap) => assert_eq!(bitmap.data.as_ref(), &[1, 2, 3, 4]),
                        CapturedFrame::PreEncoded(_) | CapturedFrame::Size(_) => panic!("expected raw frame"),
                    }

                    frame_task.abort();
                })
                .await;
        }
    }
}

#[cfg(windows)]
fn main() {
    windows_main::main();
}
