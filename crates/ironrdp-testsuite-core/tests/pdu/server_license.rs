use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
use ironrdp_pdu::rdp::server_license::cert::{CertificateType, ProprietaryCertificate, RsaPublicKey};
use ironrdp_pdu::rdp::server_license::{
    ClientLicenseInfo, ClientNewLicenseRequest, LicenseHeader, PreambleFlags, PreambleType, PreambleVersion,
    ProductInfo, ServerCertificate, ServerLicenseRequest,
};
use rstest::rstest;

/// The zero padding that follows the modulus bytes of a proprietary certificate's public key.
const MODULUS_PADDING: [u8; 8] = [0; 8];

/// The 512-bit modulus of the proprietary certificate in MS-RDPBCGR 4.1.4, in little-endian order without its padding.
const SPEC_EXAMPLE_MODULUS: [u8; 64] = [
    0xcb, 0x81, 0xfe, 0xba, 0x6d, 0x61, 0xc3, 0x55, 0x05, 0xd5, 0x5f, 0x2e, 0x87, 0xf8, 0x71, 0x94, 0xd6, 0xf1, 0xa5,
    0xcb, 0xf1, 0x5f, 0x0c, 0x3d, 0xf8, 0x70, 0x02, 0x96, 0xc4, 0xfb, 0x9b, 0xc8, 0x3c, 0x2d, 0x55, 0xae, 0xe8, 0xff,
    0x32, 0x75, 0xea, 0x68, 0x79, 0xe5, 0xa2, 0x01, 0xfd, 0x31, 0xa0, 0xb1, 0x1f, 0x55, 0xa6, 0x1f, 0xc1, 0xf6, 0xd1,
    0x83, 0x88, 0x63, 0x26, 0x56, 0x12, 0xbc,
];

#[derive(Debug, Clone, Copy)]
enum LicenseResponse {
    NewLicenseRequest,
    LicenseInfo,
}

impl LicenseResponse {
    /// Builds the response, which encrypts the premaster secret with the server's public key.
    fn build(self, license_request: &ServerLicenseRequest) -> bool {
        let client_random = [0x11; 32];
        let premaster_secret = [0x22; 48];
        match self {
            Self::NewLicenseRequest => ClientNewLicenseRequest::from_server_license_request(
                license_request,
                &client_random,
                &premaster_secret,
                "user",
                "machine",
            )
            .is_ok(),
            Self::LicenseInfo => ClientLicenseInfo::from_server_license_request(
                license_request,
                &client_random,
                &premaster_secret,
                [0; 4],
                vec![0x33; 16],
            )
            .is_ok(),
        }
    }
}

#[rstest]
// oracle: regression crates/ironrdp-testsuite-core/test_data/fuzz_regression/pdu_decode/crash-3639648ab6730a3d8617bc39badcc741c56bdc4f
#[case::zero_modulus(vec![0; 64], false)]
// oracle: mutant a guard that rejects only a zero modulus, accepting a 256-bit key
#[case::modulus_below_512_bits(vec![0xff; 32], false)]
// oracle: spec MS-RDPBCGR §4.1.4
#[case::modulus_of_512_bits(SPEC_EXAMPLE_MODULUS.to_vec(), true)]
fn licensing_accepts_only_moduli_of_at_least_512_bits(
    #[case] modulus: Vec<u8>,
    #[case] accepted: bool,
    #[values(LicenseResponse::NewLicenseRequest, LicenseResponse::LicenseInfo)] response: LicenseResponse,
) {
    // The certificate carries `modulus` in little-endian order, followed by its padding.
    let license_request = ServerLicenseRequest {
        license_header: LicenseHeader {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::LICENSE_PKT,
            },
            preamble_message_type: PreambleType::LicenseRequest,
            preamble_flags: PreambleFlags::empty(),
            preamble_version: PreambleVersion::V3,
            preamble_message_size: 0,
        },
        server_random: vec![0x44; 32],
        product_info: ProductInfo {
            version: 0x60000,
            company_name: "Microsoft Corporation".to_owned(),
            product_id: "A02".to_owned(),
        },
        server_certificate: Some(ServerCertificate {
            issued_permanently: true,
            certificate: CertificateType::Proprietary(ProprietaryCertificate {
                public_key: RsaPublicKey {
                    public_exponent: 0x0001_0001,
                    modulus: [modulus.as_slice(), &MODULUS_PADDING].concat(),
                },
                signature: vec![0; 72],
            }),
        }),
        scope_list: Vec::new(),
    };

    assert_eq!(response.build(&license_request), accepted);
}
