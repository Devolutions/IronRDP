#![allow(unused_crate_dependencies)]
#![allow(clippy::panic)]
#![allow(clippy::std_instead_of_core)]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rstest::rstest;

fn test_endpoint(name: &str) -> String {
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\ironrdp-agent-{name}-{}", std::process::id())
    }

    #[cfg(unix)]
    {
        let path = std::env::temp_dir().join(format!("ironrdp-agent-{name}-{}.sock", std::process::id()));
        path.display().to_string()
    }
}

fn spawn_daemon(endpoint: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_ironrdp-agent"))
        .arg("--endpoint")
        .arg(endpoint)
        .arg("daemon-start")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon")
}

fn agent(endpoint: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ironrdp-agent"))
        .arg("--endpoint")
        .arg(endpoint)
        .args(args)
        .env_remove("RDP_HOSTNAME")
        .env_remove("RDP_USERNAME")
        .env_remove("RDP_PASSWORD")
        .output()
        .expect("run agent")
}

fn wait_for_daemon(endpoint: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);

    while Instant::now() < deadline {
        let output = agent(endpoint, &["status"]);
        if output.status.success() {
            return;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    panic!("daemon did not become ready");
}

fn run_session_info(name: &str, info: ironrdp_rpc::ipc::SessionInfo, args: &[&str]) -> std::process::Output {
    use ironrdp_rpc::ipc::{Payload, Request, Response};
    use ironrdp_rpc::transport::{self, Listener};

    let endpoint = test_endpoint(name);
    let runtime = tokio::runtime::Runtime::new().expect("test runtime");
    runtime.block_on(async {
        let mut listener =
            Listener::bind(&transport::endpoint_from_string(endpoint.clone())).expect("bind mock daemon");
        let args = args.iter().map(|argument| (*argument).to_owned()).collect::<Vec<_>>();
        let command = tokio::task::spawn_blocking(move || {
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();
            agent(&endpoint, &args)
        });
        let serve = async {
            let mut stream = listener.accept().await.expect("accept session info request");
            let request: Request = transport::read_message(&mut stream).await.expect("read request");
            assert_eq!(request, Request::SessionInfo);
            transport::write_message(&mut stream, &Response::Ok(Payload::SessionInfo(info)))
                .await
                .expect("send session info");
            command.await.expect("run session info command")
        };
        tokio::time::timeout(Duration::from_secs(30), serve)
            .await
            .expect("session info command timeout")
    })
}

fn run_session_info_without_active_session(name: &str, args: &[&str]) -> std::process::Output {
    let endpoint = test_endpoint(name);
    let mut daemon = spawn_daemon(&endpoint);
    let result = std::panic::catch_unwind(|| {
        wait_for_daemon(&endpoint);
        agent(&endpoint, args)
    });

    let _ = daemon.kill();
    let _ = daemon.wait();

    match result {
        Ok(output) => output,
        Err(error) => std::panic::resume_unwind(error),
    }
}

#[test]
fn daemon_reports_no_active_session() {
    let endpoint = test_endpoint("ipc");
    let mut daemon = spawn_daemon(&endpoint);

    let result = std::panic::catch_unwind(|| {
        wait_for_daemon(&endpoint);

        let output = agent(&endpoint, &["status"]);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));

        let stdout = String::from_utf8(output.stdout).expect("stdout");
        assert!(stdout.contains("state: NoSession"), "{stdout}");
    });

    let _ = daemon.kill();
    let _ = daemon.wait();

    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

// oracle: mutant a command line without `wait-frame`, or a daemon that waits with no session
#[test]
fn wait_frame_reports_no_active_session() {
    let endpoint = test_endpoint("wait-frame");
    let mut daemon = spawn_daemon(&endpoint);

    let result = std::panic::catch_unwind(|| {
        wait_for_daemon(&endpoint);

        let started = Instant::now();
        let output = agent(
            &endpoint,
            &["wait-frame", "--after-frame", "3", "--timeout-ms", "60000"],
        );
        assert!(!output.status.success());
        assert!(started.elapsed() < Duration::from_secs(30), "{:?}", started.elapsed());

        let stderr = String::from_utf8(output.stderr).expect("stderr");
        assert!(stderr.contains("no active session"), "{stderr}");
    });

    let _ = daemon.kill();
    let _ = daemon.wait();

    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

#[test]
fn daemon_start_foreground_stops_after_ipc_shutdown() {
    let endpoint = test_endpoint("foreground");
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_ironrdp-agent"))
        .arg("--endpoint")
        .arg(&endpoint)
        .args(["daemon", "start", "--foreground"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start foreground daemon");

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_daemon(&endpoint);
        let stop = agent(&endpoint, &["daemon", "stop"]);
        assert!(stop.status.success(), "{}", String::from_utf8_lossy(&stop.stderr));
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Some(exit) = daemon.try_wait().expect("wait for foreground daemon") {
                assert!(exit.success(), "{exit}");
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("foreground daemon did not exit after stop");
    }));

    let _ = daemon.kill();
    let _ = daemon.wait();
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

#[test]
fn daemon_start_status_list_and_stop_use_the_selected_endpoint() {
    let endpoint = test_endpoint("lifecycle");
    let result = std::panic::catch_unwind(|| {
        let start = agent(&endpoint, &["daemon", "start"]);
        assert!(start.status.success(), "{}", String::from_utf8_lossy(&start.stderr));
        assert!(String::from_utf8_lossy(&start.stdout).contains("daemon started"));

        let status = agent(&endpoint, &["daemon", "status"]);
        assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
        assert!(String::from_utf8_lossy(&status.stdout).contains("state: NoSession"));

        let start_again = agent(&endpoint, &["daemon", "start"]);
        assert!(
            start_again.status.success(),
            "{}",
            String::from_utf8_lossy(&start_again.stderr)
        );
        assert!(String::from_utf8_lossy(&start_again.stdout).contains("already running"));

        let incompatible = agent(&endpoint, &["daemon", "start", "--prop", "desktopwidth:i:1234"]);
        assert!(!incompatible.status.success());
        assert!(String::from_utf8_lossy(&incompatible.stderr).contains("startup-only options"));

        let list = agent(&endpoint, &["daemon", "list"]);
        assert!(list.status.success(), "{}", String::from_utf8_lossy(&list.stderr));
        assert!(String::from_utf8_lossy(&list.stdout).contains("daemon running"));

        let stop = agent(&endpoint, &["daemon", "stop"]);
        assert!(stop.status.success(), "{}", String::from_utf8_lossy(&stop.stderr));
        assert!(String::from_utf8_lossy(&stop.stdout).contains("daemon stopped"));

        let absent = agent(&endpoint, &["daemon", "list"]);
        assert!(absent.status.success(), "{}", String::from_utf8_lossy(&absent.stderr));
        assert!(String::from_utf8_lossy(&absent.stdout).contains("no daemon running"));
    });

    // Only this uniquely named test endpoint is in scope, even if an assertion failed.
    let _ = agent(&endpoint, &["daemon", "stop"]);
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

#[test]
fn background_daemon_receives_overlay() {
    let endpoint = test_endpoint("overlay");
    let result = std::panic::catch_unwind(|| {
        let start = agent(
            &endpoint,
            &["daemon", "start", "--prop", "ClearTextPassword:s:example-secret"],
        );
        assert!(start.status.success(), "{}", String::from_utf8_lossy(&start.stderr));

        let status = agent(&endpoint, &["status"]);
        assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
        assert!(String::from_utf8_lossy(&status.stdout).contains("credentials loaded: true"));
    });
    let _ = agent(&endpoint, &["daemon", "stop"]);
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

#[test]
fn background_child_does_not_acknowledge_a_competing_daemon() {
    use ironrdp_rpc::transport::{self, Listener};
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    let endpoint = test_endpoint("bind-winner");
    let mut daemon = Command::new(env!("CARGO_BIN_EXE_ironrdp-agent"))
        .args(["--endpoint", &endpoint, "daemon-start", "--skip-certificate-check"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start competing daemon");
    let mut child = None;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for_daemon(&endpoint);
        let runtime = tokio::runtime::Runtime::new().expect("test runtime");
        runtime.block_on(async {
            let bootstrap = transport::endpoint_from_string(test_endpoint("bind-bootstrap"));
            let mut listener = Listener::bind(&bootstrap).expect("bind private bootstrap");
            child = Some(
                Command::new(env!("CARGO_BIN_EXE_ironrdp-agent"))
                    .args([
                        "--endpoint",
                        &endpoint,
                        "daemon-child",
                        "--bootstrap",
                        &bootstrap.to_string(),
                    ])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .expect("start losing child"),
            );
            let mut stream = tokio::time::timeout(Duration::from_secs(10), listener.accept())
                .await
                .expect("child bootstrap timeout")
                .expect("accept child");
            let config = br#"{"overlay":[],"skip_certificate_check":false,"rdpdr_drives":[],"smartcard":false}"#;
            stream.write_u32_le(u32::try_from(config.len()).unwrap()).await.unwrap();
            stream.write_all(config).await.unwrap();
            stream.flush().await.unwrap();
            let acknowledgement = tokio::time::timeout(Duration::from_secs(10), stream.read_u8())
                .await
                .expect("child must fail instead of adopting the competing daemon");
            assert_eq!(acknowledgement.unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof);
        });
        let child = child.as_mut().expect("spawned child");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(exit) = child.try_wait().expect("check child exit") {
                assert!(!exit.success(), "losing startup must fail");
                break;
            }
            assert!(Instant::now() < deadline, "losing child did not exit");
            std::thread::sleep(Duration::from_millis(50));
        }
        let status = agent(&endpoint, &["status"]);
        assert!(status.status.success(), "the competing daemon must remain running");
        assert!(String::from_utf8_lossy(&status.stdout).contains("state: NoSession"));
    }));
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = daemon.kill();
    let _ = daemon.wait();
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

#[test]
fn connect_with_explicit_endpoint_does_not_start_daemon() {
    let endpoint = test_endpoint("no-autostart");
    let output = agent(&endpoint, &["connect", "--no-prompt"]);
    assert!(!output.status.success());
    let list = agent(&endpoint, &["daemon", "list"]);
    assert!(String::from_utf8_lossy(&list.stdout).contains("no daemon running"));
}

#[test]
fn connect_can_start_a_selected_daemon_without_a_valid_rdp_configuration() {
    let endpoint = test_endpoint("auto-connect");
    let result = std::panic::catch_unwind(|| {
        let connect = agent(&endpoint, &["connect", "--auto-start", "--no-prompt"]);
        assert!(!connect.status.success());
        assert!(
            String::from_utf8_lossy(&connect.stderr).contains("missing required fields"),
            "{}",
            String::from_utf8_lossy(&connect.stderr)
        );

        let status = agent(&endpoint, &["daemon", "status"]);
        assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
        assert!(String::from_utf8_lossy(&status.stdout).contains("state: NoSession"));
    });
    let _ = agent(&endpoint, &["daemon", "stop"]);
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

#[test]
fn session_list_and_disconnect_distinguish_session_from_daemon() {
    let endpoint = test_endpoint("session-list");
    let absent = agent(&endpoint, &["session", "list"]);
    assert!(absent.status.success(), "{}", String::from_utf8_lossy(&absent.stderr));
    assert!(String::from_utf8_lossy(&absent.stdout).contains("no active sessions"));

    let result = std::panic::catch_unwind(|| {
        let start = agent(&endpoint, &["daemon", "start"]);
        assert!(start.status.success(), "{}", String::from_utf8_lossy(&start.stderr));
        let empty = agent(&endpoint, &["session", "list"]);
        assert!(empty.status.success(), "{}", String::from_utf8_lossy(&empty.stderr));
        assert!(String::from_utf8_lossy(&empty.stdout).contains("no active sessions"));

        let disconnect = agent(&endpoint, &["session", "disconnect"]);
        assert!(!disconnect.status.success());
        assert!(String::from_utf8_lossy(&disconnect.stderr).contains("no active session"));

        let guarded = agent(&endpoint, &["disconnect", "--server", "IT-HELP-RDM"]);
        assert!(!guarded.status.success());
        assert!(String::from_utf8_lossy(&guarded.stderr).contains("no active session"));

        let still_running = agent(&endpoint, &["daemon", "status"]);
        assert!(
            still_running.status.success(),
            "{}",
            String::from_utf8_lossy(&still_running.stderr)
        );
        assert!(String::from_utf8_lossy(&still_running.stdout).contains("state: NoSession"));
    });
    let _ = agent(&endpoint, &["daemon", "stop"]);
    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}

/// Runs the agent with `args` against a stand-in daemon that checks the command's one request and answers it.
fn answer_one_request(
    name: &str,
    args: &'static [&'static str],
    expected: ironrdp_rpc::ipc::Request,
    reply: ironrdp_rpc::ipc::Response,
) -> std::process::Output {
    use ironrdp_rpc::ipc::Request;
    use ironrdp_rpc::transport::{self, Listener};

    let endpoint = test_endpoint(name);
    let runtime = tokio::runtime::Runtime::new().expect("test runtime");
    runtime.block_on(async {
        let mut listener =
            Listener::bind(&transport::endpoint_from_string(endpoint.clone())).expect("bind mock daemon");
        let command = tokio::task::spawn_blocking(move || agent(&endpoint, args));
        let serve = async {
            let mut stream = listener.accept().await.expect("accept request");
            let request: Request = transport::read_message(&mut stream).await.expect("read request");
            assert_eq!(request, expected);
            transport::write_message(&mut stream, &reply).await.expect("send reply");
            command.await.expect("run command")
        };
        tokio::time::timeout(Duration::from_secs(30), serve)
            .await
            .expect("command timeout")
    })
}

/// Builds a connected session's status reply at `frame_number`, reporting nothing else.
fn status_at_frame(frame_number: u64) -> ironrdp_rpc::ipc::Response {
    use ironrdp_rpc::ipc::{ConnState, Payload, Response, StatusInfo};

    Response::Ok(Payload::Status(StatusInfo {
        state: ConnState::Connected,
        destination: None,
        width: None,
        height: None,
        message: None,
        credentials_loaded: false,
        untrusted_certificate: None,
        frame_number: Some(frame_number),
    }))
}

// oracle: mutant a status printer that drops the frame number or prints it under another label
#[test]
fn status_prints_the_frame_number() {
    use ironrdp_rpc::ipc::Request;

    let output = answer_one_request("frame-status", &["status"], Request::Status, status_at_frame(7));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.lines().any(|line| line == "frame: 7"), "{stdout}");
}

// oracle: mutant a command line that drops, swaps or zeroes the wait's options, or a wait that doesn't print its report
#[test]
fn wait_frame_sends_its_options_and_prints_the_report() {
    use ironrdp_rpc::ipc::Request;

    let output = answer_one_request(
        "frame-wait",
        &["wait-frame", "--after-frame", "3", "--timeout-ms", "60000"],
        Request::WaitFrame {
            after_frame: 3,
            timeout_ms: 60_000,
        },
        status_at_frame(13),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(stdout.lines().any(|line| line == "frame: 13"), "{stdout}");
}

// oracle: mutant a command line without `session info`, or a daemon that reports facts for a session it does not have
#[test]
fn session_info_reports_no_active_session_in_human_format() {
    let output = run_session_info_without_active_session("session-info-no-session-human", &["session", "info"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr");
    assert!(stderr.contains("no active session"), "{stderr}");
}

// oracle: mutant a command line without `session info`, or a daemon that reports facts for a session it does not have
#[test]
fn session_info_reports_no_active_session_in_json_format() {
    let output = run_session_info_without_active_session(
        "session-info-no-session-json",
        &["session", "info", "--format", "json"],
    );
    assert!(!output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout");
    assert_eq!(
        stdout.trim_end(),
        r#"{"category":"unavailable","message":"no active session","type":"error"}"#
    );
}

// oracle: regression Devolutions/IronRDP#2437
#[test]
fn session_info_for_activex_prints_the_backend_error() {
    use ironrdp_rpc::ipc::{AgentErrorCategory, Request, Response};
    use ironrdp_rpc::transport::{self, Listener};

    let endpoint = test_endpoint("session-info-activex");
    let runtime = tokio::runtime::Runtime::new().expect("test runtime");
    let output = runtime.block_on(async {
        let mut listener =
            Listener::bind(&transport::endpoint_from_string(endpoint.clone())).expect("bind ActiveX host");
        let command = tokio::task::spawn_blocking(move || {
            agent(
                &endpoint,
                &["--backend", "active-x", "session", "info", "--format", "json"],
            )
        });
        let serve = async {
            let mut stream = listener.accept().await.expect("accept session info request");
            let request: Request = transport::read_message(&mut stream).await.expect("read request");
            assert_eq!(request, Request::SessionInfo);
            transport::write_message(
                &mut stream,
                &Response::typed_error(
                    AgentErrorCategory::Unavailable,
                    "session info is unavailable through ActiveX",
                ),
            )
            .await
            .expect("send unavailable response");
            command.await.expect("run ActiveX session info command")
        };
        tokio::time::timeout(Duration::from_secs(30), serve)
            .await
            .expect("ActiveX session info command timeout")
    });

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim_end(),
        r#"{"category":"unavailable","message":"session info is unavailable through ActiveX","type":"error"}"#
    );
}

/// A session whose server reported every fact, the network characteristics only in part.
fn reported_session() -> ironrdp_rpc::ipc::SessionInfo {
    ironrdp_rpc::ipc::SessionInfo {
        remote_session_id: Some(2),
        logon_complete: true,
        network: Some(ironrdp_rpc::ipc::NetworkCharacteristicsInfo {
            base_rtt_ms: Some(3),
            bandwidth_kbps: None,
            average_rtt_ms: 4,
        }),
        monitors: vec![
            ironrdp_rpc::ipc::MonitorInfo {
                left: 0,
                top: 0,
                right: 1919,
                bottom: 1079,
                primary: true,
            },
            ironrdp_rpc::ipc::MonitorInfo {
                left: -1280,
                top: 0,
                right: -1,
                bottom: 1023,
                primary: false,
            },
        ],
        logon_error: Some(ironrdp_rpc::ipc::LogonErrorInfo {
            notification_type: 0xFFFF_FFFE,
            notification_data: 3,
        }),
    }
}

// oracle: mutant a report that drops a fact, prints an absent one, or mislabels a monitor
// oracle: mutant a report that labels a logon notification as an error
#[rstest]
#[case::reported_facts(
    "session-info-human-reported",
    reported_session(),
    &[
        "remote session id: 2",
        "logon complete: true",
        "network: average rtt 4 ms, base rtt 3 ms",
        "monitor 0: 0,0 to 1919,1079 (primary)",
        "monitor 1: -1280,0 to -1,1023",
        "logon notification: type 0xfffffffe, data 0x00000003",
    ],
)]
#[case::empty_facts(
    "session-info-human-empty",
    ironrdp_rpc::ipc::SessionInfo::default(),
    &["logon complete: false"],
)]
fn session_info_prints_human_facts(
    #[case] name: &str,
    #[case] info: ironrdp_rpc::ipc::SessionInfo,
    #[case] expected_lines: &[&str],
) {
    let output = run_session_info(name, info, &["session", "info"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.lines().collect::<Vec<_>>(), expected_lines);
}

// oracle: mutant a JSON document that omits an absent fact instead of writing null, or renames a field
#[rstest]
#[case::reported_facts(
    "session-info-json-reported",
    reported_session(),
    serde_json::json!({
        "type": "session_info",
        "remote_session_id": 2,
        "logon_complete": true,
        "network": { "base_rtt_ms": 3, "bandwidth_kbps": null, "average_rtt_ms": 4 },
        "monitors": [
            { "left": 0, "top": 0, "right": 1919, "bottom": 1079, "primary": true },
            { "left": -1280, "top": 0, "right": -1, "bottom": 1023, "primary": false },
        ],
        "logon_error": { "notification_type": 4_294_967_294u32, "notification_data": 3 },
    }),
)]
#[case::empty_facts(
    "session-info-json-empty",
    ironrdp_rpc::ipc::SessionInfo::default(),
    serde_json::json!({
        "type": "session_info",
        "remote_session_id": null,
        "logon_complete": false,
        "network": null,
        "monitors": [],
        "logon_error": null,
    }),
)]
fn session_info_prints_json_facts(
    #[case] name: &str,
    #[case] info: ironrdp_rpc::ipc::SessionInfo,
    #[case] expected: serde_json::Value,
) {
    let output = run_session_info(name, info, &["session", "info", "--format", "json"]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).expect("session info JSON");
    assert_eq!(document, expected);
}
