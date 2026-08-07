use ironrdp_core::{decode, encode_vec};
use ironrdp_pdu::gcc::{ChannelName, ChannelOptions};
use ironrdp_pdu::mcs;
use ironrdp_pdu::rdp::vc::{ChannelControlFlags, ChannelPduHeader};
use ironrdp_pdu::x224::X224;
use ironrdp_session::x224::Processor;
use ironrdp_svc::{
    CHANNEL_CHUNK_LENGTH, ChannelFlags, MAX_STATIC_CHANNELS, StaticChannelKey, StaticChannelSet, StaticVirtualChannel,
    SvcClientProcessor, SvcMessage, SvcProcessor, SvcServerProcessor, client_encode_svc_messages, make_channel_options,
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
fn static_channel_chunks_do_not_expose_protocol_headers_by_default() {
    let chunks = StaticVirtualChannel::chunkify(vec![SvcMessage::from(vec![0; CHANNEL_CHUNK_LENGTH + 1])])
        .expect("static channel message should chunk");

    assert_eq!(chunks.len(), 2);
    for chunk in chunks {
        let header = decode::<ChannelPduHeader>(chunk.filled()).expect("channel header should decode");
        assert!(!header.flags.contains(ChannelControlFlags::FLAG_SHOW_PROTOCOL));
    }
}

#[test]
fn static_channel_preserves_explicit_protocol_header_exposure() {
    let chunks = StaticVirtualChannel::chunkify(vec![
        SvcMessage::from(vec![0; CHANNEL_CHUNK_LENGTH + 1]).with_flags(ChannelFlags::SHOW_PROTOCOL),
    ])
    .expect("static channel message should chunk");

    assert_eq!(chunks.len(), 2);
    for chunk in chunks {
        let header = decode::<ChannelPduHeader>(chunk.filled()).expect("channel header should decode");
        assert!(header.flags.contains(ChannelControlFlags::FLAG_SHOW_PROTOCOL));
    }
}

#[test]
fn static_channel_encodes_a_large_response_as_complete_ordered_frames() {
    let pdu: Vec<u8> = (0..65_556)
        .map(|value| u8::try_from(value % 251).expect("value fits in u8"))
        .collect();
    let frame = client_encode_svc_messages(vec![SvcMessage::from(pdu.clone())], 1005, 1002)
        .expect("large static-channel response should encode");

    let mut encoded_frames = frame.as_slice();
    let mut reassembled = Vec::with_capacity(pdu.len());

    for index in 0..41 {
        assert!(encoded_frames.len() >= 4, "missing TPKT header for fragment {index}");
        let packet_length = usize::from(u16::from_be_bytes([encoded_frames[2], encoded_frames[3]]));
        assert!(
            packet_length <= encoded_frames.len(),
            "incomplete TPKT frame for fragment {index}"
        );

        let request = decode::<X224<mcs::SendDataRequest<'_>>>(&encoded_frames[..packet_length])
            .expect("static-channel TPKT frame should decode")
            .0;
        assert_eq!(request.channel_id, 1005);

        let header = decode::<ChannelPduHeader>(&request.user_data).expect("channel PDU header should decode");
        assert_eq!(header.length, u32::try_from(pdu.len()).expect("PDU length fits in u32"));
        assert!(!header.flags.contains(ChannelControlFlags::FLAG_SHOW_PROTOCOL));
        assert_eq!(header.flags.contains(ChannelControlFlags::FLAG_FIRST), index == 0);
        assert_eq!(header.flags.contains(ChannelControlFlags::FLAG_LAST), index == 40);

        let fragment = &request.user_data[8..];
        assert_eq!(fragment.len(), if index == 40 { 1_556 } else { CHANNEL_CHUNK_LENGTH });
        reassembled.extend_from_slice(fragment);
        encoded_frames = &encoded_frames[packet_length..];
    }

    assert!(encoded_frames.is_empty());
    assert_eq!(reassembled, pdu);
}

#[test]
fn session_uses_the_negotiated_static_channel_chunk_size() {
    let channel_name = ChannelName::from_utf8("runtime").expect("valid static channel name");
    let mut channels = StaticChannelSet::new();
    assert!(channels.set_maximum_chunk_size(16_256));
    let key = channels
        .insert_dynamic(RuntimeChannel::new("runtime", ChannelOptions::empty()))
        .expect("dynamic key");
    channels.attach_channel_id_by_key(key, 1005);

    let processor = Processor::new(channels, 1002, 1003, None, 1);
    let frame = processor
        .process_svc_messages_by_name(&channel_name, vec![SvcMessage::from(vec![0; 16_257])])
        .expect("runtime channel should be encodable");
    let first_packet_length = usize::from(u16::from_be_bytes([frame[2], frame[3]]));
    let first_request = decode::<X224<mcs::SendDataRequest<'_>>>(&frame[..first_packet_length])
        .expect("first static-channel frame should decode")
        .0;
    let first_header = decode::<ChannelPduHeader>(&first_request.user_data).expect("channel PDU header should decode");

    assert_eq!(first_request.user_data.len(), 8 + 16_256);
    assert_eq!(first_header.length, 16_257);
    assert!(first_header.flags.contains(ChannelControlFlags::FLAG_FIRST));
    assert!(!first_header.flags.contains(ChannelControlFlags::FLAG_LAST));
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
