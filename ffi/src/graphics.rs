#[diplomat::bridge]
pub mod ffi {
    use std::sync::Arc;

    use crate::utils::ffi::VecU8;

    /// The client accepts pointers up to 384x384 pixels, and each one decodes to 4 bytes per
    /// pixel, so a single pointer can hold about 576 KiB the .NET garbage collector never sees.
    /// That's why .NET callers get `Dispose` to free it right away.
    #[diplomat::attr(dotnet, manually_disposable)]
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
        pub fn get_data(&self) -> Box<VecU8> {
            Box::new(VecU8(self.0.bitmap_data.clone()))
        }
    }
}
