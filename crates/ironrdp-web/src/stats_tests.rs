#![cfg(target_arch = "wasm32")]

use futures_util::future::{Either, select};
use iron_remote_desktop::{IronError as _, Session as _};
use ironrdp::connector::NetworkCharacteristics;
use ironrdp::pdu::bitmap::{BitmapData, BitmapUpdateData, Compression};
use ironrdp::pdu::fast_path::{EncryptionFlags, FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp::pdu::geometry::InclusiveRectangle;
use ironrdp::pdu::rdp::autodetect::{AutoDetectReqPdu, AutoDetectRequest, NETCHAR_RESULT_RTT};
use ironrdp::pdu::rdp::capability_sets::{DemandActive, ServerDemandActive};
use ironrdp::pdu::rdp::finalization_messages::FontPdu;
use ironrdp::pdu::rdp::headers::{ServerDeactivateAll, ShareControlPdu, ShareDataPdu};
use wasm_bindgen_test::wasm_bindgen_test;

use super::monitor_layout_tests::{
    BrowserFixture, disconnect_frame, run_to_end, server_channel_frame, server_data, server_frame,
    test_connection_result, test_session,
};
use super::*;

const MESSAGE_CHANNEL_ID: u16 = 1005;

fn auto_detect_frame(request: AutoDetectRequest) -> Vec<u8> {
    let pdu = AutoDetectReqPdu::new(request);
    server_channel_frame(
        MESSAGE_CHANNEL_ID,
        ironrdp_core::encode_vec(&pdu).expect("encode auto-detect request"),
    )
}

/// A fast-path bitmap update of one opaque 4x4 rectangle at the origin.
fn bitmap_frame() -> Vec<u8> {
    let bitmap = ironrdp_core::encode_vec(&BitmapUpdateData {
        rectangles: vec![BitmapData {
            rectangle: InclusiveRectangle {
                left: 0,
                top: 0,
                right: 3,
                bottom: 3,
            },
            width: 4,
            height: 4,
            bits_per_pixel: 32,
            compression_flags: Compression::empty(),
            compressed_data_header: None,
            bitmap_data: &[0x11, 0x22, 0x33, 0xFF].repeat(16),
        }],
    })
    .expect("encode bitmap update");
    let update = ironrdp_core::encode_vec(&FastPathUpdatePdu {
        fragmentation: Fragmentation::Single,
        update_code: UpdateCode::Bitmap,
        compression_flags: None,
        compression_type: None,
        data: &bitmap,
    })
    .expect("encode fast-path update");
    let mut frame =
        ironrdp_core::encode_vec(&FastPathHeader::new(EncryptionFlags::empty(), update.len())).expect("encode header");
    frame.extend_from_slice(&update);
    frame
}

fn field(object: &JsValue, name: &str) -> JsValue {
    js_sys::Reflect::get(object, &JsValue::from_str(name)).expect("read field")
}

#[wasm_bindgen_test]
#[expect(clippy::float_cmp, reason = "exact byte counts read from JavaScript")]
async fn the_stats_extension_reports_what_the_session_read_drew_sent_and_was_told() {
    // oracle: spec MS-RDPBCGR §2.2.14.1.5 (baseRTT and bandwidth are optional in each result)
    // oracle: mutant a reader that leaves some frames uncounted, a reactivation's frames among them; a turn that draws
    // but is not counted; a connection result not kept; a result that drops the bandwidth kept; network fields under
    // the wrong name; a sent frame counted twice or not at all
    let fixture = BrowserFixture::new();
    let frames = [
        bitmap_frame(),
        auto_detect_frame(AutoDetectRequest::rtt_continuous(1)),
        auto_detect_frame(AutoDetectRequest::NetworkCharacteristicsResult {
            sequence_number: 2,
            request_type: NETCHAR_RESULT_RTT,
            base_rtt_ms: Some(8),
            bandwidth_kbps: None,
            average_rtt_ms: 11,
        }),
        server_frame(ShareControlPdu::ServerDeactivateAll(ServerDeactivateAll)),
        server_frame(ShareControlPdu::ServerDemandActive(ServerDemandActive {
            pdu: DemandActive {
                source_descriptor: "test".to_owned(),
                capability_sets: Vec::new(),
            },
        })),
        server_data(ShareDataPdu::FontMap(FontPdu::default())),
        disconnect_frame(),
    ];
    let expected_bytes = f64::from(u32::try_from(frames.iter().map(Vec::len).sum::<usize>()).expect("fits"));
    for frame in &frames {
        fixture.deliver(frame);
    }
    let desktop_size = DesktopSize {
        width: 800,
        height: 600,
    };
    let from_the_connection = NetworkCharacteristics {
        base_rtt_ms: Some(10),
        bandwidth_kbps: Some(5_000),
        average_rtt_ms: 12,
    };
    let (session, writer_rx) = test_session(
        &fixture,
        None,
        test_connection_result(desktop_size, None, Some(MESSAGE_CHANNEL_ID), Some(from_the_connection)),
    );
    let stats = Rc::clone(&session.stats);
    let writer = Box::pin(writer_task(Rc::clone(&stats), writer_rx, fixture.writer(), None));

    let Either::Left(((), writer)) = select(Box::pin(run_to_end(&session)), writer).await else {
        panic!("the writer ended before the session");
    };
    let object = session
        .invoke_extension(Extension::create("stats".to_owned(), JsValue::NULL))
        .unwrap_or_else(|error| panic!("stats extension: {}", error.backtrace()));
    drop(session);
    writer.await;

    assert_eq!(field(&object, "bytesReceived").as_f64(), Some(expected_bytes));
    assert_eq!(field(&object, "framesDrawn").as_f64(), Some(1.0));
    let network = field(&object, "network");
    assert_eq!(field(&network, "baseRttMs").as_f64(), Some(8.0));
    assert_eq!(field(&network, "averageRttMs").as_f64(), Some(11.0));
    assert_eq!(field(&network, "bandwidthKbps").as_f64(), Some(5_000.0));
    let sent = fixture.sent();
    assert!(0.0 < sent, "the RTT request and the reactivation are answered");
    assert_eq!(
        f64::from(u32::try_from(stats.snapshot().bytes_sent).expect("fits")),
        sent
    );
}

#[wasm_bindgen_test]
#[expect(clippy::float_cmp, reason = "an exact byte count read from JavaScript")]
async fn the_writer_counts_a_frame_once_the_socket_has_taken_it() {
    // oracle: mutant a writer that counts nothing, or counts a frame twice
    let fixture = BrowserFixture::new();
    let stats = Rc::new(crate::stats::SessionStats::default());
    let (tx, rx) = mpsc::unbounded();
    tx.unbounded_send(vec![0; 5]).expect("queue a frame");
    tx.unbounded_send(vec![0; 7]).expect("queue a frame");
    drop(tx);

    writer_task(Rc::clone(&stats), rx, fixture.writer(), None).await;

    assert_eq!(fixture.sent(), 12.0);
    assert_eq!(stats.snapshot().bytes_sent, 12);
}

#[wasm_bindgen_test]
#[expect(clippy::float_cmp, reason = "an exact byte count read from JavaScript")]
async fn the_writer_counts_nothing_the_socket_refused() {
    // oracle: mutant a writer that counts a frame before the socket takes it
    let fixture = BrowserFixture::new();
    fixture.refuse_sends();
    let stats = Rc::new(crate::stats::SessionStats::default());
    let (tx, rx) = mpsc::unbounded();
    tx.unbounded_send(vec![0; 5]).expect("queue a frame");
    drop(tx);

    writer_task(Rc::clone(&stats), rx, fixture.writer(), None).await;

    assert_eq!(fixture.sent(), 0.0);
    assert_eq!(stats.snapshot().bytes_sent, 0);
}
