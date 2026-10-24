use std::collections::{BTreeSet, VecDeque};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::thread;

use anyhow::{Context as _, anyhow, bail, ensure};
use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    Capabilities, ClipboardFormat, ClipboardGeneralCapabilityFlags, ClipboardPdu, ClipboardProtocolVersion,
    FileContentsRequest, FileContentsResponse, FileDescriptor, FormatDataRequest, FormatDataResponse, FormatList,
    FormatListResponse, LockDataId, PackedFileList,
};
use ironrdp_cliprdr_native::WinClipboard;
use ironrdp_core::{Decode as _, Encode as _, IntoOwned as _, ReadCursor, WriteCursor, encode_vec, impl_as_any};
use ironrdp_server::{CliprdrServerFactory, ServerEvent, ServerEventSender};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{Duration, timeout};
use tracing::{info, warn};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostThreadMessageW, TranslateMessage, WM_QUIT,
};

const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;
const QUEUE_SIZE: usize = 8;
const MAX_PENDING_FILE_REQUESTS: usize = 64;
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
// Zero-prefixed frames carry backend-only messages, never RDP PDUs.
const CALLBACK_FILE_LIST: [u8; 2] = [0, 1];
const MESSAGE_FILE_LIST: [u8; 2] = [0, 2];
const CALLBACK_LOCKS_CLEARED: [u8; 2] = [0, 3];

pub(crate) fn enabled() -> anyhow::Result<bool> {
    match std::env::var("IRONRDP_WTS_CLIPBOARD") {
        Err(std::env::VarError::NotPresent) => Ok(false),
        Ok(value) if value == "1" => Ok(true),
        Ok(value) if value == "0" => Ok(false),
        _ => bail!("IRONRDP_WTS_CLIPBOARD must be 0 or 1"),
    }
}

pub(crate) struct ClipboardFactory {
    tx: mpsc::Sender<Vec<u8>>,
    state: Arc<ClipboardState>,
}

pub(crate) struct ClipboardBridge {
    rx: mpsc::Receiver<Vec<u8>>,
    tx: mpsc::WeakSender<Vec<u8>>,
    state: Arc<ClipboardState>,
}

#[derive(Debug)]
struct ClipboardState {
    failed: watch::Sender<bool>,
    pending: StdMutex<VecDeque<FormatDataRequest>>,
    pending_files: StdMutex<BTreeSet<u32>>,
    sender: OnceLock<mpsc::UnboundedSender<ServerEvent>>,
}

impl ClipboardState {
    fn fail_pending(&self) {
        let count = core::mem::take(&mut *self.pending.lock().expect("clipboard format requests poisoned")).len();
        let files = core::mem::take(&mut *self.pending_files.lock().expect("clipboard file requests poisoned"));
        let responses = core::iter::repeat_with(|| ClipboardMessage::SendFormatData(FormatDataResponse::new_error()))
            .take(count)
            .chain(files.into_iter().map(|stream_id| {
                ClipboardMessage::SendFileContentsResponse(FileContentsResponse::new_error(stream_id))
            }));
        for response in responses {
            if let Some(sender) = self.sender.get()
                && sender.send(ServerEvent::Clipboard(response)).is_err()
            {
                info!("Clipboard request failed after its session closed");
                break;
            }
        }
    }
}

impl ClipboardFactory {
    pub(crate) fn new() -> (Self, ClipboardBridge) {
        let (tx, rx) = mpsc::channel(QUEUE_SIZE);
        let weak_tx = tx.downgrade();
        let (failed, _) = watch::channel(false);
        let state = Arc::new(ClipboardState {
            failed,
            pending: StdMutex::new(VecDeque::new()),
            pending_files: StdMutex::new(BTreeSet::new()),
            sender: OnceLock::new(),
        });
        (
            Self {
                tx,
                state: Arc::clone(&state),
            },
            ClipboardBridge { rx, tx: weak_tx, state },
        )
    }
}

impl ServerEventSender for ClipboardFactory {
    fn set_sender(&mut self, sender: mpsc::UnboundedSender<ServerEvent>) {
        if self.state.sender.set(sender).is_err() {
            warn!("Clipboard factory event sender was already configured");
        }
    }
}

impl CliprdrBackendFactory for ClipboardFactory {
    fn build_cliprdr_backend(&self) -> Box<dyn CliprdrBackend> {
        Box::new(RemoteClipboardBackend {
            tx: self.tx.clone(),
            state: Arc::clone(&self.state),
        })
    }
}

impl CliprdrServerFactory for ClipboardFactory {}

#[derive(Debug)]
struct RemoteClipboardBackend {
    tx: mpsc::Sender<Vec<u8>>,
    state: Arc<ClipboardState>,
}

impl_as_any!(RemoteClipboardBackend);

impl RemoteClipboardBackend {
    fn send(&self, message: anyhow::Result<Vec<u8>>) -> bool {
        let result = message.and_then(|data| {
            ensure!(!*self.state.failed.borrow(), "clipboard helper is unavailable");
            self.tx
                .try_send(data)
                .map_err(|_| anyhow!("clipboard helper queue is full or closed"))
        });
        if let Err(error) = result {
            warn!(%error, "Failed to forward clipboard callback");
            self.state.failed.send_replace(true);
            return false;
        }
        true
    }
}

impl CliprdrBackend for RemoteClipboardBackend {
    fn temporary_directory(&self) -> &str {
        ""
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
            | ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED
    }

    fn on_ready(&mut self) {
        self.send(encode_pdu(ClipboardPdu::MonitorReady));
    }

    fn on_request_format_list(&mut self) {
        // An empty IPC frame requests the local format list; it is never an RDP PDU.
        self.send(Ok(Vec::new()));
    }

    fn on_format_list_response(&mut self, ok: bool) {
        self.send(encode_pdu(ClipboardPdu::FormatListResponse(if ok {
            FormatListResponse::Ok
        } else {
            FormatListResponse::Fail
        })));
    }

    fn on_process_negotiated_capabilities(&mut self, capabilities: ClipboardGeneralCapabilityFlags) {
        self.send(encode_pdu(ClipboardPdu::Capabilities(Capabilities::new(
            ClipboardProtocolVersion::V2,
            capabilities,
        ))));
    }

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        self.send(
            FormatList::new_unicode(available_formats, true)
                .map_err(anyhow::Error::from)
                .and_then(|formats| encode_pdu(ClipboardPdu::FormatList(formats))),
        );
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        let queued = (|| {
            let mut pending = self.state.pending.lock().expect("clipboard format requests poisoned");
            pending.push_back(request.clone());
            ensure!(!*self.state.failed.borrow(), "clipboard helper is unavailable");
            ensure!(
                pending.len() <= QUEUE_SIZE,
                "clipboard helper has too many pending format requests"
            );
            Ok(pending.len() == 1)
        })();
        let sent = match queued {
            Ok(false) => return,
            Ok(true) => self.send(encode_pdu(ClipboardPdu::FormatDataRequest(request))),
            Err(error) => self.send(Err(error)),
        };
        if !sent {
            self.state.fail_pending();
        }
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        self.send(encode_pdu(ClipboardPdu::FormatDataResponse(response)));
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        let registered = {
            let mut pending = self
                .state
                .pending_files
                .lock()
                .expect("clipboard file requests poisoned");
            if !pending.insert(request.stream_id) {
                Err(anyhow!("clipboard file stream ID is already pending"))
            } else if pending.len() <= MAX_PENDING_FILE_REQUESTS {
                Ok(())
            } else {
                Err(anyhow!("clipboard helper has too many pending file requests"))
            }
        };
        if !self.send(registered.and_then(|()| encode_pdu(ClipboardPdu::FileContentsRequest(request)))) {
            self.state.fail_pending();
        }
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        self.send(encode_pdu(ClipboardPdu::FileContentsResponse(response)));
    }

    fn on_lock(&mut self, data_id: LockDataId) {
        self.send(encode_pdu(ClipboardPdu::LockData(data_id)));
    }

    fn on_unlock(&mut self, data_id: LockDataId) {
        self.send(encode_pdu(ClipboardPdu::UnlockData(data_id)));
    }

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        let mut prefix = CALLBACK_FILE_LIST.to_vec();
        prefix.push(u8::from(clip_data_id.is_some()));
        prefix.extend_from_slice(&clip_data_id.unwrap_or(0).to_le_bytes());
        self.send(encode_files(&prefix, files));
    }

    fn on_outgoing_locks_cleared(&mut self, clip_data_ids: &[LockDataId]) {
        self.send((|| {
            ensure!(
                clip_data_ids.len() <= (MAX_FRAME_SIZE - CALLBACK_LOCKS_CLEARED.len()) / 4, /* clipDataId */
                "clipboard lock list exceeds 16 MiB"
            );
            let mut frame = CALLBACK_LOCKS_CLEARED.to_vec();
            for data_id in clip_data_ids {
                frame.extend_from_slice(&data_id.0.to_le_bytes());
            }
            Ok(frame)
        })());
    }
}

impl ClipboardBridge {
    pub(crate) async fn run(
        mut self,
        stream: TcpStream,
        sender: mpsc::UnboundedSender<ServerEvent>,
    ) -> anyhow::Result<()> {
        let (mut reader, mut writer) = stream.into_split();
        let mut failed = self.state.failed.subscribe();
        ensure!(!*failed.borrow(), "clipboard bridge failed before helper startup");
        let result = tokio::select! {
            result = async {
                loop {
                    let message = decode_message(&read_frame(&mut reader).await?)?;
                    let next_request = if matches!(&message, ClipboardMessage::SendFormatData(_)) {
                        let mut pending = self.state.pending.lock().expect("clipboard format requests poisoned");
                        pending.pop_front().context("unsolicited clipboard helper response")?;
                        pending.front().cloned()
                    } else {
                        None
                    };
                    if let ClipboardMessage::SendFileContentsResponse(response) = &message {
                        ensure!(
                            self.state.pending_files.lock().expect("clipboard file requests poisoned")
                                .remove(&response.stream_id()),
                            "unsolicited clipboard helper file response"
                        );
                    }
                    let (completion, processed) = oneshot::channel();
                    sender.send(ServerEvent::ClipboardWithCompletion(message, completion))
                        .map_err(|_| anyhow!("clipboard session event receiver closed"))?;
                    processed.await.context("clipboard server dispatch ended before completion")?;
                    if let Some(request) = next_request {
                        self.tx.upgrade().context("clipboard callback queue closed")?
                            .try_send(encode_pdu(ClipboardPdu::FormatDataRequest(request))?)
                            .map_err(|_| anyhow!("clipboard helper queue is full or closed"))?;
                    }
                }
            } => result,
            result = async {
                while let Some(frame) = self.rx.recv().await {
                    timeout(WRITE_TIMEOUT, write_frame(&mut writer, &frame))
                        .await.context("clipboard helper write timed out")??;
                }
                Ok(())
            } => result,
            _ = failed.changed() => Err(anyhow!("clipboard callback bridge failed")),
        };
        self.state.failed.send_replace(true);
        self.state.fail_pending();
        if sender
            .send(ServerEvent::Clipboard(ClipboardMessage::SendInitiateCopy(Vec::new())))
            .is_err()
        {
            info!("Clipboard bridge closed with its session");
        }
        result
    }
}

impl Drop for ClipboardBridge {
    fn drop(&mut self) {
        self.state.failed.send_replace(true);
        self.state.fail_pending();
    }
}

fn encode_pdu(pdu: ClipboardPdu<'_>) -> anyhow::Result<Vec<u8>> {
    ensure!(pdu.size() <= MAX_FRAME_SIZE, "clipboard frame exceeds 16 MiB");
    encode_vec(&pdu).context("encode clipboard helper frame")
}

fn decode_pdu(frame: &[u8]) -> anyhow::Result<ClipboardPdu<'_>> {
    ensure!(
        8 <= frame.len() && frame.len() <= MAX_FRAME_SIZE,
        "invalid clipboard frame size"
    );
    let declared = u32::from_le_bytes(frame[4..8].try_into().context("read clipboard length")?);
    ensure!(
        usize::try_from(declared)? == frame.len() - 8,
        "clipboard frame payload length mismatch"
    );
    let mut cursor = ReadCursor::new(frame);
    let pdu = ClipboardPdu::decode(&mut cursor).context("decode clipboard helper frame")?;
    ensure!(cursor.is_empty(), "trailing clipboard helper data");
    Ok(pdu)
}

fn decode_message(frame: &[u8]) -> anyhow::Result<ClipboardMessage> {
    if let Some(payload) = frame.strip_prefix(&MESSAGE_FILE_LIST) {
        return Ok(ClipboardMessage::SendInitiateFileCopy(decode_files(payload)?));
    }
    match decode_pdu(frame)? {
        ClipboardPdu::FormatList(formats) => Ok(ClipboardMessage::SendInitiateCopy(formats.get_formats(true)?)),
        ClipboardPdu::FormatDataRequest(request) => Ok(ClipboardMessage::SendInitiatePaste(request.format)),
        ClipboardPdu::FormatDataResponse(response) => Ok(ClipboardMessage::SendFormatData(response.into_owned())),
        ClipboardPdu::FileContentsRequest(request) => Ok(ClipboardMessage::SendFileContentsRequest(request)),
        ClipboardPdu::FileContentsResponse(response) => {
            Ok(ClipboardMessage::SendFileContentsResponse(response.into_owned()))
        }
        _ => bail!("unsupported clipboard helper message"),
    }
}

fn encode_files(prefix: &[u8], files: &[FileDescriptor]) -> anyhow::Result<Vec<u8>> {
    let list = PackedFileList { files: files.to_vec() };
    ensure!(
        list.size() <= MAX_FRAME_SIZE.saturating_sub(prefix.len()),
        "clipboard file list exceeds 16 MiB"
    );
    let mut frame = vec![0; prefix.len() + list.size()];
    frame[..prefix.len()].copy_from_slice(prefix);
    list.encode(&mut WriteCursor::new(&mut frame[prefix.len()..]))?;
    Ok(frame)
}

fn decode_files(frame: &[u8]) -> anyhow::Result<Vec<FileDescriptor>> {
    ensure!(frame.len() <= MAX_FRAME_SIZE, "clipboard file list exceeds 16 MiB");
    let mut cursor = ReadCursor::new(frame);
    let mut list = PackedFileList::decode(&mut cursor)?;
    ensure!(cursor.is_empty(), "trailing clipboard file list data");
    for file in &mut list.files {
        if let Some((parent, name)) = file.name.rsplit_once('\\') {
            file.relative_path = Some(parent.to_owned());
            file.name = name.to_owned();
        }
    }
    Ok(list.files)
}

fn dispatch_callback(backend: &mut dyn CliprdrBackend, frame: &[u8]) -> anyhow::Result<()> {
    if frame.is_empty() {
        backend.on_request_format_list();
        return Ok(());
    }
    if let Some(payload) = frame.strip_prefix(&CALLBACK_FILE_LIST) {
        ensure!(5 <= payload.len(), "truncated clipboard file list callback");
        let clip_data_id = match payload[0] {
            0 => None,
            1 => Some(u32::from_le_bytes(payload[1..5].try_into()?)),
            _ => bail!("invalid clipboard file list lock flag"),
        };
        backend.on_remote_file_list(&decode_files(&payload[5..])?, clip_data_id);
        return Ok(());
    }
    if let Some(payload) = frame.strip_prefix(&CALLBACK_LOCKS_CLEARED) {
        ensure!(
            frame.len() <= MAX_FRAME_SIZE && payload.len().is_multiple_of(4 /* clipDataId */),
            "invalid clipboard lock list"
        );
        let data_ids = payload
            .chunks_exact(4 /* clipDataId */)
            .map(|chunk| Ok(LockDataId(u32::from_le_bytes(chunk.try_into()?))))
            .collect::<anyhow::Result<Vec<_>>>()?;
        backend.on_outgoing_locks_cleared(&data_ids);
        return Ok(());
    }
    match decode_pdu(frame)? {
        ClipboardPdu::MonitorReady => backend.on_ready(),
        ClipboardPdu::Capabilities(caps) => backend.on_process_negotiated_capabilities(caps.flags()),
        ClipboardPdu::FormatList(formats) => backend.on_remote_copy(&formats.get_formats(true)?),
        ClipboardPdu::FormatListResponse(response) => {
            backend.on_format_list_response(response == FormatListResponse::Ok);
        }
        ClipboardPdu::FormatDataRequest(request) => backend.on_format_data_request(request),
        ClipboardPdu::FormatDataResponse(response) => backend.on_format_data_response(response),
        ClipboardPdu::FileContentsRequest(request) => backend.on_file_contents_request(request),
        ClipboardPdu::FileContentsResponse(response) => backend.on_file_contents_response(response),
        ClipboardPdu::LockData(data_id) => backend.on_lock(data_id),
        ClipboardPdu::UnlockData(data_id) => backend.on_unlock(data_id),
        _ => bail!("unsupported clipboard backend callback"),
    }
    Ok(())
}

async fn read_frame(reader: &mut (dyn AsyncRead + Unpin + Send)) -> anyhow::Result<Vec<u8>> {
    let length = usize::try_from(reader.read_u32_le().await.context("read clipboard frame length")?)?;
    ensure!(length <= MAX_FRAME_SIZE, "clipboard frame exceeds 16 MiB");
    let mut frame = vec![0u8; length];
    reader.read_exact(&mut frame).await.context("read clipboard frame")?;
    Ok(frame)
}

async fn write_frame(writer: &mut (dyn AsyncWrite + Unpin + Send), frame: &[u8]) -> anyhow::Result<()> {
    ensure!(frame.len() <= MAX_FRAME_SIZE, "clipboard frame exceeds 16 MiB");
    writer.write_u32_le(u32::try_from(frame.len())?).await?;
    writer.write_all(frame).await?;
    Ok(())
}

#[derive(Debug)]
struct NativeClipboardProxy {
    tx: mpsc::Sender<ClipboardMessage>,
    failed: watch::Sender<bool>,
}

impl ClipboardMessageProxy for NativeClipboardProxy {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        if self.tx.try_send(message).is_err() {
            warn!("Native clipboard event queue is full or closed");
            self.failed.send_replace(true);
        }
    }
}

pub(crate) async fn run_helper(stream: TcpStream) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::channel(QUEUE_SIZE);
    let (failed_tx, mut failed_rx) = watch::channel(false);
    let (ready_tx, ready_rx) = oneshot::channel();
    let gui = thread::Builder::new()
        .name("ironrdp-session-clipboard".to_owned())
        .spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                // SAFETY: This new thread owns its COM apartment until CoUninitialize.
                unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok() }
                    .context("initialize clipboard COM apartment")?;
                struct Apartment;
                impl Drop for Apartment {
                    fn drop(&mut self) {
                        // SAFETY: Balances CoInitializeEx on the same thread.
                        unsafe { CoUninitialize() };
                    }
                }
                let _apartment = Apartment;
                let clipboard = WinClipboard::with_file_transfer(NativeClipboardProxy { tx, failed: failed_tx }, true)
                    .context("initialize session clipboard")?;
                // SAFETY: GetCurrentThreadId takes no arguments.
                let thread_id = unsafe { GetCurrentThreadId() };
                if ready_tx.send((clipboard.backend_factory(), thread_id)).is_err() {
                    return Ok(());
                }
                let mut message = MSG::default();
                loop {
                    // SAFETY: message is writable and the current thread owns this message loop.
                    let status = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
                    match status {
                        -1 => return Err(anyhow!("clipboard message loop failed")),
                        0 => return Ok(()),
                        _ => {
                            // SAFETY: GetMessageW initialized this message.
                            let _ = unsafe { TranslateMessage(&message) };
                            // SAFETY: GetMessageW initialized this message.
                            unsafe { DispatchMessageW(&message) };
                        }
                    }
                }
            })();
            if let Err(error) = &result {
                warn!(%error, "Session clipboard window stopped");
            }
            result
        })
        .context("start session clipboard thread")?;
    let (factory, thread_id) = ready_rx.await.context("session clipboard initialization failed")?;
    let mut backend = factory.build_cliprdr_backend();
    drop(factory);
    let (mut reader, mut writer) = stream.into_split();
    info!("Session clipboard helper ready");
    let result = tokio::select! {
        result = async {
            loop {
                let frame = read_frame(&mut reader).await?;
                dispatch_callback(backend.as_mut(), &frame)?;
            }
        } => result,
        result = async {
            while let Some(message) = rx.recv().await {
                let frame = match message {
                    ClipboardMessage::SendInitiateCopy(formats) => {
                        encode_pdu(ClipboardPdu::FormatList(FormatList::new_unicode(&formats, true)?))?
                    }
                    ClipboardMessage::SendInitiateFileCopy(files) => encode_files(&MESSAGE_FILE_LIST, &files)?,
                    ClipboardMessage::SendFileContentsRequest(request) => {
                        encode_pdu(ClipboardPdu::FileContentsRequest(request))?
                    }
                    ClipboardMessage::SendFileContentsResponse(response) => {
                        let stream_id = response.stream_id();
                        match encode_pdu(ClipboardPdu::FileContentsResponse(response)) {
                            Ok(frame) => frame,
                            Err(error) => {
                                warn!(%error, stream_id, "Clipboard file data cannot be transferred");
                                encode_pdu(ClipboardPdu::FileContentsResponse(FileContentsResponse::new_error(stream_id)))?
                            }
                        }
                    }
                    ClipboardMessage::SendInitiatePaste(format) => {
                        encode_pdu(ClipboardPdu::FormatDataRequest(FormatDataRequest { format }))?
                    }
                    ClipboardMessage::SendFormatData(response) => {
                        match encode_pdu(ClipboardPdu::FormatDataResponse(response)) {
                            Ok(frame) => frame,
                            Err(error) => {
                                warn!(%error, "Clipboard data cannot be transferred");
                                encode_pdu(ClipboardPdu::FormatDataResponse(FormatDataResponse::new_error()))?
                            }
                        }
                    }
                    ClipboardMessage::Error(error) => {
                        bail!("native clipboard backend failed: {error}");
                    }
                };
                timeout(WRITE_TIMEOUT, write_frame(&mut writer, &frame))
                    .await.context("clipboard service write timed out")??;
            }
            Ok(())
        } => result,
        _ = failed_rx.changed() => Err(anyhow!("native clipboard event queue failed")),
    };
    drop(backend);
    // SAFETY: The clipboard thread created a message queue and owns its window.
    unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }.context("stop session clipboard thread")?;
    tokio::task::spawn_blocking(move || gui.join())
        .await
        .context("join clipboard thread task")?
        .map_err(|_| anyhow!("clipboard window thread panicked"))??;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // oracle: spec MS-RDPECLIP §2.2.5.1
    #[test]
    fn helper_request_preserves_unicode_format_id() {
        let request = [4, 0, 0, 0, 4, 0, 0, 0, 13, 0, 0, 0];
        match decode_message(&request).unwrap() {
            ClipboardMessage::SendInitiatePaste(format) => assert_eq!(format.value(), 13),
            _ => panic!("expected a format request"),
        }
    }

    // oracle: spec MS-RDPECLIP §3.1.5.4.4
    #[test]
    fn helper_response_preserves_binary_data() {
        let response = [5, 0, 1, 0, 4, 0, 0, 0, 0, 128, 255, 17];
        match decode_message(&response).unwrap() {
            ClipboardMessage::SendFormatData(data) => {
                assert!(!data.is_error());
                assert_eq!(data.data(), [0, 128, 255, 17]);
            }
            _ => panic!("expected format data"),
        }
    }

    // oracle: spec MS-RDPECLIP §3.1.5.4.3
    #[test]
    fn helper_failure_does_not_become_successful_empty_data() {
        let response = [5, 0, 2, 0, 0, 0, 0, 0];
        match decode_message(&response).unwrap() {
            ClipboardMessage::SendFormatData(data) => {
                assert!(data.is_error());
                assert!(data.data().is_empty());
            }
            _ => panic!("expected failed format data"),
        }
    }

    // oracle: mutant prevent malformed helper lengths from reaching the clipboard backend
    #[test]
    fn helper_rejects_trailing_and_truncated_data() {
        assert!(decode_message(&[5, 0, 1, 0, 4, 0, 0, 0, 1, 2]).is_err());
        assert!(decode_message(&[5, 0, 1, 0, 0, 0, 0, 0, 1]).is_err());
        assert!(decode_message(&[]).is_err());
    }

    // oracle: mutant prevent oversized helper lengths from allocating their declared body
    #[tokio::test]
    async fn helper_rejects_oversized_frame_before_reading_body() {
        let mut header = &[1, 0, 0, 1][..];
        assert!(
            read_frame(&mut header)
                .await
                .unwrap_err()
                .to_string()
                .contains("16 MiB")
        );
    }

    // oracle: spec crates/ironrdp-termsrv/README.md section Clipboard redirection
    #[tokio::test]
    async fn helper_accepts_a_frame_at_the_16_mib_limit() {
        let mut wire = vec![0x5a; 4 + 16 * 1024 * 1024];
        wire[..4].copy_from_slice(&[0, 0, 0, 1]);
        let frame = read_frame(&mut wire.as_slice()).await.unwrap();
        assert_eq!(frame.len(), 16 * 1024 * 1024);
        assert!(frame.iter().all(|byte| *byte == 0x5a));
    }

    // oracle: mutant prevent callbacks for one RDP connection reaching another user's helper
    #[test]
    fn factories_have_separate_callback_queues() {
        let (first, mut first_bridge) = ClipboardFactory::new();
        let (_second, mut second_bridge) = ClipboardFactory::new();
        first.build_cliprdr_backend().on_request_format_list();
        assert_eq!(first_bridge.rx.try_recv().unwrap(), Vec::<u8>::new());
        assert!(matches!(
            second_bridge.rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    // oracle: spec MS-RDPECLIP §3.1.5.4.3
    #[test]
    fn unavailable_helper_returns_format_failure() {
        let (mut factory, bridge) = ClipboardFactory::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        factory.set_sender(tx);
        drop(bridge);
        factory
            .build_cliprdr_backend()
            .on_format_data_request(FormatDataRequest {
                format: ironrdp_cliprdr::pdu::ClipboardFormatId::CF_UNICODETEXT,
            });
        match rx.try_recv().unwrap() {
            ServerEvent::Clipboard(ClipboardMessage::SendFormatData(data)) => assert!(data.is_error()),
            _ => panic!("expected a clipboard failure response"),
        }
    }

    // oracle: spec MS-RDPECLIP §3.1.5.4.3
    #[test]
    fn helper_shutdown_fails_already_accepted_requests() {
        let (mut factory, bridge) = ClipboardFactory::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        factory.set_sender(tx);
        factory
            .build_cliprdr_backend()
            .on_format_data_request(FormatDataRequest {
                format: ironrdp_cliprdr::pdu::ClipboardFormatId::CF_UNICODETEXT,
            });
        assert!(rx.try_recv().is_err());
        drop(bridge);
        match rx.try_recv().unwrap() {
            ServerEvent::Clipboard(ClipboardMessage::SendFormatData(data)) => assert!(data.is_error()),
            _ => panic!("expected a clipboard failure response"),
        }
        assert!(rx.try_recv().is_err());
    }

    // oracle: spec MS-RDPECLIP sections 2.2.5.4 and 3.1.5.4.7
    #[test]
    fn helper_shutdown_fails_pending_file_request_by_stream_id() {
        let (mut factory, bridge) = ClipboardFactory::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        factory.set_sender(tx);
        factory
            .build_cliprdr_backend()
            .on_file_contents_request(FileContentsRequest {
                stream_id: 42,
                index: 0,
                flags: ironrdp_cliprdr::pdu::FileContentsFlags::SIZE,
                position: 0,
                requested_size: 8,
                data_id: Some(7),
            });
        assert!(rx.try_recv().is_err());
        drop(bridge);
        match rx.try_recv().unwrap() {
            ServerEvent::Clipboard(ClipboardMessage::SendFileContentsResponse(response)) => {
                assert!(response.is_error());
                assert_eq!(response.stream_id(), 42);
            }
            _ => panic!("expected a failed file response"),
        }
        assert!(rx.try_recv().is_err());
    }

    // oracle: mutant prevent concurrent requests from aliasing the same helper stream
    #[test]
    fn duplicate_pending_file_stream_fails_without_forwarding_it_twice() {
        let (mut factory, mut bridge) = ClipboardFactory::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        factory.set_sender(tx);
        let mut backend = factory.build_cliprdr_backend();
        let request = FileContentsRequest {
            stream_id: 42,
            index: 0,
            flags: ironrdp_cliprdr::pdu::FileContentsFlags::SIZE,
            position: 0,
            requested_size: 8,
            data_id: None,
        };
        backend.on_file_contents_request(request.clone());
        bridge.rx.try_recv().unwrap();
        backend.on_file_contents_request(request);
        assert!(bridge.rx.try_recv().is_err());
        match rx.try_recv().unwrap() {
            ServerEvent::Clipboard(ClipboardMessage::SendFileContentsResponse(response)) => {
                assert!(response.is_error());
                assert_eq!(response.stream_id(), 42);
            }
            _ => panic!("expected a failed file response"),
        }
        drop(bridge);
        assert!(rx.try_recv().is_err());
    }

    // oracle: spec MS-RDPECLIP sections 2.2.5.4 and 3.1.5.4.8
    #[tokio::test]
    async fn helper_file_response_retires_its_request() {
        let (mut factory, bridge) = ClipboardFactory::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        factory.set_sender(tx.clone());
        factory
            .build_cliprdr_backend()
            .on_file_contents_request(FileContentsRequest {
                stream_id: 42,
                index: 0,
                flags: ironrdp_cliprdr::pdu::FileContentsFlags::RANGE,
                position: 0,
                requested_size: 4,
                data_id: None,
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut helper = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let task = tokio::spawn(bridge.run(stream, tx));
        let response = [9, 0, 1, 0, 8, 0, 0, 0, 42, 0, 0, 0, 0, 128, 255, 17];
        write_frame(&mut helper, &response).await.unwrap();
        match timeout(WRITE_TIMEOUT, rx.recv()).await.unwrap().unwrap() {
            ServerEvent::ClipboardWithCompletion(ClipboardMessage::SendFileContentsResponse(response), completion) => {
                assert!(!response.is_error());
                assert_eq!(response.stream_id(), 42);
                assert_eq!(response.data(), [0, 128, 255, 17]);
                completion.send(()).unwrap();
            }
            _ => panic!("expected a file response"),
        }
        drop(helper);
        assert!(timeout(WRITE_TIMEOUT, task).await.unwrap().unwrap().is_err());
        while let Ok(event) = rx.try_recv() {
            assert!(!matches!(
                event,
                ServerEvent::Clipboard(ClipboardMessage::SendFileContentsResponse(_))
            ));
        }
    }

    // oracle: spec MS-RDPECLIP sections 3.1.5.4.2 and 3.1.5.4.3
    #[tokio::test]
    async fn helper_serializes_format_requests_until_their_response() {
        let (mut factory, bridge) = ClipboardFactory::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        factory.set_sender(tx.clone());
        let mut backend = factory.build_cliprdr_backend();
        backend.on_format_data_request(FormatDataRequest {
            format: ironrdp_cliprdr::pdu::ClipboardFormatId::CF_UNICODETEXT,
        });
        backend.on_format_data_request(FormatDataRequest {
            format: ironrdp_cliprdr::pdu::ClipboardFormatId::CF_DIB,
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut helper = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let task = tokio::spawn(bridge.run(stream, tx));

        let first = timeout(WRITE_TIMEOUT, read_frame(&mut helper)).await.unwrap().unwrap();
        assert!(matches!(
            decode_message(&first).unwrap(),
            ClipboardMessage::SendInitiatePaste(format) if format.value() == 13
        ));
        assert!(
            timeout(Duration::from_millis(50), read_frame(&mut helper))
                .await
                .is_err(),
            "another native request could overtake the first request's retry"
        );

        write_frame(&mut helper, &[5, 0, 2, 0, 0, 0, 0, 0]).await.unwrap();
        match timeout(WRITE_TIMEOUT, rx.recv()).await.unwrap().unwrap() {
            ServerEvent::ClipboardWithCompletion(ClipboardMessage::SendFormatData(response), completion) => {
                assert!(response.is_error());
                completion.send(()).unwrap();
            }
            _ => panic!("expected the first format's failure"),
        }
        let next = timeout(WRITE_TIMEOUT, read_frame(&mut helper)).await.unwrap().unwrap();
        assert!(matches!(
            decode_message(&next).unwrap(),
            ClipboardMessage::SendInitiatePaste(format) if format.value() == 8
        ));
        drop(helper);
        assert!(timeout(WRITE_TIMEOUT, task).await.unwrap().unwrap().is_err());
    }

    // oracle: mutant forwarding another helper frame before the previous server dispatch completes
    #[tokio::test]
    async fn helper_waits_for_server_dispatch_before_forwarding_another_frame() {
        let (_factory, bridge) = ClipboardFactory::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut helper = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (stream, _) = listener.accept().await.unwrap();
        let task = tokio::spawn(bridge.run(stream, tx));
        let format_list = [2, 0, 0, 0, 0, 0, 0, 0];
        write_frame(&mut helper, &format_list).await.unwrap();
        write_frame(&mut helper, &format_list).await.unwrap();

        let in_dispatch = timeout(WRITE_TIMEOUT, rx.recv()).await.unwrap().unwrap();
        assert!(
            timeout(Duration::from_millis(50), rx.recv()).await.is_err(),
            "dequeueing is not completion: the server may still be blocked writing the first event"
        );
        let ServerEvent::ClipboardWithCompletion(_, completion) = in_dispatch else {
            panic!("expected dispatch completion");
        };
        completion.send(()).unwrap();
        let next = timeout(WRITE_TIMEOUT, rx.recv()).await.unwrap().unwrap();
        drop(rx);
        drop(next);
        drop(helper);
        assert!(timeout(WRITE_TIMEOUT, task).await.unwrap().unwrap().is_err());
    }

    // oracle: spec MS-RDPECLIP section 2.2.5.3
    #[test]
    fn helper_file_request_preserves_range_and_lock() {
        let frame = [
            8, 0, 0, 0, 28, 0, 0, 0, 42, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 32, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 7, 0, 0,
            0,
        ];
        match decode_message(&frame).unwrap() {
            ClipboardMessage::SendFileContentsRequest(request) => {
                assert_eq!(request.stream_id, 42);
                assert_eq!(request.index, 1);
                assert_eq!(request.flags, ironrdp_cliprdr::pdu::FileContentsFlags::RANGE);
                assert_eq!(request.position, 32);
                assert_eq!(request.requested_size, 4);
                assert_eq!(request.data_id, Some(7));
            }
            _ => panic!("expected a file range request"),
        }
    }

    // oracle: mutant prevent dropping file metadata and its lock while crossing the helper boundary
    #[test]
    fn remote_file_list_callback_survives_the_helper_boundary() {
        let (first, mut first_bridge) = ClipboardFactory::new();
        let (second, mut second_bridge) = ClipboardFactory::new();
        let files = [FileDescriptor::new("document.txt")
            .with_relative_path("folder")
            .with_file_size(0x1_0000_0001)];
        first
            .build_cliprdr_backend()
            .on_remote_file_list(&files, Some(u32::MAX));
        let frame = first_bridge.rx.try_recv().unwrap();
        dispatch_callback(second.build_cliprdr_backend().as_mut(), &frame).unwrap();
        assert_eq!(second_bridge.rx.try_recv().unwrap(), frame);
    }

    // oracle: mutant preserve file callbacks, including failures and locks, across the helper boundary
    #[test]
    fn file_callbacks_survive_the_helper_boundary() {
        let (factory, mut bridge) = ClipboardFactory::new();
        let mut backend = factory.build_cliprdr_backend();
        let callbacks = [
            ClipboardPdu::FileContentsRequest(FileContentsRequest {
                stream_id: 42,
                index: 0,
                flags: ironrdp_cliprdr::pdu::FileContentsFlags::SIZE,
                position: 0,
                requested_size: 8,
                data_id: Some(7),
            }),
            ClipboardPdu::FileContentsResponse(FileContentsResponse::new_size_response(42, 0x1_0000_0001)),
            ClipboardPdu::FileContentsResponse(FileContentsResponse::new_error(43)),
            ClipboardPdu::LockData(LockDataId(7)),
            ClipboardPdu::UnlockData(LockDataId(7)),
        ];
        for callback in callbacks {
            let name = callback.message_name();
            let frame = encode_pdu(callback.clone()).unwrap();
            dispatch_callback(backend.as_mut(), &frame).expect(name);
            assert_eq!(bridge.rx.try_recv().expect(name), frame);
        }
    }

    // oracle: mutant preserve descriptor fields when a native file copy reaches the server
    #[test]
    fn helper_file_copy_preserves_descriptor_fields() {
        let files = [FileDescriptor::new("document.txt")
            .with_relative_path("folder")
            .with_file_size(0x1_0000_0001)
            .with_last_write_time(0x1122_3344_5566_7788)];
        let frame = encode_files(&[0, 2], &files).unwrap();
        match decode_message(&frame).unwrap() {
            ClipboardMessage::SendInitiateFileCopy(actual) => assert_eq!(actual, files),
            _ => panic!("expected a native file copy"),
        }
    }

    // oracle: mutant stop accepting file requests when an unresponsive helper reaches the bound
    #[test]
    fn helper_file_requests_are_bounded_even_when_the_queue_is_drained() {
        let (mut factory, mut bridge) = ClipboardFactory::new();
        let (tx, mut rx) = mpsc::unbounded_channel();
        factory.set_sender(tx);
        let mut backend = factory.build_cliprdr_backend();
        for stream_id in 0..=64 {
            backend.on_file_contents_request(FileContentsRequest {
                stream_id,
                index: 0,
                flags: ironrdp_cliprdr::pdu::FileContentsFlags::SIZE,
                position: 0,
                requested_size: 8,
                data_id: None,
            });
            let _ = bridge.rx.try_recv();
        }
        assert!(*factory.state.failed.borrow());
        let mut failed = BTreeSet::new();
        for _ in 0..=64 {
            match rx.try_recv().unwrap() {
                ServerEvent::Clipboard(ClipboardMessage::SendFileContentsResponse(response)) => {
                    assert!(response.is_error());
                    assert!(failed.insert(response.stream_id()));
                }
                _ => panic!("expected a failed file response"),
            }
        }
        assert_eq!(failed, (0..=64).collect());
        assert!(rx.try_recv().is_err());
    }

    // oracle: mutant prevent helper streams from retaining locks after their owner clears them
    #[test]
    fn cleared_file_locks_cross_the_helper_boundary() {
        let (first, mut first_bridge) = ClipboardFactory::new();
        let (second, mut second_bridge) = ClipboardFactory::new();
        first
            .build_cliprdr_backend()
            .on_outgoing_locks_cleared(&[LockDataId(0), LockDataId(u32::MAX)]);
        let frame = first_bridge.rx.try_recv().unwrap();
        dispatch_callback(second.build_cliprdr_backend().as_mut(), &frame).unwrap();
        assert_eq!(second_bridge.rx.try_recv().unwrap(), frame);
    }

    // oracle: mutant reject truncated lock metadata before dispatching a file list
    #[test]
    fn file_list_callback_requires_a_complete_lock_header() {
        let (factory, mut bridge) = ClipboardFactory::new();
        assert!(dispatch_callback(factory.build_cliprdr_backend().as_mut(), &[0, 1, 1, 0, 0, 0]).is_err());
        assert!(bridge.rx.try_recv().is_err());
    }

    // oracle: mutant reject an unknown optional-lock flag instead of treating it as a lock
    #[test]
    fn file_list_callback_rejects_an_unknown_lock_flag() {
        let (factory, mut bridge) = ClipboardFactory::new();
        assert!(
            dispatch_callback(
                factory.build_cliprdr_backend().as_mut(),
                &[0, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0],
            )
            .is_err()
        );
        assert!(bridge.rx.try_recv().is_err());
    }

    // oracle: mutant reject a file offer with bytes beyond its declared descriptor count
    #[test]
    fn file_offer_rejects_trailing_descriptor_data() {
        assert!(decode_message(&[0, 2, 0, 0, 0, 0, 1]).is_err());
    }

    // oracle: mutant reject partial lock IDs instead of silently releasing an incomplete list
    #[test]
    fn cleared_locks_callback_rejects_a_partial_lock_id() {
        let (factory, mut bridge) = ClipboardFactory::new();
        assert!(dispatch_callback(factory.build_cliprdr_backend().as_mut(), &[0, 3, 1, 0, 0]).is_err());
        assert!(bridge.rx.try_recv().is_err());
    }
}
