//! `.rdp` keys of the modern settings map beyond the connection basics, as part of the `control` module.
//!
//! `validate_modern_rdp_property` and `Control::apply_modern_rdp_property` delegate the keys they do not handle
//! themselves to [`validate_extended_rdp_property`] and [`apply_extended_rdp_property`], so `ApplySettings`, `SetRdpProperty` and `.rdp` files share one mapping.
//! Names arrive lowercase.

use super::{
    ConnectionType, Control, DeviceScaleFactor, E_INVALIDARG, E_NOTIMPL, Error, MAX_RECONNECT_ATTEMPTS, PropertySet,
    Result, Value, extended_properties, invalidate_renderer, property_bool, property_i32, property_string,
};

/// Keys reported as `VT_BOOL` by `GetRdpProperty`.
pub(super) fn is_boolean_extended_rdp_property(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "use multimon"
            | "smart sizing"
            | "redirectsmartcards"
            | "redirectlocation"
            | "span monitors"
            | "autoreconnection enabled"
            | "networkautodetect"
            | "bandwidthautodetect"
            | "promptcredentialonce"
    )
}

pub(super) fn validate_extended_rdp_property(name: &str, value: &Value) -> Result<()> {
    match name {
        "use multimon" | "smart sizing" | "redirectsmartcards" | "redirectlocation" | "autoreconnection enabled" => {
            let _ = property_bool(value)?;
            Ok(())
        }
        // IronRDP always takes part in network characteristics detection.
        "networkautodetect" | "bandwidthautodetect" => {
            if property_bool(value)? {
                Ok(())
            } else {
                Err(Error::from_hresult(E_NOTIMPL))
            }
        }
        // IronRDP neither spans one monitor across several local monitors nor shares the RD Gateway
        // credentials with the remote computer on its own.
        "span monitors" | "promptcredentialonce" => {
            if property_bool(value)? {
                Err(Error::from_hresult(E_NOTIMPL))
            } else {
                Ok(())
            }
        }
        "selectedmonitors" => {
            extended_properties::parse_selected_monitors(&property_string(value)?)?;
            Ok(())
        }
        "keyboardhook" => match property_i32(value)? {
            0..=2 => Ok(()),
            _ => Err(Error::from_hresult(E_INVALIDARG)),
        },
        "connection type" => match property_i32(value)? {
            1..=7 => Ok(()),
            _ => Err(Error::from_hresult(E_INVALIDARG)),
        },
        "authentication level" => match property_i32(value)? {
            0..=3 => Ok(()),
            _ => Err(Error::from_hresult(E_INVALIDARG)),
        },
        "autoreconnect max retries" => match u32::try_from(property_i32(value)?) {
            Ok(attempts) if attempts <= MAX_RECONNECT_ATTEMPTS => Ok(()),
            _ => Err(Error::from_hresult(E_INVALIDARG)),
        },
        "devicescalefactor" => match property_i32(value)? {
            100 | 140 | 180 => Ok(()),
            _ => Err(Error::from_hresult(E_INVALIDARG)),
        },
        _ => Err(Error::from_hresult(E_NOTIMPL)),
    }
}

fn connection_type_from_rdp(value: i32) -> ConnectionType {
    u8::try_from(value)
        .ok()
        .and_then(ConnectionType::from_specified)
        .unwrap_or(ConnectionType::Autodetect)
}

fn connection_type_to_rdp(connection_type: ConnectionType) -> Option<i32> {
    connection_type.specified_value().map(i32::from)
}

/// Applies a key [`validate_extended_rdp_property`] accepted.
pub(super) fn apply_extended_rdp_property(control: &Control, name: &str, value: &Value) {
    let int = || value.as_int().unwrap_or_default();
    let flag = || int() != 0;
    let mut compatibility = control.compatibility.borrow_mut();
    match name {
        "use multimon" => compatibility.use_multimon = flag(),
        "selectedmonitors" => {
            compatibility.extended.selected_monitors =
                extended_properties::parse_selected_monitors(value.as_str().unwrap_or_default()).unwrap_or_default();
        }
        "smart sizing" => {
            compatibility.smart_sizing = flag();
            let renderer_window = compatibility.renderer_window;
            drop(compatibility);
            invalidate_renderer(renderer_window);
        }
        "redirectsmartcards" => compatibility.redirect_smart_cards = flag(),
        "redirectlocation" => compatibility.extended.enable_location_redirection = flag(),
        "keyboardhook" => compatibility.keyboard_hook_mode = i32::try_from(int()).unwrap_or(2),
        "autoreconnection enabled" => compatibility.enable_auto_reconnect = flag(),
        "autoreconnect max retries" => {
            compatibility.max_reconnect_attempts = u32::try_from(int()).unwrap_or(MAX_RECONNECT_ATTEMPTS);
        }
        "connection type" => {
            compatibility.network_connection_type = connection_type_from_rdp(i32::try_from(int()).unwrap_or(7));
        }
        "authentication level" => {
            compatibility.authentication_level = u32::try_from(int()).unwrap_or(2);
            compatibility.authentication_level_set = true;
        }
        "devicescalefactor" => {
            compatibility.extended.device_scale_factor =
                u32::try_from(int()).ok().and_then(DeviceScaleFactor::from_value);
        }
        _ => {}
    }
}

/// Adds the current value of every key this module maps, so `GetRdpProperty` and saved `.rdp` files reflect
/// settings made through the ActiveX properties too.
pub(super) fn extended_rdp_property_snapshot(control: &Control, properties: &mut PropertySet) {
    let compatibility = control.compatibility.borrow();
    properties.insert("use multimon", compatibility.use_multimon);
    properties.insert(
        "selectedmonitors",
        extended_properties::format_selected_monitors(&compatibility.extended.selected_monitors),
    );
    properties.insert("smart sizing", compatibility.smart_sizing);
    properties.insert("redirectsmartcards", compatibility.redirect_smart_cards);
    properties.insert("redirectlocation", compatibility.extended.enable_location_redirection);
    properties.insert("span monitors", false);
    properties.insert("keyboardhook", compatibility.keyboard_hook_mode);
    properties.insert("autoreconnection enabled", compatibility.enable_auto_reconnect);
    properties.insert("autoreconnect max retries", compatibility.max_reconnect_attempts);
    properties.insert("networkautodetect", true);
    properties.insert("bandwidthautodetect", true);
    properties.insert("promptcredentialonce", false);
    if let Some(connection_type) = connection_type_to_rdp(compatibility.network_connection_type) {
        properties.insert("connection type", connection_type);
    }
    if compatibility.authentication_level_set {
        properties.insert("authentication level", compatibility.authentication_level);
    }
    if let Some(device_scale_factor) = compatibility.extended.device_scale_factor {
        properties.insert("devicescalefactor", device_scale_factor.value());
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ConnectionState, DISP_E_TYPEMISMATCH, E_UNEXPECTED, is_boolean_modern_property};
    use super::*;

    fn set(control: &Control, name: &str, value: impl Into<Value>) -> Result<()> {
        control.set_modern_rdp_property(name.to_owned(), value.into())
    }

    fn get(control: &Control, name: &str) -> Value {
        control
            .modern_rdp_property(name)
            .unwrap_or_else(|error| panic!("read {name}: {error}"))
    }

    #[test]
    fn rdp_keys_map_onto_control_settings() {
        let control = Control::new();
        set(&control, "Use Multimon", 1).expect("use multimon");
        set(&control, "SelectedMonitors", "2,0").expect("selectedmonitors");
        set(&control, "Smart Sizing", 1).expect("smart sizing");
        set(&control, "RedirectSmartCards", 1).expect("redirectsmartcards");
        set(&control, "RedirectLocation", 0).expect("redirectlocation");
        set(&control, "KeyboardHook", 0).expect("keyboardhook");
        set(&control, "AutoReconnection Enabled", 0).expect("autoreconnection enabled");
        set(&control, "AutoReconnect Max Retries", 5).expect("autoreconnect max retries");
        set(&control, "connection type", 4).expect("connection type");
        set(&control, "Authentication Level", 2).expect("authentication level");
        set(&control, "DeviceScaleFactor", 180).expect("devicescalefactor");

        let compatibility = control.compatibility.borrow();
        assert!(compatibility.use_multimon);
        assert_eq!(compatibility.extended.selected_monitors, [2, 0]);
        assert!(compatibility.smart_sizing);
        assert!(compatibility.redirect_smart_cards);
        assert!(!compatibility.extended.enable_location_redirection);
        assert_eq!(compatibility.keyboard_hook_mode, 0);
        assert!(!compatibility.enable_auto_reconnect);
        assert_eq!(compatibility.max_reconnect_attempts, 5);
        assert_eq!(compatibility.network_connection_type, ConnectionType::BroadbandHigh);
        assert_eq!(
            (
                compatibility.authentication_level,
                compatibility.authentication_level_set
            ),
            (2, true)
        );
        assert_eq!(
            compatibility.extended.device_scale_factor,
            Some(DeviceScaleFactor::Scale180Percent)
        );
        drop(compatibility);

        assert_eq!(get(&control, "selectedmonitors").as_str(), Some("2,0"));
        assert_eq!(get(&control, "connection type").as_int(), Some(4));
        assert_eq!(get(&control, "devicescalefactor").as_int(), Some(180));
        assert_eq!(get(&control, "use multimon").as_int(), Some(1));
        assert!(is_boolean_modern_property("Use Multimon"));
        assert!(!is_boolean_modern_property("keyboardhook"));
    }

    #[test]
    fn rdp_keys_reflect_settings_made_through_activex_properties() {
        let control = Control::new();
        {
            let mut compatibility = control.compatibility.borrow_mut();
            compatibility.use_multimon = true;
            compatibility.redirect_smart_cards = true;
            compatibility.keyboard_hook_mode = 1;
            compatibility.network_connection_type = ConnectionType::Wan;
        }
        assert_eq!(get(&control, "use multimon").as_int(), Some(1));
        assert_eq!(get(&control, "redirectsmartcards").as_int(), Some(1));
        assert_eq!(get(&control, "keyboardhook").as_int(), Some(1));
        assert_eq!(get(&control, "connection type").as_int(), Some(5));
        assert_eq!(get(&control, "networkautodetect").as_int(), Some(1));
        assert_eq!(get(&control, "span monitors").as_int(), Some(0));
    }

    #[test]
    fn rdp_keys_refuse_values_ironrdp_cannot_honor() {
        let control = Control::new();
        for (name, value) in [
            ("networkautodetect", 0),
            ("bandwidthautodetect", 0),
            ("span monitors", 1),
            ("promptcredentialonce", 1),
        ] {
            let error = set(&control, name, value).expect_err("unsupported behavior");
            assert_eq!(error.code(), E_NOTIMPL, "{name} = {value}");
        }
        for (name, value) in [
            ("networkautodetect", 1),
            ("bandwidthautodetect", 1),
            ("span monitors", 0),
            ("promptcredentialonce", 0),
        ] {
            set(&control, name, value).unwrap_or_else(|error| panic!("{name} = {value}: {error}"));
        }

        for (name, value) in [
            ("keyboardhook", 3),
            ("connection type", 0),
            ("connection type", 8),
            ("authentication level", 4),
            ("autoreconnect max retries", -1),
            ("autoreconnect max retries", 201),
            ("devicescalefactor", 125),
        ] {
            let error = set(&control, name, value).expect_err("out of range");
            assert_eq!(error.code(), E_INVALIDARG, "{name} = {value}");
        }
        let error = set(&control, "selectedmonitors", "primary").expect_err("not an index");
        assert_eq!(error.code(), E_INVALIDARG);
        let error = set(&control, "use multimon", "yes").expect_err("not a number");
        assert_eq!(error.code(), DISP_E_TYPEMISMATCH);

        control.state.set(ConnectionState::Connected);
        let error = set(&control, "use multimon", 1).expect_err("connected");
        assert_eq!(error.code(), E_UNEXPECTED);
        assert!(!control.compatibility.borrow().use_multimon);
    }
}
