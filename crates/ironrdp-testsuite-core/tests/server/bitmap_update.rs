use core::num::{NonZeroU16, NonZeroUsize};

use ironrdp_core::decode;
use ironrdp_graphics::rdp6::BitmapStreamDecoder;
use ironrdp_pdu::bitmap::{BitmapData, BitmapUpdateData};
use ironrdp_pdu::rdp::capability_sets::{CmdFlags, LargePointerSupportFlags};
use ironrdp_server::{BitmapUpdate, DesktopSize, DisplayUpdate, PixelFormat, UpdateEncoder, UpdateEncoderCodecs};
use proptest::prelude::*;

/// Encodes `updates` in turn with one encoder, for a client without surface commands, which gets slow-path style bitmap
/// updates, and returns every bitmap update PDU sent for each.
async fn bitmap_updates(updates: &[BitmapUpdate]) -> Vec<Vec<Vec<u8>>> {
    let desktop = DesktopSize {
        width: updates.iter().map(|u| u.x + u.width.get()).max().unwrap(),
        height: updates.iter().map(|u| u.y + u.height.get()).max().unwrap(),
    };
    let mut encoder = UpdateEncoder::new(
        desktop,
        CmdFlags::empty(),
        UpdateEncoderCodecs::new(),
        16 * 1024 * 1024,
        0,
        0,
        LargePointerSupportFlags::empty(),
    )
    .unwrap();
    let mut sent = Vec::new();
    for update in updates {
        let mut fragmenters = encoder.update(DisplayUpdate::Bitmap(update.clone()));
        let mut pdus = Vec::new();
        while let Some(fragmenter) = fragmenters.next().await {
            pdus.push(fragmenter.unwrap().data);
        }
        sent.push(pdus);
    }
    sent
}

fn encode_in_turn(updates: &[BitmapUpdate]) -> Vec<Vec<Vec<u8>>> {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    runtime.block_on(bitmap_updates(updates))
}

fn encode(update: BitmapUpdate) -> Vec<Vec<u8>> {
    encode_in_turn(&[update]).remove(0)
}

/// Calls `f` with every `TS_BITMAP_DATA` of `pdus`.
fn for_each_tile(pdus: &[Vec<u8>], mut f: impl FnMut(&BitmapData<'_>)) {
    for pdu in pdus {
        let data = decode::<BitmapUpdateData<'_>>(pdu).unwrap();
        data.rectangles.iter().for_each(&mut f);
    }
}

/// A `width`x`height` BgrX32 update at (`x`, `y`) whose pixels differ along rows and columns.
fn bitmap(x: u16, y: u16, width: u16, height: u16) -> BitmapUpdate {
    let stride = usize::from(width) * 4;
    BitmapUpdate {
        x,
        y,
        width: NonZeroU16::new(width).unwrap(),
        height: NonZeroU16::new(height).unwrap(),
        format: PixelFormat::BgrX32,
        // 251 is prime, so no row or column of a tile repeats the one before it.
        data: (0..=250u8)
            .cycle()
            .take(stride * usize::from(height))
            .collect::<Vec<u8>>()
            .into(),
        stride: NonZeroUsize::new(stride).unwrap(),
    }
}

/// Asserts that every tile fits a 64x64 planar decoding context.
fn assert_tiles_fit_64x64(pdus: &[Vec<u8>]) {
    for_each_tile(pdus, |tile| {
        assert!(
            tile.width <= 64 && tile.height <= 64,
            "{}x{} bitmap at {:?} is larger than 64x64",
            tile.width,
            tile.height,
            tile.rectangle
        );
    });
}

/// Asserts that every tile's scan width is a multiple of 4.
fn assert_scan_widths_divisible_by_4(pdus: &[Vec<u8>]) {
    for_each_tile(pdus, |tile| {
        let header = tile.compressed_data_header.as_ref().expect("compressed bitmap");
        assert_eq!(
            header.scan_width % 4,
            0,
            "scan width of the bitmap at {:?}",
            tile.rectangle
        );
    });
}

/// Asserts that a client drawing `pdus` paints every pixel of `update` exactly once, with the
/// update's color, and nothing outside it.
fn assert_draws_exactly(pdus: &[Vec<u8>], update: &BitmapUpdate) {
    let (x0, y0) = (update.x, update.y);
    let (x1, y1) = (x0 + update.width.get() - 1, y0 + update.height.get() - 1);
    let width = NonZeroUsize::from(update.width).get();
    let mut drawn = vec![0u8; width * usize::from(update.height.get())];
    let mut decoder = BitmapStreamDecoder::default();
    let mut rgb = Vec::new();
    for_each_tile(pdus, |tile| {
        let r = &tile.rectangle;
        assert!(
            [r.left, r.right].iter().all(|x| (x0..=x1).contains(x))
                && [r.top, r.bottom].iter().all(|y| (y0..=y1).contains(y)),
            "{r:?} draws outside ({x0}, {y0})..=({x1}, {y1})"
        );
        rgb.clear();
        decoder
            .decode_bitmap_stream_to_rgb24(
                tile.bitmap_data,
                &mut rgb,
                usize::from(tile.width),
                usize::from(tile.height),
            )
            .unwrap();
        for y in r.top..=r.bottom {
            // The bitmap's rows are bottom-up; its columns past the rectangle are not drawn.
            let row = usize::from(tile.height) - 1 - usize::from(y - r.top);
            for x in r.left..=r.right {
                let at = (row * usize::from(tile.width) + usize::from(x - r.left)) * 3;
                let (sx, sy) = (usize::from(x - x0), usize::from(y - y0));
                let src = &update.data[sy * update.stride.get() + sx * 4..][..3];
                assert_eq!(
                    rgb[at..at + 3],
                    [src[2], src[1], src[0]],
                    "pixel ({x}, {y}) of the bitmap at {r:?}"
                );
                drawn[sy * width + sx] += 1;
            }
        }
    });
    if let Some(i) = drawn.iter().position(|&n| n != 1) {
        panic!(
            "pixel ({}, {}) is drawn {} times",
            usize::from(x0) + i % width,
            usize::from(y0) + i / width,
            drawn[i]
        );
    }
}

// oracle: reference FreeRDP/FreeRDP@d9991b9fa1b6:libfreerdp/codec/planar.c (freerdp_bitmap_decompress_planar rejects nSrcWidth > maxWidth or nSrcHeight > maxHeight)
// oracle: reference FreeRDP/FreeRDP@d9991b9fa1b6:libfreerdp/core/codecs.c (freerdp_client_codecs_prepare sizes the bitmap update planar context to the desktop at connection)
// oracle: regression FreeRDP/FreeRDP#13668 (a server-side desktop resize left that context at its connection size)
// A bitmap larger than the context disconnects the client; a 64x64 one fits the context of any desktop of at least 64x64.
// oracle: spec MS-RDPBCGR §2.2.9.1.1.3.1.2.2 (destLeft..destBottom are the inclusive bounds drawn)
// oracle: property round-trip (RDP 6.0 planar bitmaps without color loss decode to the pixels encoded)
#[test]
fn bitmap_update_tiles_fit_freerdp_planar_context() {
    let update = bitmap(8, 4, 136, 70);
    let pdus = encode(update.clone());
    assert_tiles_fit_64x64(&pdus);
    assert_draws_exactly(&pdus, &update);
}

// oracle: spec MS-RDPBCGR §2.2.9.1.1.3.1.2.3 (cbScanWidth MUST be divisible by 4)
// oracle: spec MS-RDPBCGR §2.2.9.1.1.3.1.2.2 (destLeft..destBottom are the inclusive bounds drawn, so the padding columns are not)
// oracle: property round-trip (RDP 6.0 planar bitmaps without color loss decode to the pixels encoded)
#[test]
fn bitmap_update_of_odd_width_has_scan_width_divisible_by_4() {
    let update = bitmap(0, 0, 67, 3);
    let pdus = encode(update.clone());
    assert_scan_widths_divisible_by_4(&pdus);
    assert_draws_exactly(&pdus, &update);
}

#[test]
fn bitmap_update_after_a_larger_one_is_sent_as_on_its_own() {
    let small = bitmap(8, 4, 67, 3);
    let sent = encode_in_turn(&[bitmap(0, 0, 136, 70), small.clone()]);
    // oracle: spec MS-RDPBCGR §2.2.9.1.1.3.1.2.2 (destLeft..destBottom are the inclusive bounds drawn)
    // oracle: property round-trip (RDP 6.0 planar bitmaps without color loss decode to the pixels encoded)
    assert_draws_exactly(&sent[1], &small);
    // oracle: mutant a bitmap encoder that sends what its reused buffer kept from a larger update
    assert_eq!(sent[1], encode(small));
}

#[test]
fn bitmap_update_larger_once_encoded_than_twice_its_pixels_draws_exactly() {
    // A one-pixel update's encoding is longer than the buffer first sized for it, twice its 4 bytes of pixels.
    let update = bitmap(0, 0, 1, 1);
    let pdus = encode(update.clone());
    // oracle: spec MS-RDPBCGR §2.2.9.1.1.3.1.2.2 (destLeft..destBottom are the inclusive bounds drawn)
    // oracle: property round-trip (RDP 6.0 planar bitmaps without color loss decode to the pixels encoded)
    assert_draws_exactly(&pdus, &update);
}

/// An offset from 0 to `max`, with both ends as likely as any value in between.
fn offset(max: u16) -> impl Strategy<Value = u16> {
    prop_oneof![Just(0), Just(max), 0..=max]
}

/// Updates of up to 4x4 tiles, anywhere on a 65535x65535 desktop.
fn updates() -> impl Strategy<Value = BitmapUpdate> {
    (1u16..=256, 1u16..=256).prop_flat_map(|(width, height)| {
        (offset(u16::MAX - width), offset(u16::MAX - height)).prop_map(move |(x, y)| bitmap(x, y, width, height))
    })
}

// Same oracles as bitmap_update_tiles_fit_freerdp_planar_context and
// bitmap_update_of_odd_width_has_scan_width_divisible_by_4, over generated sizes and offsets.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn bitmap_update_tiles_fit_and_draw_any_update(update in updates()) {
        let pdus = encode(update.clone());
        assert_tiles_fit_64x64(&pdus);
        assert_scan_widths_divisible_by_4(&pdus);
        assert_draws_exactly(&pdus, &update);
    }

    #[test]
    fn bitmap_updates_in_turn_are_sent_as_on_their_own(updates in prop::collection::vec(updates(), 1..=4)) {
        let sent = encode_in_turn(&updates);
        for (pdus, update) in sent.iter().zip(&updates) {
            // oracle: spec MS-RDPBCGR 2.2.9.1.1.3.1.2.2 (destLeft..destBottom are the inclusive bounds drawn)
            // oracle: property round-trip (RDP 6.0 planar bitmaps without color loss decode to the pixels encoded)
            assert_draws_exactly(pdus, update);
            // oracle: mutant a bitmap encoder that sends what its reused buffer kept from a larger update
            prop_assert_eq!(pdus, &encode(update.clone()));
        }
    }
}
