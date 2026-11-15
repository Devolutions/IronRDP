use std::borrow::Cow;
use std::sync::Arc;

use ironrdp_connector::{
    ClientConnector, ClientConnectorState, Config, ConnectorResult, LicenseCache, LicenseExchangeState, LicenseRng,
    Sequence as _,
};
use ironrdp_core::{Decode as _, ReadCursor, WriteBuf, decode, encode_vec};
use ironrdp_pdu::mcs::{McsMessage, SendDataIndication};
use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
use ironrdp_pdu::rdp::server_license::cert::{CertificateType, ProprietaryCertificate, RsaPublicKey};
use ironrdp_pdu::rdp::server_license::{
    ClientNewLicenseRequest, LicenseEncryptionData, LicenseHeader, LicenseInformation, LicensePdu,
    PREMASTER_SECRET_SIZE, PreambleFlags, PreambleType, PreambleVersion, ProductInfo, RANDOM_NUMBER_SIZE, Scope,
    ServerCertificate, ServerLicenseRequest,
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

/// A license store that holds a matching license for every lookup.
#[derive(Debug)]
struct StoredLicense(Vec<u8>);

impl LicenseCache for StoredLicense {
    fn get_license(&self, _license_info: LicenseInformation) -> ConnectorResult<Option<Vec<u8>>> {
        Ok(Some(self.0.clone()))
    }

    fn store_license(&self, _license_info: LicenseInformation) -> ConnectorResult<()> {
        Ok(())
    }
}

/// Enters the licensing exchange and answers a Server License Request.
fn answer_license_request(mut connector: ClientConnector) -> (Vec<u8>, LicenseEncryptionData) {
    // A Server License Request on the I/O channel, carrying a proprietary certificate with a 512-bit key.
    let server_license_request = {
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
            // The connector looks a license up once per scope, so a stored license needs one to be found.
            scope_list: vec![Scope("microsoft.com".to_owned())],
        });

        encode_vec(&X224(McsMessage::SendDataIndication(SendDataIndication {
            initiator_id: USER_CHANNEL_ID,
            channel_id: IO_CHANNEL_ID,
            user_data: Cow::Owned(encode_vec(&license).unwrap()),
        })))
        .unwrap()
    };

    connector.state = ClientConnectorState::ConnectTimeAutoDetection {
        io_channel_id: IO_CHANNEL_ID,
        user_channel_id: USER_CHANNEL_ID,
    };
    let mut output = WriteBuf::new();
    connector.step(&[], None, &mut output).unwrap();
    connector.step(&server_license_request, None, &mut output).unwrap();

    let X224(McsMessage::SendDataRequest(send_data)) = decode(output.filled()).unwrap() else {
        panic!("expected a SendDataRequest");
    };
    let response = send_data.user_data.into_owned();
    let ClientConnectorState::LicensingExchange { license_exchange, .. } = connector.state else {
        panic!("expected the licensing exchange to continue");
    };
    let LicenseExchangeState::PlatformChallenge { encryption_data } = license_exchange.state else {
        panic!("expected to wait for the platform challenge");
    };

    (response, encryption_data)
}

fn answer_with_new_license_request(connector: ClientConnector) -> (ClientNewLicenseRequest, LicenseEncryptionData) {
    let (response, encryption_data) = answer_license_request(connector);
    let LicensePdu::ClientNewLicenseRequest(request) = decode(&response).unwrap() else {
        panic!("expected a Client New License Request");
    };

    (request, encryption_data)
}

/// Reads a Licensing Binary Blob, MS-RDPBCGR 2.2.1.12.1.2, and returns its data.
fn read_blob<'a>(cursor: &mut ReadCursor<'a>) -> &'a [u8] {
    let _blob_type = cursor.read_u16();
    let blob_length = usize::from(cursor.read_u16());

    cursor.read_slice(blob_length)
}

fn connector() -> ClientConnector {
    ClientConnector::new(super::test_config(), "127.0.0.1:12345".parse().unwrap())
}

// oracle: mutant injected source ignored or dropped
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
        let (request, encryption_data) = answer_with_new_license_request(connector().with_license_rng(Arc::new(rng)));

        prop_assert_eq!(request.client_random, client_random);
        prop_assert_eq!(encryption_data.premaster_secret, premaster_secret);
    }
}

// oracle: spec MS-RDPELE §3.3.5.3 (the message type)
// oracle: spec MS-RDPELE §2.2.2.3 (the license)
// oracle: mutant injected source ignored or dropped (the client random and the premaster secret)
proptest! {
    #[test]
    fn a_stored_license_is_sent_in_client_license_info(
        license in prop::collection::vec(any::<u8>(), 1..64),
        client_random in prop::array::uniform32(any::<u8>()),
        premaster_secret in prop::collection::vec(any::<u8>(), PREMASTER_SECRET_SIZE),
    ) {
        let config = Config {
            license_cache: Some(Arc::new(StoredLicense(license.clone()))),
            ..super::test_config()
        };
        let rng = FixedRng {
            client_random,
            premaster_secret: premaster_secret.clone(),
        };
        let connector = ClientConnector::new(config, "127.0.0.1:12345".parse().unwrap())
            .with_license_rng(Arc::new(rng));

        let (response, encryption_data) = answer_license_request(connector);

        // The message is read by hand against MS-RDPELE 2.2.2.3, because `LicensePdu` doesn't decode it.
        let mut cursor = ReadCursor::new(&response);
        let header = LicenseHeader::decode(&mut cursor).unwrap();
        prop_assert_eq!(header.preamble_message_type, PreambleType::LicenseInfo);

        let _preferred_key_exchange_alg = cursor.read_u32();
        let _platform_id = cursor.read_u32();
        let sent_client_random = cursor.read_slice(RANDOM_NUMBER_SIZE);
        let _encrypted_premaster_secret = read_blob(&mut cursor);
        let sent_license = read_blob(&mut cursor);

        prop_assert_eq!(sent_license, license.as_slice());
        prop_assert_eq!(sent_client_random, client_random.as_slice());
        prop_assert_eq!(encryption_data.premaster_secret, premaster_secret);
    }
}

/// The client generates both values on receipt of each Server License Request, so two connections don't repeat them.
/// This can't tell a secure generator from a weak one.
// oracle: spec MS-RDPELE §5.1.1
#[test]
fn each_connection_gets_new_values_from_the_default_rng() {
    let (first_request, first_encryption_data) = answer_with_new_license_request(connector());
    let (second_request, second_encryption_data) = answer_with_new_license_request(connector());

    assert_ne!(first_request.client_random, second_request.client_random);
    assert_ne!(
        first_encryption_data.premaster_secret,
        second_encryption_data.premaster_secret
    );
}
