#![cfg_attr(doc, doc = include_str!("../README.md"))]
#![doc(html_logo_url = "https://cdnweb.devolutions.net/images/projects/devolutions/logos/devolutions-icon-shadow.svg")]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod inner;
pub mod ndr;

#[cfg(not(feature = "std"))]
use alloc::{boxed::Box, vec, vec::Vec};
use core::fmt;

pub use inner::RemoteGuardInnerPacket;
use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, ensure_size, impl_as_any,
    invalid_field_err,
};
use ironrdp_dvc::{DvcClientProcessor, DvcEncode, DvcMessage, DvcProcessor};
use ironrdp_pdu::{PduResult, decode_err, pdu_other_err};
#[cfg(test)]
use ironrdp_svc as _;
pub use ndr::{
    Endianness, KerberosRequest, KerberosResponse, NdrError, NdrLimits, NdrProfile, NtlmRequest, NtlmResponse,
};

pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::AuthRedirection";
pub const PROTOCOL_MAGIC: u32 = 0x4EAC_C3C8;

/// The outer envelope from [MS-RDPEAR section 2.2].
///
/// Decoding requires exactly one complete envelope, not a concatenated stream.
/// `Length` excludes the 24-byte header, matching captured Windows RCG traffic rather than the specification's "overall length" wording.
/// The payload remains encrypted; `Debug` deliberately omits it.
///
/// [MS-RDPEAR section 2.2]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpear/
#[derive(Clone, PartialEq, Eq)]
pub struct AuthRedirectionPdu {
    pub reserved: u32,
    pub encrypted_payload: Vec<u8>,
}

impl AuthRedirectionPdu {
    const NAME: &'static str = "AuthRedirectionPdu";
    pub const HEADER_SIZE: usize = 4 /* ProtocolMagic */ + 4 /* Length */ + 4 /* Version */
        + 4 /* Reserved */ + 8 /* TsPkgContext */;
}

impl fmt::Debug for AuthRedirectionPdu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct(Self::NAME)
            .field("reserved", &self.reserved)
            .field("encrypted_payload_length", &self.encrypted_payload.len())
            .finish()
    }
}

impl Encode for AuthRedirectionPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        if self.encrypted_payload.is_empty() {
            return Err(invalid_field_err!("payload", "empty encrypted payload", in: dst));
        }
        let length: u32 = cast_length!("Length", self.encrypted_payload.len())?;
        dst.write_u32(PROTOCOL_MAGIC);
        dst.write_u32(length);
        dst.write_u32(0);
        dst.write_u32(self.reserved);
        dst.write_u64(0);
        dst.write_slice(&self.encrypted_payload);
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        Self::HEADER_SIZE + self.encrypted_payload.len()
    }
}

impl<'de> Decode<'de> for AuthRedirectionPdu {
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: Self::HEADER_SIZE);
        let total_length = src.len();
        if src.read_u32() != PROTOCOL_MAGIC {
            return Err(invalid_field_err!("ProtocolMagic", "invalid protocol magic", in: src));
        }
        let length = src.read_u32();
        let expected_length: u32 = cast_length!("Length", total_length - Self::HEADER_SIZE, in: src)?;
        if length != expected_length {
            return Err(invalid_field_err!("Length", "length does not match encrypted payload", in: src));
        }
        let version = src.read_u32();
        if version != 0 {
            return Err(invalid_field_err!("Version", "unsupported RDPEAR version", in: src));
        }
        let reserved = src.read_u32();
        if src.read_u64() != 0 {
            return Err(invalid_field_err!("TsPkgContext", "network context must be zero", in: src));
        }
        if src.is_empty() {
            return Err(invalid_field_err!("payload", "empty encrypted payload", in: src));
        }
        let encrypted_payload = src.read_remaining().to_vec();
        Ok(Self {
            reserved,
            encrypted_payload,
        })
    }
}

impl DvcEncode for AuthRedirectionPdu {}

/// Security operations on the live CredSSP context, supplied by the session owner.
///
/// Implementations must authenticate/decrypt, validate the DER inner packet and package header, authorize and execute the package call, and encrypt its reply.
/// They must enforce cryptographic sequencing and bind operations to the session identity.
/// There is no plaintext or unsupported-operation fallback.
pub trait AuthRedirectionBackend: Send {
    /// Returns an encrypted response payload, never plaintext.
    ///
    /// `max_payload_size` is also an allocation bound for the response.
    /// Errors must not include credentials or payload contents.
    fn process(&mut self, encrypted_payload: &[u8], max_payload_size: usize) -> PduResult<Vec<u8>>;

    /// Releases session-bound security state when the DVC closes.
    fn close(&mut self);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Unopened,
    Open(u32),
    Failed(u32),
    Closed,
}

/// Opt-in, single-use DVC client for an already authenticated, secure session.
///
/// Any processing error on the active channel disables further requests.
/// A closed client cannot reuse its released security context.
pub struct AuthRedirectionClient {
    backend: Box<dyn AuthRedirectionBackend>,
    max_message_size: usize,
    state: State,
}

impl AuthRedirectionClient {
    /// Sets a complete-message bound including the outer header.
    pub fn new(backend: Box<dyn AuthRedirectionBackend>, max_message_size: u32) -> PduResult<Self> {
        let max_message_size =
            usize::try_from(max_message_size).map_err(|_| pdu_other_err!("message limit does not fit in usize"))?;
        if max_message_size <= AuthRedirectionPdu::HEADER_SIZE {
            return Err(pdu_other_err!(
                "message limit must allow a header and encrypted payload"
            ));
        }
        Ok(Self {
            backend,
            max_message_size,
            state: State::Unopened,
        })
    }
}

impl_as_any!(AuthRedirectionClient);

impl DvcProcessor for AuthRedirectionClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn max_message_size(&self) -> Option<usize> {
        Some(self.max_message_size)
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        if self.state != State::Unopened {
            return Err(pdu_other_err!("authentication redirection client is already used"));
        }
        self.state = State::Open(channel_id);
        Ok(Vec::new())
    }

    fn process(&mut self, channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        if self.state != State::Open(channel_id) {
            return Err(pdu_other_err!("authentication redirection channel is not active"));
        }
        // A failed backend call may already have advanced the security-context sequence.
        self.state = State::Failed(channel_id);
        if self.max_message_size < payload.len() {
            return Err(pdu_other_err!(
                "authentication redirection request exceeds message limit"
            ));
        }
        let request = AuthRedirectionPdu::decode(&mut ReadCursor::new(payload)).map_err(|error| decode_err!(error))?;
        let max_payload_size = self.max_message_size - AuthRedirectionPdu::HEADER_SIZE;
        let encrypted_payload = self.backend.process(&request.encrypted_payload, max_payload_size)?;
        if encrypted_payload.is_empty() || max_payload_size < encrypted_payload.len() {
            return Err(pdu_other_err!("authentication redirection response has invalid size"));
        }
        self.state = State::Open(channel_id);
        Ok(vec![Box::new(AuthRedirectionPdu {
            reserved: 0,
            encrypted_payload,
        })])
    }

    fn close(&mut self, channel_id: u32) {
        if matches!(self.state, State::Open(id) | State::Failed(id) if id == channel_id) {
            self.state = State::Closed;
            self.backend.close();
        }
    }
}

impl DvcClientProcessor for AuthRedirectionClient {}
