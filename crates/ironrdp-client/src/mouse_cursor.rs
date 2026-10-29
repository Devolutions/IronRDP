//! Mouse cursor channel ([MS-RDPEMSC]).
//!
//! Servers that open the channel send pointer updates on it instead of the fast-path and
//! slow-path pointer updates, so its updates go through the same pointer handling.
//!
//! [MS-RDPEMSC]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpemsc/

use ironrdp_dvc::DrdynvcClient;
use ironrdp_rdpemsc::MouseCursorClient;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageOutput, SessionResult};

/// Registers the mouse cursor channel.
pub(crate) fn attach(drdynvc: DrdynvcClient) -> DrdynvcClient {
    drdynvc.with_dynamic_channel(MouseCursorClient::new())
}

/// Applies the pointer updates the channel received during this iteration.
///
/// UNTESTED: this runs after every session loop iteration, including iterations that process
/// data from a Soft-Sync tunnel, because pointer updates wait in the channel until it runs.
/// When adding a loop path that processes server data, check that it reaches this call.
pub(crate) fn present(
    active_stage: &mut ActiveStage,
    image: &mut DecodedImage,
    outputs: &mut Vec<ActiveStageOutput>,
) -> SessionResult<()> {
    let Some(mut channel) = active_stage.get_dvc_mut::<MouseCursorClient>() else {
        return Ok(());
    };
    let updates = channel.processor_mut().take_updates();
    for update in &updates {
        outputs.extend(active_stage.process_pointer_update(image, update.as_pointer_update_data())?);
    }
    Ok(())
}
