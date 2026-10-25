use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ironrdp_server::{RdpServer, RdpeiHandler, RdpeiServer, RdpeiServerFactory, ServerEvent, ServerEventSender};
use tokio::sync::mpsc;

struct NoopRdpeiHandler;

impl RdpeiHandler for NoopRdpeiHandler {}

struct RecordingRdpeiFactory {
    invoked: Arc<AtomicBool>,
}

impl ServerEventSender for RecordingRdpeiFactory {
    fn set_sender(&mut self, _sender: mpsc::UnboundedSender<ServerEvent>) {}
}

impl RdpeiServerFactory for RecordingRdpeiFactory {
    fn build_server(&self) -> RdpeiServer {
        self.invoked.store(true, Ordering::Relaxed);
        RdpeiServer::new(Box::new(NoopRdpeiHandler))
    }
}

/// `attach_channels` runs before the client ever sends a byte, so a factory
/// registered via `with_rdpei_factory` must be invoked even on a connection
/// that ends immediately. Regression test for #1773: `ironrdp-rdpei` had
/// server-side support with nothing in `ironrdp-server` ever registering it,
/// so `RdpeiServerFactory::build_server` going uncalled would silently
/// reintroduce that gap rather than fail loudly.
#[tokio::test]
async fn rdpei_factory_is_invoked_during_channel_setup() {
    let invoked = Arc::new(AtomicBool::new(false));

    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .with_rdpei_factory(Some(Box::new(RecordingRdpeiFactory {
            invoked: Arc::clone(&invoked),
        })))
        .build();

    let (client, server_side) = tokio::io::duplex(64);
    drop(client);
    let _ = server.run_connection(server_side).await;

    assert!(
        invoked.load(Ordering::Relaxed),
        "RdpeiServerFactory::build_server must be invoked during per-connection channel setup"
    );
}
