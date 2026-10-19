#[diplomat::bridge]
pub mod ffi {
    use crate::error::ffi::IronRdpError;
    use crate::pdu::ffi::InclusiveRectangle;

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

        /// Copies the pixels inside `rect` into `buffer`, at the same spot they sit in the image.
        ///
        /// Think of `buffer` as a picture laid out like this image: same pixel format, rows one after
        /// another from the top, and `buffer_stride` bytes from the start of one row to the start of
        /// the next. Only the bytes under `rect` are written, and the rest of `buffer` stays as it was.
        ///
        /// # Errors
        ///
        /// Fails when `rect` is inverted or reaches past the image, when one row of `rect` does not fit
        /// in `buffer_stride`, or when `buffer` is too short to hold the last row of `rect`.
        #[expect(clippy::nonminimal_bool, reason = "STYLE.md spells invariants out positively")]
        pub fn copy_rect_into(
            &self,
            rect: &InclusiveRectangle,
            buffer: &mut [u8],
            buffer_stride: usize,
        ) -> Result<(), Box<IronRdpError>> {
            let image = &self.0;
            let ironrdp::pdu::geometry::InclusiveRectangle {
                left,
                top,
                right,
                bottom,
            } = rect.0;

            if !(left <= right && top <= bottom) {
                return Err("rectangle is inverted".into());
            }

            if !(right < image.width() && bottom < image.height()) {
                return Err("rectangle is outside the image".into());
            }

            let bytes_per_pixel = image.bytes_per_pixel();
            let row_start = usize::from(left) * bytes_per_pixel;
            let row_end = (usize::from(right) + 1) * bytes_per_pixel;
            if !(row_end <= buffer_stride) {
                return Err("buffer stride is too small".into());
            }

            let required = usize::from(bottom)
                .checked_mul(buffer_stride)
                .and_then(|offset| offset.checked_add(row_end))
                .ok_or("buffer stride is too large")?;
            if !(required <= buffer.len()) {
                return Err("buffer is too small".into());
            }

            let image_stride = image.stride();
            let data = image.data();
            for y in usize::from(top)..=usize::from(bottom) {
                let src = y * image_stride;
                let dst = y * buffer_stride;
                buffer[dst + row_start..dst + row_end].copy_from_slice(&data[src + row_start..src + row_end]);
            }

            Ok(())
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
