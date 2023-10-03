use lazy_static::lazy_static;

use super::*;
use crate::rdp::server_license::server_license_request::cert::{CertificateType, X509CertificateChain};
use crate::rdp::server_license::server_license_request::{ProductInfo, Scope, ServerCertificate};
use crate::rdp::server_license::PREAMBLE_SIZE;

const LICENSE_HEADER_BUFFER_NO_SIZE: [u8; 6] = [
    0x80, 0x00, // flags
    0x00, 0x00, // flagsHi
    0x13, 0x03,
];

const CLIENT_RANDOM_BUFFER: [u8; 32] = [
    0x4b, 0x5b, 0x7b, 0x43, 0x63, 0x8a, 0x8, 0xfe, 0xd1, 0x7a, 0xba, 0xf5, 0x91, 0x85, 0x77, 0xfe, 0x39, 0x36, 0xf6,
    0xd7, 0x78, 0xec, 0x6a, 0xcc, 0x89, 0x4a, 0x90, 0x41, 0x2c, 0xac, 0x5a, 0x49,
];

const SERVER_RANDOM_BUFFER: [u8; 32] = [
    0x5c, 0x81, 0xf0, 0x11, 0xeb, 0xcf, 0xd1, 0xe, 0xb4, 0x1f, 0xb3, 0xba, 0x93, 0xa2, 0xd7, 0x39, 0x9, 0xaa, 0x99,
    0xe9, 0x10, 0xd4, 0xd7, 0x95, 0xdd, 0xad, 0x91, 0x69, 0x5, 0x26, 0x6b, 0x6a,
];

const ENCRYPTED_PREMASTER_SECRET: [u8; 72] = [
    0xb0, 0x95, 0xf7, 0xcb, 0x81, 0x34, 0x45, 0x85, 0x65, 0x83, 0xb2, 0xcf, 0x5b, 0xdf, 0xfe, 0x40, 0x6f, 0xd8, 0x14,
    0x14, 0x18, 0xb, 0x2b, 0x6d, 0xaa, 0xd0, 0x38, 0xa, 0xaf, 0x74, 0xe9, 0x51, 0x55, 0x2b, 0xdb, 0xfd, 0x3b, 0xd3,
    0xfb, 0x2f, 0x34, 0x92, 0xdd, 0xc8, 0xaf, 0x48, 0xf3, 0x91, 0x61, 0x8a, 0x5b, 0xbd, 0x81, 0x87, 0xec, 0xb8, 0xcc,
    0xb, 0xb, 0xc9, 0xd, 0x1c, 0xe7, 0x17, 0, 0, 0, 0, 0, 0, 0, 0,
];

const PREMASTER_SECRET_BUFFER: [u8; 48] = [
    0x14, 0x28, 0xda, 0xfb, 0xb9, 0xea, 0x38, 0xab, 0x5e, 0xa2, 0xf9, 0x4, 0xf7, 0x89, 0x9c, 0x98, 0x3d, 0x50, 0x45,
    0x77, 0xbf, 0x17, 0x81, 0x1c, 0x37, 0x87, 0xc2, 0x48, 0x13, 0xe8, 0xc9, 0x20, 0x4d, 0xdf, 0xf3, 0x27, 0xbd, 0xb6,
    0x98, 0x7e, 0x64, 0xda, 0xfe, 0x1d, 0x31, 0x2f, 0x62, 0xca,
];

const SALTED_HASH_BUFFER: [u8; 16] = [
    0xfe, 0xdc, 0x51, 0x9a, 0xdb, 0x3a, 0xc9, 0x61, 0x4, 0x7, 0x24, 0x94, 0x5d, 0xc, 0x43, 0xa7,
];

const MASTER_SECRET_BUFFER: [u8; 48] = [
    0xfe, 0xdc, 0x51, 0x9a, 0xdb, 0x3a, 0xc9, 0x61, 0x4, 0x7, 0x24, 0x94, 0x5d, 0xc, 0x43, 0xa7, 0x70, 0xe3, 0xf3, 0x0,
    0x50, 0xd7, 0xa8, 0x72, 0x3e, 0xab, 0x7e, 0x1b, 0xe4, 0x64, 0xe5, 0xc5, 0x74, 0xae, 0xed, 0x10, 0x72, 0x96, 0x2a,
    0x4c, 0x65, 0x9, 0x4f, 0x60, 0x12, 0xa9, 0x12, 0xa1,
];

const SESSION_KEY_BLOB: [u8; 48] = [
    0xf7, 0x3, 0x75, 0xb9, 0x5f, 0xda, 0xd0, 0xbe, 0xb4, 0x2a, 0xf5, 0xc1, 0x3d, 0x98, 0x85, 0x7a, 0xd6, 0xc5, 0x39,
    0x4c, 0xe3, 0xcb, 0x76, 0x61, 0xaa, 0x4a, 0xb6, 0x15, 0x7e, 0x89, 0x21, 0x3d, 0xdf, 0x5b, 0x25, 0x32, 0xee, 0x5,
    0x6, 0xd, 0x5b, 0xaa, 0x63, 0x14, 0xaf, 0xa5, 0x46, 0xf,
];

const LICENSE_KEY_BUFFER: [u8; 16] = [
    0xfa, 0x44, 0xe8, 0x78, 0xd8, 0x2b, 0x3f, 0x1d, 0x4d, 0x0, 0xa0, 0xa6, 0x55, 0xce, 0x8a, 0xb7,
];

const CLIENT_USERNAME: &str = "sample-user";
const CLIENT_MACHINE_NAME: &str = "sample-machine-name";

lazy_static! {
    pub static ref CLIENT_NEW_LICENSE_REQUEST: ClientNewLicenseRequest = ClientNewLicenseRequest {
        license_header: LicenseHeader {
            security_header: BasicSecurityHeader {
                flags: BasicSecurityHeaderFlags::LICENSE_PKT,
            },
            preamble_message_type: PreambleType::NewLicenseRequest,
            preamble_flags: PreambleFlags::empty(),
            preamble_version: PreambleVersion::V3,
            preamble_message_size: (PREAMBLE_SIZE
                + RANDOM_NUMBER_SIZE
                + LICENSE_REQUEST_STATIC_FIELDS_SIZE
                + ENCRYPTED_PREMASTER_SECRET.len()
                + CLIENT_MACHINE_NAME.len()
                + UTF8_NULL_TERMINATOR_SIZE
                + CLIENT_USERNAME.len()
                + UTF8_NULL_TERMINATOR_SIZE) as u16,
        },
        client_random: Vec::from(CLIENT_RANDOM_BUFFER.as_ref()),
        encrypted_premaster_secret: Vec::from(ENCRYPTED_PREMASTER_SECRET.as_ref()),
        client_username: CLIENT_USERNAME.to_string(),
        client_machine_name: CLIENT_MACHINE_NAME.to_string(),
    };

    pub static ref REQUEST_BUFFER: Vec<u8> = {
        let username_len = CLIENT_USERNAME.len() + UTF8_NULL_TERMINATOR_SIZE;
        let mut username_len_buf = Vec::new();
        username_len_buf.write_u16::<LittleEndian>(username_len as u16).unwrap();

        let machine_name_len = CLIENT_MACHINE_NAME.len() + UTF8_NULL_TERMINATOR_SIZE;
        let mut machine_name_len_buf = Vec::new();
        machine_name_len_buf.write_u16::<LittleEndian>(machine_name_len as u16).unwrap();

        let buf = [
            &[0x01u8, 0x00, 0x00, 0x00, // preferred_key_exchange_algorithm
            0x00, 0x00, 0x01, 0x04], // platform_id
            CLIENT_RANDOM_BUFFER.as_ref(),
            &[0x02, 0x00, // blob type
            0x48, 0x00], // blob len
            ENCRYPTED_PREMASTER_SECRET.as_ref(),
            &[0x0f, 0x00], // blob type
            username_len_buf.as_slice(),
            CLIENT_USERNAME.as_bytes(),
            &[0x00, // null
            0x10, 0x00], // blob type
            machine_name_len_buf.as_slice(), // blob len
            CLIENT_MACHINE_NAME.as_bytes(),
            &[0x00]] // null
                .concat();

        let preamble_size_field = (buf.len() + PREAMBLE_SIZE) as u16;

        [
            LICENSE_HEADER_BUFFER_NO_SIZE.as_ref(),
            &preamble_size_field.to_le_bytes(),
            buf.as_slice()
        ]
        .concat()
    };

    pub(crate) static ref SERVER_LICENSE_REQUEST: ServerLicenseRequest = ServerLicenseRequest {
        server_random: Vec::from(SERVER_RANDOM_BUFFER.as_ref()),
        product_info: ProductInfo {
            version: 0x60000,
            company_name: "Microsoft Corporation".to_string(),
            product_id: "A02".to_string(),
        },
        server_certificate: Some(ServerCertificate {
            issued_permanently: true,
            certificate: CertificateType::X509(X509CertificateChain {
                certificate_array: vec![
                    vec![0x30, 0x82, 0x03, 0xda, 0x30, 0x82, 0x02, 0xc2, 0xa0, 0x03, 0x02, 0x01, 0x02, 0x02, 0x13, 0x7f,
                    0x00, 0x00, 0x01, 0x76, 0x00, 0x8f, 0x08, 0x64, 0x08, 0x68, 0xa7, 0x63, 0x00, 0x00, 0x00, 0x00, 0x01,
                    0x76, 0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b, 0x05, 0x00, 0x30,
                    0x1d, 0x31, 0x1b, 0x30, 0x19, 0x06, 0x03, 0x55, 0x04, 0x03, 0x13, 0x12, 0x50, 0x72, 0x6f, 0x64, 0x32,
                    0x4c, 0x53, 0x52, 0x41, 0x73, 0x68, 0x61, 0x32, 0x52, 0x44, 0x53, 0x4c, 0x4d, 0x30, 0x1e, 0x17, 0x0d,
                    0x31, 0x39, 0x31, 0x30, 0x32, 0x36, 0x32, 0x32, 0x35, 0x33, 0x34, 0x30, 0x5a, 0x17, 0x0d, 0x32, 0x37,
                    0x30, 0x36, 0x30, 0x36, 0x32, 0x30, 0x34, 0x32, 0x33, 0x38, 0x5a, 0x30, 0x11, 0x31, 0x0f, 0x30, 0x0d,
                    0x06, 0x03, 0x55, 0x04, 0x03, 0x13, 0x06, 0x42, 0x65, 0x63, 0x6b, 0x65, 0x72, 0x30, 0x82, 0x01, 0x22,
                    0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00, 0x03, 0x82,
                    0x01, 0x0f, 0x00, 0x30, 0x82, 0x01, 0x0a, 0x02, 0x82, 0x01, 0x01, 0x00, 0xa8, 0x6b, 0xda, 0xae, 0x08,
                    0x1d, 0xc5, 0x05, 0x70, 0x7d, 0xa0, 0x41, 0x46, 0xb4, 0x14, 0xcf, 0xfb, 0x8e, 0x09, 0x0b, 0x0a, 0x52,
                    0x8a, 0x7f, 0x7a, 0x35, 0xb6, 0xe3, 0x0d, 0x1c, 0xbe, 0x49, 0x63, 0x41, 0x92, 0x86, 0x00, 0xa2, 0xd3,
                    0xff, 0x5b, 0x08, 0x7d, 0x2b, 0x65, 0xe4, 0xc3, 0x09, 0x68, 0x72, 0x21, 0xc4, 0xd8, 0x0a, 0x21, 0x9e,
                    0x1f, 0xdf, 0xb2, 0xaa, 0x2b, 0x42, 0x68, 0xe7, 0xeb, 0x52, 0xf8, 0x9e, 0xfc, 0x7f, 0x0f, 0x55, 0x26,
                    0x7d, 0x44, 0xfb, 0x35, 0xe5, 0xc2, 0x2c, 0xb6, 0x8d, 0x06, 0xc5, 0xdc, 0xbf, 0x66, 0xf6, 0xb2, 0xf2,
                    0x9b, 0xe2, 0x49, 0xaf, 0xfd, 0x4c, 0x69, 0x46, 0x72, 0xe0, 0x2f, 0x31, 0x77, 0x86, 0x7b, 0x5b, 0x6d,
                    0x49, 0xe6, 0xc7, 0x84, 0xd1, 0xdd, 0x56, 0x89, 0x8d, 0xbd, 0x07, 0x18, 0x01, 0x43, 0x70, 0x9b, 0x00,
                    0x71, 0x16, 0x89, 0x66, 0x2e, 0xb6, 0x5f, 0x62, 0xeb, 0x96, 0xed, 0xf2, 0xdb, 0xdb, 0xcf, 0xdd, 0xa8,
                    0xab, 0xde, 0x93, 0xb3, 0xdb, 0x54, 0xf0, 0x34, 0x4a, 0x28, 0xc3, 0x11, 0xf6, 0xb9, 0xd6, 0x45, 0x3f,
                    0x07, 0xc0, 0x8e, 0x10, 0x7a, 0x2b, 0x56, 0x15, 0xbb, 0x00, 0x9d, 0x82, 0x27, 0xf2, 0x11, 0xa3, 0xda,
                    0x03, 0xaa, 0x51, 0xc0, 0xfd, 0x90, 0xc8, 0x73, 0x81, 0xce, 0x97, 0x30, 0xa2, 0x54, 0x63, 0x6f, 0xfc,
                    0x7f, 0x5b, 0x71, 0xec, 0x11, 0xb0, 0xa0, 0xc8, 0x74, 0x3a, 0xcc, 0x1b, 0x5e, 0xcd, 0x91, 0xa8, 0x18,
                    0x92, 0xeb, 0x33, 0xc4, 0x6d, 0xb8, 0x16, 0x67, 0xe1, 0xc5, 0xa6, 0x26, 0x35, 0x48, 0xc4, 0xe7, 0x94,
                    0xeb, 0xbb, 0xb8, 0xde, 0xd3, 0xe1, 0xc0, 0xcb, 0x00, 0x20, 0xf6, 0xbc, 0xa9, 0xc5, 0x70, 0xc4, 0xda,
                    0x1b, 0x61, 0x0b, 0x9f, 0x0b, 0x19, 0x93, 0xaf, 0x8f, 0x40, 0xbb, 0x26, 0x79, 0x02, 0x03, 0x01, 0x00,
                    0x01, 0xa3, 0x82, 0x01, 0x1d, 0x30, 0x82, 0x01, 0x19, 0x30, 0x1d, 0x06, 0x03, 0x55, 0x1d, 0x0e, 0x04,
                    0x16, 0x04, 0x14, 0xa3, 0xda, 0xe5, 0xef, 0xc3, 0x1c, 0x7a, 0xcf, 0x34, 0x2b, 0xa2, 0x42, 0x2b, 0x77,
                    0xcb, 0x62, 0xfb, 0x4c, 0x28, 0x51, 0x30, 0x1f, 0x06, 0x03, 0x55, 0x1d, 0x23, 0x04, 0x18, 0x30, 0x16,
                    0x80, 0x14, 0x9c, 0xe1, 0xad, 0x8f, 0xd4, 0x86, 0xd2, 0x1c, 0x7e, 0x48, 0x32, 0xf2, 0x28, 0xfe, 0x87,
                    0x90, 0xe3, 0xb1, 0xc5, 0x8e, 0x30, 0x4a, 0x06, 0x03, 0x55, 0x1d, 0x1f, 0x04, 0x43, 0x30, 0x41, 0x30,
                    0x3f, 0xa0, 0x3d, 0xa0, 0x3b, 0x86, 0x39, 0x66, 0x69, 0x6c, 0x65, 0x3a, 0x2f, 0x2f, 0x2f, 0x2f, 0x52,
                    0x44, 0x32, 0x38, 0x31, 0x38, 0x37, 0x38, 0x30, 0x45, 0x33, 0x45, 0x45, 0x43, 0x2f, 0x43, 0x65, 0x72,
                    0x74, 0x45, 0x6e, 0x72, 0x6f, 0x6c, 0x6c, 0x2f, 0x50, 0x72, 0x6f, 0x64, 0x32, 0x4c, 0x53, 0x52, 0x41,
                    0x73, 0x68, 0x61, 0x32, 0x52, 0x44, 0x53, 0x4c, 0x4d, 0x2e, 0x63, 0x72, 0x6c, 0x30, 0x64, 0x06, 0x08,
                    0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x01, 0x01, 0x04, 0x58, 0x30, 0x56, 0x30, 0x54, 0x06, 0x08, 0x2b,
                    0x06, 0x01, 0x05, 0x05, 0x07, 0x30, 0x02, 0x86, 0x48, 0x66, 0x69, 0x6c, 0x65, 0x3a, 0x2f, 0x2f, 0x2f,
                    0x2f, 0x52, 0x44, 0x32, 0x38, 0x31, 0x38, 0x37, 0x38, 0x30, 0x45, 0x33, 0x45, 0x45, 0x43, 0x2f, 0x43,
                    0x65, 0x72, 0x74, 0x45, 0x6e, 0x72, 0x6f, 0x6c, 0x6c, 0x2f, 0x52, 0x44, 0x32, 0x38, 0x31, 0x38, 0x37,
                    0x38, 0x30, 0x45, 0x33, 0x45, 0x45, 0x43, 0x5f, 0x50, 0x72, 0x6f, 0x64, 0x32, 0x4c, 0x53, 0x52, 0x41,
                    0x73, 0x68, 0x61, 0x32, 0x52, 0x44, 0x53, 0x4c, 0x4d, 0x2e, 0x63, 0x72, 0x74, 0x30, 0x0c, 0x06, 0x03,
                    0x55, 0x1d, 0x13, 0x01, 0x01, 0xff, 0x04, 0x02, 0x30, 0x00, 0x30, 0x17, 0x06, 0x08, 0x2b, 0x06, 0x01,
                    0x04, 0x01, 0x82, 0x37, 0x12, 0x04, 0x0b, 0x16, 0x09, 0x54, 0x4c, 0x53, 0x7e, 0x42, 0x41, 0x53, 0x49,
                    0x43, 0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b, 0x05, 0x00, 0x03,
                    0x82, 0x01, 0x01, 0x00, 0x55, 0xd5, 0x94, 0x3b, 0x06, 0xef, 0xf2, 0xb0, 0xf9, 0xd7, 0x36, 0x2a, 0x36,
                    0xe0, 0xf1, 0xd9, 0x18, 0xc1, 0x89, 0x7e, 0xa2, 0xcf, 0x01, 0x6f, 0x22, 0x7b, 0x34, 0x81, 0xf0, 0x7a,
                    0x45, 0x11, 0x6e, 0x75, 0x4b, 0x0b, 0xa8, 0xcd, 0x92, 0x57, 0x19, 0x80, 0xb7, 0x6e, 0x1a, 0x4d, 0x12,
                    0x65, 0x91, 0x56, 0x38, 0x17, 0x22, 0xa2, 0x75, 0xae, 0xf9, 0x12, 0x75, 0x38, 0xf3, 0x19, 0x74, 0xea,
                    0x87, 0x46, 0x1f, 0x98, 0x2c, 0x2f, 0xf9, 0xfc, 0xb4, 0xdc, 0x25, 0xa0, 0xd3, 0x34, 0x1b, 0xbc, 0x21,
                    0xbb, 0x3d, 0x82, 0xad, 0x15, 0xc6, 0x3d, 0x02, 0x75, 0x33, 0x70, 0x25, 0x0a, 0x1a, 0xf7, 0x4c, 0xcb,
                    0x84, 0xa3, 0xc1, 0x78, 0xe6, 0xf5, 0xa1, 0x44, 0x54, 0xc8, 0x34, 0xfd, 0xef, 0xbf, 0x86, 0x81, 0x9d,
                    0x9a, 0x7e, 0xb6, 0xad, 0x71, 0x7e, 0xe4, 0xd9, 0x71, 0x6c, 0xb9, 0xe7, 0xf2, 0xd6, 0xd7, 0xbb, 0x66,
                    0x5a, 0x30, 0xf5, 0x29, 0xae, 0x02, 0x39, 0x3d, 0xea, 0x7a, 0x79, 0x1b, 0x53, 0xc5, 0xbe, 0x8d, 0xfb,
                    0xe2, 0xe4, 0x8e, 0xc2, 0x04, 0xb3, 0x0a, 0x94, 0x75, 0xa3, 0xbf, 0xd4, 0x87, 0xd2, 0x74, 0x15, 0x05,
                    0x5e, 0xd5, 0x8f, 0x94, 0x23, 0x41, 0x13, 0x3f, 0xbd, 0xed, 0x21, 0x55, 0x96, 0xe9, 0xc4, 0x93, 0x34,
                    0x7f, 0xaa, 0xea, 0xe7, 0xb1, 0x9a, 0xca, 0x25, 0x91, 0x18, 0xdf, 0x28, 0x05, 0x8e, 0x53, 0xb3, 0x8c,
                    0x8d, 0xcc, 0xf3, 0xf4, 0x78, 0x76, 0x76, 0x7b, 0x82, 0xd6, 0x75, 0x7a, 0x7d, 0xb3, 0x23, 0x2c, 0xc7,
                    0xbe, 0xa6, 0xb0, 0x50, 0x4d, 0x6c, 0xe2, 0x90, 0x85, 0x97, 0x77, 0x0d, 0x2f, 0xf5, 0x7b, 0xb0, 0xc6,
                    0xad, 0xfa, 0x9a, 0x2c, 0xdf, 0xeb, 0x0d, 0x60, 0xd3, 0x0e, 0xa8, 0x5c, 0x43, 0xab, 0x09, 0x85, 0xa3,
                    0xa9, 0x31, 0x66, 0xbd, 0xe4],
                    vec![0x30, 0x82, 0x04, 0x59, 0x30, 0x82, 0x03, 0x45, 0xa0, 0x03, 0x02, 0x01, 0x02, 0x02, 0x05, 0x01,
                    0x00, 0x00, 0x00, 0x02, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1d, 0x05, 0x00, 0x30, 0x11,
                    0x31, 0x0f, 0x30, 0x0d, 0x06, 0x03, 0x55, 0x04, 0x03, 0x13, 0x06, 0x42, 0x65, 0x63, 0x6b, 0x65, 0x72,
                    0x30, 0x1e, 0x17, 0x0d, 0x31, 0x39, 0x31, 0x30, 0x32, 0x36, 0x32, 0x33, 0x32, 0x36, 0x34, 0x35, 0x5a,
                    0x17, 0x0d, 0x33, 0x38, 0x30, 0x31, 0x31, 0x39, 0x30, 0x33, 0x31, 0x34, 0x30, 0x37, 0x5a, 0x30, 0x81,
                    0xa6, 0x31, 0x81, 0xa3, 0x30, 0x27, 0x06, 0x03, 0x55, 0x04, 0x03, 0x1e, 0x20, 0x00, 0x6e, 0x00, 0x63,
                    0x00, 0x61, 0x00, 0x63, 0x00, 0x6e, 0x00, 0x5f, 0x00, 0x69, 0x00, 0x70, 0x00, 0x5f, 0x00, 0x74, 0x00,
                    0x63, 0x00, 0x70, 0x00, 0x3a, 0x00, 0x31, 0x00, 0x32, 0x00, 0x37, 0x30, 0x33, 0x06, 0x03, 0x55, 0x04,
                    0x07, 0x1e, 0x2c, 0x00, 0x6e, 0x00, 0x63, 0x00, 0x61, 0x00, 0x63, 0x00, 0x6e, 0x00, 0x5f, 0x00, 0x69,
                    0x00, 0x70, 0x00, 0x5f, 0x00, 0x74, 0x00, 0x63, 0x00, 0x70, 0x00, 0x3a, 0x00, 0x31, 0x00, 0x32, 0x00,
                    0x37, 0x00, 0x2e, 0x00, 0x30, 0x00, 0x2e, 0x00, 0x30, 0x00, 0x2e, 0x00, 0x31, 0x30, 0x43, 0x06, 0x03,
                    0x55, 0x04, 0x05, 0x1e, 0x3c, 0x00, 0x31, 0x00, 0x42, 0x00, 0x63, 0x00, 0x4b, 0x00, 0x65, 0x00, 0x56,
                    0x00, 0x33, 0x00, 0x4d, 0x00, 0x67, 0x00, 0x74, 0x00, 0x6a, 0x00, 0x55, 0x00, 0x74, 0x00, 0x6f, 0x00,
                    0x32, 0x00, 0x50, 0x00, 0x49, 0x00, 0x68, 0x00, 0x35, 0x00, 0x52, 0x00, 0x57, 0x00, 0x56, 0x00, 0x36,
                    0x00, 0x42, 0x00, 0x58, 0x00, 0x48, 0x00, 0x77, 0x00, 0x3d, 0x00, 0x0d, 0x00, 0x0a, 0x30, 0x58, 0x30,
                    0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x0f, 0x05, 0x00, 0x03, 0x4b, 0x00, 0x30, 0x48, 0x02, 0x41,
                    0x00, 0xab, 0xac, 0x87, 0x11, 0x83, 0xbf, 0xe9, 0x48, 0x25, 0x00, 0x2c, 0x33, 0x31, 0x5e, 0x3d, 0x78,
                    0xc8, 0x5f, 0x82, 0xcb, 0x36, 0x41, 0xf5, 0xb4, 0x65, 0x15, 0xee, 0x04, 0x31, 0xae, 0xe2, 0x48, 0x58,
                    0x99, 0x7f, 0x4f, 0x90, 0x1d, 0xf7, 0x7c, 0xd7, 0xf8, 0x47, 0x93, 0xa0, 0xca, 0x9c, 0xdf, 0x91, 0xb0,
                    0x41, 0xe8, 0x05, 0x4b, 0xdc, 0x24, 0x5b, 0x72, 0xf7, 0x68, 0x91, 0x84, 0xfb, 0x19, 0x02, 0x03, 0x01,
                    0x00, 0x01, 0xa3, 0x82, 0x01, 0xf4, 0x30, 0x82, 0x01, 0xf0, 0x30, 0x14, 0x06, 0x09, 0x2b, 0x06, 0x01,
                    0x04, 0x01, 0x82, 0x37, 0x12, 0x04, 0x01, 0x01, 0xff, 0x04, 0x04, 0x01, 0x00, 0x05, 0x00, 0x30, 0x3c,
                    0x06, 0x09, 0x2b, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x12, 0x02, 0x01, 0x01, 0xff, 0x04, 0x2c, 0x4d,
                    0x00, 0x69, 0x00, 0x63, 0x00, 0x72, 0x00, 0x6f, 0x00, 0x73, 0x00, 0x6f, 0x00, 0x66, 0x00, 0x74, 0x00,
                    0x20, 0x00, 0x43, 0x00, 0x6f, 0x00, 0x72, 0x00, 0x70, 0x00, 0x6f, 0x00, 0x72, 0x00, 0x61, 0x00, 0x74,
                    0x00, 0x69, 0x00, 0x6f, 0x00, 0x6e, 0x00, 0x00, 0x00, 0x30, 0x81, 0xdd, 0x06, 0x09, 0x2b, 0x06, 0x01,
                    0x04, 0x01, 0x82, 0x37, 0x12, 0x05, 0x01, 0x01, 0xff, 0x04, 0x81, 0xcc, 0x00, 0x30, 0x00, 0x00, 0x01,
                    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x22, 0x04, 0x00, 0x00, 0x1c, 0x00, 0x4a, 0x00, 0x66, 0x00,
                    0x4a, 0x00, 0xb0, 0x00, 0x03, 0x00, 0x33, 0x00, 0x64, 0x00, 0x32, 0x00, 0x36, 0x00, 0x37, 0x00, 0x39,
                    0x00, 0x35, 0x00, 0x34, 0x00, 0x2d, 0x00, 0x65, 0x00, 0x65, 0x00, 0x62, 0x00, 0x37, 0x00, 0x2d, 0x00,
                    0x31, 0x00, 0x31, 0x00, 0x64, 0x00, 0x31, 0x00, 0x2d, 0x00, 0x62, 0x00, 0x39, 0x00, 0x34, 0x00, 0x65,
                    0x00, 0x2d, 0x00, 0x30, 0x00, 0x30, 0x00, 0x63, 0x00, 0x30, 0x00, 0x34, 0x00, 0x66, 0x00, 0x61, 0x00,
                    0x33, 0x00, 0x30, 0x00, 0x38, 0x00, 0x30, 0x00, 0x64, 0x00, 0x00, 0x00, 0x33, 0x00, 0x64, 0x00, 0x32,
                    0x00, 0x36, 0x00, 0x37, 0x00, 0x39, 0x00, 0x35, 0x00, 0x34, 0x00, 0x2d, 0x00, 0x65, 0x00, 0x65, 0x00,
                    0x62, 0x00, 0x37, 0x00, 0x2d, 0x00, 0x31, 0x00, 0x31, 0x00, 0x64, 0x00, 0x31, 0x00, 0x2d, 0x00, 0x62,
                    0x00, 0x39, 0x00, 0x34, 0x00, 0x65, 0x00, 0x2d, 0x00, 0x30, 0x00, 0x30, 0x00, 0x63, 0x00, 0x30, 0x00,
                    0x34, 0x00, 0x66, 0x00, 0x61, 0x00, 0x33, 0x00, 0x30, 0x00, 0x38, 0x00, 0x30, 0x00, 0x64, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x80, 0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x80, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x30, 0x81, 0x80, 0x06, 0x09,
                    0x2b, 0x06, 0x01, 0x04, 0x01, 0x82, 0x37, 0x12, 0x06, 0x01, 0x01, 0xff, 0x04, 0x70, 0x00, 0x30, 0x00,
                    0x00, 0x00, 0x00, 0x20, 0x00, 0x50, 0x00, 0x57, 0x00, 0x49, 0x00, 0x4e, 0x00, 0x2d, 0x00, 0x34, 0x00,
                    0x4c, 0x00, 0x34, 0x00, 0x4c, 0x00, 0x36, 0x00, 0x41, 0x00, 0x4d, 0x00, 0x42, 0x00, 0x43, 0x00, 0x53,
                    0x00, 0x51, 0x00, 0x00, 0x00, 0x30, 0x00, 0x30, 0x00, 0x34, 0x00, 0x32, 0x00, 0x39, 0x00, 0x2d, 0x00,
                    0x30, 0x00, 0x30, 0x00, 0x30, 0x00, 0x30, 0x00, 0x30, 0x00, 0x2d, 0x00, 0x33, 0x00, 0x34, 0x00, 0x39,
                    0x00, 0x37, 0x00, 0x32, 0x00, 0x2d, 0x00, 0x41, 0x00, 0x54, 0x00, 0x33, 0x00, 0x35, 0x00, 0x33, 0x00,
                    0x00, 0x00, 0x57, 0x00, 0x4f, 0x00, 0x52, 0x00, 0x4b, 0x00, 0x47, 0x00, 0x52, 0x00, 0x4f, 0x00, 0x55,
                    0x00, 0x50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x30, 0x37, 0x06, 0x03, 0x55, 0x1d, 0x23, 0x01, 0x01, 0xff,
                    0x04, 0x2d, 0x30, 0x2b, 0xa1, 0x22, 0xa4, 0x20, 0x57, 0x00, 0x49, 0x00, 0x4e, 0x00, 0x2d, 0x00, 0x34,
                    0x00, 0x4c, 0x00, 0x34, 0x00, 0x4c, 0x00, 0x36, 0x00, 0x41, 0x00, 0x4d, 0x00, 0x42, 0x00, 0x43, 0x00,
                    0x53, 0x00, 0x51, 0x00, 0x00, 0x00, 0x82, 0x05, 0x01, 0x00, 0x00, 0x00, 0x02, 0x30, 0x09, 0x06, 0x05,
                    0x2b, 0x0e, 0x03, 0x02, 0x1d, 0x05, 0x00, 0x03, 0x82, 0x01, 0x01, 0x00, 0x3e, 0xd3, 0xd5, 0x61, 0x8a,
                    0x87, 0x7b, 0x98, 0x2c, 0x6d, 0x20, 0x38, 0x12, 0x08, 0xd8, 0xf7, 0x83, 0x08, 0xf8, 0xe6, 0xb2, 0xe1,
                    0x21, 0xe1, 0x30, 0x61, 0x12, 0x19, 0xe8, 0xc1, 0x41, 0xaf, 0x59, 0x7c, 0x1e, 0x3e, 0xc8, 0x40, 0x9e,
                    0x24, 0xe8, 0x8d, 0x0c, 0x41, 0xfd, 0xf8, 0x3e, 0xa1, 0xb3, 0xac, 0x56, 0xac, 0x52, 0x91, 0x5a, 0xf8,
                    0xd0, 0x40, 0x8e, 0x13, 0x47, 0xa9, 0x8a, 0x0a, 0x62, 0x6d, 0x11, 0x89, 0x20, 0x56, 0xe7, 0xd6, 0x5f,
                    0x12, 0x44, 0x94, 0xbf, 0x63, 0x99, 0xa3, 0x42, 0x40, 0xd5, 0xc6, 0x8c, 0x1f, 0x4b, 0xf8, 0xaf, 0x83,
                    0x8e, 0xf6, 0x74, 0xb2, 0x0b, 0x55, 0x13, 0x4a, 0x76, 0xed, 0x37, 0xd8, 0x3d, 0x13, 0xe7, 0xae, 0x43,
                    0x4c, 0x9a, 0x61, 0x6c, 0x7b, 0x1b, 0xd1, 0xaa, 0x00, 0x97, 0xdf, 0x5b, 0x85, 0x9f, 0xc8, 0xee, 0x6c,
                    0xe5, 0xa2, 0x63, 0x76, 0xe4, 0x06, 0xd3, 0x2a, 0xe0, 0x55, 0xe1, 0x92, 0x78, 0xed, 0x03, 0x7b, 0x7d,
                    0x1a, 0x6e, 0xc2, 0x56, 0xdc, 0xad, 0x6e, 0xd7, 0xa9, 0xfe, 0xa7, 0xfd, 0x09, 0x0a, 0xa6, 0xd5, 0x8a,
                    0x99, 0xa4, 0x75, 0x89, 0xad, 0x84, 0xc7, 0x09, 0xf7, 0x4c, 0x6e, 0xd0, 0xe2, 0x80, 0x17, 0x62, 0xfa,
                    0x86, 0xfe, 0x43, 0x51, 0xf2, 0xb4, 0xf6, 0xef, 0x3b, 0xb3, 0x3d, 0x1f, 0xef, 0xa3, 0xcb, 0xa2, 0x57,
                    0x25, 0x7c, 0x02, 0xf2, 0x27, 0x1c, 0x87, 0x70, 0x8e, 0x84, 0x20, 0xfe, 0x1d, 0x4a, 0xc4, 0x87, 0x24,
                    0x3b, 0xba, 0xff, 0x34, 0x1a, 0xe2, 0xff, 0xa2, 0x43, 0x39, 0xd8, 0x19, 0x97, 0xf8, 0xf0, 0xf9, 0x73,
                    0xa6, 0xb6, 0x55, 0x64, 0xa6, 0xca, 0xa3, 0x48, 0x22, 0xb7, 0x1a, 0x9b, 0x98, 0x1a, 0x8e, 0x2f, 0xaa,
                    0xec, 0xc1, 0xfe, 0x25, 0x36, 0x2b, 0x70, 0x97, 0x8c, 0x5b, 0x62, 0x21, 0xc3],
                ],
            }),
        }),
        scope_list: vec![Scope(String::from("microsoft.com"))],
    };
}

#[test]
fn from_buffer_correctly_parses_client_new_license_request() {
    assert_eq!(
        *CLIENT_NEW_LICENSE_REQUEST,
        ClientNewLicenseRequest::from_buffer(&mut REQUEST_BUFFER.as_slice()).unwrap()
    );
}

#[test]
fn to_buffer_correctly_serializes_client_new_license_request() {
    let mut serialized_request = Vec::new();
    CLIENT_NEW_LICENSE_REQUEST.to_buffer(&mut serialized_request).unwrap();

    assert_eq!(REQUEST_BUFFER.as_slice(), serialized_request.as_slice());
}

#[test]
fn buffer_length_is_correct_for_client_new_license_request() {
    assert_eq!(REQUEST_BUFFER.len(), CLIENT_NEW_LICENSE_REQUEST.buffer_length());
}

#[test]
#[ignore] // FIXME: unknown/unsupported ASN.1 DER tag: 0x1e at DER byte 186
fn client_new_license_request_creates_correctly() {
    let (client_new_license_request, encryption_data) = ClientNewLicenseRequest::from_server_license_request(
        &SERVER_LICENSE_REQUEST,
        CLIENT_RANDOM_BUFFER.as_ref(),
        PREMASTER_SECRET_BUFFER.as_ref(),
        CLIENT_USERNAME,
        CLIENT_MACHINE_NAME,
    )
    .unwrap();

    assert_eq!(encryption_data.license_key, LICENSE_KEY_BUFFER.as_ref());
    assert_eq!(client_new_license_request, *CLIENT_NEW_LICENSE_REQUEST);
}

#[test]
fn salted_hash_produces_result_correctly() {
    let result = salted_hash(
        &PREMASTER_SECRET_BUFFER,
        &CLIENT_RANDOM_BUFFER,
        &SERVER_RANDOM_BUFFER,
        b"A",
    );

    assert_eq!(result, SALTED_HASH_BUFFER.as_ref());
}

#[test]
fn master_secret_generates_correctly() {
    let result = compute_master_secret(
        PREMASTER_SECRET_BUFFER.as_ref(),
        CLIENT_RANDOM_BUFFER.as_ref(),
        SERVER_RANDOM_BUFFER.as_ref(),
    );

    assert_eq!(result, MASTER_SECRET_BUFFER.as_ref());
}

#[test]
fn session_key_blob_generates_correctly() {
    let result = compute_session_key_blob(
        MASTER_SECRET_BUFFER.as_ref(),
        CLIENT_RANDOM_BUFFER.as_ref(),
        SERVER_RANDOM_BUFFER.as_ref(),
    );

    assert_eq!(result, SESSION_KEY_BLOB.as_ref());
}
