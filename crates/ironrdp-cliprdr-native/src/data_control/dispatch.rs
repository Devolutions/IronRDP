//! Wayland event dispatch for the data-control objects.
//!
//! Both protocols (`ext-data-control-v1` and `wlr-data-control-unstable-v1`)
//! have the same object model and the same events, so each handler is written
//! twice and forwards to the shared [`State`].

use wayland_client::{
    Connection, Dispatch, Proxy as _, QueueHandle,
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

// === ext-data-control-v1 ===

impl Dispatch<ExtDataControlManagerV1, ()> for Client {
    fn event(
        _state: &mut Self,
        _proxy: &ExtDataControlManagerV1,
        _event: <ExtDataControlManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // The manager has no events.
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for Client {
    fn event(
        state: &mut Self,
        _proxy: &ExtDataControlDeviceV1,
        event: <ExtDataControlDeviceV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::DataOffer { id } => {
                state.data_control.on_data_offer_ext(id);
            }
            ext_data_control_device_v1::Event::Selection { id } => {
                if id.is_some() {
                    state.data_control.on_selection();
                } else {
                    state.data_control.on_selection_cleared();
                }
            }
            ext_data_control_device_v1::Event::Finished => {
                state.data_control.on_device_finished();
            }
            ext_data_control_device_v1::Event::PrimarySelection { .. } => {
                // Only the regular clipboard is handled, not the primary selection.
                tracing::trace!("ext data control primary selection event (ignored)");
            }
            _ => {}
        }
    }

    // The `data_offer` event creates a child offer object; without this,
    // wayland-client's default panics.
    wayland_client::event_created_child!(Client, ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ExtDataControlSourceV1, ()> for Client {
    fn event(
        state: &mut Self,
        proxy: &ExtDataControlSourceV1,
        event: <ExtDataControlSourceV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if !state.data_control.is_current_source(&proxy.id()) {
            return;
        }
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                state.data_control.on_source_send(&mime_type, fd);
            }
            ext_data_control_source_v1::Event::Cancelled => {
                state.data_control.on_source_cancelled();
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtDataControlOfferV1, ()> for Client {
    fn event(
        state: &mut Self,
        _proxy: &ExtDataControlOfferV1,
        event: <ExtDataControlOfferV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.data_control.on_offer_mime_type(mime_type);
        }
    }
}

// === wlr-data-control-unstable-v1 ===

impl Dispatch<ZwlrDataControlManagerV1, ()> for Client {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrDataControlManagerV1,
        _event: <ZwlrDataControlManagerV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // The manager has no events.
    }
}

impl Dispatch<ZwlrDataControlDeviceV1, ()> for Client {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrDataControlDeviceV1,
        event: <ZwlrDataControlDeviceV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                state.data_control.on_data_offer_wlr(id);
            }
            zwlr_data_control_device_v1::Event::Selection { id } => {
                if id.is_some() {
                    state.data_control.on_selection();
                } else {
                    state.data_control.on_selection_cleared();
                }
            }
            zwlr_data_control_device_v1::Event::Finished => {
                state.data_control.on_device_finished();
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { .. } => {
                tracing::trace!("wlr data control primary selection event (ignored)");
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(Client, ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ZwlrDataControlSourceV1, ()> for Client {
    fn event(
        state: &mut Self,
        proxy: &ZwlrDataControlSourceV1,
        event: <ZwlrDataControlSourceV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if !state.data_control.is_current_source(&proxy.id()) {
            return;
        }
        match event {
            zwlr_data_control_source_v1::Event::Send { mime_type, fd } => {
                state.data_control.on_source_send(&mime_type, fd);
            }
            zwlr_data_control_source_v1::Event::Cancelled => {
                state.data_control.on_source_cancelled();
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrDataControlOfferV1, ()> for Client {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrDataControlOfferV1,
        event: <ZwlrDataControlOfferV1 as wayland_client::Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.data_control.on_offer_mime_type(mime_type);
        }
    }
}
