use ironrdp_client::rail::{RailClient, RailEvent};
use ironrdp_core::{decode, encode_vec};
use ironrdp_rail::pdu::{
    ClientStatusPdu, ClientSystemParameter, ClientSystemParametersPdu, ExecutePdu, HandshakePdu, RailPdu,
    SystemParameterRectangle,
};
use ironrdp_svc::SvcProcessor as _;

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
