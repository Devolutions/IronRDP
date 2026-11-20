use std::sync::{Arc, Mutex};

use ironrdp_core::encode_vec;
use ironrdp_dvc::DvcProcessor as _;
use ironrdp_rdpewa::pdu::decode_all;
use ironrdp_rdpewa::{
    Attachment, Attestation, DeviceInfo, E_ABORT, RdpewaClient, RdpewaClientHandler, RdpewaHandlerError,
    RdpewaResponseSender, RdpewaResult, RdpewaServerRpc, RdpewaWebAuthnRequest, UserVerification, WebAuthnDispatch,
    WebAuthnOperationRequest, WebAuthnOperationResponse, WebAuthnPara, WebAuthnRequestBody, WebAuthnSubcommand,
};

struct RecordingHandler {
    uvpaa: bool,
    seen_webauthn: Arc<Mutex<Vec<WebAuthnOperationRequest>>>,
    seen_cancels: Arc<Mutex<Vec<Vec<u8>>>>,
    response: Result<WebAuthnOperationResponse, u32>,
}

impl RdpewaClientHandler for RecordingHandler {
    fn api_version(&mut self) -> RdpewaResult<u32> {
        Ok(4)
    }

    fn is_uvpaa(&mut self) -> RdpewaResult<bool> {
        Ok(self.uvpaa)
    }

    fn cancel_current_operation(&mut self, cancellation_id: &[u8]) -> RdpewaResult<()> {
        self.seen_cancels.lock().unwrap().push(cancellation_id.to_vec());
        Ok(())
    }

    fn begin_webauthn(
        &mut self,
        request: WebAuthnOperationRequest,
        _reply: RdpewaResponseSender,
    ) -> RdpewaResult<WebAuthnDispatch> {
        self.seen_webauthn.lock().unwrap().push(request);
        match &self.response {
            Ok(response) => Ok(WebAuthnDispatch::Sync(response.clone())),
            Err(hresult) => Err(RdpewaHandlerError::new(*hresult, "ceremony rejected")),
        }
    }
}

// oracle: spec MS-RDPEWA §§2.2.1, 2.2.2, and 4.2.1
#[test]
fn server_iuvpaa_rpc_decodes_client_platform_availability() {
    let outcome = Arc::new(Mutex::new(None));
    let recorded = Arc::clone(&outcome);
    let mut server = RdpewaServerRpc::iuvpaa(
        [0x11; 16],
        Box::new(move |result| {
            *recorded.lock().unwrap() = Some(result);
        }),
    );
    let mut client = RdpewaClient::new(Box::new(RecordingHandler {
        uvpaa: true,
        seen_webauthn: Arc::default(),
        seen_cancels: Arc::default(),
        response: Err(E_ABORT),
    }));

    client.start(7).unwrap();
    let outbound = server.start(7).unwrap();
    assert_eq!(outbound.len(), 1);
    let wire_request = encode_vec(outbound[0].as_ref()).unwrap();
    // oracle: spec MS-RDPEWA §4.2.1 and RFC 8949 §4.2.3
    let mut expected_wire = vec![
        0xA4, // map(4)
        0x65, b'f', b'l', b'a', b'g', b's', 0x00, // "flags": 0
        0x67, b'c', b'o', b'm', b'm', b'a', b'n', b'd', 0x06, // "command": 6
        0x67, b't', b'i', b'm', b'e', b'o', b'u', b't', 0x00, // "timeout": 0
        0x6D, b't', b'r', b'a', b'n', b's', b'a', b'c', b't', b'i', b'o', b'n', b'I', b'd', 0x50,
    ];
    expected_wire.extend_from_slice(&[0x11; 16]);
    assert_eq!(wire_request, expected_wire);
    let reply = client.process(7, &wire_request).unwrap();
    assert_eq!(reply.len(), 1);
    let wire_reply = encode_vec(reply[0].as_ref()).unwrap();
    assert!(server.process(7, &wire_reply).unwrap().is_empty());
    assert_eq!(*outcome.lock().unwrap(), Some(Ok(true)));
}

// oracle: spec MS-RDPEWA §§2.2.1, 2.2.1.1, 2.2.2.1, 4.4.1.2, and 4.4.2.1
#[test]
fn server_webauthn_rpc_encodes_spec_parameter_keys_and_returns_ctap_response() {
    use ironrdp_rdpewa::pdu::{CborKey, CborValue, encode_ctap_response, encode_to_vec};
    use std::collections::BTreeMap;

    let seen_webauthn = Arc::new(Mutex::new(Vec::new()));
    let expected_ctap_response = {
        // oracle: spec FIPS 186-4 §D.1.2.3; NIST P-256 base point (G_x, G_y).
        const P256_GX: [u8; 32] = [
            0x6B, 0x17, 0xD1, 0xF2, 0xE1, 0x2C, 0x42, 0x47, 0xF8, 0xBC, 0xE6, 0xE5, 0x63, 0xA4, 0x40, 0xF2, 0x77, 0x03,
            0x7D, 0x81, 0x2D, 0xEB, 0x33, 0xA0, 0xF4, 0xA1, 0x39, 0x45, 0xD8, 0x98, 0xC2, 0x96,
        ];
        const P256_GY: [u8; 32] = [
            0x4F, 0xE3, 0x42, 0xE2, 0xFE, 0x1A, 0x7F, 0x9B, 0x8E, 0xE7, 0xEB, 0x4A, 0x7C, 0x0F, 0x9E, 0x16, 0x2B, 0xCE,
            0x33, 0x57, 0x6B, 0x31, 0x5E, 0xCE, 0xCB, 0xB6, 0x40, 0x68, 0x37, 0xBF, 0x51, 0xF5,
        ];
        // oracle: spec RFC 6234 §6.2; SHA-256 of `"localhost"`.
        const LOCALHOST_RP_ID_HASH: [u8; 32] = [
            73, 150, 13, 229, 136, 14, 140, 104, 116, 52, 23, 15, 100, 118, 96, 91, 143, 228, 174, 185, 162, 134, 50,
            199, 153, 92, 243, 186, 131, 29, 151, 99,
        ];

        let mut cose_key = BTreeMap::new();
        cose_key.insert(CborKey::Int(1), CborValue::Unsigned(2));
        cose_key.insert(CborKey::Int(3), CborValue::Negative(-7));
        cose_key.insert(CborKey::Int(-1), CborValue::Unsigned(1));
        cose_key.insert(CborKey::Int(-2), CborValue::Bytes(P256_GX.to_vec()));
        cose_key.insert(CborKey::Int(-3), CborValue::Bytes(P256_GY.to_vec()));

        let mut auth_data = LOCALHOST_RP_ID_HASH.to_vec();
        auth_data.extend_from_slice(&[0x45, 0, 0, 0, 0]);
        auth_data.extend_from_slice(&[0xAB; 16]);
        auth_data.extend_from_slice(&4u16.to_be_bytes());
        auth_data.extend_from_slice(&[1, 2, 3, 4]);
        auth_data.extend_from_slice(&encode_to_vec(&CborValue::Map(cose_key)).unwrap());

        let mut attestation = BTreeMap::new();
        attestation.insert(CborKey::Int(1), CborValue::Text("none".to_owned()));
        attestation.insert(CborKey::Int(2), CborValue::Bytes(auth_data));
        attestation.insert(CborKey::Int(3), CborValue::Map(BTreeMap::new()));
        encode_ctap_response(0x00, &CborValue::Map(attestation)).unwrap()
    };
    let expected_response = WebAuthnOperationResponse {
        device_info: DeviceInfo {
            provider_type: "Platform".to_owned(),
            provider_name: "TestProvider".to_owned(),
            aa_guid: [0xAB; 16],
            resident_key: Some(true),
            ..DeviceInfo::default()
        },
        status: 0,
        response: expected_ctap_response,
    };
    let mut client = RdpewaClient::new(Box::new(RecordingHandler {
        uvpaa: true,
        seen_webauthn: Arc::clone(&seen_webauthn),
        seen_cancels: Arc::default(),
        response: Ok(expected_response.clone()),
    }));

    let outcome = Arc::new(Mutex::new(None));
    let recorded = Arc::clone(&outcome);
    let cancellation_id = [0x42; 16];
    let client_data_json = br#"{"type":"webauthn.create","challenge":"AA","origin":"https://localhost"}"#.to_vec();
    // oracle: spec RFC 6234 §6.2; SHA-256 of `client_data_json`.
    let client_data_hash = vec![
        119, 211, 63, 66, 33, 115, 243, 88, 90, 39, 44, 78, 44, 140, 74, 169, 201, 126, 60, 89, 100, 19, 247, 15, 96,
        196, 106, 172, 5, 222, 12, 163,
    ];
    let ctap_cbor = {
        let mut rp = BTreeMap::new();
        rp.insert(CborKey::text("id"), CborValue::Text("localhost".to_owned()));
        rp.insert(CborKey::text("name"), CborValue::Text("Test RP".to_owned()));

        let mut user = BTreeMap::new();
        user.insert(CborKey::text("id"), CborValue::Bytes(vec![1, 2, 3, 4]));
        user.insert(CborKey::text("name"), CborValue::Text("user@localhost".to_owned()));
        user.insert(CborKey::text("displayName"), CborValue::Text("User".to_owned()));

        let mut cred_param = BTreeMap::new();
        cred_param.insert(CborKey::text("alg"), CborValue::Negative(-7));
        cred_param.insert(CborKey::text("type"), CborValue::Text("public-key".to_owned()));

        let mut ctap = BTreeMap::new();
        ctap.insert(CborKey::Int(1), CborValue::Bytes(client_data_hash));
        ctap.insert(CborKey::Int(2), CborValue::Map(rp));
        ctap.insert(CborKey::Int(3), CborValue::Map(user));
        ctap.insert(CborKey::Int(4), CborValue::Array(vec![CborValue::Map(cred_param)]));
        encode_to_vec(&CborValue::Map(ctap)).unwrap()
    };
    let mut server = RdpewaServerRpc::webauthn(
        RdpewaWebAuthnRequest {
            rp_id: Some("localhost".to_owned()),
            timeout_ms: 60_000,
            transaction_id: [0x22; 16],
            client_data_json: client_data_json.clone(),
            para: WebAuthnPara {
                attachment: Attachment::Platform,
                require_resident_key: true,
                prefer_resident_key: false,
                user_verification: UserVerification::Required,
                attestation: Attestation::Direct,
                cancellation_id: Some(cancellation_id.to_vec()),
                ui_origin: Some("https://localhost".to_owned()),
                extra: Default::default(),
            },
            body: WebAuthnRequestBody {
                subcommand: WebAuthnSubcommand::MakeCredential,
                ctap_cbor: ctap_cbor.clone(),
            },
        },
        Box::new(move |result| {
            *recorded.lock().unwrap() = Some(result);
        }),
    );

    client.start(9).unwrap();
    let outbound = server.start(9).unwrap();
    let wire_request = encode_vec(outbound[0].as_ref()).unwrap();

    let cbor = decode_all(&wire_request).unwrap();
    let map = cbor.as_map().unwrap();
    // oracle: spec MS-RDPEWA §2.2.1; CTAPCLT_UV_REQUIRED_FLAG (0x00400000).
    assert_eq!(map.get(&CborKey::text("flags")).unwrap().as_u32().unwrap(), 0x0040_0000);
    let para_map = map.get(&CborKey::text("webAuthNPara")).unwrap().as_map().unwrap();
    assert!(
        para_map
            .get(&CborKey::text("requireResident"))
            .unwrap()
            .as_bool()
            .unwrap()
    );
    assert!(
        !para_map
            .get(&CborKey::text("preferResident"))
            .unwrap()
            .as_bool()
            .unwrap()
    );
    // oracle: spec MS-RDPEWA §2.2.1.1; WEBAUTHN_ATTESTATION_CONVEYANCE_PREFERENCE_DIRECT (3).
    assert_eq!(
        para_map
            .get(&CborKey::text("attestationPreference"))
            .unwrap()
            .as_u32()
            .unwrap(),
        3
    );

    let reply = client.process(9, &wire_request).unwrap();
    let wire_reply = encode_vec(reply[0].as_ref()).unwrap();
    assert!(server.process(9, &wire_reply).unwrap().is_empty());

    let seen = seen_webauthn.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].subcommand, WebAuthnSubcommand::MakeCredential);
    assert_eq!(seen[0].rp_id.as_deref(), Some("localhost"));
    assert_eq!(seen[0].client_data_json, client_data_json);
    assert_eq!(seen[0].ctap_cbor, ctap_cbor);
    assert!(seen[0].para.require_resident_key);
    assert!(!seen[0].para.prefer_resident_key);
    assert_eq!(seen[0].para.attestation, Attestation::Direct);
    assert_eq!(
        seen[0].para.cancellation_id.as_deref(),
        Some(cancellation_id.as_slice())
    );

    let completed = outcome
        .lock()
        .unwrap()
        .clone()
        .expect("completed WebAuthn RPC")
        .unwrap();
    assert_eq!(completed.response, expected_response.response);
    assert_eq!(completed.device_info.aa_guid, [0xAB; 16]);
}

// oracle: spec MS-RDPEWA §§2.2.1 and 4.3.1
#[test]
fn server_cancel_rpc_places_cancellation_guid_in_request_field() {
    let seen_cancels = Arc::new(Mutex::new(Vec::new()));
    let mut client = RdpewaClient::new(Box::new(RecordingHandler {
        uvpaa: false,
        seen_webauthn: Arc::default(),
        seen_cancels: Arc::clone(&seen_cancels),
        response: Err(E_ABORT),
    }));
    let outcome = Arc::new(Mutex::new(None));
    let recorded = Arc::clone(&outcome);
    let cancellation_id = [0x7F; 16];
    let mut server = RdpewaServerRpc::cancel(
        cancellation_id,
        [0x33; 16],
        Box::new(move |result| {
            *recorded.lock().unwrap() = Some(result);
        }),
    );

    client.start(11).unwrap();
    let outbound = server.start(11).unwrap();
    let wire_request = encode_vec(outbound[0].as_ref()).unwrap();
    // oracle: spec MS-RDPEWA §4.3.1 and RFC 8949 §4.2.3
    let mut expected_wire = vec![
        0xA5, // map(5)
        0x65, b'f', b'l', b'a', b'g', b's', 0x00, // "flags": 0
        0x67, b'c', b'o', b'm', b'm', b'a', b'n', b'd', 0x07, // "command": 7
        0x67, b'r', b'e', b'q', b'u', b'e', b's', b't', 0x50, // "request": h'...'
    ];
    expected_wire.extend_from_slice(&cancellation_id);
    expected_wire.extend_from_slice(&[
        0x67, b't', b'i', b'm', b'e', b'o', b'u', b't', 0x00, // "timeout": 0
        0x6D, b't', b'r', b'a', b'n', b's', b'a', b'c', b't', b'i', b'o', b'n', b'I', b'd', 0x50,
    ]);
    expected_wire.extend_from_slice(&[0x33; 16]);
    assert_eq!(wire_request, expected_wire);
    let reply = client.process(11, &wire_request).unwrap();
    let wire_reply = encode_vec(reply[0].as_ref()).unwrap();
    assert!(server.process(11, &wire_reply).unwrap().is_empty());

    assert_eq!(*seen_cancels.lock().unwrap(), [cancellation_id.to_vec()]);
    assert_eq!(*outcome.lock().unwrap(), Some(Ok(())));
}

// oracle: mutant declining, closing, or dropping an unanswered WebAuthN_Channel RPC without aborting its caller once with E_ABORT (MS-ERREF §2.1, 0x80004004).
#[test]
fn dropping_unanswered_server_rpc_aborts_pending_ceremony_once() {
    use ironrdp_dvc::DrdynvcServer;
    use ironrdp_dvc::pdu::{CapabilitiesResponsePdu, CapsVersion, CreateResponsePdu, CreationStatus, DrdynvcClientPdu};
    use ironrdp_svc::SvcProcessor as _;

    let declined_calls = Arc::new(Mutex::new(Vec::new()));
    let recorded_declined = Arc::clone(&declined_calls);
    let mut drdynvc = DrdynvcServer::new().with_dynamic_channel(RdpewaServerRpc::iuvpaa(
        [0x44; 16],
        Box::new(move |result| {
            recorded_declined.lock().unwrap_or_else(|e| e.into_inner()).push(result);
        }),
    ));
    drdynvc
        .process(
            &encode_vec(&DrdynvcClientPdu::Capabilities(CapabilitiesResponsePdu::new(
                CapsVersion::V2,
            )))
            .unwrap(),
        )
        .unwrap();
    drdynvc
        .process(
            &encode_vec(&DrdynvcClientPdu::Create(CreateResponsePdu::new(
                0,
                CreationStatus::NO_LISTENER,
            )))
            .unwrap(),
        )
        .unwrap();
    // oracle: spec MS-ERREF §2.1; E_ABORT (0x80004004).
    assert_eq!(declined_calls.lock().unwrap().as_slice(), [Err(0x8000_4004)]);
    drop(drdynvc);
    assert_eq!(declined_calls.lock().unwrap().as_slice(), [Err(0x8000_4004)]);

    let closed_calls = Arc::new(Mutex::new(Vec::new()));
    let recorded_closed = Arc::clone(&closed_calls);
    let mut closed_server = RdpewaServerRpc::iuvpaa(
        [0x45; 16],
        Box::new(move |result| {
            recorded_closed.lock().unwrap_or_else(|e| e.into_inner()).push(result);
        }),
    );
    closed_server.close(12);
    assert_eq!(closed_calls.lock().unwrap().as_slice(), [Err(0x8000_4004)]);
    drop(closed_server);
    assert_eq!(closed_calls.lock().unwrap().as_slice(), [Err(0x8000_4004)]);

    let dropped_calls = Arc::new(Mutex::new(Vec::new()));
    let recorded_dropped = Arc::clone(&dropped_calls);
    let dropped_server = RdpewaServerRpc::iuvpaa(
        [0x46; 16],
        Box::new(move |result| {
            recorded_dropped.lock().unwrap_or_else(|e| e.into_inner()).push(result);
        }),
    );
    drop(dropped_server);
    assert_eq!(dropped_calls.lock().unwrap().as_slice(), [Err(0x8000_4004)]);
}

// oracle: spec CTAP2 §6 and RFC 8949 §4.2.3
#[test]
fn cbor_map_encodes_positive_negative_and_text_keys_in_canonical_byte_order() {
    use ironrdp_rdpewa::pdu::{CborKey, CborValue, encode_to_vec};
    use std::collections::BTreeMap;

    let mut cose_key = BTreeMap::new();
    cose_key.insert(CborKey::Int(-3), CborValue::Unsigned(0));
    cose_key.insert(CborKey::Int(-2), CborValue::Unsigned(0));
    cose_key.insert(CborKey::Int(-1), CborValue::Unsigned(1));
    cose_key.insert(CborKey::Int(24), CborValue::Unsigned(0));
    cose_key.insert(CborKey::Int(3), CborValue::Negative(-7));
    cose_key.insert(CborKey::Int(1), CborValue::Unsigned(2));
    cose_key.insert(CborKey::text("alg"), CborValue::Unsigned(0));
    cose_key.insert(CborKey::text("id"), CborValue::Unsigned(0));

    let encoded = encode_to_vec(&CborValue::Map(cose_key)).unwrap();
    assert_eq!(
        encoded,
        [
            0xA8, // map(8)
            0x01, 0x02, // 1: 2
            0x03, 0x26, // 3: -7
            0x18, 0x18, 0x00, // 24: 0 (two-byte major 0 before one-byte major 1)
            0x20, 0x01, // -1: 1
            0x21, 0x00, // -2: 0
            0x22, 0x00, // -3: 0
            0x62, b'i', b'd', 0x00, // "id": 0 (len 3 encoded key)
            0x63, b'a', b'l', b'g', 0x00, // "alg": 0 (len 4 encoded key)
        ]
    );
}

// oracle: spec MS-RDPEWA §§2.2.1 and 2.2.1.1
#[rstest::rstest]
#[case::required(UserVerification::Required, 0x0040_0000, 1)]
#[case::preferred(UserVerification::Preferred, 0x0080_0000, 2)]
#[case::discouraged(UserVerification::Discouraged, 0x0100_0000, 3)]
#[case::any(UserVerification::Any, 0, 0)]
fn server_webauthn_rpc_maps_user_verification_and_prefer_resident_to_wire(
    #[case] user_verification: UserVerification,
    #[case] expected_flags: u32,
    #[case] expected_uv_wire: u32,
) {
    use ironrdp_rdpewa::pdu::CborKey;

    let seen_webauthn = Arc::new(Mutex::new(Vec::new()));
    let mut client = RdpewaClient::new(Box::new(RecordingHandler {
        uvpaa: true,
        seen_webauthn: Arc::clone(&seen_webauthn),
        seen_cancels: Arc::default(),
        response: Err(E_ABORT),
    }));
    let mut server = RdpewaServerRpc::webauthn(
        RdpewaWebAuthnRequest {
            rp_id: Some("localhost".to_owned()),
            timeout_ms: 15_000,
            transaction_id: [0x51; 16],
            client_data_json: b"{}".to_vec(),
            para: WebAuthnPara {
                attachment: Attachment::CrossPlatform,
                require_resident_key: false,
                prefer_resident_key: true,
                user_verification,
                attestation: Attestation::None,
                cancellation_id: None,
                ui_origin: None,
                extra: Default::default(),
            },
            body: WebAuthnRequestBody {
                subcommand: WebAuthnSubcommand::GetAssertion,
                ctap_cbor: vec![0xA0],
            },
        },
        Box::new(|_| {}),
    );

    client.start(21).unwrap();
    let outbound = server.start(21).unwrap();
    let wire_request = encode_vec(outbound[0].as_ref()).unwrap();
    let decoded = decode_all(&wire_request).unwrap();
    let map = decoded.as_map().unwrap();
    assert_eq!(
        map.get(&CborKey::text("flags")).unwrap().as_u32().unwrap(),
        expected_flags
    );
    let para_map = map.get(&CborKey::text("webAuthNPara")).unwrap().as_map().unwrap();
    assert!(
        !para_map
            .get(&CborKey::text("requireResident"))
            .unwrap()
            .as_bool()
            .unwrap()
    );
    assert!(
        para_map
            .get(&CborKey::text("preferResident"))
            .unwrap()
            .as_bool()
            .unwrap()
    );
    assert_eq!(
        para_map
            .get(&CborKey::text("userVerification"))
            .unwrap()
            .as_u32()
            .unwrap(),
        expected_uv_wire
    );

    let _ = client.process(21, &wire_request).unwrap();
    let seen = seen_webauthn.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert!(!seen[0].para.require_resident_key);
    assert!(seen[0].para.prefer_resident_key);
    assert_eq!(seen[0].para.user_verification, user_verification);
}

// oracle: spec MS-RDPEWA §4.4.2.1
#[test]
fn server_webauthn_rpc_decodes_lowercase_device_info_keys_from_spec_example() {
    use ironrdp_rdpewa::pdu::{CborKey, CborValue, RdpewaResponse, S_OK, encode_to_vec};
    use std::collections::BTreeMap;

    let outcome = Arc::new(Mutex::new(None));
    let recorded = Arc::clone(&outcome);
    let mut server = RdpewaServerRpc::webauthn(
        RdpewaWebAuthnRequest {
            rp_id: Some("localhost".to_owned()),
            timeout_ms: 30_000,
            transaction_id: [0x61; 16],
            client_data_json: b"{}".to_vec(),
            para: WebAuthnPara::default(),
            body: WebAuthnRequestBody {
                subcommand: WebAuthnSubcommand::GetAssertion,
                ctap_cbor: vec![0xA0],
            },
        },
        Box::new(move |result| {
            *recorded.lock().unwrap() = Some(result);
        }),
    );

    let _ = server.start(22).unwrap();
    let mut device_info = BTreeMap::new();
    device_info.insert(CborKey::text("manufacturer"), CborValue::Text("Contoso".to_owned()));
    device_info.insert(CborKey::text("product"), CborValue::Text("Security Key".to_owned()));

    let mut payload = BTreeMap::new();
    payload.insert(CborKey::text("deviceInfo"), CborValue::Map(device_info));
    payload.insert(CborKey::text("status"), CborValue::Unsigned(0));
    payload.insert(CborKey::text("response"), CborValue::Bytes(vec![0x00, 0xA0]));

    let wire_reply = RdpewaResponse::with_payload(S_OK, encode_to_vec(&CborValue::Map(payload)).unwrap()).to_bytes();
    assert!(server.process(22, &wire_reply).unwrap().is_empty());

    let completed = outcome.lock().unwrap().clone().unwrap().unwrap();
    assert_eq!(completed.device_info.manufacturer, "Contoso");
    assert_eq!(completed.device_info.product, "Security Key");
}

// oracle: mutant ignoring webAuthNPara.cancellationId when CANCEL_CUR_OP omits top-level request.
#[test]
fn client_cancel_falls_back_to_webauthn_para_cancellation_id() {
    use ironrdp_rdpewa::{RdpewaRequest, RpcCommand};
    use std::collections::BTreeMap;

    let seen_cancels = Arc::new(Mutex::new(Vec::new()));
    let mut client = RdpewaClient::new(Box::new(RecordingHandler {
        uvpaa: false,
        seen_webauthn: Arc::default(),
        seen_cancels: Arc::clone(&seen_cancels),
        response: Err(E_ABORT),
    }));
    let fallback_id = vec![0x99; 16];
    let wire_request = RdpewaRequest {
        command: RpcCommand::CancelCurOp,
        flags: 0,
        rp_id: None,
        timeout_ms: 0,
        transaction_id: vec![0x71; 16],
        client_data_json: None,
        webauthn_para: Some(WebAuthnPara {
            cancellation_id: Some(fallback_id.clone()),
            ..WebAuthnPara::default()
        }),
        request_body: Vec::new(),
        raw: BTreeMap::new(),
    }
    .encode()
    .unwrap();

    client.start(23).unwrap();
    let reply = client.process(23, &wire_request).unwrap();
    assert_eq!(reply.len(), 1);
    assert_eq!(*seen_cancels.lock().unwrap(), [fallback_id]);
}

// oracle: mutant sending a queued RDPEWA request after its start guard returns false.
#[test]
fn server_rpc_start_guard_aborts_without_sending_request() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&calls);
    let mut server = RdpewaServerRpc::iuvpaa(
        [0x81; 16],
        Box::new(move |result| {
            recorded.lock().unwrap_or_else(|e| e.into_inner()).push(result);
        }),
    )
    .with_start_guard(Box::new(|| false));

    let outbound = server.start(24).unwrap();
    assert!(outbound.is_empty());
    // oracle: spec MS-ERREF §2.1; E_ABORT (0x80004004).
    assert_eq!(calls.lock().unwrap().as_slice(), [Err(0x8000_4004)]);
    drop(server);
    assert_eq!(calls.lock().unwrap().as_slice(), [Err(0x8000_4004)]);
}

// oracle: spec MS-RDPEWA §§2.2.2 and 2.2.2.1; MS-ERREF §2.1 E_FAIL (0x80004005).
#[test]
fn server_webauthn_rpc_propagates_client_hresult_and_rejects_invalid_responses() {
    use ironrdp_rdpewa::pdu::{RdpewaResponse, S_OK, WebAuthnResponsePayload};

    let make_request = || RdpewaWebAuthnRequest {
        rp_id: Some("localhost".to_owned()),
        timeout_ms: 30_000,
        transaction_id: [0x91; 16],
        client_data_json: b"{}".to_vec(),
        para: WebAuthnPara::default(),
        body: WebAuthnRequestBody {
            subcommand: WebAuthnSubcommand::MakeCredential,
            ctap_cbor: vec![0xA0],
        },
    };

    let hresult_outcome = Arc::new(Mutex::new(Vec::new()));
    let recorded_hresult = Arc::clone(&hresult_outcome);
    let mut hresult_server = RdpewaServerRpc::webauthn(
        make_request(),
        Box::new(move |result| {
            recorded_hresult.lock().unwrap_or_else(|e| e.into_inner()).push(result);
        }),
    );
    let _ = hresult_server.start(25).unwrap();
    let error_wire = RdpewaResponse::from_hresult(0x8009_0027).to_bytes();
    assert!(hresult_server.process(25, &error_wire).unwrap().is_empty());
    assert_eq!(hresult_outcome.lock().unwrap().as_slice(), [Err(0x8009_0027)]);

    let empty_ctap_outcome = Arc::new(Mutex::new(Vec::new()));
    let recorded_empty = Arc::clone(&empty_ctap_outcome);
    let mut empty_ctap_server = RdpewaServerRpc::webauthn(
        make_request(),
        Box::new(move |result| {
            recorded_empty.lock().unwrap_or_else(|e| e.into_inner()).push(result);
        }),
    );
    let _ = empty_ctap_server.start(26).unwrap();
    let empty_payload = WebAuthnResponsePayload {
        device_info: DeviceInfo::default(),
        status: 0,
        response: Vec::new(),
    }
    .encode()
    .unwrap();
    let empty_wire = RdpewaResponse::with_payload(S_OK, empty_payload).to_bytes();
    assert!(empty_ctap_server.process(26, &empty_wire).unwrap().is_empty());
    // oracle: spec MS-ERREF §2.1; E_FAIL (0x80004005).
    assert_eq!(empty_ctap_outcome.lock().unwrap().as_slice(), [Err(0x8000_4005)]);

    let truncated_outcome = Arc::new(Mutex::new(Vec::new()));
    let recorded_truncated = Arc::clone(&truncated_outcome);
    let mut truncated_server = RdpewaServerRpc::webauthn(
        make_request(),
        Box::new(move |result| {
            recorded_truncated
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(result);
        }),
    );
    let _ = truncated_server.start(27).unwrap();
    assert!(truncated_server.process(27, &[0x00, 0x00]).is_err());
    assert_eq!(truncated_outcome.lock().unwrap().as_slice(), [Err(0x8000_4005)]);
    drop(truncated_server);
    assert_eq!(truncated_outcome.lock().unwrap().as_slice(), [Err(0x8000_4005)]);
}
