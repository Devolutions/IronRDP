#[diplomat::bridge]
pub mod ffi {

    use crate::error::ffi::IronRdpError;

    #[diplomat::attr(dotnet, manually_disposable)]
    #[diplomat::opaque]
    pub struct VecU8(pub Vec<u8>);

    impl VecU8 {
        pub fn from_bytes(bytes: &[u8]) -> Box<VecU8> {
            Box::new(VecU8(bytes.to_vec()))
        }

        #[diplomat::attr(auto, getter = "size")]
        pub fn get_size(&self) -> usize {
            self.0.len()
        }

        pub fn fill(&self, buffer: &mut [u8]) -> Result<(), Box<IronRdpError>> {
            super::fill_prefix(buffer, &self.0)
        }

        pub fn new_empty() -> Box<VecU8> {
            Box::new(VecU8(Vec::new()))
        }
    }

    #[diplomat::opaque]
    pub struct BytesSlice<'a>(pub &'a [u8]);

    impl<'a> BytesSlice<'a> {
        #[diplomat::attr(auto, getter = "size")]
        pub fn get_size(&'a self) -> usize {
            self.0.len()
        }

        pub fn fill(&'a self, buffer: &'a mut [u8]) -> Result<(), Box<IronRdpError>> {
            super::fill_prefix(buffer, self.0)
        }
    }

    #[diplomat::opaque]
    pub struct U32Slice<'a>(pub &'a [u32]);

    impl<'a> U32Slice<'a> {
        #[diplomat::attr(auto, getter = "size")]
        pub fn get_size(&'a self) -> usize {
            self.0.len()
        }

        pub fn fill(&'a self, buffer: &'a mut [u32]) -> Result<(), Box<IronRdpError>> {
            super::fill_prefix(buffer, self.0)
        }
    }

    pub struct Position {
        pub x: u16,
        pub y: u16,
    }

    #[diplomat::opaque]
    pub struct OptionalUsize(pub Option<usize>);

    impl OptionalUsize {
        pub fn is_some(&self) -> bool {
            self.0.is_some()
        }

        pub fn get(&self) -> Result<usize, Box<IronRdpError>> {
            self.0.ok_or_else(|| "value is None".into())
        }
    }
}

/// Reads a monotonic clock for `received_at` arguments. Epoch is the first call; only
/// differences are meaningful. This binding targets native .NET hosts only, never
/// `wasm32-unknown-unknown`, so `std::time::Instant` is sufficient here (compare
/// `ironrdp-blocking`'s identical helper, which makes the same assumption for the same reason).
pub(crate) fn monotonic_now() -> ironrdp::connector::MonotonicInstant {
    static EPOCH: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);
    ironrdp::connector::MonotonicInstant::from_millis(u64::try_from(EPOCH.elapsed().as_millis()).unwrap_or(u64::MAX))
}

pub(crate) fn fill_prefix<T: Copy>(buffer: &mut [T], src: &[T]) -> Result<(), Box<crate::error::ffi::IronRdpError>> {
    buffer
        .get_mut(..src.len())
        .ok_or("buffer is too small")?
        .copy_from_slice(src);
    Ok(())
}
