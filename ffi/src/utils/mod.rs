
#[diplomat::bridge]
pub mod ffi {

    #[diplomat::opaque]
    pub struct SocketAddr(pub std::net::SocketAddr);

    #[diplomat::opaque]
    pub struct VecU8(pub Vec<u8>);

    impl VecU8 {
        pub fn from_byte(bytes:&[u8]) -> Box<VecU8> {
            Box::new(VecU8(bytes.to_vec()))
        }


        pub fn get_size(&self) -> usize {
            self.0.len()
        }


        pub fn fill(&self, buffer: &mut [u8]) {
            if buffer.len() < self.0.len() {
                //TODO: FIX: Should not panic, for prototype only
                panic!("Buffer is too small")
            }
            buffer.copy_from_slice(&self.0)
        }
    }

    #[diplomat::opaque]
    pub struct Any<'a>(pub &'a dyn std::any::Any);

    #[diplomat::opaque]
    pub struct StdTcpStream(pub Option<std::net::TcpStream>);

}