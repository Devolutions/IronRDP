use ironrdp_connector::DesktopSize;

#[derive(Debug)]
pub enum DisplayUpdate {
    Bitmap(BitmapUpdate),
}

#[derive(Debug)]
pub struct BitmapUpdate {
    pub top: u32,
    pub left: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub bits_per_pixel: u16,
    pub data: Vec<u8>,
}

#[async_trait::async_trait]
pub trait RdpServerDisplay {
    async fn size(&mut self) -> DesktopSize;
    async fn get_update(&mut self) -> Option<DisplayUpdate>;
}
