//! Power request redirection channel.
//!
//! The Cloud PC reports whether it currently holds power requests, and the client keeps the physical device awake while it does.
//! On Windows, an embedder can do this with `PowerSetRequest` for `PowerRequestSystemRequired` and
//! `PowerRequestDisplayRequired`, cleared again once the Cloud PC holds none.
//! See [`PROTOCOL.md`] for the layouts and the client behavior.
//!
//! [`PROTOCOL.md`]: https://github.com/Devolutions/IronRDP/blob/master/crates/ironrdp-w365/PROTOCOL.md#power-request-redirection

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, decode, ensure_size, impl_as_any,
    invalid_field_err,
};
use ironrdp_dvc::{DvcClientProcessor, DvcEncode, DvcMessage, DvcProcessor};
use ironrdp_pdu::PduResult;
use tracing::{debug, warn};

use crate::{HEADER_SIZE, PROTOCOL_VERSION_1, decode_header};

/// Dynamic virtual channel name.
pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::PowerRequestRedirection";

const READY_SIZE: usize = 4 /* ProtocolVersion */ + 4 /* ProtocolFlags */;
const FLAGS_SIZE: usize = 4 /* PowerRequestFlags */;

const PDU_TYPE_SERVER_READY: u16 = 0x0001;
const PDU_TYPE_CLIENT_READY: u16 = 0x0002;
const PDU_TYPE_POWER_REQUEST_INFO_CHANGED: u16 = 0x0003;

/// The Cloud PC holds at least one power request.
const CURRENT_REQUESTS: u32 = 0x0000_0001;

/// The Cloud PC holds no power request.
const NO_CURRENT_REQUESTS: u32 = 0x0000_0002;

/// Power request redirection PDU.
///
/// Every PDU starts with a 16-bit type and a 32-bit length that counts the whole PDU, header included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerRequestPdu {
    /// Server to client: the server's protocol version and flags.
    ServerReady { version: u32, flags: u32 },
    /// Client to server: the client's protocol version and flags.
    ClientReady { version: u32, flags: u32 },
    /// Server to client: the Cloud PC's power requests changed.
    ///
    /// Exactly one of the current-requests and no-current-requests flags is set.
    PowerRequestInfoChanged { flags: u32 },
}

impl PowerRequestPdu {
    const NAME: &'static str = "POWER_REQUEST_PDU";

    /// Creates the client-ready PDU Windows App sends.
    pub fn client_ready() -> Self {
        Self::ClientReady {
            version: PROTOCOL_VERSION_1,
            flags: 0,
        }
    }

    fn pdu_type(&self) -> u16 {
        match self {
            Self::ServerReady { .. } => PDU_TYPE_SERVER_READY,
            Self::ClientReady { .. } => PDU_TYPE_CLIENT_READY,
            Self::PowerRequestInfoChanged { .. } => PDU_TYPE_POWER_REQUEST_INFO_CHANGED,
        }
    }

    fn body_size(&self) -> usize {
        match self {
            Self::ServerReady { .. } | Self::ClientReady { .. } => READY_SIZE,
            Self::PowerRequestInfoChanged { .. } => FLAGS_SIZE,
        }
    }
}

impl Encode for PowerRequestPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u16(self.pdu_type());
        dst.write_u32(cast_length!("pduLength", self.size())?);
        match *self {
            Self::ServerReady { version, flags } | Self::ClientReady { version, flags } => {
                dst.write_u32(version);
                dst.write_u32(flags);
            }
            Self::PowerRequestInfoChanged { flags } => dst.write_u32(flags),
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn size(&self) -> usize {
        HEADER_SIZE + self.body_size()
    }
}

impl<'de> Decode<'de> for PowerRequestPdu {
    /// Decodes one PDU and consumes exactly `pduLength` bytes.
    ///
    /// Like Windows App, the decoder checks each PDU type against a minimum length and ignores bytes past the fields it knows.
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let (pdu_type, mut body) = decode_header(src, Self::NAME)?;

        match pdu_type {
            PDU_TYPE_SERVER_READY | PDU_TYPE_CLIENT_READY => {
                ensure_size!(in: body, size: READY_SIZE);
                let version = body.read_u32();
                let flags = body.read_u32();
                Ok(if pdu_type == PDU_TYPE_SERVER_READY {
                    Self::ServerReady { version, flags }
                } else {
                    Self::ClientReady { version, flags }
                })
            }
            PDU_TYPE_POWER_REQUEST_INFO_CHANGED => {
                ensure_size!(in: body, size: FLAGS_SIZE);
                Ok(Self::PowerRequestInfoChanged { flags: body.read_u32() })
            }
            _ => Err(invalid_field_err!("pduType", "unknown power request PDU type", in: src)),
        }
    }
}

impl DvcEncode for PowerRequestPdu {}

/// Host behavior for power request changes.
///
/// The processor calls it from the session loop, so it must return promptly.
pub trait PowerRequestHandler: Send {
    /// The Cloud PC started holding power requests (`true`) or released its last one (`false`).
    ///
    /// While active, the client keeps the physical device and its display awake.
    /// Closing the channel while active reports `false`.
    fn power_requests_changed(&mut self, active: bool);
}

/// Client processor for `Microsoft::Windows::RDS::PowerRequestRedirection`.
pub struct PowerRequestClient {
    handler: Box<dyn PowerRequestHandler>,
    /// Whether the handler was last told the Cloud PC holds power requests.
    active: bool,
}

impl core::fmt::Debug for PowerRequestClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PowerRequestClient").finish_non_exhaustive()
    }
}

impl PowerRequestClient {
    pub fn new(handler: Box<dyn PowerRequestHandler>) -> Self {
        Self { handler, active: false }
    }
}

impl_as_any!(PowerRequestClient);

impl DvcProcessor for PowerRequestClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        debug!(channel_id, "Power request channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu: PowerRequestPdu = match decode(payload) {
            Ok(pdu) => pdu,
            Err(error) => {
                warn!(%error, "Ignoring malformed power request PDU");
                return Ok(Vec::new());
            }
        };

        match pdu {
            PowerRequestPdu::ServerReady { version, flags } => {
                // Windows App answers with version 1 whatever version the server announces.
                debug!(version, flags, "Power request server ready");
                Ok(vec![Box::new(PowerRequestPdu::client_ready())])
            }
            PowerRequestPdu::PowerRequestInfoChanged { flags } => {
                let active = flags & CURRENT_REQUESTS != 0;
                let inactive = flags & NO_CURRENT_REQUESTS != 0;
                if active == inactive {
                    warn!(flags, "Ignoring power request flags that set both or neither state");
                } else {
                    debug!(active, "Cloud PC power requests changed");
                    self.active = active;
                    self.handler.power_requests_changed(active);
                }
                Ok(Vec::new())
            }
            PowerRequestPdu::ClientReady { .. } => {
                warn!("Ignoring a client-originated power request PDU received from the server");
                Ok(Vec::new())
            }
        }
    }

    fn close(&mut self, _channel_id: u32) {
        debug!("Power request channel closed");
        // The Cloud PC can no longer release its power requests, so stop keeping the device awake for them.
        if self.active {
            self.active = false;
            self.handler.power_requests_changed(false);
        }
    }
}

impl DvcClientProcessor for PowerRequestClient {}
