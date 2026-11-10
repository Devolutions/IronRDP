//! Client processor for the MS-RDPEGT geometry tracking channel.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use ironrdp_core::{decode, impl_as_any};
use ironrdp_dvc::{DvcClientProcessor, DvcMessage, DvcProcessor};
use ironrdp_pdu::PduResult;
use tracing::{debug, trace, warn};

use crate::CHANNEL_NAME;
use crate::pdu::{MappedGeometry, MappedGeometryPacket};

/// Largest number of geometry mappings tracked at once.
pub const MAX_MAPPINGS: usize = 1024;

/// A change to the set of geometry mappings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryEvent<'a> {
    /// A mapping was created or its geometry changed.
    Updated {
        mapping_id: u64,
        geometry: &'a MappedGeometry,
    },
    /// A mapping was removed, by the server or because the channel closed.
    Cleared { mapping_id: u64 },
}

/// Callback notified of every mapping change.
pub type GeometryObserver = Box<dyn FnMut(GeometryEvent<'_>) + Send>;

/// Client processor for `Microsoft::Windows::RDS::Geometry::v08.01`.
pub struct GeometryClient {
    mappings: BTreeMap<u64, MappedGeometry>,
    observer: Option<GeometryObserver>,
}

impl GeometryClient {
    pub fn new() -> Self {
        Self {
            mappings: BTreeMap::new(),
            observer: None,
        }
    }

    /// Notifies `observer` of every mapping change.
    #[must_use]
    pub fn with_observer(mut self, observer: GeometryObserver) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Geometry of the mapping with this identifier.
    pub fn mapping(&self, mapping_id: u64) -> Option<&MappedGeometry> {
        self.mappings.get(&mapping_id)
    }

    /// Active mappings, by identifier.
    pub fn mappings(&self) -> impl Iterator<Item = (u64, &MappedGeometry)> {
        self.mappings
            .iter()
            .map(|(mapping_id, geometry)| (*mapping_id, geometry))
    }

    fn apply(&mut self, packet: MappedGeometryPacket) {
        let mapping_id = packet.mapping_id;
        match packet.geometry {
            Some(geometry) => {
                if !self.mappings.contains_key(&mapping_id) && self.mappings.len() == MAX_MAPPINGS {
                    warn!(mapping_id, "Ignoring geometry mapping beyond the supported count");
                    return;
                }
                trace!(mapping_id, ?geometry, "Geometry mapping updated");
                self.mappings.insert(mapping_id, geometry);
                if let (Some(observer), Some(geometry)) = (self.observer.as_mut(), self.mappings.get(&mapping_id)) {
                    observer(GeometryEvent::Updated { mapping_id, geometry });
                }
            }
            None => {
                // Clearing an unknown mapping is ignored ([3.1.3]).
                if self.mappings.remove(&mapping_id).is_some() {
                    trace!(mapping_id, "Geometry mapping cleared");
                    if let Some(observer) = self.observer.as_mut() {
                        observer(GeometryEvent::Cleared { mapping_id });
                    }
                }
            }
        }
    }

    fn clear_all(&mut self) {
        let mappings = core::mem::take(&mut self.mappings);
        if let Some(observer) = self.observer.as_mut() {
            for mapping_id in mappings.into_keys() {
                observer(GeometryEvent::Cleared { mapping_id });
            }
        }
    }
}

impl Default for GeometryClient {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for GeometryClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GeometryClient")
            .field("mappings", &self.mappings)
            .finish_non_exhaustive()
    }
}

impl_as_any!(GeometryClient);

impl DvcProcessor for GeometryClient {
    fn channel_name(&self) -> &str {
        CHANNEL_NAME
    }

    fn start(&mut self, channel_id: u32) -> PduResult<Vec<DvcMessage>> {
        self.clear_all();
        debug!(channel_id, "Geometry tracking channel started");
        Ok(Vec::new())
    }

    fn process(&mut self, _channel_id: u32, payload: &[u8]) -> PduResult<Vec<DvcMessage>> {
        match decode::<MappedGeometryPacket>(payload) {
            Ok(packet) => self.apply(packet),
            Err(error) => warn!(%error, "Ignoring malformed geometry packet"),
        }
        Ok(Vec::new())
    }

    fn close(&mut self, _channel_id: u32) {
        self.clear_all();
        debug!("Geometry tracking channel closed");
    }
}

impl DvcClientProcessor for GeometryClient {}
