use std::fmt::Debug;
use std::io::{self, Read, Write};

use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use picky_asn1::tag::Tag;
use picky_asn1::wrapper::{
    Asn1SequenceOf, BitStringAsn1, ExplicitContextTag0, ExplicitContextTag1, ExplicitContextTag2, ExplicitContextTag3,
    ObjectIdentifierAsn1, OctetStringAsn1, Optional,
};
use picky_asn1_der::{Asn1DerError, Asn1RawDer};
use serde::de::{self, DeserializeOwned, Error};
use serde::{Deserialize, Serialize, ser};
use thiserror::Error;

use crate::constants::gss_api::{MIC_FILLER, MIC_TOKEN_ID, WRAP_FILLER, WRAP_TOKEN_ID};

const MIC_TOKEN_INITIATOR_DEFAULT_FLAGS: u8 = 0x04;
const MIC_TOKEN_ACCEPTOR_DEFAULT_FLAGS: u8 = 0x05;
const WRAP_TOKEN_DEFAULT_FLAGS: u8 = 0x06;
const WRAP_HEADER_LEN: usize = 16;

#[derive(Debug, Error)]
pub enum GssApiMessageError {
    #[error("Invalid token id. Expected {0:?} but got {1:?}")]
    InvalidId([u8; 2], [u8; 2]),
    #[error("IO error: {0:?}")]
    IoError(#[from] io::Error),
    #[error("Invalid MIC token filler {0:?}")]
    InvalidMicFiller([u8; 5]),
    #[error("Invalid Wrap token filler {0:?}")]
    InvalidWrapFiller(u8),
    #[error("Asn1 error: {0:?}")]
    Asn1Error(#[from] Asn1DerError),
}

/// [3.1 GSS-API](https://datatracker.ietf.org/doc/html/rfc2478#section-3.1)
///
/// ```not_rust
/// MechType::= OBJECT IDENTIFIER
/// ```
pub type MechType = ObjectIdentifierAsn1;

/// [3.2.1.  GSS-API](https://datatracker.ietf.org/doc/html/rfc2478#section-3.2.1)
///
/// ```not_rust
/// MechTypeList ::= SEQUENCE OF MechType
/// ```
pub type MechTypeList = Asn1SequenceOf<MechType>;

/// [3.2.1.  GSS-API](https://datatracker.ietf.org/doc/html/rfc2478#section-3.2.1)
///
/// ```not_rust
/// NegTokenInit ::= SEQUENCE {
///     mechTypes       [0] MechTypeList,
///     reqFlags        [1] ContextFlags  OPTIONAL,
///     mechToken       [2] OCTET STRING  OPTIONAL,
///     mechListMIC     [3] OCTET STRING  OPTIONAL,
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NegTokenInit {
    #[serde(default)]
    pub mech_types: Optional<Option<ExplicitContextTag0<MechTypeList>>>,
    #[serde(default)]
    pub req_flags: Optional<Option<ExplicitContextTag1<BitStringAsn1>>>,
    #[serde(default)]
    pub mech_token: Optional<Option<ExplicitContextTag2<OctetStringAsn1>>>,
    #[serde(default)]
    pub mech_list_mic: Optional<Option<ExplicitContextTag3<OctetStringAsn1>>>,
}

/// [3.2.1. GSS-API](https://datatracker.ietf.org/doc/html/rfc2478#section-3.2.1)
///
/// ```not_rust
/// NegTokenTarg ::= SEQUENCE {
///     negResult      [0] ENUMERATED                              OPTIONAL,
///     supportedMech  [1] MechType                                OPTIONAL,
///     responseToken  [2] OCTET STRING                            OPTIONAL,
///     mechListMIC    [3] OCTET STRING                            OPTIONAL
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NegTokenTarg {
    #[serde(default)]
    pub neg_result: Optional<Option<ExplicitContextTag0<Asn1RawDer>>>,
    #[serde(default)]
    pub supported_mech: Optional<Option<ExplicitContextTag1<MechType>>>,
    #[serde(default)]
    pub response_token: Optional<Option<ExplicitContextTag2<OctetStringAsn1>>>,
    #[serde(default)]
    pub mech_list_mic: Optional<Option<ExplicitContextTag3<OctetStringAsn1>>>,
}

pub type NegTokenTarg1 = ExplicitContextTag1<NegTokenTarg>;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct KrbMessage<T> {
    pub krb5_oid: ObjectIdentifierAsn1,
    pub krb5_token_id: [u8; 2],
    pub krb_msg: T,
}

impl<T: Serialize> KrbMessage<T> {
    pub fn encode(&self, mut data: impl Write) -> Result<(), GssApiMessageError> {
        let mut oid = Vec::new();

        {
            let mut s = picky_asn1_der::Serializer::new_to_byte_buf(&mut oid);
            self.krb5_oid.serialize(&mut s)?;
        }

        data.write_all(&oid)?;
        data.write_all(&self.krb5_token_id)?;
        data.write_all(&picky_asn1_der::to_vec(&self.krb_msg)?)?;

        Ok(())
    }
}

impl<T: DeserializeOwned> KrbMessage<T> {
    /// Deserializes `ApplicationTag0<KrbMessage<T>>`.
    pub fn decode_application_krb_message(
        mut data: &[u8],
    ) -> Result<ApplicationTag0<KrbMessage<T>>, GssApiMessageError> {
        if data.is_empty() || Tag::from(data[0]) != Tag::application_constructed(0) {
            return Err(GssApiMessageError::Asn1Error(Asn1DerError::InvalidData));
        }

        // We cannot implement the deserialization using the `Deserialize` trait
        // because the krb5 token id is not an ASN1 field, but plain two-byte value.
        // We cannot read this id using our ASN1 deserializer.

        // This is a workaround we use to read the `ApplicationTag0` tag and length bytes.
        // At the same time it will also read the first field of the `KrbMessage`.
        #[derive(Deserialize)]
        struct Container {
            krb5_oid: ObjectIdentifierAsn1,
        }

        let max_len = data.len();
        let mut reader = &mut data;
        let Container { krb5_oid } =
            Container::deserialize(&mut picky_asn1_der::Deserializer::new_from_reader(&mut reader, max_len))?;

        let mut krb5_token_id = [0, 0];
        reader.read_exact(&mut krb5_token_id)?;

        let max_len = data.len();
        let mut reader = &mut data;
        let krb_msg: T = T::deserialize(&mut picky_asn1_der::Deserializer::new_from_reader(&mut reader, max_len))?;

        Ok(ApplicationTag0(KrbMessage {
            krb5_oid,
            krb5_token_id,
            krb_msg,
        }))
    }
}

impl<T: ser::Serialize> ser::Serialize for KrbMessage<T> {
    fn serialize<S>(&self, serializer: S) -> Result<<S as ser::Serializer>::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        use serde::ser::Error;

        // We encode `KrbMessage` fields using `KrbMessage::encode` method.
        // We use the `Container` type to prepend the sequence tag and length to the encoded fields.
        #[derive(Serialize)]
        struct Container {
            buff: Asn1RawDer,
        }

        let mut buff = Vec::new();
        self.encode(&mut buff)
            .map_err(|e| S::Error::custom(format!("cannot serialize KrbMessage inner value: {:?}", e)))?;

        Container { buff: Asn1RawDer(buff) }.serialize(serializer)
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GssApiNegInit {
    pub oid: ObjectIdentifierAsn1,
    pub neg_token_init: ExplicitContextTag0<NegTokenInit>,
}

/// This [ApplicationTag0] is different from the [ApplicationTag].
/// [ApplicationTag] works as a wrapper over the inner value
/// but [ApplicationTag0] decodes/encodes inner type fields as its own fields
///
/// **Note**: The corresponding ASN1 type of the inner type is expected to be a SEQUENCE.
/// In other words, the deserialization of something like `ApplicationTag0<BitString>` will fail
/// or produce invalid value.
/// This is because of the deserialization workaround we wrote to support weird Microsoft GSS API message.
#[derive(Debug, PartialEq, Eq)]
pub struct ApplicationTag0<T>(pub T);

impl<'de, T: de::DeserializeOwned> de::Deserialize<'de> for ApplicationTag0<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        let mut raw = Asn1RawDer::deserialize(deserializer)?.0;

        if let Some(first) = raw.first_mut() {
            // ASN1 sequence tag.
            *first = Tag::SEQUENCE.inner();
        }

        let mut deserializer = picky_asn1_der::Deserializer::new_from_bytes(&raw);

        T::deserialize(&mut deserializer).map(Self).map_err(|e| D::Error::custom(format!("{:?}", e)))
    }
}

impl<T: ser::Serialize + Debug + PartialEq> ser::Serialize for ApplicationTag0<T> {
    fn serialize<S>(&self, serializer: S) -> Result<<S as ser::Serializer>::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        use serde::ser::Error;

        let mut buff = Vec::new();
        {
            let mut s = picky_asn1_der::Serializer::new_to_byte_buf(&mut buff);
            self.0
                .serialize(&mut s)
                .map_err(|e| S::Error::custom(format!("cannot serialize GssApiMessage inner value: {:?}", e)))?;
        }

        buff[0] = Tag::application_constructed(0).inner();

        Asn1RawDer(buff).serialize(serializer)
    }
}

/// [MIC Tokens](https://datatracker.ietf.org/doc/html/rfc4121#section-4.2.6.1)
///
/// Octet no Name       Description
/// --------------------------------------------------------------
/// 0..1     TOK_ID     Identification field. Contains the hex value 04 04 expressed in big-endian order
/// 2        Flags      Attributes field
/// 3..7     Filler     Contains five octets of hex value FF.
/// 8..15    SND_SEQ    Sequence number expressed in big-endian order.
/// 16..last SGN_CKSUM  Checksum
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct MicToken {
    pub flags: u8,
    pub seq_num: u64,
    pub payload: Option<Vec<u8>>,
    pub checksum: Vec<u8>,
}

impl MicToken {
    pub fn with_initiator_flags() -> Self {
        Self {
            flags: MIC_TOKEN_INITIATOR_DEFAULT_FLAGS,
            seq_num: 0,
            payload: None,
            checksum: Vec::new(),
        }
    }

    pub fn with_acceptor_flags() -> Self {
        Self {
            flags: MIC_TOKEN_ACCEPTOR_DEFAULT_FLAGS,
            seq_num: 0,
            payload: None,
            checksum: Vec::new(),
        }
    }

    pub fn with_seq_number(self, seq_num: u64) -> Self {
        let MicToken {
            flags,
            payload,
            checksum,
            ..
        } = self;
        Self {
            flags,
            seq_num,
            payload,
            checksum,
        }
    }

    pub fn header(&self) -> [u8; 16] {
        let mut header_data = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

        header_data[0..2].copy_from_slice(&MIC_TOKEN_ID);
        header_data[2] = self.flags;
        header_data[3..8].copy_from_slice(&MIC_FILLER);
        header_data[8..].copy_from_slice(&self.seq_num.to_be_bytes());

        header_data
    }

    pub fn set_checksum(&mut self, checksum: Vec<u8>) {
        self.checksum = checksum;
    }

    pub fn set_payload(&mut self, payload: Vec<u8>) {
        self.payload = Some(payload);
    }

    pub fn encode(&self, mut data: impl Write) -> Result<(), GssApiMessageError> {
        data.write_all(&MIC_TOKEN_ID)?;
        data.write_u8(self.flags)?;
        data.write_all(&MIC_FILLER)?;
        data.write_u64::<BigEndian>(self.seq_num)?;
        data.write_all(&self.checksum)?;

        Ok(())
    }

    pub fn decode(mut data: impl Read) -> Result<Self, GssApiMessageError> {
        let mut mic_token_id = [0, 0];

        data.read_exact(&mut mic_token_id)?;
        if mic_token_id != MIC_TOKEN_ID {
            return Err(GssApiMessageError::InvalidId(MIC_TOKEN_ID, mic_token_id));
        }

        let flags = data.read_u8()?;

        let mut mic_fillter = [0, 0, 0, 0, 0];

        data.read_exact(&mut mic_fillter)?;
        if mic_fillter != MIC_FILLER {
            return Err(GssApiMessageError::InvalidMicFiller(mic_fillter));
        }

        let seq_num = data.read_u64::<BigEndian>()?;

        let mut checksum = Vec::with_capacity(12);
        data.read_to_end(&mut checksum)?;

        Ok(Self {
            flags,
            seq_num,
            checksum,
            payload: None,
        })
    }
}

/// [Wrap Tokens](https://datatracker.ietf.org/doc/html/rfc4121#section-4.2.6.2)
///
/// Octet no   Name        Description
/// --------------------------------------------------------------
///  0..1     TOK_ID    Identification field. Contain the hex value 05 04 expressed in big-endian
///  2        Flags     Attributes field
///  3        Filler    Contains the hex value FF.
///  4..5     EC        Contains the "extra count" field, in big-endian order
///  6..7     RRC       Contains the "right rotation count" in big-endian order
///  8..15    SND_SEQ   Sequence number field expressed in big-endian order.
///  16..last Data      Encrypted data for Wrap tokens
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct WrapToken {
    pub flags: u8,
    pub ec: u16,
    pub rrc: u16,
    pub seq_num: u64,
    pub payload: Option<Vec<u8>>,
    pub checksum: Vec<u8>,
}

impl WrapToken {
    pub fn with_seq_number(seq_num: u64) -> Self {
        Self {
            flags: WRAP_TOKEN_DEFAULT_FLAGS,
            ec: 0,
            rrc: 0,
            seq_num,
            payload: None,
            checksum: Vec::new(),
        }
    }

    pub fn header_len() -> usize {
        WRAP_HEADER_LEN
    }

    pub fn header(&self) -> [u8; 16] {
        let mut header_data = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

        header_data[0..2].copy_from_slice(&WRAP_TOKEN_ID);
        header_data[2] = self.flags;
        header_data[3] = WRAP_FILLER;
        header_data[4..6].copy_from_slice(&self.ec.to_be_bytes());
        header_data[6..8].copy_from_slice(&self.rrc.to_be_bytes());
        header_data[8..].copy_from_slice(&self.seq_num.to_be_bytes());

        header_data
    }

    pub fn set_rrc(&mut self, rrc: u16) {
        self.rrc = rrc;
    }

    pub fn set_checksum(&mut self, checksum: Vec<u8>) {
        self.checksum = checksum;
    }

    pub fn encode(&self, mut data: impl Write) -> Result<(), GssApiMessageError> {
        data.write_all(&WRAP_TOKEN_ID)?;
        data.write_u8(self.flags)?;
        data.write_u8(WRAP_FILLER)?;
        data.write_u16::<BigEndian>(self.ec)?;
        data.write_u16::<BigEndian>(self.rrc)?;
        data.write_u64::<BigEndian>(self.seq_num)?;
        data.write_all(&self.checksum)?;

        Ok(())
    }

    pub fn decode(mut data: impl Read) -> Result<Self, GssApiMessageError> {
        let mut wrap_token_id = [0, 0];

        data.read_exact(&mut wrap_token_id)?;
        if wrap_token_id != WRAP_TOKEN_ID {
            return Err(GssApiMessageError::InvalidId(WRAP_TOKEN_ID, wrap_token_id));
        }

        let flags = data.read_u8()?;

        let filler = data.read_u8()?;
        if filler != WRAP_FILLER {
            return Err(GssApiMessageError::InvalidWrapFiller(filler));
        }

        let ec = data.read_u16::<BigEndian>()?;
        let rrc = data.read_u16::<BigEndian>()?;
        let seq_num = data.read_u64::<BigEndian>()?;

        let mut checksum = Vec::with_capacity(12);
        data.read_to_end(&mut checksum)?;

        Ok(Self {
            flags,
            ec,
            rrc,
            seq_num,
            checksum,
            payload: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use picky_asn1::restricted_string::IA5String;
    use picky_asn1::wrapper::{
        Asn1SequenceOf, ExplicitContextTag0, ExplicitContextTag1, ExplicitContextTag2, ExplicitContextTag3,
        GeneralStringAsn1, IntegerAsn1, ObjectIdentifierAsn1, OctetStringAsn1, Optional,
    };
    use picky_asn1_x509::oids;

    use crate::constants::types::TGT_REP_MSG_TYPE;
    use crate::data_types::{EncryptedData, KerberosStringAsn1, PrincipalName, Ticket, TicketInner};
    use crate::gss_api::{ApplicationTag0, MicToken, WrapToken};
    use crate::messages::TgtRep;

    use super::KrbMessage;

    #[test]
    fn mic_token() {
        let expected_raw = vec![
            4, 4, 5, 255, 255, 255, 255, 255, 0, 0, 0, 0, 86, 90, 21, 229, 142, 95, 130, 211, 64, 247, 193, 232, 123,
            169, 124, 190,
        ];
        let expected = MicToken {
            flags: 5,
            seq_num: 1448744421,
            payload: None,
            checksum: vec![142, 95, 130, 211, 64, 247, 193, 232, 123, 169, 124, 190],
        };

        let mic_token = MicToken::decode(expected_raw.as_slice()).unwrap();
        let mut mic_token_raw = Vec::new();
        mic_token.encode(&mut mic_token_raw).unwrap();

        assert_eq!(expected, mic_token);
        assert_eq!(expected_raw, mic_token_raw);
    }

    #[test]
    fn wrap_token() {
        let expected_raw = vec![
            5, 4, 6, 255, 0, 0, 0, 28, 0, 0, 0, 0, 90, 181, 116, 98, 255, 212, 120, 29, 19, 35, 95, 91, 192, 216, 160,
            95, 135, 227, 86, 195, 248, 21, 226, 203, 98, 231, 109, 149, 168, 198, 63, 143, 64, 138, 30, 8, 241, 82,
            184, 48, 216, 142, 130, 64, 115, 237, 26, 204, 70, 175, 90, 166, 133, 159, 55, 132, 201, 214, 37, 21, 33,
            64, 239, 83, 135, 18, 103, 64, 219, 219, 16, 166, 251, 120, 195, 31, 57, 126, 188, 123,
        ];
        let expected = WrapToken {
            flags: 6,
            ec: 0,
            rrc: 28,
            seq_num: 1521841250,
            payload: None,
            checksum: vec![
                255, 212, 120, 29, 19, 35, 95, 91, 192, 216, 160, 95, 135, 227, 86, 195, 248, 21, 226, 203, 98, 231,
                109, 149, 168, 198, 63, 143, 64, 138, 30, 8, 241, 82, 184, 48, 216, 142, 130, 64, 115, 237, 26, 204,
                70, 175, 90, 166, 133, 159, 55, 132, 201, 214, 37, 21, 33, 64, 239, 83, 135, 18, 103, 64, 219, 219, 16,
                166, 251, 120, 195, 31, 57, 126, 188, 123,
            ],
        };

        let wrap_token = WrapToken::decode(expected_raw.as_slice()).unwrap();

        let mut wrap_token_raw = Vec::new();
        wrap_token.encode(&mut wrap_token_raw).unwrap();

        assert_eq!(expected, wrap_token);
        assert_eq!(expected_raw, wrap_token_raw);
    }

    #[test]
    fn application_krb_message() {
        let expected_raw = [
            96, 130, 4, 112, 6, 10, 42, 134, 72, 134, 247, 18, 1, 2, 2, 3, 4, 1, 48, 130, 4, 94, 160, 3, 2, 1, 5, 161,
            3, 2, 1, 17, 162, 130, 4, 80, 97, 130, 4, 76, 48, 130, 4, 72, 160, 3, 2, 1, 5, 161, 13, 27, 11, 69, 88, 65,
            77, 80, 76, 69, 46, 67, 79, 77, 162, 32, 48, 30, 160, 3, 2, 1, 2, 161, 23, 48, 21, 27, 6, 107, 114, 98,
            116, 103, 116, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 163, 130, 4, 14, 48, 130, 4, 10, 160, 3,
            2, 1, 18, 161, 3, 2, 1, 2, 162, 130, 3, 252, 4, 130, 3, 248, 58, 176, 96, 104, 148, 116, 168, 177, 48, 197,
            115, 31, 233, 217, 105, 81, 140, 38, 30, 245, 3, 239, 15, 203, 160, 156, 134, 234, 132, 191, 71, 202, 222,
            150, 103, 171, 92, 19, 221, 17, 179, 129, 3, 255, 79, 117, 96, 161, 111, 255, 62, 72, 85, 50, 133, 190,
            217, 238, 115, 108, 74, 181, 4, 183, 174, 6, 13, 39, 157, 21, 179, 161, 38, 53, 173, 32, 179, 38, 31, 111,
            235, 99, 4, 84, 73, 19, 131, 66, 70, 86, 143, 92, 176, 35, 222, 236, 86, 11, 218, 45, 67, 13, 75, 15, 70,
            146, 109, 32, 230, 18, 73, 31, 136, 51, 36, 247, 91, 216, 147, 63, 53, 232, 52, 147, 108, 77, 95, 95, 24,
            54, 56, 188, 50, 8, 28, 34, 173, 252, 124, 28, 83, 9, 186, 41, 94, 150, 73, 86, 24, 16, 54, 251, 57, 142,
            11, 121, 241, 69, 245, 149, 245, 214, 198, 37, 119, 142, 219, 194, 2, 206, 206, 180, 158, 68, 168, 249,
            236, 216, 49, 90, 165, 237, 232, 9, 189, 248, 231, 254, 121, 205, 205, 149, 131, 30, 46, 63, 48, 145, 68,
            63, 146, 137, 77, 32, 182, 218, 225, 188, 226, 238, 82, 141, 180, 86, 90, 239, 101, 222, 8, 77, 102, 96,
            102, 226, 45, 199, 31, 76, 163, 81, 169, 147, 168, 188, 112, 196, 135, 215, 159, 30, 74, 2, 133, 200, 145,
            150, 60, 245, 124, 79, 250, 118, 6, 38, 91, 229, 40, 13, 51, 193, 1, 179, 37, 238, 58, 50, 172, 54, 24, 60,
            250, 234, 13, 91, 77, 96, 143, 253, 1, 122, 141, 197, 143, 158, 38, 85, 60, 23, 149, 87, 27, 196, 153, 10,
            122, 157, 246, 83, 225, 198, 161, 171, 201, 103, 126, 19, 156, 75, 143, 207, 166, 28, 76, 14, 185, 85, 98,
            35, 103, 220, 152, 100, 20, 97, 187, 66, 107, 94, 56, 187, 77, 120, 82, 180, 244, 20, 129, 154, 251, 5, 99,
            161, 220, 10, 238, 61, 2, 110, 72, 195, 81, 11, 11, 111, 219, 134, 142, 50, 9, 46, 224, 15, 206, 87, 24,
            142, 157, 248, 107, 93, 133, 164, 75, 147, 111, 54, 154, 158, 157, 68, 158, 222, 20, 134, 249, 211, 36, 7,
            229, 92, 130, 220, 29, 19, 82, 247, 236, 224, 7, 157, 70, 97, 70, 109, 205, 46, 44, 229, 186, 69, 127, 117,
            201, 183, 151, 77, 25, 67, 38, 211, 184, 58, 7, 179, 234, 19, 37, 181, 63, 85, 12, 4, 8, 243, 248, 136,
            134, 197, 28, 106, 99, 155, 17, 66, 223, 116, 123, 19, 88, 230, 99, 235, 56, 55, 135, 89, 57, 58, 125, 70,
            67, 141, 106, 212, 9, 78, 0, 127, 213, 142, 8, 248, 78, 211, 241, 128, 127, 194, 240, 45, 253, 228, 210,
            176, 229, 156, 0, 102, 105, 43, 64, 206, 83, 78, 130, 210, 238, 174, 206, 231, 47, 68, 225, 72, 234, 240,
            90, 253, 246, 29, 173, 119, 117, 154, 253, 51, 14, 142, 112, 20, 86, 157, 15, 103, 44, 24, 83, 40, 38, 188,
            135, 202, 60, 246, 32, 50, 51, 43, 148, 161, 58, 3, 212, 105, 169, 247, 125, 48, 35, 227, 186, 71, 158,
            243, 198, 101, 9, 233, 169, 147, 66, 107, 65, 243, 211, 135, 236, 129, 116, 182, 77, 40, 32, 212, 28, 155,
            140, 239, 48, 222, 163, 87, 100, 10, 149, 54, 126, 112, 180, 208, 225, 42, 182, 254, 79, 97, 85, 231, 109,
            231, 111, 82, 56, 57, 34, 66, 23, 204, 83, 30, 187, 191, 9, 154, 29, 231, 12, 28, 62, 132, 221, 235, 106,
            80, 220, 171, 207, 75, 44, 148, 78, 209, 252, 49, 138, 163, 159, 191, 96, 168, 149, 186, 115, 105, 229, 98,
            181, 65, 191, 225, 46, 101, 235, 203, 204, 79, 168, 140, 216, 246, 73, 69, 104, 240, 239, 121, 227, 16,
            134, 69, 150, 254, 18, 254, 223, 26, 154, 82, 26, 83, 21, 91, 1, 151, 221, 205, 114, 70, 140, 229, 219,
            189, 100, 214, 255, 207, 91, 254, 74, 103, 199, 102, 170, 173, 137, 19, 47, 129, 151, 127, 144, 182, 202,
            116, 115, 58, 214, 123, 18, 185, 81, 132, 29, 229, 80, 131, 118, 45, 185, 22, 87, 173, 173, 207, 204, 135,
            13, 254, 244, 239, 28, 250, 233, 182, 140, 163, 234, 91, 25, 49, 182, 113, 182, 47, 213, 7, 203, 133, 227,
            243, 75, 14, 250, 154, 83, 60, 23, 241, 253, 33, 106, 233, 235, 119, 71, 175, 49, 226, 125, 226, 156, 227,
            132, 189, 29, 64, 151, 168, 39, 120, 199, 110, 233, 45, 132, 197, 250, 35, 67, 68, 139, 58, 245, 247, 74,
            241, 70, 170, 174, 15, 56, 13, 130, 18, 195, 137, 90, 153, 166, 17, 152, 62, 12, 55, 51, 140, 22, 45, 171,
            25, 172, 77, 14, 201, 160, 61, 56, 132, 216, 131, 93, 162, 132, 216, 186, 179, 60, 198, 247, 229, 249, 201,
            43, 212, 227, 116, 29, 129, 9, 75, 99, 63, 218, 213, 214, 179, 204, 14, 48, 192, 232, 54, 197, 5, 235, 18,
            106, 129, 85, 100, 2, 78, 213, 83, 255, 114, 85, 78, 250, 11, 235, 182, 221, 242, 255, 252, 51, 93, 254,
            168, 35, 161, 111, 198, 77, 141, 118, 197, 155, 129, 191, 215, 193, 81, 47, 99, 1, 124, 120, 46, 148, 51,
            133, 160, 21, 187, 196, 236, 59, 175, 138, 166, 247, 162, 168, 48, 122, 100, 146, 154, 251, 27, 131, 8,
            249, 171, 237, 122, 212, 52, 195, 226, 75, 60, 248, 52, 124, 143, 121, 206, 69, 7, 24, 22, 16, 232, 178,
            254, 197, 31, 132, 98, 71, 22, 217, 145, 34, 214, 214, 189, 164, 171, 200, 232, 234, 237, 99, 76, 216, 35,
            137, 123, 207, 77, 59, 180, 170, 209, 93, 137, 89, 62, 192, 201, 20, 61, 102, 10, 255, 160, 11, 27, 254,
            213, 14, 2,
        ];
        let expected = ApplicationTag0(KrbMessage {
            krb5_oid: ObjectIdentifierAsn1::from(oids::krb5_user_to_user()),
            // TGT rep
            krb5_token_id: [0x04, 0x01],
            krb_msg: TgtRep {
                pvno: ExplicitContextTag0::from(IntegerAsn1::from(vec![5])),
                msg_type: ExplicitContextTag1::from(IntegerAsn1::from(vec![TGT_REP_MSG_TYPE])),
                ticket: ExplicitContextTag2::from(Ticket::from(TicketInner {
                    tkt_vno: ExplicitContextTag0::from(IntegerAsn1(vec![5])),
                    realm: ExplicitContextTag1::from(GeneralStringAsn1::from(
                        IA5String::from_string("EXAMPLE.COM".to_owned()).unwrap(),
                    )),
                    sname: ExplicitContextTag2::from(PrincipalName {
                        name_type: ExplicitContextTag0::from(IntegerAsn1(vec![2])),
                        name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![
                            KerberosStringAsn1::from(IA5String::from_string("krbtgt".to_owned()).unwrap()),
                            KerberosStringAsn1::from(IA5String::from_string("EXAMPLE.COM".to_owned()).unwrap()),
                        ])),
                    }),
                    enc_part: ExplicitContextTag3::from(EncryptedData {
                        etype: ExplicitContextTag0::from(IntegerAsn1(vec![18])),
                        kvno: Optional::from(Some(ExplicitContextTag1::from(IntegerAsn1(vec![2])))),
                        cipher: ExplicitContextTag2::from(OctetStringAsn1::from(vec![
                            58, 176, 96, 104, 148, 116, 168, 177, 48, 197, 115, 31, 233, 217, 105, 81, 140, 38, 30,
                            245, 3, 239, 15, 203, 160, 156, 134, 234, 132, 191, 71, 202, 222, 150, 103, 171, 92, 19,
                            221, 17, 179, 129, 3, 255, 79, 117, 96, 161, 111, 255, 62, 72, 85, 50, 133, 190, 217, 238,
                            115, 108, 74, 181, 4, 183, 174, 6, 13, 39, 157, 21, 179, 161, 38, 53, 173, 32, 179, 38, 31,
                            111, 235, 99, 4, 84, 73, 19, 131, 66, 70, 86, 143, 92, 176, 35, 222, 236, 86, 11, 218, 45,
                            67, 13, 75, 15, 70, 146, 109, 32, 230, 18, 73, 31, 136, 51, 36, 247, 91, 216, 147, 63, 53,
                            232, 52, 147, 108, 77, 95, 95, 24, 54, 56, 188, 50, 8, 28, 34, 173, 252, 124, 28, 83, 9,
                            186, 41, 94, 150, 73, 86, 24, 16, 54, 251, 57, 142, 11, 121, 241, 69, 245, 149, 245, 214,
                            198, 37, 119, 142, 219, 194, 2, 206, 206, 180, 158, 68, 168, 249, 236, 216, 49, 90, 165,
                            237, 232, 9, 189, 248, 231, 254, 121, 205, 205, 149, 131, 30, 46, 63, 48, 145, 68, 63, 146,
                            137, 77, 32, 182, 218, 225, 188, 226, 238, 82, 141, 180, 86, 90, 239, 101, 222, 8, 77, 102,
                            96, 102, 226, 45, 199, 31, 76, 163, 81, 169, 147, 168, 188, 112, 196, 135, 215, 159, 30,
                            74, 2, 133, 200, 145, 150, 60, 245, 124, 79, 250, 118, 6, 38, 91, 229, 40, 13, 51, 193, 1,
                            179, 37, 238, 58, 50, 172, 54, 24, 60, 250, 234, 13, 91, 77, 96, 143, 253, 1, 122, 141,
                            197, 143, 158, 38, 85, 60, 23, 149, 87, 27, 196, 153, 10, 122, 157, 246, 83, 225, 198, 161,
                            171, 201, 103, 126, 19, 156, 75, 143, 207, 166, 28, 76, 14, 185, 85, 98, 35, 103, 220, 152,
                            100, 20, 97, 187, 66, 107, 94, 56, 187, 77, 120, 82, 180, 244, 20, 129, 154, 251, 5, 99,
                            161, 220, 10, 238, 61, 2, 110, 72, 195, 81, 11, 11, 111, 219, 134, 142, 50, 9, 46, 224, 15,
                            206, 87, 24, 142, 157, 248, 107, 93, 133, 164, 75, 147, 111, 54, 154, 158, 157, 68, 158,
                            222, 20, 134, 249, 211, 36, 7, 229, 92, 130, 220, 29, 19, 82, 247, 236, 224, 7, 157, 70,
                            97, 70, 109, 205, 46, 44, 229, 186, 69, 127, 117, 201, 183, 151, 77, 25, 67, 38, 211, 184,
                            58, 7, 179, 234, 19, 37, 181, 63, 85, 12, 4, 8, 243, 248, 136, 134, 197, 28, 106, 99, 155,
                            17, 66, 223, 116, 123, 19, 88, 230, 99, 235, 56, 55, 135, 89, 57, 58, 125, 70, 67, 141,
                            106, 212, 9, 78, 0, 127, 213, 142, 8, 248, 78, 211, 241, 128, 127, 194, 240, 45, 253, 228,
                            210, 176, 229, 156, 0, 102, 105, 43, 64, 206, 83, 78, 130, 210, 238, 174, 206, 231, 47, 68,
                            225, 72, 234, 240, 90, 253, 246, 29, 173, 119, 117, 154, 253, 51, 14, 142, 112, 20, 86,
                            157, 15, 103, 44, 24, 83, 40, 38, 188, 135, 202, 60, 246, 32, 50, 51, 43, 148, 161, 58, 3,
                            212, 105, 169, 247, 125, 48, 35, 227, 186, 71, 158, 243, 198, 101, 9, 233, 169, 147, 66,
                            107, 65, 243, 211, 135, 236, 129, 116, 182, 77, 40, 32, 212, 28, 155, 140, 239, 48, 222,
                            163, 87, 100, 10, 149, 54, 126, 112, 180, 208, 225, 42, 182, 254, 79, 97, 85, 231, 109,
                            231, 111, 82, 56, 57, 34, 66, 23, 204, 83, 30, 187, 191, 9, 154, 29, 231, 12, 28, 62, 132,
                            221, 235, 106, 80, 220, 171, 207, 75, 44, 148, 78, 209, 252, 49, 138, 163, 159, 191, 96,
                            168, 149, 186, 115, 105, 229, 98, 181, 65, 191, 225, 46, 101, 235, 203, 204, 79, 168, 140,
                            216, 246, 73, 69, 104, 240, 239, 121, 227, 16, 134, 69, 150, 254, 18, 254, 223, 26, 154,
                            82, 26, 83, 21, 91, 1, 151, 221, 205, 114, 70, 140, 229, 219, 189, 100, 214, 255, 207, 91,
                            254, 74, 103, 199, 102, 170, 173, 137, 19, 47, 129, 151, 127, 144, 182, 202, 116, 115, 58,
                            214, 123, 18, 185, 81, 132, 29, 229, 80, 131, 118, 45, 185, 22, 87, 173, 173, 207, 204,
                            135, 13, 254, 244, 239, 28, 250, 233, 182, 140, 163, 234, 91, 25, 49, 182, 113, 182, 47,
                            213, 7, 203, 133, 227, 243, 75, 14, 250, 154, 83, 60, 23, 241, 253, 33, 106, 233, 235, 119,
                            71, 175, 49, 226, 125, 226, 156, 227, 132, 189, 29, 64, 151, 168, 39, 120, 199, 110, 233,
                            45, 132, 197, 250, 35, 67, 68, 139, 58, 245, 247, 74, 241, 70, 170, 174, 15, 56, 13, 130,
                            18, 195, 137, 90, 153, 166, 17, 152, 62, 12, 55, 51, 140, 22, 45, 171, 25, 172, 77, 14,
                            201, 160, 61, 56, 132, 216, 131, 93, 162, 132, 216, 186, 179, 60, 198, 247, 229, 249, 201,
                            43, 212, 227, 116, 29, 129, 9, 75, 99, 63, 218, 213, 214, 179, 204, 14, 48, 192, 232, 54,
                            197, 5, 235, 18, 106, 129, 85, 100, 2, 78, 213, 83, 255, 114, 85, 78, 250, 11, 235, 182,
                            221, 242, 255, 252, 51, 93, 254, 168, 35, 161, 111, 198, 77, 141, 118, 197, 155, 129, 191,
                            215, 193, 81, 47, 99, 1, 124, 120, 46, 148, 51, 133, 160, 21, 187, 196, 236, 59, 175, 138,
                            166, 247, 162, 168, 48, 122, 100, 146, 154, 251, 27, 131, 8, 249, 171, 237, 122, 212, 52,
                            195, 226, 75, 60, 248, 52, 124, 143, 121, 206, 69, 7, 24, 22, 16, 232, 178, 254, 197, 31,
                            132, 98, 71, 22, 217, 145, 34, 214, 214, 189, 164, 171, 200, 232, 234, 237, 99, 76, 216,
                            35, 137, 123, 207, 77, 59, 180, 170, 209, 93, 137, 89, 62, 192, 201, 20, 61, 102, 10, 255,
                            160, 11, 27, 254, 213, 14, 2,
                        ])),
                    }),
                })),
            },
        });

        let krb_message = KrbMessage::<TgtRep>::decode_application_krb_message(expected_raw.as_slice()).unwrap();
        let krb_message_raw = picky_asn1_der::to_vec(&expected).unwrap();

        assert_eq!(krb_message, expected);
        assert_eq!(krb_message_raw, expected_raw);
    }
}
