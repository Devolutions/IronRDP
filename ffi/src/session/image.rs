#[diplomat::bridge]
pub mod ffi {
    use crate::utils::ffi::BytesArray;


    #[diplomat::opaque]
    pub struct DecodedImage<'a>(pub &'a mut ironrdp::session::image::DecodedImage);

    impl<'a> DecodedImage<'a> {
        // The bytes array lives as long as the DecodedImage
        pub fn get_data(&'a self) -> Box<BytesArray<'a>>{
            Box::new(BytesArray(self.0.data()))
        }
    }
}