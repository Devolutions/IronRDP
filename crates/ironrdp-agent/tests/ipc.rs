#![allow(unused_crate_dependencies)]
#![allow(clippy::panic)]
#![allow(clippy::std_instead_of_core)]

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

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
