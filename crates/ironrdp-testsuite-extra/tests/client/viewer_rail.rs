//! RemoteApp window model, coordinate translation and pixel helpers of the viewer.

use ironrdp::input::MousePosition;
use ironrdp::pdu::geometry::ExclusiveRectangle;
use ironrdp::pdu::window::{IconInfo, ShowState, WindowPoint, WindowSize, WindowState, WindowStyle};
use ironrdp_viewer::rail::{
    MAX_TOTAL_WINDOW_PIXELS, MAX_WINDOW_PIXELS, ModelChanges, RailWindowModel, WindowPixels, bounded_window_size,
    crop_frame, fits_window_pixel_budget, icon_to_rgba, window_point_to_desktop,
};
use proptest::prelude::*;

const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;

fn model_with(state: WindowState) -> RailWindowModel {
    let mut model = RailWindowModel::default();
    model.apply(&state);
    model
}

// oracle: spec MS-RDPERP §2.2.1.3.1.2.1
#[test]
fn incremental_orders_keep_fields_they_do_not_carry() {
    let mut model = model_with(WindowState {
        owner_window_id: Some(0),
        style: Some(WindowStyle {
            style: 0x10cf_0000,
            extended_style: WS_EX_TOOLWINDOW,
        }),
        title: Some("Notepad".to_owned()),
        window_offset: Some(WindowPoint { x: -8, y: 100 }),
        window_size: Some(WindowSize {
            width: 640,
            height: 480,
        }),
        ..WindowState::default()
    });

    let changes = model.apply(&WindowState {
        window_offset: Some(WindowPoint { x: 20, y: 100 }),
        ..WindowState::default()
    });

    assert_eq!(
        changes,
        ModelChanges {
            geometry: true,
            ..ModelChanges::default()
        }
    );
    assert_eq!(model.title, "Notepad");
    assert_eq!((model.x, model.y, model.width, model.height), (20, 100, 640, 480));
}

// oracle: mutant re-sent unchanged values reported as changes, repainting and moving the window again
#[test]
fn resent_unchanged_values_report_no_change() {
    let state = WindowState {
        show_state: Some(ShowState::Show),
        title: Some("Notepad".to_owned()),
        window_offset: Some(WindowPoint { x: 10, y: 20 }),
        window_size: Some(WindowSize {
            width: 640,
            height: 480,
        }),
        taskbar_button: Some(0),
        ..WindowState::default()
    };
    let mut model = model_with(state.clone());

    assert_eq!(model.apply(&state), ModelChanges::default());
}

// oracle: mutant a zero-sized server window passed to winit and softbuffer, which require non-zero sizes
#[test]
fn zero_window_size_is_clamped_to_one_pixel() {
    let model = model_with(WindowState {
        window_size: Some(WindowSize { width: 0, height: 0 }),
        ..WindowState::default()
    });

    assert_eq!((model.width, model.height), (1, 1));
}

// oracle: spec MS-RDPERP §2.2.1.3.1.2.1 (ShowState 0x00: "Do not show the window")
#[test]
fn hidden_windows_are_not_visible_until_shown() {
    let mut model = model_with(WindowState {
        show_state: Some(ShowState::Hide),
        ..WindowState::default()
    });
    assert!(!model.is_visible());

    let changes = model.apply(&WindowState {
        show_state: Some(ShowState::Show),
        ..WindowState::default()
    });
    assert!(changes.show_state);
    assert!(model.is_visible());
}

// oracle: reference Win32 window ownership (window IDs are server HWNDs, and a null owner means none)
#[test]
fn owner_zero_means_no_owner() {
    let mut model = model_with(WindowState {
        owner_window_id: Some(7),
        ..WindowState::default()
    });
    assert_eq!(model.owner_window_id, Some(7));

    model.apply(&WindowState {
        owner_window_id: Some(0),
        ..WindowState::default()
    });
    assert_eq!(model.owner_window_id, None);
}

// oracle: reference Win32 extended window styles (WS_EX_TOOLWINDOW windows have no taskbar button)
#[test]
fn tool_windows_skip_the_taskbar() {
    let model = model_with(WindowState {
        style: Some(WindowStyle {
            style: 0,
            extended_style: WS_EX_TOOLWINDOW,
        }),
        ..WindowState::default()
    });

    assert!(model.skips_taskbar());
}

// oracle: reference Win32 window ownership (owned windows have no taskbar button)
#[test]
fn owned_windows_skip_the_taskbar() {
    let model = model_with(WindowState {
        owner_window_id: Some(7),
        ..WindowState::default()
    });

    assert!(model.skips_taskbar());
}

// oracle: spec MS-RDPERP §2.2.1.3.1.2.1 (TaskbarButton 0x01 removes the window's taskbar tab, which
// the viewer approximates by keeping the window off the taskbar)
#[test]
fn taskbar_button_hides_the_window_from_the_taskbar() {
    let mut model = RailWindowModel::default();
    assert!(!model.skips_taskbar());

    let changes = model.apply(&WindowState {
        taskbar_button: Some(1),
        ..WindowState::default()
    });

    assert!(changes.taskbar);
    assert!(model.skips_taskbar());
}

// oracle: reference Win32 extended window styles (WS_EX_TOOLWINDOW, WS_EX_NOACTIVATE)
#[test]
fn tool_and_non_activating_windows_do_not_take_focus_when_opened() {
    let with_extended_style = |extended_style| {
        model_with(WindowState {
            style: Some(WindowStyle {
                style: 0,
                extended_style,
            }),
            ..WindowState::default()
        })
    };

    assert!(with_extended_style(0).activates_on_open());
    assert!(!with_extended_style(WS_EX_TOOLWINDOW).activates_on_open());
    assert!(!with_extended_style(WS_EX_NOACTIVATE).activates_on_open());
}

// oracle: spec MS-RDPERP §2.2.1.3.1.2.1
#[test]
fn window_positions_translate_by_the_window_origin() {
    assert_eq!(
        window_point_to_desktop((100, 50), 10.0, 3.0),
        Some(MousePosition { x: 110, y: 53 })
    );
}

// oracle: mutant fractional window positions rounded instead of truncated to the pixel they lie in
#[test]
fn fractional_window_positions_floor_to_their_pixel() {
    assert_eq!(
        window_point_to_desktop((100, 50), 10.7, 3.2),
        Some(MousePosition { x: 110, y: 53 })
    );
}

// oracle: spec MS-RDPBCGR §2.2.8.1.1.3.1.1.3
#[test]
fn window_positions_left_of_the_desktop_clamp_to_zero() {
    assert_eq!(
        window_point_to_desktop((-8, 0), 2.0, 2.0),
        Some(MousePosition { x: 0, y: 2 })
    );
}

// oracle: spec MS-RDPBCGR §2.2.8.1.1.3.1.1.3
#[test]
fn window_positions_past_the_pointer_range_clamp_to_its_maximum() {
    assert_eq!(
        window_point_to_desktop((65_530, i32::MAX), 100.0, 0.0),
        Some(MousePosition {
            x: u16::MAX,
            y: u16::MAX,
        })
    );
}

// oracle: mutant a NaN position from the windowing system becoming pointer coordinate 0
#[test]
fn non_finite_window_positions_are_rejected() {
    assert_eq!(window_point_to_desktop((0, 0), f64::NAN, 0.0), None);
    assert_eq!(window_point_to_desktop((0, 0), 0.0, f64::INFINITY), None);
}

// 3x2 frame:
// 1 2 3
// 4 5 6
const FRAME: [u32; 6] = [1, 2, 3, 4, 5, 6];

// oracle: mutant crop offsets or strides off by one
#[test]
fn crop_inside_the_frame_copies_the_rectangle() {
    assert_eq!(crop_frame(&FRAME, (3, 2), (1, 0), (2, 2)), [2, 3, 5, 6]);
}

// oracle: mutant pixels outside the frame copied from a neighboring row instead of black
#[test]
fn crop_left_of_and_below_the_frame_pads_with_black() {
    assert_eq!(crop_frame(&FRAME, (3, 2), (-1, 1), (3, 2)), [0, 4, 5, 0, 0, 0]);
}

// oracle: mutant pixels outside the frame copied from a neighboring row instead of black
#[test]
fn crop_right_of_the_frame_pads_with_black() {
    assert_eq!(crop_frame(&FRAME, (3, 2), (2, 0), (2, 1)), [3, 0]);
}

// oracle: mutant a crop larger than the frame on both axes reading past the frame
#[test]
fn crop_larger_than_the_frame_on_both_axes_surrounds_it_with_black() {
    assert_eq!(
        crop_frame(&FRAME, (3, 2), (-1, -1), (5, 4)),
        [
            0, 0, 0, 0, 0, //
            0, 1, 2, 3, 0, //
            0, 4, 5, 6, 0, //
            0, 0, 0, 0, 0, //
        ]
    );
}

// oracle: reference crates/ironrdp-egfx/src/compositor.rs WindowUpdate (tightly-packed RGBA8888) and
// softbuffer's 0x00RRGGBB pixel format
#[test]
fn surface_updates_paint_their_region() {
    let mut pixels = WindowPixels::default();
    let region = ExclusiveRectangle {
        left: 1,
        top: 0,
        right: 2,
        bottom: 2,
    };

    pixels.apply((2, 2), &region, &[0x11, 0x22, 0x33, 0xFF, 0x44, 0x55, 0x66, 0xFF]);

    assert_eq!(pixels.pixels, [0, 0x0011_2233, 0, 0x0044_5566]);
}

// oracle: reference crates/ironrdp-egfx/src/compositor.rs WindowUpdate (an empty region carries no pixels)
#[test]
fn empty_surface_update_regions_paint_nothing() {
    let mut pixels = WindowPixels::default();
    let region = ExclusiveRectangle {
        left: 1,
        top: 0,
        right: 1,
        bottom: 2,
    };

    pixels.apply((2, 2), &region, &[0x11, 0x22, 0x33, 0xFF, 0x44, 0x55, 0x66, 0xFF]);

    assert_eq!(pixels.pixels, [0, 0, 0, 0]);
}

// oracle: spec MS-RDPERP §2.2.1.2.3
#[test]
fn icons_are_flipped_and_masked() {
    // 2x2, 24 bpp, bottom-up rows padded to four bytes; mask bit set = transparent.
    let icon = IconInfo {
        cache_entry: IconInfo::NO_CACHE_ENTRY,
        cache_id: IconInfo::NO_CACHE_ID,
        bpp: 24,
        width: 2,
        height: 2,
        bits_mask: vec![0x40, 0, 0, 0, 0x00, 0, 0, 0],
        color_table: Vec::new(),
        bits_color: vec![
            0x03, 0x02, 0x01, 0x06, 0x05, 0x04, 0, 0, // bottom row
            0x09, 0x08, 0x07, 0x0c, 0x0b, 0x0a, 0, 0, // top row
        ],
    };

    let (rgba, width, height) = icon_to_rgba(&icon).unwrap();

    assert_eq!((width, height), (2, 2));
    assert_eq!(
        rgba,
        [
            0x07, 0x08, 0x09, 0xff, 0x0a, 0x0b, 0x0c, 0xff, // top row
            0x01, 0x02, 0x03, 0xff, 0x04, 0x05, 0x06, 0x00, // bottom row, right pixel masked
        ]
    );
}

// oracle: spec MS-RDPERP §2.2.1.2.3
#[test]
fn icons_with_alpha_ignore_the_mask() {
    let icon = IconInfo {
        cache_entry: IconInfo::NO_CACHE_ENTRY,
        cache_id: IconInfo::NO_CACHE_ID,
        bpp: 32,
        width: 1,
        height: 1,
        bits_mask: vec![0x80, 0, 0, 0],
        color_table: Vec::new(),
        bits_color: vec![0x03, 0x02, 0x01, 0x80],
    };

    assert_eq!(icon_to_rgba(&icon).unwrap().0, [0x01, 0x02, 0x03, 0x80]);
}

// oracle: mutant a window allowed past the per-window pixel budget
#[test]
fn window_content_past_the_per_window_budget_is_refused() {
    assert!(fits_window_pixel_budget((7680, 4320), 0));
    assert!(!fits_window_pixel_budget((7681, 4320), 0));
    assert!(!fits_window_pixel_budget((u16::MAX, u16::MAX), 0));
}

// oracle: mutant windows allowed past the total pixel budget
#[test]
fn window_content_past_the_total_budget_is_refused() {
    let others = MAX_TOTAL_WINDOW_PIXELS - MAX_WINDOW_PIXELS;
    assert!(fits_window_pixel_budget((7680, 4320), others));
    assert!(!fits_window_pixel_budget((7680, 4320), others + 1));
}

proptest! {
    // oracle: reference per-pixel model of the crop_frame contract in crates/ironrdp-viewer/src/rail.rs
    #[test]
    fn crop_copies_the_frame_and_pads_the_rest_with_black(
        frame_width in 0u16..6,
        frame_height in 0u16..6,
        x in -8i32..8,
        y in -8i32..8,
        width in 0u32..10,
        height in 0u32..10,
    ) {
        let frame: Vec<u32> = (1..=u32::from(frame_width) * u32::from(frame_height)).collect();

        let crop = crop_frame(&frame, (frame_width, frame_height), (x, y), (width, height));

        prop_assert_eq!(crop.len(), usize::try_from(width * height).unwrap());
        for row in 0..i64::from(height) {
            for column in 0..i64::from(width) {
                let (source_x, source_y) = (i64::from(x) + column, i64::from(y) + row);
                let expected = if (0..i64::from(frame_width)).contains(&source_x)
                    && (0..i64::from(frame_height)).contains(&source_y)
                {
                    frame[usize::try_from(source_y * i64::from(frame_width) + source_x).unwrap()]
                } else {
                    0
                };
                prop_assert_eq!(crop[usize::try_from(row * i64::from(width) + column).unwrap()], expected);
            }
        }
    }
}

// oracle: spec MS-RDPERP §2.2.1.2.3 (1 bpp color bits index a two-entry RGBQUAD color table)
#[test]
fn one_bit_icons_use_the_color_table() {
    let icon = IconInfo {
        cache_entry: IconInfo::NO_CACHE_ENTRY,
        cache_id: IconInfo::NO_CACHE_ID,
        bpp: 1,
        width: 2,
        height: 1,
        bits_mask: vec![0x00, 0x00],
        color_table: vec![0x00, 0x00, 0x00, 0x00, 0x30, 0x20, 0x10, 0x00],
        bits_color: vec![0x40, 0x00], // index 0 then index 1
    };

    let (rgba, _, _) = icon_to_rgba(&icon).unwrap();

    assert_eq!(rgba, [0x00, 0x00, 0x00, 0xFF, 0x10, 0x20, 0x30, 0xFF]);
}

// oracle: spec MS-RDPERP §2.2.1.2.3 (4 bpp color bits index a 16-entry RGBQUAD color table)
#[test]
fn four_bit_icons_use_the_color_table() {
    let mut color_table = vec![0; 16 * 4];
    color_table[2 * 4..2 * 4 + 3].copy_from_slice(&[0x03, 0x02, 0x01]);
    color_table[0xF * 4..0xF * 4 + 3].copy_from_slice(&[0x06, 0x05, 0x04]);
    let icon = IconInfo {
        cache_entry: IconInfo::NO_CACHE_ENTRY,
        cache_id: IconInfo::NO_CACHE_ID,
        bpp: 4,
        width: 2,
        height: 1,
        bits_mask: vec![0x00, 0x00],
        color_table,
        bits_color: vec![0x2F, 0x00], // index 2 then index 15
    };

    let (rgba, _, _) = icon_to_rgba(&icon).unwrap();

    assert_eq!(rgba, [0x01, 0x02, 0x03, 0xFF, 0x04, 0x05, 0x06, 0xFF]);
}

proptest! {
    // oracle: mutant icon fields from the server indexing past the color, mask or palette buffers
    #[test]
    fn icon_decoding_never_panics_on_server_data(
        bpp in prop_oneof![Just(1u8), Just(4u8), Just(8u8), Just(16u8), Just(24u8), Just(32u8), any::<u8>()],
        width in 0u16..40,
        height in 0u16..40,
        bits_mask in proptest::collection::vec(any::<u8>(), 0..256),
        color_table in proptest::collection::vec(any::<u8>(), 0..80),
        bits_color in proptest::collection::vec(any::<u8>(), 0..1024),
    ) {
        let icon = IconInfo {
            cache_entry: IconInfo::NO_CACHE_ENTRY,
            cache_id: IconInfo::NO_CACHE_ID,
            bpp,
            width,
            height,
            bits_mask,
            color_table,
            bits_color,
        };

        if let Some((rgba, decoded_width, decoded_height)) = icon_to_rgba(&icon) {
            prop_assert_eq!((decoded_width, decoded_height), (u32::from(width), u32::from(height)));
            prop_assert_eq!(rgba.len(), usize::from(width) * usize::from(height) * 4);
        }
    }
}

// oracle: mutant a server window size reaches native window creation or a painting buffer unbounded
#[rstest::rstest]
#[case::within_the_budget((1920, 1080), (1920, 1080))]
#[case::empty((0, 0), (1, 1))]
#[case::too_wide((40_000, 10), (16_384, 10))]
#[case::too_large_an_area((32_766, 32_766), (16_384, 2_025))]
fn server_window_sizes_are_bounded_for_local_windows(#[case] size: (u32, u32), #[case] bounded: (u32, u32)) {
    let (width, height) = bounded_window_size(size.0, size.1);

    assert_eq!((width, height), bounded);
    assert!(usize::try_from(u64::from(width) * u64::from(height)).unwrap() <= MAX_WINDOW_PIXELS);
}
