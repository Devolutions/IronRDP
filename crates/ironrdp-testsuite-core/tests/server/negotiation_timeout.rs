//! Regression coverage for the bounded negotiation and authentication phase.
//!
//! Everything before `accept_finalize` (X.224 negotiation, the TLS handshake and
//! CredSSP) reads from a peer that has not authenticated yet. Without a bound, a
//! client that connects and goes silent holds the connection forever, and under
//! the default `ConnectionPolicy::Queue` every later client waits behind it.

use core::time::Duration;

use ironrdp_server::{RdpServer, TransportTls};
use tokio::net::{TcpListener, TcpStream};

/// The server's `NEGOTIATION_TIMEOUT`.
const BUDGET: Duration = Duration::from_secs(60);

/// Comfortably longer than [`BUDGET`]. Under paused time this costs no
/// wall-clock time; it only turns "the server never times out" into a clean
/// failure instead of a hung test.
const TEST_DEADLINE: Duration = Duration::from_secs(300);

fn server() -> RdpServer {
    RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .build()
}

/// A client that connects and sends nothing must be dropped once the budget
/// runs out, and not before.
///
/// Fails without the bound: the server parks on the read, the runtime advances
/// to `TEST_DEADLINE` instead, and the assertion reports that the connection
/// was still being held.
#[tokio::test(start_paused = true)]
async fn silent_client_is_dropped_after_the_negotiation_budget() {
    let mut server = server();
    let (client, server_side) = tokio::io::duplex(4096);

    // `RdpServer`'s future is !Send, hence the LocalSet rather than a plain spawn.
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let serving = tokio::task::spawn_local(async move {
                server.run_connection_with(server_side, TransportTls::AlreadyDone).await
            });

            // A person may be typing a password inside this phase: nothing
            // should give up early.
            tokio::time::sleep(BUDGET - Duration::from_secs(1)).await;
            assert!(
                !serving.is_finished(),
                "the server gave up before the negotiation budget"
            );

            let outcome = tokio::time::timeout(TEST_DEADLINE, serving).await;
            let joined = outcome
                .expect("server never gave up on a client that sent nothing; the connection is held indefinitely");
            let result = joined.expect("serving task panicked");
            let err = result.expect_err("a silent client must surface as an error, not a clean disconnect");
            assert!(
                err.to_string().contains("negotiate"),
                "expected the negotiation timeout error, got: {err}"
            );
            drop(client);
        })
        .await;
}

/// The same bound applies to the split `run_connection_handshake` API.
#[tokio::test(start_paused = true)]
async fn silent_client_is_dropped_by_run_connection_handshake() {
    let mut server = server();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let client = TcpStream::connect(addr).await.expect("connect");
    let (server_side, _) = listener.accept().await.expect("accept");

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let handshake =
                tokio::task::spawn_local(async move { server.run_connection_handshake(server_side).await.map(|_| ()) });

            let outcome = tokio::time::timeout(TEST_DEADLINE, handshake).await;
            let joined = outcome.expect("run_connection_handshake never gave up on a client that sent nothing");
            let result = joined.expect("handshake task panicked");
            let err = result.expect_err("a silent client must surface as an error");
            assert!(
                err.to_string().contains("negotiate"),
                "expected the negotiation timeout error, got: {err}"
            );
            drop(client);
        })
        .await;
}
