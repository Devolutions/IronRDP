use ironrdp_egfx::decode::DecodedFrame;

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
