//! Server Redirection PDU types.
//!
//! Defined in [\[MS-RDPBCGR\] 2.2.13].
//!
//! [\[MS-RDPBCGR\] 2.2.13]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/

use core::fmt;

use bitflags::bitflags;
use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_fixed_part_size,
    ensure_size, invalid_field_err, read_padding, write_padding,
};

use crate::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
use crate::utils::{self, CharacterSet};

/// `SEC_REDIRECTION_PKT`, the value of the Server Redirection Packet `Flags` field.
const SEC_REDIRECTION_PKT: u16 = 0x0400;

/// Size of the optional trailing `Pad` field of the Server Redirection Packet.
const PACKET_PAD_SIZE: usize = 8;

/// `ELEMENT_TYPE_CERTIFICATE`, the Certificate Meta Element type holding an X.509 certificate.
pub const ELEMENT_TYPE_CERTIFICATE: u32 = 32;

/// `ENCODING_TYPE_ASN1_DER`, the Certificate Meta Element encoding for ASN.1 DER data.
pub const ENCODING_TYPE_ASN1_DER: u32 = 1;

bitflags! {
    /// `RedirFlags` field of the Server Redirection Packet.
    ///
    /// Defined in [\[MS-RDPBCGR\] 2.2.13.1].
    ///
    /// [\[MS-RDPBCGR\] 2.2.13.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
    pub struct ServerRedirectionFlags: u32 {
        const TARGET_NET_ADDRESS = 0x0000_0001;
        const LOAD_BALANCE_INFO = 0x0000_0002;
        const USERNAME = 0x0000_0004;
        const DOMAIN = 0x0000_0008;
        const PASSWORD = 0x0000_0010;
        const DONT_STORE_USERNAME = 0x0000_0020;
        const SMARTCARD_LOGON = 0x0000_0040;
        const NO_REDIRECT = 0x0000_0080;
        const TARGET_FQDN = 0x0000_0100;
        const TARGET_NETBIOS_NAME = 0x0000_0200;
        const TARGET_NET_ADDRESSES = 0x0000_0800;
        const CLIENT_TSV_URL = 0x0000_1000;
        const SERVER_TSV_CAPABLE = 0x0000_2000;
        const PASSWORD_IS_PK_ENCRYPTED = 0x0000_4000;
        const REDIRECTION_GUID = 0x0000_8000;
        const TARGET_CERTIFICATE = 0x0001_0000;

        const _ = !0;
    }
}

impl ServerRedirectionFlags {
    /// Flags that announce an optional field of the packet.
    const FIELD_PRESENCE: Self = Self::TARGET_NET_ADDRESS
        .union(Self::LOAD_BALANCE_INFO)
        .union(Self::USERNAME)
        .union(Self::DOMAIN)
        .union(Self::PASSWORD)
        .union(Self::TARGET_FQDN)
        .union(Self::TARGET_NETBIOS_NAME)
        .union(Self::TARGET_NET_ADDRESSES)
        .union(Self::CLIENT_TSV_URL)
        .union(Self::REDIRECTION_GUID)
        .union(Self::TARGET_CERTIFICATE);
}

/// [2.2.13.1] Server Redirection Packet (RDP_SERVER_REDIRECTION_PACKET)
///
/// The RDP_SERVER_REDIRECTION_PACKET structure contains information to enable a client to reconnect to a session on a
/// specified server.
///
/// Each optional field is present on the wire exactly when it is `Some`: encoding derives the field-presence bits of
/// `RedirFlags` from the fields and takes every other bit from [`Self::flags`].
///
/// [2.2.13.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
#[derive(Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct ServerRedirectionPdu {
    /// The session identifier to which the client must reconnect.
    ///
    /// It goes in the `RedirectedSessionID` field of the Client Cluster Data on reconnection.
    pub session_id: u32,
    /// `RedirFlags` as received.
    pub flags: ServerRedirectionFlags,
    /// IP address of the target server.
    pub target_net_address: Option<String>,
    /// Opaque load-balancing information, sent back as the X.224 routing token on reconnection.
    pub load_balance_info: Option<Vec<u8>>,
    pub username: Option<String>,
    pub domain: Option<String>,
    /// Password or opaque password cookie for the target server.
    ///
    /// When [`ServerRedirectionFlags::PASSWORD_IS_PK_ENCRYPTED`] is set this is an encrypted blob for RDSTLS.
    pub password: Option<Vec<u8>>,
    /// Fully qualified domain name of the target machine.
    pub target_fqdn: Option<String>,
    /// NetBIOS name of the target machine.
    pub target_netbios_name: Option<String>,
    /// The `TsvUrl` that the client previously sent in its load-balancing information.
    pub tsv_url: Option<Vec<u8>>,
    /// Base64-encoded GUID in Unicode format identifying the redirected connection.
    pub redirection_guid: Option<Vec<u8>>,
    /// The certificate of the target server.
    pub target_certificate: Option<TargetCertificateContainer>,
    /// IP addresses of the target server.
    pub target_net_addresses: Option<Vec<String>>,
}

impl fmt::Debug for ServerRedirectionPdu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The password and its cookie are secrets.
        f.debug_struct("ServerRedirectionPdu")
            .field("session_id", &self.session_id)
            .field("flags", &self.flags)
            .field("target_net_address", &self.target_net_address)
            .field("load_balance_info", &self.load_balance_info.as_deref().map(LossyBytes))
            .field("username", &self.username)
            .field("domain", &self.domain)
            .field("password", &self.password.as_ref().map(|password| password.len()))
            .field("target_fqdn", &self.target_fqdn)
            .field("target_netbios_name", &self.target_netbios_name)
            .field("tsv_url", &self.tsv_url.as_deref().map(LossyBytes))
            .field("redirection_guid", &self.redirection_guid.as_deref().map(LossyBytes))
            .field("target_certificate", &self.target_certificate)
            .field("target_net_addresses", &self.target_net_addresses)
            .finish()
    }
}

struct LossyBytes<'a>(&'a [u8]);

impl fmt::Debug for LossyBytes<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", String::from_utf8_lossy(self.0))
    }
}

impl ServerRedirectionPdu {
    const NAME: &'static str = "ServerRedirectionPdu";

    const FIXED_PART_SIZE: usize = 2 /* Flags */ + 2 /* Length */ + 4 /* SessionID */ + 4 /* RedirFlags */;

    /// The `RedirFlags` value written on the wire.
    pub fn wire_flags(&self) -> ServerRedirectionFlags {
        let mut flags = self.flags.difference(ServerRedirectionFlags::FIELD_PRESENCE);
        flags.set(
            ServerRedirectionFlags::TARGET_NET_ADDRESS,
            self.target_net_address.is_some(),
        );
        flags.set(
            ServerRedirectionFlags::LOAD_BALANCE_INFO,
            self.load_balance_info.is_some(),
        );
        flags.set(ServerRedirectionFlags::USERNAME, self.username.is_some());
        flags.set(ServerRedirectionFlags::DOMAIN, self.domain.is_some());
        flags.set(ServerRedirectionFlags::PASSWORD, self.password.is_some());
        flags.set(ServerRedirectionFlags::TARGET_FQDN, self.target_fqdn.is_some());
        flags.set(
            ServerRedirectionFlags::TARGET_NETBIOS_NAME,
            self.target_netbios_name.is_some(),
        );
        flags.set(ServerRedirectionFlags::CLIENT_TSV_URL, self.tsv_url.is_some());
        flags.set(
            ServerRedirectionFlags::REDIRECTION_GUID,
            self.redirection_guid.is_some(),
        );
        flags.set(
            ServerRedirectionFlags::TARGET_CERTIFICATE,
            self.target_certificate.is_some(),
        );
        flags.set(
            ServerRedirectionFlags::TARGET_NET_ADDRESSES,
            self.target_net_addresses.is_some(),
        );
        flags
    }

    /// Whether the PDU is informational only and requires no redirection (`LB_NOREDIRECT`).
    pub fn is_informational(&self) -> bool {
        self.flags.contains(ServerRedirectionFlags::NO_REDIRECT)
    }
}

impl Encode for ServerRedirectionPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        dst.write_u16(SEC_REDIRECTION_PKT);
        dst.write_u16(cast_length!("Length", self.size(), in: dst)?);
        dst.write_u32(self.session_id);
        dst.write_u32(self.wire_flags().bits());

        if let Some(value) = &self.target_net_address {
            write_unicode_field(dst, value)?;
        }
        if let Some(value) = &self.load_balance_info {
            write_bytes_field(dst, value)?;
        }
        if let Some(value) = &self.username {
            write_unicode_field(dst, value)?;
        }
        if let Some(value) = &self.domain {
            write_unicode_field(dst, value)?;
        }
        if let Some(value) = &self.password {
            write_bytes_field(dst, value)?;
        }
        if let Some(value) = &self.target_fqdn {
            write_unicode_field(dst, value)?;
        }
        if let Some(value) = &self.target_netbios_name {
            write_unicode_field(dst, value)?;
        }
        if let Some(value) = &self.tsv_url {
            write_bytes_field(dst, value)?;
        }
        if let Some(value) = &self.redirection_guid {
            write_bytes_field(dst, value)?;
        }
        if let Some(value) = &self.target_certificate {
            let text = value.to_base64();
            dst.write_u32(cast_length!("TargetCertificateLength", text.len() * 2, in: dst)?);
            for code_unit in text.encode_utf16() {
                dst.write_u16(code_unit);
            }
        }
        if let Some(addresses) = &self.target_net_addresses {
            dst.write_u32(cast_length!(
                "TargetNetAddressesLength",
                target_net_addresses_size(addresses), in: dst
            )?);
            dst.write_u32(cast_length!("addressCount", addresses.len(), in: dst)?);
            for address in addresses {
                write_unicode_field(dst, address)?;
            }
        }

        write_padding!(dst, PACKET_PAD_SIZE);

        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        let unicode = |value: &Option<String>| value.as_deref().map_or(0, |value| 4 /* Length */ + unicode_size(value));
        let bytes = |value: &Option<Vec<u8>>| value.as_ref().map_or(0, |value| 4 /* Length */ + value.len());

        Self::FIXED_PART_SIZE
            + unicode(&self.target_net_address) // TargetNetAddress
            + bytes(&self.load_balance_info) // LoadBalanceInfo
            + unicode(&self.username) // UserName
            + unicode(&self.domain) // Domain
            + bytes(&self.password) // Password
            + unicode(&self.target_fqdn) // TargetFQDN
            + unicode(&self.target_netbios_name) // TargetNetBiosName
            + bytes(&self.tsv_url) // TsvUrl
            + bytes(&self.redirection_guid) // RedirectionGuid
            + self
                .target_certificate
                .as_ref()
                .map_or(0, |value| 4 /* TargetCertificateLength */ + value.base64_len() * 2) // TargetCertificate
            + self
                .target_net_addresses
                .as_deref()
                .map_or(0, |value| 4 /* TargetNetAddressesLength */ + target_net_addresses_size(value)) // TargetNetAddresses
            + PACKET_PAD_SIZE // Pad
    }
}

impl<'de> Decode<'de> for ServerRedirectionPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let start = src.pos();

        let flags = src.read_u16();
        if flags != SEC_REDIRECTION_PKT {
            return Err(invalid_field_err!("Flags", "not SEC_REDIRECTION_PKT", in: src));
        }
        let length = usize::from(src.read_u16());
        let session_id = src.read_u32();
        // The server sets the field-presence bits, and the others are informational; unknown bits are kept so that
        // re-encoding reproduces them.
        let flags = ServerRedirectionFlags::from_bits_retain(src.read_u32());

        let mut pdu = Self {
            session_id,
            flags,
            ..Self::default()
        };

        if flags.contains(ServerRedirectionFlags::TARGET_NET_ADDRESS) {
            pdu.target_net_address = Some(read_unicode_field(src, "TargetNetAddress")?);
        }
        if flags.contains(ServerRedirectionFlags::LOAD_BALANCE_INFO) {
            pdu.load_balance_info = Some(read_bytes_field(src)?.to_vec());
        }
        if flags.contains(ServerRedirectionFlags::USERNAME) {
            pdu.username = Some(read_unicode_field(src, "UserName")?);
        }
        if flags.contains(ServerRedirectionFlags::DOMAIN) {
            pdu.domain = Some(read_unicode_field(src, "Domain")?);
        }
        if flags.contains(ServerRedirectionFlags::PASSWORD) {
            pdu.password = Some(read_bytes_field(src)?.to_vec());
        }
        if flags.contains(ServerRedirectionFlags::TARGET_FQDN) {
            pdu.target_fqdn = Some(read_unicode_field(src, "TargetFQDN")?);
        }
        if flags.contains(ServerRedirectionFlags::TARGET_NETBIOS_NAME) {
            pdu.target_netbios_name = Some(read_unicode_field(src, "TargetNetBiosName")?);
        }
        if flags.contains(ServerRedirectionFlags::CLIENT_TSV_URL) {
            pdu.tsv_url = Some(read_bytes_field(src)?.to_vec());
        }
        if flags.contains(ServerRedirectionFlags::REDIRECTION_GUID) {
            pdu.redirection_guid = Some(read_bytes_field(src)?.to_vec());
        }
        if flags.contains(ServerRedirectionFlags::TARGET_CERTIFICATE) {
            let offset = src.pos();
            let text = read_bytes_field(src)?;
            pdu.target_certificate = Some(TargetCertificateContainer::from_base64_unicode(text, offset)?);
        }
        if flags.contains(ServerRedirectionFlags::TARGET_NET_ADDRESSES) {
            let field = read_bytes_field(src)?;
            let mut field = ReadCursor::new(field);
            ensure_size!(ctx: "TargetNetAddresses", in: field, size: 4);
            let count = field.read_u32();
            let mut addresses = Vec::new();
            for _ in 0..count {
                addresses.push(read_unicode_field(&mut field, "TargetNetAddress")?);
            }
            pdu.target_net_addresses = Some(addresses);
        }

        // `Length` covers the fields and the optional `Pad`. Some Windows versions omit the padding at the end of the
        // stream, so a shortfall of at most the padding size is accepted; any other mismatch means the fields do not
        // fit the declared packet.
        let consumed = src.pos() - start;
        let Some(padding) = length.checked_sub(consumed) else {
            return Err(invalid_field_err!("Length", "shorter than the packet fields", in: src));
        };
        if padding <= src.len() {
            read_padding!(src, padding);
        } else if padding - src.len() <= PACKET_PAD_SIZE {
            read_padding!(src, src.len());
        } else {
            return Err(invalid_field_err!("Length", "longer than the packet", in: src));
        }

        Ok(pdu)
    }
}

/// [2.2.13.1.2] Target Certificate Container (TARGET_CERTIFICATE_CONTAINER)
///
/// The TARGET_CERTIFICATE_CONTAINER structure is used to wrap an X.509 certificate.
///
/// On the wire the container is Base64-encoded in Unicode format.
///
/// [2.2.13.1.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct TargetCertificateContainer {
    pub elements: Vec<CertificateMetaElement>,
}

impl TargetCertificateContainer {
    const NAME: &'static str = "TargetCertificateContainer";

    /// Wraps a DER-encoded X.509 certificate.
    pub fn from_certificate_der(certificate: Vec<u8>) -> Self {
        Self {
            elements: vec![CertificateMetaElement {
                element_type: ELEMENT_TYPE_CERTIFICATE,
                encoding: ENCODING_TYPE_ASN1_DER,
                data: certificate,
            }],
        }
    }

    /// The DER-encoded X.509 certificate, from the first element of type `ELEMENT_TYPE_CERTIFICATE` and encoding
    /// `ENCODING_TYPE_ASN1_DER`.
    pub fn certificate_der(&self) -> Option<&[u8]> {
        self.elements
            .iter()
            .find(|element| {
                element.element_type == ELEMENT_TYPE_CERTIFICATE && element.encoding == ENCODING_TYPE_ASN1_DER
            })
            .map(|element| element.data.as_slice())
    }

    fn binary_len(&self) -> usize {
        self.elements
            .iter()
            .map(|element| 4 /* type */ + 4 /* encoding */ + 4 /* elementSize */ + element.data.len())
            .sum()
    }

    fn base64_len(&self) -> usize {
        self.binary_len().div_ceil(3) * 4
    }

    fn to_base64(&self) -> String {
        let mut binary = Vec::with_capacity(self.binary_len());
        for element in &self.elements {
            binary.extend_from_slice(&element.element_type.to_le_bytes());
            binary.extend_from_slice(&element.encoding.to_le_bytes());
            // An element longer than u32::MAX cannot fit the 16-bit Server Redirection Packet length, which encoding
            // rejects as soon as the size is checked.
            let size = u32::try_from(element.data.len()).unwrap_or(u32::MAX);
            binary.extend_from_slice(&size.to_le_bytes());
            binary.extend_from_slice(&element.data);
        }
        base64_encode(&binary)
    }

    /// Decodes the `TargetCertificate` field, which starts at `offset` in the Server Redirection Packet.
    fn from_base64_unicode(text: &[u8], offset: usize) -> DecodeResult<Self> {
        if !text.len().is_multiple_of(2) {
            return Err(invalid_field_err!(
                Self::NAME,
                "TargetCertificate",
                "odd Unicode length",
                at: offset
            ));
        }
        let mut ascii = Vec::with_capacity(text.len() / 2);
        for code_unit in text.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])) {
            match u8::try_from(code_unit) {
                Ok(byte) => ascii.push(byte),
                Err(_) => {
                    return Err(invalid_field_err!(
                        Self::NAME,
                        "TargetCertificate",
                        "non-ASCII Base64 character",
                        at: offset
                    ));
                }
            }
        }
        let binary = base64_decode(&ascii)
            .ok_or_else(|| invalid_field_err!(Self::NAME, "TargetCertificate", "invalid Base64", at: offset))?;

        let mut src = ReadCursor::new(&binary);
        let mut elements = Vec::new();
        while !src.is_empty() {
            ensure_size!(ctx: Self::NAME, in: src, size: 12);
            let element_type = src.read_u32();
            let encoding = src.read_u32();
            let size = cast_length!("elementSize", src.read_u32())?;
            ensure_size!(ctx: Self::NAME, in: src, size: size);
            elements.push(CertificateMetaElement {
                element_type,
                encoding,
                data: src.read_slice(size).to_vec(),
            });
        }

        Ok(Self { elements })
    }
}

/// [2.2.13.1.2.1] Certificate Meta Element (CERTIFICATE_META_ELEMENT)
///
/// The CERTIFICATE_META_ELEMENT structure specifies an element contained within a Target Certificate Container
/// structure.
///
/// [2.2.13.1.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct CertificateMetaElement {
    /// `type`; only [`ELEMENT_TYPE_CERTIFICATE`] is meaningful.
    pub element_type: u32,
    /// `encoding`; only [`ENCODING_TYPE_ASN1_DER`] is meaningful.
    pub encoding: u32,
    pub data: Vec<u8>,
}

impl fmt::Debug for CertificateMetaElement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CertificateMetaElement")
            .field("element_type", &self.element_type)
            .field("encoding", &self.encoding)
            .field("data_len", &self.data.len())
            .finish()
    }
}

/// [2.2.13.2.1] Standard Security Server Redirection PDU (TS_STANDARD_SECURITY_SERVER_REDIRECTION)
///
/// The Standard Security Server Redirection PDU is sent by the server to the client to instruct it to reconnect to an
/// existing session on another server.
///
/// This type covers the MCS user data, a Basic Security Header carrying `SEC_REDIRECTION_PKT` followed by the packet.
/// The PDU is encrypted with Standard RDP Security; this crate handles it only once it is decrypted.
///
/// [2.2.13.2.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
pub struct StandardSecurityServerRedirection {
    pub redirection: ServerRedirectionPdu,
}

impl StandardSecurityServerRedirection {
    const NAME: &'static str = "StandardSecurityServerRedirection";

    const FIXED_PART_SIZE: usize = BasicSecurityHeader::FIXED_PART_SIZE;
}

impl Encode for StandardSecurityServerRedirection {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());

        BasicSecurityHeader {
            flags: BasicSecurityHeaderFlags::REDIRECTION_PKT,
        }
        .encode(dst)?;
        self.redirection.encode(dst)
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::FIXED_PART_SIZE + self.redirection.size()
    }
}

impl<'de> Decode<'de> for StandardSecurityServerRedirection {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);

        let security_header = BasicSecurityHeader::decode(src)?;
        if !security_header
            .flags
            .contains(BasicSecurityHeaderFlags::REDIRECTION_PKT)
        {
            return Err(invalid_field_err!("securityHeader", "missing SEC_REDIRECTION_PKT", in: src));
        }
        if security_header.flags.contains(BasicSecurityHeaderFlags::ENCRYPT) {
            return Err(invalid_field_err!(
                "securityHeader",
                "encrypted Standard RDP Security PDUs are not supported",
                in: src
            ));
        }

        let redirection = ServerRedirectionPdu::decode(src)?;

        Ok(Self { redirection })
    }
}

/// Body of the [2.2.13.3.1] Enhanced Security Server Redirection PDU (TS_ENHANCED_SECURITY_SERVER_REDIRECTION), after
/// its Share Control Header.
///
/// [2.2.13.3.1]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/
pub(crate) struct EnhancedSecurityServerRedirectionBody;

impl EnhancedSecurityServerRedirectionBody {
    /// `pad2Octets` before the packet.
    const PAD2_SIZE: usize = 2;
    /// The optional `pad1Octet` after the packet, which Windows sends.
    const PAD1_SIZE: usize = 1;

    pub(crate) fn encode(redirection: &ServerRedirectionPdu, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: Self::size(redirection));
        write_padding!(dst, Self::PAD2_SIZE);
        redirection.encode(dst)?;
        write_padding!(dst, Self::PAD1_SIZE);
        Ok(())
    }

    pub(crate) fn size(redirection: &ServerRedirectionPdu) -> usize {
        Self::PAD2_SIZE + redirection.size() + Self::PAD1_SIZE
    }

    pub(crate) fn decode(src: &mut ReadCursor<'_>) -> DecodeResult<ServerRedirectionPdu> {
        ensure_size!(ctx: "EnhancedSecurityServerRedirection", in: src, size: Self::PAD2_SIZE);
        read_padding!(src, Self::PAD2_SIZE);
        let redirection = ServerRedirectionPdu::decode(src)?;
        if Self::PAD1_SIZE <= src.len() {
            read_padding!(src, Self::PAD1_SIZE);
        }
        Ok(redirection)
    }
}

/// Decodes a Server Redirection PDU from the user data of an MCS Send Data Indication, if it is one.
///
/// Both the Enhanced Security variant, behind a Share Control Header of type `PDUTYPE_SERVER_REDIR_PKT`, and the
/// Standard Security variant, behind a Basic Security Header carrying `SEC_REDIRECTION_PKT`, are recognized.
/// Returns `Ok(None)` for any other PDU.
pub fn decode_server_redirection(user_data: &[u8]) -> DecodeResult<Option<ServerRedirectionPdu>> {
    // PDU type of a Share Control Header, in the second 16-bit word.
    const PDUTYPE_SERVER_REDIR_PKT: u16 = 0x000A;
    const PDU_TYPE_MASK: u16 = 0x000F;

    let Some(header) = user_data.get(..4) else {
        return Ok(None);
    };
    let first = u16::from_le_bytes([header[0], header[1]]);
    let second = u16::from_le_bytes([header[2], header[3]]);

    // A Share Control Header always carries a non-zero PDU type where a Basic Security Header carries `flagsHi`,
    // which is zero.
    if second == 0 {
        if first & BasicSecurityHeaderFlags::REDIRECTION_PKT.bits() == 0 {
            return Ok(None);
        }
        let pdu: StandardSecurityServerRedirection = ironrdp_core::decode(user_data)?;
        return Ok(Some(pdu.redirection));
    }

    if second & PDU_TYPE_MASK != PDUTYPE_SERVER_REDIR_PKT {
        return Ok(None);
    }
    let header: crate::rdp::headers::ShareControlHeader = ironrdp_core::decode(user_data)?;
    match header.share_control_pdu {
        crate::rdp::headers::ShareControlPdu::ServerRedirect(redirection) => Ok(Some(redirection)),
        _ => Ok(None),
    }
}

fn unicode_size(value: &str) -> usize {
    utils::encoded_str_len(value, CharacterSet::Unicode, true)
}

fn target_net_addresses_size(addresses: &[String]) -> usize {
    4 /* addressCount */
        + addresses
            .iter()
            .map(|address| 4 /* addressLength */ + unicode_size(address))
            .sum::<usize>()
}

fn write_unicode_field(dst: &mut WriteCursor<'_>, value: &str) -> EncodeResult<()> {
    dst.write_u32(cast_length!("length", unicode_size(value), in: dst)?);
    utils::write_string_to_cursor(dst, value, CharacterSet::Unicode, true)
}

fn write_bytes_field(dst: &mut WriteCursor<'_>, value: &[u8]) -> EncodeResult<()> {
    dst.write_u32(cast_length!("length", value.len(), in: dst)?);
    dst.write_slice(value);
    Ok(())
}

fn read_bytes_field<'de>(src: &mut ReadCursor<'de>) -> DecodeResult<&'de [u8]> {
    ensure_size!(ctx: ServerRedirectionPdu::NAME, in: src, size: 4);
    let length = cast_length!("length", src.read_u32())?;
    ensure_size!(ctx: ServerRedirectionPdu::NAME, in: src, size: length);
    Ok(src.read_slice(length))
}

fn read_unicode_field(src: &mut ReadCursor<'_>, field: &'static str) -> DecodeResult<String> {
    let bytes = read_bytes_field(src)?;
    if !bytes.len().is_multiple_of(2) {
        return Err(invalid_field_err!(field, "odd Unicode string length", in: src));
    }
    utils::decode_string(bytes, CharacterSet::Unicode, true)
}

const BASE64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> String {
    let mut text = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let sextets = [
            b0 >> 2,
            ((b0 & 0x03) << 4) | (b1 >> 4),
            ((b1 & 0x0F) << 2) | (b2 >> 6),
            b2 & 0x3F,
        ];
        for (index, sextet) in sextets.into_iter().enumerate() {
            if index <= chunk.len() {
                text.push(char::from(BASE64_ALPHABET[usize::from(sextet)]));
            } else {
                text.push('=');
            }
        }
    }
    text
}

/// Decodes standard Base64, ignoring the line breaks, spaces and null terminators that servers insert.
fn base64_decode(text: &[u8]) -> Option<Vec<u8>> {
    let mut data = Vec::with_capacity(text.len() / 4 * 3);
    let mut accumulator = 0u32;
    let mut bits = 0u32;
    let mut padding = false;
    for &character in text {
        let sextet = match character {
            b'A'..=b'Z' => character - b'A',
            b'a'..=b'z' => character - b'a' + 26,
            b'0'..=b'9' => character - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                padding = true;
                continue;
            }
            b'\r' | b'\n' | b' ' | b'\t' | 0 => continue,
            _ => return None,
        };
        if padding {
            return None;
        }
        // Keeps at most 14 bits: fewer than 8 are pending before a sextet is appended.
        accumulator = ((accumulator << 6) | u32::from(sextet)) & 0x3FFF;
        bits += 6;
        if 8 <= bits {
            bits -= 8;
            data.push(u8::try_from((accumulator >> bits) & 0xFF).ok()?);
        }
    }
    Some(data)
}
