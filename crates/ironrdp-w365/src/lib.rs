#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![no_std]

extern crate alloc;

use ironrdp_core::{DecodeResult, ReadCursor, ensure_size, invalid_field_err};

pub mod battery;
pub mod power_request;
pub mod screen_capture;
pub mod vdcswitch;

/// Protocol version every channel in this crate offers and accepts.
pub const PROTOCOL_VERSION_1: u32 = 0x0001_0000;

/// Size of the battery status and power request PDU header: a 16-bit type and a 32-bit length that counts the whole PDU.
const HEADER_SIZE: usize = 2 /* PduType */ + 4 /* PduLength */;

/// Decodes a battery status or power request PDU header.
///
/// Returns the PDU type and a cursor over the rest of the PDU, and consumes exactly `pduLength` bytes from `src`.
fn decode_header<'de>(src: &mut ReadCursor<'de>, context: &'static str) -> DecodeResult<(u16, ReadCursor<'de>)> {
    ensure_size!(ctx: context, in: src, size: HEADER_SIZE);
    let pdu_type = src.read_u16();
    let pdu_length = usize::try_from(src.read_u32()).unwrap_or(usize::MAX);
    if !(HEADER_SIZE <= pdu_length && pdu_length - HEADER_SIZE <= src.len()) {
        return Err(invalid_field_err!(
            context,
            "pduLength",
            "PDU length does not fit its payload",
            in: src
        ));
    }
    Ok((pdu_type, ReadCursor::new(src.read_slice(pdu_length - HEADER_SIZE))))
}
