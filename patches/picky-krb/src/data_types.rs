use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use picky_asn1::wrapper::{
    Asn1SequenceOf, BitStringAsn1, ExplicitContextTag0, ExplicitContextTag1, ExplicitContextTag2, ExplicitContextTag3,
    ExplicitContextTag4, ExplicitContextTag5, ExplicitContextTag6, ExplicitContextTag7, ExplicitContextTag8,
    ExplicitContextTag9, ExplicitContextTag10, GeneralStringAsn1, GeneralizedTimeAsn1, IntegerAsn1, OctetStringAsn1,
    Optional,
};
use picky_asn1_der::application_tag::ApplicationTag;
use serde::de::Error;
use serde::{Deserialize, Deserializer, Serialize, de};

use std::fmt::Debug;
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::{fmt, io};

use crate::constants::types::{
    AUTHENTICATOR_TYPE, ENC_AP_REP_PART_TYPE, ENC_TICKET_PART_TYPE, KRB_PRIV_ENC_PART, TICKET_TYPE,
};
use crate::messages::KrbError;

/// [RFC 4120 5.2.1](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not-rust
/// KerberosString  ::= GeneralString (IA5String)
/// ```
pub type KerberosStringAsn1 = GeneralStringAsn1;

/// [2.2.2 KDC_PROXY_MESSAGE](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-kkdcp/5778aff5-b182-4b97-a970-29c7f911eef2)
pub type Realm = KerberosStringAsn1;

/// [RFC 4120 5.2.2](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// PrincipalName   ::= SEQUENCE {
///         name-type       [0] Int32,
///         name-string     [1] SEQUENCE OF KerberosString
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct PrincipalName {
    pub name_type: ExplicitContextTag0<IntegerAsn1>,
    pub name_string: ExplicitContextTag1<Asn1SequenceOf<KerberosStringAsn1>>,
}

/// [RFC 4120 5.2.3](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// KerberosTime    ::= GeneralizedTime -- with no fractional seconds
/// ```
pub type KerberosTime = GeneralizedTimeAsn1;

/// [RFC 4120 5.2.4](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// Microseconds    ::= INTEGER (0..999999)
/// ```
pub type Microseconds = IntegerAsn1;

/// [RFC 4120 5.2.5](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// HostAddress   ::= SEQUENCE {
///         addr-type       [0] Int32,
///         address         [1] OCTET STRING
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct HostAddress {
    pub addr_type: ExplicitContextTag0<IntegerAsn1>,
    pub address: ExplicitContextTag1<OctetStringAsn1>,
}

/// [RFC 4120 5.2.5](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// HostAddresses   -- NOTE: subtly different from rfc1510,
///                 -- but has a value mapping and encodes the same
///         ::= SEQUENCE OF HostAddress
/// ```
pub type HostAddresses = Asn1SequenceOf<HostAddress>;

/// [RFC 4120 5.2.6](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// AuthorizationData       ::= SEQUENCE OF SEQUENCE {
///         ad-type         [0] Int32,
///         ad-data         [1] OCTET STRING
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct AuthorizationDataInner {
    pub ad_type: ExplicitContextTag0<IntegerAsn1>,
    pub ad_data: ExplicitContextTag1<OctetStringAsn1>,
}

pub type AuthorizationData = Asn1SequenceOf<AuthorizationDataInner>;

/// [RFC 4120 5.2.7](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// PA-DATA         ::= SEQUENCE {
///         padata-type     [1] Int32,
///         padata-value    [2] OCTET STRING
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct PaData {
    pub padata_type: ExplicitContextTag1<IntegerAsn1>,
    pub padata_data: ExplicitContextTag2<OctetStringAsn1>,
}

/// [RFC 4120 5.2.8](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// KerberosFlags   ::= BIT STRING (SIZE (32..MAX))
/// ```
pub type KerberosFlags = BitStringAsn1;

/// [RFC 4120 5.2.9](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// EncryptedData   ::= SEQUENCE {
///         etype   [0] Int32 -- EncryptionType --,
///         kvno    [1] UInt32 OPTIONAL,
///         cipher  [2] OCTET STRING -- ciphertext
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct EncryptedData {
    pub etype: ExplicitContextTag0<IntegerAsn1>,
    pub kvno: Optional<Option<ExplicitContextTag1<IntegerAsn1>>>,
    pub cipher: ExplicitContextTag2<OctetStringAsn1>,
}

/// [RFC 4120 5.2.9](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// EncryptionKey   ::= SEQUENCE {
///         keytype         [0] Int32 -- actually encryption type --,
///         keyvalue        [1] OCTET STRING
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct EncryptionKey {
    pub key_type: ExplicitContextTag0<IntegerAsn1>,
    pub key_value: ExplicitContextTag1<OctetStringAsn1>,
}

/// [RFC 4120 5.3](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// Ticket          ::= [APPLICATION 1] SEQUENCE {
///         tkt-vno         [0] INTEGER (5),
///         realm           [1] Realm,
///         sname           [2] PrincipalName,
///         enc-part        [3] EncryptedData
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct TicketInner {
    pub tkt_vno: ExplicitContextTag0<IntegerAsn1>,
    pub realm: ExplicitContextTag1<Realm>,
    pub sname: ExplicitContextTag2<PrincipalName>,
    pub enc_part: ExplicitContextTag3<EncryptedData>,
}

pub type Ticket = ApplicationTag<TicketInner, TICKET_TYPE>;

/// [RFC 4120 5.3](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// TransitedEncoding       ::= SEQUENCE {
///         tr-type         [0] Int32 -- must be registered --,
///         contents        [1] OCTET STRING
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct TransitedEncoding {
    pub tr_type: ExplicitContextTag0<IntegerAsn1>,
    pub contents: ExplicitContextTag1<OctetStringAsn1>,
}

/// [RFC 4120 5.3](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// EncTicketPart   ::= [APPLICATION 3] SEQUENCE {
///         flags                   [0] TicketFlags,
///         key                     [1] EncryptionKey,
///         crealm                  [2] Realm,
///         cname                   [3] PrincipalName,
///         transited               [4] TransitedEncoding,
///         authtime                [5] KerberosTime,
///         starttime               [6] KerberosTime OPTIONAL,
///         endtime                 [7] KerberosTime,
///         renew-till              [8] KerberosTime OPTIONAL,
///         caddr                   [9] HostAddresses OPTIONAL,
///         authorization-data      [10] AuthorizationData OPTIONAL
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct EncTicketPartInner {
    pub flags: ExplicitContextTag0<KerberosFlags>,
    pub key: ExplicitContextTag1<EncryptionKey>,
    pub crealm: ExplicitContextTag2<Realm>,
    pub cname: ExplicitContextTag3<PrincipalName>,
    pub transited: ExplicitContextTag4<TransitedEncoding>,
    pub auth_time: ExplicitContextTag5<KerberosTime>,
    pub starttime: Optional<Option<ExplicitContextTag6<KerberosTime>>>,
    pub endtime: ExplicitContextTag7<KerberosTime>,
    #[serde(default)]
    pub renew_till: Optional<Option<ExplicitContextTag8<KerberosTime>>>,
    #[serde(default)]
    pub caddr: Optional<Option<ExplicitContextTag9<HostAddresses>>>,
    #[serde(default)]
    pub authorization_data: Optional<Option<ExplicitContextTag10<AuthorizationData>>>,
}

pub type EncTicketPart = ApplicationTag<EncTicketPartInner, ENC_TICKET_PART_TYPE>;

/// [RFC 4120 5.4.2](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// LastReq         ::=     SEQUENCE OF SEQUENCE {
///         lr-type         [0] Int32,
///         lr-value        [1] KerberosTime
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct LastReqInner {
    pub lr_type: ExplicitContextTag0<IntegerAsn1>,
    pub lr_value: ExplicitContextTag1<KerberosTime>,
}
pub type LastReq = Asn1SequenceOf<LastReqInner>;

/// [MS-KILE 2.2.2](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-KILE/%5bMS-KILE%5d.pdf)
///
/// ```not_rust
/// KERB-ERROR-DATA ::= SEQUENCE {
///     data-type [1] INTEGER,
///     data-value [2] OCTET STRING OPTIONAL
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct KerbErrorData {
    pub data_type: ExplicitContextTag1<IntegerAsn1>,
    #[serde(default)]
    pub data_value: Optional<Option<ExplicitContextTag2<BitStringAsn1>>>,
}

/// [RFC 4120 ](https://datatracker.ietf.org/doc/html/rfc4120#section-5.2.7.2)
///
/// ```not_rust
/// PA-ENC-TS-ENC           ::= SEQUENCE {
///         patimestamp     [0] KerberosTime -- client's time --,
///         pausec          [1] Microseconds OPTIONAL
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct PaEncTsEnc {
    pub patimestamp: ExplicitContextTag0<KerberosTime>,
    #[serde(default)]
    pub pausec: Optional<Option<ExplicitContextTag1<Microseconds>>>,
}

/// [RFC 4120 ](https://datatracker.ietf.org/doc/html/rfc4120#section-5.2.7.2)
///
/// ```not_rust
/// PA-ENC-TIMESTAMP        ::= EncryptedData -- PA-ENC-TS-ENC
/// ```
pub type PaEncTimestamp = EncryptedData;

/// [MS-KILE 2.2.3](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-KILE/%5bMS-KILE%5d.pdf)
///
/// ```not_rust
/// KERB-PA-PAC-REQUEST ::= SEQUENCE {
///     include-pac[0] BOOLEAN
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct KerbPaPacRequest {
    pub include_pac: ExplicitContextTag0<bool>,
}

/// [MS-KILE 2.2.10](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-KILE/%5bMS-KILE%5d.pdf)
///
/// ```not_rust
/// PA-PAC-OPTIONS ::= SEQUENCE {
///     flags                ::= KerberosFlags
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct PaPacOptions {
    pub flags: ExplicitContextTag0<KerberosFlags>,
}

/// [RFC 4120](https://datatracker.ietf.org/doc/html/rfc4120#section-5.5.1)
///
/// ```not_rust
/// APOptions       ::= KerberosFlags
/// ```
pub type ApOptions = KerberosFlags;

/// [RFC 4120](https://datatracker.ietf.org/doc/html/rfc4120#section-5.2.9)
///
/// ```not_rust
/// Checksum        ::= SEQUENCE {
///         cksumtype       [0] Int32,
///         checksum        [1] OCTET STRING
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct Checksum {
    pub cksumtype: ExplicitContextTag0<IntegerAsn1>,
    pub checksum: ExplicitContextTag1<OctetStringAsn1>,
}

/// [RFC 4120](https://datatracker.ietf.org/doc/html/rfc4120#section-5.5.1)
///
/// ```not_rust
/// Authenticator   ::= [APPLICATION 2] SEQUENCE  {
///         authenticator-vno       [0] INTEGER (5),
///         crealm                  [1] Realm,
///         cname                   [2] PrincipalName,
///         cksum                   [3] Checksum OPTIONAL,
///         cusec                   [4] Microseconds,
///         ctime                   [5] KerberosTime,
///         subkey                  [6] EncryptionKey OPTIONAL,
///         seq-number              [7] UInt32 OPTIONAL,
///         authorization-data      [8] AuthorizationData OPTIONAL
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct AuthenticatorInner {
    pub authenticator_vno: ExplicitContextTag0<IntegerAsn1>,
    pub crealm: ExplicitContextTag1<Realm>,
    pub cname: ExplicitContextTag2<PrincipalName>,
    pub cksum: Optional<Option<ExplicitContextTag3<Checksum>>>,
    pub cusec: ExplicitContextTag4<Microseconds>,
    pub ctime: ExplicitContextTag5<KerberosTime>,
    #[serde(default)]
    pub subkey: Optional<Option<ExplicitContextTag6<EncryptionKey>>>,
    #[serde(default)]
    pub seq_number: Optional<Option<ExplicitContextTag7<IntegerAsn1>>>,
    #[serde(default)]
    pub authorization_data: Optional<Option<ExplicitContextTag8<AuthorizationData>>>,
}
pub type Authenticator = ApplicationTag<AuthenticatorInner, AUTHENTICATOR_TYPE>;

/// [RFC 4120](https://datatracker.ietf.org/doc/html/rfc4120#section-5.5.2)
///
/// ```not_rust
/// EncAPRepPart    ::= [APPLICATION 27] SEQUENCE {
///         ctime           [0] KerberosTime,
///         cusec           [1] Microseconds,
///         subkey          [2] EncryptionKey OPTIONAL,
///         seq-number      [3] UInt32 OPTIONAL
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct EncApRepPartInner {
    pub ctime: ExplicitContextTag0<KerberosTime>,
    pub cusec: ExplicitContextTag1<Microseconds>,
    #[serde(default)]
    pub subkey: Optional<Option<ExplicitContextTag2<EncryptionKey>>>,
    #[serde(default)]
    pub seq_number: Optional<Option<ExplicitContextTag3<IntegerAsn1>>>,
}
pub type EncApRepPart = ApplicationTag<EncApRepPartInner, ENC_AP_REP_PART_TYPE>;

/// [RFC 4120](https://datatracker.ietf.org/doc/html/rfc4120#section-5.2.7.5)
///
/// ```not_rust
/// ETYPE-INFO2-ENTRY       ::= SEQUENCE {
///         etype           [0] Int32,
///         salt            [1] KerberosString OPTIONAL,
///         s2kparams       [2] OCTET STRING OPTIONAL
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct EtypeInfo2Entry {
    pub etype: ExplicitContextTag0<IntegerAsn1>,
    #[serde(default)]
    pub salt: Optional<Option<ExplicitContextTag1<KerberosStringAsn1>>>,
    #[serde(default)]
    pub s2kparams: Optional<Option<ExplicitContextTag2<OctetStringAsn1>>>,
}

/// [RFC 4120](https://datatracker.ietf.org/doc/html/rfc4120#section-5.2.7.5)
///
/// ```not_rust
/// ETYPE-INFO2              ::= SEQUENCE SIZE (1..MAX) OF ETYPE-INFO2-ENTRY
/// ```
pub type EtypeInfo2 = Asn1SequenceOf<EtypeInfo2Entry>;

/// [RFC 4120](https://datatracker.ietf.org/doc/html/rfc4120#section-5.7.1)
///
/// ```not_rust
/// EncKrbPrivPart  ::= [APPLICATION 28] SEQUENCE {
///         user-data       [0] OCTET STRING,
///         timestamp       [1] KerberosTime OPTIONAL,
///         usec            [2] Microseconds OPTIONAL,
///         seq-number      [3] UInt32 OPTIONAL,
///         s-address       [4] HostAddress -- sender's addr --,
///         r-address       [5] HostAddress OPTIONAL -- recip's addr
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct EncKrbPrivPartInner {
    pub user_data: ExplicitContextTag0<OctetStringAsn1>,
    pub timestamp: Optional<Option<ExplicitContextTag1<KerberosTime>>>,
    pub usec: Optional<Option<ExplicitContextTag2<KerberosTime>>>,
    pub seq_number: Optional<Option<ExplicitContextTag3<IntegerAsn1>>>,
    pub s_address: ExplicitContextTag4<HostAddress>,
    #[serde(default)]
    pub r_address: Optional<Option<ExplicitContextTag5<HostAddress>>>,
}

pub type EncKrbPrivPart = ApplicationTag<EncKrbPrivPartInner, KRB_PRIV_ENC_PART>;

/// [RFC 3244](https://datatracker.ietf.org/doc/html/rfc3244.html#section-2)
///
/// ```not_rust
/// ChangePasswdData ::=  SEQUENCE {
///     newpasswd[0]   OCTET STRING,
///     targname[1]    PrincipalName OPTIONAL,
///     targrealm[2]   Realm OPTIONAL
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct ChangePasswdData {
    pub new_passwd: ExplicitContextTag0<OctetStringAsn1>,
    pub target_name: Optional<Option<ExplicitContextTag1<PrincipalName>>>,
    pub target_realm: Optional<Option<ExplicitContextTag2<Realm>>>,
}

pub trait ResultExt<'a, T>
where
    T: Deserialize<'a>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'a>,
        Self: Sized;
}

impl<'de, T: Deserialize<'de>> ResultExt<'de, T> for Result<T, KrbError> {
    fn deserialize<D>(deserializer: D) -> Result<Self, <D as de::Deserializer<'de>>::Error>
    where
        D: Deserializer<'de>,
        Self: Sized,
    {
        struct Visitor<V>(PhantomData<V>);

        impl<'de, V: de::Deserialize<'de>> de::Visitor<'de> for Visitor<V> {
            type Value = Result<V, KrbError>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a valid DER-encoded KbResult")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                match seq.next_element() {
                    Ok(value) => value
                        .ok_or_else(|| A::Error::missing_field("Missing KrbResult value"))
                        .map(|value| Ok(value)),
                    Err(_) => match seq.next_element() {
                        Ok(error_value) => error_value
                            .ok_or_else(|| A::Error::missing_field("Missing KrbResult value"))
                            .map(|error_value| Err(error_value)),
                        Err(err) => Err(err),
                    },
                }
            }
        }

        deserializer.deserialize_enum("KrbResult", &["Ok", "Err"], Visitor::<T>(PhantomData))
    }
}

pub type KrbResult<T> = Result<T, KrbError>;

/// [2.2.6 KERB-AD-RESTRICTION-ENTRY](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-KILE/%5bMS-KILE%5d.pdf)
///
/// ```not_rust
/// KERB-AD-RESTRICTION-ENTRY ::= SEQUENCE {
/// restriction-type [0] Int32,
/// restriction [1] OCTET STRING
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct KerbAdRestrictionEntry {
    pub restriction_type: ExplicitContextTag0<IntegerAsn1>,
    pub restriction: ExplicitContextTag1<OctetStringAsn1>,
}

/// [3.1.1.4 Machine ID](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-KILE/%5bMS-KILE%5d.pdf)
/// KILE implements a 32-byte binary random string machine ID
pub const MACHINE_ID_LENGTH: usize = 32;

/// [2.2.5 LSAP_TOKEN_INFO_INTEGRITY](https://winprotocoldoc.blob.core.windows.net/productionwindowsarchives/MS-KILE/%5bMS-KILE%5d.pdf)
///
/// ```not_rust
/// typedef struct _LSAP_TOKEN_INFO_INTEGRITY {
///     unsigned long Flags;
///     unsigned long TokenIL;
///     unsigned char MachineID[32];
/// } LSAP_TOKEN_INFO_INTEGRITY,
/// ```
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct LsapTokenInfoIntegrity {
    pub flags: u32,
    pub token_il: u32,
    pub machine_id: [u8; MACHINE_ID_LENGTH],
}

impl LsapTokenInfoIntegrity {
    pub fn encode(&self, mut to: impl Write) -> io::Result<()> {
        to.write_u32::<LittleEndian>(self.flags)?;
        to.write_u32::<LittleEndian>(self.token_il)?;
        to.write_all(&self.machine_id)?;

        Ok(())
    }

    pub fn decode(mut from: impl Read) -> io::Result<Self> {
        let flags = from.read_u32::<LittleEndian>()?;
        let token_il = from.read_u32::<LittleEndian>()?;

        let mut machine_id = [0; MACHINE_ID_LENGTH];
        from.read_exact(&mut machine_id)?;

        Ok(Self {
            flags,
            token_il,
            machine_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::constants::error_codes::KDC_ERR_C_PRINCIPAL_UNKNOWN;
    use crate::data_types::{
        AuthenticatorInner, AuthorizationData, AuthorizationDataInner, Checksum, EncApRepPart, EncApRepPartInner,
        EncKrbPrivPart, EncKrbPrivPartInner, EncryptedData, EncryptionKey, EtypeInfo2Entry, HostAddress,
        KerbPaPacRequest, KerberosStringAsn1, KerberosTime, LastReqInner, PaData, PrincipalName,
    };
    use crate::messages::{AsReq, KdcReq, KdcReqBody, KrbError, KrbErrorInner};
    use picky_asn1::bit_string::BitString;
    use picky_asn1::date::Date;
    use picky_asn1::restricted_string::Ia5String;
    use picky_asn1::wrapper::{
        Asn1SequenceOf, BitStringAsn1, ExplicitContextTag0, ExplicitContextTag1, ExplicitContextTag2,
        ExplicitContextTag3, ExplicitContextTag4, ExplicitContextTag5, ExplicitContextTag6, ExplicitContextTag7,
        ExplicitContextTag8, ExplicitContextTag9, ExplicitContextTag10, ExplicitContextTag11, GeneralStringAsn1,
        GeneralizedTimeAsn1, IntegerAsn1, OctetStringAsn1, Optional,
    };
    use picky_asn1_der::application_tag::ApplicationTag;

    use super::{ChangePasswdData, Microseconds, PaEncTsEnc};

    #[test]
    fn change_passwd_data() {
        let expected_raw = [
            48, 47, 160, 14, 4, 12, 113, 119, 101, 81, 87, 69, 49, 50, 51, 33, 64, 35, 161, 14, 48, 12, 160, 2, 2, 0,
            161, 6, 48, 4, 27, 2, 101, 51, 162, 13, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77,
        ];
        let expected = ChangePasswdData {
            // qweQWE123!@#
            new_passwd: ExplicitContextTag0::from(OctetStringAsn1::from(vec![
                113, 119, 101, 81, 87, 69, 49, 50, 51, 33, 64, 35,
            ])),
            target_name: Optional::from(Some(ExplicitContextTag1::from(PrincipalName {
                name_type: ExplicitContextTag0::from(IntegerAsn1::from(vec![])),
                name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![KerberosStringAsn1::from(
                    Ia5String::from_string("e3".into()).unwrap(),
                )])),
            }))),
            target_realm: Optional::from(Some(ExplicitContextTag2::from(KerberosStringAsn1::from(
                Ia5String::from_string("EXAMPLE.COM".into()).unwrap(),
            )))),
        };

        let change_password_data: ChangePasswdData = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let raw_change_password_data = picky_asn1_der::to_vec(&change_password_data).unwrap();

        assert_eq!(change_password_data, expected);
        assert_eq!(raw_change_password_data, expected_raw);
    }

    #[test]
    fn enc_krb_priv_part() {
        let expected_raw = [
            124, 25, 48, 23, 160, 4, 4, 2, 0, 0, 164, 15, 48, 13, 160, 3, 2, 1, 2, 161, 6, 4, 4, 192, 168, 0, 108,
        ];
        let expected = EncKrbPrivPart::from(EncKrbPrivPartInner {
            user_data: ExplicitContextTag0::from(OctetStringAsn1::from(vec![0, 0])),
            timestamp: Optional::from(None),
            usec: Optional::from(None),
            seq_number: Optional::from(None),
            s_address: ExplicitContextTag4::from(HostAddress {
                addr_type: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x02])),
                address: ExplicitContextTag1::from(OctetStringAsn1::from(vec![0xc0, 0xa8, 0x00, 0x6c])),
            }),
            r_address: Optional::from(None),
        });

        let enc_krb_priv: EncKrbPrivPart = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let raw_enc_krb_priv = picky_asn1_der::to_vec(&enc_krb_priv).unwrap();

        assert_eq!(enc_krb_priv, expected);
        assert_eq!(raw_enc_krb_priv, expected_raw);
    }

    #[test]
    fn kerberos_string_decode() {
        // EXAMPLE.COM
        let expected = [27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77];

        let s: KerberosStringAsn1 = picky_asn1_der::from_bytes(&expected).unwrap();
        let data = picky_asn1_der::to_vec(&s).unwrap();

        assert_eq!(data, expected);
    }

    #[test]
    fn pa_data() {
        let expected_raw = [
            48, 39, 161, 3, 2, 1, 19, 162, 32, 4, 30, 48, 28, 48, 26, 160, 3, 2, 1, 18, 161, 19, 27, 17, 69, 88, 65,
            77, 80, 76, 69, 46, 67, 79, 77, 109, 121, 117, 115, 101, 114,
        ];
        let expected = PaData {
            padata_type: ExplicitContextTag1::from(IntegerAsn1(vec![19])),
            padata_data: ExplicitContextTag2::from(OctetStringAsn1::from(vec![
                48, 28, 48, 26, 160, 3, 2, 1, 18, 161, 19, 27, 17, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 109,
                121, 117, 115, 101, 114,
            ])),
        };

        let pa_data: PaData = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let pa_data_raw = picky_asn1_der::to_vec(&pa_data).unwrap();

        assert_eq!(pa_data, expected);
        assert_eq!(pa_data_raw, expected_raw);
    }

    #[test]
    fn simple_principal_name() {
        let expected_raw = [
            48, 17, 160, 3, 2, 1, 1, 161, 10, 48, 8, 27, 6, 109, 121, 117, 115, 101, 114,
        ];
        let expected = PrincipalName {
            name_type: ExplicitContextTag0::from(IntegerAsn1(vec![1])),
            name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![GeneralStringAsn1::from(
                Ia5String::from_string("myuser".to_owned()).unwrap(),
            )])),
        };

        let principal_name: PrincipalName = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let principal_name_raw = picky_asn1_der::to_vec(&principal_name).unwrap();

        assert_eq!(principal_name, expected);
        assert_eq!(principal_name_raw, expected_raw);
    }

    #[test]
    fn principal_name_with_two_names() {
        let expected_raw = [
            48, 30, 160, 3, 2, 1, 2, 161, 23, 48, 21, 27, 6, 107, 114, 98, 116, 103, 116, 27, 11, 69, 88, 65, 77, 80,
            76, 69, 46, 67, 79, 77,
        ];
        let expected = PrincipalName {
            name_type: ExplicitContextTag0::from(IntegerAsn1(vec![2])),
            name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![
                GeneralStringAsn1::from(Ia5String::from_string("krbtgt".to_owned()).unwrap()),
                GeneralStringAsn1::from(Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap()),
            ])),
        };

        let principal_name: PrincipalName = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let principal_name_raw = picky_asn1_der::to_vec(&principal_name).unwrap();

        assert_eq!(principal_name, expected);
        assert_eq!(principal_name_raw, expected_raw);
    }

    #[test]
    fn encrypted_data() {
        let expected_raw = [
            48, 129, 252, 160, 3, 2, 1, 18, 161, 3, 2, 1, 1, 162, 129, 239, 4, 129, 236, 166, 11, 233, 202, 198, 160,
            29, 10, 87, 131, 189, 15, 170, 61, 216, 210, 116, 104, 91, 174, 27, 255, 246, 126, 9, 92, 141, 206, 172,
            100, 96, 56, 84, 172, 9, 156, 37, 4, 92, 135, 41, 130, 246, 8, 54, 42, 41, 176, 92, 106, 237, 35, 183, 179,
            141, 35, 17, 246, 38, 42, 131, 226, 151, 25, 155, 134, 251, 197, 4, 209, 223, 122, 135, 145, 113, 169, 139,
            100, 130, 4, 142, 227, 213, 137, 187, 187, 116, 173, 88, 35, 219, 206, 106, 232, 35, 124, 199, 228, 153,
            170, 194, 86, 183, 67, 40, 142, 56, 178, 201, 25, 33, 213, 76, 70, 189, 240, 217, 22, 78, 147, 70, 0, 176,
            78, 67, 33, 216, 37, 52, 200, 21, 104, 186, 190, 171, 60, 13, 250, 138, 135, 27, 159, 235, 29, 163, 193, 2,
            67, 193, 141, 29, 199, 166, 251, 18, 114, 237, 192, 174, 207, 150, 33, 219, 215, 79, 157, 85, 132, 250,
            159, 108, 151, 54, 134, 207, 119, 91, 132, 123, 47, 36, 56, 24, 110, 26, 7, 182, 219, 17, 220, 11, 44, 181,
            227, 25, 25, 244, 14, 56, 130, 82, 227, 114, 54, 167, 75, 202, 140, 245, 136, 61, 29, 22, 247, 154, 5, 33,
            161, 145, 60, 203, 132, 37, 17, 134, 162, 141, 159, 46, 146, 88, 115, 114, 245, 76, 57,
        ];
        let expected = EncryptedData {
            etype: ExplicitContextTag0::from(IntegerAsn1(vec![18])),
            kvno: Optional::from(Option::Some(ExplicitContextTag1::from(IntegerAsn1(vec![1])))),
            cipher: ExplicitContextTag2::from(OctetStringAsn1(vec![
                166, 11, 233, 202, 198, 160, 29, 10, 87, 131, 189, 15, 170, 61, 216, 210, 116, 104, 91, 174, 27, 255,
                246, 126, 9, 92, 141, 206, 172, 100, 96, 56, 84, 172, 9, 156, 37, 4, 92, 135, 41, 130, 246, 8, 54, 42,
                41, 176, 92, 106, 237, 35, 183, 179, 141, 35, 17, 246, 38, 42, 131, 226, 151, 25, 155, 134, 251, 197,
                4, 209, 223, 122, 135, 145, 113, 169, 139, 100, 130, 4, 142, 227, 213, 137, 187, 187, 116, 173, 88, 35,
                219, 206, 106, 232, 35, 124, 199, 228, 153, 170, 194, 86, 183, 67, 40, 142, 56, 178, 201, 25, 33, 213,
                76, 70, 189, 240, 217, 22, 78, 147, 70, 0, 176, 78, 67, 33, 216, 37, 52, 200, 21, 104, 186, 190, 171,
                60, 13, 250, 138, 135, 27, 159, 235, 29, 163, 193, 2, 67, 193, 141, 29, 199, 166, 251, 18, 114, 237,
                192, 174, 207, 150, 33, 219, 215, 79, 157, 85, 132, 250, 159, 108, 151, 54, 134, 207, 119, 91, 132,
                123, 47, 36, 56, 24, 110, 26, 7, 182, 219, 17, 220, 11, 44, 181, 227, 25, 25, 244, 14, 56, 130, 82,
                227, 114, 54, 167, 75, 202, 140, 245, 136, 61, 29, 22, 247, 154, 5, 33, 161, 145, 60, 203, 132, 37, 17,
                134, 162, 141, 159, 46, 146, 88, 115, 114, 245, 76, 57,
            ])),
        };

        let encrypted_data: EncryptedData = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let encrypted_data_raw = picky_asn1_der::to_vec(&encrypted_data).unwrap();

        assert_eq!(encrypted_data, expected);
        assert_eq!(encrypted_data_raw, expected_raw);
    }

    #[test]
    fn encrypted_data_without_kvno() {
        let expected_raw = [
            48, 130, 1, 21, 160, 3, 2, 1, 18, 162, 130, 1, 12, 4, 130, 1, 8, 198, 68, 255, 54, 137, 75, 224, 202, 101,
            33, 67, 17, 110, 98, 71, 39, 211, 155, 248, 29, 67, 235, 64, 135, 38, 247, 252, 121, 38, 244, 112, 7, 92,
            223, 58, 122, 21, 75, 1, 183, 126, 177, 187, 35, 220, 164, 120, 191, 136, 112, 166, 111, 34, 115, 221, 212,
            207, 236, 145, 74, 218, 228, 6, 251, 150, 88, 5, 199, 157, 87, 69, 191, 129, 114, 240, 96, 216, 115, 34,
            43, 124, 147, 144, 154, 148, 221, 49, 107, 4, 38, 242, 48, 80, 144, 188, 74, 23, 0, 113, 223, 172, 60, 185,
            84, 71, 18, 174, 116, 47, 53, 194, 8, 111, 184, 62, 178, 21, 231, 245, 102, 113, 15, 224, 32, 92, 108, 177,
            22, 114, 31, 14, 147, 34, 77, 69, 90, 30, 77, 83, 75, 223, 245, 140, 148, 243, 39, 224, 51, 228, 101, 36,
            221, 5, 255, 184, 46, 254, 218, 229, 175, 41, 207, 229, 107, 247, 160, 6, 83, 91, 1, 77, 195, 201, 148, 27,
            184, 197, 93, 255, 58, 101, 70, 225, 253, 247, 20, 247, 1, 31, 209, 47, 198, 35, 201, 28, 24, 188, 189,
            177, 198, 141, 65, 249, 178, 224, 27, 79, 183, 238, 206, 181, 94, 0, 116, 114, 244, 155, 83, 88, 3, 10,
            223, 2, 215, 133, 201, 99, 136, 211, 56, 105, 144, 140, 196, 232, 216, 54, 173, 195, 10, 92, 161, 233, 13,
            170, 136, 25, 162, 203, 75, 83, 149, 180, 47, 66, 147, 10, 206, 211, 146, 253, 18, 212, 17,
        ];
        let expected = EncryptedData {
            etype: ExplicitContextTag0::from(IntegerAsn1(vec![18])),
            kvno: Optional::from(Option::None),
            cipher: ExplicitContextTag2::from(OctetStringAsn1(vec![
                198, 68, 255, 54, 137, 75, 224, 202, 101, 33, 67, 17, 110, 98, 71, 39, 211, 155, 248, 29, 67, 235, 64,
                135, 38, 247, 252, 121, 38, 244, 112, 7, 92, 223, 58, 122, 21, 75, 1, 183, 126, 177, 187, 35, 220, 164,
                120, 191, 136, 112, 166, 111, 34, 115, 221, 212, 207, 236, 145, 74, 218, 228, 6, 251, 150, 88, 5, 199,
                157, 87, 69, 191, 129, 114, 240, 96, 216, 115, 34, 43, 124, 147, 144, 154, 148, 221, 49, 107, 4, 38,
                242, 48, 80, 144, 188, 74, 23, 0, 113, 223, 172, 60, 185, 84, 71, 18, 174, 116, 47, 53, 194, 8, 111,
                184, 62, 178, 21, 231, 245, 102, 113, 15, 224, 32, 92, 108, 177, 22, 114, 31, 14, 147, 34, 77, 69, 90,
                30, 77, 83, 75, 223, 245, 140, 148, 243, 39, 224, 51, 228, 101, 36, 221, 5, 255, 184, 46, 254, 218,
                229, 175, 41, 207, 229, 107, 247, 160, 6, 83, 91, 1, 77, 195, 201, 148, 27, 184, 197, 93, 255, 58, 101,
                70, 225, 253, 247, 20, 247, 1, 31, 209, 47, 198, 35, 201, 28, 24, 188, 189, 177, 198, 141, 65, 249,
                178, 224, 27, 79, 183, 238, 206, 181, 94, 0, 116, 114, 244, 155, 83, 88, 3, 10, 223, 2, 215, 133, 201,
                99, 136, 211, 56, 105, 144, 140, 196, 232, 216, 54, 173, 195, 10, 92, 161, 233, 13, 170, 136, 25, 162,
                203, 75, 83, 149, 180, 47, 66, 147, 10, 206, 211, 146, 253, 18, 212, 17,
            ])),
        };

        let encrypted_data: EncryptedData = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let encrypted_data_raw = picky_asn1_der::to_vec(&encrypted_data).unwrap();

        assert_eq!(encrypted_data, expected);
        assert_eq!(encrypted_data_raw, expected_raw);
    }

    #[test]
    fn host_address() {
        let expected_raw = [
            0x30, 0x19, 0xa0, 0x03, 0x02, 0x01, 0x14, 0xa1, 0x12, 0x04, 0x10, 0x48, 0x4f, 0x4c, 0x4c, 0x4f, 0x57, 0x42,
            0x41, 0x53, 0x54, 0x49, 0x4f, 0x4e, 0x20, 0x20, 0x20,
        ];
        let expected = HostAddress {
            addr_type: ExplicitContextTag0::from(IntegerAsn1(vec![20])),
            address: ExplicitContextTag1::from(OctetStringAsn1(vec![
                72, 79, 76, 76, 79, 87, 66, 65, 83, 84, 73, 79, 78, 32, 32, 32,
            ])),
        };

        let host_address: HostAddress = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let host_address_raw = picky_asn1_der::to_vec(&host_address).unwrap();

        assert_eq!(host_address, expected);
        assert_eq!(host_address_raw, expected_raw);
    }

    #[test]
    fn encryption_key() {
        let expected_raw = [
            48, 41, 160, 3, 2, 1, 18, 161, 34, 4, 32, 23, 138, 210, 243, 7, 121, 117, 180, 99, 86, 230, 62, 222, 63,
            251, 46, 242, 161, 37, 67, 254, 103, 199, 93, 74, 174, 166, 64, 17, 198, 242, 144,
        ];
        let expected = EncryptionKey {
            key_type: ExplicitContextTag0::from(IntegerAsn1(vec![18])),
            key_value: ExplicitContextTag1::from(OctetStringAsn1(vec![
                23, 138, 210, 243, 7, 121, 117, 180, 99, 86, 230, 62, 222, 63, 251, 46, 242, 161, 37, 67, 254, 103,
                199, 93, 74, 174, 166, 64, 17, 198, 242, 144,
            ])),
        };

        let encryption_key: EncryptionKey = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let encryption_key_raw = picky_asn1_der::to_vec(&encryption_key).unwrap();

        assert_eq!(encryption_key, expected);
        assert_eq!(encryption_key_raw, expected_raw);
    }

    #[test]
    fn last_req_inner() {
        let expected_raw = [
            48, 24, 160, 3, 2, 1, 0, 161, 17, 24, 15, 49, 57, 55, 48, 48, 49, 48, 49, 48, 48, 48, 48, 48, 48, 90,
        ];
        let expected = LastReqInner {
            lr_type: ExplicitContextTag0::from(IntegerAsn1(vec![0])),
            lr_value: ExplicitContextTag1::from(KerberosTime::from(Date::new(1970, 1, 1, 0, 0, 0).unwrap())),
        };

        let last_req_inner: LastReqInner = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let last_req_inner_raw = picky_asn1_der::to_vec(&last_req_inner).unwrap();

        assert_eq!(last_req_inner, expected);
        assert_eq!(last_req_inner_raw, expected_raw);
    }

    #[test]
    fn authenticator() {
        let expected_raw = [
            98, 130, 1, 14, 48, 130, 1, 10, 160, 3, 2, 1, 5, 161, 13, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79,
            77, 162, 15, 48, 13, 160, 3, 2, 1, 1, 161, 6, 48, 4, 27, 2, 112, 51, 163, 37, 48, 35, 160, 5, 2, 3, 0, 128,
            3, 161, 26, 4, 24, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 62, 0, 0, 0, 164, 3, 2, 1,
            8, 165, 17, 24, 15, 50, 48, 50, 50, 48, 52, 48, 53, 48, 56, 49, 57, 52, 54, 90, 166, 43, 48, 41, 160, 3, 2,
            1, 18, 161, 34, 4, 32, 137, 180, 229, 144, 148, 18, 158, 111, 110, 0, 13, 63, 21, 116, 77, 186, 198, 9,
            166, 152, 141, 83, 211, 88, 142, 95, 34, 169, 63, 91, 71, 97, 167, 6, 2, 4, 104, 244, 223, 174, 168, 111,
            48, 109, 48, 107, 160, 3, 2, 1, 1, 161, 100, 4, 98, 48, 96, 48, 14, 160, 4, 2, 2, 0, 143, 161, 6, 4, 4, 0,
            64, 0, 0, 48, 78, 160, 4, 2, 2, 0, 144, 161, 70, 4, 68, 84, 0, 69, 0, 82, 0, 77, 0, 83, 0, 82, 0, 86, 0,
            47, 0, 112, 0, 51, 0, 46, 0, 113, 0, 107, 0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 46, 0, 99, 0, 111, 0,
            109, 0, 64, 0, 81, 0, 75, 0, 65, 0, 84, 0, 73, 0, 79, 0, 78, 0, 46, 0, 67, 0, 79, 0, 77, 0,
        ];

        let expected: ApplicationTag<_, 2> = ApplicationTag(AuthenticatorInner {
            authenticator_vno: ExplicitContextTag0::from(IntegerAsn1(vec![5])),
            crealm: ExplicitContextTag1::from(GeneralStringAsn1::from(
                Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap(),
            )),
            cname: ExplicitContextTag2::from(PrincipalName {
                name_type: ExplicitContextTag0::from(IntegerAsn1(vec![1])),
                name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![KerberosStringAsn1::from(
                    Ia5String::from_string("p3".to_owned()).unwrap(),
                )])),
            }),
            cksum: Optional::from(Some(ExplicitContextTag3::from(Checksum {
                cksumtype: ExplicitContextTag0::from(IntegerAsn1(vec![0x00, 0x80, 0x03])),
                checksum: ExplicitContextTag1::from(OctetStringAsn1::from(vec![
                    0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x3E, 0x00, 0x00, 0x00,
                ])),
            }))),
            cusec: ExplicitContextTag4::from(IntegerAsn1::from(vec![0x08])),
            ctime: ExplicitContextTag5::from(KerberosTime::from(Date::new(2022, 4, 5, 8, 19, 46).unwrap())),
            subkey: Optional::from(Some(ExplicitContextTag6::from(EncryptionKey {
                key_type: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x12])),
                key_value: ExplicitContextTag1::from(OctetStringAsn1::from(vec![
                    0x89, 0xB4, 0xE5, 0x90, 0x94, 0x12, 0x9E, 0x6F, 0x6E, 0x00, 0x0D, 0x3F, 0x15, 0x74, 0x4D, 0xBA,
                    0xC6, 0x09, 0xA6, 0x98, 0x8D, 0x53, 0xD3, 0x58, 0x8E, 0x5F, 0x22, 0xA9, 0x3F, 0x5B, 0x47, 0x61,
                ])),
            }))),
            seq_number: Optional::from(Some(ExplicitContextTag7::from(IntegerAsn1::from(vec![
                0x68, 0xf4, 0xdf, 0xae,
            ])))),
            authorization_data: Optional::from(Some(ExplicitContextTag8::from(AuthorizationData::from(vec![
                AuthorizationDataInner {
                    ad_type: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x01])),
                    ad_data: ExplicitContextTag1::from(OctetStringAsn1::from(vec![
                        48, 96, 48, 14, 160, 4, 2, 2, 0, 143, 161, 6, 4, 4, 0, 64, 0, 0, 48, 78, 160, 4, 2, 2, 0, 144,
                        161, 70, 4, 68, 84, 0, 69, 0, 82, 0, 77, 0, 83, 0, 82, 0, 86, 0, 47, 0, 112, 0, 51, 0, 46, 0,
                        113, 0, 107, 0, 97, 0, 116, 0, 105, 0, 111, 0, 110, 0, 46, 0, 99, 0, 111, 0, 109, 0, 64, 0, 81,
                        0, 75, 0, 65, 0, 84, 0, 73, 0, 79, 0, 78, 0, 46, 0, 67, 0, 79, 0, 77, 0,
                    ])),
                },
            ])))),
        });

        let authenticator: ApplicationTag<AuthenticatorInner, 2> = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let authenticator_raw = picky_asn1_der::to_vec(&authenticator).unwrap();

        assert_eq!(authenticator, expected);
        assert_eq!(authenticator_raw, expected_raw);
    }

    #[test]
    fn kerb_pa_pac_request() {
        let expected_raw = [48, 5, 160, 3, 1, 1, 255];
        let expected = KerbPaPacRequest {
            include_pac: ExplicitContextTag0::from(true),
        };

        let kerb_pa_pac_request: KerbPaPacRequest = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let kerb_pa_pac_request_raw = picky_asn1_der::to_vec(&kerb_pa_pac_request).unwrap();

        assert_eq!(kerb_pa_pac_request, expected);
        assert_eq!(kerb_pa_pac_request_raw, expected_raw);
    }

    #[test]
    fn etype_info2_entry() {
        let expected_raw = [
            48, 22, 160, 3, 2, 1, 18, 161, 15, 27, 13, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 112, 51,
        ];
        let expected = EtypeInfo2Entry {
            etype: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x12])),
            salt: Optional::from(Some(ExplicitContextTag1::from(KerberosStringAsn1::from(
                Ia5String::from_string("EXAMPLE.COMp3".to_owned()).unwrap(),
            )))),
            s2kparams: Optional::from(None),
        };

        let etype_info2_entry: EtypeInfo2Entry = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let etype_info2_entry_raw = picky_asn1_der::to_vec(&etype_info2_entry).unwrap();

        assert_eq!(etype_info2_entry, expected);
        assert_eq!(etype_info2_entry_raw, expected_raw);
    }

    #[test]
    fn pa_enc_ts_enc() {
        let expected_raw = vec![
            48, 24, 160, 17, 24, 15, 50, 48, 50, 50, 48, 52, 48, 53, 48, 56, 49, 57, 52, 54, 90, 161, 3, 2, 1, 32,
        ];
        let expected = PaEncTsEnc {
            patimestamp: ExplicitContextTag0::from(KerberosTime::from(Date::new(2022, 4, 5, 8, 19, 46).unwrap())),
            pausec: Optional::from(Some(ExplicitContextTag1::from(Microseconds::from(vec![32])))),
        };

        let pa_enc_ts_enc: PaEncTsEnc = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let pa_enc_ts_enc_raw = picky_asn1_der::to_vec(&expected).unwrap();

        assert_eq!(pa_enc_ts_enc, expected);
        assert_eq!(pa_enc_ts_enc_raw, expected_raw);
    }

    #[test]
    fn enc_ap_rep_part() {
        let expected_raw = vec![
            123, 79, 48, 77, 160, 17, 24, 15, 50, 48, 50, 50, 48, 52, 48, 57, 49, 49, 49, 54, 52, 52, 90, 161, 3, 2, 1,
            43, 162, 43, 48, 41, 160, 3, 2, 1, 18, 161, 34, 4, 32, 225, 45, 62, 116, 165, 142, 214, 44, 102, 216, 202,
            158, 12, 78, 40, 121, 161, 178, 118, 68, 81, 178, 188, 246, 235, 201, 45, 41, 17, 64, 189, 185, 163, 6, 2,
            4, 74, 244, 122, 62,
        ];
        let expected = EncApRepPart::from(EncApRepPartInner {
            ctime: ExplicitContextTag0::from(KerberosTime::from(Date::new(2022, 4, 9, 11, 16, 44).unwrap())),
            cusec: ExplicitContextTag1::from(Microseconds::from(vec![0x2b])),
            subkey: Optional::from(Some(ExplicitContextTag2::from(EncryptionKey {
                key_type: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x12])),
                key_value: ExplicitContextTag1::from(OctetStringAsn1::from(vec![
                    0xe1, 0x2d, 0x3e, 0x74, 0xa5, 0x8e, 0xd6, 0x2c, 0x66, 0xd8, 0xca, 0x9e, 0x0c, 0x4e, 0x28, 0x79,
                    0xa1, 0xb2, 0x76, 0x44, 0x51, 0xb2, 0xbc, 0xf6, 0xeb, 0xc9, 0x2d, 0x29, 0x11, 0x40, 0xbd, 0xb9,
                ])),
            }))),
            seq_number: Optional::from(Some(ExplicitContextTag3::from(IntegerAsn1::from(vec![
                0x4a, 0xf4, 0x7a, 0x3e,
            ])))),
        });

        let enc_ap_rep_part: EncApRepPart = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let enc_ap_rep_part_raw = picky_asn1_der::to_vec(&expected).unwrap();

        assert_eq!(enc_ap_rep_part, expected);
        assert_eq!(enc_ap_rep_part_raw, expected_raw);
    }

    #[test]
    fn krb_result_decode() {
        use super::ResultExt;
        let raw_as_req = vec![
            106, 129, 181, 48, 129, 178, 161, 3, 2, 1, 5, 162, 3, 2, 1, 10, 163, 26, 48, 24, 48, 10, 161, 4, 2, 2, 0,
            150, 162, 2, 4, 0, 48, 10, 161, 4, 2, 2, 0, 149, 162, 2, 4, 0, 164, 129, 137, 48, 129, 134, 160, 7, 3, 5,
            0, 0, 0, 0, 16, 161, 19, 48, 17, 160, 3, 2, 1, 1, 161, 10, 48, 8, 27, 6, 109, 121, 117, 115, 101, 114, 162,
            13, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 163, 32, 48, 30, 160, 3, 2, 1, 2, 161, 23, 48, 21,
            27, 6, 107, 114, 98, 116, 103, 116, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 165, 17, 24, 15,
            50, 48, 50, 49, 49, 50, 50, 57, 49, 48, 51, 54, 48, 54, 90, 167, 6, 2, 4, 29, 32, 235, 11, 168, 26, 48, 24,
            2, 1, 18, 2, 1, 17, 2, 1, 20, 2, 1, 19, 2, 1, 16, 2, 1, 23, 2, 1, 25, 2, 1, 26,
        ];
        let expected_ap_req = Ok(AsReq::from(KdcReq {
            pvno: ExplicitContextTag1::from(IntegerAsn1(vec![5])),
            msg_type: ExplicitContextTag2::from(IntegerAsn1(vec![10])),
            padata: Optional::from(Some(ExplicitContextTag3::from(Asn1SequenceOf::from(vec![
                PaData {
                    padata_type: ExplicitContextTag1::from(IntegerAsn1(vec![0, 150])),
                    padata_data: ExplicitContextTag2::from(OctetStringAsn1(Vec::new())),
                },
                PaData {
                    padata_type: ExplicitContextTag1::from(IntegerAsn1(vec![0, 149])),
                    padata_data: ExplicitContextTag2::from(OctetStringAsn1(Vec::new())),
                },
            ])))),
            req_body: ExplicitContextTag4::from(KdcReqBody {
                kdc_options: ExplicitContextTag0::from(BitStringAsn1::from(BitString::with_bytes(vec![0, 0, 0, 16]))),
                cname: Optional::from(Some(ExplicitContextTag1::from(PrincipalName {
                    name_type: ExplicitContextTag0::from(IntegerAsn1(vec![1])),
                    name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![GeneralStringAsn1::from(
                        Ia5String::from_string("myuser".to_owned()).unwrap(),
                    )])),
                }))),
                realm: ExplicitContextTag2::from(GeneralStringAsn1::from(
                    Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap(),
                )),
                sname: Optional::from(Some(ExplicitContextTag3::from(PrincipalName {
                    name_type: ExplicitContextTag0::from(IntegerAsn1(vec![2])),
                    name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![
                        KerberosStringAsn1::from(Ia5String::from_string("krbtgt".to_owned()).unwrap()),
                        KerberosStringAsn1::from(Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap()),
                    ])),
                }))),
                from: Optional::from(None),
                till: ExplicitContextTag5::from(KerberosTime::from(Date::new(2021, 12, 29, 10, 36, 6).unwrap())),
                rtime: Optional::from(None),
                nonce: ExplicitContextTag7::from(IntegerAsn1(vec![29, 32, 235, 11])),
                etype: ExplicitContextTag8::from(Asn1SequenceOf::from(vec![
                    IntegerAsn1(vec![18]),
                    IntegerAsn1(vec![17]),
                    IntegerAsn1(vec![20]),
                    IntegerAsn1(vec![19]),
                    IntegerAsn1(vec![16]),
                    IntegerAsn1(vec![23]),
                    IntegerAsn1(vec![25]),
                    IntegerAsn1(vec![26]),
                ])),
                addresses: Optional::from(None),
                enc_authorization_data: Optional::from(None),
                additional_tickets: Optional::from(None),
            }),
        }));
        let raw_error = vec![
            126, 129, 151, 48, 129, 148, 160, 3, 2, 1, 5, 161, 3, 2, 1, 30, 164, 17, 24, 15, 50, 48, 50, 49, 49, 50,
            50, 56, 49, 51, 52, 48, 49, 49, 90, 165, 5, 2, 3, 12, 139, 242, 166, 3, 2, 1, 6, 167, 13, 27, 11, 69, 88,
            65, 77, 80, 76, 69, 46, 67, 79, 77, 168, 21, 48, 19, 160, 3, 2, 1, 1, 161, 12, 48, 10, 27, 8, 98, 97, 100,
            95, 117, 115, 101, 114, 169, 13, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 170, 32, 48, 30, 160,
            3, 2, 1, 2, 161, 23, 48, 21, 27, 6, 107, 114, 98, 116, 103, 116, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46,
            67, 79, 77, 171, 18, 27, 16, 67, 76, 73, 69, 78, 84, 95, 78, 79, 84, 95, 70, 79, 85, 78, 68,
        ];
        let expected_error = Err(KrbError::from(KrbErrorInner {
            pvno: ExplicitContextTag0::from(IntegerAsn1(vec![5])),
            msg_type: ExplicitContextTag1::from(IntegerAsn1(vec![30])),
            ctime: Optional::from(None),
            cusec: Optional::from(None),
            stime: ExplicitContextTag4::from(GeneralizedTimeAsn1::from(Date::new(2021, 12, 28, 13, 40, 11).unwrap())),
            susec: ExplicitContextTag5::from(IntegerAsn1(vec![12, 139, 242])),
            error_code: ExplicitContextTag6::from(KDC_ERR_C_PRINCIPAL_UNKNOWN),
            crealm: Optional::from(Some(ExplicitContextTag7::from(GeneralStringAsn1::from(
                Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap(),
            )))),
            cname: Optional::from(Some(ExplicitContextTag8::from(PrincipalName {
                name_type: ExplicitContextTag0::from(IntegerAsn1(vec![1])),
                name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![GeneralStringAsn1::from(
                    Ia5String::from_string("bad_user".to_owned()).unwrap(),
                )])),
            }))),
            realm: ExplicitContextTag9::from(GeneralStringAsn1::from(
                Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap(),
            )),
            sname: ExplicitContextTag10::from(PrincipalName {
                name_type: ExplicitContextTag0::from(IntegerAsn1(vec![2])),
                name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![
                    KerberosStringAsn1::from(Ia5String::from_string("krbtgt".to_owned()).unwrap()),
                    KerberosStringAsn1::from(Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap()),
                ])),
            }),
            e_text: Optional::from(Some(ExplicitContextTag11::from(GeneralStringAsn1::from(
                Ia5String::from_string("CLIENT_NOT_FOUND".to_owned()).unwrap(),
            )))),
            e_data: Optional::from(None),
        }));

        let mut d = picky_asn1_der::Deserializer::new_from_bytes(&raw_as_req);
        let krb_result: Result<AsReq, KrbError> = Result::deserialize(&mut d).unwrap();
        assert_eq!(expected_ap_req, krb_result);

        let mut d = picky_asn1_der::Deserializer::new_from_bytes(&raw_error);
        let krb_result: Result<AsReq, KrbError> = Result::deserialize(&mut d).unwrap();
        assert_eq!(expected_error, krb_result);
    }
}
