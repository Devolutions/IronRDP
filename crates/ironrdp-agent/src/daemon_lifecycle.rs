//! Starting and stopping the per-user daemon without tying it to a terminal.

#![allow(clippy::print_stdout)]

use core::time::Duration;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Child, Command, Stdio};

use anyhow::Context as _;
use ironrdp_daemon::daemon::{DaemonOptions, RdpdrDriveConfig};
use ironrdp_propertyset::{PropertySet, Value};
use ironrdp_rpc::ipc::{ConnState, Payload, Request, Response, StatusInfo};
use ironrdp_rpc::transport::{self, Endpoint, Listener};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const START_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_STARTUP_BYTES: u64 = 1024 * 1024;
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Settings {
    pub(crate) overlay: PropertySet,
    pub(crate) skip_certificate_check: bool,
    pub(crate) rdpdr_drives: Vec<RdpdrDriveConfig>,
    pub(crate) smartcard: bool,
}

impl Settings {
    pub(crate) async fn run(self, endpoint: Endpoint) -> anyhow::Result<()> {
        let options = DaemonOptions::default()
            .with_certificate_check_skipped(self.skip_certificate_check)
            .with_rdpdr_drives(self.rdpdr_drives)
            .with_smartcard(self.smartcard);
        ironrdp_daemon::daemon::run(endpoint, self.overlay, options).await
    }

    fn is_default(&self) -> bool {
        self.overlay.iter().next().is_none()
            && !self.skip_certificate_check
            && self.rdpdr_drives.is_empty()
            && !self.smartcard
    }

    pub(crate) async fn run_child(endpoint: Endpoint, bootstrap: String) -> anyhow::Result<()> {
        use std::io::Write as _;

        let log = open_log(&log_path()?)?;
        let result = async {
            let settings = Settings::read_bootstrap(bootstrap).await?;
            let options = DaemonOptions::default()
                .with_certificate_check_skipped(settings.skip_certificate_check)
                .with_rdpdr_drives(settings.rdpdr_drives)
                .with_smartcard(settings.smartcard);
            ironrdp_daemon::daemon::run_with_log(
                endpoint,
                settings.overlay,
                options,
                log.try_clone().context("clone daemon log handle")?,
            )
            .await
        }
        .await;
        if let Err(error) = &result {
            let _ = writeln!(&log, "daemon failed: {error:#}");
        }
        result
    }

    pub(crate) async fn read_bootstrap(bootstrap: String) -> anyhow::Result<Self> {
        let endpoint = transport::endpoint_from_string(bootstrap);
        let mut stream = transport::connect(&endpoint)
            .await
            .context("connect to daemon bootstrap endpoint")?;
        let length = u64::from(stream.read_u32_le().await.context("read daemon startup length")?);
        anyhow::ensure!(
            length <= MAX_STARTUP_BYTES,
            "daemon startup settings exceed {MAX_STARTUP_BYTES} bytes"
        );
        let mut bytes = vec![0; usize::try_from(length)?];
        stream
            .read_exact(&mut bytes)
            .await
            .context("read daemon startup settings")?;
        let config: StartupConfig = serde_json::from_slice(&bytes).context("decode daemon startup settings")?;
        config.into_settings()
    }
}

#[derive(Serialize, Deserialize)]
struct StartupConfig {
    overlay: Vec<(String, StartupValue)>,
    skip_certificate_check: bool,
    rdpdr_drives: Vec<(PathBuf, String)>,
    smartcard: bool,
}

#[derive(Serialize, Deserialize)]
enum StartupValue {
    Int(i64),
    Str(String),
}

impl From<&Settings> for StartupConfig {
    fn from(settings: &Settings) -> Self {
        let overlay = settings
            .overlay
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    Value::Int(number) => StartupValue::Int(*number),
                    Value::Str(text) => StartupValue::Str(text.clone()),
                };
                (key.to_string(), value)
            })
            .collect();
        Self {
            overlay,
            skip_certificate_check: settings.skip_certificate_check,
            rdpdr_drives: settings
                .rdpdr_drives
                .iter()
                .map(|drive| (drive.root_path().to_path_buf(), drive.display_name().to_owned()))
                .collect(),
            smartcard: settings.smartcard,
        }
    }
}

impl StartupConfig {
    fn into_settings(self) -> anyhow::Result<Settings> {
        let mut overlay = PropertySet::new();
        for (key, value) in self.overlay {
            overlay.insert(
                key,
                match value {
                    StartupValue::Int(number) => Value::Int(number),
                    StartupValue::Str(text) => Value::Str(text),
                },
            );
        }
        let rdpdr_drives = self
            .rdpdr_drives
            .into_iter()
            .map(|(root, name)| RdpdrDriveConfig::new(root, name))
            .collect::<anyhow::Result<_>>()?;
        Ok(Settings {
            overlay,
            skip_certificate_check: self.skip_certificate_check,
            rdpdr_drives,
            smartcard: self.smartcard,
        })
    }
}

pub(crate) async fn probe(endpoint: &Endpoint) -> anyhow::Result<Option<StatusInfo>> {
    for attempt in 0..5 {
        let response = tokio::time::timeout(
            Duration::from_secs(2),
            transport::send_request(endpoint, &Request::Status),
        )
        .await
        .with_context(|| format!("daemon at {endpoint} did not answer status"))?;
        match response {
            Ok(Response::Ok(Payload::Status(status))) => return Ok(Some(status)),
            Ok(Response::Ok(_)) => anyhow::bail!("unexpected status response from daemon at {endpoint}"),
            Ok(Response::Err(error)) => anyhow::bail!("daemon at {endpoint} rejected status: {error}"),
            Err(error) if endpoint_absent(&error) => return Ok(None),
            Err(error) if attempt < 4 && endpoint_closing(&error) => tokio::time::sleep(POLL_INTERVAL).await,
            Err(error) => return Err(error).with_context(|| format!("check daemon at {endpoint}")),
        }
    }
    unreachable!("the probe loop returns or errors on its last attempt")
}

/// Disconnects the active session, optionally checking its destination atomically in the daemon.
/// The daemon process remains available for another connection.
pub(crate) async fn disconnect_session(endpoint: &Endpoint, server: Option<String>) -> anyhow::Result<()> {
    let initial = probe(endpoint)
        .await?
        .with_context(|| format!("daemon is not running at {endpoint}"))?;
    anyhow::ensure!(
        matches!(
            initial.state,
            ConnState::Connecting | ConnState::Connected | ConnState::Disconnecting
        ),
        "no active session (state: {:?})",
        initial.state
    );
    let request = match server {
        Some(server) => Request::DisconnectMatching { server },
        None => Request::Disconnect,
    };
    let response = transport::send_request(endpoint, &request).await.with_context(|| {
        if matches!(request, Request::DisconnectMatching { .. }) {
            "guarded disconnect requires a current daemon; use `ironrdp-agent session disconnect` without --server or restart the daemon"
        } else {
            "send disconnect request"
        }
    })?;
    match response {
        Response::Ok(Payload::Empty) => {}
        Response::Ok(_) => anyhow::bail!("unexpected disconnect response"),
        Response::Err(error) => anyhow::bail!("{error}"),
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let status = probe(endpoint)
            .await?
            .context("session endpoint stopped while waiting for disconnect")?;
        if matches!(
            status.state,
            ConnState::NoSession | ConnState::Disconnected | ConnState::Failed
        ) {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            anyhow::bail!("disconnect still in progress; check `ironrdp-agent session list`");
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn endpoint_absent(error: &anyhow::Error) -> bool {
    error.downcast_ref::<io::Error>().is_some_and(|error| {
        error.kind() == io::ErrorKind::NotFound || cfg!(unix) && error.kind() == io::ErrorKind::ConnectionRefused
    })
}

fn endpoint_closing(error: &anyhow::Error) -> bool {
    error.downcast_ref::<io::Error>().is_some_and(|error| {
        matches!(
            error.kind(),
            io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe
        ) || cfg!(windows) && matches!(error.raw_os_error(), Some(232 | 233))
    })
}

pub(crate) async fn status(endpoint: &Endpoint) -> anyhow::Result<()> {
    let status = probe(endpoint)
        .await?
        .with_context(|| format!("daemon is not running at {endpoint}"))?;
    println!("daemon running at {endpoint}");
    print_status(&status);
    Ok(())
}

pub(crate) async fn list(endpoint: &Endpoint) -> anyhow::Result<()> {
    if let Some(status) = probe(endpoint).await? {
        println!("daemon running at {endpoint}");
        print_status(&status);
        if matches!(
            status.state,
            ConnState::Connecting | ConnState::Connected | ConnState::Disconnecting
        ) {
            println!("disconnect session: ironrdp-agent session disconnect");
        }
    } else {
        println!("no daemon running at {endpoint}");
    }
    Ok(())
}

fn print_status(status: &StatusInfo) {
    println!("state: {:?}", status.state);
    if let Some(destination) = &status.destination {
        println!("destination: {destination}");
    }
    if let (Some(width), Some(height)) = (status.width, status.height) {
        println!("resolution: {width}x{height}");
    }
}

pub(crate) async fn stop(endpoint: &Endpoint) -> anyhow::Result<()> {
    probe(endpoint)
        .await?
        .with_context(|| format!("daemon is not running at {endpoint}"))?;
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        transport::send_request(endpoint, &Request::DaemonStop),
    )
    .await
    .context("daemon did not acknowledge shutdown")?
    .context("request daemon shutdown; an older daemon may need to be stopped with Ctrl+C")?;
    match response {
        Response::Ok(Payload::Empty) => {}
        Response::Ok(_) => anyhow::bail!("unexpected daemon stop response"),
        Response::Err(error) => anyhow::bail!("daemon stop failed: {error}"),
    }
    let deadline = tokio::time::Instant::now() + STOP_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if probe(endpoint).await?.is_none() {
            tokio::time::sleep(POLL_INTERVAL).await;
            if probe(endpoint).await?.is_none() {
                println!("daemon stopped at {endpoint}");
                return Ok(());
            }
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    anyhow::bail!(
        "daemon at {endpoint} did not stop within {} seconds",
        STOP_TIMEOUT.as_secs()
    )
}

/// Ensures that an IPC listener exists. Returns true only when this call launched one.
pub(crate) async fn ensure_started(endpoint: &Endpoint, settings: Settings) -> anyhow::Result<bool> {
    if probe(endpoint).await?.is_some() {
        anyhow::ensure!(
            settings.is_default(),
            "daemon already running at {endpoint}; stop it before changing startup-only options"
        );
        return Ok(false);
    }

    let config = serde_json::to_vec(&StartupConfig::from(&settings)).context("encode daemon startup settings")?;
    anyhow::ensure!(
        u64::try_from(config.len())? <= MAX_STARTUP_BYTES,
        "daemon startup settings exceed {MAX_STARTUP_BYTES} bytes"
    );
    let log_path = log_path()?;
    let _ = open_log(&log_path)?;
    let boot_name = format!(
        "irboot-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .context("system clock precedes the Unix epoch")?
            .as_nanos()
    );
    #[cfg(windows)]
    let bootstrap = transport::default_endpoint_named(&boot_name);
    #[cfg(unix)]
    let bootstrap = Endpoint(
        log_path
            .parent()
            .context("daemon log has no directory")?
            .join(boot_name),
    );
    let mut listener = Listener::bind(&bootstrap).with_context(|| format!("bind bootstrap endpoint {bootstrap}"))?;
    let mut child = spawn_detached(endpoint, &bootstrap)?;
    let send_result: anyhow::Result<()> = async {
        let mut stream = tokio::time::timeout(START_TIMEOUT, listener.accept())
            .await
            .context("daemon did not request startup settings")?
            .context("accept daemon bootstrap connection")?;
        stream.write_u32_le(u32::try_from(config.len())?).await?;
        stream
            .write_all(&config)
            .await
            .context("send daemon startup settings")?;
        stream.flush().await.context("flush daemon startup settings")
    }
    .await;
    drop(listener);
    if let Err(error) = send_result {
        let _ = child.kill();
        child.wait();
        return Err(error);
    }

    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if probe(endpoint).await?.is_some() {
            tokio::time::sleep(POLL_INTERVAL).await;
            return Ok(child.try_wait().context("check daemon startup")?.is_none());
        }
        if let Some(exit) = child.try_wait().context("check daemon startup")? {
            // Another caller may have won the bind race. Only treat it as a failure if no
            // daemon subsequently becomes available.
            tokio::time::sleep(POLL_INTERVAL).await;
            if probe(endpoint).await?.is_some() {
                return Ok(false);
            }
            anyhow::bail!("daemon exited during startup ({exit}); see {}", log_path.display());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    let _ = child.kill();
    child.wait();
    anyhow::bail!(
        "daemon did not become ready within {} seconds; see {}",
        START_TIMEOUT.as_secs(),
        log_path.display()
    )
}

#[cfg(unix)]
struct BackgroundChild(Child);

#[cfg(unix)]
fn spawn_detached(endpoint: &Endpoint, bootstrap: &Endpoint) -> anyhow::Result<BackgroundChild> {
    use std::os::unix::process::CommandExt as _;

    let mut command = Command::new(std::env::current_exe().context("find ironrdp-agent executable")?);
    command
        .arg("--endpoint")
        .arg(endpoint.to_string())
        .arg("daemon-child")
        .arg("--bootstrap")
        .arg(bootstrap.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid has no userspace preconditions here; the child has not joined a process
    // group of which it is leader, and the call is async-signal-safe after fork.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    for name in ["RDP_PASSWORD", "RDG_PASSWORD", "RDP_USERNAME", "RDG_USERNAME"] {
        command.env_remove(name);
    }
    Ok(BackgroundChild(command.spawn().context("start background daemon")?))
}

#[cfg(unix)]
impl BackgroundChild {
    fn try_wait(&mut self) -> io::Result<Option<String>> {
        self.0.try_wait().map(|status| status.map(|status| status.to_string()))
    }

    fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
    }

    fn wait(&mut self) {
        let _ = self.0.wait();
    }
}

#[cfg(windows)]
mod windows_process {
    use std::ffi::{OsStr, OsString};
    use std::os::windows::ffi::OsStrExt as _;

    use anyhow::Context as _;
    use ironrdp_rpc::transport::Endpoint;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{
        CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, GetExitCodeProcess, PROCESS_INFORMATION,
        STARTUPINFOW, TerminateProcess, WaitForSingleObject,
    };
    use windows::core::{PCWSTR, PWSTR};

    pub(super) struct BackgroundChild(HANDLE);

    impl Drop for BackgroundChild {
        fn drop(&mut self) {
            // SAFETY: CreateProcessW returned this owned handle.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    impl BackgroundChild {
        pub(super) fn try_wait(&mut self) -> anyhow::Result<Option<String>> {
            // SAFETY: self.0 is a valid process handle until this guard is dropped.
            match unsafe { WaitForSingleObject(self.0, 0) } {
                WAIT_TIMEOUT => Ok(None),
                WAIT_OBJECT_0 => {
                    let mut code = 0;
                    // SAFETY: The process handle and output pointer are valid.
                    unsafe { GetExitCodeProcess(self.0, &mut code) }?;
                    Ok(Some(format!("exit code {code}")))
                }
                _ => anyhow::bail!("wait for background daemon failed: {}", std::io::Error::last_os_error()),
            }
        }

        pub(super) fn kill(&mut self) -> anyhow::Result<()> {
            // SAFETY: self.0 is a valid handle for the child this call started.
            unsafe { TerminateProcess(self.0, 1) }.context("terminate unresponsive daemon")
        }

        pub(super) fn wait(&mut self) {
            // SAFETY: self.0 is a valid process handle until this guard is dropped.
            let _ = unsafe { WaitForSingleObject(self.0, 5000) };
        }
    }

    // Backslashes preceding a quote must be doubled in the Windows command line.
    fn quote(argument: &OsStr) -> String {
        let text = argument.to_string_lossy();
        let mut result = String::from("\"");
        let mut backslashes = 0;
        for character in text.chars() {
            match character {
                '\\' => backslashes += 1,
                '"' => {
                    result.extend(core::iter::repeat_n('\\', backslashes * 2 + 1));
                    result.push('"');
                    backslashes = 0;
                }
                _ => {
                    result.extend(core::iter::repeat_n('\\', backslashes));
                    result.push(character);
                    backslashes = 0;
                }
            }
        }
        result.extend(core::iter::repeat_n('\\', backslashes * 2));
        result.push('"');
        result
    }

    pub(super) fn spawn_detached(endpoint: &Endpoint, bootstrap: &Endpoint) -> anyhow::Result<BackgroundChild> {
        let exe = std::env::current_exe().context("find ironrdp-agent executable")?;
        let exe_wide: Vec<u16> = exe.as_os_str().encode_wide().chain([0]).collect();
        let line = format!(
            "{} --endpoint {} daemon-child --bootstrap {}",
            quote(exe.as_os_str()),
            quote(OsStr::new(&endpoint.to_string())),
            quote(OsStr::new(&bootstrap.to_string())),
        );
        let mut line_wide: Vec<u16> = OsStr::new(&line).encode_wide().chain([0]).collect();

        let mut environment = Vec::new();
        let mut vars: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(key, _)| {
                !["RDP_PASSWORD", "RDG_PASSWORD", "RDP_USERNAME", "RDG_USERNAME"]
                    .iter()
                    .any(|sensitive| key.to_string_lossy().eq_ignore_ascii_case(sensitive))
            })
            .collect();
        vars.sort_by_key(|(key, _)| key.to_string_lossy().to_ascii_lowercase());
        for (key, value) in vars {
            environment.extend(key.encode_wide());
            environment.push(u16::from(b'='));
            environment.extend(value.encode_wide());
            environment.push(0);
        }
        environment.push(0);
        if environment.len() == 1 {
            environment.push(0);
        }

        let startup = STARTUPINFOW {
            cb: u32::try_from(size_of::<STARTUPINFOW>()).expect("STARTUPINFOW fits in u32"),
            ..Default::default()
        };
        let mut process = PROCESS_INFORMATION::default();
        // SAFETY: The UTF-16 strings, environment block, and output structs remain alive for the
        // duration of CreateProcessW. No parent handles are inherited, including captured stdio.
        unsafe {
            CreateProcessW(
                PCWSTR(exe_wide.as_ptr()),
                Some(PWSTR(line_wide.as_mut_ptr())),
                None,
                None,
                false,
                CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
                Some(environment.as_ptr().cast()),
                PCWSTR::null(),
                &startup,
                &mut process,
            )
        }
        .context("start background daemon")?;
        // SAFETY: CreateProcessW returned owned thread and process handles; the process handle
        // stays in BackgroundChild while the thread handle is not needed after startup.
        let _ = unsafe { CloseHandle(process.hThread) };
        Ok(BackgroundChild(process.hProcess))
    }

    #[cfg(test)]
    mod tests {
        use std::ffi::OsStr;

        use super::quote;

        #[test]
        fn quotes_windows_executable_and_pipe_arguments() {
            assert_eq!(
                quote(OsStr::new("C:\\Program Files\\agent.exe")),
                "\"C:\\Program Files\\agent.exe\""
            );
            assert_eq!(quote(OsStr::new("a\\")), "\"a\\\\\"");
            assert_eq!(quote(OsStr::new("a\"b")), "\"a\\\"b\"");
        }
    }
}

#[cfg(windows)]
use windows_process::spawn_detached;

fn log_path() -> anyhow::Result<PathBuf> {
    #[cfg(windows)]
    let directory = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .context("LOCALAPPDATA is required to store the daemon log")?
        .join("ironrdp-agent");
    #[cfg(unix)]
    let directory = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state")))
        .context("HOME or XDG_STATE_HOME is required to store the daemon log")?
        .join("ironrdp-agent");

    fs::create_dir_all(&directory).with_context(|| format!("create {}", directory.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("restrict {}", directory.display()))?;
    }
    Ok(directory.join("daemon.log"))
}

fn open_log(path: &PathBuf) -> anyhow::Result<fs::File> {
    let truncate = fs::metadata(path).is_ok_and(|metadata| metadata.len() >= MAX_LOG_BYTES);
    let mut options = OpenOptions::new();
    options.create(true).write(true);
    if truncate {
        options.truncate(true);
    } else {
        options.append(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("open daemon log {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_settings_round_trip_values_without_logging_secrets() {
        let mut settings = Settings::default();
        settings.overlay.insert("ClearTextPassword", "example-secret");
        settings.overlay.insert("desktopwidth", 1234i64);
        settings.smartcard = true;

        let serialized = serde_json::to_vec(&StartupConfig::from(&settings)).expect("serialize");
        let config: StartupConfig = serde_json::from_slice(&serialized).expect("deserialize");
        let decoded = config.into_settings().expect("restore settings");
        assert_eq!(decoded.overlay, settings.overlay);
        assert!(decoded.smartcard);
        assert!(!decoded.is_default());
    }

    #[tokio::test]
    async fn disconnect_waits_for_session_without_stopping_the_listener() {
        let name = format!(
            "ir-disconnect-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .subsec_nanos()
        );
        let endpoint = transport::default_endpoint_named(&name);
        let mut listener = Listener::bind(&endpoint).expect("bind isolated test endpoint");
        let server = async move {
            let mut disconnected = false;
            let mut polls_after_disconnect = 0;
            for _ in 0..5 {
                let mut stream = listener.accept().await.expect("accept");
                let request: Request = transport::read_message(&mut stream).await.expect("read request");
                let response = match request {
                    Request::Disconnect => {
                        disconnected = true;
                        Response::ok()
                    }
                    Request::Status => {
                        let state = if !disconnected {
                            ConnState::Connected
                        } else if polls_after_disconnect == 0 {
                            ConnState::Disconnecting
                        } else {
                            ConnState::Disconnected
                        };
                        if disconnected {
                            polls_after_disconnect += 1;
                        }
                        Response::Ok(Payload::Status(StatusInfo {
                            state,
                            destination: Some("server.example:3389".to_owned()),
                            width: None,
                            height: None,
                            message: None,
                            credentials_loaded: false,
                            untrusted_certificate: None,
                        }))
                    }
                    _ => panic!("unexpected request to mock daemon"),
                };
                transport::write_message(&mut stream, &response)
                    .await
                    .expect("send response");
            }
            disconnected
        };

        let client = async {
            disconnect_session(&endpoint, None)
                .await
                .expect("disconnect active session");
            assert_eq!(
                probe(&endpoint).await.expect("still listening").expect("status").state,
                ConnState::Disconnected
            );
        };
        let ((), disconnected) = tokio::join!(client, server);
        assert!(disconnected);
    }
}
