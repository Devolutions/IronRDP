use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
use ironrdp_pdu::rdp::server_license::cert::{CertificateType, ProprietaryCertificate, RsaPublicKey};
use ironrdp_pdu::rdp::server_license::{
    ClientLicenseInfo, ClientNewLicenseRequest, LicenseHeader, PreambleFlags, PreambleType, PreambleVersion,
    ProductInfo, ServerCertificate, ServerLicenseRequest,
};
use rstest::rstest;

/// The zero padding that follows the modulus bytes of a proprietary certificate's public key.
const MODULUS_PADDING: [u8; 8] = [0; 8];

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

// oracle: reference crates/ironrdp-pdu/src/rdp/server_license/server_license_request/tests.rs
#[rstest]
#[case::zero_modulus(vec![0; 64], false)]
#[case::modulus_below_512_bits(vec![0xff; 32], false)]
#[case::modulus_of_512_bits(vec![0xff; 64], true)]
fn licensing_accepts_only_moduli_of_at_least_512_bits(
    #[case] modulus: Vec<u8>,
    #[case] accepted: bool,
    #[values(LicenseResponse::NewLicenseRequest, LicenseResponse::LicenseInfo)] response: LicenseResponse,
) {
    let license_request = license_request_with_modulus(modulus);

    assert_eq!(response.build(&license_request), accepted);
}

/// A license request whose proprietary certificate carries `modulus`, in little-endian order without its padding.
fn license_request_with_modulus(modulus: Vec<u8>) -> ServerLicenseRequest {
    ServerLicenseRequest {
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
    }
}
