#[diplomat::bridge]
pub mod ffi {
    use crate::utils::ffi::BytesSlice;

    #[diplomat::opaque]
    pub struct DecodedImage(pub ironrdp::session::image::DecodedImage);

    impl DecodedImage {
        pub fn new(pixel_format: PixelFormat, width: u16, height: u16) -> Box<Self> {
            Box::new(DecodedImage(ironrdp::session::image::DecodedImage::new(
                pixel_format.into(),
                width,
                height,
            )))
        }

        // The bytes array lives as long as the DecodedImage
        pub fn get_data(&self) -> Box<BytesSlice<'_>> {
            Box::new(BytesSlice(self.0.data()))
        }

        pub fn get_width(&self) -> u16 {
            self.0.width()
        }

        pub fn get_height(&self) -> u16 {
            self.0.height()
        }
    }

    #[diplomat::enum_convert(ironrdp::graphics::image_processing::PixelFormat)]
    pub enum PixelFormat {
        ARgb32,
        XRgb32,
        ABgr32,
        XBgr32,
        BgrA32,
        BgrX32,
        RgbA32,
        RgbX32,
    }
}
