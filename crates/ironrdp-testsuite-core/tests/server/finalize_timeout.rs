//! Regression coverage for the bounded `accept_finalize` handshake.
//!
//! A client can finish the security handshake and then stop producing PDUs,
//! leaving the server blocked on a socket read. `RdpServer` serves one
//! connection at a time, so without a bound that connection holds the server
//! indefinitely — observed in the field for 45 minutes against a real client
//! that completed every channel join and then never sent its Client Info PDU.

use core::time::Duration;

use ironrdp_core::{WriteBuf, decode, encode_buf};
use ironrdp_pdu::nego::{self, SecurityProtocol};
use ironrdp_pdu::x224::X224;
use ironrdp_server::{RdpServer, TransportTls};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

/// Comfortably longer than the server's own `FINALIZE_TIMEOUT`, so whichever
/// deadline fires first tells us whether the server is bounded. Under paused
/// time this costs no wall-clock time either way — it only has to be a
/// deadline the runtime can advance to when nothing else is pending, which is
/// what turns "the server never times out" into a clean failure instead of a
/// hung test.
const TEST_DEADLINE: Duration = Duration::from_secs(300);

/// A client that completes X.224 negotiation and then goes silent must be
/// dropped, not waited on forever.
///
/// Fails without the bound: with no timer of its own the server parks on the
/// read, the runtime advances to `TEST_DEADLINE` instead, and the assertion
/// below reports that the connection was still being held.
#[tokio::test(start_paused = true)]
async fn client_that_stalls_during_finalize_is_dropped() {
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .build();

    let (mut client, server_side) = tokio::io::duplex(4096);

    // `RdpServer`'s future is !Send, hence the LocalSet rather than a plain spawn.
    let local = tokio::task::LocalSet::new();
    local
        .run_until(async move {
            // `AlreadyDone` keeps the test free of TLS and certificates; with
            // `with_no_security` the negotiation resolves to plain RDP and goes
            // straight to the finalize handshake either way.
            let serving = tokio::task::spawn_local(async move {
                server.run_connection_with(server_side, TransportTls::AlreadyDone).await
            });

            // X.224 negotiation, and nothing after it — no MCS Connect Initial,
            // which is precisely what the stalled client in the field failed to send.
            let request = nego::ConnectionRequest {
                nego_data: None,
                flags: nego::RequestFlags::empty(),
                protocol: SecurityProtocol::empty(),
                correlation_info: None,
            };
            let mut buf = WriteBuf::new();
            encode_buf(&X224(request), &mut buf).expect("encode connection request");
            client.write_all(buf.filled()).await.expect("send connection request");

            let mut confirm = [0u8; 128];
            let read = client.read(&mut confirm).await.expect("read connection confirm");
            let _ = decode::<X224<nego::ConnectionConfirm>>(&confirm[..read]).expect("server answered the negotiation");

            // Deliberately silent from here.
            let outcome = tokio::time::timeout(TEST_DEADLINE, serving).await;

            let joined = outcome.expect(
                "server never gave up on a client that stalled during finalize; the connection (and, since \
                 RdpServer serves one at a time, the server itself) is held indefinitely",
            );
            let result = joined.expect("serving task panicked");
            let err = result.expect_err("a stalled client must surface as an error, not a clean disconnect");
            assert!(
                err.to_string().contains("finalize"),
                "expected the finalize timeout error, got: {err}"
            );
        })
        .await;
}
