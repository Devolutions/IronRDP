#[diplomat::bridge]
pub mod ffi {
    use std::sync::Arc;

    use crate::utils::ffi::BytesSlice;

    #[diplomat::opaque]
    pub struct DecodedPointer(pub Arc<ironrdp::graphics::pointer::DecodedPointer>);

    impl DecodedPointer {
        #[diplomat::attr(auto, getter = "width")]
        pub fn get_width(&self) -> u16 {
            self.0.width
        }

        #[diplomat::attr(auto, getter = "height")]
        pub fn get_height(&self) -> u16 {
            self.0.height
        }

        #[diplomat::attr(auto, getter = "hotspot_x")]
        pub fn get_hotspot_x(&self) -> u16 {
            self.0.hotspot_x
        }

        #[diplomat::attr(auto, getter = "hotspot_y")]
        pub fn get_hotspot_y(&self) -> u16 {
            self.0.hotspot_y
        }

        #[diplomat::attr(auto, getter = "data")]
        pub fn get_data(&self) -> Box<BytesSlice<'_>> {
            Box::new(BytesSlice(&self.0.bitmap_data))
        }
    }
}
