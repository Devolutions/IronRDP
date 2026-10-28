use ironrdp_core::{ReadCursor, decode, decode_cursor};
use ironrdp_pdu::fast_path::{FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp_pdu::pointer::{ColorPointerAttribute, LargePointerAttribute, PointerAttribute};
use ironrdp_pdu::rdp::capability_sets::{CmdFlags, LargePointerSupportFlags};
use ironrdp_server::{ColorPointer, DesktopSize, DisplayUpdate, RGBAPointer, UpdateEncoder, UpdateEncoderCodecs};

fn test_encoder(pointer_cache_size: u16, flags: LargePointerSupportFlags) -> UpdateEncoder {
    UpdateEncoder::new(
        DesktopSize {
            width: 800,
            height: 600,
        },
        CmdFlags::empty(),
        UpdateEncoderCodecs::default(),
        8 * 1024 * 1024,
        pointer_cache_size,
        20,
        flags,
    )
    .unwrap()
}

// oracle: spec MS-RDPBCGR §2.2.9.1.2.1, §2.2.9.1.2.1.5, and §2.2.9.1.2.1.6
// System pointer updates have a 3-byte TS_FP_UPDATE header and a zero-sized body.
#[rstest::rstest]
#[case::hidden_pointer(DisplayUpdate::HidePointer, UpdateCode::HiddenPointer)]
#[case::default_pointer(DisplayUpdate::DefaultPointer, UpdateCode::DefaultPointer)]
#[tokio::test]
async fn system_pointer_updates_emit_one_empty_frame(#[case] update: DisplayUpdate, #[case] code: UpdateCode) {
    let mut encoder = test_encoder(20, LargePointerSupportFlags::empty());
    let mut updates = encoder.update(update);
    let mut fragmenter = updates.next().await.unwrap().unwrap();
    let mut bytes = [0; 16];
    let length = fragmenter
        .next(&mut bytes)
        .expect("system pointer frame must not be dropped");
    let mut cursor = ReadCursor::new(&bytes[..length]);
    let header = decode_cursor::<FastPathHeader>(&mut cursor).unwrap();
    let update = decode_cursor::<FastPathUpdatePdu<'_>>(&mut cursor).unwrap();
    assert_eq!(header.data_length, 3);
    assert_eq!(update.update_code, code);
    assert_eq!(update.fragmentation, Fragmentation::Single);
    assert!(update.data.is_empty());
    assert!(cursor.is_empty());
    assert!(fragmenter.next(&mut bytes).is_none());
    assert!(updates.next().await.is_none());
}

async fn assert_encoded_pointer_update(
    flags: LargePointerSupportFlags,
    update: DisplayUpdate,
    expected_code: UpdateCode,
    expected_bpp: u16,
    expected_width: u16,
    expected_height: u16,
) {
    let mut encoder = test_encoder(20, flags);
    let mut updates = encoder.update(update);
    let mut fragmenter = updates
        .next()
        .await
        .expect("supported pointer must not be dropped")
        .unwrap();
    let mut bytes = [0; 4096];
    let length = fragmenter.next(&mut bytes).expect("pointer frame");
    let mut cursor = ReadCursor::new(&bytes[..length]);
    let _ = decode_cursor::<FastPathHeader>(&mut cursor).unwrap();
    let pdu = decode_cursor::<FastPathUpdatePdu<'_>>(&mut cursor).unwrap();
    assert_eq!(pdu.update_code, expected_code);
    match pdu.update_code {
        UpdateCode::ColorPointer => {
            let attr = decode::<ColorPointerAttribute<'_>>(pdu.data).unwrap();
            assert_eq!((attr.width, attr.height), (expected_width, expected_height));
            assert!(!attr.and_mask.is_empty());
        }
        UpdateCode::NewPointer => {
            let attr = decode::<PointerAttribute<'_>>(pdu.data).unwrap();
            assert_eq!(attr.xor_bpp, expected_bpp);
            assert_eq!(
                (attr.color_pointer.width, attr.color_pointer.height),
                (expected_width, expected_height)
            );
            assert!(!attr.color_pointer.and_mask.is_empty());
        }
        UpdateCode::LargePointer => {
            let attr = decode::<LargePointerAttribute<'_>>(pdu.data).unwrap();
            assert_eq!(attr.xor_bpp, expected_bpp);
            assert_eq!((attr.width, attr.height), (expected_width, expected_height));
            assert!(!attr.and_mask.is_empty());
        }
        other => panic!("unexpected pointer update code: {other:?}"),
    }
    assert!(updates.next().await.is_none());
}

// oracle: spec MS-RDPBCGR §2.2.7.2.7, §2.2.9.1.1.4.4, and §2.2.9.1.2.1.11
// Section 2.2.9.1.1.4.4 caps Color/New Pointer updates at 32x32 (or 96x96 with LARGE_POINTER_FLAG_96x96).
// Section 2.2.9.1.2.1.11 specifies TS_FP_LARGEPOINTERATTRIBUTE for shapes larger than that ceiling up to 384x384.
#[rstest::rstest]
#[case::base(32, LargePointerSupportFlags::empty(), UpdateCode::NewPointer)]
#[case::new_96(64, LargePointerSupportFlags::UP_TO_96X96_PIXELS, UpdateCode::NewPointer)]
#[case::new_96_max(96, LargePointerSupportFlags::UP_TO_96X96_PIXELS, UpdateCode::NewPointer)]
#[case::large_only(64, LargePointerSupportFlags::UP_TO_384X384_PIXELS, UpdateCode::LargePointer)]
#[case::both(64, LargePointerSupportFlags::all(), UpdateCode::NewPointer)]
#[case::above_96(128, LargePointerSupportFlags::all(), UpdateCode::LargePointer)]
#[case::max_384(384, LargePointerSupportFlags::all(), UpdateCode::LargePointer)]
#[tokio::test]
async fn alpha_pointer_uses_the_negotiated_update(
    #[case] width: u16,
    #[case] flags: LargePointerSupportFlags,
    #[case] expected: UpdateCode,
) {
    let height = 2;
    let update = DisplayUpdate::RGBAPointer(RGBAPointer {
        cache_index: 0,
        width,
        height,
        hot_x: 0,
        hot_y: 0,
        data: [10, 20, 30, 255].repeat(usize::from(width) * usize::from(height)),
    });
    assert_encoded_pointer_update(flags, update, expected, 32, width, height).await;
}

// oracle: spec MS-RDPBCGR §2.2.7.2.7, §2.2.9.1.1.4.4, and §2.2.9.1.2.1.11
// Color pointers at 24 bpp use ColorPointer within the negotiated ceiling and promote to LargePointer above it.
#[rstest::rstest]
#[case::base(32, LargePointerSupportFlags::empty(), UpdateCode::ColorPointer)]
#[case::color_96(64, LargePointerSupportFlags::UP_TO_96X96_PIXELS, UpdateCode::ColorPointer)]
#[case::color_96_max(96, LargePointerSupportFlags::UP_TO_96X96_PIXELS, UpdateCode::ColorPointer)]
#[case::large_only(64, LargePointerSupportFlags::UP_TO_384X384_PIXELS, UpdateCode::LargePointer)]
#[case::both(64, LargePointerSupportFlags::all(), UpdateCode::ColorPointer)]
#[case::above_96(128, LargePointerSupportFlags::all(), UpdateCode::LargePointer)]
#[case::max_384(384, LargePointerSupportFlags::all(), UpdateCode::LargePointer)]
#[tokio::test]
async fn color_pointer_uses_the_negotiated_update(
    #[case] width: u16,
    #[case] flags: LargePointerSupportFlags,
    #[case] expected: UpdateCode,
) {
    let height = 2;
    let update = DisplayUpdate::ColorPointer(ColorPointer {
        cache_index: 0,
        width,
        height,
        hot_x: 0,
        hot_y: 0,
        and_mask: vec![0; usize::from(width).div_ceil(16) * 2 * usize::from(height)],
        xor_mask: vec![0xAA; (usize::from(width) * 3).next_multiple_of(2) * usize::from(height)],
    });
    assert_encoded_pointer_update(flags, update, expected, 24, width, height).await;
}

// oracle: spec MS-RDPBCGR §2.2.7.1.5, §2.2.7.2.7, §2.2.9.1.1.4.4, and §2.2.9.1.2.1.11
// An alpha pointer that exceeds the client's negotiated pointer size ceiling or lacks pointer cache support is not sent.
#[rstest::rstest]
#[case::above_32_without_flags(20, 33, 32, LargePointerSupportFlags::empty())]
#[case::above_96_without_384(20, 97, 96, LargePointerSupportFlags::UP_TO_96X96_PIXELS)]
#[case::above_384_width(20, 385, 384, LargePointerSupportFlags::all())]
#[case::above_384_height(20, 384, 385, LargePointerSupportFlags::all())]
#[case::no_pointer_cache(0, 64, 64, LargePointerSupportFlags::all())]
#[tokio::test]
async fn alpha_pointer_exceeding_the_negotiated_ceiling_is_dropped(
    #[case] pointer_cache_size: u16,
    #[case] width: u16,
    #[case] height: u16,
    #[case] flags: LargePointerSupportFlags,
) {
    let mut encoder = test_encoder(pointer_cache_size, flags);
    let mut updates = encoder.update(DisplayUpdate::RGBAPointer(RGBAPointer {
        cache_index: 0,
        width,
        height,
        hot_x: 0,
        hot_y: 0,
        data: [10, 20, 30, 255].repeat(usize::from(width) * usize::from(height)),
    }));
    assert!(updates.next().await.is_none());
}

// oracle: spec MS-RDPBCGR §2.2.7.1.5, §2.2.7.2.7, §2.2.9.1.1.4.4, and §2.2.9.1.2.1.11
// A color pointer that exceeds the client's negotiated pointer size ceiling or cannot promote without pointer cache support is not sent.
#[rstest::rstest]
#[case::above_32_without_flags(20, 33, 32, LargePointerSupportFlags::empty())]
#[case::above_96_without_384(20, 97, 96, LargePointerSupportFlags::UP_TO_96X96_PIXELS)]
#[case::above_384_width(20, 385, 384, LargePointerSupportFlags::all())]
#[case::above_384_height(20, 384, 385, LargePointerSupportFlags::all())]
#[case::no_pointer_cache_for_large(0, 128, 128, LargePointerSupportFlags::all())]
#[tokio::test]
async fn color_pointer_exceeding_the_negotiated_ceiling_is_dropped(
    #[case] pointer_cache_size: u16,
    #[case] width: u16,
    #[case] height: u16,
    #[case] flags: LargePointerSupportFlags,
) {
    let mut encoder = test_encoder(pointer_cache_size, flags);
    let mut updates = encoder.update(DisplayUpdate::ColorPointer(ColorPointer {
        cache_index: 0,
        width,
        height,
        hot_x: 0,
        hot_y: 0,
        and_mask: vec![0; usize::from(width).div_ceil(16) * 2 * usize::from(height)],
        xor_mask: vec![0xAA; (usize::from(width) * 3).next_multiple_of(2) * usize::from(height)],
    }));
    assert!(updates.next().await.is_none());
}
