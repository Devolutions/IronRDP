#[diplomat::bridge]
pub mod ffi {

    use crate::error::{ffi::IronRdpError, ValueConsumedError};

    #[diplomat::opaque]
    pub struct SocketAddr(pub std::net::SocketAddr);

    impl SocketAddr {
        pub fn look_up(host: &str, port: u16) -> Result<Box<SocketAddr>, Box<IronRdpError>> {
            use std::net::ToSocketAddrs as _;
            let addr = (host, port)
                .to_socket_addrs()?
                .next()
                .ok_or("Failed to resolve address")?;
            Ok(Box::new(SocketAddr(addr)))
        }

        // named from_ffi_str to avoid conflict with std::net::SocketAddr::from_str
        pub fn from_ffi_str(addr: &str) -> Result<Box<SocketAddr>, Box<IronRdpError>> {
            let addr = addr.parse().map_err(|_| "Failed to parse address")?;
            Ok(Box::new(SocketAddr(addr)))
        }
    }

    #[diplomat::opaque]
    pub struct VecU8(pub Vec<u8>);

    impl VecU8 {
        pub fn from_byte(bytes: &[u8]) -> Box<VecU8> {
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

    impl StdTcpStream {
        pub fn connect(addr: &SocketAddr) -> Result<Box<StdTcpStream>, Box<IronRdpError>> {
            let stream = std::net::TcpStream::connect(addr.0)?;
            Ok(Box::new(StdTcpStream(Some(stream))))
        }

        pub fn set_read_timeout(&mut self) -> Result<(), Box<IronRdpError>> {
            let stream = self
                .0
                .as_ref()
                .ok_or_else(|| ValueConsumedError::for_item("StdTcpStream"))?;
            stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
            Ok(())
        }
    }

    #[diplomat::opaque]
    pub struct OptionalUsize(pub Option<usize>);

    impl OptionalUsize {
        pub fn is_some(&self) -> bool {
            self.0.is_some()
        }

        pub fn get(&self) -> Result<usize, Box<IronRdpError>> {
            self.0.ok_or_else(|| "Value is None".into())
        }
    }
}
