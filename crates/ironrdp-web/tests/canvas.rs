#![expect(unused_crate_dependencies, reason = "only the canvas pixel helpers are under test")]

#[expect(dead_code, reason = "the browser canvas methods are not used by the pixel tests")]
#[path = "../src/canvas.rs"]
mod canvas;

#[path = "../src/image.rs"]
mod image;

use ironrdp::graphics::image_processing::PixelFormat;
use ironrdp::pdu::Action;
use ironrdp::pdu::codecs::rfx::{self, Block, CodecChannel};
use ironrdp::pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp::pdu::geometry::{ExclusiveRectangle, InclusiveRectangle};
use ironrdp::pdu::surface_commands::{ExtendedBitmapDataPdu, SurfaceBitsPdu, SurfaceCommand};
use ironrdp::session::image::DecodedImage;
use ironrdp::session::{ActiveStageBuilder, ActiveStageOutput};
use ironrdp_core::{WriteBuf, encode_vec};

#[test]
fn regions_are_clipped_before_dimensions_are_calculated() {
    let rectangle = |[left, top, right, bottom]: [u16; 4]| InclusiveRectangle {
        left,
        top,
        right,
        bottom,
    };

    for ((width, height), coordinates, expected) in [
        ((64, 128), [13, 17, 28, 19], Some([13, 17, 28, 19])),
        ((64, 128), [48, 0, 79, 127], Some([48, 0, 63, 127])),
        ((64, 128), [3, 120, 7, 150], Some([3, 120, 7, 127])),
        ((64, 128), [60, 120, 80, 150], Some([60, 120, 63, 127])),
        ((64, 128), [64, 0, 79, 127], None),
        ((64, 128), [0, 128, 63, 150], None),
        ((64, 128), [9, 3, 8, 7], None),
        ((64, 128), [3, 9, 7, 8], None),
        ((64, 128), [0, 0, u16::MAX, u16::MAX], Some([0, 0, 63, 127])),
        ((64, 128), [63, 127, 63, 127], Some([63, 127, 63, 127])),
        ((0, 128), [0, 0, 63, 127], None),
        ((64, 0), [0, 0, 63, 127], None),
    ] {
        let source = DecodedImage::new(PixelFormat::RgbA32, width, height);
        let region = rectangle(coordinates);
        assert_eq!(
            canvas::clip_region(&source, &region),
            expected.map(rectangle),
            "framebuffer={width}x{height}, region={region:?}"
        );
    }
}

#[test]
fn remotefx_region_outside_the_framebuffer_is_clipped_before_extraction() {
    // An empty tile set still reports its region, even when the RFX channel is wider
    // than the framebuffer. Previously whole-row extraction hid its right overflow.
    let mut codec = Vec::new();
    for block in [
        Block::Sync(rfx::SyncPdu),
        Block::CodecVersions(rfx::CodecVersionsPdu),
        Block::Channels(rfx::ChannelsPdu(vec![rfx::RfxChannel { width: 80, height: 128 }])),
        Block::CodecChannel(CodecChannel::Context(rfx::ContextPdu {
            flags: rfx::OperatingMode::empty(),
            entropy_algorithm: rfx::EntropyAlgorithm::Rlgr3,
        })),
        Block::CodecChannel(CodecChannel::FrameBegin(rfx::FrameBeginPdu {
            index: 1,
            number_of_regions: 1,
        })),
        Block::CodecChannel(CodecChannel::Region(rfx::RegionPdu {
            rectangles: vec![rfx::RfxRectangle {
                x: 48,
                y: 0,
                width: 32,
                height: 128,
            }],
        })),
        Block::CodecChannel(CodecChannel::TileSet(rfx::TileSetPdu {
            entropy_algorithm: rfx::EntropyAlgorithm::Rlgr3,
            quants: Vec::new(),
            tiles: Vec::new(),
        })),
        Block::CodecChannel(CodecChannel::FrameEnd(rfx::FrameEndPdu)),
    ] {
        codec.extend(encode_vec(&block).expect("encode RFX block"));
    }
    let surface = encode_vec(&SurfaceCommand::SetSurfaceBits(SurfaceBitsPdu {
        destination: ExclusiveRectangle {
            left: 0,
            top: 0,
            right: 80,
            bottom: 128,
        },
        extended_bitmap_data: ExtendedBitmapDataPdu {
            bpp: 32,
            codec_id: 3,
            width: 80,
            height: 128,
            header: None,
            data: &codec,
        },
    }))
    .expect("encode surface command");
    let update = encode_vec(&FastPathUpdatePdu {
        fragmentation: Fragmentation::Single,
        update_code: UpdateCode::SurfaceCommands,
        compression_flags: None,
        compression_type: None,
        data: &surface,
    })
    .expect("encode FastPath update");
    let mut wire =
        encode_vec(&FastPathHeader::new(EncryptionFlags::empty(), update.len())).expect("encode FastPath header");
    wire.extend(update);

    let mut source = DecodedImage::new(PixelFormat::RgbA32, 64, 128);
    let original = source.data().to_vec();
    let mut stage = ActiveStageBuilder {
        static_channels: Default::default(),
        user_channel_id: 1001,
        io_channel_id: 1003,
        message_channel_id: None,
        share_id: 1,
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build();
    let outputs = stage
        .process(&mut source, Action::FastPath, &wire, None)
        .expect("decode RFX surface update");
    let region = outputs
        .into_iter()
        .find_map(|output| match output {
            ActiveStageOutput::GraphicsUpdate(region) => Some(region),
            _ => None,
        })
        .expect("empty RFX tile set reports its region");
    assert_eq!(
        region,
        InclusiveRectangle {
            left: 48,
            top: 0,
            right: 79,
            bottom: 127,
        }
    );

    let clipped = canvas::clip_region(&source, &region).expect("region overlaps framebuffer");
    let mut buffer = WriteBuf::new();
    let selected = image::extract_partial_image(&source, clipped, &mut buffer);
    assert_eq!(
        selected,
        InclusiveRectangle {
            left: 48,
            top: 0,
            right: 63,
            bottom: 127,
        }
    );
    assert_eq!(buffer.filled_len(), 16 * 128 * 4);
    assert!(buffer.filled().iter().all(|&byte| byte == 0));
    assert_eq!(source.data(), original);
}

#[test]
fn complete_pixels_are_opaque_without_changing_rgb_partial_pixels_or_surrounding_bytes() {
    const PATTERN: [u8; 12] = [11, 41, 95, 0, 29, 97, 183, 127, 211, 137, 55, 255];

    // Cover every SIMD alignment and tail, including empty slices and incomplete RGBA pixels.
    for length in 0..=67 {
        for offset in 0..16 {
            let mut buffer: Vec<u8> = (0..length + 32).map(|index| PATTERN[index % PATTERN.len()]).collect();
            let original = buffer.clone();

            canvas::make_opaque(&mut buffer[offset..offset + length]);

            assert_eq!(&buffer[..offset], &original[..offset]);
            assert_eq!(&buffer[offset + length..], &original[offset + length..]);
            for (index, &byte) in original[offset..offset + length].iter().enumerate() {
                let expected = if index % 4 == 3 { 255 } else { byte };
                assert_eq!(
                    buffer[offset + index],
                    expected,
                    "length={length}, offset={offset}, byte={index}"
                );
            }
        }
    }
}

#[test]
fn opaque_copy_preserves_rgb_tails_and_surrounding_bytes_at_independent_offsets() {
    for length in 0..=67 {
        for source_offset in 0..16 {
            for target_offset in 0..16 {
                let source: Vec<u8> = (0..length + 32)
                    .map(|index| u8::try_from(index * 37 % 256).expect("byte pattern"))
                    .collect();
                let original = source.clone();
                let mut target = vec![0x5A; length + 32];
                canvas::copy_opaque_rgba(
                    &source[source_offset..source_offset + length],
                    &mut target[target_offset..target_offset + length],
                );
                assert_eq!(source, original);
                assert!(target[..target_offset].iter().all(|&byte| byte == 0x5A));
                assert!(target[target_offset + length..].iter().all(|&byte| byte == 0x5A));
                for index in 0..length {
                    let expected = if index % 4 == 3 {
                        255
                    } else {
                        source[source_offset + index]
                    };
                    assert_eq!(target[target_offset + index], expected);
                }
            }
        }
    }
}

#[test]
fn opaque_rows_append_and_reuse_without_mutating_a_transparent_framebuffer() {
    for width in [1919, 1920, 1921] {
        let source = DecodedImage::new(PixelFormat::RgbA32, width, 1080);
        let original = source.data().to_vec();
        let mut buffer = WriteBuf::new();
        for (top, bottom) in [(0, 1079), (17, 18), (23, 64)] {
            buffer.clear();
            buffer.write_slice(&[11, 23, 47]);
            canvas::copy_opaque_rows(
                &source,
                &InclusiveRectangle {
                    left: 0,
                    top,
                    right: width - 1,
                    bottom,
                },
                &mut buffer,
            );
            assert_eq!(&buffer.filled()[..3], &[11, 23, 47]);
            assert_eq!(
                buffer.filled_len(),
                3 + usize::from(width) * usize::from(bottom - top + 1) * 4
            );
            assert!(
                buffer.filled()[3..]
                    .chunks_exact(4)
                    .all(|pixel| pixel == [0, 0, 0, 255])
            );
            assert_eq!(source.data(), original);
        }
    }
}
