use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use ironrdp_async::FramedWrite as _;
use ironrdp_core::{encode_vec, impl_as_any};
use ironrdp_dvc::{DrdynvcServer, DvcMessage, DvcServerProcessor};
use ironrdp_pdu::PduResult;
use ironrdp_pdu::mcs::{DisconnectProviderUltimatum, DisconnectReason};
use ironrdp_pdu::x224::X224;
use ironrdp_server::{
    DesktopSize, DisplayContext, DisplayUpdate, DynamicChannelFactory, EchoDvcBridge, RdpServer, RdpServerDisplay,
    RdpServerDisplayUpdates, ServerResult,
};
use ironrdp_tokio::TokioFramed;

struct ResourceChannel(Arc<AtomicUsize>);

impl Drop for ResourceChannel {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

impl_as_any!(ResourceChannel);

impl DvcServerProcessor for ResourceChannel {}

impl ironrdp_dvc::DvcProcessor for ResourceChannel {
    fn channel_name(&self) -> &str {
        "test::resource"
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, _payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        Ok(Vec::new())
    }
}

struct TestDisplay;

#[async_trait]
impl RdpServerDisplay for TestDisplay {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize {
            width: 1280,
            height: 720,
        }
    }

    async fn updates(&mut self, _: DisplayContext) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
        Ok(Box::new(PendingUpdates))
    }

    async fn offers_display_control(&mut self) -> bool {
        false
    }
}

struct PendingUpdates;

#[async_trait]
impl RdpServerDisplayUpdates for PendingUpdates {
    async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>> {
        core::future::pending().await
    }
}

struct ResourceFactory {
    dropped: Arc<AtomicUsize>,
    order: Arc<AtomicUsize>,
}

impl DynamicChannelFactory for ResourceFactory {
    fn attach(&self, channels: DrdynvcServer) -> DrdynvcServer {
        assert_eq!(self.order.fetch_add(1, Ordering::Relaxed) % 2, 0);
        assert!(channels.get_channel_id_by_type::<EchoDvcBridge>().is_some());
        channels.with_dynamic_channel(ResourceChannel(Arc::clone(&self.dropped)))
    }
}

struct OrderFactory(Arc<AtomicUsize>);

impl DynamicChannelFactory for OrderFactory {
    fn attach(&self, channels: DrdynvcServer) -> DrdynvcServer {
        assert_eq!(self.0.fetch_add(1, Ordering::Relaxed) % 2, 1);
        assert!(channels.get_channel_id_by_type::<EchoDvcBridge>().is_some());
        assert!(channels.get_channel_id_by_type::<ResourceChannel>().is_some());
        channels
    }
}

#[tokio::test]
async fn dynamic_channel_factory_builds_and_releases_a_fresh_processor_per_connection() {
    // mutant: Skip or reorder a factory, remove a built-in channel, or retain its processor across connections.
    let dropped = Arc::new(AtomicUsize::new(0));
    let order = Arc::new(AtomicUsize::new(0));
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_display_handler(TestDisplay)
        .with_dynamic_channel_factory(Box::new(ResourceFactory {
            dropped: Arc::clone(&dropped),
            order: Arc::clone(&order),
        }))
        .with_dynamic_channel_factory(Box::new(OrderFactory(Arc::clone(&order))))
        .build();

    for connection_index in 1..=2 {
        let (client, server_stream) = tokio::io::duplex(65536);
        let client = async move {
            let mut client = TokioFramed::new(client);
            super::activation::play_client_with_capabilities(&mut client, false, &[], None).await;
            client
                .write_all(
                    &encode_vec(&X224(DisconnectProviderUltimatum {
                        reason: DisconnectReason::UserRequested,
                    }))
                    .expect("encode disconnect"),
                )
                .await
                .expect("disconnect client");
        };
        let (result, ()) = tokio::join!(server.run_connection(server_stream), client);
        result.expect("connection ends normally");

        assert_eq!(dropped.load(Ordering::Relaxed), connection_index);
    }
}
