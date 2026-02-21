use std::io::{self, Read, Write};

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use uuid::Uuid;

use super::data_types::{
    AuthScheme, AuthSchemeVector, ByteVector, CHECKSUM_HEADER_LEN, Checksum, Extension, ExtensionVector, MessageHeader,
    MessageType,
};
use super::{
    CHECKSUM_SCHEME_RFC3961, NEGOEXTS_MESSAGE_SIGNATURE, NegoexDataType, NegoexMessage, PROTOCOL_VERSION,
    RANDOM_ARRAY_SIZE,
};

const NEGO_MESSAGE_HEADER_LEN: u32 = 40 /* header */ + (RANDOM_ARRAY_SIZE as u32) + 8 /* protocol version */ + 8 /* auth schemes header */ + 8 /* extensions header */;
const EXCHANGE_MESSAGE_HEADER_LEN: u32 = 40 /* header */ + 16 /* auth scheme */ + 8 /* exchange header */;
const VERIFY_MESSAGE_HEADER_LEN: u32 = 40 /* header */ + 16 /* auth scheme */ + 4 /* padding */ + CHECKSUM_HEADER_LEN;

/// [2.2.6.3 NEGO_MESSAGE](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-NEGOEX/%5bMS-NEGOEX%5d.pdf)
/// ```not_rust
/// struct
/// {
///     MESSAGE_HEADER Header;
///     UCHAR Random[32];
///     ULONG64 ProtocolVersion;
///     AUTH_SCHEME_VECTOR AuthSchemes;
///     EXTENSION_VECTOR Extensions;
/// } NEGO_MESSAGE;
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nego {
    pub header: MessageHeader,
    pub random: [u8; RANDOM_ARRAY_SIZE],
    pub protocol_version: u64,
    pub auth_schemes: AuthSchemeVector,
    pub extensions: ExtensionVector,
}

impl Nego {
    pub fn new(
        message_type: MessageType,
        conversation_id: Uuid,
        sequence_num: u32,
        random: [u8; RANDOM_ARRAY_SIZE],
        auth_schemes: Vec<Uuid>,
        extensions: Vec<Extension>,
    ) -> Self {
        let auth_schemes: AuthSchemeVector = auth_schemes.into_iter().collect();

        let mut header = MessageHeader {
            signature: NEGOEXTS_MESSAGE_SIGNATURE,
            message_type,
            sequence_num,
            header_len: NEGO_MESSAGE_HEADER_LEN,
            message_len: 0,
            conversation_id,
        };

        header.message_len = (header.size() + RANDOM_ARRAY_SIZE + 8 /* protocol version */ + auth_schemes.size() + extensions.size())
            as u32;

        Self {
            header,
            random,
            protocol_version: PROTOCOL_VERSION,
            auth_schemes,
            extensions,
        }
    }
}

impl NegoexMessage for Nego {
    type Error = io::Error;

    fn decode(message: &[u8]) -> Result<Self, Self::Error> {
        let mut from = message;

        let header: MessageHeader = NegoexDataType::decode(&mut from, &[])?;

        let mut random = [0; RANDOM_ARRAY_SIZE];
        from.read_exact(&mut random)?;

        let protocol_version = from.read_u64::<LittleEndian>()?;

        let auth_schemes = NegoexDataType::decode(&mut from, message)?;

        let extensions = NegoexDataType::decode(&mut from, message)?;

        Ok(Self {
            header,
            random,
            protocol_version,
            auth_schemes,
            extensions,
        })
    }

    fn encode(&self, mut to: impl Write) -> Result<(), Self::Error> {
        let mut message_header = Vec::new();
        let mut message_data = Vec::new();

        let offset = self.header.header_len as usize;

        self.header.encode(&mut message_header)?;

        message_header.write_all(&self.random)?;

        message_header.write_u64::<LittleEndian>(self.protocol_version)?;

        self.auth_schemes
            .encode_with_payload(offset, &mut message_header, &mut message_data)?;

        self.extensions
            .encode_with_payload(offset, &mut message_header, &mut message_data)?;

        to.write_all(&message_header)?;
        to.write_all(&message_data)?;

        Ok(())
    }
}

/// [2.2.6.4 EXCHANGE_MESSAGE](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-NEGOEX/%5bMS-NEGOEX%5d.pdf)
/// ```not_rust
/// struct
/// {
///     MESSAGE_HEADER Header;
///     AUTH_SCHEME AuthScheme;
///     BYTE_VECTOR Exchange;
/// } EXCHANGE_MESSAGE;
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    pub header: MessageHeader,
    pub auth_scheme: AuthScheme,
    pub exchange: ByteVector,
}

impl Exchange {
    pub fn new(
        message_type: MessageType,
        conversation_id: Uuid,
        sequence_num: u32,
        auth_scheme: Uuid,
        exchange: Vec<u8>,
    ) -> Self {
        let mut header = MessageHeader {
            signature: NEGOEXTS_MESSAGE_SIGNATURE,
            message_type,
            sequence_num,
            header_len: EXCHANGE_MESSAGE_HEADER_LEN,
            message_len: 0,
            conversation_id,
        };

        header.message_len = (header.size() + auth_scheme.size() + exchange.size()) as u32;

        Exchange {
            header,
            auth_scheme,
            exchange,
        }
    }
}

impl NegoexMessage for Exchange {
    type Error = io::Error;

    fn decode(message: &[u8]) -> Result<Self, Self::Error> {
        let mut from = message;

        Ok(Self {
            header: NegoexDataType::decode(&mut from, message)?,
            auth_scheme: NegoexDataType::decode(&mut from, message)?,
            exchange: NegoexDataType::decode(&mut from, message)?,
        })
    }

    fn encode(&self, mut to: impl Write) -> Result<(), Self::Error> {
        let offset = self.header.header_len as usize;

        let mut message_header = Vec::new();
        let mut message_data = Vec::new();

        self.header.encode(&mut message_header)?;

        self.auth_scheme.encode(&mut message_header)?;

        self.exchange
            .encode_with_payload(offset, &mut message_header, &mut message_data)?;

        to.write_all(&message_header)?;
        to.write_all(&message_data)?;

        Ok(())
    }
}

/// [2.2.6.5 VERIFY_MESSAGE](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-NEGOEX/%5bMS-NEGOEX%5d.pdf)
/// ```not_rust
/// struct
/// {
///     MESSAGE_HEADER Header;
///     AUTH_SCHEME AuthScheme;
///     CHECKSUM Checksum;
/// } VERIFY_MESSAGE;
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verify {
    pub header: MessageHeader,
    pub auth_scheme: AuthScheme,
    pub checksum: Checksum,
}

impl Verify {
    pub fn new(
        message_type: MessageType,
        conversation_id: Uuid,
        sequence_num: u32,
        auth_scheme: Uuid,
        checksum_type: u32,
        checksum_value: Vec<u8>,
    ) -> Self {
        let mut header = MessageHeader {
            signature: NEGOEXTS_MESSAGE_SIGNATURE,
            message_type,
            sequence_num,
            header_len: VERIFY_MESSAGE_HEADER_LEN,
            message_len: 0,
            conversation_id,
        };

        let checksum = Checksum {
            header_len: CHECKSUM_HEADER_LEN,
            checksum_scheme: CHECKSUM_SCHEME_RFC3961,
            checksum_type,
            checksum_value,
        };

        header.message_len = (header.size() + auth_scheme.size() + checksum.size()) as u32;

        Self {
            header,
            auth_scheme,
            checksum,
        }
    }
}

impl NegoexMessage for Verify {
    type Error = io::Error;

    fn decode(message: &[u8]) -> Result<Self, Self::Error> {
        let mut from = message;

        Ok(Self {
            header: NegoexDataType::decode(&mut from, message)?,
            auth_scheme: NegoexDataType::decode(&mut from, message)?,
            checksum: NegoexDataType::decode(&mut from, message)?,
        })
    }

    fn encode(&self, mut to: impl Write) -> Result<(), Self::Error> {
        let offset = self.header.header_len as usize;

        let mut message_header = Vec::new();
        // 4 byte padding
        let mut message_data = vec![0, 0, 0, 0];

        self.header.encode(&mut message_header)?;

        self.auth_scheme.encode(&mut message_header)?;

        self.checksum
            .encode_with_payload(offset, &mut message_header, &mut message_data)?;

        to.write_all(&message_header)?;
        to.write_all(&message_data)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use crate::constants::cksum_types::HMAC_SHA1_96_AES256;
    use crate::negoex::data_types::{Checksum, Guid, MessageHeader, MessageType};
    use crate::negoex::{CHECKSUM_SCHEME_RFC3961, NEGOEXTS_MESSAGE_SIGNATURE, NegoexMessage};

    use super::{Exchange, Nego, Verify};

    #[test]
    fn nego_decode() {
        let encoded = [
            78, 69, 71, 79, 69, 88, 84, 83, 0, 0, 0, 0, 0, 0, 0, 0, 96, 0, 0, 0, 112, 0, 0, 0, 90, 7, 41, 59, 145, 243,
            51, 175, 161, 180, 162, 18, 36, 157, 124, 180, 171, 30, 157, 109, 166, 119, 29, 212, 26, 40, 14, 87, 69,
            187, 217, 132, 195, 93, 44, 219, 112, 114, 184, 136, 25, 92, 118, 239, 113, 111, 71, 120, 0, 0, 0, 0, 0, 0,
            0, 0, 96, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 92, 51, 83, 13, 234, 249, 13, 77, 178, 236, 74, 227,
            120, 110, 195, 8,
        ];

        let nego = Nego::decode(&encoded).unwrap();

        assert_eq!(
            Nego {
                header: MessageHeader {
                    signature: NEGOEXTS_MESSAGE_SIGNATURE,
                    message_type: MessageType::InitiatorNego,
                    sequence_num: 0,
                    header_len: 96,
                    message_len: 112,
                    conversation_id: Guid::from_str("3b29075a-f391-af33-a1b4-a212249d7cb4").unwrap(),
                },
                random: [
                    171, 30, 157, 109, 166, 119, 29, 212, 26, 40, 14, 87, 69, 187, 217, 132, 195, 93, 44, 219, 112,
                    114, 184, 136, 25, 92, 118, 239, 113, 111, 71, 120
                ],
                protocol_version: 0,
                auth_schemes: vec![Guid::from_str("0d53335c-f9ea-4d0d-b2ec-4ae3786ec308").unwrap()],
                extensions: Vec::new(),
            },
            nego
        );
    }

    #[test]
    fn nego_encode() {
        let nego = Nego {
            header: MessageHeader {
                signature: NEGOEXTS_MESSAGE_SIGNATURE,
                message_type: MessageType::InitiatorNego,
                sequence_num: 0,
                header_len: 96,
                message_len: 112,
                conversation_id: Guid::from_str("3b29075a-f391-af33-a1b4-a212249d7cb4").unwrap(),
            },
            random: [
                171, 30, 157, 109, 166, 119, 29, 212, 26, 40, 14, 87, 69, 187, 217, 132, 195, 93, 44, 219, 112, 114,
                184, 136, 25, 92, 118, 239, 113, 111, 71, 120,
            ],
            protocol_version: 0,
            auth_schemes: vec![Guid::from_str("0d53335c-f9ea-4d0d-b2ec-4ae3786ec308").unwrap()],
            extensions: Vec::new(),
        };

        let mut encoded = Vec::new();
        nego.encode(&mut encoded).unwrap();

        assert_eq!(
            &[
                78, 69, 71, 79, 69, 88, 84, 83, 0, 0, 0, 0, 0, 0, 0, 0, 96, 0, 0, 0, 112, 0, 0, 0, 90, 7, 41, 59, 145,
                243, 51, 175, 161, 180, 162, 18, 36, 157, 124, 180, 171, 30, 157, 109, 166, 119, 29, 212, 26, 40, 14,
                87, 69, 187, 217, 132, 195, 93, 44, 219, 112, 114, 184, 136, 25, 92, 118, 239, 113, 111, 71, 120, 0, 0,
                0, 0, 0, 0, 0, 0, 96, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 92, 51, 83, 13, 234, 249, 13, 77,
                178, 236, 74, 227, 120, 110, 195, 8,
            ],
            encoded.as_slice()
        );
    }

    #[test]
    fn exchange_decode() {
        let encoded = [
            78, 69, 71, 79, 69, 88, 84, 83, 3, 0, 0, 0, 3, 0, 0, 0, 64, 0, 0, 0, 238, 0, 0, 0, 90, 7, 41, 59, 145, 243,
            51, 175, 161, 180, 162, 18, 36, 157, 124, 180, 92, 51, 83, 13, 234, 249, 13, 77, 178, 236, 74, 227, 120,
            110, 195, 8, 64, 0, 0, 0, 174, 0, 0, 0, 48, 129, 171, 160, 129, 168, 48, 129, 165, 48, 81, 128, 79, 48, 77,
            49, 75, 48, 73, 6, 3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0, 105,
            0, 122, 0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0,
            101, 0, 115, 0, 115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 48, 39, 128, 37, 48, 35, 49, 33, 48,
            31, 6, 3, 85, 4, 3, 19, 24, 84, 111, 107, 101, 110, 32, 83, 105, 103, 110, 105, 110, 103, 32, 80, 117, 98,
            108, 105, 99, 32, 75, 101, 121, 48, 39, 128, 37, 48, 35, 49, 33, 48, 31, 6, 3, 85, 4, 3, 19, 24, 84, 111,
            107, 101, 110, 32, 83, 105, 103, 110, 105, 110, 103, 32, 80, 117, 98, 108, 105, 99, 32, 75, 101, 121,
        ];

        let exchange = Exchange::decode(&encoded).unwrap();

        assert_eq!(
            Exchange {
                header: MessageHeader {
                    signature: NEGOEXTS_MESSAGE_SIGNATURE,
                    message_type: MessageType::AcceptorMetaData,
                    sequence_num: 3,
                    header_len: 64,
                    message_len: 238,
                    conversation_id: Guid::from_str("3b29075a-f391-af33-a1b4-a212249d7cb4").unwrap(),
                },
                auth_scheme: Guid::from_str("0d53335c-f9ea-4d0d-b2ec-4ae3786ec308").unwrap(),
                exchange: vec![
                    48, 129, 171, 160, 129, 168, 48, 129, 165, 48, 81, 128, 79, 48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3,
                    30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116,
                    0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0,
                    115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 48, 39, 128, 37, 48, 35, 49, 33, 48, 31, 6,
                    3, 85, 4, 3, 19, 24, 84, 111, 107, 101, 110, 32, 83, 105, 103, 110, 105, 110, 103, 32, 80, 117, 98,
                    108, 105, 99, 32, 75, 101, 121, 48, 39, 128, 37, 48, 35, 49, 33, 48, 31, 6, 3, 85, 4, 3, 19, 24,
                    84, 111, 107, 101, 110, 32, 83, 105, 103, 110, 105, 110, 103, 32, 80, 117, 98, 108, 105, 99, 32,
                    75, 101, 121
                ],
            },
            exchange
        );
    }

    #[test]
    fn exchange_encode() {
        let exchange = Exchange {
            header: MessageHeader {
                signature: NEGOEXTS_MESSAGE_SIGNATURE,
                message_type: MessageType::InitiatorMetaData,
                sequence_num: 1,
                header_len: 64,
                message_len: 297,
                conversation_id: Guid::from_str("3b29075a-f391-af33-a1b4-a212249d7cb4").unwrap(),
            },
            auth_scheme: Guid::from_str("0d53335c-f9ea-4d0d-b2ec-4ae3786ec308").unwrap(),
            exchange: vec![
                48, 129, 230, 160, 129, 169, 48, 129, 166, 48, 81, 128, 79, 48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30,
                66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116, 0, 105,
                0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32,
                0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 48, 81, 128, 79, 48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30,
                66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116, 0, 105,
                0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32,
                0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 161, 56, 48, 54, 160, 17, 27, 15, 87, 69, 76, 76, 75, 78, 79,
                87, 78, 58, 80, 75, 85, 50, 85, 161, 33, 48, 31, 160, 3, 2, 1, 2, 161, 24, 48, 22, 27, 7, 84, 69, 82,
                77, 83, 82, 86, 27, 11, 65, 90, 82, 68, 79, 87, 78, 45, 87, 49, 48,
            ],
        };

        let mut encoded = Vec::new();
        exchange.encode(&mut encoded).unwrap();

        assert_eq!(
            &[
                78, 69, 71, 79, 69, 88, 84, 83, 2, 0, 0, 0, 1, 0, 0, 0, 64, 0, 0, 0, 41, 1, 0, 0, 90, 7, 41, 59, 145,
                243, 51, 175, 161, 180, 162, 18, 36, 157, 124, 180, 92, 51, 83, 13, 234, 249, 13, 77, 178, 236, 74,
                227, 120, 110, 195, 8, 64, 0, 0, 0, 233, 0, 0, 0, 48, 129, 230, 160, 129, 169, 48, 129, 166, 48, 81,
                128, 79, 48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0,
                97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45,
                0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 48, 81,
                128, 79, 48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0,
                97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45,
                0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 161, 56,
                48, 54, 160, 17, 27, 15, 87, 69, 76, 76, 75, 78, 79, 87, 78, 58, 80, 75, 85, 50, 85, 161, 33, 48, 31,
                160, 3, 2, 1, 2, 161, 24, 48, 22, 27, 7, 84, 69, 82, 77, 83, 82, 86, 27, 11, 65, 90, 82, 68, 79, 87,
                78, 45, 87, 49, 48
            ],
            encoded.as_slice(),
        );
    }

    #[test]
    fn verify_decode() {
        let encoded = [
            78, 69, 71, 79, 69, 88, 84, 83, 6, 0, 0, 0, 7, 0, 0, 0, 80, 0, 0, 0, 92, 0, 0, 0, 90, 7, 41, 59, 145, 243,
            51, 175, 161, 180, 162, 18, 36, 157, 124, 180, 92, 51, 83, 13, 234, 249, 13, 77, 178, 236, 74, 227, 120,
            110, 195, 8, 20, 0, 0, 0, 1, 0, 0, 0, 16, 0, 0, 0, 80, 0, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0, 228, 167, 112,
            148, 23, 131, 204, 12, 13, 36, 58, 87,
        ];

        let verify = Verify::decode(&encoded).unwrap();

        assert_eq!(
            Verify {
                header: MessageHeader {
                    signature: NEGOEXTS_MESSAGE_SIGNATURE,
                    message_type: MessageType::Verify,
                    sequence_num: 7,
                    header_len: 80,
                    message_len: 92,
                    conversation_id: Guid::from_str("3b29075a-f391-af33-a1b4-a212249d7cb4").unwrap(),
                },
                auth_scheme: Guid::from_str("0d53335c-f9ea-4d0d-b2ec-4ae3786ec308").unwrap(),
                checksum: Checksum {
                    header_len: 20,
                    checksum_scheme: CHECKSUM_SCHEME_RFC3961,
                    checksum_type: HMAC_SHA1_96_AES256 as u32,
                    checksum_value: vec![228, 167, 112, 148, 23, 131, 204, 12, 13, 36, 58, 87],
                },
            },
            verify
        )
    }

    #[test]
    fn verify_encode() {
        let verify = Verify {
            header: MessageHeader {
                signature: NEGOEXTS_MESSAGE_SIGNATURE,
                message_type: MessageType::Verify,
                sequence_num: 9,
                header_len: 76,
                message_len: 88,
                conversation_id: Guid::from_str("3b29075a-f391-af33-a1b4-a212249d7cb4").unwrap(),
            },
            auth_scheme: Guid::from_str("0d53335c-f9ea-4d0d-b2ec-4ae3786ec308").unwrap(),
            checksum: Checksum {
                header_len: 20,
                checksum_scheme: CHECKSUM_SCHEME_RFC3961,
                checksum_type: HMAC_SHA1_96_AES256 as u32,
                checksum_value: vec![80, 14, 142, 6, 58, 29, 106, 165, 72, 160, 111, 12],
            },
        };

        let mut encoded = Vec::new();
        verify.encode(&mut encoded).unwrap();

        assert_eq!(
            &[
                78, 69, 71, 79, 69, 88, 84, 83, 6, 0, 0, 0, 9, 0, 0, 0, 76, 0, 0, 0, 88, 0, 0, 0, 90, 7, 41, 59, 145,
                243, 51, 175, 161, 180, 162, 18, 36, 157, 124, 180, 92, 51, 83, 13, 234, 249, 13, 77, 178, 236, 74,
                227, 120, 110, 195, 8, 20, 0, 0, 0, 1, 0, 0, 0, 16, 0, 0, 0, 76, 0, 0, 0, 12, 0, 0, 0, 0, 0, 0, 0, 80,
                14, 142, 6, 58, 29, 106, 165, 72, 160, 111, 12
            ],
            encoded.as_slice()
        );
    }
}
