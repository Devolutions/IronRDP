use ironrdp_connector::DesktopSize;
pub use ironrdp_graphics::image_processing::PixelFormat;

#[derive(Debug, Clone)]
pub enum DisplayUpdate {
    Bitmap(BitmapUpdate),
}

#[derive(Debug, Clone, Copy)]
pub enum PixelOrder {
    TopToBottom,
    BottomToTop,
}

#[derive(Debug, Clone)]
pub struct BitmapUpdate {
    pub top: u32,
    pub left: u32,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub order: PixelOrder,
    pub data: Vec<u8>,
}

#[async_trait::async_trait]
pub trait RdpServerDisplay {
    async fn size(&mut self) -> DesktopSize;
    async fn get_update(&mut self) -> Option<DisplayUpdate>;
}
