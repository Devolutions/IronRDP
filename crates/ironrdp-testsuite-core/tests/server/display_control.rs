use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use ironrdp_server::{DesktopSize, RdpServer, RdpServerDisplay, RdpServerDisplayUpdates, ServerResult};

struct DecliningDisplay {
    consulted: Arc<AtomicBool>,
}

#[async_trait]
impl RdpServerDisplay for DecliningDisplay {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize {
            width: 1024,
            height: 768,
        }
    }

    async fn updates(&mut self) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
        unreachable!("a connection that ends during setup never asks for updates")
    }

    async fn offers_display_control(&mut self) -> bool {
        self.consulted.store(true, Ordering::Relaxed);
        false
    }
}

/// `offers_display_control` decides whether `attach_channels` registers the
/// Display Control channel, so it must be consulted during per-connection
/// channel setup, which runs before the client ever sends a byte.
#[tokio::test]
async fn display_control_offer_is_consulted_during_channel_setup() {
    let consulted = Arc::new(AtomicBool::new(false));

    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_display_handler(DecliningDisplay {
            consulted: Arc::clone(&consulted),
        })
        .build();

    let (client, server_side) = tokio::io::duplex(64);
    drop(client);
    let _ = server.run_connection(server_side).await;

    assert!(
        consulted.load(Ordering::Relaxed),
        "RdpServerDisplay::offers_display_control must be consulted during per-connection channel setup"
    );
}
