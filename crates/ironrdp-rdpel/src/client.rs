//! Client state machine for the MS-RDPEL dynamic virtual channel.

use alloc::{boxed::Box, vec, vec::Vec};

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor, encode_dvc_messages};
use ironrdp_pdu::{PduResult, decode_err, encode_err, pdu_other_err};
use ironrdp_svc::{ChannelFlags, SvcMessage};
use tracing::{debug, warn};

use crate::CHANNEL_NAME;
use crate::pdu::{
    BaseLocation3dPdu, FourByteFloat, FourByteSignedInteger, Location2dDeltaPdu, Location3dDeltaPdu, LocationPdu,
    ReadyPdu,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Location {
    latitude: i32,
    longitude: i32,
    altitude: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    WaitingServerReady,
    Ready,
}

/// Client processor for `Microsoft::Windows::RDS::Location`.
#[derive(Debug)]
pub struct LocationClient {
    state: State,
    channel_id: Option<u32>,
    previous: Option<Location>,
}

impl LocationClient {
    pub fn new() -> Self {
        Self {
            state: State::WaitingServerReady,
            channel_id: None,
            previous: None,
        }
    }

    /// Returns whether the server-ready/client-ready exchange has completed.
    pub fn ready(&self) -> bool {
        self.state == State::Ready
    }

    /// Encodes one explicit caller-supplied location update for the active channel.
    ///
    /// The first update is absolute.
    /// Later updates use the 2D delta PDU while altitude is unchanged and the 3D delta PDU otherwise.
    /// The previous-location store advances only after encoding succeeds.
    pub fn send_location(&mut self, latitude: f64, longitude: f64, altitude: i32) -> PduResult<Vec<SvcMessage>> {
        if !self.ready() {
            return Err(pdu_other_err!(
                "LocationClient::send_location",
                "location channel is not ready"
            ));
        }
        let channel_id = self.channel_id.ok_or_else(|| {
            pdu_other_err!(
                "LocationClient::send_location",
                "location channel has no assigned dynamic channel id"
            )
        })?;
        if !(-90.0..=90.0).contains(&latitude)
            || !(-180.0..=180.0).contains(&longitude)
            || !(-0x0FFF_FFFF..=0x0FFF_FFFF).contains(&altitude)
        {
            return Err(pdu_other_err!(
                "LocationClient::send_location",
                "location coordinates are out of range"
            ));
        }
        let latitude = FourByteFloat::coordinate(latitude).map_err(|error| encode_err!(error))?;
        let longitude = FourByteFloat::coordinate(longitude).map_err(|error| encode_err!(error))?;
        let current = Location {
            latitude: latitude.coordinate_units(),
            longitude: longitude.coordinate_units(),
            altitude,
        };
        let pdu = match self.previous {
            None => LocationPdu::BaseLocation3d(BaseLocation3dPdu {
                latitude,
                longitude,
                altitude: FourByteSignedInteger::new(altitude).map_err(|error| encode_err!(error))?,
                speed: None,
                heading: None,
                horizontal_accuracy: None,
                source: None,
            }),
            Some(previous) if previous.altitude == altitude => LocationPdu::Location2dDelta(Location2dDeltaPdu {
                latitude_delta: FourByteFloat::from_coordinate_units(previous.latitude - current.latitude),
                longitude_delta: FourByteFloat::from_coordinate_units(previous.longitude - current.longitude),
                speed_delta: None,
                heading_delta: None,
            }),
            Some(previous) => LocationPdu::Location3dDelta(Location3dDeltaPdu {
                latitude_delta: FourByteFloat::from_coordinate_units(previous.latitude - current.latitude),
                longitude_delta: FourByteFloat::from_coordinate_units(previous.longitude - current.longitude),
                altitude_delta: FourByteSignedInteger::new(previous.altitude - altitude)
                    .map_err(|error| encode_err!(error))?,
                speed_delta: None,
                heading_delta: None,
            }),
        };
        let messages = encode_dvc_messages(channel_id, vec![Box::new(pdu)], ChannelFlags::empty())
            .map_err(|error| encode_err!(error))?;
        self.previous = Some(current);
        Ok(messages)
    }
}

impl Default for LocationClient {
    fn default() -> Self {
        Self::new()
    }
}

impl_as_any!(LocationClient);

impl DvcProcessor for LocationClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.channel_id = Some(channel_id);
        self.state = State::WaitingServerReady;
        self.previous = None;
        debug!(channel_id, "Location channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        let pdu: LocationPdu = decode(payload).map_err(|error| decode_err!(error))?;
        match pdu {
            LocationPdu::ServerReady(server_ready) if self.state == State::WaitingServerReady => {
                debug!(version = ?server_ready.protocol_version, "Location server ready");
                self.state = State::Ready;
                Ok(vec![Box::new(LocationPdu::ClientReady(ReadyPdu::v1()))])
            }
            LocationPdu::ServerReady(_) => {
                warn!("Ignoring out-of-sequence location server-ready PDU");
                Ok(Vec::new())
            }
            _ => {
                warn!("Ignoring client-originated location PDU received from server");
                Ok(Vec::new())
            }
        }
    }

    fn close(&mut self, _channel_id: u32) {
        self.channel_id = None;
        self.state = State::WaitingServerReady;
        self.previous = None;
        debug!("Location channel closed");
    }
}

impl DvcClientProcessor for LocationClient {}
