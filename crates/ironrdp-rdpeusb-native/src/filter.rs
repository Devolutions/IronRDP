//! Selection of the local USB devices to redirect.
//!
//! Two textual forms are accepted:
//!
//! - The FreeRDP `/usb` option syntax, through [`UsbRedirectionConfig::parse_cli`]:
//!   `id:VID:PID[#VID:PID...]`, `addr:BUS:ADDR[#BUS:ADDR...]`, `auto`, and `filter:CLASS[#CLASS...]`
//!   or `filter:none`, separated by commas. Every number is hexadecimal, as in FreeRDP.
//! - The `.rdp` file `usbdevicestoredirect` property, through [`UsbRedirectionConfig::parse_rdp`]:
//!   semicolon-separated `*`, `USB\VID_xxxx&PID_yyyy[...]` device instance IDs, and `-` prefixed
//!   exclusions. Device setup class GUIDs (`{...}`) cannot be resolved from USB descriptors and are
//!   skipped.

use core::fmt;

/// USB base class codes kept local by default when redirecting automatically.
///
/// These devices are better served by the high-level RDP redirections (input, audio, drives and
/// smart cards), and grabbing them would usually take the local keyboard or mouse away. This matches
/// the FreeRDP default for `/usb:auto`.
pub const DEFAULT_LOCAL_CLASSES: &[u8] = &[
    0x01, /* Audio */
    0x03, /* HID */
    0x08, /* Mass storage */
    0x09, /* Hub */
    0x0b, /* Smart card */
];

const CLASS_HUB: u8 = 0x09;

/// Facts about a local USB device that redirection rules are matched against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDeviceIdentity {
    pub vendor_id: u16,
    pub product_id: u16,
    pub bus_number: u8,
    pub address: u8,
    /// `bDeviceClass` from the device descriptor.
    pub device_class: u8,
    /// `bInterfaceClass` of every interface of the active configuration.
    pub interface_classes: Vec<u8>,
}

impl UsbDeviceIdentity {
    fn has_class(&self, class: u8) -> bool {
        self.device_class == class || self.interface_classes.contains(&class)
    }
}

/// A rule selecting local USB devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbDeviceMatch {
    /// Every device that is not in a class kept local.
    Any,
    /// Devices with this vendor ID, and this product ID when present.
    VidPid { vendor_id: u16, product_id: Option<u16> },
    /// The device at this bus number and device address.
    BusAddress { bus_number: u8, address: u8 },
}

impl UsbDeviceMatch {
    fn matches(self, device: &UsbDeviceIdentity) -> bool {
        match self {
            Self::Any => true,
            Self::VidPid { vendor_id, product_id } => {
                device.vendor_id == vendor_id && product_id.is_none_or(|product_id| device.product_id == product_id)
            }
            Self::BusAddress { bus_number, address } => device.bus_number == bus_number && device.address == address,
        }
    }
}

/// Which local USB devices to redirect, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbRedirectionConfig {
    /// Devices to redirect when the channel opens, and again when they are plugged back in.
    pub devices: Vec<UsbDeviceMatch>,
    /// Devices never to redirect, even when another rule selects them.
    pub exclude: Vec<UsbDeviceMatch>,
    /// Redirect devices plugged in during the session.
    pub auto: bool,
    /// Base classes that [`UsbDeviceMatch::Any`] and [`Self::auto`] keep local.
    ///
    /// Explicit VID/PID and address rules are not affected.
    pub local_classes: Vec<u8>,
}

impl Default for UsbRedirectionConfig {
    fn default() -> Self {
        Self {
            devices: Vec::new(),
            exclude: Vec::new(),
            auto: false,
            local_classes: DEFAULT_LOCAL_CLASSES.to_vec(),
        }
    }
}

impl UsbRedirectionConfig {
    /// Whether this configuration can ever redirect a device.
    pub fn is_enabled(&self) -> bool {
        self.auto || !self.devices.is_empty()
    }

    /// Whether hotplug notifications are needed to apply this configuration.
    pub fn needs_hotplug(&self) -> bool {
        self.auto || self.devices.iter().any(|rule| matches!(rule, UsbDeviceMatch::VidPid { .. }))
    }

    /// Whether a device present when the channel opens is redirected.
    pub fn redirect_at_connect(&self, device: &UsbDeviceIdentity) -> bool {
        self.selects(device, false)
    }

    /// Whether a device plugged in during the session is redirected.
    pub fn redirect_on_hotplug(&self, device: &UsbDeviceIdentity) -> bool {
        self.selects(device, true)
    }

    fn selects(&self, device: &UsbDeviceIdentity, hotplug: bool) -> bool {
        // Redirecting a hub would take every device behind it away, and RDPEUSB cannot carry hubs.
        if device.has_class(CLASS_HUB) || self.exclude.iter().any(|rule| rule.matches(device)) {
            return false;
        }

        let kept_local = self.local_classes.iter().any(|class| device.has_class(*class));
        let explicitly_selected = self.devices.iter().any(|rule| match rule {
            UsbDeviceMatch::Any => false,
            // Bus addresses are reassigned on every plug, so only VID/PID rules follow a device.
            UsbDeviceMatch::BusAddress { .. } => !hotplug && rule.matches(device),
            UsbDeviceMatch::VidPid { .. } => rule.matches(device),
        });
        let selected_by_any = self.devices.contains(&UsbDeviceMatch::Any) || (hotplug && self.auto);

        explicitly_selected || (selected_by_any && !kept_local)
    }

    /// Merge another configuration into this one.
    pub fn merge(&mut self, other: Self) {
        self.devices.extend(other.devices);
        self.exclude.extend(other.exclude);
        self.auto |= other.auto;
        if other.local_classes != DEFAULT_LOCAL_CLASSES {
            self.local_classes = other.local_classes;
        }
    }

    /// Parse a FreeRDP-style `/usb` option value, such as `id:046d:c52b#1050:0407,auto`.
    ///
    /// `dbg` is accepted and ignored.
    pub fn parse_cli(spec: &str) -> Result<Self, UsbFilterParseError> {
        let mut config = Self::default();

        for item in spec.split(',').map(str::trim).filter(|item| !item.is_empty()) {
            let (key, value) = item.split_once(':').unwrap_or((item, ""));
            match key.to_ascii_lowercase().as_str() {
                "auto" if value.is_empty() => config.auto = true,
                "dbg" if value.is_empty() => {}
                "id" | "addr" | "dev" if !value.is_empty() => {
                    let by_address = key.eq_ignore_ascii_case("addr");
                    for pair in value.split('#') {
                        let (first, second) = pair.split_once(':').ok_or_else(|| UsbFilterParseError::new(item))?;
                        let rule = if by_address {
                            UsbDeviceMatch::BusAddress {
                                bus_number: parse_hex(first, item)?,
                                address: parse_hex(second, item)?,
                            }
                        } else {
                            UsbDeviceMatch::VidPid {
                                vendor_id: parse_hex(first, item)?,
                                product_id: Some(parse_hex(second, item)?),
                            }
                        };
                        config.devices.push(rule);
                    }
                }
                "filter" if !value.is_empty() => {
                    config.local_classes = if value.eq_ignore_ascii_case("none") {
                        Vec::new()
                    } else {
                        value
                            .split('#')
                            .map(|class| parse_hex(class, item))
                            .collect::<Result<_, _>>()?
                    };
                }
                _ => return Err(UsbFilterParseError::new(item)),
            }
        }

        Ok(config)
    }

    /// Parse the `.rdp` file `usbdevicestoredirect` property value.
    ///
    /// `*` redirects every device outside the classes kept local, including devices plugged in during
    /// the session.
    pub fn parse_rdp(value: &str) -> Result<Self, UsbFilterParseError> {
        let mut config = Self::default();

        for item in value.split(';').map(str::trim).filter(|item| !item.is_empty()) {
            let (exclude, entry) = match item.strip_prefix('-') {
                Some(entry) => (true, entry),
                None => (false, item),
            };

            let rule = if entry == "*" {
                UsbDeviceMatch::Any
            } else if entry.starts_with('{') && entry.ends_with('}') {
                tracing::debug!(entry, "Skipping USB device setup class GUID, which cannot be matched");
                continue;
            } else {
                parse_instance_id(entry).ok_or_else(|| UsbFilterParseError::new(item))?
            };

            if exclude {
                config.exclude.push(rule);
            } else {
                if rule == UsbDeviceMatch::Any {
                    config.auto = true;
                }
                config.devices.push(rule);
            }
        }

        Ok(config)
    }
}

/// Parse `USB\VID_xxxx&PID_yyyy[&...][\instance]` into a VID/PID rule.
fn parse_instance_id(entry: &str) -> Option<UsbDeviceMatch> {
    let hardware_id = entry.split('\\').nth(1)?;
    if !entry.get(..4)?.eq_ignore_ascii_case("USB\\") {
        return None;
    }

    let mut vendor_id = None;
    let mut product_id = None;
    for part in hardware_id.split('&') {
        let (key, value) = part.split_once('_')?;
        if key.eq_ignore_ascii_case("VID") {
            vendor_id = Some(u16::from_str_radix(value, 16).ok()?);
        } else if key.eq_ignore_ascii_case("PID") {
            product_id = Some(u16::from_str_radix(value, 16).ok()?);
        }
    }

    Some(UsbDeviceMatch::VidPid {
        vendor_id: vendor_id?,
        product_id,
    })
}

fn parse_hex<T: TryFrom<u32>>(value: &str, item: &str) -> Result<T, UsbFilterParseError> {
    let value = value.trim();
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    u32::from_str_radix(value, 16)
        .ok()
        .and_then(|value| T::try_from(value).ok())
        .ok_or_else(|| UsbFilterParseError::new(item))
}

/// Error returned for a USB redirection rule that cannot be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbFilterParseError {
    item: String,
}

impl UsbFilterParseError {
    fn new(item: &str) -> Self {
        Self { item: item.to_owned() }
    }

    /// The rule that could not be parsed.
    pub fn item(&self) -> &str {
        &self.item
    }
}

impl fmt::Display for UsbFilterParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid USB redirection rule `{}`", self.item)
    }
}

impl core::error::Error for UsbFilterParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(vendor_id: u16, product_id: u16, device_class: u8, interface_classes: &[u8]) -> UsbDeviceIdentity {
        UsbDeviceIdentity {
            vendor_id,
            product_id,
            bus_number: 1,
            address: 4,
            device_class,
            interface_classes: interface_classes.to_vec(),
        }
    }

    #[test]
    fn parses_freerdp_id_and_addr_lists() {
        let config = UsbRedirectionConfig::parse_cli("id:046d:C52B#1050:0407,addr:1:4,auto").expect("valid spec");

        assert_eq!(
            config.devices,
            [
                UsbDeviceMatch::VidPid {
                    vendor_id: 0x046d,
                    product_id: Some(0xc52b)
                },
                UsbDeviceMatch::VidPid {
                    vendor_id: 0x1050,
                    product_id: Some(0x0407)
                },
                UsbDeviceMatch::BusAddress {
                    bus_number: 1,
                    address: 4
                },
            ]
        );
        assert!(config.auto);
        assert_eq!(config.local_classes, DEFAULT_LOCAL_CLASSES);
    }

    #[test]
    fn parses_freerdp_class_filter() {
        let config = UsbRedirectionConfig::parse_cli("auto,filter:03#e0").expect("valid spec");
        assert_eq!(config.local_classes, [0x03, 0xe0]);

        let config = UsbRedirectionConfig::parse_cli("auto,filter:none").expect("valid spec");
        assert!(config.local_classes.is_empty());
    }

    #[test]
    fn rejects_malformed_cli_rules() {
        for spec in ["id:046d", "id:xyz:1", "addr:100:1", "id:1:10000", "bogus", "auto:1", "filter:"] {
            let error = UsbRedirectionConfig::parse_cli(spec).expect_err(spec);
            assert!(spec.contains(error.item()), "{spec}");
        }
    }

    #[test]
    fn parses_rdp_property() {
        let config = UsbRedirectionConfig::parse_rdp(
            "USB\\VID_045E&PID_0040\\6&2C7FB8E&0&1; {36fc9e60-c465-11cf-8056-444553540000};-USB\\VID_1050&PID_0407",
        )
        .expect("valid property");

        assert_eq!(
            config.devices,
            [UsbDeviceMatch::VidPid {
                vendor_id: 0x045e,
                product_id: Some(0x0040)
            }]
        );
        assert_eq!(
            config.exclude,
            [UsbDeviceMatch::VidPid {
                vendor_id: 0x1050,
                product_id: Some(0x0407)
            }]
        );
        assert!(!config.auto);
    }

    #[test]
    fn rdp_wildcard_redirects_present_and_hotplugged_devices() {
        let config = UsbRedirectionConfig::parse_rdp("*").expect("valid property");

        assert!(config.auto);
        assert!(config.redirect_at_connect(&device(0x2e8a, 0x000a, 0xef, &[0xff])));
        assert!(config.redirect_on_hotplug(&device(0x2e8a, 0x000a, 0xef, &[0xff])));
        // HID stays local under the wildcard.
        assert!(!config.redirect_at_connect(&device(0x046d, 0xc52b, 0x00, &[0x03])));
    }

    #[test]
    fn rejects_malformed_rdp_entries() {
        for value in ["USB\\VID_XYZ&PID_0001", "PCI\\VEN_8086&DEV_1234", "USB"] {
            assert!(UsbRedirectionConfig::parse_rdp(value).is_err(), "{value}");
        }
    }

    #[test]
    fn explicit_ids_override_local_classes_but_not_exclusions() {
        let mut config = UsbRedirectionConfig::parse_cli("id:046d:c52b").expect("valid spec");
        let mouse = device(0x046d, 0xc52b, 0x00, &[0x03]);

        assert!(config.redirect_at_connect(&mouse));
        assert!(config.redirect_on_hotplug(&mouse));

        config.exclude.push(UsbDeviceMatch::VidPid {
            vendor_id: 0x046d,
            product_id: None,
        });
        assert!(!config.redirect_at_connect(&mouse));
    }

    #[test]
    fn auto_only_applies_to_hotplug() {
        let config = UsbRedirectionConfig::parse_cli("auto").expect("valid spec");
        let board = device(0x2e8a, 0x000a, 0xef, &[0xff]);

        assert!(!config.redirect_at_connect(&board));
        assert!(config.redirect_on_hotplug(&board));
        assert!(!config.redirect_on_hotplug(&device(0x0781, 0x5581, 0x00, &[0x08])));
    }

    #[test]
    fn address_rules_do_not_follow_hotplug() {
        let config = UsbRedirectionConfig::parse_cli("addr:1:4").expect("valid spec");
        let board = device(0x2e8a, 0x000a, 0xef, &[0xff]);

        assert!(config.redirect_at_connect(&board));
        assert!(!config.redirect_on_hotplug(&board));
        assert!(!config.needs_hotplug());
    }

    #[test]
    fn hubs_are_never_redirected() {
        let config = UsbRedirectionConfig::parse_cli("id:05e3:0610,filter:none,auto").expect("valid spec");
        let hub = device(0x05e3, 0x0610, CLASS_HUB, &[CLASS_HUB]);

        assert!(!config.redirect_at_connect(&hub));
        assert!(!config.redirect_on_hotplug(&hub));
    }
}
