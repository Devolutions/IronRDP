use core::num::{NonZeroU16, NonZeroUsize};

use ironrdp_core::decode;
use ironrdp_pdu::bitmap::BitmapUpdateData;
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_pdu::rdp::capability_sets::{CmdFlags, LargePointerSupportFlags};
use ironrdp_server::{BitmapUpdate, DesktopSize, DisplayUpdate, PixelFormat, UpdateEncoder, UpdateEncoderCodecs};

/// One `TS_BITMAP_DATA` of a bitmap update, as the client reads it.
#[derive(Debug)]
struct Tile {
    rectangle: InclusiveRectangle,
    width: u16,
    height: u16,
}

/// Encodes `update` for a client without surface commands, which gets slow-path style bitmap
/// updates, and returns every `TS_BITMAP_DATA` sent for it.
async fn bitmap_tiles(desktop: DesktopSize, update: BitmapUpdate) -> Vec<Tile> {
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
    let mut updates = encoder.update(DisplayUpdate::Bitmap(update));
    let mut tiles = Vec::new();
    while let Some(fragmenter) = updates.next().await {
        let fragmenter = fragmenter.unwrap();
        let data = decode::<BitmapUpdateData<'_>>(&fragmenter.data).unwrap();
        tiles.extend(data.rectangles.iter().map(|r| Tile {
            rectangle: r.rectangle.clone(),
            width: r.width,
            height: r.height,
        }));
    }
    tiles
}

fn bitmap(x: u16, y: u16, width: u16, height: u16) -> BitmapUpdate {
    let stride = usize::from(width) * 4;
    BitmapUpdate {
        x,
        y,
        width: NonZeroU16::new(width).unwrap(),
        height: NonZeroU16::new(height).unwrap(),
        format: PixelFormat::BgrX32,
        data: (0..stride * usize::from(height))
            .map(|i| u8::try_from(i % 251).expect("below 251"))
            .collect::<Vec<u8>>()
            .into(),
        stride: NonZeroUsize::new(stride).unwrap(),
    }
}

/// Asserts that the tiles' destination rectangles draw every pixel of `area` exactly once and
/// nothing outside it.
fn assert_covers_exactly(tiles: &[Tile], area: &InclusiveRectangle) {
    let width = usize::from(area.right - area.left + 1);
    let height = usize::from(area.bottom - area.top + 1);
    let mut drawn = vec![0u8; width * height];
    for tile in tiles {
        let r = &tile.rectangle;
        assert!(
            r.left >= area.left && r.right <= area.right && r.top >= area.top && r.bottom <= area.bottom,
            "{tile:?} draws outside {area:?}"
        );
        for y in r.top..=r.bottom {
            for x in r.left..=r.right {
                drawn[usize::from(y - area.top) * width + usize::from(x - area.left)] += 1;
            }
        }
    }
    assert!(
        drawn.iter().all(|&n| n == 1),
        "tiles do not cover {area:?} exactly once"
    );
}

// oracle: reference FreeRDP/FreeRDP@d9991b9fa1b6:libfreerdp/core/codecs.c (planar context created 64x64)
// oracle: reference FreeRDP/FreeRDP@d9991b9fa1b6:libfreerdp/codec/planar.c (planar_decompress rejects nSrcWidth > maxWidth or nSrcHeight > maxHeight)
// FreeRDP decodes the planar bitmaps of a bitmap update with that context, so a larger one disconnects it.
#[tokio::test]
async fn bitmap_update_tiles_fit_freerdp_planar_context() {
    let tiles = bitmap_tiles(DesktopSize { width: 160, height: 80 }, bitmap(8, 4, 136, 70)).await;
    for tile in &tiles {
        assert!(tile.width <= 64 && tile.height <= 64, "{tile:?} is larger than 64x64");
    }
    // oracle: spec MS-RDPBCGR §2.2.9.1.1.3.1.2.2 (destLeft..destBottom are the inclusive bounds drawn)
    assert_covers_exactly(
        &tiles,
        &InclusiveRectangle {
            left: 8,
            top: 4,
            right: 143,
            bottom: 73,
        },
    );
}
