#[diplomat::bridge]
pub mod ffi {

    use crate::error::ffi::IronRdpError;

    #[diplomat::opaque]
    pub struct VecVecU8(pub Vec<Vec<u8>>);

    impl VecVecU8 {
        pub fn get_len(&self) -> usize {
            self.0.len()
        }

        pub fn get_vecu8(&self, index: usize) -> Result<Box<VecU8>, Box<IronRdpError>> {
            if index >= self.0.len() {
                return Err("index out of bounds".into());
            }
            Ok(Box::new(VecU8(self.0[index].clone())))
        }

        pub fn get_slice<'a>(&'a self, index: usize) -> Result<Box<BytesSlice<'a>>, Box<IronRdpError>> {
            if index >= self.0.len() {
                return Err("index out of bounds".into());
            }
            Ok(Box::new(BytesSlice(&self.0[index])))
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

        pub fn fill(&self, buffer: &mut [u8]) -> Result<(), Box<IronRdpError>> {
            if buffer.len() < self.0.len() {
                return Err("buffer is too small".into());
            }
            buffer.copy_from_slice(&self.0);
            Ok(())
        }

        pub fn new_empty() -> Box<VecU8> {
            Box::new(VecU8(Vec::new()))
        }
    }

    #[diplomat::opaque]
    pub struct BytesSlice<'a>(pub &'a [u8]);

    impl<'a> BytesSlice<'a> {
        pub fn get_size(&'a self) -> usize {
            self.0.len()
        }

        pub fn fill(&'a self, buffer: &'a mut [u8]) -> Result<(), Box<IronRdpError>> {
            if buffer.len() < self.0.len() {
                return Err("buffer is too small".into());
            }
            buffer.copy_from_slice(self.0);
            Ok(())
        }
    }

    #[diplomat::opaque]
    pub struct U32Slice<'a>(pub &'a [u32]);

    impl<'a> U32Slice<'a> {
        pub fn get_size(&'a self) -> usize {
            self.0.len()
        }

        pub fn fill(&'a self, buffer: &'a mut [u32]) -> Result<(), Box<IronRdpError>> {
            if buffer.len() < self.0.len() {
                return Err("buffer is too small".into());
            }
            buffer.copy_from_slice(self.0);
            Ok(())
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

    #[diplomat::opaque]
    pub struct OptionalString(pub(crate) Option<String>);

    impl OptionalString {
        pub fn is_some(&self) -> bool {
            self.0.is_some()
        }

        pub fn new(value: &str) -> Box<OptionalString> {
            Box::new(OptionalString(Some(value.to_owned())))
        }

        pub fn new_empty() -> Box<OptionalString> {
            Box::new(OptionalString(None))
        }
    }
}

impl From<&ffi::OptionalString> for Option<String> {
    fn from(value: &ffi::OptionalString) -> Self {
        value.0.clone()
    }
}
