use ironrdp_egfx::avc444::{Yuv420Planes, Yuv444Frame};
use ironrdp_pdu::geometry::ExclusiveRectangle;

const W: usize = 64;
const H: usize = 48;

/// A YUV444 source frame with distinct values in every plane and position.
fn source_frame() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let mut y = vec![0u8; W * H];
    let mut u = vec![0u8; W * H];
    let mut v = vec![0u8; W * H];
    for row in 0..H {
        for col in 0..W {
            let i = row * W + col;
            y[i] = u8::try_from((row * 7 + col * 3) % 256).expect("fits");
            u[i] = u8::try_from((row * 11 + col * 5 + 17) % 256).expect("fits");
            v[i] = u8::try_from((row * 13 + col * 9 + 101) % 256).expect("fits");
        }
    }
    (y, u, v)
}

/// Owned YUV420p planes with strides equal to their widths.
struct Planes420 {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl Planes420 {
    fn new() -> Self {
        Self {
            y: vec![0; W * H],
            u: vec![0; W / 2 * H / 2],
            v: vec![0; W / 2 * H / 2],
        }
    }

    fn view(&self) -> Yuv420Planes<'_> {
        Yuv420Planes {
            y: &self.y,
            y_stride: W,
            u: &self.u,
            u_stride: W / 2,
            v: &self.v,
            v_stride: W / 2,
        }
    }
}

/// Split a YUV444 frame into main and auxiliary YUV420p views in YUV444v2
/// layout. A port of FreeRDP's encoder, `general_RGBToAVC444YUVv2_ANY_DOUBLE_ROW`
/// (libfreerdp/primitives/prim_YUV.c), working from YUV planes instead of RGB.
#[expect(clippy::similar_names, reason = "Y, U and V plane names")]
fn split_v2(y444: &[u8], u444: &[u8], v444: &[u8]) -> (Planes420, Planes420) {
    let (mut main, mut aux) = (Planes420::new(), Planes420::new());
    let (half, quarter) = (W / 2, W / 4);
    for row in (0..H).step_by(2) {
        for col in (0..W).step_by(2) {
            let (a, b, c, d) = (
                row * W + col,
                row * W + col + 1,
                (row + 1) * W + col,
                (row + 1) * W + col + 1,
            );

            // B1: luma.
            for i in [a, b, c, d] {
                main.y[i] = y444[i];
            }

            // B2, B3: 2x2 average of U and V.
            let average = |p: &[u8]| {
                u8::try_from((u32::from(p[a]) + u32::from(p[b]) + u32::from(p[c]) + u32::from(p[d])) / 4)
                    .expect("average of u8 fits")
            };
            main.u[row / 2 * half + col / 2] = average(u444);
            main.v[row / 2 * half + col / 2] = average(v444);

            // B4, B5: odd columns, both rows; U in the left half, V in the right.
            aux.y[row * W + col / 2] = u444[b];
            aux.y[row * W + half + col / 2] = v444[b];
            aux.y[(row + 1) * W + col / 2] = u444[d];
            aux.y[(row + 1) * W + half + col / 2] = v444[d];

            // B6-B9: even columns of the odd row; columns 4x go to the
            // auxiliary U plane, 4x+2 to the auxiliary V plane.
            let target = if col % 4 == 0 { &mut aux.u } else { &mut aux.v };
            target[row / 2 * half + col / 4] = u444[c];
            target[row / 2 * half + quarter + col / 4] = v444[c];
        }
    }
    (main, aux)
}

fn full_frame() -> [ExclusiveRectangle; 1] {
    [ExclusiveRectangle {
        left: 0,
        top: 0,
        right: u16::try_from(W).expect("fits"),
        bottom: u16::try_from(H).expect("fits"),
    }]
}

#[test]
fn main_then_auxiliary_view_reconstructs_yuv444() {
    let (y, u, v) = source_frame();
    let (main, aux) = split_v2(&y, &u, &v);

    let mut frame = Yuv444Frame::new(W, H).expect("frame");
    frame.apply_main_view(&main.view(), &full_frame()).expect("main view");
    frame
        .apply_auxiliary_view_v2(&aux.view(), &full_frame())
        .expect("auxiliary view");

    assert_eq!(frame.y(), y.as_slice(), "luma");
    for row in 0..H {
        for col in 0..W {
            let i = row * W + col;
            if row % 2 == 0 && col % 2 == 0 {
                // Only the main view's 2x2 average exists for these positions.
                let block = |p: &[u8]| {
                    (u32::from(p[i]) + u32::from(p[i + 1]) + u32::from(p[i + W]) + u32::from(p[i + W + 1])) / 4
                };
                assert_eq!(u32::from(frame.u()[i]), block(&u), "U average at ({col},{row})");
                assert_eq!(u32::from(frame.v()[i]), block(&v), "V average at ({col},{row})");
            } else {
                assert_eq!(frame.u()[i], u[i], "U at ({col},{row})");
                assert_eq!(frame.v()[i], v[i], "V at ({col},{row})");
            }
        }
    }
}

#[test]
fn main_view_alone_upsamples_chroma() {
    // A luma-only update shows at YUV420 quality: each 2x2 block takes the
    // main view's chroma sample.
    let (y, u, v) = source_frame();
    let (main, _) = split_v2(&y, &u, &v);

    let mut frame = Yuv444Frame::new(W, H).expect("frame");
    frame.apply_main_view(&main.view(), &full_frame()).expect("main view");

    for row in 0..H {
        for col in 0..W {
            let (i, j) = (row * W + col, row / 2 * (W / 2) + col / 2);
            assert_eq!(frame.u()[i], main.u[j]);
            assert_eq!(frame.v()[i], main.v[j]);
        }
    }
}

#[test]
fn updates_stay_inside_their_regions() {
    let (y, u, v) = source_frame();
    let (main, aux) = split_v2(&y, &u, &v);
    let mut frame = Yuv444Frame::new(W, H).expect("frame");
    frame.apply_main_view(&main.view(), &full_frame()).expect("main view");
    frame
        .apply_auxiliary_view_v2(&aux.view(), &full_frame())
        .expect("auxiliary view");
    let before = frame.clone();

    // A luma-only update to the top-left macroblock, from an all-black frame.
    let black = Planes420 {
        y: vec![0; W * H],
        u: vec![128; W / 2 * H / 2],
        v: vec![128; W / 2 * H / 2],
    };
    let region = [ExclusiveRectangle {
        left: 0,
        top: 0,
        right: 16,
        bottom: 16,
    }];
    frame.apply_main_view(&black.view(), &region).expect("main view");

    for row in 0..H {
        for col in 0..W {
            let i = row * W + col;
            if row < 16 && col < 16 {
                assert_eq!((frame.y()[i], frame.u()[i], frame.v()[i]), (0, 128, 128));
            } else {
                assert_eq!(
                    (frame.y()[i], frame.u()[i], frame.v()[i]),
                    (before.y()[i], before.u()[i], before.v()[i]),
                    "outside the region at ({col},{row})"
                );
            }
        }
    }
}

#[test]
fn regions_expand_to_whole_macroblocks() {
    // A region covering part of a macroblock updates the whole macroblock.
    let white = Planes420 {
        y: vec![255; W * H],
        u: vec![128; W / 2 * H / 2],
        v: vec![128; W / 2 * H / 2],
    };
    let mut frame = Yuv444Frame::new(W, H).expect("frame");
    let region = [ExclusiveRectangle {
        left: 20,
        top: 3,
        right: 21,
        bottom: 4,
    }];
    frame.apply_main_view(&white.view(), &region).expect("main view");
    for row in 0..H {
        for col in 0..W {
            let inside = (16..32).contains(&col) && row < 16;
            assert_eq!(frame.y()[row * W + col] == 255, inside, "({col},{row})");
        }
    }
}

#[test]
fn rejects_views_smaller_than_the_frame() {
    let mut frame = Yuv444Frame::new(W, H).expect("frame");
    let short = Planes420 {
        y: vec![0; W * H - 1],
        u: vec![0; W / 2 * H / 2],
        v: vec![0; W / 2 * H / 2],
    };
    assert!(frame.apply_main_view(&short.view(), &full_frame()).is_err());
    assert!(frame.apply_auxiliary_view_v2(&short.view(), &full_frame()).is_err());
}

#[test]
fn rejects_sizes_the_layout_cannot_hold() {
    // The auxiliary view packs chroma into quarters of each row, and 4:2:0
    // needs whole row pairs.
    assert!(Yuv444Frame::new(W + 2, H).is_err());
    assert!(Yuv444Frame::new(W, H + 1).is_err());
    assert!(Yuv444Frame::new(W, 0).is_err());
    // Cropped heights are fine, for example 1080 decoded from a 1088 stream.
    assert!(Yuv444Frame::new(1920, 1080).is_ok());
}

/// Through `GraphicsPipelineClient` with OpenH264: the two views are encoded
/// as one H.264 stream by a single encoder, as MS-RDPEGFX 2.2.4.5 requires,
/// sent as one AVC444v2 PDU, and decoded and combined by the client.
#[cfg(feature = "openh264-bundled")]
mod end_to_end {
    use std::sync::{Arc, Mutex};

    use ironrdp_core::encode_vec;
    use ironrdp_dvc::DvcProcessor as _;
    use ironrdp_egfx::client::{BitmapUpdate, GraphicsPipelineClient, GraphicsPipelineHandler};
    use ironrdp_egfx::decode::OpenH264Decoder;
    use ironrdp_egfx::pdu::{
        Avc420BitmapStream, Avc444BitmapStream, CapabilitiesConfirmPdu, CapabilitiesV107Flags, CapabilitySet,
        Codec1Type, CreateSurfacePdu, Encoding, GfxPdu, PixelFormat, QuantQuality, WireToSurface1Pdu,
    };
    use ironrdp_graphics::zgfx::wrap_uncompressed;

    use super::*;

    /// Collects the destination and RGBA of every bitmap update.
    struct Capture(Arc<Mutex<Vec<(ExclusiveRectangle, Vec<u8>)>>>);

    impl GraphicsPipelineHandler for Capture {
        fn on_bitmap_updated(&mut self, update: &BitmapUpdate) {
            assert_eq!(update.codec_id, Codec1Type::Avc444v2);
            self.0
                .lock()
                .expect("lock")
                .push((update.destination_rectangle.clone(), update.data.clone()));
        }
    }

    /// A source with one-pixel chroma stripes: U alternates by column, V by
    /// row. YUV420 averages them away; YUV444 keeps them.
    fn striped_source() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let mut y = vec![0u8; W * H];
        let mut u = vec![0u8; W * H];
        let mut v = vec![0u8; W * H];
        for row in 0..H {
            for col in 0..W {
                let i = row * W + col;
                y[i] = u8::try_from(64 + (row + col) % 128).expect("fits");
                u[i] = if col % 2 == 0 { 78 } else { 178 };
                v[i] = if row % 2 == 0 { 78 } else { 178 };
            }
        }
        (y, u, v)
    }

    fn mean_abs_error(a: &[u8], b: &[u8]) -> f64 {
        assert_eq!(a.len(), b.len());
        let total: u64 = a.iter().zip(b).map(|(x, y)| u64::from(x.abs_diff(*y))).sum();
        #[expect(clippy::cast_precision_loss, reason = "test statistic")]
        let mean = total as f64 / a.len() as f64;
        mean
    }

    fn avc420_stream<'a>(data: &'a [u8], regions: &[ExclusiveRectangle]) -> Avc420BitmapStream<'a> {
        Avc420BitmapStream {
            rectangles: regions.to_vec(),
            quant_qual_vals: vec![
                QuantQuality {
                    quantization_parameter: 22,
                    progressive: false,
                    quality: 100,
                };
                regions.len()
            ],
            data,
        }
    }

    fn process(client: &mut GraphicsPipelineClient, pdu: &GfxPdu) {
        let bytes = encode_vec(pdu).expect("encode PDU");
        client.process(0, &wrap_uncompressed(&bytes)).expect("process PDU");
    }

    /// Encode the main view then the auxiliary view with one encoder: one
    /// H.264 stream.
    fn encode_views(main: &Planes420, aux: &Planes420) -> (Vec<u8>, Vec<u8>) {
        let config = openh264::encoder::EncoderConfig::new()
            .skip_frames(false)
            .bitrate(openh264::encoder::BitRate::from_bps(50_000_000));
        let mut encoder =
            openh264::encoder::Encoder::with_api_config(openh264::OpenH264API::from_source(), config).expect("encoder");
        let mut encode = |planes: &Planes420| {
            let yuv = openh264::formats::YUVSlices::new((&planes.y, &planes.u, &planes.v), (W, H), (W, W / 2, W / 2));
            encoder.encode(&yuv).expect("encode").to_vec()
        };
        let (main_h264, aux_h264) = (encode(main), encode(aux));
        assert!(!main_h264.is_empty() && !aux_h264.is_empty(), "encoder skipped a view");
        (main_h264, aux_h264)
    }

    /// Send one AVC444v2 PDU covering the whole surface, with both views
    /// updating `regions`, and return the bitmap updates it produces.
    fn decode_through_client(
        main_h264: &[u8],
        aux_h264: &[u8],
        regions: &[ExclusiveRectangle],
    ) -> Vec<(ExclusiveRectangle, Vec<u8>)> {
        let updates = Arc::new(Mutex::new(Vec::new()));
        let mut client = GraphicsPipelineClient::new(
            Box::new(Capture(Arc::clone(&updates))),
            Some(Box::new(OpenH264Decoder::new().expect("decoder"))),
        );
        process(
            &mut client,
            &GfxPdu::CapabilitiesConfirm(CapabilitiesConfirmPdu::from_typed(&CapabilitySet::V10_7 {
                flags: CapabilitiesV107Flags::empty(),
            })),
        );
        process(
            &mut client,
            &GfxPdu::CreateSurface(CreateSurfacePdu {
                surface_id: 1,
                width: u16::try_from(W).expect("fits"),
                height: u16::try_from(H).expect("fits"),
                pixel_format: PixelFormat::XRgb,
            }),
        );
        let stream = Avc444BitmapStream {
            encoding: Encoding::LUMA_AND_CHROMA,
            stream1: avc420_stream(main_h264, regions),
            stream2: Some(avc420_stream(aux_h264, regions)),
        };
        process(
            &mut client,
            &GfxPdu::WireToSurface1(WireToSurface1Pdu {
                surface_id: 1,
                codec_id: Codec1Type::Avc444v2,
                pixel_format: PixelFormat::XRgb,
                destination_rectangle: full_frame()[0].clone(),
                bitmap_data: encode_vec(&stream).expect("encode AVC444 stream"),
            }),
        );

        let updates = updates.lock().expect("lock").clone();
        updates
    }

    #[test]
    fn avc444v2_through_the_client_keeps_full_chroma() {
        let (y, u, v) = striped_source();
        let (main, aux) = split_v2(&y, &u, &v);
        let (main_h264, aux_h264) = encode_views(&main, &aux);

        // Both views update the same region: one bitmap update.
        let updates = decode_through_client(&main_h264, &aux_h264, &full_frame());
        assert_eq!(updates.len(), 1, "one bitmap update");
        let (rect, got) = &updates[0];
        assert_eq!(rect, &full_frame()[0]);

        // References without H.264 loss: the full reconstruction, and the
        // main view alone (what AVC420 would show).
        let mut full = Yuv444Frame::new(W, H).expect("frame");
        full.apply_main_view(&main.view(), &full_frame()).expect("main");
        full.apply_auxiliary_view_v2(&aux.view(), &full_frame()).expect("aux");
        let mut luma_only = Yuv444Frame::new(W, H).expect("frame");
        luma_only.apply_main_view(&main.view(), &full_frame()).expect("main");

        let to_444 = mean_abs_error(got, &full.to_rgba(&full_frame()[0]).expect("rgba"));
        let to_420 = mean_abs_error(got, &luma_only.to_rgba(&full_frame()[0]).expect("rgba"));
        assert!(
            to_444 * 4.0 < to_420,
            "expected the output to match YUV444, not YUV420: error {to_444:.2} vs {to_420:.2}"
        );
    }

    #[test]
    fn avc444v2_paints_only_the_updated_regions() {
        // The reconstructed frame is current only inside the PDU's regions;
        // painting the rest of the destination would overwrite the surface
        // with stale or empty frame content.
        let (y, u, v) = striped_source();
        let (main, aux) = split_v2(&y, &u, &v);
        let (main_h264, aux_h264) = encode_views(&main, &aux);
        let regions = [
            ExclusiveRectangle {
                left: 16,
                top: 16,
                right: 32,
                bottom: 32,
            },
            ExclusiveRectangle {
                left: 40,
                top: 8,
                right: 60,
                bottom: 20,
            },
        ];

        let updates = decode_through_client(&main_h264, &aux_h264, &regions);
        let rects: Vec<_> = updates.iter().map(|(rect, _)| rect.clone()).collect();
        assert_eq!(rects, regions);

        // Each update holds its own region's pixels, not the frame's top-left.
        let mut full = Yuv444Frame::new(W, H).expect("frame");
        full.apply_main_view(&main.view(), &full_frame()).expect("main");
        full.apply_auxiliary_view_v2(&aux.view(), &full_frame()).expect("aux");
        for (rect, data) in &updates {
            let at_origin = ExclusiveRectangle {
                left: 0,
                top: 0,
                right: rect.right - rect.left,
                bottom: rect.bottom - rect.top,
            };
            let to_region = mean_abs_error(data, &full.to_rgba(rect).expect("rgba"));
            let to_origin = mean_abs_error(data, &full.to_rgba(&at_origin).expect("rgba"));
            assert!(
                to_region * 4.0 < to_origin,
                "update for {rect:?} doesn't match its region: error {to_region:.2} vs {to_origin:.2} at the origin"
            );
        }
    }
}
