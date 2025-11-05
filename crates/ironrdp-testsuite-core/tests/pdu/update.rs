use ironrdp_connector::{connection_activation::ConnectionActivationSequence, DesktopSize};
use ironrdp_connector::{ConnectionResult as ConnResult, Credentials as ConnCreds};
use ironrdp_core::{decode, encode_vec};
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_pdu::bitmap::Compression as BitmapCompression;
use ironrdp_pdu::gcc;
use ironrdp_pdu::geometry::{InclusiveRectangle, Rectangle as _};
use ironrdp_pdu::rdp::capability_sets;
use ironrdp_pdu::rdp::client_info::TimezoneInfo;
use ironrdp_pdu::rdp::headers::{ShareDataHeader, ShareDataPduType};
use ironrdp_pdu::update::{BitmapDataOwned, BitmapUpdateOwned, ShareUpdate};
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageOutput};
use ironrdp_svc::StaticChannelSet;
use proptest::prelude::*;
use ironrdp_graphics::color_conversion::rdp_16bit_to_rgb;
use ironrdp_graphics::rdp6::BitmapStreamEncoder;
use ironrdp_pdu::bitmap::CompressedDataHeader;

// Build a minimal slow-path Update (Bitmap) ShareDataHeader payload and check decode + encode roundtrip.
#[test]
fn slow_path_update_bitmap_roundtrip() {
    // Build Update payload: updateType=Bitmap (0x0001), pad2octets=0x0000, then TS_UPDATE_BITMAP_DATA
    // TS_UPDATE_BITMAP_DATA: nrect=1, one TS_BITMAP_DATA rectangle
    // Rectangle: left=0, top=0, right=0, bottom=0
    // width=1, height=1, bpp=24, flags=0, dataLen=3, data=0xAA 0xBB 0xCC

    let update_payload: [u8; 2 + 2 + 2 + 8 + 2 + 2 + 2 + 2 + 2 + 3] = [
        // updateType (Bitmap)
        0x01, 0x00,
        // pad2octets
        0x00, 0x00,
        // TS_UPDATE_BITMAP_DATA header: nrect
        0x01, 0x00,
        // TS_BITMAP_DATA
        0x00, 0x00, 0x00, 0x00, // left, top
        0x00, 0x00, 0x00, 0x00, // right, bottom
        0x01, 0x00, // width
        0x01, 0x00, // height
        0x18, 0x00, // bpp = 24
        0x00, 0x00, // compression flags
        0x03, 0x00, // data len
        0xAA, 0xBB, 0xCC, // pixel data
    ];

    // ShareDataHeader fields (without ShareControlHeader):
    // pad=0x00, stream_id=Low(0x01), uncompressedLen=update_size + 1 + 1 + 2
    let uncompressed_len: u16 = (update_payload.len() as u16) + 4; // pduType+compType+compLen
    let mut header_bytes = Vec::new();
    header_bytes.extend_from_slice(&[0x00, 0x01]); // pad + streamPriority
    header_bytes.extend_from_slice(&uncompressed_len.to_le_bytes());
    header_bytes.push(ShareDataPduType::Update as u8); // pdu type
    header_bytes.push(0x00); // compression type + flags
    header_bytes.extend_from_slice(&[0x00, 0x00]); // compressed length
    header_bytes.extend_from_slice(&update_payload);

    // Decode ShareDataHeader.
    let decoded: ShareDataHeader = decode(header_bytes.as_slice()).expect("decode ShareDataHeader");
    // Quick smoke: ensure variant is Update.
    assert_eq!(decoded.share_data_pdu.as_short_name(), "Update PDU");

    // Encode back and compare.
    let reencoded = encode_vec(&decoded).expect("encode ShareDataHeader");
    assert_eq!(reencoded, header_bytes);
}

proptest! {
    #[test]
    fn x224_slow_path_update_applies_and_emits_region_via_active_stage(x in 0u16..32, y in 0u16..24, w in 1u16..16, h in 1u16..16) {
        use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
        use ironrdp_pdu::rdp::headers::{ShareControlHeader, ShareControlPdu, ShareDataPdu};
        use ironrdp_pdu::x224::X224 as X224Wrap;

        let img_w: u16 = 48;
        let img_h: u16 = 32;

        let right = (x.saturating_add(w).saturating_sub(1)).min(img_w - 1);
        let bottom = (y.saturating_add(h).saturating_sub(1)).min(img_h - 1);
        let left = x.min(right);
        let top = y.min(bottom);

        let io_channel_id = 1002;
        let user_channel_id = 1007;
        let desktop = DesktopSize { width: img_w, height: img_h };

        let config = ironrdp_connector::Config {
            desktop_size: desktop,
            desktop_scale_factor: 100,
            enable_tls: false,
            enable_credssp: false,
            credentials: ConnCreds::UsernamePassword { username: "u".into(), password: "p".into() },
            domain: None,
            client_build: 0,
            client_name: "test".into(),
            keyboard_type: gcc::KeyboardType::IbmEnhanced,
            keyboard_subtype: 0,
            keyboard_functional_keys_count: 12,
            keyboard_layout: 0,
            ime_file_name: String::new(),
            bitmap: None,
            dig_product_id: String::new(),
            client_dir: String::new(),
            platform: capability_sets::MajorPlatformType::UNSPECIFIED,
            hardware_id: None,
            request_data: None,
            autologon: false,
            enable_audio_playback: false,
            performance_flags: ironrdp_pdu::rdp::client_info::PerformanceFlags::empty(),
            license_cache: None,
            timezone_info: TimezoneInfo::default(),
            enable_server_pointer: true,
            pointer_software_rendering: false,
        };

        let connection_activation = ConnectionActivationSequence::new(config, io_channel_id, user_channel_id);
        let conn = ConnResult {
            io_channel_id,
            user_channel_id,
            static_channels: StaticChannelSet::new(),
            desktop_size: desktop,
            enable_server_pointer: true,
            pointer_software_rendering: false,
            connection_activation,
        };

        let mut stage = ActiveStage::new(conn);
        let mut image = DecodedImage::new(PixelFormat::RgbA32, img_w, img_h);

        let rect = InclusiveRectangle { left, top, right, bottom };
        let width = u16::from(rect.width());
        let height = u16::from(rect.height());
        let mut pixels = vec![0u8; usize::from(width) * usize::from(height) * 2];
        for (i, b) in pixels.iter_mut().enumerate() { *b = (i as u8).wrapping_mul(3).wrapping_add(1); }
        let pixels_clone = pixels.clone();

        let update = BitmapUpdateOwned { rectangles: vec![BitmapDataOwned {
            rectangle: rect.clone(), width, height, bits_per_pixel: 16,
            compression_flags: BitmapCompression::empty(), compressed_data_header: None, bitmap_data: pixels,
        }] };

        let share_control = ShareControlHeader {
            share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
                share_data_pdu: ShareDataPdu::Update(ShareUpdate::Bitmap(update)),
                stream_priority: ironrdp_pdu::rdp::headers::StreamPriority::Low,
                compression_flags: ironrdp_pdu::rdp::headers::CompressionFlags::empty(),
                compression_type: ironrdp_pdu::rdp::client_info::CompressionType::K8,
            }),
            pdu_source: user_channel_id,
            share_id: 1,
        };
        let user_data = encode_vec(&share_control).unwrap();
        let indication = McsMessage::SendDataIndication(SendDataIndication {
            initiator_id: user_channel_id,
            channel_id: io_channel_id,
            user_data: std::borrow::Cow::Owned(user_data),
        });
        let frame = encode_vec(&X224Wrap(indication)).unwrap();

        let outputs = stage.process(&mut image, ironrdp_pdu::Action::X224, &frame).unwrap();
        prop_assert!(outputs.iter().any(|o| matches!(o, ActiveStageOutput::GraphicsUpdate(r) if *r == rect)));

        // Verify framebuffer content matches the expected RGB converted from our 16bpp input.
        let img_data = image.data();
        let stride = image.stride();
        let bpp = image.bytes_per_pixel();

        for row in 0..usize::from(height) {
            // Source pixel rows are bottom-up in RDP; our decoder flips them when applying.
            let src_row = usize::from(height - 1) - row;
            let dst_y = usize::from(top) + row;

            for col in 0..usize::from(width) {
                let src_off = (src_row * usize::from(width) + col) * 2;
                let lo = pixels_clone[src_off];
                let hi = pixels_clone[src_off + 1];
                let rgb16 = u16::from_le_bytes([lo, hi]);
                let [r, g, b] = rdp_16bit_to_rgb(rgb16);

                let dst_x = usize::from(left) + col;
                let dst_idx = dst_y * stride + dst_x * bpp;
                let px = &img_data[dst_idx..dst_idx + 4];

                prop_assert_eq!(px, &[r, g, b, 0xFF]);
            }
        }
    }

    #[test]
    fn x224_slow_path_update_decompresses_and_writes_pixels_properly(
        x in 0u16..32, y in 0u16..24,
        // width multiple of 4 to satisfy scan_width % 4 == 0
        w_mult in 1u16..8, h in 1u16..16,
        rle in proptest::bool::ANY,
    ) {
        use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
        use ironrdp_pdu::rdp::headers::{ShareControlHeader, ShareControlPdu, ShareDataPdu};
        use ironrdp_pdu::x224::X224 as X224Wrap;

        let img_w: u16 = 64;
        let img_h: u16 = 48;

        let width = (w_mult * 4).min(img_w); // ensure divisible by 4
        let right = (x.saturating_add(width).saturating_sub(1)).min(img_w - 1);
        let bottom = (y.saturating_add(h).saturating_sub(1)).min(img_h - 1);
        let left = x.min(right);
        let top = y.min(bottom);
        let width = u16::from(InclusiveRectangle { left, top, right, bottom }.width());
        let height = u16::from(InclusiveRectangle { left, top, right, bottom }.height());

        let io_channel_id = 1002;
        let user_channel_id = 1007;
        let desktop = DesktopSize { width: img_w, height: img_h };

        let config = ironrdp_connector::Config {
            desktop_size: desktop,
            desktop_scale_factor: 100,
            enable_tls: false,
            enable_credssp: false,
            credentials: ConnCreds::UsernamePassword { username: "u".into(), password: "p".into() },
            domain: None,
            client_build: 0,
            client_name: "test".into(),
            keyboard_type: gcc::KeyboardType::IbmEnhanced,
            keyboard_subtype: 0,
            keyboard_functional_keys_count: 12,
            keyboard_layout: 0,
            ime_file_name: String::new(),
            bitmap: None,
            dig_product_id: String::new(),
            client_dir: String::new(),
            platform: capability_sets::MajorPlatformType::UNSPECIFIED,
            hardware_id: None,
            request_data: None,
            autologon: false,
            enable_audio_playback: false,
            performance_flags: ironrdp_pdu::rdp::client_info::PerformanceFlags::empty(),
            license_cache: None,
            timezone_info: TimezoneInfo::default(),
            enable_server_pointer: true,
            pointer_software_rendering: false,
        };

        let connection_activation = ConnectionActivationSequence::new(config, io_channel_id, user_channel_id);
        let conn = ConnResult {
            io_channel_id,
            user_channel_id,
            static_channels: StaticChannelSet::new(),
            desktop_size: desktop,
            enable_server_pointer: true,
            pointer_software_rendering: false,
            connection_activation,
        };

        let mut stage = ActiveStage::new(conn);
        let mut image = DecodedImage::new(PixelFormat::RgbA32, img_w, img_h);

        // Build a simple RGB test pattern (top-down) for the rectangle area
        let rect = InclusiveRectangle { left, top, right, bottom };
        let w = usize::from(width);
        let h = usize::from(height);
        let mut rgb24 = vec![0u8; w * h * 3];
        for row in 0..h {
            for col in 0..w {
                let idx = (row * w + col) * 3;
                rgb24[idx + 0] = (col as u8).wrapping_mul(3).wrapping_add(5); // R
                rgb24[idx + 1] = (row as u8).wrapping_mul(5).wrapping_add(7); // G
                rgb24[idx + 2] = (col as u8 ^ row as u8).wrapping_add(11);   // B
            }
        }

        // Encode using RDP6 BitmapStreamEncoder (RGB planes)
        let mut encoder = BitmapStreamEncoder::new(w, h);
        let mut pdu = vec![0u8; w * h * 4 + 2];
        let written = encoder.encode_bitmap::<ironrdp_graphics::rdp6::RgbChannels>(&rgb24, &mut pdu, rle).unwrap();
        pdu.truncate(written);

        // Build slow-path BitmapUpdate (32 bpp compressed RDP6)
        let main_body_size = u16::try_from(pdu.len()).unwrap_or(u16::MAX);
        let uncompressed_size = u16::try_from(w * h * 3).unwrap_or(u16::MAX);
        let update = BitmapUpdateOwned {
            rectangles: vec![BitmapDataOwned {
                rectangle: rect.clone(),
                width,
                height,
                bits_per_pixel: 32,
                compression_flags: BitmapCompression::BITMAP_COMPRESSION,
                compressed_data_header: Some(CompressedDataHeader {
                    main_body_size,
                    scan_width: width,
                    uncompressed_size,
                }),
                bitmap_data: pdu.clone(),
            }],
        };

        let share_control = ShareControlHeader {
            share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
                share_data_pdu: ShareDataPdu::Update(ShareUpdate::Bitmap(update)),
                stream_priority: ironrdp_pdu::rdp::headers::StreamPriority::Low,
                compression_flags: ironrdp_pdu::rdp::headers::CompressionFlags::empty(),
                compression_type: ironrdp_pdu::rdp::client_info::CompressionType::K8,
            }),
            pdu_source: user_channel_id,
            share_id: 1,
        };
        let user_data = encode_vec(&share_control).unwrap();
        let indication = McsMessage::SendDataIndication(SendDataIndication {
            initiator_id: user_channel_id,
            channel_id: io_channel_id,
            user_data: std::borrow::Cow::Owned(user_data),
        });
        let frame = encode_vec(&X224Wrap(indication)).unwrap();

        let outputs = stage.process(&mut image, ironrdp_pdu::Action::X224, &frame).unwrap();
        prop_assert!(outputs.iter().any(|o| matches!(o, ActiveStageOutput::GraphicsUpdate(r) if *r == rect)));

        // Verify the framebuffer contains the expected RGB (converted from our decompressed RGB24)
        let img_data = image.data();
        let stride = image.stride();
        let bpp = image.bytes_per_pixel();
        for row in 0..h {
            // The decoder produces top-down RGB24; apply_rgb24 uses flip=true, so rows are flipped
            let src_row = h - 1 - row;
            let dst_y = usize::from(top) + row;
            for col in 0..w {
                let sidx = (src_row * w + col) * 3;
                let r = rgb24[sidx];
                let g = rgb24[sidx + 1];
                let b = rgb24[sidx + 2];

                let dst_x = usize::from(left) + col;
                let didx = dst_y * stride + dst_x * bpp;
                let px = &img_data[didx..didx + 4];
                prop_assert_eq!(px, &[r, g, b, 0xFF]);
            }
        }
    }
}
