#[diplomat::bridge]
pub mod ffi {

    use crate::error::ffi::IronRdpError;

    #[diplomat::opaque]
    pub struct WriteBuf(pub ironrdp::pdu::write_buf::WriteBuf);

    impl WriteBuf {
        pub fn new() -> Box<WriteBuf> {
            Box::new(WriteBuf(ironrdp::pdu::write_buf::WriteBuf::new()))
        }

        pub fn clear(&mut self) {
            self.0.clear();
        }

        pub fn read_into_buf(&mut self, buf: &mut [u8]) -> Result<(), Box<IronRdpError>> {
            buf.copy_from_slice(&self.0[..buf.len()]);
            Ok(())
        }
    }

    #[diplomat::opaque]
    pub struct SecurityProtocol(pub ironrdp::pdu::nego::SecurityProtocol);

    #[diplomat::opaque]
    pub struct ConnectInitial(pub ironrdp::pdu::mcs::ConnectInitial);

    #[diplomat::opaque]
    pub struct InclusiveRectangle(pub ironrdp::pdu::geometry::InclusiveRectangle);

    #[diplomat::opaque]
    pub struct IronRdpPdu; // A struct representing the ironrdp_pdu crate

    #[diplomat::opaque]
    pub struct PduInfo(pub ironrdp::pdu::PduInfo);

    impl PduInfo {
        pub fn get_action(&self) -> Box<Action> {
            Box::new(Action(self.0.action))
        }

        pub fn get_length(&self) -> usize {
            self.0.length
        }
    }

    #[diplomat::opaque]
    pub struct Action(pub ironrdp::pdu::Action);

    impl IronRdpPdu {
        pub fn new() -> Box<IronRdpPdu> {
            Box::new(IronRdpPdu)
        }

        pub fn find_size(&self, bytes: &[u8]) -> Result<Option<Box<PduInfo>>, Box<IronRdpError>> {
            Ok(ironrdp::pdu::find_size(bytes)?.map(PduInfo).map(Box::new))
        }
    }

    #[diplomat::opaque]
    pub struct FastPathInputEvent(pub ironrdp::pdu::input::fast_path::FastPathInputEvent);

    #[diplomat::opaque]
    pub struct FastPathInputEventIterator(pub Vec<ironrdp::pdu::input::fast_path::FastPathInputEvent>);
}

impl From<Vec<ironrdp::pdu::input::fast_path::FastPathInputEvent>> for ffi::FastPathInputEventIterator {
    fn from(value: Vec<ironrdp::pdu::input::fast_path::FastPathInputEvent>) -> Self {
        ffi::FastPathInputEventIterator(value)
    }
}
