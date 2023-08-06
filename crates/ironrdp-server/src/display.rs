pub use ironrdp_acceptor::DesktopSize;
pub use ironrdp_graphics::image_processing::PixelFormat;

/// Display Update
///
/// Contains all types of display updates currently supported by the server implementation
/// and the RDP spec
///
#[derive(Debug, Clone)]
pub enum DisplayUpdate {
    Bitmap(BitmapUpdate),
}

#[derive(Debug, Clone, Copy)]
pub enum PixelOrder {
    TopToBottom,
    BottomToTop,
}

/// Bitmap Display Update
///
/// Bitmap updates are encoded using RDP 6.0 compression, fragmented and sent using
/// Fastpath Server Updates
///
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

/// Display Update reciever for an RDP server
///
/// The RDP server will repeatadly call the `get_update` method to receive display updates which
/// will then be encoded and sent to the client
///
/// # Example
///
/// ```
/// pub struct DisplayHandler {
///     width: u16,
///     height: u16,
///     receiver: tokio::sync::mpsc::Receiver<DisplayUpdate>,
/// }
///
/// #[async_trait::async_trait]
/// impl RdpServerDisplay for DisplayHandler {
///     async fn size(&mut self) -> DesktopSize {
///         DesktopSize { self.height, self.width }
///     }
///
///     async fn get_update(&mut self) -> Option<DisplayUpdate> {
///         self.receiver.recv().await
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait RdpServerDisplay {
    async fn size(&mut self) -> DesktopSize;
    async fn get_update(&mut self) -> Option<DisplayUpdate>;
}
