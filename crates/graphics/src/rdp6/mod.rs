//! This module provides the RDP6 bitmap decoder implementation

pub(crate) mod bitmap_stream;
pub(crate) mod rle;

pub use bitmap_stream::BitmapStreamDecoder;
pub use rle::RleError;
