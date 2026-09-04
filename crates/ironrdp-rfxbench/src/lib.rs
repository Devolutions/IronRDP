//! Fixtures and a driver for benchmarking the RemoteFX (RFX) *decode* path.
//!
//! This crate builds realistic RFX bitstreams with the encoder and
//! then measures decoding them.
//!
//! The same code is built for `wasm32-unknown-unknown` (see `wasm/run.mjs`) so
//! the browser target can be measured with the identical workload.

#![allow(unused_crate_dependencies)] // Used by benches.

use ironrdp_core::{Encode as _, WriteCursor};
use ironrdp_graphics::color_conversion::to_64x64_ycbcr_tile;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::rfx_encode_component;
use ironrdp_pdu::codecs::rfx::{
    self, Block, ChannelsPdu, CodecChannel, CodecVersionsPdu, EntropyAlgorithm, FrameBeginPdu, FrameEndPdu,
    OperatingMode, Quant, RegionPdu, RfxChannel, RfxRectangle, SyncPdu, TileSetPdu,
};
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::rfx::DecodingContext;

pub const TILE: usize = 64;

/// Bytes per pixel of the source and decoded images.
const BPP: usize = 4;

const FORMAT: PixelFormat = PixelFormat::RgbA32;

/// Synthetic screen content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pattern {
    /// Flat-ish desktop: solid background, a few windows, borders, a taskbar.
    /// The common case for an idle or lightly-used session.
    Desktop,
    /// Dense small text on white. Realistic worst case for real workloads
    /// (a terminal, an editor, a spreadsheet) and the heaviest on RLGR.
    Text,
    /// Smooth 2D gradient: almost everything quantizes away. Best case.
    Gradient,
    /// Photographic-ish content: smooth blobs plus fine detail.
    Photo,
    /// Pseudorandom bytes. Not realistic, but exercises the RLGR phase well.
    Noise,
}

impl Pattern {
    pub const ALL: [Self; 5] = [Self::Desktop, Self::Text, Self::Gradient, Self::Photo, Self::Noise];

    pub fn name(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Text => "text",
            Self::Gradient => "gradient",
            Self::Photo => "photo",
            Self::Noise => "noise",
        }
    }

    pub fn from_u32(v: u32) -> Option<Self> {
        Self::ALL.get(usize::try_from(v).ok()?).copied()
    }
}

/// Deterministic xorshift so fixtures are reproducible across runs.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
}

/// Renders `pattern` into a `width * height` RGBA buffer.
pub fn render(pattern: Pattern, width: usize, height: usize) -> Vec<u8> {
    let mut buf = vec![0u8; width * height * BPP];
    let mut rng = Rng(0x1234_5678);

    for y in 0..height {
        for x in 0..width {
            let [r, g, b] = match pattern {
                Pattern::Desktop => desktop_px(x, y, width, height),
                Pattern::Text => text_px(x, y),
                Pattern::Gradient => gradient_px(x, y, width, height),
                Pattern::Photo => photo_px(x, y),
                Pattern::Noise => {
                    let [r, g, b, _] = rng.next().to_le_bytes();
                    [r, g, b]
                }
            };
            let o = (y * width + x) * BPP;
            buf[o] = r;
            buf[o + 1] = g;
            buf[o + 2] = b;
            buf[o + 3] = 0xff;
        }
    }

    buf
}

fn desktop_px(x: usize, y: usize, width: usize, height: usize) -> [u8; 3] {
    // Taskbar.
    if y + 40 >= height {
        return if y + 40 == height {
            [0x50, 0x50, 0x50]
        } else {
            [0x20, 0x20, 0x20]
        };
    }

    // Two overlapping windows with title bars, borders and a text region.
    for (i, (wx, wy, ww, wh)) in [(60usize, 50usize, 700usize, 500usize), (420, 260, 760, 560)]
        .into_iter()
        .enumerate()
    {
        if x >= wx && x < (wx + ww).min(width) && y >= wy && y < (wy + wh).min(height) {
            let lx = x - wx;
            let ly = y - wy;
            if ly < 2 || lx < 2 || lx + 2 >= ww || ly + 2 >= wh {
                return [0x70, 0x70, 0x78]; // border
            }
            if ly < 32 {
                return if i == 0 { [0x1f, 0x5f, 0x9f] } else { [0x2f, 0x2f, 0x3f] }; // title bar
            }
            // Client area: white with text lines in the upper half.
            if ly < wh / 2 {
                return text_px(lx, ly);
            }
            return [0xf4, 0xf4, 0xf4];
        }
    }

    // Desktop background: a very slow gradient, as most themes have.
    let v = 0x30 + (x + y) * 0x20 / (width + height);
    [channel(v / 3), channel(v / 2), channel(v)]
}

/// Narrow a computed channel value to a byte.
fn channel(v: usize) -> u8 {
    u8::try_from(v).unwrap_or(u8::MAX)
}

/// Simulates a text-like image: dense small (7-pixel) glyphs on a solid background.
fn text_px(x: usize, y: usize) -> [u8; 3] {
    let line = y / 13;
    let row = y % 13;
    if row >= 9 {
        return [0xff, 0xff, 0xff]; // leading between lines
    }
    let cell = x / 7;
    let col = x % 7;
    if col >= 6 {
        return [0xff, 0xff, 0xff]; // inter-character gap
    }
    // Deterministic per-glyph bitmap.
    let h = (cell.wrapping_mul(2_654_435_761) ^ line.wrapping_mul(40_503) ^ row.wrapping_mul(2_246_822_519)) >> 7;
    if h & 1 != 0 || (h & 6) == 6 {
        [0x11, 0x11, 0x11]
    } else {
        [0xff, 0xff, 0xff]
    }
}

fn gradient_px(x: usize, y: usize, width: usize, height: usize) -> [u8; 3] {
    [
        channel(x * 255 / width.max(1)),
        channel(y * 255 / height.max(1)),
        channel((x + y) * 255 / (width + height).max(1)),
    ]
}

fn photo_px(x: usize, y: usize) -> [u8; 3] {
    // Cheap integer simulation of a photo with smooth blobs and fine detail.
    let a = (x * x + y * y) >> 6;
    let b = (x * 3 + y * 7) >> 1;
    let c = (x ^ y) & 0x1f;
    [
        channel(a % 200 + 28),
        channel(b % 180 + 40),
        channel((a / 3 + b / 5 + c) % 210 + 20),
    ]
}

/// An RFX bitstream pair for one screen: the header/first frame that primes the
/// decoder's context, and a steady-state frame with every tile.
pub struct Fixture {
    pub pattern: Pattern,
    pub width: u16,
    pub height: u16,
    pub algorithm: EntropyAlgorithm,
    /// Sync + Context + Channels + CodecVersions + a one-tile frame.
    pub init: Vec<u8>,
    /// FrameBegin + Region + TileSet(all tiles) + FrameEnd, as a server sends
    /// once the context is established.
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

    // RLGR does not guarantee compression; a noise tile can expand.
    let mut store = vec![0u8; tiles_x * tiles_y * 3 * TILE * TILE * 2];
    let mut tiles = Vec::with_capacity(tiles_x * tiles_y);
    let mut rest = store.as_mut_slice();

    for ty in 0..tiles_y {
        for tx in 0..tiles_x {
            let (buf, tail) = rest.split_at_mut(3 * TILE * TILE * 2);
            rest = tail;
            tiles.push(encode_tile(&image, w, h, tx, ty, &quant, algorithm, buf));
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
            flags: OperatingMode::IMAGE_MODE,
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
        pattern,
        width,
        height,
        algorithm,
        wire_bytes: frame.len(),
        init,
        frame,
        tiles: tiles.len(),
    }
}

#[expect(clippy::too_many_arguments, reason = "internal fixture helper")]
#[expect(clippy::similar_names, reason = "cb and cr are the RFX component names")]
fn encode_tile<'a>(
    image: &[u8],
    width: usize,
    height: usize,
    tx: usize,
    ty: usize,
    quant: &Quant,
    algorithm: EntropyAlgorithm,
    buf: &'a mut [u8],
) -> rfx::Tile<'a> {
    let x = tx * TILE;
    let y = ty * TILE;
    let stride = width * BPP;

    let mut comps = [[0i16; TILE * TILE], [0i16; TILE * TILE], [0i16; TILE * TILE]];
    let [y_plane, cb_plane, cr_plane] = &mut comps;

    to_64x64_ycbcr_tile(
        &image[y * stride + x * BPP..],
        u32::try_from((width - x).min(TILE)).expect("tile width fits"),
        u32::try_from((height - y).min(TILE)).expect("tile height fits"),
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
    let size: usize = blocks.iter().map(ironrdp_core::Encode::size).sum();
    let mut out = vec![0u8; size];
    let mut cursor = WriteCursor::new(&mut out);
    for block in blocks {
        block.encode(&mut cursor).expect("encode block");
    }
    let pos = cursor.pos();
    out.truncate(pos);
    out
}

/// Holds the decoder state across frames, as a live session does.
pub struct Driver {
    context: DecodingContext,
    image: DecodedImage,
    destination: InclusiveRectangle,
}

impl Driver {
    /// Builds a driver and primes it with the fixture's header frame.
    pub fn new(fixture: &Fixture) -> Self {
        let mut this = Self {
            context: DecodingContext::new(),
            image: DecodedImage::new(FORMAT, fixture.width, fixture.height),
            destination: InclusiveRectangle {
                left: 0,
                top: 0,
                right: fixture.width - 1,
                bottom: fixture.height - 1,
            },
        };
        this.decode(&fixture.init);
        this
    }

    /// Decodes one frame and returns a checksum of the touched region, so the
    /// optimizer cannot discard the work.
    pub fn decode(&mut self, frame: &[u8]) -> u64 {
        let mut cursor = ironrdp_core::ReadCursor::new(frame);
        let (_frame_id, rect) = self
            .context
            .decode(&mut self.image, &self.destination, &mut cursor)
            .expect("decode frame");

        // Sample the updated region rather than hashing every byte: enough to
        // keep the decode live without the checksum itself showing up in the
        // measurement.
        let data = self.image.data();
        let stride = self.image.stride();
        let mut sum = u64::from(rect.left) ^ (u64::from(rect.bottom) << 16);
        let mut row = usize::from(rect.top);
        while row <= usize::from(rect.bottom) {
            let off = row * stride + usize::from(rect.left) * BPP;
            if off < data.len() {
                sum = sum.wrapping_mul(0x0100_0000_01b3) ^ u64::from(data[off]);
            }
            row += 16;
        }
        sum
    }

    pub fn image(&self) -> &DecodedImage {
        &self.image
    }
}

/// Entropy-coded data for a single tile, for benchmarking the decode stages in
/// isolation.
pub struct StageFixture {
    pub quant: Quant,
    pub algorithm: EntropyAlgorithm,
    /// Y, Cb and Cr entropy-coded planes, exactly as they arrive on the wire.
    pub encoded: [Vec<u8>; 3],
}

impl StageFixture {
    /// Runs the entropy + subband + dequant + DWT chain, returning the three
    /// spatial-domain planes that `ycbcr_to_rgba` consumes.
    pub fn planes(&self) -> [Vec<i16>; 3] {
        let mut temp = vec![0i16; TILE * TILE];
        core::array::from_fn(|i| {
            let mut out = vec![0i16; TILE * TILE];
            ironrdp_graphics::rlgr::decode(self.algorithm, &self.encoded[i], &mut out).expect("rlgr");
            ironrdp_graphics::subband_reconstruction::decode(&mut out[4032..]);
            ironrdp_graphics::quantization::decode(&mut out, &self.quant);
            ironrdp_graphics::dwt::decode(&mut out, &mut temp);
            out
        })
    }
}

/// Encodes one representative tile of `pattern` and returns its three planes.
pub fn stage_fixture(pattern: Pattern, algorithm: EntropyAlgorithm) -> StageFixture {
    // A 1920x1080 screen with the tile taken from the middle of the image, where
    // every pattern has content.
    const W: usize = 1920;
    const H: usize = 1080;
    let image = render(pattern, W, H);
    let quant = Quant::default();

    let mut buf = vec![0u8; 3 * TILE * TILE * 2];
    let tile = encode_tile(&image, W, H, 8, 6, &quant, algorithm, &mut buf);
    let encoded = [tile.y_data.to_vec(), tile.cb_data.to_vec(), tile.cr_data.to_vec()];

    StageFixture {
        quant,
        algorithm,
        encoded,
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm;
