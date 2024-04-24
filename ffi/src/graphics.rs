#[diplomat::bridge]
pub mod ffi {
    use std::rc::Rc;

    use crate::utils::ffi::BytesSlice;

    #[diplomat::opaque]
    pub struct DecodedPointer(pub Rc<ironrdp::graphics::pointer::DecodedPointer>);

    impl DecodedPointer {
        pub fn get_width(&self) -> u16 {
            self.0.width
        }

        pub fn get_height(&self) -> u16 {
            self.0.height
        }

        pub fn get_hotspot_x(&self) -> u16 {
            self.0.hotspot_x
        }

        pub fn get_hotspot_y(&self) -> u16 {
            self.0.hotspot_y
        }

        pub fn get_data(&self) -> Box<BytesSlice<'_>> {
            Box::new(BytesSlice(&self.0.bitmap_data))
        }
    }
}
