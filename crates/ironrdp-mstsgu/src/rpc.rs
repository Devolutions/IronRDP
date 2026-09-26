//! DCE/RPC common-header, fragment, NTLM association, and RPCH v2 setup codecs.
//!
//! This module frames connection-oriented DCE/RPC PDUs and the initial RPC-over-HTTP v2 RTS exchange.
//! It is not a live RPC-over-HTTP transport.
//! The staged TsProxy NDR control codecs do not provide a live RPC-over-HTTP transport.
//! Caller-owned packet-integrity framing is available, but authentication-provider processing and the RPCH client
//! belong in later work.
//!
//! [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
//! [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
//! [MS-RPCH]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpch/10cf271a-1191-4f4b-961e-6bd9561eef83

use core::{fmt, time::Duration};

use sspi::{
    AuthIdentity, AuthIdentityBuffers, BufferType, ClientRequestFlags, CredentialUse, DataRepresentation,
    InitializeSecurityContextResult, Ntlm, SecurityBuffer, SecurityStatus, Sspi as _, SspiImpl as _, Username,
};
use uuid::Uuid;

use crate::{Error, GwErrorExt as _, GwErrorKind};

/// DCE/RPC common-header size.
///
/// [MS-RPCE] 2.2.2.1 / [C706] 12.6.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
pub const RPC_COMMON_HEADER_SIZE: usize = 1 /* rpc_vers */
    + 1 /* rpc_vers_minor */
    + 1 /* PTYPE */
    + 1 /* pfc_flags */
    + 4 /* packed_drep */
    + 2 /* frag_length */
    + 2 /* auth_length */
    + 4; /* call_id */

const RPC_REQUEST_HEADER_SIZE: usize =
    4 /* alloc_hint */ + 2 /* p_cont_id */ + 2 /* opnum */;
const RPC_RESPONSE_HEADER_SIZE: usize =
    4 /* alloc_hint */ + 2 /* p_cont_id */ + 1 /* cancel_count */ + 1 /* reserved */;
const RPC_FAULT_HEADER_SIZE: usize = RPC_RESPONSE_HEADER_SIZE + 4 /* status */ + 4 /* reserved2 */;
const RPC_SEC_TRAILER_SIZE: usize = RPC_SECURITY_TRAILER_SIZE;
const RPC_BIND_HEADER_SIZE: usize =
    2 /* max_xmit_frag */ + 2 /* max_recv_frag */ + 4 /* assoc_group_id */ + 1 /* n_context_elem */ + 1 /* reserved */ + 2 /* reserved2 */;
const RPC_BIND_ACK_HEADER_SIZE: usize =
    2 /* max_xmit_frag */ + 2 /* max_recv_frag */ + 4 /* assoc_group_id */ + 2 /* sec_addr length */;
const RPC_PRESENTATION_CONTEXT_HEADER_SIZE: usize =
    2 /* p_cont_id */ + 1 /* n_transfer_syn */ + 1 /* reserved */;
const RPC_SYNTAX_IDENTIFIER_SIZE: usize = 16 /* UUID */ + 2 /* version major */ + 2; /* version minor */
const RPC_PRESENTATION_RESULT_SIZE: usize =
    2 /* result */ + 2 /* reason */ + RPC_SYNTAX_IDENTIFIER_SIZE /* transfer syntax */;

/// Connection-oriented DCE/RPC major version.
pub const RPC_VERSION: u8 = 5;
/// Connection-oriented DCE/RPC minor version.
pub const RPC_VERSION_MINOR: u8 = 0;
/// Little-endian IEEE floating-point packed data representation.
pub const RPC_DREP_LITTLE_ENDIAN: [u8; 4] = [0x10, 0, 0, 0];

/// First fragment of a fragmented PDU ([C706] 12.6.2).
pub const PFC_FIRST_FRAG: u8 = 0x01;
/// Last fragment of a fragmented PDU ([C706] 12.6.2).
pub const PFC_LAST_FRAG: u8 = 0x02;
/// Header-signing support on bind-related PDUs ([MS-RPCE] 2.2.2.3).
///
/// For other PDU types, this bit is `PFC_PENDING_CANCEL`.
pub const PFC_SUPPORT_HEADER_SIGN: u8 = 0x04;
/// Packet-integrity authentication level.
///
/// [MS-RPCE] 2.2.2.12.
pub const RPC_AUTH_LEVEL_PACKET_INTEGRITY: u8 = 5;

/// DCE/RPC security-trailer size.
///
/// [MS-RPCE] 2.2.2.11 / [C706] 12.6.
pub const RPC_SECURITY_TRAILER_SIZE: usize = 1 /* auth_type */
    + 1 /* auth_level */
    + 1 /* auth_pad_length */
    + 1 /* auth_reserved */
    + 4; /* auth_context_id */

/// `response` PDU type ([C706] 12.6.4.9).
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
pub const PTYPE_RESPONSE: u8 = 2;
/// `fault` PDU type ([C706] 12.6.4.9).
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
pub const PTYPE_FAULT: u8 = 3;
/// `request` PDU type ([C706] 12.6.4.8).
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
pub const PTYPE_REQUEST: u8 = 0;
/// `bind` PDU type ([C706] 12.6.4.3).
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
pub const PTYPE_BIND: u8 = 11;
/// `bind_ack` PDU type ([C706] 12.6.4.4).
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
pub const PTYPE_BIND_ACK: u8 = 12;
/// `bind_nak` PDU type ([C706] 12.6.4.5).
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
pub const PTYPE_BIND_NAK: u8 = 13;
/// `rpc_auth_3` PDU type ([MS-RPCE] 2.2.2.10).
///
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
pub const PTYPE_RPC_AUTH_3: u8 = 16;
/// `rts` PDU type ([MS-RPCH] 2.2.3.2).
pub const PTYPE_RTS: u8 = 20;

/// Exact size of the CONN/A1 body on the initial RPCH OUT request.
pub const RPCH_OUT_CONTENT_LENGTH: usize = 76;

// The existing response codecs default to the conventional first presentation context.
// Context-aware decoders support other negotiated identifiers.
const RPC_CONTEXT_ID: u16 = 0;

/// Conventional DCE/RPC fragment maximum used for the initial bind.
pub const DEFAULT_FRAGMENT_SIZE: u16 = 0x10b8;

/// Maximum-sized fragment equivalents the stream may buffer before the caller drains them.
pub const MAX_PENDING_RPC_FRAGMENTS: usize = 16;

const MAXIMUM_RESPONSE_ALLOC_HINT: usize = 0x7fff_ffff;
const RPC_AUTH_TYPE_WINNT: u8 = 0x0a;
const RPC_AUTH_CONTEXT_ID: u32 = 1;
const RTS_HEADER_SIZE: usize = RPC_COMMON_HEADER_SIZE + 4 /* flags and command count */;
const RTS_PFC_FLAGS: u8 = PFC_FIRST_FRAG | PFC_LAST_FRAG;
const RTS_FLAG_NONE: u16 = 0;
const RTS_FLAG_PING: u16 = 0x0001;
const RTS_FLAG_OTHER_CMD: u16 = 0x0002;
const RTS_FLAG_RECYCLE_CHANNEL: u16 = 0x0004;
const RTS_FLAG_OUT_CHANNEL: u16 = 0x0010;
const RTS_VERSION: u32 = 1;
const RTS_COMMAND_RECEIVE_WINDOW_SIZE: u32 = 0;
const RTS_COMMAND_FLOW_CONTROL_ACK: u32 = 1;
const RTS_COMMAND_CONNECTION_TIMEOUT: u32 = 2;
const RTS_COMMAND_COOKIE: u32 = 3;
const RTS_COMMAND_CHANNEL_LIFETIME: u32 = 4;
const RTS_COMMAND_CLIENT_KEEPALIVE: u32 = 5;
const RTS_COMMAND_VERSION: u32 = 6;
const RTS_COMMAND_ASSOCIATION_GROUP_ID: u32 = 12;
const RTS_COMMAND_DESTINATION: u32 = 13;
const RTS_COMMAND_ANCE: u32 = 10;
const RTS_DESTINATION_FD_CLIENT: u32 = 0;
const RTS_DESTINATION_FD_SERVER: u32 = 2;
const RTS_MIN_RECEIVE_WINDOW_SIZE: u32 = 8 * 1024;
const RTS_MAX_RECEIVE_WINDOW_SIZE: u32 = 256 * 1024;
const RTS_MIN_CONNECTION_TIMEOUT: u32 = 120_000;
const RTS_MAX_CONNECTION_TIMEOUT: u32 = 14_400_000;
const RTS_MIN_CHANNEL_LIFETIME: u32 = 128 * 1024;
const RTS_MAX_CHANNEL_LIFETIME: u32 = 2 * 1024 * 1024 * 1024;
const RTS_MIN_CLIENT_KEEPALIVE: u32 = 60_000;
const DEFAULT_CLIENT_KEEPALIVE: u32 = 300_000;

/// Errors reported by the DCE/RPC common-header and fragment codecs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcPduError {
    Truncated {
        actual: usize,
        required: usize,
    },
    UnsupportedVersion {
        major: u8,
        minor: u8,
    },
    UnsupportedDataRepresentation {
        value: [u8; 4],
    },
    InvalidFragmentLength {
        fragment_length: u16,
    },
    IncompleteFragment {
        actual: usize,
        fragment_length: u16,
    },
    AuthenticationUnsupported {
        auth_length: u16,
    },
    AuthenticationRequired,
    EmptyAuthenticationVerifier,
    UnsupportedAuthenticationLevel {
        actual: u8,
    },
    InvalidSecurityTrailer {
        fragment_length: u16,
        auth_length: u16,
    },
    InvalidAuthenticationPadding {
        actual: u8,
    },
    NonZeroAuthenticationPadding,
    NonZeroAuthenticationReserved {
        actual: u8,
    },
    UnexpectedAuthenticationType {
        expected: u8,
        actual: u8,
    },
    UnexpectedAuthenticationLevel {
        expected: u8,
        actual: u8,
    },
    UnexpectedAuthenticationContextId {
        expected: u32,
        actual: u32,
    },
    EmptyAuthenticationToken,
    UnalignedSecurityTrailer {
        offset: usize,
    },
    AuthenticationVerifierLength {
        expected: u16,
        actual: usize,
    },
    UnexpectedPduType {
        expected: u8,
        actual: u8,
    },
    FragmentedPduUnsupported {
        flags: u8,
    },
    UnexpectedResponseFragment {
        flags: u8,
    },
    ResponseFragmentCallId {
        expected: u32,
        actual: u32,
    },
    ResponseFragmentAuthentication {
        expected_auth_type: u8,
        expected_auth_level: u8,
        expected_auth_context_id: u32,
        actual_auth_type: u8,
        actual_auth_level: u8,
        actual_auth_context_id: u32,
    },
    ResponseStubTooLarge {
        actual: usize,
        maximum: usize,
    },
    InvalidFragmentSize {
        maximum: u16,
    },
    FragmentExceedsMaximum {
        fragment_length: u16,
        maximum: u16,
    },
    PendingBytesExceedMaximum {
        actual: usize,
        maximum: usize,
    },
    LengthOverflow,
    UnexpectedContextId {
        expected: u16,
        actual: u16,
    },
    InvalidAllocHint {
        alloc_hint: u32,
        stub_length: usize,
    },
    EmptyPresentationContexts,
    TooManyPresentationContexts {
        actual: usize,
    },
    DuplicatePresentationContext {
        context_id: u16,
    },
    EmptyTransferSyntaxes {
        context_id: u16,
    },
    TooManyTransferSyntaxes {
        context_id: u16,
        actual: usize,
    },
    FragmentTooSmall {
        maximum: u16,
        required: usize,
    },
    InvalidBindAckLength {
        actual: usize,
        expected: usize,
    },
    InvalidBindNakVersionsLength {
        actual: usize,
        expected: usize,
    },
    UnexpectedRtsCallId {
        actual: u32,
    },
    InvalidRtsPfcFlags {
        actual: u8,
    },
    UnexpectedRtsFlags {
        expected: u16,
        actual: u16,
    },
    UnexpectedRtsCommandCount {
        expected: u16,
        actual: u16,
    },
    InvalidRtsBodyLength {
        expected: usize,
        actual: usize,
    },
    UnexpectedRtsCommandType {
        expected: u32,
        actual: u32,
    },
    UnexpectedRtsDestination {
        expected: u32,
        actual: u32,
    },
    InvalidRtsReceiveWindowSize {
        actual: u32,
    },
    InvalidRtsConnectionTimeout {
        actual: u32,
    },
    InvalidRtsChannelLifetime {
        actual: u32,
    },
    InvalidRtsClientKeepalive {
        actual: u32,
    },
}

impl fmt::Display for RpcPduError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { actual, required } => {
                write!(f, "truncated rpc pdu: got {actual} bytes, need at least {required}")
            }
            Self::UnsupportedVersion { major, minor } => {
                write!(f, "unsupported rpc version {major}.{minor}")
            }
            Self::UnsupportedDataRepresentation { value } => {
                write!(f, "unsupported rpc data representation {value:02x?}")
            }
            Self::InvalidFragmentLength { fragment_length } => {
                write!(f, "invalid rpc fragment length {fragment_length}")
            }
            Self::IncompleteFragment {
                actual,
                fragment_length,
            } => {
                write!(f, "incomplete rpc fragment: got {actual} bytes, need {fragment_length}")
            }
            Self::AuthenticationUnsupported { auth_length } => {
                write!(
                    f,
                    "rpc authentication verifier of length {auth_length} is not supported"
                )
            }
            Self::AuthenticationRequired => f.write_str("rpc authentication token is required"),
            Self::EmptyAuthenticationVerifier => f.write_str("rpc authentication verifier must not be empty"),
            Self::UnsupportedAuthenticationLevel { actual } => {
                write!(
                    f,
                    "unsupported rpc authentication level {actual}; only packet integrity is supported"
                )
            }
            Self::InvalidSecurityTrailer {
                fragment_length,
                auth_length,
            } => {
                write!(
                    f,
                    "invalid rpc security trailer for {fragment_length}-byte fragment and {auth_length}-byte token"
                )
            }
            Self::InvalidAuthenticationPadding { actual } => {
                write!(f, "rpc authentication padding {actual} exceeds the pdu body")
            }
            Self::NonZeroAuthenticationPadding => f.write_str("rpc authentication padding is not zero"),
            Self::NonZeroAuthenticationReserved { actual } => {
                write!(f, "nonzero rpc authentication reserved byte {actual}")
            }
            Self::UnexpectedAuthenticationType { expected, actual } => {
                write!(f, "unexpected rpc authentication type {actual}, expected {expected}")
            }
            Self::UnexpectedAuthenticationLevel { expected, actual } => {
                write!(f, "unexpected rpc authentication level {actual}, expected {expected}")
            }
            Self::UnexpectedAuthenticationContextId { expected, actual } => {
                write!(
                    f,
                    "unexpected rpc authentication context id {actual}, expected {expected}"
                )
            }
            Self::EmptyAuthenticationToken => f.write_str("empty rpc authentication token"),
            Self::UnalignedSecurityTrailer { offset } => {
                write!(f, "rpc security trailer at offset {offset} is not 16-byte aligned")
            }
            Self::AuthenticationVerifierLength { expected, actual } => {
                write!(
                    f,
                    "rpc authentication verifier length {actual} does not match reserved length {expected}"
                )
            }
            Self::UnexpectedPduType { expected, actual } => {
                write!(f, "unexpected rpc pdu type {actual}, expected {expected}")
            }
            Self::FragmentedPduUnsupported { flags } => {
                write!(f, "fragmented rpc pdu with flags 0x{flags:02x} is not supported")
            }
            Self::UnexpectedResponseFragment { flags } => {
                write!(f, "unexpected rpc response fragment flags 0x{flags:02x}")
            }
            Self::ResponseFragmentCallId { expected, actual } => {
                write!(f, "rpc response fragment call id {actual} does not match {expected}")
            }
            Self::ResponseFragmentAuthentication {
                expected_auth_type,
                expected_auth_level,
                expected_auth_context_id,
                actual_auth_type,
                actual_auth_level,
                actual_auth_context_id,
            } => {
                write!(
                    f,
                    "rpc response authentication ({actual_auth_type}, {actual_auth_level}, {actual_auth_context_id}) does not match ({expected_auth_type}, {expected_auth_level}, {expected_auth_context_id})"
                )
            }
            Self::ResponseStubTooLarge { actual, maximum } => {
                write!(f, "rpc response stub length {actual} exceeds {maximum}")
            }
            Self::InvalidFragmentSize { maximum } => {
                write!(f, "invalid rpc fragment maximum {maximum}")
            }
            Self::FragmentExceedsMaximum {
                fragment_length,
                maximum,
            } => {
                write!(
                    f,
                    "rpc fragment length {fragment_length} exceeds negotiated maximum {maximum}"
                )
            }
            Self::PendingBytesExceedMaximum { actual, maximum } => {
                write!(f, "pending rpc stream size {actual} exceeds maximum {maximum}")
            }
            Self::LengthOverflow => f.write_str("rpc pdu length overflow"),
            Self::UnexpectedContextId { expected, actual } => {
                write!(
                    f,
                    "unexpected rpc presentation context id {actual}, expected {expected}"
                )
            }
            Self::InvalidAllocHint {
                alloc_hint,
                stub_length,
            } => {
                write!(
                    f,
                    "invalid rpc allocation hint {alloc_hint} for {stub_length}-byte stub"
                )
            }
            Self::EmptyPresentationContexts => f.write_str("rpc bind must contain a presentation context"),
            Self::TooManyPresentationContexts { actual } => {
                write!(f, "rpc bind contains {actual} presentation contexts")
            }
            Self::DuplicatePresentationContext { context_id } => {
                write!(f, "duplicate rpc presentation context id {context_id}")
            }
            Self::EmptyTransferSyntaxes { context_id } => {
                write!(f, "rpc presentation context {context_id} has no transfer syntax")
            }
            Self::TooManyTransferSyntaxes { context_id, actual } => {
                write!(
                    f,
                    "rpc presentation context {context_id} has {actual} transfer syntaxes"
                )
            }
            Self::FragmentTooSmall { maximum, required } => {
                write!(f, "rpc fragment maximum {maximum} cannot hold {required} bytes")
            }
            Self::InvalidBindAckLength { actual, expected } => {
                write!(f, "invalid rpc bind_ack body length {actual}, expected {expected}")
            }
            Self::InvalidBindNakVersionsLength { actual, expected } => {
                write!(
                    f,
                    "invalid rpc bind_nak body length {actual}: expected {expected} or at least {}",
                    expected.saturating_add(16)
                )
            }
            Self::UnexpectedRtsCallId { actual } => write!(f, "unexpected rts call id {actual}"),
            Self::InvalidRtsPfcFlags { actual } => {
                write!(f, "invalid rts packet flags 0x{actual:02x}")
            }
            Self::UnexpectedRtsFlags { expected, actual } => {
                write!(f, "unexpected rts flags 0x{actual:04x}, expected 0x{expected:04x}")
            }
            Self::UnexpectedRtsCommandCount { expected, actual } => {
                write!(f, "unexpected rts command count {actual}, expected {expected}")
            }
            Self::InvalidRtsBodyLength { expected, actual } => {
                write!(f, "invalid rts body length {actual}, expected {expected}")
            }
            Self::UnexpectedRtsCommandType { expected, actual } => {
                write!(f, "unexpected rts command type {actual}, expected {expected}")
            }
            Self::UnexpectedRtsDestination { expected, actual } => {
                write!(f, "unexpected rts destination {actual}, expected {expected}")
            }
            Self::InvalidRtsReceiveWindowSize { actual } => {
                write!(f, "invalid rts receive window size {actual}")
            }
            Self::InvalidRtsConnectionTimeout { actual } => {
                write!(f, "invalid rts connection timeout {actual}")
            }
            Self::InvalidRtsChannelLifetime { actual } => {
                write!(f, "invalid rts channel lifetime {actual}")
            }
            Self::InvalidRtsClientKeepalive { actual } => {
                write!(f, "invalid rts client keepalive {actual}")
            }
        }
    }
}

impl core::error::Error for RpcPduError {}

/// A DCE/RPC syntax version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcSyntaxVersion {
    major: u16,
    minor: u16,
}

impl RpcSyntaxVersion {
    /// Creates a syntax version from its major and minor components.
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Major syntax version.
    pub const fn major(self) -> u16 {
        self.major
    }

    /// Minor syntax version.
    pub const fn minor(self) -> u16 {
        self.minor
    }
}

/// A DCE/RPC syntax identifier.
///
/// [MS-RPCE] 2.2.2.7 / [C706] 12.6.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcSyntaxIdentifier {
    uuid: Uuid,
    version: RpcSyntaxVersion,
}

impl RpcSyntaxIdentifier {
    /// Creates a syntax identifier from its UUID and version.
    pub const fn new(uuid: Uuid, version: RpcSyntaxVersion) -> Self {
        Self { uuid, version }
    }

    /// Syntax UUID.
    pub const fn uuid(self) -> Uuid {
        self.uuid
    }

    /// Syntax version.
    pub const fn version(self) -> RpcSyntaxVersion {
        self.version
    }
}

/// One presentation context requested in an RPC bind.
///
/// [C706] 12.6.4.3.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcPresentationContext<'a> {
    pub context_id: u16,
    pub abstract_syntax: RpcSyntaxIdentifier,
    pub transfer_syntaxes: &'a [RpcSyntaxIdentifier],
}

/// One presentation-context result returned by an RPC bind acknowledgement.
///
/// [MS-RPCE] 2.2.2.4 / [C706] 12.6.4.4.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcPresentationResult {
    pub result: u16,
    pub reason: u16,
    pub transfer_syntax: RpcSyntaxIdentifier,
}

/// A decoded, unauthenticated RPC bind acknowledgement.
///
/// The decoded fragment sizes are the minima of the client offer and server
/// acknowledgement in each direction.
///
/// [C706] 12.6.4.4.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcBindAck<'a> {
    pub call_id: u32,
    pub fragment_sizes: RpcFragmentSizes,
    pub association_group_id: u32,
    pub secondary_address: &'a [u8],
    pub results: Vec<RpcPresentationResult>,
}

/// An authenticated RPC bind acknowledgement and its server authentication token.
///
/// [MS-RPCE] 2.2.2.11 and 2.2.2.12.
///
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcAuthenticatedBindAck<'a> {
    bind_ack: RpcBindAck<'a>,
    token: &'a [u8],
    supports_header_signing: bool,
}

impl RpcAuthenticatedBindAck<'_> {
    /// The validated bind acknowledgement.
    pub const fn bind_ack(&self) -> &RpcBindAck<'_> {
        &self.bind_ack
    }

    /// The server authentication token from the DCE/RPC security trailer.
    pub const fn token(&self) -> &[u8] {
        self.token
    }

    /// Whether the server acknowledged header-signing support.
    pub const fn supports_header_signing(&self) -> bool {
        self.supports_header_signing
    }
}

/// Client-side NTLM association state for the DCE/RPC security context.
///
/// This state owns an independent NTLM package and credential handle.
/// It must not be shared with the HTTP authentication context.
///
/// [MS-RPCE] 3.3.1.5.2.1 / [MS-NLMP] 3.1.5.1.
///
/// [MS-NLMP]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-nlmp/a82d61d7-0851-48aa-93ed-5c534c075a40
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
pub struct RpcNtlmAuth {
    ntlm: Ntlm,
    credentials_handle: Option<AuthIdentityBuffers>,
    state: RpcNtlmAuthState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RpcNtlmAuthState {
    Initial,
    AwaitingChallenge,
    Established,
}

impl RpcNtlmAuth {
    /// Creates a client-side NTLM security context for one DCE/RPC connection.
    pub fn new(username: &str, password: &str) -> Result<Self, Error> {
        let username = Username::parse(username)
            .or_else(|_| Username::new(username, None))
            .map_err(|error| Error::custom("parse rpc username", error))?;
        let identity = AuthIdentity {
            username,
            password: password.to_owned().into(),
        };
        let mut ntlm = Ntlm::new();
        let credentials_handle = ntlm
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Outbound)
            .with_auth_data(&identity)
            .execute(&mut ntlm)
            .map_err(|error| Error::custom("acquire rpc ntlm credentials", error))?
            .credentials_handle;

        Ok(Self {
            ntlm,
            credentials_handle,
            state: RpcNtlmAuthState::Initial,
        })
    }

    /// Produces the NTLM Type-1 token for an authenticated bind PDU.
    pub fn initial_token(&mut self) -> Result<Vec<u8>, Error> {
        if self.state != RpcNtlmAuthState::Initial {
            return Err(Error::new("rpc ntlm initial token", GwErrorKind::Connect));
        }

        let token = self.next_token(None)?;
        if self.state != RpcNtlmAuthState::AwaitingChallenge {
            return Err(Error::new("rpc ntlm initial token", GwErrorKind::Connect));
        }

        Ok(token)
    }

    /// Consumes the bind_ack Type-2 token and produces the rpc_auth_3 Type-3 token.
    pub fn continue_token(&mut self, challenge: &[u8]) -> Result<Vec<u8>, Error> {
        if self.state != RpcNtlmAuthState::AwaitingChallenge || challenge.is_empty() {
            return Err(Error::new("rpc ntlm continuation token", GwErrorKind::Connect));
        }

        let token = self.next_token(Some(challenge))?;
        if self.state != RpcNtlmAuthState::Established {
            return Err(Error::new("rpc ntlm continuation token", GwErrorKind::Connect));
        }

        Ok(token)
    }

    /// Whether the Type-3 token has completed this NTLM association.
    pub fn is_complete(&self) -> bool {
        self.state == RpcNtlmAuthState::Established
    }

    fn next_token(&mut self, input: Option<&[u8]>) -> Result<Vec<u8>, Error> {
        let mut input_token = [SecurityBuffer::new(
            input.map(<[u8]>::to_vec).unwrap_or_default(),
            BufferType::Token,
        )];
        let mut output_token = [SecurityBuffer::new(Vec::with_capacity(1024), BufferType::Token)];
        let mut builder = self
            .ntlm
            .initialize_security_context()
            .with_credentials_handle(&mut self.credentials_handle)
            .with_context_requirements(
                ClientRequestFlags::USE_DCE_STYLE
                    | ClientRequestFlags::INTEGRITY
                    | ClientRequestFlags::REPLAY_DETECT
                    | ClientRequestFlags::SEQUENCE_DETECT
                    | ClientRequestFlags::ALLOCATE_MEMORY,
            )
            .with_target_data_representation(DataRepresentation::Native)
            .with_input(&mut input_token)
            .with_output(&mut output_token);

        let InitializeSecurityContextResult { status, .. } = self
            .ntlm
            .initialize_security_context_impl(&mut builder)
            .map_err(|error| Error::custom("initialize rpc ntlm security context", error))?
            .resolve_to_result()
            .map_err(|error| Error::custom("initialize rpc ntlm security context", error))?;

        match status {
            SecurityStatus::Ok => self.state = RpcNtlmAuthState::Established,
            SecurityStatus::ContinueNeeded => self.state = RpcNtlmAuthState::AwaitingChallenge,
            other => {
                return Err(Error::new("rpc ntlm security status", GwErrorKind::Connect)
                    .with_source(std::io::Error::other(format!("unexpected ntlm status: {other:?}"))));
            }
        }

        Ok(core::mem::take(&mut output_token[0].buffer))
    }
}

/// An RPC runtime version advertised in a bind rejection.
///
/// [MS-RPCE] 2.2.2.9 / [C706] 12.6.4.5.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcProtocolVersion {
    major: u8,
    minor: u8,
}

impl RpcProtocolVersion {
    /// Creates an RPC runtime version.
    pub const fn new(major: u8, minor: u8) -> Self {
        Self { major, minor }
    }

    /// Major runtime version.
    pub const fn major(self) -> u8 {
        self.major
    }

    /// Minor runtime version.
    pub const fn minor(self) -> u8 {
        self.minor
    }
}

/// A decoded, unauthenticated RPC bind rejection.
///
/// [MS-RPCE] 2.2.2.9 / [C706] 12.6.4.5.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcBindNak {
    pub call_id: u32,
    pub reason: u16,
    pub supported_versions: Vec<RpcProtocolVersion>,
    pub extended_error_signature: Option<Uuid>,
}

/// Negotiated client-to-server and server-to-client fragment maxima.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcFragmentSizes {
    max_xmit: u16,
    max_recv: u16,
}

impl RpcFragmentSizes {
    /// Conventional DCE/RPC fragment maxima used for the initial bind.
    pub const DEFAULT: Self = Self {
        max_xmit: DEFAULT_FRAGMENT_SIZE,
        max_recv: DEFAULT_FRAGMENT_SIZE,
    };

    /// Creates fragment maxima, requiring each to hold at least one common header.
    pub fn new(max_xmit: u16, max_recv: u16) -> Result<Self, RpcPduError> {
        for maximum in [max_xmit, max_recv] {
            if usize::from(maximum) < RPC_COMMON_HEADER_SIZE {
                return Err(RpcPduError::InvalidFragmentSize { maximum });
            }
        }

        Ok(Self { max_xmit, max_recv })
    }

    /// Client-to-server fragment maximum.
    pub const fn max_xmit(self) -> u16 {
        self.max_xmit
    }

    /// Server-to-client fragment maximum.
    pub const fn max_recv(self) -> u16 {
        self.max_recv
    }
}

/// The parsed 16-byte DCE/RPC common header.
///
/// [MS-RPCE] 2.2.2.1 / [C706] 12.6.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcCommonHeader {
    ptype: u8,
    pfc_flags: u8,
    fragment_length: u16,
    auth_length: u16,
    call_id: u32,
}

impl RpcCommonHeader {
    /// Parses one common header and verifies that its claimed fragment is present.
    ///
    /// The returned header can be used to split a transport buffer at
    /// [`Self::fragment_length`]. Any bytes following that fragment are left for
    /// the transport to process as later PDUs.
    pub fn decode(source: &[u8]) -> Result<Self, RpcPduError> {
        let header = Self::decode_prefix(source)?;
        if source.len() < usize::from(header.fragment_length) {
            return Err(RpcPduError::IncompleteFragment {
                actual: source.len(),
                fragment_length: header.fragment_length,
            });
        }

        Ok(header)
    }

    /// Parses a common header without requiring the complete fragment.
    ///
    /// This is used by stream transports to validate a fragment length before
    /// buffering the whole PDU.
    fn decode_prefix(source: &[u8]) -> Result<Self, RpcPduError> {
        let header = source.get(..RPC_COMMON_HEADER_SIZE).ok_or(RpcPduError::Truncated {
            actual: source.len(),
            required: RPC_COMMON_HEADER_SIZE,
        })?;

        let major = header[0];
        let minor = header[1];
        if (major, minor) != (RPC_VERSION, RPC_VERSION_MINOR) {
            return Err(RpcPduError::UnsupportedVersion { major, minor });
        }

        let drep = [header[4], header[5], header[6], header[7]];
        if drep != RPC_DREP_LITTLE_ENDIAN {
            return Err(RpcPduError::UnsupportedDataRepresentation { value: drep });
        }

        let fragment_length = u16::from_le_bytes([header[8], header[9]]);
        if usize::from(fragment_length) < RPC_COMMON_HEADER_SIZE {
            return Err(RpcPduError::InvalidFragmentLength { fragment_length });
        }

        Ok(Self {
            ptype: header[2],
            pfc_flags: header[3],
            fragment_length,
            auth_length: u16::from_le_bytes([header[10], header[11]]),
            call_id: u32::from_le_bytes([header[12], header[13], header[14], header[15]]),
        })
    }

    /// Claimed fragment length, including this header.
    pub const fn fragment_length(self) -> u16 {
        self.fragment_length
    }

    /// PDU type.
    pub const fn ptype(self) -> u8 {
        self.ptype
    }

    /// Packet flags, including first/last fragment.
    pub const fn pfc_flags(self) -> u8 {
        self.pfc_flags
    }

    /// Authentication verifier length.
    pub const fn auth_length(self) -> u16 {
        self.auth_length
    }

    /// Call identifier shared by fragments of one RPC call.
    pub const fn call_id(self) -> u32 {
        self.call_id
    }

    /// Encodes a common header for a body that excludes the security trailer and
    /// authentication token.
    ///
    /// A future authentication codec can use `auth_length` to append its
    /// padding, security trailer, and token without changing header layout.
    fn encode(
        ptype: u8,
        pfc_flags: u8,
        call_id: u32,
        body_length: usize,
        auth_length: u16,
    ) -> Result<[u8; RPC_COMMON_HEADER_SIZE], RpcPduError> {
        let authentication_length = if auth_length == 0 {
            0
        } else {
            8usize /* security trailer */
                .checked_add(usize::from(auth_length))
                .ok_or(RpcPduError::LengthOverflow)?
        };
        let fragment_length = RPC_COMMON_HEADER_SIZE
            .checked_add(body_length)
            .and_then(|length| length.checked_add(authentication_length))
            .ok_or(RpcPduError::LengthOverflow)?;
        let fragment_length = u16::try_from(fragment_length).map_err(|_| RpcPduError::LengthOverflow)?;

        let mut header = [0; RPC_COMMON_HEADER_SIZE];
        header[0] = RPC_VERSION;
        header[1] = RPC_VERSION_MINOR;
        header[2] = ptype;
        header[3] = pfc_flags;
        header[4..8].copy_from_slice(&RPC_DREP_LITTLE_ENDIAN);
        header[8..10].copy_from_slice(&fragment_length.to_le_bytes());
        header[10..12].copy_from_slice(&auth_length.to_le_bytes());
        header[12..16].copy_from_slice(&call_id.to_le_bytes());
        Ok(header)
    }
}

/// Authentication fields supplied by the security-context owner when framing a PDU.
///
/// `verifier_length` reserves the exact token length that the security provider will later produce.
/// This type does not create a token or own any sequence state.
/// Only [`RPC_AUTH_LEVEL_PACKET_INTEGRITY`] is supported; packet privacy requires body wrapping outside this layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcAuthenticationInfo {
    pub auth_type: u8,
    pub auth_level: u8,
    pub auth_context_id: u32,
    pub verifier_length: u16,
}

/// The decoded DCE/RPC `sec_trailer` fields.
///
/// [MS-RPCE] 2.2.2.11 / [C706] 12.6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcSecurityTrailer {
    pub auth_type: u8,
    pub auth_level: u8,
    pub auth_pad_length: u8,
    pub auth_reserved: u8,
    pub auth_context_id: u32,
}

/// The PDU segments that a security provider receives in this order.
///
/// `body` includes the authentication padding octets.
/// The security-context owner selects the protection flags for each segment and owns sequence use.
///
/// [MS-RPCE] 2.2.2.1 and 3.3.1.5.2.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcAuthenticationSegments<'a> {
    pub header: &'a [u8],
    pub body: &'a [u8],
    pub security_trailer: &'a [u8],
}

/// An authenticated DCE/RPC fragment awaiting caller-owned verifier validation.
///
/// The verifier is opaque authentication-provider output.
/// Call [`Self::verify_with`] before consuming the PDU-specific body or reassembling the fragment.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RpcUnverifiedAuthenticatedFragment<'a> {
    fragment: &'a [u8],
    header: RpcCommonHeader,
    body_end: usize,
    security_trailer_offset: usize,
    security_trailer: RpcSecurityTrailer,
}

impl fmt::Debug for RpcUnverifiedAuthenticatedFragment<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RpcUnverifiedAuthenticatedFragment")
            .field("header", &self.header)
            .field("security_trailer", &self.security_trailer)
            .field("verifier_length", &self.verifier().len())
            .finish()
    }
}

impl<'a> RpcUnverifiedAuthenticatedFragment<'a> {
    /// Common header parsed from this fragment.
    pub const fn header(self) -> RpcCommonHeader {
        self.header
    }

    /// Parsed security-trailer fields.
    pub const fn security_trailer(self) -> RpcSecurityTrailer {
        self.security_trailer
    }

    /// Opaque authentication-provider output following the security trailer.
    pub fn verifier(&self) -> &'a [u8] {
        &self.fragment[self.security_trailer_offset + RPC_SECURITY_TRAILER_SIZE..]
    }

    fn authentication_segments(&self) -> RpcAuthenticationSegments<'a> {
        RpcAuthenticationSegments {
            header: &self.fragment[..RPC_COMMON_HEADER_SIZE],
            body: &self.fragment[RPC_COMMON_HEADER_SIZE..self.security_trailer_offset],
            security_trailer: &self.fragment
                [self.security_trailer_offset..self.security_trailer_offset + RPC_SECURITY_TRAILER_SIZE],
        }
    }

    /// Invokes the caller-owned verifier and returns a fragment whose body may be consumed.
    ///
    /// The verifier receives the original wire segments and opaque verifier bytes.
    /// It owns algorithm selection, protection flags, and sequence-number use.
    pub fn verify_with<E>(
        self,
        verifier: impl FnOnce(RpcAuthenticationSegments<'a>, &'a [u8]) -> Result<(), E>,
    ) -> Result<RpcVerifiedAuthenticatedFragment<'a>, E> {
        verifier(self.authentication_segments(), self.verifier())?;
        Ok(RpcVerifiedAuthenticatedFragment { fragment: self })
    }
}

/// An authenticated DCE/RPC fragment whose caller-owned verifier accepted it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcVerifiedAuthenticatedFragment<'a> {
    fragment: RpcUnverifiedAuthenticatedFragment<'a>,
}

impl<'a> RpcVerifiedAuthenticatedFragment<'a> {
    /// Common header parsed from this fragment.
    pub const fn header(self) -> RpcCommonHeader {
        self.fragment.header()
    }

    /// PDU-specific body before authentication padding.
    pub fn body(&self) -> &'a [u8] {
        &self.fragment.fragment[RPC_COMMON_HEADER_SIZE..self.fragment.body_end]
    }

    /// Authentication padding octets before the security trailer.
    pub fn authentication_padding(&self) -> &'a [u8] {
        &self.fragment.fragment[self.fragment.body_end..self.fragment.security_trailer_offset]
    }

    /// Parsed security-trailer fields.
    pub const fn security_trailer(self) -> RpcSecurityTrailer {
        self.fragment.security_trailer()
    }

    /// Decodes an authenticated response fragment for `context_id`.
    ///
    /// [C706] 12.6.4.9.
    ///
    /// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
    pub fn decode_response_fragment_for_context(&self, context_id: u16) -> Result<RpcResponse<'a>, RpcPduError> {
        if self.header().ptype() != PTYPE_RESPONSE {
            return Err(RpcPduError::UnexpectedPduType {
                expected: PTYPE_RESPONSE,
                actual: self.header().ptype(),
            });
        }

        decode_rpc_response_body(self.header(), self.body(), context_id)
    }

    /// Decodes an authenticated response fragment for the first presentation context.
    pub fn decode_response_fragment(&self) -> Result<RpcResponse<'a>, RpcPduError> {
        self.decode_response_fragment_for_context(RPC_CONTEXT_ID)
    }

    /// Decodes an authenticated bind acknowledgement.
    ///
    /// [C706] 12.6.4.4.
    ///
    /// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
    pub fn decode_bind_ack(&self, offered_fragment_sizes: RpcFragmentSizes) -> Result<RpcBindAck<'a>, RpcPduError> {
        if self.header().ptype() != PTYPE_BIND_ACK {
            return Err(RpcPduError::UnexpectedPduType {
                expected: PTYPE_BIND_ACK,
                actual: self.header().ptype(),
            });
        }
        validate_single_fragment(self.header())?;

        decode_rpc_bind_ack_body(self.header(), self.body(), offered_fragment_sizes)
    }
}

/// An authenticated PDU awaiting its caller-provided verifier.
///
/// The security-context owner must use [`Self::authentication_segments`] for per-PDU protection
/// when required, then call [`Self::finish`] with exactly the reserved provider output.
/// It owns all authentication algorithm and sequence-number decisions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcPreparedAuthenticatedPdu {
    pdu: Vec<u8>,
    body_end: usize,
    security_trailer_offset: usize,
    verifier_length: u16,
}

impl RpcPreparedAuthenticatedPdu {
    /// Returns the header, padded PDU body, and security trailer as distinct security-provider segments.
    pub fn authentication_segments(&self) -> RpcAuthenticationSegments<'_> {
        RpcAuthenticationSegments {
            header: &self.pdu[..RPC_COMMON_HEADER_SIZE],
            body: &self.pdu[RPC_COMMON_HEADER_SIZE..self.security_trailer_offset],
            security_trailer: &self.pdu
                [self.security_trailer_offset..self.security_trailer_offset + RPC_SECURITY_TRAILER_SIZE],
        }
    }

    /// Returns the PDU-specific body before authentication padding.
    pub fn body(&self) -> &[u8] {
        &self.pdu[RPC_COMMON_HEADER_SIZE..self.body_end]
    }

    /// Complete fragment size, including authentication padding, security trailer, and verifier reservation.
    pub fn fragment_length(&self) -> usize {
        self.pdu.len()
    }

    /// Finishes the PDU with the verifier returned by the security provider.
    pub fn finish(mut self, verifier: &[u8]) -> Result<Vec<u8>, RpcPduError> {
        if verifier.len() != usize::from(self.verifier_length) {
            return Err(RpcPduError::AuthenticationVerifierLength {
                expected: self.verifier_length,
                actual: verifier.len(),
            });
        }

        self.pdu[self.security_trailer_offset + RPC_SECURITY_TRAILER_SIZE..].copy_from_slice(verifier);
        Ok(self.pdu)
    }
}

/// Frames a complete DCE/RPC PDU with a reserved authentication verifier.
///
/// The caller supplies the PDU-specific body and PFC flags.
/// This function applies only the security-trailer layout and does not validate PDU-specific rules.
///
/// [MS-RPCE] 2.2.2.1 and 2.2.2.11 / [C706] 12.6.
pub fn prepare_rpc_authenticated_pdu(
    ptype: u8,
    pfc_flags: u8,
    call_id: u32,
    body: &[u8],
    authentication: RpcAuthenticationInfo,
) -> Result<RpcPreparedAuthenticatedPdu, RpcPduError> {
    if authentication.verifier_length == 0 {
        return Err(RpcPduError::EmptyAuthenticationVerifier);
    }
    if authentication.auth_level != RPC_AUTH_LEVEL_PACKET_INTEGRITY {
        return Err(RpcPduError::UnsupportedAuthenticationLevel {
            actual: authentication.auth_level,
        });
    }

    let authentication_padding_length = (16 - (RPC_COMMON_HEADER_SIZE + body.len()) % 16) % 16;
    let authentication_padding_length =
        u8::try_from(authentication_padding_length).map_err(|_| RpcPduError::LengthOverflow)?;
    let body_length = body
        .len()
        .checked_add(usize::from(authentication_padding_length))
        .ok_or(RpcPduError::LengthOverflow)?;
    let header = RpcCommonHeader::encode(ptype, pfc_flags, call_id, body_length, authentication.verifier_length)?;
    let security_trailer_offset = RPC_COMMON_HEADER_SIZE
        .checked_add(body_length)
        .ok_or(RpcPduError::LengthOverflow)?;
    let pdu_length = security_trailer_offset
        .checked_add(RPC_SECURITY_TRAILER_SIZE)
        .and_then(|length| length.checked_add(usize::from(authentication.verifier_length)))
        .ok_or(RpcPduError::LengthOverflow)?;
    let mut pdu = Vec::with_capacity(pdu_length);
    pdu.extend_from_slice(&header);
    pdu.extend_from_slice(body);
    pdu.resize(security_trailer_offset, 0);
    pdu.push(authentication.auth_type);
    pdu.push(authentication.auth_level);
    pdu.push(authentication_padding_length);
    pdu.push(0); // auth_reserved
    pdu.extend_from_slice(&authentication.auth_context_id.to_le_bytes());
    pdu.resize(pdu_length, 0);
    debug_assert_eq!(pdu.len(), pdu_length);

    Ok(RpcPreparedAuthenticatedPdu {
        pdu,
        body_end: RPC_COMMON_HEADER_SIZE + body.len(),
        security_trailer_offset,
        verifier_length: authentication.verifier_length,
    })
}

/// Decodes and splits one complete authenticated DCE/RPC fragment.
///
/// The returned authentication segments retain the original wire bytes for a caller-owned verifier.
///
/// [MS-RPCE] 2.2.2.1, 2.2.2.11, and 3.3.1.5.2.2 / [C706] 12.6.
pub fn decode_rpc_authenticated_fragment(
    source: &[u8],
    maximum_fragment_size: u16,
) -> Result<RpcUnverifiedAuthenticatedFragment<'_>, RpcPduError> {
    let header = RpcCommonHeader::decode(source)?;
    if header.fragment_length() > maximum_fragment_size {
        return Err(RpcPduError::FragmentExceedsMaximum {
            fragment_length: header.fragment_length(),
            maximum: maximum_fragment_size,
        });
    }
    if header.auth_length() == 0 {
        return Err(RpcPduError::AuthenticationUnsupported { auth_length: 0 });
    }

    let fragment_length = usize::from(header.fragment_length());
    let security_trailer_offset = fragment_length
        .checked_sub(usize::from(header.auth_length()))
        .and_then(|length| length.checked_sub(RPC_SECURITY_TRAILER_SIZE))
        .ok_or_else(|| RpcPduError::InvalidSecurityTrailer {
            fragment_length: header.fragment_length(),
            auth_length: header.auth_length(),
        })?;
    if security_trailer_offset < RPC_COMMON_HEADER_SIZE {
        return Err(RpcPduError::InvalidSecurityTrailer {
            fragment_length: header.fragment_length(),
            auth_length: header.auth_length(),
        });
    }
    if !(security_trailer_offset - RPC_COMMON_HEADER_SIZE).is_multiple_of(16) {
        return Err(RpcPduError::UnalignedSecurityTrailer {
            offset: security_trailer_offset,
        });
    }

    let fragment = &source[..fragment_length];
    let security_trailer = RpcSecurityTrailer {
        auth_type: fragment[security_trailer_offset],
        auth_level: fragment[security_trailer_offset + 1],
        auth_pad_length: fragment[security_trailer_offset + 2],
        auth_reserved: fragment[security_trailer_offset + 3],
        auth_context_id: u32::from_le_bytes(
            fragment[security_trailer_offset + 4..security_trailer_offset + RPC_SECURITY_TRAILER_SIZE]
                .try_into()
                .map_err(|_| RpcPduError::LengthOverflow)?,
        ),
    };
    if security_trailer.auth_level != RPC_AUTH_LEVEL_PACKET_INTEGRITY {
        return Err(RpcPduError::UnsupportedAuthenticationLevel {
            actual: security_trailer.auth_level,
        });
    }
    let body_length = security_trailer_offset - RPC_COMMON_HEADER_SIZE;
    let authentication_padding_length = usize::from(security_trailer.auth_pad_length);
    if authentication_padding_length > body_length {
        return Err(RpcPduError::InvalidAuthenticationPadding {
            actual: security_trailer.auth_pad_length,
        });
    }

    Ok(RpcUnverifiedAuthenticatedFragment {
        fragment,
        header,
        body_end: security_trailer_offset - authentication_padding_length,
        security_trailer_offset,
        security_trailer,
    })
}

/// Incrementally frames DCE/RPC PDUs from a byte stream.
///
/// Each yielded buffer is exactly one complete DCE/RPC fragment. The stream
/// validates the common header and negotiated receive maximum before retaining
/// a claimed fragment, preventing a peer from making the client buffer an
/// oversized PDU.
#[derive(Debug)]
pub struct RpcPduStream {
    buffer: Vec<u8>,
    maximum_fragment_size: u16,
    maximum_pending_bytes: usize,
}

impl RpcPduStream {
    /// Creates a stream that rejects fragments above `maximum_fragment_size`.
    pub fn new(maximum_fragment_size: u16) -> Result<Self, RpcPduError> {
        if usize::from(maximum_fragment_size) < RPC_COMMON_HEADER_SIZE {
            return Err(RpcPduError::InvalidFragmentSize {
                maximum: maximum_fragment_size,
            });
        }

        Ok(Self {
            buffer: Vec::new(),
            maximum_fragment_size,
            maximum_pending_bytes: usize::from(maximum_fragment_size)
                .checked_mul(MAX_PENDING_RPC_FRAGMENTS)
                .ok_or(RpcPduError::LengthOverflow)?,
        })
    }

    /// Adds received bytes to the pending DCE/RPC stream.
    ///
    /// The caller must drain complete fragments with [`Self::next_fragment`] before
    /// buffering more than [`MAX_PENDING_RPC_FRAGMENTS`] fragments.
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), RpcPduError> {
        let pending = self
            .buffer
            .len()
            .checked_add(bytes.len())
            .ok_or(RpcPduError::LengthOverflow)?;
        if pending > self.maximum_pending_bytes {
            return Err(RpcPduError::PendingBytesExceedMaximum {
                actual: pending,
                maximum: self.maximum_pending_bytes,
            });
        }

        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    /// Returns the next complete DCE/RPC fragment, if one has been received.
    pub fn next_fragment(&mut self) -> Result<Option<Vec<u8>>, RpcPduError> {
        let header = match RpcCommonHeader::decode_prefix(&self.buffer) {
            Ok(header) => header,
            Err(RpcPduError::Truncated { .. }) => return Ok(None),
            Err(error) => return Err(error),
        };

        if header.fragment_length > self.maximum_fragment_size {
            return Err(RpcPduError::FragmentExceedsMaximum {
                fragment_length: header.fragment_length,
                maximum: self.maximum_fragment_size,
            });
        }

        let fragment_length = usize::from(header.fragment_length);
        if self.buffer.len() < fragment_length {
            return Ok(None);
        }

        Ok(Some(self.buffer.drain(..fragment_length).collect()))
    }
}

/// A decoded, single-fragment RPC response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcResponse<'a> {
    pub call_id: u32,
    pub pfc_flags: u8,
    pub alloc_hint: u32,
    pub cancel_count: u8,
    /// Decoded reserved byte; response encoders always write zero.
    pub reserved: u8,
    pub stub: &'a [u8],
}

impl RpcResponse<'_> {
    /// Whether this fragment carries the last-fragment flag, terminating its response
    /// ([C706] 12.6.2).
    pub fn is_last_fragment(&self) -> bool {
        self.pfc_flags & PFC_LAST_FRAG != 0
    }
}

/// An owned RPC response reassembled from one or more response fragments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcReassembledResponse {
    pub call_id: u32,
    /// Cancellation count from the last response fragment.
    pub cancel_count: u8,
    pub reserved: u8,
    pub stub: Vec<u8>,
}

/// Bounded reassembler for consecutive DCE/RPC response fragments.
///
/// Every fragment must belong to the same call and the first/last-fragment flags
/// must delimit exactly one complete response.
#[derive(Debug)]
pub struct RpcResponseReassembler {
    maximum_stub_size: usize,
    call_id: Option<u32>,
    cancel_count: u8,
    reserved: u8,
    maximum_claimed_stub_end: usize,
    maximum_claimed_stub_hint: u32,
    stub: Vec<u8>,
}

impl RpcResponseReassembler {
    /// Creates a reassembler that rejects stubs larger than `maximum_stub_size`
    /// or the MS-RPCE response `alloc_hint` limit.
    pub fn new(maximum_stub_size: usize) -> Self {
        Self {
            maximum_stub_size: maximum_stub_size.min(MAXIMUM_RESPONSE_ALLOC_HINT),
            call_id: None,
            cancel_count: 0,
            reserved: 0,
            maximum_claimed_stub_end: 0,
            maximum_claimed_stub_hint: 0,
            stub: Vec::new(),
        }
    }

    /// Adds one decoded response fragment and returns an owned response after
    /// receiving its last fragment.
    pub fn push(&mut self, response: RpcResponse<'_>) -> Result<Option<RpcReassembledResponse>, RpcPduError> {
        let flags = response.pfc_flags & (PFC_FIRST_FRAG | PFC_LAST_FRAG);
        match self.call_id {
            None if flags & PFC_FIRST_FRAG == 0 => return Err(RpcPduError::UnexpectedResponseFragment { flags }),
            Some(_) if flags & PFC_FIRST_FRAG != 0 => return Err(RpcPduError::UnexpectedResponseFragment { flags }),
            Some(expected) if expected != response.call_id => {
                return Err(RpcPduError::ResponseFragmentCallId {
                    expected,
                    actual: response.call_id,
                });
            }
            _ => {}
        }

        let stub_offset = self.stub.len();
        let stub_length = stub_offset
            .checked_add(response.stub.len())
            .ok_or(RpcPduError::LengthOverflow)?;
        if stub_length > self.maximum_stub_size {
            return Err(RpcPduError::ResponseStubTooLarge {
                actual: stub_length,
                maximum: self.maximum_stub_size,
            });
        }
        let claimed_stub_end = if response.alloc_hint == 0 {
            0
        } else {
            stub_offset
                .checked_add(usize::try_from(response.alloc_hint).map_err(|_| RpcPduError::LengthOverflow)?)
                .ok_or(RpcPduError::LengthOverflow)?
        };
        if claimed_stub_end > self.maximum_stub_size {
            return Err(RpcPduError::ResponseStubTooLarge {
                actual: claimed_stub_end,
                maximum: self.maximum_stub_size,
            });
        }

        if self.call_id.is_none() {
            self.call_id = Some(response.call_id);
            self.reserved = response.reserved;
        }
        self.cancel_count = response.cancel_count;
        if claimed_stub_end > self.maximum_claimed_stub_end {
            self.maximum_claimed_stub_end = claimed_stub_end;
            self.maximum_claimed_stub_hint = response.alloc_hint;
        }
        self.stub.extend_from_slice(response.stub);

        if flags & PFC_LAST_FRAG == 0 {
            return Ok(None);
        }

        let total_length = self.stub.len();
        if total_length < self.maximum_claimed_stub_end {
            let alloc_hint = self.maximum_claimed_stub_hint;
            self.reset();
            return Err(RpcPduError::InvalidAllocHint {
                alloc_hint,
                stub_length: total_length,
            });
        }

        let response = RpcReassembledResponse {
            call_id: self.call_id.ok_or(RpcPduError::UnexpectedResponseFragment { flags })?,
            cancel_count: self.cancel_count,
            reserved: self.reserved,
            stub: core::mem::take(&mut self.stub),
        };
        self.reset();
        Ok(Some(response))
    }

    fn reset(&mut self) {
        self.call_id = None;
        self.cancel_count = 0;
        self.reserved = 0;
        self.maximum_claimed_stub_end = 0;
        self.maximum_claimed_stub_hint = 0;
        self.stub.clear();
    }
}

/// Bounded reassembler for caller-verified authenticated DCE/RPC response fragments.
///
/// Every fragment must use the same authentication type, level, and context identifier.
///
/// [MS-RPCE] 2.2.2.11.
#[derive(Debug)]
pub struct RpcAuthenticatedResponseReassembler {
    response_reassembler: RpcResponseReassembler,
    authentication: Option<RpcSecurityTrailer>,
}

impl RpcAuthenticatedResponseReassembler {
    /// Creates a reassembler that rejects stubs larger than `maximum_stub_size`
    /// or the MS-RPCE response `alloc_hint` limit.
    pub fn new(maximum_stub_size: usize) -> Self {
        Self {
            response_reassembler: RpcResponseReassembler::new(maximum_stub_size),
            authentication: None,
        }
    }

    /// Adds a caller-verified response fragment for `context_id`.
    pub fn push_for_context(
        &mut self,
        fragment: RpcVerifiedAuthenticatedFragment<'_>,
        context_id: u16,
    ) -> Result<Option<RpcReassembledResponse>, RpcPduError> {
        let actual = fragment.security_trailer();
        if let Some(expected) = self.authentication
            && (expected.auth_type != actual.auth_type
                || expected.auth_level != actual.auth_level
                || expected.auth_context_id != actual.auth_context_id)
        {
            if fragment.header().pfc_flags() & PFC_LAST_FRAG != 0 {
                self.response_reassembler.reset();
                self.authentication = None;
            }

            return Err(RpcPduError::ResponseFragmentAuthentication {
                expected_auth_type: expected.auth_type,
                expected_auth_level: expected.auth_level,
                expected_auth_context_id: expected.auth_context_id,
                actual_auth_type: actual.auth_type,
                actual_auth_level: actual.auth_level,
                actual_auth_context_id: actual.auth_context_id,
            });
        }

        let response = match fragment.decode_response_fragment_for_context(context_id) {
            Ok(response) => response,
            Err(error) => {
                if fragment.header().pfc_flags() & PFC_LAST_FRAG != 0
                    && self.response_reassembler.call_id == Some(fragment.header().call_id())
                {
                    self.response_reassembler.reset();
                    self.authentication = None;
                }

                return Err(error);
            }
        };
        let reassembled = self.response_reassembler.push(response);
        match reassembled {
            Ok(Some(response)) => {
                self.authentication = None;
                Ok(Some(response))
            }
            Ok(None) => {
                self.authentication = Some(actual);
                Ok(None)
            }
            Err(error) => {
                if self.response_reassembler.call_id.is_none() {
                    self.authentication = None;
                }

                Err(error)
            }
        }
    }

    /// Adds a caller-verified response fragment for the first presentation context.
    pub fn push(
        &mut self,
        fragment: RpcVerifiedAuthenticatedFragment<'_>,
    ) -> Result<Option<RpcReassembledResponse>, RpcPduError> {
        self.push_for_context(fragment, RPC_CONTEXT_ID)
    }
}

/// A decoded, single-fragment RPC fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcFault<'a> {
    pub call_id: u32,
    pub pfc_flags: u8,
    pub alloc_hint: u32,
    pub cancel_count: u8,
    pub reserved: u8,
    pub status: u32,
    /// Decoded reserved field; fault encoders always write zero.
    pub reserved2: u32,
    /// Trailing fault data, which MS-RPCE clients must ignore.
    ///
    /// When `reserved & 1 != 0`, it contains extended error information whose
    /// length is derived from `alloc_hint`.
    pub stub: &'a [u8],
}

/// Encodes one complete, unauthenticated RPC bind PDU.
///
/// [C706] 12.6.4.3.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
pub fn encode_rpc_bind(
    call_id: u32,
    fragment_sizes: RpcFragmentSizes,
    association_group_id: u32,
    presentation_contexts: &[RpcPresentationContext<'_>],
) -> Result<Vec<u8>, RpcPduError> {
    let body = encode_rpc_bind_body(fragment_sizes, association_group_id, presentation_contexts)?;
    ensure_fragment_fits(body.len(), fragment_sizes.max_xmit())?;
    encode_unprotected_pdu(PTYPE_BIND, call_id, body)
}

/// Frames an authenticated RPC bind PDU with a caller-provided verifier reservation.
///
/// `supports_header_signing` advertises the client's header-signing capability in this bind PDU.
/// Header signing is agreed only if the server also sets the bit in its bind acknowledgement.
/// The caller must pass the initial `GSS_Init_sec_context` output to [`RpcPreparedAuthenticatedPdu::finish`];
/// it is an authentication token, not a signature of the returned authentication segments.
///
/// [MS-RPCE] 2.2.2.3, 2.2.2.11, and 3.3.1.5.2.2 / [C706] 12.6.4.3.
pub fn prepare_rpc_authenticated_bind(
    call_id: u32,
    fragment_sizes: RpcFragmentSizes,
    association_group_id: u32,
    presentation_contexts: &[RpcPresentationContext<'_>],
    supports_header_signing: bool,
    authentication: RpcAuthenticationInfo,
) -> Result<RpcPreparedAuthenticatedPdu, RpcPduError> {
    let body = encode_rpc_bind_body(fragment_sizes, association_group_id, presentation_contexts)?;
    let pfc_flags = PFC_FIRST_FRAG
        | PFC_LAST_FRAG
        | if supports_header_signing {
            PFC_SUPPORT_HEADER_SIGN
        } else {
            0
        };
    let pdu = prepare_rpc_authenticated_pdu(PTYPE_BIND, pfc_flags, call_id, &body, authentication)?;
    ensure_complete_fragment_fits(pdu.fragment_length(), fragment_sizes.max_xmit())?;
    Ok(pdu)
}

/// Encodes one complete NTLM-authenticated RPC bind PDU.
///
/// The caller supplies the Type-1 token produced by [`RpcNtlmAuth::initial_token`].
/// This configures packet-integrity authentication for the association but does not
/// encode signed RPC request or response PDUs.
///
/// [MS-RPCE] 2.2.2.11, 2.2.2.12, and 4.2 / [C706] 12.6.4.3.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
pub fn encode_rpc_bind_with_ntlm_auth(
    call_id: u32,
    fragment_sizes: RpcFragmentSizes,
    association_group_id: u32,
    presentation_contexts: &[RpcPresentationContext<'_>],
    type1_token: &[u8],
) -> Result<Vec<u8>, RpcPduError> {
    let body = encode_rpc_bind_body(fragment_sizes, association_group_id, presentation_contexts)?;
    let pdu = encode_authenticated_pdu(PTYPE_BIND, call_id, body, type1_token)?;
    ensure_encoded_fragment_fits(&pdu, fragment_sizes.max_xmit())?;
    Ok(pdu)
}

fn encode_rpc_bind_body(
    fragment_sizes: RpcFragmentSizes,
    association_group_id: u32,
    presentation_contexts: &[RpcPresentationContext<'_>],
) -> Result<Vec<u8>, RpcPduError> {
    if presentation_contexts.is_empty() {
        return Err(RpcPduError::EmptyPresentationContexts);
    }
    if u8::try_from(presentation_contexts.len()).is_err() {
        return Err(RpcPduError::TooManyPresentationContexts {
            actual: presentation_contexts.len(),
        });
    }

    let mut body_length = RPC_BIND_HEADER_SIZE;
    for (index, context) in presentation_contexts.iter().enumerate() {
        if presentation_contexts[..index]
            .iter()
            .any(|previous| previous.context_id == context.context_id)
        {
            return Err(RpcPduError::DuplicatePresentationContext {
                context_id: context.context_id,
            });
        }
        if context.transfer_syntaxes.is_empty() {
            return Err(RpcPduError::EmptyTransferSyntaxes {
                context_id: context.context_id,
            });
        }
        if u8::try_from(context.transfer_syntaxes.len()).is_err() {
            return Err(RpcPduError::TooManyTransferSyntaxes {
                context_id: context.context_id,
                actual: context.transfer_syntaxes.len(),
            });
        }

        body_length = body_length
            .checked_add(RPC_PRESENTATION_CONTEXT_HEADER_SIZE)
            .and_then(|length| length.checked_add(RPC_SYNTAX_IDENTIFIER_SIZE))
            .and_then(|length| {
                length.checked_add(
                    context
                        .transfer_syntaxes
                        .len()
                        .checked_mul(RPC_SYNTAX_IDENTIFIER_SIZE)?,
                )
            })
            .ok_or(RpcPduError::LengthOverflow)?;
    }
    let mut body = Vec::with_capacity(body_length);
    body.extend_from_slice(&fragment_sizes.max_xmit().to_le_bytes());
    body.extend_from_slice(&fragment_sizes.max_recv().to_le_bytes());
    body.extend_from_slice(&association_group_id.to_le_bytes());
    body.push(u8::try_from(presentation_contexts.len()).map_err(|_| RpcPduError::LengthOverflow)?);
    body.push(0); // reserved
    body.extend_from_slice(&0u16.to_le_bytes()); // reserved2
    for context in presentation_contexts {
        body.extend_from_slice(&context.context_id.to_le_bytes());
        body.push(u8::try_from(context.transfer_syntaxes.len()).map_err(|_| RpcPduError::LengthOverflow)?);
        body.push(0); // reserved
        encode_syntax_identifier(&mut body, context.abstract_syntax);
        for syntax in context.transfer_syntaxes {
            encode_syntax_identifier(&mut body, *syntax);
        }
    }

    Ok(body)
}

/// Decodes one complete, unauthenticated RPC bind acknowledgement.
///
/// [C706] 12.6.4.4.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
pub fn decode_rpc_bind_ack(
    source: &[u8],
    offered_fragment_sizes: RpcFragmentSizes,
) -> Result<RpcBindAck<'_>, RpcPduError> {
    let (header, body) = decode_unprotected_single_fragment(source, PTYPE_BIND_ACK, offered_fragment_sizes.max_recv())?;
    decode_rpc_bind_ack_body(header, body, offered_fragment_sizes)
}

/// Decodes an NTLM-authenticated RPC bind acknowledgement and extracts its Type-2 token.
///
/// This validates the DCE/RPC security trailer and header-signing negotiation.
/// Use [`RpcAuthenticatedBindAck::supports_header_signing`] to retain the server result.
/// The caller passes the extracted token to [`RpcNtlmAuth::continue_token`].
///
/// [MS-RPCE] 2.2.2.3, 2.2.2.11, 2.2.2.12, and 4.2.
///
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
pub fn decode_rpc_bind_ack_with_ntlm_auth(
    source: &[u8],
    offered_fragment_sizes: RpcFragmentSizes,
) -> Result<RpcAuthenticatedBindAck<'_>, RpcPduError> {
    let (header, body, token) =
        decode_authenticated_single_fragment(source, PTYPE_BIND_ACK, offered_fragment_sizes.max_recv())?;
    let bind_ack = decode_rpc_bind_ack_body(header, body, offered_fragment_sizes)?;
    let supports_header_signing = header.pfc_flags() & PFC_SUPPORT_HEADER_SIGN != 0;
    Ok(RpcAuthenticatedBindAck {
        bind_ack,
        token,
        supports_header_signing,
    })
}

/// Encodes the NTLM Type-3 continuation token in an rpc_auth_3 PDU.
///
/// The caller supplies the Type-3 token produced by [`RpcNtlmAuth::continue_token`].
/// It does not enable signed RPC request or response PDUs.
///
/// [MS-RPCE] 2.2.2.10, 2.2.2.11, 2.2.2.12, and 4.2.
///
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
pub fn encode_rpc_auth_3(
    call_id: u32,
    fragment_sizes: RpcFragmentSizes,
    type3_token: &[u8],
) -> Result<Vec<u8>, RpcPduError> {
    let pdu = encode_authenticated_pdu(PTYPE_RPC_AUTH_3, call_id, vec![0; 4], type3_token)?;
    ensure_encoded_fragment_fits(&pdu, fragment_sizes.max_xmit())?;
    Ok(pdu)
}
fn decode_rpc_bind_ack_body<'a>(
    header: RpcCommonHeader,
    body: &'a [u8],
    offered_fragment_sizes: RpcFragmentSizes,
) -> Result<RpcBindAck<'a>, RpcPduError> {
    let fixed = body.get(..RPC_BIND_ACK_HEADER_SIZE).ok_or(RpcPduError::Truncated {
        actual: body.len(),
        required: RPC_BIND_ACK_HEADER_SIZE,
    })?;
    let server_fragment_sizes = RpcFragmentSizes::new(read_u16(fixed, 0)?, read_u16(fixed, 2)?)?;
    let secondary_address_end = RPC_BIND_ACK_HEADER_SIZE
        .checked_add(usize::from(read_u16(fixed, 8)?))
        .ok_or(RpcPduError::LengthOverflow)?;
    let result_list_offset = secondary_address_end
        .checked_add(3)
        .ok_or(RpcPduError::LengthOverflow)?
        & !3;
    let result_list_end = result_list_offset.checked_add(4).ok_or(RpcPduError::LengthOverflow)?;
    let result_list = body
        .get(result_list_offset..result_list_end)
        .ok_or(RpcPduError::Truncated {
            actual: body.len(),
            required: result_list_end,
        })?;
    let result_count = usize::from(result_list[0]);
    let results_end = result_list_end
        .checked_add(
            result_count
                .checked_mul(RPC_PRESENTATION_RESULT_SIZE)
                .ok_or(RpcPduError::LengthOverflow)?,
        )
        .ok_or(RpcPduError::LengthOverflow)?;
    if body.len() != results_end {
        return Err(RpcPduError::InvalidBindAckLength {
            actual: body.len(),
            expected: results_end,
        });
    }

    let mut results = Vec::with_capacity(result_count);
    for result in body[result_list_end..].chunks_exact(RPC_PRESENTATION_RESULT_SIZE) {
        results.push(RpcPresentationResult {
            result: read_u16(result, 0)?,
            reason: read_u16(result, 2)?,
            transfer_syntax: decode_syntax_identifier(&result[4..])?,
        });
    }

    let fragment_sizes = RpcFragmentSizes::new(
        offered_fragment_sizes.max_xmit().min(server_fragment_sizes.max_recv()),
        offered_fragment_sizes.max_recv().min(server_fragment_sizes.max_xmit()),
    )?;
    Ok(RpcBindAck {
        call_id: header.call_id(),
        fragment_sizes,
        association_group_id: read_u32(fixed, 4)?,
        secondary_address: &body[RPC_BIND_ACK_HEADER_SIZE..secondary_address_end],
        results,
    })
}

/// Decodes one complete, unauthenticated RPC bind rejection.
///
/// [MS-RPCE] 2.2.2.9 / [C706] 12.6.4.5.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
pub fn decode_rpc_bind_nak(source: &[u8], maximum_fragment_size: u16) -> Result<RpcBindNak, RpcPduError> {
    let (header, body) = decode_unprotected_single_fragment(source, PTYPE_BIND_NAK, maximum_fragment_size)?;
    let fixed = body.get(..3).ok_or(RpcPduError::Truncated {
        actual: body.len(),
        required: 3,
    })?;
    let versions_end = 3usize
        .checked_add(
            usize::from(fixed[2])
                .checked_mul(2)
                .ok_or(RpcPduError::LengthOverflow)?,
        )
        .ok_or(RpcPduError::LengthOverflow)?;
    if body.len() < versions_end || (versions_end < body.len() && body.len() < versions_end + 16) {
        return Err(RpcPduError::InvalidBindNakVersionsLength {
            actual: body.len(),
            expected: versions_end,
        });
    }

    let supported_versions = body[3..versions_end]
        .chunks_exact(2)
        .map(|version| RpcProtocolVersion::new(version[0], version[1]))
        .collect();
    let extended_error_signature = match body.get(versions_end..versions_end + 16) {
        Some(signature) => Some(Uuid::from_bytes_le(
            signature.try_into().map_err(|_| RpcPduError::LengthOverflow)?,
        )),
        None => None,
    };
    Ok(RpcBindNak {
        call_id: header.call_id(),
        reason: read_u16(fixed, 0)?,
        supported_versions,
        extended_error_signature,
    })
}

/// Encodes an unauthenticated RPC request into DCE/RPC fragments.
///
/// Each fragment advertises the remaining stub size through `alloc_hint`.
///
/// [MS-RPCE] 2.2.2.6 / [C706] 12.6.4.8.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
pub fn encode_rpc_request_fragments(
    call_id: u32,
    context_id: u16,
    opnum: u16,
    stub: &[u8],
    fragment_sizes: RpcFragmentSizes,
) -> Result<Vec<Vec<u8>>, RpcPduError> {
    u32::try_from(stub.len()).map_err(|_| RpcPduError::LengthOverflow)?;
    let maximum_stub_size = usize::from(fragment_sizes.max_xmit())
        .checked_sub(RPC_COMMON_HEADER_SIZE + RPC_REQUEST_HEADER_SIZE)
        .ok_or_else(|| RpcPduError::FragmentTooSmall {
            maximum: fragment_sizes.max_xmit(),
            required: RPC_COMMON_HEADER_SIZE + RPC_REQUEST_HEADER_SIZE,
        })?;
    if maximum_stub_size == 0 && !stub.is_empty() {
        return Err(RpcPduError::FragmentTooSmall {
            maximum: fragment_sizes.max_xmit(),
            required: RPC_COMMON_HEADER_SIZE + RPC_REQUEST_HEADER_SIZE + 1,
        });
    }

    let mut fragments = Vec::new();
    let mut offset = 0;
    loop {
        let remaining = stub.len().checked_sub(offset).ok_or(RpcPduError::LengthOverflow)?;
        let fragment_stub_length = remaining.min(maximum_stub_size);
        let mut pfc_flags = if offset == 0 { PFC_FIRST_FRAG } else { 0 };
        if fragment_stub_length == remaining {
            pfc_flags |= PFC_LAST_FRAG;
        }

        let mut body = Vec::with_capacity(RPC_REQUEST_HEADER_SIZE + fragment_stub_length);
        body.extend_from_slice(
            &u32::try_from(remaining)
                .map_err(|_| RpcPduError::LengthOverflow)?
                .to_le_bytes(),
        );
        body.extend_from_slice(&context_id.to_le_bytes());
        body.extend_from_slice(&opnum.to_le_bytes());
        body.extend_from_slice(&stub[offset..offset + fragment_stub_length]);
        fragments.push(encode_unprotected_pdu_with_flags(
            PTYPE_REQUEST,
            pfc_flags,
            call_id,
            body,
        )?);

        if fragment_stub_length == remaining {
            return Ok(fragments);
        }
        offset = offset
            .checked_add(fragment_stub_length)
            .ok_or(RpcPduError::LengthOverflow)?;
    }
}

/// Prepares authenticated RPC request fragments with a verifier reservation in every fragment.
///
/// Each fragment advertises the remaining stub size through `alloc_hint`.
/// The caller must create each verifier from its corresponding [`RpcPreparedAuthenticatedPdu::authentication_segments`].
///
/// [MS-RPCE] 2.2.2.6 and 2.2.2.11 / [C706] 12.6.4.8.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
pub fn prepare_rpc_authenticated_request_fragments(
    call_id: u32,
    context_id: u16,
    opnum: u16,
    stub: &[u8],
    fragment_sizes: RpcFragmentSizes,
    authentication: RpcAuthenticationInfo,
) -> Result<Vec<RpcPreparedAuthenticatedPdu>, RpcPduError> {
    u32::try_from(stub.len()).map_err(|_| RpcPduError::LengthOverflow)?;
    if authentication.verifier_length == 0 {
        return Err(RpcPduError::EmptyAuthenticationVerifier);
    }

    let minimum_fragment_size = RPC_COMMON_HEADER_SIZE
        .checked_add(16 /* request header plus authentication padding */)
        .and_then(|length| length.checked_add(RPC_SECURITY_TRAILER_SIZE))
        .and_then(|length| length.checked_add(usize::from(authentication.verifier_length)))
        .ok_or(RpcPduError::LengthOverflow)?;
    let maximum_padded_body_length = usize::from(fragment_sizes.max_xmit())
        .checked_sub(RPC_COMMON_HEADER_SIZE)
        .and_then(|length| length.checked_sub(RPC_SECURITY_TRAILER_SIZE))
        .and_then(|length| length.checked_sub(usize::from(authentication.verifier_length)))
        .ok_or_else(|| RpcPduError::FragmentTooSmall {
            maximum: fragment_sizes.max_xmit(),
            required: minimum_fragment_size,
        })?
        & !15;
    if maximum_padded_body_length < RPC_REQUEST_HEADER_SIZE {
        return Err(RpcPduError::FragmentTooSmall {
            maximum: fragment_sizes.max_xmit(),
            required: minimum_fragment_size,
        });
    }
    let maximum_stub_size = maximum_padded_body_length
        .checked_sub(RPC_REQUEST_HEADER_SIZE)
        .ok_or(RpcPduError::LengthOverflow)?;

    let mut fragments = Vec::new();
    let mut offset = 0;
    loop {
        let remaining = stub.len().checked_sub(offset).ok_or(RpcPduError::LengthOverflow)?;
        let fragment_stub_length = remaining.min(maximum_stub_size);
        let mut pfc_flags = if offset == 0 { PFC_FIRST_FRAG } else { 0 };
        if fragment_stub_length == remaining {
            pfc_flags |= PFC_LAST_FRAG;
        }

        let mut body = Vec::with_capacity(RPC_REQUEST_HEADER_SIZE + fragment_stub_length);
        body.extend_from_slice(
            &u32::try_from(remaining)
                .map_err(|_| RpcPduError::LengthOverflow)?
                .to_le_bytes(),
        );
        body.extend_from_slice(&context_id.to_le_bytes());
        body.extend_from_slice(&opnum.to_le_bytes());
        body.extend_from_slice(&stub[offset..offset + fragment_stub_length]);
        let pdu = prepare_rpc_authenticated_pdu(PTYPE_REQUEST, pfc_flags, call_id, &body, authentication)?;
        ensure_complete_fragment_fits(pdu.fragment_length(), fragment_sizes.max_xmit())?;
        fragments.push(pdu);

        if fragment_stub_length == remaining {
            return Ok(fragments);
        }
        offset = offset
            .checked_add(fragment_stub_length)
            .ok_or(RpcPduError::LengthOverflow)?;
    }
}

/// Encodes one complete, unauthenticated RPC response PDU.
pub fn encode_rpc_response(call_id: u32, stub: &[u8]) -> Result<Vec<u8>, RpcPduError> {
    let alloc_hint = u32::try_from(stub.len()).map_err(|_| RpcPduError::LengthOverflow)?;
    encode_rpc_response_fragment(RpcResponse {
        call_id,
        pfc_flags: PFC_FIRST_FRAG | PFC_LAST_FRAG,
        alloc_hint,
        cancel_count: 0,
        reserved: 0,
        stub,
    })
}

/// Encodes one unauthenticated RPC response fragment.
pub fn encode_rpc_response_fragment(response: RpcResponse<'_>) -> Result<Vec<u8>, RpcPduError> {
    let mut body = Vec::with_capacity(RPC_RESPONSE_HEADER_SIZE + response.stub.len());
    body.extend_from_slice(&response.alloc_hint.to_le_bytes());
    body.extend_from_slice(&RPC_CONTEXT_ID.to_le_bytes());
    body.push(response.cancel_count);
    body.push(0); // reserved
    body.extend_from_slice(response.stub);
    encode_unprotected_pdu_with_flags(PTYPE_RESPONSE, response.pfc_flags, response.call_id, body)
}

/// Encodes one complete, unauthenticated RPC fault PDU.
pub fn encode_rpc_fault(fault: RpcFault<'_>) -> Result<Vec<u8>, RpcPduError> {
    let mut body = Vec::with_capacity(RPC_FAULT_HEADER_SIZE + fault.stub.len());
    body.extend_from_slice(&fault.alloc_hint.to_le_bytes());
    body.extend_from_slice(&RPC_CONTEXT_ID.to_le_bytes());
    body.push(fault.cancel_count);
    body.push(fault.reserved);
    body.extend_from_slice(&fault.status.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes()); // reserved2
    body.extend_from_slice(fault.stub);
    encode_unprotected_pdu(PTYPE_FAULT, fault.call_id, body)
}

/// Decodes one complete, unauthenticated RPC response PDU.
pub fn decode_rpc_response(source: &[u8], maximum_fragment_size: u16) -> Result<RpcResponse<'_>, RpcPduError> {
    decode_rpc_response_for_context(source, maximum_fragment_size, RPC_CONTEXT_ID)
}

/// Decodes one complete, unauthenticated RPC response PDU for `context_id`.
///
/// [C706] 12.6.4.9.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
pub fn decode_rpc_response_for_context(
    source: &[u8],
    maximum_fragment_size: u16,
    context_id: u16,
) -> Result<RpcResponse<'_>, RpcPduError> {
    let (header, body) = decode_unprotected_single_fragment(source, PTYPE_RESPONSE, maximum_fragment_size)?;
    let response = decode_rpc_response_body(header, body, context_id)?;
    validate_single_response_alloc_hint(response)?;
    Ok(response)
}

/// Decodes one unauthenticated RPC response fragment for a response reassembler.
pub fn decode_rpc_response_fragment(source: &[u8], maximum_fragment_size: u16) -> Result<RpcResponse<'_>, RpcPduError> {
    decode_rpc_response_fragment_for_context(source, maximum_fragment_size, RPC_CONTEXT_ID)
}

/// Decodes one unauthenticated RPC response fragment for `context_id`.
///
/// [C706] 12.6.4.9.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
pub fn decode_rpc_response_fragment_for_context(
    source: &[u8],
    maximum_fragment_size: u16,
    context_id: u16,
) -> Result<RpcResponse<'_>, RpcPduError> {
    let (header, body) = decode_unprotected_fragment(source, PTYPE_RESPONSE, maximum_fragment_size)?;
    decode_rpc_response_body(header, body, context_id)
}

/// Decodes one complete, unauthenticated RPC fault PDU.
pub fn decode_rpc_fault(source: &[u8], maximum_fragment_size: u16) -> Result<RpcFault<'_>, RpcPduError> {
    decode_rpc_fault_for_context(source, maximum_fragment_size, RPC_CONTEXT_ID)
}

/// Decodes one complete, unauthenticated RPC fault PDU for `context_id`.
///
/// [MS-RPCE] 2.2.2.8 / [C706] 12.6.4.9.
///
/// [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
/// [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d
pub fn decode_rpc_fault_for_context(
    source: &[u8],
    maximum_fragment_size: u16,
    context_id: u16,
) -> Result<RpcFault<'_>, RpcPduError> {
    let (header, body) = decode_unprotected_single_fragment(source, PTYPE_FAULT, maximum_fragment_size)?;
    let fault_header = body.get(..RPC_FAULT_HEADER_SIZE).ok_or(RpcPduError::Truncated {
        actual: body.len(),
        required: RPC_FAULT_HEADER_SIZE,
    })?;
    let actual_context_id = read_u16(fault_header, 4)?;
    if actual_context_id != context_id {
        return Err(RpcPduError::UnexpectedContextId {
            expected: context_id,
            actual: actual_context_id,
        });
    }

    Ok(RpcFault {
        call_id: header.call_id(),
        pfc_flags: header.pfc_flags(),
        alloc_hint: read_u32(fault_header, 0)?,
        cancel_count: fault_header[6],
        reserved: fault_header[7],
        status: read_u32(fault_header, 8)?,
        reserved2: read_u32(fault_header, 12)?,
        stub: &body[RPC_FAULT_HEADER_SIZE..],
    })
}

fn decode_rpc_response_body<'a>(
    header: RpcCommonHeader,
    body: &'a [u8],
    context_id: u16,
) -> Result<RpcResponse<'a>, RpcPduError> {
    let request_header = body.get(..RPC_RESPONSE_HEADER_SIZE).ok_or(RpcPduError::Truncated {
        actual: body.len(),
        required: RPC_RESPONSE_HEADER_SIZE,
    })?;
    let alloc_hint = read_u32(request_header, 0)?;
    let actual_context_id = read_u16(request_header, 4)?;
    if actual_context_id != context_id {
        return Err(RpcPduError::UnexpectedContextId {
            expected: context_id,
            actual: actual_context_id,
        });
    }
    let stub = &body[RPC_RESPONSE_HEADER_SIZE..];
    Ok(RpcResponse {
        call_id: header.call_id(),
        pfc_flags: header.pfc_flags(),
        alloc_hint,
        cancel_count: request_header[6],
        reserved: request_header[7],
        stub,
    })
}

fn validate_single_response_alloc_hint(response: RpcResponse<'_>) -> Result<(), RpcPduError> {
    if response.alloc_hint != 0 && usize::try_from(response.alloc_hint).map_or(true, |hint| hint > response.stub.len())
    {
        return Err(RpcPduError::InvalidAllocHint {
            alloc_hint: response.alloc_hint,
            stub_length: response.stub.len(),
        });
    }

    Ok(())
}

fn decode_unprotected_fragment(
    source: &[u8],
    expected_ptype: u8,
    maximum_fragment_size: u16,
) -> Result<(RpcCommonHeader, &[u8]), RpcPduError> {
    let header = RpcCommonHeader::decode(source)?;
    if header.ptype() != expected_ptype {
        return Err(RpcPduError::UnexpectedPduType {
            expected: expected_ptype,
            actual: header.ptype(),
        });
    }
    if header.fragment_length() > maximum_fragment_size {
        return Err(RpcPduError::FragmentExceedsMaximum {
            fragment_length: header.fragment_length(),
            maximum: maximum_fragment_size,
        });
    }
    if header.auth_length() != 0 {
        return Err(RpcPduError::AuthenticationUnsupported {
            auth_length: header.auth_length(),
        });
    }

    Ok((
        header,
        &source[RPC_COMMON_HEADER_SIZE..usize::from(header.fragment_length())],
    ))
}

fn decode_unprotected_single_fragment(
    source: &[u8],
    expected_ptype: u8,
    maximum_fragment_size: u16,
) -> Result<(RpcCommonHeader, &[u8]), RpcPduError> {
    let (header, body) = decode_unprotected_fragment(source, expected_ptype, maximum_fragment_size)?;
    validate_single_fragment(header)?;
    Ok((header, body))
}

fn encode_authenticated_pdu(ptype: u8, call_id: u32, mut body: Vec<u8>, token: &[u8]) -> Result<Vec<u8>, RpcPduError> {
    if token.is_empty() {
        return Err(RpcPduError::EmptyAuthenticationToken);
    }

    let auth_length = u16::try_from(token.len()).map_err(|_| RpcPduError::LengthOverflow)?;
    let padding_length = (16 - body.len() % 16) % 16;
    body.resize(
        body.len()
            .checked_add(padding_length)
            .ok_or(RpcPduError::LengthOverflow)?,
        0,
    );
    let header = RpcCommonHeader::encode(
        ptype,
        PFC_FIRST_FRAG | PFC_LAST_FRAG | PFC_SUPPORT_HEADER_SIGN,
        call_id,
        body.len(),
        auth_length,
    )?;
    let mut pdu = Vec::with_capacity(
        RPC_COMMON_HEADER_SIZE
            .checked_add(body.len())
            .and_then(|length| length.checked_add(RPC_SEC_TRAILER_SIZE))
            .and_then(|length| length.checked_add(token.len()))
            .ok_or(RpcPduError::LengthOverflow)?,
    );
    pdu.extend_from_slice(&header);
    pdu.extend_from_slice(&body);
    pdu.push(RPC_AUTH_TYPE_WINNT);
    pdu.push(RPC_AUTH_LEVEL_PACKET_INTEGRITY);
    pdu.push(u8::try_from(padding_length).map_err(|_| RpcPduError::LengthOverflow)?);
    pdu.push(0); // auth_reserved
    pdu.extend_from_slice(&RPC_AUTH_CONTEXT_ID.to_le_bytes());
    pdu.extend_from_slice(token);
    Ok(pdu)
}

fn decode_authenticated_single_fragment(
    source: &[u8],
    expected_ptype: u8,
    maximum_fragment_size: u16,
) -> Result<(RpcCommonHeader, &[u8], &[u8]), RpcPduError> {
    let header = RpcCommonHeader::decode(source)?;
    if header.ptype() != expected_ptype {
        return Err(RpcPduError::UnexpectedPduType {
            expected: expected_ptype,
            actual: header.ptype(),
        });
    }
    if header.fragment_length() > maximum_fragment_size {
        return Err(RpcPduError::FragmentExceedsMaximum {
            fragment_length: header.fragment_length(),
            maximum: maximum_fragment_size,
        });
    }
    if header.auth_length() == 0 {
        return Err(RpcPduError::AuthenticationRequired);
    }
    validate_single_fragment(header)?;

    let fragment_length = usize::from(header.fragment_length());
    let trailer_offset = fragment_length
        .checked_sub(usize::from(header.auth_length()))
        .and_then(|offset| offset.checked_sub(RPC_SEC_TRAILER_SIZE))
        .ok_or_else(|| RpcPduError::InvalidSecurityTrailer {
            fragment_length: header.fragment_length(),
            auth_length: header.auth_length(),
        })?;
    if trailer_offset < RPC_COMMON_HEADER_SIZE {
        return Err(RpcPduError::InvalidSecurityTrailer {
            fragment_length: header.fragment_length(),
            auth_length: header.auth_length(),
        });
    }

    let trailer_end = trailer_offset
        .checked_add(RPC_SEC_TRAILER_SIZE)
        .ok_or(RpcPduError::LengthOverflow)?;
    let trailer = source
        .get(trailer_offset..trailer_end)
        .ok_or_else(|| RpcPduError::InvalidSecurityTrailer {
            fragment_length: header.fragment_length(),
            auth_length: header.auth_length(),
        })?;
    if trailer[0] != RPC_AUTH_TYPE_WINNT {
        return Err(RpcPduError::UnexpectedAuthenticationType {
            expected: RPC_AUTH_TYPE_WINNT,
            actual: trailer[0],
        });
    }
    if trailer[1] != RPC_AUTH_LEVEL_PACKET_INTEGRITY {
        return Err(RpcPduError::UnexpectedAuthenticationLevel {
            expected: RPC_AUTH_LEVEL_PACKET_INTEGRITY,
            actual: trailer[1],
        });
    }
    if trailer[3] != 0 {
        return Err(RpcPduError::NonZeroAuthenticationReserved { actual: trailer[3] });
    }
    let auth_context_id = u32::from_le_bytes(trailer[4..8].try_into().map_err(|_| RpcPduError::LengthOverflow)?);
    if auth_context_id != RPC_AUTH_CONTEXT_ID {
        return Err(RpcPduError::UnexpectedAuthenticationContextId {
            expected: RPC_AUTH_CONTEXT_ID,
            actual: auth_context_id,
        });
    }

    let body_with_padding = &source[RPC_COMMON_HEADER_SIZE..trailer_offset];
    let padding_length = usize::from(trailer[2]);
    // The gateway profile accepts canonical zero-filled padding and uses
    // auth_pad_length rather than a fixed bind_ack layout.
    let body_length = body_with_padding
        .len()
        .checked_sub(padding_length)
        .ok_or(RpcPduError::InvalidAuthenticationPadding { actual: trailer[2] })?;
    if body_with_padding[body_length..].iter().any(|&byte| byte != 0) {
        return Err(RpcPduError::NonZeroAuthenticationPadding);
    }

    let token = source
        .get(trailer_end..fragment_length)
        .ok_or_else(|| RpcPduError::InvalidSecurityTrailer {
            fragment_length: header.fragment_length(),
            auth_length: header.auth_length(),
        })?;
    debug_assert_eq!(token.len(), usize::from(header.auth_length()));
    Ok((header, &body_with_padding[..body_length], token))
}

fn encode_unprotected_pdu(ptype: u8, call_id: u32, body: Vec<u8>) -> Result<Vec<u8>, RpcPduError> {
    encode_unprotected_pdu_with_flags(ptype, PFC_FIRST_FRAG | PFC_LAST_FRAG, call_id, body)
}

fn encode_unprotected_pdu_with_flags(
    ptype: u8,
    pfc_flags: u8,
    call_id: u32,
    body: Vec<u8>,
) -> Result<Vec<u8>, RpcPduError> {
    let header = RpcCommonHeader::encode(ptype, pfc_flags, call_id, body.len(), 0)?;
    let mut pdu = Vec::with_capacity(RPC_COMMON_HEADER_SIZE + body.len());
    pdu.extend_from_slice(&header);
    pdu.extend_from_slice(&body);
    Ok(pdu)
}

fn validate_single_fragment(header: RpcCommonHeader) -> Result<(), RpcPduError> {
    if header.pfc_flags() & (PFC_FIRST_FRAG | PFC_LAST_FRAG) != PFC_FIRST_FRAG | PFC_LAST_FRAG {
        return Err(RpcPduError::FragmentedPduUnsupported {
            flags: header.pfc_flags(),
        });
    }

    Ok(())
}

fn ensure_fragment_fits(body_length: usize, maximum_fragment_size: u16) -> Result<(), RpcPduError> {
    let fragment_length = RPC_COMMON_HEADER_SIZE
        .checked_add(body_length)
        .ok_or(RpcPduError::LengthOverflow)?;
    let fragment_length = u16::try_from(fragment_length).map_err(|_| RpcPduError::LengthOverflow)?;
    if fragment_length > maximum_fragment_size {
        return Err(RpcPduError::FragmentExceedsMaximum {
            fragment_length,
            maximum: maximum_fragment_size,
        });
    }

    Ok(())
}

fn ensure_encoded_fragment_fits(pdu: &[u8], maximum_fragment_size: u16) -> Result<(), RpcPduError> {
    ensure_complete_fragment_fits(pdu.len(), maximum_fragment_size)
}

fn ensure_complete_fragment_fits(fragment_length: usize, maximum_fragment_size: u16) -> Result<(), RpcPduError> {
    let fragment_length = u16::try_from(fragment_length).map_err(|_| RpcPduError::LengthOverflow)?;
    if fragment_length > maximum_fragment_size {
        return Err(RpcPduError::FragmentExceedsMaximum {
            fragment_length,
            maximum: maximum_fragment_size,
        });
    }

    Ok(())
}

fn encode_syntax_identifier(target: &mut Vec<u8>, syntax: RpcSyntaxIdentifier) {
    target.extend_from_slice(&syntax.uuid().to_bytes_le());
    target.extend_from_slice(&syntax.version().major().to_le_bytes());
    target.extend_from_slice(&syntax.version().minor().to_le_bytes());
}

fn decode_syntax_identifier(source: &[u8]) -> Result<RpcSyntaxIdentifier, RpcPduError> {
    let source = source.get(..RPC_SYNTAX_IDENTIFIER_SIZE).ok_or(RpcPduError::Truncated {
        actual: source.len(),
        required: RPC_SYNTAX_IDENTIFIER_SIZE,
    })?;
    Ok(RpcSyntaxIdentifier::new(
        Uuid::from_bytes_le(source[..16].try_into().map_err(|_| RpcPduError::LengthOverflow)?),
        RpcSyntaxVersion::new(read_u16(source, 16)?, read_u16(source, 18)?),
    ))
}

#[expect(
    dead_code,
    reason = "the control codecs are staged before an RPC transport consumes them"
)]
pub(crate) mod tsgu {
    use core::fmt;

    use super::RpcSyntaxVersion;
    use uuid::Uuid;

    /// TsProxy RPC interface identifier.
    ///
    /// [MS-TSGU] 3.2.1 and Appendix A.
    ///
    /// [MS-TSGU]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-tsgu/0007d661-a86d-4e8f-89f7-7f77f8824188
    pub(crate) const TSPROXY_RPC_INTERFACE_ID: Uuid = Uuid::from_u128(0x44e265dd_7daf_42cd_8560_3cdb6e7a2729);
    /// TsProxy RPC interface version.
    pub(crate) const TSPROXY_RPC_INTERFACE_VERSION: RpcSyntaxVersion = RpcSyntaxVersion::new(1, 3);
    /// NDR32 transfer-syntax identifier.
    pub(crate) const NDR32_TRANSFER_SYNTAX_ID: Uuid = Uuid::from_u128(0x8a885d04_1ceb_11c9_9fe8_08002b104860);
    /// NDR32 transfer-syntax version.
    pub(crate) const NDR32_TRANSFER_SYNTAX_VERSION: RpcSyntaxVersion = RpcSyntaxVersion::new(2, 0);

    /// `TsProxyCreateTunnel` operation number.
    pub(crate) const TSPROXY_CREATE_TUNNEL_OPNUM: u16 = 1;
    /// `TsProxyAuthorizeTunnel` operation number.
    pub(crate) const TSPROXY_AUTHORIZE_TUNNEL_OPNUM: u16 = 2;
    /// `TsProxyMakeTunnelCall` operation number.
    pub(crate) const TSPROXY_MAKE_TUNNEL_CALL_OPNUM: u16 = 3;
    /// `TsProxyCreateChannel` operation number.
    pub(crate) const TSPROXY_CREATE_CHANNEL_OPNUM: u16 = 4;
    /// `TsProxyCloseChannel` operation number.
    pub(crate) const TSPROXY_CLOSE_CHANNEL_OPNUM: u16 = 6;
    /// `TsProxyCloseTunnel` operation number.
    pub(crate) const TSPROXY_CLOSE_TUNNEL_OPNUM: u16 = 7;
    /// `TsProxySetupReceivePipe` operation number.
    pub(crate) const TSPROXY_SETUP_RECEIVE_PIPE_OPNUM: u16 = 8;
    /// `TsProxySendToServer` operation number.
    pub(crate) const TSPROXY_SEND_TO_SERVER_OPNUM: u16 = 9;

    const NDR_REFERENT_ID: u32 = 0x0002_0000;
    const TSG_COMPONENT_ID: u16 = 0x5452;
    const TSG_PACKET_TYPE_VERSIONCAPS: u32 = 0x0000_5643;
    const TSG_PACKET_TYPE_VERSIONCAPS_ID: u16 = 0x5643;
    const TSG_PACKET_TYPE_QUARREQUEST: u32 = 0x0000_5152;
    const TSG_PACKET_TYPE_RESPONSE: u32 = 0x0000_5052;
    const TSG_PACKET_TYPE_QUARENC_RESPONSE: u32 = 0x0000_4552;
    const TSG_PACKET_TYPE_MSGREQUEST: u32 = 0x0000_4752;
    const TSG_PACKET_TYPE_MESSAGE: u32 = 0x0000_4750;
    const TSG_TUNNEL_CALL_ASYNC_MSG_REQUEST: u32 = 1;
    const TSG_TUNNEL_CANCEL_ASYNC_MSG_REQUEST: u32 = 2;
    const TSG_ASYNC_MESSAGE_CONSENT: u32 = 1;
    const TSG_ASYNC_MESSAGE_SERVICE: u32 = 2;
    const TSG_ASYNC_MESSAGE_REAUTH: u32 = 3;
    const TSG_CAPABILITY_TYPE_NAP: u32 = 1;
    const TSG_NAP_CAPABILITY_QUAR_SOH: u32 = 0x0000_0001;
    const TSG_NAP_CAPABILITY_IDLE_TIMEOUT: u32 = 0x0000_0002;
    const SUPPORTED_CAPABILITIES: u32 = TSG_NAP_CAPABILITY_QUAR_SOH | TSG_NAP_CAPABILITY_IDLE_TIMEOUT;
    const TSG_PROTOCOL_VERSION: u16 = 1;
    const E_PROXY_QUARANTINE_ACCESSDENIED: u32 = 0x8007_59ed;
    const MAX_CAPABILITIES: usize = 32;
    const MAX_MACHINE_NAME_CHARS: usize = 513;
    const MAX_RESOURCE_NAME_CHARS: usize = 32_767;
    const MAX_STATEMENT_OF_HEALTH_SIZE: usize = 8_000;
    const MAX_RESPONSE_DATA_SIZE: usize = 24_000;
    const MAX_CERT_CHAIN_CHARS: usize = 24_000;
    const MAX_MESSAGE_CHARS: usize = 65_536;
    const MAX_RAW_RPC_STUB_SIZE: usize = 32_767;

    /// A TS Gateway RPC context handle in its 20-byte wire representation.
    ///
    /// [MS-TSGU] 2.2.2.1 through 2.2.2.4 and 3.2.2.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(crate) struct RpcContextHandle([u8; Self::SIZE]);

    impl RpcContextHandle {
        pub(crate) const SIZE: usize = 20;

        pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, RpcStubError> {
            let bytes: &[u8; Self::SIZE] = bytes
                .try_into()
                .map_err(|_| RpcStubError::ContextHandleLength { actual: bytes.len() })?;
            Ok(Self(*bytes))
        }

        pub(crate) const fn as_bytes(self) -> [u8; Self::SIZE] {
            self.0
        }

        pub(crate) fn require_non_null(self) -> Result<NonNullRpcContextHandle, RpcStubError> {
            if self.0.iter().all(|byte| *byte == 0) {
                return Err(RpcStubError::NullContextHandle);
            }

            Ok(NonNullRpcContextHandle(self))
        }
    }

    impl fmt::Debug for RpcContextHandle {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("RpcContextHandle(..)")
        }
    }

    /// A validated non-null TS Gateway RPC context handle.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct NonNullRpcContextHandle(RpcContextHandle);

    impl NonNullRpcContextHandle {
        const fn as_bytes(self) -> [u8; RpcContextHandle::SIZE] {
            self.0.as_bytes()
        }
    }

    /// Errors reported by the bounded NDR32 TS Gateway control-stub codecs.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) enum RpcStubError {
        ContextHandleLength { actual: usize },
        NullContextHandle,
        EmptyResourceName,
        EmbeddedNulInResourceName,
        EmbeddedNulInMachineName,
        ResourceNameTooLong { actual: usize, maximum: usize },
        MachineNameTooLong { actual: usize, maximum: usize },
        StatementOfHealthTooLarge { actual: usize },
        ResponseDataTooLarge { actual: usize },
        CertificateChainTooLarge { actual: usize },
        CapabilityCountTooLarge { actual: u32 },
        MessageTooLong { actual: usize },
        RawStubTooLarge { actual: usize },
        BufferCount { actual: usize },
        EmptyFirstBuffer,
        LengthOverflow,
        ResponseLength { actual: usize, expected: usize },
        RequiredNdrPointerIsNull,
        UnexpectedNdrPointer { actual: u32 },
        UnexpectedPacketId { expected: u32, actual: u32 },
        UnexpectedPacketSwitch { expected: u32, actual: u32 },
        UnexpectedMessageSwitch { expected: u32, actual: u32 },
        UnexpectedComponentId { expected: u16, actual: u16 },
        UnexpectedProtocolVersion { major: u16, minor: u16 },
        UnexpectedCapabilityType { expected: u32, actual: u32 },
        UnsupportedCapabilities { actual: u32 },
        InvalidQuarantineCapabilities { actual: u16 },
        MissingCertificateChain,
        InvalidNdrArrayLength { actual: u32, expected: u32 },
        InvalidNdrBoolean { value: u32 },
        NonZeroReservedRedirectionFlag,
        ConflictingRedirectionFlags,
        InvalidQuarencFlags { actual: u32 },
        InvalidUtf16,
        UnterminatedNdrString,
        InvalidMessageType { actual: u32 },
        RpcStatus { value: u32 },
        QuarantineAccessDenied { response_data: Vec<u8> },
    }

    impl fmt::Display for RpcStubError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::ContextHandleLength { actual } => {
                    write!(
                        f,
                        "invalid rpc context handle length {actual}, expected {}",
                        RpcContextHandle::SIZE
                    )
                }
                Self::NullContextHandle => f.write_str("rpc context handle must not be null"),
                Self::EmptyResourceName => f.write_str("resource name must not be empty"),
                Self::EmbeddedNulInResourceName => f.write_str("resource name must not contain a nul character"),
                Self::EmbeddedNulInMachineName => f.write_str("machine name must not contain a nul character"),
                Self::ResourceNameTooLong { actual, maximum } => {
                    write!(f, "resource name length {actual} exceeds {maximum}")
                }
                Self::MachineNameTooLong { actual, maximum } => {
                    write!(f, "machine name length {actual} exceeds {maximum}")
                }
                Self::StatementOfHealthTooLarge { actual } => {
                    write!(
                        f,
                        "statement of health length {actual} exceeds {MAX_STATEMENT_OF_HEALTH_SIZE}"
                    )
                }
                Self::ResponseDataTooLarge { actual } => {
                    write!(f, "response data length {actual} exceeds {MAX_RESPONSE_DATA_SIZE}")
                }
                Self::CertificateChainTooLarge { actual } => {
                    write!(f, "certificate chain length {actual} exceeds {MAX_CERT_CHAIN_CHARS}")
                }
                Self::CapabilityCountTooLarge { actual } => {
                    write!(f, "capability count {actual} exceeds {MAX_CAPABILITIES}")
                }
                Self::MessageTooLong { actual } => write!(f, "message length {actual} exceeds {MAX_MESSAGE_CHARS}"),
                Self::RawStubTooLarge { actual } => {
                    write!(f, "raw rpc stub length {actual} exceeds {MAX_RAW_RPC_STUB_SIZE}")
                }
                Self::BufferCount { actual } => write!(f, "invalid buffer count {actual}, expected 1 through 3"),
                Self::EmptyFirstBuffer => f.write_str("first buffer must not be empty"),
                Self::LengthOverflow => f.write_str("rpc stub length overflow"),
                Self::ResponseLength { actual, expected } => {
                    write!(f, "invalid rpc stub length {actual}, expected {expected}")
                }
                Self::RequiredNdrPointerIsNull => f.write_str("required ndr pointer is null"),
                Self::UnexpectedNdrPointer { actual } => write!(f, "unexpected ndr pointer value 0x{actual:08x}"),
                Self::UnexpectedPacketId { expected, actual } => {
                    write!(f, "unexpected packet id 0x{actual:08x}, expected 0x{expected:08x}")
                }
                Self::UnexpectedPacketSwitch { expected, actual } => {
                    write!(f, "unexpected packet switch 0x{actual:08x}, expected 0x{expected:08x}")
                }
                Self::UnexpectedMessageSwitch { expected, actual } => {
                    write!(f, "unexpected message switch {actual}, expected {expected}")
                }
                Self::UnexpectedComponentId { expected, actual } => {
                    write!(f, "unexpected component id 0x{actual:04x}, expected 0x{expected:04x}")
                }
                Self::UnexpectedProtocolVersion { major, minor } => {
                    write!(f, "unexpected protocol version {major}.{minor}, expected 1.1")
                }
                Self::UnexpectedCapabilityType { expected, actual } => {
                    write!(f, "unexpected capability type {actual}, expected {expected}")
                }
                Self::UnsupportedCapabilities { actual } => {
                    write!(f, "unsupported capabilities 0x{actual:08x}")
                }
                Self::InvalidQuarantineCapabilities { actual } => {
                    write!(f, "invalid quarantine capabilities 0x{actual:04x}")
                }
                Self::MissingCertificateChain => f.write_str("quarantine support requires a certificate chain"),
                Self::InvalidNdrArrayLength { actual, expected } => {
                    write!(f, "invalid ndr array length {actual}, expected {expected}")
                }
                Self::InvalidNdrBoolean { value } => write!(f, "invalid ndr boolean value {value}"),
                Self::NonZeroReservedRedirectionFlag => f.write_str("reserved redirection flag must be zero"),
                Self::ConflictingRedirectionFlags => f.write_str("enable-all and disable-all flags conflict"),
                Self::InvalidQuarencFlags { actual } => write!(f, "invalid quarenc flags {actual}"),
                Self::InvalidUtf16 => f.write_str("invalid utf-16 string"),
                Self::UnterminatedNdrString => f.write_str("unterminated ndr string"),
                Self::InvalidMessageType { actual } => write!(f, "invalid message type {actual}"),
                Self::RpcStatus { value } => write!(f, "rpc operation returned hresult 0x{value:08x}"),
                Self::QuarantineAccessDenied { .. } => {
                    f.write_str("rpc operation denied access due to quarantine policy")
                }
            }
        }
    }

    impl core::error::Error for RpcStubError {}

    /// NDR32 `TsProxyCreateTunnel` request stub.
    ///
    /// [MS-TSGU] 2.2.9.2.1.2 and 3.2.6.1.1.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct TsProxyCreateTunnelRequest {
        capabilities: u32,
    }

    impl TsProxyCreateTunnelRequest {
        pub(crate) const fn new(capabilities: u32) -> Self {
            Self { capabilities }
        }

        pub(crate) fn encode(self) -> Result<Vec<u8>, RpcStubError> {
            validate_capabilities(self.capabilities)?;
            let mut output = Vec::with_capacity(48);
            output.extend_from_slice(&TSG_PACKET_TYPE_VERSIONCAPS.to_le_bytes()); // packetId
            output.extend_from_slice(&TSG_PACKET_TYPE_VERSIONCAPS.to_le_bytes()); // union switch
            encode_ndr_pointer(&mut output, 0); // packetVersionCaps
            output.extend_from_slice(&TSG_COMPONENT_ID.to_le_bytes()); // componentId
            output.extend_from_slice(&TSG_PACKET_TYPE_VERSIONCAPS_ID.to_le_bytes()); // packetId
            encode_ndr_pointer(&mut output, 1); // TSGCaps
            output.extend_from_slice(&1u32.to_le_bytes()); // numCapabilities
            output.extend_from_slice(&1u16.to_le_bytes()); // majorVersion
            output.extend_from_slice(&1u16.to_le_bytes()); // minorVersion
            output.extend_from_slice(&0u16.to_le_bytes()); // quarantineCapabilities
            output.extend_from_slice(&0u16.to_le_bytes()); // NDR alignment
            output.extend_from_slice(&1u32.to_le_bytes()); // TSGCaps max count
            output.extend_from_slice(&TSG_CAPABILITY_TYPE_NAP.to_le_bytes()); // capabilityType
            output.extend_from_slice(&TSG_CAPABILITY_TYPE_NAP.to_le_bytes()); // union switch
            output.extend_from_slice(&self.capabilities.to_le_bytes()); // capabilities
            Ok(output)
        }
    }

    /// NDR32 `TsProxyAuthorizeTunnel` request stub.
    ///
    /// [MS-TSGU] 2.2.9.2.1.4 and 3.2.6.1.2.
    #[derive(Debug)]
    pub(crate) struct TsProxyAuthorizeTunnelRequest<'a> {
        tunnel_context: NonNullRpcContextHandle,
        machine_name: &'a str,
        statement_of_health: &'a [u8],
    }

    impl<'a> TsProxyAuthorizeTunnelRequest<'a> {
        pub(crate) const fn new(
            tunnel_context: NonNullRpcContextHandle,
            machine_name: &'a str,
            statement_of_health: &'a [u8],
        ) -> Self {
            Self {
                tunnel_context,
                machine_name,
                statement_of_health,
            }
        }

        pub(crate) fn encode(&self) -> Result<Vec<u8>, RpcStubError> {
            let machine_name = encode_ndr_machine_name(self.machine_name)?;
            if self.statement_of_health.len() > MAX_STATEMENT_OF_HEALTH_SIZE {
                return Err(RpcStubError::StatementOfHealthTooLarge {
                    actual: self.statement_of_health.len(),
                });
            }

            let machine_name_len = u32::try_from(machine_name.len()).map_err(|_| RpcStubError::LengthOverflow)?;
            let statement_of_health_len =
                u32::try_from(self.statement_of_health.len()).map_err(|_| RpcStubError::LengthOverflow)?;
            let mut output = Vec::with_capacity(52 + machine_name.len() * 2 + self.statement_of_health.len());
            output.extend_from_slice(&self.tunnel_context.as_bytes()); // tunnelContext
            output.extend_from_slice(&TSG_PACKET_TYPE_QUARREQUEST.to_le_bytes()); // packetId
            output.extend_from_slice(&TSG_PACKET_TYPE_QUARREQUEST.to_le_bytes()); // union switch
            encode_ndr_pointer(&mut output, 0); // packetQuarRequest
            output.extend_from_slice(&0u32.to_le_bytes()); // flags
            encode_ndr_pointer(&mut output, 1); // machineName
            output.extend_from_slice(&machine_name_len.to_le_bytes()); // nameLength
            if self.statement_of_health.is_empty() {
                output.extend_from_slice(&0u32.to_le_bytes()); // data
            } else {
                encode_ndr_pointer(&mut output, 2); // data
            }
            output.extend_from_slice(&statement_of_health_len.to_le_bytes()); // dataLen
            encode_ndr_string_referent(&mut output, &machine_name)?;
            if !self.statement_of_health.is_empty() {
                output.extend_from_slice(&statement_of_health_len.to_le_bytes()); // data max count
                output.extend_from_slice(self.statement_of_health);
                pad_ndr_4(&mut output);
            }
            Ok(output)
        }
    }

    /// NDR32 `TsProxyCreateChannel` request stub.
    ///
    /// [MS-TSGU] 2.2.9.1 and 3.2.6.1.4.
    #[derive(Debug)]
    pub(crate) struct TsProxyCreateChannelRequest<'a> {
        tunnel_context: NonNullRpcContextHandle,
        resource_name: &'a str,
        port: u16,
    }

    impl<'a> TsProxyCreateChannelRequest<'a> {
        pub(crate) const fn new(tunnel_context: NonNullRpcContextHandle, resource_name: &'a str, port: u16) -> Self {
            Self {
                tunnel_context,
                resource_name,
                port,
            }
        }

        pub(crate) fn encode(&self) -> Result<Vec<u8>, RpcStubError> {
            let resource_name = encode_ndr_resource_name(self.resource_name)?;
            let mut output = Vec::with_capacity(48 + resource_name.len() * 2);
            output.extend_from_slice(&self.tunnel_context.as_bytes()); // tunnelContext
            encode_ndr_pointer(&mut output, 0); // resourceName
            output.extend_from_slice(&1u32.to_le_bytes()); // numResourceNames
            output.extend_from_slice(&0u32.to_le_bytes()); // alternateResourceNames
            output.extend_from_slice(&0u16.to_le_bytes()); // numAlternateResourceNames
            output.extend_from_slice(&0u16.to_le_bytes()); // NDR alignment
            output.extend_from_slice(&((u32::from(self.port) << 16) | 3).to_le_bytes()); // port
            output.extend_from_slice(&1u32.to_le_bytes()); // resourceName max count
            encode_ndr_pointer(&mut output, 1); // resourceName item
            encode_ndr_string_referent(&mut output, &resource_name)?;
            Ok(output)
        }
    }

    /// NDR32 `TsProxyMakeTunnelCall` request for an administrative message.
    ///
    /// [MS-TSGU] 2.2.9.2.1.8 and 3.2.6.1.3.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct TsProxyMakeTunnelCallRequest {
        tunnel_context: NonNullRpcContextHandle,
        proc_id: u32,
    }

    impl TsProxyMakeTunnelCallRequest {
        const SIZE: usize = RpcContextHandle::SIZE /* tunnelContext */
            + 4 /* procId */
            + 4 /* packetId */
            + 4 /* union switch */
            + 4 /* packetMsgRequest */
            + 4; /* maxMessagesPerBatch */

        pub(crate) const fn new(tunnel_context: NonNullRpcContextHandle) -> Self {
            Self {
                tunnel_context,
                proc_id: TSG_TUNNEL_CALL_ASYNC_MSG_REQUEST,
            }
        }

        /// Cancels an outstanding asynchronous message request during shutdown.
        ///
        /// [MS-TSGU] 3.2.6.3.2.
        pub(crate) const fn cancel_pending(tunnel_context: NonNullRpcContextHandle) -> Self {
            Self {
                tunnel_context,
                proc_id: TSG_TUNNEL_CANCEL_ASYNC_MSG_REQUEST,
            }
        }

        pub(crate) fn encode(self) -> [u8; Self::SIZE] {
            let mut output = [0; Self::SIZE];
            let mut offset = 0;
            output[offset..offset + RpcContextHandle::SIZE].copy_from_slice(&self.tunnel_context.as_bytes());
            offset += RpcContextHandle::SIZE;
            output[offset..offset + 4].copy_from_slice(&self.proc_id.to_le_bytes());
            offset += 4;
            output[offset..offset + 4].copy_from_slice(&TSG_PACKET_TYPE_MSGREQUEST.to_le_bytes());
            offset += 4;
            output[offset..offset + 4].copy_from_slice(&TSG_PACKET_TYPE_MSGREQUEST.to_le_bytes());
            offset += 4;
            output[offset..offset + 4].copy_from_slice(&NDR_REFERENT_ID.to_le_bytes());
            offset += 4;
            output[offset..offset + 4].copy_from_slice(&1u32.to_le_bytes());
            output
        }
    }

    /// NDR32 context-handle request shared by `TsProxyCloseChannel` and `TsProxyCloseTunnel`.
    ///
    /// [MS-TSGU] 3.2.6.3.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct TsProxyCloseContextRequest {
        context: NonNullRpcContextHandle,
    }

    impl TsProxyCloseContextRequest {
        pub(crate) const fn new(context: NonNullRpcContextHandle) -> Self {
            Self { context }
        }

        pub(crate) const fn encode(self) -> [u8; RpcContextHandle::SIZE] {
            self.context.as_bytes()
        }
    }

    /// Decodes the updated context handle and HRESULT returned by a close operation.
    pub(crate) fn decode_tsgu_close_context_response(source: &[u8]) -> Result<RpcContextHandle, RpcStubError> {
        const RESPONSE_SIZE: usize = RpcContextHandle::SIZE /* context */ + 4 /* HRESULT */;
        if source.len() != RESPONSE_SIZE {
            return Err(RpcStubError::ResponseLength {
                actual: source.len(),
                expected: RESPONSE_SIZE,
            });
        }
        validate_hresult(read_trailing_hresult(source, RESPONSE_SIZE)?)?;
        RpcContextHandle::from_bytes(&source[..RpcContextHandle::SIZE])
    }

    /// Raw `TsProxySetupReceivePipe` request stub.
    ///
    /// [MS-TSGU] 2.2.9.4.1 and 3.6.5.3.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct TsProxySetupReceivePipeRequest {
        channel_context: NonNullRpcContextHandle,
    }

    impl TsProxySetupReceivePipeRequest {
        pub(crate) const SIZE: usize = RpcContextHandle::SIZE;

        pub(crate) const fn new(channel_context: NonNullRpcContextHandle) -> Self {
            Self { channel_context }
        }

        pub(crate) const fn encode(self) -> [u8; Self::SIZE] {
            self.channel_context.as_bytes()
        }
    }

    /// Decodes the terminal return value of a `TsProxySetupReceivePipe` response.
    ///
    /// The caller must provide Stub Data from the final response fragment.
    ///
    /// [MS-TSGU] 2.2.9.4.3 and 3.6.5.5.
    pub(crate) fn decode_tsgu_setup_receive_pipe_final_response(source: &[u8]) -> Result<u32, RpcStubError> {
        const RESPONSE_SIZE: usize = 4 /* return value */;
        if source.len() != RESPONSE_SIZE {
            return Err(RpcStubError::ResponseLength {
                actual: source.len(),
                expected: RESPONSE_SIZE,
            });
        }
        read_u32(source, 0)
    }

    /// Raw `TsProxySendToServer` request stub builder.
    ///
    /// [MS-TSGU] 2.2.9.3 and 3.6.5.1.
    #[derive(Debug)]
    pub(crate) struct TsProxySendToServerRequest<'a> {
        channel_context: NonNullRpcContextHandle,
        buffers: [&'a [u8]; 3],
        buffer_count: usize,
    }

    impl<'a> TsProxySendToServerRequest<'a> {
        const PREFIX_SIZE: usize =
            RpcContextHandle::SIZE /* channelContext */ + 4 /* totalDataBytes */ + 4 /* numBuffers */;

        /// Starts a request with its required non-empty first buffer.
        pub(crate) fn new(
            channel_context: NonNullRpcContextHandle,
            first_buffer: &'a [u8],
        ) -> Result<Self, RpcStubError> {
            if first_buffer.is_empty() {
                return Err(RpcStubError::EmptyFirstBuffer);
            }
            Ok(Self {
                channel_context,
                buffers: [first_buffer, &[], &[]],
                buffer_count: 1,
            })
        }

        /// Adds an optional second or third buffer.
        pub(crate) fn push_buffer(&mut self, buffer: &'a [u8]) -> Result<(), RpcStubError> {
            if self.buffer_count == self.buffers.len() {
                return Err(RpcStubError::BufferCount {
                    actual: self.buffer_count + 1,
                });
            }
            self.buffers[self.buffer_count] = buffer;
            self.buffer_count += 1;
            Ok(())
        }

        /// Encodes the contiguous raw RPC stub.
        pub(crate) fn encode(&self) -> Result<Vec<u8>, RpcStubError> {
            let encoded_len = self.encoded_len()?;
            let total_data_bytes =
                u32::try_from(encoded_len - Self::PREFIX_SIZE).map_err(|_| RpcStubError::LengthOverflow)?;
            let buffer_count = u32::try_from(self.buffer_count).map_err(|_| RpcStubError::LengthOverflow)?;
            let mut output = Vec::with_capacity(encoded_len);
            output.extend_from_slice(&self.channel_context.as_bytes());
            output.extend_from_slice(&total_data_bytes.to_be_bytes());
            output.extend_from_slice(&buffer_count.to_be_bytes());
            for buffer in self.buffers() {
                let length = u32::try_from(buffer.len()).map_err(|_| RpcStubError::LengthOverflow)?;
                output.extend_from_slice(&length.to_be_bytes());
            }
            for buffer in self.buffers() {
                output.extend_from_slice(buffer);
            }
            debug_assert_eq!(output.len(), encoded_len);
            Ok(output)
        }

        fn encoded_len(&self) -> Result<usize, RpcStubError> {
            let payload_len = self.buffers().iter().try_fold(0usize, |total, buffer| {
                total.checked_add(buffer.len()).ok_or(RpcStubError::LengthOverflow)
            })?;
            let length_fields_len = 4usize
                .checked_mul(self.buffer_count)
                .ok_or(RpcStubError::LengthOverflow)?;
            let total_data_bytes = payload_len
                .checked_add(length_fields_len)
                .ok_or(RpcStubError::LengthOverflow)?;
            let encoded_len = Self::PREFIX_SIZE
                .checked_add(total_data_bytes)
                .ok_or(RpcStubError::LengthOverflow)?;
            if encoded_len > MAX_RAW_RPC_STUB_SIZE {
                return Err(RpcStubError::RawStubTooLarge { actual: encoded_len });
            }
            Ok(encoded_len)
        }

        fn buffers(&self) -> &[&'a [u8]] {
            &self.buffers[..self.buffer_count]
        }
    }

    /// Decodes the final `TsProxySendToServer` return value.
    pub(crate) fn decode_tsgu_send_to_server_response(source: &[u8]) -> Result<u32, RpcStubError> {
        const RESPONSE_SIZE: usize = 4 /* return value */;
        if source.len() != RESPONSE_SIZE {
            return Err(RpcStubError::ResponseLength {
                actual: source.len(),
                expected: RESPONSE_SIZE,
            });
        }
        read_u32(source, 0)
    }

    /// Decoded non-messaging `TsProxyCreateTunnel` response values.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct TsProxyCreateTunnelResponse {
        pub(crate) tunnel_context: NonNullRpcContextHandle,
        pub(crate) tunnel_id: u32,
        pub(crate) nonce: Uuid,
        pub(crate) capabilities: u32,
    }

    /// Decodes a non-messaging `TsProxyCreateTunnel` response stub.
    ///
    /// Consent-signing `TSG_PACKET_CAPS_RESPONSE` stubs are deliberately excluded.
    pub(crate) fn decode_tsgu_create_tunnel_response(
        source: &[u8],
    ) -> Result<TsProxyCreateTunnelResponse, RpcStubError> {
        const MINIMUM_RESPONSE_SIZE: usize =
            4 /* TSGPacketResponse */ + RpcContextHandle::SIZE /* tunnelContext */ + 4 /* tunnelId */ + 4 /* HRESULT */;
        validate_hresult(read_trailing_hresult(source, MINIMUM_RESPONSE_SIZE)?)?;

        const FIXED_SIZE: usize = 4 /* TSGPacketResponse */
            + 4 /* packetId */
            + 4 /* union switch */
            + 4 /* packetQuarEncResponse */
            + 4 /* flags */
            + 4 /* certChainLen */
            + 4 /* certChainData */
            + 16 /* nonce */
            + 4; /* versionCaps */
        let fixed = source.get(..FIXED_SIZE).ok_or(RpcStubError::ResponseLength {
            actual: source.len(),
            expected: FIXED_SIZE,
        })?;
        require_ndr_pointer(read_u32(fixed, 0)?)?;
        validate_packet(fixed, TSG_PACKET_TYPE_QUARENC_RESPONSE)?;
        require_ndr_pointer(read_u32(fixed, 12)?)?;
        let flags = read_u32(fixed, 16)?;
        if flags != 0 {
            return Err(RpcStubError::InvalidQuarencFlags { actual: flags });
        }
        let certificate_chain_len = usize::try_from(read_u32(fixed, 20)?).map_err(|_| RpcStubError::LengthOverflow)?;
        if certificate_chain_len > MAX_CERT_CHAIN_CHARS {
            return Err(RpcStubError::CertificateChainTooLarge {
                actual: certificate_chain_len,
            });
        }
        let certificate_chain_pointer = read_u32(fixed, 24)?;
        if certificate_chain_len != 0 {
            require_ndr_pointer(certificate_chain_pointer)?;
        }
        let nonce = Uuid::from_bytes_le(fixed[28..44].try_into().map_err(|_| RpcStubError::LengthOverflow)?);
        require_ndr_pointer(read_u32(fixed, 44)?)?;

        let mut offset = FIXED_SIZE;
        if certificate_chain_pointer != 0 {
            let (certificate, next_offset) = decode_ndr_utf16_string(source, offset, certificate_chain_len)?;
            if !certificate.is_empty() && certificate.contains('\0') {
                return Err(RpcStubError::InvalidUtf16);
            }
            offset = next_offset;
        }

        const VERSION_CAPS_SIZE: usize = 2 /* componentId */
            + 2 /* packetId */
            + 4 /* TSGCaps */
            + 4 /* numCapabilities */
            + 2 /* majorVersion */
            + 2 /* minorVersion */
            + 2 /* quarantineCapabilities */
            + 2; /* NDR alignment */
        let version_caps_end = offset
            .checked_add(VERSION_CAPS_SIZE)
            .ok_or(RpcStubError::LengthOverflow)?;
        let version_caps = source
            .get(offset..version_caps_end)
            .ok_or(RpcStubError::ResponseLength {
                actual: source.len(),
                expected: version_caps_end,
            })?;
        if read_u16(version_caps, 0)? != TSG_COMPONENT_ID {
            return Err(RpcStubError::UnexpectedComponentId {
                expected: TSG_COMPONENT_ID,
                actual: read_u16(version_caps, 0)?,
            });
        }
        let capabilities_pointer = read_u32(version_caps, 4)?;
        let capability_count = read_u32(version_caps, 8)?;
        if usize::try_from(capability_count).map_err(|_| RpcStubError::LengthOverflow)? > MAX_CAPABILITIES {
            return Err(RpcStubError::CapabilityCountTooLarge {
                actual: capability_count,
            });
        }
        let major_version = read_u16(version_caps, 12)?;
        let minor_version = read_u16(version_caps, 14)?;
        if major_version != TSG_PROTOCOL_VERSION || minor_version != TSG_PROTOCOL_VERSION {
            return Err(RpcStubError::UnexpectedProtocolVersion {
                major: major_version,
                minor: minor_version,
            });
        }
        let quarantine_capabilities = read_u16(version_caps, 16)?;
        if quarantine_capabilities > 1 {
            return Err(RpcStubError::InvalidQuarantineCapabilities {
                actual: quarantine_capabilities,
            });
        }
        if quarantine_capabilities == 1 && certificate_chain_len == 0 {
            return Err(RpcStubError::MissingCertificateChain);
        }
        if capabilities_pointer == 0 && capability_count != 0 {
            return Err(RpcStubError::RequiredNdrPointerIsNull);
        }
        offset = version_caps_end;
        if capabilities_pointer != 0 {
            let capability_array_count = read_u32(source, offset)?;
            if capability_array_count != capability_count {
                return Err(RpcStubError::InvalidNdrArrayLength {
                    actual: capability_array_count,
                    expected: capability_count,
                });
            }
            offset = offset.checked_add(4).ok_or(RpcStubError::LengthOverflow)?;
        }

        let mut capabilities = 0;
        for _ in 0..capability_count {
            let capability_type = read_u32(source, offset)?;
            let capability_switch = read_u32(source, offset.checked_add(4).ok_or(RpcStubError::LengthOverflow)?)?;
            if capability_type != TSG_CAPABILITY_TYPE_NAP {
                return Err(RpcStubError::UnexpectedCapabilityType {
                    expected: TSG_CAPABILITY_TYPE_NAP,
                    actual: capability_type,
                });
            }
            if capability_switch != capability_type {
                return Err(RpcStubError::UnexpectedCapabilityType {
                    expected: capability_type,
                    actual: capability_switch,
                });
            }
            let capability = read_u32(source, offset.checked_add(8).ok_or(RpcStubError::LengthOverflow)?)?;
            validate_capabilities(capability)?;
            capabilities |= capability;
            offset = offset.checked_add(12).ok_or(RpcStubError::LengthOverflow)?;
        }

        const TRAILER_SIZE: usize = RpcContextHandle::SIZE /* tunnelContext */ + 4 /* tunnelId */ + 4 /* HRESULT */;
        let response_size = offset.checked_add(TRAILER_SIZE).ok_or(RpcStubError::LengthOverflow)?;
        if source.len() != response_size {
            return Err(RpcStubError::ResponseLength {
                actual: source.len(),
                expected: response_size,
            });
        }
        let tunnel_context =
            RpcContextHandle::from_bytes(&source[offset..offset + RpcContextHandle::SIZE])?.require_non_null()?;
        let tunnel_id = read_u32(source, offset + RpcContextHandle::SIZE)?;

        Ok(TsProxyCreateTunnelResponse {
            tunnel_context,
            tunnel_id,
            nonce,
            capabilities,
        })
    }

    /// Device-redirection policy returned by `TsProxyAuthorizeTunnel`.
    ///
    /// [MS-TSGU] 2.2.9.2.1.5.2.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct TsProxyRedirectionFlags {
        pub(crate) enable_all: bool,
        pub(crate) disable_all: bool,
        pub(crate) drive_disabled: bool,
        pub(crate) printer_disabled: bool,
        pub(crate) port_disabled: bool,
        pub(crate) clipboard_disabled: bool,
        pub(crate) pnp_disabled: bool,
    }

    /// Decoded `TsProxyAuthorizeTunnel` response values.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct TsProxyAuthorizeTunnelResponse {
        pub(crate) response_data: Vec<u8>,
        pub(crate) redirection_flags: TsProxyRedirectionFlags,
    }

    /// Decodes a `TsProxyAuthorizeTunnel` response stub.
    ///
    /// [MS-TSGU] 2.2.9.2.1.5 and 3.2.6.1.2.
    pub(crate) fn decode_tsgu_authorize_tunnel_response(
        source: &[u8],
    ) -> Result<TsProxyAuthorizeTunnelResponse, RpcStubError> {
        const FIXED_SIZE: usize = 4 /* TSGPacketResponse */
            + 4 /* packetId */
            + 4 /* union switch */
            + 4 /* packetResponse */
            + 4 /* flags */
            + 4 /* reserved */
            + 4 /* responseData */
            + 4 /* responseDataLen */
            + 8 * 4; /* redirectionFlags */
        const MINIMUM_RESPONSE_SIZE: usize = 4 /* TSGPacketResponse */ + 4 /* HRESULT */;
        let hresult = read_trailing_hresult(source, MINIMUM_RESPONSE_SIZE)?;
        if hresult != 0 {
            let has_quarantine_response = hresult == E_PROXY_QUARANTINE_ACCESSDENIED
                && source.len() >= FIXED_SIZE + 4
                && read_u32(source, 0)? != 0;
            if !has_quarantine_response {
                return Err(RpcStubError::RpcStatus { value: hresult });
            }
        }

        let fixed = source.get(..FIXED_SIZE).ok_or(RpcStubError::ResponseLength {
            actual: source.len(),
            expected: FIXED_SIZE,
        })?;
        require_ndr_pointer(read_u32(fixed, 0)?)?;
        validate_packet(fixed, TSG_PACKET_TYPE_RESPONSE)?;
        require_ndr_pointer(read_u32(fixed, 12)?)?;
        if read_u32(fixed, 16)? != TSG_PACKET_TYPE_QUARREQUEST {
            return Err(RpcStubError::UnexpectedPacketId {
                expected: TSG_PACKET_TYPE_QUARREQUEST,
                actual: read_u32(fixed, 16)?,
            });
        }
        let response_data_pointer = read_u32(fixed, 24)?;
        let response_data_len = usize::try_from(read_u32(fixed, 28)?).map_err(|_| RpcStubError::LengthOverflow)?;
        if response_data_len > MAX_RESPONSE_DATA_SIZE {
            return Err(RpcStubError::ResponseDataTooLarge {
                actual: response_data_len,
            });
        }
        if response_data_len != 0 {
            require_ndr_pointer(response_data_pointer)?;
        }

        let redirection_flags = TsProxyRedirectionFlags {
            enable_all: decode_ndr_boolean(fixed, 32)?,
            disable_all: decode_ndr_boolean(fixed, 36)?,
            drive_disabled: decode_ndr_boolean(fixed, 40)?,
            printer_disabled: decode_ndr_boolean(fixed, 44)?,
            port_disabled: decode_ndr_boolean(fixed, 48)?,
            clipboard_disabled: decode_ndr_boolean(fixed, 56)?,
            pnp_disabled: decode_ndr_boolean(fixed, 60)?,
        };
        if decode_ndr_boolean(fixed, 52)? {
            return Err(RpcStubError::NonZeroReservedRedirectionFlag);
        }
        if redirection_flags.enable_all && redirection_flags.disable_all {
            return Err(RpcStubError::ConflictingRedirectionFlags);
        }

        let mut offset = FIXED_SIZE;
        let response_data = if response_data_pointer == 0 {
            if response_data_len != 0 {
                return Err(RpcStubError::RequiredNdrPointerIsNull);
            }
            Vec::new()
        } else {
            let array_count = usize::try_from(read_u32(source, offset)?).map_err(|_| RpcStubError::LengthOverflow)?;
            if array_count != response_data_len {
                return Err(RpcStubError::InvalidNdrArrayLength {
                    actual: u32::try_from(array_count).map_err(|_| RpcStubError::LengthOverflow)?,
                    expected: u32::try_from(response_data_len).map_err(|_| RpcStubError::LengthOverflow)?,
                });
            }
            offset = offset.checked_add(4).ok_or(RpcStubError::LengthOverflow)?;
            let data_end = offset
                .checked_add(response_data_len)
                .ok_or(RpcStubError::LengthOverflow)?;
            let response_data = source.get(offset..data_end).ok_or(RpcStubError::ResponseLength {
                actual: source.len(),
                expected: data_end,
            })?;
            offset = padded_to_ndr(data_end, 4)?;
            response_data.to_vec()
        };
        let response_size = offset.checked_add(4).ok_or(RpcStubError::LengthOverflow)?;
        if source.len() != response_size {
            return Err(RpcStubError::ResponseLength {
                actual: source.len(),
                expected: response_size,
            });
        }
        if hresult == E_PROXY_QUARANTINE_ACCESSDENIED {
            return Err(RpcStubError::QuarantineAccessDenied { response_data });
        }

        Ok(TsProxyAuthorizeTunnelResponse {
            response_data,
            redirection_flags,
        })
    }

    /// Server message returned by `TsProxyMakeTunnelCall`.
    ///
    /// [MS-TSGU] 2.2.9.2.1.9 and 3.2.6.1.3.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) enum TsProxyTunnelMessage {
        None,
        Consent {
            display_mandatory: bool,
            consent_mandatory: bool,
            text: String,
        },
        Service {
            display_mandatory: bool,
            text: String,
        },
        Reauthenticate {
            tunnel_context: u64,
        },
    }

    /// Decodes one `TsProxyMakeTunnelCall` response stub.
    pub(crate) fn decode_tsgu_make_tunnel_call_response(source: &[u8]) -> Result<TsProxyTunnelMessage, RpcStubError> {
        const MINIMUM_RESPONSE_SIZE: usize = 4 /* TSGPacketResponse */ + 4 /* HRESULT */;
        validate_hresult(read_trailing_hresult(source, MINIMUM_RESPONSE_SIZE)?)?;

        const FIXED_SIZE: usize = 4 /* TSGPacketResponse */
            + 4 /* packetId */
            + 4 /* union switch */
            + 4 /* packetMsgResponse */
            + 4 /* msgId */
            + 4 /* msgType */
            + 4 /* isMsgPresent */
            + 4 /* messagePacket union switch */
            + 4; /* messagePacket pointer */
        let fixed = source.get(..FIXED_SIZE).ok_or(RpcStubError::ResponseLength {
            actual: source.len(),
            expected: FIXED_SIZE,
        })?;
        require_ndr_pointer(read_u32(fixed, 0)?)?;
        validate_packet(fixed, TSG_PACKET_TYPE_MESSAGE)?;
        require_ndr_pointer(read_u32(fixed, 12)?)?;

        let message_type = read_u32(fixed, 20)?;
        validate_message_type(message_type)?;
        let message_present = decode_ndr_boolean(fixed, 24)?;
        let message_switch = read_u32(fixed, 28)?;
        if message_switch != message_type {
            return Err(RpcStubError::UnexpectedMessageSwitch {
                expected: message_type,
                actual: message_switch,
            });
        }
        if !message_present {
            const RESPONSE_SIZE: usize = FIXED_SIZE + 4 /* HRESULT */;
            if source.len() != RESPONSE_SIZE {
                return Err(RpcStubError::ResponseLength {
                    actual: source.len(),
                    expected: RESPONSE_SIZE,
                });
            }
            return Ok(TsProxyTunnelMessage::None);
        }
        require_ndr_pointer(read_u32(fixed, 32)?)?;

        match message_type {
            TSG_ASYNC_MESSAGE_REAUTH => {
                let reauthentication_start = padded_to_ndr(FIXED_SIZE, 8)?;
                let response_size = reauthentication_start
                    .checked_add(8 /* tunnelContext */ + 4 /* HRESULT */)
                    .ok_or(RpcStubError::LengthOverflow)?;
                if source.len() != response_size {
                    return Err(RpcStubError::ResponseLength {
                        actual: source.len(),
                        expected: response_size,
                    });
                }
                let tunnel_context = u64::from_le_bytes(
                    source[reauthentication_start..reauthentication_start + 8]
                        .try_into()
                        .map_err(|_| RpcStubError::LengthOverflow)?,
                );
                Ok(TsProxyTunnelMessage::Reauthenticate { tunnel_context })
            }
            TSG_ASYNC_MESSAGE_CONSENT | TSG_ASYNC_MESSAGE_SERVICE => {
                const STRING_FIXED_SIZE: usize = 4 /* isDisplayMandatory */
                    + 4 /* isConsentMandatory */
                    + 4 /* msgBytes */
                    + 4; /* msgBuffer */
                let string_start = FIXED_SIZE;
                let string_end = string_start
                    .checked_add(STRING_FIXED_SIZE)
                    .ok_or(RpcStubError::LengthOverflow)?;
                let string_fixed = source
                    .get(string_start..string_end)
                    .ok_or(RpcStubError::ResponseLength {
                        actual: source.len(),
                        expected: string_end,
                    })?;
                let display_mandatory = decode_ndr_boolean(string_fixed, 0)?;
                let consent_mandatory = decode_ndr_boolean(string_fixed, 4)?;
                let message_chars =
                    usize::try_from(read_u32(string_fixed, 8)?).map_err(|_| RpcStubError::LengthOverflow)?;
                if message_chars > MAX_MESSAGE_CHARS {
                    return Err(RpcStubError::MessageTooLong { actual: message_chars });
                }
                let message_pointer = read_u32(string_fixed, 12)?;
                if message_chars != 0 {
                    require_ndr_pointer(message_pointer)?;
                }

                let (message_bytes, response_size) = if message_pointer == 0 {
                    (&[][..], string_end.checked_add(4).ok_or(RpcStubError::LengthOverflow)?)
                } else {
                    let array_count =
                        usize::try_from(read_u32(source, string_end)?).map_err(|_| RpcStubError::LengthOverflow)?;
                    if array_count != message_chars {
                        return Err(RpcStubError::InvalidNdrArrayLength {
                            actual: u32::try_from(array_count).map_err(|_| RpcStubError::LengthOverflow)?,
                            expected: u32::try_from(message_chars).map_err(|_| RpcStubError::LengthOverflow)?,
                        });
                    }
                    let message_start = string_end.checked_add(4).ok_or(RpcStubError::LengthOverflow)?;
                    let message_len = message_chars.checked_mul(2).ok_or(RpcStubError::LengthOverflow)?;
                    let message_end = message_start
                        .checked_add(message_len)
                        .ok_or(RpcStubError::LengthOverflow)?;
                    let response_size = padded_to_ndr(message_end, 4)?
                        .checked_add(4 /* HRESULT */)
                        .ok_or(RpcStubError::LengthOverflow)?;
                    let message_bytes = source
                        .get(message_start..message_end)
                        .ok_or(RpcStubError::ResponseLength {
                            actual: source.len(),
                            expected: message_end,
                        })?;
                    (message_bytes, response_size)
                };
                if source.len() != response_size {
                    return Err(RpcStubError::ResponseLength {
                        actual: source.len(),
                        expected: response_size,
                    });
                }
                if message_chars != 0 && message_bytes[message_bytes.len() - 2..] != [0, 0] {
                    return Err(RpcStubError::UnterminatedNdrString);
                }
                let message_units: Vec<u16> = message_bytes
                    .chunks_exact(2)
                    .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
                    .collect();
                let text = String::from_utf16(&message_units[..message_units.len().saturating_sub(1)])
                    .map_err(|_| RpcStubError::InvalidUtf16)?;

                Ok(if message_type == TSG_ASYNC_MESSAGE_CONSENT {
                    TsProxyTunnelMessage::Consent {
                        display_mandatory,
                        consent_mandatory,
                        text,
                    }
                } else {
                    TsProxyTunnelMessage::Service {
                        display_mandatory,
                        text,
                    }
                })
            }
            actual => Err(RpcStubError::InvalidMessageType { actual }),
        }
    }

    /// Validates the null response and HRESULT from a cancelled message request.
    ///
    /// [MS-TSGU] 3.2.6.1.3.
    pub(crate) fn decode_tsgu_cancel_tunnel_call_response(source: &[u8]) -> Result<(), RpcStubError> {
        const RESPONSE_SIZE: usize = 4 /* null TSGPacketResponse */ + 4 /* HRESULT */;
        if source.len() != RESPONSE_SIZE {
            return Err(RpcStubError::ResponseLength {
                actual: source.len(),
                expected: RESPONSE_SIZE,
            });
        }
        let response_pointer = read_u32(source, 0)?;
        if response_pointer != 0 {
            return Err(RpcStubError::UnexpectedNdrPointer {
                actual: response_pointer,
            });
        }
        validate_hresult(read_trailing_hresult(source, RESPONSE_SIZE)?)
    }

    /// Decoded `TsProxyCreateChannel` response values.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) struct TsProxyCreateChannelResponse {
        pub(crate) channel_context: NonNullRpcContextHandle,
        pub(crate) channel_id: u32,
    }

    /// Decodes a `TsProxyCreateChannel` response stub.
    ///
    /// [MS-TSGU] 3.2.6.1.4.
    pub(crate) fn decode_tsgu_create_channel_response(
        source: &[u8],
    ) -> Result<TsProxyCreateChannelResponse, RpcStubError> {
        const RESPONSE_SIZE: usize = RpcContextHandle::SIZE /* channelContext */ + 4 /* channelId */ + 4 /* HRESULT */;
        if source.len() != RESPONSE_SIZE {
            return Err(RpcStubError::ResponseLength {
                actual: source.len(),
                expected: RESPONSE_SIZE,
            });
        }
        validate_hresult(read_trailing_hresult(source, RESPONSE_SIZE)?)?;
        let channel_context = RpcContextHandle::from_bytes(&source[..RpcContextHandle::SIZE])?.require_non_null()?;
        let channel_id = read_u32(source, RpcContextHandle::SIZE)?;
        Ok(TsProxyCreateChannelResponse {
            channel_context,
            channel_id,
        })
    }

    fn encode_ndr_pointer(output: &mut Vec<u8>, index: u32) {
        output.extend_from_slice(&(NDR_REFERENT_ID + index * 4).to_le_bytes());
    }

    fn encode_ndr_resource_name(value: &str) -> Result<Vec<u16>, RpcStubError> {
        if value.is_empty() {
            return Err(RpcStubError::EmptyResourceName);
        }
        if value.contains('\0') {
            return Err(RpcStubError::EmbeddedNulInResourceName);
        }
        let mut encoded: Vec<_> = value.encode_utf16().collect();
        encoded.push(0);
        if encoded.len() > MAX_RESOURCE_NAME_CHARS {
            return Err(RpcStubError::ResourceNameTooLong {
                actual: encoded.len(),
                maximum: MAX_RESOURCE_NAME_CHARS,
            });
        }
        Ok(encoded)
    }

    fn encode_ndr_machine_name(value: &str) -> Result<Vec<u16>, RpcStubError> {
        if value.contains('\0') {
            return Err(RpcStubError::EmbeddedNulInMachineName);
        }
        let mut encoded: Vec<_> = value.encode_utf16().collect();
        encoded.push(0);
        if encoded.len() > MAX_MACHINE_NAME_CHARS {
            return Err(RpcStubError::MachineNameTooLong {
                actual: encoded.len(),
                maximum: MAX_MACHINE_NAME_CHARS,
            });
        }
        Ok(encoded)
    }

    fn encode_ndr_string_referent(output: &mut Vec<u8>, value: &[u16]) -> Result<(), RpcStubError> {
        let length = u32::try_from(value.len()).map_err(|_| RpcStubError::LengthOverflow)?;
        output.extend_from_slice(&length.to_le_bytes()); // max count
        output.extend_from_slice(&0u32.to_le_bytes()); // offset
        output.extend_from_slice(&length.to_le_bytes()); // actual count
        for character in value {
            output.extend_from_slice(&character.to_le_bytes());
        }
        pad_ndr_4(output);
        Ok(())
    }

    fn decode_ndr_utf16_string(
        source: &[u8],
        offset: usize,
        expected_count: usize,
    ) -> Result<(String, usize), RpcStubError> {
        let max_count = usize::try_from(read_u32(source, offset)?).map_err(|_| RpcStubError::LengthOverflow)?;
        let first_index = read_u32(source, offset.checked_add(4).ok_or(RpcStubError::LengthOverflow)?)?;
        let actual_count = usize::try_from(read_u32(
            source,
            offset.checked_add(8).ok_or(RpcStubError::LengthOverflow)?,
        )?)
        .map_err(|_| RpcStubError::LengthOverflow)?;
        if max_count != expected_count || first_index != 0 || actual_count != expected_count {
            return Err(RpcStubError::InvalidNdrArrayLength {
                actual: u32::try_from(actual_count).map_err(|_| RpcStubError::LengthOverflow)?,
                expected: u32::try_from(expected_count).map_err(|_| RpcStubError::LengthOverflow)?,
            });
        }
        let characters_start = offset.checked_add(12).ok_or(RpcStubError::LengthOverflow)?;
        let byte_len = expected_count.checked_mul(2).ok_or(RpcStubError::LengthOverflow)?;
        let characters_end = characters_start
            .checked_add(byte_len)
            .ok_or(RpcStubError::LengthOverflow)?;
        let characters = source
            .get(characters_start..characters_end)
            .ok_or(RpcStubError::ResponseLength {
                actual: source.len(),
                expected: characters_end,
            })?;
        let utf16: Vec<u16> = characters
            .chunks_exact(2)
            .map(|character| u16::from_le_bytes([character[0], character[1]]))
            .collect();
        if utf16.last() != Some(&0) {
            return Err(RpcStubError::UnterminatedNdrString);
        }
        let text =
            String::from_utf16(&utf16[..utf16.len().saturating_sub(1)]).map_err(|_| RpcStubError::InvalidUtf16)?;
        Ok((text, padded_to_ndr(characters_end, 4)?))
    }

    fn pad_ndr_4(output: &mut Vec<u8>) {
        let padding = (4 - output.len() % 4) % 4;
        output.resize(output.len() + padding, 0);
    }

    fn padded_to_ndr(length: usize, alignment: usize) -> Result<usize, RpcStubError> {
        debug_assert!(alignment.is_power_of_two());
        length
            .checked_add(alignment - 1)
            .map(|value| value & !(alignment - 1))
            .ok_or(RpcStubError::LengthOverflow)
    }

    fn require_ndr_pointer(pointer: u32) -> Result<(), RpcStubError> {
        if pointer == 0 {
            return Err(RpcStubError::RequiredNdrPointerIsNull);
        }
        Ok(())
    }

    fn validate_packet(source: &[u8], expected_packet_id: u32) -> Result<(), RpcStubError> {
        let packet_id = read_u32(source, 4)?;
        if packet_id != expected_packet_id {
            return Err(RpcStubError::UnexpectedPacketId {
                expected: expected_packet_id,
                actual: packet_id,
            });
        }
        let packet_switch = read_u32(source, 8)?;
        if packet_switch != packet_id {
            return Err(RpcStubError::UnexpectedPacketSwitch {
                expected: packet_id,
                actual: packet_switch,
            });
        }
        Ok(())
    }

    fn validate_message_type(value: u32) -> Result<(), RpcStubError> {
        match value {
            TSG_ASYNC_MESSAGE_CONSENT | TSG_ASYNC_MESSAGE_SERVICE | TSG_ASYNC_MESSAGE_REAUTH => Ok(()),
            actual => Err(RpcStubError::InvalidMessageType { actual }),
        }
    }

    fn decode_ndr_boolean(source: &[u8], offset: usize) -> Result<bool, RpcStubError> {
        match read_u32(source, offset)? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(RpcStubError::InvalidNdrBoolean { value }),
        }
    }

    fn validate_hresult(value: u32) -> Result<(), RpcStubError> {
        if value != 0 {
            return Err(RpcStubError::RpcStatus { value });
        }
        Ok(())
    }

    fn read_trailing_hresult(source: &[u8], minimum_size: usize) -> Result<u32, RpcStubError> {
        if source.len() < minimum_size {
            return Err(RpcStubError::ResponseLength {
                actual: source.len(),
                expected: minimum_size,
            });
        }
        read_u32(source, source.len() - 4)
    }

    fn validate_capabilities(capabilities: u32) -> Result<(), RpcStubError> {
        if capabilities & !SUPPORTED_CAPABILITIES != 0 {
            return Err(RpcStubError::UnsupportedCapabilities { actual: capabilities });
        }
        Ok(())
    }

    fn read_u16(source: &[u8], offset: usize) -> Result<u16, RpcStubError> {
        let end = offset.checked_add(2).ok_or(RpcStubError::LengthOverflow)?;
        let bytes = source.get(offset..end).ok_or(RpcStubError::ResponseLength {
            actual: source.len(),
            expected: end,
        })?;
        Ok(u16::from_le_bytes(
            bytes.try_into().map_err(|_| RpcStubError::LengthOverflow)?,
        ))
    }

    fn read_u32(source: &[u8], offset: usize) -> Result<u32, RpcStubError> {
        let end = offset.checked_add(4).ok_or(RpcStubError::LengthOverflow)?;
        let bytes = source.get(offset..end).ok_or(RpcStubError::ResponseLength {
            actual: source.len(),
            expected: end,
        })?;
        Ok(u32::from_le_bytes(
            bytes.try_into().map_err(|_| RpcStubError::LengthOverflow)?,
        ))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const CONTEXT: [u8; RpcContextHandle::SIZE] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x10, 0x11, 0x12,
            0x13, 0x14,
        ];

        fn non_null_context() -> NonNullRpcContextHandle {
            RpcContextHandle::from_bytes(&CONTEXT)
                .expect("valid handle")
                .require_non_null()
                .expect("non-null")
        }

        #[test]
        fn requests_encode_bounded_ndr32_control_vectors() {
            let create_tunnel = TsProxyCreateTunnelRequest::new(3)
                .encode()
                .expect("supported capabilities");
            assert_eq!(create_tunnel.len(), 48);
            assert_eq!(
                create_tunnel,
                [
                    TSG_PACKET_TYPE_VERSIONCAPS.to_le_bytes().as_slice(),
                    &TSG_PACKET_TYPE_VERSIONCAPS.to_le_bytes(),
                    &NDR_REFERENT_ID.to_le_bytes(),
                    &TSG_COMPONENT_ID.to_le_bytes(),
                    &TSG_PACKET_TYPE_VERSIONCAPS_ID.to_le_bytes(),
                    &(NDR_REFERENT_ID + 4).to_le_bytes(),
                    &1u32.to_le_bytes(),
                    &1u16.to_le_bytes(),
                    &1u16.to_le_bytes(),
                    &0u16.to_le_bytes(),
                    &0u16.to_le_bytes(),
                    &1u32.to_le_bytes(),
                    &TSG_CAPABILITY_TYPE_NAP.to_le_bytes(),
                    &TSG_CAPABILITY_TYPE_NAP.to_le_bytes(),
                    &3u32.to_le_bytes(),
                ]
                .concat()
            );

            let context = RpcContextHandle::from_bytes(&CONTEXT)
                .expect("valid handle")
                .require_non_null()
                .expect("non-null");
            let channel = TsProxyCreateChannelRequest::new(context, "rdp.example", 3389)
                .encode()
                .expect("valid endpoint");
            assert_eq!(&channel[..20], &CONTEXT);
            assert_eq!(
                u32::from_le_bytes(channel[36..40].try_into().expect("port")),
                (3389u32 << 16) | 3
            );

            let authorize = TsProxyAuthorizeTunnelRequest::new(context, "host", &[1, 2])
                .encode()
                .expect("valid authorization");
            assert_eq!(&authorize[..20], &CONTEXT);
            assert_eq!(
                u32::from_le_bytes(authorize[20..24].try_into().expect("packet id")),
                TSG_PACKET_TYPE_QUARREQUEST
            );

            let authorize = TsProxyAuthorizeTunnelRequest::new(context, "", &[])
                .encode()
                .expect("empty machine name");
            assert_eq!(
                u32::from_le_bytes(authorize[40..44].try_into().expect("name length")),
                1
            );
        }

        #[test]
        fn requests_reject_invalid_strings_and_bounded_data() {
            let context = RpcContextHandle::from_bytes(&CONTEXT)
                .expect("valid handle")
                .require_non_null()
                .expect("non-null");
            assert_eq!(
                TsProxyCreateChannelRequest::new(context, "bad\0name", 3389).encode(),
                Err(RpcStubError::EmbeddedNulInResourceName)
            );
            assert_eq!(
                TsProxyAuthorizeTunnelRequest::new(context, "bad\0name", &[]).encode(),
                Err(RpcStubError::EmbeddedNulInMachineName)
            );
            assert_eq!(
                TsProxyCreateTunnelRequest::new(0x0000_0004).encode(),
                Err(RpcStubError::UnsupportedCapabilities { actual: 0x0000_0004 })
            );
            assert_eq!(
                TsProxyAuthorizeTunnelRequest::new(context, "host", &[0; MAX_STATEMENT_OF_HEALTH_SIZE + 1]).encode(),
                Err(RpcStubError::StatementOfHealthTooLarge {
                    actual: MAX_STATEMENT_OF_HEALTH_SIZE + 1
                })
            );
            assert_eq!(
                RpcContextHandle::from_bytes(&[0; 19]),
                Err(RpcStubError::ContextHandleLength { actual: 19 })
            );
            assert_eq!(
                RpcContextHandle::from_bytes(&[0; 20])
                    .expect("sized")
                    .require_non_null(),
                Err(RpcStubError::NullContextHandle)
            );
        }

        #[test]
        fn create_tunnel_response_decodes_non_messaging_capabilities() {
            let response = create_tunnel_response();
            assert_eq!(
                decode_tsgu_create_tunnel_response(&response).expect("valid response"),
                TsProxyCreateTunnelResponse {
                    tunnel_context: RpcContextHandle::from_bytes(&CONTEXT)
                        .expect("valid handle")
                        .require_non_null()
                        .expect("non-null"),
                    tunnel_id: 42,
                    nonce: Uuid::from_u128(0x00112233_4455_6677_8899_aabbccddeeff),
                    capabilities: 3,
                }
            );

            let mut caps_response = response;
            caps_response[4..8].copy_from_slice(&0x0000_4350u32.to_le_bytes());
            assert_eq!(
                decode_tsgu_create_tunnel_response(&caps_response),
                Err(RpcStubError::UnexpectedPacketId {
                    expected: TSG_PACKET_TYPE_QUARENC_RESPONSE,
                    actual: 0x0000_4350,
                })
            );

            let mut zero_capabilities = create_tunnel_response();
            zero_capabilities[52..60].fill(0);
            zero_capabilities.drain(68..84);
            assert_eq!(
                decode_tsgu_create_tunnel_response(&zero_capabilities)
                    .expect("null capability pointer with zero count"),
                TsProxyCreateTunnelResponse {
                    tunnel_context: RpcContextHandle::from_bytes(&CONTEXT)
                        .expect("valid handle")
                        .require_non_null()
                        .expect("non-null"),
                    tunnel_id: 42,
                    nonce: Uuid::from_u128(0x00112233_4455_6677_8899_aabbccddeeff),
                    capabilities: 0,
                }
            );
        }

        #[test]
        fn create_tunnel_response_rejects_invalid_pointers_counts_utf16_and_hresult() {
            let mut response = create_tunnel_response();
            response[20..24].copy_from_slice(&u32::try_from(MAX_CERT_CHAIN_CHARS + 1).expect("fits").to_le_bytes());
            assert_eq!(
                decode_tsgu_create_tunnel_response(&response),
                Err(RpcStubError::CertificateChainTooLarge {
                    actual: MAX_CERT_CHAIN_CHARS + 1
                })
            );

            let mut response = create_tunnel_response();
            response[56..60].copy_from_slice(&33u32.to_le_bytes());
            assert_eq!(
                decode_tsgu_create_tunnel_response(&response),
                Err(RpcStubError::CapabilityCountTooLarge { actual: 33 })
            );

            let mut response = create_tunnel_response();
            response[12..16].fill(0);
            assert_eq!(
                decode_tsgu_create_tunnel_response(&response),
                Err(RpcStubError::RequiredNdrPointerIsNull)
            );

            let mut response = create_tunnel_response();
            response[60..62].copy_from_slice(&2u16.to_le_bytes());
            assert_eq!(
                decode_tsgu_create_tunnel_response(&response),
                Err(RpcStubError::UnexpectedProtocolVersion { major: 2, minor: 1 })
            );

            let mut response = create_tunnel_response();
            response[80..84].copy_from_slice(&0x0000_0004u32.to_le_bytes());
            assert_eq!(
                decode_tsgu_create_tunnel_response(&response),
                Err(RpcStubError::UnsupportedCapabilities { actual: 0x0000_0004 })
            );

            let mut response = create_tunnel_response();
            response[64..66].copy_from_slice(&1u16.to_le_bytes());
            assert_eq!(
                decode_tsgu_create_tunnel_response(&response),
                Err(RpcStubError::MissingCertificateChain)
            );

            let mut response = create_tunnel_response();
            response[64..66].copy_from_slice(&2u16.to_le_bytes());
            assert_eq!(
                decode_tsgu_create_tunnel_response(&response),
                Err(RpcStubError::InvalidQuarantineCapabilities { actual: 2 })
            );

            let malformed_utf16 = [
                2u32.to_le_bytes().as_slice(),
                &0u32.to_le_bytes(),
                &2u32.to_le_bytes(),
                &0xd800u16.to_le_bytes(),
                &0u16.to_le_bytes(),
            ]
            .concat();
            assert_eq!(
                decode_ndr_utf16_string(&malformed_utf16, 0, 2),
                Err(RpcStubError::InvalidUtf16)
            );

            let mut response = create_tunnel_response();
            let hresult_offset = response.len() - 4;
            response[..4].fill(0);
            response[hresult_offset..].copy_from_slice(&0x8007_59d8u32.to_le_bytes());
            assert_eq!(
                decode_tsgu_create_tunnel_response(&response),
                Err(RpcStubError::RpcStatus { value: 0x8007_59d8 })
            );
        }

        #[test]
        fn authorize_response_decodes_data_policy_and_rejects_malformed_values() {
            let response = authorize_response(&[1, 2, 3]);
            assert_eq!(
                decode_tsgu_authorize_tunnel_response(&response).expect("valid response"),
                TsProxyAuthorizeTunnelResponse {
                    response_data: vec![1, 2, 3],
                    redirection_flags: TsProxyRedirectionFlags {
                        enable_all: false,
                        disable_all: false,
                        drive_disabled: true,
                        printer_disabled: false,
                        port_disabled: false,
                        clipboard_disabled: true,
                        pnp_disabled: false,
                    },
                }
            );

            let mut response = authorize_response(&[]);
            response[32..36].copy_from_slice(&1u32.to_le_bytes());
            response[36..40].copy_from_slice(&1u32.to_le_bytes());
            assert_eq!(
                decode_tsgu_authorize_tunnel_response(&response),
                Err(RpcStubError::ConflictingRedirectionFlags)
            );

            let mut response = authorize_response(&[]);
            response[60..64].copy_from_slice(&2u32.to_le_bytes());
            assert_eq!(
                decode_tsgu_authorize_tunnel_response(&response),
                Err(RpcStubError::InvalidNdrBoolean { value: 2 })
            );

            let mut response = authorize_response(&[]);
            response[52..56].copy_from_slice(&1u32.to_le_bytes());
            assert_eq!(
                decode_tsgu_authorize_tunnel_response(&response),
                Err(RpcStubError::NonZeroReservedRedirectionFlag)
            );

            let mut response = authorize_response(&[]);
            response[28..32].copy_from_slice(&u32::try_from(MAX_RESPONSE_DATA_SIZE + 1).expect("fits").to_le_bytes());
            assert_eq!(
                decode_tsgu_authorize_tunnel_response(&response),
                Err(RpcStubError::ResponseDataTooLarge {
                    actual: MAX_RESPONSE_DATA_SIZE + 1
                })
            );

            let mut response = authorize_response(&[4, 5, 6]);
            let hresult_offset = response.len() - 4;
            response[hresult_offset..].copy_from_slice(&E_PROXY_QUARANTINE_ACCESSDENIED.to_le_bytes());
            assert_eq!(
                decode_tsgu_authorize_tunnel_response(&response),
                Err(RpcStubError::QuarantineAccessDenied {
                    response_data: vec![4, 5, 6]
                })
            );

            let mut response = authorize_response(&[]);
            let hresult_offset = response.len() - 4;
            response[..4].fill(0);
            response[hresult_offset..].copy_from_slice(&0x8007_59d8u32.to_le_bytes());
            assert_eq!(
                decode_tsgu_authorize_tunnel_response(&response),
                Err(RpcStubError::RpcStatus { value: 0x8007_59d8 })
            );
        }

        #[test]
        fn channel_response_requires_non_null_context_and_success_hresult() {
            let mut response = Vec::from(CONTEXT);
            response.extend_from_slice(&17u32.to_le_bytes());
            response.extend_from_slice(&0u32.to_le_bytes());
            assert_eq!(
                decode_tsgu_create_channel_response(&response).expect("valid response"),
                TsProxyCreateChannelResponse {
                    channel_context: RpcContextHandle::from_bytes(&CONTEXT)
                        .expect("valid handle")
                        .require_non_null()
                        .expect("non-null"),
                    channel_id: 17,
                }
            );

            response[..20].fill(0);
            assert_eq!(
                decode_tsgu_create_channel_response(&response),
                Err(RpcStubError::NullContextHandle)
            );

            let mut response = Vec::from(CONTEXT);
            response.extend_from_slice(&17u32.to_le_bytes());
            response.extend_from_slice(&0x8007_59d8u32.to_le_bytes());
            response[..20].fill(0);
            assert_eq!(
                decode_tsgu_create_channel_response(&response),
                Err(RpcStubError::RpcStatus { value: 0x8007_59d8 })
            );
        }

        #[test]
        fn make_tunnel_call_encodes_message_and_cancel_requests() {
            let request = TsProxyMakeTunnelCallRequest::new(non_null_context()).encode();
            assert_eq!(
                request.as_slice(),
                [
                    CONTEXT.as_slice(),
                    &TSG_TUNNEL_CALL_ASYNC_MSG_REQUEST.to_le_bytes(),
                    &TSG_PACKET_TYPE_MSGREQUEST.to_le_bytes(),
                    &TSG_PACKET_TYPE_MSGREQUEST.to_le_bytes(),
                    &NDR_REFERENT_ID.to_le_bytes(),
                    &1u32.to_le_bytes(),
                ]
                .concat()
            );

            let request = TsProxyMakeTunnelCallRequest::cancel_pending(non_null_context()).encode();
            assert_eq!(
                request.as_slice(),
                [
                    CONTEXT.as_slice(),
                    &TSG_TUNNEL_CANCEL_ASYNC_MSG_REQUEST.to_le_bytes(),
                    &TSG_PACKET_TYPE_MSGREQUEST.to_le_bytes(),
                    &TSG_PACKET_TYPE_MSGREQUEST.to_le_bytes(),
                    &NDR_REFERENT_ID.to_le_bytes(),
                    &1u32.to_le_bytes(),
                ]
                .concat()
            );

            assert_eq!(decode_tsgu_cancel_tunnel_call_response(&[0; 8]), Ok(()));
            assert_eq!(
                decode_tsgu_cancel_tunnel_call_response(&[1, 0, 0, 0, 0, 0, 0, 0]),
                Err(RpcStubError::UnexpectedNdrPointer { actual: 1 })
            );
            assert_eq!(
                decode_tsgu_cancel_tunnel_call_response(&[0, 0, 0, 0, 5, 0, 0, 0]),
                Err(RpcStubError::RpcStatus { value: 5 })
            );
        }

        #[test]
        fn make_tunnel_call_decodes_none_string_and_reauthentication_messages() {
            let cancelled = [0u32.to_le_bytes().as_slice(), &0x8007_071au32.to_le_bytes()].concat();
            assert_eq!(
                decode_tsgu_make_tunnel_call_response(&cancelled),
                Err(RpcStubError::RpcStatus { value: 0x8007_071a })
            );

            let none = [
                NDR_REFERENT_ID.to_le_bytes().as_slice(),
                &TSG_PACKET_TYPE_MESSAGE.to_le_bytes(),
                &TSG_PACKET_TYPE_MESSAGE.to_le_bytes(),
                &(NDR_REFERENT_ID + 4).to_le_bytes(),
                &0u32.to_le_bytes(),
                &TSG_ASYNC_MESSAGE_SERVICE.to_le_bytes(),
                &0u32.to_le_bytes(),
                &TSG_ASYNC_MESSAGE_SERVICE.to_le_bytes(),
                &0u32.to_le_bytes(),
                &0u32.to_le_bytes(),
            ]
            .concat();
            assert_eq!(
                decode_tsgu_make_tunnel_call_response(&none),
                Ok(TsProxyTunnelMessage::None)
            );
            let mut invalid_none = none;
            invalid_none[20..24].copy_from_slice(&4u32.to_le_bytes());
            invalid_none[28..32].copy_from_slice(&4u32.to_le_bytes());
            assert_eq!(
                decode_tsgu_make_tunnel_call_response(&invalid_none),
                Err(RpcStubError::InvalidMessageType { actual: 4 })
            );

            let consent = [
                NDR_REFERENT_ID.to_le_bytes().as_slice(),
                &TSG_PACKET_TYPE_MESSAGE.to_le_bytes(),
                &TSG_PACKET_TYPE_MESSAGE.to_le_bytes(),
                &(NDR_REFERENT_ID + 4).to_le_bytes(),
                &0u32.to_le_bytes(),
                &TSG_ASYNC_MESSAGE_CONSENT.to_le_bytes(),
                &1u32.to_le_bytes(),
                &TSG_ASYNC_MESSAGE_CONSENT.to_le_bytes(),
                &(NDR_REFERENT_ID + 8).to_le_bytes(),
                &1u32.to_le_bytes(),
                &1u32.to_le_bytes(),
                &2u32.to_le_bytes(),
                &(NDR_REFERENT_ID + 12).to_le_bytes(),
                &2u32.to_le_bytes(),
                &[b'x', 0, 0, 0],
                &0u32.to_le_bytes(),
            ]
            .concat();
            assert_eq!(
                decode_tsgu_make_tunnel_call_response(&consent),
                Ok(TsProxyTunnelMessage::Consent {
                    display_mandatory: true,
                    consent_mandatory: true,
                    text: "x".to_owned(),
                })
            );
            let mut mismatched_consent = consent;
            mismatched_consent[28..32].copy_from_slice(&TSG_ASYNC_MESSAGE_SERVICE.to_le_bytes());
            assert_eq!(
                decode_tsgu_make_tunnel_call_response(&mismatched_consent),
                Err(RpcStubError::UnexpectedMessageSwitch {
                    expected: TSG_ASYNC_MESSAGE_CONSENT,
                    actual: TSG_ASYNC_MESSAGE_SERVICE,
                })
            );

            let service = [
                NDR_REFERENT_ID.to_le_bytes().as_slice(),
                &TSG_PACKET_TYPE_MESSAGE.to_le_bytes(),
                &TSG_PACKET_TYPE_MESSAGE.to_le_bytes(),
                &(NDR_REFERENT_ID + 4).to_le_bytes(),
                &0u32.to_le_bytes(),
                &TSG_ASYNC_MESSAGE_SERVICE.to_le_bytes(),
                &1u32.to_le_bytes(),
                &TSG_ASYNC_MESSAGE_SERVICE.to_le_bytes(),
                &(NDR_REFERENT_ID + 8).to_le_bytes(),
                &1u32.to_le_bytes(),
                &0u32.to_le_bytes(),
                &3u32.to_le_bytes(),
                &(NDR_REFERENT_ID + 12).to_le_bytes(),
                &3u32.to_le_bytes(),
                &[b'h', 0, b'i', 0, 0, 0],
                &[0, 0],
                &0u32.to_le_bytes(),
            ]
            .concat();
            assert_eq!(
                decode_tsgu_make_tunnel_call_response(&service),
                Ok(TsProxyTunnelMessage::Service {
                    display_mandatory: true,
                    text: "hi".to_owned(),
                })
            );

            let reauthentication = [
                NDR_REFERENT_ID.to_le_bytes().as_slice(),
                &TSG_PACKET_TYPE_MESSAGE.to_le_bytes(),
                &TSG_PACKET_TYPE_MESSAGE.to_le_bytes(),
                &(NDR_REFERENT_ID + 4).to_le_bytes(),
                &0u32.to_le_bytes(),
                &TSG_ASYNC_MESSAGE_REAUTH.to_le_bytes(),
                &1u32.to_le_bytes(),
                &TSG_ASYNC_MESSAGE_REAUTH.to_le_bytes(),
                &(NDR_REFERENT_ID + 8).to_le_bytes(),
                &[0; 4],
                &0x1122_3344_5566_7788u64.to_le_bytes(),
                &0u32.to_le_bytes(),
            ]
            .concat();
            assert_eq!(
                decode_tsgu_make_tunnel_call_response(&reauthentication),
                Ok(TsProxyTunnelMessage::Reauthenticate {
                    tunnel_context: 0x1122_3344_5566_7788,
                })
            );
        }

        #[test]
        fn close_context_encodes_and_decodes_the_updated_handle() {
            assert_eq!(TsProxyCloseContextRequest::new(non_null_context()).encode(), CONTEXT);

            let response = [CONTEXT.as_slice(), &0u32.to_le_bytes()].concat();
            assert_eq!(
                decode_tsgu_close_context_response(&response),
                Ok(RpcContextHandle::from_bytes(&CONTEXT).expect("valid handle"))
            );

            let mut failed_response = response;
            failed_response[RpcContextHandle::SIZE..].copy_from_slice(&0x8007_0005u32.to_le_bytes());
            assert_eq!(
                decode_tsgu_close_context_response(&failed_response),
                Err(RpcStubError::RpcStatus { value: 0x8007_0005 })
            );
        }

        #[test]
        fn setup_receive_pipe_encodes_context_and_decodes_its_final_response() {
            assert_eq!(
                TsProxySetupReceivePipeRequest::new(non_null_context()).encode(),
                CONTEXT
            );
            assert_eq!(
                decode_tsgu_setup_receive_pipe_final_response(&0x8007_59f6u32.to_le_bytes()),
                Ok(0x8007_59f6)
            );
            assert_eq!(
                decode_tsgu_setup_receive_pipe_final_response(&[0; 3]),
                Err(RpcStubError::ResponseLength { actual: 3, expected: 4 })
            );
        }

        #[test]
        fn send_to_server_encodes_one_and_three_buffers() {
            let request = TsProxySendToServerRequest::new(non_null_context(), &[4, 0, 0, 3]).expect("valid request");
            assert_eq!(
                request.encode().expect("valid encoding"),
                [
                    CONTEXT.as_slice(),
                    &[0, 0, 0, 8],
                    &[0, 0, 0, 1],
                    &[0, 0, 0, 4],
                    &[4, 0, 0, 3],
                ]
                .concat()
            );

            let mut request = TsProxySendToServerRequest::new(non_null_context(), &[1]).expect("valid request");
            request.push_buffer(&[2, 3]).expect("second buffer");
            request.push_buffer(&[4, 5, 6]).expect("third buffer");
            assert_eq!(
                request.encode().expect("valid encoding"),
                [
                    CONTEXT.as_slice(),
                    &[0, 0, 0, 18],
                    &[0, 0, 0, 3],
                    &[0, 0, 0, 1],
                    &[0, 0, 0, 2],
                    &[0, 0, 0, 3],
                    &[1, 2, 3, 4, 5, 6],
                ]
                .concat()
            );
        }

        #[test]
        fn send_to_server_validates_buffer_shape_length_and_return_value() {
            assert!(matches!(
                TsProxySendToServerRequest::new(non_null_context(), &[]),
                Err(RpcStubError::EmptyFirstBuffer)
            ));

            let mut request = TsProxySendToServerRequest::new(non_null_context(), &[1]).expect("valid request");
            request.push_buffer(&[]).expect("second buffer");
            request.push_buffer(&[]).expect("third buffer");
            assert_eq!(request.push_buffer(&[]), Err(RpcStubError::BufferCount { actual: 4 }));

            let payload = vec![0; MAX_RAW_RPC_STUB_SIZE - TsProxySendToServerRequest::PREFIX_SIZE - 4];
            let request = TsProxySendToServerRequest::new(non_null_context(), &payload).expect("maximum request");
            assert_eq!(
                request.encode().expect("maximum-size encoding").len(),
                MAX_RAW_RPC_STUB_SIZE
            );

            let payload = vec![0; MAX_RAW_RPC_STUB_SIZE - TsProxySendToServerRequest::PREFIX_SIZE - 3];
            let request = TsProxySendToServerRequest::new(non_null_context(), &payload).expect("oversized request");
            assert_eq!(
                request.encode(),
                Err(RpcStubError::RawStubTooLarge {
                    actual: MAX_RAW_RPC_STUB_SIZE + 1,
                })
            );

            assert_eq!(decode_tsgu_send_to_server_response(&0u32.to_le_bytes()), Ok(0));
            assert_eq!(decode_tsgu_send_to_server_response(&5u32.to_le_bytes()), Ok(5));
        }

        fn create_tunnel_response() -> Vec<u8> {
            let nonce = Uuid::from_u128(0x00112233_4455_6677_8899_aabbccddeeff);
            [
                NDR_REFERENT_ID.to_le_bytes().as_slice(),
                &TSG_PACKET_TYPE_QUARENC_RESPONSE.to_le_bytes(),
                &TSG_PACKET_TYPE_QUARENC_RESPONSE.to_le_bytes(),
                &(NDR_REFERENT_ID + 4).to_le_bytes(),
                &0u32.to_le_bytes(),
                &0u32.to_le_bytes(),
                &0u32.to_le_bytes(),
                &nonce.to_bytes_le(),
                &(NDR_REFERENT_ID + 8).to_le_bytes(),
                &TSG_COMPONENT_ID.to_le_bytes(),
                &TSG_PACKET_TYPE_VERSIONCAPS_ID.to_le_bytes(),
                &(NDR_REFERENT_ID + 12).to_le_bytes(),
                &1u32.to_le_bytes(),
                &1u16.to_le_bytes(),
                &1u16.to_le_bytes(),
                &0u16.to_le_bytes(),
                &0u16.to_le_bytes(),
                &1u32.to_le_bytes(),
                &TSG_CAPABILITY_TYPE_NAP.to_le_bytes(),
                &TSG_CAPABILITY_TYPE_NAP.to_le_bytes(),
                &3u32.to_le_bytes(),
                &CONTEXT,
                &42u32.to_le_bytes(),
                &0u32.to_le_bytes(),
            ]
            .concat()
        }

        fn authorize_response(response_data: &[u8]) -> Vec<u8> {
            let response_data_len = u32::try_from(response_data.len()).expect("test data fits");
            let response_data_pointer = if response_data.is_empty() {
                0
            } else {
                NDR_REFERENT_ID + 8
            };
            let mut response = [
                NDR_REFERENT_ID.to_le_bytes().as_slice(),
                &TSG_PACKET_TYPE_RESPONSE.to_le_bytes(),
                &TSG_PACKET_TYPE_RESPONSE.to_le_bytes(),
                &(NDR_REFERENT_ID + 4).to_le_bytes(),
                &TSG_PACKET_TYPE_QUARREQUEST.to_le_bytes(),
                &0u32.to_le_bytes(),
                &response_data_pointer.to_le_bytes(),
                &response_data_len.to_le_bytes(),
                &0u32.to_le_bytes(),
                &0u32.to_le_bytes(),
                &1u32.to_le_bytes(),
                &0u32.to_le_bytes(),
                &0u32.to_le_bytes(),
                &0u32.to_le_bytes(),
                &1u32.to_le_bytes(),
                &0u32.to_le_bytes(),
            ]
            .concat();
            if !response_data.is_empty() {
                response.extend_from_slice(&response_data_len.to_le_bytes());
                response.extend_from_slice(response_data);
                pad_ndr_4(&mut response);
            }
            response.extend_from_slice(&0u32.to_le_bytes());
            response
        }
    }
}

/// An exact 16-byte RPC-over-HTTP RTS cookie.
///
/// [MS-RPCH] 2.2.3.5.4.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RtsCookie([u8; Self::SIZE]);

impl RtsCookie {
    /// Size of an RTS cookie.
    pub const SIZE: usize = 16;

    /// Creates a cookie from its wire representation.
    pub const fn new(bytes: [u8; Self::SIZE]) -> Self {
        Self(bytes)
    }

    /// Returns the wire representation.
    pub const fn as_bytes(&self) -> &[u8; Self::SIZE] {
        &self.0
    }
}

impl fmt::Debug for RtsCookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RtsCookie(..)")
    }
}

/// Client-controlled values used to open an RPCH virtual connection.
///
/// [MS-RPCH] 2.2.3.5.1, 2.2.3.5.5, and 2.2.3.5.6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpchV2Settings {
    receive_window_size: u32,
    channel_lifetime: u32,
    client_keepalive: u32,
}

impl RpchV2Settings {
    /// Creates validated client RPCH settings.
    pub fn new(receive_window_size: u32, channel_lifetime: u32, client_keepalive: u32) -> Result<Self, RpcPduError> {
        validate_rts_receive_window_size(receive_window_size)?;
        validate_rts_channel_lifetime(channel_lifetime)?;
        validate_rts_client_keepalive(client_keepalive)?;

        Ok(Self {
            receive_window_size,
            channel_lifetime,
            client_keepalive,
        })
    }

    /// Local receive window advertised in CONN/A1.
    pub const fn receive_window_size(self) -> u32 {
        self.receive_window_size
    }

    /// Requested IN-channel lifetime.
    pub const fn channel_lifetime(self) -> u32 {
        self.channel_lifetime
    }

    /// Requested client keepalive interval.
    pub const fn client_keepalive(self) -> u32 {
        self.client_keepalive
    }
}

impl Default for RpchV2Settings {
    fn default() -> Self {
        Self {
            receive_window_size: 64 * 1024,
            channel_lifetime: 1024 * 1024 * 1024,
            client_keepalive: DEFAULT_CLIENT_KEEPALIVE,
        }
    }
}

/// RPCH virtual-connection opening state.
///
/// [MS-RPCH] 3.2.2.4.1.2 and 3.2.2.5.2 through 3.2.2.5.4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpchV2State {
    /// No channel request has started.
    Initial,
    /// The authenticated IN request has started.
    InRequestStarted,
    /// CONN/A1 is ready as the OUT request body.
    OutRequestStarted,
    /// CONN/B1 was produced for the IN request body.
    AwaitingOutResponse,
    /// A successful OUT response was accepted.
    AwaitingA3,
    /// CONN/A3 was accepted.
    AwaitingC2,
    /// The default IN and OUT channels are ready for RPC PDUs.
    Open,
    /// An invalid setup event terminated the opening sequence.
    Failed,
}

/// Errors reported while advancing RPCH v2 connection setup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpchV2Error {
    /// An event is not valid in the current setup state.
    InvalidState { action: &'static str, state: RpchV2State },
    /// The OUT HTTP response was not successful.
    OutResponseStatus { actual: u16 },
    /// An RTS PDU was malformed or semantically invalid.
    Rts(RpcPduError),
}

impl fmt::Display for RpchV2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidState { action, state } => {
                write!(f, "cannot {action} while rpch setup is {state:?}")
            }
            Self::OutResponseStatus { actual } => {
                write!(f, "invalid rpch OUT response status {actual}")
            }
            Self::Rts(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for RpchV2Error {}

impl From<RpcPduError> for RpchV2Error {
    fn from(error: RpcPduError) -> Self {
        Self::Rts(error)
    }
}

/// Values received in an IN_R1/A4 RTS PDU.
///
/// [MS-RPCH] 2.2.4.13.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtsInRecycleA4 {
    version: u32,
    receive_window_size: u32,
    connection_timeout: u32,
}

impl RtsInRecycleA4 {
    /// Protocol version selected from the IN_R1/A2 and server versions.
    ///
    /// Clients ignore received Version values ([MS-RPCH] 2.2.3.5.7).
    pub const fn version(self) -> u32 {
        self.version
    }

    /// Receive window supplied for the successor IN channel.
    pub const fn receive_window_size(self) -> u32 {
        self.receive_window_size
    }

    /// Connection timeout supplied for the successor IN channel.
    pub const fn connection_timeout(self) -> u32 {
        self.connection_timeout
    }
}

/// Values received in an OUT_R1/A6 RTS PDU.
///
/// [MS-RPCH] 2.2.4.28.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtsOutRecycleA6 {
    version: u32,
    connection_timeout: u32,
}

impl RtsOutRecycleA6 {
    /// Protocol version selected from the OUT_R1/A4 and server versions.
    ///
    /// Clients ignore received Version values ([MS-RPCH] 2.2.3.5.7).
    pub const fn version(self) -> u32 {
        self.version
    }

    /// Connection timeout supplied for the successor OUT channel.
    ///
    /// This value is informational to clients ([MS-RPCH] 2.2.4.28).
    pub const fn connection_timeout(self) -> u32 {
        self.connection_timeout
    }
}

/// State of the virtual IN channel recycling sequence.
///
/// [MS-RPCH] 3.2.2.5.5 and 3.2.2.5.12.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpchInChannelRecycleState {
    /// The default IN channel is open.
    Open,
    /// IN_R1/A1 was produced and IN_R1/A4 is expected.
    AwaitingA4,
    /// IN_R1/A5 was produced and must be sent before the channel is reopened.
    AwaitingA5,
    /// An invalid recycle event was received.
    Failed,
}

/// State of the virtual OUT channel recycling sequence.
///
/// [MS-RPCH] 3.2.2.5.6 through 3.2.2.5.8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpchOutChannelRecycleState {
    /// The default OUT channel is open.
    Open,
    /// OUT_R1/A2 was accepted and OUT_R1/A6 is expected.
    AwaitingA6,
    /// OUT_R1/A6 was accepted and OUT_R1/A10 is expected.
    AwaitingA10,
    /// OUT_R1/A11 was produced and must be sent before the channel is reopened.
    AwaitingA11,
    /// An invalid recycle event was received.
    Failed,
}

/// Errors reported while advancing an RPCH v2 channel recycling sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpchChannelRecycleError {
    /// An IN recycling event is not valid in the current state.
    InvalidInState {
        action: &'static str,
        state: RpchInChannelRecycleState,
    },
    /// An OUT recycling event is not valid in the current state.
    InvalidOutState {
        action: &'static str,
        state: RpchOutChannelRecycleState,
    },
    /// An RTS PDU was malformed or semantically invalid.
    Rts(RpcPduError),
}

impl fmt::Display for RpchChannelRecycleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInState { action, state } => {
                write!(f, "cannot {action} while IN recycling is {state:?}")
            }
            Self::InvalidOutState { action, state } => {
                write!(f, "cannot {action} while OUT recycling is {state:?}")
            }
            Self::Rts(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for RpchChannelRecycleError {}

impl From<RpcPduError> for RpchChannelRecycleError {
    fn from(error: RpcPduError) -> Self {
        Self::Rts(error)
    }
}

/// Stateful validation for the initial RPCH v2 CONN sequence.
///
/// This transport-independent state machine produces initial request bodies and accepts the OUT response status plus CONN/A3 and CONN/C2.
/// It does not send HTTP requests or consume a live transport.
///
/// [MS-RPCH] 3.2.2.4.1.2 and 3.2.2.5.2 through 3.2.2.5.4.
#[derive(Debug)]
pub struct RpchV2Setup {
    settings: RpchV2Settings,
    virtual_connection_cookie: RtsCookie,
    out_channel_cookie: RtsCookie,
    in_channel_cookie: RtsCookie,
    association_group_id: RtsCookie,
    state: RpchV2State,
    in_channel_ping_timeout: Option<u32>,
    connection_timeout: Option<u32>,
    peer_receive_window_size: Option<u32>,
}

impl RpchV2Setup {
    /// Creates a setup state machine with caller-supplied connection cookies.
    pub const fn new(
        settings: RpchV2Settings,
        virtual_connection_cookie: RtsCookie,
        out_channel_cookie: RtsCookie,
        in_channel_cookie: RtsCookie,
        association_group_id: RtsCookie,
    ) -> Self {
        Self {
            settings,
            virtual_connection_cookie,
            out_channel_cookie,
            in_channel_cookie,
            association_group_id,
            state: RpchV2State::Initial,
            in_channel_ping_timeout: None,
            connection_timeout: None,
            peer_receive_window_size: None,
        }
    }

    /// Current setup state.
    pub const fn state(&self) -> RpchV2State {
        self.state
    }

    /// Server connection timeout used for IN-channel ping scheduling.
    pub const fn in_channel_ping_timeout(&self) -> Option<u32> {
        self.in_channel_ping_timeout
    }

    /// Negotiated virtual-connection timeout.
    pub const fn connection_timeout(&self) -> Option<u32> {
        self.connection_timeout
    }

    /// Peer receive window advertised in CONN/C2.
    pub const fn peer_receive_window_size(&self) -> Option<u32> {
        self.peer_receive_window_size
    }

    /// Records that the authenticated IN request has started.
    pub fn start_in_request(&mut self) -> Result<(), RpchV2Error> {
        if self.state != RpchV2State::Initial {
            return self.fail(RpchV2Error::InvalidState {
                action: "start the IN request",
                state: self.state,
            });
        }

        self.state = RpchV2State::InRequestStarted;
        Ok(())
    }

    /// Produces the exact 76-byte CONN/A1 body for the OUT request.
    pub fn out_request_body(&mut self) -> Result<Vec<u8>, RpchV2Error> {
        if self.state != RpchV2State::InRequestStarted {
            return self.fail(RpchV2Error::InvalidState {
                action: "start the OUT request",
                state: self.state,
            });
        }

        let body = encode_rts_conn_a1(
            self.virtual_connection_cookie,
            self.out_channel_cookie,
            self.settings.receive_window_size,
        )?;
        debug_assert_eq!(body.len(), RPCH_OUT_CONTENT_LENGTH);
        self.state = RpchV2State::OutRequestStarted;
        Ok(body)
    }

    /// Produces CONN/B1, the first PDU in the IN request body.
    pub fn in_request_initial_pdu(&mut self) -> Result<Vec<u8>, RpchV2Error> {
        if self.state != RpchV2State::OutRequestStarted {
            return self.fail(RpchV2Error::InvalidState {
                action: "send CONN/B1",
                state: self.state,
            });
        }

        let pdu = encode_rts_conn_b1(
            self.virtual_connection_cookie,
            self.in_channel_cookie,
            self.settings.channel_lifetime,
            self.settings.client_keepalive,
            self.association_group_id,
        )?;
        self.state = RpchV2State::AwaitingOutResponse;
        Ok(pdu)
    }

    /// Accepts the OUT HTTP response status before processing its RTS body.
    pub fn accept_out_response(&mut self, status: u16) -> Result<(), RpchV2Error> {
        if self.state != RpchV2State::AwaitingOutResponse {
            return self.fail(RpchV2Error::InvalidState {
                action: "accept the OUT response",
                state: self.state,
            });
        }
        if status != 200 {
            return self.fail(RpchV2Error::OutResponseStatus { actual: status });
        }

        self.state = RpchV2State::AwaitingA3;
        Ok(())
    }

    /// Processes the next initial RTS PDU from the OUT response body.
    pub fn receive_out_pdu(&mut self, pdu: &[u8]) -> Result<(), RpchV2Error> {
        match self.state {
            RpchV2State::AwaitingA3 => {
                let connection_timeout = match decode_rts_conn_a3(pdu) {
                    Ok(connection_timeout) => connection_timeout,
                    Err(error) => return self.fail(error.into()),
                };
                self.in_channel_ping_timeout = Some(connection_timeout);
                self.state = RpchV2State::AwaitingC2;
                Ok(())
            }
            RpchV2State::AwaitingC2 => {
                let (peer_receive_window_size, connection_timeout) = match decode_rts_conn_c2(pdu) {
                    Ok(values) => values,
                    Err(error) => return self.fail(error.into()),
                };
                self.connection_timeout = Some(connection_timeout);
                self.peer_receive_window_size = Some(peer_receive_window_size);
                self.state = RpchV2State::Open;
                Ok(())
            }
            state => self.fail(RpchV2Error::InvalidState {
                action: "consume an OUT setup PDU",
                state,
            }),
        }
    }

    /// Creates accounting for the established default IN and OUT channels.
    pub fn flow_control(&self) -> Result<RpchFlowControl, RpchV2Error> {
        if self.state != RpchV2State::Open {
            return Err(RpchV2Error::InvalidState {
                action: "create RPCH flow-control state",
                state: self.state,
            });
        }

        let peer_receive_window_size = self.peer_receive_window_size.ok_or(RpchV2Error::InvalidState {
            action: "read the peer receive window",
            state: self.state,
        })?;
        Ok(RpchFlowControl::new(
            self.settings.receive_window_size,
            peer_receive_window_size,
            self.out_channel_cookie,
            self.in_channel_cookie,
        ))
    }

    /// Creates the ping schedule for the established default IN channel.
    ///
    /// `keepalive_interval` is configured by the higher layer, independently
    /// of the proxy-facing ClientKeepalive command in CONN/B1.
    pub fn ping_schedule(&self, keepalive_interval: Duration, now: Duration) -> Result<RpchPingSchedule, RpchV2Error> {
        if self.state != RpchV2State::Open {
            return Err(RpchV2Error::InvalidState {
                action: "create RPCH ping schedule",
                state: self.state,
            });
        }

        let connection_timeout = self.in_channel_ping_timeout.ok_or(RpchV2Error::InvalidState {
            action: "read the IN channel connection timeout",
            state: self.state,
        })?;
        Ok(RpchPingSchedule::new(
            Duration::from_millis(u64::from(connection_timeout)),
            keepalive_interval,
            now,
        ))
    }

    /// Creates state validation for channel recycling after the initial CONN sequence opens.
    ///
    /// [MS-RPCH] 3.2.2.5.5 through 3.2.2.5.8 and 3.2.2.5.12.
    pub fn channel_recycling(&mut self) -> Result<RpchV2ChannelRecycle<'_>, RpchV2Error> {
        if self.state != RpchV2State::Open {
            return Err(RpchV2Error::InvalidState {
                action: "create RPCH channel recycling state",
                state: self.state,
            });
        }

        Ok(RpchV2ChannelRecycle {
            setup: self,
            in_state: RpchInChannelRecycleState::Open,
            out_state: RpchOutChannelRecycleState::Open,
            successor_in_channel_cookie: None,
            successor_out_channel_cookie: None,
        })
    }

    fn fail<T>(&mut self, error: RpchV2Error) -> Result<T, RpchV2Error> {
        self.state = RpchV2State::Failed;
        Err(error)
    }
}

/// Stateful validation for the RPCH v2 R1 channel recycling sequences.
///
/// The caller creates and switches transport channels around the returned RTS PDUs.
/// This type owns no I/O and validates only the protocol-visible ordering.
///
/// [MS-RPCH] 3.2.2.5.5 through 3.2.2.5.8 and 3.2.2.5.12.
#[derive(Debug)]
pub struct RpchV2ChannelRecycle<'a> {
    setup: &'a mut RpchV2Setup,
    in_state: RpchInChannelRecycleState,
    out_state: RpchOutChannelRecycleState,
    successor_in_channel_cookie: Option<RtsCookie>,
    successor_out_channel_cookie: Option<RtsCookie>,
}

impl RpchV2ChannelRecycle<'_> {
    /// Current virtual IN channel recycling state.
    pub const fn in_state(&self) -> RpchInChannelRecycleState {
        self.in_state
    }

    /// Current virtual OUT channel recycling state.
    pub const fn out_state(&self) -> RpchOutChannelRecycleState {
        self.out_state
    }

    /// Current default IN channel cookie.
    pub const fn default_in_channel_cookie(&self) -> RtsCookie {
        self.setup.in_channel_cookie
    }

    /// Current default OUT channel cookie.
    pub const fn default_out_channel_cookie(&self) -> RtsCookie {
        self.setup.out_channel_cookie
    }

    /// Connection timeout for the current default IN channel.
    pub fn in_channel_ping_timeout(&self) -> u32 {
        match self.setup.in_channel_ping_timeout {
            Some(timeout) => timeout,
            None => unreachable!("opened RPCH v2 setup has an IN channel connection timeout"),
        }
    }

    /// Peer receive window for the current default IN channel.
    pub fn peer_receive_window_size(&self) -> u32 {
        match self.setup.peer_receive_window_size {
            Some(window_size) => window_size,
            None => unreachable!("opened RPCH v2 setup has a peer receive window"),
        }
    }

    /// Starts IN_R1 channel recycling and returns IN_R1/A1 for the successor IN channel.
    ///
    /// The caller must send the PDU immediately after opening the successor IN request.
    ///
    /// [MS-RPCH] 2.2.4.10 and 3.2.2.5.12.
    pub fn start_in_recycling(
        &mut self,
        successor_channel_cookie: RtsCookie,
    ) -> Result<Vec<u8>, RpchChannelRecycleError> {
        if self.in_state != RpchInChannelRecycleState::Open {
            return self.fail(RpchChannelRecycleError::InvalidInState {
                action: "start IN channel recycling",
                state: self.in_state,
            });
        }

        let pdu = encode_rts_in_recycle_a1(
            self.setup.virtual_connection_cookie,
            self.setup.in_channel_cookie,
            successor_channel_cookie,
        )?;
        self.successor_in_channel_cookie = Some(successor_channel_cookie);
        self.in_state = RpchInChannelRecycleState::AwaitingA4;
        Ok(pdu)
    }

    /// Accepts IN_R1/A4 and returns IN_R1/A5 for the predecessor IN channel.
    ///
    /// The returned PDU must be sent after predecessor-channel PDUs have drained.
    ///
    /// [MS-RPCH] 2.2.4.13, 2.2.4.14, and 3.2.2.5.5.
    pub fn receive_in_recycle_a4(&mut self, pdu: &[u8]) -> Result<Vec<u8>, RpchChannelRecycleError> {
        if self.in_state != RpchInChannelRecycleState::AwaitingA4 {
            return self.fail(RpchChannelRecycleError::InvalidInState {
                action: "accept IN_R1/A4",
                state: self.in_state,
            });
        }

        let a4 = match decode_rts_in_recycle_a4(pdu) {
            Ok(a4) => a4,
            Err(error) => return self.fail(error.into()),
        };
        let successor_channel_cookie = match self.successor_in_channel_cookie {
            Some(cookie) => cookie,
            None => {
                return self.fail(RpchChannelRecycleError::InvalidInState {
                    action: "read the successor IN channel cookie",
                    state: self.in_state,
                });
            }
        };
        let a5 = encode_rts_in_recycle_a5(successor_channel_cookie)?;
        self.setup.in_channel_cookie = successor_channel_cookie;
        self.setup.in_channel_ping_timeout = Some(a4.connection_timeout);
        self.setup.peer_receive_window_size = Some(a4.receive_window_size);
        self.successor_in_channel_cookie = None;
        self.in_state = RpchInChannelRecycleState::AwaitingA5;
        Ok(a5)
    }

    /// Completes IN recycling after IN_R1/A5 was sent and the successor IN channel was unplugged.
    ///
    /// The caller must retain the supplied state for the virtual connection.
    ///
    /// [MS-RPCH] 3.2.2.5.5.
    pub fn finish_in_recycling(
        &mut self,
        flow_control: &mut RpchFlowControl,
        ping_schedule: &mut RpchPingSchedule,
    ) -> Result<(), RpchChannelRecycleError> {
        if self.in_state != RpchInChannelRecycleState::AwaitingA5 {
            return self.fail(RpchChannelRecycleError::InvalidInState {
                action: "finish IN channel recycling",
                state: self.in_state,
            });
        }

        flow_control.reset_sending_channel(self.peer_receive_window_size(), self.default_in_channel_cookie());
        ping_schedule.set_connection_timeout(self.in_channel_ping_timeout());
        self.in_state = RpchInChannelRecycleState::Open;
        Ok(())
    }

    /// Accepts OUT_R1/A2 and returns OUT_R1/A3 for the successor OUT channel.
    ///
    /// The caller creates the successor OUT channel before sending the returned PDU.
    ///
    /// [MS-RPCH] 2.2.4.24, 2.2.4.25, and 3.2.2.5.6.
    pub fn receive_out_recycle_a2(
        &mut self,
        pdu: &[u8],
        successor_channel_cookie: RtsCookie,
    ) -> Result<Vec<u8>, RpchChannelRecycleError> {
        if self.out_state != RpchOutChannelRecycleState::Open {
            return self.fail(RpchChannelRecycleError::InvalidOutState {
                action: "accept OUT_R1/A2",
                state: self.out_state,
            });
        }

        if let Err(error) = decode_rts_out_recycle_a2(pdu) {
            return self.fail(error.into());
        }
        let a3 = encode_rts_out_recycle_a3(
            self.setup.virtual_connection_cookie,
            self.setup.out_channel_cookie,
            successor_channel_cookie,
            self.setup.settings.receive_window_size,
        )?;
        self.successor_out_channel_cookie = Some(successor_channel_cookie);
        self.out_state = RpchOutChannelRecycleState::AwaitingA6;
        Ok(a3)
    }

    /// Accepts OUT_R1/A6 and returns OUT_R1/A7 for the default IN channel.
    ///
    /// [MS-RPCH] 2.2.4.28, 2.2.4.29, and 3.2.2.5.7.
    pub fn receive_out_recycle_a6(&mut self, pdu: &[u8]) -> Result<Vec<u8>, RpchChannelRecycleError> {
        if self.out_state != RpchOutChannelRecycleState::AwaitingA6 {
            return self.fail(RpchChannelRecycleError::InvalidOutState {
                action: "accept OUT_R1/A6",
                state: self.out_state,
            });
        }

        if let Err(error) = decode_rts_out_recycle_a6(pdu) {
            return self.fail(error.into());
        }
        let successor_channel_cookie = match self.successor_out_channel_cookie {
            Some(cookie) => cookie,
            None => {
                return self.fail(RpchChannelRecycleError::InvalidOutState {
                    action: "read the successor OUT channel cookie",
                    state: self.out_state,
                });
            }
        };
        let a7 = encode_rts_out_recycle_a7(successor_channel_cookie)?;
        self.out_state = RpchOutChannelRecycleState::AwaitingA10;
        Ok(a7)
    }

    /// Accepts OUT_R1/A10 and returns OUT_R1/A11 for the successor OUT channel.
    ///
    /// [MS-RPCH] 2.2.4.32, 2.2.4.33, and 3.2.2.5.8.
    pub fn receive_out_recycle_a10(&mut self, pdu: &[u8]) -> Result<Vec<u8>, RpchChannelRecycleError> {
        if self.out_state != RpchOutChannelRecycleState::AwaitingA10 {
            return self.fail(RpchChannelRecycleError::InvalidOutState {
                action: "accept OUT_R1/A10",
                state: self.out_state,
            });
        }

        if let Err(error) = decode_rts_out_recycle_a10(pdu) {
            return self.fail(error.into());
        }
        let successor_channel_cookie = match self.successor_out_channel_cookie {
            Some(cookie) => cookie,
            None => {
                return self.fail(RpchChannelRecycleError::InvalidOutState {
                    action: "read the successor OUT channel cookie",
                    state: self.out_state,
                });
            }
        };
        let a11 = encode_rts_out_recycle_a11()?;
        self.setup.out_channel_cookie = successor_channel_cookie;
        self.successor_out_channel_cookie = None;
        self.out_state = RpchOutChannelRecycleState::AwaitingA11;
        Ok(a11)
    }

    /// Completes OUT recycling after OUT_R1/A11 was sent.
    ///
    /// The caller must retain the supplied state for the virtual connection.
    ///
    /// [MS-RPCH] 3.2.2.5.8.
    pub fn finish_out_recycling(&mut self, flow_control: &mut RpchFlowControl) -> Result<(), RpchChannelRecycleError> {
        if self.out_state != RpchOutChannelRecycleState::AwaitingA11 {
            return self.fail(RpchChannelRecycleError::InvalidOutState {
                action: "finish OUT channel recycling",
                state: self.out_state,
            });
        }

        flow_control.reset_receiving_channel(self.default_out_channel_cookie());
        self.out_state = RpchOutChannelRecycleState::Open;
        Ok(())
    }

    fn fail<T>(&mut self, error: RpchChannelRecycleError) -> Result<T, RpchChannelRecycleError> {
        // A protocol error terminates the entire virtual connection.
        self.setup.state = RpchV2State::Failed;
        self.in_state = RpchInChannelRecycleState::Failed;
        self.out_state = RpchOutChannelRecycleState::Failed;
        Err(error)
    }
}

/// A flow-control acknowledgement for an RPCH channel.
///
/// [MS-RPCH] 2.2.3.4 and 2.2.4.50.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RtsFlowControlAck {
    bytes_received: u32,
    available_window: u32,
    channel_cookie: RtsCookie,
}

impl RtsFlowControlAck {
    /// Creates a flow-control acknowledgement.
    pub const fn new(bytes_received: u32, available_window: u32, channel_cookie: RtsCookie) -> Self {
        Self {
            bytes_received,
            available_window,
            channel_cookie,
        }
    }

    /// Total bytes received by the peer.
    pub const fn bytes_received(self) -> u32 {
        self.bytes_received
    }

    /// Receiver capacity available when the acknowledgement was sent.
    pub const fn available_window(self) -> u32 {
        self.available_window
    }

    /// Cookie identifying the acknowledged channel.
    pub const fn channel_cookie(self) -> RtsCookie {
        self.channel_cookie
    }
}

/// Errors reported while applying RPCH flow-control accounting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpchFlowControlError {
    /// A platform-sized PDU length does not fit the wire counter.
    PduLengthOverflow { actual: usize },
    /// A sent PDU does not fit the available peer window.
    SendWindowExhausted { pdu_size: u32, available_window: u32 },
    /// A received PDU does not fit the local receive window.
    ReceiveWindowExhausted { pdu_size: u32, available_window: u32 },
    /// A consumer released more bytes than are queued locally.
    PduNotQueued { pdu_size: u32, queued_bytes: u32 },
    /// The received-byte counter would overflow.
    BytesReceivedOverflow { current: u32, pdu_size: u32 },
    /// The sent-byte counter would overflow.
    BytesSentOverflow { current: u32, pdu_size: u32 },
    /// An acknowledgement claims bytes that were not sent.
    InvalidFlowControlAck { bytes_received: u32, bytes_sent: u32 },
    /// An acknowledgement reduced its cumulative byte counter.
    RegressingFlowControlAck {
        bytes_received: u32,
        previous_bytes_received: u32,
    },
    /// An acknowledgement advertises more capacity than the peer window.
    FlowControlAckWindowExceedsPeer {
        available_window: u32,
        peer_receive_window_size: u32,
    },
    /// The acknowledgement cannot cover sent but unacknowledged bytes.
    FlowControlAckExhaustsSenderWindow {
        available_window: u32,
        unacknowledged_bytes: u32,
    },
}

impl fmt::Display for RpchFlowControlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PduLengthOverflow { actual } => write!(f, "rpch pdu length {actual} exceeds u32"),
            Self::SendWindowExhausted {
                pdu_size,
                available_window,
            } => write!(
                f,
                "rpch pdu size {pdu_size} exceeds available send window {available_window}"
            ),
            Self::ReceiveWindowExhausted {
                pdu_size,
                available_window,
            } => write!(
                f,
                "rpch pdu size {pdu_size} exceeds available receive window {available_window}"
            ),
            Self::PduNotQueued { pdu_size, queued_bytes } => write!(
                f,
                "rpch consumed pdu size {pdu_size} exceeds queued bytes {queued_bytes}"
            ),
            Self::BytesReceivedOverflow { current, pdu_size } => {
                write!(f, "rpch received byte count overflows: {current} + {pdu_size}")
            }
            Self::BytesSentOverflow { current, pdu_size } => {
                write!(f, "rpch sent byte count overflows: {current} + {pdu_size}")
            }
            Self::InvalidFlowControlAck {
                bytes_received,
                bytes_sent,
            } => write!(
                f,
                "rpch flow-control ack bytes received {bytes_received} exceeds bytes sent {bytes_sent}"
            ),
            Self::RegressingFlowControlAck {
                bytes_received,
                previous_bytes_received,
            } => write!(
                f,
                "rpch flow-control ack bytes received {bytes_received} precedes {previous_bytes_received}"
            ),
            Self::FlowControlAckWindowExceedsPeer {
                available_window,
                peer_receive_window_size,
            } => write!(
                f,
                "rpch flow-control ack window {available_window} exceeds peer receive window {peer_receive_window_size}"
            ),
            Self::FlowControlAckExhaustsSenderWindow {
                available_window,
                unacknowledged_bytes,
            } => write!(
                f,
                "rpch flow-control ack window {available_window} is below unacknowledged bytes {unacknowledged_bytes}"
            ),
        }
    }
}

impl core::error::Error for RpchFlowControlError {}

/// Flow-control state for the default RPCH IN and OUT channels.
///
/// RPC PDUs received on the OUT channel consume the local receive window.
/// RPC PDUs sent on the IN channel consume the peer receive window.
///
/// [MS-RPCH] 3.2.1.4.1 and 3.2.1.5.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpchFlowControl {
    receive_window_size: u32,
    receive_available_window: u32,
    receive_available_window_advertised: i64,
    receive_bytes_received: u32,
    peer_receive_window_size: u32,
    send_available_window: u32,
    send_bytes_sent: u32,
    send_bytes_acknowledged: u32,
    receive_channel_cookie: RtsCookie,
    send_channel_cookie: RtsCookie,
}

impl RpchFlowControl {
    fn new(
        receive_window_size: u32,
        peer_receive_window_size: u32,
        receive_channel_cookie: RtsCookie,
        send_channel_cookie: RtsCookie,
    ) -> Self {
        Self {
            receive_window_size,
            receive_available_window: receive_window_size,
            receive_available_window_advertised: i64::from(receive_window_size),
            receive_bytes_received: 0,
            peer_receive_window_size,
            send_available_window: peer_receive_window_size,
            send_bytes_sent: 0,
            send_bytes_acknowledged: 0,
            receive_channel_cookie,
            send_channel_cookie,
        }
    }

    fn reset_sending_channel(&mut self, peer_receive_window_size: u32, channel_cookie: RtsCookie) {
        self.peer_receive_window_size = peer_receive_window_size;
        self.send_available_window = peer_receive_window_size;
        self.send_bytes_sent = 0;
        self.send_bytes_acknowledged = 0;
        self.send_channel_cookie = channel_cookie;
    }

    fn reset_receiving_channel(&mut self, channel_cookie: RtsCookie) {
        self.receive_available_window = self.receive_window_size;
        self.receive_available_window_advertised = i64::from(self.receive_window_size);
        self.receive_bytes_received = 0;
        self.receive_channel_cookie = channel_cookie;
    }

    /// Current available capacity for IN-channel RPC PDUs.
    pub const fn send_available_window(&self) -> u32 {
        self.send_available_window
    }

    /// Current available capacity in the local OUT-channel receive window.
    pub const fn receive_available_window(&self) -> u32 {
        self.receive_available_window
    }

    /// Records an RPC PDU sent on the default IN channel.
    pub fn sent_rpc_pdu(&mut self, pdu_length: usize) -> Result<(), RpchFlowControlError> {
        let pdu_size =
            u32::try_from(pdu_length).map_err(|_| RpchFlowControlError::PduLengthOverflow { actual: pdu_length })?;
        if self.send_available_window <= pdu_size {
            return Err(RpchFlowControlError::SendWindowExhausted {
                pdu_size,
                available_window: self.send_available_window,
            });
        }

        self.send_bytes_sent =
            self.send_bytes_sent
                .checked_add(pdu_size)
                .ok_or(RpchFlowControlError::BytesSentOverflow {
                    current: self.send_bytes_sent,
                    pdu_size,
                })?;
        self.send_available_window -= pdu_size;
        Ok(())
    }

    /// Records an RPC PDU received on the default OUT channel.
    pub fn received_rpc_pdu(&mut self, pdu_length: usize) -> Result<(), RpchFlowControlError> {
        let pdu_size =
            u32::try_from(pdu_length).map_err(|_| RpchFlowControlError::PduLengthOverflow { actual: pdu_length })?;
        if pdu_size > self.receive_available_window {
            return Err(RpchFlowControlError::ReceiveWindowExhausted {
                pdu_size,
                available_window: self.receive_available_window,
            });
        }

        self.receive_bytes_received =
            self.receive_bytes_received
                .checked_add(pdu_size)
                .ok_or(RpchFlowControlError::BytesReceivedOverflow {
                    current: self.receive_bytes_received,
                    pdu_size,
                })?;
        self.receive_available_window -= pdu_size;
        self.receive_available_window_advertised -= i64::from(pdu_size);
        Ok(())
    }

    /// Records a higher layer consuming bytes from the local receive window.
    ///
    /// Returns an acknowledgement when at least half the local window has been reclaimed since the previous acknowledgement.
    pub fn consumed_rpc_pdu(&mut self, pdu_length: usize) -> Result<Option<RtsFlowControlAck>, RpchFlowControlError> {
        let pdu_size =
            u32::try_from(pdu_length).map_err(|_| RpchFlowControlError::PduLengthOverflow { actual: pdu_length })?;
        let queued_bytes = self.receive_window_size - self.receive_available_window;
        if pdu_size > queued_bytes {
            return Err(RpchFlowControlError::PduNotQueued { pdu_size, queued_bytes });
        }
        self.receive_available_window += pdu_size;

        let reclaimed_window = i64::from(self.receive_available_window) - self.receive_available_window_advertised;
        if reclaimed_window < i64::from(self.receive_window_size / 2) {
            return Ok(None);
        }

        self.receive_available_window_advertised = i64::from(self.receive_available_window);
        Ok(Some(RtsFlowControlAck::new(
            self.receive_bytes_received,
            self.receive_available_window,
            self.receive_channel_cookie,
        )))
    }

    /// Applies an acknowledgement received on the default OUT channel.
    ///
    /// An acknowledgement for another channel is ignored.
    pub fn receive_flow_control_ack(&mut self, ack: RtsFlowControlAck) -> Result<bool, RpchFlowControlError> {
        if ack.channel_cookie != self.send_channel_cookie {
            return Ok(false);
        }
        if ack.bytes_received > self.send_bytes_sent {
            return Err(RpchFlowControlError::InvalidFlowControlAck {
                bytes_received: ack.bytes_received,
                bytes_sent: self.send_bytes_sent,
            });
        }
        if ack.bytes_received < self.send_bytes_acknowledged {
            return Err(RpchFlowControlError::RegressingFlowControlAck {
                bytes_received: ack.bytes_received,
                previous_bytes_received: self.send_bytes_acknowledged,
            });
        }
        if ack.available_window > self.peer_receive_window_size {
            return Err(RpchFlowControlError::FlowControlAckWindowExceedsPeer {
                available_window: ack.available_window,
                peer_receive_window_size: self.peer_receive_window_size,
            });
        }

        let unacknowledged_bytes = self.send_bytes_sent - ack.bytes_received;
        self.send_available_window = ack.available_window.checked_sub(unacknowledged_bytes).ok_or(
            RpchFlowControlError::FlowControlAckExhaustsSenderWindow {
                available_window: ack.available_window,
                unacknowledged_bytes,
            },
        )?;
        self.send_bytes_acknowledged = ack.bytes_received;
        Ok(true)
    }
}

/// Schedules PING PDUs for the default RPCH IN channel.
///
/// [MS-RPCH] 3.2.1.2.1, 3.2.1.2.2, and 3.2.2.6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpchPingSchedule {
    connection_timeout: Duration,
    keepalive_interval: Duration,
    last_send: Duration,
}

impl RpchPingSchedule {
    const fn new(connection_timeout: Duration, keepalive_interval: Duration, now: Duration) -> Self {
        Self {
            connection_timeout,
            keepalive_interval,
            last_send: now,
        }
    }

    fn set_connection_timeout(&mut self, connection_timeout: u32) {
        self.connection_timeout = Duration::from_millis(u64::from(connection_timeout));
    }

    /// Returns whether a PING must be sent at `now`.
    pub fn ping_due(&self, now: Duration) -> bool {
        let elapsed = now.saturating_sub(self.last_send);
        elapsed >= self.connection_timeout
            || (!self.keepalive_interval.is_zero() && elapsed >= self.keepalive_interval / 2)
    }

    /// Records a PDU sent on the default IN channel.
    pub fn record_send(&mut self, now: Duration) {
        self.last_send = now;
    }
}

/// Encodes the client CONN/A1 RTS PDU for the OUT channel.
///
/// [MS-RPCH] 2.2.4.2.
pub fn encode_rts_conn_a1(
    virtual_connection_cookie: RtsCookie,
    out_channel_cookie: RtsCookie,
    receive_window_size: u32,
) -> Result<Vec<u8>, RpcPduError> {
    validate_rts_receive_window_size(receive_window_size)?;

    let mut commands = Vec::with_capacity(
        8 /* version */
            + 20 /* virtual connection cookie */
            + 20 /* OUT channel cookie */
            + 8, /* receive window size */
    );
    encode_rts_u32_command(&mut commands, RTS_COMMAND_VERSION, RTS_VERSION);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, virtual_connection_cookie);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, out_channel_cookie);
    encode_rts_u32_command(&mut commands, RTS_COMMAND_RECEIVE_WINDOW_SIZE, receive_window_size);
    encode_rts_pdu(RTS_FLAG_NONE, 4, commands)
}

/// Encodes the client CONN/B1 RTS PDU for the IN channel.
///
/// [MS-RPCH] 2.2.4.5.
pub fn encode_rts_conn_b1(
    virtual_connection_cookie: RtsCookie,
    in_channel_cookie: RtsCookie,
    channel_lifetime: u32,
    client_keepalive: u32,
    association_group_id: RtsCookie,
) -> Result<Vec<u8>, RpcPduError> {
    validate_rts_channel_lifetime(channel_lifetime)?;
    validate_rts_client_keepalive(client_keepalive)?;

    let mut commands = Vec::with_capacity(
        8 /* version */
            + 20 /* virtual connection cookie */
            + 20 /* IN channel cookie */
            + 8 /* channel lifetime */
            + 8 /* client keepalive */
            + 20, /* association group ID */
    );
    encode_rts_u32_command(&mut commands, RTS_COMMAND_VERSION, RTS_VERSION);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, virtual_connection_cookie);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, in_channel_cookie);
    encode_rts_u32_command(&mut commands, RTS_COMMAND_CHANNEL_LIFETIME, channel_lifetime);
    encode_rts_u32_command(&mut commands, RTS_COMMAND_CLIENT_KEEPALIVE, client_keepalive);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_ASSOCIATION_GROUP_ID, association_group_id);
    encode_rts_pdu(RTS_FLAG_NONE, 6, commands)
}

/// Encodes the client IN_R1/A1 RTS PDU for a successor IN channel.
///
/// [MS-RPCH] 2.2.4.10.
pub fn encode_rts_in_recycle_a1(
    virtual_connection_cookie: RtsCookie,
    predecessor_channel_cookie: RtsCookie,
    successor_channel_cookie: RtsCookie,
) -> Result<Vec<u8>, RpcPduError> {
    let mut commands = Vec::with_capacity(
        8 /* version */
            + 20 /* virtual connection cookie */
            + 20 /* predecessor IN channel cookie */
            + 20, /* successor IN channel cookie */
    );
    encode_rts_u32_command(&mut commands, RTS_COMMAND_VERSION, RTS_VERSION);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, virtual_connection_cookie);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, predecessor_channel_cookie);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, successor_channel_cookie);
    encode_rts_pdu(RTS_FLAG_RECYCLE_CHANNEL, 4, commands)
}

/// Encodes the client IN_R1/A5 RTS PDU for a predecessor IN channel.
///
/// [MS-RPCH] 2.2.4.14.
pub fn encode_rts_in_recycle_a5(successor_channel_cookie: RtsCookie) -> Result<Vec<u8>, RpcPduError> {
    let mut commands = Vec::with_capacity(20 /* successor IN channel cookie */);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, successor_channel_cookie);
    encode_rts_pdu(RTS_FLAG_NONE, 1, commands)
}

/// Decodes and validates an OUT_R1/A2 RTS PDU.
///
/// [MS-RPCH] 2.2.4.24.
pub fn decode_rts_out_recycle_a2(source: &[u8]) -> Result<(), RpcPduError> {
    let body = decode_rts_pdu(source, RTS_FLAG_RECYCLE_CHANNEL, 1, 8)?;
    let destination = decode_rts_u32_command(body, 0, RTS_COMMAND_DESTINATION)?;
    if destination != RTS_DESTINATION_FD_CLIENT {
        return Err(RpcPduError::UnexpectedRtsDestination {
            expected: RTS_DESTINATION_FD_CLIENT,
            actual: destination,
        });
    }
    Ok(())
}

/// Encodes the client OUT_R1/A3 RTS PDU for a successor OUT channel.
///
/// [MS-RPCH] 2.2.4.25.
pub fn encode_rts_out_recycle_a3(
    virtual_connection_cookie: RtsCookie,
    predecessor_channel_cookie: RtsCookie,
    successor_channel_cookie: RtsCookie,
    receive_window_size: u32,
) -> Result<Vec<u8>, RpcPduError> {
    validate_rts_receive_window_size(receive_window_size)?;

    let mut commands = Vec::with_capacity(
        8 /* version */
            + 20 /* virtual connection cookie */
            + 20 /* predecessor OUT channel cookie */
            + 20 /* successor OUT channel cookie */
            + 8, /* receive window size */
    );
    encode_rts_u32_command(&mut commands, RTS_COMMAND_VERSION, RTS_VERSION);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, virtual_connection_cookie);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, predecessor_channel_cookie);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, successor_channel_cookie);
    encode_rts_u32_command(&mut commands, RTS_COMMAND_RECEIVE_WINDOW_SIZE, receive_window_size);
    encode_rts_pdu(RTS_FLAG_RECYCLE_CHANNEL, 5, commands)
}

/// Decodes and validates an OUT_R1/A6 RTS PDU.
///
/// [MS-RPCH] 2.2.4.28.
pub fn decode_rts_out_recycle_a6(source: &[u8]) -> Result<RtsOutRecycleA6, RpcPduError> {
    let body = decode_rts_pdu(source, RTS_FLAG_OUT_CHANNEL, 3, 24)?;
    let destination = decode_rts_u32_command(body, 0, RTS_COMMAND_DESTINATION)?;
    if destination != RTS_DESTINATION_FD_CLIENT {
        return Err(RpcPduError::UnexpectedRtsDestination {
            expected: RTS_DESTINATION_FD_CLIENT,
            actual: destination,
        });
    }

    let version = decode_rts_u32_command(body, 8, RTS_COMMAND_VERSION)?;
    let connection_timeout = decode_rts_u32_command(body, 16, RTS_COMMAND_CONNECTION_TIMEOUT)?;
    validate_rts_connection_timeout(connection_timeout)?;
    Ok(RtsOutRecycleA6 {
        version,
        connection_timeout,
    })
}

/// Encodes the client OUT_R1/A7 RTS PDU for the default IN channel.
///
/// [MS-RPCH] 2.2.4.29.
pub fn encode_rts_out_recycle_a7(successor_channel_cookie: RtsCookie) -> Result<Vec<u8>, RpcPduError> {
    let mut commands = Vec::with_capacity(
        8 /* destination */
            + 20, /* successor OUT channel cookie */
    );
    encode_rts_u32_command(&mut commands, RTS_COMMAND_DESTINATION, RTS_DESTINATION_FD_SERVER);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, successor_channel_cookie);
    encode_rts_pdu(RTS_FLAG_OUT_CHANNEL, 2, commands)
}

/// Decodes and validates an OUT_R1/A10 RTS PDU.
///
/// [MS-RPCH] 2.2.4.32.
pub fn decode_rts_out_recycle_a10(source: &[u8]) -> Result<(), RpcPduError> {
    let body = decode_rts_pdu(source, RTS_FLAG_NONE, 1, 4)?;
    let command_type = read_u32(body, 0)?;
    if command_type != RTS_COMMAND_ANCE {
        return Err(RpcPduError::UnexpectedRtsCommandType {
            expected: RTS_COMMAND_ANCE,
            actual: command_type,
        });
    }
    Ok(())
}

/// Encodes the client OUT_R1/A11 RTS PDU for a successor OUT channel.
///
/// [MS-RPCH] 2.2.4.33.
pub fn encode_rts_out_recycle_a11() -> Result<Vec<u8>, RpcPduError> {
    encode_rts_pdu(RTS_FLAG_NONE, 1, RTS_COMMAND_ANCE.to_le_bytes().to_vec())
}

/// Encodes an RPCH PING RTS PDU.
///
/// [MS-RPCH] 2.2.4.49.
pub fn encode_rts_ping() -> Result<Vec<u8>, RpcPduError> {
    encode_rts_pdu(RTS_FLAG_PING, 0, Vec::new())
}

/// Encodes an RPCH flow-control acknowledgement.
///
/// [MS-RPCH] 2.2.4.50.
pub fn encode_rts_flow_control_ack(ack: RtsFlowControlAck) -> Result<Vec<u8>, RpcPduError> {
    let mut commands = Vec::with_capacity(
        4 /* command type */
            + 4 /* bytes received */
            + 4 /* available window */
            + RtsCookie::SIZE, /* channel cookie */
    );
    commands.extend_from_slice(&RTS_COMMAND_FLOW_CONTROL_ACK.to_le_bytes());
    commands.extend_from_slice(&ack.bytes_received.to_le_bytes());
    commands.extend_from_slice(&ack.available_window.to_le_bytes());
    commands.extend_from_slice(ack.channel_cookie.as_bytes());
    encode_rts_pdu(RTS_FLAG_OTHER_CMD, 1, commands)
}

/// Decodes an RPCH flow-control acknowledgement.
///
/// [MS-RPCH] 2.2.4.50.
pub fn decode_rts_flow_control_ack(source: &[u8]) -> Result<RtsFlowControlAck, RpcPduError> {
    let body = decode_rts_pdu(source, RTS_FLAG_OTHER_CMD, 1, 28)?;
    let command_type = read_u32(body, 0)?;
    if command_type != RTS_COMMAND_FLOW_CONTROL_ACK {
        return Err(RpcPduError::UnexpectedRtsCommandType {
            expected: RTS_COMMAND_FLOW_CONTROL_ACK,
            actual: command_type,
        });
    }

    let channel_cookie = body
        .get(12..12 + RtsCookie::SIZE)
        .ok_or(RpcPduError::Truncated {
            actual: body.len(),
            required: 12 + RtsCookie::SIZE,
        })?
        .try_into()
        .map(RtsCookie::new)
        .map_err(|_| RpcPduError::LengthOverflow)?;
    Ok(RtsFlowControlAck::new(
        read_u32(body, 4)?,
        read_u32(body, 8)?,
        channel_cookie,
    ))
}

fn decode_rts_conn_a3(source: &[u8]) -> Result<u32, RpcPduError> {
    let body = decode_rts_pdu(source, RTS_FLAG_NONE, 1, 8)?;
    let connection_timeout = decode_rts_u32_command(body, 0, RTS_COMMAND_CONNECTION_TIMEOUT)?;
    validate_rts_connection_timeout(connection_timeout)?;
    Ok(connection_timeout)
}

fn decode_rts_conn_c2(source: &[u8]) -> Result<(u32, u32), RpcPduError> {
    let body = decode_rts_pdu(source, RTS_FLAG_NONE, 3, 24)?;
    let _version = decode_rts_u32_command(body, 0, RTS_COMMAND_VERSION)?;
    let receive_window_size = decode_rts_u32_command(body, 8, RTS_COMMAND_RECEIVE_WINDOW_SIZE)?;
    let connection_timeout = decode_rts_u32_command(body, 16, RTS_COMMAND_CONNECTION_TIMEOUT)?;
    validate_rts_receive_window_size(receive_window_size)?;
    validate_rts_connection_timeout(connection_timeout)?;
    Ok((receive_window_size, connection_timeout))
}

/// Decodes and validates an IN_R1/A4 RTS PDU.
///
/// [MS-RPCH] 2.2.4.13.
pub fn decode_rts_in_recycle_a4(source: &[u8]) -> Result<RtsInRecycleA4, RpcPduError> {
    let body = decode_rts_pdu(source, RTS_FLAG_NONE, 4, 32)?;
    let destination = decode_rts_u32_command(body, 0, RTS_COMMAND_DESTINATION)?;
    if destination != RTS_DESTINATION_FD_CLIENT {
        return Err(RpcPduError::UnexpectedRtsDestination {
            expected: RTS_DESTINATION_FD_CLIENT,
            actual: destination,
        });
    }

    let version = decode_rts_u32_command(body, 8, RTS_COMMAND_VERSION)?;
    let receive_window_size = decode_rts_u32_command(body, 16, RTS_COMMAND_RECEIVE_WINDOW_SIZE)?;
    let connection_timeout = decode_rts_u32_command(body, 24, RTS_COMMAND_CONNECTION_TIMEOUT)?;
    validate_rts_receive_window_size(receive_window_size)?;
    validate_rts_connection_timeout(connection_timeout)?;
    Ok(RtsInRecycleA4 {
        version,
        receive_window_size,
        connection_timeout,
    })
}

fn encode_rts_pdu(flags: u16, command_count: u16, commands: Vec<u8>) -> Result<Vec<u8>, RpcPduError> {
    let body_length = 4usize.checked_add(commands.len()).ok_or(RpcPduError::LengthOverflow)?;
    let pdu_length = RTS_HEADER_SIZE
        .checked_add(commands.len())
        .ok_or(RpcPduError::LengthOverflow)?;
    let header = RpcCommonHeader::encode(PTYPE_RTS, RTS_PFC_FLAGS, 0, body_length, 0)?;
    let mut pdu = Vec::with_capacity(pdu_length);
    pdu.extend_from_slice(&header);
    pdu.extend_from_slice(&flags.to_le_bytes());
    pdu.extend_from_slice(&command_count.to_le_bytes());
    pdu.extend_from_slice(&commands);
    Ok(pdu)
}

fn encode_rts_u32_command(output: &mut Vec<u8>, command_type: u32, value: u32) {
    output.extend_from_slice(&command_type.to_le_bytes());
    output.extend_from_slice(&value.to_le_bytes());
}

fn encode_rts_cookie_command(output: &mut Vec<u8>, command_type: u32, cookie: RtsCookie) {
    output.extend_from_slice(&command_type.to_le_bytes());
    output.extend_from_slice(cookie.as_bytes());
}

fn decode_rts_pdu(
    source: &[u8],
    expected_flags: u16,
    expected_command_count: u16,
    expected_body_length: usize,
) -> Result<&[u8], RpcPduError> {
    let header = RpcCommonHeader::decode(source)?;
    if header.ptype != PTYPE_RTS {
        return Err(RpcPduError::UnexpectedPduType {
            expected: PTYPE_RTS,
            actual: header.ptype,
        });
    }
    if header.pfc_flags != RTS_PFC_FLAGS {
        return Err(RpcPduError::InvalidRtsPfcFlags {
            actual: header.pfc_flags,
        });
    }
    if header.auth_length != 0 {
        return Err(RpcPduError::AuthenticationUnsupported {
            auth_length: header.auth_length,
        });
    }
    if header.call_id != 0 {
        return Err(RpcPduError::UnexpectedRtsCallId { actual: header.call_id });
    }

    let pdu = &source[..usize::from(header.fragment_length)];
    let rts_header = pdu
        .get(RPC_COMMON_HEADER_SIZE..RTS_HEADER_SIZE)
        .ok_or(RpcPduError::Truncated {
            actual: pdu.len(),
            required: RTS_HEADER_SIZE,
        })?;
    let flags = read_u16(rts_header, 0)?;
    if flags != expected_flags {
        return Err(RpcPduError::UnexpectedRtsFlags {
            expected: expected_flags,
            actual: flags,
        });
    }
    let command_count = read_u16(rts_header, 2)?;
    if command_count != expected_command_count {
        return Err(RpcPduError::UnexpectedRtsCommandCount {
            expected: expected_command_count,
            actual: command_count,
        });
    }

    let body = &pdu[RTS_HEADER_SIZE..];
    if body.len() != expected_body_length {
        return Err(RpcPduError::InvalidRtsBodyLength {
            expected: expected_body_length,
            actual: body.len(),
        });
    }
    Ok(body)
}

fn decode_rts_u32_command(source: &[u8], offset: usize, expected_type: u32) -> Result<u32, RpcPduError> {
    let command_type = read_u32(source, offset)?;
    if command_type != expected_type {
        return Err(RpcPduError::UnexpectedRtsCommandType {
            expected: expected_type,
            actual: command_type,
        });
    }

    let value_offset = offset.checked_add(4).ok_or(RpcPduError::LengthOverflow)?;
    read_u32(source, value_offset)
}

fn validate_rts_receive_window_size(value: u32) -> Result<(), RpcPduError> {
    if !(RTS_MIN_RECEIVE_WINDOW_SIZE..=RTS_MAX_RECEIVE_WINDOW_SIZE).contains(&value) {
        return Err(RpcPduError::InvalidRtsReceiveWindowSize { actual: value });
    }
    Ok(())
}

fn validate_rts_connection_timeout(value: u32) -> Result<(), RpcPduError> {
    if !(RTS_MIN_CONNECTION_TIMEOUT..=RTS_MAX_CONNECTION_TIMEOUT).contains(&value) {
        return Err(RpcPduError::InvalidRtsConnectionTimeout { actual: value });
    }
    Ok(())
}

fn validate_rts_channel_lifetime(value: u32) -> Result<(), RpcPduError> {
    if !(RTS_MIN_CHANNEL_LIFETIME..=RTS_MAX_CHANNEL_LIFETIME).contains(&value) {
        return Err(RpcPduError::InvalidRtsChannelLifetime { actual: value });
    }
    Ok(())
}

fn validate_rts_client_keepalive(value: u32) -> Result<(), RpcPduError> {
    if value != 0 && value < RTS_MIN_CLIENT_KEEPALIVE {
        return Err(RpcPduError::InvalidRtsClientKeepalive { actual: value });
    }
    Ok(())
}

fn read_u16(source: &[u8], offset: usize) -> Result<u16, RpcPduError> {
    let end = offset.checked_add(2).ok_or(RpcPduError::LengthOverflow)?;
    let bytes = source.get(offset..end).ok_or(RpcPduError::Truncated {
        actual: source.len(),
        required: end,
    })?;
    Ok(u16::from_le_bytes(
        bytes.try_into().map_err(|_| RpcPduError::LengthOverflow)?,
    ))
}

fn read_u32(source: &[u8], offset: usize) -> Result<u32, RpcPduError> {
    let end = offset.checked_add(4).ok_or(RpcPduError::LengthOverflow)?;
    let bytes = source.get(offset..end).ok_or(RpcPduError::Truncated {
        actual: source.len(),
        required: end,
    })?;
    Ok(u32::from_le_bytes(
        bytes.try_into().map_err(|_| RpcPduError::LengthOverflow)?,
    ))
}
