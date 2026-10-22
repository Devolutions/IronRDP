//! AVC444 and AVC444v2 decoding, from the server's sub-streams to the client's output.
//!
//! The views travel through a decoder that returns the planes it was given, so the tests check the
//! view packing, the stream handling and the color conversion without H.264 quantization.

use ironrdp_core::encode_vec;
use ironrdp_dvc::{DvcMessage, DvcProcessor as _};
use ironrdp_egfx::avc444::{Avc444Layout, Yuv444Frame};
use ironrdp_egfx::client::GraphicsPipelineClient;
use ironrdp_egfx::decode::{DecodedFrame, DecoderError, DecoderResult, H264Decoder, Yuv420Frame};
use ironrdp_egfx::pdu::{Avc420Region, CapabilitiesAdvertisePdu, CapabilitySet, Encoding};
use ironrdp_egfx::server::GraphicsPipelineServer;
use ironrdp_pdu::geometry::ExclusiveRectangle;
use rstest::rstest;

/// Neither dimension is a whole number of macroblocks, so the views carry padding.
const WIDTH: u16 = 40;
const HEIGHT: u16 = 24;

/// Largest per-channel difference the integer color transforms of MS-RDPEGFX 3.3.8.3.1 leave
/// after a round trip.
const TOLERANCE: u8 = 4;

// ============================================================================
// Plane-transport decoder
// ============================================================================

/// A decoder whose bitstream is the view itself, as written by [`pack`].
struct PlaneDecoder;

impl H264Decoder for PlaneDecoder {
    fn decode(&mut self, data: &[u8]) -> DecoderResult<DecodedFrame> {
        self.decode_yuv420(data).map(|view| view.to_rgba())
    }

    fn decode_yuv420(&mut self, data: &[u8]) -> DecoderResult<Yuv420Frame> {
        let header = data.get(..4).ok_or_else(|| DecoderError::msg("missing view header"))?;
        let width = u16::from_le_bytes([header[0], header[1]]);
        let height = u16::from_le_bytes([header[2], header[3]]);
        let (chroma_width, chroma_height) = (width.div_ceil(2), height.div_ceil(2));
        let luma_size = usize::from(width) * usize::from(height);
        let chroma_size = usize::from(chroma_width) * usize::from(chroma_height);
        let planes = &data[4..];
        if planes.len() != luma_size + 2 * chroma_size {
            return Err(DecoderError::msg("view size mismatch"));
        }
        let (y, chroma) = planes.split_at(luma_size);
        let (u, v) = chroma.split_at(chroma_size);
        Yuv420Frame::new(
            width,
            height,
            y.to_vec(),
            usize::from(width),
            u.to_vec(),
            v.to_vec(),
            usize::from(chroma_width),
        )
    }

    fn supports_yuv420(&self) -> bool {
        true
    }
}

fn pack(view: &Yuv420Frame) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(&view.width().to_le_bytes());
    data.extend_from_slice(&view.height().to_le_bytes());
    for row in (0..).map_while(|row| view.luma_row(row)) {
        data.extend_from_slice(row);
    }
    for row in (0..).map_while(|row| view.u_row(row)) {
        data.extend_from_slice(row);
    }
    for row in (0..).map_while(|row| view.v_row(row)) {
        data.extend_from_slice(row);
    }
    data
}

// ============================================================================
// Pictures
// ============================================================================

/// A per-pixel checkerboard of two colors, which 4:2:0 subsampling cannot keep.
///
/// The pair changes every eight pixels, so the picture also has large-scale structure. Each pair
/// differs enough in both U and V that the reverse chroma filter of MS-RDPEGFX 3.3.8.3.2 applies
/// rather than keeping the subsampled value.
fn checkerboard(seed: usize) -> Vec<u8> {
    checkerboard_of(seed, usize::from(WIDTH), usize::from(HEIGHT))
}

fn checkerboard_of(seed: usize, width: usize, height: usize) -> Vec<u8> {
    const PAIRS: [[[u8; 3]; 2]; 4] = [
        [[220, 30, 40], [20, 70, 230]],
        [[30, 200, 40], [200, 40, 220]],
        [[240, 220, 30], [20, 150, 170]],
        [[250, 120, 20], [40, 90, 250]],
    ];
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let pair = PAIRS[(x / 8 + y / 8 + seed) % PAIRS.len()];
            rgba.extend_from_slice(&pair[(x + y) % 2]);
            rgba.push(0xFF);
        }
    }
    rgba
}

fn max_difference(actual: &[u8], expected: &[u8]) -> u8 {
    assert_eq!(actual.len(), expected.len(), "pictures differ in size");
    actual
        .iter()
        .zip(expected)
        .map(|(actual, expected)| actual.abs_diff(*expected))
        .max()
        .unwrap_or(0)
}

fn full_rect() -> ExclusiveRectangle {
    ExclusiveRectangle {
        left: 0,
        top: 0,
        right: WIDTH,
        bottom: HEIGHT,
    }
}

fn region(left: u16, right: u16) -> Avc420Region {
    rect_region(left, 0, right, HEIGHT)
}

fn rect_region(left: u16, top: u16, right: u16, bottom: u16) -> Avc420Region {
    Avc420Region {
        left,
        top,
        right,
        bottom,
        quantization_parameter: 22,
        quality: 100,
    }
}

fn gray(level: u8) -> Vec<u8> {
    [level, level, level, 0xFF].repeat(usize::from(WIDTH) * usize::from(HEIGHT))
}

// ============================================================================
// Server to client
// ============================================================================

/// A server and a client that negotiated capabilities and share one mapped surface.
struct Session {
    server: GraphicsPipelineServer,
    client: GraphicsPipelineClient,
    surface_id: u16,
    /// What the client has painted, tightly packed RGBA.
    canvas: Vec<u8>,
}

impl Session {
    fn new() -> Self {
        struct ServerSink;
        impl ironrdp_egfx::server::GraphicsPipelineHandler for ServerSink {
            fn capabilities_advertise(&mut self, _pdu: &CapabilitiesAdvertisePdu) {}
            fn on_ready(&mut self, _negotiated: &CapabilitySet) {}
        }
        struct ClientSink;
        impl ironrdp_egfx::client::GraphicsPipelineHandler for ClientSink {}

        let mut server = GraphicsPipelineServer::new(Box::new(ServerSink));
        let mut client = GraphicsPipelineClient::new(Box::new(ClientSink), Some(Box::new(PlaneDecoder)));
        for advertise in client.start(0).expect("client start") {
            for confirm in server
                .process(0, &bytes(&advertise))
                .expect("server takes the advertisement")
            {
                client
                    .process(0, &bytes(&confirm))
                    .expect("client takes the confirmation");
            }
        }
        let mut session = Self {
            server,
            client,
            surface_id: 0,
            canvas: vec![0; usize::from(WIDTH) * usize::from(HEIGHT) * 4],
        };
        session.flush();

        session.server.set_output_dimensions(WIDTH, HEIGHT);
        session.surface_id = session.server.create_surface(WIDTH, HEIGHT).expect("surface");
        assert!(session.server.map_surface_to_output(session.surface_id, 0, 0));
        session.flush();
        session
    }

    /// Delivers the server's queued PDUs to the client and paints the client's output.
    fn flush(&mut self) {
        for message in self.server.drain_output() {
            let responses = self.client.process(0, &bytes(&message)).expect("client takes the PDU");
            for response in responses {
                self.server
                    .process(0, &bytes(&response))
                    .expect("server takes the response");
            }
        }
        let row_bytes = usize::from(WIDTH) * 4;
        for update in self.client.drain_output() {
            let width = usize::from(update.region.right - update.region.left) * 4;
            for (row, pixels) in update.data.chunks_exact(width).enumerate() {
                let start = (usize::from(update.region.top) + row) * row_bytes + usize::from(update.region.left) * 4;
                self.canvas[start..start + width].copy_from_slice(pixels);
            }
        }
    }

    fn send(
        &mut self,
        layout: Avc444Layout,
        encoding: Encoding,
        stream1: (&Yuv420Frame, &[Avc420Region]),
        stream2: Option<(&Yuv420Frame, &[Avc420Region])>,
    ) {
        let data1 = pack(stream1.0);
        let data2 = stream2.map(|(view, _)| pack(view));
        let regions2 = stream2.map(|(_, regions)| regions);
        let sent = match layout {
            Avc444Layout::V1 => {
                assert_ne!(
                    encoding,
                    Encoding::CHROMA,
                    "the server sends AVC444 chroma only with luma"
                );
                self.server
                    .try_send_avc444_frame(self.surface_id, &data1, stream1.1, data2.as_deref(), regions2, 0)
            }
            Avc444Layout::V2 => self.server.try_send_avc444v2_frame(
                self.surface_id,
                encoding,
                &data1,
                stream1.1,
                data2.as_deref(),
                regions2,
                0,
            ),
        };
        sent.expect("the server queues the frame");
        self.flush();
    }
}

fn bytes(message: &DvcMessage) -> Vec<u8> {
    encode_vec(message.as_ref()).expect("encode DVC message")
}

// ============================================================================
// Tests
// ============================================================================

// oracle: spec MS-RDPEGFX §3.3.8.3.2
#[rstest]
#[case::avc444(Avc444Layout::V1)]
#[case::avc444v2(Avc444Layout::V2)]
fn views_recombine_into_the_original_picture(#[case] layout: Avc444Layout) {
    let original = checkerboard(0);
    let picture = Yuv444Frame::from_rgba(&original, WIDTH, HEIGHT).expect("picture");
    let (main, auxiliary) = picture.split(layout).expect("views");

    let mut combined = Yuv444Frame::new(WIDTH, HEIGHT);
    combined.apply_main_view(&main, &[full_rect()]);
    let subsampled = combined.to_rgba(&full_rect());
    combined.apply_auxiliary_view(layout, &auxiliary, &[full_rect()]);
    let recombined = combined.to_rgba(&full_rect());

    let difference = max_difference(&recombined, &original);
    assert!(difference <= TOLERANCE, "difference {difference}");
    // The main view alone is the 4:2:0 picture, which loses the checkerboard's chroma.
    assert!(max_difference(&subsampled, &original) > 50);
}

/// Forward transform of MS-RDPEGFX 3.3.8.3.1, written from the specification's matrix.
fn spec_yuv(pixel: &[u8]) -> [u8; 3] {
    let [r, g, b] = [pixel[0], pixel[1], pixel[2]].map(i32::from);
    let clamp = |value: i32| u8::try_from(value.clamp(0, 255)).expect("clamped to a byte");
    [
        clamp((54 * r + 183 * g + 18 * b) >> 8),
        clamp(((-29 * r - 99 * g + 128 * b) >> 8) + 128),
        clamp(((128 * r - 116 * g - 12 * b) >> 8) + 128),
    ]
}

/// Each auxiliary view sample is the 4:4:4 sample the specification's B4 to B9 formulas place
/// there. The picture is three macroblocks wide, so the AVC444v2 halves start at an odd macroblock.
// oracle: spec MS-RDPEGFX §3.3.8.3.2
// oracle: spec MS-RDPEGFX §3.3.8.3.3
#[rstest]
#[case::avc444(Avc444Layout::V1)]
#[case::avc444v2(Avc444Layout::V2)]
fn auxiliary_views_follow_the_specification_layout(#[case] layout: Avc444Layout) {
    const W: usize = 48;
    const H: usize = 32;
    let mut rgba = Vec::with_capacity(W * H * 4);
    for i in 0..W * H {
        // Distinct, irregular colors, so a misplaced sample shows.
        let seed = u32::try_from(i).expect("index fits in u32").wrapping_mul(2_654_435_761);
        let [r, g, b, _] = seed.to_le_bytes();
        rgba.extend_from_slice(&[r, g, b, 0xFF]);
    }
    let yuv = |x: usize, y: usize| spec_yuv(&rgba[(y * W + x) * 4..]);
    let (u444, v444) = (|x, y| yuv(x, y)[1], |x, y| yuv(x, y)[2]);

    let picture = Yuv444Frame::from_rgba(&rgba, 48, 32).expect("picture");
    let (main, aux) = picture.split(layout).expect("views");
    for y in 0..H {
        let luma = main.luma_row(y).expect("main luma row");
        assert!((0..W).all(|x| luma[x] == yuv(x, y)[0]), "B1 row {y}");
    }

    let luma = |row| aux.luma_row(row).expect("auxiliary luma row");
    let (u, v) = (
        |row| aux.u_row(row).expect("auxiliary U row"),
        |row| aux.v_row(row).expect("auxiliary V row"),
    );
    match layout {
        Avc444Layout::V1 => {
            for macroblock in (0..H).step_by(16) {
                for y in 0..8 {
                    for x in 0..W {
                        assert_eq!(luma(macroblock + y)[x], u444(x, macroblock + 2 * y + 1), "B4");
                        assert_eq!(luma(macroblock + 8 + y)[x], v444(x, macroblock + 2 * y + 1), "B5");
                    }
                }
            }
            for y in 0..H / 2 {
                for x in 0..W / 2 {
                    assert_eq!(u(y)[x], u444(2 * x + 1, 2 * y), "B6");
                    assert_eq!(v(y)[x], v444(2 * x + 1, 2 * y), "B7");
                }
            }
        }
        Avc444Layout::V2 => {
            for y in 0..H {
                for x in 0..W / 2 {
                    assert_eq!(luma(y)[x], u444(2 * x + 1, y), "B4");
                    assert_eq!(luma(y)[W / 2 + x], v444(2 * x + 1, y), "B5");
                }
            }
            for y in 0..H / 2 {
                for x in 0..W / 4 {
                    assert_eq!(u(y)[x], u444(4 * x, 2 * y + 1), "B6");
                    assert_eq!(u(y)[W / 4 + x], v444(4 * x, 2 * y + 1), "B7");
                    assert_eq!(v(y)[x], u444(4 * x + 2, 2 * y + 1), "B8");
                    assert_eq!(v(y)[W / 4 + x], v444(4 * x + 2, 2 * y + 1), "B9");
                }
            }
        }
    }
}

/// Views packed straight from the specification's B1 to B9 formulas, with each 2x2 chroma
/// average in the main view, recombine into the original picture.
// oracle: spec MS-RDPEGFX §3.3.8.3.2
// oracle: spec MS-RDPEGFX §3.3.8.3.3
#[rstest]
#[case::avc444(Avc444Layout::V1)]
#[case::avc444v2(Avc444Layout::V2)]
fn views_packed_from_the_specification_recombine(#[case] layout: Avc444Layout) {
    const W: usize = 48;
    const H: usize = 32;
    let original = checkerboard_of(0, W, H);
    let yuv: Vec<[u8; 3]> = original.chunks_exact(4).map(spec_yuv).collect();
    let at = |x: usize, y: usize| yuv[y * W + x];
    let (u444, v444) = (|x, y| at(x, y)[1], |x, y| at(x, y)[2]);
    let average = |plane: usize, x: usize, y: usize| {
        let sum: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
            .iter()
            .map(|(dx, dy)| u32::from(at(2 * x + dx, 2 * y + dy)[plane]))
            .sum();
        u8::try_from((sum + 2) / 4).expect("average of bytes")
    };

    let (half_w, half_h) = (W / 2, H / 2);
    let main_y: Vec<u8> = (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .map(|(x, y)| at(x, y)[0])
        .collect();
    let main_u: Vec<u8> = (0..half_h)
        .flat_map(|y| (0..half_w).map(move |x| (x, y)))
        .map(|(x, y)| average(1, x, y))
        .collect();
    let main_v: Vec<u8> = (0..half_h)
        .flat_map(|y| (0..half_w).map(move |x| (x, y)))
        .map(|(x, y)| average(2, x, y))
        .collect();

    let (mut aux_y, mut aux_u, mut aux_v) = (vec![0; W * H], vec![0; half_w * half_h], vec![0; half_w * half_h]);
    match layout {
        Avc444Layout::V1 => {
            for macroblock in (0..H).step_by(16) {
                for y in 0..8 {
                    for x in 0..W {
                        aux_y[(macroblock + y) * W + x] = u444(x, macroblock + 2 * y + 1);
                        aux_y[(macroblock + 8 + y) * W + x] = v444(x, macroblock + 2 * y + 1);
                    }
                }
            }
            for y in 0..half_h {
                for x in 0..half_w {
                    aux_u[y * half_w + x] = u444(2 * x + 1, 2 * y);
                    aux_v[y * half_w + x] = v444(2 * x + 1, 2 * y);
                }
            }
        }
        Avc444Layout::V2 => {
            for y in 0..H {
                for x in 0..half_w {
                    aux_y[y * W + x] = u444(2 * x + 1, y);
                    aux_y[y * W + half_w + x] = v444(2 * x + 1, y);
                }
            }
            for y in 0..half_h {
                for x in 0..W / 4 {
                    aux_u[y * half_w + x] = u444(4 * x, 2 * y + 1);
                    aux_u[y * half_w + W / 4 + x] = v444(4 * x, 2 * y + 1);
                    aux_v[y * half_w + x] = u444(4 * x + 2, 2 * y + 1);
                    aux_v[y * half_w + W / 4 + x] = v444(4 * x + 2, 2 * y + 1);
                }
            }
        }
    }

    let (w16, h16) = (u16::try_from(W).expect("width"), u16::try_from(H).expect("height"));
    let main = Yuv420Frame::new(w16, h16, main_y, W, main_u, main_v, half_w).expect("main view");
    let aux = Yuv420Frame::new(w16, h16, aux_y, W, aux_u, aux_v, half_w).expect("auxiliary view");
    let everything = ExclusiveRectangle {
        left: 0,
        top: 0,
        right: w16,
        bottom: h16,
    };
    let mut combined = Yuv444Frame::new(w16, h16);
    combined.apply_main_view(&main, core::slice::from_ref(&everything));
    combined.apply_auxiliary_view(layout, &aux, core::slice::from_ref(&everything));

    let difference = max_difference(&combined.to_rgba(&everything), &original);
    assert!(difference <= TOLERANCE, "difference {difference}");
}

/// The bytes a picture is charged for match the bytes it holds, across macroblock edges.
// oracle: mutant allocation_size and allocated_bytes disagreeing, leaking or over-crediting the budget
#[test]
fn a_picture_holds_exactly_its_allocation_size() {
    let sizes = (0..=33).flat_map(|w| (0..=33).map(move |h| (w, h)));
    for (width, height) in sizes.chain([(u16::MAX, 1), (1, u16::MAX)]) {
        assert_eq!(
            Yuv444Frame::new(width, height).allocated_bytes(),
            Yuv444Frame::allocation_size(width, height),
            "{width}x{height}"
        );
    }
}

// oracle: property round-trip
#[test]
fn a_converted_picture_reads_back_unchanged() {
    let original = checkerboard(0);
    let picture = Yuv444Frame::from_rgba(&original, WIDTH, HEIGHT).expect("picture");
    let difference = max_difference(&picture.to_rgba(&full_rect()), &original);
    assert!(difference <= TOLERANCE, "difference {difference}");
}

// oracle: spec MS-RDPEGFX §3.3.8.3.2
#[rstest]
#[case::avc444(Avc444Layout::V1)]
#[case::avc444v2(Avc444Layout::V2)]
fn a_main_view_keeps_luma_outside_its_rectangles(#[case] layout: Avc444Layout) {
    let mut session = Session::new();
    let (main, auxiliary) = Yuv444Frame::from_rgba(&gray(100), WIDTH, HEIGHT)
        .expect("picture")
        .split(layout)
        .expect("views");
    let everything = [region(0, WIDTH)];
    session.send(
        layout,
        Encoding::LUMA_AND_CHROMA,
        (&main, &everything),
        Some((&auxiliary, &everything)),
    );

    // The main rectangle and the auxiliary rectangle split the first 2x2 block.
    let (main, auxiliary) = Yuv444Frame::from_rgba(&gray(200), WIDTH, HEIGHT)
        .expect("picture")
        .split(layout)
        .expect("views");
    session.send(
        layout,
        Encoding::LUMA_AND_CHROMA,
        (&main, &[rect_region(1, 0, 2, 1)]),
        Some((&auxiliary, &[rect_region(0, 0, 1, 1)])),
    );

    let pixel = |x: usize| &session.canvas[x * 4..x * 4 + 3];
    assert!(max_difference(pixel(0), &[100; 3]) <= TOLERANCE, "{:?}", pixel(0));
    assert!(max_difference(pixel(1), &[200; 3]) <= TOLERANCE, "{:?}", pixel(1));
}

// oracle: mutant the AVC444 picture allocated outside the compositor budget
#[test]
fn avc444_on_a_surface_the_compositor_refused_is_dropped() {
    use ironrdp_egfx::pdu::{
        Avc420BitmapStream, Avc444BitmapStream, CapabilitiesConfirmPdu, CapabilitiesV107Flags, Codec1Type,
        CreateSurfacePdu, GfxPdu, PixelFormat, QuantQuality, WireToSurface1Pdu,
    };
    use ironrdp_graphics::zgfx::wrap_uncompressed;

    struct ClientSink;
    impl ironrdp_egfx::client::GraphicsPipelineHandler for ClientSink {}
    let send = |client: &mut GraphicsPipelineClient, pdu: &GfxPdu| {
        let encoded = encode_vec(pdu).expect("encode PDU");
        client.process(0, &wrap_uncompressed(&encoded))
    };

    let mut client = GraphicsPipelineClient::new(Box::new(ClientSink), Some(Box::new(PlaneDecoder)));
    let confirm = CapabilitiesConfirmPdu::from_typed(&CapabilitySet::V10_7 {
        flags: CapabilitiesV107Flags::empty(),
    });
    send(&mut client, &GfxPdu::CapabilitiesConfirm(confirm)).expect("confirm");
    // A 1 GiB canvas exceeds the compositor budget, so the surface has no pixels.
    let create = CreateSurfacePdu {
        surface_id: 1,
        width: 16384,
        height: 16384,
        pixel_format: PixelFormat::XRgb,
    };
    send(&mut client, &GfxPdu::CreateSurface(create)).expect("create surface");

    let (main, _) = Yuv444Frame::from_rgba(&gray(100), WIDTH, HEIGHT)
        .expect("picture")
        .split(Avc444Layout::V1)
        .expect("views");
    let data = pack(&main);
    let rect = full_rect();
    let stream = Avc444BitmapStream {
        encoding: Encoding::LUMA,
        stream1: Avc420BitmapStream {
            rectangles: vec![rect.clone()],
            quant_qual_vals: vec![QuantQuality {
                quantization_parameter: 22,
                progressive: false,
                quality: 100,
            }],
            data: &data,
        },
        stream2: None,
    };
    let wire = WireToSurface1Pdu {
        surface_id: 1,
        codec_id: Codec1Type::Avc444,
        pixel_format: PixelFormat::XRgb,
        destination_rectangle: rect,
        bitmap_data: encode_vec(&stream).expect("encode AVC444 stream"),
    };
    send(&mut client, &GfxPdu::WireToSurface1(wire)).expect("the session survives");
    assert_eq!(client.dropped_updates(), 1);
}

// oracle: mutant AVC444 sets dropped or not negotiated with a YUV decoder
#[test]
fn the_client_negotiates_avc444_with_a_yuv_decoder() {
    let session = Session::new();
    assert!(
        matches!(
            session.client.negotiated_capabilities(),
            Some(CapabilitySet::V10_7 { .. })
        ),
        "negotiated {:?}",
        session.client.negotiated_capabilities()
    );
    assert!(session.client.codec_capabilities().avc444);
    assert!(session.server.supports_avc444());
}

// oracle: spec MS-RDPEGFX §2.2.4.5
#[rstest]
#[case::avc444(Avc444Layout::V1)]
#[case::avc444v2(Avc444Layout::V2)]
fn luma_and_chroma_round_trip_through_the_server(#[case] layout: Avc444Layout) {
    let mut session = Session::new();
    let original = checkerboard(0);
    let picture = Yuv444Frame::from_rgba(&original, WIDTH, HEIGHT).expect("picture");
    let (main, auxiliary) = picture.split(layout).expect("views");
    let regions = [region(0, WIDTH)];

    session.send(
        layout,
        Encoding::LUMA_AND_CHROMA,
        (&main, &regions),
        Some((&auxiliary, &regions)),
    );

    let difference = max_difference(&session.canvas, &original);
    assert!(difference <= TOLERANCE, "difference {difference}");
}

// oracle: spec MS-RDPEGFX §2.2.4.6
#[test]
fn a_chroma_update_completes_an_earlier_luma_update() {
    let layout = Avc444Layout::V2;
    let mut session = Session::new();
    let original = checkerboard(0);
    let picture = Yuv444Frame::from_rgba(&original, WIDTH, HEIGHT).expect("picture");
    let (main, auxiliary) = picture.split(layout).expect("views");
    let regions = [region(0, WIDTH)];

    session.send(layout, Encoding::LUMA, (&main, &regions), None);
    assert!(
        max_difference(&session.canvas, &original) > 50,
        "a luma subframe alone paints the 4:2:0 picture"
    );

    session.send(layout, Encoding::CHROMA, (&auxiliary, &regions), None);
    assert!(max_difference(&session.canvas, &original) <= TOLERANCE);
}

// oracle: spec MS-RDPEGFX §3.3.8.3.2
#[rstest]
#[case::avc444(Avc444Layout::V1)]
#[case::avc444v2(Avc444Layout::V2)]
fn an_update_only_paints_its_region_rectangles(#[case] layout: Avc444Layout) {
    let mut session = Session::new();
    let first = checkerboard(0);
    let (main, auxiliary) = Yuv444Frame::from_rgba(&first, WIDTH, HEIGHT)
        .expect("picture")
        .split(layout)
        .expect("views");
    let everything = [region(0, WIDTH)];
    session.send(
        layout,
        Encoding::LUMA_AND_CHROMA,
        (&main, &everything),
        Some((&auxiliary, &everything)),
    );

    let second = checkerboard(1);
    let (main, auxiliary) = Yuv444Frame::from_rgba(&second, WIDTH, HEIGHT)
        .expect("picture")
        .split(layout)
        .expect("views");
    let left = [region(0, 16)];
    session.send(
        layout,
        Encoding::LUMA_AND_CHROMA,
        (&main, &left),
        Some((&auxiliary, &left)),
    );

    let row_bytes = usize::from(WIDTH) * 4;
    for (row, (painted, (first, second))) in session
        .canvas
        .chunks_exact(row_bytes)
        .zip(first.chunks_exact(row_bytes).zip(second.chunks_exact(row_bytes)))
        .enumerate()
    {
        let (painted_left, painted_right) = painted.split_at(16 * 4);
        assert!(
            max_difference(painted_left, &second[..16 * 4]) <= TOLERANCE,
            "row {row} left of the region edge shows the second picture"
        );
        assert!(
            max_difference(painted_right, &first[16 * 4..]) <= TOLERANCE,
            "row {row} right of the region edge keeps the first picture"
        );
    }
}

// oracle: spec MS-RDPEGFX §2.2.3.3
#[test]
fn a_decoder_without_yuv_planes_advertises_no_avc444() {
    /// A decoder that only returns RGBA.
    struct RgbaDecoder;
    impl H264Decoder for RgbaDecoder {
        fn decode(&mut self, _data: &[u8]) -> DecoderResult<DecodedFrame> {
            Ok(DecodedFrame::new(vec![0; 16 * 16 * 4], 16, 16))
        }
    }
    struct ClientSink;
    impl ironrdp_egfx::client::GraphicsPipelineHandler for ClientSink {}

    let mut client = GraphicsPipelineClient::new(Box::new(ClientSink), Some(Box::new(RgbaDecoder)));
    let advertised = client.start(0).expect("client start");
    let advertised = bytes(&advertised[0]);
    let mut cursor = ironrdp_core::ReadCursor::new(&advertised);
    let ironrdp_egfx::pdu::GfxPdu::CapabilitiesAdvertise(caps) =
        ironrdp_core::decode_cursor(&mut cursor).expect("advertisement")
    else {
        panic!("expected a capabilities advertisement");
    };
    let versions: Vec<_> = caps.0.iter().map(|cap| cap.version).collect();
    assert_eq!(
        versions,
        [
            ironrdp_egfx::pdu::CapabilityVersion::V8_1,
            ironrdp_egfx::pdu::CapabilityVersion::V8
        ]
    );
}
