use ironrdp_client::rail::{RailClient, RailControlEvent, RailEvent, RailInputEvent};
use ironrdp_core::{decode, encode_vec};
use ironrdp_rail::pdu::{
    AccentColor, ClientStatusPdu, ClientSystemParameter, ClientSystemParametersPdu, CloakPdu, ExecutePdu, FilterKeys,
    HandshakeExPdu, HandshakePdu, HighContrast, LocalMoveSizePdu, MinMaxInfoPdu, MoveSizeType, RailPdu, Rectangle,
    SnapArrangePdu, StickyKeys, SystemParameterRectangle, TaskbarInfoPdu, TaskbarMessage, ToggleKeys, WindowMovePdu,
};
use ironrdp_svc::{SvcMessage, SvcProcessor as _};
use rstest::rstest;

#[test]
fn queued_execute_is_released_after_handshake_fallback() {
    let mut client = RailClient::new(1, 1024, 768);
    client
        .queue_execute(ExecutePdu {
            flags: 0,
            executable: "app".to_owned(),
            working_directory: String::new(),
            arguments: String::new(),
        })
        .unwrap();
    client
        .process(&encode_vec(&RailPdu::Handshake(HandshakePdu { build_number: 1 })).unwrap())
        .unwrap();

    let messages: Vec<_> = client.release_queued_after_handshake().unwrap().into();
    assert_eq!(messages.len(), 1);
    assert!(matches!(
        decode::<RailPdu>(&messages[0].encode_unframed_pdu().unwrap()).unwrap(),
        RailPdu::Execute(_)
    ));
    assert!(matches!(
        client.take_events().as_slice(),
        [
            RailEvent::Handshake {
                queued_execute_count: 1,
                ..
            },
            RailEvent::PostHandshakeQueueReleased {
                released_execute_count: 1
            }
        ]
    ));
}

#[test]
fn server_cannot_send_client_input() {
    let mut client = RailClient::new(1, 1, 1);
    client
        .process(&encode_vec(&RailPdu::Handshake(HandshakePdu { build_number: 1 })).unwrap())
        .unwrap();

    assert!(
        client
            .process(
                &encode_vec(&RailPdu::Execute(ExecutePdu {
                    flags: 0,
                    executable: "app".to_owned(),
                    working_directory: String::new(),
                    arguments: String::new(),
                }))
                .unwrap()
            )
            .is_err()
    );
}

#[test]
fn handshake_advertises_supported_server_controls() {
    let mut client = RailClient::new(1, 1, 1);
    let messages = client
        .process(&encode_vec(&RailPdu::Handshake(HandshakePdu { build_number: 1 })).unwrap())
        .unwrap();

    assert!(matches!(
            decode::<RailPdu>(&messages[1].encode_unframed_pdu().unwrap()).unwrap(),
            RailPdu::ClientStatus(ClientStatusPdu { flags })
                if flags
                    == ClientStatusPdu::Z_ORDER_SYNC
                        | ClientStatusPdu::POWER_DISPLAY_REQUEST
                        | ClientStatusPdu::BIDIRECTIONAL_CLOAK
    ));
}

#[test]
fn handshake_advertises_configured_client_status_flags() {
    let mut client = RailClient::new(1, 1, 1).with_client_status_flags(0);
    let messages = client
        .process(&encode_vec(&RailPdu::Handshake(HandshakePdu { build_number: 1 })).unwrap())
        .unwrap();

    assert!(matches!(
        decode::<RailPdu>(&messages[1].encode_unframed_pdu().unwrap()).unwrap(),
        RailPdu::ClientStatus(ClientStatusPdu { flags: 0 })
    ));
}

#[test]
fn desktop_synchronization_releases_queued_execute() {
    let mut client = RailClient::new(1, 1, 1);
    client
        .queue_execute(ExecutePdu {
            flags: 0,
            executable: "app".to_owned(),
            working_directory: String::new(),
            arguments: String::new(),
        })
        .unwrap();
    client
        .process(&encode_vec(&RailPdu::Handshake(HandshakePdu { build_number: 1 })).unwrap())
        .unwrap();

    let messages: Vec<_> = client.complete_desktop_synchronization().unwrap().into();

    assert!(matches!(
        decode::<RailPdu>(&messages[0].encode_unframed_pdu().unwrap()).unwrap(),
        RailPdu::Execute(_)
    ));
    assert!(matches!(
        client.take_events().as_slice(),
        [
            RailEvent::Handshake { .. },
            RailEvent::DesktopSynchronized {
                released_execute_count: 1
            }
        ]
    ));
}

#[test]
fn desktop_size_update_sends_display_change_and_work_area() {
    let mut client = RailClient::new(1, 1, 1);
    client
        .process(&encode_vec(&RailPdu::Handshake(HandshakePdu { build_number: 1 })).unwrap())
        .unwrap();

    let messages: Vec<_> = client.update_desktop_size(1024, 768).into();

    assert!(matches!(
        decode::<RailPdu>(&messages[0].encode_unframed_pdu().unwrap()).unwrap(),
        RailPdu::ClientSystemParameters(ClientSystemParametersPdu {
            parameter: ClientSystemParameter::DisplayChange(SystemParameterRectangle {
                right: 1024,
                bottom: 768,
                ..
            })
        })
    ));
    assert!(matches!(
        decode::<RailPdu>(&messages[1].encode_unframed_pdu().unwrap()).unwrap(),
        RailPdu::ClientSystemParameters(ClientSystemParametersPdu {
            parameter: ClientSystemParameter::WorkArea(SystemParameterRectangle {
                right: 1024,
                bottom: 768,
                ..
            })
        })
    ));
}

fn handshake(client: &mut RailClient, handshake_ex_flags: Option<u32>) -> Vec<SvcMessage> {
    let pdu = match handshake_ex_flags {
        Some(flags) => RailPdu::HandshakeEx(HandshakeExPdu { build_number: 1, flags }),
        None => RailPdu::Handshake(HandshakePdu { build_number: 1 }),
    };
    client.process(&encode_vec(&pdu).unwrap()).unwrap()
}

fn sent_pdus(messages: Vec<SvcMessage>) -> Vec<RailPdu> {
    messages
        .into_iter()
        .map(|message| decode::<RailPdu>(&message.encode_unframed_pdu().unwrap()).unwrap())
        .collect()
}

/// Decodes the system parameters among `messages`, failing on any PDU other than
/// the Handshake and Client Status PDUs that open initialization.
fn sent_system_parameters(messages: Vec<SvcMessage>) -> Vec<ClientSystemParameter> {
    sent_pdus(messages)
        .into_iter()
        .filter_map(|pdu| match pdu {
            RailPdu::ClientSystemParameters(pdu) => Some(pdu.parameter),
            other => {
                assert!(
                    matches!(other, RailPdu::Handshake(_) | RailPdu::ClientStatus(_)),
                    "unexpected RAIL PDU {other:?}"
                );
                None
            }
        })
        .collect()
}

fn rectangle(right: u16, bottom: u16) -> SystemParameterRectangle {
    SystemParameterRectangle {
        left: 0,
        top: 0,
        right,
        bottom,
    }
}

fn established_client(handshake_ex_flags: Option<u32>) -> RailClient {
    let mut client = RailClient::new(1, 800, 600);
    handshake(&mut client, handshake_ex_flags);
    client.complete_desktop_synchronization().unwrap();
    client.take_events();
    client
}

/// Whether `parameter` is sent at initialization, when the host supplies it up front,
/// and on update, when the host reports it after the handshake.
fn sent_at_initialization_and_on_update(
    parameter: &ClientSystemParameter,
    handshake_ex_flags: Option<u32>,
) -> (bool, bool) {
    let mut client = RailClient::new(1, 800, 600)
        .with_system_parameters(vec![parameter.clone()])
        .unwrap();
    let at_initialization = sent_system_parameters(handshake(&mut client, handshake_ex_flags)).contains(parameter);

    let mut client = RailClient::new(1, 800, 600);
    handshake(&mut client, handshake_ex_flags);
    let messages = client.update_system_parameters(vec![parameter.clone()]).unwrap();
    let on_update = sent_system_parameters(messages.into()).contains(parameter);

    (at_initialization, on_update)
}

const ALL_EXTENDED_SPI: u32 =
    HandshakeExPdu::EXTENDED_SPI | HandshakeExPdu::EXTENDED_SPI_2 | HandshakeExPdu::EXTENDED_SPI_3;

/// Asserts that `parameter` is withheld after a plain Handshake and after a HandshakeEx
/// advertising every extended level but `required_flag`, and sent once `required_flag`
/// alone is advertised.
fn assert_gated_on(parameter: &ClientSystemParameter, required_flag: u32) {
    assert_eq!(
        sent_at_initialization_and_on_update(parameter, None),
        (false, false),
        "plain Handshake"
    );
    assert_eq!(
        sent_at_initialization_and_on_update(parameter, Some(ALL_EXTENDED_SPI & !required_flag)),
        (false, false),
        "HandshakeEx without the required flag"
    );
    assert_eq!(
        sent_at_initialization_and_on_update(parameter, Some(required_flag)),
        (true, true),
        "HandshakeEx with the required flag"
    );
}

fn accent_color() -> AccentColor {
    AccentColor {
        fields_valid_flags: 0,
        accent_color: 0x0078_d4ff,
        colorization_color: 0,
        colorization_color_balance: 0,
        colorization_afterglow: 0,
        colorization_afterglow_balance: 0,
        colorization_blur_balance: 0,
        colorization_glass_attribute: 0,
        color_prevalence: 0,
        enable_window_colorization: 0,
        accent_color_menu: 0,
        start_color_menu: 0,
        accent_palette: Vec::new(),
    }
}

// oracle: spec MS-RDPERP §2.2.2.4.1 (these SystemParam values carry no HandshakeEx condition)
#[rstest]
#[case::full_window_drag(ClientSystemParameter::FullWindowDrag(true))]
#[case::keyboard_cues(ClientSystemParameter::KeyboardCues(true))]
#[case::keyboard_preference(ClientSystemParameter::KeyboardPreference(true))]
#[case::work_area(ClientSystemParameter::WorkArea(rectangle(800, 560)))]
#[case::mouse_button_swap(ClientSystemParameter::MouseButtonSwap(true))]
#[case::taskbar_position(ClientSystemParameter::TaskbarPosition(rectangle(800, 40)))]
#[case::high_contrast(ClientSystemParameter::HighContrast(HighContrast {
    flags: 0x0000_0001,
    color_scheme: "High Contrast Black".to_owned(),
}))]
fn unconditional_system_parameters_are_sent_after_a_plain_handshake(#[case] parameter: ClientSystemParameter) {
    assert_eq!(sent_at_initialization_and_on_update(&parameter, None), (true, true));
}

// oracle: spec MS-RDPERP §2.2.2.4.1 ("SHOULD NOT be sent if the server did not advertise the
// EXTENDED_SPI_SUPPORTED flag in the HandshakeEx PDU")
#[rstest]
#[case::caret_width(ClientSystemParameter::CaretWidth(2))]
#[case::sticky_keys(ClientSystemParameter::StickyKeys(StickyKeys { flags: 0 }))]
#[case::toggle_keys(ClientSystemParameter::ToggleKeys(ToggleKeys { flags: 0 }))]
#[case::filter_keys(ClientSystemParameter::FilterKeys(FilterKeys {
    flags: 0,
    wait_time: 0,
    delay_time: 0,
    repeat_time: 0,
    bounce_time: 0,
}))]
fn extended_spi_parameters_require_extended_spi_support(#[case] parameter: ClientSystemParameter) {
    assert_gated_on(&parameter, HandshakeExPdu::EXTENDED_SPI);
}

// oracle: spec MS-RDPERP §2.2.2.4.1 ("SHOULD NOT be sent if the server did not advertise the
// EXTENDED_SPI_2_SUPPORTED flag in the HandshakeEx PDU")
#[rstest]
#[case::display_animations_enabled(ClientSystemParameter::DisplayAnimationsEnabled(true))]
#[case::display_advanced_effects_enabled(ClientSystemParameter::DisplayAdvancedEffectsEnabled(true))]
#[case::display_auto_hide_scrollbars(ClientSystemParameter::DisplayAutoHideScrollbars(true))]
#[case::display_message_duration(ClientSystemParameter::DisplayMessageDuration(5))]
#[case::closed_caption_font_color(ClientSystemParameter::ClosedCaptionFontColor(1))]
#[case::closed_caption_font_opacity(ClientSystemParameter::ClosedCaptionFontOpacity(1))]
#[case::closed_caption_font_size(ClientSystemParameter::ClosedCaptionFontSize(1))]
#[case::closed_caption_font_style(ClientSystemParameter::ClosedCaptionFontStyle(1))]
#[case::closed_caption_font_edge_effect(ClientSystemParameter::ClosedCaptionFontEdgeEffect(1))]
#[case::closed_caption_background_color(ClientSystemParameter::ClosedCaptionBackgroundColor(1))]
#[case::closed_caption_background_opacity(ClientSystemParameter::ClosedCaptionBackgroundOpacity(1))]
#[case::closed_caption_region_color(ClientSystemParameter::ClosedCaptionRegionColor(1))]
#[case::closed_caption_region_opacity(ClientSystemParameter::ClosedCaptionRegionOpacity(1))]
fn extended_spi_2_parameters_require_extended_spi_2_support(#[case] parameter: ClientSystemParameter) {
    assert_gated_on(&parameter, HandshakeExPdu::EXTENDED_SPI_2);
}

// oracle: spec MS-RDPERP §2.2.2.4.1 ("SHOULD NOT be sent if the server did not advertise the
// EXTENDED_SPI_3_SUPPORTED flag in the HandshakeEx PDU")
#[rstest]
#[case::accent_color(ClientSystemParameter::AccentColor(accent_color()))]
#[case::system_uses_light_theme(ClientSystemParameter::SystemUsesLightTheme(true))]
#[case::apps_use_light_theme(ClientSystemParameter::AppsUseLightTheme(true))]
fn extended_spi_3_parameters_require_extended_spi_3_support(#[case] parameter: ClientSystemParameter) {
    assert_gated_on(&parameter, HandshakeExPdu::EXTENDED_SPI_3);
}

// oracle: spec MS-RDPERP §2.2.2.4.1 (RAIL_SPI_DISPLAYCHANGE reports the display resolution, which
// the client takes from its desktop size)
#[test]
fn host_supplied_display_change_is_discarded_for_the_desktop_size() {
    let host_display = ClientSystemParameter::DisplayChange(rectangle(1, 1));
    let mut client = RailClient::new(1, 800, 600)
        .with_system_parameters(vec![host_display.clone()])
        .unwrap();

    let at_initialization = sent_system_parameters(handshake(&mut client, None));
    let on_update: Vec<_> = client.update_system_parameters(vec![host_display]).unwrap().into();
    let after_resize = sent_system_parameters(client.update_desktop_size(1024, 768).into());

    assert!(at_initialization.contains(&ClientSystemParameter::DisplayChange(rectangle(800, 600))));
    assert!(!at_initialization.contains(&ClientSystemParameter::DisplayChange(rectangle(1, 1))));
    assert!(on_update.is_empty());
    assert!(after_resize.contains(&ClientSystemParameter::DisplayChange(rectangle(1024, 768))));
}

// oracle: spec MS-RDPERP §2.2.2.4.1 (SPI_SETCARETWIDTH: "The minimum allowed value is 0x00000001")
#[test]
fn invalid_system_parameters_are_rejected_when_supplied() {
    let invalid = vec![ClientSystemParameter::CaretWidth(0)];

    assert!(
        RailClient::new(1, 800, 600)
            .with_system_parameters(invalid.clone())
            .is_err()
    );
    assert!(RailClient::new(1, 800, 600).update_system_parameters(invalid).is_err());
}

// oracle: mutant host work area dropped, or desktop work area sent alongside it
#[test]
fn host_work_area_replaces_the_desktop_rectangle() {
    let work_area = rectangle(800, 560);
    let mut client = RailClient::new(1, 800, 600)
        .with_system_parameters(vec![ClientSystemParameter::WorkArea(work_area)])
        .unwrap();

    let sent = sent_system_parameters(handshake(&mut client, None));

    assert!(sent.contains(&ClientSystemParameter::DisplayChange(rectangle(800, 600))));
    assert!(sent.contains(&ClientSystemParameter::WorkArea(work_area)));
    assert!(!sent.contains(&ClientSystemParameter::WorkArea(rectangle(800, 600))));
}

// oracle: mutant unchanged values re-sent, or changed values dropped
#[test]
fn system_parameter_updates_send_only_changed_values() {
    let mut client = RailClient::new(1, 800, 600);
    handshake(&mut client, None);

    let first = client
        .update_system_parameters(vec![ClientSystemParameter::KeyboardCues(true)])
        .unwrap();
    let repeated = client
        .update_system_parameters(vec![ClientSystemParameter::KeyboardCues(true)])
        .unwrap();

    assert_eq!(
        sent_system_parameters(first.into()),
        [ClientSystemParameter::KeyboardCues(true)]
    );
    assert!(sent_system_parameters(repeated.into()).is_empty());
}

// oracle: mutant parameters sent before the handshake, or lost before initialization
#[test]
fn system_parameter_updates_before_the_handshake_are_sent_at_initialization() {
    let mut client = RailClient::new(1, 800, 600);

    let early: Vec<_> = client
        .update_system_parameters(vec![ClientSystemParameter::KeyboardCues(true)])
        .unwrap()
        .into();
    let sent = sent_system_parameters(handshake(&mut client, None));

    assert!(early.is_empty());
    assert!(sent.contains(&ClientSystemParameter::KeyboardCues(true)));
    assert!(!sent.contains(&ClientSystemParameter::KeyboardCues(false)));
}

/// The number of RemoteApp launches `RailClient` keeps pending.
const MAX_PENDING_EXECUTES: usize = 64;

fn execute() -> ExecutePdu {
    ExecutePdu {
        flags: 0,
        executable: "app".to_owned(),
        working_directory: String::new(),
        arguments: String::new(),
    }
}

// oracle: mutant pending launches unbounded, or queued launches lost at synchronization
#[test]
fn queued_executes_do_not_exceed_pending_limit() {
    let mut client = RailClient::new(1, 1, 1);
    for _ in 0..MAX_PENDING_EXECUTES {
        client.queue_execute(execute()).unwrap();
    }

    let over_limit = client.queue_execute(execute());
    handshake(&mut client, None);
    let messages: Vec<_> = client.complete_desktop_synchronization().unwrap().into();

    assert!(over_limit.is_err());
    assert_eq!(messages.len(), MAX_PENDING_EXECUTES);
}

const WINDOW_RECTANGLE: Rectangle = Rectangle {
    left: 10,
    top: 20,
    right: 410,
    bottom: 320,
};

const WINDOW_MOVE: WindowMovePdu = WindowMovePdu {
    window_id: 7,
    rectangle: WINDOW_RECTANGLE,
};

const SNAP_ARRANGE: SnapArrangePdu = SnapArrangePdu {
    window_id: 7,
    rectangle: WINDOW_RECTANGLE,
};

const CLOAK: CloakPdu = CloakPdu {
    window_id: 7,
    cloaked: true,
};

// oracle: spec MS-RDPERP §2.2.2.7.4 (Client Window Move PDU) and §2.2.2.12.1 (Window Cloak State Change PDU)
#[rstest]
#[case::window_move(RailInputEvent::WindowMove(WINDOW_MOVE), RailPdu::WindowMove(WINDOW_MOVE))]
#[case::cloak(RailInputEvent::Cloak(CLOAK), RailPdu::Cloak(CLOAK))]
fn input_events_are_sent_as_their_pdu(#[case] event: RailInputEvent, #[case] expected: RailPdu) {
    let mut client = established_client(None);

    let sent = sent_pdus(client.queue_input(event).unwrap().into());

    assert_eq!(sent, [expected]);
}

// oracle: spec MS-RDPERP §3.2.5.2.7.5 (send the Window Snap PDU when the server advertises
// SNAP_ARRANGE_SUPPORTED, the Window Move PDU otherwise)
#[rstest]
#[case::server_supports_snap(Some(HandshakeExPdu::SNAP_ARRANGE), RailPdu::SnapArrange(SNAP_ARRANGE))]
#[case::server_without_snap(Some(HandshakeExPdu::EXTENDED_SPI), RailPdu::WindowMove(WINDOW_MOVE))]
#[case::plain_handshake(None, RailPdu::WindowMove(WINDOW_MOVE))]
fn snap_arrange_follows_server_support(#[case] handshake_ex_flags: Option<u32>, #[case] expected: RailPdu) {
    let mut client = established_client(handshake_ex_flags);

    let messages = client.queue_input(RailInputEvent::SnapArrange(SNAP_ARRANGE)).unwrap();

    assert_eq!(sent_pdus(messages.into()), [expected]);
}

// oracle: mutant server move/size and taskbar controls dropped instead of reaching the host
#[test]
fn server_window_controls_reach_the_host() {
    let mut client = established_client(None);
    let min_max = MinMaxInfoPdu {
        window_id: 7,
        values: [800, 600, 0, 0, 100, 50, 800, 600],
    };
    let move_size = LocalMoveSizePdu {
        window_id: 7,
        is_start: true,
        move_size_type: MoveSizeType::Move,
        x: 10,
        y: 4,
    };
    let taskbar = TaskbarInfoPdu {
        message: TaskbarMessage::Register,
        window_id_tab: 7,
        body: 0,
    };

    for pdu in [
        RailPdu::MinMaxInfo(min_max),
        RailPdu::LocalMoveSize(move_size),
        RailPdu::TaskbarInfo(taskbar),
    ] {
        client.process(&encode_vec(&pdu).unwrap()).unwrap();
    }

    assert_eq!(
        client.take_events(),
        [
            RailEvent::Control(RailControlEvent::MinMaxInfo(min_max)),
            RailEvent::Control(RailControlEvent::LocalMoveSize(move_size)),
            RailEvent::Control(RailControlEvent::TaskbarInfo(taskbar)),
        ]
    );
}
