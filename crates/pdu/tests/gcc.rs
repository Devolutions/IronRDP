use ironrdp_pdu::gcc::conference_create::*;
use ironrdp_pdu::gcc::*;
use ironrdp_pdu::PduParsing as _;
use ironrdp_pdu_samples::cluster_data::*;
use ironrdp_pdu_samples::conference_create::*;
use ironrdp_pdu_samples::core_data::*;
use ironrdp_pdu_samples::gcc::*;
use ironrdp_pdu_samples::network_data::*;
use ironrdp_pdu_samples::security_data::*;

#[test]
fn from_buffer_correctly_parses_client_gcc_blocks_without_optional_data_blocks() {
    let buffer = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER;

    assert_eq!(
        *CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS,
        ClientGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_gcc_blocks_with_one_optional_data_blocks() {
    let buffer = CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER;

    assert_eq!(
        *CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD,
        ClientGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_gcc_blocks_with_all_optional_data_blocks() {
    let buffer = CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS_BUFFER;

    assert_eq!(
        *CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS,
        ClientGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_gcc_blocks_with_optional_data_blocks_in_different_order() {
    let buffer = CLIENT_GCC_WITH_OPTIONAL_FIELDS_IN_DIFFERENT_ORDER_BUFFER;

    assert_eq!(
        *CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS,
        ClientGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_fails_on_invalid_gcc_type_for_client_gcc_blocks() {
    let mut buffer = CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS_BUFFER;
    buffer[CLIENT_GCC_CORE_BLOCK_BUFFER.len()] = 0x00;

    assert!(ClientGccBlocks::from_buffer(buffer.as_slice()).is_err());
}

#[test]
fn to_buffer_correctly_serializes_client_gcc_blocks_without_optional_data_blocks() {
    let data = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_slice(), buf.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_client_gcc_blocks_with_one_optional_data_blocks() {
    let data = CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD.clone();
    let expected_buffer = CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_slice(), buf.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_client_gcc_blocks_with_all_optional_data_blocks() {
    let data = CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS.clone();
    let expected_buffer = CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_slice(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_gcc_blocks_without_optional_data_blocks() {
    let data = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_gcc_blocks_with_one_optional_data_blocks() {
    let data = CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD.clone();
    let expected_buffer_len = CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_gcc_blocks_with_all_optional_data_blocks() {
    let data = CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_server_gcc_blocks_without_optional_data_blocks() {
    let buffer = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER;

    assert_eq!(
        *SERVER_GCC_WITHOUT_OPTIONAL_FIELDS,
        ServerGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_server_gcc_blocks_with_optional_data_blocks() {
    let buffer = SERVER_GCC_WITH_OPTIONAL_FIELDS_BUFFER;

    assert_eq!(
        *SERVER_GCC_WITH_OPTIONAL_FIELDS,
        ServerGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_server_gcc_blocks_with_optional_data_blocks_in_different_order() {
    let buffer = SERVER_GCC_WITH_OPTIONAL_FIELDS_IN_DIFFERENT_ORDER_BUFFER;

    assert_eq!(
        *SERVER_GCC_WITH_OPTIONAL_FIELDS,
        ServerGccBlocks::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_fails_on_invalid_gcc_type_for_server_gcc_blocks() {
    let mut buffer = SERVER_GCC_WITH_OPTIONAL_FIELDS_BUFFER;
    buffer[SERVER_GCC_CORE_BLOCK_BUFFER.len()] = 0x00;

    assert!(ServerGccBlocks::from_buffer(buffer.as_slice()).is_err());
}

#[test]
fn to_buffer_correctly_serializes_server_gcc_blocks_without_optional_data_blocks() {
    let data = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_slice(), buf.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_server_gcc_blocks_with_optional_data_blocks() {
    let data = SERVER_GCC_WITH_OPTIONAL_FIELDS.clone();
    let expected_buffer = SERVER_GCC_WITH_OPTIONAL_FIELDS_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_slice(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_gcc_blocks_without_optional_data_blocks() {
    let data = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_server_gcc_blocks_with_optional_data_blocks() {
    let data = SERVER_GCC_WITH_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = SERVER_GCC_WITH_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_handles_invalid_lengths_in_user_data_header() {
    let buffer: [u8; 4] = [0x01, 0xc0, 0x00, 0x00];

    assert!(UserDataHeader::<ClientGccType>::from_buffer(buffer.as_ref()).is_err());
}

#[test]
fn from_buffer_correctly_parses_client_cluster_data() {
    let buffer = CLUSTER_DATA_BUFFER.as_ref();

    assert_eq!(*CLUSTER_DATA, ClientClusterData::from_buffer(buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_client_cluster_data() {
    let data = CLUSTER_DATA.clone();
    let expected_buffer = CLUSTER_DATA_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_cluster_data() {
    let data = CLUSTER_DATA.clone();
    let expected_buffer_len = CLUSTER_DATA_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_client_core_data_without_optional_fields() {
    let buffer = CLIENT_CORE_DATA_BUFFER.as_ref();

    assert_eq!(
        *CLIENT_CORE_DATA_WITHOUT_OPTIONAL_FIELDS,
        ClientCoreData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_core_data_without_few_optional_fields() {
    let buffer = CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL_BUFFER;

    assert_eq!(
        *CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL,
        ClientCoreData::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_core_data_with_all_optional_fields() {
    let buffer = CLIENT_OPTIONAL_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_BUFFER;

    assert_eq!(
        *CLIENT_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS,
        ClientCoreData::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_client_core_data_without_optional_fields() {
    let core_data = CLIENT_CORE_DATA_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer = CLIENT_CORE_DATA_BUFFER.as_ref();

    let mut buf = Vec::new();
    core_data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer, buf.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_client_core_data_without_few_optional_fields() {
    let core_data = CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL.clone();
    let expected_buffer = CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL_BUFFER;

    let mut buf = Vec::new();
    core_data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_slice(), buf.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_client_core_data_with_all_optional_fields() {
    let core_data = CLIENT_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS.clone();
    let expected_buffer = CLIENT_OPTIONAL_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_BUFFER;

    let mut buf = Vec::new();
    core_data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_slice(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_core_data_without_optional_fields() {
    let data = CLIENT_CORE_DATA_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = CLIENT_CORE_DATA_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_core_data_without_few_optional_fields() {
    let data = CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL.clone();
    let expected_buffer_len = CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_core_data_with_all_optional_fields() {
    let data = CLIENT_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = CLIENT_OPTIONAL_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn client_color_depth_is_color_depth_if_post_beta_color_depth_is_absent() {
    let buffer = CLIENT_CORE_DATA_BUFFER.as_ref();

    let core_data = ClientCoreData::from_buffer(buffer).unwrap();
    let expected_client_color_depth: ClientColorDepth = From::from(core_data.color_depth);

    assert_eq!(expected_client_color_depth, core_data.client_color_depth());
}

#[test]
fn client_color_depth_is_post_beta_color_depth_if_high_color_depth_is_absent() {
    let buffer = CLIENT_OPTIONAL_CORE_DATA_TO_HIGH_COLOR_DEPTH_BUFFER_BUFFER;

    let core_data = ClientCoreData::from_buffer(buffer.as_slice()).unwrap();
    let expected_client_color_depth: ClientColorDepth =
        From::from(core_data.optional_data.post_beta2_color_depth.unwrap());

    assert_eq!(expected_client_color_depth, core_data.client_color_depth());
}

#[test]
fn client_color_depth_is_high_color_depth_if_want_32_bpp_flag_is_absent() {
    let buffer = CLIENT_OPTIONAL_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_BUFFER;

    let core_data = ClientCoreData::from_buffer(buffer.as_slice()).unwrap();
    let expected_client_color_depth: ClientColorDepth = From::from(core_data.optional_data.high_color_depth.unwrap());

    assert_eq!(expected_client_color_depth, core_data.client_color_depth());
}

#[test]
fn client_color_depth_is_32_bpp_if_want_32_bpp_flag_is_set() {
    let buffer = CLIENT_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_WITH_WANT_32_BPP_EARLY_FLAG_BUFFER.clone();
    let expected_core_data = CLIENT_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_WITH_WANT_32_BPP_EARLY_FLAG.clone();
    let expected_client_color_depth = ClientColorDepth::Bpp32;

    let core_data = ClientCoreData::from_buffer(buffer.as_slice()).unwrap();

    assert_eq!(expected_core_data, core_data);
    assert_eq!(expected_client_color_depth, core_data.client_color_depth());
}

#[test]
fn from_buffer_correctly_parses_server_core_data_without_optional_fields() {
    let buffer = SERVER_CORE_DATA_BUFFER.as_ref();

    assert_eq!(*SERVER_CORE_DATA, ServerCoreData::from_buffer(buffer).unwrap());
}

#[test]
fn from_buffer_correctly_parses_server_core_data_without_few_optional_fields() {
    let buffer = SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER;

    assert_eq!(
        *SERVER_CORE_DATA_TO_FLAGS,
        ServerCoreData::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_server_core_data_with_all_optional_fields() {
    let buffer = SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_BUFFER;

    assert_eq!(
        *SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS,
        ServerCoreData::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_server_core_data_without_optional_fields() {
    let data = SERVER_CORE_DATA.clone();
    let expected_buffer = SERVER_CORE_DATA_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_server_core_data_without_few_optional_fields() {
    let data = SERVER_CORE_DATA_TO_FLAGS.clone();
    let expected_buffer = SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_slice(), buf.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_server_core_data_with_all_optional_fields() {
    let core_data = SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS.clone();
    let expected_buffer = SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_BUFFER;

    let mut buf = Vec::new();
    core_data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_slice(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_core_data_without_optional_fields() {
    let data = SERVER_CORE_DATA.clone();
    let expected_buffer_len = SERVER_CORE_DATA_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_server_core_data_without_few_optional_fields() {
    let data = SERVER_CORE_DATA_TO_FLAGS.clone();
    let expected_buffer_len = SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_server_core_data_with_all_optional_fields() {
    let data = SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = SERVER_CORE_DATA_WITH_ALL_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_server_message_channel_data() {
    let buffer = ironrdp_pdu_samples::message_channel_data::SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER.as_ref();

    assert_eq!(
        ironrdp_pdu_samples::message_channel_data::SERVER_GCC_MESSAGE_CHANNEL_BLOCK,
        ServerMessageChannelData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_server_message_channel_data() {
    let data = ironrdp_pdu_samples::message_channel_data::SERVER_GCC_MESSAGE_CHANNEL_BLOCK.clone();
    let expected_buffer = ironrdp_pdu_samples::message_channel_data::SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_message_channel_data() {
    let data = ironrdp_pdu_samples::message_channel_data::SERVER_GCC_MESSAGE_CHANNEL_BLOCK.clone();
    let expected_buffer_len = ironrdp_pdu_samples::message_channel_data::SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_client_monitor_data_without_monitors() {
    let buffer = ironrdp_pdu_samples::monitor_data::MONITOR_DATA_WITHOUT_MONITORS_BUFFER.as_ref();

    assert_eq!(
        *ironrdp_pdu_samples::monitor_data::MONITOR_DATA_WITHOUT_MONITORS,
        ClientMonitorData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_monitor_data_with_monitors() {
    let buffer = ironrdp_pdu_samples::monitor_data::MONITOR_DATA_WITH_MONITORS_BUFFER.as_ref();

    assert_eq!(
        *ironrdp_pdu_samples::monitor_data::MONITOR_DATA_WITH_MONITORS,
        ClientMonitorData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_client_monitor_data_without_monitors() {
    let data = ironrdp_pdu_samples::monitor_data::MONITOR_DATA_WITHOUT_MONITORS.clone();
    let expected_buffer = ironrdp_pdu_samples::monitor_data::MONITOR_DATA_WITHOUT_MONITORS_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_client_monitor_data_with_monitors() {
    let data = ironrdp_pdu_samples::monitor_data::MONITOR_DATA_WITH_MONITORS.clone();
    let expected_buffer = ironrdp_pdu_samples::monitor_data::MONITOR_DATA_WITH_MONITORS_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_monitor_data_without_monitors() {
    let data = ironrdp_pdu_samples::monitor_data::MONITOR_DATA_WITHOUT_MONITORS.clone();
    let expected_buffer_len = ironrdp_pdu_samples::monitor_data::MONITOR_DATA_WITHOUT_MONITORS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_monitor_data_with_monitors() {
    let data = ironrdp_pdu_samples::monitor_data::MONITOR_DATA_WITH_MONITORS.clone();
    let expected_buffer_len = ironrdp_pdu_samples::monitor_data::MONITOR_DATA_WITH_MONITORS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_client_monitor_extended_data_without_monitors() {
    let buffer = ironrdp_pdu_samples::monitor_extended_data::MONITOR_DATA_WITHOUT_MONITORS_BUFFER.as_ref();

    assert_eq!(
        *ironrdp_pdu_samples::monitor_extended_data::MONITOR_DATA_WITHOUT_MONITORS,
        ClientMonitorExtendedData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_monitor_extended_data_with_monitors() {
    let buffer = ironrdp_pdu_samples::monitor_extended_data::MONITOR_DATA_WITH_MONITORS_BUFFER.as_ref();

    assert_eq!(
        *ironrdp_pdu_samples::monitor_extended_data::MONITOR_DATA_WITH_MONITORS,
        ClientMonitorExtendedData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_client_monitor_extended_data_without_monitors() {
    let data = ironrdp_pdu_samples::monitor_extended_data::MONITOR_DATA_WITHOUT_MONITORS.clone();
    let expected_buffer = ironrdp_pdu_samples::monitor_extended_data::MONITOR_DATA_WITHOUT_MONITORS_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_client_monitor_extended_data_with_monitors() {
    let data = ironrdp_pdu_samples::monitor_extended_data::MONITOR_DATA_WITH_MONITORS.clone();
    let expected_buffer = ironrdp_pdu_samples::monitor_extended_data::MONITOR_DATA_WITH_MONITORS_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_monitor_extended_data_without_monitors() {
    let data = ironrdp_pdu_samples::monitor_extended_data::MONITOR_DATA_WITHOUT_MONITORS.clone();
    let expected_buffer_len = ironrdp_pdu_samples::monitor_extended_data::MONITOR_DATA_WITHOUT_MONITORS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_monitor_extended_data_with_monitors() {
    let data = ironrdp_pdu_samples::monitor_extended_data::MONITOR_DATA_WITH_MONITORS.clone();
    let expected_buffer_len = ironrdp_pdu_samples::monitor_extended_data::MONITOR_DATA_WITH_MONITORS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_server_multi_transport_channel_data() {
    let buffer =
        ironrdp_pdu_samples::multi_transport_channel_data::SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER.as_ref();

    assert_eq!(
        *ironrdp_pdu_samples::multi_transport_channel_data::SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK,
        MultiTransportChannelData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_server_multi_transport_channel_data() {
    let data = ironrdp_pdu_samples::multi_transport_channel_data::SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK.clone();
    let expected_buffer =
        ironrdp_pdu_samples::multi_transport_channel_data::SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_multi_transport_channel_data() {
    let data = ironrdp_pdu_samples::multi_transport_channel_data::SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK.clone();
    let expected_buffer_len =
        ironrdp_pdu_samples::multi_transport_channel_data::SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_client_network_data_without_channels() {
    let buffer = CLIENT_NETWORK_DATA_WITHOUT_CHANNELS_BUFFER.as_ref();

    assert_eq!(
        *CLIENT_NETWORK_DATA_WITHOUT_CHANNELS,
        ClientNetworkData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_client_network_data_with_channels() {
    let buffer = CLIENT_NETWORK_DATA_WITH_CHANNELS_BUFFER.as_ref();

    assert_eq!(
        *CLIENT_NETWORK_DATA_WITH_CHANNELS,
        ClientNetworkData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_client_network_data_without_channels() {
    let data = CLIENT_NETWORK_DATA_WITHOUT_CHANNELS.clone();
    let expected_buffer = CLIENT_NETWORK_DATA_WITHOUT_CHANNELS_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_client_network_data_with_channels() {
    let data = CLIENT_NETWORK_DATA_WITH_CHANNELS.clone();
    let expected_buffer = CLIENT_NETWORK_DATA_WITH_CHANNELS_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_network_data_without_channels() {
    let data = CLIENT_NETWORK_DATA_WITHOUT_CHANNELS.clone();
    let expected_buffer_len = CLIENT_NETWORK_DATA_WITHOUT_CHANNELS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_client_network_data_with_channels() {
    let data = CLIENT_NETWORK_DATA_WITH_CHANNELS.clone();
    let expected_buffer_len = CLIENT_NETWORK_DATA_WITH_CHANNELS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_server_network_data_without_channels_id() {
    let buffer = SERVER_NETWORK_DATA_WITHOUT_CHANNELS_ID_BUFFER.as_ref();

    assert_eq!(
        *SERVER_NETWORK_DATA_WITHOUT_CHANNELS_ID,
        ServerNetworkData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_server_network_data_with_channels_id() {
    let buffer = SERVER_NETWORK_DATA_WITH_CHANNELS_ID_BUFFER.as_ref();

    assert_eq!(
        *SERVER_NETWORK_DATA_WITH_CHANNELS_ID,
        ServerNetworkData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_server_network_data_without_channels_id() {
    let data = SERVER_NETWORK_DATA_WITHOUT_CHANNELS_ID.clone();
    let expected_buffer = SERVER_NETWORK_DATA_WITHOUT_CHANNELS_ID_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_server_network_data_with_channels_id() {
    let data = SERVER_NETWORK_DATA_WITH_CHANNELS_ID.clone();
    let expected_buffer = SERVER_NETWORK_DATA_WITH_CHANNELS_ID_BUFFER;

    let mut buf = Vec::new();
    data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_server_network_data_without_channels_id() {
    let data = SERVER_NETWORK_DATA_WITHOUT_CHANNELS_ID.clone();
    let expected_buffer_len = SERVER_NETWORK_DATA_WITHOUT_CHANNELS_ID_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_server_network_data_with_channels_id() {
    let data = SERVER_NETWORK_DATA_WITH_CHANNELS_ID.clone();
    let expected_buffer_len = SERVER_NETWORK_DATA_WITH_CHANNELS_ID_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_client_security_data() {
    let buffer = CLIENT_SECURITY_DATA_BUFFER.as_ref();

    assert_eq!(*CLIENT_SECURITY_DATA, ClientSecurityData::from_buffer(buffer).unwrap());
}

#[test]
fn to_buffer_correctly_serializes_client_security_data() {
    let security_data = CLIENT_SECURITY_DATA.clone();
    let expected_buffer = CLIENT_SECURITY_DATA_BUFFER;

    let mut buf = Vec::new();
    security_data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_security_data() {
    let data = CLIENT_SECURITY_DATA.clone();
    let expected_buffer_len = CLIENT_SECURITY_DATA_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_server_security_data_without_optional_fields() {
    let buffer = SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS_BUFFER.as_ref();

    assert_eq!(
        *SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS,
        ServerSecurityData::from_buffer(buffer).unwrap()
    );
}

#[test]
fn from_buffer_correctly_parses_server_security_data_with_all_fields() {
    let buffer = SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_BUFFER;

    assert_eq!(
        *SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS,
        ServerSecurityData::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn from_buffer_server_security_data_fails_with_invalid_server_random_length() {
    let buffer = SERVER_SECURITY_DATA_WITH_INVALID_SERVER_RANDOM_BUFFER;

    match ServerSecurityData::from_buffer(buffer.as_slice()) {
        Err(SecurityDataError::InvalidServerRandomLen(_)) => (),
        res => panic!("Expected the invalid server random length error, got: {res:?}"),
    };
}

#[test]
fn to_buffer_correctly_serializes_server_security_data_without_optional_fields() {
    let security_data = SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer = SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS_BUFFER;

    let mut buf = Vec::new();
    security_data.to_buffer(&mut buf).unwrap();

    assert_eq!(expected_buffer.as_ref(), buf.as_slice());
}

#[test]
fn to_buffer_correctly_serializes_server_security_data_with_optional_fields() {
    let mut buf = Vec::new();
    SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS.to_buffer(&mut buf).unwrap();
    assert_eq!(buf, SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_BUFFER.as_slice());
}

#[test]
fn to_buffer_server_security_data_fails_on_mismatch_of_required_and_optional_fields() {
    let security_data = SERVER_SECURITY_DATA_WITH_MISMATCH_OF_REQUIRED_AND_OPTIONAL_FIELDS.clone();

    let mut buf = Vec::new();
    match security_data.to_buffer(&mut buf) {
        Err(SecurityDataError::InvalidInput(_)) => (),
        res => panic!("Expected the invalid input error, got: {res:?}"),
    };
}

#[test]
fn buffer_length_is_correct_for_server_security_data_without_optional_fields() {
    let data = SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = SERVER_SECURITY_DATA_WITHOUT_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn buffer_length_is_correct_for_server_security_data_with_optional_fields() {
    let data = SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS.clone();
    let expected_buffer_len = SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}

#[test]
fn from_buffer_correctly_parses_conference_create_request() {
    assert_eq!(
        *CONFERENCE_CREATE_REQUEST,
        ConferenceCreateRequest::from_buffer(CONFERENCE_CREATE_REQUEST_BUFFER.as_slice()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_conference_create_request() {
    let mut buf = Vec::new();
    CONFERENCE_CREATE_REQUEST.to_buffer(&mut buf).unwrap();
    assert_eq!(buf.as_slice(), CONFERENCE_CREATE_REQUEST_BUFFER.as_slice());
}

#[test]
fn buffer_length_is_correct_for_conference_create_request() {
    let len = CONFERENCE_CREATE_REQUEST.buffer_length();
    assert_eq!(len, CONFERENCE_CREATE_REQUEST_BUFFER.len());
}

#[test]
fn from_buffer_correctly_parses_conference_create_response() {
    let buffer = CONFERENCE_CREATE_RESPONSE_BUFFER;

    assert_eq!(
        *CONFERENCE_CREATE_RESPONSE,
        ConferenceCreateResponse::from_buffer(buffer.as_slice()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_conference_create_response() {
    let data = CONFERENCE_CREATE_RESPONSE.clone();
    let expected_buffer = CONFERENCE_CREATE_RESPONSE_BUFFER;

    let mut buff = Vec::new();
    data.to_buffer(&mut buff).unwrap();

    assert_eq!(expected_buffer.as_slice(), buff.as_slice());
}

#[test]
fn buffer_length_is_correct_for_conference_create_response() {
    let data = CONFERENCE_CREATE_RESPONSE.clone();
    let expected_buffer_len = CONFERENCE_CREATE_RESPONSE_BUFFER.len();

    let len = data.buffer_length();

    assert_eq!(expected_buffer_len, len);
}
