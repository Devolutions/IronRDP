//! Server state machine for the [MS-RDPEL] dynamic virtual channel.
//!
//! [MS-RDPEL]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpel/

use alloc::{boxed::Box, vec, vec::Vec};

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcMessage, DvcProcessor, DvcServerProcessor};
use ironrdp_pdu::PduResult;
use tracing::{debug, warn};

use crate::CHANNEL_NAME;
use crate::pdu::{
    BaseLocation3dPdu, FourByteFloat, FourByteSignedInteger, LocationPdu, LocationSource, ProtocolVersion, ReadyPdu,
};

const NANODEGREES_PER_DEGREE: i64 = 1_000_000_000;
const MAX_LATITUDE_NANODEGREES: i64 = 90 * NANODEGREES_PER_DEGREE;
const MAX_LONGITUDE_NANODEGREES: i64 = 180 * NANODEGREES_PER_DEGREE;
const MAX_HEADING_NANODEGREES: i64 = 360 * NANODEGREES_PER_DEGREE;

/// Reconstructed client geographic fix delivered to [`LocationHandler`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocationFix {
    pub latitude: f64,
    pub longitude: f64,
    pub altitude: i32,
    pub speed: Option<f64>,
    pub heading: Option<f64>,
    pub horizontal_accuracy: Option<f64>,
    pub source: Option<LocationSource>,
}

/// Callback interface for server-side [MS-RDPEL] location updates.
///
/// [MS-RDPEL]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpel/
pub trait LocationHandler: Send {
    fn on_ready(&mut self, version: ProtocolVersion) {
        debug!(?version, "Location client ready");
    }

    fn on_location(&mut self, fix: LocationFix);

    fn on_closed(&mut self) {}
}

#[derive(Debug, Clone, Copy)]
struct StoredFix {
    latitude_nanodeg: i64,
    longitude_nanodeg: i64,
    altitude: i32,
    speed_nanodeg: Option<i64>,
    heading_nanodeg: Option<i64>,
    horizontal_accuracy: Option<f64>,
    source: Option<LocationSource>,
}

impl StoredFix {
    fn to_public(self) -> LocationFix {
        LocationFix {
            latitude: nanodegrees_to_f64(self.latitude_nanodeg),
            longitude: nanodegrees_to_f64(self.longitude_nanodeg),
            altitude: self.altitude,
            speed: self.speed_nanodeg.map(nanodegrees_to_f64),
            heading: self.heading_nanodeg.map(nanodegrees_to_f64),
            horizontal_accuracy: self.horizontal_accuracy,
            source: self.source,
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    clippy::as_conversions,
    reason = "bounded 26-bit FOUR_BYTE_FLOAT values scaled to 10^-9 units fit within the 53-bit f64 mantissa after stripping trailing decimal zeros"
)]
fn nanodegrees_to_f64(value: i64) -> f64 {
    let mut magnitude = value.unsigned_abs();
    let mut exponent = 9i32;
    while 0 < exponent && magnitude.is_multiple_of(10) {
        magnitude /= 10;
        exponent -= 1;
    }
    let scaled = (magnitude as f64) / 10f64.powi(exponent);
    if value < 0 { -scaled } else { scaled }
}

#[derive(Debug, Clone, Copy)]
enum State {
    Closed,
    WaitingClientReady,
    Ready { previous: Option<StoredFix> },
}

/// Server processor for `Microsoft::Windows::RDS::Location`.
pub struct LocationServer {
    handler: Box<dyn LocationHandler>,
    protocol_version: ProtocolVersion,
    state: State,
}

impl LocationServer {
    #[must_use]
    pub fn new(handler: Box<dyn LocationHandler>) -> Self {
        Self {
            handler,
            protocol_version: ProtocolVersion::V2,
            state: State::Closed,
        }
    }

    #[must_use]
    pub fn with_protocol_version(mut self, version: ProtocolVersion) -> Self {
        self.protocol_version = version;
        self
    }

    fn apply_base(&mut self, pdu: BaseLocation3dPdu) {
        let lat = pdu.latitude.nanodegrees();
        let lon = pdu.longitude.nanodegrees();
        let alt = pdu.altitude.value();
        if !valid_coordinates(lat, lon, alt) {
            warn!("Ignoring out-of-range base location PDU and invalidating delta base");
            self.state = State::Ready { previous: None };
            return;
        }
        let speed_nanodeg = pdu.speed.map(FourByteFloat::nanodegrees);
        let heading_nanodeg = pdu.heading.map(FourByteFloat::nanodegrees);
        let horizontal_accuracy_nanodeg = pdu.horizontal_accuracy.map(FourByteFloat::nanodegrees);
        let valid_telemetry = speed_nanodeg.is_none_or(|s| 0 <= s)
            && heading_nanodeg.is_none_or(|h| (0..=MAX_HEADING_NANODEGREES).contains(&h))
            && horizontal_accuracy_nanodeg.is_none_or(|a| 0 <= a);
        if !valid_telemetry {
            warn!("Ignoring base location PDU with invalid optional telemetry and invalidating delta base");
            self.state = State::Ready { previous: None };
            return;
        }
        let stored = StoredFix {
            latitude_nanodeg: lat,
            longitude_nanodeg: lon,
            altitude: alt,
            speed_nanodeg,
            heading_nanodeg,
            horizontal_accuracy: horizontal_accuracy_nanodeg.map(nanodegrees_to_f64),
            source: pdu.source,
        };
        self.state = State::Ready { previous: Some(stored) };
        self.handler.on_location(stored.to_public());
    }

    fn apply_delta(
        &mut self,
        prev: StoredFix,
        latitude_delta: FourByteFloat,
        longitude_delta: FourByteFloat,
        altitude_delta: i32,
        speed_delta: Option<FourByteFloat>,
        heading_delta: Option<FourByteFloat>,
    ) {
        // MS-RDPEL 2.2.2.4 and 2.2.2.5: client encodes `previous - current`, so `current = previous - delta`.
        let lat = prev.latitude_nanodeg.saturating_sub(latitude_delta.nanodegrees());
        let lon = prev.longitude_nanodeg.saturating_sub(longitude_delta.nanodegrees());
        let altitude = prev.altitude.saturating_sub(altitude_delta);
        if !valid_coordinates(lat, lon, altitude) {
            warn!("Ignoring out-of-range location delta PDU and invalidating delta base");
            self.state = State::Ready { previous: None };
            return;
        }
        let Some((speed_nanodeg, heading_nanodeg)) =
            apply_motion_deltas(prev.speed_nanodeg, prev.heading_nanodeg, speed_delta, heading_delta)
        else {
            warn!("Ignoring location delta PDU with invalid motion delta and invalidating delta base");
            self.state = State::Ready { previous: None };
            return;
        };
        let stored = StoredFix {
            latitude_nanodeg: lat,
            longitude_nanodeg: lon,
            altitude,
            speed_nanodeg,
            heading_nanodeg,
            ..prev
        };
        self.state = State::Ready { previous: Some(stored) };
        self.handler.on_location(stored.to_public());
    }
}

fn valid_coordinates(lat_nanodeg: i64, lon_nanodeg: i64, altitude: i32) -> bool {
    (-MAX_LATITUDE_NANODEGREES..=MAX_LATITUDE_NANODEGREES).contains(&lat_nanodeg)
        && (-MAX_LONGITUDE_NANODEGREES..=MAX_LONGITUDE_NANODEGREES).contains(&lon_nanodeg)
        && (FourByteSignedInteger::MIN..=FourByteSignedInteger::MAX).contains(&altitude)
}

fn apply_motion_deltas(
    prev_speed: Option<i64>,
    prev_heading: Option<i64>,
    speed_delta: Option<FourByteFloat>,
    heading_delta: Option<FourByteFloat>,
) -> Option<(Option<i64>, Option<i64>)> {
    match (speed_delta, heading_delta) {
        (None, None) => Some((prev_speed, prev_heading)),
        (Some(ds), Some(dh)) => {
            let s = prev_speed?.checked_sub(ds.nanodegrees())?;
            let h = prev_heading?.checked_sub(dh.nanodegrees())?;
            (0 <= s && (0..=MAX_HEADING_NANODEGREES).contains(&h)).then_some((Some(s), Some(h)))
        }
        _ => None,
    }
}

impl_as_any!(LocationServer);

impl DvcProcessor for LocationServer {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn max_message_size(&self) -> Option<usize> {
        Some(4096)
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.state = State::WaitingClientReady;
        debug!(channel_id, version = ?self.protocol_version, "Sending location server-ready PDU");
        Ok(vec![Box::new(LocationPdu::ServerReady(ReadyPdu {
            protocol_version: self.protocol_version,
            flags: Some(0),
        }))])
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu: LocationPdu = match decode(payload) {
            Ok(pdu) => pdu,
            Err(error) => {
                if let State::Ready { previous } = &mut self.state {
                    *previous = None;
                }
                warn!(%error, "Ignoring malformed client location PDU and invalidating delta base");
                return Ok(Vec::new());
            }
        };
        match (self.state, pdu) {
            (State::WaitingClientReady, LocationPdu::ClientReady(ready)) => {
                let negotiated = match (self.protocol_version, ready.protocol_version) {
                    (ProtocolVersion::V2, ProtocolVersion::V2) => ProtocolVersion::V2,
                    _ => ProtocolVersion::V1,
                };
                self.state = State::Ready { previous: None };
                self.handler.on_ready(negotiated);
            }
            (State::Ready { .. }, LocationPdu::BaseLocation3d(base)) => self.apply_base(base),
            (State::Ready { previous: Some(prev) }, LocationPdu::Location2dDelta(delta)) => {
                self.apply_delta(
                    prev,
                    delta.latitude_delta,
                    delta.longitude_delta,
                    0,
                    delta.speed_delta,
                    delta.heading_delta,
                );
            }
            (State::Ready { previous: Some(prev) }, LocationPdu::Location3dDelta(delta)) => {
                self.apply_delta(
                    prev,
                    delta.latitude_delta,
                    delta.longitude_delta,
                    delta.altitude_delta.value(),
                    delta.speed_delta,
                    delta.heading_delta,
                );
            }
            (_, LocationPdu::ServerReady(_)) => {
                if let State::Ready { previous } = &mut self.state {
                    *previous = None;
                }
                warn!("Ignoring server-originated location PDU received from client");
            }
            _ => {
                if let State::Ready { previous } = &mut self.state {
                    *previous = None;
                }
                warn!("Ignoring out-of-sequence client location PDU");
            }
        }
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        self.state = State::Closed;
        self.handler.on_closed();
    }
}

impl DvcServerProcessor for LocationServer {}
