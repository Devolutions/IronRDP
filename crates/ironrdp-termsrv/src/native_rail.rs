use core::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, anyhow};
use ironrdp_core::{Decode as _, ReadCursor, encode_vec};
use ironrdp_pdu::rdp::capability_sets::{CapabilitySet, Rail, RailSupportLevel, WindowList, WindowSupportLevel};
use ironrdp_pdu::rdp::client_info::ClientInfoFlags;
use ironrdp_pdu::window::WindowingOrder;
use ironrdp_server::ironrdp_acceptor::RemoteAppCapabilities;
use ironrdp_server::{
    ConnectionHandler, ServerError, ServerErrorExt as _, ServerEvent, ServerResult, WindowingOrders,
    WindowingOrdersStatus,
};
use ironrdp_wtsprotocol_ipc::RemoteAppSettings;
use tokio::sync::{Semaphore, mpsc};
use tokio::time::timeout;
use tracing::{debug, warn};
use windows::Win32::Foundation::ERROR_BROKEN_PIPE;

use super::{IO_TIMEOUT, NativeChannels, NativePipe, OPEN_TIMEOUT};
use crate::windows_main::generate_capture_helper_token;

const MAX_ORDER_SIZE: usize = 65_535;

pub(crate) fn remote_app_enabled() -> anyhow::Result<bool> {
    match std::env::var("IRONRDP_WTS_REMOTEAPP") {
        Err(std::env::VarError::NotPresent) => Ok(false),
        Ok(value) if value == "0" => Ok(false),
        Ok(value) if value == "1" => Ok(true),
        _ => anyhow::bail!("IRONRDP_WTS_REMOTEAPP must be 0 or 1"),
    }
}

pub(super) struct RemoteAppState {
    enabled: AtomicBool,
    settings: Mutex<Option<RemoteAppSettings>>,
    encoder: Arc<Semaphore>,
}

impl Default for RemoteAppState {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            settings: Mutex::new(None),
            encoder: Arc::new(Semaphore::new(1)),
        }
    }
}

impl NativeChannels {
    pub(crate) fn configure_remote_app(&self, enabled: bool) {
        self.remote_app.enabled.store(enabled, Ordering::Relaxed);
    }

    pub(crate) fn remote_app_capabilities(&self) -> Option<RemoteAppCapabilities> {
        self.remote_app
            .enabled
            .load(Ordering::Relaxed)
            .then(|| RemoteAppCapabilities {
                rail: Rail {
                    support_level: RailSupportLevel::all(),
                },
                window_list: WindowList {
                    support_level: WindowSupportLevel::SupportedEx,
                    num_icon_caches: 3,
                    num_icon_cache_entries: 12,
                },
            })
    }

    pub(crate) fn remote_app_settings(&self) -> anyhow::Result<Option<RemoteAppSettings>> {
        Ok(self
            .remote_app
            .settings
            .lock()
            .map_err(|_| anyhow!("native RemoteApp settings lock poisoned"))?
            .clone())
    }

    pub(crate) fn connection_handler(self: &Arc<Self>) -> Box<dyn ConnectionHandler> {
        Box::new(RemoteAppObserver(Arc::clone(self)))
    }

    pub(super) async fn open_window_orders(&self, session_id: u32) -> anyhow::Result<String> {
        let (mut binding, sender) = self.bind(session_id)?;
        anyhow::ensure!(
            self.remote_app_settings()?
                .is_some_and(|settings| settings.enabled && settings.window_capability_set.is_some()),
            "RemoteApp window orders were not negotiated"
        );
        let instance = Arc::clone(&self.remote_app.encoder)
            .try_acquire_owned()
            .context("native RemoteApp encoder is already open")?;
        let pipe_name = format!(r"\\.\pipe\IronRdpNativeDvc-{}", generate_capture_helper_token()?);
        let pipe = (self.create_pipe)(&pipe_name)?;
        drop(tokio::spawn(async move {
            let _instance = instance;
            let result = tokio::select! {
                biased;
                _ = binding.changed() => Err(anyhow!("native RemoteApp session binding changed")),
                result = receive_orders(&pipe, &sender) => result,
            };
            match result {
                Ok(()) => debug!(session_id, "Native RemoteApp encoder closed"),
                Err(error) => warn!(session_id, %error, "Native RemoteApp encoder stopped"),
            }
        }));
        Ok(pipe_name)
    }
}

struct RemoteAppObserver(Arc<NativeChannels>);

impl ConnectionHandler for RemoteAppObserver {
    fn on_client_info(&mut self, flags: ClientInfoFlags) -> ServerResult<()> {
        let enabled = flags.contains(ClientInfoFlags::RAIL);
        if enabled && !self.0.remote_app.enabled.load(Ordering::Relaxed) {
            return Err(ServerError::reason(
                "native RemoteApp settings",
                "RemoteApp is disabled",
            ));
        }
        *self
            .0
            .remote_app
            .settings
            .lock()
            .map_err(|_| ServerError::reason("native RemoteApp settings", "lock poisoned"))? =
            Some(RemoteAppSettings {
                enabled,
                client_rail_support_level: None,
                window_capability_set: None,
                client_info_flags: flags.bits(),
                ..RemoteAppSettings::default()
            });
        Ok(())
    }

    fn on_client_session_config(
        &mut self,
        flags: ClientInfoFlags,
        performance_flags: ironrdp_pdu::rdp::client_info::PerformanceFlags,
        keyboard_layout: u32,
        keyboard_type: ironrdp_pdu::gcc::KeyboardType,
        keyboard_subtype: u32,
        keyboard_functional_keys_count: u32,
        ime_file_name: &str,
        desktop_size: ironrdp_server::DesktopSize,
    ) -> ServerResult<()> {
        let mut settings = self
            .0
            .remote_app
            .settings
            .lock()
            .map_err(|_| ServerError::reason("native RemoteApp settings", "lock poisoned"))?;
        let settings = settings
            .as_mut()
            .ok_or_else(|| ServerError::reason("native RemoteApp settings", "Client Info is unavailable"))?;
        settings.client_info_flags = flags.bits();
        settings.performance_flags = performance_flags.bits();
        settings.keyboard_layout = keyboard_layout;
        settings.keyboard_type = u32::from(keyboard_type);
        settings.keyboard_subtype = keyboard_subtype;
        settings.keyboard_function_key = keyboard_functional_keys_count;
        settings.ime_file_name = ime_file_name.to_owned();
        settings.desktop_width = desktop_size.width;
        settings.desktop_height = desktop_size.height;
        Ok(())
    }

    fn on_remote_app_capabilities(&mut self, capabilities: Option<&RemoteAppCapabilities>) -> ServerResult<()> {
        let mut settings = self
            .0
            .remote_app
            .settings
            .lock()
            .map_err(|_| ServerError::reason("native RemoteApp settings", "lock poisoned"))?;
        let settings = settings
            .as_mut()
            .ok_or_else(|| ServerError::reason("native RemoteApp settings", "Client Info is unavailable"))?;
        settings.client_rail_support_level = capabilities.map(|caps| caps.rail.support_level.bits());
        settings.window_capability_set = capabilities
            .map(|caps| encode_vec(&CapabilitySet::WindowList(caps.window_list.clone())))
            .transpose()
            .map_err(ServerError::encode)?;
        Ok(())
    }
}

async fn receive_orders(pipe: &NativePipe, sender: &mpsc::UnboundedSender<ServerEvent>) -> anyhow::Result<()> {
    timeout(OPEN_TIMEOUT, pipe.connect())
        .await
        .context("native RemoteApp pipe connection timed out")??;
    let mut decoder = OrderDecoder::default();
    loop {
        let data = match pipe
            .read(MAX_ORDER_SIZE)
            .await
            .context("native RemoteApp read worker failed")?
        {
            Ok(data) => data,
            Err(error)
                if error
                    .downcast_ref::<windows::core::Error>()
                    .is_some_and(|error| error.code() == ERROR_BROKEN_PIPE.to_hresult()) =>
            {
                return decoder.finish();
            }
            Err(error) => return Err(error),
        };
        let mut remaining = data.as_slice();
        while let Some(order) = decoder.next(&mut remaining)? {
            timeout(IO_TIMEOUT, send_order(sender, order))
                .await
                .context("native RemoteApp window-order write timed out")??;
        }
    }
}

#[derive(Default)]
struct OrderDecoder {
    pending: Vec<u8>,
}

impl OrderDecoder {
    fn next(&mut self, data: &mut &[u8]) -> anyhow::Result<Option<WindowingOrder>> {
        if self.pending.len() < 3 {
            let take = (3 - self.pending.len()).min(data.len());
            self.pending.extend_from_slice(&data[..take]);
            *data = &data[take..];
        }
        if self.pending.len() < 3 {
            return Ok(None);
        }
        anyhow::ensure!(self.pending[0] == 0x2e, "native RemoteApp data is not a window order");
        let length = usize::from(u16::from_le_bytes([self.pending[1], self.pending[2]]));
        anyhow::ensure!(7 <= length, "native RemoteApp window order is too short");
        let take = (length - self.pending.len()).min(data.len());
        self.pending.extend_from_slice(&data[..take]);
        *data = &data[take..];
        if self.pending.len() < length {
            return Ok(None);
        }
        let mut cursor = ReadCursor::new(&self.pending);
        let order = WindowingOrder::decode(&mut cursor).context("decode native RemoteApp window order")?;
        anyhow::ensure!(cursor.eof(), "native RemoteApp window order contains trailing data");
        self.pending.clear();
        Ok(Some(order))
    }

    fn finish(self) -> anyhow::Result<()> {
        anyhow::ensure!(self.pending.is_empty(), "native RemoteApp pipe closed within an order");
        Ok(())
    }
}

async fn send_order(sender: &mpsc::UnboundedSender<ServerEvent>, order: WindowingOrder) -> anyhow::Result<()> {
    loop {
        let (request, completion) = WindowingOrders::new(vec![order.clone()]);
        sender
            .send(ServerEvent::WindowingOrders(request))
            .map_err(|_| anyhow!("native RemoteApp transport closed"))?;
        if completion.await.context("native RemoteApp write completion lost")?? == WindowingOrdersStatus::Written {
            return Ok(());
        }
        // Reactivation can discard an event before its graphics write begins.
        // The graphics lock prevents an in-progress write from being interrupted by a resize.
        tracing::trace!("Retrying a window order discarded before transmission");
        tokio::time::sleep(core::time::Duration::from_millis(10)).await;
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_pdu::window::{WindowOrder, WindowOrderBody};

    use super::*;

    const DELETED: [u8; 11] = [0x2e, 11, 0, 0, 0, 0, 0x21, 7, 0, 0, 0];

    // oracle: spec MS-RDPERP §2.2.1.3.1.2.4: eleven bytes encode deletion of window 7.
    #[test]
    fn window_order_survives_every_header_and_body_split() {
        for split in 1..DELETED.len() {
            let mut decoder = OrderDecoder::default();
            assert!(decoder.next(&mut &DELETED[..split]).unwrap().is_none());
            let order = decoder.next(&mut &DELETED[split..]).unwrap().unwrap();
            assert_eq!(
                order,
                WindowingOrder::Window(WindowOrder {
                    window_id: 7,
                    is_new: false,
                    body: WindowOrderBody::Deleted,
                })
            );
            decoder.finish().unwrap();
        }
    }

    // oracle: spec MS-RDPERP §2.2.1.3: orderSize delimits consecutive orders.
    #[test]
    fn consecutive_window_orders_are_not_merged() {
        let bytes = DELETED.repeat(2);
        let mut input = bytes.as_slice();
        let mut decoder = OrderDecoder::default();
        assert!(decoder.next(&mut input).unwrap().is_some());
        assert_eq!(input, DELETED);
        assert!(decoder.next(&mut input).unwrap().is_some());
        assert!(input.is_empty());
        assert!(decoder.next(&mut input).unwrap().is_none());
        decoder.finish().unwrap();
    }

    // oracle: spec MS-RDPERP §2.2.1.3: controlFlags identifies a window order and orderSize includes its header.
    #[test]
    fn invalid_complete_headers_fail_without_waiting_for_a_body() {
        for header in [[0, 11, 0], [0x2e, 0, 0], [0x2e, 6, 0]] {
            assert!(
                OrderDecoder::default().next(&mut header.as_slice()).is_err(),
                "{header:?}"
            );
        }
    }

    // oracle: spec MS-RDPERP §2.2.1.3: an order must contain all orderSize bytes.
    #[test]
    fn closing_with_a_partial_header_or_body_is_an_error() {
        for length in 1..DELETED.len() {
            let mut decoder = OrderDecoder::default();
            assert!(decoder.next(&mut &DELETED[..length]).unwrap().is_none());
            assert!(
                decoder.finish().is_err(),
                "accepted {length} bytes of an eleven-byte order"
            );
        }
    }

    // oracle: spec MS-RDPERP §2.2.1.3.1.2.4: deleted orders contain only WindowId after the common header.
    #[test]
    fn trailing_bytes_inside_an_order_are_rejected() {
        let mut bytes = DELETED.to_vec();
        bytes[1] = 12;
        bytes.push(0);
        assert!(OrderDecoder::default().next(&mut bytes.as_slice()).is_err());
    }

    // oracle: spec MS-RDPERP §2.2.1.3.1.2.1: an empty title and 8190 rectangles fill a 65535-byte order.
    #[test]
    fn maximum_size_order_can_arrive_in_separate_pipe_messages() {
        let mut bytes = vec![
            0x2e, 0xff, 0xff, // Control and maximum u16 order size.
            4, 1, 0, 1, // Window, title and window rectangles.
            7, 0, 0, 0, // WindowId.
            0, 0, // Empty title.
            0xfe, 0x1f, // 8190 rectangles.
        ];
        bytes.resize(65_535, 0);
        let mut decoder = OrderDecoder::default();
        assert!(decoder.next(&mut &bytes[..65_534]).unwrap().is_none());
        let Some(WindowingOrder::Window(window)) = decoder.next(&mut &bytes[65_534..]).unwrap() else {
            panic!("expected the complete window update");
        };
        assert_eq!(window.window_id, 7);
        let WindowOrderBody::State(state) = window.body else {
            panic!("expected window state");
        };
        assert_eq!(state.title.as_deref(), Some(""));
        assert_eq!(state.window_rects.unwrap().len(), 8190);
        decoder.finish().unwrap();
    }
}
