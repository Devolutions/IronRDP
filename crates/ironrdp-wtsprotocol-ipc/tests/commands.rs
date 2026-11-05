use ironrdp_wtsprotocol_ipc::{
    DEFAULT_MAX_FRAME_SIZE, ProviderCommand, read_json_message, write_frame, write_json_message,
};
use serde as _;
use serde_json as _;

// oracle: mutant a companion upgraded before the provider DLL must treat a frame without the flag as a dynamic open.
#[test]
fn open_virtual_channel_without_the_static_flag_is_dynamic() {
    let mut frame = Vec::new();
    write_frame(
        &mut frame,
        br#"{"type":"open_virtual_channel","connection_id":1,"session_id":2,"endpoint_name":"Devolutions::Now::Agent"}"#,
    )
    .unwrap();

    let command: ProviderCommand = read_json_message(&mut frame.as_slice(), DEFAULT_MAX_FRAME_SIZE).unwrap();

    assert_eq!(
        command,
        ProviderCommand::OpenVirtualChannel {
            connection_id: 1,
            session_id: 2,
            endpoint_name: "Devolutions::Now::Agent".to_owned(),
            static_channel: false,
        }
    );
}

// oracle: property round-trip
#[test]
fn open_virtual_channel_keeps_the_static_flag() {
    for static_channel in [false, true] {
        let command = ProviderCommand::OpenVirtualChannel {
            connection_id: 7,
            session_id: 3,
            endpoint_name: "RDPDR".to_owned(),
            static_channel,
        };
        let mut frame = Vec::new();
        write_json_message(&mut frame, &command).unwrap();

        let decoded: ProviderCommand = read_json_message(&mut frame.as_slice(), DEFAULT_MAX_FRAME_SIZE).unwrap();

        assert_eq!(decoded, command);
    }
}
