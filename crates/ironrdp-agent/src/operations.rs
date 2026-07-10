//! Durable, daemon-owned NOW operation records.
//!
//! The NOW byte stream has one active execution at a time. This module intentionally separates that
//! transport lifetime from an IPC client lifetime: a client may disconnect and later replay retained
//! output or observe the terminal status without affecting the remote operation.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, bail};
use tokio::io::AsyncWrite;
use tokio::sync::Notify;

use crate::ipc::{NowExecutionRequest, NowOperationInfo, NowOperationState, NowStream, Payload, Response};
use crate::now::{NowClient, NowOperation, NowOperationEvent, NowStdinChunk};
use crate::transport::write_message;

/// Bounded output available for a later operation replay.
pub(crate) const OUTPUT_RETENTION_BYTES: usize = 8 * 1024 * 1024;
const MAX_RETAINED_OPERATIONS: usize = 32;
const MAX_TOTAL_RETAINED_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_STDIN_FRAGMENT_BYTES: usize = 64 * 1024;

#[derive(Clone)]
pub(crate) struct NowOperationManager {
    now: Arc<NowClient>,
    records: Arc<Mutex<BTreeMap<u64, OperationRecord>>>,
}

struct OperationRecord {
    info: NowOperationInfo,
    output: VecDeque<OutputEvent>,
    notifier: Arc<Notify>,
    stdin: Option<tokio::sync::mpsc::Sender<NowStdinChunk>>,
    stdin_closed: bool,
}

#[derive(Clone)]
struct OutputEvent {
    sequence: u64,
    stream: NowStream,
    data: Vec<u8>,
}

impl NowOperationManager {
    pub(crate) fn new(now: Arc<NowClient>) -> Self {
        Self {
            now,
            records: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    fn prune_records(&self) {
        let mut records = self.records.lock().expect("NOW operation records poisoned");
        prune_records(&mut records);
    }

    pub(crate) fn start(&self, request: NowExecutionRequest) -> anyhow::Result<NowOperationInfo> {
        let kind = request.kind;
        let detached = request.detached;
        let stdin_closed = request.stdin.is_some();
        let operation = self.now.start_execution(request)?;
        let info = NowOperationInfo {
            operation_id: operation.id,
            kind,
            state: if detached {
                NowOperationState::Detached
            } else {
                NowOperationState::Running
            },
            started_unix_ms: unix_time_ms(),
            finished_unix_ms: None,
            exit_code: None,
            error: None,
            stdout_bytes: 0,
            stderr_bytes: 0,
            retained_bytes: 0,
            dropped_bytes: 0,
            next_sequence: 1,
        };
        self.records.lock().expect("NOW operation records poisoned").insert(
            operation.id,
            OperationRecord {
                info: info.clone(),
                output: VecDeque::new(),
                notifier: Arc::new(Notify::new()),
                stdin: Some(operation.stdin.clone()),
                stdin_closed,
            },
        );
        self.prune_records();
        let manager = self.clone();
        tokio::spawn(async move {
            manager.collect(operation).await;
        });
        Ok(info)
    }

    pub(crate) fn list(&self) -> Vec<NowOperationInfo> {
        self.records
            .lock()
            .expect("NOW operation records poisoned")
            .values()
            .map(|record| record.info.clone())
            .collect()
    }

    pub(crate) fn status(&self, operation_id: u64) -> anyhow::Result<NowOperationInfo> {
        self.records
            .lock()
            .expect("NOW operation records poisoned")
            .get(&operation_id)
            .map(|record| record.info.clone())
            .with_context(|| format!("NOW operation {operation_id} was not found"))
    }

    pub(crate) fn cancel(&self, operation_id: u64) -> anyhow::Result<NowOperationInfo> {
        {
            let records = self.records.lock().expect("NOW operation records poisoned");
            let record = records
                .get(&operation_id)
                .with_context(|| format!("NOW operation {operation_id} was not found"))?;
            if is_terminal(record.info.state) {
                bail!("NOW operation {operation_id} is already complete");
            }

            if record.info.state == NowOperationState::Detached {
                bail!("detached NOW operation {operation_id} cannot be cancelled or observed");
            }
        }
        self.now.cancel(operation_id)?;
        let mut records = self.records.lock().expect("NOW operation records poisoned");
        let record = records
            .get_mut(&operation_id)
            .expect("operation exists after cancellation validation");
        record.info.state = NowOperationState::Cancelling;
        record.notifier.notify_waiters();
        Ok(record.info.clone())
    }

    /// Delivers a bounded stdin fragment while the protocol worker owns the NOW byte stream.
    pub(crate) async fn write_stdin(&self, operation_id: u64, data: Vec<u8>, last: bool) -> anyhow::Result<()> {
        if data.len() > MAX_STDIN_FRAGMENT_BYTES {
            bail!(
                "NOW standard input fragment exceeds the {} KiB limit",
                MAX_STDIN_FRAGMENT_BYTES / 1024
            );
        }
        let sender = {
            let mut records = self.records.lock().expect("NOW operation records poisoned");
            let record = records
                .get_mut(&operation_id)
                .with_context(|| format!("NOW operation {operation_id} was not found"))?;
            if record.info.state != NowOperationState::Running {
                bail!("NOW operation {operation_id} is not accepting standard input");
            }
            if record.stdin_closed {
                bail!("NOW operation {operation_id} standard input is already closed");
            }
            if last {
                record.stdin_closed = true;
            }
            record
                .stdin
                .as_ref()
                .cloned()
                .context("NOW operation standard input is unavailable")?
        };
        sender
            .try_send(NowStdinChunk { data, last })
            .map_err(|error| match error {
                tokio::sync::mpsc::error::TrySendError::Full(_) => {
                    anyhow::anyhow!("NOW operation standard input is backpressured; retry")
                }
                tokio::sync::mpsc::error::TrySendError::Closed(_) => {
                    anyhow::anyhow!("NOW operation standard input worker stopped")
                }
            })
    }

    pub(crate) async fn attach<S>(
        &self,
        operation_id: u64,
        mut after_sequence: u64,
        stream: &mut S,
    ) -> anyhow::Result<()>
    where
        S: AsyncWrite + Unpin,
    {
        loop {
            let notifier = {
                let records = self.records.lock().expect("NOW operation records poisoned");
                let record = records
                    .get(&operation_id)
                    .with_context(|| format!("NOW operation {operation_id} was not found"))?;
                Arc::clone(&record.notifier)
            };
            // Register before taking the snapshot. `enable` closes the otherwise subtle race where
            // a producer notifies between observing no events and awaiting this attachment.
            let notification = notifier.notified();
            tokio::pin!(notification);
            notification.as_mut().enable();
            let (events, info) = {
                let records = self.records.lock().expect("NOW operation records poisoned");
                let record = records
                    .get(&operation_id)
                    .with_context(|| format!("NOW operation {operation_id} was not found"))?;
                (
                    record
                        .output
                        .iter()
                        .filter(|event| event.sequence > after_sequence)
                        .cloned()
                        .collect::<Vec<_>>(),
                    record.info.clone(),
                )
            };

            for event in events {
                after_sequence = event.sequence;
                write_message(
                    stream,
                    &Response::Ok(Payload::NowExecutionData {
                        operation_id,
                        sequence: event.sequence,
                        stream: event.stream,
                        data: event.data,
                    }),
                )
                .await?;
            }

            if is_terminal(info.state) {
                return write_terminal(stream, info).await;
            }

            notification.await;
        }
    }

    async fn collect(&self, mut operation: NowOperation) {
        while let Some(event) = operation.events.recv().await {
            let mut request_cancel = false;
            {
                let mut records = self.records.lock().expect("NOW operation records poisoned");
                let Some(record) = records.get_mut(&operation.id) else {
                    return;
                };
                match event {
                    NowOperationEvent::Data { stream, data } => {
                        request_cancel = retain_data(record, stream, data);
                    }
                    NowOperationEvent::Finished { exit_code } => {
                        if record.info.state != NowOperationState::Detached {
                            record.info.state = NowOperationState::Succeeded;
                            record.info.exit_code = Some(exit_code);
                        }
                        record.info.finished_unix_ms = Some(unix_time_ms());
                        record.stdin = None;
                    }
                    NowOperationEvent::Failed { message } => {
                        record.info.finished_unix_ms = Some(unix_time_ms());
                        if record.info.state == NowOperationState::Cancelling {
                            record.info.state = NowOperationState::Cancelled;
                            if record.info.error.is_none() {
                                record.info.error = Some(message);
                            }
                        } else {
                            record.info.state = NowOperationState::Failed;
                            record.info.error = Some(message);
                        }
                        record.stdin = None;
                    }
                }
                record.notifier.notify_waiters();
            }
            self.prune_records();
            if request_cancel {
                if let Err(error) = self.now.cancel(operation.id) {
                    let mut records = self.records.lock().expect("NOW operation records poisoned");
                    if let Some(record) = records.get_mut(&operation.id) {
                        record.info.state = NowOperationState::Failed;
                        record.info.finished_unix_ms = Some(unix_time_ms());
                        record.info.error = Some(format!(
                            "NOW output retention limit reached and cancellation failed: {error:#}"
                        ));
                        record.notifier.notify_waiters();
                    }
                }
            }
        }
    }
}

fn prune_records(records: &mut BTreeMap<u64, OperationRecord>) {
    while records.len() > MAX_RETAINED_OPERATIONS
        || records.values().map(|record| record.info.retained_bytes).sum::<u64>()
            > u64::try_from(MAX_TOTAL_RETAINED_OUTPUT_BYTES).expect("retention budget fits in u64")
    {
        let Some(operation_id) = records
            .iter()
            .find_map(|(operation_id, record)| is_terminal(record.info.state).then_some(*operation_id))
        else {
            return;
        };
        records.remove(&operation_id);
    }
}

fn retain_data(record: &mut OperationRecord, stream: NowStream, data: Vec<u8>) -> bool {
    match stream {
        NowStream::Stdout => record.info.stdout_bytes += u64::try_from(data.len()).unwrap_or(u64::MAX),
        NowStream::Stderr => record.info.stderr_bytes += u64::try_from(data.len()).unwrap_or(u64::MAX),
    }
    let data_len = data.len();
    if record.info.retained_bytes + u64::try_from(data_len).unwrap_or(u64::MAX)
        <= u64::try_from(OUTPUT_RETENTION_BYTES).expect("retention cap fits in u64")
    {
        let sequence = record.info.next_sequence;
        record.info.next_sequence += 1;
        record.info.retained_bytes += u64::try_from(data_len).unwrap_or(u64::MAX);
        record.output.push_back(OutputEvent { sequence, stream, data });
        false
    } else {
        record.info.dropped_bytes += u64::try_from(data_len).unwrap_or(u64::MAX);
        if record.info.state == NowOperationState::Running {
            record.info.state = NowOperationState::Cancelling;
            record.info.error = Some(format!(
                "NOW output exceeded the {} MiB retained operation limit; cancellation requested",
                OUTPUT_RETENTION_BYTES / (1024 * 1024)
            ));
            true
        } else {
            false
        }
    }
}

async fn write_terminal<S>(stream: &mut S, info: NowOperationInfo) -> anyhow::Result<()>
where
    S: AsyncWrite + Unpin,
{
    match info.state {
        NowOperationState::Succeeded => {
            write_message(
                stream,
                &Response::Ok(Payload::NowExecutionResult {
                    operation_id: info.operation_id,
                    exit_code: info.exit_code.unwrap_or(0),
                }),
            )
            .await
        }
        NowOperationState::Detached => write_message(stream, &Response::Ok(Payload::NowOperationInfo(info))).await,
        NowOperationState::Cancelled | NowOperationState::Failed => {
            write_message(
                stream,
                &Response::error(info.error.unwrap_or_else(|| "NOW operation failed".to_owned())),
            )
            .await
        }
        NowOperationState::Running | NowOperationState::Cancelling => unreachable!("terminal state checked above"),
    }
}

fn is_terminal(state: NowOperationState) -> bool {
    matches!(
        state,
        NowOperationState::Succeeded
            | NowOperationState::Failed
            | NowOperationState::Cancelled
            | NowOperationState::Detached
    )
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> OperationRecord {
        OperationRecord {
            info: NowOperationInfo {
                operation_id: 17,
                kind: crate::ipc::NowExecutionKind::PowerShell,
                state: NowOperationState::Running,
                started_unix_ms: 1,
                finished_unix_ms: None,
                exit_code: None,
                error: None,
                stdout_bytes: 0,
                stderr_bytes: 0,
                retained_bytes: 0,
                dropped_bytes: 0,
                next_sequence: 1,
            },
            output: VecDeque::new(),
            notifier: Arc::new(Notify::new()),
            stdin: None,
            stdin_closed: false,
        }
    }

    #[test]
    fn retained_output_is_sequenced_and_bounded() {
        let mut record = record();
        assert!(!retain_data(&mut record, NowStream::Stdout, b"one".to_vec()));
        assert!(!retain_data(&mut record, NowStream::Stderr, b"two".to_vec()));
        assert_eq!(record.info.stdout_bytes, 3);
        assert_eq!(record.info.stderr_bytes, 3);
        assert_eq!(record.info.retained_bytes, 6);
        assert_eq!(record.info.next_sequence, 3);
        assert_eq!(record.output[0].sequence, 1);
        assert_eq!(record.output[1].sequence, 2);
    }

    #[test]
    fn overflowing_retention_requests_cancellation_without_unbounded_storage() {
        let mut record = record();
        record.info.retained_bytes = u64::try_from(OUTPUT_RETENTION_BYTES).expect("retention cap fits");
        assert!(retain_data(&mut record, NowStream::Stdout, vec![0]));
        assert_eq!(record.info.state, NowOperationState::Cancelling);
        assert_eq!(record.info.dropped_bytes, 1);
        assert!(record.output.is_empty());
    }

    #[test]
    fn terminal_record_retention_evicts_oldest_operations() {
        let mut records = BTreeMap::new();
        for operation_id in 1..=MAX_RETAINED_OPERATIONS + 1 {
            let mut value = record();
            value.info.operation_id = u64::try_from(operation_id).expect("operation ID fits");
            value.info.state = NowOperationState::Succeeded;
            records.insert(value.info.operation_id, value);
        }
        prune_records(&mut records);
        assert_eq!(records.len(), MAX_RETAINED_OPERATIONS);
        assert!(!records.contains_key(&1));
        assert!(records.contains_key(&u64::try_from(MAX_RETAINED_OPERATIONS + 1).expect("operation ID fits")));
    }
}
