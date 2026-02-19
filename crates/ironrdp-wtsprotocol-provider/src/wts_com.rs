#![expect(
    clippy::as_pointer_underscore,
    clippy::inline_always,
    clippy::multiple_unsafe_ops_per_block
)]

use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use std::ffi::CString;
use std::fs::OpenOptions;
use std::io::{Read as _, Write};
use std::os::windows::io::{AsRawHandle as _, RawHandle};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::thread;

use ironrdp_pdu::nego;
use ironrdp_wtsprotocol_ipc::{
    default_pipe_name, pipe_path, read_json_message, resolve_pipe_name_from_env, write_json_message, ProviderCommand,
    ServiceEvent, DEFAULT_MAX_FRAME_SIZE,
};
use parking_lot::Mutex;
use tracing::{debug, info, warn};
use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, ERROR_BROKEN_PIPE, ERROR_IO_INCOMPLETE, ERROR_NO_DATA,
    ERROR_SEM_TIMEOUT, E_NOINTERFACE, E_NOTIMPL, E_POINTER, E_UNEXPECTED, HANDLE, HANDLE_PTR,
};
use windows::Win32::System::Com::Marshal::CoMarshalInterThreadInterfaceInStream;
use windows::Win32::System::Com::StructuredStorage::CoGetInterfaceAndReleaseStream;
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, IClassFactory, IClassFactory_Impl, IStream, COINIT_MULTITHREADED,
};
use windows::Win32::System::Pipes::PeekNamedPipe;
use windows::Win32::System::RemoteDesktop::{
    IWRdsProtocolConnection, IWRdsProtocolConnectionCallback, IWRdsProtocolConnection_Impl,
    IWRdsProtocolLicenseConnection, IWRdsProtocolListener, IWRdsProtocolListenerCallback, IWRdsProtocolListener_Impl,
    IWRdsProtocolLogonErrorRedirector, IWRdsProtocolManager, IWRdsProtocolManager_Impl, IWRdsProtocolSettings,
    IWRdsProtocolShadowConnection, WTSVirtualChannelClose, WTSVirtualChannelOpenEx, WTSVirtualChannelRead,
    WTSVirtualChannelWrite, WRDS_CONNECTION_SETTINGS, WRDS_LISTENER_SETTINGS, WRDS_LISTENER_SETTING_LEVEL,
    WRDS_SETTINGS, WTS_CHANNEL_OPTION_DYNAMIC, WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH, WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW,
    WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED, WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL, WTS_CLIENT_DATA, WTS_PROPERTY_VALUE,
    WTS_PROTOCOL_STATUS, WTS_SERVICE_STATE, WTS_SESSION_ID, WTS_USER_CREDENTIAL,
};
use windows_core::{implement, Interface as _, BOOL, GUID, PCSTR, PCWSTR};
use windows_core::{IUnknown, HRESULT};

use crate::auth_bridge::{CredsspPolicy, CredsspServerBridge};
use crate::connection::ProtocolConnection;
use crate::listener::ProtocolListener;
use crate::manager::ProtocolManager;

const S_OK: HRESULT = HRESULT(0);
const S_FALSE: HRESULT = HRESULT(1);

pub const IRONRDP_PROTOCOL_MANAGER_CLSID: GUID = GUID::from_u128(0x89c7ed1e_25e5_4b15_8f52_ae6df4a5ceaf);
pub const IRONRDP_PROTOCOL_MANAGER_CLSID_STR: &str = "{89C7ED1E-25E5-4B15-8F52-AE6DF4A5CEAF}";

static SERVER_LOCK_COUNT: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_OBJECT_COUNT: AtomicUsize = AtomicUsize::new(0);

const IRONRDP_CLIPRDR_CHANNEL_NAME: &str = "cliprdr";
const IRONRDP_RDPSND_CHANNEL_NAME: &str = "rdpsnd";
const IRONRDP_DRDYNVC_CHANNEL_NAME: &str = "drdynvc";
const IRONRDP_DISPLAYCONTROL_CHANNEL_NAME: &str = "Microsoft::Windows::RDS::DisplayControl";
const IRONRDP_GRAPHICS_CHANNEL_NAME: &str = "Microsoft::Windows::RDS::Graphics";
const IRONRDP_AINPUT_CHANNEL_NAME: &str = "FreeRDP::Advanced::Input";
const IRONRDP_ECHO_CHANNEL_NAME: &str = "ECHO";
const VIRTUAL_CHANNEL_FORWARDER_READ_TIMEOUT_MS: u32 = 100;
const VIRTUAL_CHANNEL_FORWARDER_BUFFER_SIZE: usize = 64 * 1024;
const VIRTUAL_CHANNEL_FORWARDER_OUTBOUND_QUEUE_SIZE: usize = 100;
const VIRTUAL_CHANNEL_PIPE_BRIDGE_ENV: &str = "IRONRDP_WTS_VC_BRIDGE_PIPE_PREFIX";
const VIRTUAL_CHANNEL_PIPE_BRIDGE_QUEUE_SIZE: usize = 200;
const VIRTUAL_CHANNEL_PIPE_BRIDGE_RECONNECT_DELAY: Duration = Duration::from_millis(500);
const VIRTUAL_CHANNEL_PIPE_BRIDGE_SEND_TIMEOUT: Duration = Duration::from_millis(100);
const VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE: usize = 1024 * 1024;

type SharedVirtualChannelBridgeHandler = Arc<dyn VirtualChannelBridgeHandler>;

static VIRTUAL_CHANNEL_BRIDGE_HANDLER: OnceLock<Mutex<Option<SharedVirtualChannelBridgeHandler>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualChannelRouteKind {
    Unknown,
    IronRdpStatic,
    IronRdpDynamicBackbone,
    IronRdpDynamicEndpoint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualChannelBridgeEndpoint {
    pub endpoint_name: String,
    pub static_channel: bool,
    pub route_kind: VirtualChannelRouteKind,
}

#[derive(Clone)]
pub struct VirtualChannelBridgeTx {
    endpoint: VirtualChannelBridgeEndpoint,
    outbound_tx: mpsc::SyncSender<Vec<u8>>,
}

impl VirtualChannelBridgeTx {
    pub fn endpoint(&self) -> &VirtualChannelBridgeEndpoint {
        &self.endpoint
    }

    pub fn send(&self, payload: Vec<u8>) -> windows_core::Result<()> {
        self.outbound_tx
            .send(payload)
            .map_err(|_| windows_core::Error::new(E_UNEXPECTED, "virtual channel bridge sender is closed"))
    }
}

pub trait VirtualChannelBridgeHandler: Send + Sync {
    fn on_channel_opened(&self, endpoint: &VirtualChannelBridgeEndpoint, tx: VirtualChannelBridgeTx);

    fn on_channel_data(&self, endpoint: &VirtualChannelBridgeEndpoint, data: &[u8]);

    fn on_channel_closed(&self, endpoint: &VirtualChannelBridgeEndpoint);
}

pub fn set_virtual_channel_bridge_handler(handler: Option<Arc<dyn VirtualChannelBridgeHandler>>) {
    *virtual_channel_bridge_handler_slot().lock() = handler;
}

fn virtual_channel_bridge_handler_slot() -> &'static Mutex<Option<SharedVirtualChannelBridgeHandler>> {
    VIRTUAL_CHANNEL_BRIDGE_HANDLER.get_or_init(|| Mutex::new(None))
}

fn get_virtual_channel_bridge_handler() -> Option<SharedVirtualChannelBridgeHandler> {
    virtual_channel_bridge_handler_slot().lock().clone()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IronRdpVirtualChannelServer {
    Cliprdr,
    Rdpsnd,
    Drdynvc,
    DisplayControl,
    Graphics,
    AdvancedInput,
    Echo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VirtualChannelBridgePlan {
    route_kind: VirtualChannelRouteKind,
    hook_target: Option<IronRdpVirtualChannelServer>,
    preferred_dynamic_priority: Option<u32>,
}

impl VirtualChannelBridgePlan {
    fn for_endpoint(is_static: bool, hook_target: Option<IronRdpVirtualChannelServer>) -> Self {
        let route_kind = match hook_target {
            Some(IronRdpVirtualChannelServer::Cliprdr) | Some(IronRdpVirtualChannelServer::Rdpsnd) => {
                VirtualChannelRouteKind::IronRdpStatic
            }
            Some(IronRdpVirtualChannelServer::Drdynvc) => VirtualChannelRouteKind::IronRdpDynamicBackbone,
            Some(IronRdpVirtualChannelServer::DisplayControl)
            | Some(IronRdpVirtualChannelServer::Graphics)
            | Some(IronRdpVirtualChannelServer::AdvancedInput)
            | Some(IronRdpVirtualChannelServer::Echo) => VirtualChannelRouteKind::IronRdpDynamicEndpoint,
            None => VirtualChannelRouteKind::Unknown,
        };

        let preferred_dynamic_priority = if is_static {
            None
        } else {
            hook_target.map(IronRdpVirtualChannelServer::default_dynamic_priority)
        };

        Self {
            route_kind,
            hook_target,
            preferred_dynamic_priority,
        }
    }

    fn should_prepare_forwarding(self) -> bool {
        self.route_kind != VirtualChannelRouteKind::Unknown
    }
}

impl IronRdpVirtualChannelServer {
    fn name(self) -> &'static str {
        match self {
            Self::Cliprdr => IRONRDP_CLIPRDR_CHANNEL_NAME,
            Self::Rdpsnd => IRONRDP_RDPSND_CHANNEL_NAME,
            Self::Drdynvc => IRONRDP_DRDYNVC_CHANNEL_NAME,
            Self::DisplayControl => IRONRDP_DISPLAYCONTROL_CHANNEL_NAME,
            Self::Graphics => IRONRDP_GRAPHICS_CHANNEL_NAME,
            Self::AdvancedInput => IRONRDP_AINPUT_CHANNEL_NAME,
            Self::Echo => IRONRDP_ECHO_CHANNEL_NAME,
        }
    }

    fn requires_drdynvc_backbone(self) -> bool {
        matches!(
            self,
            Self::DisplayControl | Self::Graphics | Self::AdvancedInput | Self::Echo
        )
    }

    fn default_dynamic_priority(self) -> u32 {
        match self {
            Self::AdvancedInput => WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL,
            Self::Graphics => WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH,
            Self::DisplayControl => WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED,
            Self::Echo => WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW,
            Self::Cliprdr | Self::Rdpsnd | Self::Drdynvc => WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW,
        }
    }
}

fn endpoint_name_eq(lhs: &str, rhs: &str) -> bool {
    lhs.eq_ignore_ascii_case(rhs)
}

fn ironrdp_virtual_channel_server(endpoint_name: &str, is_static: bool) -> Option<IronRdpVirtualChannelServer> {
    if is_static {
        if endpoint_name_eq(endpoint_name, IRONRDP_CLIPRDR_CHANNEL_NAME) {
            return Some(IronRdpVirtualChannelServer::Cliprdr);
        }
        if endpoint_name_eq(endpoint_name, IRONRDP_RDPSND_CHANNEL_NAME) {
            return Some(IronRdpVirtualChannelServer::Rdpsnd);
        }
        if endpoint_name_eq(endpoint_name, IRONRDP_DRDYNVC_CHANNEL_NAME) {
            return Some(IronRdpVirtualChannelServer::Drdynvc);
        }

        return None;
    }

    if endpoint_name_eq(endpoint_name, IRONRDP_DISPLAYCONTROL_CHANNEL_NAME) {
        return Some(IronRdpVirtualChannelServer::DisplayControl);
    }
    if endpoint_name_eq(endpoint_name, IRONRDP_GRAPHICS_CHANNEL_NAME) {
        return Some(IronRdpVirtualChannelServer::Graphics);
    }
    if endpoint_name_eq(endpoint_name, IRONRDP_AINPUT_CHANNEL_NAME) {
        return Some(IronRdpVirtualChannelServer::AdvancedInput);
    }
    if endpoint_name_eq(endpoint_name, IRONRDP_ECHO_CHANNEL_NAME) {
        return Some(IronRdpVirtualChannelServer::Echo);
    }

    None
}

fn is_dynamic_channel_priority(value: u32) -> bool {
    matches!(
        value,
        WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
            | WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
            | WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH
            | WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL
    )
}

fn virtual_channel_requested_priority(
    is_static: bool,
    requested_priority: u32,
    hook_target: Option<IronRdpVirtualChannelServer>,
) -> u32 {
    if is_static {
        return requested_priority;
    }

    if requested_priority == WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW {
        return hook_target
            .map(IronRdpVirtualChannelServer::default_dynamic_priority)
            .unwrap_or(requested_priority);
    }

    if is_dynamic_channel_priority(requested_priority) {
        return requested_priority;
    }

    hook_target
        .map(IronRdpVirtualChannelServer::default_dynamic_priority)
        .unwrap_or(requested_priority)
}

#[derive(Debug)]
struct ListenerWorker {
    stop_tx: mpsc::Sender<()>,
    join_handle: thread::JoinHandle<()>,
}

#[derive(Debug, Clone)]
struct ProviderControlBridge {
    pipe_name: Option<String>,
    optional_connection: bool,
}

#[derive(Debug, Clone)]
struct IncomingConnection {
    connection_id: u32,
    peer_addr: Option<String>,
}

impl ProviderControlBridge {
    fn from_env() -> Self {
        if let Some(pipe_name) = resolve_pipe_name_from_env() {
            return Self {
                pipe_name: Some(pipe_name),
                optional_connection: false,
            };
        }

        Self {
            pipe_name: Some(default_pipe_name()),
            optional_connection: true,
        }
    }

    fn start_listen(&self, listener_name: &str) -> windows_core::Result<bool> {
        let Some(event) = self.send_command(&ProviderCommand::StartListen {
            listener_name: listener_name.to_owned(),
        })?
        else {
            return Ok(false);
        };

        match event {
            ServiceEvent::ListenerStarted {
                listener_name: started_listener,
            } if started_listener == listener_name => Ok(true),
            ServiceEvent::Ack => Ok(true),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            other => Err(windows_core::Error::new(
                E_UNEXPECTED,
                format!("unexpected service event on start listen: {other:?}"),
            )),
        }
    }

    fn stop_listen(&self, listener_name: &str) -> windows_core::Result<()> {
        let Some(event) = self.send_command(&ProviderCommand::StopListen {
            listener_name: listener_name.to_owned(),
        })?
        else {
            return Ok(());
        };

        match event {
            ServiceEvent::ListenerStopped {
                listener_name: stopped_listener,
            } if stopped_listener == listener_name => Ok(()),
            ServiceEvent::Ack => Ok(()),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            other => Err(windows_core::Error::new(
                E_UNEXPECTED,
                format!("unexpected service event on stop listen: {other:?}"),
            )),
        }
    }

    fn accept_connection(&self, connection_id: u32) -> windows_core::Result<()> {
        let Some(event) = self.send_command(&ProviderCommand::AcceptConnection { connection_id })? else {
            return Ok(());
        };

        match event {
            ServiceEvent::ConnectionReady {
                connection_id: ready_connection_id,
            } if ready_connection_id == connection_id => Ok(()),
            ServiceEvent::Ack => Ok(()),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            other => Err(windows_core::Error::new(
                E_UNEXPECTED,
                format!("unexpected service event on accept connection: {other:?}"),
            )),
        }
    }

    fn close_connection(&self, connection_id: u32) -> windows_core::Result<()> {
        let Some(event) = self.send_command(&ProviderCommand::CloseConnection { connection_id })? else {
            return Ok(());
        };

        match event {
            ServiceEvent::Ack => Ok(()),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            _ => Ok(()),
        }
    }

    fn wait_for_incoming(
        &self,
        listener_name: &str,
        timeout_ms: u32,
    ) -> windows_core::Result<Option<IncomingConnection>> {
        let Some(event) = self.send_command(&ProviderCommand::WaitForIncoming {
            listener_name: listener_name.to_owned(),
            timeout_ms,
        })?
        else {
            return Ok(None);
        };

        match event {
            ServiceEvent::IncomingConnection {
                listener_name: service_listener_name,
                connection_id,
                peer_addr,
            } => {
                if service_listener_name != listener_name {
                    return Err(windows_core::Error::new(
                        E_UNEXPECTED,
                        format!(
                            "incoming connection listener mismatch: expected {listener_name} got {service_listener_name}"
                        ),
                    ));
                }

                Ok(Some(IncomingConnection {
                    connection_id,
                    peer_addr,
                }))
            }
            ServiceEvent::NoIncoming | ServiceEvent::Ack => Ok(None),
            ServiceEvent::Error { message } => Err(windows_core::Error::new(E_UNEXPECTED, message)),
            other => Err(windows_core::Error::new(
                E_UNEXPECTED,
                format!("unexpected service event on wait incoming: {other:?}"),
            )),
        }
    }

    fn send_command(&self, command: &ProviderCommand) -> windows_core::Result<Option<ServiceEvent>> {
        let Some(pipe_name) = self.pipe_name.as_ref() else {
            return Ok(None);
        };

        let full_pipe_name = pipe_path(pipe_name);
        let pipe_result = OpenOptions::new().read(true).write(true).open(&full_pipe_name);

        let mut pipe = match pipe_result {
            Ok(pipe) => pipe,
            Err(error) if self.optional_connection && is_optional_control_pipe_error(&error) => {
                debug!(%error, pipe = %full_pipe_name, "Companion control pipe not available; using local fallback");
                return Ok(None);
            }
            Err(error) => {
                return Err(io_error_to_windows_error(error, "failed to connect to control pipe"));
            }
        };

        write_json_message(&mut pipe, command)
            .map_err(|error| io_error_to_windows_error(error, "failed to send control command"))?;

        let event = read_json_message::<ServiceEvent>(&mut pipe, DEFAULT_MAX_FRAME_SIZE)
            .map_err(|error| io_error_to_windows_error(error, "failed to read control response"))?;

        Ok(Some(event))
    }
}

fn is_optional_control_pipe_error(error: &std::io::Error) -> bool {
    use std::io::ErrorKind;

    matches!(
        error.kind(),
        ErrorKind::NotFound
            | ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionReset
            | ErrorKind::TimedOut
            | ErrorKind::BrokenPipe
            | ErrorKind::WouldBlock
    )
}

pub fn create_protocol_manager_com() -> IWRdsProtocolManager {
    install_default_virtual_channel_bridge_handler_from_env();
    ComProtocolManager::new().into()
}

fn install_default_virtual_channel_bridge_handler_from_env() {
    if get_virtual_channel_bridge_handler().is_some() {
        return;
    }

    let Some(pipe_prefix) = std::env::var(VIRTUAL_CHANNEL_PIPE_BRIDGE_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return;
    };

    info!(
        pipe_prefix = %pipe_prefix,
        "Installing default virtual channel named-pipe bridge handler"
    );
    set_virtual_channel_bridge_handler(Some(Arc::new(NamedPipeBridgeHandler::new(pipe_prefix))));
}

struct NamedPipeBridgeHandler {
    pipe_prefix: String,
    workers: Mutex<std::collections::HashMap<String, NamedPipeBridgeWorker>>,
    bridge_txs: Mutex<std::collections::HashMap<String, VirtualChannelBridgeTx>>,
}

impl NamedPipeBridgeHandler {
    fn new(pipe_prefix: String) -> Self {
        Self {
            pipe_prefix,
            workers: Mutex::new(std::collections::HashMap::new()),
            bridge_txs: Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn restart_worker(
        &self,
        endpoint: &VirtualChannelBridgeEndpoint,
        tx: VirtualChannelBridgeTx,
    ) -> mpsc::SyncSender<Vec<u8>> {
        let endpoint_key = bridge_endpoint_key(endpoint);
        let pipe_path = bridge_pipe_path(&self.pipe_prefix, endpoint);
        let endpoint_for_worker = endpoint.clone();

        let (to_pipe_tx, to_pipe_rx) = mpsc::sync_channel(VIRTUAL_CHANNEL_PIPE_BRIDGE_QUEUE_SIZE);
        let (stop_tx, stop_rx) = mpsc::channel();

        let join_handle = thread::spawn(move || {
            run_named_pipe_bridge_worker(endpoint_for_worker, pipe_path, tx, to_pipe_rx, stop_rx)
        });

        let mut workers = self.workers.lock();
        if let Some(previous) = workers.insert(
            endpoint_key,
            NamedPipeBridgeWorker {
                stop_tx,
                to_pipe_tx: to_pipe_tx.clone(),
                join_handle,
            },
        ) {
            previous.stop_and_join();
        }

        to_pipe_tx
    }

    fn stop_worker(&self, endpoint: &VirtualChannelBridgeEndpoint) {
        let endpoint_key = bridge_endpoint_key(endpoint);
        if let Some(worker) = self.workers.lock().remove(&endpoint_key) {
            worker.stop_and_join();
        }
    }

    fn get_bridge_tx(&self, endpoint: &VirtualChannelBridgeEndpoint) -> Option<VirtualChannelBridgeTx> {
        self.bridge_txs.lock().get(&bridge_endpoint_key(endpoint)).cloned()
    }
}

impl VirtualChannelBridgeHandler for NamedPipeBridgeHandler {
    fn on_channel_opened(&self, endpoint: &VirtualChannelBridgeEndpoint, tx: VirtualChannelBridgeTx) {
        self.bridge_txs.lock().insert(bridge_endpoint_key(endpoint), tx.clone());
        let _ = self.restart_worker(endpoint, tx);
    }

    fn on_channel_data(&self, endpoint: &VirtualChannelBridgeEndpoint, data: &[u8]) {
        let endpoint_key = bridge_endpoint_key(endpoint);
        let worker_tx = {
            let workers = self.workers.lock();
            workers.get(&endpoint_key).map(|worker| worker.to_pipe_tx.clone())
        };

        let tx = if let Some(tx) = worker_tx {
            tx
        } else {
            let Some(bridge_tx) = self.get_bridge_tx(endpoint) else {
                warn!(
                    endpoint = %endpoint.endpoint_name,
                    "Named-pipe bridge worker unavailable and bridge tx is not registered"
                );
                return;
            };

            self.restart_worker(endpoint, bridge_tx)
        };

        let result = tx.send(data.to_vec());

        if result.is_err() {
            warn!(
                endpoint = %endpoint.endpoint_name,
                "Failed to queue payload into named-pipe bridge worker"
            );
        }
    }

    fn on_channel_closed(&self, endpoint: &VirtualChannelBridgeEndpoint) {
        self.bridge_txs.lock().remove(&bridge_endpoint_key(endpoint));
        self.stop_worker(endpoint);
    }
}

struct NamedPipeBridgeWorker {
    stop_tx: mpsc::Sender<()>,
    to_pipe_tx: mpsc::SyncSender<Vec<u8>>,
    join_handle: thread::JoinHandle<()>,
}

impl NamedPipeBridgeWorker {
    fn stop_and_join(self) {
        if let Err(error) = self.stop_tx.send(()) {
            warn!(%error, "Failed to stop named-pipe bridge worker");
        }

        if let Err(error) = self.join_handle.join() {
            warn!(?error, "Named-pipe bridge worker thread panicked");
        }
    }
}

fn run_named_pipe_bridge_worker(
    endpoint: VirtualChannelBridgeEndpoint,
    pipe_path: String,
    to_channel_tx: VirtualChannelBridgeTx,
    to_pipe_rx: mpsc::Receiver<Vec<u8>>,
    stop_rx: mpsc::Receiver<()>,
) {
    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        let open_result = OpenOptions::new().read(true).write(true).open(&pipe_path);
        let mut pipe = match open_result {
            Ok(pipe) => {
                info!(endpoint = %endpoint.endpoint_name, pipe = %pipe_path, "Connected named-pipe bridge worker");
                pipe
            }
            Err(error) => {
                debug!(
                    endpoint = %endpoint.endpoint_name,
                    pipe = %pipe_path,
                    %error,
                    "Named-pipe bridge worker waiting for server"
                );
                thread::sleep(VIRTUAL_CHANNEL_PIPE_BRIDGE_RECONNECT_DELAY);
                continue;
            }
        };

        let mut from_pipe_buffer = Vec::with_capacity(4096);

        loop {
            if stop_rx.try_recv().is_ok() {
                return;
            }

            let mut write_failed = false;

            match to_pipe_rx.recv_timeout(VIRTUAL_CHANNEL_PIPE_BRIDGE_SEND_TIMEOUT) {
                Ok(payload) => {
                    if let Err(error) = write_length_prefixed(&mut pipe, &payload) {
                        warn!(
                            endpoint = %endpoint.endpoint_name,
                            pipe = %pipe_path,
                            %error,
                            "Named-pipe bridge write failed; reconnecting"
                        );
                        write_failed = true;
                    }

                    if write_failed {
                        break;
                    }

                    while let Ok(queued_payload) = to_pipe_rx.try_recv() {
                        if let Err(error) = write_length_prefixed(&mut pipe, &queued_payload) {
                            warn!(
                                endpoint = %endpoint.endpoint_name,
                                pipe = %pipe_path,
                                %error,
                                "Named-pipe bridge write failed while draining queue; reconnecting"
                            );
                            write_failed = true;
                            break;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }

            if write_failed {
                break;
            }

            if let Err(error) =
                pump_named_pipe_inbound_frames(&mut pipe, &endpoint, &to_channel_tx, &mut from_pipe_buffer)
            {
                warn!(
                    endpoint = %endpoint.endpoint_name,
                    pipe = %pipe_path,
                    %error,
                    "Named-pipe bridge read failed; reconnecting"
                );
                break;
            }
        }
    }
}

fn pump_named_pipe_inbound_frames(
    pipe: &mut std::fs::File,
    endpoint: &VirtualChannelBridgeEndpoint,
    to_channel_tx: &VirtualChannelBridgeTx,
    read_buffer: &mut Vec<u8>,
) -> std::io::Result<()> {
    let mut chunk = [0u8; 8192];

    loop {
        let available = named_pipe_available_bytes(pipe)?;
        if available == 0 {
            break;
        }

        let read_len = usize::try_from(available).unwrap_or(usize::MAX).min(chunk.len());

        let read_count = match pipe.read(&mut chunk[..read_len]) {
            Ok(0) => {
                return Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "named pipe closed"));
            }
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };

        read_buffer.extend_from_slice(&chunk[..read_count]);
        drain_length_prefixed_pipe_frames(endpoint, to_channel_tx, read_buffer)?;
    }

    Ok(())
}

fn named_pipe_available_bytes(pipe: &std::fs::File) -> std::io::Result<u32> {
    let mut total_bytes_available = 0u32;

    // SAFETY: `pipe.as_raw_handle()` returns a live OS handle for this file. We only ask
    // for the available byte count and provide a valid out-pointer.
    unsafe {
        PeekNamedPipe(
            HANDLE(pipe.as_raw_handle()),
            None,
            0,
            None,
            Some(&mut total_bytes_available),
            None,
        )
    }
    .map_err(|error| {
        let kind = if error.code() == HRESULT::from_win32(ERROR_BROKEN_PIPE.0)
            || error.code() == HRESULT::from_win32(ERROR_NO_DATA.0)
        {
            std::io::ErrorKind::BrokenPipe
        } else {
            std::io::ErrorKind::Other
        };

        std::io::Error::new(kind, format!("failed to peek named pipe: {error}"))
    })?;

    Ok(total_bytes_available)
}

fn drain_length_prefixed_pipe_frames(
    endpoint: &VirtualChannelBridgeEndpoint,
    to_channel_tx: &VirtualChannelBridgeTx,
    read_buffer: &mut Vec<u8>,
) -> std::io::Result<()> {
    let mut frame_offset = 0usize;

    while read_buffer.len().saturating_sub(frame_offset) >= 4 {
        let frame_len_u32 = u32::from_le_bytes([
            read_buffer[frame_offset],
            read_buffer[frame_offset + 1],
            read_buffer[frame_offset + 2],
            read_buffer[frame_offset + 3],
        ]);
        let frame_len = usize::try_from(frame_len_u32).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "named-pipe bridge frame length does not fit in usize",
            )
        })?;

        if frame_len > VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "named-pipe bridge frame length exceeds limit (len={frame_len}, max={VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE})"
                ),
            ));
        }

        let frame_total_len = 4usize + frame_len;
        if read_buffer.len().saturating_sub(frame_offset) < frame_total_len {
            break;
        }

        let payload_start = frame_offset + 4;
        let payload_end = payload_start + frame_len;
        let payload = read_buffer[payload_start..payload_end].to_vec();

        match to_channel_tx.outbound_tx.try_send(payload) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                warn!(
                    endpoint = %endpoint.endpoint_name,
                    "Dropped named-pipe inbound frame because virtual channel outbound queue is full"
                );
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "virtual channel bridge sender is closed",
                ));
            }
        }

        frame_offset = payload_end;
    }

    if frame_offset > 0 {
        read_buffer.drain(..frame_offset);
    }

    Ok(())
}

fn write_length_prefixed(mut writer: impl Write, payload: &[u8]) -> std::io::Result<()> {
    let len = u32::try_from(payload.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "payload too large"))?;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(payload)
}

fn bridge_endpoint_key(endpoint: &VirtualChannelBridgeEndpoint) -> String {
    let kind = if endpoint.static_channel { "svc" } else { "dvc" };
    format!("{kind}:{}", endpoint.endpoint_name.to_ascii_lowercase())
}

fn bridge_pipe_path(pipe_prefix: &str, endpoint: &VirtualChannelBridgeEndpoint) -> String {
    let normalized_prefix = if pipe_prefix.starts_with(r"\\.\pipe\") {
        pipe_prefix.to_owned()
    } else {
        format!(r"\\.\pipe\{pipe_prefix}")
    };

    let kind = if endpoint.static_channel { "svc" } else { "dvc" };
    let channel = sanitize_pipe_segment(&endpoint.endpoint_name);

    format!("{normalized_prefix}.{kind}.{channel}")
}

fn sanitize_pipe_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());

    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('_');
        }
    }

    if out.is_empty() {
        "channel".to_owned()
    } else {
        out
    }
}

#[implement(IClassFactory)]
struct ProtocolManagerClassFactory;

impl IClassFactory_Impl for ProtocolManagerClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: windows_core::Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut core::ffi::c_void,
    ) -> windows_core::Result<()> {
        if ppvobject.is_null() || riid.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null COM output pointer"));
        }

        if punkouter.is_some() {
            return Err(windows_core::Error::new(
                CLASS_E_NOAGGREGATION,
                "aggregation is not supported",
            ));
        }

        // SAFETY: `ppvobject` is non-null (checked above) and COM expects us to
        // initialize out-pointers on all paths.
        unsafe { *ppvobject = core::ptr::null_mut() };

        // SAFETY: `riid` is non-null (checked above) and points to a valid GUID per COM contract.
        let requested_iid = unsafe { *riid };

        if requested_iid == IWRdsProtocolManager::IID {
            let manager = create_protocol_manager_com();
            // SAFETY: `ppvobject` is non-null and this branch returns a valid COM interface pointer.
            unsafe { *ppvobject = manager.into_raw() };

            return Ok(());
        }

        if requested_iid == IUnknown::IID {
            let manager = create_protocol_manager_com();
            let unknown: IUnknown = manager.cast()?;
            // SAFETY: `ppvobject` is non-null and this branch returns a valid COM interface pointer.
            unsafe { *ppvobject = unknown.into_raw() };

            return Ok(());
        }

        Err(windows_core::Error::new(
            E_NOINTERFACE,
            "requested interface is not supported",
        ))
    }

    fn LockServer(&self, flock: BOOL) -> windows_core::Result<()> {
        if flock.as_bool() {
            SERVER_LOCK_COUNT.fetch_add(1, Ordering::SeqCst);
        } else {
            let _ =
                SERVER_LOCK_COUNT.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| current.checked_sub(1));
        }

        Ok(())
    }
}

#[expect(unreachable_pub)]
#[unsafe(no_mangle)]
pub extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> HRESULT {
    let result = dll_get_class_object_impl(rclsid, riid, ppv);

    match result {
        Ok(()) => S_OK,
        Err(error) => error.code(),
    }
}

#[expect(unreachable_pub)]
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if SERVER_LOCK_COUNT.load(Ordering::SeqCst) == 0 && ACTIVE_OBJECT_COUNT.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

fn dll_get_class_object_impl(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut core::ffi::c_void,
) -> windows_core::Result<()> {
    if ppv.is_null() || riid.is_null() || rclsid.is_null() {
        return Err(windows_core::Error::new(E_POINTER, "null class object pointer"));
    }

    // SAFETY: `ppv` is non-null (checked above) and COM expects out-pointers to be initialized.
    unsafe { *ppv = core::ptr::null_mut() };

    // SAFETY: `rclsid` is non-null (checked above) and points to a valid GUID per COM contract.
    let requested_clsid = unsafe { *rclsid };
    if requested_clsid != IRONRDP_PROTOCOL_MANAGER_CLSID {
        return Err(windows_core::Error::new(
            CLASS_E_CLASSNOTAVAILABLE,
            "unknown protocol manager CLSID",
        ));
    }

    let factory: IClassFactory = ProtocolManagerClassFactory.into();
    // SAFETY: `riid` is non-null (checked above) and points to a valid GUID per COM contract.
    let requested_iid = unsafe { *riid };

    if requested_iid == IClassFactory::IID {
        // SAFETY: `ppv` is non-null and this branch returns a valid COM interface pointer.
        unsafe { *ppv = factory.into_raw() };

        return Ok(());
    }

    if requested_iid == IUnknown::IID {
        let unknown: IUnknown = factory.cast()?;
        // SAFETY: `ppv` is non-null and this branch returns a valid COM interface pointer.
        unsafe { *ppv = unknown.into_raw() };

        return Ok(());
    }

    Err(windows_core::Error::new(
        E_NOINTERFACE,
        "requested class factory interface is not supported",
    ))
}

#[implement(IWRdsProtocolManager)]
struct ComProtocolManager {
    _lifetime: ComObjectLifetime,
    inner: ProtocolManager,
}

impl ComProtocolManager {
    fn new() -> Self {
        Self {
            _lifetime: ComObjectLifetime::new(),
            inner: ProtocolManager::new(),
        }
    }
}

#[derive(Debug)]
struct ComObjectLifetime;

impl ComObjectLifetime {
    fn new() -> Self {
        ACTIVE_OBJECT_COUNT.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for ComObjectLifetime {
    fn drop(&mut self) {
        ACTIVE_OBJECT_COUNT.fetch_sub(1, Ordering::SeqCst);
    }
}

impl IWRdsProtocolManager_Impl for ComProtocolManager_Impl {
    fn Initialize(
        &self,
        _piwrdssettings: windows_core::Ref<'_, IWRdsProtocolSettings>,
        _pwrdssettings: *const WRDS_SETTINGS,
    ) -> windows_core::Result<()> {
        info!("Initialized protocol manager");
        Ok(())
    }

    fn CreateListener(&self, wszlistenername: &PCWSTR) -> windows_core::Result<IWRdsProtocolListener> {
        let listener_name = if wszlistenername.is_null() {
            "IRDP-Tcp".to_owned()
        } else {
            // SAFETY: listener name is provided by termservice and expected to be a valid
            // NUL-terminated wide string.
            unsafe { wszlistenername.to_string() }.map_err(|error| {
                windows_core::Error::new(E_UNEXPECTED, format!("failed to decode listener name: {error}"))
            })?
        };

        info!(listener_name = %listener_name, "Created protocol listener");
        Ok(ComProtocolListener::new(self.inner.create_listener(), listener_name).into())
    }

    fn NotifyServiceStateChange(&self, _ptsservicestatechange: *const WTS_SERVICE_STATE) -> windows_core::Result<()> {
        info!("Received service state change notification");
        Ok(())
    }

    fn NotifySessionOfServiceStart(&self, _sessionid: *const WTS_SESSION_ID) -> windows_core::Result<()> {
        debug!("Received session service start notification");
        Ok(())
    }

    fn NotifySessionOfServiceStop(&self, _sessionid: *const WTS_SESSION_ID) -> windows_core::Result<()> {
        debug!("Received session service stop notification");
        Ok(())
    }

    fn NotifySessionStateChange(&self, _sessionid: *const WTS_SESSION_ID, eventid: u32) -> windows_core::Result<()> {
        debug!(eventid, "Received session state change notification");
        Ok(())
    }

    fn NotifySettingsChange(&self, _pwrdssettings: *const WRDS_SETTINGS) -> windows_core::Result<()> {
        info!("Received protocol settings change notification");
        Ok(())
    }

    fn Uninitialize(&self) -> windows_core::Result<()> {
        info!("Uninitialized protocol manager");
        Ok(())
    }
}

#[implement(IWRdsProtocolListener)]
struct ComProtocolListener {
    inner: Arc<ProtocolListener>,
    listener_name: String,
    control_bridge: ProviderControlBridge,
    callback: Mutex<Option<IWRdsProtocolListenerCallback>>,
    worker: Mutex<Option<ListenerWorker>>,
}

impl ComProtocolListener {
    fn new(inner: ProtocolListener, listener_name: String) -> Self {
        Self {
            inner: Arc::new(inner),
            listener_name,
            control_bridge: ProviderControlBridge::from_env(),
            callback: Mutex::new(None),
            worker: Mutex::new(None),
        }
    }
}

impl IWRdsProtocolListener_Impl for ComProtocolListener_Impl {
    fn GetSettings(
        &self,
        _wrdslistenersettinglevel: WRDS_LISTENER_SETTING_LEVEL,
    ) -> windows_core::Result<WRDS_LISTENER_SETTINGS> {
        Ok(WRDS_LISTENER_SETTINGS::default())
    }

    fn StartListen(&self, pcallback: windows_core::Ref<'_, IWRdsProtocolListenerCallback>) -> windows_core::Result<()> {
        let callback = pcallback
            .ok()
            .map_err(|_| windows_core::Error::new(E_POINTER, "null listener callback"))?
            .clone();

        if self.worker.lock().is_some() {
            info!("Protocol listener already started");
            return Ok(());
        }

        let control_bridge_enabled = self.control_bridge.start_listen(&self.listener_name)?;

        let (stop_tx, stop_rx) = mpsc::channel();
        // SAFETY: we marshal a valid COM callback interface pointer into a stream token
        // so the worker thread can unmarshal it in its own COM apartment.
        let callback_stream =
            unsafe { CoMarshalInterThreadInterfaceInStream(&IWRdsProtocolListenerCallback::IID, &callback) }?;
        let callback_stream_token = stream_ptr_to_token(callback_stream.into_raw());
        let listener = Arc::clone(&self.inner);
        let control_bridge = self.control_bridge.clone();
        let listener_name = self.listener_name.clone();

        let join_handle = thread::spawn(move || {
            // SAFETY: each worker thread initializes and uninitializes COM exactly once.
            let co_initialize = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if let Err(error) = co_initialize.ok() {
                warn!(%error, "Failed to initialize COM on listener worker thread");
                return;
            }

            // SAFETY: token was produced by `IStream::into_raw` in this process.
            let callback_stream = unsafe { IStream::from_raw(token_to_stream_ptr(callback_stream_token)) };
            // SAFETY: stream token contains a marshaled callback interface and this function
            // transfers ownership of the stream reference back to COM.
            let callback_for_worker =
                unsafe { CoGetInterfaceAndReleaseStream::<_, IWRdsProtocolListenerCallback>(&callback_stream) };
            std::mem::forget(callback_stream);

            let callback_for_worker = match callback_for_worker {
                Ok(callback_for_worker) => callback_for_worker,
                Err(error) => {
                    warn!(%error, "Failed to unmarshal listener callback in worker thread");
                    // SAFETY: paired with successful `CoInitializeEx` above.
                    unsafe { CoUninitialize() };
                    return;
                }
            };

            if control_bridge_enabled {
                loop {
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }

                    let incoming = match control_bridge.wait_for_incoming(&listener_name, 250) {
                        Ok(incoming) => incoming,
                        Err(error) => {
                            warn!(%error, listener_name = %listener_name, "Failed to poll incoming connection from companion service");
                            thread::sleep(Duration::from_millis(200));
                            continue;
                        }
                    };

                    let Some(incoming) = incoming else {
                        continue;
                    };

                    let connection_entry = listener.create_connection_with_id(incoming.connection_id);
                    let connection_callback_slot = Arc::new(Mutex::new(None));
                    let connection: IWRdsProtocolConnection = ComProtocolConnection::new(
                        connection_entry,
                        Arc::clone(&connection_callback_slot),
                        control_bridge.clone(),
                    )
                    .into();

                    let settings = WRDS_CONNECTION_SETTINGS::default();

                    // SAFETY: COM callback and connection object are valid for call duration.
                    match unsafe { callback_for_worker.OnConnected(&connection, &settings) } {
                        Ok(connection_callback) => {
                            *connection_callback_slot.lock() = Some(connection_callback);
                            debug!(
                                connection_id = incoming.connection_id,
                                peer_addr = ?incoming.peer_addr,
                                "Dispatched incoming connection from companion service"
                            );
                        }
                        Err(error) => {
                            warn!(
                                %error,
                                connection_id = incoming.connection_id,
                                "Failed to dispatch OnConnected callback for incoming connection"
                            );
                        }
                    }
                }
            } else {
                let bootstrap_connection = listener.create_connection();
                let connection_callback_slot = Arc::new(Mutex::new(None));
                let connection: IWRdsProtocolConnection = ComProtocolConnection::new(
                    bootstrap_connection,
                    Arc::clone(&connection_callback_slot),
                    control_bridge,
                )
                .into();

                let settings = WRDS_CONNECTION_SETTINGS::default();

                // SAFETY: COM callback and connection object are valid for call duration.
                match unsafe { callback_for_worker.OnConnected(&connection, &settings) } {
                    Ok(connection_callback) => {
                        *connection_callback_slot.lock() = Some(connection_callback);
                    }
                    Err(error) => {
                        warn!(%error, "Failed to dispatch OnConnected callback");
                    }
                }

                let _ = stop_rx.recv();
            }

            // SAFETY: paired with successful `CoInitializeEx` above.
            unsafe { CoUninitialize() };
        });

        *self.worker.lock() = Some(ListenerWorker { stop_tx, join_handle });
        *self.callback.lock() = Some(callback);
        info!("Started protocol listener worker");

        Ok(())
    }

    fn StopListen(&self) -> windows_core::Result<()> {
        if let Some(worker) = self.worker.lock().take() {
            if let Err(error) = worker.stop_tx.send(()) {
                warn!(%error, "Failed to signal listener worker stop");
            }

            if let Err(error) = worker.join_handle.join() {
                warn!(?error, "Listener worker thread panicked");
            }
        }

        *self.callback.lock() = None;

        if let Err(error) = self.control_bridge.stop_listen(&self.listener_name) {
            warn!(%error, listener_name = %self.listener_name, "Failed to stop companion service listener");
        }

        info!("Stopped protocol listener worker");
        Ok(())
    }
}

#[implement(IWRdsProtocolConnection)]
struct ComProtocolConnection {
    inner: Arc<ProtocolConnection>,
    auth_bridge: CredsspServerBridge,
    connection_callback: Arc<Mutex<Option<IWRdsProtocolConnectionCallback>>>,
    control_bridge: ProviderControlBridge,
    ready_notified: Mutex<bool>,
    last_input_time: Mutex<u64>,
    input_video_handles: Mutex<Option<InputVideoHandles>>,
    virtual_channels: Mutex<Vec<VirtualChannelHandle>>,
    virtual_channel_forwarders: Mutex<Vec<VirtualChannelForwarderWorker>>,
}

impl ComProtocolConnection {
    fn new(
        inner: Arc<ProtocolConnection>,
        connection_callback: Arc<Mutex<Option<IWRdsProtocolConnectionCallback>>>,
        control_bridge: ProviderControlBridge,
    ) -> Self {
        Self {
            inner,
            auth_bridge: CredsspServerBridge::default(),
            connection_callback,
            control_bridge,
            ready_notified: Mutex::new(false),
            last_input_time: Mutex::new(0),
            input_video_handles: Mutex::new(None),
            virtual_channels: Mutex::new(Vec::new()),
            virtual_channel_forwarders: Mutex::new(Vec::new()),
        }
    }

    fn notify_ready(&self) -> windows_core::Result<()> {
        let mut ready_notified = self.ready_notified.lock();
        if *ready_notified {
            return Ok(());
        }

        self.control_bridge.accept_connection(self.inner.connection_id())?;

        let callback = self
            .connection_callback
            .lock()
            .as_ref()
            .cloned()
            .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "connection callback is not initialized"))?;

        // SAFETY: callback was obtained from termservice for this connection.
        unsafe { callback.OnReady() }?;

        *ready_notified = true;
        Ok(())
    }

    fn release_connection_callback(&self) {
        *self.connection_callback.lock() = None;
    }

    fn ensure_input_video_handles(&self) -> windows_core::Result<()> {
        let mut handles_guard = self.input_video_handles.lock();
        if handles_guard.is_none() {
            *handles_guard = Some(InputVideoHandles::open()?);
        }

        Ok(())
    }

    fn get_keyboard_mouse_handles(&self) -> windows_core::Result<(HANDLE_PTR, HANDLE_PTR)> {
        self.ensure_input_video_handles()?;

        let handles_guard = self.input_video_handles.lock();
        let handles = handles_guard
            .as_ref()
            .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "input handles are not initialized"))?;

        Ok((handles.keyboard.as_handle_ptr(), handles.mouse.as_handle_ptr()))
    }

    fn get_video_handle(&self) -> windows_core::Result<HANDLE_PTR> {
        self.ensure_input_video_handles()?;

        let handles_guard = self.input_video_handles.lock();
        let handles = handles_guard
            .as_ref()
            .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "video handle is not initialized"))?;

        Ok(handles.video.as_handle_ptr())
    }

    fn release_input_video_handles(&self) {
        let mut handles_guard = self.input_video_handles.lock();
        *handles_guard = None;
    }

    fn release_virtual_channels(&self) {
        let mut channels = self.virtual_channels.lock();
        channels.clear();
    }

    fn release_virtual_channel_forwarders(&self) {
        let mut workers = self.virtual_channel_forwarders.lock();

        for worker in workers.drain(..) {
            let endpoint_name = worker.endpoint.endpoint_name.clone();

            if let Err(error) = worker.stop_tx.send(()) {
                warn!(
                    endpoint = %endpoint_name,
                    %error,
                    "Failed to signal virtual channel forwarder stop"
                );
            }

            if let Err(error) = worker.join_handle.join() {
                warn!(
                    endpoint = %endpoint_name,
                    ?error,
                    "Virtual channel forwarder thread panicked"
                );
            }
        }
    }

    fn find_virtual_channel(&self, endpoint_name: &str, is_static: bool) -> Option<(HANDLE, VirtualChannelBridgePlan)> {
        self.virtual_channels
            .lock()
            .iter()
            .find(|channel| channel.matches(endpoint_name, is_static))
            .map(|channel| (channel.raw(), channel.bridge_plan))
    }

    fn register_virtual_channel(
        &self,
        handle: HANDLE,
        endpoint_name: Option<String>,
        is_static: bool,
        bridge_plan: VirtualChannelBridgePlan,
    ) -> windows_core::Result<HANDLE> {
        let endpoint_name_for_worker = endpoint_name.clone();
        let mut channels = self.virtual_channels.lock();
        channels.push(VirtualChannelHandle::new(handle, endpoint_name, is_static, bridge_plan));

        let channel_handle = channels
            .last()
            .map(VirtualChannelHandle::raw)
            .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "virtual channel storage failure"))?;

        drop(channels);

        if let Some(endpoint_name) = endpoint_name_for_worker {
            self.maybe_start_virtual_channel_forwarder(channel_handle, endpoint_name, is_static, bridge_plan);
        }

        Ok(channel_handle)
    }

    fn maybe_start_virtual_channel_forwarder(
        &self,
        channel_handle: HANDLE,
        endpoint_name: String,
        is_static: bool,
        bridge_plan: VirtualChannelBridgePlan,
    ) {
        if !bridge_plan.should_prepare_forwarding() {
            return;
        }

        let Some(handler) = get_virtual_channel_bridge_handler() else {
            return;
        };

        let endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name,
            static_channel: is_static,
            route_kind: bridge_plan.route_kind,
        };

        let (stop_tx, stop_rx) = mpsc::channel();
        let (outbound_tx, outbound_rx) = mpsc::sync_channel(VIRTUAL_CHANNEL_FORWARDER_OUTBOUND_QUEUE_SIZE);

        let tx = VirtualChannelBridgeTx {
            endpoint: endpoint.clone(),
            outbound_tx,
        };

        handler.on_channel_opened(&endpoint, tx);

        let endpoint_for_worker = endpoint.clone();
        let handler_for_worker = Arc::clone(&handler);
        let channel_handle_raw = handle_to_raw_usize(channel_handle);
        let join_handle = thread::spawn(move || {
            run_virtual_channel_forwarder(
                channel_handle_raw,
                endpoint_for_worker,
                handler_for_worker,
                outbound_rx,
                stop_rx,
            )
        });

        self.virtual_channel_forwarders
            .lock()
            .push(VirtualChannelForwarderWorker {
                endpoint,
                stop_tx,
                join_handle,
            });

        info!("Started virtual channel forwarder");
    }

    fn open_virtual_channel_by_name(
        &self,
        session_id: u32,
        endpoint_name: &str,
        is_static: bool,
        requested_priority: u32,
        bridge_plan: VirtualChannelBridgePlan,
    ) -> windows_core::Result<HANDLE> {
        if let Some((existing, _existing_bridge_plan)) = self.find_virtual_channel(endpoint_name, is_static) {
            return Ok(existing);
        }

        let endpoint_name_cstring = CString::new(endpoint_name)
            .map_err(|_| windows_core::Error::new(E_UNEXPECTED, "virtual channel endpoint contains NUL byte"))?;
        let endpoint = PCSTR::from_raw(endpoint_name_cstring.as_ptr().cast::<u8>());
        let flags = virtual_channel_open_flags(is_static, requested_priority);

        // SAFETY: `endpoint` points to a valid NUL-terminated string for the duration of the call.
        let channel = unsafe { WTSVirtualChannelOpenEx(session_id, endpoint, flags) }?;

        if let Some((existing, _existing_bridge_plan)) = self.find_virtual_channel(endpoint_name, is_static) {
            // SAFETY: `channel` is a handle returned by `WTSVirtualChannelOpenEx`.
            if let Err(error) = unsafe { WTSVirtualChannelClose(channel) } {
                warn!(%error, "Failed to close duplicate virtual channel handle");
            }

            return Ok(existing);
        }

        self.register_virtual_channel(channel, Some(endpoint_name.to_owned()), is_static, bridge_plan)
    }

    fn ensure_ironrdp_drdynvc_channel(&self, session_id: u32) -> windows_core::Result<HANDLE> {
        self.open_virtual_channel_by_name(
            session_id,
            IRONRDP_DRDYNVC_CHANNEL_NAME,
            true,
            0,
            VirtualChannelBridgePlan::for_endpoint(true, Some(IronRdpVirtualChannelServer::Drdynvc)),
        )
    }
}

impl IWRdsProtocolConnection_Impl for ComProtocolConnection_Impl {
    fn GetLogonErrorRedirector(&self) -> windows_core::Result<IWRdsProtocolLogonErrorRedirector> {
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "logon error redirector is not implemented",
        ))
    }

    fn AcceptConnection(&self) -> windows_core::Result<()> {
        self.auth_bridge.validate_security_protocol(
            CredsspPolicy::default(),
            nego::SecurityProtocol::HYBRID | nego::SecurityProtocol::HYBRID_EX,
        )?;

        self.inner.accept_connection().map_err(transition_error)?;
        self.notify_ready()?;
        Ok(())
    }

    fn GetClientData(&self, pclientdata: *mut WTS_CLIENT_DATA) -> windows_core::Result<()> {
        if pclientdata.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null client data pointer"));
        }

        // SAFETY: `pclientdata` is non-null (checked above) and points to a writable buffer
        // provided by the caller.
        let client_data = unsafe { &mut *pclientdata };
        *client_data = WTS_CLIENT_DATA::default();
        client_data.fEnableWindowsKey = true;
        client_data.fInheritAutoLogon = BOOL(1);
        client_data.fNoAudioPlayback = true;
        copy_wide(&mut client_data.ProtocolName, "IRDP-WTS");

        Ok(())
    }

    fn GetClientMonitorData(&self, pnummonitors: *mut u32, pprimarymonitor: *mut u32) -> windows_core::Result<()> {
        if !pnummonitors.is_null() {
            // SAFETY: `pnummonitors` is non-null and points to a writable buffer provided by the caller.
            unsafe { *pnummonitors = 1 };
        }

        if !pprimarymonitor.is_null() {
            // SAFETY: `pprimarymonitor` is non-null and points to a writable buffer provided by the caller.
            unsafe { *pprimarymonitor = 0 };
        }

        Ok(())
    }

    fn GetUserCredentials(&self, _pusercreds: *mut WTS_USER_CREDENTIAL) -> windows_core::Result<()> {
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "plaintext credential fallback is disabled",
        ))
    }

    fn GetLicenseConnection(&self) -> windows_core::Result<IWRdsProtocolLicenseConnection> {
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "license connection is not implemented",
        ))
    }

    fn AuthenticateClientToSession(&self, _sessionid: *mut WTS_SESSION_ID) -> windows_core::Result<()> {
        Ok(())
    }

    fn NotifySessionId(
        &self,
        sessionid: *const WTS_SESSION_ID,
        _sessionhandle: HANDLE_PTR,
    ) -> windows_core::Result<()> {
        if sessionid.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null session id pointer"));
        }

        // SAFETY: `sessionid` is non-null (checked above) and points to a valid session id structure.
        let wts_session_id = unsafe { (*sessionid).SessionId };
        self.inner.notify_session_id(wts_session_id).map_err(transition_error)?;

        Ok(())
    }

    fn GetInputHandles(
        &self,
        pkeyboardhandle: *mut HANDLE_PTR,
        pmousehandle: *mut HANDLE_PTR,
        pbeephandle: *mut HANDLE_PTR,
    ) -> windows_core::Result<()> {
        if pkeyboardhandle.is_null() || pmousehandle.is_null() || pbeephandle.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null input handle output pointer"));
        }

        let (keyboard_handle, mouse_handle) = self.get_keyboard_mouse_handles()?;

        // SAFETY: out-pointers were validated as non-null above.
        *unsafe { &mut *pkeyboardhandle } = keyboard_handle;
        // SAFETY: out-pointers were validated as non-null above.
        *unsafe { &mut *pmousehandle } = mouse_handle;
        // SAFETY: out-pointers were validated as non-null above.
        *unsafe { &mut *pbeephandle } = HANDLE_PTR::default();

        debug!("Returned protocol keyboard and mouse handles");

        Ok(())
    }

    fn GetVideoHandle(&self) -> windows_core::Result<HANDLE_PTR> {
        self.get_video_handle()
    }

    fn ConnectNotify(&self, _sessionid: u32) -> windows_core::Result<()> {
        self.inner.connect_notify().map_err(transition_error)?;
        Ok(())
    }

    fn IsUserAllowedToLogon(
        &self,
        _sessionid: u32,
        _usertoken: HANDLE_PTR,
        _pdomainname: &PCWSTR,
        _pusername: &PCWSTR,
    ) -> windows_core::Result<()> {
        Ok(())
    }

    fn SessionArbitrationEnumeration(
        &self,
        _husertoken: HANDLE_PTR,
        _bsinglesessionperuserenabled: BOOL,
        _psessionidarray: *mut u32,
        _pdwsessionidentifiercount: *mut u32,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "session arbitration uses default behavior",
        ))
    }

    fn LogonNotify(
        &self,
        _hclienttoken: HANDLE_PTR,
        _wszusername: &PCWSTR,
        _wszdomainname: &PCWSTR,
        _sessionid: *const WTS_SESSION_ID,
        _pwrdsconnectionsettings: *mut WRDS_CONNECTION_SETTINGS,
    ) -> windows_core::Result<()> {
        self.inner.logon_notify().map_err(transition_error)?;
        Ok(())
    }

    fn PreDisconnect(&self, _disconnectreason: u32) -> windows_core::Result<()> {
        Ok(())
    }

    fn DisconnectNotify(&self) -> windows_core::Result<()> {
        self.inner.disconnect_notify().map_err(transition_error)?;

        if let Err(error) = self.control_bridge.close_connection(self.inner.connection_id()) {
            warn!(%error, connection_id = self.inner.connection_id(), "Failed to notify companion service on disconnect");
        }

        *self.ready_notified.lock() = false;
        self.release_virtual_channel_forwarders();
        self.release_input_video_handles();
        self.release_virtual_channels();
        self.release_connection_callback();
        Ok(())
    }

    fn Close(&self) -> windows_core::Result<()> {
        if let Err(error) = self.control_bridge.close_connection(self.inner.connection_id()) {
            warn!(%error, connection_id = self.inner.connection_id(), "Failed to notify companion service on close");
        }

        *self.ready_notified.lock() = false;
        self.release_virtual_channel_forwarders();
        self.release_virtual_channels();
        self.release_input_video_handles();
        self.release_connection_callback();
        self.inner.close().map_err(transition_error)?;
        Ok(())
    }

    fn GetProtocolStatus(&self, pprotocolstatus: *mut WTS_PROTOCOL_STATUS) -> windows_core::Result<()> {
        if pprotocolstatus.is_null() {
            return Err(windows_core::Error::new(E_POINTER, "null protocol status pointer"));
        }

        // SAFETY: `pprotocolstatus` is non-null (checked above) and points to a writable buffer.
        *unsafe { &mut *pprotocolstatus } = WTS_PROTOCOL_STATUS::default();

        Ok(())
    }

    fn GetLastInputTime(&self) -> windows_core::Result<u64> {
        Ok(*self.last_input_time.lock())
    }

    fn SetErrorInfo(&self, ulerror: u32) -> windows_core::Result<()> {
        warn!(ulerror, "Received protocol error info");
        Ok(())
    }

    fn CreateVirtualChannel(
        &self,
        szendpointname: &PCSTR,
        bstatic: BOOL,
        requestedpriority: u32,
    ) -> windows_core::Result<usize> {
        let session_id = self
            .inner
            .session_id()
            .ok_or_else(|| windows_core::Error::new(E_UNEXPECTED, "session id is not available for virtual channel"))?;

        let endpoint = *szendpointname;
        if endpoint.is_null() {
            return Err(windows_core::Error::new(
                E_POINTER,
                "null virtual channel endpoint pointer",
            ));
        }

        let is_static = bstatic.as_bool();
        // SAFETY: `endpoint` is non-null (checked above) and points to a NUL-terminated
        // string provided by the caller.
        let endpoint_name = match unsafe { endpoint.to_string() } {
            Ok(name) => Some(name),
            Err(error) => {
                warn!(%error, "Failed to decode virtual channel endpoint name");
                None
            }
        };

        let hook_target = endpoint_name
            .as_deref()
            .and_then(|name| ironrdp_virtual_channel_server(name, is_static));
        let bridge_plan = VirtualChannelBridgePlan::for_endpoint(is_static, hook_target);

        let effective_priority = virtual_channel_requested_priority(is_static, requestedpriority, hook_target);

        if let Some(target) = hook_target {
            if target.requires_drdynvc_backbone() {
                if let Err(error) = self.ensure_ironrdp_drdynvc_channel(session_id) {
                    warn!(
                        %error,
                        endpoint = target.name(),
                        "Failed to pre-open DRDYNVC backbone for IronRDP dynamic channel"
                    );
                }
            }
        }

        if let Some(name) = endpoint_name.as_deref() {
            if let Some((existing_channel, existing_bridge_plan)) = self.find_virtual_channel(name, is_static) {
                debug!(
                    session_id,
                    endpoint = name,
                    static_channel = is_static,
                    route_kind = ?existing_bridge_plan.route_kind,
                    "Reusing virtual channel handle"
                );
                return Ok(handle_to_raw_usize(existing_channel));
            }
        }

        let flags = virtual_channel_open_flags(is_static, effective_priority);

        let channel = if let Some(name) = endpoint_name.as_deref() {
            self.open_virtual_channel_by_name(session_id, name, is_static, effective_priority, bridge_plan)?
        } else {
            // SAFETY: `endpoint` points to a valid NUL-terminated string for the duration of the call.
            let channel = unsafe { WTSVirtualChannelOpenEx(session_id, endpoint, flags) }?;
            self.register_virtual_channel(channel, None, is_static, bridge_plan)?
        };

        if bridge_plan.should_prepare_forwarding() {
            info!(
                session_id,
                static_channel = is_static,
                route_kind = ?bridge_plan.route_kind,
                preferred_dynamic_priority = bridge_plan.preferred_dynamic_priority,
                "Prepared virtual channel forwarding metadata"
            );
        }

        if let Some(target) = hook_target {
            info!(
                session_id,
                endpoint = target.name(),
                static_channel = is_static,
                requestedpriority,
                effective_priority,
                flags,
                "Hooked IronRDP virtual channel server endpoint"
            );
        } else {
            debug!(
                session_id,
                requestedpriority,
                effective_priority,
                static_channel = is_static,
                flags,
                "Created virtual channel"
            );
        }

        Ok(handle_to_raw_usize(channel))
    }

    fn QueryProperty(
        &self,
        _querytype: &GUID,
        _ulnumentriesin: u32,
        _ulnumentriesout: u32,
        _ppropertyentriesin: *const WTS_PROPERTY_VALUE,
        _ppropertyentriesout: *mut WTS_PROPERTY_VALUE,
    ) -> windows_core::Result<()> {
        Err(windows_core::Error::new(E_NOTIMPL, "query property is not implemented"))
    }

    fn GetShadowConnection(&self) -> windows_core::Result<IWRdsProtocolShadowConnection> {
        Err(windows_core::Error::new(
            E_NOTIMPL,
            "shadow connection is not implemented",
        ))
    }

    fn NotifyCommandProcessCreated(&self, _sessionid: u32) -> windows_core::Result<()> {
        Ok(())
    }
}

fn copy_wide<const N: usize>(target: &mut [u16; N], value: &str) {
    let mut utf16 = value.encode_utf16().take(N.saturating_sub(1));

    for (index, code_unit) in utf16.by_ref().enumerate() {
        target[index] = code_unit;
    }
}

fn transition_error(message: &'static str) -> windows_core::Error {
    windows_core::Error::new(E_UNEXPECTED, message)
}

fn io_error_to_windows_error(error: std::io::Error, context: &'static str) -> windows_core::Error {
    windows_core::Error::new(E_UNEXPECTED, format!("{context}: {error}"))
}

#[expect(clippy::as_conversions)]
fn stream_ptr_to_token(stream_ptr: *mut core::ffi::c_void) -> usize {
    stream_ptr as usize
}

#[expect(clippy::as_conversions)]
fn token_to_stream_ptr(token: usize) -> *mut core::ffi::c_void {
    token as *mut core::ffi::c_void
}

#[expect(clippy::as_conversions)]
fn handle_to_raw_usize(handle: HANDLE) -> usize {
    handle.0 as usize
}

#[expect(clippy::as_conversions)]
fn raw_usize_to_handle(raw: usize) -> HANDLE {
    HANDLE(raw as *mut core::ffi::c_void)
}

#[derive(Debug)]
struct InputVideoHandles {
    keyboard: OpenDeviceHandle,
    mouse: OpenDeviceHandle,
    video: OpenDeviceHandle,
}

impl InputVideoHandles {
    fn open() -> windows_core::Result<Self> {
        let keyboard = OpenDeviceHandle::open_first_available(
            "keyboard",
            "IRONRDP_WTS_KEYBOARD_DEVICE",
            &[r"\\.\KeyboardClass0", r"\\.\KeyboardClass1"],
        )?;

        let mouse = OpenDeviceHandle::open_first_available(
            "mouse",
            "IRONRDP_WTS_MOUSE_DEVICE",
            &[r"\\.\PointerClass0", r"\\.\PointerClass1"],
        )?;

        let video = OpenDeviceHandle::open_first_available(
            "video",
            "IRONRDP_WTS_VIDEO_DEVICE",
            &[r"\\.\RdpVideoMiniport", r"\\.\DISPLAY"],
        )?;

        info!(
            keyboard_device = %keyboard.path,
            mouse_device = %mouse.path,
            video_device = %video.path,
            "Opened protocol input and video device handles"
        );

        Ok(Self { keyboard, mouse, video })
    }
}

#[derive(Debug)]
struct OpenDeviceHandle {
    path: String,
    file: std::fs::File,
}

#[derive(Debug)]
struct VirtualChannelHandle {
    handle: HANDLE,
    endpoint_name: Option<String>,
    static_channel: bool,
    bridge_plan: VirtualChannelBridgePlan,
}

struct VirtualChannelForwarderWorker {
    endpoint: VirtualChannelBridgeEndpoint,
    stop_tx: mpsc::Sender<()>,
    join_handle: thread::JoinHandle<()>,
}

impl VirtualChannelHandle {
    fn new(
        handle: HANDLE,
        endpoint_name: Option<String>,
        static_channel: bool,
        bridge_plan: VirtualChannelBridgePlan,
    ) -> Self {
        Self {
            handle,
            endpoint_name,
            static_channel,
            bridge_plan,
        }
    }

    fn raw(&self) -> HANDLE {
        self.handle
    }

    fn matches(&self, endpoint_name: &str, is_static: bool) -> bool {
        self.static_channel == is_static
            && self
                .endpoint_name
                .as_deref()
                .is_some_and(|name| endpoint_name_eq(name, endpoint_name))
    }
}

impl Drop for VirtualChannelHandle {
    fn drop(&mut self) {
        // SAFETY: `self.handle` is a handle returned by `WTSVirtualChannelOpenEx`.
        if let Err(error) = unsafe { WTSVirtualChannelClose(self.handle) } {
            warn!(%error, "Failed to close virtual channel handle");
        }
    }
}

fn virtual_channel_open_flags(is_static: bool, requested_priority: u32) -> u32 {
    if is_static {
        return 0;
    }

    let dynamic_priority = match requested_priority {
        WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        | WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
        | WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH
        | WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL => requested_priority,
        _ => WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW,
    };

    WTS_CHANNEL_OPTION_DYNAMIC | dynamic_priority
}

fn run_virtual_channel_forwarder(
    channel_handle_raw: usize,
    endpoint: VirtualChannelBridgeEndpoint,
    bridge_handler: SharedVirtualChannelBridgeHandler,
    outbound_rx: mpsc::Receiver<Vec<u8>>,
    stop_rx: mpsc::Receiver<()>,
) {
    let channel_handle = raw_usize_to_handle(channel_handle_raw);
    let mut read_buffer = vec![0u8; VIRTUAL_CHANNEL_FORWARDER_BUFFER_SIZE];

    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }

        while let Ok(payload) = outbound_rx.try_recv() {
            let mut bytes_written = 0;
            // SAFETY: `channel_handle` is a live virtual channel handle, and `payload` points to
            // a valid buffer for the duration of the call.
            if let Err(error) = unsafe { WTSVirtualChannelWrite(channel_handle, &payload, &mut bytes_written) } {
                warn!(
                    endpoint = %endpoint.endpoint_name,
                    ?error,
                    "Failed to write virtual channel payload"
                );
                break;
            }
        }

        let mut bytes_read = 0;
        // SAFETY: `channel_handle` is a live virtual channel handle. `read_buffer` and `bytes_read`
        // are valid out-buffers for the duration of the call.
        match unsafe {
            WTSVirtualChannelRead(
                channel_handle,
                VIRTUAL_CHANNEL_FORWARDER_READ_TIMEOUT_MS,
                &mut read_buffer,
                &mut bytes_read,
            )
        } {
            Ok(()) => {
                if bytes_read == 0 {
                    continue;
                }

                let Ok(read_len) = usize::try_from(bytes_read) else {
                    warn!(
                        endpoint = %endpoint.endpoint_name,
                        bytes_read,
                        "Virtual channel forwarder read length does not fit in usize"
                    );
                    break;
                };
                bridge_handler.on_channel_data(&endpoint, &read_buffer[..read_len]);
            }
            Err(error) => {
                if is_virtual_channel_read_timeout(&error) {
                    continue;
                }

                warn!(
                    endpoint = %endpoint.endpoint_name,
                    ?error,
                    "Virtual channel forwarder read failed"
                );
                break;
            }
        }
    }

    bridge_handler.on_channel_closed(&endpoint);
}

fn is_virtual_channel_read_timeout(error: &windows_core::Error) -> bool {
    let code = error.code();

    code == HRESULT::from_win32(ERROR_SEM_TIMEOUT.0)
        || code == HRESULT::from_win32(ERROR_IO_INCOMPLETE.0)
        || code == HRESULT::from_win32(ERROR_NO_DATA.0)
}

impl OpenDeviceHandle {
    fn open_first_available(
        device_kind: &str,
        env_var_name: &str,
        fallback_paths: &[&str],
    ) -> windows_core::Result<Self> {
        let configured_path = std::env::var(env_var_name).ok();

        let mut candidate_paths = Vec::with_capacity(fallback_paths.len() + usize::from(configured_path.is_some()));
        if let Some(path) = configured_path {
            if !path.trim().is_empty() {
                candidate_paths.push(path);
            }
        }

        candidate_paths.extend(fallback_paths.iter().map(|path| (*path).to_owned()));

        let mut failures = Vec::new();

        for path in candidate_paths {
            let open_result = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .or_else(|_| OpenOptions::new().read(true).open(&path));

            match open_result {
                Ok(file) => {
                    return Ok(Self { path, file });
                }
                Err(error) => {
                    failures.push(format!("{path}: {error}"));
                }
            }
        }

        let error_message = format!(
            "failed to open {device_kind} device handle; set {env_var_name} to an accessible device path; attempts: {}",
            failures.join(" | ")
        );

        Err(windows_core::Error::new(E_NOTIMPL, error_message))
    }

    fn as_handle_ptr(&self) -> HANDLE_PTR {
        raw_handle_to_handle_ptr(self.file.as_raw_handle())
    }
}

#[expect(clippy::as_conversions)]
fn raw_handle_to_handle_ptr(raw_handle: RawHandle) -> HANDLE_PTR {
    HANDLE_PTR(raw_handle as usize)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{
        bridge_pipe_path, drain_length_prefixed_pipe_frames, ironrdp_virtual_channel_server, sanitize_pipe_segment,
        virtual_channel_open_flags, virtual_channel_requested_priority, IronRdpVirtualChannelServer,
        VirtualChannelBridgeEndpoint, VirtualChannelBridgePlan, VirtualChannelBridgeTx, VirtualChannelRouteKind,
        VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE,
    };
    use windows::Win32::System::RemoteDesktop::{
        WTS_CHANNEL_OPTION_DYNAMIC, WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH, WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW,
        WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED, WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL,
    };

    #[test]
    fn static_channels_use_zero_flags() {
        assert_eq!(virtual_channel_open_flags(true, 123), 0);
    }

    #[test]
    fn dynamic_channels_map_priority_flags() {
        assert_eq!(
            virtual_channel_open_flags(false, WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
        assert_eq!(
            virtual_channel_open_flags(false, WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
        );
        assert_eq!(
            virtual_channel_open_flags(false, WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH
        );
        assert_eq!(
            virtual_channel_open_flags(false, WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL
        );
    }

    #[test]
    fn dynamic_channels_fallback_to_low_for_unknown_priority() {
        assert_eq!(
            virtual_channel_open_flags(false, 1),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
        assert_eq!(
            virtual_channel_open_flags(false, 123),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
        assert_eq!(
            virtual_channel_open_flags(false, u32::MAX),
            WTS_CHANNEL_OPTION_DYNAMIC | WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
    }

    #[test]
    fn recognizes_ironrdp_static_virtual_channel_servers() {
        assert_eq!(
            ironrdp_virtual_channel_server("cliprdr", true),
            Some(IronRdpVirtualChannelServer::Cliprdr)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("RDPSND", true),
            Some(IronRdpVirtualChannelServer::Rdpsnd)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("DrDyNvC", true),
            Some(IronRdpVirtualChannelServer::Drdynvc)
        );
        assert_eq!(ironrdp_virtual_channel_server("cliprdr", false), None);
    }

    #[test]
    fn recognizes_ironrdp_dynamic_virtual_channel_servers() {
        assert_eq!(
            ironrdp_virtual_channel_server("Microsoft::Windows::RDS::DisplayControl", false),
            Some(IronRdpVirtualChannelServer::DisplayControl)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("microsoft::windows::rds::graphics", false),
            Some(IronRdpVirtualChannelServer::Graphics)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("FreeRDP::Advanced::Input", false),
            Some(IronRdpVirtualChannelServer::AdvancedInput)
        );
        assert_eq!(
            ironrdp_virtual_channel_server("echo", false),
            Some(IronRdpVirtualChannelServer::Echo)
        );
    }

    #[test]
    fn ironrdp_dynamic_channels_use_recommended_priorities_when_unknown() {
        assert_eq!(
            virtual_channel_requested_priority(false, 0, Some(IronRdpVirtualChannelServer::DisplayControl)),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
        );
        assert_eq!(
            virtual_channel_requested_priority(false, 0, Some(IronRdpVirtualChannelServer::Graphics)),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_HIGH
        );
        assert_eq!(
            virtual_channel_requested_priority(false, 0, Some(IronRdpVirtualChannelServer::AdvancedInput)),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_REAL
        );
        assert_eq!(
            virtual_channel_requested_priority(false, 0, Some(IronRdpVirtualChannelServer::Echo)),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_LOW
        );
    }

    #[test]
    fn explicit_dynamic_priority_is_preserved() {
        assert_eq!(
            virtual_channel_requested_priority(
                false,
                WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED,
                Some(IronRdpVirtualChannelServer::AdvancedInput)
            ),
            WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED
        );
    }

    #[test]
    fn dynamic_server_backbone_requirements_are_exposed() {
        assert!(IronRdpVirtualChannelServer::DisplayControl.requires_drdynvc_backbone());
        assert!(IronRdpVirtualChannelServer::Graphics.requires_drdynvc_backbone());
        assert!(IronRdpVirtualChannelServer::AdvancedInput.requires_drdynvc_backbone());
        assert!(IronRdpVirtualChannelServer::Echo.requires_drdynvc_backbone());
        assert!(!IronRdpVirtualChannelServer::Cliprdr.requires_drdynvc_backbone());
        assert!(!IronRdpVirtualChannelServer::Rdpsnd.requires_drdynvc_backbone());
        assert!(!IronRdpVirtualChannelServer::Drdynvc.requires_drdynvc_backbone());
    }

    #[test]
    fn bridge_plan_classifies_known_ironrdp_routes() {
        assert_eq!(
            VirtualChannelBridgePlan::for_endpoint(true, Some(IronRdpVirtualChannelServer::Cliprdr)).route_kind,
            VirtualChannelRouteKind::IronRdpStatic
        );
        assert_eq!(
            VirtualChannelBridgePlan::for_endpoint(true, Some(IronRdpVirtualChannelServer::Drdynvc)).route_kind,
            VirtualChannelRouteKind::IronRdpDynamicBackbone
        );
        assert_eq!(
            VirtualChannelBridgePlan::for_endpoint(false, Some(IronRdpVirtualChannelServer::DisplayControl)).route_kind,
            VirtualChannelRouteKind::IronRdpDynamicEndpoint
        );
        assert_eq!(
            VirtualChannelBridgePlan::for_endpoint(false, None).route_kind,
            VirtualChannelRouteKind::Unknown
        );
    }

    #[test]
    fn bridge_plan_exposes_forwarding_preparation_and_priority() {
        let unknown_plan = VirtualChannelBridgePlan::for_endpoint(false, None);
        assert!(!unknown_plan.should_prepare_forwarding());
        assert_eq!(unknown_plan.preferred_dynamic_priority, None);

        let display_plan =
            VirtualChannelBridgePlan::for_endpoint(false, Some(IronRdpVirtualChannelServer::DisplayControl));
        assert!(display_plan.should_prepare_forwarding());
        assert_eq!(
            display_plan.preferred_dynamic_priority,
            Some(WTS_CHANNEL_OPTION_DYNAMIC_PRI_MED)
        );

        let static_plan = VirtualChannelBridgePlan::for_endpoint(true, Some(IronRdpVirtualChannelServer::Rdpsnd));
        assert!(static_plan.should_prepare_forwarding());
        assert_eq!(static_plan.preferred_dynamic_priority, None);
    }

    #[test]
    fn sanitize_pipe_segment_normalizes_name() {
        assert_eq!(sanitize_pipe_segment("ClipRdr"), "cliprdr");
        assert_eq!(
            sanitize_pipe_segment("Microsoft::Windows::RDS::Graphics"),
            "microsoft__windows__rds__graphics"
        );
        assert_eq!(sanitize_pipe_segment(""), "channel");
    }

    #[test]
    fn bridge_pipe_path_uses_svc_and_dvc_suffixes() {
        let static_endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name: "cliprdr".to_owned(),
            static_channel: true,
            route_kind: VirtualChannelRouteKind::IronRdpStatic,
        };

        let dynamic_endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name: "Microsoft::Windows::RDS::Graphics".to_owned(),
            static_channel: false,
            route_kind: VirtualChannelRouteKind::IronRdpDynamicEndpoint,
        };

        assert_eq!(
            bridge_pipe_path("IronRdpVcBridge", &static_endpoint),
            r"\\.\pipe\IronRdpVcBridge.svc.cliprdr"
        );
        assert_eq!(
            bridge_pipe_path(r"\\.\pipe\Bridge", &dynamic_endpoint),
            r"\\.\pipe\Bridge.dvc.microsoft__windows__rds__graphics"
        );
    }

    #[test]
    fn drain_length_prefixed_frames_forwards_payloads() {
        let endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name: "ECHO".to_owned(),
            static_channel: false,
            route_kind: VirtualChannelRouteKind::IronRdpDynamicEndpoint,
        };
        let (outbound_tx, outbound_rx) = mpsc::sync_channel(4);
        let bridge_tx = VirtualChannelBridgeTx {
            endpoint: endpoint.clone(),
            outbound_tx,
        };

        let mut framed = Vec::new();
        framed.extend_from_slice(&(3u32).to_le_bytes());
        framed.extend_from_slice(b"abc");
        framed.extend_from_slice(&(2u32).to_le_bytes());
        framed.extend_from_slice(b"de");

        drain_length_prefixed_pipe_frames(&endpoint, &bridge_tx, &mut framed).expect("framed payload should parse");

        assert!(framed.is_empty());
        assert_eq!(outbound_rx.try_recv().expect("first payload"), b"abc");
        assert_eq!(outbound_rx.try_recv().expect("second payload"), b"de");
    }

    #[test]
    fn drain_length_prefixed_frames_rejects_oversized_frame() {
        let endpoint = VirtualChannelBridgeEndpoint {
            endpoint_name: "ECHO".to_owned(),
            static_channel: false,
            route_kind: VirtualChannelRouteKind::IronRdpDynamicEndpoint,
        };
        let (outbound_tx, _outbound_rx) = mpsc::sync_channel(1);
        let bridge_tx = VirtualChannelBridgeTx {
            endpoint: endpoint.clone(),
            outbound_tx,
        };

        let mut framed = Vec::new();
        let oversized_len =
            u32::try_from(VIRTUAL_CHANNEL_PIPE_BRIDGE_MAX_FRAME_SIZE).expect("max frame size should fit in u32") + 1;
        framed.extend_from_slice(&oversized_len.to_le_bytes());

        let error = drain_length_prefixed_pipe_frames(&endpoint, &bridge_tx, &mut framed)
            .expect_err("oversized frame should fail");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}
