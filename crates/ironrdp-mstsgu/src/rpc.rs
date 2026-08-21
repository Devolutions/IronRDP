#![cfg_attr(test, allow(unused_crate_dependencies))]

//! DCE/RPC common-header and fragment codecs.
//!
//! This module frames connection-oriented DCE/RPC PDUs.
//! It is not a live RPC-over-HTTP transport.
//! The staged TsProxy NDR control codecs do not provide a live RPC-over-HTTP transport.
//! RTS, packet-integrity signing, and the RPCH client belong in later work.
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

// This fragment foundation assumes the conventional first presentation context.
// A later bind codec will negotiate and supply the context identifier.
const RPC_CONTEXT_ID: u16 = 0;

/// Conventional DCE/RPC fragment maximum used for the initial bind.
pub const DEFAULT_FRAGMENT_SIZE: u16 = 0x10b8;

/// Maximum-sized fragment equivalents the stream may buffer before the caller drains them.
pub const MAX_PENDING_RPC_FRAGMENTS: usize = 16;

const MAXIMUM_RESPONSE_ALLOC_HINT: usize = 0x7fff_ffff;

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

/// A DCE/RPC syntax version staged for a later bind codec.
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
        pfc_flags: header.pfc_flags(),
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

#[expect(
    dead_code,
    reason = "the control codecs are staged before an RPC transport consumes them"
)]
mod tsgu {
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
    /// `TsProxyCreateChannel` operation number.
    pub(crate) const TSPROXY_CREATE_CHANNEL_OPNUM: u16 = 4;

    const NDR_REFERENT_ID: u32 = 0x0002_0000;
    const TSG_COMPONENT_ID: u16 = 0x5452;
    const TSG_PACKET_TYPE_VERSIONCAPS: u32 = 0x0000_5643;
    const TSG_PACKET_TYPE_VERSIONCAPS_ID: u16 = 0x5643;
    const TSG_PACKET_TYPE_QUARREQUEST: u32 = 0x0000_5152;
    const TSG_PACKET_TYPE_RESPONSE: u32 = 0x0000_5052;
    const TSG_PACKET_TYPE_QUARENC_RESPONSE: u32 = 0x0000_4552;
    const TSG_CAPABILITY_TYPE_NAP: u32 = 1;
    const MAX_CAPABILITIES: usize = 32;
    const MAX_MACHINE_NAME_CHARS: usize = 513;
    const MAX_RESOURCE_NAME_CHARS: usize = 32_767;
    const MAX_STATEMENT_OF_HEALTH_SIZE: usize = 8_000;
    const MAX_RESPONSE_DATA_SIZE: usize = 24_000;
    const MAX_CERT_CHAIN_CHARS: usize = 24_000;

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
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum RpcStubError {
        ContextHandleLength { actual: usize },
        NullContextHandle,
        EmptyResourceName,
        EmbeddedNulInResourceName,
        ResourceNameTooLong { actual: usize, maximum: usize },
        StatementOfHealthTooLarge { actual: usize },
        ResponseDataTooLarge { actual: usize },
        CertificateChainTooLarge { actual: usize },
        CapabilityCountTooLarge { actual: u32 },
        LengthOverflow,
        ResponseLength { actual: usize, expected: usize },
        RequiredNdrPointerIsNull,
        UnexpectedPacketId { expected: u32, actual: u32 },
        UnexpectedPacketSwitch { expected: u32, actual: u32 },
        UnexpectedComponentId { expected: u16, actual: u16 },
        UnexpectedCapabilityType { expected: u32, actual: u32 },
        InvalidNdrArrayLength { actual: u32, expected: u32 },
        InvalidNdrBoolean { value: u32 },
        ConflictingRedirectionFlags,
        InvalidQuarencFlags { actual: u32 },
        InvalidUtf16,
        UnterminatedNdrString,
        RpcStatus { value: u32 },
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
                Self::ResourceNameTooLong { actual, maximum } => {
                    write!(f, "resource name length {actual} exceeds {maximum}")
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
                Self::LengthOverflow => f.write_str("rpc stub length overflow"),
                Self::ResponseLength { actual, expected } => {
                    write!(f, "invalid rpc stub length {actual}, expected {expected}")
                }
                Self::RequiredNdrPointerIsNull => f.write_str("required ndr pointer is null"),
                Self::UnexpectedPacketId { expected, actual } => {
                    write!(f, "unexpected packet id 0x{actual:08x}, expected 0x{expected:08x}")
                }
                Self::UnexpectedPacketSwitch { expected, actual } => {
                    write!(f, "unexpected packet switch 0x{actual:08x}, expected 0x{expected:08x}")
                }
                Self::UnexpectedComponentId { expected, actual } => {
                    write!(f, "unexpected component id 0x{actual:04x}, expected 0x{expected:04x}")
                }
                Self::UnexpectedCapabilityType { expected, actual } => {
                    write!(f, "unexpected capability type {actual}, expected {expected}")
                }
                Self::InvalidNdrArrayLength { actual, expected } => {
                    write!(f, "invalid ndr array length {actual}, expected {expected}")
                }
                Self::InvalidNdrBoolean { value } => write!(f, "invalid ndr boolean value {value}"),
                Self::ConflictingRedirectionFlags => f.write_str("enable-all and disable-all flags conflict"),
                Self::InvalidQuarencFlags { actual } => write!(f, "invalid quarenc flags {actual}"),
                Self::InvalidUtf16 => f.write_str("invalid utf-16 string"),
                Self::UnterminatedNdrString => f.write_str("unterminated ndr string"),
                Self::RpcStatus { value } => write!(f, "rpc operation returned hresult 0x{value:08x}"),
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

        pub(crate) fn encode(self) -> Vec<u8> {
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
            output
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
            let machine_name = encode_ndr_string(self.machine_name, MAX_MACHINE_NAME_CHARS)?;
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
            let resource_name = encode_ndr_string(self.resource_name, MAX_RESOURCE_NAME_CHARS)?;
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
        require_ndr_pointer(read_u32(version_caps, 4)?)?;
        let capability_count = read_u32(version_caps, 8)?;
        if usize::try_from(capability_count).map_err(|_| RpcStubError::LengthOverflow)? > MAX_CAPABILITIES {
            return Err(RpcStubError::CapabilityCountTooLarge {
                actual: capability_count,
            });
        }
        offset = version_caps_end;
        let capability_array_count = read_u32(source, offset)?;
        if capability_array_count != capability_count {
            return Err(RpcStubError::InvalidNdrArrayLength {
                actual: capability_array_count,
                expected: capability_count,
            });
        }
        offset = offset.checked_add(4).ok_or(RpcStubError::LengthOverflow)?;

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
            capabilities |= read_u32(source, offset.checked_add(8).ok_or(RpcStubError::LengthOverflow)?)?;
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
        validate_hresult(read_u32(source, offset + RpcContextHandle::SIZE + 4)?)?;

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
            offset = padded_to_ndr_4(data_end)?;
            response_data.to_vec()
        };
        let response_size = offset.checked_add(4).ok_or(RpcStubError::LengthOverflow)?;
        if source.len() != response_size {
            return Err(RpcStubError::ResponseLength {
                actual: source.len(),
                expected: response_size,
            });
        }
        validate_hresult(read_u32(source, offset)?)?;

        Ok(TsProxyAuthorizeTunnelResponse {
            response_data,
            redirection_flags,
        })
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
        let channel_context = RpcContextHandle::from_bytes(&source[..RpcContextHandle::SIZE])?.require_non_null()?;
        let channel_id = read_u32(source, RpcContextHandle::SIZE)?;
        validate_hresult(read_u32(source, RpcContextHandle::SIZE + 4)?)?;
        Ok(TsProxyCreateChannelResponse {
            channel_context,
            channel_id,
        })
    }

    fn encode_ndr_pointer(output: &mut Vec<u8>, index: u32) {
        output.extend_from_slice(&(NDR_REFERENT_ID + index * 4).to_le_bytes());
    }

    fn encode_ndr_string(value: &str, maximum: usize) -> Result<Vec<u16>, RpcStubError> {
        if value.is_empty() {
            return Err(RpcStubError::EmptyResourceName);
        }
        if value.contains('\0') {
            return Err(RpcStubError::EmbeddedNulInResourceName);
        }
        let mut encoded: Vec<_> = value.encode_utf16().collect();
        encoded.push(0);
        if encoded.len() > maximum {
            return Err(RpcStubError::ResourceNameTooLong {
                actual: encoded.len(),
                maximum,
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
        Ok((text, padded_to_ndr_4(characters_end)?))
    }

    fn pad_ndr_4(output: &mut Vec<u8>) {
        let padding = (4 - output.len() % 4) % 4;
        output.resize(output.len() + padding, 0);
    }

    fn padded_to_ndr_4(length: usize) -> Result<usize, RpcStubError> {
        length
            .checked_add(3)
            .map(|value| value & !3)
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

        #[test]
        fn requests_encode_bounded_ndr32_control_vectors() {
            let create_tunnel = TsProxyCreateTunnelRequest::new(3).encode();
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
            response[28..32].copy_from_slice(&u32::try_from(MAX_RESPONSE_DATA_SIZE + 1).expect("fits").to_le_bytes());
            assert_eq!(
                decode_tsgu_authorize_tunnel_response(&response),
                Err(RpcStubError::ResponseDataTooLarge {
                    actual: MAX_RESPONSE_DATA_SIZE + 1
                })
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
            assert_eq!(
                decode_tsgu_create_channel_response(&response),
                Err(RpcStubError::RpcStatus { value: 0x8007_59d8 })
            );
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
