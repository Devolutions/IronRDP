use ironrdp::graphics::color_conversion::to_64x64_ycbcr_tile;
use ironrdp::graphics::progressive::encode_first_pass;
use ironrdp::pdu::codecs::rfx::progressive::{
    ComponentCodecQuant, FLAG_DWT_REDUCE_EXTRAPOLATE, ProgressiveBlock, ProgressiveContextPdu,
    ProgressiveFrameBeginPdu, ProgressiveFrameEndPdu, ProgressiveRegion, ProgressiveSyncPdu, ProgressiveTile,
    TileSimple, encode_progressive_stream,
};
use ironrdp::pdu::codecs::rfx::{Quant, RfxRectangle};

use super::fixture::FORMAT;
use super::pattern::{Pattern, render};
use super::{BPP, TILE, TILE_SIZE_BYTES};

/// The classic RemoteFX encoder scales samples up by 32 (11.5 fixed point), while Progressive samples are unscaled.
const CLASSIC_SAMPLE_SHIFT: u32 = 5;

/// RemoteFX Progressive bitstreams ([2.2.4.2] RFX_PROGRESSIVE_BITMAP_STREAM) for one full-screen update.
/// Decode `init` once, then reuse `frame` for steady-state measurements.
///
/// [2.2.4.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpegfx/0796cf31-578c-4f47-b4d3-4f005eef63d3
pub struct ProgressiveFixture {
    pub width: u16,
    pub height: u16,
    /// SYNC and CONTEXT blocks, which establish the codec context before timing.
    pub init: Vec<u8>,
    /// One region of original `TILE_SIMPLE` tiles covering the surface, without the context blocks.
    pub frame: Vec<u8>,
}

impl ProgressiveFixture {
    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// Encodes `pattern` at `width` x `height` into a Progressive bitstream of simple tiles.
#[expect(clippy::similar_names, reason = "cb and cr are the RFX component names")]
pub fn progressive_fixture(pattern: Pattern, width: u16, height: u16) -> ProgressiveFixture {
    let w = usize::from(width);
    let h = usize::from(height);
    let image = render(pattern, w, h);
    let stride = w * BPP;

    // The same values as the classic fixtures' `Quant::default()`.
    // Both codecs divide unscaled coefficients by 2^(q - 6), so the quality matches.
    let Quant {
        ll3,
        lh3,
        hl3,
        hh3,
        lh2,
        hl2,
        hh2,
        lh1,
        hl1,
        hh1,
    } = Quant::default();
    let quant = ComponentCodecQuant {
        ll3,
        hl3,
        lh3,
        hh3,
        hl2,
        lh2,
        hh2,
        hl1,
        lh1,
        hh1,
    };

    let tiles_x = w.div_ceil(TILE);
    let tiles_y = h.div_ceil(TILE);

    let mut store = vec![0u8; tiles_x * tiles_y * TILE_SIZE_BYTES];
    let mut tiles = Vec::with_capacity(tiles_x * tiles_y);
    let mut rest = store.as_mut_slice();

    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let (buf, tail) = rest.split_at_mut(TILE_SIZE_BYTES);
            rest = tail;

            let x = tx * TILE;
            let y = ty * TILE;

            let mut planes = [[0i16; TILE * TILE]; 3];
            let [y_plane, cb_plane, cr_plane] = &mut planes;
            to_64x64_ycbcr_tile(
                &image[y * stride + x * BPP..],
                u32::try_from((w - x).min(TILE)).expect("tile width fits"),
                u32::try_from((h - y).min(TILE)).expect("tile height fits"),
                u32::try_from(stride).expect("stride fits"),
                FORMAT,
                y_plane,
                cb_plane,
                cr_plane,
            )
            .expect("ycbcr conversion");

            let third = buf.len() / 3;
            let (y_buf, tail) = buf.split_at_mut(third);
            let (cb_buf, cr_buf) = tail.split_at_mut(third);

            let [y_data, cb_data, cr_data] =
                [(y_buf, y_plane), (cb_buf, cb_plane), (cr_buf, cr_plane)].map(|(out, plane)| {
                    for sample in plane.iter_mut() {
                        *sample >>= CLASSIC_SAMPLE_SHIFT;
                    }
                    let len = encode_first_pass(plane, out, &quant, &ComponentCodecQuant::LOSSLESS, true)
                        .expect("encode component");
                    &out[..len]
                });

            tiles.push(ProgressiveTile::Simple(TileSimple {
                quant_idx_y: 0,
                quant_idx_cb: 0,
                quant_idx_cr: 0,
                x_idx: u16::try_from(tx).expect("tile index fits"),
                y_idx: u16::try_from(ty).expect("tile index fits"),
                flags: 0,
                y_data,
                cb_data,
                cr_data,
                tail_data: &[],
            }));
        }
    }

    let frame = encode_progressive_stream(&[
        ProgressiveBlock::FrameBegin(ProgressiveFrameBeginPdu {
            frame_index: 0,
            region_count: 1,
        }),
        ProgressiveBlock::Region(ProgressiveRegion {
            tile_size: u8::try_from(TILE).expect("tile size fits"),
            rects: vec![RfxRectangle {
                x: 0,
                y: 0,
                width,
                height,
            }],
            quant_vals: vec![quant],
            quant_prog_vals: Vec::new(),
            flags: FLAG_DWT_REDUCE_EXTRAPOLATE,
            tiles,
        }),
        ProgressiveBlock::FrameEnd(ProgressiveFrameEndPdu),
    ])
    .expect("encode frame");

    let init = encode_progressive_stream(&[
        ProgressiveBlock::Sync(ProgressiveSyncPdu),
        ProgressiveBlock::Context(ProgressiveContextPdu {
            context_id: 0,
            tile_size: u16::try_from(TILE).expect("tile size fits"),
            flags: FLAG_DWT_REDUCE_EXTRAPOLATE,
        }),
    ])
    .expect("encode context");

    ProgressiveFixture {
        width,
        height,
        init,
        frame,
    }
}
