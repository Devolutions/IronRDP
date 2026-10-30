use ironrdp_client::multipoint::{CacheEntry, DataFlow, MultipointSettings, SerializedCache, VolumeChange};
use ironrdp_client::output_channel::{OutputEventReceiver, OutputEventSender, output_channel};
use ironrdp_client::rdp::RdpOutputEvent;
use ironrdp_core::encode_vec;
use ironrdp_dvc::{DvcMessage, DvcProcessor as _};
use ironrdp_rdpadrv::pdu::{AudioLevelPdu, DriveLetterPdu};
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
