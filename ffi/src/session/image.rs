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
        /// the next.
        /// Only the bytes under `rect` are written, and the rest of `buffer` stays as it was.
        /// The rules it follows are in [ffi/INTENT.md], "Copying a rectangle into a caller buffer".
        ///
        /// # Errors
        ///
        /// Fails when `rect` is inverted or reaches past the image, when one row of `rect` does not fit
        /// in `buffer_stride`, when the last row of `rect` would start past `usize::MAX`, or when `buffer`
        /// is too short to hold the last row of `rect`.
        ///
        /// [ffi/INTENT.md]: ../../INTENT.md#copying-a-rectangle
        #[expect(
            clippy::nonminimal_bool,
            reason = "each check is written as the invariant it guards, e.g. `row_end <= buffer_stride`"
        )]
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
            // INVARIANT: for y in top..=bottom: dst + row_end <= buffer.len() and src + row_end <= data.len()
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

#[cfg(test)]
mod tests {
    use super::ffi::{DecodedImage, PixelFormat};
    use crate::pdu::ffi::InclusiveRectangle;

    // oracle: intent ffi/INTENT.md §copying-a-rectangle
    #[test]
    fn copy_rect_into_rejects_an_inverted_rectangle() {
        let image = DecodedImage::new(PixelFormat::BgrA32, 8, 4);
        let rect = InclusiveRectangle(ironrdp::pdu::geometry::InclusiveRectangle {
            left: 3,
            top: 0,
            right: 2,
            bottom: 0,
        });

        let mut buffer = [0xff; 8 * 4 * 4];

        image.copy_rect_into(&rect, &mut buffer, 8 * 4).unwrap_err();

        assert!(buffer.iter().all(|&b| b == 0xff));
    }
}
