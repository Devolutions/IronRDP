use ironrdp_cfg::AudioMode;
use ironrdp_client::__test_build_connector;
use ironrdp_client::config::{ConfigBuilder, Destination};
use ironrdp_connector::{AudioPlaybackMode, ClientConnector};
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_svc::SvcMessage;

const RDPSND: ChannelName = ChannelName::from_static(b"rdpsnd\0\0");

#[derive(Debug)]
struct CustomRdpsndChannel;

ironrdp_core::impl_as_any!(CustomRdpsndChannel);

impl ironrdp_svc::SvcProcessor for CustomRdpsndChannel {
    fn channel_name(&self) -> ChannelName {
        RDPSND
    }

    fn process(&mut self, _payload: &[u8]) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }
}

impl ironrdp_svc::SvcClientProcessor for CustomRdpsndChannel {}

fn complete_builder() -> ConfigBuilder {
    ConfigBuilder::new()
        .with_destination(Destination::new("server.example:3389").unwrap())
        .with_username("user")
        .with_password("password")
        .with_client_build(1)
        .with_client_dir("C:\\")
        .with_platform(MajorPlatformType::WINDOWS)
        .with_client_name("client")
}

/// Builds a connector for the `.rdp` audio mode with the client's own sound output turned off.
///
/// Also reports whether an RDPSND channel is attached.
fn connector_without_sound(builder: ConfigBuilder, mode: AudioMode) -> (ClientConnector, bool) {
    let mut config = builder.with_audio_mode(mode).build().expect("valid configuration");
    config.__test_set_sound(false);
    let connector = __test_build_connector(&config, None).expect("build connector");
    let has_rdpsnd = connector.static_channels.get_by_channel_name_key(&RDPSND).is_some();
    (connector, has_rdpsnd)
}

// oracle: spec MS-RDPBCGR §2.2.1.11.1.1 (INFO_NOAUDIOPLAYBACK is omitted when audio playback is redirected to the client over RDPSND)
#[test]
fn custom_rdpsnd_static_channel_keeps_audio_playback_enabled() {
    let (connector, has_rdpsnd) = connector_without_sound(
        complete_builder().with_static_channel(|_| Some(CustomRdpsndChannel)),
        AudioMode::RedirectToClient,
    );

    assert_eq!(connector.config.audio_playback_mode(), AudioPlaybackMode::Client);
    assert!(has_rdpsnd);
}

// oracle: spec MS-RDPBCGR §2.2.1.11.1.1 (INFO_NOAUDIOPLAYBACK is omitted when audio playback is redirected to the client over RDPSND)
#[test]
fn custom_rdpsnd_static_channel_instances_keeps_audio_playback_enabled() {
    let (connector, has_rdpsnd) = connector_without_sound(
        complete_builder().with_static_channel_instances(|_| vec![CustomRdpsndChannel]),
        AudioMode::RedirectToClient,
    );

    assert_eq!(connector.config.audio_playback_mode(), AudioPlaybackMode::Client);
    assert!(has_rdpsnd);
}

// oracle: spec MS-RDPBCGR §2.2.1.11.1.1 (INFO_NOAUDIOPLAYBACK disables audio redirection when playback is disabled)
#[test]
fn custom_rdpsnd_static_channel_respects_disabled_audio_playback() {
    let (connector, has_rdpsnd) = connector_without_sound(
        complete_builder().with_static_channel(|_| Some(CustomRdpsndChannel)),
        AudioMode::Disabled,
    );

    assert_eq!(connector.config.audio_playback_mode(), AudioPlaybackMode::Disabled);
    assert!(has_rdpsnd);
}

// Turning sound off must update the mode itself, because a set mode overrides the legacy flag.
// oracle: spec MS-RDPBCGR §2.2.1.11.1.1 (INFO_NOAUDIOPLAYBACK disables audio redirection when no playback channel is registered)
#[test]
fn sound_off_disables_client_playback() {
    let (connector, _) = connector_without_sound(complete_builder(), AudioMode::RedirectToClient);

    assert_eq!(connector.config.audio_playback_mode(), AudioPlaybackMode::Disabled);
}

// oracle: spec MS-RDPBCGR §2.2.1.11.1.1 (INFO_REMOTECONSOLEAUDIO plays audio on the server, so losing local sound output leaves the mode unchanged)
#[test]
fn sound_off_keeps_server_playback() {
    let (connector, _) = connector_without_sound(complete_builder(), AudioMode::PlayOnServer);

    assert_eq!(connector.config.audio_playback_mode(), AudioPlaybackMode::Server);
}
