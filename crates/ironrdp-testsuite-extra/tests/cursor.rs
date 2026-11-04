use ironrdp_server::DisplayUpdate;
use ironrdp_server_native::cursor::{CursorCapture, CursorImage};
use rstest::rstest;
use windows::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject, HGDIOBJ};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, DestroyCursor, HCURSOR, ICONINFO};

fn update(image: CursorImage) -> DisplayUpdate {
    CursorCapture::default().update(Ok(Some(image))).unwrap().unwrap()
}

// oracle: spec MS-RDPBCGR §2.2.9.1.1.4.4, bottom-up BGR with word-aligned XOR and AND rows.
#[test]
fn color_cursor_preserves_rows_padding_and_hotspot() -> anyhow::Result<()> {
    let pixels = [
        3, 2, 1, 0, 6, 5, 4, 0, 9, 8, 7, 0, 30, 20, 10, 0, 60, 50, 40, 0, 90, 80, 70, 0,
    ];
    let image = CursorImage::from_bitmaps(3, 2, 1, 1, &[0xa0, 0, 0, 0, 0x40, 0, 0, 0], Some(&pixels))?;
    let DisplayUpdate::ColorPointer(pointer) = update(image) else {
        panic!("expected explicit masks")
    };
    assert_eq!(
        pointer.xor_mask,
        [30, 20, 10, 60, 50, 40, 90, 80, 70, 0, 3, 2, 1, 6, 5, 4, 9, 8, 7, 0]
    );
    assert_eq!(pointer.and_mask, [0x40, 0, 0xa0, 0]);
    assert_eq!(
        (pointer.width, pointer.height, pointer.hot_x, pointer.hot_y),
        (3, 2, 1, 1)
    );
    Ok(())
}

// oracle: reference Win32 "Using Cursors", monochrome AND/XOR truth table.
#[test]
fn monochrome_cursor_keeps_black_white_transparency_and_inversion() -> anyhow::Result<()> {
    let image = CursorImage::from_bitmaps(4, 1, 0, 0, &[0x30, 0, 0, 0, 0x50, 0, 0, 0], None)?;
    let DisplayUpdate::ColorPointer(pointer) = update(image) else {
        panic!("expected explicit masks")
    };
    assert_eq!(pointer.xor_mask, [0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255]);
    assert_eq!(pointer.and_mask, [0x30, 0]);
    Ok(())
}

// oracle: reference Win32 "Using Cursors", alpha cursor pixels are premultiplied BGRA.
#[test]
fn alpha_cursor_preserves_native_bgra_and_bottom_up_rows() -> anyhow::Result<()> {
    let image = CursorImage::from_bitmaps(1, 2, 0, 1, &[0xff; 8], Some(&[60, 30, 100, 128, 0, 0, 0, 0]))?;
    let DisplayUpdate::RGBAPointer(pointer) = update(image) else {
        panic!("expected an alpha pointer")
    };
    assert_eq!(pointer.data, [0, 0, 0, 0, 60, 30, 100, 128]);
    assert_eq!(
        (pointer.width, pointer.height, pointer.hot_x, pointer.hot_y),
        (1, 2, 0, 1)
    );
    Ok(())
}

// oracle: mutant applying the 96-pixel New Pointer ceiling before the server can negotiate Large Pointer.
#[test]
fn maximum_shape_reaches_server_encoder() -> anyhow::Result<()> {
    let image = CursorImage::from_bitmaps(384, 384, 383, 383, &vec![0; 48 * 384], Some(&vec![255; 384 * 384 * 4]))?;
    let DisplayUpdate::RGBAPointer(pointer) = update(image) else {
        panic!("expected an alpha pointer")
    };
    assert_eq!(
        (pointer.width, pointer.height, pointer.hot_x, pointer.hot_y),
        (384, 384, 383, 383)
    );
    assert_eq!(pointer.data.len(), 384 * 384 * 4);
    Ok(())
}

// oracle: mutant accepting hotspot coordinates outside the captured image.
#[rstest]
#[case::x_outside(1, 1, 1, 0)]
#[case::y_outside(1, 1, 0, 1)]
fn invalid_hotspot_is_rejected(#[case] width: u16, #[case] height: u16, #[case] hot_x: u16, #[case] hot_y: u16) {
    assert!(CursorImage::from_bitmaps(width, height, hot_x, hot_y, &[0; 4], Some(&[0; 4])).is_err());
}

// oracle: mutant accepting empty or unbounded geometry before allocating native bitmap buffers.
#[rstest]
#[case::zero_width(0, 1)]
#[case::zero_height(1, 0)]
#[case::too_wide(385, 1)]
#[case::too_tall(1, 385)]
fn invalid_dimensions_are_rejected(#[case] width: u16, #[case] height: u16) {
    let mask_len = usize::from(width).div_ceil(32) * 4 * usize::from(height) * 2;
    assert!(CursorImage::from_bitmaps(width, height, 0, 0, &vec![0; mask_len], None).is_err());
}

// oracle: mutant slicing bitmap rows before checking the supplied buffers' lengths.
#[rstest]
#[case::short_color_mask(3, Some(4))]
#[case::long_color_mask(5, Some(4))]
#[case::short_color(4, Some(3))]
#[case::long_color(4, Some(5))]
#[case::short_monochrome_mask(7, None)]
#[case::long_monochrome_mask(9, None)]
fn malformed_bitmap_lengths_are_rejected(#[case] mask_len: usize, #[case] color_len: Option<usize>) {
    let color = color_len.map(|length| vec![0; length]);
    assert!(CursorImage::from_bitmaps(1, 1, 0, 0, &vec![0; mask_len], color.as_deref()).is_err());
}

// oracle: mutant duplicate shapes are resent, hidden cursors cannot restore, or equal pixels hide hotspot changes.
#[test]
fn shape_changes_and_visibility_are_forwarded_once() -> anyhow::Result<()> {
    let mut capture = CursorCapture::default();
    assert!(matches!(capture.update(Ok(None))?, Some(DisplayUpdate::HidePointer)));
    assert!(capture.update(Ok(None))?.is_none());
    let image = CursorImage::from_bitmaps(2, 1, 0, 0, &[0; 4], Some(&[255; 8]))?;
    assert!(matches!(
        capture.update(Ok(Some(image.clone())))?,
        Some(DisplayUpdate::RGBAPointer(_))
    ));
    assert!(capture.update(Ok(Some(image.clone())))?.is_none());
    let moved_hotspot = CursorImage::from_bitmaps(2, 1, 1, 0, &[0; 4], Some(&[255; 8]))?;
    assert!(matches!(
        capture.update(Ok(Some(moved_hotspot)))?,
        Some(DisplayUpdate::RGBAPointer(_))
    ));
    assert!(matches!(capture.update(Ok(None))?, Some(DisplayUpdate::HidePointer)));
    assert!(matches!(
        capture.update(Ok(Some(image)))?,
        Some(DisplayUpdate::RGBAPointer(_))
    ));
    Ok(())
}

// oracle: mutant retaining a cached shape after callers hide the pointer on capture failure.
#[test]
fn capture_error_resends_unchanged_shape_on_recovery() -> anyhow::Result<()> {
    let mut capture = CursorCapture::default();
    let image = CursorImage::from_bitmaps(1, 1, 0, 0, &[0; 4], Some(&[255; 4]))?;
    assert!(matches!(
        capture.update(Ok(Some(image.clone())))?,
        Some(DisplayUpdate::RGBAPointer(_))
    ));
    let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "cursor desktop unavailable");
    assert_eq!(
        capture.update(Err(error)).unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert!(matches!(
        capture.update(Ok(Some(image)))?,
        Some(DisplayUpdate::RGBAPointer(_))
    ));
    Ok(())
}

struct OwnedCursor(HCURSOR);

impl Drop for OwnedCursor {
    fn drop(&mut self) {
        // SAFETY: This cursor was created by CreateIconIndirect and is owned by this guard.
        unsafe { DestroyCursor(self.0) }.unwrap();
    }
}

// oracle: reference Win32 GetIconInfo, a monochrome cursor has an AND mask above its XOR mask.
#[test]
fn native_monochrome_capture_keeps_masks_and_hotspot() -> anyhow::Result<()> {
    // SAFETY: The buffer has two word-aligned monochrome rows for the 4x2 bitmap.
    let mask = unsafe { CreateBitmap(4, 2, 1, 1, Some([0x30u8, 0, 0x50, 0].as_ptr().cast())) };
    assert!(!mask.is_invalid());
    let info = ICONINFO {
        fIcon: false.into(),
        xHotspot: 2,
        yHotspot: 0,
        hbmMask: mask,
        ..Default::default()
    };
    // SAFETY: The bitmap is valid and CreateIconIndirect copies it.
    let cursor = unsafe { CreateIconIndirect(&info) };
    // SAFETY: The bitmap is no longer needed after CreateIconIndirect.
    assert!(unsafe { DeleteObject(HGDIOBJ(mask.0)) }.as_bool());
    let cursor = OwnedCursor(HCURSOR(cursor?.0));
    let DisplayUpdate::ColorPointer(pointer) = update(CursorImage::from_handle(cursor.0)?) else {
        panic!("expected explicit masks")
    };
    assert_eq!(
        (pointer.width, pointer.height, pointer.hot_x, pointer.hot_y),
        (4, 1, 2, 0)
    );
    assert_eq!(pointer.and_mask, [0x30, 0]);
    assert_eq!(pointer.xor_mask, [0, 0, 0, 255, 255, 255, 0, 0, 0, 255, 255, 255]);
    Ok(())
}

// oracle: reference Win32 "Using Cursors", alpha cursor pixels are premultiplied BGRA.
#[test]
fn native_alpha_capture_preserves_premultiplied_channels() -> anyhow::Result<()> {
    // SAFETY: Each buffer has a complete scanline for its bitmap's format.
    let mask = unsafe { CreateBitmap(1, 1, 1, 1, Some([0u8; 2].as_ptr().cast())) };
    // SAFETY: The buffer has one complete 32-bit pixel.
    let color = unsafe { CreateBitmap(1, 1, 1, 32, Some([60u8, 30, 100, 128].as_ptr().cast())) };
    let info = ICONINFO {
        fIcon: false.into(),
        xHotspot: 0,
        yHotspot: 0,
        hbmMask: mask,
        hbmColor: color,
    };
    // SAFETY: The bitmaps remain live and CreateIconIndirect copies them.
    let cursor = unsafe { CreateIconIndirect(&info) };
    for bitmap in [mask, color] {
        // SAFETY: These bitmaps are no longer needed after CreateIconIndirect.
        assert!(unsafe { DeleteObject(HGDIOBJ(bitmap.0)) }.as_bool());
    }
    let cursor = OwnedCursor(HCURSOR(cursor?.0));
    let DisplayUpdate::RGBAPointer(pointer) = update(CursorImage::from_handle(cursor.0)?) else {
        panic!("expected an alpha pointer")
    };
    assert_eq!(pointer.data, [60, 30, 100, 128]);
    Ok(())
}
