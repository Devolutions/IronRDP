#[cfg(not(windows))]
fn main() {
    eprintln!("ceviche-service is only supported on windows");
}

#[cfg(windows)]
mod windows_main {
    use core::num::{NonZeroI32, NonZeroU16, NonZeroUsize};
    use core::ptr::null_mut;
    use std::collections::{HashMap, VecDeque};
    use std::io;
    use core::net::SocketAddr;
    use std::sync::Arc;

    use anyhow::{anyhow, Context as _};
    use ironrdp_server::{
        BitmapUpdate, DesktopSize, DisplayUpdate, PixelFormat, RdpServer, RdpServerDisplay, RdpServerDisplayUpdates,
    };
    use ironrdp_server::tokio_rustls::{rustls, TlsAcceptor};
    use ironrdp_wtsprotocol_ipc::{
        default_pipe_name, pipe_path, resolve_pipe_name_from_env, ProviderCommand, ServiceEvent, DEFAULT_MAX_FRAME_SIZE,
    };
    use rustls_cng::signer::CngSigningKey;
    use rustls_cng::store::{CertStore, CertStoreType};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::net::windows::named_pipe;
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;
    use tokio::time::{sleep, timeout, Duration};
    use tracing::{error, info, warn};
    use tracing_subscriber::EnvFilter;
    use windows::core::{w, PCWSTR, PWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC,
        DeleteObject, GetDC, HGDIOBJ, ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows::Win32::Security::Cryptography::{
        CertAddCertificateContextToStore, CertCloseStore, CertCreateSelfSignCertificate, CertFindCertificateInStore,
        CertFreeCertificateContext, CertOpenStore, CertStrToNameW, CERT_CONTEXT, CERT_FIND_SUBJECT_STR_W,
        CERT_CREATE_SELFSIGN_FLAGS, CERT_NCRYPT_KEY_SPEC, CERT_OPEN_STORE_FLAGS, CERT_QUERY_ENCODING_TYPE,
        CERT_STORE_ADD_REPLACE_EXISTING, CRYPT_INTEGER_BLOB, NCRYPT_ALLOW_EXPORT_FLAG,
        NCRYPT_ALLOW_PLAINTEXT_EXPORT_FLAG, NCRYPT_FLAGS,
        CERT_STORE_PROV_SYSTEM_W, CERT_SYSTEM_STORE_LOCAL_MACHINE, CERT_X500_NAME_STR, CRYPT_KEY_PROV_INFO,
        NCryptCreatePersistedKey, NCryptFinalizeKey, NCryptFreeObject, NCryptOpenStorageProvider, NCryptSetProperty,
        BCRYPT_RSA_ALGORITHM, HCERTSTORE, MS_KEY_STORAGE_PROVIDER, NCRYPT_EXPORT_POLICY_PROPERTY, NCRYPT_HANDLE,
        NCRYPT_LENGTH_PROPERTY, NCRYPT_MACHINE_KEY_FLAG, NCRYPT_PROV_HANDLE, PKCS_7_ASN_ENCODING,
        X509_ASN_ENCODING,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

    const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
    const LISTEN_ADDR_ENV: &str = "IRONRDP_WTS_LISTEN_ADDR";
    const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:4489";
    const CAPTURE_INTERVAL: Duration = Duration::from_millis(100);
    const TLS_CERT_SUBJECT_FIND: &str = "IronRDP Ceviche Service";
    const TLS_KEY_NAME: &str = "IronRdpCevicheTlsKey";

    struct GdiDisplay {
        desktop_size: DesktopSize,
    }

    impl GdiDisplay {
        fn new() -> anyhow::Result<Self> {
            let desktop_size = desktop_size_from_gdi().context("failed to query desktop size")?;

            info!(
                width = desktop_size.width,
                height = desktop_size.height,
                "Initialized GDI display source"
            );

            Ok(Self { desktop_size })
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
                GdiDisplayUpdates::new(self.desktop_size).context("failed to initialize GDI display updates")?,
            ))
        }
    }

    struct GdiDisplayUpdates {
        desktop_size: DesktopSize,
        sent_first_frame: bool,
    }

    impl GdiDisplayUpdates {
        fn new(size: DesktopSize) -> anyhow::Result<Self> {
            let _ = desktop_size_nonzero(size)?;

            Ok(Self {
                desktop_size: size,
                sent_first_frame: false,
            })
        }
    }

    #[async_trait::async_trait]
    impl RdpServerDisplayUpdates for GdiDisplayUpdates {
        async fn next_update(&mut self) -> anyhow::Result<Option<DisplayUpdate>> {
            if self.sent_first_frame {
                sleep(CAPTURE_INTERVAL).await;
            }

            let bitmap = capture_bitmap_update(self.desktop_size).context("failed to capture desktop frame with GDI")?;
            self.sent_first_frame = true;

            Ok(Some(DisplayUpdate::Bitmap(bitmap)))
        }
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

        // SAFETY: Passing a null HWND obtains the DC for the entire screen.
        let screen_dc = unsafe { GetDC(Some(HWND::default())) };
        if screen_dc.0.is_null() {
            return Err(anyhow!("GetDC returned a null screen device context"));
        }

        // SAFETY: `screen_dc` is a valid display DC obtained above.
        let memory_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
        if memory_dc.0.is_null() {
            // SAFETY: `screen_dc` was acquired with `GetDC` and must be released.
            let _ = unsafe { ReleaseDC(Some(HWND::default()), screen_dc) };
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
        let bitmap = unsafe {
            CreateDIBSection(
                Some(screen_dc),
                &bitmap_info,
                DIB_RGB_COLORS,
                &mut bits_ptr,
                None,
                0,
            )
        }
        .map_err(|error| anyhow!("CreateDIBSection failed: {error}"))?;

        if bitmap.0.is_null() {
            // SAFETY: `memory_dc` and `screen_dc` are valid handles created above.
            let _ = unsafe { DeleteDC(memory_dc) };
            // SAFETY: `screen_dc` was acquired with `GetDC` and must be released.
            let _ = unsafe { ReleaseDC(Some(HWND::default()), screen_dc) };
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
            let _ = unsafe { ReleaseDC(Some(HWND::default()), screen_dc) };
            return Err(anyhow!("SelectObject failed for capture bitmap"));
        }

        // SAFETY: all DC handles are valid and dimensions are taken from initialized state.
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
                SRCCOPY,
            )
        };

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
                let _ = unsafe { ReleaseDC(Some(HWND::default()), screen_dc) };
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
        let _ = unsafe { ReleaseDC(Some(HWND::default()), screen_dc) };

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
            return Err(anyhow!("screen metrics returned zero-sized desktop ({width_u16}x{height_u16})"));
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

            self.listeners.insert(listener_name.clone(), ManagedListener { join_handle });

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
                    warn!(%error, connection_id, "IronRDP connection task failed");
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

        let display = GdiDisplay::new().context("failed to initialize GDI display handler")?;
        let tls_acceptor = make_tls_acceptor().context("failed to initialize TLS acceptor")?;

        let mut server = RdpServer::builder()
            .with_addr(([127, 0, 0, 1], 0))
            .with_tls(tls_acceptor)
            .with_no_input()
            .with_display_handler(display)
            .build();

        server
            .run_connection(stream)
            .await
            .with_context(|| format!("failed to run IronRDP session for connection {connection_id}"))?;

        info!(connection_id, peer_addr = ?peer_addr, "IronRDP session task finished");
        Ok(())
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

            let key_handle = ctx
                .acquire_key(true)
                .context("acquire private key for certificate")?;
            let key = CngSigningKey::new(key_handle).context("wrap CNG signing key")?;

            let chain = ctx
                .as_chain_der()
                .context("certificate chain is not available")?;

            let certs = chain
                .into_iter()
                .map(rustls::pki_types::CertificateDer::from)
                .collect();

            Ok(Arc::new(rustls::sign::CertifiedKey {
                cert: certs,
                key: Arc::new(key),
                ocsp: None,
            }))
        }
    }

    impl rustls::server::ResolvesServerCert for WindowsStoreCertResolver {
        fn resolve(
            &self,
            _client_hello: rustls::server::ClientHello<'_>,
        ) -> Option<Arc<rustls::sign::CertifiedKey>> {
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
        unsafe {
            CertAddCertificateContextToStore(
                Some(store),
                cert_ctx,
                CERT_STORE_ADD_REPLACE_EXISTING,
                None,
            )
        }
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

        let pipe_name = resolve_pipe_name_from_env().unwrap_or_else(default_pipe_name);
        let bind_addr = resolve_bind_addr()?;

        info!(pipe = %pipe_name, "Starting ceviche service control loop");
        info!(bind_addr = %bind_addr, "Configured service listener bind address");

        let mut state = ServiceState::new(bind_addr);

        #[expect(clippy::infinite_loop, reason = "service runs indefinitely; failures are handled inside the loop")]
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
        let payload = serde_json::to_vec(event)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("failed to serialize event: {error}")))?;

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
