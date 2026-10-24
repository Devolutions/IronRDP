use ironrdp_core::{ReadCursor, decode_cursor};
use ironrdp_pdu::fast_path::{FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp_pdu::rdp::capability_sets::{CmdFlags, LargePointerSupportFlags};
use ironrdp_server::encoder::{UpdateEncoder, UpdateEncoderCodecs};
use ironrdp_server::{DesktopSize, DisplayUpdate, RGBAPointer};

// oracle: spec MS-RDPBCGR §2.2.9.1.2.1.5 and §2.2.9.1.2.1.6; system pointer updates have zero-sized bodies.
#[tokio::test]
async fn system_pointer_updates_emit_one_empty_frame() {
    let mut encoder = UpdateEncoder::new(
        DesktopSize {
            width: 800,
            height: 600,
        },
        CmdFlags::empty(),
        UpdateEncoderCodecs::default(),
        65536,
        20,
        LargePointerSupportFlags::empty(),
    )
    .unwrap();
    for (update, code) in [
        (DisplayUpdate::HidePointer, UpdateCode::HiddenPointer),
        (DisplayUpdate::DefaultPointer, UpdateCode::DefaultPointer),
    ] {
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
}

// oracle: spec MS-RDPBCGR §2.2.7.2.7 and §2.2.9.1.1.4.4; the 96 and 384 flags select separate update ceilings.
#[rstest::rstest]
#[case::base(32, LargePointerSupportFlags::empty(), UpdateCode::NewPointer)]
#[case::new_96(64, LargePointerSupportFlags::UP_TO_96X96_PIXELS, UpdateCode::NewPointer)]
#[case::large_only(64, LargePointerSupportFlags::UP_TO_384X384_PIXELS, UpdateCode::LargePointer)]
#[case::both(64, LargePointerSupportFlags::all(), UpdateCode::NewPointer)]
#[tokio::test]
async fn alpha_pointer_uses_the_negotiated_update(
    #[case] width: u16,
    #[case] flags: LargePointerSupportFlags,
    #[case] expected: UpdateCode,
) {
    let mut encoder = UpdateEncoder::new(
        DesktopSize {
            width: 800,
            height: 600,
        },
        CmdFlags::empty(),
        UpdateEncoderCodecs::default(),
        65536,
        20,
        flags,
    )
    .unwrap();
    let mut updates = encoder.update(DisplayUpdate::RGBAPointer(RGBAPointer {
        cache_index: 0,
        width,
        height: 1,
        hot_x: 0,
        hot_y: 0,
        data: [10, 20, 30, 255].repeat(usize::from(width)),
    }));
    let mut update = updates
        .next()
        .await
        .expect("supported pointer must not be dropped")
        .unwrap();
    let mut bytes = [0; 1024];
    let length = update.next(&mut bytes).expect("pointer frame");
    let mut cursor = ReadCursor::new(&bytes[..length]);
    let _ = decode_cursor::<FastPathHeader>(&mut cursor).unwrap();
    assert_eq!(
        decode_cursor::<FastPathUpdatePdu<'_>>(&mut cursor).unwrap().update_code,
        expected
    );
    assert!(updates.next().await.is_none());
}
