use super::{agent, spawn_daemon, test_endpoint, wait_for_daemon};

#[test]
fn daemon_reports_no_active_session() {
    let endpoint = test_endpoint("ipc");
    let mut daemon = spawn_daemon(&endpoint, false);

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
