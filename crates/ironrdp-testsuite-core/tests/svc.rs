use ironrdp_core::{ReadCursor, decode, decode_cursor, encode_vec};
use ironrdp_pdu::gcc::{ChannelName, ChannelOptions};
use ironrdp_pdu::mcs;
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
use ironrdp_pdu::x224::X224;
use ironrdp_session::{ActiveStageBuilder, x224::Processor};
use ironrdp_svc::{
    CHANNEL_CHUNK_LENGTH, MAX_CHANNEL_CHUNK_LENGTH, MAX_STATIC_CHANNELS, StaticChannelKey, StaticChannelSet,
    StaticVirtualChannel, SvcClientProcessor, SvcMessage, SvcProcessor, SvcServerProcessor, make_channel_options,
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

#[derive(Debug)]
struct StartingRuntimeChannel;

ironrdp_svc::impl_as_any!(StartingRuntimeChannel);

impl SvcProcessor for StartingRuntimeChannel {
    fn channel_name(&self) -> ChannelName {
        ChannelName::from_utf8("start").expect("valid static channel name")
    }

    fn start(&mut self) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
        Ok(vec![SvcMessage::from(vec![1])])
    }

    fn process(&mut self, _payload: &[u8]) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }
}

impl SvcServerProcessor for StartingRuntimeChannel {}

#[derive(Debug)]
struct TypedChannel<const ID: usize>;

impl<const ID: usize> ironrdp_core::AsAny for TypedChannel<ID> {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

impl<const ID: usize> SvcProcessor for TypedChannel<ID> {
    fn channel_name(&self) -> ChannelName {
        ChannelName::from_utf8("typed").expect("valid static channel name")
    }

    fn process(&mut self, _payload: &[u8]) -> ironrdp_pdu::PduResult<Vec<SvcMessage>> {
        Ok(Vec::new())
    }
}

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
fn typed_channels_enforce_the_static_channel_limit() {
    macro_rules! insert_typed_channels {
        ($channels:expr; $($id:literal),+ $(,)?) => {
            $(
                assert!($channels.insert(TypedChannel::<$id>).is_none());
            )+
        };
    }

    let mut channels = StaticChannelSet::new();
    insert_typed_channels!(
        channels;
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
        25, 26, 27, 28, 29, 30
    );

    assert!(channels.insert(TypedChannel::<31>).is_none());
    assert_eq!(channels.len(), MAX_STATIC_CHANNELS);
    assert!(channels.get_by_type::<TypedChannel<31>>().is_none());
}

#[test]
fn dynamic_channels_are_included_in_startup_iteration() {
    let mut channels = StaticChannelSet::new();
    let key = channels.insert_dynamic(StartingRuntimeChannel).expect("dynamic key");
    channels.attach_channel_id_by_key(key, 1005);

    let startup_message_count = channels
        .iter_by_key_mut()
        .map(|(_, channel, channel_id)| {
            assert_eq!(channel_id, Some(1005));
            channel.start().expect("channel should start").len()
        })
        .sum::<usize>();

    assert_eq!(startup_message_count, 1);
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
fn static_channel_chunks_show_protocol_header() {
    let chunks = StaticVirtualChannel::chunkify(vec![SvcMessage::from(vec![0; CHANNEL_CHUNK_LENGTH + 1])])
        .expect("static channel message should chunk");

    assert_eq!(chunks.len(), 2);
    for chunk in chunks {
        let header = decode::<ChannelPduHeader>(chunk.filled()).expect("channel header should decode");
        assert!(header.flags.contains(ChannelControlFlags::FLAG_SHOW_PROTOCOL));
    }
}

#[test]
fn static_channel_chunk_size_defaults_to_1600_and_rejects_out_of_range_values() {
    let mut channels = StaticChannelSet::new();

    assert_eq!(channels.maximum_chunk_size(), CHANNEL_CHUNK_LENGTH);
    assert!(!channels.set_maximum_chunk_size(CHANNEL_CHUNK_LENGTH - 1));
    assert_eq!(channels.maximum_chunk_size(), CHANNEL_CHUNK_LENGTH);
    assert!(!channels.set_maximum_chunk_size(MAX_CHANNEL_CHUNK_LENGTH + 1));
    assert_eq!(channels.maximum_chunk_size(), CHANNEL_CHUNK_LENGTH);
    assert!(channels.set_maximum_chunk_size(MAX_CHANNEL_CHUNK_LENGTH));
    assert_eq!(channels.maximum_chunk_size(), MAX_CHANNEL_CHUNK_LENGTH);
}

#[test]
fn reactivated_session_applies_the_static_channel_chunk_size() {
    let channel_name = ChannelName::from_utf8("runtime").expect("valid static channel name");
    let mut channels = StaticChannelSet::new();
    let key = channels
        .insert_dynamic(RuntimeChannel::new("runtime", ChannelOptions::empty()))
        .expect("dynamic key");
    channels.attach_channel_id_by_key(key, 1005);

    let mut active_stage = ActiveStageBuilder {
        static_channels: channels,
        user_channel_id: 1002,
        io_channel_id: 1003,
        message_channel_id: None,
        share_id: 1,
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
    }
    .build();
    assert!(active_stage.reactivate(1003, 1002, 2, false, false, 4096));
    let frame = active_stage
        .process_svc_messages_by_name(&channel_name, vec![SvcMessage::from(vec![0; 4097])])
        .expect("runtime channel should be encodable");

    let mut cursor = ReadCursor::new(&frame);
    let first: X224<mcs::SendDataRequest<'_>> = decode_cursor(&mut cursor).expect("first chunk should decode");
    let second: X224<mcs::SendDataRequest<'_>> = decode_cursor(&mut cursor).expect("second chunk should decode");
    assert!(cursor.is_empty());

    let first_header = decode::<ChannelPduHeader>(first.0.user_data.as_ref()).expect("first header should decode");
    assert_eq!(first_header.length, 4097);
    assert_eq!(first.0.user_data.len() - 8, 4096);
    assert!(
        first_header
            .flags
            .contains(ChannelControlFlags::FLAG_FIRST | ChannelControlFlags::FLAG_SHOW_PROTOCOL)
    );
    assert!(!first_header.flags.contains(ChannelControlFlags::FLAG_LAST));

    let second_header = decode::<ChannelPduHeader>(second.0.user_data.as_ref()).expect("second header should decode");
    assert_eq!(second_header.length, 4097);
    assert_eq!(second.0.user_data.len() - 8, 1);
    assert!(
        second_header
            .flags
            .contains(ChannelControlFlags::FLAG_LAST | ChannelControlFlags::FLAG_SHOW_PROTOCOL)
    );
    assert!(!second_header.flags.contains(ChannelControlFlags::FLAG_FIRST));
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
