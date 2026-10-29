use ironrdp_egfx::decode::{DecodedFrame, Yuv420Frame};

#[test]
fn getters_return_constructor_inputs() {
    let data = vec![0u8; 2 * 3 * 4];
    let frame = DecodedFrame::new(data.clone(), 2, 3);
    assert_eq!(frame.data(), data.as_slice());
    assert_eq!(frame.width(), 2);
    assert_eq!(frame.height(), 3);
}

#[test]
fn into_data_yields_owned_buffer() {
    let data = vec![0xAAu8; 4 * 4 * 4];
    let frame = DecodedFrame::new(data.clone(), 4, 4);
    assert_eq!(frame.into_data(), data);
}

// oracle: mutant planes accepted that are narrower or shorter than the picture
#[test]
fn yuv420_planes_must_cover_the_picture() {
    // A 4x2 picture has 2x1 chroma planes.
    let planes = |luma: usize, chroma: usize| (vec![0u8; luma], vec![0u8; chroma], vec![0u8; chroma]);

    let (y, u, v) = planes(4 * 2, 2);
    assert!(Yuv420Frame::new(4, 2, y, 4, u, v, 2).is_ok());

    let (y, u, v) = planes(4 * 2, 2);
    assert!(
        Yuv420Frame::new(4, 2, y, 3, u, v, 2).is_err(),
        "luma stride narrower than the picture"
    );
    let (y, u, v) = planes(4 * 2, 2);
    assert!(
        Yuv420Frame::new(4, 2, y, 4, u, v, 1).is_err(),
        "chroma stride narrower than the picture"
    );
    let (y, u, v) = planes(4 * 2 - 1, 2);
    assert!(
        Yuv420Frame::new(4, 2, y, 4, u, v, 2).is_err(),
        "luma plane one byte short"
    );
    let (y, u, v) = planes(4 * 2, 1);
    assert!(
        Yuv420Frame::new(4, 2, y, 4, u, v, 2).is_err(),
        "chroma planes one byte short"
    );
}
