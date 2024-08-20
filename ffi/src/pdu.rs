#[diplomat::bridge]
pub mod ffi {

    use ironrdp::pdu::geometry::Rectangle;

    use crate::{error::ffi::IronRdpError, utils::ffi::VecU8};

    #[diplomat::opaque]
    pub struct WriteBuf(pub ironrdp_core::WriteBuf);

    impl WriteBuf {
        pub fn new() -> Box<WriteBuf> {
            Box::new(WriteBuf(ironrdp_core::WriteBuf::new()))
        }

        pub fn clear(&mut self) {
            self.0.clear();
        }

        pub fn read_into_buf(&mut self, buf: &mut [u8]) -> Result<(), Box<IronRdpError>> {
            buf.copy_from_slice(&self.0[..buf.len()]);
            Ok(())
        }

        pub fn get_filled(&self) -> Box<VecU8> {
            Box::new(VecU8(self.0.filled().to_vec()))
        }
    }

    #[diplomat::opaque]
    pub struct SecurityProtocol(pub ironrdp::pdu::nego::SecurityProtocol);

    #[diplomat::opaque]
    pub struct ConnectInitial(pub ironrdp::pdu::mcs::ConnectInitial);

    #[diplomat::opaque]
    pub struct InclusiveRectangle(pub ironrdp::pdu::geometry::InclusiveRectangle);

    impl InclusiveRectangle {
        pub fn get_left(&self) -> u16 {
            self.0.left
        }

        pub fn get_top(&self) -> u16 {
            self.0.top
        }

        pub fn get_right(&self) -> u16 {
            self.0.right
        }

        pub fn get_bottom(&self) -> u16 {
            self.0.bottom
        }

        pub fn get_width(&self) -> u16 {
            self.0.width()
        }

        pub fn get_height(&self) -> u16 {
            self.0.height()
        }
    }

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
