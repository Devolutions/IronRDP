//! Windowing Alternate Secondary Drawing Orders ([MS-RDPERP] 2.2.1.3).
//!
//! The wire vectors are laid out field by field from the specification; FreeRDP's
//! `libfreerdp/core/window.c` confirms the `BitsMask`, `ColorTable`, `BitsColor`
//! order of TS_ICON_INFO.
//!
//! [MS-RDPERP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdperp

use ironrdp_core::{ReadCursor, WriteCursor, decode, encode_vec};
use ironrdp_pdu::geometry::ExclusiveRectangle;
use ironrdp_pdu::window::{
    CachedIconInfo, DesktopOrder, IconInfo, IconSource, MonitoredDesktop, NotifyIconInfoTip, NotifyIconOrder,
    NotifyIconState, ResizeMargins, ShowState, WindowIcon, WindowOrder, WindowOrderBody, WindowPoint, WindowSize,
    WindowState, WindowStyle, WindowingOrder, encode_slow_path_windowing_orders_update,
    slow_path_windowing_orders_update_size, try_decode_fast_path_windowing_orders,
    try_decode_slow_path_windowing_orders,
};
use rstest::rstest;

fn assert_round_trip(bytes: &[u8], expected: &WindowingOrder) {
    let decoded = decode::<WindowingOrder>(bytes).unwrap();
    assert_eq!(&decoded, expected);
    assert_eq!(encode_vec(expected).unwrap(), bytes);
}

#[rustfmt::skip]
const NEW_WINDOW: [u8; 72] = [
    0x2e, 0x48, 0x00, // controlFlags, orderSize = 72
    0x1e, 0x1f, 0x00, 0x11, // fieldsPresent: WINDOW | STATE_NEW | OWNER | TITLE | STYLE | SHOW | WNDRECTS | VISIBILITY | WNDSIZE | WNDOFFSET | VISOFFSET
    0x42, 0x00, 0x00, 0x00, // WindowId
    0x00, 0x00, 0x00, 0x00, // OwnerWindowId
    0x00, 0x00, 0xcf, 0x10, // Style: WS_OVERLAPPEDWINDOW | WS_VISIBLE
    0x00, 0x01, 0x00, 0x00, // ExtendedStyle: WS_EX_WINDOWEDGE
    0x05, // ShowState: SW_SHOWNORMAL
    0x02, 0x00, 0x41, 0x00, // TitleInfo: "A"
    0xf8, 0xff, 0xff, 0xff, 0x64, 0x00, 0x00, 0x00, // WindowOffsetX = -8, WindowOffsetY = 100
    0x80, 0x02, 0x00, 0x00, 0xe0, 0x01, 0x00, 0x00, // WindowWidth = 640, WindowHeight = 480
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x02, 0xe0, 0x01, // NumWindowRects, WindowRects
    0xf8, 0xff, 0xff, 0xff, 0x64, 0x00, 0x00, 0x00, // VisibleOffsetX = -8, VisibleOffsetY = 100
    0x01, 0x00, 0x08, 0x00, 0x00, 0x00, 0x78, 0x02, 0xd8, 0x01, // NumVisibilityRects, VisibilityRects
];

fn new_window() -> WindowingOrder {
    WindowingOrder::Window(WindowOrder {
        window_id: 0x42,
        is_new: true,
        body: WindowOrderBody::State(Box::new(WindowState {
            owner_window_id: Some(0),
            style: Some(WindowStyle {
                style: 0x10cf_0000,
                extended_style: 0x0000_0100,
            }),
            show_state: Some(ShowState::Show),
            title: Some("A".to_owned()),
            window_offset: Some(WindowPoint { x: -8, y: 100 }),
            window_size: Some(WindowSize {
                width: 640,
                height: 480,
            }),
            window_rects: Some(vec![ExclusiveRectangle {
                left: 0,
                top: 0,
                right: 640,
                bottom: 480,
            }]),
            visible_offset: Some(WindowPoint { x: -8, y: 100 }),
            visibility_rects: Some(vec![ExclusiveRectangle {
                left: 8,
                top: 0,
                right: 632,
                bottom: 472,
            }]),
            ..WindowState::default()
        })),
    })
}

// oracle: spec MS-RDPERP §2.2.1.3.1.2.1
#[test]
fn new_window_state_order() {
    let expected = new_window();

    assert_round_trip(&NEW_WINDOW, &expected);
    assert!(!expected.requires_extended_support());
}

// oracle: spec MS-RDPERP §2.2.1.3.1.2.2 and §2.2.1.2.3
#[test]
fn window_icon_order_keeps_mask_color_table_and_bits_apart() {
    #[rustfmt::skip]
    let bytes = [
        0x2e, 0x29, 0x00, // controlFlags, orderSize = 41
        0x00, 0x20, 0x00, 0x41, // fieldsPresent: WINDOW | ICON | ICON_BIG
        0x42, 0x00, 0x00, 0x00, // WindowId
        0x03, 0x00, // CacheEntry
        0x01, // CacheId
        0x01, // Bpp
        0x02, 0x00, 0x02, 0x00, // Width, Height
        0x08, 0x00, // CbColorTable
        0x04, 0x00, // CbBitsMask
        0x04, 0x00, // CbBitsColor
        0x80, 0x00, 0x40, 0x00, // BitsMask
        0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0x00, // ColorTable
        0xc0, 0x00, 0x00, 0x00, // BitsColor
    ];
    let expected = WindowingOrder::Window(WindowOrder {
        window_id: 0x42,
        is_new: false,
        body: WindowOrderBody::Icon(WindowIcon {
            big: true,
            overlay: false,
            icon: IconSource::Icon(IconInfo {
                cache_entry: 3,
                cache_id: 1,
                bpp: 1,
                width: 2,
                height: 2,
                bits_mask: vec![0x80, 0x00, 0x40, 0x00],
                color_table: vec![0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0x00],
                bits_color: vec![0xc0, 0x00, 0x00, 0x00],
            }),
        }),
    });

    assert_round_trip(&bytes, &expected);
}

// oracle: spec MS-RDPERP §2.2.1.3.1.2.3 and §2.2.1.2.4
#[test]
fn window_cached_icon_order() {
    #[rustfmt::skip]
    let bytes = [
        0x2e, 0x0e, 0x00, // controlFlags, orderSize = 14
        0x00, 0x00, 0x00, 0x81, // fieldsPresent: WINDOW | CACHED_ICON
        0x42, 0x00, 0x00, 0x00, // WindowId
        0x03, 0x00, 0x01, // CacheEntry, CacheId
    ];
    let expected = WindowingOrder::Window(WindowOrder {
        window_id: 0x42,
        is_new: false,
        body: WindowOrderBody::Icon(WindowIcon {
            big: false,
            overlay: false,
            icon: IconSource::Cached(CachedIconInfo {
                cache_entry: 3,
                cache_id: 1,
            }),
        }),
    });

    assert_round_trip(&bytes, &expected);
}

// oracle: spec MS-RDPERP §2.2.1.3.1.2.4
#[test]
fn deleted_window_order() {
    let bytes = [0x2e, 0x0b, 0x00, 0x00, 0x00, 0x00, 0x21, 0x07, 0x00, 0x00, 0x00];
    let expected = WindowingOrder::Window(WindowOrder {
        window_id: 7,
        is_new: false,
        body: WindowOrderBody::Deleted,
    });

    assert_round_trip(&bytes, &expected);
}

// oracle: spec MS-RDPERP §2.2.1.3.2.2.1 and §2.2.1.3.2.2.2
#[test]
fn new_notification_icon_with_balloon_tooltip() {
    #[rustfmt::skip]
    let bytes = [
        0x2e, 0x30, 0x00, // controlFlags, orderSize = 48
        0x0f, 0x00, 0x00, 0x92, // fieldsPresent: NOTIFY | STATE_NEW | CACHED_ICON | TIP | INFO_TIP | STATE | VERSION
        0x42, 0x00, 0x00, 0x00, // WindowId
        0x07, 0x00, 0x00, 0x00, // NotifyIconId
        0x04, 0x00, 0x00, 0x00, // Version: NOTIFYICON_VERSION_4
        0x04, 0x00, 0x48, 0x00, 0x69, 0x00, // ToolTip: "Hi"
        0x10, 0x27, 0x00, 0x00, // Timeout = 10000 ms
        0x01, 0x00, 0x00, 0x00, // InfoFlags: NIIF_INFO
        0x02, 0x00, 0x54, 0x00, // InfoTipText: "T"
        0x02, 0x00, 0x55, 0x00, // Title: "U"
        0x00, 0x00, 0x00, 0x00, // State: visible
        0x03, 0x00, 0x01, // CachedIcon
    ];
    let expected = WindowingOrder::NotifyIcon(NotifyIconOrder {
        window_id: 0x42,
        notify_icon_id: 7,
        is_new: true,
        state: Some(Box::new(NotifyIconState {
            version: Some(4),
            tooltip: Some("Hi".to_owned()),
            info_tip: Some(NotifyIconInfoTip {
                timeout: 10_000,
                info_flags: NotifyIconInfoTip::NIIF_INFO,
                text: "T".to_owned(),
                title: "U".to_owned(),
            }),
            state: Some(0),
            icon: Some(IconSource::Cached(CachedIconInfo {
                cache_entry: 3,
                cache_id: 1,
            })),
        })),
    });

    assert_round_trip(&bytes, &expected);
}

// oracle: spec MS-RDPERP §2.2.1.3.3.2.1
#[test]
fn actively_monitored_desktop_with_active_window_and_z_order() {
    #[rustfmt::skip]
    let bytes = [
        0x2e, 0x14, 0x00, // controlFlags, orderSize = 20
        0x32, 0x00, 0x00, 0x04, // fieldsPresent: DESKTOP | HOOKED | ZORDER | ACTIVE_WND
        0x42, 0x00, 0x00, 0x00, // ActiveWindowId
        0x02, // NumWindowIds
        0x42, 0x00, 0x00, 0x00, 0x43, 0x00, 0x00, 0x00, // WindowIds
    ];
    let expected = WindowingOrder::Desktop(DesktopOrder::ActivelyMonitored(MonitoredDesktop {
        hooked: true,
        arc_began: false,
        arc_completed: false,
        active_window_id: Some(0x42),
        z_order: Some(vec![0x42, 0x43]),
    }));

    assert_round_trip(&bytes, &expected);
    let WindowingOrder::Desktop(desktop) = expected else {
        unreachable!()
    };
    assert!(!desktop.resets_windows());
}

// oracle: spec MS-RDPERP §3.2.5.1.3
// A non-monitored desktop and the start of an auto-reconnect resend both invalidate
// the client's window state.
#[rstest]
#[case::non_monitored(&[0x2e, 0x07, 0x00, 0x01, 0x00, 0x00, 0x04])]
#[case::hooked_arc_began(&[0x2e, 0x07, 0x00, 0x0a, 0x00, 0x00, 0x04])]
fn desktop_orders_that_discard_windows(#[case] bytes: &[u8]) {
    let WindowingOrder::Desktop(desktop) = decode::<WindowingOrder>(bytes).unwrap() else {
        panic!("expected a desktop order");
    };
    assert!(desktop.resets_windows());
}

// oracle: spec MS-RDPERP §2.2.1.1.2
#[test]
fn client_area_size_requires_extended_support() {
    #[rustfmt::skip]
    let bytes = [
        0x2e, 0x13, 0x00, // controlFlags, orderSize = 19
        0x00, 0x00, 0x01, 0x01, // fieldsPresent: WINDOW | CLIENTAREASIZE
        0x07, 0x00, 0x00, 0x00, // WindowId
        0x10, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, // ClientAreaWidth, ClientAreaHeight
    ];

    assert!(decode::<WindowingOrder>(&bytes).unwrap().requires_extended_support());
}

// oracle: spec MS-RDPERP §2.2.1.3
#[rstest]
#[case::order_size_past_the_data(&[0x2e, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x21, 0x07, 0x00, 0x00, 0x00])]
#[case::truncated_title(&[0x2e, 0x0b, 0x00, 0x04, 0x00, 0x00, 0x01, 0x07, 0x00, 0x00, 0x00])]
#[case::deleted_window_with_additional_fields(&[0x2e, 0x0b, 0x00, 0x04, 0x00, 0x00, 0x21, 0x07, 0x00, 0x00, 0x00])]
#[case::both_icon_representations(
    &[0x2e, 0x0e, 0x00, 0x00, 0x00, 0x00, 0xc1, 0x07, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01]
)]
#[case::unknown_window_flag(&[0x2e, 0x0b, 0x00, 0x20, 0x00, 0x00, 0x01, 0x07, 0x00, 0x00, 0x00])]
#[case::new_notification_icon_without_an_icon(
    &[0x2e, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x12, 0x07, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]
)]
fn rejects_malformed_window_orders(#[case] bytes: &[u8]) {
    assert!(decode::<WindowingOrder>(bytes).is_err());
}

// oracle: spec MS-RDPEGDI §2.2.2.1
#[test]
fn slow_path_update_decodes_every_order() {
    let mut update = vec![0, 0, 2, 0, 0, 0]; // pad2OctetsA, numberOrders = 2, pad2OctetsB
    update.extend_from_slice(&NEW_WINDOW);
    update.extend_from_slice(&[0x2e, 0x0b, 0x00, 0x00, 0x00, 0x00, 0x21, 0x07, 0x00, 0x00, 0x00]);

    let decoded = try_decode_slow_path_windowing_orders(&mut ReadCursor::new(&update)).unwrap();

    assert_eq!(decoded.orders.len(), 2);
    assert_eq!(decoded.orders[0], new_window());
    assert!(!decoded.contains_non_window_orders);
}

// oracle: spec MS-RDPEGDI §2.2.2.2.1.1.2
#[test]
fn fast_path_update_stops_at_the_first_non_window_order() {
    let mut update = vec![2, 0]; // numberOrders = 2
    update.extend_from_slice(&[0x2e, 0x0b, 0x00, 0x00, 0x00, 0x00, 0x21, 0x07, 0x00, 0x00, 0x00]);
    update.push(0x01); // a primary drawing order

    let decoded = try_decode_fast_path_windowing_orders(&mut ReadCursor::new(&update)).unwrap();

    assert_eq!(decoded.orders.len(), 1);
    assert!(decoded.contains_non_window_orders);
}

// oracle: spec MS-RDPERP §2.2.1.3.1.2.1
#[test]
fn window_state_order_with_the_remaining_optional_fields() {
    #[rustfmt::skip]
    let bytes = [
        0x2e, 0x38, 0x00, // controlFlags, orderSize = 56
        0xc1, 0xc0, 0xee, 0x09, // fieldsPresent: WINDOW | APPBAR_EDGE | APPBAR_STATE | RESIZE_MARGIN_X | CLIENTAREAOFFSET
                                // | CLIENTDELTA | RPCONTENT | ROOTPARENT | ENFORCE_SERVER_ZORDER | ICON_OVERLAY_NULL
                                // | OVERLAY_DESCRIPTION | TASKBAR_BUTTON | RESIZE_MARGIN_Y
        0x07, 0x00, 0x00, 0x00, // WindowId
        0x0a, 0x00, 0x00, 0x00, 0x14, 0x00, 0x00, 0x00, // ClientOffsetX = 10, ClientOffsetY = 20
        0x04, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, // WindowLeftResizeMargin, WindowRightResizeMargin
        0x06, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, // WindowTopResizeMargin, WindowBottomResizeMargin
        0x01, // RPContent
        0x34, 0x12, 0x00, 0x00, // RootParentHandle
        0x03, 0x00, 0x00, 0x00, 0x1e, 0x00, 0x00, 0x00, // WindowClientDeltaX = 3, WindowClientDeltaY = 30
        0x02, 0x00, 0x42, 0x00, // OverlayDescription: "B"
        0x01, // TaskbarButton
        0x01, // EnforceServerZOrder
        0x01, // AppBarState
        0x02, // AppBarEdge
    ];
    let expected = WindowingOrder::Window(WindowOrder {
        window_id: 7,
        is_new: false,
        body: WindowOrderBody::State(Box::new(WindowState {
            client_offset: Some(WindowPoint { x: 10, y: 20 }),
            resize_margin_x: Some(ResizeMargins { start: 4, end: 5 }),
            resize_margin_y: Some(ResizeMargins { start: 6, end: 8 }),
            rp_content: Some(true),
            root_parent_handle: Some(0x1234),
            client_delta: Some(WindowPoint { x: 3, y: 30 }),
            overlay_description: Some("B".to_owned()),
            icon_overlay_null: true,
            taskbar_button: Some(1),
            enforce_server_z_order: Some(true),
            app_bar_state: Some(1),
            app_bar_edge: Some(2),
            ..WindowState::default()
        })),
    });

    assert_round_trip(&bytes, &expected);
    assert!(expected.requires_extended_support());
}

// oracle: spec MS-RDPERP §2.2.1.3.2.2.1 and §2.2.1.3.2.2.2
#[rstest]
#[case::version_1(NotifyIconState {
    version: Some(1),
    ..NotifyIconState::default()
})]
#[case::state_2(NotifyIconState {
    state: Some(2),
    ..NotifyIconState::default()
})]
#[case::info_flags_0x04(NotifyIconState {
    info_tip: Some(NotifyIconInfoTip {
        timeout: 0,
        info_flags: 0x04,
        text: String::new(),
        title: String::new(),
    }),
    ..NotifyIconState::default()
})]
fn notification_icon_values_the_decoder_rejects_are_not_encoded(#[case] state: NotifyIconState) {
    let order = WindowingOrder::NotifyIcon(NotifyIconOrder {
        window_id: 7,
        notify_icon_id: 1,
        is_new: false,
        state: Some(Box::new(state)),
    });

    assert!(encode_vec(&order).is_err());
}

#[test]
fn slow_path_orders_update_encoding_inverts_its_decoding() {
    let orders = [
        new_window(),
        decode::<WindowingOrder>(&[0x2e, 0x0b, 0x00, 0x00, 0x00, 0x00, 0x21, 0x07, 0x00, 0x00, 0x00]).unwrap(),
    ];
    let mut update = vec![0; slow_path_windowing_orders_update_size(&orders)];

    encode_slow_path_windowing_orders_update(&orders, &mut WriteCursor::new(&mut update)).unwrap();

    // oracle: spec MS-RDPBCGR §2.2.9.1.1.3.1 and MS-RDPEGDI §2.2.2.1
    assert_eq!(&update[..8], [0, 0, 0, 0, 2, 0, 0, 0]); // updateType: ORDERS, pad2OctetsA, numberOrders = 2, pad2OctetsB
    // oracle: property round-trip
    let decoded = try_decode_slow_path_windowing_orders(&mut ReadCursor::new(&update[2..])).unwrap();
    assert_eq!(decoded.orders, orders);
    assert!(!decoded.contains_non_window_orders);
}

// oracle: reference Devolutions/IronRDP@1de38a2ee553:crates/ironrdp-pdu/src/basic_output/window.rs
// (validate_window_fields accepts ICON_BIG and ICON_OVERLAY on a state order; neither carries data)
// oracle: spec MS-RDPERP §2.2.1.3.1.2.2 (flag values: ICON_BIG 0x00002000, ICON_OVERLAY 0x00100000)
#[rstest]
#[case::icon_big(0x0100_2000)]
#[case::icon_overlay(0x0110_0000)]
#[case::icon_big_and_overlay(0x0110_2000)]
fn icon_flags_without_an_icon_are_ignored(#[case] fields_present: u32) {
    let mut bytes = vec![0x2e, 0x0b, 0x00]; // controlFlags, orderSize = 11
    bytes.extend_from_slice(&fields_present.to_le_bytes());
    bytes.extend_from_slice(&7u32.to_le_bytes()); // WindowId

    assert_eq!(
        decode::<WindowingOrder>(&bytes).unwrap(),
        WindowingOrder::Window(WindowOrder {
            window_id: 7,
            is_new: false,
            body: WindowOrderBody::State(Box::default()),
        })
    );
}
