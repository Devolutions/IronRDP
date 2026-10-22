//! Tests against the real Windows clipboard.
//!
//! A `CliprdrClient` backed by [`WinClipboard`] talks to an in-memory `CliprdrServer` playing the
//! remote, so both directions go through the actual OLE and Win32 clipboard, and remote files are
//! pasted by Explorer itself. They replace the clipboard contents and briefly open an Explorer
//! window, so they are ignored by default and do nothing unless `IRONRDP_WIN_CLIPBOARD_LIVE=1` is
//! set. Run them one at a time, as they share the clipboard:
//!
//! ```text
//! IRONRDP_WIN_CLIPBOARD_LIVE=1 cargo test -p ironrdp-testsuite-extra -- --ignored --test-threads=1 cliprdr_native::windows_live
//! ```
//!
//! They need `powershell.exe` (Windows PowerShell), which they use to set the clipboard and paste.

use core::ffi::c_void;
use core::time::Duration;
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Instant;

use ironrdp_cliprdr::backend::{ClipboardMessage, ClipboardMessageProxy, CliprdrBackend, CliprdrBackendFactory};
use ironrdp_cliprdr::pdu::{
    ClipboardFileAttributes, ClipboardFormat, ClipboardFormatId, ClipboardFormatName, ClipboardGeneralCapabilityFlags,
    FileContentsFlags, FileContentsRequest, FileContentsResponse, FileDescriptor, FormatDataRequest,
    FormatDataResponse, LockDataId, OwnedFormatDataResponse, PackedFileList,
};
use ironrdp_cliprdr::{CliprdrClient, CliprdrServer};
use ironrdp_cliprdr_native::WinClipboard;
use ironrdp_cliprdr_native::windows_file_list::{is_directory, is_file_list_format};
use ironrdp_core::{IntoOwned as _, impl_as_any};
use ironrdp_svc::{SvcMessage, SvcProcessor as _};
use windows::Win32::Foundation::{HGLOBAL, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{
    DVASPECT_CONTENT, FORMATETC, IStream, STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET, TYMED_HGLOBAL, TYMED_ISTREAM,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, GetClipboardOwner, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{OleGetClipboard, OleInitialize, OleUninitialize, ReleaseStgMedium};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EnumThreadWindows, GetClassNameA, GetMessageW, GetWindowThreadProcessId, MSG, PostMessageW,
    PostThreadMessageW, TranslateMessage, WM_ACTIVATEAPP, WM_QUIT,
};
use windows::core::{BOOL, w};

use super::TempDir;

const TIMEOUT: Duration = Duration::from_secs(20);

fn enabled() -> bool {
    std::env::var("IRONRDP_WIN_CLIPBOARD_LIVE").as_deref() == Ok("1")
}

/// A file the in-memory server offers, with generated contents.
#[derive(Debug, Clone)]
struct ServedFile {
    descriptor: FileDescriptor,
    seed: u64,
}

impl ServedFile {
    fn file(relative_path: Option<&str>, name: &str, size: u64, seed: u64) -> Self {
        let mut descriptor = FileDescriptor::new(name)
            .with_attributes(ClipboardFileAttributes::ARCHIVE)
            .with_file_size(size);
        if let Some(relative_path) = relative_path {
            descriptor = descriptor.with_relative_path(relative_path);
        }
        Self { descriptor, seed }
    }

    fn directory(relative_path: Option<&str>, name: &str) -> Self {
        let mut descriptor = FileDescriptor::new(name).with_attributes(ClipboardFileAttributes::DIRECTORY);
        if let Some(relative_path) = relative_path {
            descriptor = descriptor.with_relative_path(relative_path);
        }
        Self { descriptor, seed: 0 }
    }

    fn is_directory(&self) -> bool {
        is_directory(&self.descriptor)
    }

    fn size(&self) -> u64 {
        self.descriptor.file_size.unwrap_or(0)
    }

    fn bytes(&self, position: u64, len: u64) -> Vec<u8> {
        let end = position.saturating_add(len).min(self.size());
        (position..end).map(|position| pattern(self.seed, position)).collect()
    }

    fn local_path(&self, root: &Path) -> PathBuf {
        let mut path = root.to_path_buf();
        if let Some(relative_path) = &self.descriptor.relative_path {
            path.extend(relative_path.split('\\'));
        }
        path.join(&self.descriptor.name)
    }
}

fn pattern(seed: u64, position: u64) -> u8 {
    let value = position
        .wrapping_mul(31)
        .wrapping_add(position >> 11)
        .wrapping_add(seed)
        % 251;
    u8::try_from(value).unwrap()
}

#[derive(Debug)]
enum ServerEvent {
    RemoteCopy(Vec<ClipboardFormat>),
    FileList(Vec<FileDescriptor>, Option<u32>),
    FormatDataRequest(FormatDataRequest),
    FormatDataResponse(OwnedFormatDataResponse),
    FileContentsRequest(FileContentsRequest),
    FileContentsResponse(FileContentsResponse<'static>),
}

/// Backend of the in-memory server: forwards everything to the loopback thread.
#[derive(Debug)]
struct ServerBackend(mpsc::Sender<ServerEvent>);

impl_as_any!(ServerBackend);

impl ServerBackend {
    fn send(&self, event: ServerEvent) {
        let _ = self.0.send(event);
    }
}

impl CliprdrBackend for ServerBackend {
    fn temporary_directory(&self) -> &str {
        ".cliprdr"
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
            | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS
            | ClipboardGeneralCapabilityFlags::CAN_LOCK_CLIPDATA
            | ClipboardGeneralCapabilityFlags::HUGE_FILE_SUPPORT_ENABLED
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {}

    fn on_process_negotiated_capabilities(&mut self, _capabilities: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, available_formats: &[ClipboardFormat]) {
        self.send(ServerEvent::RemoteCopy(available_formats.to_vec()));
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        self.send(ServerEvent::FormatDataRequest(request));
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        self.send(ServerEvent::FormatDataResponse(response.into_owned()));
    }

    fn on_file_contents_request(&mut self, request: FileContentsRequest) {
        self.send(ServerEvent::FileContentsRequest(request));
    }

    fn on_file_contents_response(&mut self, response: FileContentsResponse<'_>) {
        self.send(ServerEvent::FileContentsResponse(response.into_owned()));
    }

    fn on_lock(&mut self, _data_id: LockDataId) {}

    fn on_unlock(&mut self, _data_id: LockDataId) {}

    fn on_remote_file_list(&mut self, files: &[FileDescriptor], clip_data_id: Option<u32>) {
        self.send(ServerEvent::FileList(files.to_vec(), clip_data_id));
    }
}

#[derive(Debug)]
struct Proxy(mpsc::Sender<ClipboardMessage>);

impl ClipboardMessageProxy for Proxy {
    fn send_clipboard_message(&self, message: ClipboardMessage) {
        let _ = self.0.send(message);
    }
}

enum LoopbackCommand {
    CopyFiles(Vec<ServedFile>),
    CopyFormats(Vec<ClipboardFormat>, String),
    Paste(ClipboardFormatId),
    RequestFileContents(FileContentsRequest),
}

#[derive(Clone, Copy)]
enum Side {
    Client,
    Server,
}

/// The "session" thread: carries PDUs between the client and the server, and plays the server.
struct Loopback {
    client: CliprdrClient,
    server: CliprdrServer,
    served_files: Vec<ServedFile>,
    served_text: String,
    errors: Arc<Mutex<Vec<String>>>,
    to_test: mpsc::Sender<ServerEvent>,
}

impl Loopback {
    fn deliver(&mut self, to: Side, messages: Vec<SvcMessage>) {
        let mut queue = messages
            .into_iter()
            .map(|message| (to, message.encode_unframed_pdu().unwrap()))
            .collect::<VecDeque<_>>();

        while let Some((side, payload)) = queue.pop_front() {
            let (result, other) = match side {
                Side::Client => (self.client.process(&payload), Side::Server),
                Side::Server => (self.server.process(&payload), Side::Client),
            };

            match result {
                Ok(responses) => queue.extend(
                    responses
                        .into_iter()
                        .map(|message| (other, message.encode_unframed_pdu().unwrap())),
                ),
                Err(error) => self.error(format!("processing a PDU failed: {error}")),
            }
        }
    }

    fn error(&self, error: String) {
        self.errors.lock().unwrap().push(error);
    }

    /// What `ironrdp-client` does with the messages of its clipboard backend; errors are
    /// session-fatal there, so any one fails the test.
    fn on_client_message(&mut self, message: ClipboardMessage) {
        let result = match message {
            ClipboardMessage::SendInitiateCopy(formats) => self.client.initiate_copy(&formats),
            ClipboardMessage::SendInitiateFileCopy(files) => self.client.initiate_file_copy(files),
            ClipboardMessage::SendFormatData(response) => self.client.submit_format_data(response),
            ClipboardMessage::SendInitiatePaste(format) => self.client.initiate_paste(format),
            ClipboardMessage::SendFileContentsRequest(request) => self.client.request_file_contents(request),
            ClipboardMessage::SendFileContentsResponse(response) => self.client.submit_file_contents(response),
            ClipboardMessage::Error(error) => {
                self.error(format!("clipboard backend error: {error}"));
                return;
            }
        };

        match result {
            Ok(messages) => self.deliver(Side::Server, messages.into()),
            Err(error) => self.error(format!("client rejected a backend message: {error}")),
        }
    }

    fn on_server_event(&mut self, event: ServerEvent) {
        match event {
            ServerEvent::FileContentsRequest(request) => {
                let index = usize::try_from(request.index).unwrap();
                let response = match self.served_files.get(index) {
                    Some(file) if file.is_directory() => FileContentsResponse::new_error(request.stream_id),
                    Some(file) if request.flags.contains(FileContentsFlags::SIZE) => {
                        FileContentsResponse::new_size_response(request.stream_id, file.size())
                    }
                    Some(file) => FileContentsResponse::new_data_response(
                        request.stream_id,
                        file.bytes(request.position, u64::from(request.requested_size)),
                    ),
                    None => FileContentsResponse::new_error(request.stream_id),
                };
                let messages = self.server.submit_file_contents(response).unwrap();
                self.deliver(Side::Client, messages.into());
            }
            ServerEvent::FormatDataRequest(request) => {
                let response = if request.format == ClipboardFormatId::CF_UNICODETEXT {
                    FormatDataResponse::new_unicode_string(&self.served_text).into_owned()
                } else {
                    FormatDataResponse::new_error()
                };
                let messages = self.server.submit_format_data(response).unwrap();
                self.deliver(Side::Client, messages.into());
            }
            event => {
                let _ = self.to_test.send(event);
            }
        }
    }

    fn on_command(&mut self, command: LoopbackCommand) {
        let result = match command {
            LoopbackCommand::CopyFiles(files) => {
                let descriptors = files.iter().map(|file| file.descriptor.clone()).collect();
                self.served_files = files;
                self.server.initiate_file_copy(descriptors)
            }
            LoopbackCommand::CopyFormats(formats, text) => {
                self.served_text = text;
                self.server.initiate_copy(&formats)
            }
            LoopbackCommand::Paste(format) => self.server.initiate_paste(format),
            LoopbackCommand::RequestFileContents(request) => self.server.request_file_contents(request),
        };

        match result {
            Ok(messages) => self.deliver(Side::Client, messages.into()),
            Err(error) => self.error(format!("server command failed: {error}")),
        }
    }
}

/// [`WinClipboard`] on its own STA thread with a message loop, wired to an in-memory server.
struct Harness {
    gui_thread_id: u32,
    gui: Option<thread::JoinHandle<()>>,
    commands: Option<mpsc::Sender<LoopbackCommand>>,
    session: Option<thread::JoinHandle<()>>,
    events: mpsc::Receiver<ServerEvent>,
    errors: Arc<Mutex<Vec<String>>>,
}

impl Harness {
    fn start() -> Self {
        // `RUST_LOG=ironrdp_cliprdr_native=trace` shows what the backend does.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();

        let (client_tx, client_rx) = mpsc::channel();
        let (gui_tx, gui_rx) = mpsc::channel();

        let gui = thread::spawn(move || {
            // SAFETY: Balanced below, on this thread.
            unsafe { OleInitialize(None) }.unwrap();

            let clipboard = WinClipboard::with_file_transfer(Proxy(client_tx), true).unwrap();
            // SAFETY: Always safe to call.
            let thread_id = unsafe { GetCurrentThreadId() };
            gui_tx.send((thread_id, clipboard.backend_factory())).unwrap();

            let mut message = MSG::default();
            // SAFETY: `message` is valid for writes; this thread owns its queue.
            while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
                // SAFETY: `message` came from `GetMessageW`.
                let _ = unsafe { TranslateMessage(&message) };
                // SAFETY: Same as above.
                unsafe { DispatchMessageW(&message) };
            }

            drop(clipboard);
            // SAFETY: Balances `OleInitialize` above.
            unsafe { OleUninitialize() };
        });

        let (gui_thread_id, factory) = gui_rx.recv().unwrap();

        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let errors = Arc::new(Mutex::new(Vec::new()));
        let session_errors = Arc::clone(&errors);

        let session = thread::spawn(move || run_session(factory, client_rx, command_rx, event_tx, session_errors));

        let harness = Self {
            gui_thread_id,
            gui: Some(gui),
            commands: Some(command_tx),
            session: Some(session),
            events: event_rx,
            errors,
        };

        // The format list completing the channel initialization: the server is ready after it.
        harness.expect_event(|event| matches!(event, ServerEvent::RemoteCopy(_)).then_some(()));

        harness
    }

    fn command(&self, command: LoopbackCommand) {
        self.commands.as_ref().unwrap().send(command).unwrap();
    }

    fn expect_event<T>(&self, mut accept: impl FnMut(ServerEvent) -> Option<T>) -> T {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            self.assert_no_errors();
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = self.events.recv_timeout(remaining).expect("expected server event");
            if let Some(value) = accept(event) {
                return value;
            }
        }
    }

    fn assert_no_errors(&self) {
        let errors = self.errors.lock().unwrap();
        assert!(errors.is_empty(), "loopback errors: {errors:?}");
    }

    /// Whether the clipboard owner is a window of the clipboard thread: its own window for
    /// delay-rendered formats, or the OLE clipboard window for a data object.
    fn owns_clipboard(&self) -> bool {
        // SAFETY: Always safe to call.
        let Ok(owner) = (unsafe { GetClipboardOwner() }) else {
            return false;
        };
        // SAFETY: Always safe to call.
        unsafe { GetWindowThreadProcessId(owner, None) == self.gui_thread_id }
    }

    /// Makes the clipboard window believe the application lost focus, as when the user switches
    /// to another application to copy something there.
    fn deactivate(&self) {
        extern "system" fn find(window: HWND, found: LPARAM) -> BOOL {
            let mut class = [0u8; 64];
            // SAFETY: `class` is valid for writes.
            let len = unsafe { GetClassNameA(window, &mut class) };
            if &class[..usize::try_from(len).unwrap()] == b"IronRDPClipboardMonitor" {
                let found = core::ptr::with_exposed_provenance_mut::<Option<HWND>>(usize::try_from(found.0).unwrap());
                // SAFETY: `found` is the address of the `Option<HWND>` below.
                unsafe { *found = Some(window) };
            }
            BOOL(1)
        }

        let mut window: Option<HWND> = None;
        let found = LPARAM(isize::try_from((&raw mut window).expose_provenance()).unwrap());
        // SAFETY: The callback writes into `window`, which outlives the call.
        let _ = unsafe { EnumThreadWindows(self.gui_thread_id, Some(find), found) };
        let window = window.expect("clipboard window");

        // SAFETY: Plain message post.
        unsafe { PostMessageW(Some(window), WM_ACTIVATEAPP, WPARAM(0), LPARAM(0)) }.unwrap();
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.commands = None;
        if let Some(session) = self.session.take() {
            let _ = session.join();
        }
        // SAFETY: Plain message post.
        let _ = unsafe { PostThreadMessageW(self.gui_thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(gui) = self.gui.take() {
            let _ = gui.join();
        }
    }
}

fn run_session(
    factory: Box<dyn CliprdrBackendFactory + Send>,
    client_rx: mpsc::Receiver<ClipboardMessage>,
    command_rx: mpsc::Receiver<LoopbackCommand>,
    to_test: mpsc::Sender<ServerEvent>,
    errors: Arc<Mutex<Vec<String>>>,
) {
    let (server_tx, server_rx) = mpsc::channel();

    let mut loopback = Loopback {
        client: CliprdrClient::new(factory.build_cliprdr_backend()),
        server: CliprdrServer::new(Box::new(ServerBackend(server_tx))),
        served_files: Vec::new(),
        served_text: String::new(),
        errors,
        to_test,
    };

    let _ = loopback.client.start().unwrap();
    let start = loopback.server.start().unwrap();
    loopback.deliver(Side::Client, start);

    let mut last_timeouts = Instant::now();

    loop {
        while let Ok(message) = client_rx.try_recv() {
            loopback.on_client_message(message);
        }
        while let Ok(event) = server_rx.try_recv() {
            loopback.on_server_event(event);
        }

        match command_rx.recv_timeout(Duration::from_millis(2)) {
            Ok(command) => loopback.on_command(command),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        if Duration::from_secs(1) <= last_timeouts.elapsed() {
            last_timeouts = Instant::now();
            let messages = loopback.client.drive_timeouts().unwrap();
            loopback.deliver(Side::Server, messages.into());
            let messages = loopback.server.drive_timeouts().unwrap();
            loopback.deliver(Side::Client, messages.into());
        }
    }
}

/// Pastes the clipboard into a folder the way a user does: Explorer opens a window on it, and the
/// paste runs inside `explorer.exe` (through that window's own folder object). The window is closed
/// once the expected files are there.
///
/// Pasting from the PowerShell process instead (`Shell.Application` is in-process) doesn't reach
/// the clipboard's data object when PowerShell runs in a job, as it does under some test runners.
const EXPLORER_PASTE: &str = r#"
param([string]$Destination, [int]$ExpectedFiles, [long]$ExpectedBytes)

$shell = New-Object -ComObject Shell.Application
$target = (Resolve-Path -LiteralPath $Destination).Path
Start-Process explorer.exe -ArgumentList "`"$target`""

$window = $null
$deadline = (Get-Date).AddSeconds(20)
while (-not $window -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 200
    foreach ($candidate in @($shell.Windows())) {
        try { if ($candidate.Document.Folder.Self.Path -eq $target) { $window = $candidate } } catch {}
    }
}
if (-not $window) { throw "no Explorer window on $target" }

$window.Document.Folder.Self.InvokeVerb('Paste')

$deadline = (Get-Date).AddSeconds(60)
do {
    Start-Sleep -Milliseconds 250
    $files = @(Get-ChildItem -Recurse -File -LiteralPath $target)
    $size = ($files | Measure-Object -Property Length -Sum).Sum
} while ((Get-Date) -lt $deadline -and ($files.Count -ne $ExpectedFiles -or $size -ne $ExpectedBytes))

try { $window.Quit() } catch {}
"#;

fn explorer_paste(destination: &Path, expected_files: usize, expected_bytes: u64) {
    let scripts = TempDir::new();
    let script = scripts.path().join("paste.ps1");
    fs::write(&script, EXPLORER_PASTE).unwrap();

    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Sta",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .arg(destination)
        .arg(expected_files.to_string())
        .arg(expected_bytes.to_string())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "paste failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn powershell(script: &str) {
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Sta", "-Command", script])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "powershell failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

fn set_clipboard_files(paths: &[&Path]) {
    let paths = paths.iter().map(|path| quote(path)).collect::<Vec<_>>().join(",");
    powershell(&format!("Set-Clipboard -Path {paths}"));
}

fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        thread::sleep(Duration::from_millis(50));
    }
}

fn clipboard_has(format: u32) -> bool {
    // SAFETY: Always safe to call.
    unsafe { IsClipboardFormatAvailable(format) }.is_ok()
}

fn registered_format(name: windows::core::PCWSTR) -> u32 {
    // SAFETY: `name` is a valid null-terminated string.
    unsafe { RegisterClipboardFormatW(name) }
}

/// What an OLE consumer in another apartment reads from the clipboard: the file list, and a
/// range of the `lindex`-th file through its `IStream`.
struct OleRead {
    files: Vec<FileDescriptor>,
    stream_size: u64,
    data: Vec<u8>,
}

fn read_through_ole(lindex: i32, position: u64, len: usize) -> OleRead {
    thread::spawn(move || {
        // SAFETY: Balanced below, on this thread.
        unsafe { OleInitialize(None) }.unwrap();

        let result = {
            // SAFETY: OLE is initialized on this thread.
            let data_object = unsafe { OleGetClipboard() }.unwrap();

            let descriptor_format = FORMATETC {
                cfFormat: u16::try_from(registered_format(w!("FileGroupDescriptorW"))).unwrap(),
                ptd: core::ptr::null_mut(),
                dwAspect: DVASPECT_CONTENT.0,
                lindex: -1,
                tymed: TYMED_HGLOBAL.0.cast_unsigned(),
            };
            // SAFETY: Valid FORMATETC.
            let mut medium = unsafe { data_object.GetData(&descriptor_format) }.unwrap();
            // SAFETY: `GetData` was asked for an HGLOBAL.
            let hglobal = unsafe { medium.u.hGlobal };
            let bytes = hglobal_bytes(hglobal);
            // SAFETY: Released once.
            unsafe { ReleaseStgMedium(&mut medium) };
            let files = ironrdp_core::decode::<PackedFileList>(&bytes).unwrap().files;

            let contents_format = FORMATETC {
                cfFormat: u16::try_from(registered_format(w!("FileContents"))).unwrap(),
                ptd: core::ptr::null_mut(),
                dwAspect: DVASPECT_CONTENT.0,
                lindex,
                tymed: TYMED_ISTREAM.0.cast_unsigned(),
            };
            // SAFETY: Valid FORMATETC.
            let mut medium = unsafe { data_object.GetData(&contents_format) }.unwrap();
            // SAFETY: `GetData` was asked for an IStream.
            let stream: IStream = unsafe { (*medium.u.pstm).clone() }.unwrap();
            // SAFETY: Released once; `stream` holds its own reference.
            unsafe { ReleaseStgMedium(&mut medium) };

            let mut stat = STATSTG::default();
            // SAFETY: `stat` is valid for writes; no name requested.
            unsafe { stream.Stat(&mut stat, STATFLAG_NONAME) }.unwrap();

            // SAFETY: Plain COM call.
            unsafe { stream.Seek(i64::try_from(position).unwrap(), STREAM_SEEK_SET, None) }.unwrap();

            let mut data = vec![0u8; len];
            let mut filled = 0;
            while filled < len {
                let mut read = 0u32;
                let wanted = u32::try_from(len - filled).unwrap();
                // SAFETY: The buffer is valid for `wanted` bytes from `filled`.
                unsafe {
                    stream.Read(
                        data[filled..].as_mut_ptr().cast::<c_void>(),
                        wanted,
                        Some(&raw mut read),
                    )
                }
                .ok()
                .unwrap();
                if read == 0 {
                    break;
                }
                filled += usize::try_from(read).unwrap();
            }
            data.truncate(filled);

            OleRead {
                files,
                stream_size: stat.cbSize,
                data,
            }
        };

        // SAFETY: Balances `OleInitialize` above.
        unsafe { OleUninitialize() };
        result
    })
    .join()
    .unwrap()
}

/// Copies the contents of a global memory handle that stays valid during the call.
fn hglobal_bytes(hglobal: HGLOBAL) -> Vec<u8> {
    // SAFETY: The caller keeps the handle valid.
    let size = unsafe { GlobalSize(hglobal) };
    // SAFETY: Same as above.
    let ptr = unsafe { GlobalLock(hglobal) }.cast::<u8>();
    assert!(!ptr.is_null());
    // SAFETY: The locked block holds `size` bytes.
    let bytes = unsafe { core::slice::from_raw_parts(ptr, size) }.to_vec();
    // SAFETY: Locked just above.
    let _ = unsafe { GlobalUnlock(hglobal) };
    bytes
}

fn read_clipboard_text() -> Option<String> {
    // Another application may have it open for a moment.
    wait_until("the clipboard can be opened", || {
        // SAFETY: Opened without a window, closed below.
        unsafe { OpenClipboard(None) }.is_ok()
    });

    // SAFETY: The clipboard is open; the handle stays valid until it is closed.
    let text = unsafe { GetClipboardData(ClipboardFormatId::CF_UNICODETEXT.value()) }
        .ok()
        .map(|handle| hglobal_bytes(HGLOBAL(handle.0)));

    // SAFETY: Opened above.
    let _ = unsafe { CloseClipboard() };

    let units = text?
        .chunks_exact(2)
        .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
        .take_while(|unit| *unit != 0)
        .collect::<Vec<_>>();
    String::from_utf16(&units).ok()
}

#[test]
#[ignore = "replaces the Windows clipboard contents, set IRONRDP_WIN_CLIPBOARD_LIVE=1 to run"]
fn remote_files_paste_into_explorer() {
    if !enabled() {
        return;
    }

    let files = vec![
        ServedFile::directory(None, "pasted"),
        ServedFile::directory(Some("pasted"), "nested"),
        ServedFile::file(Some("pasted\\nested"), "small.txt", 100, 1),
        ServedFile::file(Some("pasted"), "empty.bin", 0, 2),
        // More than one chunk and read-ahead window, not a multiple of either.
        ServedFile::file(Some("pasted"), "big.bin", 3 * 1024 * 1024 + 123, 3),
        ServedFile::file(None, "top.txt", 5000, 4),
    ];

    let harness = Harness::start();
    harness.command(LoopbackCommand::CopyFiles(files.clone()));

    let descriptor_format = registered_format(w!("FileGroupDescriptorW"));
    wait_until("remote files are on the clipboard", || {
        harness.owns_clipboard() && clipboard_has(descriptor_format)
    });
    assert!(clipboard_has(registered_format(w!("FileContents"))));

    let file_count = files.iter().filter(|file| !file.is_directory()).count();
    let total_size = files.iter().map(ServedFile::size).sum::<u64>();

    let destination = TempDir::new();
    explorer_paste(destination.path(), file_count, total_size);

    for file in &files {
        let path = file.local_path(destination.path());
        if file.is_directory() {
            assert!(path.is_dir(), "{} was not created", path.display());
        } else {
            wait_until(&format!("{} is complete", path.display()), || {
                fs::metadata(&path).is_ok_and(|metadata| metadata.len() == file.size())
            });
            assert!(
                fs::read(&path).unwrap() == file.bytes(0, file.size()),
                "{} differs",
                path.display()
            );
        }
    }

    harness.assert_no_errors();
}

#[test]
#[ignore = "replaces the Windows clipboard contents, set IRONRDP_WIN_CLIPBOARD_LIVE=1 to run"]
fn remote_huge_file_streams_past_4_gib() {
    if !enabled() {
        return;
    }

    let size = 5 * 1024 * 1024 * 1024 + 17;
    let files = vec![ServedFile::file(None, "huge.bin", size, 9)];

    let harness = Harness::start();
    harness.command(LoopbackCommand::CopyFiles(files.clone()));

    let descriptor_format = registered_format(w!("FileGroupDescriptorW"));
    wait_until("remote files are on the clipboard", || {
        harness.owns_clipboard() && clipboard_has(descriptor_format)
    });

    let position = 4 * 1024 * 1024 * 1024 + 1000;
    let read = read_through_ole(0, position, 300_000);

    assert_eq!(read.files.len(), 1);
    assert_eq!(read.files[0].name, "huge.bin");
    assert_eq!(read.files[0].file_size, Some(size));
    assert_eq!(read.stream_size, size);
    assert!(read.data == files[0].bytes(position, 300_000));

    // Reading the tail stops at the end of the file.
    let read = read_through_ole(0, size - 10, 100);
    assert!(read.data == files[0].bytes(size - 10, 10));

    harness.assert_no_errors();
}

#[test]
#[ignore = "replaces the Windows clipboard contents, set IRONRDP_WIN_CLIPBOARD_LIVE=1 to run"]
fn remote_formats_that_cannot_be_rendered_are_not_offered() {
    if !enabled() {
        return;
    }

    let harness = Harness::start();
    harness.command(LoopbackCommand::CopyFormats(
        vec![
            ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT),
            ClipboardFormat::new(ClipboardFormatId::CF_BITMAP),
            ClipboardFormat::new(ClipboardFormatId::CF_ENHMETAFILE),
            ClipboardFormat::new(ClipboardFormatId::CF_HDROP),
            ClipboardFormat::new(ClipboardFormatId::new(0xC0A0))
                .with_name(ClipboardFormatName::new_static("FileNameW")),
        ],
        "remote text".to_owned(),
    ));

    wait_until("remote text is on the clipboard", || {
        harness.owns_clipboard() && clipboard_has(ClipboardFormatId::CF_UNICODETEXT.value())
    });

    assert!(!clipboard_has(ClipboardFormatId::CF_BITMAP.value()));
    assert!(!clipboard_has(ClipboardFormatId::CF_ENHMETAFILE.value()));
    assert!(!clipboard_has(ClipboardFormatId::CF_HDROP.value()));
    assert!(!clipboard_has(registered_format(w!("FileNameW"))));

    assert_eq!(read_clipboard_text().as_deref(), Some("remote text"));

    harness.assert_no_errors();
}

/// Fetches the file list the client advertised, then every file through SIZE and RANGE requests.
fn fetch_local_files(harness: &Harness) -> Vec<(FileDescriptor, Vec<u8>)> {
    let file_list_format = harness.expect_event(|event| match event {
        ServerEvent::RemoteCopy(formats) => formats
            .iter()
            .find(|format| is_file_list_format(format))
            .map(|format| format.id()),
        _ => None,
    });

    harness.command(LoopbackCommand::Paste(file_list_format));
    let (files, clip_data_id) = harness
        .expect_event(|event| match event {
            ServerEvent::FileList(files, clip_data_id) => Some(Ok((files, clip_data_id))),
            ServerEvent::FormatDataResponse(response) => Some(Err(response)),
            _ => None,
        })
        .expect("file list");
    assert!(
        clip_data_id.is_some(),
        "the server should have locked the client clipboard"
    );

    let mut stream_id = 100;
    let mut request = |index: usize, flags: FileContentsFlags, position: u64, requested_size: u32| {
        stream_id += 1;
        let expected_stream_id = stream_id;
        harness.command(LoopbackCommand::RequestFileContents(FileContentsRequest {
            stream_id,
            index: i32::try_from(index).unwrap(),
            flags,
            position,
            requested_size,
            data_id: None,
        }));
        harness.expect_event(|event| match event {
            ServerEvent::FileContentsResponse(response) if response.stream_id() == expected_stream_id => {
                assert!(!response.is_error(), "file contents request failed");
                Some(response.data().to_vec())
            }
            _ => None,
        })
    };

    files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            if is_directory(file) {
                return (file.clone(), Vec::new());
            }

            let size = u64::from_le_bytes(request(index, FileContentsFlags::SIZE, 0, 8).try_into().unwrap());
            assert_eq!(Some(size), file.file_size);

            let mut data = Vec::new();
            while u64::try_from(data.len()).unwrap() < size {
                let position = u64::try_from(data.len()).unwrap();
                let wanted = u32::try_from((size - position).min(512 * 1024)).unwrap();
                let chunk = request(index, FileContentsFlags::RANGE, position, wanted);
                assert!(!chunk.is_empty());
                data.extend_from_slice(&chunk);
            }

            (file.clone(), data)
        })
        .collect()
}

#[test]
#[ignore = "replaces the Windows clipboard contents, set IRONRDP_WIN_CLIPBOARD_LIVE=1 to run"]
fn local_files_reach_the_remote() {
    if !enabled() {
        return;
    }

    let source = TempDir::new();
    let upload = source.path().join("upload");
    fs::create_dir_all(upload.join("sub")).unwrap();
    fs::create_dir(upload.join("empty")).unwrap();
    let text_contents = (0..10_000u32)
        .map(|i| u8::try_from(i % 256).unwrap())
        .collect::<Vec<_>>();
    let binary_contents = (0..(2 * 1024 * 1024 + 7u32))
        .map(|i| u8::try_from(i % 249).unwrap())
        .collect::<Vec<_>>();
    fs::write(upload.join("a.txt"), &text_contents).unwrap();
    fs::write(upload.join("sub").join("b.bin"), &binary_contents).unwrap();
    let single = source.path().join("single.txt");
    fs::write(&single, b"single file").unwrap();

    // On the clipboard before the channel is up: offered once it is ready.
    set_clipboard_files(&[&upload, &single]);

    let harness = Harness::start();
    let fetched = fetch_local_files(&harness);

    let listed = fetched
        .iter()
        .map(|(file, data)| (file.relative_path.as_deref(), file.name.as_str(), data.as_slice()))
        .collect::<Vec<_>>();
    assert_eq!(
        listed,
        [
            (None, "upload", &[][..]),
            (Some("upload"), "a.txt", text_contents.as_slice()),
            (Some("upload"), "empty", &[][..]),
            (Some("upload"), "sub", &[][..]),
            (Some("upload\\sub"), "b.bin", binary_contents.as_slice()),
            (None, "single.txt", &b"single file"[..]),
        ]
    );

    // A copy made while connected, in another application.
    harness.deactivate();
    let second = source.path().join("second.txt");
    fs::write(&second, b"second copy").unwrap();
    set_clipboard_files(&[&second]);

    let fetched = fetch_local_files(&harness);
    assert_eq!(fetched.len(), 1);
    assert_eq!(fetched[0].0.name, "second.txt");
    assert_eq!(fetched[0].1, b"second copy");

    harness.assert_no_errors();
}

#[test]
#[ignore = "replaces the Windows clipboard contents, set IRONRDP_WIN_CLIPBOARD_LIVE=1 to run"]
fn local_handle_formats_are_not_advertised() {
    if !enabled() {
        return;
    }

    let harness = Harness::start();
    harness.deactivate();

    // A bitmap: Windows also offers CF_DIB and CF_DIBV5 for it, synthesized from CF_BITMAP.
    powershell(
        "Add-Type -AssemblyName System.Windows.Forms, System.Drawing; \
         [System.Windows.Forms.Clipboard]::SetImage((New-Object System.Drawing.Bitmap 4, 4))",
    );

    let formats = harness.expect_event(|event| match event {
        ServerEvent::RemoteCopy(formats) if !formats.is_empty() => Some(formats),
        _ => None,
    });
    let ids = formats.iter().map(ClipboardFormat::id).collect::<Vec<_>>();

    assert!(ids.contains(&ClipboardFormatId::CF_DIB), "{formats:?}");
    for id in [
        ClipboardFormatId::CF_BITMAP,
        ClipboardFormatId::CF_PALETTE,
        ClipboardFormatId::CF_METAFILEPICT,
        ClipboardFormatId::CF_ENHMETAFILE,
    ] {
        assert!(!ids.contains(&id), "{id:?} advertised: {formats:?}");
    }

    harness.assert_no_errors();
}

#[test]
#[ignore = "replaces the Windows clipboard contents, set IRONRDP_WIN_CLIPBOARD_LIVE=1 to run"]
fn virtual_files_from_another_application_reach_the_remote() {
    if !enabled() {
        return;
    }

    // Two sessions: files copied in the first one are on the clipboard as virtual files (no
    // CF_HDROP), as when copying from another RDP client, and pasted into the second one.
    let first = Harness::start();
    let second = Harness::start();
    second.deactivate();

    let files = vec![
        ServedFile::directory(None, "virtual"),
        ServedFile::file(Some("virtual"), "a.bin", 300_000, 5),
        ServedFile::file(None, "b.txt", 10, 6),
    ];
    first.command(LoopbackCommand::CopyFiles(files.clone()));

    let fetched = fetch_local_files(&second);

    let listed = fetched
        .iter()
        .map(|(file, data)| (file.relative_path.as_deref(), file.name.as_str(), data.clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        listed,
        [
            (None, "virtual", Vec::new()),
            (Some("virtual"), "a.bin", files[1].bytes(0, 300_000)),
            (None, "b.txt", files[2].bytes(0, 10)),
        ]
    );

    first.assert_no_errors();
    second.assert_no_errors();
}
