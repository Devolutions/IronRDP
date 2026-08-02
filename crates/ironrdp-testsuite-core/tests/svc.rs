use ironrdp_core::{decode, encode_vec};
use ironrdp_pdu::gcc::{ChannelName, ChannelOptions};
use ironrdp_pdu::mcs;
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
use ironrdp_pdu::x224::X224;
use ironrdp_session::x224::Processor;
use ironrdp_svc::{
    MAX_STATIC_CHANNELS, StaticChannelKey, StaticChannelSet, StaticVirtualChannel, SvcClientProcessor, SvcMessage,
    SvcProcessor, SvcServerProcessor, make_channel_options,
};

#[derive(Debug)]
struct RuntimeChannel {
    name: ChannelName,
    options: ChannelOptions,
}

impl RuntimeChannel {
    fn new(name: &str, options: ChannelOptions) -> Self {
        Self {
            name: ChannelName::from_utf8(name).expect("valid static channel name"),
            options,
        }
    }
}

ironrdp_svc::impl_as_any!(RuntimeChannel);

impl SvcProcessor for RuntimeChannel {
    fn channel_name(&self) -> ChannelName {
        self.name.clone()
    }

    fn channel_options(&self) -> ChannelOptions {
        self.options
    }

    fn process(&mut self, _payload: &[u8]) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }
}

impl SvcClientProcessor for RuntimeChannel {}
impl SvcServerProcessor for RuntimeChannel {}

fn channel_chunk(data: &[u8], length: u32, flags: ChannelControlFlags) -> Vec<u8> {
    let mut chunk = encode_vec(&ChannelPduHeader { length, flags }).expect("channel header should encode");
    chunk.extend_from_slice(data);
    chunk
}

#[test]
fn runtime_channels_have_independent_keys_options_and_ids() {
    let first_name = ChannelName::from_utf8("first").expect("valid static channel name");
    let second_name = ChannelName::from_utf8("second").expect("valid static channel name");
    let mut channels = StaticChannelSet::new();
    let first = channels
        .insert_dynamic(RuntimeChannel::new("first", ChannelOptions::COMPRESS))
        .expect("dynamic key");
    let second = channels
        .insert_dynamic(RuntimeChannel::new("second", ChannelOptions::PRI_HIGH))
        .expect("dynamic key");

    assert_ne!(first, second);
    assert_eq!(channels.len(), 2);
    assert!(channels.get_by_key(first).is_some());
    assert!(channels.get_by_key(second).is_some());
    assert_eq!(
        make_channel_options(channels.get_by_channel_name_key(&first_name).expect("first channel").1),
        ChannelOptions::COMPRESS
    );
    assert_eq!(
        make_channel_options(
            channels
                .get_by_channel_name_key(&second_name)
                .expect("second channel")
                .1
        ),
        ChannelOptions::PRI_HIGH
    );

    channels.attach_channel_id_by_key(first, 1005);
    channels.attach_channel_id_by_key(second, 1006);
    assert_eq!(channels.get_channel_id_by_channel_name(&first_name), Some(1005));
    assert_eq!(channels.get_channel_id_by_channel_name(&second_name), Some(1006));

    assert_eq!(
        channels.attach_channel_id_by_key(StaticChannelKey::Dynamic(u64::MAX), 1005),
        None
    );
    assert_eq!(channels.get_channel_id_by_channel_name(&first_name), Some(1005));

    channels.attach_channel_id_by_key(second, 1005);
    assert_eq!(channels.get_channel_id_by_channel_name(&first_name), None);
    assert_eq!(channels.get_channel_id_by_channel_name(&second_name), Some(1005));
    assert_eq!(channels.get_key_by_channel_id(1005), Some(second));
}

#[test]
fn runtime_channels_enforce_the_static_channel_limit() {
    let mut channels = StaticChannelSet::new();

    for index in 0..MAX_STATIC_CHANNELS {
        assert!(
            channels
                .insert_dynamic(RuntimeChannel::new(&format!("ch{index:02}"), ChannelOptions::empty()))
                .is_some()
        );
    }

    assert!(
        channels
            .insert_dynamic(RuntimeChannel::new("overflow", ChannelOptions::empty()))
            .is_none()
    );
}

#[test]
fn static_channel_rejects_malformed_chunk_sequences() {
    let mut channel = StaticVirtualChannel::new(RuntimeChannel::new("chunk", ChannelOptions::empty()));

    assert!(
        channel
            .process(&channel_chunk(b"last", 4, ChannelControlFlags::FLAG_LAST))
            .is_err()
    );
    assert!(
        channel
            .process(&channel_chunk(b"first", 10, ChannelControlFlags::FLAG_FIRST))
            .is_ok()
    );
    assert!(
        channel
            .process(&channel_chunk(b"nested", 10, ChannelControlFlags::FLAG_FIRST))
            .is_err()
    );
    assert!(
        channel
            .process(&channel_chunk(
                b"wrong",
                6,
                ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST
            ))
            .is_err()
    );
    assert!(
        channel
            .process(&channel_chunk(b"plain", 6, ChannelControlFlags::empty()))
            .is_err()
    );
    assert!(
        channel
            .process(&channel_chunk(
                b"complete",
                8,
                ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_LAST
            ))
            .is_ok()
    );
}

#[test]
fn session_encodes_messages_for_runtime_channel_name() {
    let channel_name = ChannelName::from_utf8("runtime").expect("valid static channel name");
    let mut channels = StaticChannelSet::new();
    let key = channels
        .insert_dynamic(RuntimeChannel::new("runtime", ChannelOptions::empty()))
        .expect("dynamic key");
    channels.attach_channel_id_by_key(key, 1005);

    let processor = Processor::new(channels, 1002, 1003, None, 1);
    let frame = processor
        .process_svc_messages_by_name(&channel_name, vec![SvcMessage::from(vec![1, 2, 3])])
        .expect("runtime channel should be encodable");
    let request = decode::<X224<mcs::SendDataRequest<'_>>>(&frame)
        .expect("encoded runtime channel frame should decode")
        .0;

    assert_eq!(request.channel_id, 1005);
}
