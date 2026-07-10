#![cfg(windows)]
#![expect(
    unused_crate_dependencies,
    reason = "the package's library dependencies are also linked into this integration test"
)]

use std::process::Command;

use ironrdp_agent::ipc::{NowExecutionKind, Payload, Request, Response};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::windows::named_pipe::ServerOptions;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn now_process_preserves_remote_exit_code() {
    let endpoint = format!(r"\\.\pipe\ironrdp-agent-remote-exit-{}", std::process::id());
    let server = ServerOptions::new()
        .first_pipe_instance(true)
        .create(&endpoint)
        .expect("create test IPC endpoint");

    let server_task = tokio::spawn(async move {
        server.connect().await.expect("accept CLI connection");

        let mut server = server;
        let mut len = [0u8; 4];
        server.read_exact(&mut len).await.expect("read request length");
        let body_len = usize::try_from(u32::from_le_bytes(len)).expect("request length fits usize");
        let mut body = vec![0; body_len];
        server.read_exact(&mut body).await.expect("read request body");
        let request = ironrdp_core::decode_owned::<Request>(&body).expect("decode request");
        assert!(matches!(
            request,
            Request::NowExecute(ref request) if request.kind == NowExecutionKind::PowerShell
        ));

        for response in [
            Response::Ok(Payload::NowExecutionStarted { operation_id: 9 }),
            Response::Ok(Payload::NowExecutionData {
                operation_id: 9,
                stream: ironrdp_agent::ipc::NowStream::Stdout,
                data: b"stdout".to_vec(),
            }),
            Response::Ok(Payload::NowExecutionData {
                operation_id: 9,
                stream: ironrdp_agent::ipc::NowStream::Stderr,
                data: b"stderr".to_vec(),
            }),
            Response::Ok(Payload::NowExecutionResult {
                operation_id: 9,
                exit_code: 7,
            }),
        ] {
            let body = ironrdp_core::encode_vec(&response).expect("encode response");
            let body_len = u32::try_from(body.len()).expect("response length fits u32");
            server
                .write_all(&body_len.to_le_bytes())
                .await
                .expect("write response length");
            server.write_all(&body).await.expect("write response body");
        }
        server.flush().await.expect("flush response");
    });

    let endpoint_for_cli = endpoint.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_ironrdp-agent"))
            .args(["--endpoint", &endpoint_for_cli, "now", "pwsh", "Write-Output ignored"])
            .output()
            .expect("run ironrdp-agent")
    })
    .await
    .expect("join CLI process");

    server_task.await.expect("join IPC server");
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout, b"stdout");
    assert_eq!(output.stderr, b"stderr");
}
