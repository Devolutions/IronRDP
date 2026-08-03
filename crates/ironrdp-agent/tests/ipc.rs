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

    let stop = agent(&endpoint, &["stop"]);
    if result.is_ok() {
        assert!(stop.status.success(), "{}", String::from_utf8_lossy(&stop.stderr));

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if daemon.try_wait().expect("check daemon exit").is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    if daemon.try_wait().expect("check daemon exit").is_none() {
        let _ = daemon.kill();
        let _ = daemon.wait();
        if result.is_ok() {
            panic!("daemon did not stop");
        }
    }

    if let Err(error) = result {
        std::panic::resume_unwind(error);
    }
}
