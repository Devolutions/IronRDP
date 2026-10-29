//! The mstscax `IMsRdpExtendedSettings` property names, as part of the `control` module.
//!
//! mstscax accepts a fixed set of names in `put_Property` and `get_Property`. The names IronRDP can honor are
//! handled here; the others fail with `E_NOTIMPL`, so a host never believes a setting took effect when it did not.

use super::*;

/// Every name mstscax accepts in `IMsRdpExtendedSettings::put_Property`.
pub(super) const MSTSCAX_PUT_PROPERTY_NAMES: &[&str] = &[
    "AudioPlaybackDevice",
    "AudioCaptureDevice",
    "RequestSessionId",
    "EnableMediaOptimizations",
    "IgnoreServerGeneratedMouseMoves",
    "IgnoreClientSideMouseInput",
    "ConnectionBarShowQualityButton",
    "RDGIsKDCProxy",
    "KDCProxyName",
    "ShowSessionDiagnostics",
    "ConnectToChildSession",
    "ConnectToAgentSession",
    "PhysicalDesktopWidth",
    "PhysicalDesktopHeight",
    "DesktopOrientation",
    "DesktopScaleFactor",
    "DeviceScaleFactor",
    "DisableCredentialsDelegation",
    "EnableFrameBufferRedirection",
    "FrameBufferRedirectionPresenter",
    "EnableRemoteEdgeBar",
    "RdpPerfLogger",
    "DisableFullScreenHotkey",
    "EnableRemoteTouchVisuals",
    "DisableTouchRemoting",
    "DisableTouchpadAppGestureRemoting",
    "DisableTouchpadGlobalGestureRemoting",
    "MaximumTouchContacts",
    "ZoomLevel",
    "TouchDevicePresent",
    "DisableSeamlessLanguageBar",
    "ShowConnectionInformation",
    "HvSocketServiceId",
    "HvSocketEnabled",
    "WslgModeEnabled",
    "WslgSharedMemoryPath",
    "AppContainerID",
    "DisplayViewOrientation",
    "MonitorGapWidth",
    "CounterRotateTopLandscapeDisplay",
    "IgnoreCursors",
    "RdpAllowTestRecording",
    "RdpSetRecorderDLLName",
    "DisableAutoReconnectComponent",
    "IsCollabScenario",
    "EnableZoom",
    "EnableHardwareMode",
    "RequestUseNewOutputPresenter",
    "DisableH264HardwareDecode",
    "CorrelationId",
    "DiagnosticsInfo",
    "RDmiDiagnosticsUrl",
    "EndpointFedAuth",
    "EnableRdsAadAuth",
    "allowed security protocols",
    "HvsiPolicy",
    "RestrictedLogon",
    "RedirectedAuthentication",
    "UseURCP",
    "HiDefRemoteAppContainerGUID",
    "GatewayCertificateLogonAuthority",
    "MultipenRemotingSupported",
    "AllowSasSequenceTransmission",
    "EnableLocationRedirection",
    "SelectedMonitors",
    "Workspace Id",
    "PrintingProgressMode",
    "ClientDeviceName",
    "SuspendHeartbeatCheck",
    "ManualClipboardSyncEnabled",
    "EnableVailMonitorConfig",
    "UiaCrossMachineServices",
    "UiaRemoteMachineId",
    "AllowAxToContainerEvents",
    "AllowRelativeMouseMode",
    "RedirectTextProcessing",
    "RemoteAppMouseMoveInject",
    "RedirectionWarningDialogVersion",
    "RemoteApplicationAppID",
    "RemoteApplicationFile",
];

/// The names mstscax accepts only in `IMsRdpExtendedSettings::get_Property`.
pub(super) const MSTSCAX_GET_ONLY_PROPERTY_NAMES: &[&str] = &[
    "RailMode",
    "RemoteSessionId",
    "ServerSupportsEdgeActions",
    "ServerSupportsEdgeActions2",
    "TSGTransportIsUsed",
    "UDPTransportConnected",
];

/// Settings from mstscax extended property names that have no AdvancedSettings counterpart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ExtendedSettings {
    /// `PhysicalDesktopWidth` in millimeters; zero leaves the physical size unspecified.
    pub(super) physical_desktop_width: u32,
    /// `PhysicalDesktopHeight` in millimeters; zero leaves the physical size unspecified.
    pub(super) physical_desktop_height: u32,
    /// `DesktopOrientation`; `None` derives landscape or portrait from the desktop size.
    pub(super) desktop_orientation: Option<ironrdp_pdu::gcc::MonitorOrientation>,
    /// `DeviceScaleFactor`; `None` follows the desktop scale factor.
    pub(super) device_scale_factor: Option<DeviceScaleFactor>,
    /// `SelectedMonitors`: indices into the local monitor enumeration. Empty selects every monitor.
    pub(super) selected_monitors: Vec<u32>,
    /// `EnableLocationRedirection`.
    pub(super) enable_location_redirection: bool,
    /// `DisableAutoReconnectComponent`.
    pub(super) disable_auto_reconnect: bool,
    /// `DisableTouchRemoting`: touch input falls back to mouse emulation.
    pub(super) disable_touch_remoting: bool,
    /// `MaximumTouchContacts`, advertised over RDPEI.
    pub(super) max_touch_contacts: u16,
    /// `AudioPlaybackDevice`: a WASAPI endpoint ID; empty follows the system default.
    pub(super) audio_playback_device: String,
    /// `AudioCaptureDevice`: a WASAPI endpoint ID; empty follows the system default.
    pub(super) audio_capture_device: String,
}

impl Default for ExtendedSettings {
    fn default() -> Self {
        Self {
            physical_desktop_width: 0,
            physical_desktop_height: 0,
            desktop_orientation: None,
            device_scale_factor: None,
            selected_monitors: Vec::new(),
            enable_location_redirection: true,
            disable_auto_reconnect: false,
            disable_touch_remoting: false,
            max_touch_contacts: ironrdp_client::rdp::RDPEI_MAX_TOUCH_CONTACTS,
            audio_playback_device: String::new(),
            audio_capture_device: String::new(),
        }
    }
}

impl ExtendedSettings {
    /// The physical desktop size, when both dimensions are specified.
    pub(super) fn physical_desktop_size(&self) -> Option<(u32, u32)> {
        (self.physical_desktop_width != 0 && self.physical_desktop_height != 0)
            .then_some((self.physical_desktop_width, self.physical_desktop_height))
    }
}

/// Converts a WASAPI endpoint ID, as mstscax receives it, into an IronRDP audio device selector.
///
/// MMDevice endpoint IDs start with `{`; anything else is passed through, so an IronRDP device ID or name keeps
/// working.
pub(super) fn audio_device_selector(endpoint: &str) -> Option<String> {
    if endpoint.is_empty() {
        None
    } else if endpoint.starts_with('{') {
        Some(format!("wasapi:{endpoint}"))
    } else {
        Some(endpoint.to_owned())
    }
}

/// Parses a monitor selection: monitor indices separated by commas or semicolons.
pub(super) fn parse_selected_monitors(value: &str) -> Result<Vec<u32>> {
    let mut monitors = Vec::new();
    for entry in value.split([',', ';']).map(str::trim).filter(|entry| !entry.is_empty()) {
        let monitor = entry
            .parse::<u32>()
            .map_err(|_| Error::new(E_INVALIDARG, "SelectedMonitors must list monitor indices"))?;
        if !monitors.contains(&monitor) {
            monitors.push(monitor);
        }
    }
    Ok(monitors)
}

pub(super) fn format_selected_monitors(monitors: &[u32]) -> String {
    monitors.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

/// Keeps the monitors `selected` lists, in local enumeration order. An empty selection keeps every monitor.
///
/// When the primary monitor is not selected, the first selected monitor becomes the session's primary monitor.
/// Indices that name no local monitor are ignored, but at least one selected monitor must exist.
pub(super) fn select_host_monitors(host_monitors: Vec<HostMonitor>, selected: &[u32]) -> Result<Vec<HostMonitor>> {
    if selected.is_empty() {
        return Ok(host_monitors);
    }
    let mut monitors = host_monitors
        .into_iter()
        .enumerate()
        .filter_map(|(index, monitor)| {
            let index = u32::try_from(index).ok()?;
            selected.contains(&index).then_some((index, monitor))
        })
        .collect::<Vec<_>>();
    if !monitors.iter().any(|(_, monitor)| monitor.primary) {
        let primary = selected
            .iter()
            .find_map(|selected| monitors.iter().position(|(index, _)| index == selected))
            .ok_or_else(|| Error::new(E_INVALIDARG, "SelectedMonitors names no local monitor"))?;
        monitors[primary].1.primary = true;
    }
    Ok(monitors.into_iter().map(|(_, monitor)| monitor).collect())
}

/// Builds the GCC Client Monitor Extended Data matching the monitors of `topology`.
///
/// `desktop_scale_factor` and `device_scale_factor`, when set, override every monitor's own scale, as in
/// [`MonitorTopology::display_control_layout`]. The physical size is left unspecified.
pub(super) fn client_monitor_extended_data(
    topology: &MonitorTopology,
    desktop_scale_factor: Option<u32>,
    device_scale_factor: Option<DeviceScaleFactor>,
) -> ironrdp_pdu::gcc::ClientMonitorExtendedData {
    use ironrdp_pdu::gcc::{ClientMonitorExtendedData, ExtendedMonitorInfo};

    ClientMonitorExtendedData {
        extended_monitors_info: topology
            .attributes
            .iter()
            .map(|attributes| {
                let desktop_scale_factor = MonitorLayoutEntry::adjust_desktop_scale_factor(
                    desktop_scale_factor.unwrap_or(attributes.desktop_scale_factor),
                );
                ExtendedMonitorInfo {
                    physical_width: 0,
                    physical_height: 0,
                    orientation: gcc_orientation(attributes.orientation),
                    desktop_scale_factor,
                    device_scale_factor: device_scale_factor
                        .unwrap_or_else(|| DeviceScaleFactor::from_desktop_scale_factor(desktop_scale_factor))
                        .value(),
                }
            })
            .collect(),
    }
}

fn gcc_orientation(orientation: MonitorOrientation) -> ironrdp_pdu::gcc::MonitorOrientation {
    match orientation {
        MonitorOrientation::Landscape => ironrdp_pdu::gcc::MonitorOrientation::Landscape,
        MonitorOrientation::Portrait => ironrdp_pdu::gcc::MonitorOrientation::Portrait,
        MonitorOrientation::LandscapeFlipped => ironrdp_pdu::gcc::MonitorOrientation::LandscapeFlipped,
        MonitorOrientation::PortraitFlipped => ironrdp_pdu::gcc::MonitorOrientation::PortraitFlipped,
    }
}

fn gcc_orientation_from_angle(angle: u32) -> Option<ironrdp_pdu::gcc::MonitorOrientation> {
    MonitorOrientation::from_angle(angle).map(gcc_orientation)
}

/// Reads an mstscax `VT_UI4` property value. `VT_I4` is accepted too, as scripting hosts pass integers that way.
fn variant_u32(value: &VARIANT) -> Result<u32> {
    let header = variant_header(value);
    match header.vt {
        VT_UI4 => Ok(unsafe { header.Anonymous.ulVal }),
        VT_I4 => u32::try_from(unsafe { header.Anonymous.lVal })
            .map_err(|_| Error::new(E_INVALIDARG, "the property value must not be negative")),
        _ => Err(Error::from_hresult(DISP_E_TYPEMISMATCH)),
    }
}

fn variant_u32_out(value: u32) -> VARIANT {
    let mut variant = VARIANT::default();
    let header = variant_header_mut(&mut variant);
    header.vt = VT_UI4;
    header.Anonymous.ulVal = value;
    variant
}

/// Handles the mstscax extended property `name` when IronRDP supports it, or returns `None`.
pub(super) fn put_property(control: &Control, name: &str, value: *mut VARIANT) -> Option<Result<()>> {
    let name = MSTSCAX_PUT_PROPERTY_NAMES
        .iter()
        .copied()
        .find(|candidate| candidate.eq_ignore_ascii_case(name))?;
    // An unsupported name returns `None` before the value is examined, so the caller reports `E_NOTIMPL` even for a
    // null value.
    let apply = supported_property_put(name)?;
    if value.is_null() {
        return Some(Err(Error::from_hresult(E_POINTER)));
    }
    let value = unsafe { &*value };
    trace_host_call(&format!("IMsRdpExtendedSettings::put_{name}"));
    Some(apply(control, value))
}

type PropertyPut = fn(&Control, &VARIANT) -> Result<()>;

/// Returns how to apply the extended property `name`, or `None` when IronRDP does not support it.
fn supported_property_put(name: &str) -> Option<PropertyPut> {
    let apply: PropertyPut = match name {
        // Settings the session reads at any time.
        "DisableTouchRemoting" => |control, value| {
            control.compatibility.borrow_mut().extended.disable_touch_remoting = variant_bool(value, ptr::null_mut())?;
            Ok(())
        },
        "DisableAutoReconnectComponent" => |control, value| {
            control.compatibility.borrow_mut().extended.disable_auto_reconnect = variant_bool(value, ptr::null_mut())?;
            Ok(())
        },
        // IronRDP always sets INFO_LOGONNOTIFY, so the server always reports the session ID.
        "RequestSessionId" => |_, value| {
            if variant_bool(value, ptr::null_mut())? {
                Ok(())
            } else {
                Err(Error::from_hresult(E_NOTIMPL))
            }
        },
        // IronRDP synchronizes the clipboard automatically and has no connection information panel.
        "ManualClipboardSyncEnabled" | "ShowConnectionInformation" => |_, value| {
            if variant_bool(value, ptr::null_mut())? {
                Err(Error::from_hresult(E_NOTIMPL))
            } else {
                Ok(())
            }
        },
        // The remaining settings shape the connection, so they only change before it starts.
        "AudioPlaybackDevice" => |control, value| {
            before_connecting(control, |compatibility| {
                compatibility.extended.audio_playback_device =
                    validate_activex_extended_string(variant_string(value, ptr::null_mut())?)?;
                Ok(())
            })
        },
        "AudioCaptureDevice" => |control, value| {
            before_connecting(control, |compatibility| {
                compatibility.extended.audio_capture_device =
                    validate_activex_extended_string(variant_string(value, ptr::null_mut())?)?;
                Ok(())
            })
        },
        "PhysicalDesktopWidth" => |control, value| {
            before_connecting(control, |compatibility| {
                compatibility.extended.physical_desktop_width = variant_u32(value)?;
                Ok(())
            })
        },
        "PhysicalDesktopHeight" => |control, value| {
            before_connecting(control, |compatibility| {
                compatibility.extended.physical_desktop_height = variant_u32(value)?;
                Ok(())
            })
        },
        "DesktopOrientation" => |control, value| {
            before_connecting(control, |compatibility| {
                compatibility.extended.desktop_orientation = Some(
                    gcc_orientation_from_angle(variant_u32(value)?)
                        .ok_or_else(|| Error::new(E_INVALIDARG, "orientation must be 0, 90, 180 or 270 degrees"))?,
                );
                Ok(())
            })
        },
        "DesktopScaleFactor" => |control, value| {
            before_connecting(control, |compatibility| {
                let scale = variant_u32(value)?;
                if !(100..=500).contains(&scale) {
                    return Err(Error::new(
                        E_INVALIDARG,
                        "desktop scale factor must be between 100 and 500",
                    ));
                }
                compatibility.desktop_scale_factor = Some(scale);
                Ok(())
            })
        },
        "DeviceScaleFactor" => |control, value| {
            before_connecting(control, |compatibility| {
                compatibility.extended.device_scale_factor = Some(
                    DeviceScaleFactor::from_value(variant_u32(value)?)
                        .ok_or_else(|| Error::new(E_INVALIDARG, "device scale factor must be 100, 140 or 180"))?,
                );
                Ok(())
            })
        },
        "MaximumTouchContacts" => |control, value| {
            before_connecting(control, |compatibility| {
                compatibility.extended.max_touch_contacts = variant_u32(value)
                    .ok()
                    .filter(|contacts| (1..=256).contains(contacts))
                    .and_then(|contacts| u16::try_from(contacts).ok())
                    .ok_or_else(|| Error::new(E_INVALIDARG, "maximum touch contacts must be between 1 and 256"))?;
                Ok(())
            })
        },
        "EnableLocationRedirection" => |control, value| {
            before_connecting(control, |compatibility| {
                compatibility.extended.enable_location_redirection = variant_bool(value, ptr::null_mut())?;
                Ok(())
            })
        },
        "SelectedMonitors" => |control, value| {
            before_connecting(control, |compatibility| {
                compatibility.extended.selected_monitors =
                    parse_selected_monitors(&variant_string(value, ptr::null_mut())?)?;
                Ok(())
            })
        },
        _ => return None,
    };
    Some(apply)
}

/// Applies a setting that shapes the connection, which only changes before the connection starts.
fn before_connecting(control: &Control, apply: impl FnOnce(&mut CompatibilitySettings) -> Result<()>) -> Result<()> {
    let mut compatibility = control.compatibility.borrow_mut();
    active_x_connection_settings_mutable(control.state.get(), &compatibility)?;
    apply(&mut compatibility)
}

/// Reads the mstscax extended property `name` when IronRDP supports it, or returns `None`.
pub(super) fn get_property(control: &Control, name: &str, value: *mut VARIANT) -> Option<Result<()>> {
    let name = MSTSCAX_PUT_PROPERTY_NAMES
        .iter()
        .chain(MSTSCAX_GET_ONLY_PROPERTY_NAMES)
        .copied()
        .find(|candidate| candidate.eq_ignore_ascii_case(name))?;
    let compatibility = control.compatibility.borrow();
    let extended = &compatibility.extended;
    let variant = match name {
        "AudioPlaybackDevice" => variant_bstr(extended.audio_playback_device.clone()),
        "AudioCaptureDevice" => variant_bstr(extended.audio_capture_device.clone()),
        "RequestSessionId" => variant_bool_value(true),
        "PhysicalDesktopWidth" => variant_u32_out(extended.physical_desktop_width),
        "PhysicalDesktopHeight" => variant_u32_out(extended.physical_desktop_height),
        "DesktopOrientation" => variant_u32_out(u32::from(
            extended
                .desktop_orientation
                .map_or(0, |orientation| orientation.as_u16()),
        )),
        "DesktopScaleFactor" => variant_u32_out(compatibility.desktop_scale_factor.unwrap_or(100)),
        "DeviceScaleFactor" => variant_u32_out(
            extended
                .device_scale_factor
                .map_or(100, |device_scale_factor| device_scale_factor.value()),
        ),
        "DisableTouchRemoting" => variant_bool_value(extended.disable_touch_remoting),
        "MaximumTouchContacts" => variant_u32_out(u32::from(extended.max_touch_contacts)),
        "ShowConnectionInformation" | "ManualClipboardSyncEnabled" => variant_bool_value(false),
        "DisableAutoReconnectComponent" => variant_bool_value(extended.disable_auto_reconnect),
        "EnableLocationRedirection" => variant_bool_value(extended.enable_location_redirection),
        "SelectedMonitors" => variant_bstr(format_selected_monitors(&extended.selected_monitors)),
        "RailMode" => variant_bool_value(control.remote_application.borrow().enabled),
        // Like mstscax, zero until the server reports the session with a completed logon, and kept
        // after a disconnect.
        "RemoteSessionId" => variant_u32_out(control.remote_session_id.get()),
        // The IronRDP ActiveX client has no UDP transport.
        "UDPTransportConnected" => variant_bool_value(false),
        _ => return None,
    };
    drop(compatibility);
    trace_host_call(&format!("IMsRdpExtendedSettings::get_{name}"));
    Some(write_out(value, variant))
}

/// Enumerates the local monitors the session mirrors: every monitor, or the `SelectedMonitors` subset.
pub(super) fn local_monitor_topology(control: &Control) -> Result<MonitorTopology> {
    let selected = control.compatibility.borrow().extended.selected_monitors.clone();
    MonitorTopology::from_host_monitors(select_host_monitors(local_host_monitors()?, &selected)?)
}

/// Whether touch input goes to the session over RDPEI rather than to mouse emulation.
pub(super) fn touch_remoting_enabled(control: &Control) -> bool {
    !control.compatibility.borrow().extended.disable_touch_remoting
}

/// Whether the session may reconnect automatically after a network failure.
pub(super) fn auto_reconnect_enabled(control: &Control) -> bool {
    let compatibility = control.compatibility.borrow();
    compatibility.enable_auto_reconnect && !compatibility.extended.disable_auto_reconnect
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(control: &Control, name: &str, mut value: VARIANT) -> Result<()> {
        let result = put_property(control, name, &mut value).unwrap_or_else(|| panic!("{name} is handled"));
        free_owned_bstr_variant(&mut value);
        result
    }

    fn get(control: &Control, name: &str) -> VARIANT {
        let mut value = VARIANT::default();
        get_property(control, name, &mut value)
            .unwrap_or_else(|| panic!("{name} is readable"))
            .unwrap_or_else(|error| panic!("read {name}: {error}"));
        value
    }

    fn get_u32(control: &Control, name: &str) -> u32 {
        let value = get(control, name);
        let header = variant_header(&value);
        assert_eq!(header.vt, VT_UI4, "{name} is a VT_UI4");
        unsafe { header.Anonymous.ulVal }
    }

    fn get_bool(control: &Control, name: &str) -> bool {
        variant_bool(&get(control, name), ptr::null_mut()).expect("a VT_BOOL")
    }

    fn get_string(control: &Control, name: &str) -> String {
        let mut value = get(control, name);
        let string = variant_string(&value, ptr::null_mut()).expect("a VT_BSTR");
        free_owned_bstr_variant(&mut value);
        string
    }

    fn host_monitor(left: i32, width: i32, primary: bool, attributes: MonitorAttributes) -> HostMonitor {
        HostMonitor {
            rect: RECT {
                left,
                top: 0,
                right: left + width,
                bottom: 1_080,
            },
            primary,
            attributes,
        }
    }

    #[test]
    fn readme_lists_every_mstscax_extended_property() {
        let readme = include_str!("../README.md");
        for name in MSTSCAX_PUT_PROPERTY_NAMES.iter().chain(MSTSCAX_GET_ONLY_PROPERTY_NAMES) {
            assert!(
                readme.contains(&format!("| `{name}` |")),
                "the README extended property table lists `{name}`"
            );
        }
    }

    #[test]
    fn display_properties_round_trip_with_mstscax_types() {
        let control = Control::new();
        assert_eq!(get_u32(&control, "DesktopScaleFactor"), 100);
        assert_eq!(get_u32(&control, "DeviceScaleFactor"), 100);

        put(&control, "DesktopScaleFactor", variant_u32_out(150)).expect("set the desktop scale factor");
        put(&control, "DeviceScaleFactor", variant_u32_out(140)).expect("set the device scale factor");
        put(&control, "PhysicalDesktopWidth", variant_u32_out(600)).expect("set the physical width");
        put(&control, "PhysicalDesktopHeight", variant_i32(340)).expect("accept VT_I4 from scripting hosts");
        put(&control, "DesktopOrientation", variant_u32_out(270)).expect("set the orientation");

        assert_eq!(get_u32(&control, "DesktopScaleFactor"), 150);
        assert_eq!(get_u32(&control, "DeviceScaleFactor"), 140);
        assert_eq!(get_u32(&control, "PhysicalDesktopWidth"), 600);
        assert_eq!(get_u32(&control, "PhysicalDesktopHeight"), 340);
        assert_eq!(get_u32(&control, "DesktopOrientation"), 270);
        let compatibility = control.compatibility.borrow();
        assert_eq!(compatibility.desktop_scale_factor, Some(150));
        assert_eq!(compatibility.extended.physical_desktop_size(), Some((600, 340)));
        assert_eq!(
            compatibility.extended.desktop_orientation,
            Some(ironrdp_pdu::gcc::MonitorOrientation::PortraitFlipped)
        );
    }

    #[test]
    fn display_properties_reject_values_the_protocol_cannot_carry() {
        let control = Control::new();
        for (name, value) in [
            ("DesktopScaleFactor", 99),
            ("DesktopScaleFactor", 501),
            ("DeviceScaleFactor", 125),
            ("DesktopOrientation", 45),
            ("MaximumTouchContacts", 0),
            ("MaximumTouchContacts", 257),
        ] {
            let error = put(&control, name, variant_u32_out(value)).expect_err("an out-of-range value");
            assert_eq!(error.code(), E_INVALIDARG, "{name} = {value}");
        }
        let error = put(&control, "PhysicalDesktopWidth", variant_i32(-1)).expect_err("a negative value");
        assert_eq!(error.code(), E_INVALIDARG);
        let error =
            put(&control, "DesktopScaleFactor", variant_bstr("150".to_owned())).expect_err("a string is not a scale");
        assert_eq!(error.code(), DISP_E_TYPEMISMATCH);
        assert_eq!(control.compatibility.borrow().extended, ExtendedSettings::default());
        assert_eq!(control.compatibility.borrow().desktop_scale_factor, None);
    }

    #[test]
    fn connection_properties_are_fixed_once_connected() {
        let control = Control::new();
        control.state.set(ConnectionState::Connected);
        for name in ["DesktopScaleFactor", "MaximumTouchContacts", "PhysicalDesktopWidth"] {
            let error = put(&control, name, variant_u32_out(100)).expect_err("connected");
            assert_eq!(error.code(), E_UNEXPECTED, "{name}");
        }
        let error = put(&control, "EnableLocationRedirection", variant_bool_value(false)).expect_err("connected");
        assert_eq!(error.code(), E_UNEXPECTED);

        // Touch remoting and automatic reconnection are read while the session runs.
        put(&control, "DisableTouchRemoting", variant_bool_value(true)).expect("disable touch remoting");
        assert!(!touch_remoting_enabled(&control));
        assert!(get_bool(&control, "DisableTouchRemoting"));
        put(&control, "DisableAutoReconnectComponent", variant_bool_value(true)).expect("disable reconnection");
        assert!(!auto_reconnect_enabled(&control));
        assert!(get_bool(&control, "DisableAutoReconnectComponent"));
    }

    #[test]
    fn redirection_and_input_properties_round_trip() {
        let control = Control::new();
        assert!(get_bool(&control, "EnableLocationRedirection"));
        assert_eq!(get_u32(&control, "MaximumTouchContacts"), 10);

        put(&control, "EnableLocationRedirection", variant_bool_value(false)).expect("disable location");
        put(&control, "MaximumTouchContacts", variant_u32_out(20)).expect("raise the contact limit");
        put(
            &control,
            "AudioPlaybackDevice",
            variant_bstr("{0.0.0.00000000}.{1d2e3f40-0000-0000-0000-000000000000}".to_owned()),
        )
        .expect("select a playback endpoint");
        put(&control, "AudioCaptureDevice", variant_bstr("Headset".to_owned())).expect("select a capture device");

        assert!(!get_bool(&control, "EnableLocationRedirection"));
        assert_eq!(get_u32(&control, "MaximumTouchContacts"), 20);
        assert_eq!(
            get_string(&control, "AudioPlaybackDevice"),
            "{0.0.0.00000000}.{1d2e3f40-0000-0000-0000-000000000000}"
        );
        assert_eq!(get_string(&control, "AudioCaptureDevice"), "Headset");
        assert!(touch_remoting_enabled(&control));
        assert!(auto_reconnect_enabled(&control));
    }

    #[test]
    fn unimplemented_behaviors_are_refused_rather_than_ignored() {
        let control = Control::new();
        put(&control, "RequestSessionId", variant_bool_value(true)).expect("the session ID is always requested");
        let error = put(&control, "RequestSessionId", variant_bool_value(false)).expect_err("cannot opt out");
        assert_eq!(error.code(), E_NOTIMPL);
        for name in ["ManualClipboardSyncEnabled", "ShowConnectionInformation"] {
            put(&control, name, variant_bool_value(false)).expect("the default behavior");
            let error = put(&control, name, variant_bool_value(true)).expect_err("not implemented");
            assert_eq!(error.code(), E_NOTIMPL, "{name}");
            assert!(!get_bool(&control, name));
        }
        assert!(get_bool(&control, "RequestSessionId"));
        assert!(!get_bool(&control, "UDPTransportConnected"));
        assert!(!get_bool(&control, "RailMode"));
        control.remote_application.borrow_mut().enabled = true;
        assert!(get_bool(&control, "RailMode"));

        let mut value = variant_bool_value(true);
        assert!(put_property(&control, "RailMode", &mut value).is_none());
        assert!(put_property(&control, "EnableFrameBufferRedirection", &mut value).is_none());
        assert!(get_property(&control, "ServerSupportsEdgeActions", &mut value).is_none());
    }

    #[test]
    fn remote_session_id_follows_the_current_connection() {
        let control = Control::new();
        assert_eq!(get_u32(&control, "RemoteSessionId"), 0);

        control.connection_generation.set(7);
        control.events.events.lock().expect("event queue is available").extend([
            WorkerEvent::RemoteSessionId {
                generation: 6,
                session_id: 9,
            },
            WorkerEvent::RemoteSessionId {
                generation: 7,
                session_id: 3,
            },
        ]);
        control.dispatch_pending_events();
        assert_eq!(get_u32(&control, "RemoteSessionId"), 3);

        let mut value = variant_u32_out(4);
        assert!(put_property(&control, "RemoteSessionId", &mut value).is_none());
    }

    #[test]
    fn extended_settings_interface_dispatches_mstscax_names() {
        let control: IMsRdpClient10 = Control::new().into();
        let extended = control
            .cast::<IMsRdpExtendedSettings>()
            .expect("control supports IMsRdpExtendedSettings");
        let mut value = variant_bstr("1;0".to_owned());
        unsafe { extended.put_Property(BSTR::from("selectedmonitors").as_ptr(), &mut value) }
            .expect("names are case-insensitive");
        free_owned_bstr_variant(&mut value);
        let mut value = VARIANT::default();
        unsafe { extended.get_Property(BSTR::from("SelectedMonitors").as_ptr(), &mut value) }
            .expect("read the monitor selection");
        assert_eq!(variant_string(&value, ptr::null_mut()).expect("a VT_BSTR"), "1,0");
        free_owned_bstr_variant(&mut value);

        let mut value = variant_bool_value(true);
        let error = unsafe { extended.put_Property(BSTR::from("EnableRemoteEdgeBar").as_ptr(), &mut value) }
            .expect_err("unsupported names are refused");
        assert_eq!(error.code(), E_NOTIMPL);
    }

    #[test]
    fn selected_monitors_parse_like_mstsc() {
        assert_eq!(parse_selected_monitors("").expect("empty"), Vec::<u32>::new());
        assert_eq!(
            parse_selected_monitors(" 2; 0,1 ,0").expect("mixed separators"),
            vec![2, 0, 1]
        );
        assert_eq!(
            parse_selected_monitors("1,a").expect_err("not an index").code(),
            E_INVALIDARG
        );
        assert_eq!(format_selected_monitors(&[2, 0, 1]), "2,0,1");
    }

    #[test]
    fn selected_monitors_keep_the_subset_and_promote_a_primary() {
        let monitors = || {
            vec![
                host_monitor(0, 1_920, true, MonitorAttributes::default()),
                host_monitor(1_920, 1_280, false, MonitorAttributes::default()),
                host_monitor(3_200, 1_600, false, MonitorAttributes::default()),
            ]
        };

        assert_eq!(
            select_host_monitors(monitors(), &[]).expect("every monitor"),
            monitors()
        );

        // The primary monitor is not selected: the first listed monitor that exists takes its place.
        let selected = select_host_monitors(monitors(), &[7, 2, 1]).expect("two monitors");
        assert_eq!(selected.len(), 2);
        assert!(!selected[0].primary);
        assert!(selected[1].primary);
        let topology = MonitorTopology::from_host_monitors(selected).expect("a valid topology");
        assert_eq!((topology.desktop_width, topology.desktop_height), (2_880, 1_080));
        assert!(topology.monitors[1].flags.contains(MonitorFlags::PRIMARY));
        assert_eq!((topology.monitors[1].left, topology.monitors[1].top), (0, 0));
        assert_eq!(topology.monitors[0].left, -1_280);

        let error = select_host_monitors(monitors(), &[5]).expect_err("no such monitor");
        assert_eq!(error.code(), E_INVALIDARG);
    }

    #[test]
    fn client_monitor_extended_data_follows_each_monitor() {
        let topology = MonitorTopology::from_host_monitors(vec![
            host_monitor(0, 1_920, true, MonitorAttributes::default()),
            host_monitor(
                1_920,
                1_080,
                false,
                MonitorAttributes {
                    desktop_scale_factor: 150,
                    orientation: MonitorOrientation::Portrait,
                },
            ),
        ])
        .expect("a valid topology");

        let data = client_monitor_extended_data(&topology, None, None);
        let monitors = &data.extended_monitors_info;
        assert_eq!(monitors.len(), 2);
        assert_eq!(monitors[0].orientation, ironrdp_pdu::gcc::MonitorOrientation::Landscape);
        assert_eq!(
            (monitors[0].desktop_scale_factor, monitors[0].device_scale_factor),
            (100, 100)
        );
        assert_eq!(monitors[1].orientation, ironrdp_pdu::gcc::MonitorOrientation::Portrait);
        assert_eq!(
            (monitors[1].desktop_scale_factor, monitors[1].device_scale_factor),
            (150, 140)
        );
        assert_eq!((monitors[1].physical_width, monitors[1].physical_height), (0, 0));

        let data = client_monitor_extended_data(&topology, Some(200), Some(DeviceScaleFactor::Scale100Percent));
        assert!(
            data.extended_monitors_info
                .iter()
                .all(|monitor| (monitor.desktop_scale_factor, monitor.device_scale_factor) == (200, 100))
        );
    }

    #[test]
    fn audio_endpoints_select_wasapi_devices() {
        assert_eq!(audio_device_selector(""), None);
        assert_eq!(
            audio_device_selector("{0.0.1.00000000}.{guid}").as_deref(),
            Some("wasapi:{0.0.1.00000000}.{guid}")
        );
        assert_eq!(audio_device_selector("Headset").as_deref(), Some("Headset"));
    }
}
