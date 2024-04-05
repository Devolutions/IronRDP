#[diplomat::bridge]
pub mod ffi {
    use crate::utils::ffi::BytesSlice;


    #[diplomat::opaque]
    pub struct DecodedImage<'a>(pub &'a mut ironrdp::session::image::DecodedImage);

    impl<'a> DecodedImage<'a> {
        // The bytes array lives as long as the DecodedImage
        pub fn get_data(&'a self) -> Box<BytesSlice<'a>>{
            Box::new(BytesSlice(self.0.data()))
        }

        pub fn get_width(&self) -> u16 {
            self.0.width()
        }

        pub fn get_height(&self) -> u16 {
            self.0.height()
        }
    }
}