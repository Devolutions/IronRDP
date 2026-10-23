use ironrdp_graphics::alpha::{AlphaDecodeError, AlphaRun, decode};

/// `alphaSig` (0x414C, "LA" on the wire) followed by `compressed`.
fn header(compressed: u16) -> Vec<u8> {
    let mut stream = vec![0x4C, 0x41];
    stream.extend_from_slice(&compressed.to_le_bytes());
    stream
}

/// Materialize the decoded opacity, one byte per pixel.
fn decode_to_vec(stream: &[u8], width: u16, height: u16) -> Result<Vec<u8>, AlphaDecodeError> {
    let mut alpha = vec![None; usize::from(width) * usize::from(height)];
    decode(stream, width, height, |first_pixel, run| match run {
        AlphaRun::Values(values) => {
            for (i, value) in values.iter().enumerate() {
                alpha[first_pixel + i] = Some(*value);
            }
        }
        AlphaRun::Fill { value, len } => alpha[first_pixel..first_pixel + len].fill(Some(value)),
    })?;
    Ok(alpha
        .into_iter()
        .map(|value| value.expect("every pixel is covered"))
        .collect())
}

#[test]
fn raw_stream_carries_one_byte_per_pixel() {
    // oracle: spec MS-RDPEGFX §2.2.4.3 (compressed 0: raw values, top-left to bottom-right)
    let mut stream = header(0);
    stream.extend_from_slice(&[0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);

    assert_eq!(
        decode_to_vec(&stream, 3, 2),
        Ok(vec![0x00, 0x11, 0x22, 0x33, 0x44, 0x55])
    );
}

#[test]
fn raw_stream_ignores_trailing_bytes() {
    // oracle: reference awakecoding/FreeRDP@16a7c2ec2fd0:libfreerdp/gdi/gfx.c gdi_SurfaceCommand_Alpha
    // (reads width x height raw bytes and ignores the rest of the command)
    let mut stream = header(0);
    stream.extend_from_slice(&[1, 2, 3, 4, 0xEE]);

    assert_eq!(decode_to_vec(&stream, 2, 2), Ok(vec![1, 2, 3, 4]));
}

#[test]
fn raw_stream_shorter_than_the_bitmap_is_truncated() {
    // oracle: spec MS-RDPEGFX §2.2.4.3 (the segments MUST encode the whole bitmap area)
    let mut stream = header(0);
    stream.extend_from_slice(&[1, 2, 3]);

    assert_eq!(decode_to_vec(&stream, 2, 2), Err(AlphaDecodeError::Truncated));
}

#[test]
fn rle_one_byte_run_lengths() {
    // oracle: spec MS-RDPEGFX §2.2.4.3.1 (runLengthFactor1 below 0xFF is the run length)
    let mut stream = header(1);
    // runValue 0xFF x 2, runValue 0x00 x 1, runValue 0x7F x 3.
    stream.extend_from_slice(&[0xFF, 2, 0x00, 1, 0x7F, 3]);

    assert_eq!(
        decode_to_vec(&stream, 3, 2),
        Ok(vec![0xFF, 0xFF, 0x00, 0x7F, 0x7F, 0x7F])
    );
}

#[test]
fn rle_two_byte_run_length_behind_the_0xff_escape() {
    // oracle: spec MS-RDPEGFX §2.2.4.3.1 (runLengthFactor1 0xFF: runLengthFactor2 is the length)
    let mut stream = header(1);
    stream.extend_from_slice(&[0xAB, 0xFF]);
    stream.extend_from_slice(&300u16.to_le_bytes());

    assert_eq!(decode_to_vec(&stream, 20, 15), Ok(vec![0xAB; 300]));
}

#[test]
fn rle_four_byte_run_length_behind_the_0xffff_escape() {
    // oracle: spec MS-RDPEGFX §2.2.4.3.1 (runLengthFactor2 0xFFFF: runLengthFactor3 is the length)
    let mut stream = header(1);
    stream.extend_from_slice(&[0x40, 0xFF, 0xFF, 0xFF]);
    stream.extend_from_slice(&70_000u32.to_le_bytes());

    let alpha = decode_to_vec(&stream, 350, 200).unwrap();
    assert_eq!(alpha.len(), 70_000);
    assert!(alpha.iter().all(|value| *value == 0x40));
}

#[test]
fn a_run_longer_than_any_buffer_is_handed_over_without_materializing_it() {
    // oracle: spec MS-RDPEGFX §2.2.4.3.1 (one runLengthFactor3 run covers the 65535 x 65535 area)
    let mut stream = header(1);
    stream.extend_from_slice(&[0x10, 0xFF, 0xFF, 0xFF]);
    stream.extend_from_slice(&(65_535u32 * 65_535).to_le_bytes());

    let mut runs = Vec::new();
    decode(&stream, 65_535, 65_535, |first_pixel, run| {
        runs.push((first_pixel, run.len()))
    })
    .unwrap();

    assert_eq!(runs, [(0, 65_535 * 65_535)]);
}

#[test]
fn rle_zero_length_runs_are_allowed() {
    // oracle: spec MS-RDPEGFX §2.2.4.3.1 (runLengthFactor1 values below 0xFF, zero included, are
    // run lengths)
    let mut stream = header(1);
    stream.extend_from_slice(&[0x10, 0, 0x20, 2]);

    assert_eq!(decode_to_vec(&stream, 2, 1), Ok(vec![0x20, 0x20]));
}

#[test]
fn rle_run_past_the_bitmap_is_rejected_before_anything_is_applied() {
    // oracle: spec MS-RDPEGFX §2.2.4.3 (the pixel count MUST equal the bitmap area)
    let mut stream = header(1);
    stream.extend_from_slice(&[0x10, 2, 0x20, 3]);

    let mut applied = 0;
    let result = decode(&stream, 2, 2, |_, _| applied += 1);

    assert_eq!(
        result,
        Err(AlphaDecodeError::PixelCountMismatch {
            expected: 4,
            decoded: 5
        })
    );
    assert_eq!(applied, 0);
}

#[test]
fn rle_runs_past_a_full_bitmap_are_rejected() {
    // oracle: spec MS-RDPEGFX §2.2.4.3 (the pixel count MUST equal the bitmap area)
    let mut stream = header(1);
    stream.extend_from_slice(&[0x10, 4, 0x20, 1]);

    assert_eq!(
        decode_to_vec(&stream, 2, 2),
        Err(AlphaDecodeError::PixelCountMismatch {
            expected: 4,
            decoded: 5
        })
    );
}

#[test]
fn empty_rle_runs_past_a_full_bitmap_are_allowed() {
    // oracle: spec MS-RDPEGFX §2.2.4.3.1 (a zero run length covers no pixel, so the count still
    // equals the area)
    let mut stream = header(1);
    stream.extend_from_slice(&[0x10, 4, 0x20, 0]);

    assert_eq!(decode_to_vec(&stream, 2, 2), Ok(vec![0x10; 4]));
}

#[test]
fn an_empty_raw_bitmap_applies_no_run() {
    // oracle: mutant (an empty run handed to the caller)
    let mut runs = 0;

    decode(&header(0), 0, 7, |_, _| runs += 1).unwrap();

    assert_eq!(runs, 0);
}

#[test]
fn rle_stream_ending_before_the_bitmap_is_full_is_truncated() {
    // oracle: spec MS-RDPEGFX §2.2.4.3 (the pixel count MUST equal the bitmap area)
    let mut stream = header(1);
    stream.extend_from_slice(&[0x10, 3]);

    assert_eq!(decode_to_vec(&stream, 2, 2), Err(AlphaDecodeError::Truncated));
}

#[test]
fn rle_escape_without_its_wider_length_is_truncated() {
    // oracle: spec MS-RDPEGFX §2.2.4.3.1 (runLengthFactor2 MUST follow a 0xFF runLengthFactor1)
    let mut stream = header(1);
    stream.extend_from_slice(&[0x10, 0xFF, 0x01]);

    assert_eq!(decode_to_vec(&stream, 2, 2), Err(AlphaDecodeError::Truncated));
}

#[test]
fn wrong_signature_is_rejected() {
    // oracle: spec MS-RDPEGFX §2.2.4.3 (alphaSig MUST be 0x414C)
    assert_eq!(
        decode_to_vec(&[0x41, 0x4C, 0, 0, 1], 1, 1),
        Err(AlphaDecodeError::InvalidSignature(0x4C41))
    );
}

#[test]
fn missing_header_is_truncated() {
    // oracle: spec MS-RDPEGFX §2.2.4.3 (alphaSig and compressed are 2 bytes each)
    assert_eq!(decode_to_vec(&[0x4C, 0x41, 0], 1, 1), Err(AlphaDecodeError::Truncated));
}
