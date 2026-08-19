//! DCE/RPC and raw RPC stub codecs for the MS-TSGU RPC-over-HTTP transport.
//!
//! This module implements the NDR32 control-operation stubs and raw data-plane
//! stubs required by the MS-TSGU RPC-over-HTTP transport.

use core::fmt;
use core::time::Duration;

use ironrdp_core::{Decode, Encode, ReadCursor, WriteCursor, ensure_fixed_part_size, ensure_size};
use sspi::{
    AuthIdentity, AuthIdentityBuffers, BufferType, ClientRequestFlags, CredentialUse, DataRepresentation,
    InitializeSecurityContextResult, Ntlm, SecurityBuffer, SecurityBufferRef, SecurityStatus, Sspi as _, SspiImpl as _,
    Username,
};
use uuid::Uuid;

use crate::{Error, GwErrorExt as _, GwErrorKind};

/// MS-TSGU RPC interface identifier.
///
/// [MS-TSGU] 1.9.1 / Appendix A.
pub(crate) const TSPROXY_RPC_INTERFACE_ID: Uuid = Uuid::from_u128(0x44e265dd_7daf_42cd_8560_3cdb6e7a2729);

/// `TsProxySetupReceivePipe` RPC operation number.
///
/// [MS-TSGU] 3.2.6 / Appendix A.
pub(crate) const TSPROXY_SETUP_RECEIVE_PIPE_OPNUM: u16 = 8;

/// `TsProxyCreateTunnel` RPC operation number.
///
/// [MS-TSGU] 3.2.6 / Appendix A.
pub(crate) const TSPROXY_CREATE_TUNNEL_OPNUM: u16 = 1;

/// `TsProxyAuthorizeTunnel` RPC operation number.
///
/// [MS-TSGU] 3.2.6 / Appendix A.
pub(crate) const TSPROXY_AUTHORIZE_TUNNEL_OPNUM: u16 = 2;

/// `TsProxyMakeTunnelCall` RPC operation number.
///
/// [MS-TSGU] 3.2.6 / Appendix A.
pub(crate) const TSPROXY_MAKE_TUNNEL_CALL_OPNUM: u16 = 3;

/// `TsProxyCreateChannel` RPC operation number.
///
/// [MS-TSGU] 3.2.6 / Appendix A.
pub(crate) const TSPROXY_CREATE_CHANNEL_OPNUM: u16 = 4;

/// `TsProxyCloseChannel` RPC operation number.
///
/// [MS-TSGU] 3.2.6 / Appendix A.
pub(crate) const TSPROXY_CLOSE_CHANNEL_OPNUM: u16 = 6;

/// `TsProxyCloseTunnel` RPC operation number.
///
/// [MS-TSGU] 3.2.6 / Appendix A.
pub(crate) const TSPROXY_CLOSE_TUNNEL_OPNUM: u16 = 7;

/// `TsProxySendToServer` RPC operation number.
///
/// [MS-TSGU] 3.2.6 / Appendix A.
pub(crate) const TSPROXY_SEND_TO_SERVER_OPNUM: u16 = 9;

/// Maximum raw stub size accepted by the `max_is(32767)` operation parameters.
const MAX_RPC_MESSAGE_SIZE: usize = 32_767;
const NDR_REFERENT_ID: u32 = 0x0002_0000;
const TSG_COMPONENT_ID: u16 = 0x5452;
const TSG_PACKET_TYPE_VERSIONCAPS: u32 = 0x0000_5643;
const TSG_PACKET_TYPE_QUARREQUEST: u32 = 0x0000_5152;
const TSG_PACKET_TYPE_RESPONSE: u32 = 0x0000_5052;
const TSG_PACKET_TYPE_QUARENC_RESPONSE: u32 = 0x0000_4552;
const TSG_PACKET_TYPE_CAPS_RESPONSE: u32 = 0x0000_4350;
const TSG_PACKET_TYPE_REAUTH: u32 = 0x0000_5250;
const TSG_PACKET_TYPE_MSGREQUEST: u32 = 0x0000_4752;
const TSG_PACKET_TYPE_MESSAGE: u32 = 0x0000_4750;
const TSG_TUNNEL_CALL_ASYNC_MSG_REQUEST: u32 = 1;
const TSG_TUNNEL_CANCEL_ASYNC_MSG_REQUEST: u32 = 2;
const TSG_ASYNC_MESSAGE_CONSENT: u32 = 1;
const TSG_ASYNC_MESSAGE_SERVICE: u32 = 2;
const TSG_ASYNC_MESSAGE_REAUTH: u32 = 3;
const TSG_CAPABILITY_TYPE_NAP: u32 = 1;
const TSG_MAX_CERT_CHAIN_LEN: usize = 24_000;
const TSG_MAX_RESPONSE_DATA_SIZE: usize = 24_000;
const TSG_MAX_MESSAGE_CHARS: usize = 65_536;

/// A TS Gateway RPC context handle in its 20-byte network representation.
///
/// The representation is shared by the serialize and no-serialize tunnel and
/// channel handle types. [MS-TSGU] 2.2.2.1 and 2.2.2.2 specify that the
/// no-serialize forms are identical on the wire.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct RpcContextHandle([u8; Self::SIZE]);

impl RpcContextHandle {
    /// Size of an RPC context handle network representation.
    pub(crate) const SIZE: usize = 20;
    const FIXED_PART_SIZE: usize = Self::SIZE;

    /// Creates a context handle from exactly one network representation.
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, RpcWireError> {
        let bytes: &[u8; Self::SIZE] = bytes
            .try_into()
            .map_err(|_| RpcWireError::ContextHandleLength { actual: bytes.len() })?;

        Ok(Self(*bytes))
    }

    /// Returns the handle network representation unchanged.
    pub(crate) const fn as_bytes(&self) -> &[u8; Self::SIZE] {
        &self.0
    }

    /// Returns `true` if this is the null RPC context handle.
    pub(crate) fn is_null(&self) -> bool {
        self.0.iter().all(|byte| *byte == 0)
    }

    /// Converts this handle to a non-null handle required by channel operations.
    pub(crate) fn require_non_null(self) -> Result<NonNullRpcContextHandle, RpcWireError> {
        if self.is_null() {
            return Err(RpcWireError::NullContextHandle);
        }

        Ok(NonNullRpcContextHandle(self))
    }
}

impl fmt::Debug for RpcContextHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RpcContextHandle(..)")
    }
}

impl Encode for RpcContextHandle {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> ironrdp_core::EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_slice(&self.0);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "RPC_CONTEXT_HANDLE"
    }

    fn size(&self) -> usize {
        Self::SIZE
    }
}

impl Decode<'_> for RpcContextHandle {
    fn decode(src: &mut ReadCursor<'_>) -> ironrdp_core::DecodeResult<Self> {
        ensure_fixed_part_size!(in: src);
        Ok(Self(src.read_array()))
    }
}

/// A non-null RPC context handle accepted by channel-related operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NonNullRpcContextHandle(RpcContextHandle);

impl NonNullRpcContextHandle {
    /// Returns the handle network representation unchanged.
    const fn as_bytes(&self) -> &[u8; RpcContextHandle::SIZE] {
        self.0.as_bytes()
    }
}

/// Byte order selected by the surrounding DCE/RPC PDU data representation.
///
/// The raw four-byte final receive-pipe stub does not identify its own byte
/// order, so callers must derive this from their DCE/RPC implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RpcStubByteOrder {
    LittleEndian,
    BigEndian,
}

/// Errors reported by the raw TS Gateway RPC stub codecs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RpcWireError {
    ContextHandleLength { actual: usize },
    NullContextHandle,
    EmptyResourceName,
    EmbeddedNulInResourceName,
    BufferCount { actual: usize },
    EmptyFirstBuffer,
    LengthOverflow,
    RequestTooLarge { actual: usize },
    OutputLength { actual: usize, expected: usize },
    ResponseLength { actual: usize, expected: usize },
    RpcStatus { value: u32 },
    FinalReturnValueLength { actual: usize },
    UnexpectedPacketId { expected: u32, actual: u32 },
    UnexpectedPacketSwitch { expected: u32, actual: u32 },
    RequiredNdrPointerIsNull,
    UnexpectedNdrPointer { actual: u32 },
    InvalidNdrBoolean { value: u32 },
    ConflictingRedirectionFlags,
    ResponseDataTooLarge { actual: usize },
    UnsupportedCapabilityCount { actual: u32 },
    UnexpectedCapabilityType { expected: u32, actual: u32 },
    InvalidNdrArrayLength { actual: u32, expected: u32 },
    InvalidQuarencFlags { actual: u32 },
    CertificateChainTooLarge { actual: usize },
    UnterminatedNdrString,
    InvalidMessageType { actual: u32 },
    MessageTooLarge { actual: usize },
}

impl fmt::Display for RpcWireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContextHandleLength { actual } => {
                write!(
                    f,
                    "invalid rpc context handle length {actual}, expected {}",
                    RpcContextHandle::SIZE
                )
            }
            Self::NullContextHandle => f.write_str("context handle must not be null"),
            Self::EmptyResourceName => f.write_str("resource name must not be empty"),
            Self::EmbeddedNulInResourceName => f.write_str("resource name must not contain a nul character"),
            Self::BufferCount { actual } => write!(f, "invalid buffer count {actual}, expected 1 through 3"),
            Self::EmptyFirstBuffer => f.write_str("first buffer must not be empty"),
            Self::LengthOverflow => f.write_str("rpc stub length overflow"),
            Self::RequestTooLarge { actual } => {
                write!(f, "rpc stub length {actual} exceeds {MAX_RPC_MESSAGE_SIZE}")
            }
            Self::OutputLength { actual, expected } => {
                write!(f, "invalid output length {actual}, expected {expected}")
            }
            Self::ResponseLength { actual, expected } => {
                write!(f, "invalid rpc response length {actual}, expected {expected}")
            }
            Self::RpcStatus { value } => write!(f, "rpc operation returned status 0x{value:08x}"),
            Self::FinalReturnValueLength { actual } => {
                write!(f, "invalid final receive-pipe stub length {actual}, expected 4")
            }
            Self::UnexpectedPacketId { expected, actual } => {
                write!(
                    f,
                    "unexpected TS Gateway packet id 0x{actual:08x}, expected 0x{expected:08x}"
                )
            }
            Self::UnexpectedPacketSwitch { expected, actual } => {
                write!(
                    f,
                    "unexpected TS Gateway packet switch 0x{actual:08x}, expected 0x{expected:08x}"
                )
            }
            Self::RequiredNdrPointerIsNull => f.write_str("required NDR pointer is null"),
            Self::UnexpectedNdrPointer { actual } => write!(f, "unexpected NDR pointer value 0x{actual:08x}"),
            Self::InvalidNdrBoolean { value } => write!(f, "invalid NDR boolean value {value}"),
            Self::ConflictingRedirectionFlags => f.write_str("enable-all and disable-all redirection flags conflict"),
            Self::ResponseDataTooLarge { actual } => {
                write!(
                    f,
                    "TS Gateway response data length {actual} exceeds {TSG_MAX_RESPONSE_DATA_SIZE}"
                )
            }
            Self::UnsupportedCapabilityCount { actual } => {
                write!(f, "unsupported TS Gateway capability count {actual}")
            }
            Self::UnexpectedCapabilityType { expected, actual } => {
                write!(f, "unexpected TS Gateway capability type {actual}, expected {expected}")
            }
            Self::InvalidNdrArrayLength { actual, expected } => {
                write!(f, "invalid NDR array length {actual}, expected {expected}")
            }
            Self::InvalidQuarencFlags { actual } => {
                write!(f, "invalid TS Gateway QUARENC flags {actual}")
            }
            Self::CertificateChainTooLarge { actual } => {
                write!(
                    f,
                    "TS Gateway certificate chain length {actual} exceeds {TSG_MAX_CERT_CHAIN_LEN}"
                )
            }
            Self::UnterminatedNdrString => f.write_str("unterminated NDR string"),
            Self::InvalidMessageType { actual } => write!(f, "invalid TS Gateway message type {actual}"),
            Self::MessageTooLarge { actual } => {
                write!(f, "TS Gateway message length {actual} exceeds {TSG_MAX_MESSAGE_CHARS}")
            }
        }
    }
}

impl core::error::Error for RpcWireError {}

/// NDR32 `TsProxyCreateTunnel` request stub.
///
/// The terminal-server gateway interface carries an additional 60-byte
/// interface-negotiation trailer after the documented packet. Existing Windows
/// gateways require this trailer despite it not being described by MS-TSGU.
///
/// [MS-TSGU] 3.2.6.1.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TsProxyCreateTunnelRequest {
    capabilities: u32,
}

impl TsProxyCreateTunnelRequest {
    pub(crate) const fn new(capabilities: u32) -> Self {
        Self { capabilities }
    }

    pub(crate) fn encode(self) -> Vec<u8> {
        let mut output = Vec::with_capacity(108);

        output.extend_from_slice(&TSG_PACKET_TYPE_VERSIONCAPS.to_le_bytes());
        output.extend_from_slice(&TSG_PACKET_TYPE_VERSIONCAPS.to_le_bytes());
        encode_ndr_pointer(&mut output, 0);

        output.extend_from_slice(&TSG_COMPONENT_ID.to_le_bytes());
        output.extend_from_slice(
            &u16::try_from(TSG_PACKET_TYPE_VERSIONCAPS)
                .expect("packet type fits in u16")
                .to_le_bytes(),
        );
        encode_ndr_pointer(&mut output, 1);
        output.extend_from_slice(&1u32.to_le_bytes()); // numCapabilities
        output.extend_from_slice(&1u16.to_le_bytes()); // majorVersion
        output.extend_from_slice(&1u16.to_le_bytes()); // minorVersion
        output.extend_from_slice(&0u16.to_le_bytes()); // quarantineCapabilities
        output.extend_from_slice(&0u16.to_le_bytes()); // NDR alignment
        output.extend_from_slice(&1u32.to_le_bytes()); // capability array max count
        output.extend_from_slice(&TSG_CAPABILITY_TYPE_NAP.to_le_bytes());
        output.extend_from_slice(&TSG_CAPABILITY_TYPE_NAP.to_le_bytes());
        output.extend_from_slice(&self.capabilities.to_le_bytes());

        output.extend_from_slice(&[0x8a, 0xe3, 0x13, 0x71, 0x02, 0xf4, 0x36, 0x71]);
        output.extend_from_slice(&0x0004_0001u32.to_le_bytes());
        output.extend_from_slice(&1u32.to_le_bytes());
        output.extend_from_slice(&[2, 0x40, 0x28, 0]);
        encode_syntax_identifier(&mut output, TSPROXY_RPC_INTERFACE_ID, TSPROXY_RPC_INTERFACE_VERSION);
        encode_syntax_identifier(&mut output, NDR32_TRANSFER_SYNTAX_ID, NDR32_TRANSFER_SYNTAX_VERSION);

        debug_assert_eq!(output.len(), 108);
        output
    }
}

/// NDR32 `TsProxyCreateTunnel` reauthentication request stub.
///
/// [MS-TSGU] 2.2.9.2.1.11 and 4.1.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TsProxyReauthenticateTunnelRequest {
    tunnel_context: u64,
    capabilities: u32,
}

impl TsProxyReauthenticateTunnelRequest {
    pub(crate) const fn new(tunnel_context: u64, capabilities: u32) -> Self {
        Self {
            tunnel_context,
            capabilities,
        }
    }

    pub(crate) fn encode(self) -> Vec<u8> {
        let mut output = Vec::with_capacity(64);

        output.extend_from_slice(&TSG_PACKET_TYPE_REAUTH.to_le_bytes());
        output.extend_from_slice(&TSG_PACKET_TYPE_REAUTH.to_le_bytes());
        encode_ndr_pointer(&mut output, 0);

        output.extend_from_slice(&self.tunnel_context.to_le_bytes());
        output.extend_from_slice(&TSG_PACKET_TYPE_VERSIONCAPS.to_le_bytes());
        encode_ndr_pointer(&mut output, 1);

        output.extend_from_slice(&TSG_COMPONENT_ID.to_le_bytes());
        output.extend_from_slice(
            &u16::try_from(TSG_PACKET_TYPE_VERSIONCAPS)
                .expect("packet type fits in u16")
                .to_le_bytes(),
        );
        encode_ndr_pointer(&mut output, 2);
        output.extend_from_slice(&1u32.to_le_bytes()); // numCapabilities
        output.extend_from_slice(&1u16.to_le_bytes()); // majorVersion
        output.extend_from_slice(&1u16.to_le_bytes()); // minorVersion
        output.extend_from_slice(&0u16.to_le_bytes()); // quarantineCapabilities
        output.extend_from_slice(&0u16.to_le_bytes()); // NDR alignment
        output.extend_from_slice(&1u32.to_le_bytes()); // capability array max count
        output.extend_from_slice(&TSG_CAPABILITY_TYPE_NAP.to_le_bytes());
        output.extend_from_slice(&TSG_CAPABILITY_TYPE_NAP.to_le_bytes());
        output.extend_from_slice(&self.capabilities.to_le_bytes());

        debug_assert_eq!(output.len(), 64);
        output
    }
}

/// NDR32 `TsProxyMakeTunnelCall` request for one queued server message.
///
/// [MS-TSGU] 2.2.9.2.1.8 and 3.2.6.1.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TsProxyMakeTunnelCallRequest {
    tunnel_context: NonNullRpcContextHandle,
    proc_id: u32,
}

impl TsProxyMakeTunnelCallRequest {
    pub(crate) const fn new(tunnel_context: NonNullRpcContextHandle) -> Self {
        Self {
            tunnel_context,
            proc_id: TSG_TUNNEL_CALL_ASYNC_MSG_REQUEST,
        }
    }

    /// Cancels an unreturned asynchronous message request during tunnel shutdown.
    ///
    /// [MS-TSGU] 3.2.6.1.3 and 3.2.6.3.2.
    pub(crate) const fn cancel_pending(tunnel_context: NonNullRpcContextHandle) -> Self {
        Self {
            tunnel_context,
            proc_id: TSG_TUNNEL_CANCEL_ASYNC_MSG_REQUEST,
        }
    }

    pub(crate) fn encode(self) -> [u8; 40] {
        let mut output = [0; 40];
        output[..RpcContextHandle::SIZE].copy_from_slice(self.tunnel_context.as_bytes());
        output[20..24].copy_from_slice(&self.proc_id.to_le_bytes());
        output[24..28].copy_from_slice(&TSG_PACKET_TYPE_MSGREQUEST.to_le_bytes());
        output[28..32].copy_from_slice(&TSG_PACKET_TYPE_MSGREQUEST.to_le_bytes());
        output[32..36].copy_from_slice(&NDR_REFERENT_ID.to_le_bytes());
        output[36..40].copy_from_slice(&1u32.to_le_bytes()); // maxMessagesPerBatch
        output
    }
}

/// NDR32 context-handle stub for `TsProxyCloseChannel` and `TsProxyCloseTunnel`.
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
        *self.context.as_bytes()
    }
}

/// Validates the final context handle and HRESULT returned by a close operation.
pub(crate) fn decode_tsgu_close_context_response(source: &[u8]) -> Result<(), RpcWireError> {
    const RESPONSE_SIZE: usize = RpcContextHandle::SIZE + 4 /* return value */;
    if source.len() != RESPONSE_SIZE {
        return Err(RpcWireError::ResponseLength {
            actual: source.len(),
            expected: RESPONSE_SIZE,
        });
    }
    let return_value = read_u32(source, RpcContextHandle::SIZE).map_err(|_| RpcWireError::LengthOverflow)?;
    if return_value != 0 {
        return Err(RpcWireError::RpcStatus { value: return_value });
    }
    Ok(())
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

    pub(crate) fn encode(&self) -> Result<Vec<u8>, RpcWireError> {
        let resource_name = encode_ndr_string(self.resource_name)?;
        let mut output = Vec::with_capacity(48 + 2 * resource_name.len());

        output.extend_from_slice(self.tunnel_context.as_bytes());
        encode_ndr_pointer(&mut output, 0); // resourceName
        output.extend_from_slice(&1u32.to_le_bytes()); // numResourceNames
        output.extend_from_slice(&0u32.to_le_bytes()); // alternateResourceNames
        output.extend_from_slice(&0u16.to_le_bytes()); // numAlternateResourceNames
        output.extend_from_slice(&0u16.to_le_bytes()); // NDR alignment
        output.extend_from_slice(&((u32::from(self.port) << 16) | 3).to_le_bytes());

        output.extend_from_slice(&1u32.to_le_bytes()); // resourceName array max count
        encode_ndr_pointer(&mut output, 1); // first resourceName
        encode_ndr_string_referent(&mut output, &resource_name)?;

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

    pub(crate) fn encode(&self) -> Result<Vec<u8>, RpcWireError> {
        let machine_name = encode_ndr_string(self.machine_name)?;
        let machine_name_len = u32::try_from(machine_name.len()).map_err(|_| RpcWireError::LengthOverflow)?;
        let statement_of_health_len =
            u32::try_from(self.statement_of_health.len()).map_err(|_| RpcWireError::LengthOverflow)?;
        let mut output = Vec::with_capacity(64 + 2 * machine_name.len() + self.statement_of_health.len());

        output.extend_from_slice(self.tunnel_context.as_bytes());
        output.extend_from_slice(&TSG_PACKET_TYPE_QUARREQUEST.to_le_bytes());
        output.extend_from_slice(&TSG_PACKET_TYPE_QUARREQUEST.to_le_bytes());
        encode_ndr_pointer(&mut output, 0); // packetQuarRequest
        output.extend_from_slice(&0u32.to_le_bytes()); // flags
        encode_ndr_pointer(&mut output, 1); // machineName
        output.extend_from_slice(&machine_name_len.to_le_bytes()); // nameLength
        if self.statement_of_health.is_empty() {
            output.extend_from_slice(&0u32.to_le_bytes());
        } else {
            encode_ndr_pointer(&mut output, 2); // data
        }
        output.extend_from_slice(&statement_of_health_len.to_le_bytes()); // dataLen
        encode_ndr_string_referent(&mut output, &machine_name)?;
        if !self.statement_of_health.is_empty() {
            output.extend_from_slice(&statement_of_health_len.to_le_bytes()); // max count
            output.extend_from_slice(self.statement_of_health);
            pad_ndr_4(&mut output);
        }

        Ok(output)
    }
}

/// Decoded `TsProxyCreateChannel` response values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TsProxyCreateChannelResponse {
    pub(crate) channel_context: NonNullRpcContextHandle,
    pub(crate) channel_id: u32,
}

/// Decoded `TsProxyCreateTunnel` response values.
///
/// [MS-TSGU] 2.2.9.2.1.6 and 3.2.6.1.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TsProxyCreateTunnelResponse {
    pub(crate) tunnel_context: NonNullRpcContextHandle,
    pub(crate) tunnel_id: u32,
    pub(crate) nonce: Uuid,
    pub(crate) capabilities: u32,
}

/// Decodes one non-messaging `TsProxyCreateTunnel` response stub.
///
/// A gateway returns `TSG_PACKET_TYPE_QUARENC_RESPONSE` unless the client and
/// server negotiated consent-signing. The latter has additional NDR structures
/// and is decoded separately once legacy RPC message handling is available.
pub(crate) fn decode_tsgu_create_tunnel_response(source: &[u8]) -> Result<TsProxyCreateTunnelResponse, RpcWireError> {
    const PACKET_HEADER_SIZE: usize = 4 /* TSG packet pointer */
        + 4 /* packet ID */
        + 4 /* union switch */
        + 4; /* QUARENC response pointer */
    const QUARENC_FIXED_SIZE: usize = 4 /* flags */
        + 4 /* certificate chain length */
        + 4 /* certificate chain pointer */
        + 16 /* nonce */
        + 4; /* version capabilities pointer */
    let fixed_size = PACKET_HEADER_SIZE
        .checked_add(QUARENC_FIXED_SIZE)
        .ok_or(RpcWireError::LengthOverflow)?;
    let fixed = source.get(..fixed_size).ok_or(RpcWireError::ResponseLength {
        actual: source.len(),
        expected: fixed_size,
    })?;
    if read_u32(fixed, 0).map_err(|_| RpcWireError::LengthOverflow)? == 0
        || read_u32(fixed, 12).map_err(|_| RpcWireError::LengthOverflow)? == 0
    {
        return Err(RpcWireError::RequiredNdrPointerIsNull);
    }

    // The server returns TSG_PACKET_TYPE_CAPS_RESPONSE when the negotiated capabilities
    // include consent signing, embedding the quarantine response followed by a consent
    // message; otherwise it returns TSG_PACKET_TYPE_QUARENC_RESPONSE. Both share the same
    // quarantine-response prefix, so accept either ([MS-TSGU] 3.2.6.1.1).
    let packet_id = read_u32(fixed, 4).map_err(|_| RpcWireError::LengthOverflow)?;
    if packet_id != TSG_PACKET_TYPE_QUARENC_RESPONSE && packet_id != TSG_PACKET_TYPE_CAPS_RESPONSE {
        return Err(RpcWireError::UnexpectedPacketId {
            expected: TSG_PACKET_TYPE_QUARENC_RESPONSE,
            actual: packet_id,
        });
    }
    let packet_switch = read_u32(fixed, 8).map_err(|_| RpcWireError::LengthOverflow)?;
    if packet_switch != packet_id {
        return Err(RpcWireError::UnexpectedPacketSwitch {
            expected: packet_id,
            actual: packet_switch,
        });
    }
    let flags = read_u32(fixed, PACKET_HEADER_SIZE).map_err(|_| RpcWireError::LengthOverflow)?;
    if flags != 0 {
        return Err(RpcWireError::InvalidQuarencFlags { actual: flags });
    }

    let certificate_chain_length =
        usize::try_from(read_u32(fixed, PACKET_HEADER_SIZE + 4).map_err(|_| RpcWireError::LengthOverflow)?)
            .map_err(|_| RpcWireError::LengthOverflow)?;
    if certificate_chain_length > TSG_MAX_CERT_CHAIN_LEN {
        return Err(RpcWireError::CertificateChainTooLarge {
            actual: certificate_chain_length,
        });
    }
    let certificate_chain_pointer =
        read_u32(fixed, PACKET_HEADER_SIZE + 8).map_err(|_| RpcWireError::LengthOverflow)?;
    if certificate_chain_length != 0 && certificate_chain_pointer == 0 {
        return Err(RpcWireError::RequiredNdrPointerIsNull);
    }
    let nonce = Uuid::from_bytes_le(
        fixed[PACKET_HEADER_SIZE + 12..PACKET_HEADER_SIZE + 28]
            .try_into()
            .map_err(|_| RpcWireError::LengthOverflow)?,
    );
    if read_u32(fixed, PACKET_HEADER_SIZE + 28).map_err(|_| RpcWireError::LengthOverflow)? == 0 {
        return Err(RpcWireError::RequiredNdrPointerIsNull);
    }

    let mut offset = fixed_size;
    if certificate_chain_pointer != 0 {
        let array_header_end = offset.checked_add(12).ok_or(RpcWireError::LengthOverflow)?;
        let array_header = source
            .get(offset..array_header_end)
            .ok_or(RpcWireError::ResponseLength {
                actual: source.len(),
                expected: array_header_end,
            })?;
        let max_count = read_u32(array_header, 0).map_err(|_| RpcWireError::LengthOverflow)?;
        let offset_count = read_u32(array_header, 4).map_err(|_| RpcWireError::LengthOverflow)?;
        let actual_count = read_u32(array_header, 8).map_err(|_| RpcWireError::LengthOverflow)?;
        let expected_count = u32::try_from(certificate_chain_length).map_err(|_| RpcWireError::LengthOverflow)?;
        if max_count != expected_count || offset_count != 0 || actual_count != expected_count {
            return Err(RpcWireError::InvalidNdrArrayLength {
                actual: actual_count,
                expected: expected_count,
            });
        }
        let certificate_bytes = certificate_chain_length
            .checked_mul(2)
            .ok_or(RpcWireError::LengthOverflow)?;
        let certificate_end = array_header_end
            .checked_add(certificate_bytes)
            .ok_or(RpcWireError::LengthOverflow)?;
        let certificate = source
            .get(array_header_end..certificate_end)
            .ok_or(RpcWireError::ResponseLength {
                actual: source.len(),
                expected: certificate_end,
            })?;
        if certificate_chain_length != 0 && certificate[certificate.len() - 2..] != [0, 0] {
            return Err(RpcWireError::UnterminatedNdrString);
        }
        offset = (certificate_end + 3) & !3;
    }

    // Locate the version capabilities structure by its component ID and packet type
    // signature. A capabilities (consent-signing) response inserts the consent message's
    // fixed fields between the quarantine fixed part and this pointee, so its offset is
    // not fixed ([MS-TSGU] 2.2.9.2.1.7). Scanning on the unambiguous signature is robust
    // to that insertion.
    let version_caps_id = u16::try_from(TSG_PACKET_TYPE_VERSIONCAPS).expect("packet type fits in u16");
    let version_caps_signature = [
        TSG_COMPONENT_ID.to_le_bytes()[0],
        TSG_COMPONENT_ID.to_le_bytes()[1],
        version_caps_id.to_le_bytes()[0],
        version_caps_id.to_le_bytes()[1],
    ];
    let version_caps_offset = (offset..source.len().saturating_sub(3))
        .step_by(4)
        .find(|&candidate| source[candidate..candidate + 4] == version_caps_signature)
        .ok_or(RpcWireError::UnexpectedPacketId {
            expected: TSG_PACKET_TYPE_VERSIONCAPS,
            actual: 0,
        })?;

    let version_caps_end = version_caps_offset
        .checked_add(24)
        .ok_or(RpcWireError::LengthOverflow)?;
    let version_caps = source
        .get(version_caps_offset..version_caps_end)
        .ok_or(RpcWireError::ResponseLength {
            actual: source.len(),
            expected: version_caps_end,
        })?;
    if read_u16(version_caps, 0).map_err(|_| RpcWireError::LengthOverflow)? != TSG_COMPONENT_ID
        || read_u16(version_caps, 2).map_err(|_| RpcWireError::LengthOverflow)? != version_caps_id
        || read_u32(version_caps, 4).map_err(|_| RpcWireError::LengthOverflow)? == 0
    {
        return Err(RpcWireError::UnexpectedPacketId {
            expected: TSG_PACKET_TYPE_VERSIONCAPS,
            actual: u32::from(read_u16(version_caps, 2).map_err(|_| RpcWireError::LengthOverflow)?),
        });
    }
    let capability_count = read_u32(version_caps, 8).map_err(|_| RpcWireError::LengthOverflow)?;
    if capability_count != 1 {
        return Err(RpcWireError::UnsupportedCapabilityCount {
            actual: capability_count,
        });
    }
    let array_count = read_u32(version_caps, 20).map_err(|_| RpcWireError::LengthOverflow)?;
    if array_count != capability_count {
        return Err(RpcWireError::InvalidNdrArrayLength {
            actual: array_count,
            expected: capability_count,
        });
    }

    let capability_end = version_caps_end.checked_add(12).ok_or(RpcWireError::LengthOverflow)?;
    let capability = source
        .get(version_caps_end..capability_end)
        .ok_or(RpcWireError::ResponseLength {
            actual: source.len(),
            expected: capability_end,
        })?;
    let capability_type = read_u32(capability, 0).map_err(|_| RpcWireError::LengthOverflow)?;
    let capability_switch = read_u32(capability, 4).map_err(|_| RpcWireError::LengthOverflow)?;
    if capability_type != TSG_CAPABILITY_TYPE_NAP {
        return Err(RpcWireError::UnexpectedCapabilityType {
            expected: TSG_CAPABILITY_TYPE_NAP,
            actual: capability_type,
        });
    }
    if capability_switch != capability_type {
        return Err(RpcWireError::UnexpectedCapabilityType {
            expected: capability_type,
            actual: capability_switch,
        });
    }

    // Both the quarantine and capabilities responses end with the tunnel context handle,
    // the tunnel ID, and the HRESULT return value. A capabilities response inserts a
    // consent message between the capability array and these trailing fields, so read
    // them from the end of the stub ([MS-TSGU] 2.2.9.2.1.6, 2.2.9.2.1.7).
    const TRAILER_SIZE: usize = RpcContextHandle::SIZE + 4 /* tunnel id */ + 4 /* return value */;
    let context_offset = source
        .len()
        .checked_sub(TRAILER_SIZE)
        .filter(|offset| *offset >= capability_end)
        .ok_or(RpcWireError::ResponseLength {
            actual: source.len(),
            expected: capability_end + TRAILER_SIZE,
        })?;
    let context_end = context_offset
        .checked_add(RpcContextHandle::SIZE)
        .ok_or(RpcWireError::LengthOverflow)?;
    let tunnel_context = RpcContextHandle::from_bytes(&source[context_offset..context_end])?.require_non_null()?;
    let tunnel_id = read_u32(source, context_end).map_err(|_| RpcWireError::LengthOverflow)?;
    let return_value = read_u32(source, context_end + 4).map_err(|_| RpcWireError::LengthOverflow)?;
    if return_value != 0 {
        return Err(RpcWireError::RpcStatus { value: return_value });
    }

    Ok(TsProxyCreateTunnelResponse {
        tunnel_context,
        tunnel_id,
        nonce,
        capabilities: read_u32(capability, 8).map_err(|_| RpcWireError::LengthOverflow)?,
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
///
/// [MS-TSGU] 2.2.9.2.1.5 and 3.2.6.1.2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TsProxyAuthorizeTunnelResponse {
    pub(crate) response_data: Vec<u8>,
    pub(crate) redirection_flags: TsProxyRedirectionFlags,
}

/// Decodes one `TsProxyAuthorizeTunnel` response stub.
///
/// The opaque response data may contain a statement-of-health response and an
/// idle timeout, depending on capabilities negotiated by `TsProxyCreateTunnel`.
pub(crate) fn decode_tsgu_authorize_tunnel_response(
    source: &[u8],
) -> Result<TsProxyAuthorizeTunnelResponse, RpcWireError> {
    const FIXED_SIZE: usize = 4 /* TSG packet pointer */
        + 4 /* packet ID */
        + 4 /* union switch */
        + 4 /* packet response pointer */
        + 4 /* flags */
        + 4 /* reserved */
        + 4 /* response data pointer */
        + 4 /* response data length */
        + 8 * 4; /* redirection flags */
    let fixed = source.get(..FIXED_SIZE).ok_or(RpcWireError::ResponseLength {
        actual: source.len(),
        expected: FIXED_SIZE,
    })?;
    if read_u32(fixed, 0).map_err(|_| RpcWireError::LengthOverflow)? == 0
        || read_u32(fixed, 12).map_err(|_| RpcWireError::LengthOverflow)? == 0
    {
        return Err(RpcWireError::RequiredNdrPointerIsNull);
    }

    let packet_id = read_u32(fixed, 4).map_err(|_| RpcWireError::LengthOverflow)?;
    if packet_id != TSG_PACKET_TYPE_RESPONSE {
        return Err(RpcWireError::UnexpectedPacketId {
            expected: TSG_PACKET_TYPE_RESPONSE,
            actual: packet_id,
        });
    }
    let packet_switch = read_u32(fixed, 8).map_err(|_| RpcWireError::LengthOverflow)?;
    if packet_switch != TSG_PACKET_TYPE_RESPONSE {
        return Err(RpcWireError::UnexpectedPacketSwitch {
            expected: TSG_PACKET_TYPE_RESPONSE,
            actual: packet_switch,
        });
    }
    let flags = read_u32(fixed, 16).map_err(|_| RpcWireError::LengthOverflow)?;
    if flags != TSG_PACKET_TYPE_QUARREQUEST {
        return Err(RpcWireError::UnexpectedPacketId {
            expected: TSG_PACKET_TYPE_QUARREQUEST,
            actual: flags,
        });
    }

    let response_data_pointer = read_u32(fixed, 24).map_err(|_| RpcWireError::LengthOverflow)?;
    let response_data_length = usize::try_from(read_u32(fixed, 28).map_err(|_| RpcWireError::LengthOverflow)?)
        .map_err(|_| RpcWireError::LengthOverflow)?;
    if response_data_length > TSG_MAX_RESPONSE_DATA_SIZE {
        return Err(RpcWireError::ResponseDataTooLarge {
            actual: response_data_length,
        });
    }
    if response_data_length != 0 && response_data_pointer == 0 {
        return Err(RpcWireError::RequiredNdrPointerIsNull);
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
    if redirection_flags.enable_all && redirection_flags.disable_all {
        return Err(RpcWireError::ConflictingRedirectionFlags);
    }

    let response_data = if response_data_pointer == 0 {
        if source.len() != FIXED_SIZE + 4 {
            return Err(RpcWireError::ResponseLength {
                actual: source.len(),
                expected: FIXED_SIZE + 4,
            });
        }
        Vec::new()
    } else {
        let data_header_end = FIXED_SIZE.checked_add(4).ok_or(RpcWireError::LengthOverflow)?;
        let data_end = data_header_end
            .checked_add(response_data_length)
            .ok_or(RpcWireError::LengthOverflow)?;
        let padded_data_end = (data_end + 3) & !3;
        let return_value_end = padded_data_end.checked_add(4).ok_or(RpcWireError::LengthOverflow)?;
        if source.len() != return_value_end {
            return Err(RpcWireError::ResponseLength {
                actual: source.len(),
                expected: return_value_end,
            });
        }
        let max_count = usize::try_from(read_u32(source, FIXED_SIZE).map_err(|_| RpcWireError::LengthOverflow)?)
            .map_err(|_| RpcWireError::LengthOverflow)?;
        if max_count != response_data_length {
            return Err(RpcWireError::ResponseLength {
                actual: max_count,
                expected: response_data_length,
            });
        }
        source[data_header_end..data_end].to_vec()
    };

    let return_value_offset = source.len().checked_sub(4).ok_or(RpcWireError::LengthOverflow)?;
    let return_value = read_u32(source, return_value_offset).map_err(|_| RpcWireError::LengthOverflow)?;
    if return_value != 0 {
        return Err(RpcWireError::RpcStatus { value: return_value });
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
pub(crate) fn decode_tsgu_make_tunnel_call_response(source: &[u8]) -> Result<TsProxyTunnelMessage, RpcWireError> {
    const FIXED_SIZE: usize = 4 /* TSG packet pointer */
        + 4 /* packet ID */
        + 4 /* union switch */
        + 4 /* message response pointer */
        + 4 /* message ID */
        + 4 /* message type */
        + 4 /* is message present */
        + 4; /* message union pointer */
    let fixed = source.get(..FIXED_SIZE).ok_or(RpcWireError::ResponseLength {
        actual: source.len(),
        expected: FIXED_SIZE,
    })?;
    if read_u32(fixed, 0).map_err(|_| RpcWireError::LengthOverflow)? == 0
        || read_u32(fixed, 12).map_err(|_| RpcWireError::LengthOverflow)? == 0
    {
        return Err(RpcWireError::RequiredNdrPointerIsNull);
    }

    let packet_id = read_u32(fixed, 4).map_err(|_| RpcWireError::LengthOverflow)?;
    if packet_id != TSG_PACKET_TYPE_MESSAGE {
        return Err(RpcWireError::UnexpectedPacketId {
            expected: TSG_PACKET_TYPE_MESSAGE,
            actual: packet_id,
        });
    }
    let packet_switch = read_u32(fixed, 8).map_err(|_| RpcWireError::LengthOverflow)?;
    if packet_switch != TSG_PACKET_TYPE_MESSAGE {
        return Err(RpcWireError::UnexpectedPacketSwitch {
            expected: TSG_PACKET_TYPE_MESSAGE,
            actual: packet_switch,
        });
    }

    let message_type = read_u32(fixed, 20).map_err(|_| RpcWireError::LengthOverflow)?;
    let message_present = decode_ndr_boolean(fixed, 24)?;
    let message_pointer = read_u32(fixed, 28).map_err(|_| RpcWireError::LengthOverflow)?;
    if !message_present {
        if source.len() != FIXED_SIZE + 4 {
            return Err(RpcWireError::ResponseLength {
                actual: source.len(),
                expected: FIXED_SIZE + 4,
            });
        }
        let return_value = read_u32(source, FIXED_SIZE).map_err(|_| RpcWireError::LengthOverflow)?;
        if return_value != 0 {
            return Err(RpcWireError::RpcStatus { value: return_value });
        }
        return Ok(TsProxyTunnelMessage::None);
    }
    if message_pointer == 0 {
        return Err(RpcWireError::RequiredNdrPointerIsNull);
    }

    match message_type {
        TSG_ASYNC_MESSAGE_REAUTH => {
            let response_end = FIXED_SIZE.checked_add(12).ok_or(RpcWireError::LengthOverflow)?;
            if source.len() != response_end {
                return Err(RpcWireError::ResponseLength {
                    actual: source.len(),
                    expected: response_end,
                });
            }
            let tunnel_context = u64::from_le_bytes(
                source[FIXED_SIZE..FIXED_SIZE + 8]
                    .try_into()
                    .map_err(|_| RpcWireError::LengthOverflow)?,
            );
            let return_value = read_u32(source, FIXED_SIZE + 8).map_err(|_| RpcWireError::LengthOverflow)?;
            if return_value != 0 {
                return Err(RpcWireError::RpcStatus { value: return_value });
            }
            Ok(TsProxyTunnelMessage::Reauthenticate { tunnel_context })
        }
        TSG_ASYNC_MESSAGE_CONSENT | TSG_ASYNC_MESSAGE_SERVICE => {
            let string_fixed_end = FIXED_SIZE.checked_add(16).ok_or(RpcWireError::LengthOverflow)?;
            let string_fixed = source
                .get(FIXED_SIZE..string_fixed_end)
                .ok_or(RpcWireError::ResponseLength {
                    actual: source.len(),
                    expected: string_fixed_end,
                })?;
            let display_mandatory = decode_ndr_boolean(string_fixed, 0)?;
            let consent_mandatory = decode_ndr_boolean(string_fixed, 4)?;
            let message_chars = usize::try_from(read_u32(string_fixed, 8).map_err(|_| RpcWireError::LengthOverflow)?)
                .map_err(|_| RpcWireError::LengthOverflow)?;
            if message_chars > TSG_MAX_MESSAGE_CHARS {
                return Err(RpcWireError::MessageTooLarge { actual: message_chars });
            }
            let message_buffer_pointer = read_u32(string_fixed, 12).map_err(|_| RpcWireError::LengthOverflow)?;
            if message_chars != 0 && message_buffer_pointer == 0 {
                return Err(RpcWireError::RequiredNdrPointerIsNull);
            }

            let (message_start, message_end) = if message_buffer_pointer == 0 {
                (string_fixed_end, string_fixed_end)
            } else {
                let array_header_end = string_fixed_end.checked_add(4).ok_or(RpcWireError::LengthOverflow)?;
                let array_header =
                    source
                        .get(string_fixed_end..array_header_end)
                        .ok_or(RpcWireError::ResponseLength {
                            actual: source.len(),
                            expected: array_header_end,
                        })?;
                let array_count = usize::try_from(read_u32(array_header, 0).map_err(|_| RpcWireError::LengthOverflow)?)
                    .map_err(|_| RpcWireError::LengthOverflow)?;
                if array_count != message_chars {
                    return Err(RpcWireError::InvalidNdrArrayLength {
                        actual: u32::try_from(array_count).map_err(|_| RpcWireError::LengthOverflow)?,
                        expected: u32::try_from(message_chars).map_err(|_| RpcWireError::LengthOverflow)?,
                    });
                }
                let message_bytes = message_chars.checked_mul(2).ok_or(RpcWireError::LengthOverflow)?;
                let message_end = array_header_end
                    .checked_add(message_bytes)
                    .ok_or(RpcWireError::LengthOverflow)?;
                (array_header_end, message_end)
            };
            let return_value_end = message_end.checked_add(4).ok_or(RpcWireError::LengthOverflow)?;
            if source.len() != return_value_end {
                return Err(RpcWireError::ResponseLength {
                    actual: source.len(),
                    expected: return_value_end,
                });
            }
            let message_bytes = &source[message_start..message_end];
            if message_chars != 0 && message_bytes[message_bytes.len() - 2..] != [0, 0] {
                return Err(RpcWireError::UnterminatedNdrString);
            }
            let message_units = message_bytes
                .chunks_exact(2)
                .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
                .collect::<Vec<_>>();
            let text = String::from_utf16(&message_units[..message_units.len().saturating_sub(1)])
                .map_err(|_| RpcWireError::UnterminatedNdrString)?;
            let return_value = read_u32(source, message_end).map_err(|_| RpcWireError::LengthOverflow)?;
            if return_value != 0 {
                return Err(RpcWireError::RpcStatus { value: return_value });
            }

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
        actual => Err(RpcWireError::InvalidMessageType { actual }),
    }
}

/// Validates the null packet response and HRESULT from a cancelled message request.
///
/// [MS-TSGU] 3.2.6.1.3.
pub(crate) fn decode_tsgu_cancel_tunnel_call_response(source: &[u8]) -> Result<(), RpcWireError> {
    const RESPONSE_SIZE: usize = 4 /* null TSGPacketResponse pointer */ + 4 /* return value */;
    if source.len() != RESPONSE_SIZE {
        return Err(RpcWireError::ResponseLength {
            actual: source.len(),
            expected: RESPONSE_SIZE,
        });
    }
    let packet_response_pointer = read_u32(source, 0).map_err(|_| RpcWireError::LengthOverflow)?;
    if packet_response_pointer != 0 {
        return Err(RpcWireError::UnexpectedNdrPointer {
            actual: packet_response_pointer,
        });
    }
    let return_value = read_u32(source, 4).map_err(|_| RpcWireError::LengthOverflow)?;
    if return_value != 0 {
        return Err(RpcWireError::RpcStatus { value: return_value });
    }
    Ok(())
}

/// Decodes one `TsProxyCreateChannel` response stub.
///
/// [MS-TSGU] 3.2.6.1.4.
pub(crate) fn decode_tsgu_create_channel_response(source: &[u8]) -> Result<TsProxyCreateChannelResponse, RpcWireError> {
    const RESPONSE_SIZE: usize = RpcContextHandle::SIZE + 4 /* channelId */ + 4 /* return value */;
    if source.len() != RESPONSE_SIZE {
        return Err(RpcWireError::ResponseLength {
            actual: source.len(),
            expected: RESPONSE_SIZE,
        });
    }

    let channel_context = RpcContextHandle::from_bytes(&source[..RpcContextHandle::SIZE])?.require_non_null()?;
    let channel_id = u32::from_le_bytes(source[20..24].try_into().expect("fixed-size response slice"));
    let return_value = u32::from_le_bytes(source[24..28].try_into().expect("fixed-size response slice"));
    if return_value != 0 {
        return Err(RpcWireError::RpcStatus { value: return_value });
    }

    Ok(TsProxyCreateChannelResponse {
        channel_context,
        channel_id,
    })
}

fn encode_ndr_pointer(output: &mut Vec<u8>, index: u32) {
    output.extend_from_slice(&(NDR_REFERENT_ID + index * 4).to_le_bytes());
}

fn decode_ndr_boolean(source: &[u8], offset: usize) -> Result<bool, RpcWireError> {
    match read_u32(source, offset).map_err(|_| RpcWireError::LengthOverflow)? {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(RpcWireError::InvalidNdrBoolean { value }),
    }
}

fn encode_ndr_string(value: &str) -> Result<Vec<u16>, RpcWireError> {
    if value.is_empty() {
        return Err(RpcWireError::EmptyResourceName);
    }
    if value.contains('\0') {
        return Err(RpcWireError::EmbeddedNulInResourceName);
    }

    let mut encoded: Vec<_> = value.encode_utf16().collect();
    encoded.push(0);
    Ok(encoded)
}

fn encode_ndr_string_referent(output: &mut Vec<u8>, value: &[u16]) -> Result<(), RpcWireError> {
    let length = u32::try_from(value.len()).map_err(|_| RpcWireError::LengthOverflow)?;
    output.extend_from_slice(&length.to_le_bytes()); // max count
    output.extend_from_slice(&0u32.to_le_bytes()); // offset
    output.extend_from_slice(&length.to_le_bytes()); // actual count
    for character in value {
        output.extend_from_slice(&character.to_le_bytes());
    }
    pad_ndr_4(output);

    Ok(())
}

fn pad_ndr_4(output: &mut Vec<u8>) {
    let padding = (4 - output.len() % 4) % 4;
    output.resize(output.len() + padding, 0);
}

/// Raw `TsProxySetupReceivePipe` request stub.
///
/// [MS-TSGU] 2.2.9.4.1 and 3.6.5.3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TsProxySetupReceivePipeRequest {
    channel_context: NonNullRpcContextHandle,
}

impl TsProxySetupReceivePipeRequest {
    /// Size of the complete raw request stub.
    pub(crate) const SIZE: usize = RpcContextHandle::SIZE;

    pub(crate) const fn new(channel_context: NonNullRpcContextHandle) -> Self {
        Self { channel_context }
    }

    /// Encodes the request into an output slice of exactly 20 bytes.
    pub(crate) fn encode_into(&self, output: &mut [u8]) -> Result<(), RpcWireError> {
        if output.len() != Self::SIZE {
            return Err(RpcWireError::OutputLength {
                actual: output.len(),
                expected: Self::SIZE,
            });
        }

        output.copy_from_slice(self.channel_context.as_bytes());
        Ok(())
    }

    /// Encodes the request into its fixed-size raw stub.
    pub(crate) fn encode(&self) -> [u8; Self::SIZE] {
        *self.channel_context.as_bytes()
    }
}

/// Raw `TsProxySendToServer` request stub builder.
///
/// The payload slices are borrowed until encoding, so only construction of the
/// final contiguous RPC stub copies payload data.
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
        RpcContextHandle::SIZE /* Context Handle */ + 4 /* Total Bytes */ + 4 /* Number of Buffers */;

    /// Starts a request with its required, non-empty first buffer.
    pub(crate) fn new(channel_context: NonNullRpcContextHandle, first_buffer: &'a [u8]) -> Result<Self, RpcWireError> {
        if first_buffer.is_empty() {
            return Err(RpcWireError::EmptyFirstBuffer);
        }

        Ok(Self {
            channel_context,
            buffers: [first_buffer, &[], &[]],
            buffer_count: 1,
        })
    }

    /// Adds an optional second or third buffer.
    pub(crate) fn push_buffer(&mut self, buffer: &'a [u8]) -> Result<(), RpcWireError> {
        if self.buffer_count == self.buffers.len() {
            return Err(RpcWireError::BufferCount {
                actual: self.buffer_count + 1,
            });
        }

        self.buffers[self.buffer_count] = buffer;
        self.buffer_count += 1;
        Ok(())
    }

    /// Returns the complete raw stub length after validating the `max_is(32767)` bound.
    pub(crate) fn encoded_len(&self) -> Result<usize, RpcWireError> {
        let payload_len = self.payload_len()?;
        let length_fields_len = 4usize
            .checked_mul(self.buffer_count)
            .ok_or(RpcWireError::LengthOverflow)?;
        let total_data_bytes = payload_len
            .checked_add(length_fields_len)
            .ok_or(RpcWireError::LengthOverflow)?;
        let encoded_len = Self::PREFIX_SIZE
            .checked_add(total_data_bytes)
            .ok_or(RpcWireError::LengthOverflow)?;

        if encoded_len > MAX_RPC_MESSAGE_SIZE {
            return Err(RpcWireError::RequestTooLarge { actual: encoded_len });
        }

        Ok(encoded_len)
    }

    /// Encodes this request into an output slice of exactly [`Self::encoded_len`] bytes.
    pub(crate) fn encode_into(&self, output: &mut [u8]) -> Result<(), RpcWireError> {
        let encoded_len = self.encoded_len()?;
        if output.len() != encoded_len {
            return Err(RpcWireError::OutputLength {
                actual: output.len(),
                expected: encoded_len,
            });
        }

        let payload_len = self.payload_len()?;
        let total_data_bytes = payload_len
            .checked_add(4 * self.buffer_count)
            .ok_or(RpcWireError::LengthOverflow)?;
        let total_data_bytes = u32::try_from(total_data_bytes).map_err(|_| RpcWireError::LengthOverflow)?;
        let buffer_count = u32::try_from(self.buffer_count).map_err(|_| RpcWireError::LengthOverflow)?;

        let mut offset = 0;
        output[offset..offset + RpcContextHandle::SIZE].copy_from_slice(self.channel_context.as_bytes());
        offset += RpcContextHandle::SIZE;

        // The length fields are little-endian. [MS-TSGU] 3.6.5.1 says "network byte
        // order", but the authoritative client (mstsc) emits little-endian, and a real RD
        // Gateway rejects the big-endian framing.
        output[offset..offset + 4].copy_from_slice(&total_data_bytes.to_le_bytes());
        offset += 4;
        output[offset..offset + 4].copy_from_slice(&buffer_count.to_le_bytes());
        offset += 4;

        for buffer in self.buffers() {
            let buffer_len = u32::try_from(buffer.len()).map_err(|_| RpcWireError::LengthOverflow)?;
            output[offset..offset + 4].copy_from_slice(&buffer_len.to_le_bytes());
            offset += 4;
        }

        for buffer in self.buffers() {
            let end = offset.checked_add(buffer.len()).ok_or(RpcWireError::LengthOverflow)?;
            output[offset..end].copy_from_slice(buffer);
            offset = end;
        }

        debug_assert_eq!(offset, encoded_len);
        Ok(())
    }

    /// Encodes this request into a newly allocated contiguous raw RPC stub.
    pub(crate) fn encode(&self) -> Result<Vec<u8>, RpcWireError> {
        let mut output = vec![0; self.encoded_len()?];
        self.encode_into(&mut output)?;
        Ok(output)
    }

    fn buffers(&self) -> &[&'a [u8]] {
        &self.buffers[..self.buffer_count]
    }

    fn payload_len(&self) -> Result<usize, RpcWireError> {
        self.buffers().iter().try_fold(0usize, |total, buffer| {
            total.checked_add(buffer.len()).ok_or(RpcWireError::LengthOverflow)
        })
    }
}

/// Parses exactly the four-byte final `TsProxySetupReceivePipe` return-value stub.
///
/// This helper intentionally does not inspect a DCE/RPC PDU or infer byte order.
/// The caller must provide the data representation determined by its surrounding
/// DCE/RPC decoder.
///
/// [MS-TSGU] 2.2.9.4.3 and 3.6.5.5.
pub(crate) fn parse_receive_pipe_final_return_value_stub(
    stub: &[u8],
    byte_order: RpcStubByteOrder,
) -> Result<u32, RpcWireError> {
    let return_value: &[u8; 4] = stub
        .try_into()
        .map_err(|_| RpcWireError::FinalReturnValueLength { actual: stub.len() })?;

    Ok(match byte_order {
        RpcStubByteOrder::LittleEndian => u32::from_le_bytes(*return_value),
        RpcStubByteOrder::BigEndian => u32::from_be_bytes(*return_value),
    })
}

/// DCE/RPC common-header size.
///
/// [MS-RPCE] 2.2.2.1 / [C706] 12.6.
const RPC_COMMON_HEADER_SIZE: usize = 16;
const RPC_REQUEST_HEADER_SIZE: usize = RPC_COMMON_HEADER_SIZE + 8 /* request fields */;
const RPC_BIND_BODY_SIZE: usize = 12 /* bind fields and context-list header */
    + 4 /* presentation-context header */
    + 20 /* abstract syntax */
    + 20 /* transfer syntax */;
const RPC_BIND_PDU_SIZE: usize = RPC_COMMON_HEADER_SIZE + RPC_BIND_BODY_SIZE;

const RPC_VERSION: u8 = 5;
const RPC_VERSION_MINOR: u8 = 0;
const RPC_DREP_LITTLE_ENDIAN: [u8; 4] = [0x10, 0, 0, 0];
const PFC_FIRST_FRAG: u8 = 0x01;
const PFC_LAST_FRAG: u8 = 0x02;
const PFC_SUPPORT_HEADER_SIGN: u8 = 0x04;
/// Concurrent multiplexing: advertised in the bind so the association can carry a
/// held-open asynchronous call (TsProxyMakeTunnelCall) alongside synchronous calls
/// ([C706] 12.6.2 / the PFC_CONC_MPX flag).
const PFC_CONC_MPX: u8 = 0x10;

const PTYPE_REQUEST: u8 = 0;
const PTYPE_RESPONSE: u8 = 2;
const PTYPE_FAULT: u8 = 3;
const PTYPE_BIND: u8 = 11;
const PTYPE_BIND_ACK: u8 = 12;
const PTYPE_BIND_NAK: u8 = 13;
const PTYPE_RPC_AUTH_3: u8 = 16;
const PTYPE_RTS: u8 = 20;

const RPC_CONTEXT_ID: u16 = 0;
const RPC_AUTH_TYPE_WINNT: u8 = 0x0a;
const RPC_AUTH_LEVEL_PACKET_INTEGRITY: u8 = 0x05;
const RPC_AUTH_CONTEXT_ID: u32 = 0;
const RPC_SEC_TRAILER_SIZE: usize = 8;
const DEFAULT_FRAGMENT_SIZE: u16 = 0x10b8;
const NDR32_TRANSFER_SYNTAX_ID: Uuid = Uuid::from_u128(0x8a885d04_1ceb_11c9_9fe8_08002b104860);
const NDR32_TRANSFER_SYNTAX_VERSION: RpcSyntaxVersion = RpcSyntaxVersion::new(2, 0);
const TSPROXY_RPC_INTERFACE_VERSION: RpcSyntaxVersion = RpcSyntaxVersion::new(1, 3);

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
const RPCH_OUT_CONTENT_LENGTH: u32 = 76;
const RPCH_OUT_CONTENT_LENGTH_USIZE: usize = 76;
const RPCH_MIN_CHANNEL_CONTENT_LENGTH: u32 = 128 * 1024;
const RPCH_MAX_CHANNEL_CONTENT_LENGTH: u32 = 2 * 1024 * 1024 * 1024;
const MAX_PENDING_RPC_FRAGMENTS: usize = 16;

/// Errors reported by the DCE/RPC connection-oriented PDU codecs.
///
/// The codec supports the little-endian NDR32 path and the packet-integrity
/// NTLM bind exchange needed by the TS Gateway RPC-over-HTTP runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RpcPduError {
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
    MissingSupportHeaderSign,
    InvalidSecurityTrailer {
        fragment_length: u16,
        auth_length: u16,
    },
    InvalidAuthenticationPadding {
        actual: u8,
        expected: usize,
    },
    NonZeroAuthenticationPadding,
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
    FragmentTooLarge {
        actual: usize,
        maximum: u16,
    },
    LengthOverflow,
    UnexpectedPresentationContextCount {
        actual: u8,
    },
    PresentationContextRejected {
        result: u16,
        reason: u16,
    },
    UnexpectedTransferSyntax {
        identifier: Uuid,
        version: RpcSyntaxVersion,
    },
    UnexpectedContextId {
        actual: u16,
    },
    InvalidAllocHint {
        alloc_hint: u32,
        stub_length: usize,
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
            Self::MissingSupportHeaderSign => f.write_str("rpc pdu does not support header signing"),
            Self::InvalidSecurityTrailer {
                fragment_length,
                auth_length,
            } => {
                write!(
                    f,
                    "invalid rpc security trailer for {fragment_length}-byte fragment and {auth_length}-byte token"
                )
            }
            Self::InvalidAuthenticationPadding { actual, expected } => {
                write!(f, "invalid rpc authentication padding {actual}, expected {expected}")
            }
            Self::NonZeroAuthenticationPadding => f.write_str("rpc authentication padding is not zero"),
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
            Self::FragmentTooLarge { actual, maximum } => {
                write!(f, "rpc pdu length {actual} exceeds maximum {maximum}")
            }
            Self::LengthOverflow => f.write_str("rpc pdu length overflow"),
            Self::UnexpectedPresentationContextCount { actual } => {
                write!(f, "unexpected rpc presentation-context count {actual}, expected 1")
            }
            Self::PresentationContextRejected { result, reason } => {
                write!(
                    f,
                    "rpc presentation context rejected with result {result}, reason {reason}"
                )
            }
            Self::UnexpectedTransferSyntax { identifier, version } => {
                write!(
                    f,
                    "unexpected rpc transfer syntax {identifier} version {}.{}",
                    version.major, version.minor
                )
            }
            Self::UnexpectedContextId { actual } => {
                write!(
                    f,
                    "unexpected rpc presentation context id {actual}, expected {RPC_CONTEXT_ID}"
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
            Self::InvalidBindNakVersionsLength { actual, expected } => {
                write!(f, "invalid rpc bind_nak versions length {actual}, expected {expected}")
            }
            Self::UnexpectedRtsCallId { actual } => {
                write!(f, "unexpected rts call id {actual}, expected 0")
            }
            Self::InvalidRtsPfcFlags { actual } => {
                write!(f, "invalid rts pfc flags 0x{actual:02x}")
            }
            Self::UnexpectedRtsFlags { expected, actual } => {
                write!(f, "unexpected rts flags 0x{actual:04x}, expected 0x{expected:04x}")
            }
            Self::UnexpectedRtsCommandCount { expected, actual } => {
                write!(f, "unexpected rts command count {actual}, expected {expected}")
            }
            Self::InvalidRtsBodyLength { expected, actual } => {
                write!(f, "invalid rts command body length {actual}, expected {expected}")
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
pub(crate) struct RpcSyntaxVersion {
    major: u16,
    minor: u16,
}

impl RpcSyntaxVersion {
    const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

/// Negotiated client-to-server and server-to-client fragment maxima.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RpcFragmentSizes {
    max_xmit: u16,
    max_recv: u16,
}

impl RpcFragmentSizes {
    /// Conventional DCE/RPC fragment maxima used for the initial bind.
    pub(crate) const DEFAULT: Self = Self {
        max_xmit: DEFAULT_FRAGMENT_SIZE,
        max_recv: DEFAULT_FRAGMENT_SIZE,
    };

    pub(crate) fn new(max_xmit: u16, max_recv: u16) -> Result<Self, RpcPduError> {
        for maximum in [max_xmit, max_recv] {
            if usize::from(maximum) < RPC_COMMON_HEADER_SIZE {
                return Err(RpcPduError::InvalidFragmentSize { maximum });
            }
        }

        Ok(Self { max_xmit, max_recv })
    }

    pub(crate) const fn max_xmit(self) -> u16 {
        self.max_xmit
    }

    pub(crate) const fn max_recv(self) -> u16 {
        self.max_recv
    }
}

/// The parsed 16-byte DCE/RPC common header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RpcCommonHeader {
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
    pub(crate) fn decode(source: &[u8]) -> Result<Self, RpcPduError> {
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

    pub(crate) const fn fragment_length(self) -> u16 {
        self.fragment_length
    }

    pub(crate) const fn ptype(self) -> u8 {
        self.ptype
    }

    pub(crate) const fn pfc_flags(self) -> u8 {
        self.pfc_flags
    }

    pub(crate) const fn auth_length(self) -> u16 {
        self.auth_length
    }

    pub(crate) const fn call_id(self) -> u32 {
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
            8usize
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

/// Incrementally frames DCE/RPC PDUs from an RPCH response stream.
///
/// Each yielded buffer is exactly one complete DCE/RPC fragment. The stream
/// validates the common header and negotiated receive maximum before retaining
/// a claimed fragment, preventing a peer from making the client buffer an
/// oversized PDU.
#[derive(Debug)]
pub(crate) struct RpcPduStream {
    buffer: Vec<u8>,
    maximum_fragment_size: u16,
    maximum_pending_bytes: usize,
}

impl RpcPduStream {
    pub(crate) fn new(maximum_fragment_size: u16) -> Result<Self, RpcPduError> {
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

    /// Adds received RPCH response bytes to the pending DCE/RPC stream.
    ///
    /// The caller must drain complete fragments with [`Self::next`] before
    /// buffering more than [`MAX_PENDING_RPC_FRAGMENTS`] fragments.
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<(), RpcPduError> {
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
    pub(crate) fn next(&mut self) -> Result<Option<Vec<u8>>, RpcPduError> {
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

/// RPCH HTTP channel selected for a request.
///
/// [MS-RPCH] 2.1.2.1.1 and 2.1.2.1.2.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RpchHttpChannel {
    In,
    Out,
}

impl RpchHttpChannel {
    const fn method(self) -> &'static str {
        match self {
            Self::In => "RPC_IN_DATA",
            Self::Out => "RPC_OUT_DATA",
        }
    }

    const fn expected_content_length(self) -> Option<u32> {
        match self {
            Self::In => None,
            Self::Out => Some(RPCH_OUT_CONTENT_LENGTH),
        }
    }
}

/// RPCH virtual-connection opening state.
///
/// [MS-RPCH] 3.2.2.4.1.2 and 3.2.2.5.2 through 3.2.2.5.4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RpcHttpV2State {
    Initial,
    InRequestStarted,
    OutRequestStarted,
    AwaitingOutResponse,
    AwaitingA3,
    AwaitingC2,
    Open,
    Failed,
}

/// Errors returned while building or advancing an RPCH v2 connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RpcHttpV2Error {
    InvalidState {
        action: &'static str,
        state: RpcHttpV2State,
    },
    InvalidGatewayHost,
    InvalidTargetServer,
    InvalidTargetPort,
    InvalidContentLength {
        channel: RpchHttpChannel,
        actual: u32,
    },
    InvalidHttpHeader,
    OutResponseStatus {
        actual: u16,
    },
    OutResponseContentType,
    OutResponseContentLength {
        actual: Option<u32>,
    },
    UnsupportedProtocolVersion {
        actual: u32,
    },
    Rpc(RpcPduError),
}

impl fmt::Display for RpcHttpV2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidState { action, state } => {
                write!(f, "cannot {action} while rpch setup is {state:?}")
            }
            Self::InvalidGatewayHost => f.write_str("invalid rpch gateway host"),
            Self::InvalidTargetServer => f.write_str("invalid rpch target server"),
            Self::InvalidTargetPort => f.write_str("invalid rpch target port"),
            Self::InvalidContentLength { channel, actual } => {
                write!(f, "invalid rpch {channel:?} content length {actual}")
            }
            Self::InvalidHttpHeader => f.write_str("invalid rpch HTTP header"),
            Self::OutResponseStatus { actual } => {
                write!(f, "invalid rpch OUT response status {actual}")
            }
            Self::OutResponseContentType => f.write_str("invalid rpch OUT response content type"),
            Self::OutResponseContentLength { actual } => match actual {
                Some(actual) => write!(f, "invalid rpch OUT response content length {actual}"),
                None => f.write_str("missing rpch OUT response content length"),
            },
            Self::UnsupportedProtocolVersion { actual } => {
                write!(f, "unsupported rpch protocol version {actual}")
            }
            Self::Rpc(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for RpcHttpV2Error {}

impl From<RpcPduError> for RpcHttpV2Error {
    fn from(error: RpcPduError) -> Self {
        Self::Rpc(error)
    }
}

/// Client-controlled values used when opening an RPCH virtual connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RpcHttpV2Settings {
    receive_window_size: u32,
    channel_lifetime: u32,
    client_keepalive: u32,
}

impl RpcHttpV2Settings {
    const DEFAULT_CLIENT_KEEPALIVE: u32 = 300_000;

    pub(crate) fn new(
        receive_window_size: u32,
        channel_lifetime: u32,
        client_keepalive: u32,
    ) -> Result<Self, RpcPduError> {
        validate_rts_receive_window_size(receive_window_size)?;
        validate_rts_channel_lifetime(channel_lifetime)?;
        validate_rts_client_keepalive(client_keepalive)?;

        Ok(Self {
            receive_window_size,
            channel_lifetime,
            client_keepalive,
        })
    }

    pub(crate) const fn receive_window_size(self) -> u32 {
        self.receive_window_size
    }

    /// Returns the exact `Content-Length` for the default IN channel.
    pub(crate) const fn channel_lifetime(self) -> u32 {
        self.channel_lifetime
    }

    pub(crate) const fn client_keepalive(self) -> u32 {
        self.client_keepalive
    }

    const fn effective_client_keepalive(self) -> u32 {
        if self.client_keepalive == 0 {
            Self::DEFAULT_CLIENT_KEEPALIVE
        } else {
            self.client_keepalive
        }
    }
}

impl Default for RpcHttpV2Settings {
    fn default() -> Self {
        Self {
            receive_window_size: 64 * 1024,
            channel_lifetime: 1024 * 1024 * 1024,
            client_keepalive: Self::DEFAULT_CLIENT_KEEPALIVE,
        }
    }
}

/// Stateful validation for the initial RPCH v2 CONN sequence.
///
/// The HTTP implementation must first start the authenticated IN request, then
/// send the fixed-length OUT request returned by [`Self::out_request_body`],
/// then write [`Self::in_request_initial_pdu`] to the authenticated streaming
/// IN body. Only then may it accept the OUT response and feed its body PDUs to
/// [`Self::receive_out_pdu`].
///
/// [MS-RPCH] 3.2.2.4.1.2 and 3.2.2.5.2 through 3.2.2.5.4.
#[derive(Debug)]
pub(crate) struct RpcHttpV2Setup {
    settings: RpcHttpV2Settings,
    virtual_connection_cookie: RtsCookie,
    out_channel_cookie: RtsCookie,
    in_channel_cookie: RtsCookie,
    association_group_id: RtsCookie,
    state: RpcHttpV2State,
    in_ping_timeout: Option<u32>,
    connection_timeout: Option<u32>,
    peer_receive_window_size: Option<u32>,
}

impl RpcHttpV2Setup {
    pub(crate) fn new(settings: RpcHttpV2Settings) -> Self {
        Self::with_cookies(
            settings,
            RtsCookie::new(*Uuid::new_v4().as_bytes()),
            RtsCookie::new(*Uuid::new_v4().as_bytes()),
            RtsCookie::new(*Uuid::new_v4().as_bytes()),
            RtsCookie::new(*Uuid::new_v4().as_bytes()),
        )
    }

    pub(crate) const fn with_cookies(
        settings: RpcHttpV2Settings,
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
            state: RpcHttpV2State::Initial,
            in_ping_timeout: None,
            connection_timeout: None,
            peer_receive_window_size: None,
        }
    }

    pub(crate) const fn state(&self) -> RpcHttpV2State {
        self.state
    }

    pub(crate) const fn in_channel_content_length(&self) -> u32 {
        self.settings.channel_lifetime()
    }

    pub(crate) const fn connection_timeout(&self) -> Option<u32> {
        self.connection_timeout
    }

    pub(crate) const fn in_ping_timeout(&self) -> Option<u32> {
        self.in_ping_timeout
    }

    pub(crate) const fn peer_receive_window_size(&self) -> Option<u32> {
        self.peer_receive_window_size
    }

    /// Records that the authenticated IN request headers are in flight.
    ///
    /// The caller must not write an IN body PDU before
    /// [`Self::in_request_initial_pdu`] is returned.
    pub(crate) fn start_in_request(&mut self) -> Result<(), RpcHttpV2Error> {
        if self.state != RpcHttpV2State::Initial {
            return self.fail(RpcHttpV2Error::InvalidState {
                action: "start the IN request",
                state: self.state,
            });
        }

        self.state = RpcHttpV2State::InRequestStarted;
        Ok(())
    }

    /// Returns the exact 76-byte CONN/A1 body for the OUT request.
    pub(crate) fn out_request_body(&mut self) -> Result<Vec<u8>, RpcHttpV2Error> {
        if self.state != RpcHttpV2State::InRequestStarted {
            return self.fail(RpcHttpV2Error::InvalidState {
                action: "start the OUT request",
                state: self.state,
            });
        }

        let body = encode_rts_conn_a1(
            self.virtual_connection_cookie,
            self.out_channel_cookie,
            self.settings.receive_window_size(),
        )
        .map_err(RpcHttpV2Error::from)?;
        debug_assert_eq!(body.len(), RPCH_OUT_CONTENT_LENGTH_USIZE);
        self.state = RpcHttpV2State::OutRequestStarted;
        Ok(body)
    }

    /// Returns the CONN/B1 PDU, the first body PDU for the streaming IN request.
    pub(crate) fn in_request_initial_pdu(&mut self) -> Result<Vec<u8>, RpcHttpV2Error> {
        if self.state != RpcHttpV2State::OutRequestStarted {
            return self.fail(RpcHttpV2Error::InvalidState {
                action: "send CONN/B1",
                state: self.state,
            });
        }

        let pdu = encode_rts_conn_b1(
            self.virtual_connection_cookie,
            self.in_channel_cookie,
            self.settings.channel_lifetime(),
            self.settings.client_keepalive(),
            self.association_group_id,
        )
        .map_err(RpcHttpV2Error::from)?;
        self.state = RpcHttpV2State::AwaitingOutResponse;
        Ok(pdu)
    }

    /// Validates the OUT response before its PDU body is consumed.
    pub(crate) fn accept_out_response(
        &mut self,
        status: u16,
        content_type: Option<&str>,
        content_length: Option<u32>,
    ) -> Result<(), RpcHttpV2Error> {
        if self.state != RpcHttpV2State::AwaitingOutResponse {
            return self.fail(RpcHttpV2Error::InvalidState {
                action: "accept the OUT response",
                state: self.state,
            });
        }
        if status != http::StatusCode::OK.as_u16() {
            return self.fail(RpcHttpV2Error::OutResponseStatus { actual: status });
        }
        if !content_type.is_some_and(|value| value.trim().eq_ignore_ascii_case("application/rpc")) {
            return self.fail(RpcHttpV2Error::OutResponseContentType);
        }
        if !content_length
            .is_some_and(|value| (RPCH_MIN_CHANNEL_CONTENT_LENGTH..=RPCH_MAX_CHANNEL_CONTENT_LENGTH).contains(&value))
        {
            return self.fail(RpcHttpV2Error::OutResponseContentLength { actual: content_length });
        }

        self.state = RpcHttpV2State::AwaitingA3;
        Ok(())
    }

    /// Validates the next CONN/A3 or CONN/C2 PDU from the OUT response body.
    pub(crate) fn receive_out_pdu(&mut self, pdu: &[u8]) -> Result<(), RpcHttpV2Error> {
        match self.state {
            RpcHttpV2State::AwaitingA3 => {
                let a3 = match decode_rts_conn_a3(pdu) {
                    Ok(a3) => a3,
                    Err(error) => return self.fail(error.into()),
                };
                self.in_ping_timeout = Some(a3.connection_timeout);
                self.state = RpcHttpV2State::AwaitingC2;
                Ok(())
            }
            RpcHttpV2State::AwaitingC2 => {
                let c2 = match decode_rts_conn_c2(pdu) {
                    Ok(c2) => c2,
                    Err(error) => return self.fail(error.into()),
                };
                if c2.version != RTS_VERSION {
                    return self.fail(RpcHttpV2Error::UnsupportedProtocolVersion { actual: c2.version });
                }

                self.connection_timeout = Some(c2.connection_timeout);
                self.peer_receive_window_size = Some(c2.receive_window_size);
                self.state = RpcHttpV2State::Open;
                Ok(())
            }
            state => self.fail(RpcHttpV2Error::InvalidState {
                action: "consume an OUT setup PDU",
                state,
            }),
        }
    }

    /// Rejects use of the RPC PDU streams until the CONN sequence has completed.
    pub(crate) fn require_open(&mut self) -> Result<(), RpcHttpV2Error> {
        if self.state == RpcHttpV2State::Open {
            Ok(())
        } else {
            self.fail(RpcHttpV2Error::InvalidState {
                action: "send an RPC PDU",
                state: self.state,
            })
        }
    }

    /// Creates the flow-control state for the established default IN and OUT channels.
    ///
    /// [MS-RPCH] 3.2.1.4.1 and 3.2.1.5.1.
    pub(crate) fn flow_control(&self) -> Result<RpcHttpV2FlowControl, RpcHttpV2Error> {
        if self.state != RpcHttpV2State::Open {
            return Err(RpcHttpV2Error::InvalidState {
                action: "create RPCH flow-control state",
                state: self.state,
            });
        }

        let peer_receive_window_size = self.peer_receive_window_size.ok_or(RpcHttpV2Error::InvalidState {
            action: "read the peer receive window",
            state: self.state,
        })?;
        Ok(RpcHttpV2FlowControl::new(
            self.settings.receive_window_size(),
            peer_receive_window_size,
            self.out_channel_cookie,
            self.in_channel_cookie,
        ))
    }

    /// Creates the ping schedule for the established default IN channel.
    ///
    /// [MS-RPCH] 3.2.1.2.1, 3.2.1.2.2, and 3.2.2.6.
    pub(crate) fn ping_schedule(&self, now: Duration) -> Result<RpcHttpV2PingSchedule, RpcHttpV2Error> {
        if self.state != RpcHttpV2State::Open {
            return Err(RpcHttpV2Error::InvalidState {
                action: "create RPCH ping schedule",
                state: self.state,
            });
        }

        let connection_timeout = self.in_ping_timeout.ok_or(RpcHttpV2Error::InvalidState {
            action: "read the IN channel connection timeout",
            state: self.state,
        })?;
        Ok(RpcHttpV2PingSchedule::new(
            Duration::from_millis(u64::from(connection_timeout)),
            Duration::from_millis(u64::from(self.settings.effective_client_keepalive())),
            now,
        ))
    }

    fn fail<T>(&mut self, error: RpcHttpV2Error) -> Result<T, RpcHttpV2Error> {
        self.state = RpcHttpV2State::Failed;
        Err(error)
    }
}

/// Flow-control state for the default RPCH IN and OUT channels.
///
/// RPC PDUs received on the OUT channel consume the local receive window.
/// RPC PDUs sent on the IN channel consume the peer's advertised receive window.
///
/// [MS-RPCH] 3.2.1.1.4, 3.2.1.1.5, 3.2.1.4.1, and 3.2.1.5.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RpcHttpV2FlowControl {
    receive_window_size: u32,
    receive_available_window: u32,
    // This tracks advertised capacity minus later queued PDUs and can become negative.
    receive_available_window_advertised: i64,
    receive_bytes_received: u32,
    peer_receive_window_size: u32,
    send_available_window: u32,
    send_bytes_sent: u32,
    receive_channel_cookie: RtsCookie,
    send_channel_cookie: RtsCookie,
}

/// Errors reported while applying RPCH receive-window accounting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RpcHttpV2FlowControlError {
    PduLengthOverflow {
        actual: usize,
    },
    SendWindowExhausted {
        pdu_size: u32,
        available_window: u32,
    },
    ReceiveWindowExhausted {
        pdu_size: u32,
        available_window: u32,
    },
    PduNotQueued {
        pdu_size: u32,
        queued_bytes: u32,
    },
    BytesReceivedOverflow {
        current: u32,
        pdu_size: u32,
    },
    BytesSentOverflow {
        current: u32,
        pdu_size: u32,
    },
    InvalidFlowControlAck {
        bytes_received: u32,
        bytes_sent: u32,
    },
    FlowControlAckWindowExceedsPeer {
        available_window: u32,
        peer_receive_window_size: u32,
    },
    FlowControlAckExhaustsSenderWindow {
        available_window: u32,
        unacknowledged_bytes: u32,
    },
}

impl fmt::Display for RpcHttpV2FlowControlError {
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

impl core::error::Error for RpcHttpV2FlowControlError {}

/// Schedules PING PDUs for the default RPCH IN channel.
///
/// The caller records every PDU sent on that channel with [`Self::record_send`].
/// A PING is due when the negotiated connection timeout expires without traffic,
/// or halfway through the requested keepalive interval.
///
/// [MS-RPCH] 3.2.1.2.1, 3.2.1.2.2, and 3.2.2.6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RpcHttpV2PingSchedule {
    connection_timeout: Duration,
    keepalive_interval: Duration,
    last_send: Duration,
}

impl RpcHttpV2PingSchedule {
    const fn new(connection_timeout: Duration, keepalive_interval: Duration, now: Duration) -> Self {
        Self {
            connection_timeout,
            keepalive_interval,
            last_send: now,
        }
    }

    /// Returns whether a PING must be sent at `now`.
    pub(crate) fn ping_due(&self, now: Duration) -> bool {
        let elapsed = now.saturating_sub(self.last_send);
        elapsed >= self.connection_timeout
            || (!self.keepalive_interval.is_zero() && elapsed >= self.keepalive_interval / 2)
    }

    /// Records a PDU sent on the default IN channel.
    pub(crate) fn record_send(&mut self, now: Duration) {
        self.last_send = now;
    }
}

impl RpcHttpV2FlowControl {
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
            receive_channel_cookie,
            send_channel_cookie,
        }
    }

    pub(crate) const fn send_available_window(&self) -> u32 {
        self.send_available_window
    }

    pub(crate) const fn receive_available_window(&self) -> u32 {
        self.receive_available_window
    }

    /// Records an RPC PDU queued on the default IN channel.
    ///
    /// The caller must wait for a flow-control acknowledgement when this returns
    /// [`RpcHttpV2FlowControlError::SendWindowExhausted`].
    pub(crate) fn sent_rpc_pdu(&mut self, pdu_length: usize) -> Result<(), RpcHttpV2FlowControlError> {
        let pdu_size = u32::try_from(pdu_length)
            .map_err(|_| RpcHttpV2FlowControlError::PduLengthOverflow { actual: pdu_length })?;
        if self.send_available_window <= pdu_size {
            return Err(RpcHttpV2FlowControlError::SendWindowExhausted {
                pdu_size,
                available_window: self.send_available_window,
            });
        }

        self.send_bytes_sent =
            self.send_bytes_sent
                .checked_add(pdu_size)
                .ok_or(RpcHttpV2FlowControlError::BytesSentOverflow {
                    current: self.send_bytes_sent,
                    pdu_size,
                })?;
        self.send_available_window -= pdu_size;
        Ok(())
    }

    /// Records an RPC PDU queued in the local receive window.
    pub(crate) fn received_rpc_pdu(&mut self, pdu_length: usize) -> Result<(), RpcHttpV2FlowControlError> {
        let pdu_size = u32::try_from(pdu_length)
            .map_err(|_| RpcHttpV2FlowControlError::PduLengthOverflow { actual: pdu_length })?;
        if pdu_size > self.receive_available_window {
            return Err(RpcHttpV2FlowControlError::ReceiveWindowExhausted {
                pdu_size,
                available_window: self.receive_available_window,
            });
        }

        self.receive_bytes_received = self.receive_bytes_received.checked_add(pdu_size).ok_or(
            RpcHttpV2FlowControlError::BytesReceivedOverflow {
                current: self.receive_bytes_received,
                pdu_size,
            },
        )?;
        self.receive_available_window -= pdu_size;
        self.receive_available_window_advertised -= i64::from(pdu_size);
        Ok(())
    }

    /// Records a higher-layer consumer releasing an RPC PDU from the receive window.
    ///
    /// When enough space has been reclaimed since the last acknowledgement, returns
    /// the acknowledgement to send on the IN channel.
    pub(crate) fn consumed_rpc_pdu(
        &mut self,
        pdu_length: usize,
    ) -> Result<Option<RtsFlowControlAck>, RpcHttpV2FlowControlError> {
        let pdu_size = u32::try_from(pdu_length)
            .map_err(|_| RpcHttpV2FlowControlError::PduLengthOverflow { actual: pdu_length })?;
        let queued_bytes = self.receive_window_size - self.receive_available_window;
        if pdu_size > queued_bytes {
            return Err(RpcHttpV2FlowControlError::PduNotQueued { pdu_size, queued_bytes });
        }
        self.receive_available_window += pdu_size;

        let reclaimed_window = i64::from(self.receive_available_window) - self.receive_available_window_advertised;
        if reclaimed_window <= i64::from(self.receive_window_size / 2) {
            return Ok(None);
        }

        self.receive_available_window_advertised = i64::from(self.receive_available_window);
        Ok(Some(RtsFlowControlAck {
            bytes_received: self.receive_bytes_received,
            available_window: self.receive_available_window,
            channel_cookie: self.receive_channel_cookie,
        }))
    }

    /// Applies a flow-control acknowledgement received on the OUT channel.
    ///
    /// An acknowledgement for another channel is discarded as required by
    /// [MS-RPCH] 3.2.1.5.1.2 and returns `Ok(false)`.
    pub(crate) fn receive_flow_control_ack(
        &mut self,
        ack: RtsFlowControlAck,
    ) -> Result<bool, RpcHttpV2FlowControlError> {
        if ack.channel_cookie != self.send_channel_cookie {
            return Ok(false);
        }
        if ack.bytes_received > self.send_bytes_sent {
            return Err(RpcHttpV2FlowControlError::InvalidFlowControlAck {
                bytes_received: ack.bytes_received,
                bytes_sent: self.send_bytes_sent,
            });
        }
        if ack.available_window > self.peer_receive_window_size {
            return Err(RpcHttpV2FlowControlError::FlowControlAckWindowExceedsPeer {
                available_window: ack.available_window,
                peer_receive_window_size: self.peer_receive_window_size,
            });
        }

        let unacknowledged_bytes = self.send_bytes_sent - ack.bytes_received;
        self.send_available_window = ack.available_window.checked_sub(unacknowledged_bytes).ok_or(
            RpcHttpV2FlowControlError::FlowControlAckExhaustsSenderWindow {
                available_window: ack.available_window,
                unacknowledged_bytes,
            },
        )?;
        Ok(true)
    }
}

/// Builds a standards-conforming RPCH v2 request with a caller-supplied body.
///
/// The authenticated HTTP request retry loop should call this only after it has
/// selected the final authorization header. In particular, the streaming IN
/// body must remain uncommitted until that loop has completed.
///
/// [MS-RPCH] 2.1.2.1.1, 2.1.2.1.2, and 2.2.2.
pub(crate) fn build_rpch_v2_request<B>(
    channel: RpchHttpChannel,
    gateway_host: &str,
    target: &crate::GwConnectTarget,
    content_length: u32,
    authorization: Option<&str>,
    cookie: Option<&str>,
    body: B,
) -> Result<http::Request<B>, RpcHttpV2Error> {
    validate_rpch_content_length(channel, content_length)?;
    let uri = rpch_v2_uri(&target.server, target.server_port)?;
    let host = rpch_host_header(gateway_host)?;

    let mut request = http::Request::builder()
        .method(channel.method())
        .uri(uri)
        .header(http::header::ACCEPT, "application/rpc")
        .header(http::header::CACHE_CONTROL, "no-cache")
        .header(http::header::CONNECTION, "Keep-Alive")
        .header(http::header::CONTENT_LENGTH, content_length)
        .header(http::header::HOST, host)
        .header(http::header::PRAGMA, "no-cache")
        .header("Protocol", "1.0")
        .header(http::header::USER_AGENT, "MSRPC");
    if let Some(authorization) = authorization {
        request = request.header(http::header::AUTHORIZATION, authorization);
    }
    if let Some(cookie) = cookie {
        request = request.header(http::header::COOKIE, cookie);
    }

    request.body(body).map_err(|_| RpcHttpV2Error::InvalidHttpHeader)
}

fn validate_rpch_content_length(channel: RpchHttpChannel, content_length: u32) -> Result<(), RpcHttpV2Error> {
    match channel.expected_content_length() {
        Some(expected) if content_length == expected => Ok(()),
        Some(_) => Err(RpcHttpV2Error::InvalidContentLength {
            channel,
            actual: content_length,
        }),
        None if (RPCH_MIN_CHANNEL_CONTENT_LENGTH..=RPCH_MAX_CHANNEL_CONTENT_LENGTH).contains(&content_length) => Ok(()),
        None => Err(RpcHttpV2Error::InvalidContentLength {
            channel,
            actual: content_length,
        }),
    }
}

fn rpch_v2_uri(server: &str, port: u16) -> Result<String, RpcHttpV2Error> {
    let server = server
        .strip_prefix('[')
        .and_then(|server| server.strip_suffix(']'))
        .unwrap_or(server);
    if server.is_empty()
        || server.len() >= 1024
        || !server
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':'))
    {
        return Err(RpcHttpV2Error::InvalidTargetServer);
    }
    if port == 0 {
        return Err(RpcHttpV2Error::InvalidTargetPort);
    }

    Ok(format!("/rpc/rpcproxy.dll?{server}:{port}"))
}

fn rpch_host_header(gateway_host: &str) -> Result<String, RpcHttpV2Error> {
    if gateway_host.is_empty() || gateway_host.bytes().any(|byte| byte.is_ascii_control() || byte == b' ') {
        return Err(RpcHttpV2Error::InvalidGatewayHost);
    }

    Ok(if gateway_host.contains(':') {
        format!("[{gateway_host}]")
    } else {
        gateway_host.to_owned()
    })
}

/// The successful result of binding the single TS Gateway presentation context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RpcBinding {
    fragment_sizes: RpcFragmentSizes,
    call_id: u32,
}

impl RpcBinding {
    pub(crate) const fn fragment_sizes(self) -> RpcFragmentSizes {
        self.fragment_sizes
    }

    pub(crate) const fn call_id(self) -> u32 {
        self.call_id
    }
}

/// The authenticated result of binding the single TS Gateway presentation context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RpcAuthenticatedBindAck<'a> {
    binding: RpcBinding,
    token: &'a [u8],
}

impl RpcAuthenticatedBindAck<'_> {
    pub(crate) const fn binding(&self) -> RpcBinding {
        self.binding
    }

    pub(crate) const fn token(&self) -> &[u8] {
        self.token
    }
}

/// Client-side NTLM state for the DCE/RPC security context.
///
/// This state intentionally owns an independent security package and credential
/// handle; it is not shared with HTTP authentication.
pub(crate) struct RpcNtlmAuth {
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
    pub(crate) fn new(username: &str, password: &str) -> Result<Self, Error> {
        let identity = rpc_auth_identity(username, password)?;
        let mut ntlm = Ntlm::new();
        let credentials_handle = ntlm
            .acquire_credentials_handle()
            .with_credential_use(CredentialUse::Outbound)
            .with_auth_data(&identity)
            .execute(&mut ntlm)
            .map_err(|e| Error::custom("acquire rpc ntlm credentials", e))?
            .credentials_handle;

        Ok(Self {
            ntlm,
            credentials_handle,
            state: RpcNtlmAuthState::Initial,
        })
    }

    /// Produces the NTLM Type-1 token for an authenticated bind PDU.
    pub(crate) fn initial_token(&mut self) -> Result<Vec<u8>, Error> {
        if self.state != RpcNtlmAuthState::Initial {
            return Err(Error::new("rpc ntlm initial token", GwErrorKind::Connect));
        }

        let token = self.next_token(None)?;
        if self.state != RpcNtlmAuthState::AwaitingChallenge {
            return Err(Error::new("rpc ntlm initial token", GwErrorKind::Connect));
        }

        Ok(token)
    }

    /// Consumes the bind_ack Type-2 token and produces the Type-3 token.
    pub(crate) fn continue_token(&mut self, challenge: &[u8]) -> Result<Vec<u8>, Error> {
        if self.state != RpcNtlmAuthState::AwaitingChallenge {
            return Err(Error::new("rpc ntlm continuation token", GwErrorKind::Connect));
        }

        let token = self.next_token(Some(challenge))?;
        if self.state != RpcNtlmAuthState::Established {
            return Err(Error::new("rpc ntlm continuation token", GwErrorKind::Connect));
        }

        Ok(token)
    }

    /// Produces the packet-integrity signature for an established security context.
    pub(crate) fn make_signature(&mut self, data: &mut [u8], sequence_number: u32) -> Result<[u8; 16], Error> {
        if self.state != RpcNtlmAuthState::Established {
            return Err(Error::new("make rpc ntlm signature", GwErrorKind::Connect));
        }

        let mut signature = [0; 16];
        let mut buffers = [
            SecurityBufferRef::data_buf(data),
            SecurityBufferRef::token_buf(&mut signature),
        ];
        self.ntlm
            .make_signature(0, &mut buffers, sequence_number)
            .map_err(|e| Error::custom("make rpc ntlm signature", e))?;
        Ok(signature)
    }

    /// Verifies the packet-integrity signature for an established security context.
    pub(crate) fn verify_signature(
        &mut self,
        data: &mut [u8],
        signature: &mut [u8],
        sequence_number: u32,
    ) -> Result<(), Error> {
        if self.state != RpcNtlmAuthState::Established {
            return Err(Error::new("verify rpc ntlm signature", GwErrorKind::Connect));
        }

        let mut buffers = [
            SecurityBufferRef::data_buf(data),
            SecurityBufferRef::token_buf(signature),
        ];
        self.ntlm
            .verify_signature(&mut buffers, sequence_number)
            .map(|_| ())
            .map_err(|e| Error::custom("verify rpc ntlm signature", e))
    }

    pub(crate) fn is_complete(&self) -> bool {
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
            .map_err(|e| Error::custom("initialize rpc ntlm security context", e))?
            .resolve_to_result()
            .map_err(|e| Error::custom("initialize rpc ntlm security context", e))?;

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

/// A decoded, single-fragment RPC response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RpcResponse<'a> {
    pub(crate) call_id: u32,
    pfc_flags: u8,
    pub(crate) alloc_hint: u32,
    pub(crate) cancel_count: u8,
    pub(crate) reserved: u8,
    pub(crate) stub: &'a [u8],
}

impl RpcResponse<'_> {
    /// Whether this fragment carries the last-fragment flag, terminating its response
    /// ([C706] 12.6.2).
    pub(crate) fn is_last_fragment(&self) -> bool {
        self.pfc_flags & PFC_LAST_FRAG != 0
    }
}

/// An owned RPC response reassembled from one or more response fragments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RpcReassembledResponse {
    pub(crate) call_id: u32,
    pub(crate) cancel_count: u8,
    pub(crate) reserved: u8,
    pub(crate) stub: Vec<u8>,
}

/// Bounded reassembler for consecutive DCE/RPC response fragments.
///
/// Every fragment must belong to the same call and the first/last-fragment flags
/// must delimit exactly one complete response.
#[derive(Debug)]
pub(crate) struct RpcResponseReassembler {
    maximum_stub_size: usize,
    call_id: Option<u32>,
    cancel_count: u8,
    reserved: u8,
    alloc_hints: Vec<(usize, u32)>,
    stub: Vec<u8>,
}

impl RpcResponseReassembler {
    pub(crate) fn new(maximum_stub_size: usize) -> Self {
        Self {
            maximum_stub_size,
            call_id: None,
            cancel_count: 0,
            reserved: 0,
            alloc_hints: Vec::new(),
            stub: Vec::new(),
        }
    }

    /// Adds one decoded response fragment and returns an owned response after
    /// receiving its last fragment.
    pub(crate) fn push(&mut self, response: RpcResponse<'_>) -> Result<Option<RpcReassembledResponse>, RpcPduError> {
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

        if self.call_id.is_none() {
            self.call_id = Some(response.call_id);
            self.cancel_count = response.cancel_count;
            self.reserved = response.reserved;
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
        if response.alloc_hint != 0 {
            self.alloc_hints.push((stub_offset, response.alloc_hint));
        }
        self.stub.extend_from_slice(response.stub);

        if flags & PFC_LAST_FRAG == 0 {
            return Ok(None);
        }

        let total_length = self.stub.len();
        let invalid_alloc_hint =
            self.alloc_hints
                .iter()
                .find_map(|(offset, alloc_hint)| match usize::try_from(*alloc_hint) {
                    Ok(hint) if hint <= total_length - *offset => None,
                    _ => Some(*alloc_hint),
                });
        if let Some(alloc_hint) = invalid_alloc_hint {
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
        self.alloc_hints.clear();
        self.stub.clear();
    }
}

/// A decoded, single-fragment RPC fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RpcFault<'a> {
    pub(crate) call_id: u32,
    pub(crate) alloc_hint: u32,
    pub(crate) cancel_count: u8,
    pub(crate) reserved: u8,
    pub(crate) status: u32,
    pub(crate) reserved2: u32,
    pub(crate) stub: &'a [u8],
}

/// An exact 16-byte RPC-over-HTTP RTS cookie.
///
/// [MS-RPCH] 2.2.3.5.4.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct RtsCookie([u8; Self::SIZE]);

impl RtsCookie {
    pub(crate) const SIZE: usize = 16;

    pub(crate) const fn new(bytes: [u8; Self::SIZE]) -> Self {
        Self(bytes)
    }

    pub(crate) const fn as_bytes(&self) -> &[u8; Self::SIZE] {
        &self.0
    }
}

impl fmt::Debug for RtsCookie {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RtsCookie(..)")
    }
}

/// Values received in a CONN/A3 RTS PDU.
///
/// [MS-RPCH] 2.2.4.4.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RtsConnA3 {
    pub(crate) connection_timeout: u32,
}

/// Values received in a CONN/C2 RTS PDU.
///
/// [MS-RPCH] 2.2.4.9.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RtsConnC2 {
    pub(crate) version: u32,
    pub(crate) receive_window_size: u32,
    pub(crate) connection_timeout: u32,
}

/// Values received in an IN_R1/A4 RTS PDU.
///
/// [MS-RPCH] 2.2.4.13.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RtsInRecycleA4 {
    pub(crate) version: u32,
    pub(crate) receive_window_size: u32,
    pub(crate) connection_timeout: u32,
}

/// Values received in an OUT_R1/A6 RTS PDU.
///
/// [MS-RPCH] 2.2.4.28.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RtsOutRecycleA6 {
    pub(crate) version: u32,
    pub(crate) connection_timeout: u32,
}

/// A flow-control acknowledgement for an RPCH channel.
///
/// [MS-RPCH] 2.2.3.4 and 2.2.4.50.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RtsFlowControlAck {
    pub(crate) bytes_received: u32,
    pub(crate) available_window: u32,
    pub(crate) channel_cookie: RtsCookie,
}

/// Encodes the client CONN/A1 RTS PDU for the OUT channel.
///
/// [MS-RPCH] 2.2.4.2.
pub(crate) fn encode_rts_conn_a1(
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
pub(crate) fn encode_rts_conn_b1(
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
            + 20, /* association group id */
    );
    encode_rts_u32_command(&mut commands, RTS_COMMAND_VERSION, RTS_VERSION);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, virtual_connection_cookie);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, in_channel_cookie);
    encode_rts_u32_command(&mut commands, RTS_COMMAND_CHANNEL_LIFETIME, channel_lifetime);
    encode_rts_u32_command(&mut commands, RTS_COMMAND_CLIENT_KEEPALIVE, client_keepalive);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_ASSOCIATION_GROUP_ID, association_group_id);
    encode_rts_pdu(RTS_FLAG_NONE, 6, commands)
}

/// Encodes the client IN_R1/A1 RTS PDU for an IN-channel successor.
///
/// [MS-RPCH] 2.2.4.10.
pub(crate) fn encode_rts_in_recycle_a1(
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

/// Encodes the client IN_R1/A5 RTS PDU on the predecessor IN channel.
///
/// [MS-RPCH] 2.2.4.14.
pub(crate) fn encode_rts_in_recycle_a5(successor_channel_cookie: RtsCookie) -> Result<Vec<u8>, RpcPduError> {
    let mut commands = Vec::with_capacity(20 /* successor IN channel cookie */);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, successor_channel_cookie);
    encode_rts_pdu(RTS_FLAG_NONE, 1, commands)
}

/// Validates an OUT_R1/A2 RTS PDU that starts OUT-channel recycling.
///
/// [MS-RPCH] 2.2.4.24 and 3.2.2.5.6.
pub(crate) fn decode_rts_out_recycle_a2(source: &[u8]) -> Result<(), RpcPduError> {
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

/// Encodes the client OUT_R1/A3 RTS PDU for an OUT-channel successor.
///
/// [MS-RPCH] 2.2.4.25.
pub(crate) fn encode_rts_out_recycle_a3(
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

/// Decodes and validates an OUT_R1/A6 RTS PDU sent to the client.
///
/// [MS-RPCH] 2.2.4.28 and 3.2.2.5.7.
pub(crate) fn decode_rts_out_recycle_a6(source: &[u8]) -> Result<RtsOutRecycleA6, RpcPduError> {
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

/// Encodes the client OUT_R1/A7 RTS PDU on the default IN channel.
///
/// [MS-RPCH] 2.2.4.29.
pub(crate) fn encode_rts_out_recycle_a7(successor_channel_cookie: RtsCookie) -> Result<Vec<u8>, RpcPduError> {
    let mut commands = Vec::with_capacity(
        8 /* destination */
            + 20, /* successor OUT channel cookie */
    );
    encode_rts_u32_command(&mut commands, RTS_COMMAND_DESTINATION, RTS_DESTINATION_FD_SERVER);
    encode_rts_cookie_command(&mut commands, RTS_COMMAND_COOKIE, successor_channel_cookie);
    encode_rts_pdu(RTS_FLAG_OUT_CHANNEL, 2, commands)
}

/// Validates an OUT_R1/A10 RTS PDU that confirms a successor OUT channel.
///
/// [MS-RPCH] 2.2.4.32 and 3.2.2.5.8.
pub(crate) fn decode_rts_out_recycle_a10(source: &[u8]) -> Result<(), RpcPduError> {
    let body = decode_rts_pdu(source, RTS_FLAG_NONE, 1, 4)?;
    decode_rts_empty_command(body, 0, RTS_COMMAND_ANCE)?;
    Ok(())
}

/// Encodes the client OUT_R1/A11 RTS PDU for the successor OUT channel.
///
/// [MS-RPCH] 2.2.4.33.
pub(crate) fn encode_rts_out_recycle_a11() -> Result<Vec<u8>, RpcPduError> {
    encode_rts_pdu(RTS_FLAG_NONE, 1, RTS_COMMAND_ANCE.to_le_bytes().to_vec())
}

/// Decodes and validates a server CONN/A3 RTS PDU.
///
/// [MS-RPCH] 2.2.4.4.
pub(crate) fn decode_rts_conn_a3(source: &[u8]) -> Result<RtsConnA3, RpcPduError> {
    let body = decode_rts_pdu(source, RTS_FLAG_NONE, 1, 8)?;
    let connection_timeout = decode_rts_u32_command(body, 0, RTS_COMMAND_CONNECTION_TIMEOUT)?;
    validate_rts_connection_timeout(connection_timeout)?;

    Ok(RtsConnA3 { connection_timeout })
}

/// Decodes and validates a server CONN/C2 RTS PDU.
///
/// [MS-RPCH] 2.2.4.9.
pub(crate) fn decode_rts_conn_c2(source: &[u8]) -> Result<RtsConnC2, RpcPduError> {
    let body = decode_rts_pdu(source, RTS_FLAG_NONE, 3, 24)?;
    let version = decode_rts_u32_command(body, 0, RTS_COMMAND_VERSION)?;
    let receive_window_size = decode_rts_u32_command(body, 8, RTS_COMMAND_RECEIVE_WINDOW_SIZE)?;
    let connection_timeout = decode_rts_u32_command(body, 16, RTS_COMMAND_CONNECTION_TIMEOUT)?;
    validate_rts_receive_window_size(receive_window_size)?;
    validate_rts_connection_timeout(connection_timeout)?;

    Ok(RtsConnC2 {
        version,
        receive_window_size,
        connection_timeout,
    })
}

/// Decodes and validates an IN_R1/A4 RTS PDU sent to the client.
///
/// [MS-RPCH] 2.2.4.13 and 3.2.2.5.5.
pub(crate) fn decode_rts_in_recycle_a4(source: &[u8]) -> Result<RtsInRecycleA4, RpcPduError> {
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

/// Encodes the client Ping RTS PDU.
///
/// [MS-RPCH] 2.2.4.49.
pub(crate) fn encode_rts_ping() -> Result<Vec<u8>, RpcPduError> {
    encode_rts_pdu(RTS_FLAG_PING, 0, Vec::new())
}

/// Decodes and validates a Ping RTS PDU.
///
/// [MS-RPCH] 2.2.4.49.
pub(crate) fn decode_rts_ping(source: &[u8]) -> Result<(), RpcPduError> {
    let _ = decode_rts_pdu(source, RTS_FLAG_PING, 0, 0)?;
    Ok(())
}

/// Encodes a flow-control acknowledgement for data received on an RPCH channel.
///
/// [MS-RPCH] 2.2.4.50.
pub(crate) fn encode_rts_flow_control_ack(ack: RtsFlowControlAck) -> Result<Vec<u8>, RpcPduError> {
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

/// Decodes and validates a flow-control acknowledgement.
///
/// [MS-RPCH] 2.2.4.50.
pub(crate) fn decode_rts_flow_control_ack(source: &[u8]) -> Result<RtsFlowControlAck, RpcPduError> {
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
    Ok(RtsFlowControlAck {
        bytes_received: read_u32(body, 4)?,
        available_window: read_u32(body, 8)?,
        channel_cookie,
    })
}

/// The reason returned by an RPC bind_nak PDU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RpcBindNakReason(pub(crate) u16);

impl fmt::Display for RpcBindNakReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            0 => f.write_str("reason not specified"),
            1 => f.write_str("temporary congestion"),
            2 => f.write_str("local limit exceeded"),
            4 => f.write_str("protocol version not supported"),
            8 => f.write_str("authentication type not recognized"),
            9 => f.write_str("invalid checksum"),
            value => write!(f, "unknown provider rejection reason {value}"),
        }
    }
}

/// A decoded RPC bind_nak PDU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RpcBindNak {
    pub(crate) call_id: u32,
    pub(crate) reason: RpcBindNakReason,
    pub(crate) supported_versions: Vec<RpcSyntaxVersion>,
    pub(crate) extended_error_signature: Option<Uuid>,
}

/// Encodes a single-context, unauthenticated TS Gateway RPC bind PDU.
///
/// [MS-TSGU] 1.9.1 and Appendix A / [MS-RPCE] 2.2.2.7 / [C706] 12.6.
pub(crate) fn encode_tsgu_bind(call_id: u32, fragment_sizes: RpcFragmentSizes) -> Result<Vec<u8>, RpcPduError> {
    ensure_pdu_fits(RPC_BIND_PDU_SIZE, fragment_sizes.max_xmit)?;

    encode_unprotected_pdu(PTYPE_BIND, call_id, encode_tsgu_bind_body(fragment_sizes))
}

/// Encodes a single-context, packet-integrity NTLM TS Gateway RPC bind PDU.
///
/// [MS-RPCE] 2.2.2.3 and 2.2.2.11-12 / [MS-TSGU] 3.2.5.
pub(crate) fn encode_tsgu_bind_with_ntlm_auth(
    call_id: u32,
    fragment_sizes: RpcFragmentSizes,
    type1_token: &[u8],
) -> Result<Vec<u8>, RpcPduError> {
    let pdu = encode_authenticated_pdu(PTYPE_BIND, call_id, encode_tsgu_bind_body(fragment_sizes), type1_token)?;
    ensure_pdu_fits(pdu.len(), fragment_sizes.max_xmit)?;
    Ok(pdu)
}

/// Decodes a bind_ack and validates the offered TS Gateway presentation context.
pub(crate) fn decode_tsgu_bind_ack(
    source: &[u8],
    offered_fragment_sizes: RpcFragmentSizes,
) -> Result<RpcBinding, RpcPduError> {
    let (header, body) = decode_unprotected_single_fragment(source, PTYPE_BIND_ACK, offered_fragment_sizes.max_recv)?;
    decode_tsgu_bind_ack_body(header, body, offered_fragment_sizes)
}

/// Decodes an authenticated bind_ack, validates its NTLM verifier, and returns
/// the Type-2 token for [`RpcNtlmAuth::continue_token`].
pub(crate) fn decode_tsgu_bind_ack_with_ntlm_auth<'a>(
    source: &'a [u8],
    offered_fragment_sizes: RpcFragmentSizes,
) -> Result<RpcAuthenticatedBindAck<'a>, RpcPduError> {
    let (header, body, token) =
        decode_authenticated_single_fragment(source, PTYPE_BIND_ACK, offered_fragment_sizes.max_recv, true)?;
    let binding = decode_tsgu_bind_ack_body(header, body, offered_fragment_sizes)?;
    Ok(RpcAuthenticatedBindAck { binding, token })
}

/// Encodes the Type-3 continuation token in an rpc_auth_3 PDU.
///
/// [MS-RPCE] 2.2.2.10-12.
pub(crate) fn encode_rpc_auth_3(
    call_id: u32,
    fragment_sizes: RpcFragmentSizes,
    type3_token: &[u8],
) -> Result<Vec<u8>, RpcPduError> {
    let pdu = encode_authenticated_pdu(PTYPE_RPC_AUTH_3, call_id, vec![0; 4], type3_token)?;
    ensure_pdu_fits(pdu.len(), fragment_sizes.max_xmit)?;
    Ok(pdu)
}

fn encode_tsgu_bind_body(fragment_sizes: RpcFragmentSizes) -> Vec<u8> {
    let mut body = Vec::with_capacity(RPC_BIND_BODY_SIZE);
    body.extend_from_slice(&fragment_sizes.max_xmit.to_le_bytes());
    body.extend_from_slice(&fragment_sizes.max_recv.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes()); // assoc_group_id
    body.push(1); // n_context_elem
    body.push(0); // reserved
    body.extend_from_slice(&0u16.to_le_bytes()); // reserved2
    body.extend_from_slice(&RPC_CONTEXT_ID.to_le_bytes());
    body.push(1); // n_transfer_syn
    body.push(0); // reserved
    encode_syntax_identifier(&mut body, TSPROXY_RPC_INTERFACE_ID, TSPROXY_RPC_INTERFACE_VERSION);
    encode_syntax_identifier(&mut body, NDR32_TRANSFER_SYNTAX_ID, NDR32_TRANSFER_SYNTAX_VERSION);

    debug_assert_eq!(body.len(), RPC_BIND_BODY_SIZE);
    body
}

fn decode_tsgu_bind_ack_body(
    header: RpcCommonHeader,
    body: &[u8],
    offered_fragment_sizes: RpcFragmentSizes,
) -> Result<RpcBinding, RpcPduError> {
    if body.len() < 10 {
        return Err(RpcPduError::Truncated {
            actual: body.len(),
            required: 10,
        });
    }

    let server_max_xmit = read_u16(body, 0)?;
    let server_max_recv = read_u16(body, 2)?;
    let server_fragment_sizes = RpcFragmentSizes::new(server_max_xmit, server_max_recv)?;
    let secondary_address_length = usize::from(read_u16(body, 8)?);
    let result_list_offset = 10usize
        .checked_add(secondary_address_length)
        .and_then(|offset| offset.checked_add(3))
        .map(|offset| offset & !3)
        .ok_or(RpcPduError::LengthOverflow)?;
    let result_list_header_end = result_list_offset.checked_add(4).ok_or(RpcPduError::LengthOverflow)?;
    let result_list = body
        .get(result_list_offset..result_list_header_end)
        .ok_or(RpcPduError::Truncated {
            actual: body.len(),
            required: result_list_header_end,
        })?;

    if result_list[0] != 1 {
        return Err(RpcPduError::UnexpectedPresentationContextCount { actual: result_list[0] });
    }

    let result_offset = result_list_header_end;
    let result_end = result_offset.checked_add(24).ok_or(RpcPduError::LengthOverflow)?;
    let result = body.get(result_offset..result_end).ok_or(RpcPduError::Truncated {
        actual: body.len(),
        required: result_end,
    })?;
    if result_end != body.len() {
        return Err(RpcPduError::Truncated {
            actual: body.len(),
            required: result_end,
        });
    }

    let result_code = read_u16(result, 0)?;
    let reason = read_u16(result, 2)?;
    if result_code != 0 || reason != 0 {
        return Err(RpcPduError::PresentationContextRejected {
            result: result_code,
            reason,
        });
    }

    let transfer_syntax_bytes = result.get(4..24).ok_or(RpcPduError::Truncated {
        actual: result.len(),
        required: 24,
    })?;
    let transfer_syntax = (
        Uuid::from_bytes_le(
            transfer_syntax_bytes[..16]
                .try_into()
                .map_err(|_| RpcPduError::LengthOverflow)?,
        ),
        RpcSyntaxVersion::new(
            read_u16(transfer_syntax_bytes, 16)?,
            read_u16(transfer_syntax_bytes, 18)?,
        ),
    );
    if transfer_syntax != (NDR32_TRANSFER_SYNTAX_ID, NDR32_TRANSFER_SYNTAX_VERSION) {
        return Err(RpcPduError::UnexpectedTransferSyntax {
            identifier: transfer_syntax.0,
            version: transfer_syntax.1,
        });
    }

    RpcFragmentSizes::new(
        offered_fragment_sizes.max_xmit.min(server_fragment_sizes.max_recv),
        offered_fragment_sizes.max_recv.min(server_fragment_sizes.max_xmit),
    )
    .map(|fragment_sizes| RpcBinding {
        fragment_sizes,
        call_id: header.call_id,
    })
}

/// Decodes an RPC bind_nak PDU and preserves its advertised protocol versions.
pub(crate) fn decode_bind_nak(source: &[u8]) -> Result<RpcBindNak, RpcPduError> {
    let (header, body) = decode_unprotected_single_fragment(source, PTYPE_BIND_NAK, u16::MAX)?;
    if body.len() < 3 {
        return Err(RpcPduError::Truncated {
            actual: body.len(),
            required: 3,
        });
    }

    let reason = RpcBindNakReason(read_u16(body, 0)?);
    let versions_length = usize::from(body[2]);
    let expected_length = 3usize
        .checked_add(versions_length.checked_mul(2).ok_or(RpcPduError::LengthOverflow)?)
        .ok_or(RpcPduError::LengthOverflow)?;
    if body.len() < expected_length || (body.len() > expected_length && body.len() < expected_length + 16) {
        return Err(RpcPduError::InvalidBindNakVersionsLength {
            actual: body.len(),
            expected: expected_length,
        });
    }

    let supported_versions = body[3..]
        .get(..versions_length * 2)
        .ok_or(RpcPduError::LengthOverflow)?
        .chunks_exact(2)
        .map(|version| RpcSyntaxVersion::new(u16::from(version[0]), u16::from(version[1])))
        .collect();
    let extended_error_signature = match body.get(expected_length..expected_length + 16) {
        Some(signature) => Some(Uuid::from_bytes_le(
            signature.try_into().map_err(|_| RpcPduError::LengthOverflow)?,
        )),
        None => None,
    };
    Ok(RpcBindNak {
        call_id: header.call_id,
        reason,
        supported_versions,
        extended_error_signature,
    })
}

/// Encodes one complete, unauthenticated TS Gateway request PDU.
///
/// The current runtime emits only a first-and-last fragment. Callers must split
/// larger stubs before this layer gains DCE/RPC fragment reassembly support.
pub(crate) fn encode_rpc_request(
    call_id: u32,
    opnum: u16,
    stub: &[u8],
    fragment_sizes: RpcFragmentSizes,
) -> Result<Vec<u8>, RpcPduError> {
    let body = encode_rpc_request_body(opnum, stub)?;
    let pdu_length = RPC_COMMON_HEADER_SIZE
        .checked_add(body.len())
        .ok_or(RpcPduError::LengthOverflow)?;
    ensure_pdu_fits(pdu_length, fragment_sizes.max_xmit)?;

    encode_unprotected_pdu(PTYPE_REQUEST, call_id, body)
}

/// Encodes an unauthenticated TS Gateway request into DCE/RPC fragments.
///
/// Each fragment advertises the remaining stub size through `alloc_hint`, as
/// required for interoperable DCE/RPC fragmentation.
///
/// [MS-RPCE] 2.2.2.6.
pub(crate) fn encode_rpc_request_fragments(
    call_id: u32,
    opnum: u16,
    stub: &[u8],
    fragment_sizes: RpcFragmentSizes,
) -> Result<Vec<Vec<u8>>, RpcPduError> {
    let maximum_stub_size = maximum_unprotected_request_stub_size(fragment_sizes.max_xmit)?;
    rpc_request_fragment_bodies(opnum, stub, maximum_stub_size)?
        .into_iter()
        .map(|(pfc_flags, body)| encode_unprotected_pdu_with_flags(PTYPE_REQUEST, pfc_flags, call_id, body))
        .collect()
}

/// Encodes and signs one complete packet-integrity protected TS Gateway request.
///
/// The request remains single-fragment until the RPC runtime gains fragment
/// reassembly. The NTLM signature covers the complete common header, padded
/// request body, and security trailer.
pub(crate) fn encode_rpc_request_with_ntlm_auth(
    auth: &mut RpcNtlmAuth,
    sequence_number: u32,
    call_id: u32,
    opnum: u16,
    stub: &[u8],
    fragment_sizes: RpcFragmentSizes,
) -> Result<Vec<u8>, Error> {
    let body =
        encode_rpc_request_body(opnum, stub).map_err(|e| Error::custom("encode authenticated rpc request", e))?;
    let mut pdu =
        encode_authenticated_pdu_with_flags(PTYPE_REQUEST, PFC_FIRST_FRAG | PFC_LAST_FRAG, call_id, body, &[0; 16])
            .map_err(|e| Error::custom("encode authenticated rpc request", e))?;
    ensure_pdu_fits(pdu.len(), fragment_sizes.max_xmit)
        .map_err(|e| Error::custom("encode authenticated rpc request", e))?;

    let signature_offset = pdu
        .len()
        .checked_sub(16)
        .ok_or_else(|| Error::new("encode authenticated rpc request", GwErrorKind::Encode))?;
    let signature = auth.make_signature(&mut pdu[..signature_offset], sequence_number)?;
    pdu[signature_offset..].copy_from_slice(&signature);
    Ok(pdu)
}

/// Encodes and signs a TS Gateway request split across packet-integrity fragments.
///
/// The supplied sequence number is advanced once for every emitted fragment.
///
/// [MS-RPCE] 2.2.2.6.
pub(crate) fn encode_rpc_request_fragments_with_ntlm_auth(
    auth: &mut RpcNtlmAuth,
    sequence_number: &mut u32,
    call_id: u32,
    opnum: u16,
    stub: &[u8],
    fragment_sizes: RpcFragmentSizes,
) -> Result<Vec<Vec<u8>>, Error> {
    let maximum_stub_size = maximum_authenticated_request_stub_size(fragment_sizes.max_xmit)
        .map_err(|e| Error::custom("encode authenticated rpc request fragments", e))?;
    let initial_sequence_number = *sequence_number;
    let request_bodies = rpc_request_fragment_bodies(opnum, stub, maximum_stub_size)
        .map_err(|e| Error::custom("encode authenticated rpc request fragments", e))?;
    let mut fragments = Vec::with_capacity(request_bodies.len());
    for (fragment_index, (pfc_flags, body)) in request_bodies.into_iter().enumerate() {
        let mut pdu = encode_authenticated_pdu_with_flags(PTYPE_REQUEST, pfc_flags, call_id, body, &[0; 16])
            .map_err(|e| Error::custom("encode authenticated rpc request fragments", e))?;
        ensure_pdu_fits(pdu.len(), fragment_sizes.max_xmit)
            .map_err(|e| Error::custom("encode authenticated rpc request fragments", e))?;
        let signature_offset = pdu
            .len()
            .checked_sub(16)
            .ok_or_else(|| Error::new("encode authenticated rpc request fragments", GwErrorKind::Encode))?;
        let fragment_index = u32::try_from(fragment_index)
            .map_err(|_| Error::new("encode authenticated rpc request fragments", GwErrorKind::Encode))?;
        let signature = auth.make_signature(
            &mut pdu[..signature_offset],
            initial_sequence_number.wrapping_add(fragment_index),
        )?;
        pdu[signature_offset..].copy_from_slice(&signature);
        fragments.push(pdu);
    }
    *sequence_number = initial_sequence_number.wrapping_add(
        u32::try_from(fragments.len())
            .map_err(|_| Error::new("encode authenticated rpc request fragments", GwErrorKind::Encode))?,
    );
    Ok(fragments)
}

/// Decodes one complete, unauthenticated RPC response PDU.
pub(crate) fn decode_rpc_response<'a>(
    source: &'a [u8],
    maximum_fragment_size: u16,
) -> Result<RpcResponse<'a>, RpcPduError> {
    let (header, body) = decode_unprotected_single_fragment(source, PTYPE_RESPONSE, maximum_fragment_size)?;
    let response = decode_rpc_response_body(header, body)?;
    validate_single_response_alloc_hint(response)?;
    Ok(response)
}

/// Decodes one unauthenticated RPC response fragment for a response reassembler.
pub(crate) fn decode_rpc_response_fragment<'a>(
    source: &'a [u8],
    maximum_fragment_size: u16,
) -> Result<RpcResponse<'a>, RpcPduError> {
    let (header, body) = decode_unprotected_fragment(source, PTYPE_RESPONSE, maximum_fragment_size)?;
    decode_rpc_response_body(header, body)
}

/// Verifies and decodes one packet-integrity protected RPC response PDU.
pub(crate) fn decode_rpc_response_with_ntlm_auth<'a>(
    auth: &mut RpcNtlmAuth,
    sequence_number: u32,
    source: &'a [u8],
    maximum_fragment_size: u16,
) -> Result<RpcResponse<'a>, Error> {
    let (header, body, signature) =
        decode_authenticated_single_fragment(source, PTYPE_RESPONSE, maximum_fragment_size, false)
            .map_err(|e| Error::custom("decode authenticated rpc response", e))?;
    let signature_offset = usize::from(header.fragment_length)
        .checked_sub(usize::from(header.auth_length))
        .ok_or_else(|| Error::new("decode authenticated rpc response", GwErrorKind::Decode))?;
    let mut protected_data = source[..signature_offset].to_vec();
    let mut signature = signature.to_vec();
    auth.verify_signature(&mut protected_data, &mut signature, sequence_number)?;

    let response =
        decode_rpc_response_body(header, body).map_err(|e| Error::custom("decode authenticated rpc response", e))?;
    validate_single_response_alloc_hint(response).map_err(|e| Error::custom("decode authenticated rpc response", e))?;
    Ok(response)
}

/// Verifies and decodes one packet-integrity protected RPC response fragment.
pub(crate) fn decode_rpc_response_fragment_with_ntlm_auth<'a>(
    auth: &mut RpcNtlmAuth,
    sequence_number: u32,
    source: &'a [u8],
    maximum_fragment_size: u16,
) -> Result<RpcResponse<'a>, Error> {
    let (header, body, signature) = decode_authenticated_fragment(source, PTYPE_RESPONSE, maximum_fragment_size, false)
        .map_err(|e| Error::custom("decode authenticated rpc response fragment", e))?;
    let signature_offset = usize::from(header.fragment_length)
        .checked_sub(usize::from(header.auth_length))
        .ok_or_else(|| Error::new("decode authenticated rpc response fragment", GwErrorKind::Decode))?;
    let mut protected_data = source[..signature_offset].to_vec();
    let mut signature = signature.to_vec();
    auth.verify_signature(&mut protected_data, &mut signature, sequence_number)?;

    decode_rpc_response_body(header, body).map_err(|e| Error::custom("decode authenticated rpc response fragment", e))
}

fn encode_rpc_request_body(opnum: u16, stub: &[u8]) -> Result<Vec<u8>, RpcPduError> {
    let alloc_hint = u32::try_from(stub.len()).map_err(|_| RpcPduError::LengthOverflow)?;
    encode_rpc_request_body_with_alloc_hint(opnum, alloc_hint, stub)
}

fn encode_rpc_request_body_with_alloc_hint(opnum: u16, alloc_hint: u32, stub: &[u8]) -> Result<Vec<u8>, RpcPduError> {
    let body_length = 8usize.checked_add(stub.len()).ok_or(RpcPduError::LengthOverflow)?;
    let mut body = Vec::with_capacity(body_length);
    body.extend_from_slice(&alloc_hint.to_le_bytes());
    body.extend_from_slice(&RPC_CONTEXT_ID.to_le_bytes());
    body.extend_from_slice(&opnum.to_le_bytes());
    body.extend_from_slice(stub);
    Ok(body)
}

fn rpc_request_fragment_bodies(
    opnum: u16,
    stub: &[u8],
    maximum_stub_size: usize,
) -> Result<Vec<(u8, Vec<u8>)>, RpcPduError> {
    if maximum_stub_size == 0 && !stub.is_empty() {
        return Err(RpcPduError::FragmentTooLarge {
            actual: RPC_COMMON_HEADER_SIZE
                + 8 /* request header */
                + 1, /* stub byte */
            maximum: u16::try_from(RPC_COMMON_HEADER_SIZE).expect("header size fits in u16") + 8,
        });
    }

    let mut fragments = Vec::new();
    let mut offset = 0;
    loop {
        let remaining = stub.len().checked_sub(offset).ok_or(RpcPduError::LengthOverflow)?;
        let fragment_stub_size = remaining.min(maximum_stub_size);
        let is_last = fragment_stub_size == remaining;
        let mut pfc_flags = if offset == 0 { PFC_FIRST_FRAG } else { 0 };
        if is_last {
            pfc_flags |= PFC_LAST_FRAG;
        }
        let alloc_hint = u32::try_from(remaining).map_err(|_| RpcPduError::LengthOverflow)?;
        let body =
            encode_rpc_request_body_with_alloc_hint(opnum, alloc_hint, &stub[offset..offset + fragment_stub_size])?;
        fragments.push((pfc_flags, body));
        if is_last {
            return Ok(fragments);
        }
        offset = offset
            .checked_add(fragment_stub_size)
            .ok_or(RpcPduError::LengthOverflow)?;
    }
}

fn maximum_unprotected_request_stub_size(maximum_fragment_size: u16) -> Result<usize, RpcPduError> {
    usize::from(maximum_fragment_size)
        .checked_sub(RPC_COMMON_HEADER_SIZE + 8 /* request header */)
        .ok_or(RpcPduError::FragmentTooLarge {
            actual: RPC_COMMON_HEADER_SIZE + 8,
            maximum: maximum_fragment_size,
        })
}

fn maximum_authenticated_request_stub_size(maximum_fragment_size: u16) -> Result<usize, RpcPduError> {
    const AUTHENTICATED_PDU_OVERHEAD: usize = RPC_COMMON_HEADER_SIZE + RPC_SEC_TRAILER_SIZE + 16 /* signature */;
    let maximum_body_size = usize::from(maximum_fragment_size)
        .checked_sub(AUTHENTICATED_PDU_OVERHEAD)
        .ok_or(RpcPduError::FragmentTooLarge {
            actual: AUTHENTICATED_PDU_OVERHEAD + 16, /* minimum padded request body */
            maximum: maximum_fragment_size,
        })?;
    let maximum_padded_body_size = maximum_body_size & !15;
    maximum_padded_body_size
        .checked_sub(8 /* request header */)
        .ok_or(RpcPduError::FragmentTooLarge {
            actual: AUTHENTICATED_PDU_OVERHEAD + 16, /* minimum padded request body */
            maximum: maximum_fragment_size,
        })
}

fn decode_rpc_response_body<'a>(header: RpcCommonHeader, body: &'a [u8]) -> Result<RpcResponse<'a>, RpcPduError> {
    let request_header = body.get(..8).ok_or(RpcPduError::Truncated {
        actual: body.len(),
        required: 8,
    })?;
    let alloc_hint = read_u32(request_header, 0)?;
    let context_id = read_u16(request_header, 4)?;
    if context_id != RPC_CONTEXT_ID {
        return Err(RpcPduError::UnexpectedContextId { actual: context_id });
    }
    let stub = &body[8..];
    Ok(RpcResponse {
        call_id: header.call_id,
        pfc_flags: header.pfc_flags,
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

/// Decodes one complete, unauthenticated RPC fault PDU.
pub(crate) fn decode_rpc_fault<'a>(source: &'a [u8], maximum_fragment_size: u16) -> Result<RpcFault<'a>, RpcPduError> {
    let (header, body) = decode_unprotected_single_fragment(source, PTYPE_FAULT, maximum_fragment_size)?;
    let fault_header = body.get(..16).ok_or(RpcPduError::Truncated {
        actual: body.len(),
        required: 16,
    })?;
    let context_id = read_u16(fault_header, 4)?;
    if context_id != RPC_CONTEXT_ID {
        return Err(RpcPduError::UnexpectedContextId { actual: context_id });
    }

    Ok(RpcFault {
        call_id: header.call_id,
        alloc_hint: read_u32(fault_header, 0)?,
        cancel_count: fault_header[6],
        reserved: fault_header[7],
        status: read_u32(fault_header, 8)?,
        reserved2: read_u32(fault_header, 12)?,
        stub: &body[16..],
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

fn decode_rts_empty_command(source: &[u8], offset: usize, expected_type: u32) -> Result<(), RpcPduError> {
    let command_type = read_u32(source, offset)?;
    if command_type != expected_type {
        return Err(RpcPduError::UnexpectedRtsCommandType {
            expected: expected_type,
            actual: command_type,
        });
    }
    Ok(())
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

fn decode_unprotected_fragment(
    source: &[u8],
    expected_ptype: u8,
    maximum_fragment_size: u16,
) -> Result<(RpcCommonHeader, &[u8]), RpcPduError> {
    let header = RpcCommonHeader::decode(source)?;
    if header.ptype != expected_ptype {
        return Err(RpcPduError::UnexpectedPduType {
            expected: expected_ptype,
            actual: header.ptype,
        });
    }
    if header.fragment_length > maximum_fragment_size {
        return Err(RpcPduError::FragmentExceedsMaximum {
            fragment_length: header.fragment_length,
            maximum: maximum_fragment_size,
        });
    }
    if header.auth_length != 0 {
        return Err(RpcPduError::AuthenticationUnsupported {
            auth_length: header.auth_length,
        });
    }

    Ok((
        header,
        &source[RPC_COMMON_HEADER_SIZE..usize::from(header.fragment_length)],
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

fn encode_authenticated_pdu(ptype: u8, call_id: u32, body: Vec<u8>, token: &[u8]) -> Result<Vec<u8>, RpcPduError> {
    encode_authenticated_pdu_with_flags(
        ptype,
        PFC_FIRST_FRAG | PFC_LAST_FRAG | PFC_SUPPORT_HEADER_SIGN | PFC_CONC_MPX,
        call_id,
        body,
        token,
    )
}

fn encode_authenticated_pdu_with_flags(
    ptype: u8,
    pfc_flags: u8,
    call_id: u32,
    mut body: Vec<u8>,
    token: &[u8],
) -> Result<Vec<u8>, RpcPduError> {
    if token.is_empty() {
        return Err(RpcPduError::EmptyAuthenticationToken);
    }

    let auth_length = u16::try_from(token.len()).map_err(|_| RpcPduError::LengthOverflow)?;
    // The security trailer must be 16-byte aligned with respect to the PDU body
    // ([MS-RPCE] 2.2.2.11); pad the body so it is.
    let padding_length = (16 - body.len() % 16) % 16;
    body.resize(
        body.len()
            .checked_add(padding_length)
            .ok_or(RpcPduError::LengthOverflow)?,
        0,
    );
    let header = RpcCommonHeader::encode(ptype, pfc_flags, call_id, body.len(), auth_length)?;
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

fn decode_authenticated_fragment(
    source: &[u8],
    expected_ptype: u8,
    maximum_fragment_size: u16,
    require_support_header_sign: bool,
) -> Result<(RpcCommonHeader, &[u8], &[u8]), RpcPduError> {
    let header = RpcCommonHeader::decode(source)?;
    if header.ptype != expected_ptype {
        return Err(RpcPduError::UnexpectedPduType {
            expected: expected_ptype,
            actual: header.ptype,
        });
    }
    if require_support_header_sign && header.pfc_flags & PFC_SUPPORT_HEADER_SIGN == 0 {
        return Err(RpcPduError::MissingSupportHeaderSign);
    }
    if header.fragment_length > maximum_fragment_size {
        return Err(RpcPduError::FragmentExceedsMaximum {
            fragment_length: header.fragment_length,
            maximum: maximum_fragment_size,
        });
    }
    if header.auth_length == 0 {
        return Err(RpcPduError::AuthenticationRequired);
    }

    let fragment_length = usize::from(header.fragment_length);
    let trailer_offset = fragment_length
        .checked_sub(usize::from(header.auth_length))
        .and_then(|offset| offset.checked_sub(RPC_SEC_TRAILER_SIZE))
        .ok_or(RpcPduError::InvalidSecurityTrailer {
            fragment_length: header.fragment_length,
            auth_length: header.auth_length,
        })?;
    if trailer_offset < RPC_COMMON_HEADER_SIZE {
        return Err(RpcPduError::InvalidSecurityTrailer {
            fragment_length: header.fragment_length,
            auth_length: header.auth_length,
        });
    }

    let trailer_end = trailer_offset
        .checked_add(RPC_SEC_TRAILER_SIZE)
        .ok_or(RpcPduError::LengthOverflow)?;
    let trailer = source
        .get(trailer_offset..trailer_end)
        .ok_or(RpcPduError::InvalidSecurityTrailer {
            fragment_length: header.fragment_length,
            auth_length: header.auth_length,
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
    let auth_context_id = u32::from_le_bytes(trailer[4..8].try_into().map_err(|_| RpcPduError::LengthOverflow)?);
    if auth_context_id != RPC_AUTH_CONTEXT_ID {
        return Err(RpcPduError::UnexpectedAuthenticationContextId {
            expected: RPC_AUTH_CONTEXT_ID,
            actual: auth_context_id,
        });
    }

    let body_with_padding = &source[RPC_COMMON_HEADER_SIZE..trailer_offset];
    let padding_length = usize::from(trailer[2]);
    // Per MS-RPCE the auth_pad_len field is authoritative for how many bytes of the
    // stub are padding. Real gateways align the stub to NDR (4-byte) boundaries, not
    // the 16-byte boundary our encoder uses, so validate against auth_pad_len and
    // require zero padding bytes instead of assuming a fixed alignment.
    let body_length = body_with_padding
        .len()
        .checked_sub(padding_length)
        .filter(|length| *length != 0)
        .ok_or(RpcPduError::InvalidAuthenticationPadding {
            actual: trailer[2],
            expected: 0,
        })?;
    if body_with_padding[body_length..].iter().any(|&byte| byte != 0) {
        return Err(RpcPduError::NonZeroAuthenticationPadding);
    }

    let token = source
        .get(trailer_end..fragment_length)
        .ok_or(RpcPduError::InvalidSecurityTrailer {
            fragment_length: header.fragment_length,
            auth_length: header.auth_length,
        })?;
    debug_assert_eq!(token.len(), usize::from(header.auth_length));
    Ok((header, &body_with_padding[..body_length], token))
}

fn decode_authenticated_single_fragment(
    source: &[u8],
    expected_ptype: u8,
    maximum_fragment_size: u16,
    require_support_header_sign: bool,
) -> Result<(RpcCommonHeader, &[u8], &[u8]), RpcPduError> {
    let (header, body, token) = decode_authenticated_fragment(
        source,
        expected_ptype,
        maximum_fragment_size,
        require_support_header_sign,
    )?;
    validate_single_fragment(header)?;
    Ok((header, body, token))
}

fn validate_single_fragment(header: RpcCommonHeader) -> Result<(), RpcPduError> {
    if header.pfc_flags & (PFC_FIRST_FRAG | PFC_LAST_FRAG) != PFC_FIRST_FRAG | PFC_LAST_FRAG {
        return Err(RpcPduError::FragmentedPduUnsupported {
            flags: header.pfc_flags,
        });
    }

    Ok(())
}

fn rpc_auth_identity(username: &str, password: &str) -> Result<AuthIdentity, Error> {
    let username = Username::parse(username)
        .or_else(|_| Username::new(username, None))
        .map_err(|e| Error::custom("parse rpc username", e))?;

    Ok(AuthIdentity {
        username,
        password: password.to_owned().into(),
    })
}

fn encode_syntax_identifier(output: &mut Vec<u8>, identifier: Uuid, version: RpcSyntaxVersion) {
    output.extend_from_slice(&identifier.to_bytes_le());
    output.extend_from_slice(&version.major.to_le_bytes());
    output.extend_from_slice(&version.minor.to_le_bytes());
}

fn ensure_pdu_fits(actual: usize, maximum: u16) -> Result<(), RpcPduError> {
    if actual > usize::from(maximum) {
        return Err(RpcPduError::FragmentTooLarge { actual, maximum });
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

/// Server-side PDU encoders for the in-process mock RPC proxy.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    /// CONN/A3 with the given connection timeout ([MS-RPCH] 2.2.3.1).
    pub(crate) fn rts_conn_a3(connection_timeout: u32) -> Vec<u8> {
        let mut commands = Vec::new();
        encode_rts_u32_command(&mut commands, 2 /* ConnectionTimeout */, connection_timeout);
        encode_rts_pdu(RTS_FLAG_NONE, 1, commands).expect("valid A3")
    }

    /// CONN/C2 with the negotiated version, receive window, and timeout ([MS-RPCH] 2.2.3.8).
    pub(crate) fn rts_conn_c2(version: u32, receive_window_size: u32, connection_timeout: u32) -> Vec<u8> {
        let mut commands = Vec::new();
        encode_rts_u32_command(&mut commands, 6 /* Version */, version);
        encode_rts_u32_command(&mut commands, 0 /* ReceiveWindowSize */, receive_window_size);
        encode_rts_u32_command(&mut commands, 2 /* ConnectionTimeout */, connection_timeout);
        encode_rts_pdu(RTS_FLAG_NONE, 3, commands).expect("valid C2")
    }

    /// Unauthenticated bind acknowledgement accepting the NDR32 context.
    pub(crate) fn bind_ack(call_id: u32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&0x2000u16.to_le_bytes()); // max_xmit_frag
        body.extend_from_slice(&0x2000u16.to_le_bytes()); // max_recv_frag
        body.extend_from_slice(&0u32.to_le_bytes()); // assoc group
        body.extend_from_slice(&0u16.to_le_bytes()); // secondary address length (none)
        body.extend_from_slice(&[0, 0]); // NDR alignment
        body.extend_from_slice(&[1, 0, 0, 0]); // one presentation result (u8 + reserved u8 + u16)
        body.extend_from_slice(&[0, 0, 0, 0]); // accepted (result + reason)
        // NDR32 transfer syntax identifier in wire order.
        body.extend_from_slice(&[
            0x04, 0x5d, 0x88, 0x8a, 0xeb, 0x1c, 0xc9, 0x11, 0x9f, 0xe8, 0x08, 0x00, 0x2b, 0x10, 0x48, 0x60,
        ]);
        body.extend_from_slice(&2u32.to_le_bytes()); // NDR32 version
        encode_unprotected_pdu(PTYPE_BIND_ACK, call_id, body).expect("valid bind ack")
    }

    /// Single-fragment unauthenticated RPC response carrying `stub`.
    pub(crate) fn rpc_response(call_id: u32, stub: &[u8]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&u32::try_from(stub.len()).expect("stub length fits").to_le_bytes()); // alloc hint
        body.extend_from_slice(&0u16.to_le_bytes()); // context id
        body.extend_from_slice(&0u8.to_le_bytes()); // cancel count
        body.extend_from_slice(&0u8.to_le_bytes()); // reserved
        body.extend_from_slice(stub);
        encode_unprotected_pdu(PTYPE_RESPONSE, call_id, body).expect("valid rpc response")
    }

    /// `TsProxyCreateTunnel` success response with the given context and tunnel ID.
    pub(crate) fn create_tunnel_response(tunnel_context: &[u8; RpcContextHandle::SIZE], tunnel_id: u32) -> Vec<u8> {
        let nonce = Uuid::from_u128(0x0011_2233_4455_6677_8899_aabb_ccdd_eeff);
        let mut response = Vec::new();
        response.extend_from_slice(&NDR_REFERENT_ID.to_le_bytes()); // TSG packet
        response.extend_from_slice(&TSG_PACKET_TYPE_QUARENC_RESPONSE.to_le_bytes());
        response.extend_from_slice(&TSG_PACKET_TYPE_QUARENC_RESPONSE.to_le_bytes());
        response.extend_from_slice(&(NDR_REFERENT_ID + 4).to_le_bytes()); // QUARENC response
        response.extend_from_slice(&0u32.to_le_bytes()); // flags
        response.extend_from_slice(&0u32.to_le_bytes()); // certificate chain length
        response.extend_from_slice(&0u32.to_le_bytes()); // certificate chain pointer
        response.extend_from_slice(&nonce.into_bytes());
        response.extend_from_slice(&(NDR_REFERENT_ID + 8).to_le_bytes()); // version capabilities
        response.extend_from_slice(&TSG_COMPONENT_ID.to_le_bytes());
        response.extend_from_slice(
            &u16::try_from(TSG_PACKET_TYPE_VERSIONCAPS)
                .expect("packet type fits in u16")
                .to_le_bytes(),
        );
        response.extend_from_slice(&(NDR_REFERENT_ID + 12).to_le_bytes()); // capability array
        response.extend_from_slice(&1u32.to_le_bytes()); // number of capabilities
        response.extend_from_slice(&1u16.to_le_bytes()); // major version
        response.extend_from_slice(&1u16.to_le_bytes()); // minor version
        response.extend_from_slice(&0u16.to_le_bytes()); // quarantine capabilities
        response.extend_from_slice(&0u16.to_le_bytes()); // NDR alignment
        response.extend_from_slice(&1u32.to_le_bytes()); // capability array max count
        response.extend_from_slice(&TSG_CAPABILITY_TYPE_NAP.to_le_bytes());
        response.extend_from_slice(&TSG_CAPABILITY_TYPE_NAP.to_le_bytes());
        response.extend_from_slice(&0x0000_0003u32.to_le_bytes());
        response.extend_from_slice(tunnel_context);
        response.extend_from_slice(&tunnel_id.to_le_bytes());
        response.extend_from_slice(&0u32.to_le_bytes()); // HRESULT
        response
    }

    /// `TsProxyAuthorizeTunnel` success response with empty response data.
    pub(crate) fn authorize_tunnel_response() -> Vec<u8> {
        let mut response = Vec::new();
        response.extend_from_slice(&NDR_REFERENT_ID.to_le_bytes()); // TSG packet
        response.extend_from_slice(&TSG_PACKET_TYPE_RESPONSE.to_le_bytes());
        response.extend_from_slice(&TSG_PACKET_TYPE_RESPONSE.to_le_bytes());
        response.extend_from_slice(&(NDR_REFERENT_ID + 4).to_le_bytes()); // packet response
        response.extend_from_slice(&TSG_PACKET_TYPE_QUARREQUEST.to_le_bytes());
        response.extend_from_slice(&0u32.to_le_bytes()); // reserved
        response.extend_from_slice(&0u32.to_le_bytes()); // response data pointer (none)
        response.extend_from_slice(&0u32.to_le_bytes()); // response data length
        response.extend_from_slice(&0u32.to_le_bytes()); // enable all
        response.extend_from_slice(&0u32.to_le_bytes()); // disable all
        response.extend_from_slice(&0u32.to_le_bytes()); // drive disabled
        response.extend_from_slice(&0u32.to_le_bytes()); // printer disabled
        response.extend_from_slice(&0u32.to_le_bytes()); // port disabled
        response.extend_from_slice(&0u32.to_le_bytes()); // reserved
        response.extend_from_slice(&0u32.to_le_bytes()); // clipboard disabled
        response.extend_from_slice(&0u32.to_le_bytes()); // PnP disabled
        response.extend_from_slice(&0u32.to_le_bytes()); // HRESULT
        response
    }

    /// `TsProxyCreateChannel` success response with the given context and channel ID.
    pub(crate) fn create_channel_response(channel_context: &[u8; RpcContextHandle::SIZE], channel_id: u32) -> Vec<u8> {
        let mut response = Vec::new();
        response.extend_from_slice(channel_context);
        response.extend_from_slice(&channel_id.to_le_bytes());
        response.extend_from_slice(&0u32.to_le_bytes()); // HRESULT
        response
    }

    /// `TsProxySetupReceivePipe` terminal response: the 4-byte return value sent when the
    /// pipe closes or fails ([MS-TSGU] 3.2.6.2.3).
    pub(crate) fn receive_pipe_final_return_value(result: u32) -> Vec<u8> {
        result.to_le_bytes().to_vec()
    }

    /// `TsProxyMakeTunnelCall` response carrying a service message ([MS-TSGU] 2.2.9.2.1.9.1).
    pub(crate) fn make_tunnel_call_service_response(text: &str) -> Vec<u8> {
        let utf16: Vec<u8> = text.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
        let char_count = u32::try_from(text.chars().count() + 1).expect("message length fits");
        let padded_len = (utf16.len() + 3) & !3;
        let mut response = Vec::new();
        response.extend_from_slice(&NDR_REFERENT_ID.to_le_bytes()); // TSG packet
        response.extend_from_slice(&TSG_PACKET_TYPE_MESSAGE.to_le_bytes());
        response.extend_from_slice(&TSG_PACKET_TYPE_MESSAGE.to_le_bytes());
        response.extend_from_slice(&(NDR_REFERENT_ID + 4).to_le_bytes()); // message response
        response.extend_from_slice(&1u32.to_le_bytes()); // message ID
        response.extend_from_slice(&TSG_ASYNC_MESSAGE_SERVICE.to_le_bytes());
        response.extend_from_slice(&1u32.to_le_bytes()); // is message present
        response.extend_from_slice(&(NDR_REFERENT_ID + 8).to_le_bytes()); // service message
        response.extend_from_slice(&1u32.to_le_bytes()); // display mandatory
        response.extend_from_slice(&0u32.to_le_bytes()); // consent mandatory
        response.extend_from_slice(&char_count.to_le_bytes()); // message characters
        response.extend_from_slice(&(NDR_REFERENT_ID + 12).to_le_bytes()); // message buffer
        response.extend_from_slice(&char_count.to_le_bytes()); // NDR array count
        response.extend_from_slice(&utf16);
        response.resize(response.len() + (padded_len - utf16.len()), 0); // NDR alignment
        response.extend_from_slice(&0u32.to_le_bytes()); // HRESULT
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const CHANNEL_CONTEXT: [u8; RpcContextHandle::SIZE] = [
        0, 0, 0, 0, 0x36, 0x41, 0x18, 0x41, 0xdd, 0x2d, 0x84, 0x43, 0x83, 0x63, 0x82, 0xcc, 0xb6, 0xea, 0xf3, 0xf9,
    ];

    fn channel_context() -> NonNullRpcContextHandle {
        RpcContextHandle::from_bytes(&CHANNEL_CONTEXT)
            .expect("valid context handle")
            .require_non_null()
            .expect("non-null context handle")
    }

    #[test]
    fn close_context_encodes_and_validates_the_context_handle_stub() {
        assert_eq!(
            TsProxyCloseContextRequest::new(channel_context()).encode(),
            CHANNEL_CONTEXT
        );

        let response = [CHANNEL_CONTEXT.as_slice(), &[0, 0, 0, 0]].concat();
        assert_eq!(decode_tsgu_close_context_response(&response), Ok(()));

        let mut failed_response = response;
        failed_response[RpcContextHandle::SIZE..].copy_from_slice(&0x8007_0005u32.to_le_bytes());
        assert_eq!(
            decode_tsgu_close_context_response(&failed_response),
            Err(RpcWireError::RpcStatus { value: 0x8007_0005 })
        );
        assert_eq!(
            decode_tsgu_close_context_response(&failed_response[..RpcContextHandle::SIZE]),
            Err(RpcWireError::ResponseLength {
                actual: RpcContextHandle::SIZE,
                expected: RpcContextHandle::SIZE + 4,
            })
        );
    }

    fn completed_rpc_ntlm_auth() -> RpcNtlmAuth {
        let mut auth = RpcNtlmAuth::new(r"CONTOSO\alice", "secret").expect("valid credentials");
        let _type1 = auth.initial_token().expect("offline Type-1 token");
        let type2 = [
            b"NTLMSSP\0".as_slice(),
            &[2, 0, 0, 0],
            &[8, 0, 8, 0, 56, 0, 0, 0],
            &0xe288_82b7u32.to_le_bytes(),
            &[0x26, 0x6e, 0xcd, 0x75, 0xaa, 0x41, 0xe7, 0x6f],
            &[0; 8],
            &[64, 0, 64, 0, 64, 0, 0, 0],
            &[6, 1, 0xb0, 0x1d, 0, 0, 0, 0x0f],
            &[0x57, 0, 0x49, 0, 0x4e, 0, 0x37, 0],
            &[
                2, 0, 8, 0, 0x57, 0, 0x49, 0, 0x4e, 0, 0x37, 0, 1, 0, 8, 0, 0x57, 0, 0x49, 0, 0x4e, 0, 0x37, 0, 4, 0,
                8, 0, 0x77, 0, 0x69, 0, 0x6e, 0, 0x37, 0, 3, 0, 8, 0, 0x77, 0, 0x69, 0, 0x6e, 0, 0x37, 0, 7, 0, 8, 0,
                0xa9, 0x8d, 0x9b, 0x1a, 0x6c, 0xb0, 0xcb, 1, 0, 0, 0, 0,
            ],
        ]
        .concat();
        let _type3 = auth.continue_token(&type2).expect("valid Type-2 token");
        assert!(auth.is_complete());
        auth
    }

    fn conn_a3_pdu() -> Vec<u8> {
        [
            [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
            [28, 0, 0, 0, 0, 0, 0, 0].as_slice(),
            [0, 0, 1, 0].as_slice(),
            [2, 0, 0, 0, 0xc0, 0xd4, 1, 0].as_slice(),
        ]
        .concat()
    }

    fn conn_c2_pdu(version: u32) -> Vec<u8> {
        [
            [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
            [44, 0, 0, 0, 0, 0, 0, 0].as_slice(),
            [0, 0, 3, 0].as_slice(),
            [6, 0, 0, 0].as_slice(),
            version.to_le_bytes().as_slice(),
            [0, 0, 0, 0, 0, 0, 2, 0].as_slice(),
            [2, 0, 0, 0, 0xc0, 0xd4, 1, 0].as_slice(),
        ]
        .concat()
    }

    #[test]
    fn rpc_context_handle_is_exactly_twenty_bytes() {
        assert_eq!(
            RpcContextHandle::from_bytes(&CHANNEL_CONTEXT[..RpcContextHandle::SIZE - 1]),
            Err(RpcWireError::ContextHandleLength {
                actual: RpcContextHandle::SIZE - 1
            })
        );
        assert_eq!(
            RpcContextHandle::from_bytes(&[0; RpcContextHandle::SIZE + 1]),
            Err(RpcWireError::ContextHandleLength {
                actual: RpcContextHandle::SIZE + 1
            })
        );

        let handle = RpcContextHandle::from_bytes(&CHANNEL_CONTEXT).expect("valid context handle");
        let mut encoded = [0; RpcContextHandle::SIZE];
        handle
            .encode(&mut WriteCursor::new(&mut encoded))
            .expect("sufficient output");
        assert_eq!(encoded, CHANNEL_CONTEXT);

        let decoded = RpcContextHandle::decode(&mut ReadCursor::new(&encoded)).expect("sufficient input");
        assert_eq!(decoded, handle);
        assert!(RpcContextHandle::decode(&mut ReadCursor::new(&encoded[..RpcContextHandle::SIZE - 1])).is_err());
    }

    #[test]
    fn null_context_handle_is_rejected_for_channel_requests() {
        let null = RpcContextHandle::from_bytes(&[0; RpcContextHandle::SIZE]).expect("valid null representation");
        assert!(null.is_null());
        assert_eq!(null.require_non_null(), Err(RpcWireError::NullContextHandle));
    }

    #[test]
    fn setup_receive_pipe_contains_only_the_channel_context() {
        let request = TsProxySetupReceivePipeRequest::new(channel_context());
        assert_eq!(request.encode(), CHANNEL_CONTEXT);

        let mut output = [0; RpcContextHandle::SIZE - 1];
        assert_eq!(
            request.encode_into(&mut output),
            Err(RpcWireError::OutputLength {
                actual: output.len(),
                expected: RpcContextHandle::SIZE
            })
        );
    }

    #[test]
    fn send_to_server_encodes_single_buffer_little_endian() {
        let request = TsProxySendToServerRequest::new(channel_context(), &[4, 0, 0, 3]).expect("valid request");
        assert_eq!(
            request.encode().expect("valid encoding"),
            [
                CHANNEL_CONTEXT.as_slice(),
                &[8, 0, 0, 0], // totalDataBytes
                &[1, 0, 0, 0], // numBuffers
                &[4, 0, 0, 0], // buffer1Length
                &[4, 0, 0, 3],
            ]
            .concat()
        );
    }

    #[test]
    fn send_to_server_encodes_three_buffers_little_endian() {
        let mut request = TsProxySendToServerRequest::new(channel_context(), &[1]).expect("valid request");
        request.push_buffer(&[2, 3]).expect("second buffer");
        request.push_buffer(&[4, 5, 6]).expect("third buffer");

        assert_eq!(
            request.encode().expect("valid encoding"),
            [
                CHANNEL_CONTEXT.as_slice(),
                &[18, 0, 0, 0], // totalDataBytes: 1 + 2 + 3 + 3 length fields
                &[3, 0, 0, 0],  // numBuffers
                &[1, 0, 0, 0],  // buffer1Length
                &[2, 0, 0, 0],  // buffer2Length
                &[3, 0, 0, 0],  // buffer3Length
                &[1, 2, 3, 4, 5, 6],
            ]
            .concat()
        );
    }

    #[test]
    fn send_to_server_validates_buffer_count_and_size() {
        assert!(matches!(
            TsProxySendToServerRequest::new(channel_context(), &[]),
            Err(RpcWireError::EmptyFirstBuffer)
        ));

        let mut request = TsProxySendToServerRequest::new(channel_context(), &[1]).expect("valid request");
        request.push_buffer(&[]).expect("second buffer");
        request.push_buffer(&[]).expect("third buffer");
        assert_eq!(request.push_buffer(&[]), Err(RpcWireError::BufferCount { actual: 4 }));

        let payload = vec![0; MAX_RPC_MESSAGE_SIZE];
        let request = TsProxySendToServerRequest::new(channel_context(), &payload).expect("valid request shape");
        assert_eq!(
            request.encoded_len(),
            Err(RpcWireError::RequestTooLarge {
                actual: RpcContextHandle::SIZE + 4 + 4 + 4 + payload.len()
            })
        );

        let payload = vec![0; MAX_RPC_MESSAGE_SIZE - TsProxySendToServerRequest::PREFIX_SIZE - 4];
        let request = TsProxySendToServerRequest::new(channel_context(), &payload).expect("maximum request shape");
        assert_eq!(request.encoded_len(), Ok(MAX_RPC_MESSAGE_SIZE));
    }

    #[test]
    fn create_tunnel_encodes_the_version_caps_and_interface_trailer() {
        let request = TsProxyCreateTunnelRequest::new(0x7856_3412).encode();
        let expected: [&[u8]; 10] = [
            &[0x43, 0x56, 0, 0, 0x43, 0x56, 0, 0, 0, 0, 2, 0],
            &[0x52, 0x54, 0x43, 0x56, 4, 0, 2, 0],
            &[1, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0],
            &[1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0x12, 0x34, 0x56, 0x78],
            &[0x8a, 0xe3, 0x13, 0x71, 0x02, 0xf4, 0x36, 0x71],
            &[1, 0, 4, 0, 1, 0, 0, 0, 2, 0x40, 0x28, 0],
            &TSPROXY_RPC_INTERFACE_ID.to_bytes_le(),
            &[1, 0, 3, 0],
            &NDR32_TRANSFER_SYNTAX_ID.to_bytes_le(),
            &[2, 0, 0, 0],
        ];
        assert_eq!(request, expected.concat());
    }

    #[test]
    fn reauthenticate_tunnel_encodes_the_tunnel_context_and_version_caps() {
        let request = TsProxyReauthenticateTunnelRequest::new(0x1122_3344_5566_7788, 3).encode();
        let expected: [&[u8]; 5] = [
            &[0x50, 0x52, 0, 0, 0x50, 0x52, 0, 0, 0, 0, 2, 0],
            &[
                0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x43, 0x56, 0, 0, 4, 0, 2, 0,
            ],
            &[0x52, 0x54, 0x43, 0x56, 8, 0, 2, 0],
            &[1, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0],
            &[1, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0],
        ];
        assert_eq!(request, expected.concat());
    }

    #[test]
    fn make_tunnel_call_requests_one_queued_server_message() {
        let request = TsProxyMakeTunnelCallRequest::new(channel_context()).encode();
        assert_eq!(
            request.as_slice(),
            [
                CHANNEL_CONTEXT.as_slice(),
                &[1, 0, 0, 0], // TSG_TUNNEL_CALL_ASYNC_MSG_REQUEST
                &[0x52, 0x47, 0, 0, 0x52, 0x47, 0, 0, 0, 0, 2, 0],
                &[1, 0, 0, 0], // maxMessagesPerBatch
            ]
            .concat()
        );
    }

    #[test]
    fn cancel_tunnel_call_uses_the_message_request_stub_and_requires_a_null_response() {
        let request = TsProxyMakeTunnelCallRequest::cancel_pending(channel_context()).encode();
        assert_eq!(
            request.as_slice(),
            [
                CHANNEL_CONTEXT.as_slice(),
                &[2, 0, 0, 0], // TSG_TUNNEL_CANCEL_ASYNC_MSG_REQUEST
                &[0x52, 0x47, 0, 0, 0x52, 0x47, 0, 0, 0, 0, 2, 0],
                &[1, 0, 0, 0], // maxMessagesPerBatch
            ]
            .concat()
        );
        assert_eq!(decode_tsgu_cancel_tunnel_call_response(&[0; 8]), Ok(()));
        assert_eq!(
            decode_tsgu_cancel_tunnel_call_response(&[1, 0, 0, 0, 0, 0, 0, 0]),
            Err(RpcWireError::UnexpectedNdrPointer { actual: 1 })
        );
        assert_eq!(
            decode_tsgu_cancel_tunnel_call_response(&[0, 0, 0, 0, 5, 0, 0, 0]),
            Err(RpcWireError::RpcStatus { value: 5 })
        );
    }

    #[test]
    fn create_channel_encodes_a_single_resource_name_and_endpoint_port() {
        let request = TsProxyCreateChannelRequest::new(channel_context(), "rdp.example", 3389)
            .encode()
            .expect("valid endpoint");
        assert_eq!(
            request,
            [
                CHANNEL_CONTEXT.as_slice(),
                &[0, 0, 2, 0],                           // resourceName
                &[1, 0, 0, 0],                           // numResourceNames
                &[0, 0, 0, 0],                           // alternateResourceNames
                &[0, 0, 0, 0],                           // numAlternateResourceNames and alignment
                &[3, 0, 0x3d, 0x0d],                     // RDP protocol and port
                &[1, 0, 0, 0],                           // resourceName array max count
                &[4, 0, 2, 0],                           // first resourceName
                &[12, 0, 0, 0, 0, 0, 0, 0, 12, 0, 0, 0], // string bounds
                &[
                    b'r', 0, b'd', 0, b'p', 0, b'.', 0, b'e', 0, b'x', 0, b'a', 0, b'm', 0, b'p', 0, b'l', 0, b'e', 0,
                    0, 0,
                ],
            ]
            .concat()
        );
    }

    #[test]
    fn authorize_tunnel_encodes_machine_name_and_optional_health_statement() {
        let request = TsProxyAuthorizeTunnelRequest::new(channel_context(), "client", &[1, 2, 3])
            .encode()
            .expect("valid authorization");
        assert_eq!(
            request,
            [
                CHANNEL_CONTEXT.as_slice(),
                &[0x52, 0x51, 0, 0, 0x52, 0x51, 0, 0], // TSG_PACKET_TYPE_QUARREQUEST
                &[0, 0, 2, 0],                         // packetQuarRequest
                &[0, 0, 0, 0],                         // flags
                &[4, 0, 2, 0],                         // machineName
                &[7, 0, 0, 0],                         // nameLength
                &[8, 0, 2, 0],                         // data
                &[3, 0, 0, 0],                         // dataLen
                &[7, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0], // string bounds
                &[b'c', 0, b'l', 0, b'i', 0, b'e', 0, b'n', 0, b't', 0, 0, 0, 0, 0],
                &[3, 0, 0, 0, 1, 2, 3, 0], // data array and NDR alignment
            ]
            .concat()
        );

        let request = TsProxyAuthorizeTunnelRequest::new(channel_context(), "client", &[])
            .encode()
            .expect("valid authorization without health statement");
        assert_eq!(&request[44..52], &[0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn create_channel_rejects_invalid_resource_names_and_response_statuses() {
        assert_eq!(
            TsProxyCreateChannelRequest::new(channel_context(), "", 3389).encode(),
            Err(RpcWireError::EmptyResourceName)
        );
        assert_eq!(
            TsProxyCreateChannelRequest::new(channel_context(), "rdp\0.example", 3389).encode(),
            Err(RpcWireError::EmbeddedNulInResourceName)
        );

        let response = [CHANNEL_CONTEXT.as_slice(), &[0x12, 0x34, 0x56, 0x78], &[0, 0, 0, 0]].concat();
        assert_eq!(
            decode_tsgu_create_channel_response(&response),
            Ok(TsProxyCreateChannelResponse {
                channel_context: channel_context(),
                channel_id: 0x7856_3412,
            })
        );

        let mut failed_response = response;
        failed_response[24..28].copy_from_slice(&0x8007_59dau32.to_le_bytes());
        assert_eq!(
            decode_tsgu_create_channel_response(&failed_response),
            Err(RpcWireError::RpcStatus { value: 0x8007_59da })
        );
    }

    #[test]
    fn receive_pipe_final_return_value_requires_explicit_byte_order() {
        assert_eq!(
            parse_receive_pipe_final_return_value_stub(&[0x78, 0x56, 0x34, 0x12], RpcStubByteOrder::LittleEndian),
            Ok(0x1234_5678)
        );
        assert_eq!(
            parse_receive_pipe_final_return_value_stub(&[0x12, 0x34, 0x56, 0x78], RpcStubByteOrder::BigEndian),
            Ok(0x1234_5678)
        );
        assert_eq!(
            parse_receive_pipe_final_return_value_stub(&[0; 3], RpcStubByteOrder::LittleEndian),
            Err(RpcWireError::FinalReturnValueLength { actual: 3 })
        );
    }
    #[test]
    fn tsgu_bind_encodes_the_single_ndr32_context_exactly() {
        assert_eq!(
            encode_tsgu_bind(0x7856_3412, RpcFragmentSizes::DEFAULT).expect("valid bind"),
            [
                [5, 0, PTYPE_BIND, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
                &[0x48, 0, 0, 0, 0x12, 0x34, 0x56, 0x78],
                &[0xb8, 0x10, 0xb8, 0x10, 0, 0, 0, 0],
                &[1, 0, 0, 0, 0, 0, 1, 0],
                &[
                    0xdd, 0x65, 0xe2, 0x44, 0xaf, 0x7d, 0xcd, 0x42, 0x85, 0x60, 0x3c, 0xdb, 0x6e, 0x7a, 0x27, 0x29
                ],
                &[1, 0, 3, 0],
                &[
                    0x04, 0x5d, 0x88, 0x8a, 0xeb, 0x1c, 0xc9, 0x11, 0x9f, 0xe8, 0x08, 0x00, 0x2b, 0x10, 0x48, 0x60
                ],
                &[2, 0, 0, 0],
            ]
            .concat()
        );
    }

    #[test]
    fn authenticated_tsgu_bind_has_aligned_security_trailer() {
        let token = [1, 2, 3, 4];
        let bind = encode_tsgu_bind_with_ntlm_auth(0x7856_3412, RpcFragmentSizes::DEFAULT, &token)
            .expect("valid authenticated bind");

        assert_eq!(bind.len(), 92);
        assert_eq!(
            bind,
            [
                [
                    5,
                    0,
                    PTYPE_BIND,
                    PFC_FIRST_FRAG | PFC_LAST_FRAG | PFC_SUPPORT_HEADER_SIGN | PFC_CONC_MPX,
                    0x10,
                    0,
                    0,
                    0
                ]
                .as_slice(),
                &[92, 0, 4, 0, 0x12, 0x34, 0x56, 0x78],
                &encode_tsgu_bind_body(RpcFragmentSizes::DEFAULT),
                &[0; 8],
                &[RPC_AUTH_TYPE_WINNT, RPC_AUTH_LEVEL_PACKET_INTEGRITY, 8, 0, 0, 0, 0, 0,],
                &token,
            ]
            .concat()
        );
    }

    #[test]
    fn authenticated_bind_ack_extracts_type2_token() {
        let token = [0x82, 1, 2, 3];
        let body = [
            [0x00u8, 0x20, 0x00, 0x30, 0, 0, 0, 0].as_slice(),
            &[0, 0, 0, 0],
            &[1, 0, 0, 0, 0, 0, 0, 0],
            &[
                0x04, 0x5d, 0x88, 0x8a, 0xeb, 0x1c, 0xc9, 0x11, 0x9f, 0xe8, 0x08, 0x00, 0x2b, 0x10, 0x48, 0x60,
            ],
            &[2, 0, 0, 0],
        ]
        .concat();
        let bind_ack = encode_authenticated_pdu(PTYPE_BIND_ACK, 0x7856_3412, body, &token).expect("valid bind ack");

        assert_eq!(bind_ack.len(), 76);
        assert_eq!(
            &bind_ack[64..72],
            &[RPC_AUTH_TYPE_WINNT, RPC_AUTH_LEVEL_PACKET_INTEGRITY, 8, 0, 0, 0, 0, 0]
        );
        let ack = decode_tsgu_bind_ack_with_ntlm_auth(&bind_ack, RpcFragmentSizes::DEFAULT)
            .expect("accepted authenticated bind ack");
        assert_eq!(ack.token(), token);
        assert_eq!(ack.binding().call_id(), 0x7856_3412);
        assert_eq!(
            ack.binding().fragment_sizes(),
            RpcFragmentSizes::new(0x10b8, 0x10b8).expect("valid negotiated maxima")
        );
    }

    #[test]
    fn rpc_auth_3_has_required_pad_and_security_trailer() {
        let token = [5, 6, 7, 8];
        let auth3 = encode_rpc_auth_3(0x7856_3412, RpcFragmentSizes::DEFAULT, &token).expect("valid auth3");

        assert_eq!(
            auth3,
            [
                [
                    5,
                    0,
                    PTYPE_RPC_AUTH_3,
                    PFC_FIRST_FRAG | PFC_LAST_FRAG | PFC_SUPPORT_HEADER_SIGN | PFC_CONC_MPX,
                    0x10,
                    0,
                    0,
                    0,
                ]
                .as_slice(),
                &[44, 0, 4, 0, 0x12, 0x34, 0x56, 0x78],
                &[0; 16],
                &[RPC_AUTH_TYPE_WINNT, RPC_AUTH_LEVEL_PACKET_INTEGRITY, 12, 0, 0, 0, 0, 0,],
                &token,
            ]
            .concat()
        );
    }

    #[test]
    fn protected_request_uses_a_normal_request_pfc_flags() {
        let body =
            encode_rpc_request_body(TSPROXY_SETUP_RECEIVE_PIPE_OPNUM, &[0xaa, 0xbb]).expect("valid request body");
        let request = encode_authenticated_pdu_with_flags(
            PTYPE_REQUEST,
            PFC_FIRST_FRAG | PFC_LAST_FRAG,
            0x7856_3412,
            body,
            &[0; 16],
        )
        .expect("valid protected request");

        assert_eq!(request[3], PFC_FIRST_FRAG | PFC_LAST_FRAG);
        assert_eq!(&request[10..12], &16u16.to_le_bytes());
        let (header, body, signature) =
            decode_authenticated_single_fragment(&request, PTYPE_REQUEST, RpcFragmentSizes::DEFAULT.max_xmit(), false)
                .expect("normal request flags accepted");
        assert_eq!(header.call_id(), 0x7856_3412);
        assert_eq!(
            body,
            &[
                2,
                0,
                0,
                0,
                0,
                0,
                u8::try_from(TSPROXY_SETUP_RECEIVE_PIPE_OPNUM).expect("opnum fits in u8"),
                0,
                0xaa,
                0xbb
            ]
        );
        assert_eq!(signature, [0; 16]);
        assert_eq!(
            decode_authenticated_single_fragment(&request, PTYPE_REQUEST, RpcFragmentSizes::DEFAULT.max_xmit(), true),
            Err(RpcPduError::MissingSupportHeaderSign)
        );
    }

    #[test]
    fn protected_response_preserves_the_unpadded_stub() {
        let response = encode_authenticated_pdu_with_flags(
            PTYPE_RESPONSE,
            PFC_FIRST_FRAG | PFC_LAST_FRAG,
            0x7856_3412,
            vec![3, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3],
            &[0; 16],
        )
        .expect("valid protected response");
        let (header, body, signature) = decode_authenticated_single_fragment(
            &response,
            PTYPE_RESPONSE,
            RpcFragmentSizes::DEFAULT.max_recv(),
            false,
        )
        .expect("normal response flags accepted");

        assert_eq!(signature, [0; 16]);
        assert_eq!(
            decode_rpc_response_body(header, body).expect("valid response body"),
            RpcResponse {
                call_id: 0x7856_3412,
                pfc_flags: PFC_FIRST_FRAG | PFC_LAST_FRAG,
                alloc_hint: 3,
                cancel_count: 0,
                reserved: 0,
                stub: &[1, 2, 3],
            }
        );
    }

    #[test]
    fn authenticated_bind_ack_rejects_invalid_verifier_layout() {
        let token = [0x82, 1, 2, 3];
        let body = [
            [0x00u8, 0x20, 0x00, 0x30, 0, 0, 0, 0].as_slice(),
            &[0, 0, 0, 0],
            &[1, 0, 0, 0, 0, 0, 0, 0],
            &[
                0x04, 0x5d, 0x88, 0x8a, 0xeb, 0x1c, 0xc9, 0x11, 0x9f, 0xe8, 0x08, 0x00, 0x2b, 0x10, 0x48, 0x60,
            ],
            &[2, 0, 0, 0],
        ]
        .concat();
        let bind_ack = encode_authenticated_pdu(PTYPE_BIND_ACK, 1, body, &token).expect("valid bind ack");

        let mut missing_header_sign = bind_ack.clone();
        missing_header_sign[3] &= !PFC_SUPPORT_HEADER_SIGN;
        assert_eq!(
            decode_tsgu_bind_ack_with_ntlm_auth(&missing_header_sign, RpcFragmentSizes::DEFAULT),
            Err(RpcPduError::MissingSupportHeaderSign)
        );

        let mut wrong_auth_type = bind_ack.clone();
        wrong_auth_type[64] = 9;
        assert_eq!(
            decode_tsgu_bind_ack_with_ntlm_auth(&wrong_auth_type, RpcFragmentSizes::DEFAULT),
            Err(RpcPduError::UnexpectedAuthenticationType {
                expected: RPC_AUTH_TYPE_WINNT,
                actual: 9
            })
        );

        let mut wrong_context = bind_ack.clone();
        wrong_context[68..72].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            decode_tsgu_bind_ack_with_ntlm_auth(&wrong_context, RpcFragmentSizes::DEFAULT),
            Err(RpcPduError::UnexpectedAuthenticationContextId {
                expected: RPC_AUTH_CONTEXT_ID,
                actual: 1
            })
        );
        let mut non_zero_padding = bind_ack.clone();
        non_zero_padding[63] = 1;
        assert_eq!(
            decode_tsgu_bind_ack_with_ntlm_auth(&non_zero_padding, RpcFragmentSizes::DEFAULT),
            Err(RpcPduError::NonZeroAuthenticationPadding)
        );

        let mut invalid_trailer_bounds = bind_ack;
        invalid_trailer_bounds[10..12].copy_from_slice(&61u16.to_le_bytes());
        assert_eq!(
            decode_tsgu_bind_ack_with_ntlm_auth(&invalid_trailer_bounds, RpcFragmentSizes::DEFAULT),
            Err(RpcPduError::InvalidSecurityTrailer {
                fragment_length: 76,
                auth_length: 61
            })
        );
    }

    #[test]
    fn rpc_ntlm_initial_token_is_non_empty() {
        let mut auth = RpcNtlmAuth::new(r"CONTOSO\alice", "secret").expect("valid credentials");
        let token = auth.initial_token().expect("offline Type-1 token");

        assert_eq!(&token[..12], b"NTLMSSP\0\x01\0\0\0");
        assert!(!auth.is_complete());
        assert!(auth.initial_token().is_err());
        assert!(auth.make_signature(&mut [], 0).is_err());
    }

    #[test]
    fn rpc_ntlm_packet_integrity_signature_covers_the_sequence_number() {
        let mut auth = completed_rpc_ntlm_auth();
        let mut first = b"authenticated rpc message".to_vec();
        let mut second = first.clone();
        let first_signature = auth.make_signature(&mut first, 1).expect("first signature");
        let second_signature = auth.make_signature(&mut second, 2).expect("second signature");

        assert_ne!(first_signature, [0; 16]);
        assert_ne!(first_signature, second_signature);
    }

    #[test]
    fn protected_request_is_signed_after_its_security_trailer_is_encoded() {
        let mut auth = completed_rpc_ntlm_auth();
        let request = encode_rpc_request_with_ntlm_auth(
            &mut auth,
            1,
            0x7856_3412,
            TSPROXY_SETUP_RECEIVE_PIPE_OPNUM,
            &[0xaa, 0xbb],
            RpcFragmentSizes::DEFAULT,
        )
        .expect("valid signed request");

        assert_eq!(request[3], PFC_FIRST_FRAG | PFC_LAST_FRAG);
        assert_ne!(&request[request.len() - 16..], &[0; 16]);
        assert_eq!(request[request.len() - 24], RPC_AUTH_TYPE_WINNT);
    }

    #[test]
    fn bind_ack_negotiates_the_single_accepted_ndr32_context() {
        let bind_ack = [
            [5, 0, PTYPE_BIND_ACK, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &[0x38, 0, 0, 0, 0x12, 0x34, 0x56, 0x78],
            &[0x00, 0x20, 0x00, 0x30, 0, 0, 0, 0],
            &[0, 0, 0, 0],
            &[1, 0, 0, 0, 0, 0, 0, 0],
            &[
                0x04, 0x5d, 0x88, 0x8a, 0xeb, 0x1c, 0xc9, 0x11, 0x9f, 0xe8, 0x08, 0x00, 0x2b, 0x10, 0x48, 0x60,
            ],
            &[2, 0, 0, 0],
        ]
        .concat();

        let binding = decode_tsgu_bind_ack(&bind_ack, RpcFragmentSizes::DEFAULT).expect("accepted bind ack");
        assert_eq!(binding.call_id(), 0x7856_3412);
        assert_eq!(
            binding.fragment_sizes(),
            RpcFragmentSizes::new(0x10b8, 0x10b8).expect("valid negotiated maxima")
        );
    }

    #[test]
    fn bind_nak_preserves_reason_and_supported_versions() {
        let bind_nak = [
            [5, 0, PTYPE_BIND_NAK, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &[23, 0, 0, 0, 4, 3, 2, 1],
            &[4, 0, 2, 5, 0, 4, 0],
        ]
        .concat();

        assert_eq!(
            decode_bind_nak(&bind_nak).expect("valid bind nak"),
            RpcBindNak {
                call_id: 0x0102_0304,
                reason: RpcBindNakReason(4),
                supported_versions: vec![RpcSyntaxVersion::new(5, 0), RpcSyntaxVersion::new(4, 0)],
                extended_error_signature: None,
            }
        );
    }

    #[test]
    fn bind_nak_accepts_the_optional_extended_error_signature() {
        let bind_nak = [
            [5, 0, PTYPE_BIND_NAK, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &[39, 0, 0, 0, 4, 3, 2, 1],
            &[4, 0, 2, 5, 0, 4, 0],
            &[
                0x04, 0x5d, 0x88, 0x8a, 0xeb, 0x1c, 0xc9, 0x11, 0x9f, 0xe8, 0x08, 0x00, 0x2b, 0x10, 0x48, 0x60,
            ],
        ]
        .concat();

        assert_eq!(
            decode_bind_nak(&bind_nak)
                .expect("valid bind nak")
                .extended_error_signature,
            Some(NDR32_TRANSFER_SYNTAX_ID)
        );
    }

    #[test]
    fn request_response_and_fault_vectors_round_trip_metadata_and_stubs() {
        let request = encode_rpc_request(
            0x7856_3412,
            TSPROXY_SETUP_RECEIVE_PIPE_OPNUM,
            &[0xaa, 0xbb],
            RpcFragmentSizes::DEFAULT,
        )
        .expect("request fits");
        assert_eq!(
            request,
            [
                [5, 0, PTYPE_REQUEST, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
                &[26, 0, 0, 0, 0x12, 0x34, 0x56, 0x78],
                &[
                    2,
                    0,
                    0,
                    0,
                    0,
                    0,
                    u8::try_from(TSPROXY_SETUP_RECEIVE_PIPE_OPNUM).expect("opnum fits in u8"),
                    0,
                    0xaa,
                    0xbb
                ],
            ]
            .concat()
        );

        let response = [
            [5, 0, PTYPE_RESPONSE, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &[27, 0, 0, 0, 0x12, 0x34, 0x56, 0x78],
            &[3, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3],
        ]
        .concat();
        assert_eq!(
            decode_rpc_response(&response, DEFAULT_FRAGMENT_SIZE).expect("valid response"),
            RpcResponse {
                call_id: 0x7856_3412,
                pfc_flags: PFC_FIRST_FRAG | PFC_LAST_FRAG,
                alloc_hint: 3,
                cancel_count: 0,
                reserved: 0,
                stub: &[1, 2, 3],
            }
        );
        let mut fragmented_response = response;
        fragmented_response[3] = PFC_FIRST_FRAG;
        assert_eq!(
            decode_rpc_response_fragment(&fragmented_response, DEFAULT_FRAGMENT_SIZE).expect("valid response fragment"),
            RpcResponse {
                call_id: 0x7856_3412,
                pfc_flags: PFC_FIRST_FRAG,
                alloc_hint: 3,
                cancel_count: 0,
                reserved: 0,
                stub: &[1, 2, 3],
            }
        );

        let fault = [
            [5, 0, PTYPE_FAULT, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &[34, 0, 0, 0, 4, 3, 2, 1],
            &[2, 0, 0, 0, 0, 0, 0, 0, 0xef, 0xbe, 0xad, 0xde, 1, 0, 0, 0, 0x12, 0x34],
        ]
        .concat();
        assert_eq!(
            decode_rpc_fault(&fault, DEFAULT_FRAGMENT_SIZE).expect("valid fault"),
            RpcFault {
                call_id: 0x0102_0304,
                alloc_hint: 2,
                cancel_count: 0,
                reserved: 0,
                status: 0xdead_beef,
                reserved2: 1,
                stub: &[0x12, 0x34],
            }
        );
    }

    #[test]
    fn common_header_rejects_invalid_versions_drep_and_fragment_lengths() {
        assert_eq!(
            RpcCommonHeader::decode(&[0; RPC_COMMON_HEADER_SIZE - 1]),
            Err(RpcPduError::Truncated {
                actual: RPC_COMMON_HEADER_SIZE - 1,
                required: RPC_COMMON_HEADER_SIZE
            })
        );

        let mut pdu = encode_rpc_request(1, 0, &[], RpcFragmentSizes::DEFAULT).expect("valid request");
        pdu[0] = 4;
        assert_eq!(
            RpcCommonHeader::decode(&pdu),
            Err(RpcPduError::UnsupportedVersion { major: 4, minor: 0 })
        );

        let mut pdu = encode_rpc_request(1, 0, &[], RpcFragmentSizes::DEFAULT).expect("valid request");
        pdu[4] = 0;
        assert_eq!(
            RpcCommonHeader::decode(&pdu),
            Err(RpcPduError::UnsupportedDataRepresentation { value: [0, 0, 0, 0] })
        );

        let mut pdu = encode_rpc_request(1, 0, &[], RpcFragmentSizes::DEFAULT).expect("valid request");
        pdu[8..10].copy_from_slice(&15u16.to_le_bytes());
        assert_eq!(
            RpcCommonHeader::decode(&pdu),
            Err(RpcPduError::InvalidFragmentLength { fragment_length: 15 })
        );
    }

    #[test]
    fn authorize_tunnel_response_decodes_response_data_and_redirection_policy() {
        let mut response = Vec::new();
        response.extend_from_slice(&NDR_REFERENT_ID.to_le_bytes()); // TSG packet
        response.extend_from_slice(&TSG_PACKET_TYPE_RESPONSE.to_le_bytes());
        response.extend_from_slice(&TSG_PACKET_TYPE_RESPONSE.to_le_bytes());
        response.extend_from_slice(&(NDR_REFERENT_ID + 4).to_le_bytes()); // packet response
        response.extend_from_slice(&TSG_PACKET_TYPE_QUARREQUEST.to_le_bytes());
        response.extend_from_slice(&0u32.to_le_bytes()); // reserved
        response.extend_from_slice(&(NDR_REFERENT_ID + 8).to_le_bytes()); // response data
        response.extend_from_slice(&3u32.to_le_bytes());
        response.extend_from_slice(&0u32.to_le_bytes()); // enable all
        response.extend_from_slice(&0u32.to_le_bytes()); // disable all
        response.extend_from_slice(&1u32.to_le_bytes()); // drive disabled
        response.extend_from_slice(&0u32.to_le_bytes()); // printer disabled
        response.extend_from_slice(&0u32.to_le_bytes()); // port disabled
        response.extend_from_slice(&0u32.to_le_bytes()); // reserved
        response.extend_from_slice(&1u32.to_le_bytes()); // clipboard disabled
        response.extend_from_slice(&0u32.to_le_bytes()); // PnP disabled
        response.extend_from_slice(&3u32.to_le_bytes()); // max count
        response.extend_from_slice(&[1, 2, 3]);
        response.push(0); // NDR alignment
        response.extend_from_slice(&0u32.to_le_bytes()); // HRESULT

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
    }

    #[test]
    fn authorize_tunnel_response_rejects_invalid_packet_and_policy_values() {
        let mut response = vec![0; 68];
        response[0..4].copy_from_slice(&NDR_REFERENT_ID.to_le_bytes());
        response[4..8].copy_from_slice(&TSG_PACKET_TYPE_RESPONSE.to_le_bytes());
        response[8..12].copy_from_slice(&TSG_PACKET_TYPE_RESPONSE.to_le_bytes());
        response[12..16].copy_from_slice(&(NDR_REFERENT_ID + 4).to_le_bytes());
        response[16..20].copy_from_slice(&TSG_PACKET_TYPE_QUARREQUEST.to_le_bytes());

        response[4..8].copy_from_slice(&TSG_PACKET_TYPE_QUARREQUEST.to_le_bytes());
        assert_eq!(
            decode_tsgu_authorize_tunnel_response(&response),
            Err(RpcWireError::UnexpectedPacketId {
                expected: TSG_PACKET_TYPE_RESPONSE,
                actual: TSG_PACKET_TYPE_QUARREQUEST,
            })
        );
        response[4..8].copy_from_slice(&TSG_PACKET_TYPE_RESPONSE.to_le_bytes());

        response[32..36].copy_from_slice(&1u32.to_le_bytes());
        response[36..40].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            decode_tsgu_authorize_tunnel_response(&response),
            Err(RpcWireError::ConflictingRedirectionFlags)
        );
        response[36..40].copy_from_slice(&0u32.to_le_bytes());

        response[28..32].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            decode_tsgu_authorize_tunnel_response(&response),
            Err(RpcWireError::RequiredNdrPointerIsNull)
        );
    }

    #[test]
    fn make_tunnel_call_response_decodes_reauthentication() {
        let response = [
            NDR_REFERENT_ID.to_le_bytes().as_slice(), // TSG packet
            &TSG_PACKET_TYPE_MESSAGE.to_le_bytes(),
            &TSG_PACKET_TYPE_MESSAGE.to_le_bytes(),
            &(NDR_REFERENT_ID + 4).to_le_bytes(), // message response
            &1u32.to_le_bytes(),                  // message ID
            &TSG_ASYNC_MESSAGE_REAUTH.to_le_bytes(),
            &1u32.to_le_bytes(),                  // is message present
            &(NDR_REFERENT_ID + 8).to_le_bytes(), // reauth message
            &0x1122_3344_5566_7788u64.to_le_bytes(),
            &0u32.to_le_bytes(), // HRESULT
        ]
        .concat();

        assert_eq!(
            decode_tsgu_make_tunnel_call_response(&response).expect("valid response"),
            TsProxyTunnelMessage::Reauthenticate {
                tunnel_context: 0x1122_3344_5566_7788,
            }
        );
    }

    #[test]
    fn make_tunnel_call_response_decodes_service_message() {
        let response = [
            NDR_REFERENT_ID.to_le_bytes().as_slice(), // TSG packet
            &TSG_PACKET_TYPE_MESSAGE.to_le_bytes(),
            &TSG_PACKET_TYPE_MESSAGE.to_le_bytes(),
            &(NDR_REFERENT_ID + 4).to_le_bytes(), // message response
            &1u32.to_le_bytes(),                  // message ID
            &TSG_ASYNC_MESSAGE_SERVICE.to_le_bytes(),
            &1u32.to_le_bytes(),                   // is message present
            &(NDR_REFERENT_ID + 8).to_le_bytes(),  // service message
            &1u32.to_le_bytes(),                   // display mandatory
            &0u32.to_le_bytes(),                   // consent mandatory
            &3u32.to_le_bytes(),                   // message characters
            &(NDR_REFERENT_ID + 12).to_le_bytes(), // message buffer
            &3u32.to_le_bytes(),                   // NDR array count
            &[b'h', 0, b'i', 0, 0, 0],
            &0u32.to_le_bytes(), // HRESULT
        ]
        .concat();

        assert_eq!(
            decode_tsgu_make_tunnel_call_response(&response).expect("valid response"),
            TsProxyTunnelMessage::Service {
                display_mandatory: true,
                text: "hi".to_owned(),
            }
        );
    }

    fn create_tunnel_response() -> Vec<u8> {
        let nonce = Uuid::from_u128(0x00112233_4455_6677_8899_aabbccddeeff);
        let mut response = Vec::new();
        response.extend_from_slice(&NDR_REFERENT_ID.to_le_bytes()); // TSG packet
        response.extend_from_slice(&TSG_PACKET_TYPE_QUARENC_RESPONSE.to_le_bytes());
        response.extend_from_slice(&TSG_PACKET_TYPE_QUARENC_RESPONSE.to_le_bytes());
        response.extend_from_slice(&(NDR_REFERENT_ID + 4).to_le_bytes()); // QUARENC response
        response.extend_from_slice(&0u32.to_le_bytes()); // flags
        response.extend_from_slice(&0u32.to_le_bytes()); // certificate chain length
        response.extend_from_slice(&0u32.to_le_bytes()); // certificate chain pointer
        response.extend_from_slice(&nonce.to_bytes_le());
        response.extend_from_slice(&(NDR_REFERENT_ID + 8).to_le_bytes()); // version capabilities
        response.extend_from_slice(&TSG_COMPONENT_ID.to_le_bytes());
        response.extend_from_slice(
            &u16::try_from(TSG_PACKET_TYPE_VERSIONCAPS)
                .expect("packet type fits in u16")
                .to_le_bytes(),
        );
        response.extend_from_slice(&(NDR_REFERENT_ID + 12).to_le_bytes()); // capability array
        response.extend_from_slice(&1u32.to_le_bytes()); // number of capabilities
        response.extend_from_slice(&1u16.to_le_bytes()); // major version
        response.extend_from_slice(&1u16.to_le_bytes()); // minor version
        response.extend_from_slice(&0u16.to_le_bytes()); // quarantine capabilities
        response.extend_from_slice(&0u16.to_le_bytes()); // NDR alignment
        response.extend_from_slice(&1u32.to_le_bytes()); // capability array max count
        response.extend_from_slice(&TSG_CAPABILITY_TYPE_NAP.to_le_bytes());
        response.extend_from_slice(&TSG_CAPABILITY_TYPE_NAP.to_le_bytes());
        response.extend_from_slice(&0x0000_0003u32.to_le_bytes());
        response.extend_from_slice(&CHANNEL_CONTEXT);
        response.extend_from_slice(&42u32.to_le_bytes()); // tunnel ID
        response.extend_from_slice(&0u32.to_le_bytes()); // HRESULT
        response
    }

    #[test]
    fn create_tunnel_response_decodes_context_nonce_and_capabilities() {
        assert_eq!(
            decode_tsgu_create_tunnel_response(&create_tunnel_response()).expect("valid response"),
            TsProxyCreateTunnelResponse {
                tunnel_context: channel_context(),
                tunnel_id: 42,
                nonce: Uuid::from_u128(0x00112233_4455_6677_8899_aabbccddeeff),
                capabilities: 3,
            }
        );
    }

    #[test]
    fn create_tunnel_response_rejects_invalid_capability_count() {
        let mut response = create_tunnel_response();
        response[56..60].copy_from_slice(&2u32.to_le_bytes());

        assert_eq!(
            decode_tsgu_create_tunnel_response(&response),
            Err(RpcWireError::UnsupportedCapabilityCount { actual: 2 })
        );
    }

    #[test]
    fn create_tunnel_response_rejects_reserved_flags_and_oversized_certificates() {
        let mut response = create_tunnel_response();
        response[16..20].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            decode_tsgu_create_tunnel_response(&response),
            Err(RpcWireError::InvalidQuarencFlags { actual: 1 })
        );

        let mut response = create_tunnel_response();
        let length = u32::try_from(TSG_MAX_CERT_CHAIN_LEN + 1).expect("test value fits");
        response[20..24].copy_from_slice(&length.to_le_bytes());
        assert_eq!(
            decode_tsgu_create_tunnel_response(&response),
            Err(RpcWireError::CertificateChainTooLarge {
                actual: TSG_MAX_CERT_CHAIN_LEN + 1,
            })
        );
    }

    #[test]
    fn pdu_stream_yields_complete_fragments_without_losing_partial_data() {
        let first = encode_rpc_request(1, 0, &[1, 2], RpcFragmentSizes::DEFAULT).expect("valid request");
        let second = encode_rpc_request(2, 1, &[3], RpcFragmentSizes::DEFAULT).expect("valid request");
        let mut stream = RpcPduStream::new(RpcFragmentSizes::DEFAULT.max_recv()).expect("valid maximum");

        stream
            .push(&first[..RPC_COMMON_HEADER_SIZE - 1])
            .expect("bounded partial fragment");
        assert_eq!(stream.next(), Ok(None));

        stream
            .push(&first[RPC_COMMON_HEADER_SIZE - 1..])
            .expect("bounded first fragment");
        stream.push(&second).expect("bounded second fragment");
        assert_eq!(stream.next(), Ok(Some(first)));
        assert_eq!(stream.next(), Ok(Some(second)));
        assert_eq!(stream.next(), Ok(None));
    }

    #[test]
    fn pdu_stream_rejects_oversized_and_invalid_fragments_before_buffering_them() {
        let mut stream = RpcPduStream::new(32).expect("valid maximum");
        let oversized = encode_rpc_request(1, 0, &[0; 17], RpcFragmentSizes::DEFAULT).expect("valid request");
        stream
            .push(&oversized[..RPC_COMMON_HEADER_SIZE])
            .expect("bounded oversized header");
        assert_eq!(
            stream.next(),
            Err(RpcPduError::FragmentExceedsMaximum {
                fragment_length: u16::try_from(oversized.len()).expect("test length fits in u16"),
                maximum: 32,
            })
        );

        let mut stream = RpcPduStream::new(RpcFragmentSizes::DEFAULT.max_recv()).expect("valid maximum");
        stream
            .push(&[4; RPC_COMMON_HEADER_SIZE])
            .expect("bounded invalid header");
        assert_eq!(
            stream.next(),
            Err(RpcPduError::UnsupportedVersion { major: 4, minor: 4 })
        );
    }

    #[test]
    fn request_fragmenter_sets_pfc_flags_and_remaining_allocation_hints() {
        let fragments = encode_rpc_request_fragments(
            7,
            TSPROXY_SETUP_RECEIVE_PIPE_OPNUM,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9],
            RpcFragmentSizes::new(32, 32).expect("valid fragment sizes"),
        )
        .expect("request fragments");

        assert_eq!(fragments.len(), 2);
        assert_eq!(fragments[0][3], PFC_FIRST_FRAG);
        assert_eq!(fragments[1][3], PFC_LAST_FRAG);
        assert_eq!(&fragments[0][16..20], &9u32.to_le_bytes());
        assert_eq!(&fragments[1][16..20], &1u32.to_le_bytes());
        assert_eq!(&fragments[0][24..], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(&fragments[1][24..], &[9]);
        assert_eq!(
            encode_rpc_request_fragments(
                7,
                TSPROXY_SETUP_RECEIVE_PIPE_OPNUM,
                &[1],
                RpcFragmentSizes::new(24, 24).expect("valid fragment sizes"),
            ),
            Err(RpcPduError::FragmentTooLarge {
                actual: 25,
                maximum: 24,
            })
        );
    }

    #[test]
    fn response_reassembler_requires_ordered_fragments_and_bounds_the_stub() {
        let mut reassembler = RpcResponseReassembler::new(5);
        let first = RpcResponse {
            call_id: 7,
            pfc_flags: PFC_FIRST_FRAG,
            alloc_hint: 5,
            cancel_count: 0,
            reserved: 0,
            stub: &[1, 2],
        };
        assert_eq!(reassembler.push(first), Ok(None));
        let last = RpcResponse {
            call_id: 7,
            pfc_flags: PFC_LAST_FRAG,
            alloc_hint: 3,
            cancel_count: 0,
            reserved: 0,
            stub: &[3, 4, 5],
        };
        assert_eq!(
            reassembler.push(last),
            Ok(Some(RpcReassembledResponse {
                call_id: 7,
                cancel_count: 0,
                reserved: 0,
                stub: vec![1, 2, 3, 4, 5],
            }))
        );

        let unexpected = RpcResponse {
            call_id: 8,
            pfc_flags: PFC_LAST_FRAG,
            alloc_hint: 1,
            cancel_count: 0,
            reserved: 0,
            stub: &[1],
        };
        assert_eq!(
            reassembler.push(unexpected),
            Err(RpcPduError::UnexpectedResponseFragment { flags: PFC_LAST_FRAG })
        );

        let oversized = RpcResponse {
            call_id: 8,
            pfc_flags: PFC_FIRST_FRAG | PFC_LAST_FRAG,
            alloc_hint: 6,
            cancel_count: 0,
            reserved: 0,
            stub: &[1, 2, 3, 4, 5, 6],
        };
        assert_eq!(
            reassembler.push(oversized),
            Err(RpcPduError::ResponseStubTooLarge { actual: 6, maximum: 5 })
        );
    }

    #[test]
    fn rpc_decoders_reject_unsupported_fragments_auth_and_untrusted_lengths() {
        let mut response = [
            [5, 0, PTYPE_RESPONSE, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &[24, 0, 0, 0, 1, 0, 0, 0],
            &[0, 0, 0, 0, 0, 0, 0, 0],
        ]
        .concat();
        response[3] = PFC_FIRST_FRAG;
        assert_eq!(
            decode_rpc_response(&response, DEFAULT_FRAGMENT_SIZE),
            Err(RpcPduError::FragmentedPduUnsupported { flags: PFC_FIRST_FRAG })
        );

        response[3] = PFC_FIRST_FRAG | PFC_LAST_FRAG;
        response[10..12].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(
            decode_rpc_response(&response, DEFAULT_FRAGMENT_SIZE),
            Err(RpcPduError::AuthenticationUnsupported { auth_length: 1 })
        );

        response[10..12].copy_from_slice(&0u16.to_le_bytes());
        response[16..20].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            decode_rpc_response(&response, DEFAULT_FRAGMENT_SIZE),
            Err(RpcPduError::InvalidAllocHint {
                alloc_hint: 1,
                stub_length: 0
            })
        );

        let oversized = vec![0; usize::from(DEFAULT_FRAGMENT_SIZE) - RPC_REQUEST_HEADER_SIZE + 1];
        assert!(matches!(
            encode_rpc_request(1, 0, &oversized, RpcFragmentSizes::DEFAULT),
            Err(RpcPduError::FragmentTooLarge { .. })
        ));
    }

    #[test]
    fn bind_ack_rejects_unsuccessful_or_wrong_context_negotiation() {
        let mut bind_ack = [
            [5, 0, PTYPE_BIND_ACK, PFC_FIRST_FRAG | PFC_LAST_FRAG, 0x10, 0, 0, 0].as_slice(),
            &[0x38, 0, 0, 0, 0, 0, 0, 0],
            &[0xb8, 0x10, 0xb8, 0x10, 0, 0, 0, 0],
            &[0, 0, 0, 0],
            &[1, 0, 0, 0, 0, 0, 0, 0],
            &[
                0x04, 0x5d, 0x88, 0x8a, 0xeb, 0x1c, 0xc9, 0x11, 0x9f, 0xe8, 0x08, 0x00, 0x2b, 0x10, 0x48, 0x60,
            ],
            &[2, 0, 0, 0],
        ]
        .concat();
        bind_ack[32..34].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(
            decode_tsgu_bind_ack(&bind_ack, RpcFragmentSizes::DEFAULT),
            Err(RpcPduError::PresentationContextRejected { result: 2, reason: 0 })
        );

        bind_ack[32..34].copy_from_slice(&0u16.to_le_bytes());
        bind_ack[36] ^= 1;
        assert!(matches!(
            decode_tsgu_bind_ack(&bind_ack, RpcFragmentSizes::DEFAULT),
            Err(RpcPduError::UnexpectedTransferSyntax { .. })
        ));
    }

    #[test]
    fn rts_conn_a1_and_b1_encode_exact_initial_flow_vectors() {
        let virtual_connection_cookie = RtsCookie::new([
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        ]);
        let out_channel_cookie = RtsCookie::new([
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
        ]);
        let in_channel_cookie = RtsCookie::new([
            0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x2f,
        ]);
        let association_group_id = RtsCookie::new([
            0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f,
        ]);

        assert_eq!(
            encode_rts_conn_a1(virtual_connection_cookie, out_channel_cookie, 128 * 1024).expect("valid A1"),
            [
                [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
                &[76, 0, 0, 0, 0, 0, 0, 0],
                &[0, 0, 4, 0],
                &[6, 0, 0, 0, 1, 0, 0, 0],
                &[3, 0, 0, 0],
                virtual_connection_cookie.as_bytes(),
                &[3, 0, 0, 0],
                out_channel_cookie.as_bytes(),
                &[0, 0, 0, 0, 0, 0, 2, 0],
            ]
            .concat()
        );
        assert_eq!(
            encode_rts_conn_b1(
                virtual_connection_cookie,
                in_channel_cookie,
                256 * 1024,
                0,
                association_group_id,
            )
            .expect("valid B1"),
            [
                [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
                &[104, 0, 0, 0, 0, 0, 0, 0],
                &[0, 0, 6, 0],
                &[6, 0, 0, 0, 1, 0, 0, 0],
                &[3, 0, 0, 0],
                virtual_connection_cookie.as_bytes(),
                &[3, 0, 0, 0],
                in_channel_cookie.as_bytes(),
                &[4, 0, 0, 0, 0, 0, 4, 0],
                &[5, 0, 0, 0, 0, 0, 0, 0],
                &[12, 0, 0, 0],
                association_group_id.as_bytes(),
            ]
            .concat()
        );
    }

    #[test]
    fn rts_ping_and_flow_control_ack_round_trip() {
        assert_eq!(
            encode_rts_ping().expect("valid ping"),
            [
                [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
                &[20, 0, 0, 0, 0, 0, 0, 0],
                &[1, 0, 0, 0],
            ]
            .concat()
        );
        decode_rts_ping(&encode_rts_ping().expect("valid ping")).expect("valid ping");

        let ack = RtsFlowControlAck {
            bytes_received: 0x7856_3412,
            available_window: 128 * 1024,
            channel_cookie: RtsCookie::new([
                0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
            ]),
        };
        let encoded = encode_rts_flow_control_ack(ack).expect("valid acknowledgement");
        assert_eq!(
            encoded,
            [
                [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
                &[48, 0, 0, 0, 0, 0, 0, 0],
                &[2, 0, 1, 0],
                &[1, 0, 0, 0, 0x12, 0x34, 0x56, 0x78, 0, 0, 2, 0],
                ack.channel_cookie.as_bytes(),
            ]
            .concat()
        );
        assert_eq!(
            decode_rts_flow_control_ack(&encoded).expect("valid acknowledgement"),
            ack
        );
    }

    #[test]
    fn rts_ping_and_flow_control_ack_reject_invalid_headers() {
        let mut ping = encode_rts_ping().expect("valid ping");
        ping[16..18].copy_from_slice(&RTS_FLAG_OTHER_CMD.to_le_bytes());
        assert_eq!(
            decode_rts_ping(&ping),
            Err(RpcPduError::UnexpectedRtsFlags {
                expected: RTS_FLAG_PING,
                actual: RTS_FLAG_OTHER_CMD,
            })
        );

        let ack = RtsFlowControlAck {
            bytes_received: 0,
            available_window: 128 * 1024,
            channel_cookie: RtsCookie::new([0; RtsCookie::SIZE]),
        };
        let mut encoded = encode_rts_flow_control_ack(ack).expect("valid acknowledgement");
        encoded[20..24].copy_from_slice(&RTS_COMMAND_COOKIE.to_le_bytes());
        assert_eq!(
            decode_rts_flow_control_ack(&encoded),
            Err(RpcPduError::UnexpectedRtsCommandType {
                expected: RTS_COMMAND_FLOW_CONTROL_ACK,
                actual: RTS_COMMAND_COOKIE,
            })
        );
    }

    #[test]
    fn rts_conn_a3_and_c2_decode_initial_flow_vectors() {
        let a3_bytes = [
            [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
            &[28, 0, 0, 0, 0, 0, 0, 0],
            &[0, 0, 1, 0],
            &[2, 0, 0, 0, 0xc0, 0xd4, 1, 0],
        ]
        .concat();
        assert_eq!(
            decode_rts_conn_a3(&a3_bytes).expect("valid A3"),
            RtsConnA3 {
                connection_timeout: 120_000
            }
        );

        let c2_bytes = [
            [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
            &[44, 0, 0, 0, 0, 0, 0, 0],
            &[0, 0, 3, 0],
            &[6, 0, 0, 0, 0x12, 0x34, 0x56, 0x78],
            &[0, 0, 0, 0, 0, 0x20, 0, 0],
            &[2, 0, 0, 0, 0xc0, 0xd4, 1, 0],
        ]
        .concat();
        assert_eq!(
            decode_rts_conn_c2(&c2_bytes).expect("valid C2"),
            RtsConnC2 {
                version: 0x7856_3412,
                receive_window_size: 8 * 1024,
                connection_timeout: 120_000,
            }
        );

        let mut conn_c2_with_invalid_receive_window = c2_bytes;
        conn_c2_with_invalid_receive_window[32..36].copy_from_slice(&(8u32 * 1024 - 1).to_le_bytes());
        assert_eq!(
            decode_rts_conn_c2(&conn_c2_with_invalid_receive_window),
            Err(RpcPduError::InvalidRtsReceiveWindowSize { actual: 8 * 1024 - 1 })
        );
    }

    #[test]
    fn rts_in_recycling_encodes_client_messages_and_decodes_a4() {
        let virtual_connection_cookie = RtsCookie::new([0x10; RtsCookie::SIZE]);
        let predecessor_channel_cookie = RtsCookie::new([0x20; RtsCookie::SIZE]);
        let successor_channel_cookie = RtsCookie::new([0x30; RtsCookie::SIZE]);
        assert_eq!(
            encode_rts_in_recycle_a1(
                virtual_connection_cookie,
                predecessor_channel_cookie,
                successor_channel_cookie,
            )
            .expect("valid IN_R1/A1"),
            [
                [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
                &[88, 0, 0, 0, 0, 0, 0, 0],
                &[
                    u8::try_from(RTS_FLAG_RECYCLE_CHANNEL).expect("flag fits in u8"),
                    0,
                    4,
                    0
                ],
                &[6, 0, 0, 0, 1, 0, 0, 0],
                &[3, 0, 0, 0],
                virtual_connection_cookie.as_bytes(),
                &[3, 0, 0, 0],
                predecessor_channel_cookie.as_bytes(),
                &[3, 0, 0, 0],
                successor_channel_cookie.as_bytes(),
            ]
            .concat()
        );
        assert_eq!(
            encode_rts_in_recycle_a5(successor_channel_cookie).expect("valid IN_R1/A5"),
            [
                [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
                &[40, 0, 0, 0, 0, 0, 0, 0],
                &[0, 0, 1, 0],
                &[3, 0, 0, 0],
                successor_channel_cookie.as_bytes(),
            ]
            .concat()
        );

        let a4 = [
            [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
            &[52, 0, 0, 0, 0, 0, 0, 0],
            &[0, 0, 4, 0],
            &[13, 0, 0, 0, 0, 0, 0, 0],
            &[6, 0, 0, 0, 0x12, 0x34, 0x56, 0x78],
            &[0, 0, 0, 0, 0, 0x20, 0, 0],
            &[2, 0, 0, 0, 0xc0, 0xd4, 1, 0],
        ]
        .concat();
        assert_eq!(
            decode_rts_in_recycle_a4(&a4).expect("valid IN_R1/A4"),
            RtsInRecycleA4 {
                version: 0x7856_3412,
                receive_window_size: 8 * 1024,
                connection_timeout: 120_000,
            }
        );

        let mut invalid_destination = a4;
        invalid_destination[24..28].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            decode_rts_in_recycle_a4(&invalid_destination),
            Err(RpcPduError::UnexpectedRtsDestination {
                expected: RTS_DESTINATION_FD_CLIENT,
                actual: 1,
            })
        );
    }

    #[test]
    fn rts_out_recycling_encodes_client_messages_and_decodes_peer_messages() {
        let virtual_connection_cookie = RtsCookie::new([0x10; RtsCookie::SIZE]);
        let predecessor_channel_cookie = RtsCookie::new([0x20; RtsCookie::SIZE]);
        let successor_channel_cookie = RtsCookie::new([0x30; RtsCookie::SIZE]);
        let a2 = [
            [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
            &[28, 0, 0, 0, 0, 0, 0, 0],
            &[
                u8::try_from(RTS_FLAG_RECYCLE_CHANNEL).expect("flag fits in u8"),
                0,
                1,
                0,
            ],
            &[13, 0, 0, 0, 0, 0, 0, 0],
        ]
        .concat();
        decode_rts_out_recycle_a2(&a2).expect("valid OUT_R1/A2");
        assert_eq!(
            encode_rts_out_recycle_a3(
                virtual_connection_cookie,
                predecessor_channel_cookie,
                successor_channel_cookie,
                8 * 1024,
            )
            .expect("valid OUT_R1/A3"),
            [
                [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
                &[96, 0, 0, 0, 0, 0, 0, 0],
                &[
                    u8::try_from(RTS_FLAG_RECYCLE_CHANNEL).expect("flag fits in u8"),
                    0,
                    5,
                    0
                ],
                &[6, 0, 0, 0, 1, 0, 0, 0],
                &[3, 0, 0, 0],
                virtual_connection_cookie.as_bytes(),
                &[3, 0, 0, 0],
                predecessor_channel_cookie.as_bytes(),
                &[3, 0, 0, 0],
                successor_channel_cookie.as_bytes(),
                &[0, 0, 0, 0, 0, 0x20, 0, 0],
            ]
            .concat()
        );

        let a6 = [
            [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
            &[44, 0, 0, 0, 0, 0, 0, 0],
            &[u8::try_from(RTS_FLAG_OUT_CHANNEL).expect("flag fits in u8"), 0, 3, 0],
            &[13, 0, 0, 0, 0, 0, 0, 0],
            &[6, 0, 0, 0, 0x12, 0x34, 0x56, 0x78],
            &[2, 0, 0, 0, 0xc0, 0xd4, 1, 0],
        ]
        .concat();
        assert_eq!(
            decode_rts_out_recycle_a6(&a6).expect("valid OUT_R1/A6"),
            RtsOutRecycleA6 {
                version: 0x7856_3412,
                connection_timeout: 120_000,
            }
        );
        assert_eq!(
            encode_rts_out_recycle_a7(successor_channel_cookie).expect("valid OUT_R1/A7"),
            [
                [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
                &[48, 0, 0, 0, 0, 0, 0, 0],
                &[u8::try_from(RTS_FLAG_OUT_CHANNEL).expect("flag fits in u8"), 0, 2, 0],
                &[13, 0, 0, 0, 2, 0, 0, 0],
                &[3, 0, 0, 0],
                successor_channel_cookie.as_bytes(),
            ]
            .concat()
        );

        let a10 = [
            [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
            &[24, 0, 0, 0, 0, 0, 0, 0],
            &[0, 0, 1, 0],
            &[10, 0, 0, 0],
        ]
        .concat();
        decode_rts_out_recycle_a10(&a10).expect("valid OUT_R1/A10");
        assert_eq!(encode_rts_out_recycle_a11().expect("valid OUT_R1/A11"), a10);
    }

    #[test]
    fn rts_initial_flow_rejects_invalid_headers_commands_and_fixed_values() {
        let mut conn_a3 = [
            [5, 0, PTYPE_RTS, RTS_PFC_FLAGS, 0x10, 0, 0, 0].as_slice(),
            &[28, 0, 0, 0, 0, 0, 0, 0],
            &[0, 0, 1, 0],
            &[2, 0, 0, 0, 0xc0, 0xd4, 1, 0],
        ]
        .concat();

        conn_a3[2] = PTYPE_REQUEST;
        assert_eq!(
            decode_rts_conn_a3(&conn_a3),
            Err(RpcPduError::UnexpectedPduType {
                expected: PTYPE_RTS,
                actual: PTYPE_REQUEST
            })
        );
        conn_a3[2] = PTYPE_RTS;

        let mut common_header_only = conn_a3[..RPC_COMMON_HEADER_SIZE].to_vec();
        common_header_only[8..10]
            .copy_from_slice(&(u16::try_from(RPC_COMMON_HEADER_SIZE).expect("header size fits in u16")).to_le_bytes());
        assert_eq!(
            decode_rts_conn_a3(&common_header_only),
            Err(RpcPduError::Truncated {
                actual: RPC_COMMON_HEADER_SIZE,
                required: RTS_HEADER_SIZE
            })
        );

        conn_a3[3] |= 0x04;
        assert_eq!(
            decode_rts_conn_a3(&conn_a3),
            Err(RpcPduError::InvalidRtsPfcFlags {
                actual: RTS_PFC_FLAGS | 0x04
            })
        );
        conn_a3[3] = RTS_PFC_FLAGS;

        conn_a3[16..18].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(
            decode_rts_conn_a3(&conn_a3),
            Err(RpcPduError::UnexpectedRtsFlags {
                expected: RTS_FLAG_NONE,
                actual: 1
            })
        );
        conn_a3[16..18].copy_from_slice(&RTS_FLAG_NONE.to_le_bytes());

        conn_a3[10..12].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(
            decode_rts_conn_a3(&conn_a3),
            Err(RpcPduError::AuthenticationUnsupported { auth_length: 1 })
        );
        conn_a3[10..12].copy_from_slice(&0u16.to_le_bytes());

        conn_a3[8..10].copy_from_slice(&27u16.to_le_bytes());
        assert_eq!(
            decode_rts_conn_a3(&conn_a3),
            Err(RpcPduError::InvalidRtsBodyLength { expected: 8, actual: 7 })
        );
        conn_a3[8..10].copy_from_slice(&28u16.to_le_bytes());

        conn_a3[12] = 1;
        assert_eq!(
            decode_rts_conn_a3(&conn_a3),
            Err(RpcPduError::UnexpectedRtsCallId { actual: 1 })
        );
        conn_a3[12] = 0;

        conn_a3[18..20].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(
            decode_rts_conn_a3(&conn_a3),
            Err(RpcPduError::UnexpectedRtsCommandCount { expected: 1, actual: 2 })
        );
        conn_a3[18..20].copy_from_slice(&1u16.to_le_bytes());

        conn_a3[20..24].copy_from_slice(&RTS_COMMAND_VERSION.to_le_bytes());
        assert_eq!(
            decode_rts_conn_a3(&conn_a3),
            Err(RpcPduError::UnexpectedRtsCommandType {
                expected: RTS_COMMAND_CONNECTION_TIMEOUT,
                actual: RTS_COMMAND_VERSION
            })
        );
        conn_a3[20..24].copy_from_slice(&RTS_COMMAND_CONNECTION_TIMEOUT.to_le_bytes());

        conn_a3[24..28].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            decode_rts_conn_a3(&conn_a3),
            Err(RpcPduError::InvalidRtsConnectionTimeout { actual: 0 })
        );

        assert_eq!(
            encode_rts_conn_a1(RtsCookie::new([0; 16]), RtsCookie::new([1; 16]), 8 * 1024 - 1),
            Err(RpcPduError::InvalidRtsReceiveWindowSize { actual: 8 * 1024 - 1 })
        );
        assert_eq!(
            encode_rts_conn_b1(
                RtsCookie::new([0; 16]),
                RtsCookie::new([1; 16]),
                128 * 1024 - 1,
                0,
                RtsCookie::new([2; 16])
            ),
            Err(RpcPduError::InvalidRtsChannelLifetime { actual: 128 * 1024 - 1 })
        );
        assert_eq!(
            encode_rts_conn_b1(
                RtsCookie::new([0; 16]),
                RtsCookie::new([1; 16]),
                128 * 1024,
                59_999,
                RtsCookie::new([2; 16])
            ),
            Err(RpcPduError::InvalidRtsClientKeepalive { actual: 59_999 })
        );
    }

    #[test]
    fn rpch_setup_enforces_initial_sequence_and_opens_after_a3_and_c2() {
        let settings = RpcHttpV2Settings::new(128 * 1024, 256 * 1024, 0).expect("valid settings");
        let virtual_connection_cookie = RtsCookie::new([0x10; RtsCookie::SIZE]);
        let out_channel_cookie = RtsCookie::new([0x20; RtsCookie::SIZE]);
        let in_channel_cookie = RtsCookie::new([0x30; RtsCookie::SIZE]);
        let association_group_id = RtsCookie::new([0x40; RtsCookie::SIZE]);
        let mut setup = RpcHttpV2Setup::with_cookies(
            settings,
            virtual_connection_cookie,
            out_channel_cookie,
            in_channel_cookie,
            association_group_id,
        );

        setup.start_in_request().expect("start IN request");
        assert_eq!(setup.state(), RpcHttpV2State::InRequestStarted);
        assert_eq!(
            setup.out_request_body().expect("CONN/A1"),
            encode_rts_conn_a1(virtual_connection_cookie, out_channel_cookie, 128 * 1024).expect("valid CONN/A1")
        );
        assert_eq!(setup.state(), RpcHttpV2State::OutRequestStarted);
        assert_eq!(
            setup.in_request_initial_pdu().expect("CONN/B1"),
            encode_rts_conn_b1(
                virtual_connection_cookie,
                in_channel_cookie,
                256 * 1024,
                0,
                association_group_id,
            )
            .expect("valid CONN/B1")
        );

        setup
            .accept_out_response(http::StatusCode::OK.as_u16(), Some("application/rpc"), Some(128 * 1024))
            .expect("successful OUT response");
        setup.receive_out_pdu(&conn_a3_pdu()).expect("CONN/A3");
        assert_eq!(setup.state(), RpcHttpV2State::AwaitingC2);
        assert_eq!(setup.in_ping_timeout(), Some(120_000));

        setup.receive_out_pdu(&conn_c2_pdu(1)).expect("CONN/C2");
        assert_eq!(setup.state(), RpcHttpV2State::Open);
        assert_eq!(setup.connection_timeout(), Some(120_000));
        assert_eq!(setup.peer_receive_window_size(), Some(128 * 1024));
        setup.require_open().expect("opened setup permits RPC PDUs");
        assert!(
            setup
                .ping_schedule(Duration::ZERO)
                .expect("opened setup provides a ping schedule")
                .ping_due(Duration::from_secs(120))
        );
    }

    #[test]
    fn rpch_flow_control_tracks_windows_and_acknowledgements() {
        let settings = RpcHttpV2Settings::new(128 * 1024, 256 * 1024, 0).expect("valid settings");
        let virtual_connection_cookie = RtsCookie::new([0x10; RtsCookie::SIZE]);
        let out_channel_cookie = RtsCookie::new([0x20; RtsCookie::SIZE]);
        let in_channel_cookie = RtsCookie::new([0x30; RtsCookie::SIZE]);
        let association_group_id = RtsCookie::new([0x40; RtsCookie::SIZE]);
        let mut setup = RpcHttpV2Setup::with_cookies(
            settings,
            virtual_connection_cookie,
            out_channel_cookie,
            in_channel_cookie,
            association_group_id,
        );
        setup.start_in_request().expect("start IN request");
        let _ = setup.out_request_body().expect("CONN/A1");
        let _ = setup.in_request_initial_pdu().expect("CONN/B1");
        setup
            .accept_out_response(http::StatusCode::OK.as_u16(), Some("application/rpc"), Some(128 * 1024))
            .expect("successful OUT response");
        setup.receive_out_pdu(&conn_a3_pdu()).expect("CONN/A3");
        setup.receive_out_pdu(&conn_c2_pdu(1)).expect("CONN/C2");

        let mut flow_control = setup.flow_control().expect("open RPCH flow control");
        flow_control.sent_rpc_pdu(64 * 1024).expect("send within peer window");
        assert_eq!(flow_control.send_available_window(), 64 * 1024);
        assert_eq!(
            flow_control.receive_flow_control_ack(RtsFlowControlAck {
                bytes_received: 64 * 1024,
                available_window: 128 * 1024,
                channel_cookie: in_channel_cookie,
            }),
            Ok(true)
        );
        assert_eq!(flow_control.send_available_window(), 128 * 1024);
        assert_eq!(
            flow_control.receive_flow_control_ack(RtsFlowControlAck {
                bytes_received: 0,
                available_window: 128 * 1024,
                channel_cookie: RtsCookie::new([0xff; RtsCookie::SIZE]),
            }),
            Ok(false)
        );

        flow_control.received_rpc_pdu(100 * 1024).expect("queue received PDU");
        assert_eq!(flow_control.receive_available_window(), 28 * 1024);
        assert_eq!(
            flow_control.consumed_rpc_pdu(100 * 1024),
            Ok(Some(RtsFlowControlAck {
                bytes_received: 100 * 1024,
                available_window: 128 * 1024,
                channel_cookie: out_channel_cookie,
            }))
        );

        flow_control.received_rpc_pdu(60 * 1024).expect("queue first PDU");
        assert_eq!(flow_control.consumed_rpc_pdu(30 * 1024), Ok(None));
        flow_control
            .received_rpc_pdu(60 * 1024)
            .expect("track received PDU after partial consumption");
        assert_eq!(flow_control.receive_available_window(), 38 * 1024);

        assert_eq!(
            flow_control.sent_rpc_pdu(128 * 1024),
            Err(RpcHttpV2FlowControlError::SendWindowExhausted {
                pdu_size: 128 * 1024,
                available_window: 128 * 1024,
            })
        );
        assert_eq!(
            flow_control.received_rpc_pdu(40 * 1024),
            Err(RpcHttpV2FlowControlError::ReceiveWindowExhausted {
                pdu_size: 40 * 1024,
                available_window: 38 * 1024,
            })
        );
        assert_eq!(
            flow_control.receive_flow_control_ack(RtsFlowControlAck {
                bytes_received: 64 * 1024 + 1,
                available_window: 128 * 1024,
                channel_cookie: in_channel_cookie,
            }),
            Err(RpcHttpV2FlowControlError::InvalidFlowControlAck {
                bytes_received: 64 * 1024 + 1,
                bytes_sent: 64 * 1024,
            })
        );
    }

    #[test]
    fn rpch_ping_schedule_uses_keepalive_and_connection_timeouts() {
        let mut keepalive =
            RpcHttpV2PingSchedule::new(Duration::from_secs(120), Duration::from_secs(30), Duration::ZERO);
        assert!(!keepalive.ping_due(Duration::from_secs(14)));
        assert!(keepalive.ping_due(Duration::from_secs(15)));
        keepalive.record_send(Duration::from_secs(15));
        assert!(!keepalive.ping_due(Duration::from_secs(29)));
        assert!(keepalive.ping_due(Duration::from_secs(30)));

        let connection = RpcHttpV2PingSchedule::new(Duration::from_secs(120), Duration::ZERO, Duration::ZERO);
        assert!(!connection.ping_due(Duration::from_secs(119)));
        assert!(connection.ping_due(Duration::from_secs(120)));
    }

    #[test]
    fn rpch_settings_use_default_keepalive_for_zero() {
        let settings = RpcHttpV2Settings::new(128 * 1024, 256 * 1024, 0).expect("valid settings");
        assert_eq!(settings.client_keepalive(), 0);
        assert_eq!(settings.effective_client_keepalive(), 300_000);
    }

    #[test]
    fn rpch_setup_rejects_wrong_order_and_invalid_c2_version() {
        let mut setup = RpcHttpV2Setup::new(RpcHttpV2Settings::default());
        assert_eq!(
            setup.in_request_initial_pdu(),
            Err(RpcHttpV2Error::InvalidState {
                action: "send CONN/B1",
                state: RpcHttpV2State::Initial,
            })
        );
        assert_eq!(setup.state(), RpcHttpV2State::Failed);

        let mut setup = RpcHttpV2Setup::new(RpcHttpV2Settings::default());
        setup.start_in_request().expect("start IN request");
        let _ = setup.out_request_body().expect("CONN/A1");
        let _ = setup.in_request_initial_pdu().expect("CONN/B1");
        setup
            .accept_out_response(http::StatusCode::OK.as_u16(), Some("application/rpc"), Some(128 * 1024))
            .expect("OUT response");
        setup.receive_out_pdu(&conn_a3_pdu()).expect("CONN/A3");
        let unsupported_c2 = conn_c2_pdu(2);
        assert_eq!(
            setup.receive_out_pdu(&unsupported_c2),
            Err(RpcHttpV2Error::UnsupportedProtocolVersion { actual: 2 })
        );
        assert_eq!(setup.state(), RpcHttpV2State::Failed);
    }

    #[test]
    fn rpch_setup_requires_a_bounded_out_response_body() {
        let mut setup = RpcHttpV2Setup::new(RpcHttpV2Settings::default());
        setup.start_in_request().expect("start IN request");
        let _ = setup.out_request_body().expect("CONN/A1");
        let _ = setup.in_request_initial_pdu().expect("CONN/B1");

        assert_eq!(
            setup.accept_out_response(http::StatusCode::OK.as_u16(), Some("application/rpc"), None),
            Err(RpcHttpV2Error::OutResponseContentLength { actual: None })
        );
        assert_eq!(setup.state(), RpcHttpV2State::Failed);
    }

    #[test]
    fn rpch_http_request_encodes_required_headers_and_target() {
        let target = crate::GwConnectTarget {
            gw_endpoint: "rdg.contoso.com".to_owned(),
            gw_user: "alice".to_owned(),
            gw_pass: "secret".to_owned(),
            server: "rdp.contoso.com".to_owned(),
            server_port: 3389,
            smart_card: None,
        };
        let request = build_rpch_v2_request(
            RpchHttpChannel::In,
            "rdg.contoso.com",
            &target,
            128 * 1024,
            Some("Basic YWxpY2U6c2VjcmV0"),
            Some("session=abc"),
            (),
        )
        .expect("valid IN request");

        assert_eq!(request.method(), "RPC_IN_DATA");
        assert_eq!(request.uri(), "/rpc/rpcproxy.dll?rdp.contoso.com:3389");
        assert_eq!(request.headers()[http::header::ACCEPT], "application/rpc");
        assert_eq!(request.headers()[http::header::CACHE_CONTROL], "no-cache");
        assert_eq!(request.headers()[http::header::CONNECTION], "Keep-Alive");
        assert_eq!(request.headers()[http::header::CONTENT_LENGTH], "131072");
        assert_eq!(request.headers()[http::header::HOST], "rdg.contoso.com");
        assert_eq!(request.headers()[http::header::PRAGMA], "no-cache");
        assert_eq!(request.headers()["Protocol"], "1.0");
        assert_eq!(request.headers()[http::header::USER_AGENT], "MSRPC");
        assert_eq!(request.headers()[http::header::AUTHORIZATION], "Basic YWxpY2U6c2VjcmV0");
        assert_eq!(request.headers()[http::header::COOKIE], "session=abc");

        assert!(matches!(
            build_rpch_v2_request(
                RpchHttpChannel::Out,
                "rdg.contoso.com",
                &target,
                RPCH_OUT_CONTENT_LENGTH - 1,
                None,
                None,
                (),
            ),
            Err(RpcHttpV2Error::InvalidContentLength {
                channel: RpchHttpChannel::Out,
                actual,
            }) if actual == RPCH_OUT_CONTENT_LENGTH - 1
        ));
        let mut invalid_target = target;
        invalid_target.server = "rdp/invalid".to_owned();
        assert!(matches!(
            build_rpch_v2_request(
                RpchHttpChannel::In,
                "rdg.contoso.com",
                &invalid_target,
                128 * 1024,
                None,
                None,
                (),
            ),
            Err(RpcHttpV2Error::InvalidTargetServer)
        ));
    }

    #[test]
    fn pdu_stream_caps_pending_bytes() {
        let mut stream = RpcPduStream::new(32).expect("valid maximum");
        assert_eq!(
            stream.push(&[0; 32 * MAX_PENDING_RPC_FRAGMENTS + 1]),
            Err(RpcPduError::PendingBytesExceedMaximum {
                actual: 32 * MAX_PENDING_RPC_FRAGMENTS + 1,
                maximum: 32 * MAX_PENDING_RPC_FRAGMENTS,
            })
        );
    }
}
