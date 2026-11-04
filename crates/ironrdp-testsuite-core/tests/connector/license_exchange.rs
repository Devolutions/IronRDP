use std::borrow::Cow;
use std::sync::Arc;

use ironrdp_connector::{ClientConnector, ClientConnectorState, LicenseExchangeState, LicenseRng, Sequence as _};
use ironrdp_core::{WriteBuf, decode, encode_vec};
use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
use ironrdp_pdu::rdp::server_license::cert::{CertificateType, ProprietaryCertificate, RsaPublicKey};
use ironrdp_pdu::rdp::server_license::{
    ClientNewLicenseRequest, LicenseEncryptionData, LicenseHeader, LicensePdu, PREMASTER_SECRET_SIZE, PreambleFlags,
    PreambleType, PreambleVersion, ProductInfo, RANDOM_NUMBER_SIZE, ServerCertificate, ServerLicenseRequest,
};
use ironrdp_pdu::x224::X224;
use proptest::prelude::*;

const USER_CHANNEL_ID: u16 = 1002;
const IO_CHANNEL_ID: u16 = 1003;

#[derive(Debug)]
struct FixedRng {
    client_random: [u8; RANDOM_NUMBER_SIZE],
    premaster_secret: Vec<u8>,
}

impl LicenseRng for FixedRng {
    fn fill_secrets(
        &self,
        client_random: &mut [u8; RANDOM_NUMBER_SIZE],
        premaster_secret: &mut [u8; PREMASTER_SECRET_SIZE],
    ) {
        *client_random = self.client_random;
        premaster_secret.copy_from_slice(&self.premaster_secret);
    }
}

/// A Server License Request on the I/O channel, carrying a proprietary certificate with a 512-bit key.
fn server_license_request() -> Vec<u8> {
    let license = LicensePdu::ServerLicenseRequest(ServerLicenseRequest {
        license_header: LicenseHeader {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::LICENSE_PKT,
            },
            preamble_message_type: PreambleType::LicenseRequest,
            preamble_flags: PreambleFlags::empty(),
            preamble_version: PreambleVersion::V3,
            preamble_message_size: 0,
        },
        server_random: vec![0x44; RANDOM_NUMBER_SIZE],
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
                    modulus: [vec![0xff; 64], vec![0; 8]].concat(),
                },
                signature: vec![0; 72],
            }),
        }),
        scope_list: Vec::new(),
    });

    encode_vec(&X224(McsMessage::SendDataIndication(SendDataIndication {
        initiator_id: USER_CHANNEL_ID,
        channel_id: IO_CHANNEL_ID,
        user_data: Cow::Owned(encode_vec(&license).unwrap()),
    })))
    .unwrap()
}

/// Enters the licensing exchange and answers a Server License Request.
fn answer_license_request(mut connector: ClientConnector) -> (ClientNewLicenseRequest, LicenseEncryptionData) {
    connector.state = ClientConnectorState::ConnectTimeAutoDetection {
        io_channel_id: IO_CHANNEL_ID,
        user_channel_id: USER_CHANNEL_ID,
    };
    let mut output = WriteBuf::new();
    connector.step(&[], None, &mut output).unwrap();
    connector.step(&server_license_request(), None, &mut output).unwrap();

    let X224(McsMessage::SendDataRequest(send_data)) = decode(output.filled()).unwrap() else {
        panic!("expected a SendDataRequest");
    };
    let LicensePdu::ClientNewLicenseRequest(request) = decode(&send_data.user_data).unwrap() else {
        panic!("expected a Client New License Request");
    };
    let ClientConnectorState::LicensingExchange { license_exchange, .. } = connector.state else {
        panic!("expected the licensing exchange to continue");
    };
    let LicenseExchangeState::PlatformChallenge { encryption_data } = license_exchange.state else {
        panic!("expected to wait for the platform challenge");
    };

    (request, encryption_data)
}

fn connector() -> ClientConnector {
    ClientConnector::new(super::test_config(), "127.0.0.1:12345".parse().unwrap())
}

// oracle: spec MS-RDPELE §2.2.2.2 (the request carries the client's random number and premaster secret)
proptest! {
    #[test]
    fn the_license_request_uses_the_injected_rng(
        client_random in prop::array::uniform32(any::<u8>()),
        premaster_secret in prop::collection::vec(any::<u8>(), PREMASTER_SECRET_SIZE),
    ) {
        let rng = FixedRng {
            client_random,
            premaster_secret: premaster_secret.clone(),
        };
        let (request, encryption_data) = answer_license_request(connector().with_license_rng(Arc::new(rng)));

        prop_assert_eq!(request.client_random, client_random);
        prop_assert_eq!(encryption_data.premaster_secret, premaster_secret);
    }
}

/// The client generates both values with a cryptographically secure generator, so two connections never repeat them.
// oracle: spec MS-RDPELE §2.2.2.2
#[test]
fn each_connection_gets_new_values_from_the_default_rng() {
    let (first_request, first_encryption_data) = answer_license_request(connector());
    let (second_request, second_encryption_data) = answer_license_request(connector());

    assert_ne!(first_request.client_random, second_request.client_random);
    assert_ne!(
        first_encryption_data.premaster_secret,
        second_encryption_data.premaster_secret
    );
}
