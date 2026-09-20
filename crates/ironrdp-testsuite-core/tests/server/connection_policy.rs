//! Coverage for [`ConnectionPolicy`] in `RdpServer::run`.
//!
//! `run` serves one connection at a time. `Queue` (the default outside
//! `Hybrid` security) leaves a second connection unanswered in the listen
//! backlog until the first ends; `Reject` closes it at once so the client
//! fails fast instead of appearing to hang.

use core::net::SocketAddr;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;
use std::sync::Arc;

use ironrdp_server::{ConnectionHandler, ConnectionPolicy, RdpServer, RdpServerSecurity, ServerEvent};
use tokio::io::AsyncReadExt as _;
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::server::{ClientHello, ResolvesServerCert};
use tokio_rustls::rustls::sign::CertifiedKey;

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

/// Drive `server` and assert a second connection is left unanswered while the
/// session runs: the read does not complete within the window.
async fn assert_second_connection_is_left_waiting(mut server: RdpServer) {
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

/// With `Queue`, a second connection is left unanswered while the session
/// runs.
#[tokio::test]
async fn queue_leaves_a_second_connection_waiting_during_a_session() {
    assert_second_connection_is_left_waiting(build(ConnectionPolicy::Queue)).await;
}

/// Pins the out-of-the-box policy under `with_no_security` end to end: with no
/// `with_connection_policy` call, a second connection is left waiting (`Queue`),
/// NOT served in place of the live one. `None` authenticates nothing, so a
/// `Preempt` default here would let any peer that can reach the port evict the
/// live session.
#[tokio::test]
async fn the_default_under_no_security_leaves_a_second_connection_waiting() {
    let server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .build();
    assert_second_connection_is_left_waiting(server).await;
}

/// A `TlsAcceptor` that can be constructed without any certificate material.
/// It would fail the first handshake, but nothing here handshakes: the tests
/// only need a value of each `RdpServerSecurity` variant.
#[derive(Debug)]
struct NoCert;

impl ResolvesServerCert for NoCert {
    fn resolve(&self, _client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        None
    }
}

fn tls_acceptor() -> TlsAcceptor {
    let config = tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(NoCert));
    TlsAcceptor::from(Arc::new(config))
}

/// The default policy follows the security mode: takeover out of the box only
/// where the newcomer is authenticated before it could evict anything
/// (`Hybrid`); the pre-existing queue-behind everywhere else.
#[test]
fn the_default_policy_follows_the_security_mode() {
    assert_eq!(
        ConnectionPolicy::default_for(&RdpServerSecurity::None),
        ConnectionPolicy::Queue
    );
    assert_eq!(
        ConnectionPolicy::default_for(&RdpServerSecurity::Tls(tls_acceptor())),
        ConnectionPolicy::Queue
    );
    assert_eq!(
        ConnectionPolicy::default_for(&RdpServerSecurity::Hybrid((tls_acceptor(), Vec::new()))),
        ConnectionPolicy::Preempt
    );
}

struct CountingHandler {
    accepted: Arc<AtomicUsize>,
}

impl ConnectionHandler for CountingHandler {
    fn on_accept(&mut self, _peer: SocketAddr) -> bool {
        self.accepted.fetch_add(1, Ordering::SeqCst);
        true
    }
}

/// A connection closed by `Reject` is dropped before it enters the handler's
/// lifecycle: `on_accept` sees the served connection only.
#[tokio::test]
async fn reject_does_not_consult_the_handler_for_a_closed_connection() {
    let accepted = Arc::new(AtomicUsize::new(0));
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .with_connection_policy(ConnectionPolicy::Reject)
        .with_connection_handler(Some(Box::new(CountingHandler {
            accepted: Arc::clone(&accepted),
        })))
        .build();
    let sender = server.event_sender().clone();

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let task = tokio::task::spawn_local(async move { server.run().await });
            let addr = bound_addr(&sender).await;

            let holder = TcpStream::connect(addr).await.expect("holder connect");
            let mut second = TcpStream::connect(addr).await.expect("second connect");

            let mut buf = [0u8; 8];
            let read = tokio::time::timeout(Duration::from_secs(5), second.read(&mut buf))
                .await
                .expect("Reject must answer the second connection promptly")
                .expect("read on the rejected connection");
            assert_eq!(read, 0, "Reject must close the second connection");
            assert_eq!(
                accepted.load(Ordering::SeqCst),
                1,
                "only the served connection reaches on_accept"
            );

            drop(holder);
            sender.send(ServerEvent::Quit("done".into())).expect("quit");
            let _ = tokio::time::timeout(Duration::from_secs(5), task).await;
        })
        .await;
}
