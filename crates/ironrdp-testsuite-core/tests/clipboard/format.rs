use ironrdp_cliprdr_format::bitmap::{
    BitmapError, dib_to_png, dibv5_to_png, png_to_cf_dib, png_to_cf_dibv5, validate_dib, validate_dibv5,
};
use ironrdp_cliprdr_format::html::{cf_html_to_plain_html, plain_html_to_cf_html, validate_cf_html};
use png::{BitDepth, ColorType, Encoder};

/// Encodes a tiny synthetic PNG for `png_to_cf_dib`/`png_to_cf_dibv5` regression coverage. `pixel`
/// is one sample tuple repeated across every pixel, laid out per `color_type`/`bit_depth`.
fn encode_test_png(color_type: ColorType, bit_depth: BitDepth, pixel: &[u8]) -> Vec<u8> {
    const WIDTH: u32 = 2;
    const HEIGHT: u32 = 2;

    let mut png_bytes = Vec::new();
    {
        let mut encoder = Encoder::new(&mut png_bytes, WIDTH, HEIGHT);
        encoder.set_color(color_type);
        encoder.set_depth(bit_depth);
        let mut writer = encoder.write_header().unwrap();
        let row: Vec<u8> = pixel.repeat(usize::try_from(WIDTH).unwrap());
        let image: Vec<u8> = row.repeat(usize::try_from(HEIGHT).unwrap());
        writer.write_image_data(&image).unwrap();
    }
    png_bytes
}

#[test]
fn dib_to_png_conversion_1() {
    let input = include_bytes!("../../test_data/pdu/clipboard/cf_dib.pdu");
    assert_eq!(validate_dib(input).unwrap(), input.len());
    let png = dib_to_png(input).unwrap();
    let converted = png_to_cf_dib(&png).unwrap();
    assert_eq!(converted, input);
}

const BI_BITFIELDS: u32 = 3;
const STANDARD_BGRA_MASKS: [u32; 3] = [0x00FF_0000, 0x0000_FF00, 0x0000_00FF];

/// Rewrites a `BI_RGB` `CF_DIB` fixture as `BI_BITFIELDS`: the 40-byte `BITMAPINFOHEADER` with its
/// compression patched, followed by the three color masks, then the untouched pixel data.
fn v1_dib_with_bitfields(rgb_dib: &[u8], masks: [u32; 3]) -> Vec<u8> {
    let mut dib = rgb_dib[..40].to_vec();
    dib[16..20].copy_from_slice(&BI_BITFIELDS.to_le_bytes());
    for mask in masks {
        dib.extend_from_slice(&mask.to_le_bytes());
    }
    dib.extend_from_slice(&rgb_dib[40..]);
    dib
}

/// Patches a `CF_DIBV5` fixture to `BI_BITFIELDS` with the given red/green/blue/alpha masks, which
/// live inside the fixed `BITMAPV5HEADER` right after the V1 part.
fn v5_dib_with_bitfields(dibv5: &[u8], masks: [u32; 4]) -> Vec<u8> {
    let mut dib = dibv5.to_vec();
    dib[16..20].copy_from_slice(&BI_BITFIELDS.to_le_bytes());
    for (i, mask) in masks.into_iter().enumerate() {
        let at = 40 + i * 4;
        dib[at..at + 4].copy_from_slice(&mask.to_le_bytes());
    }
    dib
}

#[test]
fn dib_to_png_accepts_v1_bitfields_with_standard_masks() {
    // Windows commonly places 32bpp `CF_DIB` on the clipboard with `BI_BITFIELDS` compression and
    // the standard BGRA masks; the pixels are laid out exactly as with `BI_RGB`, so the result must
    // be identical to decoding the `BI_RGB` fixture.
    let rgb = include_bytes!("../../test_data/pdu/clipboard/cf_dib.pdu");
    let bitfields = v1_dib_with_bitfields(rgb, STANDARD_BGRA_MASKS);

    assert_eq!(dib_to_png(&bitfields).unwrap(), dib_to_png(rgb).unwrap());
    // The logical length accounts for the masks that sit between the header and the pixels.
    assert_eq!(validate_dib(&bitfields).unwrap(), bitfields.len());
    assert_eq!(bitfields.len(), rgb.len() + 12);
}

#[test]
fn dib_to_png_rejects_v1_bitfields_with_non_standard_masks() {
    // Dropping the masks and reading the pixels as BGRA is only valid for the standard order; any
    // other order (here red and blue swapped) would silently swap channels, so it is refused.
    let rgb = include_bytes!("../../test_data/pdu/clipboard/cf_dib.pdu");
    let rgb_order = v1_dib_with_bitfields(rgb, [0x0000_00FF, 0x0000_FF00, 0x00FF_0000]);

    assert!(matches!(dib_to_png(&rgb_order), Err(BitmapError::Unsupported(_))));
    assert!(matches!(validate_dib(&rgb_order), Err(BitmapError::Unsupported(_))));
}

#[test]
fn dib_to_png_rejects_v1_bitfields_with_truncated_masks() {
    // A `BI_BITFIELDS` header that is cut off inside the color masks is a decode error, not a
    // panic and not a bitmap decoded from garbage masks.
    let rgb = include_bytes!("../../test_data/pdu/clipboard/cf_dib.pdu");
    let mut truncated = v1_dib_with_bitfields(rgb, STANDARD_BGRA_MASKS);
    truncated.truncate(40 + 8);

    assert!(matches!(dib_to_png(&truncated), Err(BitmapError::Decode(_))));
}

#[test]
fn dibv5_to_png_bitfields_masks_are_still_validated() {
    // The mask validation is shared with the `CF_DIBV5` path; make sure the V5 behavior is
    // unchanged: standard masks decode like `BI_RGB`, a non-standard order is refused.
    let rgb = include_bytes!("../../test_data/pdu/clipboard/cf_dibv5.pdu");
    let standard = v5_dib_with_bitfields(rgb, [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000]);
    let rgb_order = v5_dib_with_bitfields(rgb, [0x0000_00FF, 0x0000_FF00, 0x00FF_0000, 0xFF00_0000]);

    assert_eq!(dibv5_to_png(&standard).unwrap(), dibv5_to_png(rgb).unwrap());
    assert!(matches!(dibv5_to_png(&rgb_order), Err(BitmapError::Unsupported(_))));
}

#[test]
fn dibv5_to_png_conversion_1() {
    let input = include_bytes!("../../test_data/pdu/clipboard/cf_dibv5.pdu");
    assert_eq!(validate_dibv5(input).unwrap(), input.len());
    let png = dibv5_to_png(input).unwrap();
    let converted = png_to_cf_dibv5(&png).unwrap();
    assert_eq!(converted, input);
}

/// Reads the first pixel's RGBA bytes out of a PNG produced by `dibv5_to_png`, which always emits
/// 8-bit RGBA (it is generated from the already-normalized, alpha-preserving BGRA `DIBV5` bitmap).
fn first_pixel_rgba(png_bytes: &[u8]) -> [u8; 4] {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes)).read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    // `dibv5_to_png` always emits Rgba (preserve_alpha = true); assert that rather than silently
    // reading past the first pixel if that ever changes.
    assert_eq!(info.color_type, ColorType::Rgba, "expected an RGBA PNG");
    [buf[0], buf[1], buf[2], buf[3]]
}

#[test]
fn png_to_cf_dibv5_expands_grayscale_to_rgba() {
    // Regression test: `decode_png` used to omit `GRAY_TO_RGB`-equivalent handling, leaving a
    // grayscale source as 2-channel `GrayscaleAlpha` while the conversion downstream assumed
    // 4-channel `Rgba`, silently corrupting the output instead of failing.
    let png = encode_test_png(ColorType::Grayscale, BitDepth::Eight, &[128]);
    let dib = png_to_cf_dibv5(&png).expect("grayscale PNG must convert");
    let roundtrip = dibv5_to_png(&dib).unwrap();
    // Gray 128, no tRNS chunk: Replicated into R/G/B, fully opaque alpha.
    assert_eq!(first_pixel_rgba(&roundtrip), [128, 128, 128, 255]);
}

#[test]
fn png_to_cf_dibv5_strips_16_bit_samples() {
    // Regression test: `decode_png` used to omit `STRIP_16`, leaving 16-bit-per-channel samples
    // 16-bit while the conversion downstream assumed 8-bit, silently misreading the buffer.
    let png = encode_test_png(ColorType::Rgb, BitDepth::Sixteen, &[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc]);
    let dib = png_to_cf_dibv5(&png).expect("16-bit PNG must convert");
    let roundtrip = dibv5_to_png(&dib).unwrap();
    // STRIP_16 keeps the most-significant byte of each big-endian 16-bit sample.
    assert_eq!(first_pixel_rgba(&roundtrip), [0x12, 0x56, 0x9a, 255]);
}

#[test]
fn html_failure() {
    // Empty
    assert!(cf_html_to_plain_html(&[]).is_err());
    // Garbage
    assert!(cf_html_to_plain_html(&[0x00, 0x01, 0x02, 0x03]).is_err());
    // No headers
    assert!(cf_html_to_plain_html(b"hello world").is_err());
    // Headers with fragment size not found
    assert!(cf_html_to_plain_html(b"Version:0.9\r\n<html>nopers</html>").is_err());
    // Out of bounds headers
    assert!(cf_html_to_plain_html(b"StartFragment:999\r\nEndFragment:9999\r\n<html>nopers</html>").is_err());
}

#[test]
fn test_cf_html_to_text() {
    let input = include_bytes!("../../test_data/pdu/clipboard/cf_html.pdu");
    let actual = cf_html_to_plain_html(input).unwrap();

    // Validate that the output is valid HTML
    assert!(actual.starts_with("<b>Remote Desktop Protocol</b>"));
    assert!(actual.ends_with("</sup>"));

    // Validate roundtrip
    let cf_html = plain_html_to_cf_html(actual);
    let roundtrip_html_text = cf_html_to_plain_html(cf_html.as_bytes()).unwrap();
    assert_eq!(actual, roundtrip_html_text);

    // Add some padding (CF_HTML is not null-terminated, we need to work with data which is
    // potentially padded with arbitrary fill bytes).
    let mut cf_html = cf_html.into_bytes();
    cf_html.extend_from_slice(&[0xFF; 10]);
    assert_eq!(validate_cf_html(&cf_html).unwrap(), cf_html.len() - 10);
    let roundtrip_html_text = cf_html_to_plain_html(&cf_html).unwrap();
    assert_eq!(actual, roundtrip_html_text);
}

#[test]
fn html_without_context_is_validated_and_trimmed() {
    let fragment = "<b>Remote Desktop Protocol</b>";
    let template =
        "Version:1.0\r\nStartHTML:-1\r\nEndHTML:-1\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    let start_fragment = template.len() + "<!--StartFragment-->".len();
    let end_fragment = start_fragment + fragment.len();
    let mut cf_html = format!(
        "Version:1.0\r\nStartHTML:-1\r\nEndHTML:-1\r\nStartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n<!--StartFragment-->{fragment}<!--EndFragment-->"
    )
    .into_bytes();
    let payload_len = cf_html.len();
    cf_html.extend_from_slice(b"allocator padding");

    assert_eq!(validate_cf_html(&cf_html).unwrap(), payload_len);
    assert_eq!(cf_html_to_plain_html(&cf_html).unwrap(), fragment);
}

#[test]
fn html_rejects_missing_or_ambiguous_required_headers() {
    let valid = plain_html_to_cf_html("<b>Remote Desktop Protocol</b>");
    let missing_start_html = valid.replacen("StartHTML:", "Unknown:", 1);
    assert!(validate_cf_html(missing_start_html.as_bytes()).is_err());

    let embedded_end_html = valid.replacen("EndHTML:", "UnknownEndHTML:", 1);
    assert!(validate_cf_html(embedded_end_html.as_bytes()).is_err());

    let duplicate_version = format!("Version:1.0\r\n{valid}");
    assert!(validate_cf_html(duplicate_version.as_bytes()).is_err());
}
