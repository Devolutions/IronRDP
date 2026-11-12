//! Battery status redirection channel.
//!
//! The client reports the physical device's battery so that the Cloud PC taskbar shows it.
//! Updates are paced by the server: each battery info request, which carries the state the Cloud PC currently shows,
//! allows the client one battery-info-changed PDU, sent only when the charge state or percentage differs or no
//! update was sent yet.
//! See [`PROTOCOL.md`] for the layouts and the client behavior.
//!
//! [`PROTOCOL.md`]: https://github.com/Devolutions/IronRDP/blob/master/crates/ironrdp-w365/PROTOCOL.md#battery-status-redirection

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use ironrdp_core::{
    Decode, DecodeResult, Encode, EncodeResult, ReadCursor, WriteCursor, cast_length, decode, ensure_size, impl_as_any,
    invalid_field_err,
};
use ironrdp_dvc::{DvcClientProcessor, DvcEncode, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, encode_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, warn};

use crate::{HEADER_SIZE, PROTOCOL_VERSION_1, decode_header};

/// Dynamic virtual channel name.
pub const CHANNEL_NAME: &str = "Microsoft::Windows::RDS::BatteryStatusRedirection";

const SERVER_READY_SIZE: usize = 4 /* ProtocolVersion */ + 4 /* ProtocolFlags */;
const CLIENT_READY_SIZE: usize = 4 /* ProtocolVersion */ + 4 /* ProtocolFlags */ + 2 /* Reserved */;
const BATTERY_INFO_SIZE: usize = 4 /* BatteryState */ + 2 /* BatteryLifePercent */ + 4 /* BatteryLifeMinutes */;

const PDU_TYPE_SERVER_READY: u16 = 0x0001;
const PDU_TYPE_CLIENT_READY: u16 = 0x0002;
const PDU_TYPE_BATTERY_INFO_REQUEST: u16 = 0x0003;
const PDU_TYPE_BATTERY_INFO_CHANGED: u16 = 0x0004;

/// Charge state of the physical device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryState(pub u32);

impl BatteryState {
    /// Running on battery.
    pub const ON_BATTERY: Self = Self(0);

    /// No system battery, or its state is unknown.
    pub const NO_BATTERY: Self = Self(1);

    /// Running on AC power.
    pub const AC_POWER: Self = Self(2);
}

/// Battery reading; `PROTOCOL.md` maps it from the Win32 `SYSTEM_POWER_STATUS` fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryInfo {
    pub state: BatteryState,
    /// Remaining charge in percent, or 255 when unknown; 0 when there is no battery.
    pub percent: u16,
    /// Remaining battery life in minutes, or 0 when unknown.
    pub minutes_remaining: u32,
}

impl BatteryInfo {
    fn encode_body(&self, dst: &mut WriteCursor<'_>) {
        dst.write_u32(self.state.0);
        dst.write_u16(self.percent);
        dst.write_u32(self.minutes_remaining);
    }

    fn decode_body(src: &mut ReadCursor<'_>) -> DecodeResult<Self> {
        ensure_size!(in: src, size: BATTERY_INFO_SIZE);
        Ok(Self {
            state: BatteryState(src.read_u32()),
            percent: src.read_u16(),
            minutes_remaining: src.read_u32(),
        })
    }
}

/// Battery status redirection PDU.
///
/// Every PDU starts with a 16-bit type and a 32-bit length that counts the whole PDU, header included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryPdu {
    /// Server to client: the server's protocol version and flags.
    ServerReady { version: u32, flags: u32 },
    /// Client to server: the client's protocol version and flags, followed by two reserved bytes.
    ClientReady { version: u32, flags: u32 },
    /// Server to client: the battery state the Cloud PC shows, allowing the client its next update.
    BatteryInfoRequest(BatteryInfo),
    /// Client to server: the physical device's battery state.
    BatteryInfoChanged(BatteryInfo),
}

impl BatteryPdu {
    const NAME: &'static str = "BATTERY_STATUS_PDU";

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
            Self::BatteryInfoRequest(_) => PDU_TYPE_BATTERY_INFO_REQUEST,
            Self::BatteryInfoChanged(_) => PDU_TYPE_BATTERY_INFO_CHANGED,
        }
    }

    fn body_size(&self) -> usize {
        match self {
            Self::ServerReady { .. } => SERVER_READY_SIZE,
            Self::ClientReady { .. } => CLIENT_READY_SIZE,
            Self::BatteryInfoRequest(_) | Self::BatteryInfoChanged(_) => BATTERY_INFO_SIZE,
        }
    }
}

impl Encode for BatteryPdu {
    fn encode(&self, dst: &mut WriteCursor<'_>) -> EncodeResult<()> {
        ensure_size!(in: dst, size: self.size());
        dst.write_u16(self.pdu_type());
        dst.write_u32(cast_length!("pduLength", self.size())?);
        match self {
            Self::ServerReady { version, flags } => {
                dst.write_u32(*version);
                dst.write_u32(*flags);
            }
            Self::ClientReady { version, flags } => {
                dst.write_u32(*version);
                dst.write_u32(*flags);
                dst.write_u16(0);
            }
            Self::BatteryInfoRequest(info) | Self::BatteryInfoChanged(info) => info.encode_body(dst),
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

impl<'de> Decode<'de> for BatteryPdu {
    /// Decodes one PDU and consumes exactly `pduLength` bytes.
    ///
    /// Like Windows App, the decoder checks each PDU type against a minimum length and ignores bytes past the fields it knows.
    fn decode(src: &mut ReadCursor<'de>) -> DecodeResult<Self> {
        let (pdu_type, mut body) = decode_header(src, Self::NAME)?;

        match pdu_type {
            PDU_TYPE_SERVER_READY => {
                ensure_size!(in: body, size: SERVER_READY_SIZE);
                Ok(Self::ServerReady {
                    version: body.read_u32(),
                    flags: body.read_u32(),
                })
            }
            PDU_TYPE_CLIENT_READY => {
                ensure_size!(in: body, size: CLIENT_READY_SIZE);
                Ok(Self::ClientReady {
                    version: body.read_u32(),
                    flags: body.read_u32(),
                })
            }
            PDU_TYPE_BATTERY_INFO_REQUEST => Ok(Self::BatteryInfoRequest(BatteryInfo::decode_body(&mut body)?)),
            PDU_TYPE_BATTERY_INFO_CHANGED => Ok(Self::BatteryInfoChanged(BatteryInfo::decode_body(&mut body)?)),
            _ => Err(invalid_field_err!("pduType", "unknown battery status PDU type", in: src)),
        }
    }
}

impl DvcEncode for BatteryPdu {}

/// Host behavior for the battery status channel.
///
/// The processor calls it from the session loop, so it must return promptly.
pub trait BatteryStatusHandler: Send {
    /// The server is ready for battery readings.
    ///
    /// Supply the current reading through [`BatteryStatusClient::update_battery`], then a new one whenever it changes.
    fn battery_status_requested(&mut self);
}

/// Client processor for `Microsoft::Windows::RDS::BatteryStatusRedirection`.
pub struct BatteryStatusClient {
    handler: Box<dyn BatteryStatusHandler>,
    channel_id: Option<u32>,
    /// Latest reading supplied by the caller.
    latest: Option<BatteryInfo>,
    /// State carried by the server's last battery info request.
    server_view: Option<BatteryInfo>,
    /// Whether the server's last battery info request has not been answered yet.
    update_allowed: bool,
    /// Whether an update was sent on this channel.
    sent: bool,
}

impl core::fmt::Debug for BatteryStatusClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BatteryStatusClient")
            .field("channel_id", &self.channel_id)
            .field("latest", &self.latest)
            .field("server_view", &self.server_view)
            .field("update_allowed", &self.update_allowed)
            .field("sent", &self.sent)
            .finish_non_exhaustive()
    }
}

impl BatteryStatusClient {
    pub fn new(handler: Box<dyn BatteryStatusHandler>) -> Self {
        Self {
            handler,
            channel_id: None,
            latest: None,
            server_view: None,
            update_allowed: false,
            sent: false,
        }
    }

    /// Records a new battery reading and encodes an update when the server's pacing allows one.
    ///
    /// The reading is kept while the channel is closed and sent once the server asks for it.
    pub fn update_battery(&mut self, info: BatteryInfo) -> PduResult<Vec<SvcMessage>> {
        self.latest = Some(info);
        let Some(channel_id) = self.channel_id else {
            return Ok(Vec::new());
        };
        match self.take_update() {
            Some(pdu) => encode_dvc_messages(channel_id, vec![Box::new(pdu)], ChannelFlags::empty())
                .map_err(|error| encode_err!(error)),
            None => Ok(Vec::new()),
        }
    }

    /// Returns the update the server's pacing allows, if the latest reading differs from what the server shows.
    ///
    /// Like Windows App, the remaining minutes alone never trigger an update.
    fn take_update(&mut self) -> Option<BatteryPdu> {
        let latest = self.latest?;
        if !self.update_allowed {
            return None;
        }
        let differs = self
            .server_view
            .is_none_or(|server| server.state != latest.state || server.percent != latest.percent);
        if self.sent && !differs {
            return None;
        }
        self.update_allowed = false;
        self.sent = true;
        Some(BatteryPdu::BatteryInfoChanged(latest))
    }
}

impl_as_any!(BatteryStatusClient);

impl DvcProcessor for BatteryStatusClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.channel_id = Some(channel_id);
        self.server_view = None;
        self.update_allowed = false;
        self.sent = false;
        debug!(channel_id, "Battery status channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu: BatteryPdu = match decode(payload) {
            Ok(pdu) => pdu,
            Err(error) => {
                warn!(%error, "Ignoring malformed battery status PDU");
                return Ok(Vec::new());
            }
        };

        match pdu {
            BatteryPdu::ServerReady { version, flags } => {
                // Windows App answers with version 1 whatever version the server announces.
                debug!(version, flags, "Battery status server ready");
                self.handler.battery_status_requested();
                Ok(vec![Box::new(BatteryPdu::client_ready())])
            }
            BatteryPdu::BatteryInfoRequest(server_view) => {
                debug!(?server_view, "Cloud PC requested a battery update");
                self.server_view = Some(server_view);
                self.update_allowed = true;
                Ok(match self.take_update() {
                    Some(pdu) => vec![Box::new(pdu)],
                    None => Vec::new(),
                })
            }
            BatteryPdu::ClientReady { .. } | BatteryPdu::BatteryInfoChanged(_) => {
                warn!("Ignoring a client-originated battery status PDU received from the server");
                Ok(Vec::new())
            }
        }
    }

    fn close(&mut self, _channel_id: u32) {
        self.channel_id = None;
        self.update_allowed = false;
        debug!("Battery status channel closed");
    }
}

impl DvcClientProcessor for BatteryStatusClient {}
