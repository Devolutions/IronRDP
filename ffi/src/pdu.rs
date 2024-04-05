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
}
