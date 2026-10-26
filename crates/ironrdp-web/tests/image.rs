#![expect(unused_crate_dependencies, reason = "only the image extraction module is under test")]

use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::pdu::bitmap::{BitmapData, BitmapUpdateData, Compression};
use ironrdp::pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp::session::fast_path::ProcessorBuilder;
use ironrdp::session::image::DecodedImage;
use ironrdp_core::WriteBuf;

#[path = "../src/image.rs"]
mod image;

fn rectangle(left: u16, top: u16, width: u16, height: u16) -> InclusiveRectangle {
    InclusiveRectangle {
        left,
        top,
        right: left + width - 1,
        bottom: top + height - 1,
    }
}

fn pixel(width: u16, x: u16, y: u16) -> [u8; 4] {
    let [red, green, blue, _] = (u32::from(y) * u32::from(width) + u32::from(x)).to_le_bytes();
    [red, green, blue, 255]
}

fn patterned_image(width: u16, height: u16) -> DecodedImage {
    // Populate the real framebuffer through the public bitmap-update API.
    // Uncompressed wire pixels are bottom-up BGRX; the framebuffer is RGBA.
    let bitmap_data: Vec<u8> = (0..height)
        .rev()
        .flat_map(|y| {
            (0..width).flat_map(move |x| {
                let [red, green, blue, _] = pixel(width, x, y);
                [blue, green, red, 0]
            })
        })
        .collect();
    let update = BitmapUpdateData {
        rectangles: vec![BitmapData {
            rectangle: rectangle(0, 0, width, height),
            width,
            height,
            bits_per_pixel: 32,
            compression_flags: Compression::empty(),
            compressed_data_header: None,
            bitmap_data: &bitmap_data,
        }],
    };
    let mut image = DecodedImage::new(PixelFormat::RgbA32, width, height);
    ProcessorBuilder {
        io_channel_id: 0,
        user_channel_id: 0,
        share_id: 0,
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build()
    .process_bitmap_update(&mut image, update)
    .expect("populate test framebuffer");
    image
}

fn assert_pixels(data: &[u8], image_width: u16, region: &InclusiveRectangle) {
    assert_eq!(
        data.len(),
        usize::from(region.width()) * usize::from(region.height()) * 4
    );
    let expected = (region.top..=region.bottom)
        .flat_map(|y| (region.left..=region.right).flat_map(move |x| pixel(image_width, x, y)));
    assert!(data.iter().copied().eq(expected), "pixel mismatch for {region:?}");
}

fn assert_extraction(image: &DecodedImage, region: InclusiveRectangle, whole_rows: bool) {
    let mut buffer = WriteBuf::new();
    let actual = image::extract_partial_image(image, region.clone(), &mut buffer);
    let expected = if whole_rows {
        InclusiveRectangle {
            left: 0,
            right: image.width() - 1,
            ..region
        }
    } else {
        region.clone()
    };
    assert_eq!(actual, expected, "unexpected extraction for {region:?}");
    assert_pixels(buffer.filled(), image.width(), &actual);
}

#[test]
fn dirty_regions_extract_the_selected_rectangle_and_pixels() {
    let image = patterned_image(1920, 1080);
    for (region, whole_rows) in [
        (rectangle(0, 0, 64, 720), false),
        (rectangle(19, 23, 512, 720), false),
        (rectangle(71, 19, 1280, 720), false),
        (rectangle(71, 19, 1536, 720), true),
        (rectangle(0, 0, 1920, 1080), true),
        (rectangle(15, 22, 32, 32), false),
        (rectangle(1300, 37, 600, 1), false),
        (rectangle(100, 17, 960, 1), false),
        (rectangle(100, 17, 1536, 1), true),
        (rectangle(1919, 1079, 1, 1), false),
        (rectangle(1856, 360, 64, 720), false),
        (rectangle(640, 360, 1280, 720), false),
        (rectangle(384, 360, 1536, 720), true),
        (rectangle(0, 1, 1, 1079), false),
    ] {
        assert_extraction(&image, region, whole_rows);
    }
}

#[test]
fn whole_rows_are_limited_to_a_quarter_extra_pixels() {
    let image = patterned_image(1920, 96);
    for (width, whole_rows) in [(1535, false), (1536, true), (1537, true)] {
        assert_extraction(&image, rectangle(37, 11, width, 65), whole_rows);
    }
}

#[test]
fn small_regions_keep_the_existing_height_and_width_thresholds() {
    let image = patterned_image(640, 96);
    for (width, height, whole_rows) in [
        (511, 65, false),
        (512, 64, false),
        (512, 65, true),
        (513, 64, true),
        (512, 1, false),
        (513, 1, true),
    ] {
        assert_extraction(&image, rectangle(7, 9, width, height), whole_rows);
    }
}

#[test]
fn extraction_appends_at_the_cursor_and_supports_clear_and_reuse() {
    let image = patterned_image(1024, 96);
    let mut buffer = WriteBuf::new();
    let prefix = [0x12, 0x34, 0x56];
    buffer.write_slice(&prefix);

    // Append both compact and whole-row extractions after existing bytes.
    let small = rectangle(13, 17, 16, 3);
    let small_result = image::extract_partial_image(&image, small.clone(), &mut buffer);
    assert_eq!(small_result, small);
    assert_eq!(&buffer.filled()[..prefix.len()], &prefix);
    assert_pixels(&buffer.filled()[prefix.len()..], image.width(), &small_result);
    let first_end = buffer.filled_len();

    let wide = rectangle(17, 9, 900, 65);
    let wide_result = image::extract_partial_image(&image, wide.clone(), &mut buffer);
    assert_eq!(wide_result, rectangle(0, 9, image.width(), 65));
    assert_eq!(&buffer.filled()[..prefix.len()], &prefix);
    assert_pixels(&buffer.filled()[prefix.len()..first_end], image.width(), &small_result);
    assert_pixels(&buffer.filled()[first_end..], image.width(), &wide_result);

    // Reuse a previously large buffer for a narrow region and then a wide one.
    for region in [small, wide] {
        buffer.clear();
        let result = image::extract_partial_image(&image, region, &mut buffer);
        assert_pixels(buffer.filled(), image.width(), &result);
    }
}

#[test]
fn overdraw_comparison_does_not_overflow_u16_dimensions() {
    let image = patterned_image(u16::MAX, 3);
    for (width, whole_rows) in [(52_427, false), (52_428, true), (52_429, true), (60_000, true)] {
        assert_extraction(&image, rectangle(1, 1, width, 2), whole_rows);
    }
    assert_extraction(&image, rectangle(0, 0, u16::MAX, 3), true);
}

#[test]
fn extraction_preserves_transparent_pixels_from_a_new_framebuffer() {
    let image = DecodedImage::new(PixelFormat::RgbA32, 1024, 96);
    let mut buffer = WriteBuf::new();
    for (region, expected) in [
        (rectangle(13, 17, 16, 3), rectangle(13, 17, 16, 3)),
        (rectangle(17, 9, 900, 65), rectangle(0, 9, 1024, 65)),
    ] {
        buffer.clear();
        let result = image::extract_partial_image(&image, region, &mut buffer);
        assert_eq!(result, expected);
        assert_eq!(
            buffer.filled_len(),
            usize::from(result.width()) * usize::from(result.height()) * 4
        );
        assert!(buffer.filled().iter().all(|&byte| byte == 0));
    }
}
