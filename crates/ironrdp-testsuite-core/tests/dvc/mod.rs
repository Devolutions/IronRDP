use ironrdp_core::{ReadCursor, WriteCursor};
use ironrdp_dvc::pdu::ClosePdu;
use ironrdp_dvc::pdu::DataPdu;
use ironrdp_dvc::pdu::{CapabilitiesRequestPdu, CapabilitiesResponsePdu, CapsVersion};
use ironrdp_dvc::pdu::{CreateRequestPdu, CreateResponsePdu, CreationStatus};
use ironrdp_dvc::pdu::{DataFirstPdu, FieldType};
use ironrdp_dvc::pdu::{DrdynvcClientPdu, DrdynvcDataPdu, DrdynvcServerPdu};
use ironrdp_pdu::Decode;
use ironrdp_pdu::Encode;
use std::sync::OnceLock;

// TODO: This likely generalizes to many tests and can thus be reused outside of this module.
fn test_encodes<T: Encode>(data: &T, expected: &[u8]) {
    let mut buffer = vec![0x00; data.size()];
    let mut cursor = WriteCursor::new(&mut buffer);
    data.encode(&mut cursor).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

// TODO: This likely generalizes to many tests and can thus be reused outside of this module.
fn test_decodes<'a, T: Decode<'a> + PartialEq + std::fmt::Debug>(encoded: &'a [u8], expected: &T) {
    let mut src = ReadCursor::new(encoded);
    assert_eq!(*expected, T::decode(&mut src).unwrap());
}

mod capabilities;
mod close;
mod create;
mod data;
mod data_first;
