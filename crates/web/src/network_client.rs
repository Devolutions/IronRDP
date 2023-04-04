use ironrdp::connector::sspi::network_client::{NetworkClient, NetworkClientFactory};

#[derive(Debug, Clone)]
pub(crate) struct PlaceholderNetworkClientFactory;

impl NetworkClientFactory for PlaceholderNetworkClientFactory {
    fn network_client(&self) -> Box<dyn NetworkClient> {
        unimplemented!()
    }

    fn clone(&self) -> Box<dyn NetworkClientFactory> {
        Box::new(Clone::clone(self))
    }
}
