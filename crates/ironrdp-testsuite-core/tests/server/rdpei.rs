use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use ironrdp_core::{decode, encode_vec};
use ironrdp_pdu::nego;
use ironrdp_pdu::x224::X224;
use ironrdp_server::{RdpServer, RdpeiHandler, RdpeiServer, RdpeiServerFactory, ServerEvent, ServerEventSender};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
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

/// Channel setup runs once a connection has negotiated, so a factory
/// registered via `with_rdpei_factory` must be invoked on a connection that
/// negotiates and then ends. Regression test for #1773: `ironrdp-rdpei` had
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

    let (mut client, server_side) = tokio::io::duplex(4096);
    let client = async move {
        let request = nego::ConnectionRequest {
            nego_data: None,
            flags: nego::RequestFlags::empty(),
            protocol: nego::SecurityProtocol::empty(),
            correlation_info: None,
        };
        let request = encode_vec(&X224(request)).expect("encode connection request");
        client.write_all(&request).await.expect("send connection request");
        let mut confirm = [0u8; 128];
        let read = client.read(&mut confirm).await.expect("read connection confirm");
        let _ = decode::<X224<nego::ConnectionConfirm>>(&confirm[..read]).expect("server answered the negotiation");
    };
    let _ = tokio::join!(server.run_connection(server_side), client);

    assert!(
        invoked.load(Ordering::Relaxed),
        "RdpeiServerFactory::build_server must be invoked during per-connection channel setup"
    );
}
