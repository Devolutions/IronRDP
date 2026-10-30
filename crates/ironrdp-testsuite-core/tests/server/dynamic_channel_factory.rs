use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use ironrdp_core::{decode, encode_vec, impl_as_any};
use ironrdp_dvc::{DrdynvcServer, DvcMessage, DvcServerProcessor};
use ironrdp_pdu::PduResult;
use ironrdp_pdu::nego;
use ironrdp_pdu::x224::X224;
use ironrdp_server::{DynamicChannelFactory, EchoDvcBridge, RdpServer};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

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

struct ResourceFactory {
    created: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
    order: Arc<AtomicUsize>,
}

impl DynamicChannelFactory for ResourceFactory {
    fn attach(&self, channels: DrdynvcServer) -> DrdynvcServer {
        assert_eq!(self.order.fetch_add(1, Ordering::Relaxed) % 2, 0);
        assert!(channels.get_channel_id_by_type::<EchoDvcBridge>().is_some());
        self.created.fetch_add(1, Ordering::Relaxed);
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
    // oracle: mutant a configured factory is skipped, misordered, or reuses its previous processor.
    let created = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicUsize::new(0));
    let order = Arc::new(AtomicUsize::new(0));
    let mut server = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .with_dynamic_channel_factory(Box::new(ResourceFactory {
            created: Arc::clone(&created),
            dropped: Arc::clone(&dropped),
            order: Arc::clone(&order),
        }))
        .with_dynamic_channel_factory(Box::new(OrderFactory(Arc::clone(&order))))
        .build();

    for connection_index in 1..=2 {
        let (mut client, server_stream) = tokio::io::duplex(4096);
        let client = async move {
            let request = nego::ConnectionRequest {
                nego_data: None,
                flags: nego::RequestFlags::empty(),
                protocol: nego::SecurityProtocol::empty(),
                correlation_info: None,
            };
            client
                .write_all(&encode_vec(&X224(request)).expect("encode connection request"))
                .await
                .expect("send connection request");

            let mut confirm = [0u8; 128];
            let read = client.read(&mut confirm).await.expect("read connection confirm");
            let _ = decode::<X224<nego::ConnectionConfirm>>(&confirm[..read]).expect("server answered negotiation");
        };
        let _ = tokio::join!(server.run_connection(server_stream), client);

        assert_eq!(created.load(Ordering::Relaxed), connection_index);
        assert_eq!(dropped.load(Ordering::Relaxed), connection_index);
    }
}
