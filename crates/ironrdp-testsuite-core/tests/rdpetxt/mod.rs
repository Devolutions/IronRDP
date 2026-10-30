use std::sync::{Arc, Mutex};

use ironrdp_core::{decode, encode_vec};
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_dvc::pdu::{DrdynvcClientPdu, DrdynvcDataPdu};
use ironrdp_rdpetxt::client::{self, TextInputClient, TextInputConfig};
use ironrdp_rdpetxt::pdu::{
    AcknowledgeOperationPdu, AcknowledgeRemoteOperationPdu, AcknowledgeUndoPendingKeyEventsPdu, AcknowledgementType,
    CandidateList, CharacterEventPdu, ClientOptions, ClientPdu, CompositionTerminatedPdu,
    ControlConversionModeUpdatedPdu, CoreInputProfile, CoreInputViewOcclusion, DoReconversionPdu, EditControlFocusPdu,
    EditControlInfo, EditControlRange, FocusDepartCompletedPdu, FocusNavigatingEventPdu, GeometryChangedPdu, Guid,
    HostFocusPdu, HotKeyRegistrationData, HotKeyRegistrationPdu, KeyEventHostInfo, KeyEventPayloadPdu,
    NavigateFocusCompleteInfo, NavigateFocusInfo, NotifyClientVersionPdu, NotifyServerVersionPdu, OccludingViewsPdu,
    OcclusionList, ReconversionCandidatesPdu, RefreshClientPdu, RegisterRemoteKeyTargetPdu, ReportClientOptionsPdu,
    SelectionChangedPdu, ServerPdu, TextChangedPdu, TextFormat, TextInputHostSettings, TextInputRect, TextString,
    UndoPendingKeyEventsPdu, UpdateFormatPdu, UpdateInputProfilePdu, UpdateTextAndSelectionPdu, UpdateTextPdu,
};
use ironrdp_rdpetxt::server::{self, TextInputServer, TextInputServerHandler};
use proptest::prelude::*;
use rstest::rstest;

const SERVER_TO_CLIENT_ID: u32 = 3;
const CLIENT_TO_SERVER_ID: u32 = 4;

fn bytes(fixture: &str) -> Vec<u8> {
    hex::decode(fixture.replace(' ', "")).unwrap()
}

fn text(units: &[u16]) -> TextString {
    TextString::from_wire_units(units.to_vec())
}

/// Client processors sharing one client, both started.
fn client_channels(config: TextInputConfig) -> (client::ServerToClientChannel, client::ClientToServerChannel) {
    let client = TextInputClient::new(config).into_shared();
    let mut server_to_client = client::ServerToClientChannel::new(Arc::clone(&client));
    let mut client_to_server = client::ClientToServerChannel::new(client);
    server_to_client.start(SERVER_TO_CLIENT_ID).unwrap();
    client_to_server.start(CLIENT_TO_SERVER_ID).unwrap();
    (server_to_client, client_to_server)
}

fn receive(server_to_client: &mut client::ServerToClientChannel, pdus: &[ServerPdu]) {
    let payload: Vec<u8> = pdus.iter().flat_map(|pdu| encode_vec(pdu).unwrap()).collect();
    let replies = server_to_client.process(SERVER_TO_CLIENT_ID, &payload).unwrap();
    assert!(replies.is_empty(), "replies leave on the client-to-server channel");
}

/// PDUs the client queued, which must be addressed to the client-to-server channel.
fn sent(client_to_server: &mut client::ClientToServerChannel) -> Vec<ClientPdu> {
    client_to_server
        .pending_messages()
        .unwrap()
        .iter()
        .map(|message| {
            let encoded = message.encode_unframed_pdu().unwrap();
            let DrdynvcClientPdu::Data(data) = decode(&encoded).unwrap() else {
                panic!("expected DRDYNVC data");
            };
            // oracle: spec MS-RDPETXT §2.1
            assert_eq!(data.channel_id(), CLIENT_TO_SERVER_ID);
            let DrdynvcDataPdu::Data(data) = data else {
                panic!("expected unsplit DRDYNVC data");
            };
            decode(data.data()).unwrap()
        })
        .collect()
}

fn edit_control_focus(text_input_client_id: u32, edit_control_id: u32, gaining_focus: bool) -> ServerPdu {
    ServerPdu::EditControlFocus(EditControlFocusPdu {
        text_input_client_id,
        control_bounds: TextInputRect::default(),
        edit_info: EditControlInfo {
            id: edit_control_id,
            ..EditControlInfo::default()
        },
        gaining_focus,
        losing_focus_control_id: 0,
        losing_focus_text_input_host_id: 0,
        override_: false,
    })
}

fn register_key_target(object_id: u32, text_target_id: u32) -> ServerPdu {
    ServerPdu::RegisterRemoteKeyTarget(RegisterRemoteKeyTargetPdu {
        object_id,
        text_target_id,
        text_input_host_settings: TextInputHostSettings::default(),
        view_instance_id: 0,
        window_instance_id: 0,
    })
}

fn host_focus(text_input_host_id: u32, ordinal: u32, gaining_focus: bool) -> ServerPdu {
    ServerPdu::HostFocus(HostFocusPdu {
        text_input_host_id,
        ordinal,
        gaining_focus,
        override_: false,
    })
}

fn selection_changed(text_input_client_id: u32, edit_control_id: u32, begin: i32, end: i32) -> ServerPdu {
    ServerPdu::SelectionChanged(SelectionChangedPdu {
        text_input_client_id,
        edit_control_id,
        selection_begin: begin,
        selection_end: end,
        override_: false,
        origin_key: KeyEventHostInfo::default(),
    })
}

fn text_changed(text_input_client_id: u32, edit_control_id: u32, operation_id: u32) -> ServerPdu {
    ServerPdu::TextChanged(TextChangedPdu {
        text_input_client_id,
        edit_control_id,
        replaced_text_range: EditControlRange::default(),
        new_selection_range: EditControlRange::default(),
        operation_id,
        text_length: 0,
        origin_key: KeyEventHostInfo::default(),
        override_: false,
        no_conflict: false,
        offset1: 0,
        updated_text_region1: text(&[]),
        offset2: -1,
        updated_text_region2: text(&[]),
        key_states: Vec::new(),
    })
}

// oracle: spec MS-RDPETXT §2.2.2, §2.2.2.53
#[test]
fn header_size_counts_the_pdu_identifier_and_body() {
    let encoded = bytes("0A000000 0406 01000000 02000000");
    let pdu = ClientPdu::NotifyClientVersion(NotifyClientVersionPdu {
        version_major: 1,
        version_minor: 2,
    });

    assert_eq!(encode_vec(&pdu).unwrap(), encoded);
    assert_eq!(decode::<ClientPdu>(&encoded).unwrap(), pdu);
}

// oracle: spec MS-RDPETXT §2.2.2.10, §2.2.1.8
#[test]
fn update_text_and_selection_counts_text_in_utf16_code_units() {
    let pdu = ClientPdu::UpdateTextAndSelection(UpdateTextAndSelectionPdu {
        text_input_client_id: 1,
        edit_control_id: 2,
        text_input_host_id: 3,
        operation_id: 4,
        replace_begin: 5,
        replace_end: 7,
        new_text: TextString::new("a\u{1F600}"),
        selection_begin: 8,
        selection_end: 8,
    });
    let encoded = bytes(
        "2C000000 0102 \
         01000000 02000000 03000000 04000000 \
         05000000 07000000 \
         03000000 6100 3DD8 00DE \
         08000000 08000000",
    );

    assert_eq!(encode_vec(&pdu).unwrap(), encoded);
}

// oracle: spec MS-RDPETXT §2.2.2.36, §2.2.1.1, §2.2.1.8
#[test]
fn text_changed_decodes_every_field_in_order() {
    let encoded = bytes(
        "6A000000 0C03 \
         07000000 02000000 \
         00000000 00000000 02000000 02000000 \
         09000000 02000000 \
         0100 0100 00000000 4800 6800 0000 0000000000000000 0000 2300 00 00 00 00 00000000 00000000 000000000000 \
         00 00 \
         00000000 02000000 68006900 \
         FFFFFFFF 00000000 \
         02000000 8001",
    );

    let expected = ServerPdu::TextChanged(TextChangedPdu {
        text_input_client_id: 7,
        edit_control_id: 2,
        replaced_text_range: EditControlRange { begin: 0, end: 0 },
        new_selection_range: EditControlRange { begin: 2, end: 2 },
        operation_id: 9,
        text_length: 2,
        origin_key: KeyEventHostInfo {
            modifier_flags: 0x0001,
            event_flags: 0x0001,
            virtual_key: 0x48,
            character: u16::from(b'h'),
            scan_code: 0x23,
            ..KeyEventHostInfo::default()
        },
        override_: false,
        no_conflict: false,
        offset1: 0,
        updated_text_region1: TextString::new("hi"),
        offset2: -1,
        updated_text_region2: text(&[]),
        key_states: vec![0x80, 0x01],
    });
    assert_eq!(decode::<ServerPdu>(&encoded).unwrap(), expected);
}

// oracle: spec MS-RDPETXT §2.2.2.15
#[test]
fn reconversion_candidate_list_size_is_in_bytes() {
    let pdu = ClientPdu::ReconversionCandidates(ReconversionCandidatesPdu {
        text_input_client_id: 1,
        candidate_list: CandidateList(vec![TextString::new("a"), TextString::new("bc")]),
        edit_control_id: 2,
        reconvert_range: EditControlRange { begin: 0, end: 1 },
    });
    let encoded = bytes(
        "24000000 0602 \
         01000000 \
         0E000000 01000000 6100 02000000 62006300 \
         02000000 \
         00000000 01000000",
    );

    assert_eq!(encode_vec(&pdu).unwrap(), encoded);
    assert_eq!(decode::<ClientPdu>(&encoded).unwrap(), pdu);
}

// oracle: spec MS-RDPETXT §2.2.2.52, §2.2.1.7
#[test]
fn occluding_views_size_is_in_bytes_of_24_byte_entries() {
    let pdu = ClientPdu::OccludingViews(OccludingViewsPdu {
        core_input_view_id: 5,
        occluding_views: OcclusionList(vec![CoreInputViewOcclusion {
            event_id: 1,
            occluding_rect: TextInputRect {
                left: 10,
                top: 20,
                right: 30,
                bottom: 40,
            },
            occlusion_kind: 1,
        }]),
        precalculated_occlusion: true,
    });
    let encoded = bytes(
        "23000000 0004 \
         05000000 \
         18000000 01000000 0A000000 14000000 1E000000 28000000 01000000 \
         01",
    );

    assert_eq!(encode_vec(&pdu).unwrap(), encoded);
    assert_eq!(decode::<ClientPdu>(&encoded).unwrap(), pdu);
}

// oracle: spec MS-RDPETXT §2.2.2 (fixed structure sizes stated per field)
#[rstest]
#[case::character_event_with_44_byte_key_info(
    ClientPdu::CharacterEvent(CharacterEventPdu {
        text_input_host_id: 0,
        key_event_id: 0,
        edit_control_id: 0,
        key_down_event_id: 0,
        key_event_info: KeyEventHostInfo::default(),
    }),
    6 + 16 + 44,
)]
#[case::focus_navigating_with_52_byte_info(
    ClientPdu::FocusNavigatingEvent(FocusNavigatingEventPdu {
        text_input_host_id: 0,
        navigate_focus_info: NavigateFocusInfo::default(),
    }),
    6 + 4 + 52,
)]
#[case::focus_depart_completed_with_17_byte_info(
    ClientPdu::FocusDepartCompleted(FocusDepartCompletedPdu {
        text_input_host_id: 0,
        navigate_focus_complete_info: NavigateFocusCompleteInfo::default(),
    }),
    6 + 4 + 17,
)]
#[case::key_event_payload(
    ClientPdu::KeyEventPayload(KeyEventPayloadPdu {
        text_input_host_id: 0,
        edit_control_id: 0,
        key_event_id: 0,
        remote_key_event_id: 0,
        key_event_info: KeyEventHostInfo::default(),
        begin_payload: false,
        handled: false,
    }),
    6 + 12 + 1 + 44 + 1 + 1,
)]
#[case::update_format_with_12_byte_format(
    ClientPdu::UpdateFormat(UpdateFormatPdu {
        text_input_client_id: 0,
        edit_control_id: 0,
        text_input_host_id: 0,
        operation_id: 0,
        format_begin: 0,
        format_end: 0,
        format: TextFormat::default(),
        format_basic: 0,
    }),
    6 + 24 + 12 + 4,
)]
#[case::update_input_profile_with_82_byte_profile(
    ClientPdu::UpdateInputProfile(UpdateInputProfilePdu {
        text_input_client_id: 0,
        profile: CoreInputProfile::default(),
        initializing: false,
    }),
    6 + 4 + 82 + 1,
)]
fn client_pdu_has_the_specified_size(#[case] pdu: ClientPdu, #[case] size: usize) {
    let encoded = encode_vec(&pdu).unwrap();
    assert_eq!(encoded.len(), size);
    assert_eq!(decode::<ClientPdu>(&encoded).unwrap(), pdu);
}

// oracle: spec MS-RDPETXT §2.2.2 (fixed structure sizes stated per field)
#[rstest]
#[case::edit_control_focus_with_36_byte_info(edit_control_focus(0, 0, true), 6 + 4 + 16 + 36 + 1 + 4 + 4 + 1)]
#[case::register_key_target_with_7_byte_settings(register_key_target(0, 0), 6 + 4 + 4 + 7 + 8 + 8)]
#[case::selection_changed_with_44_byte_origin_key(selection_changed(0, 0, 0, 0), 6 + 16 + 1 + 44)]
#[case::geometry_changed(
    ServerPdu::GeometryChanged(GeometryChangedPdu {
        text_input_client_id: 0,
        edit_control_id: 0,
        control_bounds: TextInputRect::default(),
        range: EditControlRange::default(),
        range_bounds: TextInputRect::default(),
    }),
    6 + 8 + 16 + 8 + 16,
)]
#[case::hotkey_registration_with_12_byte_data(
    ServerPdu::HotKeyRegistration(HotKeyRegistrationPdu {
        registration: HotKeyRegistrationData::default(),
        enabled: true,
    }),
    6 + 12 + 1,
)]
#[case::refresh_client_without_fields(ServerPdu::RefreshClient(RefreshClientPdu {}), 6)]
fn server_pdu_has_the_specified_size(#[case] pdu: ServerPdu, #[case] size: usize) {
    let encoded = encode_vec(&pdu).unwrap();
    assert_eq!(encoded.len(), size);
    assert_eq!(decode::<ServerPdu>(&encoded).unwrap(), pdu);
}

// oracle: spec MS-RDPETXT §1.7, §2.2.2.43
#[test]
fn decoding_ignores_fields_appended_by_later_versions() {
    let encoded = bytes(
        "1E000000 1A03 \
         00000000000000000000000000000000 01000000 03000000 \
         AABBCCDD",
    );

    assert_eq!(
        decode::<ServerPdu>(&encoded).unwrap(),
        ServerPdu::NotifyServerVersion(NotifyServerVersionPdu {
            container_id: Guid::default(),
            version_major: 1,
            version_minor: 3,
        })
    );
}

// oracle: spec MS-RDPETXT §2.2.2
#[test]
fn pdu_identifier_of_the_other_direction_is_rejected() {
    // RDPTXT_NOTIFY_CLIENT_VERSION_PDU travels from client to server only.
    let encoded = bytes("0A000000 0406 01000000 02000000");

    assert!(decode::<ServerPdu>(&encoded).is_err());
}

fn utf16_units() -> impl Strategy<Value = Vec<u16>> {
    // A trailing NUL is not part of the text, so it does not survive a round trip.
    proptest::collection::vec(any::<u16>(), 0..32).prop_filter("no trailing NUL", |units| units.last() != Some(&0))
}

proptest! {
    // oracle: property round-trip
    #[test]
    fn update_text_round_trips(
        ids in any::<(u32, u32, u32, u32)>(),
        replace_begin in any::<u32>(),
        replace_end in any::<i32>(),
        new_text in utf16_units(),
    ) {
        let pdu = ClientPdu::UpdateText(UpdateTextPdu {
            text_input_client_id: ids.0,
            edit_control_id: ids.1,
            text_input_host_id: ids.2,
            operation_id: ids.3,
            replace_begin,
            replace_end,
            new_text: text(&new_text),
        });
        prop_assert_eq!(decode::<ClientPdu>(&encode_vec(&pdu).unwrap()).unwrap(), pdu);
    }

    // oracle: property round-trip
    #[test]
    fn text_changed_round_trips(
        region1 in utf16_units(),
        region2 in utf16_units(),
        key_states in proptest::collection::vec(any::<u8>(), 0..300),
        offsets in any::<(i32, i32)>(),
        flags in any::<(bool, bool)>(),
    ) {
        let ServerPdu::TextChanged(mut change) = text_changed(1, 2, 3) else { unreachable!() };
        change.updated_text_region1 = text(&region1);
        change.updated_text_region2 = text(&region2);
        change.key_states = key_states;
        (change.offset1, change.offset2) = offsets;
        (change.override_, change.no_conflict) = flags;
        let pdu = ServerPdu::TextChanged(change);
        prop_assert_eq!(decode::<ServerPdu>(&encode_vec(&pdu).unwrap()).unwrap(), pdu);
    }
}

// oracle: spec MS-RDPETXT §1.7, §2.2.2.53, §2.2.2.54
#[test]
fn client_answers_the_server_version_once_its_channel_opens() {
    let config = TextInputConfig {
        version_major: 7,
        version_minor: 0x10,
        options: ClientOptions::ENABLE_PREDICTED_KEY_REPORTING,
    };
    let client = TextInputClient::new(config).into_shared();
    let mut server_to_client = client::ServerToClientChannel::new(Arc::clone(&client));
    let mut client_to_server = client::ClientToServerChannel::new(client);
    server_to_client.start(SERVER_TO_CLIENT_ID).unwrap();
    receive(
        &mut server_to_client,
        &[ServerPdu::NotifyServerVersion(NotifyServerVersionPdu {
            container_id: Guid::default(),
            version_major: 1,
            version_minor: 3,
        })],
    );

    let replies: Vec<ClientPdu> = client_to_server
        .start(CLIENT_TO_SERVER_ID)
        .unwrap()
        .iter()
        .map(|message| decode(&encode_vec(message.as_ref()).unwrap()).unwrap())
        .collect();

    assert_eq!(
        replies,
        [
            ClientPdu::NotifyClientVersion(NotifyClientVersionPdu {
                version_major: 7,
                version_minor: 0x10,
            }),
            ClientPdu::ReportClientOptions(ReportClientOptionsPdu {
                options: ClientOptions::ENABLE_PREDICTED_KEY_REPORTING,
            }),
        ]
    );
    let locked = server_to_client.client().lock().unwrap();
    assert_eq!(locked.server_version(), Some((1, 3)));
}

#[rstest]
// oracle: spec MS-RDPETXT §3.1.5.2.3
#[case::selection_change(selection_changed(7, 2, 1, 1), AcknowledgementType::SELECTION_CHANGE)]
// oracle: spec MS-RDPETXT §3.1.5.2.4
#[case::composition_terminated(
    ServerPdu::CompositionTerminated(CompositionTerminatedPdu {
        text_input_client_id: 7,
        edit_control_id: 2,
        override_: false,
    }),
    AcknowledgementType::COMPOSITION_TERMINATED,
)]
// oracle: spec MS-RDPETXT §3.1.5.2.5
#[case::conversion_mode_updated(
    ServerPdu::ControlConversionModeUpdated(ControlConversionModeUpdatedPdu {
        text_input_client_id: 7,
        edit_control_id: 2,
        conversion_mode: 0x0000_0009,
        profile_id: Guid::default(),
    }),
    AcknowledgementType::CONVERSION_MODE_CHANGED,
)]
fn client_acknowledges_edit_control_notification(
    #[case] notification: ServerPdu,
    #[case] acknowledgement_type: AcknowledgementType,
) {
    let (mut server_to_client, mut client_to_server) = client_channels(TextInputConfig::default());

    receive(&mut server_to_client, &[notification]);

    let [ClientPdu::AcknowledgeOperation(ack)] = sent(&mut client_to_server)[..] else {
        panic!("expected one RDPTXT_ACKNOWLEDGE_OPERATION_PDU");
    };
    assert_eq!(
        (ack.text_input_client_id, ack.edit_control_id, ack.acknowledgement_type),
        (7, 2, acknowledgement_type)
    );
}

// oracle: spec MS-RDPETXT §3.1.5.2.2, §2.2.2.36
#[test]
fn client_acknowledges_text_change_with_its_operation_identifier() {
    let (mut server_to_client, mut client_to_server) = client_channels(TextInputConfig::default());

    receive(&mut server_to_client, &[text_changed(7, 2, 41)]);

    assert_eq!(
        sent(&mut client_to_server),
        [ClientPdu::AcknowledgeOperation(AcknowledgeOperationPdu {
            text_input_client_id: 7,
            edit_control_id: 2,
            acknowledgement_type: AcknowledgementType::TEXT_CHANGE,
            operation_id: 41,
        })]
    );
}

// oracle: spec MS-RDPETXT §3.1.5.2.1
#[test]
fn client_acknowledges_focus_loss_then_its_completion() {
    let (mut server_to_client, mut client_to_server) = client_channels(TextInputConfig::default());

    receive(&mut server_to_client, &[edit_control_focus(7, 2, false)]);

    let types: Vec<_> = sent(&mut client_to_server)
        .into_iter()
        .map(|pdu| match pdu {
            ClientPdu::AcknowledgeOperation(ack) => {
                assert_eq!((ack.text_input_client_id, ack.edit_control_id), (7, 2));
                ack.acknowledgement_type
            }
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(
        types,
        [
            AcknowledgementType::FOCUS_LOSS,
            AcknowledgementType::FOCUS_LEAVE_COMPLETED
        ]
    );
}

// oracle: spec MS-RDPETXT §3.1.5.2.6
#[test]
fn client_acknowledges_host_focus_loss_only() {
    let (mut server_to_client, mut client_to_server) = client_channels(TextInputConfig::default());

    receive(
        &mut server_to_client,
        &[host_focus(5, 0, true), host_focus(5, 0, false)],
    );

    let [ClientPdu::AcknowledgeHostOperation(ack)] = sent(&mut client_to_server)[..] else {
        panic!("expected one RDPTXT_ACKNOWLEDGE_HOST_OPERATION_PDU");
    };
    assert_eq!(
        (ack.text_input_host_id, ack.acknowledgement_type),
        (5, AcknowledgementType::FOCUS_LOSS)
    );
}

// oracle: spec MS-RDPETXT §3.1.5.2.7
#[test]
fn client_acknowledges_undo_request_with_its_identifier() {
    let (mut server_to_client, mut client_to_server) = client_channels(TextInputConfig::default());

    receive(
        &mut server_to_client,
        &[ServerPdu::UndoPendingKeyEvents(UndoPendingKeyEventsPdu {
            undo_request_id: 0x1234,
        })],
    );

    assert_eq!(
        sent(&mut client_to_server),
        [ClientPdu::AcknowledgeUndoPendingKeyEvents(
            AcknowledgeUndoPendingKeyEventsPdu {
                undo_request_id: 0x1234
            }
        )]
    );
}

// oracle: spec MS-RDPETXT §3.1.5.1
#[test]
fn reconversion_request_is_answered_for_the_requesting_control() {
    let (mut server_to_client, mut client_to_server) = client_channels(TextInputConfig::default());

    receive(
        &mut server_to_client,
        &[ServerPdu::DoReconversion(DoReconversionPdu {
            text_input_client_id: 7,
            edit_control_id: 2,
            return_candidates: false,
            return_range: true,
        })],
    );

    let [ClientPdu::ReconversionCandidates(ref reply)] = sent(&mut client_to_server)[..] else {
        panic!("expected one RDPTXT_RECONVERSION_CANDIDATES_PDU");
    };
    assert_eq!((reply.text_input_client_id, reply.edit_control_id), (7, 2));
}

// oracle: spec MS-RDPETXT §2.2.2 (each PDU carries its own size)
#[test]
fn every_pdu_of_a_message_is_processed() {
    let (mut server_to_client, mut client_to_server) = client_channels(TextInputConfig::default());

    receive(&mut server_to_client, &[text_changed(7, 2, 1), text_changed(7, 2, 2)]);

    let operation_ids: Vec<_> = sent(&mut client_to_server)
        .into_iter()
        .map(|pdu| match pdu {
            ClientPdu::AcknowledgeOperation(ack) => ack.operation_id,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(operation_ids, [1, 2]);
}

// oracle: spec MS-RDPETXT §2.2.2.10, §2.2.2.35, §2.2.1.8
#[test]
fn committed_text_replaces_the_selection_and_moves_the_caret_after_it() {
    let (mut server_to_client, mut client_to_server) = client_channels(TextInputConfig::default());
    receive(
        &mut server_to_client,
        &[
            register_key_target(5, 7),
            host_focus(5, 0, true),
            edit_control_focus(7, 2, true),
            selection_changed(7, 2, 3, 1),
        ],
    );
    sent(&mut client_to_server);

    let committed = server_to_client.client().lock().unwrap().commit_text("\u{1F600}");

    assert!(committed.is_some());
    let [ClientPdu::UpdateTextAndSelection(ref update)] = sent(&mut client_to_server)[..] else {
        panic!("expected one RDPTXT_UPDATE_TEXT_AND_SELECTION_PDU");
    };
    assert_eq!(
        (
            update.text_input_client_id,
            update.edit_control_id,
            update.text_input_host_id
        ),
        (7, 2, 5)
    );
    assert_eq!((update.replace_begin, update.replace_end), (1, 3));
    assert_eq!(update.new_text, TextString::new("\u{1F600}"));
    // The emoji takes two UTF-16 code units.
    assert_eq!((update.selection_begin, update.selection_end), (3, 3));
}

// oracle: spec MS-RDPETXT §2.2.2.33
#[test]
fn stale_host_focus_change_is_ignored() {
    let (mut server_to_client, mut client_to_server) = client_channels(TextInputConfig::default());
    receive(
        &mut server_to_client,
        &[
            register_key_target(5, 7),
            register_key_target(6, 7),
            host_focus(6, 3, true),
            host_focus(6, 2, false),
            edit_control_focus(7, 2, true),
        ],
    );
    sent(&mut client_to_server);

    server_to_client.client().lock().unwrap().commit_text("a");

    let [ClientPdu::UpdateTextAndSelection(ref update)] = sent(&mut client_to_server)[..] else {
        panic!("expected one RDPTXT_UPDATE_TEXT_AND_SELECTION_PDU");
    };
    assert_eq!(update.text_input_host_id, 6);
}

struct AcknowledgingHandler {
    received: Arc<Mutex<Vec<ClientPdu>>>,
}

impl TextInputServerHandler for AcknowledgingHandler {
    fn client_pdu(&mut self, pdu: ClientPdu) -> Vec<ServerPdu> {
        let reply = match &pdu {
            ClientPdu::UpdateText(update) => {
                vec![ServerPdu::AcknowledgeRemoteOperation(AcknowledgeRemoteOperationPdu {
                    text_input_client_id: update.text_input_client_id,
                    edit_control_id: update.edit_control_id,
                    operation_id: update.operation_id,
                    error_code: 0,
                })]
            }
            _ => Vec::new(),
        };
        self.received.lock().unwrap().push(pdu);
        reply
    }
}

// oracle: spec MS-RDPETXT §1.7, §2.1, §2.2.2.43
#[test]
fn server_announces_its_version_and_answers_on_the_server_to_client_channel() {
    let received = Arc::new(Mutex::new(Vec::new()));
    let server = TextInputServer::new(Box::new(AcknowledgingHandler {
        received: Arc::clone(&received),
    }))
    .with_version(1, 2)
    .into_shared();
    let mut server_to_client = server::ServerToClientChannel::new(Arc::clone(&server));
    let mut client_to_server = server::ClientToServerChannel::new(server);

    let announced: Vec<ServerPdu> = server_to_client
        .start(SERVER_TO_CLIENT_ID)
        .unwrap()
        .iter()
        .map(|message| decode(&encode_vec(message.as_ref()).unwrap()).unwrap())
        .collect();
    assert_eq!(
        announced,
        [ServerPdu::NotifyServerVersion(NotifyServerVersionPdu {
            container_id: Guid::default(),
            version_major: 1,
            version_minor: 2,
        })]
    );

    client_to_server.start(CLIENT_TO_SERVER_ID).unwrap();
    let update = ClientPdu::UpdateText(UpdateTextPdu {
        text_input_client_id: 7,
        edit_control_id: 2,
        text_input_host_id: 5,
        operation_id: 11,
        replace_begin: 0,
        replace_end: 0,
        new_text: TextString::new("a"),
    });
    assert!(
        client_to_server
            .process(CLIENT_TO_SERVER_ID, &encode_vec(&update).unwrap())
            .unwrap()
            .is_empty()
    );

    assert_eq!(*received.lock().unwrap(), [update]);
    let replies: Vec<ServerPdu> = server_to_client
        .pending_messages()
        .unwrap()
        .iter()
        .map(|message| {
            let encoded = message.encode_unframed_pdu().unwrap();
            let ironrdp_dvc::pdu::DrdynvcServerPdu::Data(data) = decode(&encoded).unwrap() else {
                panic!("expected DRDYNVC data");
            };
            assert_eq!(data.channel_id(), SERVER_TO_CLIENT_ID);
            let DrdynvcDataPdu::Data(data) = data else {
                panic!("expected unsplit DRDYNVC data");
            };
            decode(data.data()).unwrap()
        })
        .collect();
    assert_eq!(
        replies,
        [ServerPdu::AcknowledgeRemoteOperation(AcknowledgeRemoteOperationPdu {
            text_input_client_id: 7,
            edit_control_id: 2,
            operation_id: 11,
            error_code: 0,
        })]
    );
}
