use core::cell::Cell;

use ironrdp::connector::NetworkCharacteristics;
use ironrdp::pdu::rdp::autodetect::AutoDetectRequest;
use wasm_bindgen::JsValue;

/// What the session has done so far, read through the `stats` extension.
#[derive(Default)]
pub(crate) struct SessionStats {
    bytes_received: Cell<u64>,
    bytes_sent: Cell<u64>,
    frames_drawn: Cell<u64>,
    network: Cell<Option<NetworkCharacteristics>>,
}

impl SessionStats {
    pub(crate) fn add_received(&self, bytes: usize) {
        add(&self.bytes_received, bytes);
    }

    pub(crate) fn add_sent(&self, bytes: usize) {
        add(&self.bytes_sent, bytes);
    }

    pub(crate) fn add_frame_drawn(&self) {
        add(&self.frames_drawn, 1);
    }

    /// Keeps the network characteristics the connector reported with the connection result.
    pub(crate) fn set_network(&self, network: NetworkCharacteristics) {
        self.network.set(Some(network));
    }

    /// Takes a Network Characteristics Result ([MS-RDPBCGR] 2.2.14.1.5) the server sent during the session, on top
    /// of the one kept: [`NetworkCharacteristics::from_result`] says what a result without `baseRTT` or `bandwidth`
    /// keeps. Any other request is ignored.
    ///
    /// [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/228ffc5c-b60c-4d3e-9781-ac613f822fdf
    pub(crate) fn take_result(&self, request: &AutoDetectRequest) {
        if let Some(network) = NetworkCharacteristics::from_result(self.network.get(), request) {
            self.network.set(Some(network));
        }
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            bytes_received: self.bytes_received.get(),
            bytes_sent: self.bytes_sent.get(),
            frames_drawn: self.frames_drawn.get(),
            network: self.network.get(),
        }
    }

    /// Converts the figures to the JavaScript object documented in the [session statistics API].
    ///
    /// [session statistics API]: https://github.com/Devolutions/IronRDP/tree/master/web-client/iron-remote-desktop-rdp#session-statistics
    pub(crate) fn to_js(&self) -> JsValue {
        let snapshot = self.snapshot();
        let object = js_sys::Object::new();
        let set = |object: &js_sys::Object, key: &str, value: JsValue| {
            let _ = js_sys::Reflect::set(object, &JsValue::from_str(key), &value);
        };
        set(&object, "bytesReceived", number(snapshot.bytes_received));
        set(&object, "bytesSent", number(snapshot.bytes_sent));
        set(&object, "framesDrawn", number(snapshot.frames_drawn));
        let network = snapshot.network.map_or(JsValue::NULL, |network| {
            let object = js_sys::Object::new();
            if let Some(base_rtt_ms) = network.base_rtt_ms {
                set(&object, "baseRttMs", JsValue::from(base_rtt_ms));
            }
            set(&object, "averageRttMs", JsValue::from(network.average_rtt_ms));
            if let Some(bandwidth_kbps) = network.bandwidth_kbps {
                set(&object, "bandwidthKbps", JsValue::from(bandwidth_kbps));
            }
            object.into()
        });
        set(&object, "network", network);
        object.into()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(crate) bytes_received: u64,
    pub(crate) bytes_sent: u64,
    pub(crate) frames_drawn: u64,
    pub(crate) network: Option<NetworkCharacteristics>,
}

fn add(counter: &Cell<u64>, amount: usize) {
    counter.set(counter.get().saturating_add(u64::try_from(amount).unwrap_or(u64::MAX)));
}

#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "a JavaScript number; the counters stay exact below 2^53"
)]
fn number(value: u64) -> JsValue {
    JsValue::from_f64(value as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network(base_rtt_ms: Option<u32>, bandwidth_kbps: Option<u32>, average_rtt_ms: u32) -> NetworkCharacteristics {
        NetworkCharacteristics {
            base_rtt_ms,
            bandwidth_kbps,
            average_rtt_ms,
        }
    }

    fn result(base_rtt_ms: Option<u32>, bandwidth_kbps: Option<u32>, average_rtt_ms: u32) -> AutoDetectRequest {
        AutoDetectRequest::NetworkCharacteristicsResult {
            sequence_number: 1,
            request_type: 0,
            base_rtt_ms,
            bandwidth_kbps,
            average_rtt_ms,
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn counters_add_up() {
        // oracle: mutant counters that fail to accumulate, or that swap received and sent
        let stats = SessionStats::default();
        stats.add_received(10);
        stats.add_received(5);
        stats.add_sent(7);
        stats.add_frame_drawn();
        stats.add_frame_drawn();

        assert_eq!(
            stats.snapshot(),
            Snapshot {
                bytes_received: 15,
                bytes_sent: 7,
                frames_drawn: 2,
                network: None,
            }
        );
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_counter_that_hits_its_cap_never_wraps() {
        // oracle: property monotonic (a counter never wraps)
        let stats = SessionStats::default();
        stats.bytes_received.set(u64::MAX - 1);
        stats.add_received(5);

        assert!(u64::MAX - 1 <= stats.snapshot().bytes_received);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_result_without_base_rtt_or_bandwidth_keeps_the_values_reported_before() {
        // oracle: spec MS-RDPBCGR §2.2.14.1.5 (baseRTT and bandwidth are optional in each result)
        // oracle: mutant a result that drops the base RTT or the bandwidth reported before
        let stats = SessionStats::default();
        stats.take_result(&result(Some(10), Some(5_000), 12));
        stats.take_result(&result(None, None, 20));

        assert_eq!(stats.snapshot().network, Some(network(Some(10), Some(5_000), 20)));
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_result_with_base_rtt_and_bandwidth_replaces_them() {
        // oracle: mutant a result whose values do not replace the ones kept
        let stats = SessionStats::default();
        stats.take_result(&result(Some(10), Some(5_000), 12));
        stats.take_result(&result(Some(8), Some(6_000), 11));

        assert_eq!(stats.snapshot().network, Some(network(Some(8), Some(6_000), 11)));
    }
}

#[cfg(all(test, target_arch = "wasm32"))]
mod wasm_tests {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    fn field(object: &JsValue, name: &str) -> JsValue {
        js_sys::Reflect::get(object, &JsValue::from_str(name)).expect("read field")
    }

    fn keys(object: &JsValue) -> Vec<String> {
        let mut keys: Vec<_> = js_sys::Object::keys(object.unchecked_ref::<js_sys::Object>())
            .iter()
            .filter_map(|key| key.as_string())
            .collect();
        keys.sort();
        keys
    }

    #[wasm_bindgen_test]
    fn figures_are_numbers_and_network_is_null_before_a_result() {
        // oracle: mutant a renamed or missing key in the returned object
        let stats = SessionStats::default();
        stats.add_received(3);
        stats.add_sent(4);
        stats.add_frame_drawn();

        let object = stats.to_js();
        assert_eq!(keys(&object), ["bytesReceived", "bytesSent", "framesDrawn", "network"]);
        assert_eq!(field(&object, "bytesReceived").as_f64(), Some(3.0));
        assert_eq!(field(&object, "bytesSent").as_f64(), Some(4.0));
        assert_eq!(field(&object, "framesDrawn").as_f64(), Some(1.0));
        assert!(field(&object, "network").is_null());
    }

    #[wasm_bindgen_test]
    fn network_carries_only_the_fields_the_server_reported() {
        // oracle: spec MS-RDPBCGR §2.2.14.1.5 (baseRTT and bandwidth are optional in a result, averageRTT is not)
        // oracle: mutant a key present for a value the server never reported
        let stats = SessionStats::default();
        stats.set_network(NetworkCharacteristics {
            base_rtt_ms: Some(10),
            bandwidth_kbps: None,
            average_rtt_ms: 12,
        });

        let network = field(&stats.to_js(), "network");
        assert_eq!(keys(&network), ["averageRttMs", "baseRttMs"]);
        assert_eq!(field(&network, "baseRttMs").as_f64(), Some(10.0));
        assert_eq!(field(&network, "averageRttMs").as_f64(), Some(12.0));
    }
}
