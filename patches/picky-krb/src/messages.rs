use std::fmt;
use std::io::{self, Read};

use picky_asn1::tag::{TagClass, TagPeeker};
use picky_asn1::wrapper::{
    Asn1SequenceOf, ExplicitContextTag0, ExplicitContextTag1, ExplicitContextTag2, ExplicitContextTag3,
    ExplicitContextTag4, ExplicitContextTag5, ExplicitContextTag6, ExplicitContextTag7, ExplicitContextTag8,
    ExplicitContextTag9, ExplicitContextTag10, ExplicitContextTag11, ExplicitContextTag12, IntegerAsn1,
    OctetStringAsn1, Optional,
};
use picky_asn1_der::application_tag::ApplicationTag;
use picky_asn1_der::{Asn1DerError, Asn1RawDer};
use serde::ser::Error;
use serde::{Deserialize, Serialize, de, ser};

use crate::constants::krb_priv::KRB_PRIV_VERSION;
use crate::constants::types::{
    AP_REP_MSG_TYPE, AP_REQ_MSG_TYPE, AS_REP_MSG_TYPE, AS_REQ_MSG_TYPE, ENC_AS_REP_PART_TYPE, ENC_TGS_REP_PART_TYPE,
    KRB_ERROR_MSG_TYPE, KRB_PRIV, TGS_REP_MSG_TYPE, TGS_REQ_MSG_TYPE,
};
use crate::data_types::{
    ApOptions, EncryptedData, EncryptionKey, HostAddresses, KerberosFlags, KerberosStringAsn1, KerberosTime, LastReq,
    Microseconds, PaData, PrincipalName, Realm, Ticket,
};

/// [2.2.2 KDC_PROXY_MESSAGE](https://docs.microsoft.com/en-us/openspecs/windows_protocols/ms-kkdcp/5778aff5-b182-4b97-a970-29c7f911eef2)
///
/// ```not_rust
/// KDC-PROXY-MESSAGE::= SEQUENCE {
///     kerb-message           [0] OCTET STRING,
///     target-domain          [1] KERB-REALM OPTIONAL,
///     dclocator-hint         [2] INTEGER OPTIONAL
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct KdcProxyMessage {
    pub kerb_message: ExplicitContextTag0<OctetStringAsn1>,
    #[serde(default)]
    pub target_domain: Optional<Option<ExplicitContextTag1<KerberosStringAsn1>>>,
    #[serde(default)]
    pub dclocator_hint: Optional<Option<ExplicitContextTag2<IntegerAsn1>>>,
}

impl KdcProxyMessage {
    pub fn from_raw<R: ?Sized + AsRef<[u8]>>(raw: &R) -> Result<KdcProxyMessage, Asn1DerError> {
        let mut deserializer = picky_asn1_der::Deserializer::new_from_bytes(raw.as_ref());
        KdcProxyMessage::deserialize(&mut deserializer)
    }

    pub fn from_raw_kerb_message<R: ?Sized + AsRef<[u8]>>(
        raw_kerb_message: &R,
    ) -> Result<KdcProxyMessage, Asn1DerError> {
        Ok(KdcProxyMessage {
            kerb_message: ExplicitContextTag0::from(OctetStringAsn1(raw_kerb_message.as_ref().to_vec())),
            target_domain: Optional::from(None),
            dclocator_hint: Optional::from(None),
        })
    }

    pub fn to_vec(&self) -> Result<Vec<u8>, Asn1DerError> {
        picky_asn1_der::to_vec(self)
    }
}

/// [RFC 4120 5.4.1](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// KDCOptions      ::= KerberosFlags
/// KDC-REQ-BODY    ::= SEQUENCE {
///         kdc-options             [0] KDCOptions,
///         cname                   [1] PrincipalName OPTIONAL
///                                     -- Used only in AS-REQ --,
///         realm                   [2] Realm
///                                     -- Server's realm
///                                     -- Also client's in AS-REQ --,
///         sname                   [3] PrincipalName OPTIONAL,
///         from                    [4] KerberosTime OPTIONAL,
///         till                    [5] KerberosTime,
///         rtime                   [6] KerberosTime OPTIONAL,
///         nonce                   [7] UInt32,
///         etype                   [8] SEQUENCE OF Int32 -- EncryptionType
///                                     -- in preference order --,
///         addresses               [9] HostAddresses OPTIONAL,
///         enc-authorization-data  [10] EncryptedData OPTIONAL
///                                     -- AuthorizationData --,
///         additional-tickets      [11] SEQUENCE OF Ticket OPTIONAL
///                                        -- NOTE: not empty
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct KdcReqBody {
    pub kdc_options: ExplicitContextTag0<KerberosFlags>,
    pub cname: Optional<Option<ExplicitContextTag1<PrincipalName>>>,
    pub realm: ExplicitContextTag2<Realm>,
    pub sname: Optional<Option<ExplicitContextTag3<PrincipalName>>>,
    pub from: Optional<Option<ExplicitContextTag4<KerberosTime>>>,
    pub till: ExplicitContextTag5<KerberosTime>,
    pub rtime: Optional<Option<ExplicitContextTag6<KerberosTime>>>,
    pub nonce: ExplicitContextTag7<IntegerAsn1>,
    pub etype: ExplicitContextTag8<Asn1SequenceOf<IntegerAsn1>>,
    #[serde(default)]
    pub addresses: Optional<Option<ExplicitContextTag9<HostAddresses>>>,
    #[serde(default)]
    pub enc_authorization_data: Optional<Option<ExplicitContextTag10<EncryptedData>>>,
    #[serde(default)]
    pub additional_tickets: Optional<Option<ExplicitContextTag11<Asn1SequenceOf<Ticket>>>>,
}

/// [RFC 4120 5.4.1](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// KDC-REQ         ::= SEQUENCE {
///         pvno            [1] INTEGER (5) ,
///         msg-type        [2] INTEGER,
///         padata          [3] SEQUENCE OF PA-DATA OPTIONAL,
///                             -- NOTE: not empty --,
///         req-body        [4] KDC-REQ-BODY,
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct KdcReq {
    pub pvno: ExplicitContextTag1<IntegerAsn1>,
    pub msg_type: ExplicitContextTag2<IntegerAsn1>,
    pub padata: Optional<Option<ExplicitContextTag3<Asn1SequenceOf<PaData>>>>,
    pub req_body: ExplicitContextTag4<KdcReqBody>,
}

/// [RFC 4120 5.4.2](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// AS-REQ          ::= [APPLICATION 10] KDC-REQ
/// ```
pub type AsReq = ApplicationTag<KdcReq, AS_REQ_MSG_TYPE>;

/// [RFC 4120 5.4.2](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// TGS-REQ         ::= [APPLICATION 12] KDC-REQ
/// ```
pub type TgsReq = ApplicationTag<KdcReq, TGS_REQ_MSG_TYPE>;

/// [RFC 4120 5.4.2](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// KDC-REP         ::= SEQUENCE {
///         pvno            [0] INTEGER (5),
///         msg-type        [1] INTEGER (11 -- AS -- | 13 -- TGS --),
///         padata          [2] SEQUENCE OF PA-DATA OPTIONAL
///                                 -- NOTE: not empty --,
///         crealm          [3] Realm,
///         cname           [4] PrincipalName,
///         ticket          [5] Ticket,
///         enc-part        [6] EncryptedData
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct KdcRep {
    pub pvno: ExplicitContextTag0<IntegerAsn1>,
    pub msg_type: ExplicitContextTag1<IntegerAsn1>,
    pub padata: Optional<Option<ExplicitContextTag2<Asn1SequenceOf<PaData>>>>,
    pub crealm: ExplicitContextTag3<Realm>,
    pub cname: ExplicitContextTag4<PrincipalName>,
    pub ticket: ExplicitContextTag5<Ticket>,
    pub enc_part: ExplicitContextTag6<EncryptedData>,
}

/// [RFC 4120 5.4.2](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// AS-REP          ::= [APPLICATION 11] KDC-REP
/// ```
pub type AsRep = ApplicationTag<KdcRep, AS_REP_MSG_TYPE>;

/// [RFC 4120 5.4.2](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// TGS-REP         ::= [APPLICATION 13] KDC-REP
/// ```
pub type TgsRep = ApplicationTag<KdcRep, TGS_REP_MSG_TYPE>;

/// [RFC 4120 5.9.1](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// KRB-ERROR       ::= [APPLICATION 30] SEQUENCE {
///         pvno            [0] INTEGER (5),
///         msg-type        [1] INTEGER (30),
///         ctime           [2] KerberosTime OPTIONAL,
///         cusec           [3] Microseconds OPTIONAL,
///         stime           [4] KerberosTime,
///         susec           [5] Microseconds,
///         error-code      [6] Int32,
///         crealm          [7] Realm OPTIONAL,
///         cname           [8] PrincipalName OPTIONAL,
///         realm           [9] Realm -- service realm --,
///         sname           [10] PrincipalName -- service name --,
///         e-text          [11] KerberosString OPTIONAL,
///         e-data          [12] OCTET STRING OPTIONAL
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct KrbErrorInner {
    pub pvno: ExplicitContextTag0<IntegerAsn1>,
    pub msg_type: ExplicitContextTag1<IntegerAsn1>,
    pub ctime: Optional<Option<ExplicitContextTag2<KerberosTime>>>,
    pub cusec: Optional<Option<ExplicitContextTag3<KerberosTime>>>,
    pub stime: ExplicitContextTag4<KerberosTime>,
    pub susec: ExplicitContextTag5<Microseconds>,
    pub error_code: ExplicitContextTag6<u32>, /* use u32 until we can de/serialize signed integers. error_code should fit in u8. */
    pub crealm: Optional<Option<ExplicitContextTag7<Realm>>>,
    pub cname: Optional<Option<ExplicitContextTag8<PrincipalName>>>,
    pub realm: ExplicitContextTag9<Realm>,
    pub sname: ExplicitContextTag10<PrincipalName>,
    #[serde(default)]
    pub e_text: Optional<Option<ExplicitContextTag11<KerberosStringAsn1>>>,
    #[serde(default)]
    pub e_data: Optional<Option<ExplicitContextTag12<OctetStringAsn1>>>,
}

pub type KrbError = ApplicationTag<KrbErrorInner, KRB_ERROR_MSG_TYPE>;

impl fmt::Display for KrbErrorInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#04X?}", self.error_code)
    }
}

/// [RFC 4120 5.4.2](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// EncKDCRepPart   ::= SEQUENCE {
///         key             [0] EncryptionKey,
///         last-req        [1] LastReq,
///         nonce           [2] UInt32,
///         key-expiration  [3] KerberosTime OPTIONAL,
///         flags           [4] TicketFlags,
///         authtime        [5] KerberosTime,
///         starttime       [6] KerberosTime OPTIONAL,
///         endtime         [7] KerberosTime,
///         renew-till      [8] KerberosTime OPTIONAL,
///         srealm          [9] Realm,
///         sname           [10] PrincipalName,
///         caddr           [11] HostAddresses OPTIONAL
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct EncKdcRepPart {
    pub key: ExplicitContextTag0<EncryptionKey>,
    pub last_req: ExplicitContextTag1<LastReq>,
    pub nonce: ExplicitContextTag2<IntegerAsn1>,
    pub key_expiration: Optional<Option<ExplicitContextTag3<KerberosTime>>>,
    pub flags: ExplicitContextTag4<KerberosFlags>,
    pub auth_time: ExplicitContextTag5<KerberosTime>,
    pub start_time: Optional<Option<ExplicitContextTag6<KerberosTime>>>,
    pub end_time: ExplicitContextTag7<KerberosTime>,
    pub renew_till: Optional<Option<ExplicitContextTag8<KerberosTime>>>,
    pub srealm: ExplicitContextTag9<Realm>,
    pub sname: ExplicitContextTag10<PrincipalName>,
    #[serde(default)]
    pub caddr: Optional<Option<ExplicitContextTag11<HostAddresses>>>,
    // this field is not specified in RFC but present in real tickets
    #[serde(default)]
    pub encrypted_pa_data: Optional<Option<ExplicitContextTag12<Asn1SequenceOf<PaData>>>>,
}

/// [RFC 4120 5.4.2](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// EncASRepPart    ::= [APPLICATION 25] EncKDCRepPart
/// ```
pub type EncAsRepPart = ApplicationTag<EncKdcRepPart, ENC_AS_REP_PART_TYPE>;

/// [RFC 4120 5.4.2](https://www.rfc-editor.org/rfc/rfc4120.txt)
///
/// ```not_rust
/// EncTGSRepPart   ::= [APPLICATION 26] EncKDCRepPart
/// ```
pub type EncTgsRepPart = ApplicationTag<EncKdcRepPart, ENC_TGS_REP_PART_TYPE>;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct ApReqInner {
    pub pvno: ExplicitContextTag0<IntegerAsn1>,
    pub msg_type: ExplicitContextTag1<IntegerAsn1>,
    pub ap_options: ExplicitContextTag2<ApOptions>,
    pub ticket: ExplicitContextTag3<Ticket>,
    pub authenticator: ExplicitContextTag4<EncryptedData>,
}

pub type ApReq = ApplicationTag<ApReqInner, AP_REQ_MSG_TYPE>;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct ApRepInner {
    pub pvno: ExplicitContextTag0<IntegerAsn1>,
    pub msg_type: ExplicitContextTag1<IntegerAsn1>,
    pub enc_part: ExplicitContextTag2<EncryptedData>,
}
pub type ApRep = ApplicationTag<ApRepInner, AP_REP_MSG_TYPE>;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct TgtReq {
    pub pvno: ExplicitContextTag0<IntegerAsn1>,
    pub msg_type: ExplicitContextTag1<IntegerAsn1>,
    pub server_name: ExplicitContextTag2<PrincipalName>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct TgtRep {
    pub pvno: ExplicitContextTag0<IntegerAsn1>,
    pub msg_type: ExplicitContextTag1<IntegerAsn1>,
    pub ticket: ExplicitContextTag2<Ticket>,
}

/// [RFC 4120](https://datatracker.ietf.org/doc/html/rfc4120#section-5.7.1)
///
/// ```not_rust
/// KRB-PRIV        ::= [APPLICATION 21] SEQUENCE {
///         pvno            [0] INTEGER (5),
///         msg-type        [1] INTEGER (21),
///                         -- NOTE: there is no [2] tag
///         enc-part        [3] EncryptedData -- EncKrbPrivPart
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct KrbPrivInner {
    pub pvno: ExplicitContextTag0<IntegerAsn1>,
    pub msg_type: ExplicitContextTag1<IntegerAsn1>,
    pub enc_part: ExplicitContextTag3<EncryptedData>,
}

pub type KrbPriv = ApplicationTag<KrbPrivInner, KRB_PRIV>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApMessage {
    ApReq(ApReq),
    ApRep(ApRep),
}

impl<'de> de::Deserialize<'de> for ApMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        use serde::de::Error;

        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = ApMessage;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a valid DER-encoded ApMessage")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let tag_peeker: TagPeeker = seq
                    .next_element()
                    .map_err(|e| A::Error::custom(format!("Cannot deserialize application tag: {:?}", e)))?
                    .ok_or_else(|| A::Error::missing_field("ApplicationTag"))?;

                match tag_peeker.next_tag.class_and_number() {
                    (TagClass::Application, AP_REQ_MSG_TYPE) => seq
                        .next_element()?
                        .ok_or_else(|| A::Error::missing_field("Missing ApMessage::ApReq value"))
                        .map(ApMessage::ApReq),
                    (TagClass::Application, AP_REP_MSG_TYPE) => seq
                        .next_element::<ApRep>()?
                        .ok_or_else(|| A::Error::missing_field("Missing ApMessage::ApRep value"))
                        .map(ApMessage::ApRep),
                    _ => Err(A::Error::custom("Invalid tag for ApMessage. Expected ApReq or ApRep")),
                }
            }
        }

        deserializer.deserialize_enum("ApMessage", &["ApReq", "ApRep"], Visitor)
    }
}

impl ser::Serialize for ApMessage {
    fn serialize<S>(&self, serializer: S) -> Result<<S as ser::Serializer>::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        match self {
            ApMessage::ApReq(ap_req) => ap_req.serialize(serializer),
            ApMessage::ApRep(ap_rep) => ap_rep.serialize(serializer),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KrbPrivMessage {
    pub ap_message: ApMessage,
    pub krb_priv: KrbPriv,
}

impl KrbPrivMessage {
    pub fn deserialize(mut data: impl Read) -> io::Result<Self> {
        let mut buf = [0, 0];

        // read message len
        data.read_exact(&mut buf)?;

        // read version
        data.read_exact(&mut buf)?;

        if buf != KRB_PRIV_VERSION {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid KRB_PRIV_VERSION"));
        }

        // read ap_message len
        data.read_exact(&mut buf)?;

        Ok(Self {
            ap_message: picky_asn1_der::from_reader(&mut data)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("{:?}", err)))?,
            krb_priv: picky_asn1_der::from_reader(&mut data)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("{:?}", err)))?,
        })
    }
}

impl ser::Serialize for KrbPrivMessage {
    fn serialize<S>(&self, serializer: S) -> Result<<S as ser::Serializer>::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        // 2 /* message len */ + 2 /* version */ + 2 /* ap_req len */
        let mut message = vec![0, 0, 0, 0, 0, 0];

        let ap_len = picky_asn1_der::to_writer(&self.ap_message, &mut message)
            .map_err(|err| S::Error::custom(format!("Cannot serialize ap_req: {:?}", err)))?;
        let _krb_priv_len = picky_asn1_der::to_writer(&self.krb_priv, &mut message)
            .map_err(|err| S::Error::custom(format!("Cannot serialize krb_priv: {:?}", err)))?;

        let message_len = message.len();
        debug_assert_eq!(message_len, 6 + ap_len + _krb_priv_len);

        message[0..2].copy_from_slice(&(message_len as u16).to_be_bytes());
        message[2..4].copy_from_slice(&KRB_PRIV_VERSION);
        message[4..6].copy_from_slice(&(ap_len as u16).to_be_bytes());

        Asn1RawDer(message).serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use crate::constants::error_codes::{KDC_ERR_C_PRINCIPAL_UNKNOWN, KRB_AP_ERR_INAPP_CKSUM};
    use crate::data_types::{
        ApOptions, EncryptedData, KerberosFlags, KerberosStringAsn1, KerberosTime, PaData, PrincipalName, Realm,
        Ticket, TicketInner,
    };
    use crate::messages::{
        ApMessage, ApRep, ApRepInner, ApReq, ApReqInner, AsRep, AsReq, KdcProxyMessage, KdcRep, KdcReq, KdcReqBody,
        KrbError, KrbErrorInner, KrbPriv, KrbPrivInner, KrbPrivMessage, TgsReq,
    };

    use picky_asn1::bit_string::BitString;
    use picky_asn1::date::Date;
    use picky_asn1::restricted_string::Ia5String;
    use picky_asn1::wrapper::{
        Asn1SequenceOf, BitStringAsn1, ExplicitContextTag0, ExplicitContextTag1, ExplicitContextTag2,
        ExplicitContextTag3, ExplicitContextTag4, ExplicitContextTag5, ExplicitContextTag6, ExplicitContextTag7,
        ExplicitContextTag8, ExplicitContextTag9, ExplicitContextTag10, ExplicitContextTag11, GeneralStringAsn1,
        GeneralizedTimeAsn1, IntegerAsn1, OctetStringAsn1, Optional,
    };

    #[test]
    fn krb_priv_request_serialization() {
        let expected_raw: [u8; 1307] = [
            5, 27, 0, 1, 4, 177, 110, 130, 4, 173, 48, 130, 4, 169, 160, 3, 2, 1, 5, 161, 3, 2, 1, 14, 162, 7, 3, 5, 0,
            0, 0, 0, 0, 163, 130, 3, 239, 97, 130, 3, 235, 48, 130, 3, 231, 160, 3, 2, 1, 5, 161, 13, 27, 11, 81, 75,
            65, 84, 73, 79, 78, 46, 67, 79, 77, 162, 29, 48, 27, 160, 3, 2, 1, 2, 161, 20, 48, 18, 27, 6, 107, 97, 100,
            109, 105, 110, 27, 8, 99, 104, 97, 110, 103, 101, 112, 119, 163, 130, 3, 176, 48, 130, 3, 172, 160, 3, 2,
            1, 18, 161, 3, 2, 1, 2, 162, 130, 3, 158, 4, 130, 3, 154, 111, 86, 147, 251, 215, 29, 83, 198, 94, 151,
            136, 27, 199, 220, 167, 21, 228, 231, 25, 42, 81, 220, 76, 168, 63, 248, 7, 70, 229, 251, 160, 184, 86, 31,
            157, 224, 190, 19, 211, 201, 209, 24, 167, 98, 12, 241, 25, 182, 53, 11, 99, 196, 249, 53, 173, 176, 74,
            172, 119, 219, 208, 49, 102, 187, 219, 120, 216, 79, 219, 180, 37, 148, 9, 49, 176, 79, 173, 231, 16, 10,
            70, 91, 224, 15, 147, 41, 26, 219, 156, 26, 157, 205, 245, 187, 33, 230, 179, 158, 205, 24, 253, 79, 208,
            202, 252, 12, 75, 87, 52, 215, 230, 3, 251, 162, 76, 108, 120, 154, 32, 56, 125, 168, 237, 218, 135, 37,
            124, 255, 240, 248, 167, 19, 139, 155, 171, 119, 215, 146, 149, 91, 154, 92, 161, 55, 139, 240, 41, 45,
            227, 174, 33, 178, 232, 230, 206, 50, 46, 129, 239, 109, 213, 239, 6, 90, 127, 222, 5, 182, 247, 168, 220,
            223, 86, 45, 68, 203, 170, 207, 19, 96, 229, 83, 104, 23, 181, 138, 67, 233, 113, 157, 241, 36, 59, 138,
            246, 251, 65, 211, 244, 165, 103, 42, 22, 90, 114, 181, 192, 151, 243, 27, 92, 158, 53, 36, 123, 127, 169,
            180, 48, 248, 18, 212, 171, 213, 206, 94, 202, 11, 154, 23, 82, 130, 155, 130, 78, 26, 197, 35, 159, 150,
            212, 239, 164, 0, 38, 70, 186, 110, 224, 64, 204, 185, 29, 203, 222, 74, 242, 252, 169, 128, 193, 18, 234,
            135, 122, 150, 170, 34, 73, 110, 146, 113, 169, 218, 111, 44, 233, 214, 160, 174, 63, 181, 216, 75, 36, 79,
            135, 173, 110, 247, 130, 174, 242, 234, 54, 107, 90, 166, 111, 219, 2, 6, 25, 48, 11, 163, 205, 148, 228,
            40, 251, 91, 34, 132, 119, 63, 127, 73, 38, 20, 40, 245, 172, 219, 91, 241, 57, 177, 142, 229, 138, 45, 56,
            237, 121, 14, 26, 196, 162, 25, 86, 3, 255, 45, 140, 209, 254, 49, 253, 24, 133, 222, 212, 18, 229, 142,
            70, 174, 13, 30, 0, 201, 94, 205, 160, 20, 73, 220, 154, 82, 154, 155, 1, 230, 99, 201, 174, 233, 137, 250,
            17, 126, 74, 101, 35, 141, 255, 9, 112, 169, 100, 181, 145, 76, 11, 4, 129, 146, 195, 171, 182, 38, 134,
            113, 8, 113, 246, 87, 91, 199, 88, 128, 5, 133, 112, 221, 205, 85, 5, 58, 218, 140, 176, 219, 84, 76, 7,
            56, 106, 64, 90, 230, 179, 220, 137, 76, 140, 98, 189, 28, 77, 197, 145, 247, 137, 76, 103, 173, 198, 22,
            5, 124, 57, 192, 250, 30, 51, 66, 164, 213, 142, 4, 46, 102, 21, 95, 200, 105, 200, 95, 1, 33, 186, 246,
            202, 93, 252, 98, 85, 145, 128, 251, 90, 171, 185, 201, 96, 253, 245, 44, 166, 89, 68, 216, 204, 200, 198,
            179, 61, 172, 154, 49, 195, 199, 11, 108, 13, 212, 45, 73, 76, 89, 76, 164, 24, 170, 90, 227, 50, 3, 187,
            255, 44, 98, 180, 192, 80, 119, 197, 208, 130, 192, 64, 103, 186, 162, 217, 163, 190, 232, 242, 56, 50, 42,
            88, 205, 37, 77, 97, 241, 226, 183, 62, 95, 15, 251, 92, 143, 212, 64, 106, 136, 42, 181, 112, 207, 0, 92,
            18, 125, 20, 170, 210, 106, 60, 170, 41, 254, 255, 119, 66, 41, 213, 154, 218, 103, 222, 96, 123, 214, 80,
            98, 59, 178, 145, 188, 72, 177, 42, 255, 246, 253, 54, 115, 195, 151, 83, 14, 51, 191, 16, 132, 112, 166,
            246, 219, 53, 180, 189, 89, 160, 160, 201, 132, 244, 22, 18, 173, 22, 128, 131, 194, 27, 98, 110, 55, 107,
            253, 240, 0, 137, 250, 58, 41, 181, 175, 206, 130, 86, 204, 167, 194, 192, 228, 108, 108, 134, 33, 123, 3,
            108, 77, 41, 148, 50, 144, 140, 91, 230, 111, 97, 125, 79, 120, 72, 0, 152, 216, 59, 231, 77, 52, 225, 131,
            116, 72, 0, 19, 36, 249, 135, 233, 85, 165, 212, 79, 240, 127, 66, 227, 15, 151, 174, 206, 199, 75, 159,
            211, 235, 209, 59, 107, 212, 34, 230, 51, 18, 140, 35, 148, 168, 81, 71, 128, 20, 43, 39, 255, 18, 218,
            221, 98, 11, 74, 170, 141, 226, 128, 21, 34, 111, 105, 174, 49, 209, 49, 174, 252, 181, 199, 118, 102, 43,
            208, 9, 150, 115, 62, 224, 237, 137, 101, 232, 100, 139, 26, 237, 132, 219, 148, 98, 210, 51, 107, 136, 52,
            57, 222, 161, 98, 37, 34, 35, 109, 188, 106, 57, 232, 184, 104, 47, 170, 113, 37, 134, 225, 226, 251, 125,
            169, 239, 139, 215, 45, 105, 60, 150, 213, 35, 124, 198, 206, 37, 126, 154, 62, 174, 103, 44, 114, 218,
            138, 151, 220, 13, 218, 118, 194, 132, 83, 48, 253, 237, 96, 166, 246, 32, 120, 45, 81, 241, 204, 109, 65,
            113, 234, 53, 217, 173, 141, 233, 47, 71, 120, 110, 225, 163, 177, 62, 240, 235, 149, 57, 251, 231, 133,
            87, 215, 146, 102, 180, 171, 32, 137, 163, 53, 173, 93, 83, 179, 87, 33, 170, 138, 106, 202, 200, 129, 236,
            212, 65, 198, 209, 195, 220, 242, 90, 231, 185, 60, 8, 97, 157, 177, 2, 125, 79, 61, 238, 32, 159, 251, 53,
            21, 6, 114, 17, 240, 210, 143, 17, 177, 164, 129, 160, 48, 129, 157, 160, 3, 2, 1, 18, 162, 129, 149, 4,
            129, 146, 90, 166, 167, 142, 169, 235, 19, 170, 76, 205, 171, 40, 64, 12, 143, 135, 71, 73, 14, 96, 195,
            162, 246, 230, 85, 140, 39, 30, 9, 52, 131, 245, 101, 73, 138, 126, 219, 118, 5, 124, 107, 163, 52, 55,
            158, 56, 102, 190, 51, 87, 39, 34, 138, 125, 67, 208, 4, 99, 208, 29, 154, 243, 69, 93, 178, 233, 175, 232,
            139, 133, 186, 106, 29, 105, 5, 71, 188, 181, 141, 140, 220, 31, 11, 19, 153, 53, 144, 159, 12, 59, 26, 29,
            4, 30, 180, 32, 152, 198, 203, 144, 77, 149, 29, 141, 66, 223, 190, 6, 247, 162, 40, 15, 96, 127, 117, 230,
            60, 37, 53, 169, 156, 179, 9, 161, 154, 4, 245, 236, 76, 93, 132, 236, 29, 64, 66, 99, 239, 20, 245, 115,
            173, 214, 16, 19, 44, 74, 117, 98, 48, 96, 160, 3, 2, 1, 5, 161, 3, 2, 1, 21, 163, 84, 48, 82, 160, 3, 2,
            1, 18, 162, 75, 4, 73, 251, 203, 165, 65, 153, 175, 145, 139, 228, 247, 183, 60, 132, 84, 138, 176, 207,
            39, 144, 229, 155, 61, 235, 203, 48, 225, 81, 200, 141, 154, 169, 86, 173, 136, 255, 17, 200, 185, 164,
            233, 123, 86, 147, 182, 0, 66, 194, 77, 248, 33, 51, 10, 48, 206, 216, 214, 47, 12, 39, 238, 115, 28, 137,
            254, 178, 188, 52, 173, 216, 110, 145, 49, 159,
        ];
        let expected = KrbPrivMessage {
            ap_message: ApMessage::ApReq(ApReq::from(ApReqInner {
                pvno: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x05])),
                msg_type: ExplicitContextTag1::from(IntegerAsn1::from(vec![0x0e])),
                ap_options: ExplicitContextTag2::from(ApOptions::from(BitString::with_bytes(vec![0, 0, 0, 0]))),
                ticket: ExplicitContextTag3::from(Ticket::from(TicketInner {
                    tkt_vno: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x05])),
                    realm: ExplicitContextTag1::from(GeneralStringAsn1::from(
                        Ia5String::from_string("QKATION.COM".into()).unwrap(),
                    )),
                    sname: ExplicitContextTag2::from(PrincipalName {
                        name_type: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x02])),
                        name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![
                            GeneralStringAsn1::from(Ia5String::from_string("kadmin".into()).unwrap()),
                            GeneralStringAsn1::from(Ia5String::from_string("changepw".into()).unwrap()),
                        ])),
                    }),
                    enc_part: ExplicitContextTag3::from(EncryptedData {
                        etype: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x12])),
                        kvno: Optional::from(Some(ExplicitContextTag1::from(IntegerAsn1::from(vec![0x02])))),
                        cipher: ExplicitContextTag2::from(OctetStringAsn1::from(vec![
                            111, 86, 147, 251, 215, 29, 83, 198, 94, 151, 136, 27, 199, 220, 167, 21, 228, 231, 25, 42,
                            81, 220, 76, 168, 63, 248, 7, 70, 229, 251, 160, 184, 86, 31, 157, 224, 190, 19, 211, 201,
                            209, 24, 167, 98, 12, 241, 25, 182, 53, 11, 99, 196, 249, 53, 173, 176, 74, 172, 119, 219,
                            208, 49, 102, 187, 219, 120, 216, 79, 219, 180, 37, 148, 9, 49, 176, 79, 173, 231, 16, 10,
                            70, 91, 224, 15, 147, 41, 26, 219, 156, 26, 157, 205, 245, 187, 33, 230, 179, 158, 205, 24,
                            253, 79, 208, 202, 252, 12, 75, 87, 52, 215, 230, 3, 251, 162, 76, 108, 120, 154, 32, 56,
                            125, 168, 237, 218, 135, 37, 124, 255, 240, 248, 167, 19, 139, 155, 171, 119, 215, 146,
                            149, 91, 154, 92, 161, 55, 139, 240, 41, 45, 227, 174, 33, 178, 232, 230, 206, 50, 46, 129,
                            239, 109, 213, 239, 6, 90, 127, 222, 5, 182, 247, 168, 220, 223, 86, 45, 68, 203, 170, 207,
                            19, 96, 229, 83, 104, 23, 181, 138, 67, 233, 113, 157, 241, 36, 59, 138, 246, 251, 65, 211,
                            244, 165, 103, 42, 22, 90, 114, 181, 192, 151, 243, 27, 92, 158, 53, 36, 123, 127, 169,
                            180, 48, 248, 18, 212, 171, 213, 206, 94, 202, 11, 154, 23, 82, 130, 155, 130, 78, 26, 197,
                            35, 159, 150, 212, 239, 164, 0, 38, 70, 186, 110, 224, 64, 204, 185, 29, 203, 222, 74, 242,
                            252, 169, 128, 193, 18, 234, 135, 122, 150, 170, 34, 73, 110, 146, 113, 169, 218, 111, 44,
                            233, 214, 160, 174, 63, 181, 216, 75, 36, 79, 135, 173, 110, 247, 130, 174, 242, 234, 54,
                            107, 90, 166, 111, 219, 2, 6, 25, 48, 11, 163, 205, 148, 228, 40, 251, 91, 34, 132, 119,
                            63, 127, 73, 38, 20, 40, 245, 172, 219, 91, 241, 57, 177, 142, 229, 138, 45, 56, 237, 121,
                            14, 26, 196, 162, 25, 86, 3, 255, 45, 140, 209, 254, 49, 253, 24, 133, 222, 212, 18, 229,
                            142, 70, 174, 13, 30, 0, 201, 94, 205, 160, 20, 73, 220, 154, 82, 154, 155, 1, 230, 99,
                            201, 174, 233, 137, 250, 17, 126, 74, 101, 35, 141, 255, 9, 112, 169, 100, 181, 145, 76,
                            11, 4, 129, 146, 195, 171, 182, 38, 134, 113, 8, 113, 246, 87, 91, 199, 88, 128, 5, 133,
                            112, 221, 205, 85, 5, 58, 218, 140, 176, 219, 84, 76, 7, 56, 106, 64, 90, 230, 179, 220,
                            137, 76, 140, 98, 189, 28, 77, 197, 145, 247, 137, 76, 103, 173, 198, 22, 5, 124, 57, 192,
                            250, 30, 51, 66, 164, 213, 142, 4, 46, 102, 21, 95, 200, 105, 200, 95, 1, 33, 186, 246,
                            202, 93, 252, 98, 85, 145, 128, 251, 90, 171, 185, 201, 96, 253, 245, 44, 166, 89, 68, 216,
                            204, 200, 198, 179, 61, 172, 154, 49, 195, 199, 11, 108, 13, 212, 45, 73, 76, 89, 76, 164,
                            24, 170, 90, 227, 50, 3, 187, 255, 44, 98, 180, 192, 80, 119, 197, 208, 130, 192, 64, 103,
                            186, 162, 217, 163, 190, 232, 242, 56, 50, 42, 88, 205, 37, 77, 97, 241, 226, 183, 62, 95,
                            15, 251, 92, 143, 212, 64, 106, 136, 42, 181, 112, 207, 0, 92, 18, 125, 20, 170, 210, 106,
                            60, 170, 41, 254, 255, 119, 66, 41, 213, 154, 218, 103, 222, 96, 123, 214, 80, 98, 59, 178,
                            145, 188, 72, 177, 42, 255, 246, 253, 54, 115, 195, 151, 83, 14, 51, 191, 16, 132, 112,
                            166, 246, 219, 53, 180, 189, 89, 160, 160, 201, 132, 244, 22, 18, 173, 22, 128, 131, 194,
                            27, 98, 110, 55, 107, 253, 240, 0, 137, 250, 58, 41, 181, 175, 206, 130, 86, 204, 167, 194,
                            192, 228, 108, 108, 134, 33, 123, 3, 108, 77, 41, 148, 50, 144, 140, 91, 230, 111, 97, 125,
                            79, 120, 72, 0, 152, 216, 59, 231, 77, 52, 225, 131, 116, 72, 0, 19, 36, 249, 135, 233, 85,
                            165, 212, 79, 240, 127, 66, 227, 15, 151, 174, 206, 199, 75, 159, 211, 235, 209, 59, 107,
                            212, 34, 230, 51, 18, 140, 35, 148, 168, 81, 71, 128, 20, 43, 39, 255, 18, 218, 221, 98,
                            11, 74, 170, 141, 226, 128, 21, 34, 111, 105, 174, 49, 209, 49, 174, 252, 181, 199, 118,
                            102, 43, 208, 9, 150, 115, 62, 224, 237, 137, 101, 232, 100, 139, 26, 237, 132, 219, 148,
                            98, 210, 51, 107, 136, 52, 57, 222, 161, 98, 37, 34, 35, 109, 188, 106, 57, 232, 184, 104,
                            47, 170, 113, 37, 134, 225, 226, 251, 125, 169, 239, 139, 215, 45, 105, 60, 150, 213, 35,
                            124, 198, 206, 37, 126, 154, 62, 174, 103, 44, 114, 218, 138, 151, 220, 13, 218, 118, 194,
                            132, 83, 48, 253, 237, 96, 166, 246, 32, 120, 45, 81, 241, 204, 109, 65, 113, 234, 53, 217,
                            173, 141, 233, 47, 71, 120, 110, 225, 163, 177, 62, 240, 235, 149, 57, 251, 231, 133, 87,
                            215, 146, 102, 180, 171, 32, 137, 163, 53, 173, 93, 83, 179, 87, 33, 170, 138, 106, 202,
                            200, 129, 236, 212, 65, 198, 209, 195, 220, 242, 90, 231, 185, 60, 8, 97, 157, 177, 2, 125,
                            79, 61, 238, 32, 159, 251, 53, 21, 6, 114, 17, 240, 210, 143, 17, 177,
                        ])),
                    }),
                })),
                authenticator: ExplicitContextTag4::from(EncryptedData {
                    etype: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x12])),
                    kvno: Optional::from(None),
                    cipher: ExplicitContextTag2::from(OctetStringAsn1::from(vec![
                        90, 166, 167, 142, 169, 235, 19, 170, 76, 205, 171, 40, 64, 12, 143, 135, 71, 73, 14, 96, 195,
                        162, 246, 230, 85, 140, 39, 30, 9, 52, 131, 245, 101, 73, 138, 126, 219, 118, 5, 124, 107, 163,
                        52, 55, 158, 56, 102, 190, 51, 87, 39, 34, 138, 125, 67, 208, 4, 99, 208, 29, 154, 243, 69, 93,
                        178, 233, 175, 232, 139, 133, 186, 106, 29, 105, 5, 71, 188, 181, 141, 140, 220, 31, 11, 19,
                        153, 53, 144, 159, 12, 59, 26, 29, 4, 30, 180, 32, 152, 198, 203, 144, 77, 149, 29, 141, 66,
                        223, 190, 6, 247, 162, 40, 15, 96, 127, 117, 230, 60, 37, 53, 169, 156, 179, 9, 161, 154, 4,
                        245, 236, 76, 93, 132, 236, 29, 64, 66, 99, 239, 20, 245, 115, 173, 214, 16, 19, 44, 74,
                    ])),
                }),
            })),
            krb_priv: KrbPriv::from(KrbPrivInner {
                pvno: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x05])),
                msg_type: ExplicitContextTag1::from(IntegerAsn1::from(vec![0x15])),
                enc_part: ExplicitContextTag3::from(EncryptedData {
                    etype: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x12])),
                    kvno: Optional::from(None),
                    cipher: ExplicitContextTag2::from(OctetStringAsn1::from(vec![
                        251, 203, 165, 65, 153, 175, 145, 139, 228, 247, 183, 60, 132, 84, 138, 176, 207, 39, 144, 229,
                        155, 61, 235, 203, 48, 225, 81, 200, 141, 154, 169, 86, 173, 136, 255, 17, 200, 185, 164, 233,
                        123, 86, 147, 182, 0, 66, 194, 77, 248, 33, 51, 10, 48, 206, 216, 214, 47, 12, 39, 238, 115,
                        28, 137, 254, 178, 188, 52, 173, 216, 110, 145, 49, 159,
                    ])),
                }),
            }),
        };

        let krb_priv_request = KrbPrivMessage::deserialize(&expected_raw as &[u8]).unwrap();
        let raw_krb_priv_request = picky_asn1_der::to_vec(&krb_priv_request).unwrap();

        assert_eq!(krb_priv_request, expected);
        assert_eq!(raw_krb_priv_request, expected_raw);
    }

    #[test]
    fn krb_priv_response_deserialization() {
        let expected_raw: [u8; 171] = [
            0, 171, 0, 1, 0, 83, 111, 81, 48, 79, 160, 3, 2, 1, 5, 161, 3, 2, 1, 15, 162, 67, 48, 65, 160, 3, 2, 1, 18,
            162, 58, 4, 56, 218, 123, 236, 220, 101, 47, 45, 70, 106, 25, 67, 106, 10, 200, 237, 233, 168, 230, 209,
            134, 210, 70, 15, 179, 21, 129, 41, 72, 205, 206, 37, 58, 143, 60, 37, 48, 137, 187, 89, 131, 16, 52, 68,
            37, 60, 28, 215, 252, 225, 97, 29, 147, 62, 127, 19, 216, 117, 80, 48, 78, 160, 3, 2, 1, 5, 161, 3, 2, 1,
            21, 163, 66, 48, 64, 160, 3, 2, 1, 18, 162, 57, 4, 55, 244, 180, 210, 36, 82, 20, 173, 202, 122, 213, 65,
            87, 59, 79, 72, 138, 41, 183, 39, 148, 25, 196, 189, 182, 26, 48, 252, 101, 54, 24, 238, 24, 228, 212, 69,
            37, 151, 225, 49, 193, 172, 32, 236, 245, 125, 139, 33, 149, 71, 31, 65, 220, 230, 121, 86,
        ];
        let expected = KrbPrivMessage {
            ap_message: ApMessage::ApRep(ApRep::from(ApRepInner {
                pvno: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x05])),
                msg_type: ExplicitContextTag1::from(IntegerAsn1::from(vec![0x0f])),
                enc_part: ExplicitContextTag2::from(EncryptedData {
                    etype: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x12])),
                    kvno: Optional::from(None),
                    cipher: ExplicitContextTag2::from(OctetStringAsn1::from(vec![
                        218, 123, 236, 220, 101, 47, 45, 70, 106, 25, 67, 106, 10, 200, 237, 233, 168, 230, 209, 134,
                        210, 70, 15, 179, 21, 129, 41, 72, 205, 206, 37, 58, 143, 60, 37, 48, 137, 187, 89, 131, 16,
                        52, 68, 37, 60, 28, 215, 252, 225, 97, 29, 147, 62, 127, 19, 216,
                    ])),
                }),
            })),
            krb_priv: KrbPriv::from(KrbPrivInner {
                pvno: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x05])),
                msg_type: ExplicitContextTag1::from(IntegerAsn1::from(vec![0x15])),
                enc_part: ExplicitContextTag3::from(EncryptedData {
                    etype: ExplicitContextTag0::from(IntegerAsn1::from(vec![0x12])),
                    kvno: Optional::from(None),
                    cipher: ExplicitContextTag2::from(OctetStringAsn1::from(vec![
                        244, 180, 210, 36, 82, 20, 173, 202, 122, 213, 65, 87, 59, 79, 72, 138, 41, 183, 39, 148, 25,
                        196, 189, 182, 26, 48, 252, 101, 54, 24, 238, 24, 228, 212, 69, 37, 151, 225, 49, 193, 172, 32,
                        236, 245, 125, 139, 33, 149, 71, 31, 65, 220, 230, 121, 86,
                    ])),
                }),
            }),
        };

        let krb_priv_response = KrbPrivMessage::deserialize(&expected_raw as &[u8]).unwrap();
        let raw_krb_priv_response = picky_asn1_der::to_vec(&krb_priv_response).unwrap();

        assert_eq!(expected, krb_priv_response);
        assert_eq!(raw_krb_priv_response, expected_raw);
    }

    #[test]
    fn kdc_proxy_message() {
        let expected_raw = [
            0x30, 0x81, 0xd1, 0xa0, 0x81, 0xbf, 0x04, 0x81, 0xbc, 0x00, 0x00, 0x00, 0xb8, 0x6a, 0x81, 0xb5, 0x30, 0x81,
            0xb2, 0xa1, 0x03, 0x02, 0x01, 0x05, 0xa2, 0x03, 0x02, 0x01, 0x0a, 0xa3, 0x1a, 0x30, 0x18, 0x30, 0x0a, 0xa1,
            0x04, 0x02, 0x02, 0x00, 0x96, 0xa2, 0x02, 0x04, 0x00, 0x30, 0x0a, 0xa1, 0x04, 0x02, 0x02, 0x00, 0x95, 0xa2,
            0x02, 0x04, 0x00, 0xa4, 0x81, 0x89, 0x30, 0x81, 0x86, 0xa0, 0x07, 0x03, 0x05, 0x00, 0x00, 0x00, 0x00, 0x10,
            0xa1, 0x13, 0x30, 0x11, 0xa0, 0x03, 0x02, 0x01, 0x01, 0xa1, 0x0a, 0x30, 0x08, 0x1b, 0x06, 0x6d, 0x79, 0x75,
            0x73, 0x65, 0x72, 0xa2, 0x0d, 0x1b, 0x0b, 0x45, 0x58, 0x41, 0x4d, 0x50, 0x4c, 0x45, 0x2e, 0x43, 0x4f, 0x4d,
            0xa3, 0x20, 0x30, 0x1e, 0xa0, 0x03, 0x02, 0x01, 0x02, 0xa1, 0x17, 0x30, 0x15, 0x1b, 0x06, 0x6b, 0x72, 0x62,
            0x74, 0x67, 0x74, 0x1b, 0x0b, 0x45, 0x58, 0x41, 0x4d, 0x50, 0x4c, 0x45, 0x2e, 0x43, 0x4f, 0x4d, 0xa5, 0x11,
            0x18, 0x0f, 0x32, 0x30, 0x32, 0x31, 0x31, 0x32, 0x31, 0x36, 0x31, 0x38, 0x35, 0x35, 0x31, 0x30, 0x5a, 0xa7,
            0x06, 0x02, 0x04, 0x22, 0x33, 0xc9, 0xe9, 0xa8, 0x1a, 0x30, 0x18, 0x02, 0x01, 0x12, 0x02, 0x01, 0x11, 0x02,
            0x01, 0x14, 0x02, 0x01, 0x13, 0x02, 0x01, 0x10, 0x02, 0x01, 0x17, 0x02, 0x01, 0x19, 0x02, 0x01, 0x1a, 0xa1,
            0x0d, 0x1b, 0x0b, 0x45, 0x58, 0x41, 0x4d, 0x50, 0x4c, 0x45, 0x2e, 0x43, 0x4f, 0x4d,
        ];

        let expected = KdcProxyMessage {
            kerb_message: ExplicitContextTag0::from(OctetStringAsn1::from(vec![
                0, 0, 0, 184, 106, 129, 181, 48, 129, 178, 161, 3, 2, 1, 5, 162, 3, 2, 1, 10, 163, 26, 48, 24, 48, 10,
                161, 4, 2, 2, 0, 150, 162, 2, 4, 0, 48, 10, 161, 4, 2, 2, 0, 149, 162, 2, 4, 0, 164, 129, 137, 48, 129,
                134, 160, 7, 3, 5, 0, 0, 0, 0, 16, 161, 19, 48, 17, 160, 3, 2, 1, 1, 161, 10, 48, 8, 27, 6, 109, 121,
                117, 115, 101, 114, 162, 13, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 163, 32, 48, 30, 160,
                3, 2, 1, 2, 161, 23, 48, 21, 27, 6, 107, 114, 98, 116, 103, 116, 27, 11, 69, 88, 65, 77, 80, 76, 69,
                46, 67, 79, 77, 165, 17, 24, 15, 50, 48, 50, 49, 49, 50, 49, 54, 49, 56, 53, 53, 49, 48, 90, 167, 6, 2,
                4, 34, 51, 201, 233, 168, 26, 48, 24, 2, 1, 18, 2, 1, 17, 2, 1, 20, 2, 1, 19, 2, 1, 16, 2, 1, 23, 2, 1,
                25, 2, 1, 26,
            ])),
            target_domain: Optional::from(Some(ExplicitContextTag1::from(GeneralStringAsn1::from(
                Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap(),
            )))),
            dclocator_hint: Optional::from(None),
        };

        let message: KdcProxyMessage = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let message_raw = picky_asn1_der::to_vec(&message).unwrap();

        assert_eq!(message, expected);
        assert_eq!(message_raw, expected_raw);
    }

    #[test]
    fn kdc_req() {
        let expected_raw = vec![
            48, 129, 178, 161, 3, 2, 1, 5, 162, 3, 2, 1, 10, 163, 26, 48, 24, 48, 10, 161, 4, 2, 2, 0, 150, 162, 2, 4,
            0, 48, 10, 161, 4, 2, 2, 0, 149, 162, 2, 4, 0, 164, 129, 137, 48, 129, 134, 160, 7, 3, 5, 0, 0, 0, 0, 16,
            161, 19, 48, 17, 160, 3, 2, 1, 1, 161, 10, 48, 8, 27, 6, 109, 121, 117, 115, 101, 114, 162, 13, 27, 11, 69,
            88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 163, 32, 48, 30, 160, 3, 2, 1, 2, 161, 23, 48, 21, 27, 6, 107, 114,
            98, 116, 103, 116, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 165, 17, 24, 15, 50, 48, 50, 49, 49,
            50, 50, 52, 50, 49, 49, 55, 51, 51, 90, 167, 6, 2, 4, 73, 141, 213, 43, 168, 26, 48, 24, 2, 1, 18, 2, 1,
            17, 2, 1, 20, 2, 1, 19, 2, 1, 16, 2, 1, 23, 2, 1, 25, 2, 1, 26,
        ];

        let expected = KdcReq {
            pvno: ExplicitContextTag1::from(IntegerAsn1(vec![5])),
            msg_type: ExplicitContextTag2::from(IntegerAsn1(vec![10])),
            padata: Optional::from(Some(ExplicitContextTag3::from(Asn1SequenceOf(vec![
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
                till: ExplicitContextTag5::from(KerberosTime::from(Date::new(2021, 12, 24, 21, 17, 33).unwrap())),
                rtime: Optional::from(None),
                nonce: ExplicitContextTag7::from(IntegerAsn1(vec![73, 141, 213, 43])),
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
        };

        let kdc_req: KdcReq = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let kdc_req_raw = picky_asn1_der::to_vec(&kdc_req).unwrap();

        assert_eq!(expected, kdc_req);
        assert_eq!(expected_raw, kdc_req_raw);
    }

    #[test]
    fn as_req() {
        let expected_raw = vec![
            106, 129, 181, 48, 129, 178, 161, 3, 2, 1, 5, 162, 3, 2, 1, 10, 163, 26, 48, 24, 48, 10, 161, 4, 2, 2, 0,
            150, 162, 2, 4, 0, 48, 10, 161, 4, 2, 2, 0, 149, 162, 2, 4, 0, 164, 129, 137, 48, 129, 134, 160, 7, 3, 5,
            0, 0, 0, 0, 16, 161, 19, 48, 17, 160, 3, 2, 1, 1, 161, 10, 48, 8, 27, 6, 109, 121, 117, 115, 101, 114, 162,
            13, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 163, 32, 48, 30, 160, 3, 2, 1, 2, 161, 23, 48, 21,
            27, 6, 107, 114, 98, 116, 103, 116, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 165, 17, 24, 15,
            50, 48, 50, 49, 49, 50, 50, 57, 49, 48, 51, 54, 48, 54, 90, 167, 6, 2, 4, 29, 32, 235, 11, 168, 26, 48, 24,
            2, 1, 18, 2, 1, 17, 2, 1, 20, 2, 1, 19, 2, 1, 16, 2, 1, 23, 2, 1, 25, 2, 1, 26,
        ];

        let expected = AsReq::from(KdcReq {
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
        });

        let as_req: AsReq = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let as_req_raw = picky_asn1_der::to_vec(&as_req).unwrap();

        assert_eq!(expected, as_req);
        assert_eq!(expected_raw, as_req_raw);
    }

    #[test]
    fn as_rep() {
        let expected_raw = vec![
            107, 130, 2, 192, 48, 130, 2, 188, 160, 3, 2, 1, 5, 161, 3, 2, 1, 11, 162, 43, 48, 41, 48, 39, 161, 3, 2,
            1, 19, 162, 32, 4, 30, 48, 28, 48, 26, 160, 3, 2, 1, 18, 161, 19, 27, 17, 69, 88, 65, 77, 80, 76, 69, 46,
            67, 79, 77, 109, 121, 117, 115, 101, 114, 163, 13, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 164,
            19, 48, 17, 160, 3, 2, 1, 1, 161, 10, 48, 8, 27, 6, 109, 121, 117, 115, 101, 114, 165, 130, 1, 64, 97, 130,
            1, 60, 48, 130, 1, 56, 160, 3, 2, 1, 5, 161, 13, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 162,
            32, 48, 30, 160, 3, 2, 1, 2, 161, 23, 48, 21, 27, 6, 107, 114, 98, 116, 103, 116, 27, 11, 69, 88, 65, 77,
            80, 76, 69, 46, 67, 79, 77, 163, 129, 255, 48, 129, 252, 160, 3, 2, 1, 18, 161, 3, 2, 1, 1, 162, 129, 239,
            4, 129, 236, 229, 108, 127, 175, 235, 22, 11, 195, 254, 62, 101, 153, 38, 64, 83, 27, 109, 35, 253, 196,
            59, 21, 69, 124, 36, 145, 117, 98, 146, 80, 179, 3, 37, 191, 32, 69, 182, 19, 45, 245, 225, 205, 40, 33,
            245, 64, 96, 250, 167, 233, 4, 72, 222, 172, 23, 0, 66, 223, 108, 229, 56, 177, 9, 85, 252, 15, 249, 242,
            189, 240, 4, 45, 235, 72, 169, 207, 81, 60, 129, 61, 66, 191, 142, 254, 11, 231, 111, 219, 21, 155, 126,
            70, 20, 99, 169, 235, 134, 171, 70, 71, 238, 136, 156, 165, 46, 170, 53, 25, 233, 107, 78, 36, 141, 183,
            78, 123, 45, 239, 14, 239, 119, 178, 115, 146, 115, 93, 240, 130, 198, 225, 13, 175, 99, 71, 193, 252, 183,
            41, 77, 109, 158, 237, 159, 185, 164, 103, 132, 248, 223, 55, 201, 44, 74, 25, 130, 188, 76, 255, 128, 199,
            71, 137, 1, 154, 144, 17, 237, 167, 157, 123, 253, 150, 129, 189, 10, 121, 148, 70, 137, 249, 133, 43, 223,
            160, 250, 202, 175, 15, 6, 199, 177, 181, 237, 224, 226, 26, 230, 123, 219, 223, 164, 249, 206, 41, 40, 32,
            190, 14, 3, 196, 163, 41, 56, 118, 157, 114, 87, 233, 89, 178, 246, 74, 224, 43, 207, 53, 131, 32, 78, 111,
            114, 246, 153, 100, 110, 7, 166, 130, 1, 25, 48, 130, 1, 21, 160, 3, 2, 1, 18, 162, 130, 1, 12, 4, 130, 1,
            8, 14, 180, 181, 83, 180, 223, 85, 143, 123, 246, 189, 59, 97, 51, 73, 198, 5, 147, 87, 42, 240, 94, 250,
            203, 240, 45, 46, 190, 32, 135, 13, 24, 123, 127, 223, 30, 53, 200, 226, 164, 80, 207, 227, 34, 63, 139, 3,
            129, 240, 10, 193, 222, 123, 0, 64, 28, 232, 140, 63, 22, 143, 211, 114, 182, 138, 233, 103, 39, 233, 158,
            119, 215, 73, 227, 197, 80, 98, 48, 60, 62, 71, 207, 233, 144, 160, 28, 203, 79, 242, 40, 197, 224, 246,
            84, 9, 184, 188, 250, 231, 190, 97, 255, 41, 234, 238, 213, 203, 3, 192, 160, 220, 78, 78, 197, 45, 255,
            176, 13, 190, 245, 35, 208, 12, 80, 93, 81, 65, 252, 199, 184, 202, 197, 95, 49, 179, 237, 64, 116, 52,
            220, 109, 123, 202, 78, 63, 146, 121, 178, 168, 157, 84, 80, 246, 250, 75, 69, 93, 184, 48, 115, 32, 139,
            4, 90, 164, 30, 208, 100, 37, 220, 168, 165, 2, 224, 124, 102, 164, 130, 34, 66, 134, 131, 16, 7, 206, 32,
            138, 30, 217, 225, 125, 69, 82, 78, 127, 73, 216, 235, 130, 159, 41, 23, 28, 197, 19, 39, 207, 144, 160,
            197, 11, 85, 39, 102, 167, 237, 83, 132, 78, 165, 215, 173, 61, 90, 113, 215, 201, 213, 158, 19, 190, 68,
            135, 94, 136, 63, 105, 119, 225, 127, 193, 148, 33, 74, 41, 154, 68, 104, 52, 227, 188, 19, 62, 26, 55, 15,
            20, 53, 221, 200, 137, 197, 2, 243,
        ];

        let expected = AsRep::from(KdcRep {
            pvno: ExplicitContextTag0::from(IntegerAsn1(vec![5])),
            msg_type: ExplicitContextTag1::from(IntegerAsn1(vec![11])),
            padata: Optional::from(Some(ExplicitContextTag2::from(Asn1SequenceOf::from(vec![PaData {
                padata_type: ExplicitContextTag1::from(IntegerAsn1(vec![19])),
                padata_data: ExplicitContextTag2::from(OctetStringAsn1(vec![
                    48, 28, 48, 26, 160, 3, 2, 1, 18, 161, 19, 27, 17, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 109,
                    121, 117, 115, 101, 114,
                ])),
            }])))),
            crealm: ExplicitContextTag3::from(GeneralStringAsn1::from(
                Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap(),
            )),
            cname: ExplicitContextTag4::from(PrincipalName {
                name_type: ExplicitContextTag0::from(IntegerAsn1(vec![1])),
                name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![GeneralStringAsn1::from(
                    Ia5String::from_string("myuser".to_owned()).unwrap(),
                )])),
            }),
            ticket: ExplicitContextTag5::from(Ticket::from(TicketInner {
                tkt_vno: ExplicitContextTag0::from(IntegerAsn1(vec![5])),
                realm: ExplicitContextTag1::from(GeneralStringAsn1::from(
                    Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap(),
                )),
                sname: ExplicitContextTag2::from(PrincipalName {
                    name_type: ExplicitContextTag0::from(IntegerAsn1(vec![2])),
                    name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![
                        KerberosStringAsn1::from(Ia5String::from_string("krbtgt".to_owned()).unwrap()),
                        KerberosStringAsn1::from(Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap()),
                    ])),
                }),
                enc_part: ExplicitContextTag3::from(EncryptedData {
                    etype: ExplicitContextTag0::from(IntegerAsn1(vec![18])),
                    kvno: Optional::from(Some(ExplicitContextTag1::from(IntegerAsn1(vec![1])))),
                    cipher: ExplicitContextTag2::from(OctetStringAsn1::from(vec![
                        229, 108, 127, 175, 235, 22, 11, 195, 254, 62, 101, 153, 38, 64, 83, 27, 109, 35, 253, 196, 59,
                        21, 69, 124, 36, 145, 117, 98, 146, 80, 179, 3, 37, 191, 32, 69, 182, 19, 45, 245, 225, 205,
                        40, 33, 245, 64, 96, 250, 167, 233, 4, 72, 222, 172, 23, 0, 66, 223, 108, 229, 56, 177, 9, 85,
                        252, 15, 249, 242, 189, 240, 4, 45, 235, 72, 169, 207, 81, 60, 129, 61, 66, 191, 142, 254, 11,
                        231, 111, 219, 21, 155, 126, 70, 20, 99, 169, 235, 134, 171, 70, 71, 238, 136, 156, 165, 46,
                        170, 53, 25, 233, 107, 78, 36, 141, 183, 78, 123, 45, 239, 14, 239, 119, 178, 115, 146, 115,
                        93, 240, 130, 198, 225, 13, 175, 99, 71, 193, 252, 183, 41, 77, 109, 158, 237, 159, 185, 164,
                        103, 132, 248, 223, 55, 201, 44, 74, 25, 130, 188, 76, 255, 128, 199, 71, 137, 1, 154, 144, 17,
                        237, 167, 157, 123, 253, 150, 129, 189, 10, 121, 148, 70, 137, 249, 133, 43, 223, 160, 250,
                        202, 175, 15, 6, 199, 177, 181, 237, 224, 226, 26, 230, 123, 219, 223, 164, 249, 206, 41, 40,
                        32, 190, 14, 3, 196, 163, 41, 56, 118, 157, 114, 87, 233, 89, 178, 246, 74, 224, 43, 207, 53,
                        131, 32, 78, 111, 114, 246, 153, 100, 110, 7,
                    ])),
                }),
            })),
            enc_part: ExplicitContextTag6::from(EncryptedData {
                etype: ExplicitContextTag0::from(IntegerAsn1(vec![18])),
                kvno: Optional::from(None),
                cipher: ExplicitContextTag2::from(OctetStringAsn1::from(vec![
                    14, 180, 181, 83, 180, 223, 85, 143, 123, 246, 189, 59, 97, 51, 73, 198, 5, 147, 87, 42, 240, 94,
                    250, 203, 240, 45, 46, 190, 32, 135, 13, 24, 123, 127, 223, 30, 53, 200, 226, 164, 80, 207, 227,
                    34, 63, 139, 3, 129, 240, 10, 193, 222, 123, 0, 64, 28, 232, 140, 63, 22, 143, 211, 114, 182, 138,
                    233, 103, 39, 233, 158, 119, 215, 73, 227, 197, 80, 98, 48, 60, 62, 71, 207, 233, 144, 160, 28,
                    203, 79, 242, 40, 197, 224, 246, 84, 9, 184, 188, 250, 231, 190, 97, 255, 41, 234, 238, 213, 203,
                    3, 192, 160, 220, 78, 78, 197, 45, 255, 176, 13, 190, 245, 35, 208, 12, 80, 93, 81, 65, 252, 199,
                    184, 202, 197, 95, 49, 179, 237, 64, 116, 52, 220, 109, 123, 202, 78, 63, 146, 121, 178, 168, 157,
                    84, 80, 246, 250, 75, 69, 93, 184, 48, 115, 32, 139, 4, 90, 164, 30, 208, 100, 37, 220, 168, 165,
                    2, 224, 124, 102, 164, 130, 34, 66, 134, 131, 16, 7, 206, 32, 138, 30, 217, 225, 125, 69, 82, 78,
                    127, 73, 216, 235, 130, 159, 41, 23, 28, 197, 19, 39, 207, 144, 160, 197, 11, 85, 39, 102, 167,
                    237, 83, 132, 78, 165, 215, 173, 61, 90, 113, 215, 201, 213, 158, 19, 190, 68, 135, 94, 136, 63,
                    105, 119, 225, 127, 193, 148, 33, 74, 41, 154, 68, 104, 52, 227, 188, 19, 62, 26, 55, 15, 20, 53,
                    221, 200, 137, 197, 2, 243,
                ])),
            }),
        });

        let as_rep: AsRep = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let as_rep_raw = picky_asn1_der::to_vec(&as_rep).unwrap();

        assert_eq!(expected, as_rep);
        assert_eq!(expected_raw, as_rep_raw);
    }

    #[test]
    fn krb_process_tgs_error() {
        let expected_raw = vec![
            126, 129, 146, 48, 129, 143, 160, 3, 2, 1, 5, 161, 3, 2, 1, 30, 164, 17, 24, 15, 50, 48, 50, 49, 49, 50,
            51, 49, 49, 49, 48, 54, 48, 49, 90, 165, 5, 2, 3, 10, 12, 135, 166, 3, 2, 1, 50, 167, 13, 27, 11, 69, 88,
            65, 77, 80, 76, 69, 46, 67, 79, 77, 168, 19, 48, 17, 160, 3, 2, 1, 1, 161, 10, 48, 8, 27, 6, 109, 121, 117,
            115, 101, 114, 169, 13, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 170, 34, 48, 32, 160, 3, 2, 1,
            2, 161, 25, 48, 23, 27, 8, 115, 111, 109, 101, 110, 97, 109, 101, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46,
            67, 79, 77, 171, 13, 27, 11, 80, 82, 79, 67, 69, 83, 83, 95, 84, 71, 83,
        ];

        let expected = KrbError::from(KrbErrorInner {
            pvno: ExplicitContextTag0::from(IntegerAsn1(vec![5])),
            msg_type: ExplicitContextTag1::from(IntegerAsn1(vec![30])),
            ctime: Optional::from(None),
            cusec: Optional::from(None),
            stime: ExplicitContextTag4::from(GeneralizedTimeAsn1::from(Date::new(2021, 12, 31, 11, 6, 1).unwrap())),
            susec: ExplicitContextTag5::from(IntegerAsn1(vec![0x0a, 0x0c, 0x87])),
            error_code: ExplicitContextTag6::from(KRB_AP_ERR_INAPP_CKSUM),
            crealm: Optional::from(Some(ExplicitContextTag7::from(GeneralStringAsn1::from(
                Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap(),
            )))),
            cname: Optional::from(Some(ExplicitContextTag8::from(PrincipalName {
                name_type: ExplicitContextTag0::from(IntegerAsn1(vec![1])),
                name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![GeneralStringAsn1::from(
                    Ia5String::from_string("myuser".to_owned()).unwrap(),
                )])),
            }))),
            realm: ExplicitContextTag9::from(GeneralStringAsn1::from(
                Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap(),
            )),
            sname: ExplicitContextTag10::from(PrincipalName {
                name_type: ExplicitContextTag0::from(IntegerAsn1(vec![2])),
                name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![
                    KerberosStringAsn1::from(Ia5String::from_string("somename".to_owned()).unwrap()),
                    KerberosStringAsn1::from(Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap()),
                ])),
            }),
            e_text: Optional::from(Some(ExplicitContextTag11::from(GeneralStringAsn1::from(
                Ia5String::from_string("PROCESS_TGS".to_owned()).unwrap(),
            )))),
            e_data: Optional::from(None),
        });

        let krb_error: KrbError = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let krb_error_raw = picky_asn1_der::to_vec(&krb_error).unwrap();

        assert_eq!(expected, krb_error);
        assert_eq!(expected_raw, krb_error_raw);
    }

    #[test]
    fn krb_client_not_found_error() {
        let expected_raw = vec![
            126, 129, 151, 48, 129, 148, 160, 3, 2, 1, 5, 161, 3, 2, 1, 30, 164, 17, 24, 15, 50, 48, 50, 49, 49, 50,
            50, 56, 49, 51, 52, 48, 49, 49, 90, 165, 5, 2, 3, 12, 139, 242, 166, 3, 2, 1, 6, 167, 13, 27, 11, 69, 88,
            65, 77, 80, 76, 69, 46, 67, 79, 77, 168, 21, 48, 19, 160, 3, 2, 1, 1, 161, 12, 48, 10, 27, 8, 98, 97, 100,
            95, 117, 115, 101, 114, 169, 13, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 170, 32, 48, 30, 160,
            3, 2, 1, 2, 161, 23, 48, 21, 27, 6, 107, 114, 98, 116, 103, 116, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46,
            67, 79, 77, 171, 18, 27, 16, 67, 76, 73, 69, 78, 84, 95, 78, 79, 84, 95, 70, 79, 85, 78, 68,
        ];

        let expected = KrbError::from(KrbErrorInner {
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
        });

        let krb_error: KrbError = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let krb_error_raw = picky_asn1_der::to_vec(&krb_error).unwrap();

        assert_eq!(expected, krb_error);
        assert_eq!(expected_raw, krb_error_raw);
    }

    #[test]
    fn tgs_req() {
        let expected_raw = vec![
            108, 130, 2, 135, 48, 130, 2, 131, 161, 3, 2, 1, 5, 162, 3, 2, 1, 12, 163, 130, 1, 250, 48, 130, 1, 246,
            48, 130, 1, 242, 161, 3, 2, 1, 1, 162, 130, 1, 233, 4, 130, 1, 229, 110, 130, 1, 225, 48, 130, 1, 221, 160,
            3, 2, 1, 5, 161, 3, 2, 1, 14, 162, 7, 3, 5, 0, 0, 0, 0, 0, 163, 130, 1, 86, 97, 130, 1, 82, 48, 130, 1, 78,
            160, 3, 2, 1, 5, 161, 13, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 162, 32, 48, 30, 160, 3, 2,
            1, 1, 161, 23, 48, 21, 27, 6, 107, 114, 98, 116, 103, 116, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79,
            77, 163, 130, 1, 20, 48, 130, 1, 16, 160, 3, 2, 1, 18, 161, 3, 2, 1, 1, 162, 130, 1, 2, 4, 129, 255, 208,
            37, 251, 184, 33, 173, 54, 72, 142, 105, 213, 119, 99, 50, 12, 51, 85, 130, 118, 156, 163, 115, 233, 59,
            195, 44, 190, 17, 224, 214, 18, 196, 225, 140, 185, 117, 127, 179, 187, 178, 215, 23, 99, 158, 37, 55, 203,
            145, 101, 117, 161, 119, 132, 192, 3, 62, 2, 193, 16, 20, 81, 57, 55, 92, 222, 222, 67, 178, 43, 208, 213,
            126, 246, 84, 110, 105, 43, 225, 82, 89, 197, 129, 46, 145, 185, 12, 10, 53, 77, 142, 155, 59, 149, 88, 5,
            189, 96, 20, 240, 67, 208, 118, 74, 242, 53, 160, 167, 14, 184, 170, 76, 1, 143, 174, 120, 137, 24, 182,
            72, 34, 218, 56, 94, 215, 241, 221, 0, 105, 55, 217, 195, 230, 122, 222, 73, 232, 90, 115, 217, 19, 107,
            33, 181, 111, 217, 150, 142, 86, 183, 108, 2, 197, 131, 57, 170, 221, 162, 206, 147, 93, 6, 226, 156, 179,
            46, 177, 233, 184, 167, 104, 183, 137, 74, 99, 132, 174, 19, 146, 200, 59, 140, 241, 251, 108, 51, 3, 207,
            76, 19, 220, 149, 29, 12, 62, 241, 184, 112, 188, 77, 216, 208, 73, 104, 223, 153, 139, 247, 6, 46, 244,
            75, 106, 181, 233, 188, 184, 81, 247, 123, 231, 46, 139, 176, 204, 31, 18, 0, 222, 43, 113, 4, 64, 92, 63,
            1, 72, 99, 108, 226, 222, 175, 87, 85, 60, 156, 73, 75, 79, 159, 250, 232, 10, 241, 214, 191, 164, 110, 48,
            108, 160, 3, 2, 1, 18, 162, 101, 4, 99, 106, 94, 37, 142, 223, 93, 36, 146, 1, 124, 172, 242, 9, 76, 186,
            171, 5, 77, 225, 43, 160, 252, 253, 38, 235, 37, 210, 141, 117, 149, 90, 1, 37, 130, 188, 5, 244, 120, 135,
            207, 78, 51, 29, 145, 172, 119, 85, 62, 115, 181, 150, 53, 5, 85, 199, 195, 125, 106, 46, 244, 102, 110,
            195, 8, 11, 158, 4, 44, 51, 208, 88, 2, 171, 238, 108, 125, 139, 32, 25, 5, 25, 183, 43, 184, 250, 77, 164,
            24, 65, 247, 150, 138, 86, 57, 81, 74, 201, 60, 151, 164, 121, 48, 119, 160, 7, 3, 5, 0, 64, 129, 0, 16,
            162, 13, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 163, 34, 48, 32, 160, 3, 2, 1, 2, 161, 25, 48,
            23, 27, 8, 115, 111, 109, 101, 110, 97, 109, 101, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 165,
            17, 24, 15, 50, 48, 52, 49, 49, 50, 48, 53, 49, 55, 52, 53, 50, 48, 90, 166, 17, 24, 15, 50, 48, 52, 49,
            49, 50, 48, 53, 49, 55, 52, 53, 50, 48, 90, 167, 6, 2, 4, 74, 26, 112, 174, 168, 11, 48, 9, 2, 1, 18, 2, 1,
            17, 2, 1, 23,
        ];

        let expected = TgsReq::from(KdcReq {
            pvno: ExplicitContextTag1::from(IntegerAsn1(vec![5])),
            msg_type: ExplicitContextTag2::from(IntegerAsn1(vec![12])),
            padata: Optional::from(Some(ExplicitContextTag3::from(Asn1SequenceOf::from(vec![PaData {
                padata_type: ExplicitContextTag1::from(IntegerAsn1(vec![1])),
                padata_data: ExplicitContextTag2::from(OctetStringAsn1::from(vec![
                    110, 130, 1, 225, 48, 130, 1, 221, 160, 3, 2, 1, 5, 161, 3, 2, 1, 14, 162, 7, 3, 5, 0, 0, 0, 0, 0,
                    163, 130, 1, 86, 97, 130, 1, 82, 48, 130, 1, 78, 160, 3, 2, 1, 5, 161, 13, 27, 11, 69, 88, 65, 77,
                    80, 76, 69, 46, 67, 79, 77, 162, 32, 48, 30, 160, 3, 2, 1, 1, 161, 23, 48, 21, 27, 6, 107, 114, 98,
                    116, 103, 116, 27, 11, 69, 88, 65, 77, 80, 76, 69, 46, 67, 79, 77, 163, 130, 1, 20, 48, 130, 1, 16,
                    160, 3, 2, 1, 18, 161, 3, 2, 1, 1, 162, 130, 1, 2, 4, 129, 255, 208, 37, 251, 184, 33, 173, 54, 72,
                    142, 105, 213, 119, 99, 50, 12, 51, 85, 130, 118, 156, 163, 115, 233, 59, 195, 44, 190, 17, 224,
                    214, 18, 196, 225, 140, 185, 117, 127, 179, 187, 178, 215, 23, 99, 158, 37, 55, 203, 145, 101, 117,
                    161, 119, 132, 192, 3, 62, 2, 193, 16, 20, 81, 57, 55, 92, 222, 222, 67, 178, 43, 208, 213, 126,
                    246, 84, 110, 105, 43, 225, 82, 89, 197, 129, 46, 145, 185, 12, 10, 53, 77, 142, 155, 59, 149, 88,
                    5, 189, 96, 20, 240, 67, 208, 118, 74, 242, 53, 160, 167, 14, 184, 170, 76, 1, 143, 174, 120, 137,
                    24, 182, 72, 34, 218, 56, 94, 215, 241, 221, 0, 105, 55, 217, 195, 230, 122, 222, 73, 232, 90, 115,
                    217, 19, 107, 33, 181, 111, 217, 150, 142, 86, 183, 108, 2, 197, 131, 57, 170, 221, 162, 206, 147,
                    93, 6, 226, 156, 179, 46, 177, 233, 184, 167, 104, 183, 137, 74, 99, 132, 174, 19, 146, 200, 59,
                    140, 241, 251, 108, 51, 3, 207, 76, 19, 220, 149, 29, 12, 62, 241, 184, 112, 188, 77, 216, 208, 73,
                    104, 223, 153, 139, 247, 6, 46, 244, 75, 106, 181, 233, 188, 184, 81, 247, 123, 231, 46, 139, 176,
                    204, 31, 18, 0, 222, 43, 113, 4, 64, 92, 63, 1, 72, 99, 108, 226, 222, 175, 87, 85, 60, 156, 73,
                    75, 79, 159, 250, 232, 10, 241, 214, 191, 164, 110, 48, 108, 160, 3, 2, 1, 18, 162, 101, 4, 99,
                    106, 94, 37, 142, 223, 93, 36, 146, 1, 124, 172, 242, 9, 76, 186, 171, 5, 77, 225, 43, 160, 252,
                    253, 38, 235, 37, 210, 141, 117, 149, 90, 1, 37, 130, 188, 5, 244, 120, 135, 207, 78, 51, 29, 145,
                    172, 119, 85, 62, 115, 181, 150, 53, 5, 85, 199, 195, 125, 106, 46, 244, 102, 110, 195, 8, 11, 158,
                    4, 44, 51, 208, 88, 2, 171, 238, 108, 125, 139, 32, 25, 5, 25, 183, 43, 184, 250, 77, 164, 24, 65,
                    247, 150, 138, 86, 57, 81, 74, 201, 60, 151,
                ])),
            }])))),
            req_body: ExplicitContextTag4::from(KdcReqBody {
                kdc_options: ExplicitContextTag0::from(KerberosFlags::from(BitString::with_bytes([
                    0x40, 0x81, 0x00, 0x10,
                ]))),
                cname: Optional::from(None),
                realm: ExplicitContextTag2::from(Realm::from(
                    Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap(),
                )),
                sname: Optional::from(Some(ExplicitContextTag3::from(PrincipalName {
                    name_type: ExplicitContextTag0::from(IntegerAsn1(vec![2])),
                    name_string: ExplicitContextTag1::from(Asn1SequenceOf::from(vec![
                        KerberosStringAsn1::from(Ia5String::from_string("somename".to_owned()).unwrap()),
                        KerberosStringAsn1::from(Ia5String::from_string("EXAMPLE.COM".to_owned()).unwrap()),
                    ])),
                }))),
                from: Optional::from(None),
                till: ExplicitContextTag5::from(KerberosTime::from(Date::new(2041, 12, 5, 17, 45, 20).unwrap())),
                rtime: Optional::from(Some(ExplicitContextTag6::from(KerberosTime::from(
                    Date::new(2041, 12, 5, 17, 45, 20).unwrap(),
                )))),
                nonce: ExplicitContextTag7::from(IntegerAsn1(vec![0x4a, 0x1a, 0x70, 0xae])),
                etype: ExplicitContextTag8::from(Asn1SequenceOf::from(vec![
                    IntegerAsn1(vec![18]),
                    IntegerAsn1(vec![17]),
                    IntegerAsn1(vec![23]),
                ])),
                addresses: Optional::from(None),
                enc_authorization_data: Optional::from(None),
                additional_tickets: Optional::from(None),
            }),
        });

        let tgs_req: TgsReq = picky_asn1_der::from_bytes(&expected_raw).unwrap();
        let tgs_req_raw = picky_asn1_der::to_vec(&tgs_req).unwrap();

        assert_eq!(expected, tgs_req);
        assert_eq!(expected_raw, tgs_req_raw);
    }
}
