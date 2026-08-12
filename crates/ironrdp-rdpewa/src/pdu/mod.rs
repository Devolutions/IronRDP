//! MS-RDPEWA PDUs: CBOR request maps and HRESULT-prefixed responses.

mod cbor;

pub use cbor::{CborKey, CborValue, decode_all, decode_value, encode_to_vec, encode_value, encoded_size};

use ironrdp_core::{
    DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, ensure_size, invalid_field_err, other_err,
};
use ironrdp_dvc::DvcEncode;
use std::collections::BTreeMap;

/// HRESULT success.
pub const S_OK: u32 = 0;
/// HRESULT `E_NOTIMPL`.
pub const E_NOTIMPL: u32 = 0x8000_4001;
/// HRESULT `E_FAIL`.
pub const E_FAIL: u32 = 0x8000_4005;
/// HRESULT `E_INVALIDARG`.
pub const E_INVALIDARG: u32 = 0x8007_0057;
/// HRESULT `E_ABORT` (operation cancelled).
pub const E_ABORT: u32 = 0x8000_4004;

/// MS-RDPEWA RPC command identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum RpcCommand {
    WebAuthn = 5,
    Iuvpaa = 6,
    CancelCurOp = 7,
    ApiVersion = 8,
    GetCredentials = 9,
    GetAuthenticatorList = 12,
}

impl RpcCommand {
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            5 => Some(Self::WebAuthn),
            6 => Some(Self::Iuvpaa),
            7 => Some(Self::CancelCurOp),
            8 => Some(Self::ApiVersion),
            9 => Some(Self::GetCredentials),
            12 => Some(Self::GetAuthenticatorList),
            _ => None,
        }
    }

    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

/// WEB_AUTHN subcommand byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WebAuthnSubcommand {
    MakeCredential = 0x01,
    GetAssertion = 0x02,
}

impl WebAuthnSubcommand {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::MakeCredential),
            0x02 => Some(Self::GetAssertion),
            _ => None,
        }
    }
}

/// Authenticator attachment preference from `webAuthNPara`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum Attachment {
    #[default]
    Any = 0,
    Platform = 1,
    CrossPlatform = 2,
}

impl Attachment {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Platform,
            2 => Self::CrossPlatform,
            _ => Self::Any,
        }
    }
}

/// User verification requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum UserVerification {
    #[default]
    Any = 0,
    Required = 1,
    Preferred = 2,
    Discouraged = 3,
}

impl UserVerification {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::Required,
            2 => Self::Preferred,
            3 => Self::Discouraged,
            _ => Self::Any,
        }
    }
}

/// Attestation conveyance preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum Attestation {
    #[default]
    Any = 0,
    None = 1,
    Indirect = 2,
    Direct = 3,
}

impl Attestation {
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::None,
            2 => Self::Indirect,
            3 => Self::Direct,
            _ => Self::Any,
        }
    }
}

/// Optional WebAuthn parameters carried beside the CTAP request.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct WebAuthnPara {
    pub attachment: Attachment,
    pub require_resident_key: bool,
    pub user_verification: UserVerification,
    pub attestation: Attestation,
    pub cancellation_id: Option<Vec<u8>>,
    pub ui_origin: Option<String>,
    /// Remaining unknown/optional CBOR map entries.
    pub extra: BTreeMap<CborKey, CborValue>,
}

impl WebAuthnPara {
    pub fn decode(value: &CborValue) -> DecodeResult<Self> {
        let map = value.as_map()?;
        let mut para = Self::default();
        for (key, val) in map {
            match key {
                CborKey::Text(name) => match name.as_str() {
                    "attachment" => para.attachment = Attachment::from_u32(val.as_u32()?),
                    "requireResidentKey" => para.require_resident_key = val.as_bool()?,
                    "userVerification" => para.user_verification = UserVerification::from_u32(val.as_u32()?),
                    "attestation" => para.attestation = Attestation::from_u32(val.as_u32()?),
                    "cancellationId" => para.cancellation_id = Some(val.as_bytes()?.to_vec()),
                    "uiOrigin" => para.ui_origin = Some(val.as_text()?.to_owned()),
                    _ => {
                        para.extra.insert(key.clone(), val.clone());
                    }
                },
                other => {
                    para.extra.insert(other.clone(), val.clone());
                }
            }
        }
        Ok(para)
    }

    pub fn encode(&self) -> CborValue {
        let mut map = BTreeMap::new();
        map.insert(CborKey::text("attachment"), CborValue::Unsigned(self.attachment as u64));
        map.insert(
            CborKey::text("requireResidentKey"),
            CborValue::Bool(self.require_resident_key),
        );
        map.insert(
            CborKey::text("userVerification"),
            CborValue::Unsigned(self.user_verification as u64),
        );
        map.insert(
            CborKey::text("attestation"),
            CborValue::Unsigned(self.attestation as u64),
        );
        if let Some(id) = &self.cancellation_id {
            map.insert(CborKey::text("cancellationId"), CborValue::Bytes(id.clone()));
        }
        if let Some(origin) = &self.ui_origin {
            map.insert(CborKey::text("uiOrigin"), CborValue::Text(origin.clone()));
        }
        for (k, v) in &self.extra {
            map.insert(k.clone(), v.clone());
        }
        CborValue::Map(map)
    }
}

/// WEB_AUTHN request body: subcommand + CTAP CBOR payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebAuthnRequestBody {
    pub subcommand: WebAuthnSubcommand,
    /// Raw CTAP CBOR map bytes (without the leading subcommand byte).
    pub ctap_cbor: Vec<u8>,
}

impl WebAuthnRequestBody {
    pub fn decode(bytes: &[u8]) -> DecodeResult<Self> {
        if bytes.is_empty() {
            return Err(invalid_field_err!("request", "body", "empty WEB_AUTHN request"));
        }
        let subcommand = WebAuthnSubcommand::from_u8(bytes[0])
            .ok_or_else(|| invalid_field_err!("request", "subcommand", "unknown WEB_AUTHN subcommand"))?;
        Ok(Self {
            subcommand,
            ctap_cbor: bytes[1..].to_vec(),
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + self.ctap_cbor.len());
        out.push(self.subcommand as u8);
        out.extend_from_slice(&self.ctap_cbor);
        out
    }
}

/// Decoded MS-RDPEWA request.
#[derive(Debug, Clone, PartialEq)]
pub struct RdpewaRequest {
    pub command: RpcCommand,
    pub flags: u32,
    pub rp_id: Option<String>,
    pub timeout_ms: u32,
    pub transaction_id: Vec<u8>,
    pub client_data_json: Option<Vec<u8>>,
    pub webauthn_para: Option<WebAuthnPara>,
    /// For WEB_AUTHN: subcommand + CTAP body. For other commands often empty.
    pub request_body: Vec<u8>,
    /// Full original map for forward-compat.
    pub raw: BTreeMap<CborKey, CborValue>,
}

impl RdpewaRequest {
    pub fn decode(data: &[u8]) -> DecodeResult<Self> {
        let value = decode_all(data)?;
        let map = value.into_map()?;
        let command_u32 = map
            .get(&CborKey::text("command"))
            .ok_or_else(|| invalid_field_err!("request", "command", "missing"))?
            .as_u32()?;
        let command = RpcCommand::from_u32(command_u32)
            .ok_or_else(|| invalid_field_err!("request", "command", "unsupported RPC command"))?;

        let flags = map
            .get(&CborKey::text("flags"))
            .map(CborValue::as_u32)
            .transpose()?
            .unwrap_or(0);
        let rp_id = map
            .get(&CborKey::text("rpId"))
            .map(CborValue::as_text)
            .transpose()?
            .map(str::to_owned);
        let timeout_ms = map
            .get(&CborKey::text("timeout"))
            .map(CborValue::as_u32)
            .transpose()?
            .unwrap_or(0);
        let transaction_id = map
            .get(&CborKey::text("transactionId"))
            .map(CborValue::as_bytes)
            .transpose()?
            .map(|b| b.to_vec())
            .unwrap_or_default();
        let client_data_json = map
            .get(&CborKey::text("clientDataJSON"))
            .map(CborValue::as_bytes)
            .transpose()?
            .map(|b| b.to_vec());
        let webauthn_para = map
            .get(&CborKey::text("webAuthNPara"))
            .map(WebAuthnPara::decode)
            .transpose()?;
        let request_body = map
            .get(&CborKey::text("request"))
            .map(CborValue::as_bytes)
            .transpose()?
            .map(|b| b.to_vec())
            .unwrap_or_default();

        Ok(Self {
            command,
            flags,
            rp_id,
            timeout_ms,
            transaction_id,
            client_data_json,
            webauthn_para,
            request_body,
            raw: map,
        })
    }

    pub fn encode(&self) -> EncodeResult<Vec<u8>> {
        let mut map = BTreeMap::new();
        map.insert(
            CborKey::text("command"),
            CborValue::Unsigned(u64::from(self.command.as_u32())),
        );
        map.insert(CborKey::text("flags"), CborValue::Unsigned(u64::from(self.flags)));
        if let Some(rp_id) = &self.rp_id {
            map.insert(CborKey::text("rpId"), CborValue::Text(rp_id.clone()));
        }
        map.insert(
            CborKey::text("timeout"),
            CborValue::Unsigned(u64::from(self.timeout_ms)),
        );
        if !self.transaction_id.is_empty() {
            map.insert(
                CborKey::text("transactionId"),
                CborValue::Bytes(self.transaction_id.clone()),
            );
        }
        if !self.request_body.is_empty() {
            map.insert(CborKey::text("request"), CborValue::Bytes(self.request_body.clone()));
        }
        if let Some(cdj) = &self.client_data_json {
            map.insert(CborKey::text("clientDataJSON"), CborValue::Bytes(cdj.clone()));
        }
        if let Some(para) = &self.webauthn_para {
            map.insert(CborKey::text("webAuthNPara"), para.encode());
        }
        encode_to_vec(&CborValue::Map(map))
    }

    pub fn webauthn_body(&self) -> DecodeResult<WebAuthnRequestBody> {
        WebAuthnRequestBody::decode(&self.request_body)
    }
}

/// MS-RDPEWA response: little-endian HRESULT + optional payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RdpewaResponse {
    pub hresult: u32,
    pub payload: Vec<u8>,
}

impl RdpewaResponse {
    pub fn ok_empty() -> Self {
        Self {
            hresult: S_OK,
            payload: Vec::new(),
        }
    }

    pub fn from_hresult(hresult: u32) -> Self {
        Self {
            hresult,
            payload: Vec::new(),
        }
    }

    pub fn with_u32(hresult: u32, value: u32) -> Self {
        Self {
            hresult,
            payload: value.to_le_bytes().to_vec(),
        }
    }

    pub fn with_payload(hresult: u32, payload: Vec<u8>) -> Self {
        Self { hresult, payload }
    }

    pub fn decode(data: &[u8]) -> DecodeResult<Self> {
        if data.len() < 4 {
            return Err(invalid_field_err!("response", "hresult", "not enough bytes"));
        }
        let mut cursor = ReadCursor::new(data);
        let hresult = cursor.read_u32();
        Ok(Self {
            hresult,
            payload: cursor.read_remaining().to_vec(),
        })
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.payload.len());
        out.extend_from_slice(&self.hresult.to_le_bytes());
        out.extend_from_slice(&self.payload);
        out
    }

    pub fn encode_into(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        if dst.len() < 4 + self.payload.len() {
            return Err(other_err!("response", "not enough space"));
        }
        dst.write_u32(self.hresult);
        dst.write_slice(&self.payload);
        Ok(())
    }

    pub fn size(&self) -> usize {
        4 + self.payload.len()
    }

    pub fn payload_u32(&self) -> DecodeResult<u32> {
        if self.payload.len() < 4 {
            return Err(invalid_field_err!("response", "payload", "expected u32"));
        }
        Ok(u32::from_le_bytes(self.payload[..4].try_into().unwrap()))
    }
}

/// Authenticator device metadata in a WEB_AUTHN response (`deviceInfo` map).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub max_msg_size: u32,
    pub max_serialized_large_blob_array: u32,
    pub provider_type: String,
    pub provider_name: String,
    pub device_path: String,
    pub manufacturer: String,
    pub product: String,
    pub aa_guid: [u8; 16],
    pub uv_status: u32,
    pub uv_retries: u32,
    pub transports: u32,
    pub resident_key: Option<bool>,
}

impl Default for DeviceInfo {
    fn default() -> Self {
        Self {
            max_msg_size: 1200,
            max_serialized_large_blob_array: 1024,
            provider_type: String::from("Platform"),
            provider_name: String::from("IronRDPWebAuthnProvider"),
            device_path: String::new(),
            manufacturer: String::new(),
            product: String::new(),
            aa_guid: [0; 16],
            uv_status: 0,
            uv_retries: 0,
            transports: 0,
            resident_key: None,
        }
    }
}

impl DeviceInfo {
    pub fn encode(&self) -> CborValue {
        let mut map = BTreeMap::new();
        map.insert(
            CborKey::text("maxMsgSize"),
            CborValue::Unsigned(u64::from(self.max_msg_size)),
        );
        map.insert(
            CborKey::text("maxSerializedLargeBlobArray"),
            CborValue::Unsigned(u64::from(self.max_serialized_large_blob_array)),
        );
        map.insert(
            CborKey::text("providerType"),
            CborValue::Text(self.provider_type.clone()),
        );
        map.insert(
            CborKey::text("providerName"),
            CborValue::Text(self.provider_name.clone()),
        );
        map.insert(CborKey::text("devicePath"), CborValue::Text(self.device_path.clone()));
        map.insert(
            CborKey::text("Manufacturer"),
            CborValue::Text(self.manufacturer.clone()),
        );
        map.insert(CborKey::text("Product"), CborValue::Text(self.product.clone()));
        map.insert(CborKey::text("aaGuid"), CborValue::Bytes(self.aa_guid.to_vec()));
        map.insert(
            CborKey::text("uvStatus"),
            CborValue::Unsigned(u64::from(self.uv_status)),
        );
        map.insert(
            CborKey::text("uvRetries"),
            CborValue::Unsigned(u64::from(self.uv_retries)),
        );
        map.insert(
            CborKey::text("transports"),
            CborValue::Unsigned(u64::from(self.transports)),
        );
        if let Some(rk) = self.resident_key {
            map.insert(CborKey::text("residentKey"), CborValue::Bool(rk));
        }
        CborValue::Map(map)
    }

    pub fn decode(value: &CborValue) -> DecodeResult<Self> {
        let map = value.as_map()?;
        let mut info = Self::default();
        if let Some(v) = map.get(&CborKey::text("maxMsgSize")) {
            info.max_msg_size = v.as_u32()?;
        }
        if let Some(v) = map.get(&CborKey::text("maxSerializedLargeBlobArray")) {
            info.max_serialized_large_blob_array = v.as_u32()?;
        }
        if let Some(v) = map.get(&CborKey::text("providerType")) {
            info.provider_type = v.as_text()?.to_owned();
        }
        if let Some(v) = map.get(&CborKey::text("providerName")) {
            info.provider_name = v.as_text()?.to_owned();
        }
        if let Some(v) = map.get(&CborKey::text("devicePath")) {
            info.device_path = v.as_text()?.to_owned();
        }
        if let Some(v) = map.get(&CborKey::text("Manufacturer")) {
            info.manufacturer = v.as_text()?.to_owned();
        }
        if let Some(v) = map.get(&CborKey::text("Product")) {
            info.product = v.as_text()?.to_owned();
        }
        if let Some(v) = map.get(&CborKey::text("aaGuid")) {
            let b = v.as_bytes()?;
            if b.len() == 16 {
                info.aa_guid.copy_from_slice(b);
            }
        }
        if let Some(v) = map.get(&CborKey::text("uvStatus")) {
            info.uv_status = v.as_u32()?;
        }
        if let Some(v) = map.get(&CborKey::text("uvRetries")) {
            info.uv_retries = v.as_u32()?;
        }
        if let Some(v) = map.get(&CborKey::text("transports")) {
            info.transports = v.as_u32()?;
        }
        if let Some(v) = map.get(&CborKey::text("residentKey")) {
            info.resident_key = Some(v.as_bool()?);
        }
        Ok(info)
    }
}

/// WEB_AUTHN success payload CBOR map fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebAuthnResponsePayload {
    pub device_info: DeviceInfo,
    pub status: u32,
    /// CTAP status byte + CTAP CBOR response body.
    pub response: Vec<u8>,
}

impl WebAuthnResponsePayload {
    pub fn encode(&self) -> EncodeResult<Vec<u8>> {
        let mut map = BTreeMap::new();
        map.insert(CborKey::text("deviceInfo"), self.device_info.encode());
        map.insert(CborKey::text("status"), CborValue::Unsigned(u64::from(self.status)));
        map.insert(CborKey::text("response"), CborValue::Bytes(self.response.clone()));
        encode_to_vec(&CborValue::Map(map))
    }

    pub fn decode(data: &[u8]) -> DecodeResult<Self> {
        let value = decode_all(data)?;
        let map = value.as_map()?;
        let device_info = map
            .get(&CborKey::text("deviceInfo"))
            .map(DeviceInfo::decode)
            .transpose()?
            .unwrap_or_default();
        let status = map
            .get(&CborKey::text("status"))
            .or_else(|| map.get(&CborKey::text("Status")))
            .map(CborValue::as_u32)
            .transpose()?
            .unwrap_or(0);
        let response = map
            .get(&CborKey::text("response"))
            .or_else(|| map.get(&CborKey::text("Response")))
            .map(CborValue::as_bytes)
            .transpose()?
            .map(|b| b.to_vec())
            .unwrap_or_default();
        Ok(Self {
            device_info,
            status,
            response,
        })
    }
}

/// Build a CTAP-style response body: status byte + CBOR map.
pub fn encode_ctap_response(status: u8, body: &CborValue) -> EncodeResult<Vec<u8>> {
    let body_bytes = encode_to_vec(body)?;
    let mut out = Vec::with_capacity(1 + body_bytes.len());
    out.push(status);
    out.extend_from_slice(&body_bytes);
    Ok(out)
}

impl Encode for RdpewaResponse {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        self.encode_into(dst)
    }

    fn name(&self) -> &'static str {
        "RDPEWA_RESPONSE"
    }

    fn size(&self) -> usize {
        RdpewaResponse::size(self)
    }
}

impl DvcEncode for RdpewaResponse {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tx() -> Vec<u8> {
        (0u8..16).collect()
    }

    #[test]
    fn api_version_request_roundtrip() {
        let req = RdpewaRequest {
            command: RpcCommand::ApiVersion,
            flags: 0,
            rp_id: None,
            timeout_ms: 0,
            transaction_id: sample_tx(),
            client_data_json: None,
            webauthn_para: None,
            request_body: Vec::new(),
            raw: BTreeMap::new(),
        };
        let encoded = req.encode().unwrap();
        let decoded = RdpewaRequest::decode(&encoded).unwrap();
        assert_eq!(decoded.command, RpcCommand::ApiVersion);
        assert_eq!(decoded.transaction_id, sample_tx());
    }

    #[test]
    fn response_u32_payload() {
        let resp = RdpewaResponse::with_u32(S_OK, 4);
        let bytes = resp.to_bytes();
        assert_eq!(bytes.len(), 8);
        let decoded = RdpewaResponse::decode(&bytes).unwrap();
        assert_eq!(decoded.hresult, S_OK);
        assert_eq!(decoded.payload_u32().unwrap(), 4);
    }

    #[test]
    fn cancel_response_hresult_only() {
        let resp = RdpewaResponse::from_hresult(S_OK);
        let bytes = resp.to_bytes();
        assert_eq!(bytes, [0, 0, 0, 0]);
    }

    #[test]
    fn webauthn_body_parse() {
        let body = WebAuthnRequestBody {
            subcommand: WebAuthnSubcommand::MakeCredential,
            ctap_cbor: vec![0xa1, 0x01, 0x60],
        };
        let encoded = body.encode();
        let decoded = WebAuthnRequestBody::decode(&encoded).unwrap();
        assert_eq!(decoded.subcommand, WebAuthnSubcommand::MakeCredential);
        assert_eq!(decoded.ctap_cbor, vec![0xa1, 0x01, 0x60]);
    }

    #[test]
    fn iuvpaa_request_decode() {
        let req = RdpewaRequest {
            command: RpcCommand::Iuvpaa,
            flags: 0,
            rp_id: None,
            timeout_ms: 60_000,
            transaction_id: sample_tx(),
            client_data_json: None,
            webauthn_para: None,
            request_body: Vec::new(),
            raw: BTreeMap::new(),
        };
        let encoded = req.encode().unwrap();
        let decoded = RdpewaRequest::decode(&encoded).unwrap();
        assert_eq!(decoded.command, RpcCommand::Iuvpaa);
        assert_eq!(decoded.timeout_ms, 60_000);
    }

    #[test]
    fn webauthn_response_payload_roundtrip() {
        let payload = WebAuthnResponsePayload {
            device_info: DeviceInfo {
                provider_type: String::from("Platform"),
                provider_name: String::from("test"),
                ..DeviceInfo::default()
            },
            status: 0,
            response: vec![0x00, 0xa0],
        };
        let encoded = payload.encode().unwrap();
        let decoded = WebAuthnResponsePayload::decode(&encoded).unwrap();
        assert_eq!(decoded, payload);
    }
}
