use ironrdp_rdpsnd::server::RdpsndServerHandler;

use crate::ServerEventSender;

pub trait SoundServerFactory: ServerEventSender {
    fn build_backend(&self) -> Box<dyn RdpsndServerHandler>;
}
