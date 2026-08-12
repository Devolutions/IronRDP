//! Helpers for reading CTAP integer-keyed CBOR maps.

use ironrdp_rdpewa::pdu::{CborKey, CborValue};

pub(crate) fn map_get<'a>(map: &'a std::collections::BTreeMap<CborKey, CborValue>, key: i64) -> Option<&'a CborValue> {
    map.get(&CborKey::Int(key))
}

pub(crate) fn map_get_text<'a>(
    map: &'a std::collections::BTreeMap<CborKey, CborValue>,
    key: &str,
) -> Option<&'a CborValue> {
    map.get(&CborKey::Text(key.to_owned()))
}

pub(crate) fn text_field(map: &std::collections::BTreeMap<CborKey, CborValue>, key: &str) -> Option<String> {
    map_get_text(map, key).and_then(|v| v.as_text().ok()).map(str::to_owned)
}

pub(crate) fn bytes_field(map: &std::collections::BTreeMap<CborKey, CborValue>, key: &str) -> Option<Vec<u8>> {
    map_get_text(map, key)
        .and_then(|v| v.as_bytes().ok())
        .map(|b| b.to_vec())
}
