//! [`EgfxUpdate`] — the single `Send` message type carried from the EGFX DVC handler to the render
//! loop. One enum on one mpsc channel preserves the server's PDU ordering (FIFO), which matters for
//! surface lifecycle (create → map → update → delete) and frame markers.

use ironrdp::pdu::geometry::ExclusiveRectangle;

/// An EGFX event forwarded from [`super::WebGfxHandler`] (DVC side, `Send`) to the render loop.
///
/// Pixel/bitstream payloads are owned `Vec<u8>` because the borrowed slices handed to the handler
/// alias the EGFX client's reused zgfx decompression buffer; copying at the channel boundary is the
/// unavoidable cost of crossing into the render loop.
#[derive(Debug)]
pub(crate) enum EgfxUpdate {
    /// Server reset the graphics output buffer to `width`×`height`.
    ResetGraphics { width: u32, height: u32 },
    /// A surface was created (its own dimensions, independent of the output).
    SurfaceCreated { id: u16, width: u16, height: u16 },
    /// A surface was mapped to an output position; `origin` is its top-left in output coordinates.
    SurfaceMapped { id: u16, origin_x: u32, origin_y: u32 },
    /// A surface was deleted.
    SurfaceDeleted { id: u16 },
    /// Decoded RGBA pixels (uncompressed / non-AVC codecs) for a **surface-local** `dst` rectangle.
    Bitmap {
        surface_id: u16,
        dst: ExclusiveRectangle,
        width: u16,
        height: u16,
        data: Vec<u8>,
    },
    /// Raw AVC420 H.264 bitstream (AVC format: 4-byte BE length-prefixed NALs) for a surface-local
    /// `dst`; decoded asynchronously via WebCodecs on the render-loop side.
    Avc420 {
        surface_id: u16,
        dst: ExclusiveRectangle,
        bitstream: Vec<u8>,
    },
    /// A logical frame finished (`EndFrame`); a good point to present what has accumulated.
    FrameComplete { frame_id: u32 },
    /// The EGFX channel closed.
    Close,
}
