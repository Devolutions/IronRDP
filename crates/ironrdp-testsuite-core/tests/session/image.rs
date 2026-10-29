use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_session::image::DecodedImage;

fn pixel(image: &DecodedImage, x: usize, y: usize) -> [u8; 4] {
    let offset = (y * usize::from(image.width()) + x) * 4;
    image.data()[offset..offset + 4]
        .try_into()
        .expect("pixel has four channels")
}

/// Every pixel format, top-down and flipped: each pixel of the RGBA source
/// lands in its place with its channels reordered for the format, up to the
/// image's right and bottom edges, and nothing outside the rectangle changes.
#[test]
fn rgba32_lands_in_every_pixel_format() {
    let formats = [
        PixelFormat::ARgb32,
        PixelFormat::XRgb32,
        PixelFormat::ABgr32,
        PixelFormat::XBgr32,
        PixelFormat::BgrA32,
        PixelFormat::BgrX32,
        PixelFormat::RgbA32,
        PixelFormat::RgbX32,
    ];
    let rectangle = InclusiveRectangle {
        left: 1,
        top: 2,
        right: 5,
        bottom: 4,
    };
    let (width, height) = (5, 3);
    let source: Vec<u8> = (0..width * height * 4)
        .map(|i| u8::try_from(i % 251).expect("below u8::MAX"))
        .collect();
    for format in formats {
        for flip in [false, true] {
            let mut image = DecodedImage::new(format, 6, 5);
            image
                .apply_rgba32(&source, &rectangle, flip)
                .expect("rectangle fits the image");
            let [ri, gi, bi, ai] = format.channel_offsets();
            for y in 0..5 {
                for x in 0..6 {
                    let inside = (1..=5).contains(&x) && (2..=4).contains(&y);
                    let expected = if inside {
                        let row = if flip { height - 1 - (y - 2) } else { y - 2 };
                        let i = (row * width + (x - 1)) * 4;
                        let mut pixel = [0; 4];
                        pixel[ri] = source[i];
                        pixel[gi] = source[i + 1];
                        pixel[bi] = source[i + 2];
                        pixel[ai] = source[i + 3];
                        pixel
                    } else {
                        [0; 4]
                    };
                    assert_eq!(pixel(&image, x, y), expected, "{format:?} flip {flip} at ({x}, {y})");
                }
            }
        }
    }
}
