use anyhow::Result;
use std::num::NonZeroU16;

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
    pub top: u16,
    pub left: u16,
    pub width: NonZeroU16,
    pub height: NonZeroU16,
    pub format: PixelFormat,
    pub order: PixelOrder,
    pub data: Vec<u8>,
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
