#[cfg(not(windows))]
fn main() {
    eprintln!("ironrdp-termsrv is only supported on windows");
}

#[cfg(windows)]
mod windows_main {
    use core::net::{Ipv4Addr, SocketAddr};
    use core::num::{NonZeroI32, NonZeroU16, NonZeroUsize};
    use core::ptr::null_mut;
    use core::sync::atomic::{fence, Ordering};
    use std::collections::{HashMap, VecDeque};
    use std::io;
    use std::sync::Arc;
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
    use tokio::sync::{mpsc, Mutex};
    use tokio::task::JoinHandle;
    use tokio::time::{sleep, timeout, Duration};
    use tracing::{error, info, warn};
    use tracing_subscriber::EnvFilter;
    use windows::core::{w, PCWSTR, PWSTR};
    use windows::Win32::Foundation::{
        GetLastError, SetLastError, ERROR_NOT_ALL_ASSIGNED, LUID, WAIT_OBJECT_0, WAIT_TIMEOUT, WIN32_ERROR,
    };
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
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
    use windows::Win32::System::RemoteDesktop::{WTSGetActiveConsoleSessionId, WTSQueryUserToken};
    use windows::Win32::System::Threading::{
        CreateEventW, CreateProcessAsUserW, GetCurrentProcess, OpenEventW, OpenProcess, OpenProcessToken, SetEvent,
        TerminateProcess, WaitForSingleObject, CREATE_NO_WINDOW, EVENT_MODIFY_STATE, PROCESS_INFORMATION,
        PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
        KEYEVENTF_SCANCODE, KEYEVENTF_UNICODE, MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
        MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, VIRTUAL_KEY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetDesktopWindow, GetSystemMetrics, SetCursorPos, SM_CXSCREEN, SM_CYSCREEN,
    };

    use windows::Win32::Security::{
        AdjustTokenPrivileges, DuplicateTokenEx, LookupPrivilegeValueW, SecurityImpersonation, SetTokenInformation,
        TokenPrimary, TokenSessionId, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES,
        TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_PRIVILEGES, TOKEN_QUERY,
    };

    use windows::Win32::System::RemoteDesktop::{WTSEnumerateProcessesW, WTSFreeMemory, WTS_PROCESS_INFOW};

    const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
    const LISTEN_ADDR_ENV: &str = "IRONRDP_WTS_LISTEN_ADDR";
    const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:4489";
    const CAPTURE_INTERVAL: Duration = Duration::from_millis(100);
    const CAPTURE_HELPER_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
    const CAPTURE_HELPER_RETRY_DELAY: Duration = Duration::from_secs(5);
    const CAPTURE_IPC_ENV: &str = "IRONRDP_WTS_CAPTURE_IPC";
    const AUTO_LISTEN_ENV: &str = "IRONRDP_WTS_AUTO_LISTEN";
    const AUTO_LISTEN_NAME_ENV: &str = "IRONRDP_WTS_AUTO_LISTENER_NAME";
    const TLS_CERT_SUBJECT_FIND: &str = "IronRDP TermSrv";
    const TLS_KEY_NAME: &str = "IronRdpTermSrvTlsKey";
    const RDP_USERNAME_ENV: &str = "IRONRDP_RDP_USERNAME";
    const RDP_PASSWORD_ENV: &str = "IRONRDP_RDP_PASSWORD";
    const RDP_DOMAIN_ENV: &str = "IRONRDP_RDP_DOMAIN";

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
    }

    impl GdiDisplay {
        fn new(connection_id: u32, input_stream_slot: Arc<Mutex<Option<TcpStream>>>) -> anyhow::Result<Self> {
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
                )
                .context("failed to initialize GDI display updates")?,
            ))
        }
    }

    struct GdiDisplayUpdates {
        connection_id: u32,
        desktop_size: DesktopSize,
        input_stream_slot: Arc<Mutex<Option<TcpStream>>>,
        capture: Option<CaptureClient>,
        next_helper_attempt_at: Instant,
        sent_first_frame: bool,
        last_bitmap: Option<BitmapUpdate>,
        helper_frames_received: u64,
        helper_timeouts: u64,
    }

    impl GdiDisplayUpdates {
        fn new(
            connection_id: u32,
            size: DesktopSize,
            input_stream_slot: Arc<Mutex<Option<TcpStream>>>,
        ) -> anyhow::Result<Self> {
            let _ = desktop_size_nonzero(size)?;

            Ok(Self {
                connection_id,
                desktop_size: size,
                input_stream_slot,
                capture: None,
                next_helper_attempt_at: Instant::now(),
                sent_first_frame: false,
                last_bitmap: None,
                helper_frames_received: 0,
                helper_timeouts: 0,
            })
        }
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
            if self.sent_first_frame && self.capture.is_none() {
                sleep(CAPTURE_INTERVAL).await;
            }

            if self.capture.is_none() && Instant::now() >= self.next_helper_attempt_at {
                match CaptureClient::start(
                    self.connection_id,
                    self.desktop_size,
                    Arc::clone(&self.input_stream_slot),
                )
                .await
                {
                    Ok(capture) => {
                        info!(
                            connection_id = self.connection_id,
                            helper_pid = capture.pid(),
                            "Started interactive capture helper"
                        );
                        self.capture = Some(capture);
                    }
                    Err(error) => {
                        warn!(
                            connection_id = self.connection_id,
                            error = %format!("{error:#}"),
                            "Failed to start interactive capture helper; falling back to in-process GDI"
                        );
                        self.next_helper_attempt_at = Instant::now() + CAPTURE_HELPER_RETRY_DELAY;
                    }
                }
            }

            let bitmap = if let Some(capture) = &mut self.capture {
                let read_timeout = if self.helper_frames_received == 0 {
                    Duration::from_secs(1)
                } else {
                    CAPTURE_INTERVAL
                };

                match timeout(read_timeout, capture.read_frame()).await {
                    Ok(Ok(bitmap)) => {
                        self.helper_frames_received = self.helper_frames_received.saturating_add(1);
                        self.last_bitmap = Some(bitmap.clone());
                        bitmap
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
                            bitmap
                        } else {
                            let bitmap = fallback_bitmap_update(self.desktop_size)
                                .context("failed to generate fallback bitmap update")?;
                            self.last_bitmap = Some(bitmap.clone());
                            bitmap
                        }
                    }
                    Err(_) => {
                        self.helper_timeouts = self.helper_timeouts.saturating_add(1);
                        sleep(CAPTURE_INTERVAL).await;

                        if let Some(bitmap) = self.last_bitmap.clone() {
                            bitmap
                        } else {
                            let bitmap = fallback_bitmap_update(self.desktop_size)
                                .context("failed to generate fallback bitmap update")?;
                            self.last_bitmap = Some(bitmap.clone());
                            bitmap
                        }
                    }
                }
            } else {
                match capture_bitmap_update(self.desktop_size) {
                    Ok(bitmap) => {
                        self.last_bitmap = Some(bitmap.clone());
                        bitmap
                    }
                    Err(error) => {
                        if let Some(bitmap) = self.last_bitmap.clone() {
                            warn!(
                                error = %format!("{error:#}"),
                                "GDI capture failed; re-sending last bitmap"
                            );
                            bitmap
                        } else {
                            warn!(
                                error = %format!("{error:#}"),
                                "GDI capture failed; sending synthetic test pattern"
                            );
                            let bitmap = fallback_bitmap_update(self.desktop_size)
                                .context("failed to generate fallback bitmap update")?;
                            self.last_bitmap = Some(bitmap.clone());
                            bitmap
                        }
                    }
                }
            };
            self.sent_first_frame = true;

            Ok(Some(DisplayUpdate::Bitmap(bitmap)))
        }
    }

    #[derive(Clone, Copy)]
    struct SendHandle(windows::Win32::Foundation::HANDLE);

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

    enum CaptureClient {
        Tcp(HelperCaptureClient),
        SharedMem(SharedMemCaptureClient),
    }

    impl CaptureClient {
        async fn start(
            connection_id: u32,
            desktop_size: DesktopSize,
            input_stream_slot: Arc<Mutex<Option<TcpStream>>>,
        ) -> anyhow::Result<Self> {
            match capture_ipc_from_env() {
                CaptureIpc::Tcp => Ok(Self::Tcp(
                    HelperCaptureClient::start(connection_id, input_stream_slot).await?,
                )),
                CaptureIpc::SharedMem => {
                    match SharedMemCaptureClient::start(connection_id, desktop_size, Arc::clone(&input_stream_slot))
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
                                HelperCaptureClient::start(connection_id, input_stream_slot).await?,
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

        async fn read_frame(&mut self) -> anyhow::Result<BitmapUpdate> {
            match self {
                Self::Tcp(client) => client.read_frame().await,
                Self::SharedMem(client) => client.read_frame().await,
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
        async fn start(connection_id: u32, input_stream_slot: Arc<Mutex<Option<TcpStream>>>) -> anyhow::Result<Self> {
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

            let helper = spawn_capture_helper_process_tcp(local_addr, input_addr)
                .with_context(|| format!("failed to spawn capture helper for connection {connection_id}"))?;

            let (stream, _peer) = timeout(CAPTURE_HELPER_CONNECT_TIMEOUT, listener.accept())
                .await
                .map_err(|_| anyhow!("capture helper did not connect within timeout"))?
                .context("failed to accept capture helper connection")?;

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

        async fn read_frame(&mut self) -> anyhow::Result<BitmapUpdate> {
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

            let helper =
                spawn_capture_helper_process_shared_mem(&map_name, &event_name, input_addr).with_context(|| {
                    format!("failed to spawn shared-memory capture helper for connection {connection_id}")
                })?;

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

                let mut data = vec![0u8; payload_len];

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

    struct AcquiredSessionToken {
        token: windows::Win32::Foundation::HANDLE,
        desktop: &'static str,
    }

    fn spawn_capture_helper_process_tcp(
        connect_addr: SocketAddr,
        input_connect_addr: SocketAddr,
    ) -> anyhow::Result<SpawnedProcess> {
        spawn_capture_helper_process_with_args(&format!(
            "--connect {connect_addr} --input-connect {input_connect_addr}"
        ))
    }

    fn spawn_capture_helper_process_shared_mem(
        map_name: &str,
        event_name: &str,
        input_connect_addr: SocketAddr,
    ) -> anyhow::Result<SpawnedProcess> {
        spawn_capture_helper_process_with_args(&format!(
            "--shm-map \"{map_name}\" --shm-event \"{event_name}\" --input-connect {input_connect_addr}"
        ))
    }

    fn spawn_capture_helper_process_with_args(extra_args: &str) -> anyhow::Result<SpawnedProcess> {
        // SAFETY: safe to call and returns a process-global session id value.
        let session_id = unsafe { WTSGetActiveConsoleSessionId() };
        if session_id == u32::MAX {
            return Err(anyhow!("no active console session"));
        }

        let acquired =
            acquire_session_token(session_id).context("failed to acquire a token for the capture session")?;
        let user_token = acquired.token;

        let exe_path = std::env::current_exe().context("failed to resolve current executable path")?;
        let exe_path_str = exe_path
            .to_str()
            .ok_or_else(|| anyhow!("current executable path is not valid unicode"))?;

        let desktop = acquired.desktop;
        let args = format!("\"{exe_path_str}\" --capture-helper {extra_args}");

        let app_name: Vec<u16> = exe_path_str.encode_utf16().chain(Some(0)).collect();
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

        // SAFETY: close thread handle we don't need.
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(process_info.hThread);
        }

        Ok(SpawnedProcess {
            pid: process_info.dwProcessId,
            process: SendHandle(process_info.hProcess),
        })
    }

    fn acquire_session_token(session_id: u32) -> anyhow::Result<AcquiredSessionToken> {
        let mut token = windows::Win32::Foundation::HANDLE::default();

        // SAFETY: `WTSQueryUserToken` writes a token handle into `token` on success.
        let wts_result = unsafe { WTSQueryUserToken(session_id, &mut token) };
        if wts_result.is_ok() {
            return Ok(AcquiredSessionToken {
                token,
                desktop: "winsta0\\default",
            });
        }

        let wts_error = wts_result.err().unwrap_or_else(windows::core::Error::empty);
        warn!(
            session_id,
            error = %wts_error,
            error_code = %wts_error.code(),
            "WTSQueryUserToken failed; attempting to spawn helper with a session-adjusted service token"
        );

        if let Ok(token) = token_from_winlogon(session_id) {
            return Ok(AcquiredSessionToken {
                token,
                desktop: "winsta0\\winlogon",
            });
        }

        let token = duplicate_self_token_for_session(session_id)?;

        Ok(AcquiredSessionToken {
            token,
            desktop: "winsta0\\winlogon",
        })
    }

    fn token_from_winlogon(session_id: u32) -> anyhow::Result<windows::Win32::Foundation::HANDLE> {
        let mut process_info_ptr: *mut WTS_PROCESS_INFOW = null_mut();
        let mut process_count = 0u32;

        // SAFETY: WTSEnumerateProcessesW writes a buffer pointer into `process_info_ptr` on success.
        unsafe { WTSEnumerateProcessesW(None, 0, 1, &mut process_info_ptr, &mut process_count) }
            .map_err(|error| anyhow!("WTSEnumerateProcessesW failed: {error}"))
            .context("WTSEnumerateProcessesW failed")?;

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

        let mut winlogon_pid: Option<u32> = None;

        for entry in processes {
            if entry.SessionId != session_id {
                continue;
            }

            // SAFETY: pProcessName is a nul-terminated wide string pointer returned by WTSEnumerateProcessesW.
            let name = unsafe { PCWSTR(entry.pProcessName.0).to_string() }.unwrap_or_default();
            if name.eq_ignore_ascii_case("winlogon.exe") {
                winlogon_pid = Some(entry.ProcessId);
                break;
            }
        }

        let pid = winlogon_pid.ok_or_else(|| anyhow!("winlogon.exe not found in session {session_id}"))?;

        // SAFETY: OpenProcess returns a handle for the specified PID when permitted.
        let process_handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            .map_err(|error| anyhow!("OpenProcess failed: {error}"))
            .context("OpenProcess failed")?;

        struct HandleGuard(windows::Win32::Foundation::HANDLE);
        impl Drop for HandleGuard {
            fn drop(&mut self) {
                // SAFETY: handle is either valid or null; CloseHandle is safe to call.
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(self.0);
                }
            }
        }

        let _process_guard = HandleGuard(process_handle);

        let mut winlogon_token = windows::Win32::Foundation::HANDLE::default();

        // SAFETY: OpenProcessToken writes a token handle into `winlogon_token` on success.
        unsafe {
            OpenProcessToken(
                process_handle,
                TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_QUERY,
                &mut winlogon_token,
            )
        }
        .map_err(|error| anyhow!("OpenProcessToken(winlogon) failed: {error}"))
        .context("OpenProcessToken(winlogon) failed")?;

        let _token_guard = HandleGuard(winlogon_token);

        let mut primary_token = windows::Win32::Foundation::HANDLE::default();

        // SAFETY: DuplicateTokenEx writes a new token handle into `primary_token` on success.
        unsafe {
            DuplicateTokenEx(
                winlogon_token,
                TOKEN_DUPLICATE | TOKEN_ASSIGN_PRIMARY | TOKEN_QUERY | TOKEN_ADJUST_SESSIONID,
                None,
                SecurityImpersonation,
                TokenPrimary,
                &mut primary_token,
            )
        }
        .map_err(|error| anyhow!("DuplicateTokenEx(winlogon) failed: {error}"))
        .context("DuplicateTokenEx(winlogon) failed")?;

        Ok(primary_token)
    }

    fn duplicate_self_token_for_session(session_id: u32) -> anyhow::Result<windows::Win32::Foundation::HANDLE> {
        let mut process_token = windows::Win32::Foundation::HANDLE::default();

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

        let mut primary_token = windows::Win32::Foundation::HANDLE::default();

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

        let session_id_ptr = core::ptr::addr_of!(session_id).cast::<core::ffi::c_void>();

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
        let mut process_token = windows::Win32::Foundation::HANDLE::default();

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

    async fn read_capture_frame(stream: &mut TcpStream) -> anyhow::Result<BitmapUpdate> {
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
        if version != 1 {
            return Err(anyhow!("unsupported capture frame version: {version}"));
        }

        let width_u16 = u16::from_le_bytes([header[6], header[7]]);
        let height_u16 = u16::from_le_bytes([header[8], header[9]]);
        let stride_u32 = u32::from_le_bytes([header[10], header[11], header[12], header[13]]);
        let format = header[14];
        let payload_len = u32::from_le_bytes([header[20], header[21], header[22], header[23]]);

        if format != 0 {
            return Err(anyhow!("unsupported capture pixel format: {format}"));
        }

        let width = NonZeroU16::new(width_u16).ok_or_else(|| anyhow!("capture frame width is zero"))?;
        let height = NonZeroU16::new(height_u16).ok_or_else(|| anyhow!("capture frame height is zero"))?;
        let stride_usize = usize::try_from(stride_u32).map_err(|_| anyhow!("capture frame stride out of range"))?;
        let stride = NonZeroUsize::new(stride_usize).ok_or_else(|| anyhow!("capture frame stride is zero"))?;
        let payload_len_usize =
            usize::try_from(payload_len).map_err(|_| anyhow!("capture payload length out of range"))?;

        let expected = stride
            .get()
            .checked_mul(NonZeroUsize::from(height).get())
            .ok_or_else(|| anyhow!("capture payload length overflow"))?;

        if payload_len_usize != expected {
            return Err(anyhow!(
                "capture payload length mismatch (got {payload_len_usize}, expected {expected})"
            ));
        }

        let mut payload = vec![0u8; payload_len_usize];
        stream
            .read_exact(&mut payload)
            .await
            .context("failed to read capture frame payload")?;

        Ok(BitmapUpdate {
            x: 0,
            y: 0,
            width,
            height,
            format: PixelFormat::BgrA32,
            data: payload.into(),
            stride,
        })
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

        // SAFETY: `GetDesktopWindow` is safe to call and returns a process-global desktop window handle.
        let desktop_hwnd = unsafe { GetDesktopWindow() };

        // SAFETY: `desktop_hwnd` is a valid HWND for the current session desktop.
        let screen_dc = unsafe { GetDC(Some(desktop_hwnd)) };
        if screen_dc.0.is_null() {
            return Err(anyhow!("GetDC returned a null screen device context"));
        }

        // SAFETY: `screen_dc` is a valid display DC obtained above.
        let memory_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
        if memory_dc.0.is_null() {
            // SAFETY: `screen_dc` was acquired with `GetDC` and must be released.
            let _ = unsafe { ReleaseDC(Some(desktop_hwnd), screen_dc) };
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

        let mut bits_ptr: *mut core::ffi::c_void = null_mut();

        // SAFETY: `screen_dc` and `bitmap_info` are valid, and we pass a valid out-pointer for bits.
        let bitmap = unsafe { CreateDIBSection(Some(screen_dc), &bitmap_info, DIB_RGB_COLORS, &mut bits_ptr, None, 0) }
            .map_err(|error| anyhow!("CreateDIBSection failed: {error}"))?;

        if bitmap.0.is_null() {
            // SAFETY: `memory_dc` and `screen_dc` are valid handles created above.
            let _ = unsafe { DeleteDC(memory_dc) };
            // SAFETY: `screen_dc` was acquired with `GetDC` and must be released.
            let _ = unsafe { ReleaseDC(Some(desktop_hwnd), screen_dc) };
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
            let _ = unsafe { ReleaseDC(Some(desktop_hwnd), screen_dc) };
            return Err(anyhow!("SelectObject failed for capture bitmap"));
        }

        // SAFETY: all DC handles are valid and dimensions are taken from initialized state.
        let bitblt_result = unsafe { BitBlt(memory_dc, 0, 0, width_i32, height_i32, Some(screen_dc), 0, 0, SRCCOPY) };

        let mut data = vec![0u8; frame_len];
        if bitblt_result.is_ok() {
            if bits_ptr.is_null() {
                // SAFETY: clean up GDI objects created above.
                let _ = unsafe { SelectObject(memory_dc, previous_bitmap) };
                // SAFETY: clean up GDI objects created above.
                let _ = unsafe { DeleteObject(HGDIOBJ(bitmap.0)) };
                // SAFETY: clean up GDI objects created above.
                let _ = unsafe { DeleteDC(memory_dc) };
                // SAFETY: clean up GDI objects created above.
                let _ = unsafe { ReleaseDC(Some(desktop_hwnd), screen_dc) };
                return Err(anyhow!("CreateDIBSection returned a null bitmap data pointer"));
            }

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
        let _ = unsafe { ReleaseDC(Some(desktop_hwnd), screen_dc) };

        bitblt_result.map_err(|error| anyhow!("BitBlt failed while capturing desktop frame: {error}"))?;

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

        let mut data = vec![0u8; frame_len];
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

    struct ConnectionEntry {
        listener_name: String,
        peer_addr: Option<String>,
        stream: Option<TcpStream>,
        session_task: Option<JoinHandle<()>>,
    }

    struct ManagedListener {
        join_handle: JoinHandle<()>,
    }

    struct ServiceState {
        bind_addr: SocketAddr,
        listeners: HashMap<String, ManagedListener>,
        pending_incoming: VecDeque<PendingIncoming>,
        connections: HashMap<u32, ConnectionEntry>,
        next_connection_id: u32,
        accepted_tx: mpsc::UnboundedSender<AcceptedSocket>,
        accepted_rx: mpsc::UnboundedReceiver<AcceptedSocket>,
    }

    impl ServiceState {
        fn new(bind_addr: SocketAddr) -> Self {
            let (accepted_tx, accepted_rx) = mpsc::unbounded_channel();

            Self {
                bind_addr,
                listeners: HashMap::new(),
                pending_incoming: VecDeque::new(),
                connections: HashMap::new(),
                next_connection_id: 1,
                accepted_tx,
                accepted_rx,
            }
        }

        async fn handle_command(&mut self, command: ProviderCommand) -> ServiceEvent {
            self.drain_accepted();

            match command {
                ProviderCommand::StartListen { listener_name } => self.start_listen(listener_name).await,
                ProviderCommand::StopListen { listener_name } => self.stop_listen(listener_name),
                ProviderCommand::WaitForIncoming {
                    listener_name,
                    timeout_ms,
                } => self.wait_for_incoming(listener_name, timeout_ms).await,
                ProviderCommand::AcceptConnection { connection_id } => self.accept_connection(connection_id),
                ProviderCommand::CloseConnection { connection_id } => self.close_connection(connection_id),
            }
        }

        async fn start_listen(&mut self, listener_name: String) -> ServiceEvent {
            if self.listeners.contains_key(&listener_name) {
                return ServiceEvent::ListenerStarted { listener_name };
            }

            let listener = match TcpListener::bind(self.bind_addr).await {
                Ok(listener) => listener,
                Err(error) => {
                    return ServiceEvent::Error {
                        message: format!("failed to bind listener socket: {error}"),
                    };
                }
            };

            let accept_tx = self.accepted_tx.clone();
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

            self.listeners
                .insert(listener_name.clone(), ManagedListener { join_handle });

            info!(%listener_name, bind_addr = %self.bind_addr, "Started control-plane listener task");

            ServiceEvent::ListenerStarted { listener_name }
        }

        fn stop_listen(&mut self, listener_name: String) -> ServiceEvent {
            if let Some(listener) = self.listeners.remove(&listener_name) {
                listener.join_handle.abort();
            }

            self.pending_incoming
                .retain(|pending| pending.listener_name != listener_name);

            let connection_ids_to_close: Vec<u32> = self
                .connections
                .iter()
                .filter_map(|(connection_id, connection)| {
                    if connection.listener_name == listener_name {
                        Some(*connection_id)
                    } else {
                        None
                    }
                })
                .collect();

            for connection_id in connection_ids_to_close {
                let _ = self.close_connection(connection_id);
            }

            info!(%listener_name, "Stopped control-plane listener task");

            ServiceEvent::ListenerStopped { listener_name }
        }

        async fn wait_for_incoming(&mut self, listener_name: String, timeout_ms: u32) -> ServiceEvent {
            if let Some(event) = self.pop_pending_for_listener(&listener_name) {
                return event;
            }

            if timeout_ms == 0 {
                return ServiceEvent::NoIncoming;
            }

            let wait_duration = Duration::from_millis(u64::from(timeout_ms));

            match timeout(wait_duration, self.accepted_rx.recv()).await {
                Ok(Some(accepted)) => {
                    self.register_accepted(accepted);
                    self.pop_pending_for_listener(&listener_name)
                        .unwrap_or(ServiceEvent::NoIncoming)
                }
                Ok(None) => ServiceEvent::Error {
                    message: "accept channel closed".to_owned(),
                },
                Err(_) => ServiceEvent::NoIncoming,
            }
        }

        fn accept_connection(&mut self, connection_id: u32) -> ServiceEvent {
            let connection = match self.connections.get_mut(&connection_id) {
                Some(connection) => connection,
                None => {
                    return ServiceEvent::Error {
                        message: format!("unknown connection id: {connection_id}"),
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

            let session_task = tokio::task::spawn_local(async move {
                if let Err(error) = run_ironrdp_connection(connection_id, peer_addr.as_deref(), stream).await {
                    warn!(error = %format!("{error:#}"), connection_id, "IronRDP connection task failed");
                }
            });

            connection.session_task = Some(session_task);

            ServiceEvent::ConnectionReady { connection_id }
        }

        fn close_connection(&mut self, connection_id: u32) -> ServiceEvent {
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

            self.connections.insert(
                connection_id,
                ConnectionEntry {
                    listener_name: listener_name.clone(),
                    peer_addr: peer_addr.clone(),
                    stream: Some(accepted.stream),
                    session_task: None,
                },
            );

            self.pending_incoming.push_back(PendingIncoming {
                listener_name,
                connection_id,
                peer_addr,
            });
        }

        fn drain_accepted(&mut self) {
            while let Ok(accepted) = self.accepted_rx.try_recv() {
                self.register_accepted(accepted);
            }
        }
    }

    async fn run_ironrdp_connection(
        connection_id: u32,
        peer_addr: Option<&str>,
        stream: TcpStream,
    ) -> anyhow::Result<()> {
        info!(connection_id, peer_addr = ?peer_addr, "Starting IronRDP session task");

        let input_stream_slot: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
        let (input_tx, input_rx) = mpsc::unbounded_channel::<InputPacket>();
        let input_task = tokio::task::spawn_local(run_input_spooler(
            connection_id,
            Arc::clone(&input_stream_slot),
            input_rx,
        ));

        let display = GdiDisplay::new(connection_id, Arc::clone(&input_stream_slot))
            .context("failed to initialize GDI display handler")?;
        let (tls_acceptor, tls_pub_key) = make_tls_acceptor().context("failed to initialize TLS acceptor")?;

        let expected_credentials = resolve_rdp_credentials_from_env()?;

        let input_handler = TermSrvInputHandler::new(connection_id, input_tx);

        let mut server = {
            let builder = RdpServer::builder().with_addr(([127, 0, 0, 1], 0));

            let builder = if expected_credentials.is_some() {
                builder.with_hybrid(tls_acceptor, tls_pub_key)
            } else {
                drop(tls_pub_key);
                builder.with_tls(tls_acceptor)
            };

            builder
                .with_input_handler(input_handler)
                .with_display_handler(display)
                .build()
        };

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

        server
            .run_connection(stream)
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
        init_tracing()?;

        if let Some(mode) = parse_capture_helper_mode()? {
            match mode {
                CaptureHelperMode::Tcp {
                    connect_addr,
                    input_connect_addr,
                } => {
                    info!(
                        connect_addr = %connect_addr,
                        input_connect_addr = %input_connect_addr,
                        "Starting capture helper mode (tcp)"
                    );
                    return run_capture_helper_tcp(connect_addr, input_connect_addr).await;
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

        info!(pipe = %pipe_name, "Starting termsrv control loop");
        info!(bind_addr = %bind_addr, "Configured service listener bind address");

        let mut state = ServiceState::new(bind_addr);

        #[expect(
            clippy::infinite_loop,
            reason = "service runs indefinitely; failures are handled inside the loop"
        )]
        loop {
            if let Err(error) = run_server_once(&pipe_name, &mut state).await {
                warn!(%error, pipe = %pipe_name, "Control pipe loop failed; retrying");
                sleep(Duration::from_millis(200)).await;
            }
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
                if let Err(error) = run_ironrdp_connection(connection_id, Some(&peer_addr), stream).await {
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

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--capture-helper" => {
                    capture_helper = true;
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
        loop {
            let event = read_helper_input_event(&mut stream).await?;
            if let Err(error) = inject_helper_input_event(event) {
                warn!(error = %format!("{error:#}"), "Failed to inject input event");
            }
        }
    }

    async fn run_capture_helper_tcp(connect_addr: SocketAddr, input_connect_addr: SocketAddr) -> anyhow::Result<()> {
        let mut stream = TcpStream::connect(connect_addr)
            .await
            .with_context(|| format!("failed to connect to capture consumer at {connect_addr}"))?;

        let input_stream = TcpStream::connect(input_connect_addr)
            .await
            .with_context(|| format!("failed to connect to input consumer at {input_connect_addr}"))?;

        tokio::spawn(async move {
            if let Err(error) = run_input_injector(input_stream).await {
                warn!(error = %format!("{error:#}"), "Input injector task stopped");
            }
        });

        let desktop_size = desktop_size_from_gdi().context("failed to query desktop size")?;
        info!(
            width = desktop_size.width,
            height = desktop_size.height,
            "Initialized capture helper desktop size"
        );

        loop {
            match capture_bitmap_update(desktop_size) {
                Ok(bitmap) => {
                    write_capture_frame(&mut stream, &bitmap).await?;
                }
                Err(error) => {
                    warn!(
                        error = %format!("{error:#}"),
                        "Capture helper failed to capture frame"
                    );
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

        struct HandleGuard(windows::Win32::Foundation::HANDLE);
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
                }
                Err(error) => {
                    warn!(
                        error = %format!("{error:#}"),
                        "Capture helper failed to capture frame"
                    );
                    sleep(CAPTURE_INTERVAL).await;
                }
            }
        }
    }

    async fn run_server_once(pipe_name: &str, state: &mut ServiceState) -> anyhow::Result<()> {
        let full_pipe_name = pipe_path(pipe_name);

        let mut server = named_pipe::ServerOptions::new()
            .access_inbound(true)
            .access_outbound(true)
            .in_buffer_size(PIPE_BUFFER_SIZE)
            .out_buffer_size(PIPE_BUFFER_SIZE)
            .pipe_mode(named_pipe::PipeMode::Byte)
            .create(&full_pipe_name)
            .with_context(|| format!("failed to create control pipe server: {full_pipe_name}"))?;

        info!(pipe = %full_pipe_name, "Waiting for provider control connection");
        server
            .connect()
            .await
            .with_context(|| format!("failed to accept control connection on pipe: {full_pipe_name}"))?;

        info!(pipe = %full_pipe_name, "Provider control connection established");

        if let Err(error) = handle_client(&mut server, state).await {
            warn!(%error, pipe = %full_pipe_name, "Control connection handler returned error");
        }

        Ok(())
    }

    async fn handle_client(pipe: &mut named_pipe::NamedPipeServer, state: &mut ServiceState) -> anyhow::Result<()> {
        loop {
            let command = match read_command(pipe).await {
                Ok(Some(command)) => command,
                Ok(None) => {
                    info!("Provider control client disconnected");
                    return Ok(());
                }
                Err(error) => {
                    return Err(error).context("failed to read provider command");
                }
            };

            let event = state.handle_command(command).await;

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
        main_impl();
    }
}

#[cfg(windows)]
fn main() {
    windows_main::main();
}
