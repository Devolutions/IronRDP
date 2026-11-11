//! Windows drive-letter redirection shared by the daemon and the viewer.

use std::io;

use anyhow::Context as _;
use ironrdp_cfg::DriveRedirection;
use ironrdp_client::rdp::{RdpInputEvent, RdpInputSender};
use ironrdp_rdpdr_native::{
    LogicalDriveChange, LogicalDriveSelection, LogicalDriveWatcher, WindowsRdpdrBackendFactory,
};
use tokio::sync::mpsc::error::TrySendError;
use tracing::warn;

/// Adds the drive letters selected by `drives` to a connection's RDPDR factory.
///
/// Returns the shared selection to pass to [`watch_logical_drives`] when dynamic drives are
/// requested. Without drives, `factory` is returned unchanged.
pub fn logical_drive_factory(
    factory: Option<WindowsRdpdrBackendFactory>,
    drives: DriveRedirection,
) -> anyhow::Result<(Option<WindowsRdpdrBackendFactory>, Option<LogicalDriveSelection>)> {
    if drives.is_empty() {
        return Ok((factory, None));
    }

    let factory = match factory {
        Some(factory) => factory,
        None => WindowsRdpdrBackendFactory::from_drives(Vec::new()).context("empty drive configuration")?,
    };
    let selection = LogicalDriveSelection::from_letters(drives.redirects_all(), drives.letters());
    let factory = factory
        .with_logical_drives(selection.clone())
        .context("drive letters conflict with configured drives")?
        .with_dynamic_drives(drives.redirects_dynamic());
    Ok((Some(factory), drives.redirects_dynamic().then_some(selection)))
}

/// Forwards drive arrivals and removals to a session as dynamic RDPDR drive changes.
///
/// A full input queue leaves the change to the watcher's next poll; a closed queue drops it,
/// because no session will consume it.
pub fn watch_logical_drives(
    selection: LogicalDriveSelection,
    input_tx: RdpInputSender,
) -> io::Result<LogicalDriveWatcher> {
    LogicalDriveWatcher::spawn(selection, move |change| {
        let event = match change {
            LogicalDriveChange::Arrived(drive) => RdpInputEvent::AddRdpdrDrive {
                device_id: drive.device_id(),
                name: drive.display_name(),
            },
            LogicalDriveChange::Removed(drive) => RdpInputEvent::RemoveRdpdrDrive {
                device_id: drive.device_id(),
            },
        };
        match input_tx.try_send(event) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => false,
            Err(TrySendError::Closed(_)) => {
                warn!(
                    ?change,
                    "Dropping the drive change because the session input queue is closed"
                );
                true
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_letters_extend_the_connect_factory() {
        let system_drive = std::env::var("SystemDrive").expect("SystemDrive is set on Windows");
        let drives = DriveRedirection::parse(&system_drive);

        let (factory, selection) = logical_drive_factory(None, drives).expect("letter-only factory");
        assert_eq!(factory.expect("factory present").initial_drives().len(), 1);
        assert!(selection.is_none());

        let (factory, selection) =
            logical_drive_factory(None, drives.with_dynamic(true)).expect("dynamic drive factory");
        assert!(factory.is_some());
        assert!(selection.is_some());

        let (factory, selection) = logical_drive_factory(None, DriveRedirection::default()).expect("no drives");
        assert!(factory.is_none());
        assert!(selection.is_none());
    }

    #[test]
    fn hotplug_changes_reach_the_session_queue() {
        let (input_tx, mut input_rx) = RdpInputSender::channel(1);
        let watcher = watch_logical_drives(LogicalDriveSelection::default(), input_tx).expect("start watcher");
        drop(watcher);
        assert!(
            input_rx.try_recv().is_err(),
            "no change is reported without a volume transition"
        );
    }
}
