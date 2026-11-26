use ironrdp_connector::{AudioPlaybackMode, ClientConnector, ClientConnectorState, Sequence as _};
use ironrdp_core::{WriteBuf, decode};
use ironrdp_pdu::mcs::SendDataRequest;
use ironrdp_pdu::rdp::ClientInfoPdu;
use ironrdp_pdu::rdp::client_info::ClientInfoFlags;
use ironrdp_pdu::x224::X224;

#[expect(
    deprecated,
    reason = "these tests check how the legacy field interacts with the mode"
)]
fn client_info_flags(enable_audio_playback: bool, mode: Option<AudioPlaybackMode>) -> ClientInfoFlags {
    let mut config = super::test_config();
    config.enable_audio_playback = enable_audio_playback;
    config.audio_playback_mode = mode;
    let mut connector = ClientConnector::new(config, "127.0.0.1:3389".parse().unwrap());
    connector.state = ClientConnectorState::SecureSettingsExchange {
        io_channel_id: 1003,
        user_channel_id: 1002,
    };
    let mut output = WriteBuf::new();
    connector.step_no_input(&mut output).unwrap();
    let data = decode::<X224<SendDataRequest<'_>>>(output.filled()).unwrap().0;
    decode::<ClientInfoPdu>(data.user_data.as_ref())
        .unwrap()
        .client_info
        .flags
}

// oracle: spec MS-RDPBCGR §2.2.1.11.1.1 (INFO_NOAUDIOPLAYBACK, INFO_REMOTECONSOLEAUDIO); FreeRDP `audiomode` handling
#[test]
fn playback_mode_sets_client_info_flags() {
    for (mode, no_playback, remote_console) in [
        (AudioPlaybackMode::Client, false, false),
        (AudioPlaybackMode::Server, true, true),
        (AudioPlaybackMode::Disabled, true, false),
    ] {
        // The mode wins over the legacy boolean, in both directions.
        for legacy in [true, false] {
            let flags = client_info_flags(legacy, Some(mode));
            assert_eq!(
                flags.contains(ClientInfoFlags::NO_AUDIO_PLAYBACK),
                no_playback,
                "{mode:?} legacy={legacy}"
            );
            assert_eq!(
                flags.contains(ClientInfoFlags::REMOTE_CONSOLE_AUDIO),
                remote_console,
                "{mode:?} legacy={legacy}"
            );
        }
    }
}

#[test]
fn unset_playback_mode_follows_legacy_boolean() {
    let flags = client_info_flags(true, None);
    assert!(!flags.contains(ClientInfoFlags::NO_AUDIO_PLAYBACK));
    assert!(!flags.contains(ClientInfoFlags::REMOTE_CONSOLE_AUDIO));

    let flags = client_info_flags(false, None);
    assert!(flags.contains(ClientInfoFlags::NO_AUDIO_PLAYBACK));
    assert!(!flags.contains(ClientInfoFlags::REMOTE_CONSOLE_AUDIO));
}
