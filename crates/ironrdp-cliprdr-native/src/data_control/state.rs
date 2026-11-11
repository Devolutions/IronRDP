//! The data-control protocol state machine.
//!
//! [`State`] owns the protocol objects (manager, device, current offer, our
//! source) and turns compositor events into changes of the shared selection
//! state. It is deliberately free of any event loop: the worker in
//! `super::worker` feeds it events and commands.
//!
//! # Flows
//!
//! **Set selection (client to compositor):**
//! 1. A [`Command::SetSelection`] arrives with the MIME types and any data.
//! 2. A data-control source is created, the types are advertised on it and it
//!    is made the device's selection; the previous source is destroyed only
//!    afterwards, so the clipboard is never empty in between.
//! 3. A `send` event for an advertised type writes the cached data to the
//!    requested file descriptor on a worker thread. With no cached data and an
//!    `on_transfer` callback set, the paste is held until
//!    [`Command::CompleteTransfer`] answers it (delayed rendering). At most
//!    `MAX_PASTES` pastes are in flight, and a paste beyond that is closed with
//!    no data. A held paste is closed with no data after `PASTE_TIMEOUT`, and
//!    a writer gives up when its reader takes nothing for that long.
//!
//! **Selection changed (compositor to client):**
//! 1. `data_offer`, then one `offer` per MIME type, then `selection`.
//! 2. The types are stored in [`Shared`] and the change callback is called,
//!    unless the event is the echo of a selection this client set itself.
//!
//! **Read (client reads the compositor's selection):**
//! [`Command::ReceiveFromOffer`] calls `offer.receive(mime, fd)`; the caller
//! reads the other end of the pipe.

use core::{
    hash::BuildHasher as _,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use std::{
    collections::HashMap,
    fs::File,
    hash::RandomState,
    io::{self, Write as _},
    os::unix::io::{AsFd as _, OwnedFd},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Instant,
};

use nix::{
    errno::Errno,
    libc::PIPE_BUF,
    poll::{PollFd, PollFlags, PollTimeout, poll},
};
use wayland_client::{Dispatch, QueueHandle, protocol::wl_seat::WlSeat};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::ExtDataControlDeviceV1, ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::ExtDataControlOfferV1, ext_data_control_source_v1::ExtDataControlSourceV1,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, zwlr_data_control_source_v1::ZwlrDataControlSourceV1,
};

use super::mime::find_charset_variant;

// The enums below wrap the ext and wlr variants, so State can work with either protocol transparently.

/// Data control manager (either ext or wlr).
pub(crate) enum DataControlManager {
    /// ext-data-control-v1 manager.
    Ext(ExtDataControlManagerV1),
    /// wlr-data-control-unstable-v1 manager.
    Wlr(ZwlrDataControlManagerV1),
}

/// Data control device (either ext or wlr).
pub(crate) enum DataControlDevice {
    /// ext-data-control-v1 device.
    Ext(ExtDataControlDeviceV1),
    /// wlr-data-control-unstable-v1 device.
    Wlr(ZwlrDataControlDeviceV1),
}

/// Data control offer (either ext or wlr).
pub(crate) enum DataControlOffer {
    /// ext-data-control-v1 offer.
    Ext(ExtDataControlOfferV1),
    /// wlr-data-control-unstable-v1 offer.
    Wlr(ZwlrDataControlOfferV1),
}

impl DataControlOffer {
    /// Request data transfer for a MIME type.
    ///
    /// Tells the source client to write data to the provided fd.
    pub(crate) fn receive(&self, mime_type: &str, fd: &OwnedFd) {
        match self {
            DataControlOffer::Ext(offer) => offer.receive(mime_type.to_owned(), fd.as_fd()),
            DataControlOffer::Wlr(offer) => offer.receive(mime_type.to_owned(), fd.as_fd()),
        }
    }

    /// Destroy this offer.
    pub(crate) fn destroy(&self) {
        match self {
            DataControlOffer::Ext(offer) => offer.destroy(),
            DataControlOffer::Wlr(offer) => offer.destroy(),
        }
    }
}

/// Data control source (either ext or wlr).
enum DataControlSource {
    /// ext-data-control-v1 source.
    Ext(ExtDataControlSourceV1),
    /// wlr-data-control-unstable-v1 source.
    Wlr(ZwlrDataControlSourceV1),
}

impl DataControlSource {
    /// Advertise a MIME type on this source.
    fn offer(&self, mime_type: &str) {
        match self {
            DataControlSource::Ext(source) => source.offer(mime_type.to_owned()),
            DataControlSource::Wlr(source) => source.offer(mime_type.to_owned()),
        }
    }

    /// Destroy this source.
    fn destroy(&self) {
        match self {
            DataControlSource::Ext(source) => source.destroy(),
            DataControlSource::Wlr(source) => source.destroy(),
        }
    }
}

/// Commands sent from clipboard backends to the Wayland event loop thread.
#[derive(Debug)]
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) enum Command {
    /// Signal `sender` once the compositor has processed every earlier request.
    ///
    /// The selection events those requests caused are dispatched before the
    /// signal, so [`Shared`] reflects them when it arrives.
    Synchronize(std::sync::mpsc::Sender<()>),
    /// Set the clipboard selection on the compositor.
    ///
    /// Creates a data control source with the offered MIME types and
    /// stores the data for responding to `send` events.
    SetSelection {
        /// MIME types to advertise.
        mime_types: Vec<String>,
        /// Data for each MIME type (written to fd on `send` event).
        data: HashMap<String, Vec<u8>>,
    },
    /// Update source data for a MIME type without re-creating the source.
    ///
    /// Used when data wasn't available at `SetSelection` time (eager fetch
    /// from a remote clipboard). The Wayland data source stays unchanged;
    /// only the cached data map is updated so the next `send` event can
    /// serve the requested MIME type.
    UpdateSourceData {
        /// MIME type key for the data.
        mime_type: String,
        /// Data bytes to cache.
        data: Vec<u8>,
    },
    /// Receive clipboard data from the current compositor offer.
    ///
    /// Calls `offer.receive(mime_type, fd)` on the event loop thread.
    /// The caller reads from the other end of the pipe.
    ReceiveFromOffer {
        /// MIME type to request.
        mime_type: String,
        /// Write end of the pipe (compositor writes data here).
        fd: OwnedFd,
    },
    /// Answer a transfer raised through `Shared::on_transfer`.
    ///
    /// Writes `data` to every paste waiting on that transfer and caches it
    /// for later pastes of the same MIME type. `None` closes the waiting
    /// pastes with no data.
    CompleteTransfer {
        /// Serial passed to the `on_transfer` callback.
        serial: u32,
        /// The data, or `None` if the remote could not supply it.
        data: Option<Vec<u8>>,
    },
    /// Give up our selection, if we still own it.
    ClearSelection,
}

/// Shared clipboard state readable from any thread.
///
/// Updated by the event loop thread when the compositor's selection changes.
/// Read by clipboard backends to report current state.
///
/// The fields are `pub` so that the integration tests can read and set them. The struct is only
/// public under the `__test` feature, so otherwise they reach no further than the crate.
#[derive(Default)]
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) struct Shared {
    /// MIME types of the current compositor selection.
    pub mime_types: Vec<String>,
    /// Serial number, incremented on each selection change.
    pub serial: u32,
    /// Change notification callback.
    ///
    /// Called on the event loop thread when selection changes.
    pub on_change: Option<Arc<dyn Fn(Vec<String>) + Send + Sync>>,
    /// Paste callback for data not supplied up front.
    ///
    /// When set, a paste of an advertised MIME type with no cached data is
    /// held open and this is called with a serial and the MIME type; the
    /// answer comes back as `Command::CompleteTransfer`. Without
    /// it such a paste gets no data.
    pub on_transfer: Option<Arc<dyn Fn(u32, String) + Send + Sync>>,
    /// Whether our own source is still the compositor's selection source.
    ///
    /// It is set when a selection is set. It is cleared when another client
    /// takes the selection, when the selection is cleared, and when the source
    /// is cancelled or given up. [`DataControl::owns_selection`] reads it.
    ///
    /// [`DataControl::owns_selection`]: super::DataControl::owns_selection
    pub own_source_live: bool,
    /// Whether the worker thread has stopped, for any reason.
    ///
    /// The worker sets it as it is dropped, before its connection goes and with it any request
    /// still queued. A read that finds its pipe closed with no data uses it to tell a dead worker
    /// from a source with nothing to send.
    pub stopped: bool,
}

/// Lock the shared state, recovering from a poisoned lock.
///
/// `Shared` holds plain fields that are each valid on their own, so after a panic while the lock
/// was held the state is still right to read and update. Skipping the update instead would leave
/// the worker and the handle disagreeing about the clipboard.
pub(crate) fn lock_shared(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The most pastes that can be waiting for data or being written at once.
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) const MAX_PASTES: usize = 16;

/// How long a paste waits for data from `on_transfer`, and how long a reader
/// may take nothing before the writer answering it gives up.
const PASTE_TIMEOUT: Duration = Duration::from_secs(5);

/// One of the [`MAX_PASTES`] places, given back when the paste holding it ends.
struct PastePlace(Arc<AtomicUsize>);

impl PastePlace {
    /// Take a free place, or `None` when all of them are in use.
    fn take(in_flight: &Arc<AtomicUsize>) -> Option<Self> {
        in_flight
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
                (count < MAX_PASTES).then_some(count + 1)
            })
            .ok()
            .map(|_| Self(Arc::clone(in_flight)))
    }
}

impl Drop for PastePlace {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A paste in flight: the write end of the pipe its reader waits on, and the
/// place it holds until it ends.
struct Paste {
    fd: OwnedFd,
    place: PastePlace,
}

/// A paste waiting on data from `on_transfer`, with every paste of the same
/// MIME type that arrived meanwhile.
struct PendingTransfer {
    serial: u32,
    /// The waiting pastes, each with the time it stops waiting.
    waiters: Vec<(Paste, Instant)>,
}

/// Central data control state.
///
/// Manages the lifecycle of data control protocol objects and routes
/// events to the shared clipboard state.
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) struct State {
    /// The data control manager global.
    pub(crate) manager: Option<DataControlManager>,
    /// The data control device (per-seat).
    pub(crate) device: Option<DataControlDevice>,
    /// The current selection offer from the compositor.
    current_offer: Option<DataControlOffer>,
    /// The data source we created for `SetSelection` (if any).
    current_source: Option<DataControlSource>,
    /// MIME types advertised on `current_source`.
    pub(crate) current_source_mime_types: Vec<String>,
    /// The private type that marks this client's own selections.
    own_marker: OwnMarker,
    /// Data cached for our source's `send` events.
    pub(crate) source_data: HashMap<String, Arc<[u8]>>,
    /// Pending offer being built up, with the MIME types collected so far.
    ///
    /// Between the `data_offer` and `selection` events the compositor sends `offer` events with
    /// MIME types, and they accumulate here.
    pending_offer: Option<(DataControlOffer, Vec<String>)>,
    /// Pastes waiting on `on_transfer`, by requested MIME type.
    pending_transfers: HashMap<String, PendingTransfer>,
    /// How many pastes hold a place, counting those waiting and those being written.
    pastes_in_flight: Arc<AtomicUsize>,
    /// Next serial handed to `on_transfer`.
    next_transfer_serial: u32,
    /// Shared clipboard state for cross-thread access.
    #[expect(clippy::struct_field_names)]
    pub(crate) shared_state: Arc<Mutex<Shared>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            manager: None,
            device: None,
            current_offer: None,
            current_source: None,
            current_source_mime_types: Vec::new(),
            own_marker: OwnMarker::random(),
            source_data: HashMap::new(),
            pending_offer: None,
            pending_transfers: HashMap::new(),
            pastes_in_flight: Arc::default(),
            next_transfer_serial: 1,
            shared_state: Arc::new(Mutex::new(Shared::default())),
        }
    }
}

impl State {
    /// Test-only: cache data for a MIME type as if it had been set.
    #[cfg(feature = "__test")]
    pub fn insert_source_data(&mut self, mime_type: &str, data: Vec<u8>) {
        self.source_data.insert(mime_type.to_owned(), data.into());
    }

    /// Test-only: whether any source data is cached.
    #[cfg(feature = "__test")]
    pub fn has_source_data(&self) -> bool {
        !self.source_data.is_empty()
    }

    /// Test-only: pretend our source advertised these MIME types.
    #[cfg(feature = "__test")]
    pub fn set_advertised(&mut self, mime_types: Vec<String>) {
        self.current_source_mime_types = mime_types;
    }

    /// Test-only: the type that marks this client's own selections.
    #[cfg(feature = "__test")]
    pub fn own_marker(&self) -> &str {
        self.own_marker.as_str()
    }

    /// Test-only: the shared state the callbacks and readers see.
    #[cfg(feature = "__test")]
    pub fn shared(&self) -> &Arc<Mutex<Shared>> {
        &self.shared_state
    }

    /// Create a data control device from the manager and seat.
    ///
    /// Must be called after both the manager and seat are bound.
    pub(crate) fn create_device<D>(&mut self, seat: &WlSeat, qh: &QueueHandle<D>)
    where
        D: Dispatch<ExtDataControlDeviceV1, ()> + Dispatch<ZwlrDataControlDeviceV1, ()> + 'static,
    {
        let device = match &self.manager {
            Some(DataControlManager::Ext(mgr)) => DataControlDevice::Ext(mgr.get_data_device(seat, qh, ())),
            Some(DataControlManager::Wlr(mgr)) => DataControlDevice::Wlr(mgr.get_data_device(seat, qh, ())),
            None => {
                tracing::error!("Cannot create data control device: manager not bound");
                return;
            }
        };

        tracing::debug!("Created data control device");
        self.device = Some(device);
    }

    /// Handle a `data_offer` event from the device.
    ///
    /// A new offer is being introduced. Store it and start collecting
    /// MIME types from subsequent `offer` events.
    pub(crate) fn on_data_offer_ext(&mut self, offer: ExtDataControlOfferV1) {
        self.set_pending_offer(DataControlOffer::Ext(offer));
    }

    /// Handle a `data_offer` event from the device (wlr variant).
    pub(crate) fn on_data_offer_wlr(&mut self, offer: ZwlrDataControlOfferV1) {
        self.set_pending_offer(DataControlOffer::Wlr(offer));
    }

    fn set_own_source_live(&self, live: bool) {
        lock_shared(&self.shared_state).own_source_live = live;
    }

    fn set_pending_offer(&mut self, offer: DataControlOffer) {
        // An offer that never became the selection still has to be destroyed explicitly.
        if let Some((old_offer, _)) = self.pending_offer.take() {
            old_offer.destroy();
        }
        self.pending_offer = Some((offer, Vec::new()));
    }

    /// Handle an `offer` event on a data offer (MIME type offered).
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn on_offer_mime_type(&mut self, mime_type: String) {
        if let Some((_, ref mut mime_types)) = self.pending_offer {
            mime_types.push(mime_type);
        }
    }

    /// Handle the `selection` event from the device.
    ///
    /// The compositor's selection has changed. The pending offer
    /// (with accumulated MIME types) becomes the current offer.
    pub(crate) fn on_selection(&mut self) {
        // The offer of the selection being replaced is destroyed explicitly as well.
        if let Some(old) = self.current_offer.take() {
            old.destroy();
        }

        let mut mime_types = if let Some((offer, types)) = self.pending_offer.take() {
            self.current_offer = Some(offer);
            types
        } else {
            // Without a pending offer there's nothing to promote, so the selection has no types.
            Vec::new()
        };

        // The device reports every selection, our own included, and reporting
        // our own as a change makes the consumer treat it as another client's
        // copy. Our source advertises a private marker type next to the real
        // ones, so a selection that carries it is ours and one that doesn't is
        // another client's, in whichever order the compositor delivers them and
        // whatever types they offer.
        let own = self.own_marker.take_from(&mut mime_types);
        tracing::debug!(
            mime_types = ?mime_types,
            own,
            "Compositor selection changed"
        );

        // The callback runs after the lock is released: it may call back into
        // the handle (`serial`, `selection_mime_types`), which locks the same state.
        let callback = {
            let mut shared = lock_shared(&self.shared_state);
            shared.serial = shared.serial.wrapping_add(1);
            shared.mime_types.clone_from(&mime_types);
            // Another client's copy may arrive before the cancel for our source. Our marker doesn't prove
            // that a source of ours is live either, because a clipboard manager can re-offer our content
            // after the source is gone.
            shared.own_source_live = own && self.current_source.is_some();
            if own { None } else { shared.on_change.clone() }
        };
        if let Some(callback) = callback {
            callback(mime_types);
        }
    }

    /// Handle the `selection` event with a NULL offer (selection cleared).
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn on_selection_cleared(&mut self) {
        self.destroy_offers();

        tracing::debug!("Compositor selection cleared");

        let callback = {
            let mut shared = lock_shared(&self.shared_state);
            shared.serial = shared.serial.wrapping_add(1);
            shared.mime_types.clear();
            shared.own_source_live = false;
            shared.on_change.clone()
        };
        if let Some(callback) = callback {
            callback(Vec::new());
        }
    }

    /// Update cached source data for a MIME type.
    ///
    /// Inserts or replaces data in the `source_data` map without
    /// re-creating the Wayland data source. Used when data arrives
    /// after `set_selection` was called with an empty data map
    /// (eager fetch from a remote clipboard).
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn update_source_data(&mut self, mime_type: String, data: Vec<u8>) {
        tracing::debug!(
            mime_type = %mime_type,
            bytes = data.len(),
            "Source data updated (post-announcement)"
        );
        self.source_data.insert(mime_type, data.into());
    }

    /// Handle a `send` event on our data source.
    ///
    /// The compositor (or another client pasting) wants data in the
    /// specified MIME type. Cached data is written at once; otherwise, with
    /// an `on_transfer` callback set, the paste is held until the data
    /// arrives through `CompleteTransfer`, or until `PASTE_TIMEOUT` is up.
    /// With `MAX_PASTES` pastes already in flight the new one is closed with
    /// no data.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn on_source_send(&mut self, mime_type: &str, fd: OwnedFd) {
        if OwnMarker::is_marker(mime_type) {
            // A client that reads every type it is offered asks for the marker too. It has no data to
            // give, and that isn't worth a warning.
            return;
        }

        let Some(place) = PastePlace::take(&self.pastes_in_flight) else {
            tracing::warn!(mime_type, "Too many pastes in flight, closing this one");
            return;
        };
        let paste = Paste { fd, place };

        if let Some(data) = self.cached_data(mime_type) {
            write_in_background(paste, data);
            return;
        }

        let deadline = Instant::now() + PASTE_TIMEOUT;
        if let Some(pending) = self.pending_transfers.get_mut(mime_type) {
            pending.waiters.push((paste, deadline));
            return;
        }

        let on_transfer = lock_shared(&self.shared_state).on_transfer.clone();
        let advertised = self.current_source_mime_types.iter().any(|m| m == mime_type);
        let (Some(on_transfer), true) = (on_transfer, advertised) else {
            tracing::warn!(
                mime_type,
                "Paste closed with no data, the type is not advertised or no transfer callback is set"
            );
            // Returning drops the fd, which closes the pipe, so the pasting client sees no data.
            return;
        };

        let serial = self.next_transfer_serial;
        self.next_transfer_serial = self.next_transfer_serial.wrapping_add(1);
        self.pending_transfers.insert(
            mime_type.to_owned(),
            PendingTransfer {
                serial,
                waiters: vec![(paste, deadline)],
            },
        );
        tracing::debug!(mime_type, serial, "Paste held until its data arrives");
        on_transfer(serial, mime_type.to_owned());
    }

    /// Data cached for `mime_type`, or for the same type with another spelling of its
    /// charset (compositors commonly request `text/plain;charset=utf-8` for `text/plain`, and
    /// the reverse).
    fn cached_data(&self, mime_type: &str) -> Option<Arc<[u8]>> {
        let key = find_charset_variant(mime_type, self.source_data.keys().map(String::as_str))?;
        self.source_data.get(key).cloned()
    }

    /// Process a `CompleteTransfer` command.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn complete_transfer(&mut self, serial: u32, data: Option<Vec<u8>>) {
        let Some((mime_type, pending)) = self
            .pending_transfers
            .iter()
            .find(|(_, pending)| pending.serial == serial)
            .map(|(mime, _)| mime.clone())
            .and_then(|mime| self.pending_transfers.remove_entry(&mime))
        else {
            tracing::debug!(serial, "Transfer answered after its selection was replaced");
            return;
        };
        let Some(data) = data.filter(|d| !d.is_empty()) else {
            tracing::debug!(mime_type, waiters = pending.waiters.len(), "Transfer returned no data");
            return;
        };
        let data: Arc<[u8]> = data.into();
        for (paste, _) in pending.waiters {
            write_in_background(paste, Arc::clone(&data));
        }
        self.source_data.insert(mime_type, data);
    }

    /// Close the held pastes whose time is up, and return when the next one is
    /// due.
    ///
    /// A closed paste ends with no data. Its transfer stays pending, so a
    /// paste that comes meanwhile joins it instead of raising a second
    /// request, and an answer that arrives late is still kept for the next
    /// paste.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn expire_transfers(&mut self, now: Instant) -> Option<Instant> {
        for pending in self.pending_transfers.values_mut() {
            pending.waiters.retain(|(_, deadline)| now < *deadline);
        }
        self.pending_transfers
            .values()
            .flat_map(|pending| pending.waiters.iter().map(|(_, deadline)| *deadline))
            .min()
    }

    /// Process a `ClearSelection` command: give up our selection if the
    /// compositor still has it.
    pub(crate) fn clear_selection(&mut self) {
        if self.current_source.is_none() {
            return;
        }
        match &self.device {
            Some(DataControlDevice::Ext(dev)) => dev.set_selection(None),
            Some(DataControlDevice::Wlr(dev)) => dev.set_selection(None),
            None => {}
        }
        self.discard_source();
        tracing::debug!("Released our clipboard selection");
    }

    /// Destroy our data source, if there is one, and forget everything that was kept for it.
    ///
    /// Pastes still waiting for data see end-of-file.
    fn discard_source(&mut self) {
        if let Some(source) = self.current_source.take() {
            source.destroy();
        }
        self.current_source_mime_types.clear();
        self.source_data.clear();
        self.pending_transfers.clear();
        self.set_own_source_live(false);
    }

    /// Destroy the current selection offer and the offer being built up, if there are any.
    fn destroy_offers(&mut self) {
        if let Some(offer) = self.current_offer.take() {
            offer.destroy();
        }
        if let Some((offer, _)) = self.pending_offer.take() {
            offer.destroy();
        }
    }

    /// Handle the `cancelled` event on our data source.
    ///
    /// Our source has been replaced by another. Clean up.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn on_source_cancelled(&mut self) {
        tracing::debug!("Data control source cancelled");
        self.discard_source();
    }

    /// Handle the `finished` event on the device.
    ///
    /// The data control device is no longer valid.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn on_device_finished(&mut self) {
        tracing::debug!("Data control device finished");
        self.device = None;
        self.destroy_offers();
        self.discard_source();
    }

    /// Process a `SetSelection` command.
    ///
    /// Creates a new data control source, advertises MIME types, and
    /// sets it as the selection on the device.
    pub(crate) fn set_selection<D>(
        &mut self,
        mime_types: &[String],
        data: HashMap<String, Vec<u8>>,
        qh: &QueueHandle<D>,
    ) where
        D: Dispatch<ExtDataControlSourceV1, ()> + Dispatch<ZwlrDataControlSourceV1, ()> + 'static,
    {
        let new_source = match &self.manager {
            Some(DataControlManager::Ext(mgr)) => DataControlSource::Ext(mgr.create_data_source(qh, ())),
            Some(DataControlManager::Wlr(mgr)) => DataControlSource::Wlr(mgr.create_data_source(qh, ())),
            None => {
                tracing::error!("Cannot set selection: manager not bound");
                return;
            }
        };

        for mime_type in mime_types {
            new_source.offer(mime_type);
        }
        // The marker tells the echo of this selection from another client's copy.
        new_source.offer(self.own_marker.as_str());

        match (&self.device, &new_source) {
            (Some(DataControlDevice::Ext(dev)), DataControlSource::Ext(src)) => {
                dev.set_selection(Some(src));
            }
            (Some(DataControlDevice::Wlr(dev)), DataControlSource::Wlr(src)) => {
                dev.set_selection(Some(src));
            }
            _ => {
                tracing::error!("Cannot set selection: device/source protocol mismatch or no device");
                new_source.destroy();
                return;
            }
        }

        tracing::debug!(
            mime_types = ?mime_types,
            "Set clipboard selection on compositor"
        );

        // Replace, then destroy: destroying the current source first would
        // leave the clipboard empty for a moment, which clipboard managers
        // (Klipper's "prevent empty clipboard") answer by re-offering their
        // last history item.
        //
        // No event for the old source can reach the handlers afterwards. The
        // worker drained its queue before running this command, and the backend
        // swallows what arrives for a destroyed object.
        if let Some(old) = self.current_source.take() {
            old.destroy();
        }
        self.pending_transfers.clear();
        self.source_data = data
            .into_iter()
            .map(|(mime_type, bytes)| (mime_type, bytes.into()))
            .collect();
        self.current_source = Some(new_source);
        self.current_source_mime_types = mime_types.to_vec();
        self.set_own_source_live(true);
    }

    /// Process a `ReceiveFromOffer` command.
    ///
    /// Calls `offer.receive(mime_type, fd)` to request data from the
    /// compositor. The caller reads from the other end of the pipe.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "OwnedFd must be owned so it is dropped after the receive call"
    )]
    pub(crate) fn receive_from_offer(&self, mime_type: &str, fd: OwnedFd) {
        match &self.current_offer {
            Some(offer) => {
                offer.receive(mime_type, &fd);
                tracing::debug!(mime_type, "Requested clipboard data from compositor offer");
            }
            None => {
                tracing::warn!(mime_type, "ReceiveFromOffer but no current offer");
                // The selection changed after the caller looked, so there's nothing to receive.
                // Dropping the fd gives the caller EOF, and the changed serial tells it why.
            }
        }
    }
}

/// Type names that start like this belong to some client of this library.
const MARKER_PREFIX: &str = "application/x-ironrdp-source-";

/// The private type that marks the selections of one client.
///
/// A source advertises it next to the real types, so a selection that carries it is content this
/// client put on the clipboard, in whichever order the compositor delivers it relative to other
/// clients' copies. A copy by another client doesn't carry it, whatever types the copy offers. Only a
/// client that re-offers every type of our selection, the marker included, passes it on.
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) struct OwnMarker {
    /// INVARIANT: starts with `MARKER_PREFIX`, which `take_from` relies on to remove our own marker.
    name: String,
}

impl OwnMarker {
    /// A marker that no other client of this library uses, in this process or in another one.
    ///
    /// The suffix is random and carries no process id, so other clients learn nothing from it, and
    /// content that a previous run left on the clipboard doesn't read as our own.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn random() -> Self {
        // The standard library seeds `RandomState` from the operating system, which makes it a source of
        // random numbers without a dependency.
        let suffix = RandomState::new().hash_one(0u8);
        Self {
            name: format!("{MARKER_PREFIX}{suffix:016x}"),
        }
    }

    /// The type name to advertise.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn as_str(&self) -> &str {
        &self.name
    }

    /// Whether `mime_type` is the private marker of some client, ours or another's.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn is_marker(mime_type: &str) -> bool {
        mime_type.starts_with(MARKER_PREFIX)
    }

    /// Take every private marker out of `mime_types`, and say whether ours was among them.
    #[cfg_attr(feature = "__test", visibility::make(pub))]
    pub(crate) fn take_from(&self, mime_types: &mut Vec<String>) -> bool {
        let own = mime_types.contains(&self.name);
        mime_types.retain(|mime_type| !Self::is_marker(mime_type));
        own
    }
}

/// Answer a paste on a worker thread: a large image written into a slow
/// reader would otherwise block every other Wayland event.
fn write_in_background(paste: Paste, data: Arc<[u8]>) {
    let spawned = std::thread::Builder::new()
        .name("data-control-send".into())
        .spawn(move || {
            let Paste { fd, place } = paste;
            let mut pipe = File::from(fd);
            if let Err(error) = write_with_idle_timeout(&mut pipe, &data, PASTE_TIMEOUT) {
                tracing::debug!(%error, "Paste writer stopped before the end of the data");
            }
            // The place goes back before the pipe closes, so a reader that has
            // seen the whole paste can paste again at once.
            drop(place);
            drop(pipe);
        });
    if let Err(error) = spawned {
        tracing::error!(%error, "Failed to start a paste writer");
    }
}

/// Write `data` to a pipe, giving up when the reader takes nothing for `idle`.
///
/// A plain `write_all` waits for as long as the reader does not read. Waiting
/// for the pipe to become writable instead lets the wait end. A pipe is
/// writable once `PIPE_BUF` bytes fit, so writing at most that much at a time
/// cannot block. A slow reader that keeps taking data is never cut off.
#[cfg_attr(feature = "__test", visibility::make(pub))]
pub(crate) fn write_with_idle_timeout(pipe: &mut File, data: &[u8], idle: Duration) -> io::Result<()> {
    let timeout = PollTimeout::try_from(idle).unwrap_or(PollTimeout::MAX);
    let mut rest = data;
    while !rest.is_empty() {
        let mut fds = [PollFd::new(pipe.as_fd(), PollFlags::POLLOUT)];
        match poll(&mut fds, timeout) {
            Ok(0) => return Err(io::ErrorKind::TimedOut.into()),
            Ok(_) => {}
            Err(Errno::EINTR) => continue,
            Err(errno) => return Err(errno.into()),
        }

        let (chunk, _) = rest.split_at(rest.len().min(PIPE_BUF));
        let written = match pipe.write(chunk) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(written) => written,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        rest = rest.split_at(written).1;
    }
    Ok(())
}
