use ironrdp::core::{Encode as _, WriteCursor};
use ironrdp::graphics::color_conversion::to_64x64_ycbcr_tile;
use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::graphics::rfx_encode_component;
use ironrdp::pdu::codecs::rfx::{
    self, Block, ChannelsPdu, CodecChannel, CodecVersionsPdu, EntropyAlgorithm, FrameBeginPdu, FrameEndPdu,
    OperatingMode, Quant, RegionPdu, RfxChannel, RfxRectangle, SyncPdu, TileSetPdu,
};

use super::pattern::{Pattern, render};
use super::{BPP, Image, TILE, TILE_SIZE_BYTES};

pub(super) const FORMAT: PixelFormat = PixelFormat::RgbA32;

/// Video-mode RFX bitstreams using [2.2.2.2.4] TS_RFX_CONTEXT.
/// Decode `init` once, then reuse `frame` for steady-state measurements.
///
/// [2.2.2.2.4]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdprfx/bde1ce78-5d9e-44c1-8a15-5843fa12270a
pub struct Fixture {
    pub width: u16,
    pub height: u16,
    /// Header messages and one tile, used to prime the decoder before timing.
    pub init: Vec<u8>,
    /// A full-screen frame that reuses the established context.
    pub frame: Vec<u8>,
    pub tiles: usize,
    /// Encoded size of `frame`, i.e. what actually crosses the wire.
    pub wire_bytes: usize,
}

impl Fixture {
    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// Encodes `pattern` at `width` x `height` into RFX bitstreams.
pub fn fixture(pattern: Pattern, width: u16, height: u16, algorithm: EntropyAlgorithm) -> Fixture {
    let w = usize::from(width);
    let h = usize::from(height);
    let image = render(pattern, w, h);
    let quant = Quant::default();

    let tiles_x = w.div_ceil(TILE);
    let tiles_y = h.div_ceil(TILE);

    let mut store = vec![0u8; tiles_x * tiles_y * TILE_SIZE_BYTES];
    let mut tiles = Vec::with_capacity(tiles_x * tiles_y);
    let mut rest = store.as_mut_slice();

    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let (buf, tail) = rest.split_at_mut(TILE_SIZE_BYTES);
            rest = tail;

            let image = Image {
                width: w,
                height: h,
                data: &image,
            };
            tiles.push(encode_tile(&image, tx, ty, &quant, algorithm, buf));
        }
    }

    let region = RegionPdu {
        rectangles: vec![RfxRectangle {
            x: 0,
            y: 0,
            width,
            height,
        }],
    };

    let frame = encode_blocks(&[
        Block::CodecChannel(CodecChannel::FrameBegin(FrameBeginPdu {
            index: 1,
            number_of_regions: 1,
        })),
        Block::CodecChannel(CodecChannel::Region(region.clone())),
        Block::CodecChannel(CodecChannel::TileSet(TileSetPdu {
            entropy_algorithm: algorithm,
            quants: vec![quant.clone()],
            tiles: tiles.clone(),
        })),
        Block::CodecChannel(CodecChannel::FrameEnd(FrameEndPdu)),
    ]);

    let init = encode_blocks(&[
        Block::Sync(SyncPdu),
        Block::CodecChannel(CodecChannel::Context(rfx::ContextPdu {
            flags: OperatingMode::empty(),
            entropy_algorithm: algorithm,
        })),
        Block::Channels(ChannelsPdu(vec![RfxChannel {
            width: i16::try_from(width).expect("width fits i16"),
            height: i16::try_from(height).expect("height fits i16"),
        }])),
        Block::CodecVersions(CodecVersionsPdu),
        Block::CodecChannel(CodecChannel::FrameBegin(FrameBeginPdu {
            index: 0,
            number_of_regions: 1,
        })),
        Block::CodecChannel(CodecChannel::Region(region)),
        Block::CodecChannel(CodecChannel::TileSet(TileSetPdu {
            entropy_algorithm: algorithm,
            quants: vec![quant],
            tiles: tiles[..1].to_vec(),
        })),
        Block::CodecChannel(CodecChannel::FrameEnd(FrameEndPdu)),
    ]);

    Fixture {
        width,
        height,
        wire_bytes: frame.len(),
        init,
        frame,
        tiles: tiles.len(),
    }
}

#[expect(clippy::similar_names, reason = "cb and cr are the RFX component names")]
pub(super) fn encode_tile<'a>(
    image: &Image<'_>,
    tx: usize,
    ty: usize,
    quant: &Quant,
    algorithm: EntropyAlgorithm,
    buf: &'a mut [u8],
) -> rfx::Tile<'a> {
    let x = tx * TILE;
    let y = ty * TILE;
    let stride = image.width * BPP;

    let mut comps = [[0i16; TILE * TILE]; 3];
    let [y_plane, cb_plane, cr_plane] = &mut comps;

    to_64x64_ycbcr_tile(
        &image.data[y * stride + x * BPP..],
        u32::try_from((image.width - x).min(TILE)).expect("tile width fits"),
        u32::try_from((image.height - y).min(TILE)).expect("tile height fits"),
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

    let y_len = rfx_encode_component(y_plane, y_buf, quant, algorithm).expect("encode Y");
    let cb_len = rfx_encode_component(cb_plane, cb_buf, quant, algorithm).expect("encode Cb");
    let cr_len = rfx_encode_component(cr_plane, cr_buf, quant, algorithm).expect("encode Cr");

    rfx::Tile {
        y_quant_index: 0,
        cb_quant_index: 0,
        cr_quant_index: 0,
        x: u16::try_from(tx).expect("tile index fits"),
        y: u16::try_from(ty).expect("tile index fits"),
        y_data: &y_buf[..y_len],
        cb_data: &cb_buf[..cb_len],
        cr_data: &cr_buf[..cr_len],
    }
}

fn encode_blocks(blocks: &[Block<'_>]) -> Vec<u8> {
    let size = blocks.iter().map(ironrdp::core::Encode::size).sum();
    let mut out = vec![0u8; size];
    let mut cursor = WriteCursor::new(&mut out);

    for block in blocks {
        block.encode(&mut cursor).expect("encode block");
    }

    out
}
