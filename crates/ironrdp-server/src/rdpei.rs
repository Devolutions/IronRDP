pub use ironrdp_rdpei::pdu::{
    CsReadyPdu, DismissHoveringTouchContactPdu, PenEventPdu, RdpInputProtocolVersion, ScReadyFeatures, TouchContact,
    TouchContactFlags, TouchEventPdu, TouchFrame,
};
pub use ironrdp_rdpei::{RdpeiHandler, RdpeiServer};

use crate::ServerEventSender;

pub trait RdpeiServerFactory: ServerEventSender {
    fn build_handler(&self) -> Box<dyn RdpeiHandler>;
}
