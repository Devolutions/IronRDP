use picky_asn1::tag::{TagClass, TagPeeker};
use picky_asn1::wrapper::{
    Asn1SequenceOf, BitStringAsn1, ExplicitContextTag0, ExplicitContextTag1, ExplicitContextTag2, ExplicitContextTag3,
    ImplicitContextTag0, ImplicitContextTag1, ImplicitContextTag2, IntegerAsn1, ObjectIdentifierAsn1, OctetStringAsn1,
    Optional,
};
use picky_asn1_x509::AlgorithmIdentifier;
use serde::{Deserialize, Serialize, de, ser};

use crate::data_types::{Checksum, KerberosTime, PrincipalName, Realm};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Pku2uNegoReqMetadata {
    pub inner: ImplicitContextTag0<OctetStringAsn1>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Pku2uNegoBody {
    pub realm: ExplicitContextTag0<Realm>,
    pub sname: ExplicitContextTag1<PrincipalName>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Pku2uNegoReq {
    pub metadata: ExplicitContextTag0<Asn1SequenceOf<Pku2uNegoReqMetadata>>,
    pub body: ExplicitContextTag1<Pku2uNegoBody>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Pku2uNegoRep {
    pub metadata: ExplicitContextTag0<Asn1SequenceOf<Pku2uNegoReqMetadata>>,
}

/// [Generation of Client Request](https://www.rfc-editor.org/rfc/rfc4556.html#section-3.2.1)
/// ```not_rust
/// ExternalPrincipalIdentifier ::= SEQUENCE {
///    subjectName             [0] IMPLICIT OCTET STRING OPTIONAL,
///    issuerAndSerialNumber   [1] IMPLICIT OCTET STRING OPTIONAL,
///    subjectKeyIdentifier    [2] IMPLICIT OCTET STRING OPTIONAL,
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct ExternalPrincipalIdentifier {
    #[serde(default)]
    pub subject_name: Optional<Option<ImplicitContextTag0<OctetStringAsn1>>>,
    #[serde(default)]
    pub issuer_and_serial_number: Optional<Option<ImplicitContextTag1<OctetStringAsn1>>>,
    #[serde(default)]
    pub subject_key_identifier: Optional<Option<ImplicitContextTag2<OctetStringAsn1>>>,
}

/// [Generation of Client Request](https://www.rfc-editor.org/rfc/rfc4556.html#section-3.2.1)
/// ```not_rust
/// PA-PK-AS-REQ ::= SEQUENCE {
///    signedAuthPack          [0] IMPLICIT OCTET STRING,
///    trustedCertifiers       [1] SEQUENCE OF ExternalPrincipalIdentifier OPTIONAL,
///    kdcPkId                 [2] IMPLICIT OCTET STRING OPTIONAL,
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct PaPkAsReq {
    pub signed_auth_pack: ImplicitContextTag0<OctetStringAsn1>,
    #[serde(default)]
    pub trusted_certifiers: Optional<Option<ExplicitContextTag1<Asn1SequenceOf<ExternalPrincipalIdentifier>>>>,
    #[serde(default)]
    pub kdc_pk_id: Optional<Option<ImplicitContextTag2<OctetStringAsn1>>>,
}

/// [Generation of Client Request](https://www.rfc-editor.org/rfc/rfc4556.html#section-3.2.1)
/// ```not_rust
/// PKAuthenticator ::= SEQUENCE {
///    cusec                   [0] INTEGER (0..999999),
///    ctime                   [1] KerberosTime,
///    nonce                   [2] INTEGER (0..4294967295),
///    paChecksum              [3] OCTET STRING OPTIONAL,
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct PkAuthenticator {
    pub cusec: ExplicitContextTag0<IntegerAsn1>,
    pub ctime: ExplicitContextTag1<KerberosTime>,
    pub nonce: ExplicitContextTag2<IntegerAsn1>,
    #[serde(default)]
    pub pa_checksum: Optional<Option<ExplicitContextTag3<OctetStringAsn1>>>,
}

/// [Generation of Client Request](https://www.rfc-editor.org/rfc/rfc4556.html#section-3.2.1)
/// ```not_rust
/// DHNonce ::= OCTET STRING
/// ```
pub type DhNonce = OctetStringAsn1;

/// [Diffie-Hellman Key Exchange Keys](https://www.rfc-editor.org/rfc/rfc3279#section-2.3.3)
/// ```not_rust
/// ValidationParms ::= SEQUENCE {
///       seed             BIT STRING,
///       pgenCounter      INTEGER }
/// ```
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct ValidationParams {
    seed: BitStringAsn1,
    pg_gen_counter: IntegerAsn1,
}

/// [Diffie-Hellman Key Exchange Keys](https://www.rfc-editor.org/rfc/rfc3279#section-2.3.3)
/// ```not_rust
/// DomainParameters ::= SEQUENCE {
///       p       INTEGER, -- odd prime, p = jq +1
///       g       INTEGER, -- generator, g
///       q       INTEGER, -- factor of p - 1
///       j       INTEGER OPTIONAL, -- subgroup factor
///       validationParms  ValidationParms OPTIONAL }
/// ```
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct DhDomainParameters {
    pub p: IntegerAsn1,
    pub g: IntegerAsn1,
    pub q: IntegerAsn1,
    #[serde(default)]
    pub j: Optional<Option<IntegerAsn1>>,
    #[serde(default)]
    pub validation_params: Optional<Option<ValidationParams>>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct DhReqKeyInfo {
    pub identifier: ObjectIdentifierAsn1,
    pub key_info: DhDomainParameters,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct DhReqInfo {
    pub key_info: DhReqKeyInfo,
    pub key_value: BitStringAsn1,
}

/// [Generation of Client Request](https://www.rfc-editor.org/rfc/rfc4556.html#section-3.2.1)
/// ```not_rust
/// AuthPack ::= SEQUENCE {
///    pkAuthenticator         [0] PKAuthenticator,
///    clientPublicValue       [1] SubjectPublicKeyInfo OPTIONAL,
///    supportedCMSTypes       [2] SEQUENCE OF AlgorithmIdentifier OPTIONAL,
///    clientDHNonce           [3] DHNonce OPTIONAL,
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct AuthPack {
    pub pk_authenticator: ExplicitContextTag0<PkAuthenticator>,
    #[serde(default)]
    pub client_public_value: Optional<Option<ExplicitContextTag1<DhReqInfo>>>,
    #[serde(default)]
    pub supported_cms_types: Optional<Option<ExplicitContextTag2<Asn1SequenceOf<AlgorithmIdentifier>>>>,
    #[serde(default)]
    pub client_dh_nonce: Optional<Option<ExplicitContextTag3<DhNonce>>>,
}

/// [Generation of KDC Reply](https://www.rfc-editor.org/rfc/rfc4556.html#section-3.2.3)
/// ```not_rust
/// DHRepInfo ::= SEQUENCE {
///    dhSignedData            [0] IMPLICIT OCTET STRING,
///    serverDHNonce           [1] DHNonce OPTIONAL,
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct DhRepInfo {
    pub dh_signed_data: ImplicitContextTag0<OctetStringAsn1>,
    #[serde(default)]
    pub server_dh_nonce: Optional<Option<ExplicitContextTag1<DhNonce>>>,
}

/// [Generation of KDC Reply](https://www.rfc-editor.org/rfc/rfc4556.html#section-3.2.3)
/// ```not_rust
/// PA-PK-AS-REP ::= CHOICE {
///    dhInfo                  [0] DHRepInfo,
///    encKeyPack              [1] IMPLICIT OCTET STRING,
/// }
/// ```
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum PaPkAsRep {
    DhInfo(ExplicitContextTag0<DhRepInfo>),
    EncKeyPack(ImplicitContextTag1<OctetStringAsn1>),
}

impl<'de> Deserialize<'de> for PaPkAsRep {
    fn deserialize<D>(deserializer: D) -> Result<Self, <D as de::Deserializer<'de>>::Error>
    where
        D: de::Deserializer<'de>,
    {
        use std::fmt;

        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = PaPkAsRep;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a valid DER-encoded SpcLink")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let tag_peeker: TagPeeker = seq.next_element()?.ok_or_else(|| {
                    de::Error::invalid_value(
                        de::Unexpected::Other("[PaPkAsRep] choice tag is missing"),
                        &"valid choice tag",
                    )
                })?;

                let pa_pk_as_rep = match tag_peeker.next_tag.class_and_number() {
                    (TagClass::ContextSpecific, 0) => PaPkAsRep::DhInfo(seq.next_element()?.ok_or_else(|| {
                        de::Error::invalid_value(
                            de::Unexpected::Other("[PaPkAsRep] dhInfo is missing"),
                            &"valid dhInfo",
                        )
                    })?),
                    (TagClass::ContextSpecific, 1) => PaPkAsRep::EncKeyPack(seq.next_element()?.ok_or_else(|| {
                        de::Error::invalid_value(
                            de::Unexpected::Other("[PaPkAsRep] encKeyPack is missing"),
                            &"valid encKeyPack",
                        )
                    })?),
                    _ => {
                        return Err(de::Error::invalid_value(
                            de::Unexpected::Other("[PaPkAsRep] unknown choice value"),
                            &"a supported PA-PK-AS-REP choice",
                        ));
                    }
                };

                Ok(pa_pk_as_rep)
            }
        }

        deserializer.deserialize_enum("PA-PK-AS-REP", &["dhInfo", "encKeyPack"], Visitor)
    }
}

impl ser::Serialize for PaPkAsRep {
    fn serialize<S>(&self, serializer: S) -> Result<<S as ser::Serializer>::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        match self {
            PaPkAsRep::DhInfo(dh_info) => dh_info.serialize(serializer),
            PaPkAsRep::EncKeyPack(enc_key_pack) => enc_key_pack.serialize(serializer),
        }
    }
}

/// [Generation of KDC Reply](https://www.rfc-editor.org/rfc/rfc4556.html#section-3.2.3)
/// ```not_rust
/// KDCDHKeyInfo ::= SEQUENCE {
///    subjectPublicKey        [0] BIT STRING,
///    nonce                   [1] INTEGER (0..4294967295),
///    dhKeyExpiration         [2] KerberosTime OPTIONAL,
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct KdcDhKeyInfo {
    pub subject_public_key: ExplicitContextTag0<BitStringAsn1>,
    pub nonce: ExplicitContextTag1<IntegerAsn1>,
    #[serde(default)]
    pub dh_key_expiration: Optional<Option<ExplicitContextTag2<KerberosTime>>>,
}

/// [The GSS-API Binding for PKU2U](https://datatracker.ietf.org/doc/html/draft-zhu-pku2u-04#section-6)
/// ```not_rust
/// KRB-FINISHED ::= SEQUENCE {
///      gss-mic [1] Checksum,
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct KrbFinished {
    pub gss_mic: ExplicitContextTag1<Checksum>,
}

#[cfg(test)]
mod tests {
    use picky_asn1::restricted_string::Ia5String;
    use picky_asn1::wrapper::{
        Asn1SequenceOf, ExplicitContextTag0, ExplicitContextTag1, ImplicitContextTag0, ImplicitContextTag1,
        IntegerAsn1, OctetStringAsn1, Optional,
    };

    use crate::data_types::{KerberosStringAsn1, PrincipalName};
    use crate::pkinit::DhRepInfo;

    use super::{PaPkAsRep, PaPkAsReq, Pku2uNegoBody, Pku2uNegoRep, Pku2uNegoReq, Pku2uNegoReqMetadata};

    #[test]
    fn pku2u_nego_req_encode() {
        let message = Pku2uNegoReq {
            metadata: ExplicitContextTag0::from(Asn1SequenceOf::from(vec![
                Pku2uNegoReqMetadata {
                    inner: ImplicitContextTag0::from(OctetStringAsn1::from(vec![
                        48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0,
                        97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80,
                        0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49,
                        0, 93,
                    ])),
                },
                Pku2uNegoReqMetadata {
                    inner: ImplicitContextTag0::from(OctetStringAsn1::from(vec![
                        48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0,
                        97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80,
                        0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49,
                        0, 93,
                    ])),
                },
            ])),
            body: ExplicitContextTag1::from(Pku2uNegoBody {
                realm: ExplicitContextTag0::from(KerberosStringAsn1::from(
                    Ia5String::from_string("WELLKNOWN:PKU2U".into()).unwrap(),
                )),
                sname: ExplicitContextTag1::from(PrincipalName {
                    name_type: ExplicitContextTag0::from(IntegerAsn1::from(vec![2])),
                    name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![
                        KerberosStringAsn1::from(Ia5String::from_string("TERMSRV".into()).unwrap()),
                        KerberosStringAsn1::from(Ia5String::from_string("AZRDOWN-W10".into()).unwrap()),
                    ])),
                }),
            }),
        };

        let encoded = picky_asn1_der::to_vec(&message).unwrap();

        assert_eq!(
            &[
                48, 129, 230, 160, 129, 169, 48, 129, 166, 48, 81, 128, 79, 48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30,
                66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116, 0, 105,
                0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32,
                0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 48, 81, 128, 79, 48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30,
                66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116, 0, 105,
                0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32,
                0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 161, 56, 48, 54, 160, 17, 27, 15, 87, 69, 76, 76, 75, 78, 79,
                87, 78, 58, 80, 75, 85, 50, 85, 161, 33, 48, 31, 160, 3, 2, 1, 2, 161, 24, 48, 22, 27, 7, 84, 69, 82,
                77, 83, 82, 86, 27, 11, 65, 90, 82, 68, 79, 87, 78, 45, 87, 49, 48
            ],
            encoded.as_slice()
        );
    }

    #[test]
    fn pku2u_nego_rep_encode() {
        let nego_rep = Pku2uNegoRep {
            metadata: ExplicitContextTag0::from(Asn1SequenceOf::from(vec![
                Pku2uNegoReqMetadata {
                    inner: ImplicitContextTag0::from(OctetStringAsn1::from(vec![
                        48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0,
                        97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80,
                        0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49,
                        0, 93,
                    ])),
                },
                Pku2uNegoReqMetadata {
                    inner: ImplicitContextTag0::from(OctetStringAsn1::from(vec![
                        48, 35, 49, 33, 48, 31, 6, 3, 85, 4, 3, 19, 24, 84, 111, 107, 101, 110, 32, 83, 105, 103, 110,
                        105, 110, 103, 32, 80, 117, 98, 108, 105, 99, 32, 75, 101, 121,
                    ])),
                },
            ])),
        };

        let encoded = picky_asn1_der::to_vec(&nego_rep).unwrap();

        assert_eq!(
            &[
                48, 129, 128, 160, 126, 48, 124, 48, 81, 128, 79, 48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30, 66, 0,
                77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116, 0, 105, 0, 111,
                0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32, 0, 91,
                0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 48, 39, 128, 37, 48, 35, 49, 33, 48, 31, 6, 3, 85, 4, 3, 19, 24, 84,
                111, 107, 101, 110, 32, 83, 105, 103, 110, 105, 110, 103, 32, 80, 117, 98, 108, 105, 99, 32, 75, 101,
                121
            ],
            encoded.as_slice()
        );
    }

    #[test]
    fn pku2u_nego_rep_decode() {
        let raw_data = [
            48, 129, 171, 160, 129, 168, 48, 129, 165, 48, 81, 128, 79, 48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30, 66,
            0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116, 0, 105, 0, 111,
            0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32, 0, 91, 0,
            50, 0, 48, 0, 50, 0, 49, 0, 93, 48, 39, 128, 37, 48, 35, 49, 33, 48, 31, 6, 3, 85, 4, 3, 19, 24, 84, 111,
            107, 101, 110, 32, 83, 105, 103, 110, 105, 110, 103, 32, 80, 117, 98, 108, 105, 99, 32, 75, 101, 121, 48,
            39, 128, 37, 48, 35, 49, 33, 48, 31, 6, 3, 85, 4, 3, 19, 24, 84, 111, 107, 101, 110, 32, 83, 105, 103, 110,
            105, 110, 103, 32, 80, 117, 98, 108, 105, 99, 32, 75, 101, 121,
        ];

        let pku2u_nego_rep: Pku2uNegoRep = picky_asn1_der::from_bytes(&raw_data).unwrap();

        assert_eq!(
            Pku2uNegoRep {
                metadata: ExplicitContextTag0::from(Asn1SequenceOf::from(vec![
                    Pku2uNegoReqMetadata {
                        inner: ImplicitContextTag0::from(OctetStringAsn1::from(vec![
                            48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103,
                            0, 97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50,
                            0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32, 0, 91, 0, 50, 0, 48, 0,
                            50, 0, 49, 0, 93
                        ]))
                    },
                    Pku2uNegoReqMetadata {
                        inner: ImplicitContextTag0::from(OctetStringAsn1::from(vec![
                            48, 35, 49, 33, 48, 31, 6, 3, 85, 4, 3, 19, 24, 84, 111, 107, 101, 110, 32, 83, 105, 103,
                            110, 105, 110, 103, 32, 80, 117, 98, 108, 105, 99, 32, 75, 101, 121
                        ]))
                    },
                    Pku2uNegoReqMetadata {
                        inner: ImplicitContextTag0::from(OctetStringAsn1::from(vec![
                            48, 35, 49, 33, 48, 31, 6, 3, 85, 4, 3, 19, 24, 84, 111, 107, 101, 110, 32, 83, 105, 103,
                            110, 105, 110, 103, 32, 80, 117, 98, 108, 105, 99, 32, 75, 101, 121
                        ]))
                    },
                ]))
            },
            pku2u_nego_rep
        );
    }

    #[test]
    fn pa_pk_as_req_decode() {
        let raw_data = [
            48, 130, 7, 246, 128, 130, 7, 242, 48, 130, 7, 238, 2, 1, 3, 49, 11, 48, 9, 6, 5, 43, 14, 3, 2, 26, 5, 0,
            48, 130, 2, 31, 6, 7, 43, 6, 1, 5, 2, 3, 1, 160, 130, 2, 18, 4, 130, 2, 14, 48, 130, 2, 10, 160, 57, 48,
            55, 160, 5, 2, 3, 6, 219, 251, 161, 17, 24, 15, 50, 48, 50, 50, 48, 53, 49, 55, 50, 48, 50, 53, 53, 57, 90,
            162, 3, 2, 1, 0, 163, 22, 4, 20, 197, 143, 17, 64, 61, 203, 186, 45, 19, 30, 175, 125, 106, 6, 209, 5, 36,
            69, 144, 97, 161, 130, 1, 167, 48, 130, 1, 163, 48, 130, 1, 23, 6, 7, 42, 134, 72, 206, 62, 2, 1, 48, 130,
            1, 10, 2, 129, 129, 0, 255, 255, 255, 255, 255, 255, 255, 255, 201, 15, 218, 162, 33, 104, 194, 52, 196,
            198, 98, 139, 128, 220, 28, 209, 41, 2, 78, 8, 138, 103, 204, 116, 2, 11, 190, 166, 59, 19, 155, 34, 81,
            74, 8, 121, 142, 52, 4, 221, 239, 149, 25, 179, 205, 58, 67, 27, 48, 43, 10, 109, 242, 95, 20, 55, 79, 225,
            53, 109, 109, 81, 194, 69, 228, 133, 181, 118, 98, 94, 126, 198, 244, 76, 66, 233, 166, 55, 237, 107, 11,
            255, 92, 182, 244, 6, 183, 237, 238, 56, 107, 251, 90, 137, 159, 165, 174, 159, 36, 17, 124, 75, 31, 230,
            73, 40, 102, 81, 236, 230, 83, 129, 255, 255, 255, 255, 255, 255, 255, 255, 2, 1, 2, 2, 129, 128, 127, 255,
            255, 255, 255, 255, 255, 255, 228, 135, 237, 81, 16, 180, 97, 26, 98, 99, 49, 69, 192, 110, 14, 104, 148,
            129, 39, 4, 69, 51, 230, 58, 1, 5, 223, 83, 29, 137, 205, 145, 40, 165, 4, 60, 199, 26, 2, 110, 247, 202,
            140, 217, 230, 157, 33, 141, 152, 21, 133, 54, 249, 47, 138, 27, 167, 240, 154, 182, 182, 168, 225, 34,
            242, 66, 218, 187, 49, 47, 63, 99, 122, 38, 33, 116, 211, 27, 246, 181, 133, 255, 174, 91, 122, 3, 91, 246,
            247, 28, 53, 253, 173, 68, 207, 210, 215, 79, 146, 8, 190, 37, 143, 243, 36, 148, 51, 40, 246, 115, 41,
            192, 255, 255, 255, 255, 255, 255, 255, 255, 3, 129, 133, 0, 2, 129, 129, 0, 219, 78, 185, 183, 129, 7, 4,
            73, 79, 203, 237, 216, 60, 162, 113, 232, 36, 233, 162, 75, 8, 200, 168, 109, 49, 32, 207, 86, 26, 198,
            121, 143, 205, 90, 248, 169, 6, 178, 153, 1, 237, 156, 2, 145, 162, 150, 218, 232, 144, 183, 193, 58, 7,
            27, 217, 215, 160, 30, 69, 15, 211, 28, 18, 216, 145, 196, 14, 47, 119, 76, 163, 178, 243, 136, 213, 190,
            122, 108, 59, 140, 94, 32, 75, 114, 17, 239, 99, 81, 208, 221, 232, 214, 193, 129, 129, 135, 191, 117, 72,
            254, 44, 211, 92, 124, 203, 235, 196, 113, 1, 123, 74, 139, 101, 121, 212, 210, 119, 162, 26, 230, 153,
            254, 123, 68, 151, 135, 52, 29, 163, 34, 4, 32, 72, 91, 60, 222, 24, 28, 4, 155, 141, 138, 44, 10, 136, 54,
            202, 60, 146, 234, 183, 130, 109, 34, 94, 10, 87, 237, 162, 55, 173, 100, 115, 43, 160, 130, 3, 236, 48,
            130, 3, 232, 48, 130, 2, 208, 160, 3, 2, 1, 2, 2, 16, 101, 97, 27, 230, 41, 155, 188, 234, 76, 173, 92,
            163, 84, 82, 236, 140, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 11, 5, 0, 48, 77, 49, 75, 48, 73, 6,
            3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0, 105, 0, 122, 0, 97, 0,
            116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0,
            115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 48, 30, 23, 13, 50, 50, 48, 53, 49, 55, 50, 48, 50,
            48, 53, 57, 90, 23, 13, 50, 50, 48, 53, 49, 55, 50, 49, 50, 53, 53, 57, 90, 48, 129, 151, 49, 52, 48, 50,
            6, 10, 9, 146, 38, 137, 147, 242, 44, 100, 1, 25, 22, 36, 52, 99, 53, 97, 53, 101, 99, 49, 45, 57, 97, 99,
            53, 45, 52, 54, 49, 50, 45, 57, 98, 52, 99, 45, 54, 98, 101, 100, 49, 55, 56, 98, 98, 54, 53, 97, 49, 60,
            48, 58, 6, 3, 85, 4, 3, 12, 51, 83, 45, 49, 45, 49, 50, 45, 49, 45, 49, 53, 53, 50, 51, 56, 50, 57, 56, 45,
            49, 51, 48, 51, 48, 53, 52, 50, 55, 52, 45, 50, 53, 48, 51, 53, 49, 55, 51, 49, 51, 45, 51, 54, 52, 54, 56,
            55, 56, 55, 54, 53, 49, 33, 48, 31, 6, 3, 85, 4, 3, 12, 24, 109, 97, 109, 111, 114, 101, 97, 117, 64, 100,
            111, 119, 110, 104, 105, 108, 108, 112, 114, 111, 46, 120, 121, 122, 48, 130, 1, 34, 48, 13, 6, 9, 42, 134,
            72, 134, 247, 13, 1, 1, 1, 5, 0, 3, 130, 1, 15, 0, 48, 130, 1, 10, 2, 130, 1, 1, 0, 194, 219, 35, 14, 127,
            114, 121, 75, 42, 40, 170, 236, 117, 106, 194, 196, 210, 255, 45, 9, 73, 94, 25, 168, 112, 152, 56, 139,
            223, 248, 20, 133, 255, 44, 102, 133, 240, 101, 221, 72, 127, 80, 230, 220, 137, 41, 199, 165, 66, 146,
            235, 21, 59, 219, 83, 181, 107, 163, 33, 225, 17, 18, 38, 232, 38, 163, 178, 191, 144, 202, 9, 145, 190,
            252, 5, 194, 130, 72, 63, 31, 19, 101, 67, 206, 134, 119, 194, 82, 119, 201, 110, 222, 198, 249, 254, 178,
            166, 33, 81, 18, 136, 206, 131, 26, 131, 52, 205, 68, 179, 202, 45, 189, 197, 67, 56, 113, 191, 223, 174,
            106, 56, 49, 29, 108, 182, 203, 94, 192, 127, 120, 204, 3, 152, 248, 24, 76, 86, 1, 193, 242, 174, 173,
            203, 238, 184, 231, 84, 211, 111, 212, 56, 84, 68, 175, 119, 167, 107, 161, 185, 252, 229, 44, 127, 93,
            207, 140, 170, 87, 23, 92, 30, 119, 233, 113, 255, 165, 146, 56, 219, 193, 150, 120, 182, 209, 182, 204,
            42, 195, 58, 205, 119, 221, 208, 164, 245, 44, 47, 113, 155, 70, 92, 255, 251, 208, 239, 176, 67, 111, 84,
            220, 146, 164, 21, 38, 233, 249, 73, 64, 255, 158, 67, 170, 17, 172, 128, 244, 58, 234, 107, 198, 197, 228,
            47, 109, 182, 34, 202, 162, 173, 13, 14, 227, 50, 22, 164, 156, 246, 11, 199, 233, 235, 159, 67, 24, 157,
            191, 2, 3, 1, 0, 1, 163, 121, 48, 119, 48, 14, 6, 3, 85, 29, 15, 1, 1, 255, 4, 4, 3, 2, 5, 160, 48, 51, 6,
            3, 85, 29, 17, 4, 44, 48, 42, 160, 40, 6, 10, 43, 6, 1, 4, 1, 130, 55, 20, 2, 3, 160, 26, 12, 24, 109, 97,
            109, 111, 114, 101, 97, 117, 64, 100, 111, 119, 110, 104, 105, 108, 108, 112, 114, 111, 46, 120, 121, 122,
            48, 19, 6, 3, 85, 29, 37, 4, 12, 48, 10, 6, 8, 43, 6, 1, 5, 5, 7, 3, 2, 48, 27, 6, 9, 43, 6, 1, 4, 1, 130,
            55, 21, 10, 4, 14, 48, 12, 48, 10, 6, 8, 43, 6, 1, 5, 5, 7, 3, 2, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13,
            1, 1, 11, 5, 0, 3, 130, 1, 1, 0, 52, 19, 83, 79, 242, 219, 247, 152, 173, 207, 131, 233, 36, 186, 213, 25,
            104, 89, 172, 106, 112, 21, 31, 25, 139, 186, 54, 131, 147, 126, 67, 185, 59, 165, 22, 61, 147, 69, 118,
            54, 212, 229, 232, 3, 126, 50, 47, 77, 180, 165, 8, 54, 246, 20, 89, 126, 35, 253, 179, 73, 109, 45, 146,
            74, 93, 64, 173, 178, 113, 103, 240, 177, 6, 97, 76, 171, 140, 27, 192, 114, 120, 77, 24, 203, 203, 122,
            219, 171, 132, 229, 111, 70, 63, 121, 17, 89, 172, 191, 204, 169, 52, 16, 123, 41, 236, 116, 145, 236, 224,
            25, 216, 62, 58, 195, 117, 105, 150, 46, 183, 197, 180, 51, 165, 97, 11, 247, 243, 139, 18, 176, 65, 137,
            203, 78, 51, 164, 138, 202, 142, 134, 80, 75, 95, 139, 135, 86, 147, 108, 27, 126, 168, 174, 55, 197, 69,
            92, 151, 206, 207, 73, 125, 101, 119, 99, 130, 136, 18, 105, 77, 7, 28, 178, 113, 171, 58, 202, 173, 12,
            124, 32, 63, 234, 131, 127, 88, 17, 210, 29, 218, 175, 125, 34, 237, 219, 106, 207, 136, 126, 112, 70, 59,
            229, 13, 23, 13, 243, 1, 5, 255, 222, 59, 190, 227, 219, 157, 219, 211, 96, 170, 168, 82, 127, 41, 47, 239,
            51, 249, 69, 151, 69, 229, 9, 67, 243, 105, 206, 127, 3, 188, 159, 80, 19, 35, 156, 162, 196, 106, 47, 15,
            109, 17, 253, 117, 14, 126, 160, 49, 130, 1, 199, 48, 130, 1, 195, 2, 1, 1, 48, 97, 48, 77, 49, 75, 48, 73,
            6, 3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0, 105, 0, 122, 0, 97,
            0, 116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0,
            115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 2, 16, 101, 97, 27, 230, 41, 155, 188, 234, 76, 173,
            92, 163, 84, 82, 236, 140, 48, 9, 6, 5, 43, 14, 3, 2, 26, 5, 0, 160, 61, 48, 22, 6, 9, 42, 134, 72, 134,
            247, 13, 1, 9, 3, 49, 9, 6, 7, 43, 6, 1, 5, 2, 3, 1, 48, 35, 6, 9, 42, 134, 72, 134, 247, 13, 1, 9, 4, 49,
            22, 4, 20, 102, 68, 227, 41, 95, 65, 130, 138, 235, 245, 189, 250, 152, 141, 108, 239, 191, 39, 172, 175,
            48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 1, 5, 0, 4, 130, 1, 0, 6, 97, 27, 199, 212, 1, 132, 104, 88,
            138, 94, 98, 29, 14, 122, 189, 62, 172, 78, 2, 208, 64, 141, 218, 173, 137, 36, 101, 172, 75, 136, 159,
            133, 134, 37, 136, 127, 128, 177, 114, 108, 138, 53, 41, 125, 213, 123, 231, 149, 112, 215, 248, 240, 15,
            20, 141, 9, 63, 18, 80, 2, 93, 126, 125, 229, 208, 142, 219, 196, 111, 110, 121, 50, 171, 57, 164, 95, 41,
            181, 78, 215, 151, 226, 247, 93, 60, 224, 85, 152, 93, 251, 78, 73, 67, 85, 143, 42, 44, 166, 162, 187,
            135, 124, 27, 55, 145, 134, 45, 75, 252, 22, 191, 193, 63, 30, 158, 83, 198, 188, 187, 220, 121, 40, 9,
            223, 210, 9, 69, 121, 182, 193, 153, 144, 169, 235, 7, 244, 38, 135, 155, 241, 140, 192, 226, 175, 163,
            157, 233, 65, 108, 215, 223, 232, 195, 159, 136, 194, 43, 247, 102, 143, 37, 56, 209, 56, 95, 55, 127, 6,
            253, 154, 29, 163, 53, 56, 152, 106, 138, 128, 47, 129, 19, 103, 170, 72, 208, 221, 95, 111, 26, 123, 174,
            85, 164, 85, 237, 168, 190, 145, 129, 213, 212, 110, 143, 199, 57, 35, 78, 12, 134, 159, 94, 82, 191, 241,
            132, 31, 112, 46, 40, 225, 103, 146, 254, 108, 56, 160, 208, 176, 166, 214, 161, 1, 143, 105, 169, 183,
            149, 222, 225, 246, 119, 193, 223, 54, 175, 234, 92, 143, 197, 93, 158, 242, 226, 254, 27,
        ];

        let pa_pk_as_req: PaPkAsReq = picky_asn1_der::from_bytes(&raw_data).unwrap();

        assert_eq!(
            PaPkAsReq {
                signed_auth_pack: ImplicitContextTag0::from(OctetStringAsn1::from(vec![
                    48, 130, 7, 238, 2, 1, 3, 49, 11, 48, 9, 6, 5, 43, 14, 3, 2, 26, 5, 0, 48, 130, 2, 31, 6, 7, 43, 6,
                    1, 5, 2, 3, 1, 160, 130, 2, 18, 4, 130, 2, 14, 48, 130, 2, 10, 160, 57, 48, 55, 160, 5, 2, 3, 6,
                    219, 251, 161, 17, 24, 15, 50, 48, 50, 50, 48, 53, 49, 55, 50, 48, 50, 53, 53, 57, 90, 162, 3, 2,
                    1, 0, 163, 22, 4, 20, 197, 143, 17, 64, 61, 203, 186, 45, 19, 30, 175, 125, 106, 6, 209, 5, 36, 69,
                    144, 97, 161, 130, 1, 167, 48, 130, 1, 163, 48, 130, 1, 23, 6, 7, 42, 134, 72, 206, 62, 2, 1, 48,
                    130, 1, 10, 2, 129, 129, 0, 255, 255, 255, 255, 255, 255, 255, 255, 201, 15, 218, 162, 33, 104,
                    194, 52, 196, 198, 98, 139, 128, 220, 28, 209, 41, 2, 78, 8, 138, 103, 204, 116, 2, 11, 190, 166,
                    59, 19, 155, 34, 81, 74, 8, 121, 142, 52, 4, 221, 239, 149, 25, 179, 205, 58, 67, 27, 48, 43, 10,
                    109, 242, 95, 20, 55, 79, 225, 53, 109, 109, 81, 194, 69, 228, 133, 181, 118, 98, 94, 126, 198,
                    244, 76, 66, 233, 166, 55, 237, 107, 11, 255, 92, 182, 244, 6, 183, 237, 238, 56, 107, 251, 90,
                    137, 159, 165, 174, 159, 36, 17, 124, 75, 31, 230, 73, 40, 102, 81, 236, 230, 83, 129, 255, 255,
                    255, 255, 255, 255, 255, 255, 2, 1, 2, 2, 129, 128, 127, 255, 255, 255, 255, 255, 255, 255, 228,
                    135, 237, 81, 16, 180, 97, 26, 98, 99, 49, 69, 192, 110, 14, 104, 148, 129, 39, 4, 69, 51, 230, 58,
                    1, 5, 223, 83, 29, 137, 205, 145, 40, 165, 4, 60, 199, 26, 2, 110, 247, 202, 140, 217, 230, 157,
                    33, 141, 152, 21, 133, 54, 249, 47, 138, 27, 167, 240, 154, 182, 182, 168, 225, 34, 242, 66, 218,
                    187, 49, 47, 63, 99, 122, 38, 33, 116, 211, 27, 246, 181, 133, 255, 174, 91, 122, 3, 91, 246, 247,
                    28, 53, 253, 173, 68, 207, 210, 215, 79, 146, 8, 190, 37, 143, 243, 36, 148, 51, 40, 246, 115, 41,
                    192, 255, 255, 255, 255, 255, 255, 255, 255, 3, 129, 133, 0, 2, 129, 129, 0, 219, 78, 185, 183,
                    129, 7, 4, 73, 79, 203, 237, 216, 60, 162, 113, 232, 36, 233, 162, 75, 8, 200, 168, 109, 49, 32,
                    207, 86, 26, 198, 121, 143, 205, 90, 248, 169, 6, 178, 153, 1, 237, 156, 2, 145, 162, 150, 218,
                    232, 144, 183, 193, 58, 7, 27, 217, 215, 160, 30, 69, 15, 211, 28, 18, 216, 145, 196, 14, 47, 119,
                    76, 163, 178, 243, 136, 213, 190, 122, 108, 59, 140, 94, 32, 75, 114, 17, 239, 99, 81, 208, 221,
                    232, 214, 193, 129, 129, 135, 191, 117, 72, 254, 44, 211, 92, 124, 203, 235, 196, 113, 1, 123, 74,
                    139, 101, 121, 212, 210, 119, 162, 26, 230, 153, 254, 123, 68, 151, 135, 52, 29, 163, 34, 4, 32,
                    72, 91, 60, 222, 24, 28, 4, 155, 141, 138, 44, 10, 136, 54, 202, 60, 146, 234, 183, 130, 109, 34,
                    94, 10, 87, 237, 162, 55, 173, 100, 115, 43, 160, 130, 3, 236, 48, 130, 3, 232, 48, 130, 2, 208,
                    160, 3, 2, 1, 2, 2, 16, 101, 97, 27, 230, 41, 155, 188, 234, 76, 173, 92, 163, 84, 82, 236, 140,
                    48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 11, 5, 0, 48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3,
                    30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116,
                    0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0,
                    115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 48, 30, 23, 13, 50, 50, 48, 53, 49, 55, 50,
                    48, 50, 48, 53, 57, 90, 23, 13, 50, 50, 48, 53, 49, 55, 50, 49, 50, 53, 53, 57, 90, 48, 129, 151,
                    49, 52, 48, 50, 6, 10, 9, 146, 38, 137, 147, 242, 44, 100, 1, 25, 22, 36, 52, 99, 53, 97, 53, 101,
                    99, 49, 45, 57, 97, 99, 53, 45, 52, 54, 49, 50, 45, 57, 98, 52, 99, 45, 54, 98, 101, 100, 49, 55,
                    56, 98, 98, 54, 53, 97, 49, 60, 48, 58, 6, 3, 85, 4, 3, 12, 51, 83, 45, 49, 45, 49, 50, 45, 49, 45,
                    49, 53, 53, 50, 51, 56, 50, 57, 56, 45, 49, 51, 48, 51, 48, 53, 52, 50, 55, 52, 45, 50, 53, 48, 51,
                    53, 49, 55, 51, 49, 51, 45, 51, 54, 52, 54, 56, 55, 56, 55, 54, 53, 49, 33, 48, 31, 6, 3, 85, 4, 3,
                    12, 24, 109, 97, 109, 111, 114, 101, 97, 117, 64, 100, 111, 119, 110, 104, 105, 108, 108, 112, 114,
                    111, 46, 120, 121, 122, 48, 130, 1, 34, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 1, 5, 0, 3,
                    130, 1, 15, 0, 48, 130, 1, 10, 2, 130, 1, 1, 0, 194, 219, 35, 14, 127, 114, 121, 75, 42, 40, 170,
                    236, 117, 106, 194, 196, 210, 255, 45, 9, 73, 94, 25, 168, 112, 152, 56, 139, 223, 248, 20, 133,
                    255, 44, 102, 133, 240, 101, 221, 72, 127, 80, 230, 220, 137, 41, 199, 165, 66, 146, 235, 21, 59,
                    219, 83, 181, 107, 163, 33, 225, 17, 18, 38, 232, 38, 163, 178, 191, 144, 202, 9, 145, 190, 252, 5,
                    194, 130, 72, 63, 31, 19, 101, 67, 206, 134, 119, 194, 82, 119, 201, 110, 222, 198, 249, 254, 178,
                    166, 33, 81, 18, 136, 206, 131, 26, 131, 52, 205, 68, 179, 202, 45, 189, 197, 67, 56, 113, 191,
                    223, 174, 106, 56, 49, 29, 108, 182, 203, 94, 192, 127, 120, 204, 3, 152, 248, 24, 76, 86, 1, 193,
                    242, 174, 173, 203, 238, 184, 231, 84, 211, 111, 212, 56, 84, 68, 175, 119, 167, 107, 161, 185,
                    252, 229, 44, 127, 93, 207, 140, 170, 87, 23, 92, 30, 119, 233, 113, 255, 165, 146, 56, 219, 193,
                    150, 120, 182, 209, 182, 204, 42, 195, 58, 205, 119, 221, 208, 164, 245, 44, 47, 113, 155, 70, 92,
                    255, 251, 208, 239, 176, 67, 111, 84, 220, 146, 164, 21, 38, 233, 249, 73, 64, 255, 158, 67, 170,
                    17, 172, 128, 244, 58, 234, 107, 198, 197, 228, 47, 109, 182, 34, 202, 162, 173, 13, 14, 227, 50,
                    22, 164, 156, 246, 11, 199, 233, 235, 159, 67, 24, 157, 191, 2, 3, 1, 0, 1, 163, 121, 48, 119, 48,
                    14, 6, 3, 85, 29, 15, 1, 1, 255, 4, 4, 3, 2, 5, 160, 48, 51, 6, 3, 85, 29, 17, 4, 44, 48, 42, 160,
                    40, 6, 10, 43, 6, 1, 4, 1, 130, 55, 20, 2, 3, 160, 26, 12, 24, 109, 97, 109, 111, 114, 101, 97,
                    117, 64, 100, 111, 119, 110, 104, 105, 108, 108, 112, 114, 111, 46, 120, 121, 122, 48, 19, 6, 3,
                    85, 29, 37, 4, 12, 48, 10, 6, 8, 43, 6, 1, 5, 5, 7, 3, 2, 48, 27, 6, 9, 43, 6, 1, 4, 1, 130, 55,
                    21, 10, 4, 14, 48, 12, 48, 10, 6, 8, 43, 6, 1, 5, 5, 7, 3, 2, 48, 13, 6, 9, 42, 134, 72, 134, 247,
                    13, 1, 1, 11, 5, 0, 3, 130, 1, 1, 0, 52, 19, 83, 79, 242, 219, 247, 152, 173, 207, 131, 233, 36,
                    186, 213, 25, 104, 89, 172, 106, 112, 21, 31, 25, 139, 186, 54, 131, 147, 126, 67, 185, 59, 165,
                    22, 61, 147, 69, 118, 54, 212, 229, 232, 3, 126, 50, 47, 77, 180, 165, 8, 54, 246, 20, 89, 126, 35,
                    253, 179, 73, 109, 45, 146, 74, 93, 64, 173, 178, 113, 103, 240, 177, 6, 97, 76, 171, 140, 27, 192,
                    114, 120, 77, 24, 203, 203, 122, 219, 171, 132, 229, 111, 70, 63, 121, 17, 89, 172, 191, 204, 169,
                    52, 16, 123, 41, 236, 116, 145, 236, 224, 25, 216, 62, 58, 195, 117, 105, 150, 46, 183, 197, 180,
                    51, 165, 97, 11, 247, 243, 139, 18, 176, 65, 137, 203, 78, 51, 164, 138, 202, 142, 134, 80, 75, 95,
                    139, 135, 86, 147, 108, 27, 126, 168, 174, 55, 197, 69, 92, 151, 206, 207, 73, 125, 101, 119, 99,
                    130, 136, 18, 105, 77, 7, 28, 178, 113, 171, 58, 202, 173, 12, 124, 32, 63, 234, 131, 127, 88, 17,
                    210, 29, 218, 175, 125, 34, 237, 219, 106, 207, 136, 126, 112, 70, 59, 229, 13, 23, 13, 243, 1, 5,
                    255, 222, 59, 190, 227, 219, 157, 219, 211, 96, 170, 168, 82, 127, 41, 47, 239, 51, 249, 69, 151,
                    69, 229, 9, 67, 243, 105, 206, 127, 3, 188, 159, 80, 19, 35, 156, 162, 196, 106, 47, 15, 109, 17,
                    253, 117, 14, 126, 160, 49, 130, 1, 199, 48, 130, 1, 195, 2, 1, 1, 48, 97, 48, 77, 49, 75, 48, 73,
                    6, 3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0, 105, 0, 122,
                    0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0,
                    101, 0, 115, 0, 115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 2, 16, 101, 97, 27, 230, 41,
                    155, 188, 234, 76, 173, 92, 163, 84, 82, 236, 140, 48, 9, 6, 5, 43, 14, 3, 2, 26, 5, 0, 160, 61,
                    48, 22, 6, 9, 42, 134, 72, 134, 247, 13, 1, 9, 3, 49, 9, 6, 7, 43, 6, 1, 5, 2, 3, 1, 48, 35, 6, 9,
                    42, 134, 72, 134, 247, 13, 1, 9, 4, 49, 22, 4, 20, 102, 68, 227, 41, 95, 65, 130, 138, 235, 245,
                    189, 250, 152, 141, 108, 239, 191, 39, 172, 175, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 1,
                    5, 0, 4, 130, 1, 0, 6, 97, 27, 199, 212, 1, 132, 104, 88, 138, 94, 98, 29, 14, 122, 189, 62, 172,
                    78, 2, 208, 64, 141, 218, 173, 137, 36, 101, 172, 75, 136, 159, 133, 134, 37, 136, 127, 128, 177,
                    114, 108, 138, 53, 41, 125, 213, 123, 231, 149, 112, 215, 248, 240, 15, 20, 141, 9, 63, 18, 80, 2,
                    93, 126, 125, 229, 208, 142, 219, 196, 111, 110, 121, 50, 171, 57, 164, 95, 41, 181, 78, 215, 151,
                    226, 247, 93, 60, 224, 85, 152, 93, 251, 78, 73, 67, 85, 143, 42, 44, 166, 162, 187, 135, 124, 27,
                    55, 145, 134, 45, 75, 252, 22, 191, 193, 63, 30, 158, 83, 198, 188, 187, 220, 121, 40, 9, 223, 210,
                    9, 69, 121, 182, 193, 153, 144, 169, 235, 7, 244, 38, 135, 155, 241, 140, 192, 226, 175, 163, 157,
                    233, 65, 108, 215, 223, 232, 195, 159, 136, 194, 43, 247, 102, 143, 37, 56, 209, 56, 95, 55, 127,
                    6, 253, 154, 29, 163, 53, 56, 152, 106, 138, 128, 47, 129, 19, 103, 170, 72, 208, 221, 95, 111, 26,
                    123, 174, 85, 164, 85, 237, 168, 190, 145, 129, 213, 212, 110, 143, 199, 57, 35, 78, 12, 134, 159,
                    94, 82, 191, 241, 132, 31, 112, 46, 40, 225, 103, 146, 254, 108, 56, 160, 208, 176, 166, 214, 161,
                    1, 143, 105, 169, 183, 149, 222, 225, 246, 119, 193, 223, 54, 175, 234, 92, 143, 197, 93, 158, 242,
                    226, 254, 27
                ])),
                trusted_certifiers: Optional::from(None),
                kdc_pk_id: Optional::from(None),
            },
            pa_pk_as_req
        );
    }

    #[test]
    fn pa_pk_as_rep_encode_enc_key_pack() {
        let pa_pk_as_rep = PaPkAsRep::EncKeyPack(ImplicitContextTag1::from(OctetStringAsn1::from(vec![
            94, 82, 191, 241, 132, 31, 112, 46, 40, 225, 103, 146, 254, 108, 56, 160, 208, 176, 166, 214, 161,
        ])));

        let encoded = picky_asn1_der::to_vec(&pa_pk_as_rep).unwrap();

        assert_eq!(
            &[
                129, 21, 94, 82, 191, 241, 132, 31, 112, 46, 40, 225, 103, 146, 254, 108, 56, 160, 208, 176, 166, 214,
                161
            ],
            encoded.as_slice()
        );
    }

    #[test]
    fn pa_pk_as_rep_encode_dh_info() {
        let pa_pk_as_rep = PaPkAsRep::DhInfo(ExplicitContextTag0::from(DhRepInfo {
            dh_signed_data: ImplicitContextTag0::from(OctetStringAsn1::from(vec![
                221, 28, 174, 247, 196, 69, 212, 187, 37, 162, 198, 33, 238, 127, 68, 191, 239, 233, 46, 240, 67, 151,
                40, 76, 232, 41, 137, 233, 117, 199, 11, 95, 201, 123, 246, 188, 44, 122, 105, 175, 179, 204, 127, 221,
                57, 190, 66,
            ])),
            server_dh_nonce: Optional::from(Some(ExplicitContextTag1::from(OctetStringAsn1::from(vec![
                197, 87, 1, 68, 61, 12, 232, 203, 120, 225, 215, 208, 224, 194, 49,
            ])))),
        }));

        let encoded = picky_asn1_der::to_vec(&pa_pk_as_rep).unwrap();

        assert_eq!(
            &[
                160, 70, 48, 68, 128, 47, 221, 28, 174, 247, 196, 69, 212, 187, 37, 162, 198, 33, 238, 127, 68, 191,
                239, 233, 46, 240, 67, 151, 40, 76, 232, 41, 137, 233, 117, 199, 11, 95, 201, 123, 246, 188, 44, 122,
                105, 175, 179, 204, 127, 221, 57, 190, 66, 161, 17, 4, 15, 197, 87, 1, 68, 61, 12, 232, 203, 120, 225,
                215, 208, 224, 194, 49
            ],
            encoded.as_slice()
        );
    }

    #[test]
    fn pa_pk_as_rep_decode_enc_key_pack() {
        let raw_data = [
            129, 21, 94, 82, 191, 241, 132, 31, 112, 46, 40, 225, 103, 146, 254, 108, 56, 160, 208, 176, 166, 214, 161,
        ];

        let pa_pk_as_rep: PaPkAsRep = picky_asn1_der::from_bytes(&raw_data).unwrap();

        assert_eq!(
            PaPkAsRep::EncKeyPack(ImplicitContextTag1::from(OctetStringAsn1::from(vec![
                94, 82, 191, 241, 132, 31, 112, 46, 40, 225, 103, 146, 254, 108, 56, 160, 208, 176, 166, 214, 161,
            ]))),
            pa_pk_as_rep
        );
    }

    #[test]
    fn pa_pk_as_rep_decode_dh_info() {
        let raw_data = [
            160, 130, 6, 103, 48, 130, 6, 99, 128, 130, 6, 59, 48, 130, 6, 55, 2, 1, 3, 49, 11, 48, 9, 6, 5, 43, 14, 3,
            2, 26, 5, 0, 48, 129, 162, 6, 7, 43, 6, 1, 5, 2, 3, 2, 160, 129, 150, 4, 129, 147, 48, 129, 144, 160, 129,
            136, 3, 129, 133, 0, 2, 129, 129, 0, 218, 76, 235, 63, 222, 122, 67, 6, 210, 4, 219, 144, 10, 253, 105,
            197, 87, 1, 68, 61, 12, 232, 203, 120, 225, 215, 208, 224, 194, 49, 162, 89, 251, 216, 82, 14, 92, 119,
            236, 147, 132, 225, 80, 117, 104, 218, 221, 117, 104, 149, 33, 9, 225, 159, 16, 243, 57, 44, 147, 221, 164,
            8, 131, 5, 43, 219, 70, 8, 7, 60, 118, 39, 124, 30, 48, 205, 41, 150, 112, 133, 151, 136, 121, 91, 56, 12,
            251, 210, 239, 155, 85, 63, 244, 177, 112, 133, 181, 245, 110, 164, 31, 197, 241, 14, 137, 195, 223, 226,
            23, 158, 68, 27, 66, 118, 200, 170, 122, 33, 156, 104, 103, 155, 136, 28, 247, 8, 54, 20, 161, 3, 2, 1, 0,
            160, 130, 3, 179, 48, 130, 3, 175, 48, 130, 2, 151, 160, 3, 2, 1, 2, 2, 16, 85, 47, 30, 63, 139, 118, 224,
            166, 140, 108, 227, 232, 117, 255, 59, 249, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 11, 5, 0, 48,
            77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0,
            105, 0, 122, 0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99,
            0, 101, 0, 115, 0, 115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 48, 30, 23, 13, 50, 50, 48, 53,
            49, 55, 49, 57, 50, 52, 48, 54, 90, 23, 13, 50, 50, 48, 53, 49, 56, 49, 57, 50, 57, 48, 54, 90, 48, 101,
            49, 52, 48, 50, 6, 10, 9, 146, 38, 137, 147, 242, 44, 100, 1, 25, 22, 36, 52, 99, 53, 97, 53, 101, 99, 49,
            45, 57, 97, 99, 53, 45, 52, 54, 49, 50, 45, 57, 98, 52, 99, 45, 54, 98, 101, 100, 49, 55, 56, 98, 98, 54,
            53, 97, 49, 45, 48, 43, 6, 3, 85, 4, 3, 12, 36, 99, 99, 51, 100, 57, 54, 98, 52, 45, 48, 97, 48, 100, 45,
            52, 97, 53, 100, 45, 56, 98, 102, 98, 45, 56, 100, 54, 101, 97, 56, 100, 57, 53, 53, 100, 50, 48, 130, 1,
            34, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 1, 5, 0, 3, 130, 1, 15, 0, 48, 130, 1, 10, 2, 130, 1, 1,
            0, 199, 77, 166, 39, 80, 222, 84, 115, 41, 66, 75, 180, 150, 219, 181, 13, 3, 102, 176, 46, 8, 194, 243,
            198, 233, 126, 112, 105, 214, 207, 85, 254, 186, 228, 217, 23, 28, 232, 31, 209, 227, 99, 220, 60, 28, 78,
            168, 51, 162, 48, 63, 120, 240, 186, 203, 224, 154, 164, 227, 78, 224, 4, 120, 160, 170, 134, 185, 124,
            156, 51, 235, 206, 20, 62, 191, 51, 182, 195, 7, 184, 139, 80, 198, 103, 40, 37, 155, 219, 95, 56, 162,
            242, 152, 249, 204, 236, 191, 67, 64, 180, 223, 13, 207, 32, 242, 203, 172, 126, 77, 90, 197, 6, 32, 162,
            179, 253, 86, 158, 233, 147, 176, 44, 132, 150, 128, 103, 128, 157, 185, 157, 131, 50, 142, 248, 67, 68,
            217, 122, 154, 103, 143, 101, 207, 136, 219, 79, 226, 0, 159, 117, 200, 43, 80, 95, 163, 247, 218, 117, 69,
            248, 88, 64, 5, 181, 63, 109, 247, 80, 141, 174, 38, 220, 64, 250, 3, 82, 187, 52, 243, 151, 141, 237, 115,
            115, 17, 199, 29, 213, 4, 197, 242, 40, 177, 141, 170, 241, 173, 179, 212, 100, 73, 102, 21, 255, 74, 213,
            158, 11, 241, 110, 82, 139, 142, 209, 118, 197, 15, 240, 243, 50, 39, 23, 243, 172, 152, 170, 75, 97, 102,
            169, 23, 245, 147, 180, 29, 22, 58, 3, 100, 35, 6, 98, 92, 164, 55, 39, 81, 87, 58, 25, 22, 31, 234, 173,
            200, 213, 2, 3, 1, 0, 1, 163, 115, 48, 113, 48, 14, 6, 3, 85, 29, 15, 1, 1, 255, 4, 4, 3, 2, 5, 160, 48,
            45, 6, 3, 85, 29, 17, 4, 38, 48, 36, 130, 11, 65, 90, 82, 68, 79, 87, 78, 45, 87, 49, 48, 130, 11, 65, 90,
            82, 68, 79, 87, 78, 45, 87, 49, 48, 130, 8, 49, 48, 46, 49, 46, 48, 46, 52, 48, 19, 6, 3, 85, 29, 37, 4,
            12, 48, 10, 6, 8, 43, 6, 1, 5, 5, 7, 3, 1, 48, 27, 6, 9, 43, 6, 1, 4, 1, 130, 55, 21, 10, 4, 14, 48, 12,
            48, 10, 6, 8, 43, 6, 1, 5, 5, 7, 3, 1, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 11, 5, 0, 3, 130, 1,
            1, 0, 124, 211, 121, 241, 39, 108, 96, 140, 158, 149, 109, 212, 59, 78, 203, 168, 204, 137, 19, 171, 77,
            67, 61, 166, 94, 122, 197, 145, 118, 157, 192, 156, 113, 249, 60, 110, 33, 188, 169, 75, 137, 57, 64, 130,
            5, 128, 58, 248, 87, 211, 139, 50, 157, 138, 176, 226, 159, 239, 15, 103, 84, 126, 26, 147, 246, 82, 12,
            80, 56, 61, 192, 231, 75, 104, 125, 95, 59, 52, 28, 236, 7, 195, 239, 242, 49, 105, 113, 168, 210, 102,
            192, 207, 212, 185, 185, 100, 137, 250, 219, 180, 75, 84, 139, 15, 115, 187, 165, 170, 227, 48, 245, 58,
            91, 137, 220, 197, 161, 180, 99, 195, 82, 92, 119, 170, 199, 3, 161, 221, 211, 177, 124, 92, 195, 92, 210,
            165, 117, 41, 98, 234, 175, 123, 44, 252, 230, 121, 151, 208, 210, 165, 67, 150, 152, 200, 243, 21, 85,
            209, 223, 217, 73, 243, 38, 166, 60, 3, 6, 140, 26, 170, 243, 114, 205, 210, 117, 111, 235, 38, 42, 43,
            217, 170, 118, 150, 180, 38, 218, 39, 198, 156, 157, 182, 223, 198, 200, 13, 255, 101, 56, 124, 126, 126,
            10, 255, 166, 203, 26, 251, 140, 248, 229, 41, 19, 94, 135, 210, 18, 185, 30, 55, 23, 195, 54, 88, 140,
            174, 87, 65, 250, 143, 243, 163, 172, 142, 207, 190, 111, 113, 157, 204, 246, 0, 119, 11, 253, 208, 155,
            101, 40, 217, 188, 10, 14, 89, 120, 146, 49, 130, 1, 199, 48, 130, 1, 195, 2, 1, 1, 48, 97, 48, 77, 49, 75,
            48, 73, 6, 3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110, 0, 105, 0, 122,
            0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0,
            115, 0, 115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 2, 16, 85, 47, 30, 63, 139, 118, 224, 166,
            140, 108, 227, 232, 117, 255, 59, 249, 48, 9, 6, 5, 43, 14, 3, 2, 26, 5, 0, 160, 61, 48, 22, 6, 9, 42, 134,
            72, 134, 247, 13, 1, 9, 3, 49, 9, 6, 7, 43, 6, 1, 5, 2, 3, 2, 48, 35, 6, 9, 42, 134, 72, 134, 247, 13, 1,
            9, 4, 49, 22, 4, 20, 207, 84, 215, 76, 61, 237, 27, 245, 186, 203, 116, 211, 41, 149, 95, 129, 147, 149,
            136, 166, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1, 1, 5, 0, 4, 130, 1, 0, 111, 17, 1, 8, 116, 231,
            197, 34, 227, 186, 92, 57, 120, 101, 232, 20, 207, 52, 39, 202, 112, 40, 194, 203, 86, 55, 75, 186, 92, 52,
            55, 244, 26, 119, 58, 46, 200, 251, 185, 76, 244, 242, 18, 149, 207, 17, 126, 4, 41, 216, 89, 14, 137, 100,
            190, 70, 0, 183, 138, 152, 26, 56, 162, 219, 146, 159, 128, 68, 195, 190, 195, 32, 2, 64, 118, 220, 182,
            183, 218, 219, 145, 62, 246, 216, 214, 182, 244, 251, 97, 78, 232, 123, 167, 187, 154, 212, 10, 222, 244,
            232, 249, 194, 202, 26, 149, 174, 109, 15, 218, 247, 45, 209, 232, 92, 5, 87, 249, 192, 249, 91, 240, 160,
            7, 202, 196, 8, 161, 2, 41, 10, 242, 107, 43, 100, 45, 5, 75, 49, 222, 172, 249, 252, 196, 68, 71, 250,
            195, 11, 185, 161, 184, 39, 65, 22, 44, 78, 245, 132, 193, 63, 231, 94, 113, 116, 125, 210, 243, 106, 48,
            201, 50, 14, 177, 43, 168, 94, 39, 44, 221, 97, 60, 94, 230, 117, 248, 57, 143, 88, 117, 139, 75, 113, 97,
            48, 39, 13, 168, 2, 247, 89, 110, 216, 96, 161, 47, 183, 168, 204, 145, 221, 28, 174, 247, 196, 69, 212,
            187, 37, 162, 198, 33, 238, 127, 68, 191, 239, 233, 46, 240, 67, 151, 40, 76, 232, 41, 137, 233, 117, 199,
            11, 95, 201, 123, 246, 188, 44, 122, 105, 175, 179, 204, 127, 221, 57, 190, 66, 161, 34, 4, 32, 160, 135,
            139, 83, 106, 40, 32, 75, 125, 12, 23, 191, 191, 163, 215, 162, 217, 132, 196, 80, 212, 102, 88, 251, 252,
            135, 151, 137, 121, 58, 199, 71,
        ];

        let pa_pk_as_rep: PaPkAsRep = picky_asn1_der::from_bytes(&raw_data).unwrap();

        assert_eq!(
            PaPkAsRep::DhInfo(ExplicitContextTag0::from(DhRepInfo {
                dh_signed_data: ImplicitContextTag0::from(OctetStringAsn1::from(vec![
                    48, 130, 6, 55, 2, 1, 3, 49, 11, 48, 9, 6, 5, 43, 14, 3, 2, 26, 5, 0, 48, 129, 162, 6, 7, 43, 6, 1,
                    5, 2, 3, 2, 160, 129, 150, 4, 129, 147, 48, 129, 144, 160, 129, 136, 3, 129, 133, 0, 2, 129, 129,
                    0, 218, 76, 235, 63, 222, 122, 67, 6, 210, 4, 219, 144, 10, 253, 105, 197, 87, 1, 68, 61, 12, 232,
                    203, 120, 225, 215, 208, 224, 194, 49, 162, 89, 251, 216, 82, 14, 92, 119, 236, 147, 132, 225, 80,
                    117, 104, 218, 221, 117, 104, 149, 33, 9, 225, 159, 16, 243, 57, 44, 147, 221, 164, 8, 131, 5, 43,
                    219, 70, 8, 7, 60, 118, 39, 124, 30, 48, 205, 41, 150, 112, 133, 151, 136, 121, 91, 56, 12, 251,
                    210, 239, 155, 85, 63, 244, 177, 112, 133, 181, 245, 110, 164, 31, 197, 241, 14, 137, 195, 223,
                    226, 23, 158, 68, 27, 66, 118, 200, 170, 122, 33, 156, 104, 103, 155, 136, 28, 247, 8, 54, 20, 161,
                    3, 2, 1, 0, 160, 130, 3, 179, 48, 130, 3, 175, 48, 130, 2, 151, 160, 3, 2, 1, 2, 2, 16, 85, 47, 30,
                    63, 139, 118, 224, 166, 140, 108, 227, 232, 117, 255, 59, 249, 48, 13, 6, 9, 42, 134, 72, 134, 247,
                    13, 1, 1, 11, 5, 0, 48, 77, 49, 75, 48, 73, 6, 3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0,
                    114, 0, 103, 0, 97, 0, 110, 0, 105, 0, 122, 0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0,
                    50, 0, 80, 0, 45, 0, 65, 0, 99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50,
                    0, 49, 0, 93, 48, 30, 23, 13, 50, 50, 48, 53, 49, 55, 49, 57, 50, 52, 48, 54, 90, 23, 13, 50, 50,
                    48, 53, 49, 56, 49, 57, 50, 57, 48, 54, 90, 48, 101, 49, 52, 48, 50, 6, 10, 9, 146, 38, 137, 147,
                    242, 44, 100, 1, 25, 22, 36, 52, 99, 53, 97, 53, 101, 99, 49, 45, 57, 97, 99, 53, 45, 52, 54, 49,
                    50, 45, 57, 98, 52, 99, 45, 54, 98, 101, 100, 49, 55, 56, 98, 98, 54, 53, 97, 49, 45, 48, 43, 6, 3,
                    85, 4, 3, 12, 36, 99, 99, 51, 100, 57, 54, 98, 52, 45, 48, 97, 48, 100, 45, 52, 97, 53, 100, 45,
                    56, 98, 102, 98, 45, 56, 100, 54, 101, 97, 56, 100, 57, 53, 53, 100, 50, 48, 130, 1, 34, 48, 13, 6,
                    9, 42, 134, 72, 134, 247, 13, 1, 1, 1, 5, 0, 3, 130, 1, 15, 0, 48, 130, 1, 10, 2, 130, 1, 1, 0,
                    199, 77, 166, 39, 80, 222, 84, 115, 41, 66, 75, 180, 150, 219, 181, 13, 3, 102, 176, 46, 8, 194,
                    243, 198, 233, 126, 112, 105, 214, 207, 85, 254, 186, 228, 217, 23, 28, 232, 31, 209, 227, 99, 220,
                    60, 28, 78, 168, 51, 162, 48, 63, 120, 240, 186, 203, 224, 154, 164, 227, 78, 224, 4, 120, 160,
                    170, 134, 185, 124, 156, 51, 235, 206, 20, 62, 191, 51, 182, 195, 7, 184, 139, 80, 198, 103, 40,
                    37, 155, 219, 95, 56, 162, 242, 152, 249, 204, 236, 191, 67, 64, 180, 223, 13, 207, 32, 242, 203,
                    172, 126, 77, 90, 197, 6, 32, 162, 179, 253, 86, 158, 233, 147, 176, 44, 132, 150, 128, 103, 128,
                    157, 185, 157, 131, 50, 142, 248, 67, 68, 217, 122, 154, 103, 143, 101, 207, 136, 219, 79, 226, 0,
                    159, 117, 200, 43, 80, 95, 163, 247, 218, 117, 69, 248, 88, 64, 5, 181, 63, 109, 247, 80, 141, 174,
                    38, 220, 64, 250, 3, 82, 187, 52, 243, 151, 141, 237, 115, 115, 17, 199, 29, 213, 4, 197, 242, 40,
                    177, 141, 170, 241, 173, 179, 212, 100, 73, 102, 21, 255, 74, 213, 158, 11, 241, 110, 82, 139, 142,
                    209, 118, 197, 15, 240, 243, 50, 39, 23, 243, 172, 152, 170, 75, 97, 102, 169, 23, 245, 147, 180,
                    29, 22, 58, 3, 100, 35, 6, 98, 92, 164, 55, 39, 81, 87, 58, 25, 22, 31, 234, 173, 200, 213, 2, 3,
                    1, 0, 1, 163, 115, 48, 113, 48, 14, 6, 3, 85, 29, 15, 1, 1, 255, 4, 4, 3, 2, 5, 160, 48, 45, 6, 3,
                    85, 29, 17, 4, 38, 48, 36, 130, 11, 65, 90, 82, 68, 79, 87, 78, 45, 87, 49, 48, 130, 11, 65, 90,
                    82, 68, 79, 87, 78, 45, 87, 49, 48, 130, 8, 49, 48, 46, 49, 46, 48, 46, 52, 48, 19, 6, 3, 85, 29,
                    37, 4, 12, 48, 10, 6, 8, 43, 6, 1, 5, 5, 7, 3, 1, 48, 27, 6, 9, 43, 6, 1, 4, 1, 130, 55, 21, 10, 4,
                    14, 48, 12, 48, 10, 6, 8, 43, 6, 1, 5, 5, 7, 3, 1, 48, 13, 6, 9, 42, 134, 72, 134, 247, 13, 1, 1,
                    11, 5, 0, 3, 130, 1, 1, 0, 124, 211, 121, 241, 39, 108, 96, 140, 158, 149, 109, 212, 59, 78, 203,
                    168, 204, 137, 19, 171, 77, 67, 61, 166, 94, 122, 197, 145, 118, 157, 192, 156, 113, 249, 60, 110,
                    33, 188, 169, 75, 137, 57, 64, 130, 5, 128, 58, 248, 87, 211, 139, 50, 157, 138, 176, 226, 159,
                    239, 15, 103, 84, 126, 26, 147, 246, 82, 12, 80, 56, 61, 192, 231, 75, 104, 125, 95, 59, 52, 28,
                    236, 7, 195, 239, 242, 49, 105, 113, 168, 210, 102, 192, 207, 212, 185, 185, 100, 137, 250, 219,
                    180, 75, 84, 139, 15, 115, 187, 165, 170, 227, 48, 245, 58, 91, 137, 220, 197, 161, 180, 99, 195,
                    82, 92, 119, 170, 199, 3, 161, 221, 211, 177, 124, 92, 195, 92, 210, 165, 117, 41, 98, 234, 175,
                    123, 44, 252, 230, 121, 151, 208, 210, 165, 67, 150, 152, 200, 243, 21, 85, 209, 223, 217, 73, 243,
                    38, 166, 60, 3, 6, 140, 26, 170, 243, 114, 205, 210, 117, 111, 235, 38, 42, 43, 217, 170, 118, 150,
                    180, 38, 218, 39, 198, 156, 157, 182, 223, 198, 200, 13, 255, 101, 56, 124, 126, 126, 10, 255, 166,
                    203, 26, 251, 140, 248, 229, 41, 19, 94, 135, 210, 18, 185, 30, 55, 23, 195, 54, 88, 140, 174, 87,
                    65, 250, 143, 243, 163, 172, 142, 207, 190, 111, 113, 157, 204, 246, 0, 119, 11, 253, 208, 155,
                    101, 40, 217, 188, 10, 14, 89, 120, 146, 49, 130, 1, 199, 48, 130, 1, 195, 2, 1, 1, 48, 97, 48, 77,
                    49, 75, 48, 73, 6, 3, 85, 4, 3, 30, 66, 0, 77, 0, 83, 0, 45, 0, 79, 0, 114, 0, 103, 0, 97, 0, 110,
                    0, 105, 0, 122, 0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 45, 0, 80, 0, 50, 0, 80, 0, 45, 0, 65, 0,
                    99, 0, 99, 0, 101, 0, 115, 0, 115, 0, 32, 0, 91, 0, 50, 0, 48, 0, 50, 0, 49, 0, 93, 2, 16, 85, 47,
                    30, 63, 139, 118, 224, 166, 140, 108, 227, 232, 117, 255, 59, 249, 48, 9, 6, 5, 43, 14, 3, 2, 26,
                    5, 0, 160, 61, 48, 22, 6, 9, 42, 134, 72, 134, 247, 13, 1, 9, 3, 49, 9, 6, 7, 43, 6, 1, 5, 2, 3, 2,
                    48, 35, 6, 9, 42, 134, 72, 134, 247, 13, 1, 9, 4, 49, 22, 4, 20, 207, 84, 215, 76, 61, 237, 27,
                    245, 186, 203, 116, 211, 41, 149, 95, 129, 147, 149, 136, 166, 48, 13, 6, 9, 42, 134, 72, 134, 247,
                    13, 1, 1, 1, 5, 0, 4, 130, 1, 0, 111, 17, 1, 8, 116, 231, 197, 34, 227, 186, 92, 57, 120, 101, 232,
                    20, 207, 52, 39, 202, 112, 40, 194, 203, 86, 55, 75, 186, 92, 52, 55, 244, 26, 119, 58, 46, 200,
                    251, 185, 76, 244, 242, 18, 149, 207, 17, 126, 4, 41, 216, 89, 14, 137, 100, 190, 70, 0, 183, 138,
                    152, 26, 56, 162, 219, 146, 159, 128, 68, 195, 190, 195, 32, 2, 64, 118, 220, 182, 183, 218, 219,
                    145, 62, 246, 216, 214, 182, 244, 251, 97, 78, 232, 123, 167, 187, 154, 212, 10, 222, 244, 232,
                    249, 194, 202, 26, 149, 174, 109, 15, 218, 247, 45, 209, 232, 92, 5, 87, 249, 192, 249, 91, 240,
                    160, 7, 202, 196, 8, 161, 2, 41, 10, 242, 107, 43, 100, 45, 5, 75, 49, 222, 172, 249, 252, 196, 68,
                    71, 250, 195, 11, 185, 161, 184, 39, 65, 22, 44, 78, 245, 132, 193, 63, 231, 94, 113, 116, 125,
                    210, 243, 106, 48, 201, 50, 14, 177, 43, 168, 94, 39, 44, 221, 97, 60, 94, 230, 117, 248, 57, 143,
                    88, 117, 139, 75, 113, 97, 48, 39, 13, 168, 2, 247, 89, 110, 216, 96, 161, 47, 183, 168, 204, 145,
                    221, 28, 174, 247, 196, 69, 212, 187, 37, 162, 198, 33, 238, 127, 68, 191, 239, 233, 46, 240, 67,
                    151, 40, 76, 232, 41, 137, 233, 117, 199, 11, 95, 201, 123, 246, 188, 44, 122, 105, 175, 179, 204,
                    127, 221, 57, 190, 66
                ])),
                server_dh_nonce: Optional::from(Some(ExplicitContextTag1::from(OctetStringAsn1::from(vec![
                    160, 135, 139, 83, 106, 40, 32, 75, 125, 12, 23, 191, 191, 163, 215, 162, 217, 132, 196, 80, 212,
                    102, 88, 251, 252, 135, 151, 137, 121, 58, 199, 71
                ]))))
            })),
            pa_pk_as_rep
        );
    }
}
