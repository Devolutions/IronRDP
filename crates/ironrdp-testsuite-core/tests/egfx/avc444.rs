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
    const PAIRS: [[[u8; 3]; 2]; 4] = [
        [[220, 30, 40], [20, 70, 230]],
        [[30, 200, 40], [200, 40, 220]],
        [[240, 220, 30], [20, 150, 170]],
        [[250, 120, 20], [40, 90, 250]],
    ];
    let mut rgba = Vec::with_capacity(usize::from(WIDTH) * usize::from(HEIGHT) * 4);
    for y in 0..usize::from(HEIGHT) {
        for x in 0..usize::from(WIDTH) {
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
    Avc420Region {
        left,
        top: 0,
        right,
        bottom: HEIGHT,
        quantization_parameter: 22,
        quality: 100,
    }
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

#[test]
fn avc444_without_a_yuv_decoder_is_left_to_the_handler() {
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
