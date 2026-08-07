//! Durable daemon-side NOW operation retention.
//!
//! The NOW protocol worker remains entirely in `now-client`; this module translates its public
//! request/event API into daemon-owned operation IDs, bounded replay, and IPC subscriptions.

use core::time::Duration;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use now_client::{
    BatchRequest, Execution, ExecutionEvent, ExecutionStatus, NowClientError, ProcessRequest, PwshRequest, RunRequest,
    WinPsRequest,
};
use tokio::sync::{mpsc, oneshot};

use crate::ipc::{
    AgentError, AgentErrorCategory, NowCapabilities, NowExecutionKind, NowExecutionRequest, NowStream, OperationEvent,
    OperationEventKind, OperationInfo, OperationState,
};
use crate::now::{Capabilities, NowEndpoint, NowEndpointError, invalidates_handle};

const MAX_OPERATION_OUTPUT: usize = 8 * 1024 * 1024;
const MAX_TERMINAL_OPERATIONS: usize = 32;
const MAX_TOTAL_OUTPUT: usize = 32 * 1024 * 1024;
const MAX_STDIN_CHUNK: usize = 1024 * 1024;
const MAX_IPC_OUTPUT_CHUNK: usize = 1024 * 1024;
const MAX_OPERATION_EVENTS: usize = 8 * 1024;
const MAX_LIVE_SUBSCRIBERS: usize = 8;
const LIVE_SUBSCRIBER_QUEUE_CAPACITY: usize = 1;

/// Durable NOW operation manager for one RDP session.
#[derive(Clone)]
pub struct OperationManager {
    endpoint: Arc<NowEndpoint>,
    state: Arc<Mutex<OperationStateStore>>,
}

struct OperationStateStore {
    next_id: u64,
    starting: bool,
    active: Option<ActiveOperation>,
    records: BTreeMap<u64, OperationRecord>,
    terminal_order: VecDeque<u64>,
    total_output: usize,
}

struct ActiveOperation {
    id: u64,
    control: mpsc::Sender<Control>,
    cancellation_requested: bool,
}

struct OperationRecord {
    info: OperationInfo,
    events: VecDeque<OperationEvent>,
    subscribers: Vec<mpsc::Sender<OperationEvent>>,
}

enum Control {
    Stdin {
        data: Vec<u8>,
        last: bool,
        response: oneshot::Sender<Result<(), AgentError>>,
    },
    Cancel {
        response: oneshot::Sender<Result<(), AgentError>>,
    },
}

/// Snapshot and subscription returned by [`OperationManager::attach`].
pub struct OperationAttachment {
    /// Current operation metadata.
    pub info: OperationInfo,
    /// Retained events newer than the requested sequence.
    pub replay: Vec<OperationEvent>,
    /// Live events. It closes when the operation becomes terminal, the client disconnects, or the
    /// client cannot keep up with remote output.
    pub live: mpsc::Receiver<OperationEvent>,
}

impl OperationManager {
    /// Creates a manager backed by a per-RDP-session NOW endpoint.
    pub fn new(endpoint: Arc<NowEndpoint>) -> Self {
        Self {
            endpoint,
            state: Arc::new(Mutex::new(OperationStateStore {
                next_id: 1,
                starting: false,
                active: None,
                records: BTreeMap::new(),
                terminal_order: VecDeque::new(),
                total_output: 0,
            })),
        }
    }

    /// Gets NOW capabilities, lazily connecting the DVC endpoint if necessary.
    pub async fn capabilities(&self) -> Result<NowCapabilities, AgentError> {
        let handle = self.endpoint.handle().await.map_err(endpoint_error)?;
        Ok(capabilities(handle.capabilities().into()))
    }

    /// Submits a generic untracked NOW Run request.
    pub async fn run(&self, command: String, directory: Option<String>) -> Result<(), AgentError> {
        self.reserve_submission()?;
        let request = match directory {
            Some(directory) => RunRequest::new(command).with_directory(directory),
            None => RunRequest::new(command),
        };
        let result = match self.endpoint.handle().await {
            Ok(handle) => handle.run(request).await,
            Err(error) => {
                self.release_submission();
                return Err(endpoint_error(error));
            }
        };
        self.release_submission();
        match result {
            Ok(_) => Ok(()),
            Err(error) => {
                self.invalidate_if_needed(&error).await;
                Err(client_error(error))
            }
        }
    }

    /// Submits a tracked or detached NOW execution.
    pub async fn execute(&self, request: NowExecutionRequest) -> Result<OperationInfo, AgentError> {
        if request.detached {
            return self.execute_detached(request).await;
        }

        let kind = request.kind;
        self.reserve_submission()?;
        let result = self.start_tracked(request).await;
        let mut state = lock(&self.state);
        let execution = match result {
            Ok(execution) => execution,
            Err(error) => {
                state.starting = false;
                return Err(error);
            }
        };

        let (control_tx, control_rx) = mpsc::channel(32);
        let id = next_id(&mut state);
        let info = OperationInfo {
            id,
            kind,
            state: OperationState::Running,
            detached: false,
            exit_code: None,
            error: None,
            retained_output_bytes: 0,
            next_sequence: 0,
        };
        state.records.insert(
            id,
            OperationRecord {
                info: info.clone(),
                events: VecDeque::new(),
                subscribers: Vec::new(),
            },
        );
        state.active = Some(ActiveOperation {
            id,
            control: control_tx,
            cancellation_requested: false,
        });
        state.starting = false;
        drop(state);

        let manager = self.clone();
        tokio::spawn(async move {
            manager.drive_execution(id, execution, control_rx).await;
        });

        Ok(info)
    }

    /// Lists all daemon-retained operations in daemon ID order.
    pub fn list(&self) -> Vec<OperationInfo> {
        lock(&self.state)
            .records
            .values()
            .map(|record| record.info.clone())
            .collect()
    }

    /// Retrieves a retained operation.
    pub fn status(&self, operation_id: u64) -> Result<OperationInfo, AgentError> {
        lock(&self.state)
            .records
            .get(&operation_id)
            .map(|record| record.info.clone())
            .ok_or_else(operation_not_found)
    }

    /// Replays bounded retained output and subscribes to subsequent output.
    pub fn attach(&self, operation_id: u64, after_sequence: Option<u64>) -> Result<OperationAttachment, AgentError> {
        let mut state = lock(&self.state);
        let record = state.records.get_mut(&operation_id).ok_or_else(operation_not_found)?;
        record.subscribers.retain(|subscriber| !subscriber.is_closed());
        if record.subscribers.len() == MAX_LIVE_SUBSCRIBERS {
            return Err(AgentError {
                category: AgentErrorCategory::Conflict,
                message: "maximum live NOW subscribers reached".to_owned(),
            });
        }
        let (sender, receiver) = mpsc::channel(LIVE_SUBSCRIBER_QUEUE_CAPACITY);
        let replay = record
            .events
            .iter()
            .filter(|event| after_sequence.is_none_or(|sequence| event.sequence > sequence))
            .cloned()
            .collect();
        if matches!(record.info.state, OperationState::Running | OperationState::Cancelling) {
            record.subscribers.push(sender);
        }
        Ok(OperationAttachment {
            info: record.info.clone(),
            replay,
            live: receiver,
        })
    }

    /// Forwards raw stdin to the active tracked execution.
    pub async fn send_stdin(&self, operation_id: u64, data: Vec<u8>, last: bool) -> Result<(), AgentError> {
        if data.len() > MAX_STDIN_CHUNK {
            return Err(AgentError {
                category: AgentErrorCategory::InvalidRequest,
                message: format!("stdin chunk exceeds the {MAX_STDIN_CHUNK}-byte limit"),
            });
        }
        let control = self.active_control(operation_id)?;
        let (response_tx, response_rx) = oneshot::channel();
        control
            .send(Control::Stdin {
                data,
                last,
                response: response_tx,
            })
            .await
            .map_err(|_| operation_finished())?;
        response_rx.await.map_err(|_| operation_finished())?
    }

    /// Submits a cancellation request while the operation driver continues draining remote output.
    pub async fn cancel(&self, operation_id: u64) -> Result<(), AgentError> {
        let control = {
            let mut state = lock(&self.state);
            let known_operation = state.records.contains_key(&operation_id);
            let Some(active) = state.active.as_mut() else {
                return if known_operation {
                    Err(operation_finished())
                } else {
                    Err(operation_not_found())
                };
            };
            if active.id != operation_id {
                return if known_operation {
                    Err(operation_finished())
                } else {
                    Err(operation_not_found())
                };
            }
            if active.cancellation_requested {
                return Err(AgentError {
                    category: AgentErrorCategory::Conflict,
                    message: "NOW operation cancellation is already pending".to_owned(),
                });
            }
            active.cancellation_requested = true;
            active.control.clone()
        };
        let (response_tx, response_rx) = oneshot::channel();
        if control.send(Control::Cancel { response: response_tx }).await.is_err() {
            self.clear_cancellation_requested(operation_id);
            return Err(operation_finished());
        }
        response_rx.await.map_err(|_| operation_finished())?
    }

    fn active_control(&self, operation_id: u64) -> Result<mpsc::Sender<Control>, AgentError> {
        let state = lock(&self.state);
        match &state.active {
            Some(active) if active.id == operation_id => Ok(active.control.clone()),
            _ if state.records.contains_key(&operation_id) => Err(operation_finished()),
            _ => Err(operation_not_found()),
        }
    }

    async fn execute_detached(&self, request: NowExecutionRequest) -> Result<OperationInfo, AgentError> {
        if request.stdin.is_some() || request.timeout_ms.is_some() {
            return Err(AgentError {
                category: AgentErrorCategory::InvalidRequest,
                message: "detached execution cannot include stdin or a timeout".to_owned(),
            });
        }

        self.reserve_submission()?;
        let handle = match self.endpoint.handle().await {
            Ok(handle) => handle,
            Err(error) => {
                self.release_submission();
                return Err(endpoint_error(error));
            }
        };
        let kind = request.kind;
        let result = match kind {
            NowExecutionKind::Process => handle.process_detached(build_process(request)).await,
            NowExecutionKind::Batch => handle.batch_detached(build_batch(request)).await,
            NowExecutionKind::PowerShell => handle.win_ps_detached(build_win_ps(request)).await,
            NowExecutionKind::Pwsh => handle.pwsh_detached(build_pwsh(request)).await,
        };
        self.release_submission();
        if let Err(error) = result {
            self.invalidate_if_needed(&error).await;
            return Err(client_error(error));
        }

        let mut state = lock(&self.state);
        let info = OperationInfo {
            id: next_id(&mut state),
            kind,
            state: OperationState::Detached,
            detached: true,
            exit_code: None,
            error: None,
            retained_output_bytes: 0,
            next_sequence: 0,
        };
        state.records.insert(
            info.id,
            OperationRecord {
                info: info.clone(),
                events: VecDeque::new(),
                subscribers: Vec::new(),
            },
        );
        state.terminal_order.push_back(info.id);
        trim_retention(&mut state);
        Ok(info)
    }

    async fn start_tracked(&self, request: NowExecutionRequest) -> Result<Execution, AgentError> {
        let handle = self.endpoint.handle().await.map_err(endpoint_error)?;
        let kind = request.kind;
        let result = match kind {
            NowExecutionKind::Process => handle.process(build_process(request)).await,
            NowExecutionKind::Batch => handle.batch(build_batch(request)).await,
            NowExecutionKind::PowerShell => handle.win_ps(build_win_ps(request)).await,
            NowExecutionKind::Pwsh => handle.pwsh(build_pwsh(request)).await,
        };
        let execution = match result {
            Ok(execution) => execution,
            Err(error) => {
                self.invalidate_if_needed(&error).await;
                return Err(client_error(error));
            }
        };
        Ok(execution)
    }

    async fn drive_execution(
        &self,
        operation_id: u64,
        mut execution: Execution,
        mut controls: mpsc::Receiver<Control>,
    ) {
        loop {
            match tokio::time::timeout(Duration::from_millis(25), execution.next_event()).await {
                Ok(Some(event)) => self.record_client_event(operation_id, event),
                Ok(None) => break,
                Err(_) => {}
            }

            while let Ok(control) = controls.try_recv() {
                match control {
                    Control::Stdin { data, last, response } => {
                        let result = match execution.send_stdin(data, last).await {
                            Ok(()) => Ok(()),
                            Err(error) => {
                                self.invalidate_if_needed(&error).await;
                                Err(client_error(error))
                            }
                        };
                        let _ = response.send(result);
                    }
                    Control::Cancel { response } => {
                        let result = match submit_cancel(&execution).await {
                            Ok(()) => Ok(()),
                            Err(error) => {
                                self.invalidate_if_needed(&error).await;
                                self.clear_cancellation_requested(operation_id);
                                Err(client_error(error))
                            }
                        };
                        let _ = response.send(result);
                    }
                }
            }
        }

        match execution.wait().await {
            Ok(ExecutionStatus::Completed { exit_code }) => {
                self.emit(operation_id, OperationEventKind::Completed { exit_code });
                self.finish(operation_id, OperationState::Completed, Some(exit_code), None);
            }
            Ok(ExecutionStatus::Cancelled) => {
                self.emit(operation_id, OperationEventKind::Cancelled);
                self.finish(
                    operation_id,
                    OperationState::Cancelled,
                    None,
                    Some(AgentError {
                        category: AgentErrorCategory::Remote,
                        message: "now operation was cancelled".to_owned(),
                    }),
                );
            }
            Err(error) => {
                self.invalidate_if_needed(&error).await;
                let error = client_error(error);
                self.emit(operation_id, OperationEventKind::Failed(error.clone()));
                self.finish(operation_id, OperationState::Failed, None, Some(error));
            }
        }
    }

    fn record_client_event(&self, operation_id: u64, event: ExecutionEvent) {
        match event {
            ExecutionEvent::Started => self.emit(operation_id, OperationEventKind::Started),
            ExecutionEvent::Stdout { data, last } => self.emit_output(operation_id, NowStream::Stdout, data, last),
            ExecutionEvent::Stderr { data, last } => self.emit_output(operation_id, NowStream::Stderr, data, last),
            ExecutionEvent::CancelAccepted => {
                self.update_state(operation_id, OperationState::Cancelling);
                self.emit(operation_id, OperationEventKind::CancelAccepted);
            }
        }
    }

    fn emit_output(&self, operation_id: u64, stream: NowStream, data: Vec<u8>, last: bool) {
        if data.is_empty() {
            if last {
                self.emit(operation_id, OperationEventKind::Output { stream, data, last });
            }
            return;
        }

        let chunks = data.chunks(MAX_IPC_OUTPUT_CHUNK);
        let chunk_count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            self.emit(
                operation_id,
                OperationEventKind::Output {
                    stream,
                    data: chunk.to_vec(),
                    last: last && index + 1 == chunk_count,
                },
            );
        }
    }

    fn update_state(&self, operation_id: u64, state: OperationState) {
        if let Some(record) = lock(&self.state).records.get_mut(&operation_id) {
            record.info.state = state;
        }
    }

    fn emit(&self, operation_id: u64, kind: OperationEventKind) {
        let mut state = lock(&self.state);
        let (event, output_len) = {
            let Some(record) = state.records.get_mut(&operation_id) else {
                return;
            };
            let event = OperationEvent {
                operation_id,
                sequence: record.info.next_sequence,
                kind,
            };
            record.info.next_sequence = record.info.next_sequence.saturating_add(1);
            let output_len = output_size(&event.kind);
            record.info.retained_output_bytes = record
                .info
                .retained_output_bytes
                .saturating_add(u64::try_from(output_len).unwrap_or(u64::MAX));
            record.events.push_back(event.clone());
            record
                .subscribers
                .retain(|sender| sender.try_send(event.clone()).is_ok());
            (event, output_len)
        };
        state.total_output = state.total_output.saturating_add(output_len);
        trim_operation_events(&mut state, operation_id);
        trim_retention(&mut state);
        let _ = event;
    }

    fn finish(
        &self,
        operation_id: u64,
        operation_state: OperationState,
        exit_code: Option<u32>,
        error: Option<AgentError>,
    ) {
        let mut state = lock(&self.state);
        if let Some(record) = state.records.get_mut(&operation_id) {
            record.info.state = operation_state;
            record.info.exit_code = exit_code;
            record.info.error = error;
            record.subscribers.clear();
        }
        if state.active.as_ref().is_some_and(|active| active.id == operation_id) {
            state.active = None;
        }
        state.terminal_order.push_back(operation_id);
        trim_retention(&mut state);
    }

    async fn invalidate_if_needed(&self, error: &NowClientError) {
        if invalidates_handle(error) {
            self.endpoint.invalidate().await;
        }
    }

    fn clear_cancellation_requested(&self, operation_id: u64) {
        let mut state = lock(&self.state);
        if let Some(active) = state.active.as_mut().filter(|active| active.id == operation_id) {
            active.cancellation_requested = false;
        }
    }

    fn reserve_submission(&self) -> Result<(), AgentError> {
        let mut state = lock(&self.state);
        if state.starting || state.active.is_some() {
            return Err(AgentError {
                category: AgentErrorCategory::Conflict,
                message: "a NOW operation is already active".to_owned(),
            });
        }
        state.starting = true;
        Ok(())
    }

    fn release_submission(&self) {
        lock(&self.state).starting = false;
    }
}

/// Submits cancellation without waiting for the peer response so the execution event receiver keeps
/// draining. The agent permits only one submission or tracked execution at a time, and serializes
/// stdin requests, so its empty command queue accepts this first poll; the peer acknowledgement
/// remains observable as `ExecutionEvent::CancelAccepted`.
async fn submit_cancel(execution: &Execution) -> Result<(), NowClientError> {
    let cancel = execution.cancel();
    tokio::pin!(cancel);
    tokio::select! {
        biased;
        result = &mut cancel => result,
        _ = tokio::task::yield_now() => Ok(()),
    }
}

fn build_process(request: NowExecutionRequest) -> ProcessRequest {
    let mut output = ProcessRequest::new(request.command);
    if let Some(parameters) = request.parameters {
        output = output.with_parameters(parameters);
    }
    if let Some(directory) = request.directory {
        output = output.with_directory(directory);
    }
    if let Some(stdin) = request.stdin {
        output = output.with_stdin(stdin);
    }
    if let Some(timeout_ms) = request.timeout_ms {
        output = output.with_timeout(Duration::from_millis(timeout_ms));
    }
    output
}

fn build_batch(request: NowExecutionRequest) -> BatchRequest {
    let mut output = BatchRequest::new(request.command);
    if let Some(directory) = request.directory {
        output = output.with_directory(directory);
    }
    if let Some(stdin) = request.stdin {
        output = output.with_stdin(stdin);
    }
    if let Some(timeout_ms) = request.timeout_ms {
        output = output.with_timeout(Duration::from_millis(timeout_ms));
    }
    output
}

fn build_win_ps(request: NowExecutionRequest) -> WinPsRequest {
    build_power_shell(WinPsRequest::new(request.command.clone()), request)
}

fn build_pwsh(request: NowExecutionRequest) -> PwshRequest {
    build_power_shell(PwshRequest::new(request.command.clone()), request)
}

fn build_power_shell(mut output: WinPsRequest, request: NowExecutionRequest) -> WinPsRequest {
    if let Some(directory) = request.directory {
        output = output.with_directory(directory);
    }
    if let Some(stdin) = request.stdin {
        output = output.with_stdin(stdin);
    }
    if let Some(timeout_ms) = request.timeout_ms {
        output = output.with_timeout(Duration::from_millis(timeout_ms));
    }
    if request.no_profile {
        output = output.with_no_profile();
    }
    if request.non_interactive {
        output = output.with_non_interactive();
    }
    output
}

fn capabilities(value: Capabilities) -> NowCapabilities {
    NowCapabilities {
        version_major: value.version_major,
        version_minor: value.version_minor,
        heartbeat_ms: value.heartbeat_ms,
        run: value.run,
        process: value.process,
        batch: value.batch,
        powershell: value.powershell,
        pwsh: value.pwsh,
        io_redirection: value.io_redirection,
        unicode_console: value.unicode_console,
    }
}

fn endpoint_error(error: NowEndpointError) -> AgentError {
    let category = match error {
        NowEndpointError::Unavailable { .. } => AgentErrorCategory::Unavailable,
        NowEndpointError::Client(_) => AgentErrorCategory::Transport,
    };
    AgentError {
        category,
        message: format!("now endpoint: {error}"),
    }
}

fn client_error(error: NowClientError) -> AgentError {
    let category = match error {
        NowClientError::InvalidConfiguration(_) | NowClientError::InvalidRequest(_) => {
            AgentErrorCategory::InvalidRequest
        }
        NowClientError::UnsupportedCapability(_) => AgentErrorCategory::Unavailable,
        NowClientError::OperationInProgress => AgentErrorCategory::Conflict,
        NowClientError::RemoteStatus { .. } => AgentErrorCategory::Remote,
        NowClientError::Io(_)
        | NowClientError::PduEncode(_)
        | NowClientError::PduDecode(_)
        | NowClientError::Protocol(_)
        | NowClientError::FrameTooLarge { .. }
        | NowClientError::FrameBufferTooLarge { .. }
        | NowClientError::HandshakeTimeout
        | NowClientError::IncompatibleVersion { .. }
        | NowClientError::WorkerClosed(_)
        | NowClientError::EventQueueFull { .. } => AgentErrorCategory::Transport,
        NowClientError::SessionIdExhausted | NowClientError::OperationFinished { .. } => AgentErrorCategory::Internal,
    };
    AgentError {
        category,
        message: format!("now client: {error}"),
    }
}

fn operation_not_found() -> AgentError {
    AgentError {
        category: AgentErrorCategory::InvalidRequest,
        message: "unknown NOW operation".to_owned(),
    }
}

fn operation_finished() -> AgentError {
    AgentError {
        category: AgentErrorCategory::Conflict,
        message: "NOW operation is no longer active".to_owned(),
    }
}

fn next_id(state: &mut OperationStateStore) -> u64 {
    let id = state.next_id;
    state.next_id = state.next_id.checked_add(1).unwrap_or(1);
    id
}

fn output_size(kind: &OperationEventKind) -> usize {
    match kind {
        OperationEventKind::Output { data, .. } => data.len(),
        _ => 0,
    }
}

fn trim_operation_events(state: &mut OperationStateStore, operation_id: u64) {
    let Some(record) = state.records.get_mut(&operation_id) else {
        return;
    };
    while usize::try_from(record.info.retained_output_bytes).unwrap_or(usize::MAX) > MAX_OPERATION_OUTPUT
        || record.events.len() > MAX_OPERATION_EVENTS
    {
        let Some(position) = record
            .events
            .iter()
            .position(|event| matches!(event.kind, OperationEventKind::Output { .. }))
        else {
            break;
        };
        let Some(event) = record.events.remove(position) else {
            break;
        };
        let bytes = output_size(&event.kind);
        record.info.retained_output_bytes = record
            .info
            .retained_output_bytes
            .saturating_sub(u64::try_from(bytes).unwrap_or(u64::MAX));
        state.total_output = state.total_output.saturating_sub(bytes);
    }
}

fn trim_retention(state: &mut OperationStateStore) {
    while state.terminal_order.len() > MAX_TERMINAL_OPERATIONS || state.total_output > MAX_TOTAL_OUTPUT {
        let Some(id) = state.terminal_order.pop_front() else {
            break;
        };
        if let Some(record) = state.records.remove(&id) {
            state.total_output = state
                .total_output
                .saturating_sub(usize::try_from(record.info.retained_output_bytes).unwrap_or(usize::MAX));
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_output_is_bounded_per_record() {
        let mut state = OperationStateStore {
            next_id: 2,
            starting: false,
            active: None,
            records: BTreeMap::new(),
            terminal_order: VecDeque::new(),
            total_output: MAX_OPERATION_OUTPUT + 1,
        };
        let id = 1;
        let data = vec![0; MAX_OPERATION_OUTPUT + 1];
        state.records.insert(
            id,
            OperationRecord {
                info: OperationInfo {
                    id,
                    kind: NowExecutionKind::Batch,
                    state: OperationState::Running,
                    detached: false,
                    exit_code: None,
                    error: None,
                    retained_output_bytes: u64::try_from(data.len()).expect("usize fits u64"),
                    next_sequence: 1,
                },
                events: VecDeque::from([OperationEvent {
                    operation_id: id,
                    sequence: 0,
                    kind: OperationEventKind::Output {
                        stream: NowStream::Stdout,
                        data,
                        last: false,
                    },
                }]),
                subscribers: Vec::new(),
            },
        );

        trim_operation_events(&mut state, id);
        let record = state.records.get(&id).expect("record remains");
        assert_eq!(record.info.retained_output_bytes, 0);
        assert!(record.events.is_empty());
        assert_eq!(state.total_output, 0);
    }

    #[test]
    fn terminal_record_count_is_bounded() {
        let mut state = OperationStateStore {
            next_id: 34,
            starting: false,
            active: None,
            records: BTreeMap::new(),
            terminal_order: VecDeque::new(),
            total_output: 0,
        };
        for id in 1..=33 {
            state.terminal_order.push_back(id);
            state.records.insert(
                id,
                OperationRecord {
                    info: OperationInfo {
                        id,
                        kind: NowExecutionKind::Batch,
                        state: OperationState::Completed,
                        detached: false,
                        exit_code: Some(0),
                        error: None,
                        retained_output_bytes: 0,
                        next_sequence: 0,
                    },
                    events: VecDeque::new(),
                    subscribers: Vec::new(),
                },
            );
        }

        trim_retention(&mut state);
        assert_eq!(state.records.len(), MAX_TERMINAL_OPERATIONS);
        assert!(!state.records.contains_key(&1));
    }

    #[tokio::test]
    async fn slow_subscribers_are_disconnected_instead_of_buffering_output() {
        let manager = OperationManager::new(Arc::new(NowEndpoint::new().expect("endpoint allocation must succeed")));
        let id = 1;
        lock(&manager.state).records.insert(
            id,
            OperationRecord {
                info: OperationInfo {
                    id,
                    kind: NowExecutionKind::Batch,
                    state: OperationState::Running,
                    detached: false,
                    exit_code: None,
                    error: None,
                    retained_output_bytes: 0,
                    next_sequence: 0,
                },
                events: VecDeque::new(),
                subscribers: Vec::new(),
            },
        );

        let attachments: Vec<_> =
            core::iter::repeat_with(|| manager.attach(id, None).expect("subscription is accepted"))
                .take(MAX_LIVE_SUBSCRIBERS)
                .collect();
        manager.emit(id, OperationEventKind::Started);
        manager.emit(id, OperationEventKind::Started);

        assert!(
            lock(&manager.state)
                .records
                .get(&id)
                .expect("operation remains")
                .subscribers
                .is_empty()
        );
        for mut attachment in attachments {
            assert!(attachment.live.recv().await.is_some());
            assert!(attachment.live.recv().await.is_none());
        }
    }

    #[tokio::test]
    async fn dropped_subscription_can_replay_the_terminal_event() {
        let manager = OperationManager::new(Arc::new(NowEndpoint::new().expect("endpoint allocation must succeed")));
        let id = 1;
        lock(&manager.state).records.insert(
            id,
            OperationRecord {
                info: OperationInfo {
                    id,
                    kind: NowExecutionKind::Batch,
                    state: OperationState::Running,
                    detached: false,
                    exit_code: None,
                    error: None,
                    retained_output_bytes: 0,
                    next_sequence: 0,
                },
                events: VecDeque::new(),
                subscribers: Vec::new(),
            },
        );

        let mut attachment = manager.attach(id, None).expect("subscription is accepted");
        manager.emit(id, OperationEventKind::Started);
        manager.emit(id, OperationEventKind::Completed { exit_code: 0 });
        manager.finish(id, OperationState::Completed, Some(0), None);

        assert!(matches!(
            attachment.live.recv().await,
            Some(OperationEvent {
                kind: OperationEventKind::Started,
                ..
            })
        ));
        assert!(attachment.live.recv().await.is_none());

        let replay = manager
            .attach(id, Some(0))
            .expect("terminal operation remains attachable")
            .replay;
        assert!(matches!(
            replay.as_slice(),
            [OperationEvent {
                sequence: 1,
                kind: OperationEventKind::Completed { exit_code: 0 },
                ..
            }]
        ));
    }

    #[test]
    fn output_is_split_to_fit_the_ipc_message_limit() {
        let manager = OperationManager::new(Arc::new(NowEndpoint::new().expect("endpoint allocation must succeed")));
        let id = 1;
        lock(&manager.state).records.insert(
            id,
            OperationRecord {
                info: OperationInfo {
                    id,
                    kind: NowExecutionKind::Process,
                    state: OperationState::Running,
                    detached: false,
                    exit_code: None,
                    error: None,
                    retained_output_bytes: 0,
                    next_sequence: 0,
                },
                events: VecDeque::new(),
                subscribers: Vec::new(),
            },
        );

        manager.emit_output(id, NowStream::Stdout, vec![0; MAX_IPC_OUTPUT_CHUNK + 1], true);

        let record = lock(&manager.state);
        let events: Vec<_> = record
            .records
            .get(&id)
            .expect("operation remains")
            .events
            .iter()
            .collect();
        assert_eq!(events.len(), 2);
        assert_eq!(output_size(&events[0].kind), MAX_IPC_OUTPUT_CHUNK);
        assert_eq!(output_size(&events[1].kind), 1);
        assert!(matches!(
            &events[0].kind,
            OperationEventKind::Output { last: false, .. }
        ));
        assert!(matches!(&events[1].kind, OperationEventKind::Output { last: true, .. }));
    }

    #[test]
    fn non_final_empty_output_is_not_retained() {
        let manager = OperationManager::new(Arc::new(NowEndpoint::new().expect("endpoint allocation must succeed")));
        let id = 1;
        lock(&manager.state).records.insert(
            id,
            OperationRecord {
                info: OperationInfo {
                    id,
                    kind: NowExecutionKind::Process,
                    state: OperationState::Running,
                    detached: false,
                    exit_code: None,
                    error: None,
                    retained_output_bytes: 0,
                    next_sequence: 0,
                },
                events: VecDeque::new(),
                subscribers: Vec::new(),
            },
        );

        for _ in 0..=MAX_OPERATION_EVENTS {
            manager.emit_output(id, NowStream::Stdout, Vec::new(), false);
        }

        assert!(
            lock(&manager.state)
                .records
                .get(&id)
                .expect("operation remains")
                .events
                .is_empty()
        );
    }

    #[test]
    fn output_event_count_is_bounded() {
        let manager = OperationManager::new(Arc::new(NowEndpoint::new().expect("endpoint allocation must succeed")));
        let id = 1;
        lock(&manager.state).records.insert(
            id,
            OperationRecord {
                info: OperationInfo {
                    id,
                    kind: NowExecutionKind::Process,
                    state: OperationState::Running,
                    detached: false,
                    exit_code: None,
                    error: None,
                    retained_output_bytes: 0,
                    next_sequence: 0,
                },
                events: VecDeque::new(),
                subscribers: Vec::new(),
            },
        );

        for _ in 0..=MAX_OPERATION_EVENTS {
            manager.emit_output(id, NowStream::Stdout, Vec::new(), true);
        }

        assert_eq!(
            lock(&manager.state)
                .records
                .get(&id)
                .expect("operation remains")
                .events
                .len(),
            MAX_OPERATION_EVENTS
        );
    }
}
