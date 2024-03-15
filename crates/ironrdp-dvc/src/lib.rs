#![cfg_attr(not(feature = "std"), no_std)]
#![allow(unused)] // FIXME(#61): remove this annotation

// TODO: this crate is WIP

#[macro_use]
extern crate tracing;

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

// Re-export ironrdp_pdu crate for convenience
#[rustfmt::skip] // do not re-order this pub use
pub use ironrdp_pdu as pdu;
use ironrdp_svc::{self, impl_as_any, AsAny, SvcMessage};
use pdu::dvc::gfx::ServerPdu;
use pdu::dvc::{self, DataFirstPdu, DataPdu};
use pdu::write_buf::WriteBuf;
use pdu::{assert_obj_safe, cast_length, custom_err, encode_vec, other_err, PduEncode, PduParsing as _, PduResult};

mod complete_data;
use complete_data::CompleteData;

mod client;
pub use client::*;

mod server;
pub use server::*;

pub type DvcMessages = Vec<Box<dyn PduEncode + Send>>;

/// A type that is a Dynamic Virtual Channel (DVC)
///
/// Dynamic virtual channels may be created at any point during the RDP session.
/// The Dynamic Virtual Channel APIs exist to address limitations of Static Virtual Channels:
///   - Limited number of channels
///   - Packet reconstruction
pub trait DvcProcessor: AsAny + Send + Sync {
    /// The name of the channel, e.g. "Microsoft::Windows::RDS::DisplayControl"
    fn channel_name(&self) -> &str;

    /// The ID of the channel. Optional because
    /// ID's are assigned dynamically by the server.
    fn id(&self) -> Option<u32>;

    /// Sets the ID of the channel.
    fn set_id(&mut self, id: u32);

    /// Returns any messages that should be sent immediately
    /// upon the channel being created.
    fn start(&mut self, _channel_id: u32) -> PduResult<DvcMessages>;

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<DvcMessages>;

    fn close(&mut self, _channel_id: u32) {}
}

assert_obj_safe!(DvcProcessor);

const DATA_MAX_SIZE: usize = 1590;

pub(crate) fn encode_dvc_messages(
    channel_id: u32,
    messages: DvcMessages,
    flags: Option<ironrdp_svc::ChannelFlags>,
) -> PduResult<Vec<SvcMessage>> {
    let mut res = Vec::new();
    for msg in messages {
        let total_size = msg.size();

        let msg = encode_vec(msg.as_ref())?;
        let mut off = 0;

        while off < total_size {
            let rem = total_size.checked_sub(off).unwrap();
            let size = core::cmp::min(rem, DATA_MAX_SIZE);

            let pdu = if off == 0 && total_size >= DATA_MAX_SIZE {
                let total_size = cast_length!("encode_dvc_data", "totalDataSize", total_size)?;
                dvc::CommonPdu::DataFirst(dvc::DataFirstPdu::new(channel_id, total_size, DATA_MAX_SIZE))
            } else {
                dvc::CommonPdu::Data(dvc::DataPdu::new(channel_id, size))
            };

            let end = off
                .checked_add(size)
                .ok_or_else(|| other_err!("encode_dvc_data", "overflow occurred"))?;
            let mut data = Vec::new();
            pdu.to_buffer(&mut data)
                .map_err(|e| custom_err!("encode_dvc_messages", e))?;
            data.extend_from_slice(&msg[off..end]);
            let mut svc = SvcMessage::from(data);
            if let Some(flags) = flags {
                svc = svc.with_flags(flags);
            }
            res.push(svc);
            off = end;
        }
    }

    Ok(res)
}

pub struct DisplayControlClient {
    id: Option<u32>,
}

impl_as_any!(DisplayControlClient);

impl DvcProcessor for DisplayControlClient {
    fn channel_name(&self) -> &str {
        dvc::display::CHANNEL_NAME
    }

    fn id(&self) -> Option<u32> {
        self.id
    }

    fn set_id(&mut self, id: u32) {
        self.id = Some(id);
    }

    fn start(&mut self, _channel_id: u32) -> PduResult<DvcMessages> {
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<DvcMessages> {
        // TODO: We can parse the payload here for completeness sake,
        // in practice we don't need to do anything with the payload.
        debug!("Got Display PDU of length: {}", payload.len());
        Ok(Vec::new())
    }
}

impl DvcClientProcessor for DisplayControlClient {}

impl DisplayControlClient {
    pub fn new() -> Self {
        Self { id: None }
    }

    pub fn encode_monitors(&self, monitors: Vec<dvc::display::Monitor>) -> PduResult<Vec<SvcMessage>> {
        if self.id.is_none() {
            return Err(other_err!("encode_monitors", "channel id is not set"));
        }
        let mut buf = WriteBuf::new();
        let pdu = dvc::display::ClientPdu::DisplayControlMonitorLayout(dvc::display::MonitorLayoutPdu { monitors });
        encode_dvc_messages(self.id.unwrap(), vec![Box::new(pdu)], None)
    }
}

impl Default for DisplayControlClient {
    fn default() -> Self {
        Self::new()
    }
}
