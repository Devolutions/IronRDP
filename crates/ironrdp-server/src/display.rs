use core::num::NonZeroU16;

use anyhow::Result;
use bytes::{Bytes, BytesMut};
use ironrdp_displaycontrol::pdu::DisplayControlMonitorLayout;
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
    Resize(DesktopSize),
    Bitmap(BitmapUpdate),
    PointerPosition(PointerPositionAttribute),
    ColorPointer(ColorPointer),
    RGBAPointer(RGBAPointer),
    HidePointer,
    DefaultPointer,
}

#[derive(Clone)]
pub struct RGBAPointer {
    pub width: u16,
    pub height: u16,
    pub hot_x: u16,
    pub hot_y: u16,
    pub data: Vec<u8>,
}

impl core::fmt::Debug for RGBAPointer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
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

pub struct Framebuffer {
    pub width: NonZeroU16,
    pub height: NonZeroU16,
    pub format: PixelFormat,
    pub data: BytesMut,
    pub stride: usize,
}

impl TryInto<Framebuffer> for BitmapUpdate {
    type Error = &'static str;

    fn try_into(self) -> Result<Framebuffer, Self::Error> {
        assert_eq!(self.x, 0);
        assert_eq!(self.y, 0);
        Ok(Framebuffer {
            width: self.width,
            height: self.height,
            format: self.format,
            data: self.data.try_into_mut().map_err(|_| "BitmapUpdate is shared")?,
            stride: self.stride,
        })
    }
}

/// Bitmap Display Update
///
/// Bitmap updates are encoded using RDP 6.0 compression, fragmented and sent using
/// Fastpath Server Updates
///
#[derive(Clone)]
pub struct BitmapUpdate {
    pub x: u16,
    pub y: u16,
    pub width: NonZeroU16,
    pub height: NonZeroU16,
    pub format: PixelFormat,
    pub data: Bytes,
    pub stride: usize,
}

impl BitmapUpdate {
    /// Extracts a sub-region of the bitmap update.
    ///
    /// # Parameters
    ///
    /// - `x`: The x-coordinate of the top-left corner of the sub-region.
    /// - `y`: The y-coordinate of the top-left corner of the sub-region.
    /// - `width`: The width of the sub-region.
    /// - `height`: The height of the sub-region.
    ///
    /// # Returns
    ///
    /// An `Option` containing a new `BitmapUpdate` representing the sub-region if the specified
    /// dimensions are within the bounds of the original bitmap update, otherwise `None`.
    ///
    /// # Example
    ///
    /// ```
    /// # use core::num::NonZeroU16;
    /// # use bytes::Bytes;
    /// # use ironrdp_graphics::image_processing::PixelFormat;
    /// # use ironrdp_server::BitmapUpdate;
    /// let original = BitmapUpdate {
    ///     x: 0,
    ///     y: 0,
    ///     width: NonZeroU16::new(100).unwrap(),
    ///     height: NonZeroU16::new(100).unwrap(),
    ///     format: PixelFormat::ARgb32,
    ///     data: Bytes::from(vec![0; 40000]),
    ///     stride: 400,
    /// };
    ///
    /// let sub_region = original.sub(10, 10, NonZeroU16::new(50).unwrap(), NonZeroU16::new(50).unwrap());
    /// assert!(sub_region.is_some());
    /// ```
    #[must_use]
    pub fn sub(&self, x: u16, y: u16, width: NonZeroU16, height: NonZeroU16) -> Option<Self> {
        if x + width.get() > self.width.get() || y + height.get() > self.height.get() {
            None
        } else {
            let bpp = usize::from(self.format.bytes_per_pixel());
            let start = usize::from(y) * self.stride + usize::from(x) * bpp;
            let end = start + usize::from(height.get() - 1) * self.stride + usize::from(width.get()) * bpp;
            Some(Self {
                x: self.x + x,
                y: self.y + y,
                width,
                height,
                format: self.format,
                data: self.data.slice(start..end),
                stride: self.stride,
            })
        }
    }
}

impl core::fmt::Debug for BitmapUpdate {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BitmapUpdate")
            .field("x", &self.x)
            .field("y", &self.y)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .field("stride", &self.stride)
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
pub trait RdpServerDisplay: Send {
    /// This method should return the current size of the display.
    /// Currently, there is no way for the client to negotiate resolution,
    /// so the size returned by this method will be enforced.
    async fn size(&mut self) -> DesktopSize;

    /// Return a display updates receiver
    async fn updates(&mut self) -> Result<Box<dyn RdpServerDisplayUpdates>>;

    /// Request a new size for the display
    fn request_layout(&mut self, layout: DisplayControlMonitorLayout) {
        debug!(?layout, "Requesting layout")
    }
}
