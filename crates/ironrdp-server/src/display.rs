use std::num::NonZeroU16;

use anyhow::Result;
use ironrdp_pdu::pointer::PointerPositionAttribute;

#[rustfmt::skip]
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
    PointerPosition(PointerPositionAttribute),
    ColorPointer(ColorPointer),
    RGBAPointer(RGBAPointer),
    HidePointer,
    DefaultPointer,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PixelOrder {
    TopToBottom,
    BottomToTop,
}

#[derive(Clone)]
pub struct RGBAPointer {
    pub width: u16,
    pub height: u16,
    pub hot_x: u16,
    pub hot_y: u16,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for RGBAPointer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RGBAPointer")
            .field("with", &self.width)
            .field("height", &self.height)
            .field("hot_x", &self.hot_x)
            .field("hot_y", &self.hot_y)
            .field("data_len", &self.data.len())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct ColorPointer {
    pub width: u16,
    pub height: u16,
    pub hot_x: u16,
    pub hot_y: u16,
    pub and_mask: Vec<u8>,
    pub xor_mask: Vec<u8>,
}

/// Bitmap Display Update
///
/// Bitmap updates are encoded using RDP 6.0 compression, fragmented and sent using
/// Fastpath Server Updates
///
#[derive(Clone)]
pub struct BitmapUpdate {
    pub top: u16,
    pub left: u16,
    pub width: NonZeroU16,
    pub height: NonZeroU16,
    pub format: PixelFormat,
    pub order: PixelOrder,
    pub data: Vec<u8>,
}

impl std::fmt::Debug for BitmapUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BitmapUpdate")
            .field("top", &self.top)
            .field("left", &self.left)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .field("order", &self.order)
            .finish()
    }
}

/// Display Updates receiver for an RDP server
///
/// The RDP server will repeatedly call the `next_update` method to receive
/// display updates which will then be encoded and sent to the client
///
/// See [`RdpServerDisplay`] example.
#[async_trait::async_trait]
pub trait RdpServerDisplayUpdates {
    /// # Cancel safety
    ///
    /// This method MUST be cancellation safe because it is used in a
    /// `tokio::select!` statement. If some other branch completes first, it
    /// MUST be guaranteed that no data is lost.
    async fn next_update(&mut self) -> Option<DisplayUpdate>;
}

/// Display for an RDP server
///
/// # Example
///
/// ```
///# use anyhow::Result;
/// use ironrdp_server::{DesktopSize, DisplayUpdate, RdpServerDisplay, RdpServerDisplayUpdates};
///
/// pub struct DisplayUpdates {
///     receiver: tokio::sync::mpsc::Receiver<DisplayUpdate>,
/// }
///
/// #[async_trait::async_trait]
/// impl RdpServerDisplayUpdates for DisplayUpdates {
///     async fn next_update(&mut self) -> Option<DisplayUpdate> {
///         self.receiver.recv().await
///     }
/// }
///
/// pub struct DisplayHandler {
///     width: u16,
///     height: u16,
/// }
///
/// #[async_trait::async_trait]
/// impl RdpServerDisplay for DisplayHandler {
///     async fn size(&mut self) -> DesktopSize {
///         DesktopSize { width: self.width, height: self.height }
///     }
///
///     async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>> {
///         Ok(Box::new(DisplayUpdates { receiver: todo!() }))
///     }
/// }
/// ```
#[async_trait::async_trait]
pub trait RdpServerDisplay {
    /// This method should return the current size of the display.
    /// Currently, there is no way for the client to negotiate resolution,
    /// so the size returned by this method will be enforced.
    async fn size(&mut self) -> DesktopSize;

    /// Return a display updates receiver
    async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>>;
}
