use ironrdp_client::__test_build_connector;
use ironrdp_client::config::{ConfigBuilder, Destination};
use ironrdp_client::multipoint::{CacheEntry, DataFlow, MultipointSettings, SerializedCache, VolumeChange};
use ironrdp_client::output_channel::{OutputEventReceiver, OutputEventSender, output_channel};
use ironrdp_client::rdp::RdpOutputEvent;
use ironrdp_core::encode_vec;
use ironrdp_dvc::{DrdynvcClient, DvcMessage, DvcProcessor as _};
use ironrdp_pdu::rdp::capability_sets::MajorPlatformType;
use ironrdp_rdpadrv::client::{AudioLevelClient, DriveLetterClient};
use ironrdp_rdpadrv::pdu::{AudioLevelPdu, DriveLetterPdu};
use ironrdp_rdpadrv::{AUDIO_LEVEL_CHANNEL_NAME, DRIVE_LETTER_CHANNEL_NAME};
use rstest::rstest;
use tokio::sync::watch;

const CHANNEL_ID: u32 = 1;

fn encoded(messages: &[DvcMessage]) -> Vec<Vec<u8>> {
    messages
        .iter()
        .map(|message| encode_vec(message.as_ref()).expect("encode DVC message"))
        .collect()
}

async fn next_deferred_event(sender: &OutputEventSender, receiver: &mut OutputEventReceiver) -> Option<RdpOutputEvent> {
    let (_close_sender, mut close_receiver) = watch::channel(false);
    assert!(
        sender
            .__test_send_deferred(&mut close_receiver)
            .await
            .expect("output channel is open")
    );
    receiver.recv().await
}

// oracle: spec MS-RDPADRV §3.1.3.1, §3.1.5, with the processors rebuilt as a reconnect rebuilds them
#[tokio::test]
async fn reported_volume_is_saved_and_restored_by_the_next_connection() {
    let (output_sender, mut output_receiver) = output_channel(4);
    let settings = MultipointSettings::new();
    let reported = AudioLevelPdu::VolumeChange(VolumeChange {
        data_flow: DataFlow::Capture,
        volume: 0.25,
        muted: true,
    });

    let (mut audio_level, _) = settings.__test_processors(&output_sender);
    audio_level
        .process(CHANNEL_ID, &encode_vec(&reported).expect("encode"))
        .expect("process");

    assert!(matches!(
        next_deferred_event(&output_sender, &mut output_receiver).await,
        Some(RdpOutputEvent::MultipointSettingsChanged)
    ));
    let (mut audio_level, _) = settings.__test_processors(&output_sender);
    let replies = audio_level
        .process(CHANNEL_ID, &encode_vec(&AudioLevelPdu::Started).expect("encode"))
        .expect("process");
    assert_eq!(encoded(&replies), [encode_vec(&reported).expect("encode")]);
}

// oracle: spec MS-RDPADRV §3.1.3.2, §3.1.5, with the processors rebuilt as a reconnect rebuilds them
#[tokio::test]
async fn reported_drive_letters_are_saved_and_restored_by_the_next_connection() {
    let (output_sender, mut output_receiver) = output_channel(4);
    let settings = MultipointSettings::new();
    let reported = DriveLetterPdu::SerializedCache(SerializedCache {
        entries: vec![CacheEntry::dword("AB".to_owned(), 13)],
    });

    let (_, mut drive_letter) = settings.__test_processors(&output_sender);
    drive_letter
        .process(CHANNEL_ID, &encode_vec(&reported).expect("encode"))
        .expect("process");

    assert!(matches!(
        next_deferred_event(&output_sender, &mut output_receiver).await,
        Some(RdpOutputEvent::MultipointSettingsChanged)
    ));
    let (_, mut drive_letter) = settings.__test_processors(&output_sender);
    let replies = drive_letter
        .process(CHANNEL_ID, &encode_vec(&DriveLetterPdu::Started).expect("encode"))
        .expect("process");
    assert_eq!(encoded(&replies), [encode_vec(&reported).expect("encode")]);
}

// oracle: spec MS-RDPADRV §2.1, both channels registered only when the embedder supplies settings
#[rstest]
#[case::configured(Some(MultipointSettings::new()), true)]
#[case::not_configured(None, false)]
fn connector_registers_both_channels_only_when_configured(
    #[case] settings: Option<MultipointSettings>,
    #[case] registered: bool,
) {
    let config = ConfigBuilder::new()
        .with_destination(Destination::from_parts("multipoint.invalid", 3389))
        .with_username("multipoint_user")
        .with_password(String::new())
        .with_client_build(1)
        .with_client_dir(r"C:\")
        .with_platform(MajorPlatformType::WINDOWS)
        .with_client_name("client")
        .with_certificate_validation(ironrdp_tls::CertificateValidation::Strict)
        .with_multipoint_settings(settings)
        .build()
        .expect("supported multipoint test configuration");

    let mut connector = __test_build_connector(&config, None).expect("build connector");

    let drdynvc = connector
        .get_static_channel_processor::<DrdynvcClient>()
        .expect("DRDYNVC processor");
    assert_eq!(drdynvc.has_registered_dvc::<AudioLevelClient>(), registered);
    assert_eq!(drdynvc.has_registered_dvc::<DriveLetterClient>(), registered);
    assert_eq!(drdynvc.has_channel_name(AUDIO_LEVEL_CHANNEL_NAME), registered);
    assert_eq!(drdynvc.has_channel_name(DRIVE_LETTER_CHANNEL_NAME), registered);
}
