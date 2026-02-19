#[cfg(not(windows))]
fn main() {
    eprintln!("ceviche-service is only supported on windows");
}

#[cfg(windows)]
mod windows_main {
    use core::net::{Ipv4Addr, SocketAddr};
    use core::num::{NonZeroI32, NonZeroU16, NonZeroUsize};
    use core::ptr::null_mut;
    use std::collections::{HashMap, VecDeque};
    use std::io;
    use std::sync::Arc;
    use std::time::Instant;

    use anyhow::{anyhow, Context as _};
    use ironrdp_server::tokio_rustls::{rustls, TlsAcceptor};
    use ironrdp_server::{
        BitmapUpdate, Credentials, DesktopSize, DisplayUpdate, PixelFormat, RdpServer, RdpServerDisplay,
        RdpServerDisplayUpdates,
    };
    use ironrdp_wtsprotocol_ipc::{
        default_pipe_name, pipe_path, resolve_pipe_name_from_env, ProviderCommand, ServiceEvent, DEFAULT_MAX_FRAME_SIZE,
    };
    use rustls_cng::signer::CngSigningKey;
    use rustls_cng::store::{CertStore, CertStoreType};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::windows::named_pipe;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;
    use tokio::time::{sleep, timeout, Duration};
    use tracing::{error, info, warn};
    use tracing_subscriber::EnvFilter;
    use windows::core::{w, PCWSTR, PWSTR};
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
    use windows::Win32::System::RemoteDesktop::{WTSGetActiveConsoleSessionId, WTSQueryUserToken};
    use windows::Win32::System::Threading::{
        CreateProcessAsUserW, TerminateProcess, CREATE_NO_WINDOW, PROCESS_INFORMATION, STARTUPINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetDesktopWindow, GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
    const LISTEN_ADDR_ENV: &str = "IRONRDP_WTS_LISTEN_ADDR";
    const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:4489";
    const CAPTURE_INTERVAL: Duration = Duration::from_millis(100);
    const CAPTURE_HELPER_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
    const CAPTURE_HELPER_RETRY_DELAY: Duration = Duration::from_secs(5);
    const TLS_CERT_SUBJECT_FIND: &str = "IronRDP Ceviche Service";
    const TLS_KEY_NAME: &str = "IronRdpCevicheTlsKey";
    const RDP_USERNAME_ENV: &str = "IRONRDP_RDP_USERNAME";
    const RDP_PASSWORD_ENV: &str = "IRONRDP_RDP_PASSWORD";
    const RDP_DOMAIN_ENV: &str = "IRONRDP_RDP_DOMAIN";

    struct GdiDisplay {
        connection_id: u32,
        desktop_size: DesktopSize,
    }

    impl GdiDisplay {
        fn new(connection_id: u32) -> anyhow::Result<Self> {
            let desktop_size = desktop_size_from_gdi().context("failed to query desktop size")?;

            info!(
                width = desktop_size.width,
                height = desktop_size.height,
                "Initialized GDI display source"
            );

            Ok(Self {
                connection_id,
                desktop_size,
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
                GdiDisplayUpdates::new(self.connection_id, self.desktop_size)
                    .context("failed to initialize GDI display updates")?,
            ))
        }
    }

    struct GdiDisplayUpdates {
        connection_id: u32,
        desktop_size: DesktopSize,
        capture: Option<HelperCaptureClient>,
        next_helper_attempt_at: Instant,
        sent_first_frame: bool,
    }

    impl GdiDisplayUpdates {
        fn new(connection_id: u32, size: DesktopSize) -> anyhow::Result<Self> {
            let _ = desktop_size_nonzero(size)?;

            Ok(Self {
                connection_id,
                desktop_size: size,
                capture: None,
                next_helper_attempt_at: Instant::now(),
                sent_first_frame: false,
            })
        }
    }

    impl Drop for GdiDisplayUpdates {
        fn drop(&mut self) {
            if let Some(capture) = self.capture.take() {
                capture.terminate();
            }
        }
    }

    #[async_trait::async_trait]
    impl RdpServerDisplayUpdates for GdiDisplayUpdates {
        async fn next_update(&mut self) -> anyhow::Result<Option<DisplayUpdate>> {
            if self.sent_first_frame {
                sleep(CAPTURE_INTERVAL).await;
            }

            if self.capture.is_none() && Instant::now() >= self.next_helper_attempt_at {
                match HelperCaptureClient::start(self.connection_id).await {
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
                match timeout(CAPTURE_INTERVAL, capture.read_frame()).await {
                    Ok(Ok(bitmap)) => bitmap,
                    Ok(Err(error)) => {
                        warn!(
                            connection_id = self.connection_id,
                            error = %format!("{error:#}"),
                            "Interactive capture helper failed; falling back to synthetic test pattern"
                        );
                        let capture = self.capture.take();
                        if let Some(capture) = capture {
                            capture.terminate();
                        }
                        fallback_bitmap_update(self.desktop_size)
                            .context("failed to generate fallback bitmap update")?
                    }
                    Err(_) => fallback_bitmap_update(self.desktop_size)
                        .context("failed to generate fallback bitmap update")?,
                }
            } else {
                match capture_bitmap_update(self.desktop_size) {
                    Ok(bitmap) => bitmap,
                    Err(error) => {
                        warn!(
                            error = %format!("{error:#}"),
                            "GDI capture failed; sending synthetic test pattern"
                        );
                        fallback_bitmap_update(self.desktop_size)
                            .context("failed to generate fallback bitmap update")?
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

    struct HelperCaptureClient {
        helper_pid: u32,
        helper_process: SendHandle,
        stream: TcpStream,
    }

    impl HelperCaptureClient {
        async fn start(connection_id: u32) -> anyhow::Result<Self> {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .context("failed to bind local capture helper listener")?;

            let local_addr = listener
                .local_addr()
                .context("failed to query local helper listener address")?;

            let helper = spawn_capture_helper_process(local_addr)
                .with_context(|| format!("failed to spawn capture helper for connection {connection_id}"))?;

            let (stream, _peer) = timeout(CAPTURE_HELPER_CONNECT_TIMEOUT, listener.accept())
                .await
                .map_err(|_| anyhow!("capture helper did not connect within timeout"))?
                .context("failed to accept capture helper connection")?;

            Ok(Self {
                helper_pid: helper.pid,
                helper_process: helper.process,
                stream,
            })
        }

        fn pid(&self) -> u32 {
            self.helper_pid
        }

        fn terminate(self) {
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

    struct SpawnedProcess {
        pid: u32,
        process: SendHandle,
    }

    fn spawn_capture_helper_process(connect_addr: SocketAddr) -> anyhow::Result<SpawnedProcess> {
        // SAFETY: safe to call and returns a process-global session id value.
        let session_id = unsafe { WTSGetActiveConsoleSessionId() };
        if session_id == u32::MAX {
            return Err(anyhow!("no active console session"));
        }

        let mut user_token = windows::Win32::Foundation::HANDLE::default();
        // SAFETY: `WTSQueryUserToken` writes a token handle into `user_token` on success.
        unsafe { WTSQueryUserToken(session_id, &mut user_token) }
            .ok()
            .context("WTSQueryUserToken failed")?;

        let exe_path = std::env::current_exe().context("failed to resolve current executable path")?;
        let exe_path_str = exe_path
            .to_str()
            .ok_or_else(|| anyhow!("current executable path is not valid unicode"))?;

        let desktop = "winsta0\\default";
        let args = format!("\"{exe_path_str}\" --capture-helper --connect {connect_addr}",);

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

    const CAPTURE_FRAME_MAGIC: [u8; 4] = *b"IRDP";
    const CAPTURE_FRAME_HEADER_LEN: usize = 24;

    async fn read_capture_frame(stream: &mut TcpStream) -> anyhow::Result<BitmapUpdate> {
        let mut header = [0u8; CAPTURE_FRAME_HEADER_LEN];
        stream
            .read_exact(&mut header)
            .await
            .context("failed to read capture frame header")?;

        if header[0..4] != CAPTURE_FRAME_MAGIC {
            return Err(anyhow!("invalid capture frame magic"));
        }

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

        let display = GdiDisplay::new(connection_id).context("failed to initialize GDI display handler")?;
        let tls_acceptor = make_tls_acceptor().context("failed to initialize TLS acceptor")?;

        let mut server = RdpServer::builder()
            .with_addr(([127, 0, 0, 1], 0))
            .with_tls(tls_acceptor)
            .with_no_input()
            .with_display_handler(display)
            .build();

        if let Some(credentials) = resolve_rdp_credentials_from_env()? {
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

    fn make_tls_acceptor() -> anyhow::Result<TlsAcceptor> {
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

        let mut server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(resolver));

        // This adds support for the SSLKEYLOGFILE env variable (https://wiki.wireshark.org/TLS#using-the-pre-master-secret)
        server_config.key_log = Arc::new(rustls::KeyLogFile::new());

        Ok(TlsAcceptor::from(Arc::new(server_config)))
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

        if let Some(connect_addr) = parse_capture_helper_connect_addr()? {
            info!(connect_addr = %connect_addr, "Starting capture helper mode");
            return run_capture_helper(connect_addr).await;
        }

        let pipe_name = resolve_pipe_name_from_env().unwrap_or_else(default_pipe_name);
        let bind_addr = resolve_bind_addr()?;

        info!(pipe = %pipe_name, "Starting ceviche service control loop");
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

    fn parse_capture_helper_connect_addr() -> anyhow::Result<Option<SocketAddr>> {
        let mut args = std::env::args().skip(1);

        let mut capture_helper = false;
        let mut connect: Option<SocketAddr> = None;

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
                _ => {}
            }
        }

        if !capture_helper {
            return Ok(None);
        }

        let connect = connect.ok_or_else(|| anyhow!("--capture-helper requires --connect"))?;
        Ok(Some(connect))
    }

    async fn run_capture_helper(connect_addr: SocketAddr) -> anyhow::Result<()> {
        let mut stream = TcpStream::connect(connect_addr)
            .await
            .with_context(|| format!("failed to connect to capture consumer at {connect_addr}"))?;

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
                    error!(%error, "Ceviche service failed");
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
