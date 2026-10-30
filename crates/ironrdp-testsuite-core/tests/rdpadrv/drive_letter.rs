use std::sync::{Arc, Mutex};

use ironrdp_core::encode_vec;
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_rdpadrv::client::{DriveLetterClient, DriveLetterHandler};
use ironrdp_rdpadrv::pdu::{CacheEntry, DriveLetterPdu, SerializedCache};
use ironrdp_rdpadrv::server::{DriveLetterServer, DriveLetterServerHandler};
use proptest::prelude::*;
use rstest::rstest;

use super::{CHANNEL_ID, encoded};

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<SerializedCache>>>);

impl Recorder {
    fn take(&self) -> Vec<SerializedCache> {
        core::mem::take(&mut *self.0.lock().unwrap())
    }
}

impl DriveLetterHandler for Recorder {
    fn cache_changed(&mut self, cache: &SerializedCache) {
        self.0.lock().unwrap().push(cache.clone());
    }
}

impl DriveLetterServerHandler for Recorder {
    fn cache_restored(&mut self, cache: SerializedCache) {
        self.0.lock().unwrap().push(cache);
    }
}

const SADLE_STARTED: [u8; 4] = [0x01, 0x00, 0x00, 0x00];

/// One `REG_DWORD` value named `AB` holding 13.
const ONE_ENTRY_CACHE: [u8; 44] = [
    0x02, 0x00, 0x00, 0x00, // eEvent: SADLE_SerializedCache
    0x1C, 0x00, 0x00, 0x00, // cbMessageData: 28
    0x1C, 0x00, 0x00, 0x00, // cbNameValueData: 28
    0x01, 0x00, 0x00, 0x00, // cNameValuePairs
    0x18, 0x18, 0x18, 0x18, // Name marker
    0x04, 0x00, 0x00, 0x00, // cchName: 4 bytes
    0x41, 0x00, 0x42, 0x00, // szName: "AB"
    0x27, 0x27, 0x27, 0x27, // Value marker
    0x04, 0x00, 0x00, 0x00, // Value type: REG_DWORD
    0x04, 0x00, 0x00, 0x00, // cbValue
    0x0D, 0x00, 0x00, 0x00, // rgValue: 13
];

fn one_entry_cache() -> SerializedCache {
    SerializedCache {
        entries: vec![CacheEntry::dword("AB".to_owned(), 13)],
    }
}

fn started_client(client: DriveLetterClient) -> DriveLetterClient {
    let mut client = client;
    assert!(client.start(CHANNEL_ID).expect("start channel").is_empty());
    client
}

// oracle: spec MS-RDPADRV §2.2.5
#[test]
fn serialized_cache_has_specified_wire_layout() {
    let pdu = DriveLetterPdu::SerializedCache(one_entry_cache());

    assert_eq!(encode_vec(&pdu).expect("encode"), ONE_ENTRY_CACHE);
    assert_eq!(
        ironrdp_core::decode::<DriveLetterPdu>(&ONE_ENTRY_CACHE).expect("decode"),
        pdu
    );
}

// oracle: spec MS-RDPADRV §2.2.5, which places an unused field after the pairs without saying whether the data sizes count it
#[rstest]
#[case::counted_by_the_data_sizes(0x1E)]
#[case::past_the_data_sizes(0x1C)]
fn unused_bytes_after_the_last_pair_are_ignored(#[case] data_size: u8) {
    let mut bytes = ONE_ENTRY_CACHE.to_vec();
    bytes[4] = data_size; // cbMessageData
    bytes[8] = data_size; // cbNameValueData
    bytes.extend_from_slice(&[0xAA, 0xBB]); // Unused

    assert_eq!(
        ironrdp_core::decode::<DriveLetterPdu>(&bytes).expect("decode"),
        DriveLetterPdu::SerializedCache(one_entry_cache())
    );
}

// oracle: spec MS-RDPADRV §2.2.5
#[test]
fn mismatched_data_sizes_are_rejected() {
    let mut bytes = ONE_ENTRY_CACHE.to_vec();
    bytes[4] = 0x1D; // cbMessageData no longer matches cbNameValueData

    assert!(ironrdp_core::decode::<DriveLetterPdu>(&bytes).is_err());
}

// oracle: spec MS-RDPADRV §3.1.3.2
#[test]
fn initialization_without_stored_mappings_is_not_answered() {
    let mut client = started_client(DriveLetterClient::new(Box::new(Recorder::default())));
    assert!(!client.initialized());

    let replies = client.process(CHANNEL_ID, &SADLE_STARTED).expect("process");

    assert!(replies.is_empty());
    assert!(client.initialized());
}

// oracle: spec MS-RDPADRV §3.1.3.2
#[test]
fn initialization_is_answered_with_the_stored_mappings() {
    let mut client =
        started_client(DriveLetterClient::new(Box::new(Recorder::default())).with_cache(one_entry_cache()));

    let replies = encoded(&client.process(CHANNEL_ID, &SADLE_STARTED).expect("process"));

    assert_eq!(replies, [ONE_ENTRY_CACHE.to_vec()]);
}

// oracle: spec MS-RDPADRV §3.1.5
#[test]
fn reported_mappings_replace_the_stored_ones_without_an_answer() {
    let recorder = Recorder::default();
    let mut client = started_client(DriveLetterClient::new(Box::new(recorder.clone())).with_cache(one_entry_cache()));
    let reported = SerializedCache {
        entries: vec![
            CacheEntry::dword("AB".to_owned(), 6),
            CacheEntry::dword("CD".to_owned(), 13),
        ],
    };
    let reported_bytes = encode_vec(&DriveLetterPdu::SerializedCache(reported.clone())).expect("encode");

    assert!(client.process(CHANNEL_ID, &reported_bytes).expect("process").is_empty());

    assert_eq!(recorder.take(), [reported]);
    let replies = encoded(&client.process(CHANNEL_ID, &SADLE_STARTED).expect("process"));
    assert_eq!(replies, [reported_bytes]);
}

// oracle: spec MS-RDPADRV §3.1.3.2
#[test]
fn closing_the_channel_ends_initialization() {
    let mut client = started_client(DriveLetterClient::new(Box::new(Recorder::default())));
    client.process(CHANNEL_ID, &SADLE_STARTED).expect("process");

    client.close(CHANNEL_ID);

    assert!(!client.initialized());
}

// oracle: spec MS-RDPADRV §3.1.3.2, §4.2
#[test]
fn server_initializes_and_receives_the_client_mappings() {
    let restored = Recorder::default();
    let mut server = DriveLetterServer::new(Box::new(restored.clone()));
    let mut client =
        started_client(DriveLetterClient::new(Box::new(Recorder::default())).with_cache(one_entry_cache()));

    let opening = encoded(&server.start(CHANNEL_ID).expect("start server"));
    assert_eq!(opening, [SADLE_STARTED.to_vec()]);
    for reply in encoded(&client.process(CHANNEL_ID, &opening[0]).expect("process")) {
        assert!(server.process(CHANNEL_ID, &reply).expect("process").is_empty());
    }

    assert_eq!(restored.take(), [one_entry_cache()]);
}

fn any_cache_entry() -> impl Strategy<Value = CacheEntry> {
    (
        any::<String>(),
        any::<u32>(),
        proptest::collection::vec(any::<u8>(), 0..16),
    )
        .prop_map(|(name, value_type, value)| CacheEntry {
            name,
            value_type,
            value,
        })
}

fn any_pdu() -> impl Strategy<Value = DriveLetterPdu> {
    prop_oneof![
        Just(DriveLetterPdu::Started),
        proptest::collection::vec(any_cache_entry(), 0..4)
            .prop_map(|entries| DriveLetterPdu::SerializedCache(SerializedCache { entries })),
    ]
}

proptest! {
    // oracle: property round-trip
    #[test]
    fn pdu_round_trips_through_encode_and_decode(pdu in any_pdu()) {
        let bytes = encode_vec(&pdu).expect("encode PDU");

        prop_assert_eq!(ironrdp_core::decode::<DriveLetterPdu>(&bytes).expect("decode PDU"), pdu);
    }
}
