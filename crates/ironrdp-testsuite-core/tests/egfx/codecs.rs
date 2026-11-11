//! RemoteFX (`RDPGFX_CODECID_CAVIDEO`) and Alpha (`RDPGFX_CODECID_ALPHA`) commands through the
//! client, from encoded PDU to composited output.

use std::sync::{Arc, Mutex};

use ironrdp_core::{WriteCursor, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_egfx::client::{BitmapUpdate, GraphicsPipelineClient, GraphicsPipelineHandler};
use ironrdp_egfx::pdu::{
    CapabilitiesConfirmPdu, CapabilitiesV8Flags, CapabilitySet, Codec1Type, CreateSurfacePdu, EndFramePdu, GfxPdu,
    MapSurfaceToOutputPdu, PixelFormat, ResetGraphicsPdu, StartFramePdu, Timestamp, WireToSurface1Pdu,
};
use ironrdp_graphics::color_conversion::to_64x64_ycbcr_tile;
use ironrdp_graphics::image_processing::PixelFormat as ImagePixelFormat;
use ironrdp_graphics::rfx_encode_component;
use ironrdp_graphics::zgfx::wrap_uncompressed;
use ironrdp_pdu::codecs::rfx::{
    Block, ChannelsPdu, CodecChannel, CodecVersionsPdu, ContextPdu, EntropyAlgorithm, FrameBeginPdu, FrameEndPdu,
    OperatingMode, Quant, RegionPdu, RfxChannel, RfxRectangle, SyncPdu, Tile, TileSetPdu,
};
use ironrdp_pdu::geometry::ExclusiveRectangle;

// ============================================================================
// Helpers
// ============================================================================

/// Codec, destination rectangle and RGBA pixels of a handler update.
type Update = (Codec1Type, ExclusiveRectangle, Vec<u8>);

#[derive(Clone, Default)]
struct Updates(Arc<Mutex<Vec<Update>>>);

impl GraphicsPipelineHandler for Updates {
    fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
        self.0.lock().unwrap().push((
            update.codec_id,
            update.destination_rectangle.clone(),
            update.data.clone(),
        ));
    }
}

const TIMESTAMP: Timestamp = Timestamp {
    milliseconds: 0,
    seconds: 0,
    minutes: 0,
    hours: 0,
};

fn send(client: &mut GraphicsPipelineClient, pdu: GfxPdu) {
    client
        .process(0, &wrap_uncompressed(&encode_vec(&pdu).unwrap()))
        .expect("client processes the PDU");
}

fn rect(left: u16, top: u16, right: u16, bottom: u16) -> ExclusiveRectangle {
    ExclusiveRectangle {
        left,
        top,
        right,
        bottom,
    }
}

/// A client with surface 1 of `width` x `height` mapped at the output origin.
fn client_with_surface(width: u16, height: u16) -> (GraphicsPipelineClient, Updates) {
    let updates = Updates::default();
    let mut client = GraphicsPipelineClient::new(Box::new(updates.clone()), None);
    send(
        &mut client,
        GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu::from_typed(&CapabilitySet::V8 {
            flags: CapabilitiesV8Flags::THIN_CLIENT,
        })),
    );
    send(
        &mut client,
        GfxPdu::ResetGraphics(ResetGraphicsPdu {
            width: u32::from(width),
            height: u32::from(height),
            monitors: vec![],
        }),
    );
    send(
        &mut client,
        GfxPdu::CreateSurface(CreateSurfacePdu {
            surface_id: 1,
            width,
            height,
            pixel_format: PixelFormat::ARgb,
        }),
    );
    send(
        &mut client,
        GfxPdu::MapSurfaceToOutput(MapSurfaceToOutputPdu {
            surface_id: 1,
            output_origin_x: 0,
            output_origin_y: 0,
        }),
    );
    (client, updates)
}

/// Run `commands` inside one frame and return the whole output as RGBA, composed from the
/// drained deltas over a zeroed `width` x `height` buffer.
fn render_frame(client: &mut GraphicsPipelineClient, width: u16, height: u16, commands: Vec<GfxPdu>) -> Vec<u8> {
    let _ = client.drain_output();
    send(
        client,
        GfxPdu::StartFrame(StartFramePdu {
            timestamp: TIMESTAMP,
            frame_id: 1,
        }),
    );
    for command in commands {
        send(client, command);
    }
    send(client, GfxPdu::EndFrame(EndFramePdu { frame_id: 1 }));

    let stride = usize::from(width) * 4;
    let mut output = vec![0; stride * usize::from(height)];
    for update in client.drain_output() {
        let row_bytes = usize::from(update.region.right - update.region.left) * 4;
        for (row, src) in update.data.chunks_exact(row_bytes).enumerate() {
            let start = (usize::from(update.region.top) + row) * stride + usize::from(update.region.left) * 4;
            output[start..start + row_bytes].copy_from_slice(src);
        }
    }
    output
}

fn pixel(output: &[u8], width: u16, x: u16, y: u16) -> [u8; 4] {
    let offset = (usize::from(y) * usize::from(width) + usize::from(x)) * 4;
    output[offset..offset + 4].try_into().unwrap()
}

fn wire(codec_id: Codec1Type, destination_rectangle: ExclusiveRectangle, bitmap_data: Vec<u8>) -> GfxPdu {
    GfxPdu::WireToSurface1(WireToSurface1Pdu {
        surface_id: 1,
        codec_id,
        pixel_format: PixelFormat::ARgb,
        destination_rectangle,
        bitmap_data,
    })
}

fn assert_close(actual: [u8; 4], expected: [u8; 4]) {
    for (a, e) in actual.iter().zip(expected) {
        assert!(a.abs_diff(e) <= 3, "{actual:?} is not close to {expected:?}");
    }
}

/// Encode an RGBA image as a RemoteFX message the way a server does: one 64x64 tile per grid
/// cell, each component through the forward DWT, quantization and RLGR1.
fn encode_rfx(rgba: &[u8], width: u16, height: u16, rectangles: Vec<RfxRectangle>, headers: bool) -> Vec<u8> {
    let quant = Quant {
        ll3: 6,
        lh3: 6,
        hl3: 6,
        hh3: 6,
        lh2: 6,
        hl2: 6,
        hh2: 6,
        lh1: 6,
        hl1: 6,
        hh1: 6,
    };
    let algorithm = EntropyAlgorithm::Rlgr1;
    let stride = usize::from(width) * 4;

    let mut encoded = Vec::new();
    for tile_y in 0..height.div_ceil(64) {
        for tile_x in 0..width.div_ceil(64) {
            let x = usize::from(tile_x) * 64;
            let y = usize::from(tile_y) * 64;
            let tile_width = (usize::from(width) - x).min(64);
            let tile_height = (usize::from(height) - y).min(64);
            let (mut luma, mut cb, mut cr) = ([0i16; 4096], [0i16; 4096], [0i16; 4096]);
            to_64x64_ycbcr_tile(
                &rgba[y * stride + x * 4..],
                u32::try_from(tile_width).unwrap(),
                u32::try_from(tile_height).unwrap(),
                u32::try_from(stride).unwrap(),
                ImagePixelFormat::RgbA32,
                &mut luma,
                &mut cb,
                &mut cr,
            )
            .unwrap();
            let component = |input: &mut [i16; 4096]| {
                let mut out = vec![0; 8192];
                let len = rfx_encode_component(input, &mut out, &quant, algorithm).unwrap();
                out.truncate(len);
                out
            };
            encoded.push((
                tile_x,
                tile_y,
                component(&mut luma),
                component(&mut cb),
                component(&mut cr),
            ));
        }
    }

    let tiles = encoded
        .iter()
        .map(|(x, y, y_data, cb_data, cr_data)| Tile {
            y_quant_index: 0,
            cb_quant_index: 0,
            cr_quant_index: 0,
            x: *x,
            y: *y,
            y_data,
            cb_data,
            cr_data,
        })
        .collect();

    let mut blocks = Vec::new();
    if headers {
        blocks.push(Block::Sync(SyncPdu));
        blocks.push(Block::CodecChannel(CodecChannel::Context(ContextPdu {
            flags: OperatingMode::IMAGE_MODE,
            entropy_algorithm: algorithm,
        })));
        blocks.push(Block::Channels(ChannelsPdu(vec![RfxChannel {
            width: i16::try_from(width).unwrap(),
            height: i16::try_from(height).unwrap(),
        }])));
        blocks.push(Block::CodecVersions(CodecVersionsPdu));
    }
    blocks.push(Block::CodecChannel(CodecChannel::FrameBegin(FrameBeginPdu {
        index: 0,
        number_of_regions: 1,
    })));
    blocks.push(Block::CodecChannel(CodecChannel::Region(RegionPdu { rectangles })));
    blocks.push(Block::CodecChannel(CodecChannel::TileSet(TileSetPdu {
        entropy_algorithm: algorithm,
        quants: vec![quant],
        tiles,
    })));
    blocks.push(Block::CodecChannel(CodecChannel::FrameEnd(FrameEndPdu)));

    let size = blocks.iter().map(ironrdp_core::Encode::size).sum();
    let mut buffer = vec![0; size];
    let mut cursor = WriteCursor::new(&mut buffer);
    for block in &blocks {
        ironrdp_core::Encode::encode(block, &mut cursor).unwrap();
    }
    buffer
}

fn solid(width: u16, height: u16, rgba: [u8; 4]) -> Vec<u8> {
    rgba.repeat(usize::from(width) * usize::from(height))
}

// ============================================================================
// RemoteFX
// ============================================================================

#[test]
fn remotefx_paints_a_multi_tile_image() {
    // oracle: spec MS-RDPEGFX §2.2.2.1 (CAVIDEO rectangles are relative to destRect) and
    // MS-RDPRFX §2.2.2.3.4.1 (a tile sits at xIdx and yIdx times 64); colors are the test input
    const W: u16 = 100;
    const H: u16 = 70;
    // A 2x2 grid of whole tiles, red in the first column and blue in the second. The region
    // rectangle crops it to the 100x70 surface, as a server does for a surface whose size is not
    // a multiple of 64.
    let mut image = Vec::new();
    for _ in 0..128 {
        for x in 0..128 {
            image.extend_from_slice(if x < 64 {
                &[200, 30, 30, 255]
            } else {
                &[60, 90, 160, 255]
            });
        }
    }
    let message = encode_rfx(
        &image,
        128,
        128,
        vec![RfxRectangle {
            x: 0,
            y: 0,
            width: W,
            height: H,
        }],
        true,
    );

    let (mut client, updates) = client_with_surface(W, H);
    let output = render_frame(
        &mut client,
        W,
        H,
        vec![wire(Codec1Type::RemoteFx, rect(0, 0, W, H), message)],
    );

    for (x, y) in [(0, 0), (20, 60), (63, 10), (64, 10), (70, 65), (99, 69)] {
        let expected = if x < 64 { [200, 30, 30, 255] } else { [60, 90, 160, 255] };
        assert_close(pixel(&output, W, x, y), expected);
    }
    let updates = updates.0.lock().unwrap();
    assert!(updates.iter().all(|(codec, _, _)| *codec == Codec1Type::RemoteFx));
    assert_paints_exactly(&updates, &rect(0, 0, W, H));
    assert_eq!(client.dropped_updates(), 0);
}

/// Every update lies inside `area`, no two overlap, and together they cover all of it: each pixel
/// is painted exactly once.
fn assert_paints_exactly(updates: &[Update], area: &ExclusiveRectangle) {
    let size = |r: &ExclusiveRectangle| usize::from(r.right - r.left) * usize::from(r.bottom - r.top);
    for (i, (_, a, _)) in updates.iter().enumerate() {
        assert!(
            area.left <= a.left && a.right <= area.right && area.top <= a.top && a.bottom <= area.bottom,
            "{a:?} outside {area:?}"
        );
        for (_, b, _) in &updates[i + 1..] {
            let overlap = a.left.max(b.left) < a.right.min(b.right) && a.top.max(b.top) < a.bottom.min(b.bottom);
            assert!(!overlap, "{a:?} and {b:?} overlap");
        }
    }
    let painted: usize = updates.iter().map(|(_, r, _)| size(r)).sum();
    assert_eq!(painted, size(area));
}

#[test]
fn overlapping_region_rectangles_paint_each_pixel_once() {
    // oracle: spec MS-RDPRFX §2.2.2.3.3 (the region is the union of its rectangles); one paint per
    // pixel is mutant (a pixel painted once per covering rectangle)
    let square = RfxRectangle {
        x: 0,
        y: 0,
        width: 64,
        height: 64,
    };
    let mut rectangles = vec![square; 1000];
    rectangles.push(RfxRectangle {
        x: 16,
        y: 16,
        width: 32,
        height: 32,
    });
    let message = encode_rfx(&solid(64, 64, [0, 200, 0, 255]), 64, 64, rectangles, true);

    let (mut client, updates) = client_with_surface(64, 64);
    render_frame(
        &mut client,
        64,
        64,
        vec![wire(Codec1Type::RemoteFx, rect(0, 0, 64, 64), message)],
    );

    let updates = updates.0.lock().unwrap();
    assert_paints_exactly(&updates, &rect(0, 0, 64, 64));
    assert_eq!(client.dropped_updates(), 0);
}

#[test]
fn a_tile_listed_twice_is_painted_once_with_its_last_occurrence() {
    // oracle: mutant (a repeated tile decoded and painted once per occurrence, or the first
    // occurrence winning)
    let red = encode_rfx(&solid(64, 64, [200, 30, 30, 255]), 64, 64, vec![], true);
    let blue = encode_rfx(&solid(64, 64, [60, 90, 160, 255]), 64, 64, vec![], false);
    // One frame whose tileset lists the red tile, then the blue one, both at (0, 0).
    let message = two_tiles_at_origin(&red, &blue);

    let (mut client, updates) = client_with_surface(64, 64);
    let output = render_frame(
        &mut client,
        64,
        64,
        vec![wire(Codec1Type::RemoteFx, rect(0, 0, 64, 64), message)],
    );

    let updates = updates.0.lock().unwrap();
    assert_paints_exactly(&updates, &rect(0, 0, 64, 64));
    assert_close(pixel(&output, 64, 10, 10), [60, 90, 160, 255]);
}

/// One RemoteFX frame whose tileset lists the tile of `first`, then the tile of `second`, both at
/// position (0, 0). Both inputs are single-tile messages from [`encode_rfx`].
fn two_tiles_at_origin(first: &[u8], second: &[u8]) -> Vec<u8> {
    use ironrdp_core::ReadCursor;

    fn only_tile<'a>(message: &'a [u8]) -> (Tile<'a>, Quant) {
        let mut src = ReadCursor::new(message);
        while !src.is_empty() {
            if let Block::CodecChannel(CodecChannel::TileSet(tile_set)) = ironrdp_core::decode_cursor(&mut src).unwrap()
            {
                return (tile_set.tiles[0].clone(), tile_set.quants[0].clone());
            }
        }
        panic!("no tileset");
    }
    let (first, quant) = only_tile(first);
    let (second, _) = only_tile(second);

    let blocks = [
        Block::CodecChannel(CodecChannel::FrameBegin(FrameBeginPdu {
            index: 0,
            number_of_regions: 1,
        })),
        Block::CodecChannel(CodecChannel::Region(RegionPdu { rectangles: vec![] })),
        Block::CodecChannel(CodecChannel::TileSet(TileSetPdu {
            entropy_algorithm: EntropyAlgorithm::Rlgr1,
            quants: vec![quant],
            tiles: vec![first, second],
        })),
        Block::CodecChannel(CodecChannel::FrameEnd(FrameEndPdu)),
    ];
    let mut buffer = vec![0; blocks.iter().map(ironrdp_core::Encode::size).sum()];
    let mut cursor = WriteCursor::new(&mut buffer);
    for block in &blocks {
        ironrdp_core::Encode::encode(block, &mut cursor).unwrap();
    }
    buffer
}

#[test]
fn remotefx_tiles_and_rectangles_are_relative_to_the_destination() {
    // oracle: spec MS-RDPEGFX §2.2.2.1 (CAVIDEO rectangles are relative to destRect)
    const W: u16 = 128;
    const H: u16 = 128;
    let message = encode_rfx(
        &solid(64, 64, [0, 200, 0, 255]),
        64,
        64,
        // Only the 30x20 rectangle at (5, 10) of the tile is valid.
        vec![RfxRectangle {
            x: 5,
            y: 10,
            width: 30,
            height: 20,
        }],
        // Headers sent once per connection are optional in later messages.
        false,
    );

    let (mut client, updates) = client_with_surface(W, H);
    let output = render_frame(
        &mut client,
        W,
        H,
        vec![wire(Codec1Type::RemoteFx, rect(40, 50, 104, 114), message)],
    );

    let updates = updates.0.lock().unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].1, rect(45, 60, 75, 80));
    assert_close(pixel(&output, W, 45, 60), [0, 200, 0, 255]);
    assert_close(pixel(&output, W, 74, 79), [0, 200, 0, 255]);
    assert_eq!(pixel(&output, W, 44, 60), [0, 0, 0, 0], "outside the rectangle");
    assert_eq!(pixel(&output, W, 75, 79), [0, 0, 0, 0], "outside the rectangle");
}

#[test]
fn remotefx_tiles_are_clipped_to_the_surface() {
    // oracle: spec MS-RDPEGFX §3.3.5.1 (the bitmap is copied to the target surface)
    let message = encode_rfx(
        &solid(64, 64, [9, 9, 200, 255]),
        64,
        64,
        vec![RfxRectangle {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        }],
        true,
    );

    let (mut client, updates) = client_with_surface(48, 40);
    render_frame(
        &mut client,
        48,
        40,
        vec![wire(Codec1Type::RemoteFx, rect(16, 8, 48, 40), message)],
    );

    let updates = updates.0.lock().unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].1, rect(16, 8, 48, 40));
    assert_eq!(updates[0].2.len(), 32 * 32 * 4);
}

#[test]
fn an_undecodable_remotefx_command_is_dropped_not_fatal() {
    // oracle: mutant (a decode failure that ends the session or paints a partial message)
    let mut message = encode_rfx(
        &solid(64, 64, [1, 2, 3, 255]),
        64,
        64,
        vec![RfxRectangle {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        }],
        false,
    );
    // Cut the message inside its tileset.
    message.truncate(message.len() - 40);

    let (mut client, updates) = client_with_surface(64, 64);
    render_frame(
        &mut client,
        64,
        64,
        vec![wire(Codec1Type::RemoteFx, rect(0, 0, 64, 64), message)],
    );

    assert_eq!(client.dropped_updates(), 1);
    assert!(updates.0.lock().unwrap().is_empty());
}

#[test]
fn remotefx_decodes_the_ms_rdprfx_sample_into_the_surface() {
    // oracle: spec MS-RDPRFX §4.2.2, §4.2.3 and §4.2.4.1 (input messages) and §4.2.4.4 (decoded
    // pixels)
    use crate::session::rfx::{DECODED_IMAGE, ENCODED_MESSAGES};

    let (mut client, _) = client_with_surface(64, 64);
    let output = render_frame(
        &mut client,
        64,
        64,
        vec![wire(
            Codec1Type::RemoteFx,
            rect(0, 0, 64, 64),
            ENCODED_MESSAGES.to_vec(),
        )],
    );

    let expected: Vec<u8> = DECODED_IMAGE
        .chunks_exact(4)
        .flat_map(|bgrx| [bgrx[2], bgrx[1], bgrx[0], 0xFF])
        .collect();
    assert!(output == expected, "the decoded sample differs from MS-RDPRFX 4.2.4.4");
}

#[test]
fn remotefx_paints_every_frame_of_one_command() {
    // oracle: spec MS-RDPRFX §3.1.8.3.1 (a stream may carry several frames, each with its own region)
    // and MS-RDPEGFX §3.3.5.1 (the decoded bitmap is copied to the surface)
    let mut message = encode_rfx(
        &solid(64, 64, [200, 30, 30, 255]),
        64,
        64,
        vec![RfxRectangle {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        }],
        true,
    );
    // The second frame covers the right half only, without repeating the headers.
    message.extend(encode_rfx(
        &solid(128, 64, [60, 90, 160, 255]),
        128,
        64,
        vec![RfxRectangle {
            x: 64,
            y: 0,
            width: 64,
            height: 64,
        }],
        false,
    ));

    let (mut client, _) = client_with_surface(128, 64);
    let output = render_frame(
        &mut client,
        128,
        64,
        vec![wire(Codec1Type::RemoteFx, rect(0, 0, 128, 64), message)],
    );

    assert_eq!(client.dropped_updates(), 0);
    assert_close(pixel(&output, 128, 10, 10), [200, 30, 30, 255]);
    assert_close(pixel(&output, 128, 100, 50), [60, 90, 160, 255]);
}

#[test]
fn remotefx_never_paints_outside_the_destination() {
    // oracle: spec MS-RDPEGFX §2.2.2.1 (destRect bounds the bitmap data)
    const W: u16 = 128;
    const H: u16 = 128;
    // A whole 64x64 tile and region, sent for a 32x32 destination that ends inside the surface.
    let message = encode_rfx(
        &solid(64, 64, [0, 200, 0, 255]),
        64,
        64,
        vec![RfxRectangle {
            x: 0,
            y: 0,
            width: 64,
            height: 64,
        }],
        true,
    );

    let (mut client, updates) = client_with_surface(W, H);
    let output = render_frame(
        &mut client,
        W,
        H,
        vec![wire(Codec1Type::RemoteFx, rect(40, 50, 72, 82), message)],
    );

    let updates = updates.0.lock().unwrap();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].1, rect(40, 50, 72, 82));
    assert_close(pixel(&output, W, 71, 81), [0, 200, 0, 255]);
    assert_eq!(pixel(&output, W, 72, 50), [0, 0, 0, 0], "right of the destination");
    assert_eq!(pixel(&output, W, 40, 82), [0, 0, 0, 0], "below the destination");
}

// ============================================================================
// Alpha
// ============================================================================

/// Paint `color` over the whole surface with an uncompressed command, whose wire format is
/// BGRA.
fn uncompressed(width: u16, height: u16, [r, g, b, a]: [u8; 4]) -> GfxPdu {
    wire(
        Codec1Type::Uncompressed,
        rect(0, 0, width, height),
        [b, g, r, a].repeat(usize::from(width) * usize::from(height)),
    )
}

#[test]
fn raw_alpha_replaces_only_the_alpha_channel() {
    // oracle: spec MS-RDPEGFX §3.3.5.1 (only the alpha channel of the target is updated) and
    // §2.2.4.3 (raw values, top-left to bottom-right)
    let (mut client, updates) = client_with_surface(4, 2);
    // ALPHACODEC_BITMAP_STREAM: alphaSig 0x414C, compressed 0, then one byte per pixel.
    let mut alpha = vec![0x4C, 0x41, 0x00, 0x00];
    alpha.extend_from_slice(&[0, 32, 64, 96, 128, 160, 192, 255]);

    let output = render_frame(
        &mut client,
        4,
        2,
        vec![
            uncompressed(4, 2, [10, 20, 30, 255]),
            wire(Codec1Type::Alpha, rect(0, 0, 4, 2), alpha),
        ],
    );

    assert_eq!(pixel(&output, 4, 0, 0), [10, 20, 30, 0]);
    assert_eq!(pixel(&output, 4, 1, 0), [10, 20, 30, 32]);
    assert_eq!(pixel(&output, 4, 3, 1), [10, 20, 30, 255]);

    let updates = updates.0.lock().unwrap();
    let (codec, region, data) = updates.last().unwrap();
    assert_eq!(*codec, Codec1Type::Alpha);
    assert_eq!(*region, rect(0, 0, 4, 2));
    assert_eq!(&data[4..8], &[10, 20, 30, 32], "the handler sees the composited pixels");
}

#[test]
fn rle_alpha_applies_runs_across_rows_of_a_sub_rectangle() {
    // oracle: spec MS-RDPEGFX §2.2.4.3.1 (each run covers the next runLengthFactor1 pixels) and
    // §3.3.5.1 (only the alpha channel of the target is updated)
    let (mut client, _) = client_with_surface(6, 4);
    // A 4x2 rectangle at (1, 1): a run of 3 transparent pixels, then a run of 5 half-opaque ones
    // that wraps onto the second row.
    let alpha = vec![0x4C, 0x41, 0x01, 0x00, 0x00, 3, 0x80, 5];

    let output = render_frame(
        &mut client,
        6,
        4,
        vec![
            uncompressed(6, 4, [50, 60, 70, 255]),
            wire(Codec1Type::Alpha, rect(1, 1, 5, 3), alpha),
        ],
    );

    assert_eq!(pixel(&output, 6, 0, 1), [50, 60, 70, 255], "left of the rectangle");
    assert_eq!(pixel(&output, 6, 1, 1), [50, 60, 70, 0]);
    assert_eq!(pixel(&output, 6, 3, 1), [50, 60, 70, 0]);
    assert_eq!(pixel(&output, 6, 4, 1), [50, 60, 70, 0x80]);
    assert_eq!(pixel(&output, 6, 1, 2), [50, 60, 70, 0x80]);
    assert_eq!(pixel(&output, 6, 4, 2), [50, 60, 70, 0x80]);
    assert_eq!(pixel(&output, 6, 5, 2), [50, 60, 70, 255], "right of the rectangle");
    assert_eq!(pixel(&output, 6, 1, 3), [50, 60, 70, 255], "below the rectangle");
}

#[test]
fn an_alpha_run_far_larger_than_the_surface_paints_only_the_surface() {
    // oracle: spec MS-RDPEGFX §2.2.4.3.1 (one runLengthFactor3 run covers the whole destination) and
    // §3.3.5.1 (the bitmap is copied to the target surface)
    let (mut client, _) = client_with_surface(4, 4);
    // A 65535x65535 destination on a 4x4 surface, all of it one twelve-byte run.
    let mut alpha = vec![0x4C, 0x41, 0x01, 0x00, 0x40, 0xFF, 0xFF, 0xFF];
    alpha.extend_from_slice(&(65_535u32 * 65_535).to_le_bytes());

    let output = render_frame(
        &mut client,
        4,
        4,
        vec![
            uncompressed(4, 4, [5, 6, 7, 255]),
            wire(Codec1Type::Alpha, rect(0, 0, 65_535, 65_535), alpha),
        ],
    );

    assert_eq!(client.dropped_updates(), 0);
    for (x, y) in [(0, 0), (3, 0), (0, 3), (3, 3)] {
        assert_eq!(pixel(&output, 4, x, y), [5, 6, 7, 0x40]);
    }
}

#[test]
fn an_alpha_command_with_a_bad_signature_is_dropped() {
    // oracle: spec MS-RDPEGFX §2.2.4.3 (alphaSig MUST be 0x414C)
    let (mut client, _) = client_with_surface(2, 1);
    let output = render_frame(
        &mut client,
        2,
        1,
        vec![
            uncompressed(2, 1, [1, 2, 3, 255]),
            wire(Codec1Type::Alpha, rect(0, 0, 2, 1), vec![0x41, 0x4C, 0, 0, 7, 7]),
        ],
    );

    assert_eq!(client.dropped_updates(), 1);
    assert_eq!(pixel(&output, 2, 1, 0), [1, 2, 3, 255]);
}
