//! Wayland event dispatch for the data-control objects.
//!
//! Both protocols (`ext-data-control-v1` and `wlr-data-control-unstable-v1`)
//! have the same object model and the same events, so their handlers are
//! generated from one macro and forward to the shared [`State`].

use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::GlobalListContents,
    protocol::{wl_registry::WlRegistry, wl_seat::WlSeat},
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1::{self, ExtDataControlDeviceV1},
    ext_data_control_manager_v1::ExtDataControlManagerV1,
    ext_data_control_offer_v1::{self, ExtDataControlOfferV1},
    ext_data_control_source_v1::{self, ExtDataControlSourceV1},
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
    zwlr_data_control_source_v1::{self, ZwlrDataControlSourceV1},
};

use super::state::State;

/// The dispatch target the Wayland event queue calls into.
pub(crate) struct Client {
    pub(crate) data_control: State,
}

impl Dispatch<WlRegistry, GlobalListContents> for Client {
    fn event(
        _state: &mut Self,
        _proxy: &WlRegistry,
        _event: <WlRegistry as wayland_client::Proxy>::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Globals are looked up once at start-up; later additions are ignored.
    }
}

impl Dispatch<WlSeat, ()> for Client {
    fn event(
        _state: &mut Self,
        _proxy: &WlSeat,
        _event: <WlSeat as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // The seat is only an argument to `get_data_device`.
    }
}

/// Implement the handlers of one data-control protocol.
///
/// `protocol` names it in log messages, each object is given with the module that holds its
/// events, and `on_data_offer` is the `State` method that takes the protocol's new offer.
macro_rules! dispatch_data_control {
    (
        protocol: $protocol:literal,
        manager: $manager:ty,
        device: $device:ty => $device_events:ident,
        source: $source:ty => $source_events:ident,
        offer: $offer:ty => $offer_events:ident,
        on_data_offer: $on_data_offer:ident $(,)?
    ) => {
        impl Dispatch<$manager, ()> for Client {
            fn event(
                _state: &mut Self,
                _proxy: &$manager,
                _event: <$manager as wayland_client::Proxy>::Event,
                _data: &(),
                _conn: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
                // The manager has no events.
            }
        }

        impl Dispatch<$device, ()> for Client {
            fn event(
                state: &mut Self,
                _proxy: &$device,
                event: <$device as wayland_client::Proxy>::Event,
                _data: &(),
                _conn: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
                match event {
                    $device_events::Event::DataOffer { id } => {
                        state.data_control.$on_data_offer(id);
                    }
                    $device_events::Event::Selection { id } => {
                        if id.is_some() {
                            state.data_control.on_selection();
                        } else {
                            state.data_control.on_selection_cleared();
                        }
                    }
                    $device_events::Event::Finished => {
                        state.data_control.on_device_finished();
                    }
                    $device_events::Event::PrimarySelection { .. } => {
                        // Only the regular clipboard is handled, not the primary selection.
                        tracing::trace!(protocol = $protocol, "Primary selection event ignored");
                    }
                    _ => {}
                }
            }

            // The `data_offer` event creates a child offer object; without this,
            // wayland-client's default panics.
            wayland_client::event_created_child!(Client, $device, [
                $device_events::EVT_DATA_OFFER_OPCODE => ($offer, ()),
            ]);
        }

        impl Dispatch<$source, ()> for Client {
            fn event(
                state: &mut Self,
                _proxy: &$source,
                event: <$source as wayland_client::Proxy>::Event,
                _data: &(),
                _conn: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
                match event {
                    $source_events::Event::Send { mime_type, fd } => {
                        state.data_control.on_source_send(&mime_type, fd);
                    }
                    $source_events::Event::Cancelled => {
                        state.data_control.on_source_cancelled();
                    }
                    _ => {}
                }
            }
        }

        impl Dispatch<$offer, ()> for Client {
            fn event(
                state: &mut Self,
                _proxy: &$offer,
                event: <$offer as wayland_client::Proxy>::Event,
                _data: &(),
                _conn: &Connection,
                _qh: &QueueHandle<Self>,
            ) {
                if let $offer_events::Event::Offer { mime_type } = event {
                    state.data_control.on_offer_mime_type(mime_type);
                }
            }
        }
    };
}

dispatch_data_control! {
    protocol: "ext",
    manager: ExtDataControlManagerV1,
    device: ExtDataControlDeviceV1 => ext_data_control_device_v1,
    source: ExtDataControlSourceV1 => ext_data_control_source_v1,
    offer: ExtDataControlOfferV1 => ext_data_control_offer_v1,
    on_data_offer: on_data_offer_ext,
}

dispatch_data_control! {
    protocol: "wlr",
    manager: ZwlrDataControlManagerV1,
    device: ZwlrDataControlDeviceV1 => zwlr_data_control_device_v1,
    source: ZwlrDataControlSourceV1 => zwlr_data_control_source_v1,
    offer: ZwlrDataControlOfferV1 => zwlr_data_control_offer_v1,
    on_data_offer: on_data_offer_wlr,
}
