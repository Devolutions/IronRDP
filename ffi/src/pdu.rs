

#[diplomat::bridge]
pub mod ffi {

    #[diplomat::opaque]
    pub struct WriteBuf(pub ironrdp::pdu::write_buf::WriteBuf);

    impl WriteBuf {
        pub fn new() -> Box<WriteBuf> {
            Box::new(WriteBuf(ironrdp::pdu::write_buf::WriteBuf::new()))
        }

        pub fn clear(&mut self) {
            self.0.clear();
        }
    }
}