//! Coverage for [`ConnectionPolicy`] in `RdpServer::run`.
//!
//! `run` serves one connection at a time. `Queue` (the default) leaves a
//! second connection unanswered in the listen backlog until the first ends;
//! `Reject` closes it at once so the client fails fast instead of appearing to
//! hang.

use core::net::SocketAddr;
use core::time::Duration;

use ironrdp_server::{ConnectionPolicy, RdpServer, ServerEvent};
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpStream;
use tokio::sync::oneshot;

async fn bound_addr(sender: &tokio::sync::mpsc::UnboundedSender<ServerEvent>) -> SocketAddr {
    // Poll until the accept loop has bound and can answer GetLocalAddr.
    for _ in 0..200 {
        let (tx, rx) = oneshot::channel();
        if sender.send(ServerEvent::GetLocalAddr(tx)).is_ok() {
            if let Ok(Some(addr)) = rx.await {
                return addr;
            }
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("server never reported its local address");
}

fn build(policy: ConnectionPolicy) -> RdpServer {
    RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .with_connection_policy(policy)
        .build()
}

/// With `Reject`, a second connection arriving while a session runs is closed
/// immediately: the read returns EOF rather than hanging.
#[tokio::test]
async fn reject_closes_a_second_connection_during_a_session() {
    let mut server = build(ConnectionPolicy::Reject);
    let sender = server.event_sender().clone();

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let task = tokio::task::spawn_local(async move { server.run().await });
            let addr = bound_addr(&sender).await;

            // Occupy the single session (never handshakes, so run_connection
            // parks reading).
            let holder = TcpStream::connect(addr).await.expect("holder connect");
            let mut second = TcpStream::connect(addr).await.expect("second connect");

            let mut buf = [0u8; 8];
            let read = tokio::time::timeout(Duration::from_secs(5), second.read(&mut buf))
                .await
                .expect("Reject must answer the second connection promptly")
                .expect("read on the rejected connection");
            assert_eq!(read, 0, "Reject must close the second connection");

            drop(holder);
            sender.send(ServerEvent::Quit("done".into())).expect("quit");
            let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
        })
        .await;
}

/// With the default `Queue`, a second connection is left unanswered while the
/// session runs: the read does not complete within the window.
#[tokio::test]
async fn queue_leaves_a_second_connection_waiting_during_a_session() {
    let mut server = build(ConnectionPolicy::Queue);
    let sender = server.event_sender().clone();

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let task = tokio::task::spawn_local(async move { server.run().await });
            let addr = bound_addr(&sender).await;

            let holder = TcpStream::connect(addr).await.expect("holder connect");
            let mut second = TcpStream::connect(addr).await.expect("second connect");

            let mut buf = [0u8; 8];
            let outcome = tokio::time::timeout(Duration::from_millis(500), second.read(&mut buf)).await;
            assert!(
                outcome.is_err(),
                "Queue must leave the second connection waiting, not close it"
            );

            drop(holder);
            drop(second);
            sender.send(ServerEvent::Quit("done".into())).expect("quit");
            let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
        })
        .await;
}
