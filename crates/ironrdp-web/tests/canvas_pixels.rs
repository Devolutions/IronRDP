// Keep this before cfg so native builds retain the lint expectation for the empty crate.
#![expect(
    unused_crate_dependencies,
    reason = "only production canvas and image modules are under test"
)]
#![cfg(target_arch = "wasm32")]

use core::num::NonZeroU32;

use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::pdu::bitmap::{BitmapData, BitmapUpdateData, Compression};
use ironrdp::pdu::geometry::InclusiveRectangle;
use ironrdp::session::fast_path::ProcessorBuilder;
use ironrdp::session::image::DecodedImage;
use ironrdp_core::WriteBuf;
use wasm_bindgen::JsCast as _;
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

#[path = "../src/canvas.rs"]
mod canvas;
#[path = "../src/image.rs"]
mod image;

wasm_bindgen_test_configure!(run_in_browser);

fn rectangle([left, top, right, bottom]: [u16; 4]) -> InclusiveRectangle {
    InclusiveRectangle {
        left,
        top,
        right,
        bottom,
    }
}

fn patterned_image(width: u16, height: u16, seed: u8) -> DecodedImage {
    // The public bitmap decoder accepts bottom-up BGRX pixels.
    let bitmap_data: Vec<u8> = (0..height)
        .rev()
        .flat_map(|y| {
            (0..width).flat_map(move |x| {
                let [red, green, blue, _] = (u32::from(y) * u32::from(width) + u32::from(x)).to_le_bytes();
                [
                    blue.wrapping_add(seed),
                    green.wrapping_add(seed),
                    red.wrapping_add(seed),
                    0,
                ]
            })
        })
        .collect();
    let update = BitmapUpdateData {
        rectangles: vec![BitmapData {
            rectangle: rectangle([0, 0, width - 1, height - 1]),
            width,
            height,
            bits_per_pixel: 32,
            compression_flags: Compression::empty(),
            compressed_data_header: None,
            bitmap_data: &bitmap_data,
        }],
    };
    let mut source = DecodedImage::new(PixelFormat::RgbA32, width, height);
    ProcessorBuilder {
        io_channel_id: 0,
        user_channel_id: 0,
        share_id: 0,
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build()
    .process_bitmap_update(&mut source, update)
    .expect("populate framebuffer");
    source
}

struct Surface {
    canvas: canvas::Canvas,
    ctx: CanvasRenderingContext2d,
    expected: Vec<u8>,
    scratch: WriteBuf,
}

impl Surface {
    fn resize(&mut self, width: u16, height: u16) {
        self.canvas.resize(
            NonZeroU32::new(u32::from(width)).expect("nonzero width"),
            NonZeroU32::new(u32::from(height)).expect("nonzero height"),
        );
        self.expected = vec![0; usize::from(width) * usize::from(height) * 4];
    }

    fn draw_and_check(&mut self, source: &DecodedImage, coordinates: [u16; 4]) {
        let [mut left, top, mut right, mut bottom] = coordinates;
        let width = source.width();
        let height = source.height();
        let mut copied_bytes = 0;
        // Reference coverage is calculated independently of production clipping/extraction.
        if left <= right && top <= bottom && left < width && top < height {
            right = right.min(width - 1);
            bottom = bottom.min(height - 1);
            let columns = u32::from(right - left + 1);
            if (64 < bottom - top + 1 || 512 < columns) && u32::from(width) * 4 <= columns * 5 {
                left = 0;
                right = width - 1;
            }
            copied_bytes = usize::from(right - left + 1) * usize::from(bottom - top + 1) * 4;
            for y in top..=bottom {
                for x in left..=right {
                    let offset = (usize::from(y) * usize::from(width) + usize::from(x)) * 4;
                    self.expected[offset..offset + 3].copy_from_slice(&source.data()[offset..offset + 3]);
                    self.expected[offset + 3] = 255;
                }
            }
        }
        self.scratch.clear();
        self.canvas
            .draw_image(source, rectangle(coordinates), &mut self.scratch)
            .expect("draw image");
        assert_eq!(
            self.scratch.filled_len(),
            copied_bytes,
            "upload size for {coordinates:?}"
        );
        let actual = self
            .ctx
            .get_image_data(0.0, 0.0, f64::from(width), f64::from(height))
            .expect("read canvas")
            .data();
        assert!(
            actual.0 == self.expected,
            "canvas mismatch for {width}x{height}, region={coordinates:?}"
        );
    }
}

#[wasm_bindgen_test]
async fn canvas_matches_reference_through_sparse_full_clipped_and_resized_updates() {
    wasm_bindgen_test::console_log!("simd128={}", cfg!(target_feature = "simd128"));
    let element = web_sys::window()
        .expect("window")
        .document()
        .expect("document")
        .create_element("canvas")
        .expect("create canvas")
        .dyn_into::<HtmlCanvasElement>()
        .expect("canvas element");
    let canvas = canvas::Canvas::new(element.clone(), NonZeroU32::MIN, NonZeroU32::MIN).expect("canvas");
    let ctx = element
        .get_context("2d")
        .expect("context")
        .expect("2d context")
        .dyn_into::<CanvasRenderingContext2d>()
        .expect("canvas context");
    let mut surface = Surface {
        canvas,
        ctx,
        expected: Vec::new(),
        scratch: WriteBuf::new(),
    };
    for (width, height, regions) in [
        (
            1920,
            1080,
            vec![
                [37, 91, 100, 810],
                [13, 17, 44, 48],
                [37, 11, 1571, 75],
                [37, 11, 1572, 75],
                [37, 11, 1573, 75],
                [7, 11, 1806, 730],
                [1, 0, 1919, 1079],
                [0, 17, 1919, 50],
                [0, 17, 1919, 51],
                [385, 11, 2000, 75],
                [384, 11, 2000, 75],
                [383, 11, 2000, 75],
                [0, 1046, 2000, 2000],
                [0, 1045, 2000, 2000],
            ],
        ),
        (
            64,
            128,
            vec![
                [48, 0, 79, 127],
                [3, 120, 7, 150],
                [60, 120, 80, 150],
                [64, 0, 127, 127],
                [0, 128, 63, 150],
                [9, 3, 8, 7],
                [3, 9, 7, 8],
                [0, 0, u16::MAX, u16::MAX],
                [63, 127, 63, 127],
            ],
        ),
        (1921, 97, vec![[11, 17, 74, 81], [0, 3, 1920, 37], [1, 11, 1920, 75]]),
        (
            3840,
            2160,
            vec![[37, 91, 100, 1290], [0, 0, 3839, 2159], [1, 0, 3839, 2159]],
        ),
    ] {
        let source = patterned_image(width, height, 11);
        let original = source.data().to_vec();
        let background = DecodedImage::new(PixelFormat::RgbA32, width, height);
        for coordinates in regions {
            for initialized in [false, true] {
                surface.resize(width, height);
                if initialized {
                    surface.draw_and_check(&background, [0, 0, width - 1, height - 1]);
                }
                surface.draw_and_check(&source, coordinates);
                // A no-overlap update preserves pixels and leaves the reused scratch buffer empty.
                surface.draw_and_check(&source, [width, 0, width, height - 1]);
                // Let WebDriver collect results between the full-canvas comparisons.
                gloo_timers::future::TimeoutFuture::new(0).await;
            }
        }
        if width == 1920 {
            surface.resize(width, height);
            surface.draw_and_check(&source, [0, 0, width - 1, height - 1]);
            surface.draw_and_check(&source, [width, 0, width, height - 1]);
            let changed = patterned_image(width, height, 29);
            let unchanged = changed.data().to_vec();
            surface.draw_and_check(&changed, [37, 91, 100, 810]);
            surface.draw_and_check(&changed, [71, 103, 134, 822]);
            assert_eq!(changed.data(), unchanged);
        }
        assert_eq!(source.data(), original, "presentation must preserve the framebuffer");
        assert!(background.data().iter().all(|&byte| byte == 0));
    }
}

#[wasm_bindgen_test]
fn alpha_and_copy_cover_independent_offsets_and_simd_tails() {
    for length in 0..=67 {
        for source_offset in 0..16 {
            let source: Vec<u8> = (0..length + 32)
                .map(|index| u8::try_from(index * 37 % 256).expect("byte pattern"))
                .collect();
            let mut actual = source.clone();
            let mut expected = source.clone();
            for alpha in (source_offset + 3..source_offset + length).step_by(4) {
                expected[alpha] = 255;
            }
            canvas::make_opaque(&mut actual[source_offset..source_offset + length]);
            assert_eq!(actual, expected, "alpha length={length}, offset={source_offset}");
            for target_offset in 0..16 {
                let mut target = vec![0x5A; length + 32];
                let mut expected = target.clone();
                for index in 0..length {
                    expected[target_offset + index] = if index % 4 == 3 {
                        255
                    } else {
                        source[source_offset + index]
                    };
                }
                canvas::copy_opaque_rgba(
                    &source[source_offset..source_offset + length],
                    &mut target[target_offset..target_offset + length],
                );
                assert_eq!(
                    target, expected,
                    "copy length={length}, offsets={source_offset}/{target_offset}"
                );
            }
        }
    }
}
