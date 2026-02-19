#[cfg(not(windows))]
fn main() {
    eprintln!("ceviche-service is only supported on windows");
}

#[cfg(windows)]
mod windows_main {
    use std::collections::{HashMap, VecDeque};
    use std::io;
    use std::net::SocketAddr;

    use anyhow::Context as _;
    use ironrdp_server::RdpServer;
    use ironrdp_wtsprotocol_ipc::{
        default_pipe_name, pipe_path, resolve_pipe_name_from_env, ProviderCommand, ServiceEvent, DEFAULT_MAX_FRAME_SIZE,
    };
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::net::windows::named_pipe;
    use tokio::sync::mpsc;
    use tokio::task::JoinHandle;
    use tokio::time::{sleep, timeout, Duration};
    use tracing::{error, info, warn};
    use tracing_subscriber::EnvFilter;

    const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
    const LISTEN_ADDR_ENV: &str = "IRONRDP_WTS_LISTEN_ADDR";
    const DEFAULT_LISTEN_ADDR: &str = "0.0.0.0:4489";

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

        let mut server = RdpServer::builder()
            .with_addr(([127, 0, 0, 1], 0))
            .with_no_security()
            .with_no_input()
            .with_no_display()
            .build();

        server
            .run_connection(stream)
            .await
            .with_context(|| format!("failed to run IronRDP session for connection {connection_id}"))?;

        info!(connection_id, peer_addr = ?peer_addr, "IronRDP session task finished");
        Ok(())
    }

    async fn run() -> anyhow::Result<()> {
        init_tracing()?;

        let pipe_name = resolve_pipe_name_from_env().unwrap_or_else(default_pipe_name);
        let bind_addr = resolve_bind_addr()?;

        info!(pipe = %pipe_name, "Starting ceviche service control loop");
        info!(bind_addr = %bind_addr, "Configured service listener bind address");

        let mut state = ServiceState::new(bind_addr);

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
