#[diplomat::bridge]
pub mod ffi {

    use crate::error::ffi::IronRdpError;

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
        pub fn from_bytes(bytes: &[u8]) -> Box<VecU8> {
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

        pub fn new_empty() -> Box<VecU8> {
            Box::new(VecU8(Vec::new()))
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
