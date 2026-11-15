use ironrdp_wtsprotocol_ipc::{
    DEFAULT_MAX_FRAME_SIZE, ProviderCommand, read_async_frame, read_json_message, write_async_frame, write_frame,
    write_json_message,
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

// oracle: mutant drop the static flag when serializing or deserializing a virtual-channel open.
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

// oracle: mutant a configured listener port is dropped or old StartListen messages stop decoding.
#[test]
fn start_listen_accepts_old_messages_and_round_trips_a_configured_port() {
    let legacy: ProviderCommand =
        serde_json::from_slice(br#"{"type":"start_listen","listener_name":"IRDP-Tcp"}"#).unwrap();
    assert_eq!(
        legacy,
        ProviderCommand::StartListen {
            listener_name: "IRDP-Tcp".to_owned(),
            port: None,
        }
    );

    let legacy_wait: ProviderCommand =
        serde_json::from_slice(br#"{"type":"wait_for_incoming","listener_name":"IRDP-Tcp","timeout_ms":1000}"#)
            .unwrap();
    assert_eq!(
        legacy_wait,
        ProviderCommand::WaitForIncoming {
            listener_name: "IRDP-Tcp".to_owned(),
            timeout_ms: 1000,
            port: None,
        }
    );

    let command = ProviderCommand::StartListen {
        listener_name: "IRDP-Tcp".to_owned(),
        port: Some(4495),
    };
    let mut frame = Vec::new();
    write_json_message(&mut frame, &command).unwrap();
    let decoded: ProviderCommand = read_json_message(&mut frame.as_slice(), DEFAULT_MAX_FRAME_SIZE).unwrap();

    assert_eq!(decoded, command);

    let command = ProviderCommand::WaitForIncoming {
        listener_name: "IRDP-Tcp".to_owned(),
        timeout_ms: 1000,
        port: Some(4495),
    };
    let mut frame = Vec::new();
    write_json_message(&mut frame, &command).unwrap();
    let decoded: ProviderCommand = read_json_message(&mut frame.as_slice(), DEFAULT_MAX_FRAME_SIZE).unwrap();

    assert_eq!(decoded, command);
}

// oracle: reference server/termsrv/crates/ironrdp-wtsprotocol-ipc/src/lib.rs::write_frame encodes a four-byte little-endian length.
#[tokio::test]
async fn async_frame_helpers_preserve_the_length_prefixed_wire_format() {
    let (mut writer, mut reader) = tokio::io::duplex(7);
    write_async_frame(&mut writer, b"rdp", 3).await.unwrap();
    let mut encoded = [0u8; 7];
    tokio::io::AsyncReadExt::read_exact(&mut reader, &mut encoded)
        .await
        .unwrap();
    assert_eq!(encoded, [3, 0, 0, 0, b'r', b'd', b'p']);

    let (mut writer, mut reader) = tokio::io::duplex(7);
    tokio::io::AsyncWriteExt::write_all(&mut writer, &encoded)
        .await
        .unwrap();
    assert_eq!(read_async_frame(&mut reader, 3).await.unwrap(), b"rdp");
}

// oracle: mutant an oversized frame length reaches body allocation or reading.
#[tokio::test]
async fn async_frame_reader_rejects_oversized_lengths_before_reading_the_body() {
    let (mut writer, mut reader) = tokio::io::duplex(4);
    tokio::io::AsyncWriteExt::write_all(&mut writer, &17u32.to_le_bytes())
        .await
        .unwrap();
    drop(writer);

    let error = read_async_frame(&mut reader, 16).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
}
