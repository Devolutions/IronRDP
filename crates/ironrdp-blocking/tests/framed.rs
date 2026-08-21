#![allow(unused_crate_dependencies)]

use std::io;
use std::io::Cursor;

use ironrdp_blocking::Framed;
use ironrdp_core::DecodeResult;
use ironrdp_pdu::{PduHint, X224_HINT};

#[derive(Debug)]
struct ZeroSizeHint;

impl PduHint for ZeroSizeHint {
    fn find_size(&self, _bytes: &[u8]) -> DecodeResult<Option<(bool, usize)>> {
        Ok(Some((true, 0)))
    }
}

fn tpkt(payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(4 + payload.len()).expect("frame fits");
    let mut frame = vec![0x03, 0x00];
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

#[test]
fn zero_size_hint_fails_before_reading() {
    let mut framed = Framed::new(Cursor::new(tpkt(&[0xAA; 8])));

    let error = framed.read_by_hint(&ZeroSizeHint).expect_err("zero PDU size must fail");

    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let (stream, leftover) = framed.into_inner();
    assert_eq!(stream.position(), 0, "the stream must not be read");
    assert!(leftover.is_empty());
}

#[test]
fn leftover_exposes_buffered_bytes_read_only() {
    let carried_frame = tpkt(&[0xDD; 8]);
    let mut chunk = tpkt(&[0xAA; 8]);
    chunk.extend_from_slice(&carried_frame);
    let mut framed = Framed::new(Cursor::new(chunk));

    framed.read_by_hint(&X224_HINT).expect("first frame");

    let (_, leftover) = framed.into_inner();
    assert_eq!(leftover.as_bytes(), carried_frame);
}
