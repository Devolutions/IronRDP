//! The RDP 8.0 decompressor in `ironrdp-bulk` serves the dynamic virtual channels. These
//! tests hold it to the same results as the graphics pipeline's ZGFX implementation.

use ironrdp_bulk::rdp8::Rdp8Decompressor;
use ironrdp_graphics::zgfx::{CompressionMode, Compressor, compress_and_wrap_egfx};

macro_rules! zgfx_asset {
    ($name:literal) => {
        include_bytes!(concat!("../../ironrdp-graphics/src/zgfx/test_assets/", $name))
    };
}

#[test]
fn rdp8_decodes_windows_egfx_captures() {
    let pairs: [(&[u8], &[u8]); 5] = [
        (zgfx_asset!("encoded.0.bin"), zgfx_asset!("decoded.0.bin")),
        (zgfx_asset!("encoded.1.bin"), zgfx_asset!("decoded.1.bin")),
        (zgfx_asset!("encoded.2.bin"), zgfx_asset!("decoded.2.bin")),
        (zgfx_asset!("encoded.3.bin"), zgfx_asset!("decoded.3.bin")),
        (zgfx_asset!("encoded.4.bin"), zgfx_asset!("decoded.4.bin")),
    ];

    // One decompressor for the whole sequence: later captures match against earlier ones.
    let mut decompressor = Rdp8Decompressor::new();
    for (i, (encoded, decoded)) in pairs.into_iter().enumerate() {
        let mut output = Vec::new();
        let written = decompressor
            .decompress(encoded, &mut output)
            .unwrap_or_else(|e| panic!("capture #{i} should decompress: {e}"));
        assert_eq!(written, decoded.len(), "capture #{i}");
        assert_eq!(output, decoded, "capture #{i}");
    }
}

#[test]
fn rdp8_decodes_ironrdp_compressor_output() {
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(40).into_bytes();
    let mut noise = Vec::with_capacity(70_000);
    let mut state = 0x2545_f491u32;
    for _ in 0..noise.capacity() {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        noise.push(state.to_le_bytes()[0]);
    }
    // Larger than one 65,535-byte segment, so the compressor emits a multipart message.
    let large: Vec<u8> = text.iter().chain(&noise).copied().collect();

    let mut compressor = Compressor::new();
    let mut decompressor = Rdp8Decompressor::new();
    for (i, message) in [&text, &noise, &text, &large, &noise].into_iter().enumerate() {
        let compressed =
            compress_and_wrap_egfx(message, &mut compressor, CompressionMode::Always).expect("message should compress");

        let mut output = Vec::new();
        decompressor
            .decompress(&compressed, &mut output)
            .unwrap_or_else(|e| panic!("message #{i} should decompress: {e}"));
        assert_eq!(&output, message, "message #{i}");
    }
}
