use ironrdp_pdu::rdp::client_info::ClientInfoFlags;
use ironrdp_server::audio_redirection_wanted;

/// Per MS-RDPBCGR [2.2.1.11.1.1] Info Packet (TS_INFO_PACKET), audio is redirected to the client only when its flags
/// set neither INFO_NOAUDIOPLAYBACK, under which audio redirection MUST NOT take place, nor INFO_REMOTECONSOLEAUDIO,
/// which asks for the audio to be played on the server.
/// Other flags, INFO_AUDIOCAPTURE included, don't change that: "If the INFO_NOAUDIOPLAYBACK flag is not set, then audio
/// redirection SHOULD take place if the INFO_REMOTECONSOLEAUDIO (0x00002000) flag is also not set."
///
/// [2.2.1.11.1.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/732394f5-e2b5-4ac5-8a0a-35345386b0d1
// oracle: spec MS-RDPBCGR §2.2.1.11.1.1
#[rstest::rstest]
#[case::neither_audio_flag(ClientInfoFlags::empty(), true)]
#[case::no_audio_playback(ClientInfoFlags::NO_AUDIO_PLAYBACK, false)]
#[case::remote_console_audio(ClientInfoFlags::REMOTE_CONSOLE_AUDIO, false)]
#[case::both_audio_flags(ClientInfoFlags::NO_AUDIO_PLAYBACK | ClientInfoFlags::REMOTE_CONSOLE_AUDIO, false)]
#[case::audio_capture_only(ClientInfoFlags::AUDIO_CAPTURE, true)]
fn audio_redirection_follows_the_info_packet_audio_flags(#[case] flags: ClientInfoFlags, #[case] redirected: bool) {
    assert_eq!(audio_redirection_wanted(flags), redirected);
}
