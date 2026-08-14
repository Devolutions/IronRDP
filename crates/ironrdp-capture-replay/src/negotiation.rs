use ironrdp_core::decode;
use ironrdp_pdu::gcc::ChannelName;
use ironrdp_pdu::mcs::{self, McsMessage};
use ironrdp_pdu::rdp::capability_sets::CapabilitySet;
use ironrdp_pdu::rdp::headers::{self, ShareControlPdu};
use ironrdp_pdu::x224::{X224, X224Data};

use crate::transport::flatten;
use crate::{Plaintext, ReplayError};

/// A captured RDP static virtual channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticChannel {
    /// Channel name supplied by the client.
    pub name: ChannelName,
    /// Channel ID assigned by the server.
    pub id: u16,
}

/// RDP state reconstructed from the recorded connection sequence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NegotiatedState {
    /// Client MCS user channel.
    pub user_channel_id: u16,
    /// Server I/O channel.
    pub io_channel_id: u16,
    /// Optional MCS message channel.
    pub message_channel_id: Option<u16>,
    /// RDP share ID.
    pub share_id: u32,
    /// Recorded desktop width.
    pub width: u16,
    /// Recorded desktop height.
    pub height: u16,
    /// Static channels paired with their captured IDs.
    pub static_channels: Vec<StaticChannel>,
}

/// Recover the RDP connection state required to interpret recorded streams.
pub fn recover_negotiated_state(plaintext: &Plaintext) -> Result<NegotiatedState, ReplayError> {
    let client_bytes = flatten(&plaintext.client);
    let server_bytes = flatten(&plaintext.server);
    let client_connect = connect_initial(&client_bytes).ok_or(ReplayError::MissingRdpState)?;
    let server_connect = connect_response(&server_bytes).ok_or(ReplayError::MissingRdpState)?;
    let client_gcc = client_connect.conference_create_request.gcc_blocks();
    let server_gcc = server_connect.conference_create_response.gcc_blocks();
    let io_channel_id = server_gcc.network.io_channel;
    let channel_ids = &server_gcc.network.channel_ids;
    let names = client_gcc.network.as_ref().map_or_else(Vec::new, |network| {
        network.channels.iter().map(|channel| channel.name.clone()).collect()
    });
    let static_channels = static_channels(names, channel_ids)?;
    let user_channel_id = first_user_channel(&client_bytes)
        .or_else(|| first_user_channel(&server_bytes))
        .ok_or(ReplayError::MissingUserChannel)?;
    let (share_id, desktop_size) =
        first_demand_active(&server_bytes, io_channel_id).ok_or(ReplayError::MissingShareId)?;
    let (width, height) = desktop_size.unwrap_or((client_gcc.core.desktop_width, client_gcc.core.desktop_height));

    Ok(NegotiatedState {
        user_channel_id,
        io_channel_id,
        message_channel_id: server_gcc
            .message_channel
            .as_ref()
            .map(|channel| channel.mcs_message_channel_id),
        share_id,
        width,
        height,
        static_channels,
    })
}

fn static_channels(names: Vec<ChannelName>, channel_ids: &[u16]) -> Result<Vec<StaticChannel>, ReplayError> {
    if names.len() != channel_ids.len() {
        return Err(ReplayError::MissingChannelMap);
    }

    Ok(names
        .into_iter()
        .zip(channel_ids.iter().copied())
        .map(|(name, id)| StaticChannel { name, id })
        .collect())
}

fn connect_initial(bytes: &[u8]) -> Option<mcs::ConnectInitial> {
    tpkt_frames(bytes).find_map(|frame| {
        let payload = decode::<X224<X224Data<'_>>>(frame).ok()?.0;
        decode::<mcs::ConnectInitial>(payload.data.as_ref()).ok()
    })
}

fn connect_response(bytes: &[u8]) -> Option<mcs::ConnectResponse> {
    tpkt_frames(bytes).find_map(|frame| {
        let payload = decode::<X224<X224Data<'_>>>(frame).ok()?.0;
        decode::<mcs::ConnectResponse>(payload.data.as_ref()).ok()
    })
}

fn first_user_channel(bytes: &[u8]) -> Option<u16> {
    tpkt_frames(bytes).find_map(|frame| match decode::<X224<McsMessage<'_>>>(frame).ok()?.0 {
        McsMessage::AttachUserConfirm(message) => Some(message.initiator_id),
        McsMessage::ChannelJoinRequest(message) => Some(message.initiator_id),
        McsMessage::SendDataRequest(message) => Some(message.initiator_id),
        _ => None,
    })
}

fn first_demand_active(bytes: &[u8], io_channel_id: u16) -> Option<(u32, Option<(u16, u16)>)> {
    tpkt_frames(bytes).find_map(|frame| {
        let share_control = headers::decode_share_control(mcs::decode_send_data_indication(frame).ok()?).ok()?;
        if share_control.channel_id != io_channel_id {
            return None;
        }
        let ShareControlPdu::ServerDemandActive(demand_active) = share_control.pdu else {
            return None;
        };
        let desktop_size = demand_active
            .pdu
            .capability_sets
            .iter()
            .find_map(|capability_set| match capability_set {
                CapabilitySet::Bitmap(bitmap) => Some((bitmap.desktop_width, bitmap.desktop_height)),
                _ => None,
            });

        Some((share_control.share_id, desktop_size))
    })
}

fn tpkt_frames(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut offset = 0;
    core::iter::from_fn(move || {
        while offset + 4 <= bytes.len() {
            if bytes[offset] == 3 && bytes[offset + 1] == 0 {
                let length = usize::from(u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]));
                let end = offset.checked_add(length)?;
                if length >= 7 && end <= bytes.len() {
                    let frame = &bytes[offset..end];
                    offset = end;
                    return Some(frame);
                }
            }
            offset += 1;
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use ironrdp_core::encode_vec;

    use super::*;

    #[test]
    fn finds_the_mcs_user_channel() {
        let frame = encode_vec(&X224(mcs::AttachUserConfirm {
            result: 0,
            initiator_id: 1_005,
        }))
        .unwrap();

        assert_eq!(first_user_channel(&frame), Some(1_005));
    }

    #[test]
    fn rejects_streams_without_connect_sequence() {
        let error = recover_negotiated_state(&Plaintext {
            client: vec![(1, vec![3, 0, 0, 7, 2, 0xe0, 0])],
            server: Vec::new(),
        })
        .unwrap_err();

        assert!(matches!(error, ReplayError::MissingRdpState));
    }

    #[test]
    fn rejects_mismatched_static_channel_maps() {
        assert!(matches!(
            static_channels(Vec::new(), &[1]),
            Err(ReplayError::MissingChannelMap)
        ));
    }
}
