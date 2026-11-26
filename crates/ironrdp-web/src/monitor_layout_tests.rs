#![cfg(target_arch = "wasm32")]

use futures_util::future::{Either, select};
use iron_remote_desktop::{IronError as _, Session as _};
use ironrdp::connector::connection_activation::ConnectionActivationFactory;
use ironrdp::pdu::gcc::{Monitor, MonitorFlags, RdpVersion};
use ironrdp::pdu::mcs::{DisconnectProviderUltimatum, DisconnectReason, McsMessage, SendDataIndication};
use ironrdp::pdu::rdp::capability_sets::{DemandActive, InputFlags, ServerDemandActive};
use ironrdp::pdu::rdp::client_info::CompressionType;
use ironrdp::pdu::rdp::finalization_messages::{FontPdu, MonitorLayoutPdu};
use ironrdp::pdu::rdp::headers::{
    CompressionFlags, ServerDeactivateAll, ShareControlHeader, ShareControlPdu, ShareDataHeader, ShareDataPdu,
    StreamPriority,
};
use ironrdp::pdu::x224::X224;
use wasm_bindgen_test::wasm_bindgen_test;

use super::*;

pub(super) const USER_CHANNEL_ID: u16 = 1002;
pub(super) const IO_CHANNEL_ID: u16 = 1003;
pub(super) const SHARE_ID: u32 = 0x0001_0000;

struct RestoreCanvasContext(js_sys::Function);

impl Drop for RestoreCanvasContext {
    fn drop(&mut self) {
        self.0.call0(&JsValue::NULL).expect("restore canvas context");
    }
}

fn monitor(left: i32, top: i32, right: i32, bottom: i32, flags: u32) -> Monitor {
    Monitor {
        left,
        top,
        right,
        bottom,
        flags: MonitorFlags::from_bits_retain(flags),
    }
}

/// A frame the server sends on `channel_id`, as an MCS Send Data Indication.
pub(super) fn server_channel_frame(channel_id: u16, user_data: Vec<u8>) -> Vec<u8> {
    let indication = SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id,
        user_data: user_data.into(),
    };
    ironrdp_core::encode_vec(&X224(indication)).expect("encode server frame")
}

pub(super) fn server_frame(pdu: ShareControlPdu) -> Vec<u8> {
    let control = ShareControlHeader {
        share_id: SHARE_ID,
        pdu_source: USER_CHANNEL_ID,
        share_control_pdu: pdu,
    };
    server_channel_frame(
        IO_CHANNEL_ID,
        ironrdp_core::encode_vec(&control).expect("encode share control"),
    )
}

pub(super) fn server_data(pdu: ShareDataPdu) -> Vec<u8> {
    server_frame(ShareControlPdu::Data(ShareDataHeader {
        share_data_pdu: pdu,
        stream_priority: StreamPriority::Medium,
        compression_flags: CompressionFlags::empty(),
        compression_type: CompressionType::K8,
    }))
}

pub(super) fn disconnect_frame() -> Vec<u8> {
    ironrdp_core::encode_vec(&X224(McsMessage::DisconnectProviderUltimatum(
        DisconnectProviderUltimatum::from_reason(DisconnectReason::ProviderInitiated),
    )))
    .expect("encode disconnect")
}

/// The browser objects a session needs: a WebSocket whose `deliver` plays a server frame to the session and which
/// counts the bytes sent to it, and a canvas whose 2D context takes image data and draws nothing.
pub(super) struct BrowserFixture {
    socket: JsValue,
    emit: js_sys::Function,
    reader: RefCell<Option<ReadHalf<WebSocket>>>,
    writer: RefCell<Option<WriteHalf<WebSocket>>>,
    pub(super) canvas: HtmlCanvasElement,
    _restore: RestoreCanvasContext,
}

impl BrowserFixture {
    pub(super) fn new() -> Self {
        let fixture = js_sys::Function::new_no_args(
            r#"
            const previousContext = globalThis.CanvasRenderingContext2D;
            const previousImageData = globalThis.ImageData;
            globalThis.CanvasRenderingContext2D = class { putImageData() {} };
            globalThis.ImageData = class {
                constructor(data, width, height) { this.data = data; this.width = width; this.height = height; }
            };
            const socket = {
                readyState: 1,
                listeners: new Map(),
                sent: 0,
                addEventListener(type, listener) { this.listeners.set(type, listener); },
                removeEventListener() {},
                close() {},
                dispatchEvent() {},
                send(data) { this.sent += data.size ?? data.byteLength; },
                emit(data) { this.listeners.get("message")({ data }); },
            };
            return {
                socket,
                canvas: { getContext() { return new CanvasRenderingContext2D(); } },
                restore() {
                    if (previousContext === undefined) delete globalThis.CanvasRenderingContext2D;
                    else globalThis.CanvasRenderingContext2D = previousContext;
                    if (previousImageData === undefined) delete globalThis.ImageData;
                    else globalThis.ImageData = previousImageData;
                },
            };
            "#,
        )
        .call0(&JsValue::NULL)
        .expect("create browser fixture");
        let get = |name| js_sys::Reflect::get(&fixture, &JsValue::from_str(name)).expect("read browser fixture");
        let restore = RestoreCanvasContext(get("restore").unchecked_into());
        let socket = get("socket");
        let canvas: HtmlCanvasElement = get("canvas").unchecked_into();
        let websocket = WebSocket::try_from(socket.clone().unchecked_into::<web_sys::WebSocket>())
            .expect("create WebSocket adapter");
        let (reader, writer) = futures_util::AsyncReadExt::split(websocket);
        let emit: js_sys::Function = js_sys::Reflect::get(&socket, &JsValue::from_str("emit"))
            .expect("read message emitter")
            .unchecked_into();
        Self {
            socket,
            emit,
            reader: RefCell::new(Some(reader)),
            writer: RefCell::new(Some(writer)),
            canvas,
            _restore: restore,
        }
    }

    pub(super) fn deliver(&self, frame: &[u8]) {
        self.emit
            .call1(&self.socket, &js_sys::Uint8Array::from(frame).buffer())
            .expect("deliver server frame");
    }

    pub(super) fn writer(&self) -> WriteHalf<WebSocket> {
        self.writer.borrow_mut().take().expect("the writer is taken once")
    }

    /// Bytes the socket's `send` was given.
    pub(super) fn sent(&self) -> f64 {
        js_sys::Reflect::get(&self.socket, &JsValue::from_str("sent"))
            .expect("read sent bytes")
            .as_f64()
            .expect("sent bytes are a number")
    }

    /// Makes every `send` throw, as a closed socket's does.
    pub(super) fn refuse_sends(&self) {
        let throwing = js_sys::Function::new_no_args("throw new Error('socket closed')");
        js_sys::Reflect::set(&self.socket, &JsValue::from_str("send"), &throwing).expect("replace send");
    }
}

pub(super) fn test_connection_result(
    desktop_size: DesktopSize,
    monitor_layout: Option<MonitorLayoutPdu>,
    message_channel_id: Option<u16>,
    network_characteristics: Option<connector::NetworkCharacteristics>,
) -> connector::ConnectionResult {
    let config = build_config(
        String::new(),
        String::new(),
        None,
        "ironrdp-web-test".to_owned(),
        desktop_size,
        false,
        SecurityConfig {
            enable_credssp: true,
            enable_standard_rdp_security: false,
        },
    );
    connector::ConnectionResult {
        io_channel_id: IO_CHANNEL_ID,
        user_channel_id: USER_CHANNEL_ID,
        message_channel_id,
        share_id: SHARE_ID,
        static_channels: ironrdp_svc::StaticChannelSet::new(),
        desktop_size: connector::DesktopSize {
            width: desktop_size.width,
            height: desktop_size.height,
        },
        input_flags: InputFlags::empty(),
        enable_server_pointer: false,
        pointer_software_rendering: false,
        refresh_rect_support: false,
        suppress_output_support: false,
        window_support_level: None,
        monitor_layout,
        network_characteristics,
        activation_factory: ConnectionActivationFactory::new(config, IO_CHANNEL_ID, USER_CHANNEL_ID),
        compression_type: None,
        server_rdp_version: RdpVersion::V5_PLUS,
    }
}

/// A session reading the fixture's socket, with the receiver of what it writes, which must outlive the run.
pub(super) fn test_session(
    fixture: &BrowserFixture,
    monitor_layout_callback: Option<js_sys::Function>,
    connection_result: connector::ConnectionResult,
) -> (Session, mpsc::UnboundedReceiver<Vec<u8>>) {
    let (writer_tx, writer_rx) = mpsc::unbounded();
    let (input_events_tx, input_events_rx) = mpsc::unbounded();
    let session = Session {
        desktop_size: connection_result.desktop_size,
        input_database: RefCell::new(ironrdp::input::Database::new()),
        writer_tx,
        input_events_tx,
        render_canvas: fixture.canvas.clone(),
        set_cursor_style_callback: js_sys::Function::new_no_args(""),
        set_cursor_style_callback_context: JsValue::NULL,
        audio_playback_callback: None,
        camera_callback: None,
        camera: None,
        monitor_layout_callback,
        stats: Rc::new(crate::stats::SessionStats::default()),
        input_events_rx: RefCell::new(Some(input_events_rx)),
        rdp_reader: RefCell::new(fixture.reader.borrow_mut().take()),
        clipboard: RefCell::new(Some(None)),
        printer: RefCell::new(Some(None)),
        connection_result: RefCell::new(Some(connection_result)),
    };
    (session, writer_rx)
}

pub(super) async fn run_to_end(session: &Session) {
    match select(
        Box::pin(session.run()),
        Box::pin(gloo_timers::future::TimeoutFuture::new(5_000)),
    )
    .await
    {
        Either::Left((Ok(_), _)) => {}
        Either::Left((Err(error), _)) => panic!("session failed: {}", error.backtrace()),
        Either::Right(_) => panic!("session timed out"),
    }
}

#[wasm_bindgen_test]
async fn callback_receives_initial_changed_and_reactivated_layouts() {
    // oracle: reference crates/ironrdp-client/src/rdp.rs forwards layouts at these three session events.
    // oracle: spec MS-RDPBCGR §2.2.1.3.6.1 defines the coordinate order and flags.
    // oracle: regression #217 preserves unknown flag bits.
    let fixture = BrowserFixture::new();

    let initial = vec![monitor(0, 0, 799, 599, 1), monitor(-800, 0, -1, 599, 0)];
    let changed = vec![monitor(0, 0, 1023, 767, 1)];
    let reactivated = vec![monitor(0, 0, 639, 479, 1), monitor(640, -100, 1279, 379, 0x8000_0000)];
    let frames = [
        server_data(ShareDataPdu::MonitorLayout(MonitorLayoutPdu { monitors: changed })),
        server_frame(ShareControlPdu::ServerDeactivateAll(ServerDeactivateAll)),
        server_frame(ShareControlPdu::ServerDemandActive(ServerDemandActive {
            pdu: DemandActive {
                source_descriptor: "test".to_owned(),
                capability_sets: Vec::new(),
            },
        })),
        server_data(ShareDataPdu::MonitorLayout(MonitorLayoutPdu { monitors: reactivated })),
        server_data(ShareDataPdu::FontMap(FontPdu::default())),
        disconnect_frame(),
    ];
    for frame in &frames {
        fixture.deliver(frame);
    }

    let layouts = js_sys::Array::new();
    let callback: js_sys::Function =
        js_sys::Function::new_with_args("layouts", "return layout => layouts.push(layout)")
            .call1(&JsValue::NULL, &layouts)
            .expect("create layout callback")
            .unchecked_into();
    let desktop_size = DesktopSize {
        width: 800,
        height: 600,
    };
    let (session, _writer_rx) = test_session(
        &fixture,
        Some(callback),
        test_connection_result(desktop_size, Some(MonitorLayoutPdu { monitors: initial }), None, None),
    );

    run_to_end(&session).await;

    let observed: Vec<Vec<f64>> = layouts
        .iter()
        .map(|layout| {
            js_sys::Array::from(&layout)
                .iter()
                .map(|value| value.as_f64().expect("numeric monitor field"))
                .collect()
        })
        .collect();
    assert_eq!(
        observed,
        [
            vec![0.0, 0.0, 799.0, 599.0, 1.0, -800.0, 0.0, -1.0, 599.0, 0.0],
            vec![0.0, 0.0, 1023.0, 767.0, 1.0],
            vec![0.0, 0.0, 639.0, 479.0, 1.0, 640.0, -100.0, 1279.0, 379.0, 2147483648.0],
        ]
    );
}
