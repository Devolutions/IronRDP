use core::time::Duration;

use ironrdp_acceptor::RemoteAppCapabilities;
use ironrdp_core::{ReadCursor, decode_cursor};
use ironrdp_pdu::fast_path::{FastPathHeader, FastPathUpdatePdu, Fragmentation, UpdateCode};
use ironrdp_pdu::rdp::capability_sets::{
    CapabilitySet, CmdFlags, MultifragmentUpdate, Rail, RailSupportLevel, SurfaceCommands, WindowList,
    WindowSupportLevel,
};
use ironrdp_pdu::window::{
    CachedIconInfo, IconSource, WindowIcon, WindowOrder, WindowOrderBody, WindowState, WindowingOrder,
};
use ironrdp_server::{
    BitmapUpdate, ConnectionHandler, DesktopSize, DisplayContext, DisplayUpdate, PixelFormat, RdpServer,
    RdpServerDisplay, RdpServerDisplayUpdates, ServerEvent, ServerResult, TransportTls, WindowingOrders,
    WindowingOrdersStatus,
};
use ironrdp_tokio::TokioFramed;
use tokio::io::DuplexStream;
use tokio::sync::{mpsc, oneshot};

use super::activation::play_client_with_capabilities;

fn remote_app() -> RemoteAppCapabilities {
    RemoteAppCapabilities {
        rail: Rail {
            support_level: RailSupportLevel::SUPPORTED,
        },
        window_list: WindowList {
            support_level: WindowSupportLevel::SupportedEx,
            num_icon_caches: 0,
            num_icon_cache_entries: 0,
        },
    }
}

// oracle: spec MS-RDPERP §3.3.5.1.4: a RemoteApp server advertises Rail and Window List capabilities.
#[tokio::test]
async fn builder_offers_remote_app_capabilities() {
    connected(Some(remote_app()), None, |_, _, _, caps| async move {
        assert!(caps.contains(&CapabilitySet::Rail(remote_app().rail)));
        assert!(caps.contains(&CapabilitySet::WindowList(remote_app().window_list)));
    })
    .await;
}

// oracle: mutant enabling RemoteApp must not change Demand Active for default desktop servers.
#[tokio::test]
async fn desktop_server_does_not_offer_remote_app() {
    connected(None, None, |_, _, _, caps| async move {
        assert!(
            !caps
                .iter()
                .any(|cap| matches!(cap, CapabilitySet::Rail(_) | CapabilitySet::WindowList(_)))
        );
    })
    .await;
}

// oracle: spec MS-RDPBCGR §2.2.9.1.2.1.1 and MS-RDPERP §2.2.1.3.1.2.4: count precedes Deleted Window.
#[tokio::test]
async fn window_orders_include_count_and_order_bytes() {
    connected(Some(remote_app()), None, |mut client, events, _, _| async move {
        let (request, completion) = WindowingOrders::new(vec![deleted(7)]);
        events.send(ServerEvent::WindowingOrders(request)).unwrap();
        let (code, fragmentation, data) = read_update(&mut client).await;
        assert_eq!(code, UpdateCode::Orders);
        assert_eq!(fragmentation, Fragmentation::Single);
        assert_eq!(data, [1, 0, 0x2e, 11, 0, 0, 0, 0, 0x21, 7, 0, 0, 0]);
        assert_eq!(completion.await.unwrap().unwrap(), WindowingOrdersStatus::Written);
    })
    .await;
}

// oracle: spec MS-RDPBCGR §2.2.9.1.2.1: fragmentation uses FIRST, NEXT, LAST without losing payload.
#[tokio::test]
async fn fragmented_orders_complete_only_after_transport_accepts_all_fragments() {
    connected(Some(remote_app()), Some(65536), |mut client, events, _, _| async move {
        let (request, mut completion) = WindowingOrders::new((0..4000).map(deleted).collect());
        events.send(ServerEvent::WindowingOrders(request)).unwrap();
        let (code, first, mut data) = read_update(&mut client).await;
        assert_eq!(code, UpdateCode::Orders);
        assert_eq!(first, Fragmentation::First);
        assert!(matches!(
            completion.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        let (code, next, middle) = read_update(&mut client).await;
        assert_eq!(code, UpdateCode::Orders);
        assert_eq!(next, Fragmentation::Next);
        data.extend(middle);
        let (code, last, tail) = read_update(&mut client).await;
        assert_eq!(code, UpdateCode::Orders);
        assert_eq!(last, Fragmentation::Last);
        data.extend(tail);
        assert_eq!(completion.await.unwrap().unwrap(), WindowingOrdersStatus::Written);
        let mut cursor = ReadCursor::new(&data);
        assert_eq!(cursor.read_u16(), 4000);
        for id in 0..4000 {
            assert_eq!(decode_cursor::<WindowingOrder>(&mut cursor).unwrap(), deleted(id));
        }
        assert!(cursor.eof());
    })
    .await;
}

// oracle: spec MS-RDPBCGR §2.2.7.2.6: MaxRequestSize caps the full reassembled update, including count.
#[rstest::rstest]
#[case::absent(None)]
#[case::zero(Some(0))]
#[case::small(Some(18000))]
#[tokio::test]
async fn oversized_orders_are_rejected_before_writing(#[case] limit: Option<u32>) {
    connected(Some(remote_app()), limit, |mut client, events, display, _| async move {
        let (request, completion) = WindowingOrders::new(vec![deleted(7); 2000]);
        events.send(ServerEvent::WindowingOrders(request)).unwrap();
        assert!(completion.await.unwrap().is_err());
        display.send(DisplayUpdate::HidePointer).unwrap();
        assert_eq!(read_update(&mut client).await.0, UpdateCode::HiddenPointer);
    })
    .await;
}

// oracle: spec MS-RDPBCGR §2.2.7.2.6: the reassembly buffer is not needed for an unfragmented update.
#[tokio::test]
async fn single_order_can_be_written_with_zero_reassembly_limit() {
    connected(Some(remote_app()), Some(0), |mut client, events, _, _| async move {
        let (request, completion) = WindowingOrders::new(vec![deleted(7)]);
        events.send(ServerEvent::WindowingOrders(request)).unwrap();
        assert_eq!(read_update(&mut client).await.1, Fragmentation::Single);
        assert_eq!(completion.await.unwrap().unwrap(), WindowingOrdersStatus::Written);
    })
    .await;
}

// oracle: spec MS-RDPBCGR §2.2.7.2.6: a single-fragment update needs no reassembly buffer.
#[rstest::rstest]
#[case::zero(0)]
#[case::subfragment(1)]
#[tokio::test]
async fn single_order_ignores_subfragment_server_reassembly_limit(#[case] server_limit: u32) {
    connected_with_server_limit(
        Some(remote_app()),
        Some(remote_app()),
        Some(65536),
        Some(server_limit),
        |mut client, events, _, _| async move {
            let (request, completion) = WindowingOrders::new(vec![deleted(7)]);
            events.send(ServerEvent::WindowingOrders(request)).unwrap();
            assert_eq!(completion.await.unwrap().unwrap(), WindowingOrdersStatus::Written);
            assert_eq!(read_update(&mut client).await.1, Fragmentation::Single);
        },
    )
    .await;
}

// oracle: spec MS-RDPERP §3.3.5.1.5: window orders require a negotiated RAIL connection.
#[tokio::test]
async fn unnegotiated_orders_report_an_error_without_writing() {
    connected(None, None, |mut client, events, display, _| async move {
        let (request, completion) = WindowingOrders::new(vec![deleted(7)]);
        events.send(ServerEvent::WindowingOrders(request)).unwrap();
        assert!(completion.await.unwrap().is_err());
        display.send(DisplayUpdate::HidePointer).unwrap();
        assert_eq!(read_update(&mut client).await.0, UpdateCode::HiddenPointer);
    })
    .await;
}

// oracle: spec MS-RDPERP §3.3.5.1.3: RemoteApp negotiation requires the client's INFO_RAIL request.
#[tokio::test]
async fn remote_app_server_reports_none_and_rejects_orders_for_desktop_client() {
    connected_with_server_limit(
        Some(remote_app()),
        None,
        None,
        None,
        |mut client, events, display, _| async move {
            let (request, completion) = WindowingOrders::new(vec![deleted(7)]);
            events.send(ServerEvent::WindowingOrders(request)).unwrap();
            assert!(completion.await.unwrap().is_err());
            display.send(DisplayUpdate::HidePointer).unwrap();
            assert_eq!(read_update(&mut client).await.0, UpdateCode::HiddenPointer);
        },
    )
    .await;
}

// oracle: spec MS-RDPERP §2.2.1.1.2: ROOTPARENT requires TS_WINDOW_LEVEL_SUPPORTED_EX.
#[tokio::test]
async fn basic_window_support_rejects_extended_orders() {
    let mut capabilities = remote_app();
    capabilities.window_list.support_level = WindowSupportLevel::Supported;
    connected(Some(capabilities), None, |mut client, events, display, _| async move {
        let extended = WindowingOrder::Window(WindowOrder {
            window_id: 7,
            is_new: false,
            body: WindowOrderBody::State(Box::new(WindowState {
                root_parent_handle: Some(0),
                ..WindowState::default()
            })),
        });
        let (request, completion) = WindowingOrders::new(vec![extended]);
        events.send(ServerEvent::WindowingOrders(request)).unwrap();
        assert!(completion.await.unwrap().is_err());
        display.send(DisplayUpdate::HidePointer).unwrap();
        assert_eq!(read_update(&mut client).await.0, UpdateCode::HiddenPointer);
    })
    .await;
}

// oracle: spec MS-RDPERP §2.2.1.1.1: overlay icons require shell-integration support.
#[tokio::test]
async fn overlay_icon_without_shell_integration_is_rejected_before_writing() {
    connected(Some(remote_app()), None, |mut client, events, display, _| async move {
        let (request, completion) = WindowingOrders::new(vec![WindowingOrder::Window(WindowOrder {
            window_id: 7,
            is_new: false,
            body: WindowOrderBody::Icon(WindowIcon {
                big: false,
                overlay: true,
                icon: IconSource::Cached(CachedIconInfo {
                    cache_entry: 0,
                    cache_id: 0,
                }),
            }),
        })]);
        events.send(ServerEvent::WindowingOrders(request)).unwrap();
        assert!(completion.await.unwrap().is_err());
        display.send(DisplayUpdate::HidePointer).unwrap();
        assert_eq!(read_update(&mut client).await.0, UpdateCode::HiddenPointer);
    })
    .await;
}

// oracle: spec MS-RDPERP §2.2.1.1.1: overlay metadata requires shell-integration support.
#[rstest::rstest]
#[case::description(WindowState { overlay_description: Some("Status".into()), ..WindowState::default() })]
#[case::removed(WindowState { icon_overlay_null: true, ..WindowState::default() })]
#[tokio::test]
async fn overlay_metadata_without_shell_integration_is_rejected(#[case] state: WindowState) {
    connected(Some(remote_app()), None, |mut client, events, display, _| async move {
        let (request, completion) = WindowingOrders::new(vec![WindowingOrder::Window(WindowOrder {
            window_id: 7,
            is_new: false,
            body: WindowOrderBody::State(Box::new(state)),
        })]);
        events.send(ServerEvent::WindowingOrders(request)).unwrap();
        assert!(completion.await.unwrap().is_err());
        display.send(DisplayUpdate::HidePointer).unwrap();
        assert_eq!(read_update(&mut client).await.0, UpdateCode::HiddenPointer);
    })
    .await;
}

// oracle: spec MS-RDPERP §2.2.1.1.1: negotiated shell integration permits overlay updates.
#[tokio::test]
async fn negotiated_shell_integration_allows_overlay_updates() {
    let mut capabilities = remote_app();
    capabilities.rail.support_level |= RailSupportLevel::SHELL_INTEGRATION_SUPPORTED;
    connected(Some(capabilities), None, |mut client, events, _, _| async move {
        let order = WindowingOrder::Window(WindowOrder {
            window_id: 7,
            is_new: false,
            body: WindowOrderBody::State(Box::new(WindowState {
                icon_overlay_null: true,
                ..WindowState::default()
            })),
        });
        let (request, completion) = WindowingOrders::new(vec![order.clone()]);
        events.send(ServerEvent::WindowingOrders(request)).unwrap();
        let (_, _, data) = read_update(&mut client).await;
        let mut cursor = ReadCursor::new(&data);
        assert_eq!(cursor.read_u16(), 1);
        assert_eq!(decode_cursor::<WindowingOrder>(&mut cursor).unwrap(), order);
        assert_eq!(completion.await.unwrap().unwrap(), WindowingOrdersStatus::Written);
    })
    .await;
}

// oracle: mutant overflowing numberOrders would silently lose orders after the first 65535.
#[tokio::test]
async fn overflowing_order_count_is_rejected_before_writing() {
    connected(
        Some(remote_app()),
        Some(2_000_000),
        |mut client, events, display, _| async move {
            let (request, completion) = WindowingOrders::new(vec![deleted(7); 65536]);
            events.send(ServerEvent::WindowingOrders(request)).unwrap();
            assert!(completion.await.unwrap().is_err());
            display.send(DisplayUpdate::HidePointer).unwrap();
            assert_eq!(read_update(&mut client).await.0, UpdateCode::HiddenPointer);
        },
    )
    .await;
}

// oracle: mutant acknowledging a discarded event loses orders during connection cleanup.
#[tokio::test]
async fn dropping_an_unstarted_batch_reports_safe_retry() {
    let (request, completion) = WindowingOrders::new(vec![deleted(7)]);
    drop(request);
    assert_eq!(completion.await.unwrap().unwrap(), WindowingOrdersStatus::NotStarted);
}

// oracle: mutant retrying a partially transmitted update duplicates orders on the client.
#[tokio::test]
async fn transport_failure_after_first_fragment_reports_an_error() {
    connected(Some(remote_app()), Some(65536), |mut client, events, _, _| async move {
        let (request, completion) = WindowingOrders::new(vec![deleted(7); 4000]);
        events.send(ServerEvent::WindowingOrders(request)).unwrap();
        assert_eq!(read_update(&mut client).await.1, Fragmentation::First);
        drop(client);
        assert!(completion.await.unwrap().is_err());
    })
    .await;
}

// oracle: spec MS-RDPBCGR §2.2.9.1.2.1: orders cannot interrupt a fragmented graphics update.
#[tokio::test]
async fn window_orders_wait_for_all_bitmap_fragments() {
    connected(
        Some(remote_app()),
        Some(65536),
        |mut client, events, display, _| async move {
            display
                .send(DisplayUpdate::Bitmap(BitmapUpdate {
                    x: 0,
                    y: 0,
                    width: 128.try_into().unwrap(),
                    height: 64.try_into().unwrap(),
                    format: PixelFormat::ARgb32,
                    data: vec![0x55; 128 * 64 * 4].into(),
                    stride: 512.try_into().unwrap(),
                }))
                .unwrap();
            let (code, first, _) = read_update(&mut client).await;
            assert_eq!(code, UpdateCode::SurfaceCommands);
            assert_eq!(first, Fragmentation::First);
            let (request, completion) = WindowingOrders::new(vec![deleted(7)]);
            events.send(ServerEvent::WindowingOrders(request)).unwrap();
            loop {
                let (code, fragment, _) = read_update(&mut client).await;
                assert_eq!(code, UpdateCode::SurfaceCommands);
                if fragment == Fragmentation::Last {
                    break;
                }
                assert_eq!(fragment, Fragmentation::Next);
            }
            assert_eq!(read_update(&mut client).await.0, UpdateCode::Orders);
            assert_eq!(completion.await.unwrap().unwrap(), WindowingOrdersStatus::Written);
        },
    )
    .await;
}

// oracle: mutant resize must not cancel a partially written batch or insert Deactivate All between its fragments.
#[tokio::test]
async fn resize_waits_for_the_complete_window_update() {
    connected(
        Some(remote_app()),
        Some(65536),
        |mut client, events, display, _| async move {
            let (request, completion) = WindowingOrders::new(vec![deleted(7); 4000]);
            events.send(ServerEvent::WindowingOrders(request)).unwrap();
            assert_eq!(read_update(&mut client).await.1, Fragmentation::First);
            display
                .send(DisplayUpdate::Resize(DesktopSize {
                    width: 800,
                    height: 600,
                }))
                .unwrap();
            loop {
                let (code, fragment, _) = read_update(&mut client).await;
                assert_eq!(code, UpdateCode::Orders);
                if fragment == Fragmentation::Last {
                    break;
                }
                assert_eq!(fragment, Fragmentation::Next);
            }
            assert_eq!(completion.await.unwrap().unwrap(), WindowingOrdersStatus::Written);
            assert_eq!(client.read_pdu().await.unwrap().0, ironrdp_pdu::Action::X224);
        },
    )
    .await;
}

fn deleted(window_id: u32) -> WindowingOrder {
    WindowingOrder::Window(WindowOrder {
        window_id,
        is_new: false,
        body: WindowOrderBody::Deleted,
    })
}

async fn read_update(client: &mut TokioFramed<DuplexStream>) -> (UpdateCode, Fragmentation, Vec<u8>) {
    let (_, bytes) = client.read_pdu().await.unwrap();
    assert!(bytes.len() <= 16383);
    let mut cursor = ReadCursor::new(&bytes);
    let _header = decode_cursor::<FastPathHeader>(&mut cursor).unwrap();
    let update = decode_cursor::<FastPathUpdatePdu<'_>>(&mut cursor).unwrap();
    (update.update_code, update.fragmentation, update.data.to_vec())
}

async fn connected<F, Fut>(capabilities: Option<RemoteAppCapabilities>, limit: Option<u32>, run: F)
where
    F: FnOnce(
        TokioFramed<DuplexStream>,
        mpsc::UnboundedSender<ServerEvent>,
        mpsc::UnboundedSender<DisplayUpdate>,
        Vec<CapabilitySet>,
    ) -> Fut,
    Fut: Future<Output = ()>,
{
    connected_with_server_limit(capabilities.clone(), capabilities, limit, None, run).await;
}

async fn connected_with_server_limit<F, Fut>(
    server_capabilities: Option<RemoteAppCapabilities>,
    capabilities: Option<RemoteAppCapabilities>,
    limit: Option<u32>,
    server_limit: Option<u32>,
    run: F,
) where
    F: FnOnce(
        TokioFramed<DuplexStream>,
        mpsc::UnboundedSender<ServerEvent>,
        mpsc::UnboundedSender<DisplayUpdate>,
        Vec<CapabilitySet>,
    ) -> Fut,
    Fut: Future<Output = ()>,
{
    let (display_tx, display_rx) = mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = oneshot::channel();
    let (capabilities_tx, capabilities_rx) = oneshot::channel();
    let expected_capabilities = capabilities.clone();
    let mut builder = RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_display_handler(Display {
            updates: Some(display_rx),
            ready: Some(ready_tx),
        })
        .with_remote_app(server_capabilities)
        .with_connection_handler(Some(Box::new(CapabilitiesHandler(Some(capabilities_tx)))));
    if let Some(server_limit) = server_limit {
        builder = builder.with_max_request_size(server_limit);
    }
    let mut server = builder.build();
    let events = server.event_sender().clone();
    let (client, transport) = tokio::io::duplex(1024);
    tokio::time::timeout(Duration::from_secs(5), async {
        let client = async {
            let mut client = TokioFramed::new(client);
            let confirmed = capabilities.map(|caps| {
                let mut caps = vec![
                    CapabilitySet::Rail(caps.rail),
                    CapabilitySet::WindowList(caps.window_list),
                ];
                caps.push(CapabilitySet::SurfaceCommands(SurfaceCommands {
                    flags: CmdFlags::SET_SURFACE_BITS,
                }));
                if let Some(max_request_size) = limit {
                    caps.push(CapabilitySet::MultiFragmentUpdate(MultifragmentUpdate {
                        max_request_size,
                    }));
                }
                caps
            });
            let caps = play_client_with_capabilities(&mut client, true, &[], confirmed).await;
            ready_rx.await.unwrap();
            assert_eq!(capabilities_rx.await.unwrap(), expected_capabilities);
            let display_keepalive = display_tx.clone();
            run(client, events, display_tx, caps).await;
            drop(display_keepalive);
        };
        let (_, ()) = tokio::join!(server.run_connection_with(transport, TransportTls::AlreadyDone), client);
    })
    .await
    .unwrap();
}

struct CapabilitiesHandler(Option<oneshot::Sender<Option<RemoteAppCapabilities>>>);

impl ConnectionHandler for CapabilitiesHandler {
    fn on_remote_app_capabilities(&mut self, capabilities: Option<&RemoteAppCapabilities>) -> ServerResult<()> {
        self.0.take().unwrap().send(capabilities.cloned()).unwrap();
        Ok(())
    }
}

struct Display {
    updates: Option<mpsc::UnboundedReceiver<DisplayUpdate>>,
    ready: Option<oneshot::Sender<()>>,
}

#[async_trait::async_trait]
impl RdpServerDisplay for Display {
    async fn size(&mut self) -> DesktopSize {
        DesktopSize {
            width: 1280,
            height: 720,
        }
    }

    async fn updates(&mut self, _: DisplayContext) -> ServerResult<Box<dyn RdpServerDisplayUpdates>> {
        self.ready.take().unwrap().send(()).unwrap();
        Ok(Box::new(Updates(self.updates.take().unwrap())))
    }
}

struct Updates(mpsc::UnboundedReceiver<DisplayUpdate>);

#[async_trait::async_trait]
impl RdpServerDisplayUpdates for Updates {
    async fn next_update(&mut self) -> ServerResult<Option<DisplayUpdate>> {
        Ok(self.0.recv().await)
    }
}
