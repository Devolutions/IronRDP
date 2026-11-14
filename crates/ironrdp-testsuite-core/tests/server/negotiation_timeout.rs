//! Regression coverage for the server's `NEGOTIATION_TIMEOUT`, which bounds
//! everything before `accept_finalize`.

use core::time::Duration;

use ironrdp_server::{RdpServer, TransportTls};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;

/// The server's private `NEGOTIATION_TIMEOUT`; change both together. The tests
/// assert that the connection is dropped within one second after this, so any
/// change to either value fails them instead of passing silently.
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

fn assert_dropped_at_budget(elapsed: Duration) {
    assert!(
        elapsed >= BUDGET && elapsed < BUDGET + Duration::from_secs(1),
        "expected the connection to be dropped once the {BUDGET:?} budget ran out, it took {elapsed:?}"
    );
}

/// A client that connects and sends nothing must be dropped once the budget
/// runs out, and not before.
// oracle: mutant removing the timeout around negotiation (the server parks on the read, and the test deadline fires)
// oracle: mutant shortening or lengthening the budget (the drop lands outside the asserted window)
#[tokio::test(start_paused = true)]
async fn silent_client_is_dropped_after_the_negotiation_budget() {
    let mut server = server();
    // The client half stays open, and silent, for the whole test.
    let (_client, server_side) = tokio::io::duplex(4096);

    // `RdpServer`'s future is !Send, hence the LocalSet rather than a plain spawn.
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let start = Instant::now();
            let serving = tokio::task::spawn_local(async move {
                server.run_connection_with(server_side, TransportTls::AlreadyDone).await
            });

            let outcome = tokio::time::timeout(TEST_DEADLINE, serving).await;
            let joined = outcome
                .expect("server never gave up on a client that sent nothing; the connection is held indefinitely");
            let result = joined.expect("serving task panicked");
            result.expect_err("a silent client must surface as an error, not a clean disconnect");
            assert_dropped_at_budget(start.elapsed());
        })
        .await;
}

/// The same bound applies to the split `run_connection_handshake` API.
// oracle: mutant removing the timeout around negotiation (the server parks on the read, and the test deadline fires)
// oracle: mutant shortening or lengthening the budget (the drop lands outside the asserted window)
#[tokio::test(start_paused = true)]
async fn silent_client_is_dropped_by_run_connection_handshake() {
    let mut server = server();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    // The client stays connected, and silent, for the whole test.
    let _client = TcpStream::connect(addr).await.expect("connect");
    let (server_side, _) = listener.accept().await.expect("accept");

    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            let start = Instant::now();
            let handshake =
                tokio::task::spawn_local(async move { server.run_connection_handshake(server_side).await.map(|_| ()) });

            let outcome = tokio::time::timeout(TEST_DEADLINE, handshake).await;
            let joined = outcome.expect("run_connection_handshake never gave up on a client that sent nothing");
            let result = joined.expect("handshake task panicked");
            result.expect_err("a silent client must surface as an error");
            assert_dropped_at_budget(start.elapsed());
        })
        .await;
}
