use ironrdp_core::{Decode as _, Encode as _, ReadCursor, encode_vec};
use ironrdp_dvc as _;
use ironrdp_pdu as _;
use ironrdp_rdpear::ndr::*;
use ironrdp_svc as _;

fn octets() -> OctetString {
    OctetString {
        length: 3,
        value: Some(vec![0xf1, 0x82, 0x73]),
    }
}
fn asn1() -> Asn1Data {
    Asn1Data {
        pdu: 0xfedc_ba98,
        data: octets(),
    }
}
fn key() -> EncryptionKey {
    EncryptionKey {
        reserved1: ReservedPointer,
        reserved2: -197,
        reserved3: octets(),
    }
}
fn unicode() -> UnicodeString {
    UnicodeString {
        length: 6,
        maximum_length: 12,
        buffer: Some(vec![0x41, 0xd83d, 0xde00]),
    }
}
fn secrets() -> EncryptedSecrets {
    EncryptedSecrets {
        reserved1: 1,
        reserved2: 0xfe,
        reserved3: 0x80,
        reserved4: CredentialKeyType::DomainUser,
        reserved5: Opaque([0xa5; 20]),
        reserved6: octets(),
    }
}
fn output() -> OutputBuffer {
    OutputBuffer {
        length: Some(3),
        value: Some(Some(vec![0xf1, 0x82, 0x73])),
    }
}

macro_rules! assert_body_round_trip {
    ($ty:ident, $value:ident, $serialized:ident, $endian:ident) => {
        let body = $value.encode_body_with_limits($endian, NdrLimits::default()).unwrap();
        assert_eq!(body, $serialized[16..16 + body.len()]);
        assert_eq!($serialized.len() - 16 - body.len(), (8 - body.len() % 8) % 8);
        assert_eq!(
            $ty::decode_body_with_limits(&body, $endian, NdrLimits::default()).unwrap(),
            $value
        );
        assert!($ty::decode_body_with_limits(&$serialized, $endian, NdrLimits::default()).is_err());
        assert!($ty::decode_with_limits(&body, NdrLimits::default()).is_err());
        let mut carrier = vec![0xa5; 3];
        carrier.extend_from_slice(&body);
        assert_eq!(
            $ty::decode_body_with_limits(&carrier[3..], $endian, NdrLimits::default()).unwrap(),
            $value
        );
        for end in 0..body.len() {
            assert!(
                $ty::decode_body_with_limits(&body[..end], $endian, NdrLimits::default()).is_err(),
                "raw truncation {end}"
            );
        }
        for suffix in [&[0][..], &[0; 8][..], &body[..]] {
            let mut trailing = body.clone();
            trailing.extend_from_slice(suffix);
            let error = $ty::decode_body_with_limits(&trailing, $endian, NdrLimits::default()).unwrap_err();
            assert_eq!(error.reason, "trailing NDR object data");
            assert_eq!(error.offset, body.len());
        }
    };
}

macro_rules! assert_native_round_trip {
    ($ty:ident, $value:ident, $id:literal, $endian:ident, $response:literal) => {
        let profile = NdrProfile::WindowsNative;
        let bytes = $value
            .encode_with_profile($endian, NdrLimits::default(), profile)
            .unwrap();
        let root = if stringify!($ty).starts_with("Kerberos") {
            24
        } else {
            20
        };
        let public_id: u16 = $id;
        let wire_id = if (0x201..=0x204).contains(&public_id) {
            public_id + 1
        } else {
            public_id
        };
        let id = match $endian {
            Endianness::Little => wire_id.to_le_bytes(),
            Endianness::Big => wire_id.to_be_bytes(),
        };
        assert_eq!(
            bytes[16..20],
            match $endian {
                Endianness::Little => 1u32.to_le_bytes(),
                Endianness::Big => 1u32.to_be_bytes(),
            }
        );
        assert_eq!(bytes[20..root], vec![0; root - 20]);
        assert_eq!(bytes[root..root + 2], id);
        let switch = root + if $response { 8 } else { 2 };
        assert_eq!(bytes[switch..switch + 2], id);
        assert_eq!(
            $ty::decode_with_profile(&bytes, NdrLimits::default(), profile).unwrap(),
            $value
        );
        assert_eq!(
            $ty::decode_with_profile(
                &bytes,
                NdrLimits {
                    max_message_size: bytes.len(),
                    ..NdrLimits::default()
                },
                profile
            )
            .unwrap(),
            $value
        );
        assert!(
            $ty::decode_with_profile(
                &bytes,
                NdrLimits {
                    max_message_size: bytes.len() - 1,
                    ..NdrLimits::default()
                },
                profile
            )
            .is_err()
        );
        let body = $value
            .encode_body_with_profile($endian, NdrLimits::default(), profile)
            .unwrap();
        assert_eq!(body, bytes[16..16 + body.len()]);
        assert_eq!(
            $ty::decode_body_with_profile(&body, $endian, NdrLimits::default(), profile).unwrap(),
            $value
        );
        for end in 0..bytes.len() {
            assert!(
                $ty::decode_with_profile(&bytes[..end], NdrLimits::default(), profile).is_err(),
                "native truncation {end}"
            );
        }
        for end in 0..body.len() {
            assert!(
                $ty::decode_body_with_profile(&body[..end], $endian, NdrLimits::default(), profile).is_err(),
                "native body truncation {end}"
            );
        }
        let mut trailing = body;
        trailing.push(0);
        assert!($ty::decode_body_with_profile(&trailing, $endian, NdrLimits::default(), profile).is_err());
    };
}

macro_rules! request_test {
    ($test:ident, $ty:ident, $id:literal, $value:expr) => {
        #[test]
        fn $test() {
            let value: $ty = $value;
            for endian in [Endianness::Little, Endianness::Big] {
                let bytes = value.encode_with_limits(endian, NdrLimits::default()).unwrap();
                let call_id: u16 = $id;
                let id = match endian {
                    Endianness::Little => call_id.to_le_bytes(),
                    Endianness::Big => call_id.to_be_bytes(),
                };
                assert_eq!(bytes[16..18], id);
                assert_eq!(bytes[18..20], id);
                assert_eq!(
                    $ty::decode_with_limits(&bytes, NdrLimits::default()).unwrap(),
                    value
                );
                assert_body_round_trip!($ty, value, bytes, endian);
                assert_native_round_trip!($ty, value, $id, endian, false);
                for end in 0..bytes.len() {
                    assert!(
                        $ty::decode_with_limits(&bytes[..end], NdrLimits::default()).is_err(),
                        "truncation {end}"
                    );
                }
            }
            let bytes = encode_vec(&value).unwrap();
            assert_eq!($ty::decode(&mut ReadCursor::new(&bytes)).unwrap(), value);
            assert_eq!(value.size(), bytes.len());
        }
    };
}
macro_rules! response_test {
    ($test:ident, $ty:ident, $id:literal, $call:expr) => {
        #[test]
        fn $test() {
            let value = $ty {
                status: -2_147_483_000,
                call: $call,
            };
            for endian in [Endianness::Little, Endianness::Big] {
                let bytes = value.encode_with_limits(endian, NdrLimits::default()).unwrap();
                let call_id: u16 = $id;
                let id = match endian {
                    Endianness::Little => call_id.to_le_bytes(),
                    Endianness::Big => call_id.to_be_bytes(),
                };
                assert_eq!(bytes[16..18], id);
                assert_eq!(bytes[24..26], id);
                assert_eq!(
                    $ty::decode_with_limits(&bytes, NdrLimits::default()).unwrap(),
                    value
                );
                assert_body_round_trip!($ty, value, bytes, endian);
                assert_native_round_trip!($ty, value, $id, endian, true);
                for end in 0..bytes.len() {
                    assert!(
                        $ty::decode_with_limits(&bytes[..end], NdrLimits::default()).is_err(),
                        "truncation {end}"
                    );
                }
            }
            let bytes = encode_vec(&value).unwrap();
            assert_eq!($ty::decode(&mut ReadCursor::new(&bytes)).unwrap(), value);
            assert_eq!(value.size(), bytes.len());
        }
    };
}

request_test!(
    kerb_negotiate_request,
    KerberosRequest,
    0x100,
    KerberosRequest::NegotiateVersion(NegotiateVersion { version: 0x1234_5678 })
);
request_test!(
    kerb_build_as_req_request,
    KerberosRequest,
    0x101,
    KerberosRequest::BuildAsReqAuthenticator(BuildAsReqAuthenticatorRequest {
        encryption_key: Some(key()),
        armor_key: Some(key()),
        time_skew: Some(i64::MIN + 123),
    })
);
request_test!(
    kerb_verify_service_ticket_request,
    KerberosRequest,
    0x102,
    KerberosRequest::VerifyServiceTicket(VerifyServiceTicketRequest {
        packed_ticket: Some(asn1()),
        service_key: Some(key()),
        time_skew: Some(-0x1234_5678_9abc),
    })
);
request_test!(
    kerb_create_ap_req_request,
    KerberosRequest,
    0x103,
    KerberosRequest::CreateApReqAuthenticator(CreateApReqAuthenticatorRequest {
        encryption_key: Some(key()),
        sequence_number: u32::MAX,
        client_name: Some(InternalName {
            name_type: -31,
            name_count: 2,
            names: Some(vec![unicode(), unicode()])
        }),
        client_realm: Some(unicode()),
        skew_time: Some(i64::MAX),
        sub_key: Some(key()),
        auth_data: Some(asn1()),
        gss_checksum: Some(asn1()),
        key_usage: 0xfefe_abab,
    })
);
request_test!(
    kerb_decrypt_ap_reply_request,
    KerberosRequest,
    0x104,
    KerberosRequest::DecryptApReply(DecryptApReplyRequest {
        encrypted_reply: Some(asn1()),
        key: Some(key())
    })
);
request_test!(
    kerb_unpack_kdc_reply_request,
    KerberosRequest,
    0x105,
    KerberosRequest::UnpackKdcReplyBody(UnpackKdcReplyBodyRequest {
        encrypted_data: Some(asn1()),
        key: Some(key()),
        strengthen_key: Some(key()),
        pdu: u32::MAX,
        key_usage: 9,
    })
);
request_test!(
    kerb_compute_tgs_checksum_request,
    KerberosRequest,
    0x106,
    KerberosRequest::ComputeTgsChecksum(ComputeTgsChecksumRequest {
        request_body: Some(asn1()),
        key: Some(key()),
        checksum_type: u32::MAX
    })
);
request_test!(
    kerb_build_encrypted_auth_request,
    KerberosRequest,
    0x107,
    KerberosRequest::BuildEncryptedAuthData(BuildEncryptedAuthDataRequest {
        key_usage: u32::MAX,
        key: Some(key()),
        plain_auth_data: Some(asn1())
    })
);
request_test!(
    kerb_pack_ap_reply_request,
    KerberosRequest,
    0x108,
    KerberosRequest::PackApReply(PackApReplyRequest {
        reply: Some(asn1()),
        reply_body: Some(asn1()),
        session_key: Some(key())
    })
);
request_test!(
    kerb_hash_s4u_request,
    KerberosRequest,
    0x109,
    KerberosRequest::HashS4UPreauth(HashS4UPreauthRequest {
        s4u_preauth: Some(asn1()),
        key: Some(key()),
        checksum_type: i32::MIN
    })
);
request_test!(
    kerb_sign_s4u_request,
    KerberosRequest,
    0x10a,
    KerberosRequest::SignS4UPreauthData(SignS4UPreauthDataRequest {
        key: Some(key()),
        is_request: -1,
        user_id: Some(asn1()),
        checksum_type: Some(-197)
    })
);
request_test!(
    kerb_verify_checksum_request,
    KerberosRequest,
    0x10b,
    KerberosRequest::VerifyChecksum(VerifyChecksumRequest {
        key: Some(key()),
        checksum_type: 31,
        expected_checksum: octets(),
        data_to_check: octets()
    })
);
request_test!(
    kerb_decrypt_pac_request,
    KerberosRequest,
    0x113,
    KerberosRequest::DecryptPacCredentials(DecryptPacCredentialsRequest {
        key: Some(key()),
        version: u32::MAX,
        encryption_type: 18,
        data: octets()
    })
);
request_test!(
    kerb_create_ecdh_request,
    KerberosRequest,
    0x114,
    KerberosRequest::CreateEcdhKeyAgreement(CreateEcdhKeyAgreementRequest { key_bit_len: 521 })
);
request_test!(
    kerb_create_dh_request,
    KerberosRequest,
    0x115,
    KerberosRequest::CreateDhKeyAgreement(Ignored { ignored: 0xa5 })
);
request_test!(
    kerb_destroy_key_request,
    KerberosRequest,
    0x116,
    KerberosRequest::DestroyKeyAgreement(KeyAgreementRequest {
        key_agreement_handle: -1
    })
);
request_test!(
    kerb_generate_nonce_request,
    KerberosRequest,
    0x117,
    KerberosRequest::KeyAgreementGenerateNonce(KeyAgreementRequest {
        key_agreement_handle: i64::MIN
    })
);
request_test!(
    kerb_finalize_key_request,
    KerberosRequest,
    0x118,
    KerberosRequest::FinalizeKeyAgreement(FinalizeKeyAgreementRequest {
        key_agreement_handle: Some(i64::MAX),
        kerb_etype: u32::MAX,
        remote_nonce: octets(),
        x509_public_key: octets()
    })
);

response_test!(
    kerb_negotiate_response,
    KerberosResponse,
    0x100,
    KerberosResponseCall::NegotiateVersion(NegotiateVersion { version: u32::MAX })
);
response_test!(
    kerb_build_as_req_response,
    KerberosResponse,
    0x101,
    KerberosResponseCall::BuildAsReqAuthenticator(BuildAsReqAuthenticatorResponse {
        preauth_data_type: i32::MIN,
        preauth_data: octets()
    })
);
response_test!(
    kerb_verify_service_ticket_response,
    KerberosResponse,
    0x102,
    KerberosResponseCall::VerifyServiceTicket(VerifyServiceTicketResponse {
        decrypted_ticket: asn1(),
        kerb_protocol_error: i32::MIN
    })
);
response_test!(
    kerb_create_ap_req_response,
    KerberosResponse,
    0x103,
    KerberosResponseCall::CreateApReqAuthenticator(CreateApReqAuthenticatorResponse {
        authenticator_time: i64::MIN,
        authenticator: asn1(),
        kerb_protocol_error: i32::MAX
    })
);
response_test!(
    kerb_decrypt_ap_reply_response,
    KerberosResponse,
    0x104,
    KerberosResponseCall::DecryptApReply(asn1())
);
response_test!(
    kerb_unpack_kdc_reply_response,
    KerberosResponse,
    0x105,
    KerberosResponseCall::UnpackKdcReplyBody(UnpackKdcReplyBodyResponse {
        kerb_protocol_error: i32::MIN,
        reply_body: asn1()
    })
);
response_test!(
    kerb_compute_tgs_checksum_response,
    KerberosResponse,
    0x106,
    KerberosResponseCall::ComputeTgsChecksum(asn1())
);
response_test!(
    kerb_build_encrypted_auth_response,
    KerberosResponse,
    0x107,
    KerberosResponseCall::BuildEncryptedAuthData(asn1())
);
response_test!(
    kerb_pack_ap_reply_response,
    KerberosResponse,
    0x108,
    KerberosResponseCall::PackApReply(octets())
);
response_test!(
    kerb_hash_s4u_response,
    KerberosResponse,
    0x109,
    KerberosResponseCall::HashS4UPreauth(output())
);
response_test!(
    kerb_sign_s4u_response,
    KerberosResponse,
    0x10a,
    KerberosResponseCall::SignS4UPreauthData(SignS4UPreauthDataResponse {
        checksum_type: Some(i32::MIN),
        checksum: output()
    })
);
response_test!(
    kerb_verify_checksum_response,
    KerberosResponse,
    0x10b,
    KerberosResponseCall::VerifyChecksum(BoolResponse { value: i32::MIN })
);
response_test!(
    kerb_decrypt_pac_response,
    KerberosResponse,
    0x113,
    KerberosResponseCall::DecryptPacCredentials(DecryptPacCredentialsResponse {
        credentials: Some(SupplementalCredentials {
            credentials: vec![
                SupplementalCredential {
                    package_name: unicode(),
                    credentials: octets()
                },
                SupplementalCredential {
                    package_name: unicode(),
                    credentials: octets()
                },
            ]
        }),
    })
);
response_test!(
    kerb_create_ecdh_response,
    KerberosResponse,
    0x114,
    KerberosResponseCall::CreateEcdhKeyAgreement(CreateEcdhKeyAgreementResponse {
        key_agreement_handle: Some(i64::MIN),
        kerb_err: Some(i32::MIN),
        encoded_pub_key: output()
    })
);
response_test!(
    kerb_create_dh_response,
    KerberosResponse,
    0x115,
    KerberosResponseCall::CreateDhKeyAgreement(CreateDhKeyAgreementResponse {
        modulus_p: Some(octets()),
        generator_g: Some(octets()),
        factor_q: Some(octets()),
        key_agreement_handle: Some(i64::MAX),
        kerb_err: Some(i32::MIN),
        little_endian_public_key: output()
    })
);
response_test!(
    kerb_destroy_key_response,
    KerberosResponse,
    0x116,
    KerberosResponseCall::DestroyKeyAgreement(Ignored { ignored: 0x5a })
);
response_test!(
    kerb_generate_nonce_response,
    KerberosResponse,
    0x117,
    KerberosResponseCall::KeyAgreementGenerateNonce(output())
);
response_test!(
    kerb_finalize_key_response,
    KerberosResponse,
    0x118,
    KerberosResponseCall::FinalizeKeyAgreement(FinalizeKeyAgreementResponse {
        shared_key: Some(key())
    })
);

request_test!(
    ntlm_negotiate_request,
    NtlmRequest,
    0x200,
    NtlmRequest::NegotiateVersion(NegotiateVersion { version: u32::MAX })
);
request_test!(
    ntlm_lm20_response_request,
    NtlmRequest,
    0x201,
    NtlmRequest::Lm20GetNtlm3ChallengeResponse(Lm20GetNtlm3ChallengeResponseRequest {
        credential: Some(secrets()),
        user_name: Some(unicode()),
        logon_domain_name: Some(unicode()),
        server_name: Some(unicode()),
        challenge_to_client: Opaque([0xff; 8]),
    })
);
request_test!(
    ntlm_calculate_nt_response_request,
    NtlmRequest,
    0x202,
    NtlmRequest::CalculateNtResponse(CalculateNtResponseRequest {
        nt_challenge: Some(Opaque([0xa5; 8])),
        credential: Some(secrets())
    })
);
request_test!(
    ntlm_calculate_session_key_request,
    NtlmRequest,
    0x203,
    NtlmRequest::CalculateUserSessionKeyNt(CalculateUserSessionKeyNtRequest {
        nt_response: Some(Opaque([0xa5; 24])),
        credential: Some(secrets())
    })
);
request_test!(
    ntlm_compare_credentials_request,
    NtlmRequest,
    0x204,
    NtlmRequest::CompareCredentials(CompareCredentialsRequest {
        lhs_credential: Some(secrets()),
        rhs_credential: Some(secrets())
    })
);
response_test!(
    ntlm_negotiate_response,
    NtlmResponse,
    0x200,
    NtlmResponseCall::NegotiateVersion(NegotiateVersion { version: u32::MAX })
);
response_test!(
    ntlm_lm20_response_response,
    NtlmResponse,
    0x201,
    NtlmResponseCall::Lm20GetNtlm3ChallengeResponse(Lm20GetNtlm3ChallengeResponseResponse {
        ntlm3_response: NtlmResponseBuffer {
            length: 3,
            value: Some(vec![0xa5, 0x5a, 0xf0])
        },
        lm3_response: Opaque([0xfa; 24]),
        user_session_key: Opaque([0xff; 16]),
        lm_session_key: Opaque([0xa5; 8]),
    })
);
response_test!(
    ntlm_calculate_nt_response_response,
    NtlmResponse,
    0x202,
    NtlmResponseCall::CalculateNtResponse(Opaque([0xa5; 24]))
);
response_test!(
    ntlm_calculate_session_key_response,
    NtlmResponse,
    0x203,
    NtlmResponseCall::CalculateUserSessionKeyNt(Opaque([0xfa; 16]))
);
response_test!(
    ntlm_compare_credentials_response,
    NtlmResponse,
    0x204,
    NtlmResponseCall::CompareCredentials(CompareCredentialsResponse {
        are_nt_owfs_equal: i32::MIN,
        are_lm_owfs_equal: -1,
        are_sha_owfs_equal: i32::MAX
    })
);

fn hex(input: &str) -> Vec<u8> {
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digits = core::str::from_utf8(pair).expect("fixture hexadecimal is ASCII");
            u8::from_str_radix(digits, 16).expect("fixture contains hexadecimal octets")
        })
        .collect()
}

macro_rules! native_midl_vector {
    ($test:ident, $ty:ident, $hex:literal, $body_len:literal, [$($offset:literal),*], $value:expr) => {
        #[test]
        fn $test() {
            let oracle = hex($hex);
            let value: $ty = $value;
            let profile = NdrProfile::WindowsNative;
            assert_eq!($ty::decode_with_profile(&oracle, NdrLimits::default(), profile).unwrap(), value);
            assert_eq!($ty::decode_body_with_profile(&oracle[16..16 + $body_len], Endianness::Little, NdrLimits::default(), profile).unwrap(), value);
            let mut canonical = oracle;
            let mut referent = 1u32;
            for offset in [$($offset),*] {
                canonical[offset..offset + 4].copy_from_slice(&referent.to_le_bytes());
                referent += 1;
            }
            assert_eq!(value.encode_with_profile(Endianness::Little, NdrLimits::default(), profile).unwrap(), canonical);
            assert_eq!(value.encode_body_with_profile(Endianness::Little, NdrLimits::default(), profile).unwrap(), canonical[16..16 + $body_len]);
        }
    };
}

native_midl_vector!(
    independent_native_midl_kerberos_request_root,
    KerberosRequest,
    "01100800cccccccc100000000000000000000200000000000001000178563412",
    16,
    [16],
    KerberosRequest::NegotiateVersion(NegotiateVersion { version: 0x1234_5678 })
);
native_midl_vector!(
    independent_native_midl_kerberos_response_root,
    KerberosResponse,
    "01100800cccccccc180000000000000000000200000000000001000085ffffff0001000078563412",
    24,
    [16],
    KerberosResponse {
        status: -123,
        call: KerberosResponseCall::NegotiateVersion(NegotiateVersion { version: 0x1234_5678 })
    }
);
native_midl_vector!(
    independent_native_midl_ignored_key_slot,
    KerberosRequest,
    "01100800cccccccc380000000000000000000200000000000101010104000200000000000c0002000000000085ffffff03000000080002000300000001020300eb32a4f8ffffffff",
    56,
    [16, 28, 36, 52],
    KerberosRequest::BuildAsReqAuthenticator(BuildAsReqAuthenticatorRequest {
        encryption_key: Some(EncryptionKey {
            reserved1: ReservedPointer,
            reserved2: -123,
            reserved3: OctetString {
                length: 3,
                value: Some(vec![1, 2, 3])
            }
        }),
        armor_key: None,
        time_skew: Some(-123456789)
    })
);
native_midl_vector!(
    independent_native_midl_double_pointer,
    KerberosResponse,
    "01100800cccccccc300000000000000000000200000000000901000085ffffff090100000400020008000200030000000c000200030000000102030000000000",
    43,
    [16, 36, 40, 48],
    KerberosResponse {
        status: -123,
        call: KerberosResponseCall::HashS4UPreauth(OutputBuffer {
            length: Some(3),
            value: Some(Some(vec![1, 2, 3]))
        })
    }
);
native_midl_vector!(
    independent_native_midl_ntlm_request_root,
    NtlmRequest,
    "01100800cccccccc100000000000000000000200000200027856341200000000",
    12,
    [16],
    NtlmRequest::NegotiateVersion(NegotiateVersion { version: 0x1234_5678 })
);
native_midl_vector!(
    independent_native_midl_ntlm_response_mapping,
    NtlmResponse,
    "01100800cccccccc2000000000000000000002000502000085ffffff05020000ffffffff010000000000000000000000",
    28,
    [16],
    NtlmResponse {
        status: -123,
        call: NtlmResponseCall::CompareCredentials(CompareCredentialsResponse {
            are_nt_owfs_equal: -1,
            are_lm_owfs_equal: 1,
            are_sha_owfs_equal: 0
        })
    }
);

#[test]
fn native_ignored_key_slot_is_consumed_without_referent_or_value_constraint() {
    let value = KerberosRequest::BuildAsReqAuthenticator(BuildAsReqAuthenticatorRequest {
        encryption_key: Some(key()),
        armor_key: None,
        time_skew: Some(-1),
    });
    for endian in [Endianness::Little, Endianness::Big] {
        let original = value
            .encode_with_profile(endian, NdrLimits::default(), NdrProfile::WindowsNative)
            .unwrap();
        assert_eq!(original[40..44], [0; 4]);
        for slot in [0, 1, u32::MAX, 0x1234_5678] {
            let mut bytes = original.clone();
            bytes[40..44].copy_from_slice(&match endian {
                Endianness::Little => slot.to_le_bytes(),
                Endianness::Big => slot.to_be_bytes(),
            });
            assert_eq!(
                KerberosRequest::decode_with_profile(&bytes, NdrLimits::default(), NdrProfile::WindowsNative).unwrap(),
                value
            );
            assert_eq!(
                value
                    .encode_with_profile(endian, NdrLimits::default(), NdrProfile::WindowsNative)
                    .unwrap(),
                original
            );
        }
    }
}

#[test]
fn native_root_pointer_accepts_arbitrary_nonzero_ids_but_rejects_null() {
    macro_rules! check {
        ($ty:ident, $value:expr) => {
            let value: $ty = $value;
            for endian in [Endianness::Little, Endianness::Big] {
                let original = value
                    .encode_with_profile(endian, NdrLimits::default(), NdrProfile::WindowsNative)
                    .unwrap();
                for id in [1, u32::MAX, 0x0002_0000] {
                    let mut bytes = original.clone();
                    bytes[16..20].copy_from_slice(&match endian {
                        Endianness::Little => id.to_le_bytes(),
                        Endianness::Big => id.to_be_bytes(),
                    });
                    assert_eq!(
                        $ty::decode_with_profile(&bytes, NdrLimits::default(), NdrProfile::WindowsNative).unwrap(),
                        value
                    );
                }
                let mut null = original;
                null[16..20].fill(0);
                let error =
                    $ty::decode_with_profile(&null, NdrLimits::default(), NdrProfile::WindowsNative).unwrap_err();
                assert_eq!(error.reason, "native NDR call root must not be null");
                assert_eq!(error.offset, 20);
            }
        };
    }
    check!(
        KerberosRequest,
        KerberosRequest::NegotiateVersion(NegotiateVersion { version: 0 })
    );
    check!(
        KerberosResponse,
        KerberosResponse {
            status: 0,
            call: KerberosResponseCall::NegotiateVersion(NegotiateVersion { version: 0 })
        }
    );
    check!(
        NtlmRequest,
        NtlmRequest::NegotiateVersion(NegotiateVersion { version: 0 })
    );
    check!(
        NtlmResponse,
        NtlmResponse {
            status: 0,
            call: NtlmResponseCall::NegotiateVersion(NegotiateVersion { version: 0 })
        }
    );
}

#[test]
fn native_private_ntlm_operation_is_rejected_without_profile_fallback() {
    for endian in [Endianness::Little, Endianness::Big] {
        let id = match endian {
            Endianness::Little => 0x201u16.to_le_bytes(),
            Endianness::Big => 0x201u16.to_be_bytes(),
        };
        let request = NtlmRequest::NegotiateVersion(NegotiateVersion { version: 0 });
        let mut bytes = request
            .encode_with_profile(endian, NdrLimits::default(), NdrProfile::WindowsNative)
            .unwrap();
        bytes[20..22].copy_from_slice(&id);
        bytes[22..24].copy_from_slice(&id);
        assert_eq!(
            NtlmRequest::decode_with_profile(&bytes, NdrLimits::default(), NdrProfile::WindowsNative)
                .unwrap_err()
                .reason,
            "unsupported private NTLM operation"
        );
        let response = NtlmResponse {
            status: 0,
            call: NtlmResponseCall::NegotiateVersion(NegotiateVersion { version: 0 }),
        };
        let mut bytes = response
            .encode_with_profile(endian, NdrLimits::default(), NdrProfile::WindowsNative)
            .unwrap();
        bytes[20..22].copy_from_slice(&id);
        bytes[28..30].copy_from_slice(&id);
        assert_eq!(
            NtlmResponse::decode_with_profile(&bytes, NdrLimits::default(), NdrProfile::WindowsNative)
                .unwrap_err()
                .reason,
            "unsupported private NTLM operation"
        );
        assert_eq!(request.wire_call_id(NdrProfile::Published), 0x200);
        assert_eq!(request.wire_call_id(NdrProfile::WindowsNative), 0x200);
    }
}

#[test]
fn native_profile_preserves_allocation_and_element_limits_for_all_roots() {
    macro_rules! check {
        ($ty:ident, $value:expr) => {
            let value: $ty = $value;
            let profile = NdrProfile::WindowsNative;
            for endian in [Endianness::Little, Endianness::Big] {
                let bytes = value
                    .encode_with_profile(endian, NdrLimits::default(), profile)
                    .unwrap();
                let body = value
                    .encode_body_with_profile(endian, NdrLimits::default(), profile)
                    .unwrap();
                for limits in [
                    NdrLimits {
                        max_allocation: 2,
                        ..NdrLimits::default()
                    },
                    NdrLimits {
                        max_elements: 2,
                        ..NdrLimits::default()
                    },
                ] {
                    assert_eq!(
                        $ty::decode_with_profile(&bytes, limits, profile)
                            .unwrap_err()
                            .reason,
                        "NDR decoding exceeds limit"
                    );
                    assert_eq!(
                        $ty::decode_body_with_profile(&body, endian, limits, profile)
                            .unwrap_err()
                            .reason,
                        "NDR decoding exceeds limit"
                    );
                }
                assert_eq!(
                    $ty::decode_with_profile(&bytes, NdrLimits::default(), profile).unwrap(),
                    value
                );
            }
        };
    }
    check!(
        KerberosRequest,
        KerberosRequest::BuildAsReqAuthenticator(BuildAsReqAuthenticatorRequest {
            encryption_key: Some(key()),
            armor_key: None,
            time_skew: None
        })
    );
    check!(
        KerberosResponse,
        KerberosResponse {
            status: 0,
            call: KerberosResponseCall::HashS4UPreauth(output())
        }
    );
    check!(
        NtlmRequest,
        NtlmRequest::CalculateNtResponse(CalculateNtResponseRequest {
            nt_challenge: Some(Opaque([0x55; 8])),
            credential: Some(secrets())
        })
    );
    check!(
        NtlmResponse,
        NtlmResponse {
            status: 0,
            call: NtlmResponseCall::Lm20GetNtlm3ChallengeResponse(Lm20GetNtlm3ChallengeResponseResponse {
                ntlm3_response: NtlmResponseBuffer {
                    length: 3,
                    value: Some(vec![1, 2, 3])
                },
                lm3_response: Opaque([0x55; 24]),
                user_session_key: Opaque([0xaa; 16]),
                lm_session_key: Opaque([0x11; 8])
            })
        }
    );
}

#[test]
fn independent_midl_negotiate_vector() {
    let bytes = hex("01100800cccccccc08000000000000000001000178563412");
    let value = KerberosRequest::NegotiateVersion(NegotiateVersion { version: 0x1234_5678 });
    assert_eq!(
        value
            .encode_with_limits(Endianness::Little, NdrLimits::default())
            .unwrap(),
        bytes
    );
    assert_eq!(
        KerberosRequest::decode_with_limits(&bytes, NdrLimits::default()).unwrap(),
        value
    );
    assert_eq!(
        value
            .encode_body_with_limits(Endianness::Little, NdrLimits::default())
            .unwrap(),
        bytes[16..24]
    );
    assert_eq!(
        KerberosRequest::decode_body_with_limits(&bytes[16..24], Endianness::Little, NdrLimits::default()).unwrap(),
        value
    );
}

macro_rules! midl_vector {
    ($test:ident, $ty:ident, $hex:literal, $body_len:literal, [$($offset:literal),*], $value:expr) => {
        #[test]
        fn $test() {
            let oracle = hex($hex);
            let value: $ty = $value;
            assert_eq!($ty::decode_with_limits(&oracle, NdrLimits::default()).unwrap(), value);
            // Strip the two eight-byte RPC headers and the fixture's serialization-only end padding.
            let body = &oracle[16..16 + $body_len];
            assert_eq!(oracle.len() - 16 - $body_len, (8 - $body_len % 8) % 8);
            assert_eq!($ty::decode_body_with_limits(body, Endianness::Little, NdrLimits::default()).unwrap(), value);
            let mut canonical = oracle;
            let mut referent = 1u32;
            for offset in [$($offset),*] {
                canonical[offset..offset + 4].copy_from_slice(&referent.to_le_bytes());
                referent += 1;
            }
            assert_eq!(value.encode_with_limits(Endianness::Little, NdrLimits::default()).unwrap(), canonical);
            assert_eq!(
                value.encode_body_with_limits(Endianness::Little, NdrLimits::default()).unwrap(),
                canonical[16..16 + $body_len]
            );
        }
    };
}

midl_vector!(
    independent_midl_nested_null_key_layout,
    KerberosRequest,
    "01100800cccccccc3000000000000000010101010000020000000000080002000000000085ffffff03000000040002000300000001020300eb32a4f8ffffffff",
    48,
    [20, 28, 44],
    KerberosRequest::BuildAsReqAuthenticator(BuildAsReqAuthenticatorRequest {
        encryption_key: Some(EncryptionKey {
            reserved1: ReservedPointer,
            reserved2: -123,
            reserved3: OctetString {
                length: 3,
                value: Some(vec![1, 2, 3])
            }
        }),
        armor_key: None,
        time_skew: Some(-123456789),
    })
);
midl_vector!(
    independent_midl_pointer_to_pointer,
    KerberosResponse,
    "01100800cccccccc28000000000000000901000085ffffff0901000000000200040002000300000008000200030000000102030000000000",
    35,
    [28, 32, 40],
    KerberosResponse {
        status: -123,
        call: KerberosResponseCall::HashS4UPreauth(OutputBuffer {
            length: Some(3),
            value: Some(Some(vec![1, 2, 3]))
        })
    }
);
midl_vector!(
    independent_midl_conformant_credentials,
    KerberosResponse,
    "01100800cccccccc68000000000000001301000085ffffff1301000000000200020000000200000002000400040002000300000008000200020006000c000200030000001000020002000000000000000100000041000000030000000102030003000000000000000100000000d800000300000001020300",
    103,
    [28, 44, 52, 60, 68],
    KerberosResponse {
        status: -123,
        call: KerberosResponseCall::DecryptPacCredentials(DecryptPacCredentialsResponse {
            credentials: Some(SupplementalCredentials {
                credentials: vec![
                    SupplementalCredential {
                        package_name: UnicodeString {
                            length: 2,
                            maximum_length: 4,
                            buffer: Some(vec![0x41])
                        },
                        credentials: OctetString {
                            length: 3,
                            value: Some(vec![1, 2, 3])
                        }
                    },
                    SupplementalCredential {
                        package_name: UnicodeString {
                            length: 2,
                            maximum_length: 6,
                            buffer: Some(vec![0xd800])
                        },
                        credentials: OctetString {
                            length: 3,
                            value: Some(vec![1, 2, 3])
                        }
                    },
                ]
            })
        })
    }
);
midl_vector!(
    independent_midl_ntlm_secret_enum_and_unicode,
    NtlmRequest,
    "01100800cccccccc90000000000000000102010200000200080002001000020018000200ffffffffffffffff01fe80000200a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5000003000000040002000300000001020300020004000c00020002000000000000000100000041000000020004001400020002000000000000000100000041000000020004001c00020002000000000000000100000041000000",
    142,
    [20, 24, 28, 32, 76, 92, 116, 140],
    NtlmRequest::Lm20GetNtlm3ChallengeResponse(Lm20GetNtlm3ChallengeResponseRequest {
        credential: Some(EncryptedSecrets {
            reserved6: OctetString {
                length: 3,
                value: Some(vec![1, 2, 3])
            },
            ..secrets()
        }),
        user_name: Some(UnicodeString {
            length: 2,
            maximum_length: 4,
            buffer: Some(vec![0x41])
        }),
        logon_domain_name: Some(UnicodeString {
            length: 2,
            maximum_length: 4,
            buffer: Some(vec![0x41])
        }),
        server_name: Some(UnicodeString {
            length: 2,
            maximum_length: 4,
            buffer: Some(vec![0x41])
        }),
        challenge_to_client: Opaque([0xff; 8]),
    })
);
midl_vector!(
    independent_midl_ntlm_fixed_keys_and_response16,
    NtlmResponse,
    "01100800cccccccc50000000000000000102000085ffffff010200000300000000000200fafafafafafafafafafafafafafafafafafafafafafafafaffffffffffffffffffffffffffffffffa5a5a5a5a5a5a5a5030000000102030000000000",
    75,
    [32],
    NtlmResponse {
        status: -123,
        call: NtlmResponseCall::Lm20GetNtlm3ChallengeResponse(Lm20GetNtlm3ChallengeResponseResponse {
            ntlm3_response: NtlmResponseBuffer {
                length: 3,
                value: Some(vec![1, 2, 3])
            },
            lm3_response: Opaque([0xfa; 24]),
            user_session_key: Opaque([0xff; 16]),
            lm_session_key: Opaque([0xa5; 8]),
        })
    }
);

#[test]
fn independent_big_endian_wide_signed_vector() {
    let bytes = hex("01000800cccccccc00000010000000000116011600000000fedcba9876543210");
    let value = KerberosRequest::DestroyKeyAgreement(KeyAgreementRequest {
        key_agreement_handle: -0x0123_4567_89ab_cdf0,
    });
    assert_eq!(
        value.encode_with_limits(Endianness::Big, NdrLimits::default()).unwrap(),
        bytes
    );
    assert_eq!(
        KerberosRequest::decode_with_limits(&bytes, NdrLimits::default()).unwrap(),
        value
    );
    assert_eq!(
        value
            .encode_body_with_limits(Endianness::Big, NdrLimits::default())
            .unwrap(),
        bytes[16..32]
    );
    assert_eq!(
        KerberosRequest::decode_body_with_limits(&bytes[16..32], Endianness::Big, NdrLimits::default()).unwrap(),
        value
    );
}

#[test]
fn independent_midl_byte_aligned_ntlm_union_arm() {
    let bytes = hex(
        "01100800cccccccc28000000000000000202000085ffffff02025a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a000000000000",
    );
    let value = NtlmResponse {
        status: -123,
        call: NtlmResponseCall::CalculateNtResponse(Opaque([0x5a; 24])),
    };
    assert_eq!(
        value
            .encode_with_limits(Endianness::Little, NdrLimits::default())
            .unwrap(),
        bytes
    );
    assert_eq!(
        NtlmResponse::decode_with_limits(&bytes, NdrLimits::default()).unwrap(),
        value
    );
    assert_eq!(
        value
            .encode_body_with_limits(Endianness::Little, NdrLimits::default())
            .unwrap(),
        bytes[16..50]
    );
    assert_eq!(
        NtlmResponse::decode_body_with_limits(&bytes[16..50], Endianness::Little, NdrLimits::default()).unwrap(),
        value
    );
    assert_eq!(
        NtlmResponse::decode_body_with_limits(&bytes[16..], Endianness::Little, NdrLimits::default())
            .unwrap_err()
            .reason,
        "trailing NDR object data"
    );
}

#[test]
fn null_nonnull_empty_and_independent_lengths() {
    for value in [None, Some(None), Some(Some(Vec::new()))] {
        for length in [None, Some(0), Some(123)] {
            let request = KerberosResponse {
                status: 0,
                call: KerberosResponseCall::HashS4UPreauth(OutputBuffer {
                    length,
                    value: value.clone(),
                }),
            };
            let result = request.encode_with_limits(Endianness::Little, NdrLimits::default());
            if matches!(value, Some(Some(_))) && length != Some(0) {
                assert!(result.is_err());
            } else {
                let encoded = result.unwrap();
                assert_eq!(
                    KerberosResponse::decode_with_limits(&encoded, NdrLimits::default()).unwrap(),
                    request
                );
            }
        }
    }
    for buffer in [
        OctetString {
            length: 123,
            value: None,
        },
        OctetString {
            length: 0,
            value: Some(Vec::new()),
        },
        OctetString::default(),
    ] {
        let response = KerberosResponse {
            status: 0,
            call: KerberosResponseCall::PackApReply(buffer),
        };
        let bytes = response
            .encode_with_limits(Endianness::Little, NdrLimits::default())
            .unwrap();
        assert_eq!(
            KerberosResponse::decode_with_limits(&bytes, NdrLimits::default()).unwrap(),
            response
        );
    }
}

#[test]
fn caller_wire_allocation_and_element_limits() {
    let value = KerberosResponse {
        status: 0,
        call: KerberosResponseCall::PackApReply(octets()),
    };
    let bytes = value
        .encode_with_limits(Endianness::Little, NdrLimits::default())
        .unwrap();
    let limits = NdrLimits {
        max_message_size: bytes.len(),
        max_allocation: 3,
        max_elements: 3,
    };
    assert_eq!(KerberosResponse::decode_with_limits(&bytes, limits).unwrap(), value);
    for limits in [
        NdrLimits {
            max_message_size: bytes.len() - 1,
            ..limits
        },
        NdrLimits {
            max_allocation: 2,
            ..limits
        },
        NdrLimits {
            max_elements: 2,
            ..limits
        },
    ] {
        assert!(KerberosResponse::decode_with_limits(&bytes, limits).is_err());
    }
    assert!(
        value
            .encode_with_limits(
                Endianness::Little,
                NdrLimits {
                    max_message_size: bytes.len(),
                    max_allocation: bytes.len(),
                    max_elements: 3
                }
            )
            .is_ok()
    );
    assert!(
        value
            .encode_with_limits(
                Endianness::Little,
                NdrLimits {
                    max_message_size: bytes.len() - 1,
                    ..NdrLimits::default()
                }
            )
            .is_err()
    );
    assert!(
        value
            .encode_with_limits(
                Endianness::Little,
                NdrLimits {
                    max_allocation: bytes.len() - 1,
                    ..NdrLimits::default()
                }
            )
            .is_err()
    );
    assert!(
        value
            .encode_with_limits(
                Endianness::Little,
                NdrLimits {
                    max_elements: 2,
                    ..NdrLimits::default()
                }
            )
            .is_err()
    );
}

#[test]
fn hostile_count_rejected_before_allocation() {
    let value = KerberosResponse {
        status: 0,
        call: KerberosResponseCall::PackApReply(octets()),
    };
    let mut bytes = value
        .encode_with_limits(Endianness::Little, NdrLimits::default())
        .unwrap();
    bytes[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
    bytes[36..40].copy_from_slice(&u32::MAX.to_le_bytes());
    let error = KerberosResponse::decode_with_limits(
        &bytes,
        NdrLimits {
            max_allocation: usize::MAX,
            max_elements: usize::MAX,
            ..NdrLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.reason, "NDR count exceeds remaining object");
}

#[test]
fn malformed_correlations_and_discriminants() {
    let value = KerberosResponse {
        status: 0,
        call: KerberosResponseCall::PackApReply(octets()),
    };
    let original = value
        .encode_with_limits(Endianness::Little, NdrLimits::default())
        .unwrap();
    let mut bytes = original.clone();
    bytes[36..40].copy_from_slice(&4u32.to_le_bytes());
    assert_eq!(
        KerberosResponse::decode_with_limits(&bytes, NdrLimits::default())
            .unwrap_err()
            .reason,
        "NDR buffer correlation mismatch"
    );
    let mut bytes = original.clone();
    bytes[24] ^= 1;
    assert_eq!(
        KerberosResponse::decode_with_limits(&bytes, NdrLimits::default())
            .unwrap_err()
            .reason,
        "NDR union discriminant mismatch"
    );
    let mut bytes = original;
    bytes[16..18].copy_from_slice(&0x10cu16.to_le_bytes());
    bytes[24..26].copy_from_slice(&0x10cu16.to_le_bytes());
    assert_eq!(
        KerberosResponse::decode_with_limits(&bytes, NdrLimits::default())
            .unwrap_err()
            .reason,
        "unsupported RDPEAR call ID"
    );
}

#[test]
fn unicode_lengths_varying_counts_and_capacity() {
    for string in [
        UnicodeString {
            length: 0,
            maximum_length: 0,
            buffer: None,
        },
        UnicodeString {
            length: 0,
            maximum_length: 1,
            buffer: Some(Vec::new()),
        },
        UnicodeString {
            length: 2,
            maximum_length: 7,
            buffer: Some(vec![0xd800]),
        },
        unicode(),
    ] {
        let value = NtlmRequest::Lm20GetNtlm3ChallengeResponse(Lm20GetNtlm3ChallengeResponseRequest {
            user_name: Some(string),
            ..Default::default()
        });
        let bytes = value
            .encode_with_limits(Endianness::Little, NdrLimits::default())
            .unwrap();
        assert_eq!(
            NtlmRequest::decode_with_limits(&bytes, NdrLimits::default()).unwrap(),
            value
        );
    }
    for string in [
        UnicodeString {
            length: 1,
            maximum_length: 2,
            buffer: None,
        },
        UnicodeString {
            length: 4,
            maximum_length: 2,
            buffer: None,
        },
        UnicodeString {
            length: 2,
            maximum_length: 2,
            buffer: Some(Vec::new()),
        },
    ] {
        let value = NtlmRequest::Lm20GetNtlm3ChallengeResponse(Lm20GetNtlm3ChallengeResponseRequest {
            user_name: Some(string),
            ..Default::default()
        });
        assert!(
            value
                .encode_with_limits(Endianness::Little, NdrLimits::default())
                .is_err()
        );
    }
    let value = NtlmRequest::Lm20GetNtlm3ChallengeResponse(Lm20GetNtlm3ChallengeResponseRequest {
        user_name: Some(unicode()),
        ..Default::default()
    });
    let original = value
        .encode_with_limits(Endianness::Little, NdrLimits::default())
        .unwrap();
    for range in [52..56, 56..60, 60..64] {
        let mut bytes = original.clone();
        bytes[range].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(NtlmRequest::decode_with_limits(&bytes, NdrLimits::default()).is_err());
    }
}

#[test]
fn serialization_header_length_consumption_and_padding_extent() {
    let original = KerberosRequest::CreateDhKeyAgreement(Ignored { ignored: 3 })
        .encode_with_limits(Endianness::Little, NdrLimits::default())
        .unwrap();
    for position in [0, 1, 2, 8] {
        let mut bytes = original.clone();
        bytes[position] ^= 2;
        assert!(KerberosRequest::decode_with_limits(&bytes, NdrLimits::default()).is_err());
    }
    let mut bytes = original.clone();
    bytes.extend_from_slice(&[0; 8]);
    bytes[8..12].copy_from_slice(&16u32.to_le_bytes());
    assert_eq!(
        KerberosRequest::decode_with_limits(&bytes, NdrLimits::default())
            .unwrap_err()
            .reason,
        "trailing NDR object data"
    );
    let mut bytes = original;
    bytes[4..8].fill(0);
    bytes[12..16].fill(0xff);
    bytes[21..24].fill(0xcc);
    assert_eq!(
        KerberosRequest::decode_with_limits(&bytes, NdrLimits::default()).unwrap(),
        KerberosRequest::CreateDhKeyAgreement(Ignored { ignored: 3 })
    );
}

#[test]
fn encryption_key_void_referent_is_explicitly_unsupported() {
    let value = KerberosRequest::BuildAsReqAuthenticator(BuildAsReqAuthenticatorRequest {
        encryption_key: Some(key()),
        ..Default::default()
    });
    let mut bytes = value
        .encode_with_limits(Endianness::Little, NdrLimits::default())
        .unwrap();
    bytes[32..36].copy_from_slice(&1u32.to_le_bytes());
    let error = KerberosRequest::decode_with_limits(&bytes, NdrLimits::default()).unwrap_err();
    assert_eq!(error.reason, "unsupported non-null encryption-key void referent");
    assert_eq!(error.offset, 36);
}

#[test]
fn diagnostics_and_debug_redact_protocol_secrets() {
    let value = NtlmRequest::CalculateNtResponse(CalculateNtResponseRequest {
        nt_challenge: Some(Opaque([0xa5; 8])),
        credential: Some(secrets()),
    });
    let debug = format!("{value:?} {:?} {:?} {:?} {:?}", secrets(), key(), unicode(), octets());
    assert!(!debug.contains("165"));
    assert!(!debug.contains("61605"));
    assert!(!debug.contains("55357"));
    assert!(debug.contains("redacted"));
    let error = NtlmRequest::decode_with_limits(&[0xa5; 16], NdrLimits::default()).unwrap_err();
    assert!(!format!("{error:?} {error}").contains("a5"));
    let mut source = ReadCursor::new(&[0xa5; 16]);
    assert!(NtlmRequest::decode(&mut source).is_err());
    assert_eq!(source.pos(), 8);
}

#[test]
fn invalid_encoder_models_return_errors_before_output_allocation() {
    let value = KerberosResponse {
        status: 0,
        call: KerberosResponseCall::PackApReply(OctetString {
            length: u32::MAX,
            value: Some(vec![1]),
        }),
    };
    assert_eq!(value.size(), 0);
    assert!(encode_vec(&value).is_err());
    assert_eq!(
        value
            .encode_with_limits(Endianness::Little, NdrLimits::default())
            .unwrap_err()
            .reason,
        "NDR buffer correlation mismatch"
    );
    let value = KerberosRequest::CreateApReqAuthenticator(CreateApReqAuthenticatorRequest {
        client_name: Some(InternalName {
            name_type: -1,
            name_count: 2,
            names: Some(vec![unicode()]),
        }),
        ..Default::default()
    });
    assert_eq!(
        value
            .encode_with_limits(Endianness::Little, NdrLimits::default())
            .unwrap_err()
            .reason,
        "NDR name correlation mismatch"
    );
}

#[test]
fn conformant_structure_and_name_array_counts_are_checked() {
    let response = KerberosResponse {
        status: 0,
        call: KerberosResponseCall::DecryptPacCredentials(DecryptPacCredentialsResponse {
            credentials: Some(SupplementalCredentials {
                credentials: vec![SupplementalCredential::default()],
            }),
        }),
    };
    let original = response
        .encode_with_limits(Endianness::Little, NdrLimits::default())
        .unwrap();
    let mut bytes = original.clone();
    bytes[36..40].copy_from_slice(&2u32.to_le_bytes());
    assert_eq!(
        KerberosResponse::decode_with_limits(&bytes, NdrLimits::default())
            .unwrap_err()
            .reason,
        "NDR credential correlation mismatch"
    );
    let mut bytes = original;
    bytes[32..40].fill(0xff);
    assert_eq!(
        KerberosResponse::decode_with_limits(&bytes, NdrLimits::default())
            .unwrap_err()
            .reason,
        "NDR count exceeds remaining object"
    );
    let request = KerberosRequest::CreateApReqAuthenticator(CreateApReqAuthenticatorRequest {
        client_name: Some(InternalName {
            name_type: 1,
            name_count: 1,
            names: Some(vec![unicode()]),
        }),
        ..Default::default()
    });
    let mut bytes = request
        .encode_with_limits(Endianness::Little, NdrLimits::default())
        .unwrap();
    bytes[64..68].copy_from_slice(&2u32.to_le_bytes());
    assert!(KerberosRequest::decode_with_limits(&bytes, NdrLimits::default()).is_err());
}

#[test]
fn referenced_length_missing_or_mismatched_is_rejected() {
    let value = KerberosResponse {
        status: 0,
        call: KerberosResponseCall::HashS4UPreauth(output()),
    };
    let original = value
        .encode_with_limits(Endianness::Little, NdrLimits::default())
        .unwrap();
    let mut bytes = original.clone();
    bytes[44..48].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        KerberosResponse::decode_with_limits(&bytes, NdrLimits::default())
            .unwrap_err()
            .reason,
        "NDR buffer correlation mismatch"
    );
    let mut bytes = original;
    bytes[28..32].fill(0);
    assert_eq!(
        KerberosResponse::decode_with_limits(&bytes, NdrLimits::default())
            .unwrap_err()
            .reason,
        "missing NDR referenced length"
    );
}

#[test]
fn credential_enum16_rejects_undefined_discriminants() {
    let value = NtlmRequest::CalculateNtResponse(CalculateNtResponseRequest {
        credential: Some(secrets()),
        ..Default::default()
    });
    let mut bytes = value
        .encode_with_limits(Endianness::Little, NdrLimits::default())
        .unwrap();
    bytes[32..34].copy_from_slice(&0xffffu16.to_le_bytes());
    assert_eq!(
        NtlmRequest::decode_with_limits(&bytes, NdrLimits::default())
            .unwrap_err()
            .reason,
        "invalid NDR credential-key discriminant"
    );
}

#[test]
fn der_package_and_ndr_serialization_headers_are_separate() {
    let call = KerberosRequest::NegotiateVersion(NegotiateVersion { version: 1 });
    let serialized = call
        .encode_with_limits(Endianness::Little, NdrLimits::default())
        .unwrap();
    let packet = ironrdp_rdpear::RemoteGuardInnerPacket {
        package_name: b"Kerberos",
        package_call: &serialized,
    };
    let der = encode_vec(&packet).unwrap();
    let decoded = ironrdp_rdpear::RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&der)).unwrap();
    assert_eq!(decoded.package_call, serialized);
    assert_eq!(
        KerberosRequest::decode_with_limits(decoded.package_call, NdrLimits::default()).unwrap(),
        call
    );
    let header_start = der
        .windows(16)
        .position(|bytes| bytes == [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
        .unwrap();
    assert_eq!(
        der[header_start + 16..header_start + 24],
        [1, 0x10, 8, 0, 0xcc, 0xcc, 0xcc, 0xcc]
    );
}

#[test]
fn unicode_capacity_does_not_allocate_unused_code_units() {
    let value = NtlmRequest::Lm20GetNtlm3ChallengeResponse(Lm20GetNtlm3ChallengeResponseRequest {
        user_name: Some(UnicodeString {
            length: 2,
            maximum_length: u16::MAX,
            buffer: Some(vec![0xdfff]),
        }),
        ..Default::default()
    });
    let bytes = value.encode_with_limits(Endianness::Big, NdrLimits::default()).unwrap();
    let limits = NdrLimits {
        max_allocation: 2,
        max_elements: 1,
        ..NdrLimits::default()
    };
    assert_eq!(NtlmRequest::decode_with_limits(&bytes, limits).unwrap(), value);
    assert!(
        NtlmRequest::decode_with_limits(
            &bytes,
            NdrLimits {
                max_allocation: 1,
                ..limits
            }
        )
        .is_err()
    );
}

#[test]
fn raw_body_root_alignment_both_byte_orders() {
    macro_rules! check {
        ($ty:ident, $value:expr, $little:literal, $big:literal) => {
            let value: $ty = $value;
            for (endian, fixture) in [(Endianness::Little, $little), (Endianness::Big, $big)] {
                let expected = hex(fixture);
                assert_eq!(
                    value
                        .encode_body_with_limits(endian, NdrLimits::default())
                        .unwrap(),
                    expected
                );
                let mut carrier = vec![0xcc; 3];
                carrier.extend_from_slice(&expected);
                assert_eq!(
                    $ty::decode_body_with_limits(&carrier[3..], endian, NdrLimits::default()).unwrap(),
                    value
                );
            }
        };
    }
    check!(
        KerberosRequest,
        KerberosRequest::DestroyKeyAgreement(KeyAgreementRequest {
            key_agreement_handle: -0x0123_4567_89ab_cdf0,
        }),
        "16011601000000001032547698badcfe",
        "0116011600000000fedcba9876543210"
    );
    check!(
        KerberosResponse,
        KerberosResponse {
            status: -123,
            call: KerberosResponseCall::NegotiateVersion(NegotiateVersion { version: 0x1234_5678 }),
        },
        "0001000085ffffff0001000078563412",
        "01000000ffffff850100000012345678"
    );
    check!(
        NtlmRequest,
        NtlmRequest::NegotiateVersion(NegotiateVersion { version: 0x1234_5678 }),
        "0002000278563412",
        "0200020012345678"
    );
    check!(
        NtlmResponse,
        NtlmResponse {
            status: -123,
            call: NtlmResponseCall::CalculateNtResponse(Opaque([0x5a; 24])),
        },
        "0202000085ffffff02025a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a",
        "02020000ffffff8502025a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a"
    );
}

#[test]
fn raw_body_wire_allocation_and_element_limits_all_roots() {
    macro_rules! check {
        ($ty:ident, $value:expr, $elements:literal) => {
            let value: $ty = $value;
            for endian in [Endianness::Little, Endianness::Big] {
                let body = value
                    .encode_body_with_limits(endian, NdrLimits::default())
                    .unwrap();
                let decode_limits = NdrLimits {
                    max_message_size: body.len(),
                    max_allocation: $elements,
                    max_elements: $elements,
                };
                assert_eq!(
                    $ty::decode_body_with_limits(&body, endian, decode_limits).unwrap(),
                    value
                );
                for (limits, reason) in [
                    (
                        NdrLimits {
                            max_message_size: body.len() - 1,
                            ..decode_limits
                        },
                        "NDR message exceeds limit",
                    ),
                    (
                        NdrLimits {
                            max_allocation: $elements - 1,
                            ..decode_limits
                        },
                        "NDR decoding exceeds limit",
                    ),
                    (
                        NdrLimits {
                            max_elements: $elements - 1,
                            ..decode_limits
                        },
                        "NDR decoding exceeds limit",
                    ),
                ] {
                    assert_eq!(
                        $ty::decode_body_with_limits(&body, endian, limits)
                            .unwrap_err()
                            .reason,
                        reason
                    );
                }
                let encode_limits = NdrLimits {
                    max_allocation: body.len(),
                    ..decode_limits
                };
                assert_eq!(
                    value.encode_body_with_limits(endian, encode_limits).unwrap(),
                    body
                );
                for (limits, reason) in [
                    (
                        NdrLimits {
                            max_message_size: body.len() - 1,
                            ..encode_limits
                        },
                        "NDR encoding exceeds limit",
                    ),
                    (
                        NdrLimits {
                            max_allocation: body.len() - 1,
                            ..encode_limits
                        },
                        "NDR encoding exceeds limit",
                    ),
                    (
                        NdrLimits {
                            max_elements: $elements - 1,
                            ..encode_limits
                        },
                        "NDR encoding exceeds element limit",
                    ),
                ] {
                    assert_eq!(
                        value.encode_body_with_limits(endian, limits).unwrap_err().reason,
                        reason
                    );
                }
            }
        };
    }
    check!(
        KerberosRequest,
        KerberosRequest::VerifyChecksum(VerifyChecksumRequest {
            key: None,
            checksum_type: 1,
            expected_checksum: octets(),
            data_to_check: octets(),
        }),
        6
    );
    check!(
        KerberosResponse,
        KerberosResponse {
            status: 0,
            call: KerberosResponseCall::PackApReply(octets()),
        },
        3
    );
    check!(
        NtlmRequest,
        NtlmRequest::CalculateNtResponse(CalculateNtResponseRequest {
            credential: Some(secrets()),
            nt_challenge: None,
        }),
        3
    );
    check!(
        NtlmResponse,
        NtlmResponse {
            status: 0,
            call: NtlmResponseCall::Lm20GetNtlm3ChallengeResponse(Lm20GetNtlm3ChallengeResponseResponse {
                ntlm3_response: NtlmResponseBuffer {
                    length: 3,
                    value: Some(vec![1, 2, 3]),
                },
                ..Default::default()
            }),
        },
        3
    );
}

#[test]
fn raw_body_scalars_require_no_referent_budget() {
    macro_rules! check {
        ($ty:ident, $value:expr) => {
            let value: $ty = $value;
            for endian in [Endianness::Little, Endianness::Big] {
                let body = value
                    .encode_body_with_limits(endian, NdrLimits::default())
                    .unwrap();
                assert_eq!(
                    $ty::decode_body_with_limits(
                        &body,
                        endian,
                        NdrLimits {
                            max_message_size: body.len(),
                            max_allocation: 0,
                            max_elements: 0,
                        },
                    )
                    .unwrap(),
                    value
                );
                assert_eq!(
                    value
                        .encode_body_with_limits(
                            endian,
                            NdrLimits {
                                max_message_size: body.len(),
                                max_allocation: body.len(),
                                max_elements: 0,
                            },
                        )
                        .unwrap(),
                    body
                );
            }
        };
    }
    check!(
        KerberosRequest,
        KerberosRequest::NegotiateVersion(NegotiateVersion { version: 1 })
    );
    check!(
        KerberosResponse,
        KerberosResponse {
            status: 0,
            call: KerberosResponseCall::NegotiateVersion(NegotiateVersion { version: 1 }),
        }
    );
    check!(
        NtlmRequest,
        NtlmRequest::NegotiateVersion(NegotiateVersion { version: 1 })
    );
    check!(
        NtlmResponse,
        NtlmResponse {
            status: 0,
            call: NtlmResponseCall::NegotiateVersion(NegotiateVersion { version: 1 }),
        }
    );
}

#[test]
fn raw_body_unknown_key_referent_reports_body_offset() {
    let value = KerberosRequest::BuildAsReqAuthenticator(BuildAsReqAuthenticatorRequest {
        encryption_key: Some(key()),
        ..Default::default()
    });
    for endian in [Endianness::Little, Endianness::Big] {
        let mut body = value.encode_body_with_limits(endian, NdrLimits::default()).unwrap();
        body[16..20].copy_from_slice(&match endian {
            Endianness::Little => 1u32.to_le_bytes(),
            Endianness::Big => 1u32.to_be_bytes(),
        });
        let error = KerberosRequest::decode_body_with_limits(&body, endian, NdrLimits::default()).unwrap_err();
        assert_eq!(error.reason, "unsupported non-null encryption-key void referent");
        assert_eq!(error.offset, 20);
    }
}

#[test]
fn der_package_call_explicitly_composes_raw_or_v1() {
    let call = KerberosResponse {
        status: -123,
        call: KerberosResponseCall::HashS4UPreauth(output()),
    };
    for endian in [Endianness::Little, Endianness::Big] {
        let body = call.encode_body_with_limits(endian, NdrLimits::default()).unwrap();
        let serialized = call.encode_with_limits(endian, NdrLimits::default()).unwrap();
        for package_call in [&body, &serialized] {
            let der = encode_vec(&ironrdp_rdpear::RemoteGuardInnerPacket {
                package_name: b"Kerberos",
                package_call,
            })
            .unwrap();
            let packet = ironrdp_rdpear::RemoteGuardInnerPacket::decode(&mut ReadCursor::new(&der)).unwrap();
            assert_eq!(packet.package_call, package_call);
            if package_call == &body {
                assert_eq!(
                    KerberosResponse::decode_body_with_limits(packet.package_call, endian, NdrLimits::default())
                        .unwrap(),
                    call
                );
                assert!(KerberosResponse::decode_with_limits(packet.package_call, NdrLimits::default()).is_err());
            } else {
                assert_eq!(
                    KerberosResponse::decode_with_limits(packet.package_call, NdrLimits::default()).unwrap(),
                    call
                );
                assert!(
                    KerberosResponse::decode_body_with_limits(packet.package_call, endian, NdrLimits::default())
                        .is_err()
                );
            }
        }
    }
}
