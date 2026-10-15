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
            width: u32::try_from(W).expect("fits"),
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
fn region_past_the_right_edge_changes_nothing() {
    // The region clamps to an empty one at the frame's right edge, which
    // isn't a multiple of 4 here.
    let (y, u, v) = source_frame();
    let (main, aux) = split_v2(&y, &u, &v);
    let mut frame = Yuv444Frame::new(W - 7, H).expect("frame");
    let before = frame.clone();

    let past = [ExclusiveRectangle {
        left: u16::try_from(W).expect("fits"),
        top: 0,
        right: u16::try_from(W + 16).expect("fits"),
        bottom: u16::try_from(H).expect("fits"),
    }];
    frame.apply_main_view(&main.view(), &past).expect("main view");
    frame
        .apply_auxiliary_view_v2(&aux.view(), &past)
        .expect("auxiliary view");

    assert_eq!(frame.y(), before.y());
    assert_eq!(frame.u(), before.u());
    assert_eq!(frame.v(), before.v());
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
fn converts_full_range_bt709() {
    // MS-RDPEGFX's AVC is full-range BT.709: R = Y + 1.5748 (V - 128),
    // G = Y - 0.1873 (U - 128) - 0.4681 (V - 128), B = Y + 1.8556 (U - 128).
    // (Y, U, V) = (100, 180, 150) is (134.6, 79.9, 196.5).
    let planes = Planes420 {
        y: vec![100; W * H],
        u: vec![180; W / 2 * H / 2],
        v: vec![150; W / 2 * H / 2],
    };
    let mut frame = Yuv444Frame::new(W, H).expect("frame");
    frame.apply_main_view(&planes.view(), &full_frame()).expect("main view");
    let rgba = frame
        .to_rgba(&ExclusiveRectangle {
            left: 0,
            top: 0,
            right: 1,
            bottom: 1,
        })
        .expect("rgba");
    for (got, want) in rgba.iter().zip([135u8, 80, 197, 255]) {
        assert!(
            got.abs_diff(want) <= 1,
            "RGBA {rgba:?}, expected about [135, 80, 197, 255]"
        );
    }
}

#[test]
fn rejects_empty_frames() {
    assert!(Yuv444Frame::new(W, 0).is_err());
    assert!(Yuv444Frame::new(0, H).is_err());
}

#[test]
fn cropped_frame_reads_auxiliary_view_at_coded_width() {
    // The views are encoded W wide and the decoder crops them. The auxiliary
    // view's V half still starts at W / 2, not at half the cropped width.
    let (y, u, v) = source_frame();
    let (main, aux) = split_v2(&y, &u, &v);

    for (width, height) in [(W - 6, H), (W - 7, H - 1)] {
        let region = [ExclusiveRectangle {
            left: 0,
            top: 0,
            right: u16::try_from(width).expect("fits"),
            bottom: u16::try_from(height).expect("fits"),
        }];
        let mut frame = Yuv444Frame::new(width, height).expect("frame");
        frame.apply_main_view(&main.view(), &region).expect("main view");
        frame
            .apply_auxiliary_view_v2(
                &Yuv420Planes {
                    width: u32::try_from(width).expect("fits"),
                    ..aux.view()
                },
                &region,
            )
            .expect("auxiliary view");

        for row in 0..height {
            for col in 0..width {
                let (i, src) = (row * width + col, row * W + col);
                assert_eq!(frame.y()[i], y[src], "{width}x{height}: Y at ({col},{row})");
                if row % 2 == 1 || col % 2 == 1 {
                    assert_eq!(frame.u()[i], u[src], "{width}x{height}: U at ({col},{row})");
                    assert_eq!(frame.v()[i], v[src], "{width}x{height}: V at ({col},{row})");
                }
            }
        }
    }
}

#[test]
fn frame_narrower_than_the_views_reads_auxiliary_view_at_their_width() {
    // A frame clipped to a surface narrower than the picture: the auxiliary
    // view's V half still starts at half the picture's width.
    let (y, u, v) = source_frame();
    let (main, aux) = split_v2(&y, &u, &v);
    let width = 16;
    let region = [ExclusiveRectangle {
        left: 0,
        top: 0,
        right: u16::try_from(width).expect("fits"),
        bottom: u16::try_from(H).expect("fits"),
    }];
    let mut frame = Yuv444Frame::new(width, H).expect("frame");
    frame.apply_main_view(&main.view(), &region).expect("main view");
    frame
        .apply_auxiliary_view_v2(&aux.view(), &region)
        .expect("auxiliary view");

    for row in 0..H {
        for col in 0..width {
            let (i, src) = (row * width + col, row * W + col);
            assert_eq!(frame.y()[i], y[src], "Y at ({col},{row})");
            if row % 2 == 1 || col % 2 == 1 {
                assert_eq!(frame.u()[i], u[src], "U at ({col},{row})");
                assert_eq!(frame.v()[i], v[src], "V at ({col},{row})");
            }
        }
    }
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
            assert!(matches!(update.codec_id, Codec1Type::Avc444v2 | Codec1Type::Avc444));
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
        let (result, updates) = send_avc444(main_h264, aux_h264, regions);
        result.expect("process AVC444 PDU");
        updates
    }

    /// Send one AVC444v2 PDU as `decode_through_client` does, returning
    /// whether the client accepted it.
    fn send_avc444(
        main_h264: &[u8],
        aux_h264: &[u8],
        regions: &[ExclusiveRectangle],
    ) -> (ironrdp_pdu::PduResult<()>, Vec<(ExclusiveRectangle, Vec<u8>)>) {
        send_stream(&Avc444BitmapStream {
            encoding: Encoding::LUMA_AND_CHROMA,
            stream1: avc420_stream(main_h264, regions),
            stream2: Some(avc420_stream(aux_h264, regions)),
        })
    }

    /// Send `stream` as one AVC444v2 PDU to a new surface.
    fn send_stream(
        stream: &Avc444BitmapStream<'_>,
    ) -> (ironrdp_pdu::PduResult<()>, Vec<(ExclusiveRectangle, Vec<u8>)>) {
        let (mut client, updates) = client_with_surface();
        let result = send(&mut client, stream);
        let updates = updates.lock().expect("lock").clone();
        (result, updates)
    }

    type Updates = Arc<Mutex<Vec<(ExclusiveRectangle, Vec<u8>)>>>;

    /// A client with V10.7 confirmed and surface 1 created, and the bitmap
    /// updates it paints.
    fn client_with_surface() -> (GraphicsPipelineClient, Updates) {
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
        create_surface_1(&mut client);
        (client, updates)
    }

    fn create_surface_1(client: &mut GraphicsPipelineClient) {
        process(
            client,
            &GfxPdu::CreateSurface(CreateSurfacePdu {
                surface_id: 1,
                width: u16::try_from(W).expect("fits"),
                height: u16::try_from(H).expect("fits"),
                pixel_format: PixelFormat::XRgb,
            }),
        );
    }

    /// Send `stream` to surface 1 as one AVC444v2 PDU.
    fn send(client: &mut GraphicsPipelineClient, stream: &Avc444BitmapStream<'_>) -> ironrdp_pdu::PduResult<()> {
        send_as(client, Codec1Type::Avc444v2, stream)
    }

    /// Send `stream` to surface 1 as one PDU of `codec_id`.
    fn send_as(
        client: &mut GraphicsPipelineClient,
        codec_id: Codec1Type,
        stream: &Avc444BitmapStream<'_>,
    ) -> ironrdp_pdu::PduResult<()> {
        let pdu = GfxPdu::WireToSurface1(WireToSurface1Pdu {
            surface_id: 1,
            codec_id,
            pixel_format: PixelFormat::XRgb,
            destination_rectangle: full_frame()[0].clone(),
            bitmap_data: encode_vec(stream).expect("encode AVC444 stream"),
        });
        let bytes = encode_vec(&pdu).expect("encode PDU");
        client.process(0, &wrap_uncompressed(&bytes)).map(drop)
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

    #[test]
    fn avc444v2_rejects_views_of_different_sizes() {
        // Both views are one frame. An auxiliary view of another size would
        // be read at the main view's half and quarter offsets.
        let (y, u, v) = striped_source();
        let (main, _) = split_v2(&y, &u, &v);
        let (main_h264, _) = encode_views(&main, &main);
        let wide = W + 16;
        let (wide_y, wide_chroma) = (vec![128u8; wide * H], vec![128u8; wide / 2 * H / 2]);
        let mut encoder = openh264::encoder::Encoder::with_api_config(
            openh264::OpenH264API::from_source(),
            openh264::encoder::EncoderConfig::new().skip_frames(false),
        )
        .expect("encoder");
        let yuv = openh264::formats::YUVSlices::new(
            (&wide_y, &wide_chroma, &wide_chroma),
            (wide, H),
            (wide, wide / 2, wide / 2),
        );
        let wide_h264 = encoder.encode(&yuv).expect("encode").to_vec();

        let (result, updates) = send_avc444(&main_h264, &wide_h264, &full_frame());
        assert!(
            result.is_err(),
            "accepted an auxiliary view {wide} wide for a {W}-wide main view"
        );
        assert!(updates.is_empty(), "painted a frame from mismatched views");
    }

    #[test]
    fn avc444v2_chroma_without_its_luma_paints_nothing() {
        // A chroma-only update refines the luma already on the surface. With
        // no luma of its size there, painting it would show chroma over black.
        let (y, u, v) = striped_source();
        let (main, aux) = split_v2(&y, &u, &v);
        // The first frame of its own stream, so it decodes on its own.
        let (aux_h264, _) = encode_views(&aux, &main);

        let (result, updates) = send_stream(&Avc444BitmapStream {
            encoding: Encoding::CHROMA,
            stream1: avc420_stream(&aux_h264, &full_frame()),
            stream2: None,
        });
        result.expect("a chroma-only update is not an error");
        assert!(updates.is_empty(), "painted chroma with no luma under it");
    }

    fn luma_only<'a>(main_h264: &'a [u8]) -> Avc444BitmapStream<'a> {
        Avc444BitmapStream {
            encoding: Encoding::LUMA,
            stream1: avc420_stream(main_h264, &full_frame()),
            stream2: None,
        }
    }

    fn chroma_only<'a>(aux_h264: &'a [u8]) -> Avc444BitmapStream<'a> {
        Avc444BitmapStream {
            encoding: Encoding::CHROMA,
            stream1: avc420_stream(aux_h264, &full_frame()),
            stream2: None,
        }
    }

    #[test]
    fn avc444v2_luma_then_chroma_in_separate_updates_combine() {
        // A server can send the main view alone (LC = 1) and refine it later
        // with the auxiliary view alone (LC = 2); the second paints full chroma.
        let (y, u, v) = striped_source();
        let (main, aux) = split_v2(&y, &u, &v);
        let (main_h264, aux_h264) = encode_views(&main, &aux);
        let (mut client, updates) = client_with_surface();

        send(&mut client, &luma_only(&main_h264)).expect("luma-only update");
        send(&mut client, &chroma_only(&aux_h264)).expect("chroma-only update");

        let updates = updates.lock().expect("lock").clone();
        assert_eq!(updates.len(), 2, "one bitmap update per PDU");
        let mut full = Yuv444Frame::new(W, H).expect("frame");
        full.apply_main_view(&main.view(), &full_frame()).expect("main");
        full.apply_auxiliary_view_v2(&aux.view(), &full_frame()).expect("aux");
        let mut luma_only = Yuv444Frame::new(W, H).expect("frame");
        luma_only.apply_main_view(&main.view(), &full_frame()).expect("main");
        let got = &updates[1].1;
        let to_444 = mean_abs_error(got, &full.to_rgba(&full_frame()[0]).expect("rgba"));
        let to_420 = mean_abs_error(got, &luma_only.to_rgba(&full_frame()[0]).expect("rgba"));
        assert!(
            to_444 * 4.0 < to_420,
            "expected the chroma-only update to complete YUV444: error {to_444:.2} vs {to_420:.2}"
        );
    }

    #[test]
    fn avc444v2_recreated_surface_drops_the_old_frame() {
        // CreateSurface for an id in use starts a new surface; a chroma-only
        // update must not refine the old surface's luma.
        let (y, u, v) = striped_source();
        let (main, aux) = split_v2(&y, &u, &v);
        let (main_h264, aux_h264) = encode_views(&main, &aux);
        let (mut client, updates) = client_with_surface();

        send(&mut client, &luma_only(&main_h264)).expect("luma-only update");
        create_surface_1(&mut client);
        send(&mut client, &chroma_only(&aux_h264)).expect("chroma-only update");

        assert_eq!(
            updates.lock().expect("lock").len(),
            1,
            "painted chroma over the old surface's luma"
        );
    }

    #[test]
    fn avc444v1_paints_the_main_view() {
        // The v1 auxiliary layout isn't reconstructed: the main view is
        // painted at YUV420 chroma. The auxiliary view still goes through the
        // decoder, since the next frame of the stream may reference it.
        let (y, u, v) = striped_source();
        let (main, aux) = split_v2(&y, &u, &v);
        let mut encoder = openh264::encoder::Encoder::with_api_config(
            openh264::OpenH264API::from_source(),
            openh264::encoder::EncoderConfig::new()
                .skip_frames(false)
                .bitrate(openh264::encoder::BitRate::from_bps(50_000_000)),
        )
        .expect("encoder");
        let mut encode = |planes: &Planes420| {
            let yuv = openh264::formats::YUVSlices::new((&planes.y, &planes.u, &planes.v), (W, H), (W, W / 2, W / 2));
            encoder.encode(&yuv).expect("encode").to_vec()
        };
        let (main_h264, aux_h264, next_main_h264) = (encode(&main), encode(&aux), encode(&main));
        let (mut client, updates) = client_with_surface();

        send_as(
            &mut client,
            Codec1Type::Avc444,
            &Avc444BitmapStream {
                encoding: Encoding::LUMA_AND_CHROMA,
                stream1: avc420_stream(&main_h264, &full_frame()),
                stream2: Some(avc420_stream(&aux_h264, &full_frame())),
            },
        )
        .expect("v1 update");
        send_as(&mut client, Codec1Type::Avc444, &luma_only(&next_main_h264)).expect("v1 luma-only update");

        let updates = updates.lock().expect("lock").clone();
        assert_eq!(updates.len(), 2, "one bitmap update per PDU");
        let mut luma = Yuv444Frame::new(W, H).expect("frame");
        luma.apply_main_view(&main.view(), &full_frame()).expect("main");
        let want = luma.to_rgba(&full_frame()[0]).expect("rgba");
        for (i, (rect, got)) in updates.iter().enumerate() {
            assert_eq!(rect, &full_frame()[0]);
            let error = mean_abs_error(got, &want);
            assert!(error < 4.0, "update {i} doesn't match the main view: error {error:.2}");
        }
    }
}
