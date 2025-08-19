use array_concat::{concat_arrays, concat_arrays_size};
use ironrdp_pdu::gcc::{ClientGccBlocks, ClientGccType, ServerGccBlocks, ServerGccType};
use lazy_static::lazy_static;

use crate::cluster_data::{CLUSTER_DATA, CLUSTER_DATA_BUFFER};
use crate::core_data::{
    CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL,
    CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL_BUFFER, SERVER_CORE_DATA_TO_FLAGS,
    SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER,
};
use crate::message_channel_data::SERVER_GCC_MESSAGE_CHANNEL_BLOCK;
use crate::multi_transport_channel_data::SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK;
use crate::network_data::{
    CLIENT_NETWORK_DATA_WITH_CHANNELS, CLIENT_NETWORK_DATA_WITH_CHANNELS_BUFFER, SERVER_NETWORK_DATA_WITH_CHANNELS_ID,
    SERVER_NETWORK_DATA_WITH_CHANNELS_ID_BUFFER,
};
use crate::security_data::{
    CLIENT_SECURITY_DATA, CLIENT_SECURITY_DATA_BUFFER, SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS,
    SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_BUFFER,
};

const USER_HEADER_LEN: usize = 4;

const fn gcc_block_size<const N: usize>(_: [u8; N]) -> usize {
    N + USER_HEADER_LEN
}

const fn make_gcc_block_buffer<const N: usize>(data_type: u16, buffer: &[u8]) -> [u8; N] {
    const fn copy_slice<const N: usize>(src: &[u8], mut dst: [u8; N], offset: usize) -> [u8; N] {
        let mut i = src.len();
        while i > 0 {
            i -= 1;
            dst[i + offset] = src[i];
        }
        dst
    }

    if N != buffer.len() + USER_HEADER_LEN {
        panic!("invalid output array len");
    }

    let array = copy_slice(&data_type.to_le_bytes(), [0; N], 0);

    let length = (buffer.len() + USER_HEADER_LEN) as u16;
    let array = copy_slice(&length.to_le_bytes(), array, 2);

    copy_slice(buffer, array, 4)
}

pub const CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER: [u8; concat_arrays_size!(
    CLIENT_GCC_CORE_BLOCK_BUFFER,
    CLIENT_GCC_SECURITY_BLOCK_BUFFER,
    CLIENT_GCC_NETWORK_BLOCK_BUFFER
)] = concat_arrays!(
    CLIENT_GCC_CORE_BLOCK_BUFFER,
    CLIENT_GCC_SECURITY_BLOCK_BUFFER,
    CLIENT_GCC_NETWORK_BLOCK_BUFFER
);

pub const CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER: [u8; concat_arrays_size!(
    CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER,
    CLIENT_GCC_CLUSTER_BLOCK_BUFFER
)] = concat_arrays!(
    CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER,
    CLIENT_GCC_CLUSTER_BLOCK_BUFFER
);

pub const CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS_BUFFER: [u8; concat_arrays_size!(
    CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER,
    CLIENT_GCC_MONITOR_BLOCK_BUFFER,
    CLIENT_GCC_MONITOR_EXTENDED_BLOCK_BUFFER
)] = concat_arrays!(
    CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD_BUFFER,
    CLIENT_GCC_MONITOR_BLOCK_BUFFER,
    CLIENT_GCC_MONITOR_EXTENDED_BLOCK_BUFFER
);

pub const CLIENT_GCC_WITH_OPTIONAL_FIELDS_IN_DIFFERENT_ORDER_BUFFER: [u8; concat_arrays_size!(
    CLIENT_GCC_CORE_BLOCK_BUFFER,
    CLIENT_GCC_CLUSTER_BLOCK_BUFFER,
    CLIENT_GCC_SECURITY_BLOCK_BUFFER,
    CLIENT_GCC_MONITOR_BLOCK_BUFFER,
    CLIENT_GCC_NETWORK_BLOCK_BUFFER,
    CLIENT_GCC_MONITOR_EXTENDED_BLOCK_BUFFER
)] = concat_arrays!(
    CLIENT_GCC_CORE_BLOCK_BUFFER,
    CLIENT_GCC_CLUSTER_BLOCK_BUFFER,
    CLIENT_GCC_SECURITY_BLOCK_BUFFER,
    CLIENT_GCC_MONITOR_BLOCK_BUFFER,
    CLIENT_GCC_NETWORK_BLOCK_BUFFER,
    CLIENT_GCC_MONITOR_EXTENDED_BLOCK_BUFFER
);

pub const SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER: [u8; concat_arrays_size!(
    SERVER_GCC_CORE_BLOCK_BUFFER,
    SERVER_GCC_NETWORK_BLOCK_BUFFER,
    SERVER_GCC_SECURITY_BLOCK_BUFFER
)] = concat_arrays!(
    SERVER_GCC_CORE_BLOCK_BUFFER,
    SERVER_GCC_NETWORK_BLOCK_BUFFER,
    SERVER_GCC_SECURITY_BLOCK_BUFFER
);

pub const SERVER_GCC_WITH_OPTIONAL_FIELDS_BUFFER: [u8; concat_arrays_size!(
    SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER,
    SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER,
    SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER
)] = concat_arrays!(
    SERVER_GCC_WITHOUT_OPTIONAL_FIELDS_BUFFER,
    SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER,
    SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER
);

pub const SERVER_GCC_WITH_OPTIONAL_FIELDS_IN_DIFFERENT_ORDER_BUFFER: [u8; concat_arrays_size!(
    SERVER_GCC_CORE_BLOCK_BUFFER,
    SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER,
    SERVER_GCC_NETWORK_BLOCK_BUFFER,
    SERVER_GCC_SECURITY_BLOCK_BUFFER,
    SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER
)] = concat_arrays!(
    SERVER_GCC_CORE_BLOCK_BUFFER,
    SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER,
    SERVER_GCC_NETWORK_BLOCK_BUFFER,
    SERVER_GCC_SECURITY_BLOCK_BUFFER,
    SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER
);

lazy_static! {
    pub static ref CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS: ClientGccBlocks = ClientGccBlocks {
        core: CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL.clone(),
        security: CLIENT_SECURITY_DATA.clone(),
        network: Some(CLIENT_NETWORK_DATA_WITH_CHANNELS.clone()),
        cluster: None,
        monitor: None,
        message_channel: None,
        multi_transport_channel: None,
        monitor_extended: None,
    };
    pub static ref CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD: ClientGccBlocks = {
        let mut data = CLIENT_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
        data.cluster = Some(CLUSTER_DATA.clone());
        data
    };
    pub static ref CLIENT_GCC_WITH_ALL_OPTIONAL_FIELDS: ClientGccBlocks = {
        let mut data = CLIENT_GCC_WITH_CLUSTER_OPTIONAL_FIELD.clone();
        data.monitor = Some(crate::monitor_data::MONITOR_DATA_WITH_MONITORS.clone());
        data.monitor_extended = Some(crate::monitor_extended_data::MONITOR_DATA_WITH_MONITORS.clone());
        data
    };
    pub static ref SERVER_GCC_WITHOUT_OPTIONAL_FIELDS: ServerGccBlocks = ServerGccBlocks {
        core: SERVER_CORE_DATA_TO_FLAGS.clone(),
        network: SERVER_NETWORK_DATA_WITH_CHANNELS_ID.clone(),
        security: SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS.clone(),
        message_channel: None,
        multi_transport_channel: None,
    };
    pub static ref SERVER_GCC_WITH_OPTIONAL_FIELDS: ServerGccBlocks = {
        let mut data = SERVER_GCC_WITHOUT_OPTIONAL_FIELDS.clone();
        data.message_channel = Some(SERVER_GCC_MESSAGE_CHANNEL_BLOCK.clone());
        data.multi_transport_channel = Some(SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK.clone());
        data
    };
}

pub const CLIENT_GCC_CORE_BLOCK_BUFFER: [u8; gcc_block_size(
    CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL_BUFFER,
)] = make_gcc_block_buffer(
    ClientGccType::CoreData as u16,
    &CLIENT_OPTIONAL_CORE_DATA_TO_SERVER_SELECTED_PROTOCOL_BUFFER,
);

pub const CLIENT_GCC_SECURITY_BLOCK_BUFFER: [u8; gcc_block_size(CLIENT_SECURITY_DATA_BUFFER)] =
    make_gcc_block_buffer(ClientGccType::SecurityData as u16, &CLIENT_SECURITY_DATA_BUFFER);

pub const CLIENT_GCC_NETWORK_BLOCK_BUFFER: [u8; gcc_block_size(CLIENT_NETWORK_DATA_WITH_CHANNELS_BUFFER)] =
    make_gcc_block_buffer(
        ClientGccType::NetworkData as u16,
        &CLIENT_NETWORK_DATA_WITH_CHANNELS_BUFFER,
    );

pub const CLIENT_GCC_CLUSTER_BLOCK_BUFFER: [u8; gcc_block_size(CLUSTER_DATA_BUFFER)] =
    make_gcc_block_buffer(ClientGccType::ClusterData as u16, &CLUSTER_DATA_BUFFER);

pub const CLIENT_GCC_MONITOR_BLOCK_BUFFER: [u8; gcc_block_size(
    crate::monitor_data::MONITOR_DATA_WITH_MONITORS_BUFFER,
)] = make_gcc_block_buffer(
    ClientGccType::MonitorData as u16,
    &crate::monitor_data::MONITOR_DATA_WITH_MONITORS_BUFFER,
);

pub const CLIENT_GCC_MONITOR_EXTENDED_BLOCK_BUFFER: [u8; gcc_block_size(
    crate::monitor_extended_data::MONITOR_DATA_WITH_MONITORS_BUFFER,
)] = make_gcc_block_buffer(
    ClientGccType::MonitorExtendedData as u16,
    &crate::monitor_extended_data::MONITOR_DATA_WITH_MONITORS_BUFFER,
);

pub const SERVER_GCC_CORE_BLOCK_BUFFER: [u8; gcc_block_size(SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER)] =
    make_gcc_block_buffer(
        ServerGccType::CoreData as u16,
        &SERVER_CORE_DATA_TO_REQUESTED_PROTOCOL_BUFFER,
    );

pub const SERVER_GCC_NETWORK_BLOCK_BUFFER: [u8; gcc_block_size(SERVER_NETWORK_DATA_WITH_CHANNELS_ID_BUFFER)] =
    make_gcc_block_buffer(
        ServerGccType::NetworkData as u16,
        &SERVER_NETWORK_DATA_WITH_CHANNELS_ID_BUFFER,
    );

pub const SERVER_GCC_SECURITY_BLOCK_BUFFER: [u8; gcc_block_size(SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_BUFFER)] =
    make_gcc_block_buffer(
        ServerGccType::SecurityData as u16,
        &SERVER_SECURITY_DATA_WITH_OPTIONAL_FIELDS_BUFFER,
    );

pub const SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER: [u8; gcc_block_size(
    crate::message_channel_data::SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER,
)] = make_gcc_block_buffer(
    ServerGccType::MessageChannelData as u16,
    &crate::message_channel_data::SERVER_GCC_MESSAGE_CHANNEL_BLOCK_BUFFER,
);

pub const SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER: [u8; gcc_block_size(
    crate::multi_transport_channel_data::SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER,
)] = make_gcc_block_buffer(
    ServerGccType::MultiTransportChannelData as u16,
    &crate::multi_transport_channel_data::SERVER_GCC_MULTI_TRANSPORT_CHANNEL_BLOCK_BUFFER,
);
