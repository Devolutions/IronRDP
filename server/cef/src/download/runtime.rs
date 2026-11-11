use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use ironrdp_core::impl_as_any;
use ironrdp_pdu::PduResult;
use ironrdp_pdu::rdp::server_error_info::{ErrorInfo, ProtocolIndependentCode};
use ironrdp_rdpdr::pdu::efs::*;
use ironrdp_server::{RdpdrServerBackend, RdpdrServerFactory, RdpdrServerMessage, ServerEvent, ServerEventSender};
use tokio::sync::mpsc::UnboundedSender;

use super::{Budget, Limits, collision_name, safe_name};

const CHUNK: usize = 64 * 1024;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug)]
struct Entry {
    epoch: u64,
    name: String,
    path: PathBuf,
    received: u64,
    delivered: u64,
    cef_terminal: bool,
    ready: bool,
    cancelled: bool,
    retiring: bool,
    retired: bool,
    committed: bool,
    closed: bool,
    status: String,
    started: Instant,
}

#[derive(Debug)]
struct Connection {
    epoch: u64,
    device: Option<u32>,
    fenced: bool,
    /// INVARIANT: At most one IRP is outstanding, and an uncertain result fences its connection.
    pending: Option<Pending>,
    sender: Option<UnboundedSender<ServerEvent>>,
    completions: mpsc::SyncSender<Ack>,
}

#[derive(Debug)]
struct State {
    enabled: bool,
    root: PathBuf,
    nonce: u128,
    budget: Budget,
    entries: BTreeMap<u32, Entry>,
    latest: String,
    latest_failure: String,
    connection: Connection,
    stopping: bool,
}

#[derive(Clone, Debug)]
pub struct Downloads(Arc<Mutex<State>>);

impl Downloads {
    pub fn new(profile: &Path, enabled: bool, limits: Limits) -> anyhow::Result<(Self, JoinHandle<()>)> {
        let budget = Budget::new(limits)?;
        let root = profile.join("downloads");
        std::fs::create_dir(&root)?;
        let (tx, rx) = mpsc::sync_channel(4);
        let downloads = Self(Arc::new(Mutex::new(State {
            enabled,
            root,
            nonce: SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos(),
            budget,
            entries: BTreeMap::new(),
            latest: String::new(),
            latest_failure: String::new(),
            connection: Connection {
                epoch: 0,
                device: None,
                fenced: false,
                pending: None,
                sender: None,
                completions: tx,
            },
            stopping: false,
        })));
        let worker = downloads.clone();
        let handle = std::thread::spawn(move || {
            let monitor = worker.clone();
            let monitor_thread = std::thread::spawn(move || monitor.monitor());
            worker.run(rx);
            let _ = monitor_thread.join();
            let ids = worker
                .0
                .lock()
                .unwrap()
                .entries
                .iter()
                .filter(|(_, entry)| !entry.retired)
                .map(|(&id, _)| id)
                .collect::<Vec<_>>();
            for id in ids {
                worker.retire(id);
            }
        });
        Ok((downloads, handle))
    }

    pub fn available(&self) -> anyhow::Result<()> {
        let state = self.0.lock().unwrap();
        anyhow::ensure!(state.enabled, "downloads are disabled; enable --downloads");
        anyhow::ensure!(
            state.connection.device.is_some() && !state.connection.fenced && !state.stopping,
            "approved DL redirected drive is unavailable"
        );
        Ok(())
    }

    pub fn notice(&self, error: &str) {
        tracing::warn!(download_state = "rejected", error, "Download rejected");
        self.0.lock().unwrap().latest_failure = format!("Failed: {error}");
    }

    pub fn begin(&self, id: u32, name: &str) -> anyhow::Result<PathBuf> {
        self.available()?;
        let name = safe_name(name)?;
        let mut state = self.0.lock().unwrap();
        anyhow::ensure!(
            state.connection.device.is_some() && !state.connection.fenced && !state.stopping,
            "approved DL redirected drive is unavailable"
        );
        anyhow::ensure!(!state.entries.contains_key(&id), "duplicate CEF download identifier");
        let clear_failure = state.entries.values().all(|entry| entry.retired);
        state.budget.start(id)?;
        while 16 <= state.entries.len() {
            let old = state.entries.iter().find(|(_, entry)| entry.retired).map(|(&id, _)| id);
            if let Some(old) = old {
                state.entries.remove(&old);
            } else {
                anyhow::bail!("download status capacity exhausted");
            }
        }
        if clear_failure {
            state.latest.clear();
            state.latest_failure.clear();
        }
        let path = state.root.join(format!("cef-{id}.part"));
        let epoch = state.connection.epoch;
        state.entries.insert(
            id,
            Entry {
                epoch,
                name,
                path: path.clone(),
                received: 0,
                delivered: 0,
                cef_terminal: false,
                ready: false,
                cancelled: false,
                retiring: false,
                retired: false,
                committed: false,
                closed: false,
                status: "Browser download".to_owned(),
                started: Instant::now(),
            },
        );
        tracing::info!(id, download_state = "browser", "Download accepted for private staging");
        Ok(path)
    }

    pub fn progress(&self, id: u32, received: u64, complete: bool, cancelled: bool) {
        let mut state = self.0.lock().unwrap();
        let Some(entry) = state.entries.get(&id) else { return };
        if entry.retiring || entry.retired || entry.closed {
            return;
        }
        let received = entry.received.max(received);
        let changed = received != entry.received || !entry.cancelled;
        let limit_error = state.budget.observe(id, received).err();
        let entry = state.entries.get_mut(&id).unwrap();
        entry.received = received;
        entry.cef_terminal |= complete || cancelled;
        entry.ready |= complete;
        if let Some(error) = limit_error {
            entry.cancelled = true;
            entry.status = format!("Failed: {error}; observed {received} B");
            if changed {
                tracing::warn!(id, observed_bytes = received, %error, download_state = "failed", "Download staging limit exceeded");
            }
        } else if cancelled && !entry.cancelled {
            entry.cancelled = true;
            entry.status = "Failed: browser cancelled or interrupted download".to_owned();
            tracing::warn!(
                id,
                download_state = "failed",
                "Browser cancelled or interrupted download"
            );
        }
    }

    pub fn should_cancel(&self, id: u32) -> bool {
        let state = self.0.lock().unwrap();
        state.stopping || state.entries.get(&id).is_some_and(|entry| entry.cancelled)
    }

    pub fn tracked(&self, id: u32) -> bool {
        self.0.lock().unwrap().entries.contains_key(&id)
    }

    pub fn cancel_all(&self) {
        let mut state = self.0.lock().unwrap();
        for (&id, entry) in state
            .entries
            .iter_mut()
            .filter(|(_, entry)| !entry.retiring && !entry.retired && !entry.committed)
        {
            if !entry.cancelled {
                entry.cancelled = true;
                entry.status = "Cancelled by user".to_owned();
                tracing::info!(id, download_state = "cancelled", "Download cancelled by user");
            }
        }
    }

    pub fn status(&self) -> String {
        let state = self.0.lock().unwrap();
        let active: Vec<_> = state.entries.iter().filter(|(_, entry)| !entry.retired).collect();
        let progress = if active.is_empty() {
            state.latest.clone()
        } else {
            active
                .iter()
                .map(|(id, entry)| {
                    format!(
                        "#{id} {}: browser {} B / client {} B",
                        entry.status, entry.received, entry.delivered
                    )
                })
                .collect::<Vec<_>>()
                .join(" | ")
        };
        match (state.latest_failure.is_empty(), progress.is_empty()) {
            (true, _) => progress,
            (false, true) => state.latest_failure.clone(),
            (false, false) => format!("{} | {progress}", state.latest_failure),
        }
    }

    pub fn stop(&self) {
        self.cancel_all();
        self.0.lock().unwrap().stopping = true;
    }

    pub fn disconnected(&self) -> bool {
        let mut state = self.0.lock().unwrap();
        let unsafe_pending = state.connection.pending.is_some();
        state.connection.device = None;
        state.connection.fenced = true;
        for (&id, entry) in state
            .entries
            .iter_mut()
            .filter(|(_, entry)| !entry.retiring && !entry.retired && !entry.closed)
        {
            entry.cancelled = true;
            entry.status = if entry.committed {
                "Failed: client file published but connection lost before close confirmation"
            } else {
                "Failed: client disconnected"
            }
            .to_owned();
            tracing::warn!(
                id,
                download_state = "failed",
                "Download interrupted by client disconnect"
            );
        }
        unsafe_pending
    }

    pub fn factory(&self) -> Box<dyn RdpdrServerFactory> {
        Box::new(Factory(self.clone()))
    }

    fn fail(&self, id: u32, error: &anyhow::Error) {
        let mut state = self.0.lock().unwrap();
        if let Some(entry) = state.entries.get_mut(&id) {
            if entry.retiring || entry.retired {
                return;
            }
            tracing::warn!(id, %error, download_state = "failed", "Download delivery failed");
            entry.cancelled = true;
            entry.status = if entry.committed {
                format!("Failed: client file was published; {error:#}")
            } else {
                format!("Failed: {error:#}")
            };
        }
    }

    fn fence(&self, epoch: u64) {
        let mut state = self.0.lock().unwrap();
        Self::fence_locked(&mut state, epoch);
    }

    fn fence_removed_device(&self, epoch: u64, ids: &[u32]) -> bool {
        let mut state = self.0.lock().unwrap();
        if state.connection.epoch != epoch {
            return false;
        }
        if state.connection.fenced {
            return state.connection.pending.is_some();
        }
        if !state.connection.device.is_some_and(|id| ids.contains(&id)) {
            return false;
        }
        Self::fence_locked(&mut state, epoch)
    }

    fn fence_locked(state: &mut State, epoch: u64) -> bool {
        if state.connection.epoch != epoch {
            return false;
        }
        let pending = state.connection.pending.is_some();
        if state.connection.fenced {
            return pending;
        }
        state.connection.fenced = true;
        state.connection.device = None;
        for entry in state
            .entries
            .values_mut()
            .filter(|entry| !entry.retiring && !entry.retired && !entry.closed)
        {
            if !entry.cancelled {
                entry.cancelled = true;
                entry.status = "Failed: download connection fenced".to_owned();
            }
        }
        match &state.connection.sender {
            Some(sender) => {
                if let Err(error) = sender.send(ServerEvent::Disconnect(ErrorInfo::ProtocolIndependentCode(
                    ProtocolIndependentCode::RpcInitiatedDisconnect,
                ))) {
                    tracing::warn!(%error, "Failed to request disconnect of fenced download connection");
                }
            }
            None => tracing::warn!("Cannot disconnect fenced download connection without an event sender"),
        }
        pending
    }

    fn retire(&self, id: u32) {
        let path = {
            let mut state = self.0.lock().unwrap();
            let entry = state.entries.get_mut(&id).unwrap();
            // UNTESTED: A retiring entry must not be pruned until its staging file is removed.
            // Check a concurrent begin during unlink when changing this state; pruning it can panic and leak the budget.
            entry.retiring = true;
            entry.path.clone()
        };
        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                self.0.lock().unwrap().entries.get_mut(&id).unwrap().retiring = false;
                self.fail(
                    id,
                    &anyhow::Error::new(error).context("failed to remove staged download"),
                );
                return;
            }
        }
        let mut state = self.0.lock().unwrap();
        let entry = state.entries.get_mut(&id).unwrap();
        entry.retiring = false;
        entry.retired = true;
        let status = format!(
            "#{} {}: browser {} B / client {} B",
            id, entry.status, entry.received, entry.delivered
        );
        if entry.status.starts_with("Failed:") {
            state.latest_failure = status;
        } else {
            state.latest = status;
        }
        state.budget.finish(id);
    }

    fn monitor(&self) {
        loop {
            let (stopping, entries) = {
                let state = self.0.lock().unwrap();
                (
                    state.stopping,
                    state
                        .entries
                        .iter()
                        .filter(|(_, entry)| !entry.retiring && !entry.retired && !entry.closed)
                        .map(|(&id, entry)| (id, entry.path.clone(), entry.cef_terminal, entry.started))
                        .collect::<Vec<_>>(),
                )
            };
            for (id, path, cef_terminal, started) in entries {
                match std::fs::metadata(&path) {
                    Ok(metadata) => self.progress(id, metadata.len(), false, false),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound && !cef_terminal => {}
                    Err(error) if !self.should_cancel(id) => self.fail(id, &error.into()),
                    Err(_) => {}
                }
                if DOWNLOAD_TIMEOUT <= started.elapsed() && !self.should_cancel(id) {
                    self.fail(id, &anyhow::anyhow!("browser download timed out"));
                }
            }
            if stopping {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn run(&self, responses: mpsc::Receiver<Ack>) {
        loop {
            let next = {
                let state = self.0.lock().unwrap();
                if state.stopping {
                    return;
                }
                state
                    .entries
                    .iter()
                    .find(|(_, entry)| {
                        !entry.retiring
                            && !entry.retired
                            && ((entry.ready && !entry.cancelled) || (entry.cancelled && entry.cef_terminal))
                    })
                    .map(|(&id, entry)| (id, entry.epoch, entry.cancelled))
            };
            if let Some((id, epoch, cancelled)) = next {
                if !cancelled {
                    if let Err(error) = self.deliver(&responses, id) {
                        self.fail(id, &error);
                        self.fence(epoch);
                    }
                }
                self.retire(id);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn request(
        &self,
        responses: &mpsc::Receiver<Ack>,
        id: u32,
        kind: Kind,
        message: RdpdrServerMessage,
    ) -> anyhow::Result<Ack> {
        let rename_length = match &message {
            RdpdrServerMessage::SetInformation { set_buffer, .. } => Some(set_buffer.size() as u32),
            _ => None,
        };
        let (epoch, device) = {
            let mut state = self.0.lock().unwrap();
            let entry = &state.entries[&id];
            let received = entry.received;
            anyhow::ensure!(!entry.cancelled && !state.stopping, "download cancelled");
            anyhow::ensure!(
                entry.epoch == state.connection.epoch && !state.connection.fenced,
                "download connection expired"
            );
            let device = state.connection.device.context("approved download drive removed")?;
            anyhow::ensure!(state.connection.pending.is_none(), "download request slot is occupied");
            let epoch = state.connection.epoch;
            state.budget.observe(id, received)?;
            state.connection.pending = Some(Pending { device, kind });
            let sender = state
                .connection
                .sender
                .as_ref()
                .context("RDPDR event sender unavailable")?;
            sender
                .send(ServerEvent::Rdpdr(message))
                .map_err(|_| anyhow::anyhow!("RDPDR event sender closed"))?;
            (epoch, device)
        };
        let deadline = Instant::now() + RESPONSE_TIMEOUT;
        let mut received = None;
        loop {
            let state = self.0.lock().unwrap();
            if received.is_none() {
                match responses.try_recv() {
                    Ok(ack) => received = Some(ack),
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => anyhow::bail!("RDPDR completion channel closed"),
                }
            }
            if let Some(ack) = received.take() {
                if ack.epoch < epoch {
                    continue;
                }
                anyhow::ensure!(
                    ack.epoch == epoch && ack.device == device && ack.kind == kind,
                    "unexpected RDPDR acknowledgment"
                );
                if ack.status == NtStatus::SUCCESS {
                    if let Some(length) = rename_length {
                        anyhow::ensure!(
                            ack.value == length,
                            "client returned an invalid rename acknowledgment length"
                        );
                    }
                }
                tracing::debug!(id, ?kind, status = ?ack.status, "Received download RDPDR acknowledgment");
                return Ok(ack);
            }
            let invalid = state.stopping
                || state.connection.epoch != epoch
                || state.connection.fenced
                || state.entries[&id].cancelled;
            anyhow::ensure!(
                !invalid,
                "download cancelled or connection lost awaiting {kind:?}; client operation outcome is unconfirmed"
            );
            anyhow::ensure!(Instant::now() < deadline, "RDPDR acknowledgment timed out");
            drop(state);
            match responses.recv_timeout(Duration::from_millis(20)) {
                Ok(ack) => received = Some(ack),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => anyhow::bail!("RDPDR completion channel closed"),
            }
        }
    }

    fn deliver(&self, responses: &mpsc::Receiver<Ack>, id: u32) -> anyhow::Result<()> {
        let (path, name, epoch, device, nonce, expected) = {
            let mut state = self.0.lock().unwrap();
            let device = state
                .connection
                .device
                .context("approved DL redirected drive unavailable")?;
            let nonce = state.nonce;
            let entry = state.entries.get_mut(&id).unwrap();
            entry.status = "Delivering".to_owned();
            (
                entry.path.clone(),
                entry.name.clone(),
                entry.epoch,
                device,
                nonce,
                entry.received,
            )
        };
        let mut file = File::open(&path).context("failed to open completed staged download")?;
        self.progress(id, file.metadata()?.len(), false, false);
        anyhow::ensure!(
            file.metadata()?.len() == expected,
            "completed staged download length changed"
        );
        let ack = self.request(
            responses,
            id,
            Kind::Create,
            RdpdrServerMessage::Create {
                device_id: device,
                path: format!(".ircef-{nonce:x}-{epoch:x}-{id:x}.part"),
                desired_access: DesiredAccess::FILE_WRITE_DATA_OR_FILE_ADD_FILE
                    | DesiredAccess::DELETE
                    | DesiredAccess::SYNCHRONIZE,
                create_disposition: CreateDisposition::FILE_CREATE,
                create_options: CreateOptions::FILE_NON_DIRECTORY_FILE,
            },
        )?;
        ack.success()?;
        let file_id = ack.value;
        let mut offset = 0u64;
        let mut writes = 0u64;
        loop {
            let mut data = vec![0; CHUNK];
            let read = file.read(&mut data)?;
            if read == 0 {
                break;
            }
            data.truncate(read);
            anyhow::ensure!(offset + read as u64 <= expected, "staged download grew during delivery");
            let ack = self.request(
                responses,
                id,
                Kind::Write,
                RdpdrServerMessage::Write {
                    device_id: device,
                    file_id,
                    data,
                    offset,
                },
            )?;
            ack.success()?;
            anyhow::ensure!(ack.value as usize == read, "client returned a short download write");
            offset += read as u64;
            writes += 1;
            self.0.lock().unwrap().entries.get_mut(&id).unwrap().delivered = offset;
        }
        anyhow::ensure!(offset == expected, "staged download truncated during delivery");
        self.request(
            responses,
            id,
            Kind::Flush,
            RdpdrServerMessage::FlushBuffers {
                device_id: device,
                file_id,
            },
        )?
        .success()?;
        let mut published = None;
        for attempt in 0..1000 {
            let candidate = collision_name(&name, attempt);
            let ack = self.request(
                responses,
                id,
                Kind::Rename,
                RdpdrServerMessage::SetInformation {
                    device_id: device,
                    file_id,
                    set_buffer: FileInformationClass::Rename(FileRenameInformation {
                        replace_if_exists: Boolean::False,
                        file_name: candidate.clone(),
                    }),
                },
            )?;
            if ack.status == NtStatus::OBJECT_NAME_COLLISION {
                continue;
            }
            ack.success()?;
            published = Some(candidate);
            break;
        }
        let published = published.context("download filename collision limit reached")?;
        {
            let mut state = self.0.lock().unwrap();
            let entry = state.entries.get_mut(&id).unwrap();
            entry.committed = true;
            entry.status = format!("Published: {published}; awaiting client close");
        }
        tracing::info!(
            id,
            bytes = offset,
            download_state = "committed",
            "Download committed by non-replacing client rename"
        );
        self.request(
            responses,
            id,
            Kind::Close,
            RdpdrServerMessage::Close {
                device_id: device,
                file_id,
            },
        )
        .context("client file published but close was not acknowledged")?
        .success()
        .context("client file published but close failed")?;
        let mut state = self.0.lock().unwrap();
        let entry = state.entries.get_mut(&id).unwrap();
        entry.closed = true;
        entry.status = format!("Completed: {published}");
        tracing::info!(
            id,
            rdpdr_bytes = offset,
            rdpdr_writes = writes,
            download_state = "completed",
            "Download published and closed on the client over RDPDR"
        );
        Ok(())
    }
}

struct Factory(Downloads);

const MAX_RDPDR_MESSAGE_SIZE: usize = 1_024 * 1_024;

impl ServerEventSender for Factory {
    fn set_sender(&mut self, sender: UnboundedSender<ServerEvent>) {
        self.0.0.lock().unwrap().connection.sender = Some(sender);
    }
}

impl RdpdrServerFactory for Factory {
    fn build_backend(&self) -> Box<dyn RdpdrServerBackend> {
        let mut state = self.0.0.lock().unwrap();
        state.connection.epoch = state
            .connection
            .epoch
            .checked_add(1)
            .expect("connection epoch exhausted");
        state.connection.device = None;
        state.connection.fenced = false;
        state.connection.pending = None;
        Box::new(Backend {
            downloads: self.0.clone(),
            epoch: state.connection.epoch,
        })
    }
}

#[derive(Debug)]
struct Backend {
    downloads: Downloads,
    epoch: u64,
}

impl_as_any!(Backend);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Create,
    Write,
    Flush,
    Rename,
    Close,
    Unexpected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Pending {
    device: u32,
    kind: Kind,
}

#[derive(Debug)]
struct Ack {
    epoch: u64,
    device: u32,
    kind: Kind,
    status: NtStatus,
    value: u32,
}

impl Ack {
    fn success(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.status == NtStatus::SUCCESS,
            "client rejected {:?}: {:?}",
            self.kind,
            self.status
        );
        Ok(())
    }
}

impl Backend {
    fn ack(&self, response: &DeviceIoResponse, kind: Kind, value: u32) -> PduResult<()> {
        let mut state = self.downloads.0.lock().unwrap();
        if state.connection.epoch != self.epoch || state.connection.fenced {
            return Ok(());
        }
        if state.connection.pending
            != Some(Pending {
                device: response.device_id,
                kind,
            })
            || state
                .connection
                .completions
                .try_send(Ack {
                    epoch: self.epoch,
                    device: response.device_id,
                    kind,
                    status: response.io_status,
                    value,
                })
                .is_err()
        {
            drop(state);
            self.downloads.fence(self.epoch);
        } else {
            state.connection.pending = None;
        }
        Ok(())
    }
}

macro_rules! unused_completions {
    ($($method:ident($ty:ty)),* $(,)?) => {
        $(fn $method(&mut self, response: &$ty) -> PduResult<()> {
            self.ack(&response.device_io_reply, Kind::Unexpected, 0)
        })*
    };
}

impl RdpdrServerBackend for Backend {
    fn max_message_size(&self) -> Option<usize> {
        Some(MAX_RDPDR_MESSAGE_SIZE)
    }

    fn on_device_announce(&mut self, devices: &[DeviceAnnounceHeader]) -> Vec<(u32, bool)> {
        let mut state = self.downloads.0.lock().unwrap();
        if state.connection.epoch != self.epoch || state.connection.fenced {
            return devices.iter().map(|device| (device.device_id(), false)).collect();
        }
        let mut accepted = Vec::new();
        for device in devices {
            let accept = state.enabled
                && device.device_type() == DeviceType::Filesystem
                && device.preferred_dos_name() == "DL"
                && state.connection.device.is_none();
            if accept {
                state.connection.device = Some(device.device_id());
                tracing::info!(device_id = device.device_id(), "Approved download drive announced");
            }
            accepted.push((device.device_id(), accept));
        }
        accepted
    }

    fn on_device_remove(&mut self, ids: &[u32]) {
        self.downloads.fence_removed_device(self.epoch, ids);
    }
    fn try_on_device_remove(&mut self, ids: &[u32]) -> PduResult<()> {
        if self.downloads.fence_removed_device(self.epoch, ids) {
            return Err(ironrdp_pdu::pdu_other_err!(
                "cannot remove a download device with an unacknowledged request"
            ));
        }
        Ok(())
    }
    fn on_reinitialize(&mut self, _: &[u32]) -> PduResult<()> {
        let mut state = self.downloads.0.lock().unwrap();
        if state.connection.epoch != self.epoch || state.connection.fenced {
            return Err(ironrdp_pdu::pdu_other_err!("download connection is already fenced"));
        }
        if state.connection.pending.is_some() {
            drop(state);
            self.downloads.fence(self.epoch);
            return Err(ironrdp_pdu::pdu_other_err!(
                "cannot reinitialize with an unacknowledged download request"
            ));
        }
        let epoch = state
            .connection
            .epoch
            .checked_add(1)
            .ok_or_else(|| ironrdp_pdu::pdu_other_err!("download connection epoch exhausted"))?;
        state.connection.device = None;
        state.connection.epoch = epoch;
        for (&id, entry) in state
            .entries
            .iter_mut()
            .filter(|(_, entry)| !entry.retiring && !entry.retired && !entry.closed)
        {
            entry.cancelled = true;
            entry.status = if entry.committed {
                "Failed: client file published before download device reinitialized"
            } else {
                "Failed: download device reinitialized"
            }
            .to_owned();
            tracing::warn!(
                id,
                download_state = "failed",
                "Download interrupted by device reinitialization"
            );
        }
        self.epoch = epoch;
        Ok(())
    }
    fn on_client_name(&mut self, _: &str) {}
    fn on_create_complete(&mut self, response: &DeviceCreateResponse) -> PduResult<()> {
        self.ack(&response.device_io_reply, Kind::Create, response.file_id)
    }
    fn on_write_complete(&mut self, response: &DeviceWriteResponse) -> PduResult<()> {
        self.ack(&response.device_io_reply, Kind::Write, response.length)
    }
    fn on_flush_buffers_complete(&mut self, response: &DeviceFlushBuffersResponse) -> PduResult<()> {
        self.ack(&response.device_io_response, Kind::Flush, 0)
    }
    fn on_set_information_complete(&mut self, response: &ClientDriveSetInformationResponse) -> PduResult<()> {
        self.ack(response.device_io_reply(), Kind::Rename, response.length())
    }
    fn on_close_complete(&mut self, response: &DeviceCloseResponse) -> PduResult<()> {
        self.ack(&response.device_io_response, Kind::Close, 0)
    }
    unused_completions! {
        on_read_complete(DeviceReadResponse),
        on_device_control_complete(DeviceControlResponse),
        on_query_directory_complete(ClientDriveQueryDirectoryResponse),
        on_notify_change_directory_complete(ClientDriveNotifyChangeDirectoryResponse),
        on_query_volume_information_complete(ClientDriveQueryVolumeInformationResponse),
        on_lock_control_complete(ClientDriveLockControlResponse),
    }
    fn on_query_information_complete(&mut self, response: &ClientDriveQueryInformationResponse) -> PduResult<()> {
        self.ack(&response.device_io_response, Kind::Unexpected, 0)
    }
    fn on_query_security_complete(&mut self, response: &ClientDriveQuerySecurityResponse) -> PduResult<()> {
        self.ack(&response.device_io_response, Kind::Unexpected, 0)
    }
    fn on_set_security_complete(&mut self, response: &ClientDriveSetSecurityResponse) -> PduResult<()> {
        self.ack(&response.device_io_response, Kind::Unexpected, 0)
    }
}
