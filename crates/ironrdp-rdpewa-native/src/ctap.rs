//! CTAP request parsing and response encoding for MS-RDPEWA WEB_AUTHN bodies.

use std::collections::BTreeMap;

use ironrdp_rdpewa::pdu::{CborKey, CborValue, decode_all, encode_to_vec};

use crate::cbor_map::{bytes_field, map_get, text_field};

const MAKECRED_RP: i64 = 2;
const MAKECRED_USER: i64 = 3;
const MAKECRED_PUB_KEY_CRED_PARAMS: i64 = 4;
const MAKECRED_EXCLUDE_LIST: i64 = 5;
const MAKECRED_OPTIONS: i64 = 7;

const GETASSERT_RP_ID: i64 = 1;
const GETASSERT_ALLOW_LIST: i64 = 3;
const GETASSERT_OPTIONS: i64 = 5;

#[derive(Debug, Clone)]
pub(crate) struct MakeCredentialCtap {
    pub rp_id: String,
    pub rp_name: Option<String>,
    pub user_id: Vec<u8>,
    pub user_name: Option<String>,
    pub user_display_name: Option<String>,
    pub algorithms: Vec<i32>,
    pub exclude_credential_ids: Vec<Vec<u8>>,
    pub resident_key: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct GetAssertionCtap {
    pub rp_id: String,
    pub allow_credential_ids: Vec<Vec<u8>>,
}

pub(crate) fn parse_make_credential(ctap_cbor: &[u8]) -> Result<MakeCredentialCtap, &'static str> {
    let value = decode_all(ctap_cbor).map_err(|_| "invalid makeCredential CTAP CBOR")?;
    let map = value.as_map().map_err(|_| "makeCredential CTAP body is not a map")?;

    let rp_map = map_get(map, MAKECRED_RP)
        .ok_or("missing rp")?
        .as_map()
        .map_err(|_| "rp is not a map")?;
    let rp_id = text_field(rp_map, "id").ok_or("missing rp.id")?;
    let rp_name = text_field(rp_map, "name");

    let user_map = map_get(map, MAKECRED_USER)
        .ok_or("missing user")?
        .as_map()
        .map_err(|_| "user is not a map")?;
    let user_id = bytes_field(user_map, "id").ok_or("missing user.id")?;
    let user_name = text_field(user_map, "name");
    let user_display_name = text_field(user_map, "displayName");

    let mut algorithms = Vec::new();
    if let Some(params) = map_get(map, MAKECRED_PUB_KEY_CRED_PARAMS) {
        let arr = params.as_array().map_err(|_| "pubKeyCredParams is not an array")?;
        for item in arr {
            if let Ok(param_map) = item.as_map() {
                if let Some(alg) = map_get_text_i32(param_map, "alg") {
                    algorithms.push(alg);
                }
            }
        }
    }
    if algorithms.is_empty() {
        // ES256 is the common default for Windows Hello / security keys.
        algorithms.push(-7);
    }

    let exclude_credential_ids = map_get(map, MAKECRED_EXCLUDE_LIST)
        .map(credential_ids_from_list)
        .transpose()?
        .unwrap_or_default();

    let resident_key = map_get(map, MAKECRED_OPTIONS)
        .and_then(|options| options.as_map().ok())
        .and_then(|opt_map| map_get_text_bool(opt_map, "rk"))
        .unwrap_or(false);

    Ok(MakeCredentialCtap {
        rp_id,
        rp_name,
        user_id,
        user_name,
        user_display_name,
        algorithms,
        exclude_credential_ids,
        resident_key,
    })
}

pub(crate) fn parse_get_assertion(ctap_cbor: &[u8]) -> Result<GetAssertionCtap, &'static str> {
    let value = decode_all(ctap_cbor).map_err(|_| "invalid getAssertion CTAP CBOR")?;
    let map = value.as_map().map_err(|_| "getAssertion CTAP body is not a map")?;

    let rp_id = map_get(map, GETASSERT_RP_ID)
        .ok_or("missing rpId")?
        .as_text()
        .map_err(|_| "rpId is not text")?
        .to_owned();

    let allow_credential_ids = map_get(map, GETASSERT_ALLOW_LIST)
        .map(credential_ids_from_list)
        .transpose()?
        .unwrap_or_default();

    let _ = map_get(map, GETASSERT_OPTIONS);

    Ok(GetAssertionCtap {
        rp_id,
        allow_credential_ids,
    })
}

/// Build CTAP makeCredential response map: `{1:fmt, 2:authData, 3:attStmt}`.
pub(crate) fn encode_make_credential_response(
    fmt: &str,
    auth_data: &[u8],
    att_stmt_cbor: Option<&[u8]>,
) -> Result<Vec<u8>, &'static str> {
    let mut map = BTreeMap::new();
    map.insert(CborKey::Int(1), CborValue::Text(fmt.to_owned()));
    map.insert(CborKey::Int(2), CborValue::Bytes(auth_data.to_vec()));
    let att_stmt = match att_stmt_cbor {
        Some(raw) if !raw.is_empty() => decode_all(raw).unwrap_or_else(|_| CborValue::Map(BTreeMap::new())),
        _ => CborValue::Map(BTreeMap::new()),
    };
    map.insert(CborKey::Int(3), att_stmt);
    encode_to_vec(&CborValue::Map(map)).map_err(|_| "failed to encode makeCredential response")
}

/// Build CTAP getAssertion response map: `{1:cred, 2:authData, 3:sig, 4?:user}`.
pub(crate) fn encode_get_assertion_response(
    credential_id: &[u8],
    auth_data: &[u8],
    signature: &[u8],
    user_id: Option<&[u8]>,
) -> Result<Vec<u8>, &'static str> {
    let mut cred = BTreeMap::new();
    cred.insert(CborKey::Text("id".into()), CborValue::Bytes(credential_id.to_vec()));
    cred.insert(CborKey::Text("type".into()), CborValue::Text("public-key".into()));

    let mut map = BTreeMap::new();
    map.insert(CborKey::Int(1), CborValue::Map(cred));
    map.insert(CborKey::Int(2), CborValue::Bytes(auth_data.to_vec()));
    map.insert(CborKey::Int(3), CborValue::Bytes(signature.to_vec()));
    if let Some(uid) = user_id {
        if !uid.is_empty() {
            let mut user = BTreeMap::new();
            user.insert(CborKey::Text("id".into()), CborValue::Bytes(uid.to_vec()));
            map.insert(CborKey::Int(4), CborValue::Map(user));
        }
    }
    encode_to_vec(&CborValue::Map(map)).map_err(|_| "failed to encode getAssertion response")
}

/// CTAP status byte + response CBOR body.
pub(crate) fn pack_ctap_response(status: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + body.len());
    out.push(status);
    out.extend_from_slice(body);
    out
}

fn credential_ids_from_list(list: &CborValue) -> Result<Vec<Vec<u8>>, &'static str> {
    let arr = list.as_array().map_err(|_| "credential list is not an array")?;
    let mut ids = Vec::new();
    for item in arr {
        if let Ok(map) = item.as_map() {
            if let Some(id) = bytes_field(map, "id") {
                ids.push(id);
            }
        }
    }
    Ok(ids)
}

fn map_get_text_i32(map: &BTreeMap<CborKey, CborValue>, key: &str) -> Option<i32> {
    map.get(&CborKey::Text(key.to_owned()))
        .and_then(|v| v.as_i64().ok())
        .and_then(|n| i32::try_from(n).ok())
}

fn map_get_text_bool(map: &BTreeMap<CborKey, CborValue>, key: &str) -> Option<bool> {
    map.get(&CborKey::Text(key.to_owned())).and_then(|v| v.as_bool().ok())
}
