use std::sync::OnceLock;

use ironrdp_core::{Decode, Encode, ReadCursor, WriteCursor};
use ironrdp_dvc::pdu::{
    CapabilitiesRequestPdu, CapabilitiesResponsePdu, CapsVersion, ClosePdu, CreateRequestPdu, CreateResponsePdu,
    CreationStatus, DataFirstPdu, DataPdu, DrdynvcClientPdu, DrdynvcDataPdu, DrdynvcServerPdu, FieldType,
};

// TODO: This likely generalizes to many tests and can thus be reused outside of this module.
fn test_encodes<T: Encode>(data: &T, expected: &[u8]) {
    let mut buffer = vec![0x00; data.size()];
    let mut cursor = WriteCursor::new(&mut buffer);
    data.encode(&mut cursor).unwrap();
    assert_eq!(expected, buffer.as_slice());
}

// TODO: This likely generalizes to many tests and can thus be reused outside of this module.
fn test_decodes<'a, T: Decode<'a> + PartialEq + core::fmt::Debug>(encoded: &'a [u8], expected: &T) {
    let mut src = ReadCursor::new(encoded);
    assert_eq!(*expected, T::decode(&mut src).unwrap());
}

mod capabilities;
mod close;
mod create;
mod data;
mod data_first;
