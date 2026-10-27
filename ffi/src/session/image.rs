#[diplomat::bridge]
pub mod ffi {
    use crate::error::ffi::IronRdpError;

    #[diplomat::attr(dotnet, manually_disposable)]
    #[diplomat::opaque_mut]
    pub struct DecodedImage(pub ironrdp::session::image::DecodedImage);

    impl DecodedImage {
        pub fn new(pixel_format: PixelFormat, width: u16, height: u16) -> Box<Self> {
            Box::new(DecodedImage(ironrdp::session::image::DecodedImage::new(
                pixel_format.into(),
                width,
                height,
            )))
        }

        /// How many bytes the image holds, so you know how big a buffer to hand to [`Self::fill_data`].
        #[diplomat::attr(auto, getter = "data_size")]
        pub fn get_data_size(&self) -> usize {
            self.0.data().len()
        }

        /// Copies the image bytes into the start of `buffer` and leaves the rest of it alone.
        ///
        /// # Errors
        ///
        /// Fails when `buffer` is shorter than [`Self::get_data_size`].
        pub fn fill_data(&self, buffer: &mut [u8]) -> Result<(), Box<IronRdpError>> {
            crate::utils::fill_prefix(buffer, self.0.data())
        }

        #[diplomat::attr(auto, getter = "width")]
        pub fn get_width(&self) -> u16 {
            self.0.width()
        }

        #[diplomat::attr(auto, getter = "height")]
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
