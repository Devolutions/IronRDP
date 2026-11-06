use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use anyhow::{Context as _, anyhow};
use ironrdp_rdpecam::pdu::{EnumerationPdu, ProtocolVersion};
use tokio::sync::watch;

use super::{DYNAMIC_CHANNELS, STATIC_CHANNELS};

pub(super) struct Enumeration {
    pub(super) session_id: u32,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    requested: Option<ProtocolVersion>,
    selected: Option<ProtocolVersion>,
    devices: BTreeMap<String, watch::Sender<()>>,
    closed: bool,
}

pub(super) struct Lease(pub(super) Arc<Enumeration>);

impl Drop for Lease {
    fn drop(&mut self) {
        self.0.close();
    }
}

impl Enumeration {
    pub(super) fn new(session_id: u32) -> Self {
        Self {
            session_id,
            state: Mutex::default(),
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn close(&self) {
        let mut state = self.state();
        state.closed = true;
        state.devices.clear();
    }

    pub(super) fn device(&self, name: &str) -> anyhow::Result<watch::Receiver<()>> {
        let state = self.state();
        anyhow::ensure!(!state.closed, "camera enumeration is closed");
        state
            .devices
            .get(name)
            .map(watch::Sender::subscribe)
            .context("native camera channel was not announced")
    }

    pub(super) fn observe_client(&self, payload: &[u8]) -> anyhow::Result<()> {
        let message = ironrdp_core::decode::<EnumerationPdu>(payload).context("decode camera enumeration")?;
        let mut state = self.state();
        anyhow::ensure!(!state.closed, "camera enumeration is closed");
        match message {
            EnumerationPdu::SelectVersionRequest(version) => {
                anyhow::ensure!(state.requested.is_none(), "camera version was already requested");
                state.requested = Some(version);
            }
            EnumerationPdu::DeviceAdded { version, device } => {
                anyhow::ensure!(state.selected == Some(version), "camera version is not negotiated");
                anyhow::ensure!(
                    !DYNAMIC_CHANNELS
                        .iter()
                        .any(|endpoint| endpoint.name == device.channel_name)
                        && device.channel_name != "railenc"
                        && !STATIC_CHANNELS
                            .iter()
                            .filter_map(|name| name.as_str())
                            .any(|name| name.eq_ignore_ascii_case(&device.channel_name)),
                    "camera channel name is reserved"
                );
                anyhow::ensure!(
                    !state.devices.contains_key(&device.channel_name),
                    "camera channel was already announced"
                );
                anyhow::ensure!(
                    state.devices.len() < usize::from(u8::MAX),
                    "camera device count exceeds the client limit"
                );
                let (closed, _) = watch::channel(());
                state.devices.insert(device.channel_name, closed);
            }
            EnumerationPdu::DeviceRemoved { version, channel_name } => {
                anyhow::ensure!(state.selected == Some(version), "camera version is not negotiated");
                state
                    .devices
                    .remove(&channel_name)
                    .context("camera channel was not announced")?;
            }
            EnumerationPdu::SelectVersionResponse(_) => {
                return Err(anyhow!("camera version response came from the client"));
            }
        }
        Ok(())
    }

    pub(super) fn observe_native(&self, payload: &[u8]) -> anyhow::Result<()> {
        let message = ironrdp_core::decode::<EnumerationPdu>(payload).context("decode native camera enumeration")?;
        let EnumerationPdu::SelectVersionResponse(version) = message else {
            return Err(anyhow!("native camera message is not a version response"));
        };
        let mut state = self.state();
        anyhow::ensure!(!state.closed, "camera enumeration is closed");
        anyhow::ensure!(
            state.selected.is_none() && state.requested.is_some_and(|requested| version <= requested),
            "native camera version was not requested"
        );
        state.selected = Some(version);
        Ok(())
    }
}
