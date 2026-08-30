use ironrdp_core::{Decode as _, ReadCursor};
use ironrdp_egfx::pdu::{Codec2Type, GfxPdu};
use ironrdp_pdu::codecs::rfx::progressive::{
    ProgressiveBlock, ProgressiveTile, TILE_FLAG_DIFFERENCE, decode_progressive_stream,
};

fn decode(bytes: &[u8]) -> GfxPdu {
    let mut cursor = ReadCursor::new(bytes);
    GfxPdu::decode(&mut cursor).expect("decode Haven fixture")
}

/// Real GDI+ ClearCodec tile from a live winserver2025 session. See
/// `test_data/egfx/haven/README.md` for provenance.
#[test]
fn wts1_64x64_clearcodec_tile() {
    let bytes = include_bytes!("../../test_data/egfx/haven/wts1_64x64_clearcodec_tile.bin");
    let GfxPdu::WireToSurface1(pdu) = decode(bytes) else {
        panic!("expected WireToSurface1");
    };
    let rect = pdu.destination_rectangle;
    assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (128, 192, 192, 256));
    assert_eq!(rect.right - rect.left, 64, "exclusive width");
    assert_eq!(rect.bottom - rect.top, 64, "exclusive height");
}

/// ClearCodec payload carrying an NSCodec-compressed sub-band. See
/// `test_data/egfx/haven/README.md` for why this is still `Codec1Type::ClearCodec`
/// at the `WireToSurface1` layer.
#[test]
fn wts1_576x128_nscodec_subregion() {
    let bytes = include_bytes!("../../test_data/egfx/haven/wts1_576x128_nscodec_subregion.bin");
    let GfxPdu::WireToSurface1(pdu) = decode(bytes) else {
        panic!("expected WireToSurface1");
    };
    let rect = pdu.destination_rectangle;
    assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (64, 64, 640, 192));
    assert_eq!(rect.right - rect.left, 576, "exclusive width");
    assert_eq!(rect.bottom - rect.top, 128, "exclusive height");
}

/// Taskbar-strip update: a wide, short region typical of window-chrome redraws.
#[test]
fn wts1_64x32_taskbar_strip() {
    let bytes = include_bytes!("../../test_data/egfx/haven/wts1_64x32_taskbar_strip.bin");
    let GfxPdu::WireToSurface1(pdu) = decode(bytes) else {
        panic!("expected WireToSurface1");
    };
    let rect = pdu.destination_rectangle;
    assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (0, 768, 64, 800));
    assert_eq!(rect.right - rect.left, 64, "exclusive width");
    assert_eq!(rect.bottom - rect.top, 32, "exclusive height");
}

/// Smallest real-world region observed: an 8x4 micro-tile update.
#[test]
fn wts1_8x4_micro_tile() {
    let bytes = include_bytes!("../../test_data/egfx/haven/wts1_8x4_micro_tile.bin");
    let GfxPdu::WireToSurface1(pdu) = decode(bytes) else {
        panic!("expected WireToSurface1");
    };
    let rect = pdu.destination_rectangle;
    assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (374, 521, 382, 525));
    assert_eq!(rect.right - rect.left, 8, "exclusive width");
    assert_eq!(rect.bottom - rect.top, 4, "exclusive height");
}

/// The `CreateSurface` PDU that establishes the 1280x800 context the four
/// `WireToSurface1` fixtures above were captured against.
#[test]
fn create_surface_1280x800_context() {
    let bytes = include_bytes!("../../test_data/egfx/haven/create_surface_1280x800_context.bin");
    let GfxPdu::CreateSurface(pdu) = decode(bytes) else {
        panic!("expected CreateSurface");
    };
    assert_eq!(pdu.surface_id, 0);
    assert_eq!(pdu.width, 1280);
    assert_eq!(pdu.height, 800);
}

/// WireToSurface2 RemoteFX Progressive fixture with 2 difference-encoded `TILE_FIRST` tiles
/// (`flags & RFX_TILE_DIFFERENCE == 1` per MS-RDPRFX 2.2.2.3.1.2). See
/// `test_data/egfx/haven/README.md` for provenance.
#[test]
fn wts2_64x64_diff_2tiles() {
    let bytes = include_bytes!("../../test_data/egfx/haven/wts2_64x64_diff_2tiles.bin");
    let GfxPdu::WireToSurface2(pdu) = decode(bytes) else {
        panic!("expected WireToSurface2");
    };
    assert_eq!(pdu.surface_id, 0);
    assert_eq!(pdu.codec_context_id, 18);
    assert_eq!(pdu.codec_id, Codec2Type::RemoteFxProgressive);

    let blocks = decode_progressive_stream(&pdu.bitmap_data).expect("decode progressive stream");
    assert_eq!(blocks.len(), 3, "expected FrameBegin, Region, FrameEnd");

    let ProgressiveBlock::Region(region) = &blocks[1] else {
        panic!("expected Region block");
    };
    assert_eq!(region.tiles.len(), 2);

    for (i, tile) in region.tiles.iter().enumerate() {
        let ProgressiveTile::First(first) = tile else {
            panic!("expected TileFirst at index {i}");
        };
        assert_ne!(
            first.flags & TILE_FLAG_DIFFERENCE,
            0,
            "tile at ({}, {}) should have RFX_TILE_DIFFERENCE set",
            first.x_idx,
            first.y_idx
        );
        assert_eq!(first.quality, 0xFF);
        assert_eq!(first.x_idx, 3);
        assert_eq!(first.y_idx, 2 + i as u16);
    }
}

/// WireToSurface2 RemoteFX Progressive fixture with 3 difference-encoded `TILE_FIRST` tiles.
#[test]
fn wts2_64x128_diff_3tiles() {
    let bytes = include_bytes!("../../test_data/egfx/haven/wts2_64x128_diff_3tiles.bin");
    let GfxPdu::WireToSurface2(pdu) = decode(bytes) else {
        panic!("expected WireToSurface2");
    };
    assert_eq!(pdu.surface_id, 0);
    assert_eq!(pdu.codec_context_id, 24);
    assert_eq!(pdu.codec_id, Codec2Type::RemoteFxProgressive);

    let blocks = decode_progressive_stream(&pdu.bitmap_data).expect("decode progressive stream");
    let ProgressiveBlock::Region(region) = &blocks[1] else {
        panic!("expected Region block");
    };
    assert_eq!(region.tiles.len(), 3);

    for (i, tile) in region.tiles.iter().enumerate() {
        let ProgressiveTile::First(first) = tile else {
            panic!("expected TileFirst at index {i}");
        };
        assert_ne!(
            first.flags & TILE_FLAG_DIFFERENCE,
            0,
            "tile at ({}, {}) should have RFX_TILE_DIFFERENCE set",
            first.x_idx,
            first.y_idx
        );
        assert_eq!(first.quality, 0xFF);
        assert_eq!(first.x_idx, 3);
        assert_eq!(first.y_idx, 2 + i as u16);
    }
}

/// WireToSurface2 RemoteFX Progressive fixture with a 9-tile vertical strip (x=19, y=3..=11)
/// of difference-encoded tiles.
#[test]
fn wts2_37x560_diff_column_9tiles() {
    let bytes = include_bytes!("../../test_data/egfx/haven/wts2_37x560_diff_column_9tiles.bin");
    let GfxPdu::WireToSurface2(pdu) = decode(bytes) else {
        panic!("expected WireToSurface2");
    };
    assert_eq!(pdu.surface_id, 0);
    assert_eq!(pdu.codec_context_id, 7);
    assert_eq!(pdu.codec_id, Codec2Type::RemoteFxProgressive);

    let blocks = decode_progressive_stream(&pdu.bitmap_data).expect("decode progressive stream");
    let ProgressiveBlock::Region(region) = &blocks[1] else {
        panic!("expected Region block");
    };
    assert_eq!(region.tiles.len(), 9);

    for (i, tile) in region.tiles.iter().enumerate() {
        let ProgressiveTile::First(first) = tile else {
            panic!("expected TileFirst at index {i}");
        };
        assert_ne!(
            first.flags & TILE_FLAG_DIFFERENCE,
            0,
            "tile at ({}, {}) should have RFX_TILE_DIFFERENCE set",
            first.x_idx,
            first.y_idx
        );
        assert_eq!(first.quality, 0xFF);
        assert_eq!(first.x_idx, 19);
        assert_eq!(first.y_idx, 3 + i as u16);
    }
}

/// WireToSurface2 RemoteFX Progressive mixed fixture: 25 tiles with 16 base tiles and 9
/// difference-encoded tiles at coarse quality (0x00) for progressive refinement.
#[test]
fn wts2_progressive_tile_first_mixed_25tiles() {
    let bytes = include_bytes!("../../test_data/egfx/haven/wts2_progressive_tile_first_mixed_25tiles.bin");
    let GfxPdu::WireToSurface2(pdu) = decode(bytes) else {
        panic!("expected WireToSurface2");
    };
    assert_eq!(pdu.surface_id, 0);
    assert_eq!(pdu.codec_context_id, 3);
    assert_eq!(pdu.codec_id, Codec2Type::RemoteFxProgressive);

    let blocks = decode_progressive_stream(&pdu.bitmap_data).expect("decode progressive stream");
    let ProgressiveBlock::Region(region) = &blocks[1] else {
        panic!("expected Region block");
    };
    assert_eq!(region.tiles.len(), 25);

    let diff_count = region
        .tiles
        .iter()
        .filter(|t| match t {
            ProgressiveTile::First(f) => (f.flags & TILE_FLAG_DIFFERENCE) != 0,
            _ => false,
        })
        .count();

    assert_eq!(diff_count, 9, "expected exactly 9 difference-encoded tiles");
}

/// Verify that decoding a difference fixture without a prior retained tile reference
/// returns `MissingTileReference` as required by MS-RDPRFX 3.1.8.1.7.1 and #1698.
#[test]
fn wts2_diff_requires_retained_reference() {
    use ironrdp_graphics::progressive::{ProgressiveDecodeError, ProgressiveDecoder};

    let bytes = include_bytes!("../../test_data/egfx/haven/wts2_64x64_diff_2tiles.bin");
    let GfxPdu::WireToSurface2(pdu) = decode(bytes) else {
        panic!("expected WireToSurface2");
    };

    use ironrdp_pdu::codecs::rfx::RfxRectangle;
    use ironrdp_pdu::codecs::rfx::progressive::{
        ProgressiveBlock, ProgressiveContextPdu, ProgressiveFrameBeginPdu, ProgressiveFrameEndPdu, ProgressiveRegion,
        ProgressiveSyncPdu, encode_progressive_stream,
    };

    let init_blocks = [
        ProgressiveBlock::Sync(ProgressiveSyncPdu),
        ProgressiveBlock::Context(ProgressiveContextPdu {
            context_id: 0,
            tile_size: 0x40,
            flags: 0,
        }),
        ProgressiveBlock::FrameBegin(ProgressiveFrameBeginPdu {
            frame_index: 0,
            region_count: 1,
        }),
        ProgressiveBlock::Region(ProgressiveRegion {
            tile_size: 0x40,
            rects: vec![RfxRectangle {
                x: 0,
                y: 0,
                width: 64,
                height: 64,
            }],
            quant_vals: vec![],
            quant_prog_vals: vec![],
            flags: 0,
            tiles: vec![],
        }),
        ProgressiveBlock::FrameEnd(ProgressiveFrameEndPdu),
    ];
    let init_stream = encode_progressive_stream(&init_blocks).unwrap();
    let mut decoder = ProgressiveDecoder::new();
    decoder
        .decode_bitmap(0, 0, 1280, 800, &init_stream)
        .expect("context initialization should succeed");

    let res = decoder.decode_bitmap(pdu.surface_id, pdu.codec_context_id, 1280, 800, &pdu.bitmap_data);
    assert!(matches!(
        res,
        Err(ProgressiveDecodeError::MissingTileReference { x_idx: 3, y_idx: 2 })
    ));
}
