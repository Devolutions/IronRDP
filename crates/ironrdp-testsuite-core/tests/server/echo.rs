//! Coverage for [`EchoServerHandle::is_channel_open`], driving a `DrdynvcServer`
//! with the ECHO bridge through the client's PDUs.

use ironrdp_dvc::DrdynvcServer;
use ironrdp_dvc::pdu::{
    CapabilitiesResponsePdu, CapsVersion, ClosePdu, CreateResponsePdu, CreationStatus, DrdynvcClientPdu,
};
use ironrdp_server::{EchoDvcBridge, EchoServerHandle, RdpServer};
use ironrdp_svc::SvcProcessor as _;

fn handle() -> EchoServerHandle {
    RdpServer::builder()
        .with_addr(([127, 0, 0, 1], 0))
        .with_no_security()
        .with_no_input()
        .with_no_display()
        .build()
        .echo_handle()
        .clone()
}

fn client_sends(server: &mut DrdynvcServer, pdu: DrdynvcClientPdu) {
    server.process(&ironrdp_core::encode_vec(&pdu).unwrap()).unwrap();
}

/// A DVC server with the ECHO bridge, after the client sent its capabilities, and the ECHO channel id.
fn server(handle: &EchoServerHandle) -> (DrdynvcServer, u32) {
    let mut server = DrdynvcServer::new().with_dynamic_channel(EchoDvcBridge::new(handle.clone()));
    client_sends(
        &mut server,
        DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(CapsVersion::V2)),
    );
    let id = server.get_channel_id_by_type::<EchoDvcBridge>().unwrap();
    (server, id)
}

fn client_answers(server: &mut DrdynvcServer, id: u32, status: CreationStatus) {
    client_sends(server, DrdynvcClientPdu::Create(CreateResponsePdu::new(id, status)));
}

// oracle: spec MS-RDPEDYC §2.2.2.2
// oracle: spec MS-RDPEDYC §2.2.4
#[test]
fn open_after_the_client_accepts_until_it_closes() {
    let handle = handle();
    let (mut server, id) = server(&handle);
    assert!(!handle.is_channel_open(), "Create Request sent, no answer yet");
    client_answers(&mut server, id, CreationStatus::OK);
    assert!(handle.is_channel_open());
    client_sends(&mut server, DrdynvcClientPdu::Close(ClosePdu::new(id)));
    assert!(!handle.is_channel_open(), "the client closed the channel");
}

// oracle: spec MS-RDPEDYC §2.2.2.2
// oracle: regression #2339
#[test]
fn not_open_when_the_client_declines() {
    let handle = handle();
    let (mut server, id) = server(&handle);
    client_answers(&mut server, id, CreationStatus::NO_LISTENER);
    assert!(!handle.is_channel_open());
}

// oracle: mutant the open state kept after the DVC server of the connection is dropped
#[test]
fn not_open_after_the_connection_ends() {
    let handle = handle();
    let (mut server, id) = server(&handle);
    client_answers(&mut server, id, CreationStatus::OK);
    drop(server);
    assert!(!handle.is_channel_open());
}
