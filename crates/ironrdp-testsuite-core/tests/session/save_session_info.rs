use ironrdp_core::encode_vec;
use ironrdp_pdu::gcc::{Monitor, MonitorFlags};
use ironrdp_pdu::mcs::SendDataIndication;
use ironrdp_pdu::rdp::client_info::CompressionType;
use ironrdp_pdu::rdp::finalization_messages::MonitorLayoutPdu;
use ironrdp_pdu::rdp::headers::{
    CompressionFlags, ShareControlHeader, ShareControlPdu, ShareDataHeader, ShareDataPdu, StreamPriority,
};
use ironrdp_pdu::rdp::session_info::{
    InfoData, InfoType, LogonExFlags, LogonInfoExtended, SaveSessionInfoPdu, ServerAutoReconnect,
};
use ironrdp_pdu::x224::X224;
use ironrdp_session::x224::{Processor, ProcessorOutput};
use ironrdp_svc::StaticChannelSet;

const USER_CHANNEL_ID: u16 = 1002;
const IO_CHANNEL_ID: u16 = 1003;

fn make_processor() -> Processor {
    Processor::new(StaticChannelSet::new(), USER_CHANNEL_ID, IO_CHANNEL_ID, None, 0)
}

/// Frame a Save Session Info PDU the way a server does: Share Data inside a
/// Share Control header, carried in an MCS Send Data Indication on the IO
/// channel.
fn encode_save_session_info(auto_reconnect: Option<ServerAutoReconnect>) -> Vec<u8> {
    let present_fields_flags = if auto_reconnect.is_some() {
        LogonExFlags::AUTO_RECONNECT_COOKIE
    } else {
        LogonExFlags::empty()
    };

    let pdu = ShareDataPdu::SaveSessionInfo(SaveSessionInfoPdu {
        info_type: InfoType::LogonExtended,
        info_data: InfoData::LogonExtended(LogonInfoExtended {
            present_fields_flags,
            auto_reconnect,
            errors_info: None,
        }),
    });

    let control = ShareControlHeader {
        share_id: 0,
        pdu_source: USER_CHANNEL_ID,
        share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
            share_data_pdu: pdu,
            stream_priority: StreamPriority::Medium,
            compression_flags: CompressionFlags::empty(),
            compression_type: CompressionType::K8,
        }),
    };

    let indication = SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id: IO_CHANNEL_ID,
        user_data: encode_vec(&control).unwrap().into(),
    };

    encode_vec(&X224(indication)).unwrap()
}

fn encode_monitor_layout(monitors: Vec<Monitor>) -> Vec<u8> {
    let control = ShareControlHeader {
        share_id: 0,
        pdu_source: USER_CHANNEL_ID,
        share_control_pdu: ShareControlPdu::Data(ShareDataHeader {
            share_data_pdu: ShareDataPdu::MonitorLayout(MonitorLayoutPdu { monitors }),
            stream_priority: StreamPriority::Medium,
            compression_flags: CompressionFlags::empty(),
            compression_type: CompressionType::K8,
        }),
    };

    let indication = SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id: IO_CHANNEL_ID,
        user_data: encode_vec(&control).unwrap().into(),
    };

    encode_vec(&X224(indication)).unwrap()
}

/// The cookie has to reach the consumer: it is the only thing that lets a later
/// connection resume the session, and nothing else in the session layer keeps it.
#[test]
fn save_session_info_surfaces_the_auto_reconnect_cookie() {
    let cookie = ServerAutoReconnect {
        logon_id: 0x2A,
        random_bits: [0xC3; 16],
    };

    let mut bulk_decompressor = None;
    let outputs = make_processor()
        .process(&encode_save_session_info(Some(cookie.clone())), &mut bulk_decompressor)
        .unwrap();

    let surfaced = outputs
        .iter()
        .find_map(|output| match output {
            ProcessorOutput::AutoReconnectCookie(cookie) => Some(cookie),
            _ => None,
        })
        .expect("the cookie must be surfaced");

    assert_eq!(surfaced.logon_id, cookie.logon_id);
    assert_eq!(surfaced.random_bits, cookie.random_bits);

    // The logon status comes out of the same PDU and must survive alongside it:
    // reporting the cookie is not a reason to swallow the notification.
    assert!(
        outputs
            .iter()
            .any(|output| matches!(output, ProcessorOutput::SaveSessionInfo { .. })),
        "surfacing the cookie must not suppress the logon notification"
    );
}

/// A Save Session Info PDU without a cookie yields the logon notification alone.
#[test]
fn save_session_info_without_a_cookie_surfaces_no_cookie() {
    let mut bulk_decompressor = None;
    let outputs = make_processor()
        .process(&encode_save_session_info(None), &mut bulk_decompressor)
        .unwrap();

    assert!(
        !outputs
            .iter()
            .any(|output| matches!(output, ProcessorOutput::AutoReconnectCookie(_))),
        "no cookie was sent, so none may be surfaced"
    );
}

#[test]
fn monitor_layout_is_forwarded_from_the_active_io_channel() {
    let monitors = vec![Monitor {
        left: 0,
        top: 0,
        right: 799,
        bottom: 599,
        flags: MonitorFlags::PRIMARY,
    }];
    let mut bulk_decompressor = None;
    let outputs = make_processor()
        .process(&encode_monitor_layout(monitors.clone()), &mut bulk_decompressor)
        .expect("the monitor layout PDU should be processed");

    let [ProcessorOutput::MonitorLayout(actual)] = outputs.as_slice() else {
        panic!("expected a monitor layout output");
    };
    assert_eq!(actual, &monitors);

    let output = ironrdp_session::ActiveStageOutput::try_from(ProcessorOutput::MonitorLayout(actual.clone()))
        .expect("the monitor layout should be forwarded through the active stage");
    let ironrdp_session::ActiveStageOutput::MonitorLayout(actual) = output else {
        panic!("expected an active-stage monitor layout output");
    };
    assert_eq!(actual, monitors);
}
