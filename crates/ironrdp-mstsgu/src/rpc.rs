//! DCE/RPC common-header and fragment codecs.
//!
//! This module frames connection-oriented DCE/RPC PDUs.
//! It is not a live RPC-over-HTTP transport.
//! TsProxy NDR, RTS, packet-integrity signing, and the RPCH client belong in later work.
//!
//! [C706]: https://pubs.opengroup.org/onlinepubs/9629399/toc.htm
//! [MS-RPCE]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rpce/290c38b1-92fe-4229-91e6-4fc376610c8d

use core::fmt;

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

const RPC_RESPONSE_HEADER_SIZE: usize =
    4 /* alloc_hint */ + 2 /* p_cont_id */ + 1 /* cancel_count */ + 1 /* reserved */;
const RPC_FAULT_HEADER_SIZE: usize = RPC_RESPONSE_HEADER_SIZE + 4 /* status */ + 4 /* reserved2 */;

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

/// `response` PDU type ([C706] 12.6.4.9).
pub const PTYPE_RESPONSE: u8 = 2;
/// `fault` PDU type ([C706] 12.6.4.9).
pub const PTYPE_FAULT: u8 = 3;

const RPC_CONTEXT_ID: u16 = 0;

/// Conventional DCE/RPC fragment maximum used for the initial bind.
pub const DEFAULT_FRAGMENT_SIZE: u16 = 0x10b8;

/// Maximum complete fragments the stream may buffer before the caller drains them.
pub const MAX_PENDING_RPC_FRAGMENTS: usize = 16;

/// Errors reported by the DCE/RPC common-header and fragment codecs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcPduError {
    Truncated { actual: usize, required: usize },
    UnsupportedVersion { major: u8, minor: u8 },
    UnsupportedDataRepresentation { value: [u8; 4] },
    InvalidFragmentLength { fragment_length: u16 },
    IncompleteFragment { actual: usize, fragment_length: u16 },
    AuthenticationUnsupported { auth_length: u16 },
    UnexpectedPduType { expected: u8, actual: u8 },
    FragmentedPduUnsupported { flags: u8 },
    UnexpectedResponseFragment { flags: u8 },
    ResponseFragmentCallId { expected: u32, actual: u32 },
    ResponseStubTooLarge { actual: usize, maximum: usize },
    InvalidFragmentSize { maximum: u16 },
    FragmentExceedsMaximum { fragment_length: u16, maximum: u16 },
    PendingBytesExceedMaximum { actual: usize, maximum: usize },
    LengthOverflow,
    UnexpectedContextId { actual: u16 },
    InvalidAllocHint { alloc_hint: u32, stub_length: usize },
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
            Self::LengthOverflow => f.write_str("rpc pdu length overflow"),
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
    alloc_hints: Vec<(usize, u32)>,
    stub: Vec<u8>,
}

impl RpcResponseReassembler {
    /// Creates a reassembler that rejects stubs larger than `maximum_stub_size`.
    pub fn new(maximum_stub_size: usize) -> Self {
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
pub struct RpcFault<'a> {
    pub call_id: u32,
    pub alloc_hint: u32,
    pub cancel_count: u8,
    pub reserved: u8,
    pub status: u32,
    pub reserved2: u32,
    pub stub: &'a [u8],
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
    body.push(response.reserved);
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
    body.extend_from_slice(&fault.reserved2.to_le_bytes());
    body.extend_from_slice(fault.stub);
    encode_unprotected_pdu(PTYPE_FAULT, fault.call_id, body)
}

/// Decodes one complete, unauthenticated RPC response PDU.
pub fn decode_rpc_response(source: &[u8], maximum_fragment_size: u16) -> Result<RpcResponse<'_>, RpcPduError> {
    let (header, body) = decode_unprotected_single_fragment(source, PTYPE_RESPONSE, maximum_fragment_size)?;
    let response = decode_rpc_response_body(header, body)?;
    validate_single_response_alloc_hint(response)?;
    Ok(response)
}

/// Decodes one unauthenticated RPC response fragment for a response reassembler.
pub fn decode_rpc_response_fragment(source: &[u8], maximum_fragment_size: u16) -> Result<RpcResponse<'_>, RpcPduError> {
    let (header, body) = decode_unprotected_fragment(source, PTYPE_RESPONSE, maximum_fragment_size)?;
    decode_rpc_response_body(header, body)
}

/// Decodes one complete, unauthenticated RPC fault PDU.
pub fn decode_rpc_fault(source: &[u8], maximum_fragment_size: u16) -> Result<RpcFault<'_>, RpcPduError> {
    let (header, body) = decode_unprotected_single_fragment(source, PTYPE_FAULT, maximum_fragment_size)?;
    let fault_header = body.get(..RPC_FAULT_HEADER_SIZE).ok_or(RpcPduError::Truncated {
        actual: body.len(),
        required: RPC_FAULT_HEADER_SIZE,
    })?;
    let context_id = read_u16(fault_header, 4)?;
    if context_id != RPC_CONTEXT_ID {
        return Err(RpcPduError::UnexpectedContextId { actual: context_id });
    }

    Ok(RpcFault {
        call_id: header.call_id(),
        alloc_hint: read_u32(fault_header, 0)?,
        cancel_count: fault_header[6],
        reserved: fault_header[7],
        status: read_u32(fault_header, 8)?,
        reserved2: read_u32(fault_header, 12)?,
        stub: &body[RPC_FAULT_HEADER_SIZE..],
    })
}

fn decode_rpc_response_body<'a>(header: RpcCommonHeader, body: &'a [u8]) -> Result<RpcResponse<'a>, RpcPduError> {
    let request_header = body.get(..RPC_RESPONSE_HEADER_SIZE).ok_or(RpcPduError::Truncated {
        actual: body.len(),
        required: RPC_RESPONSE_HEADER_SIZE,
    })?;
    let alloc_hint = read_u32(request_header, 0)?;
    let context_id = read_u16(request_header, 4)?;
    if context_id != RPC_CONTEXT_ID {
        return Err(RpcPduError::UnexpectedContextId { actual: context_id });
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
