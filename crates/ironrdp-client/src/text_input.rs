//! Text input ([MS-RDPETXT]) wiring.
//!
//! The server sends on one channel and the client answers on the other, so the replies queued
//! while the server-to-client channel processes a message are sent after the session iteration.
//!
//! [MS-RDPETXT]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpetxt/

use std::sync::{Arc, PoisonError};

use ironrdp_dvc::{DrdynvcClient, DvcMessageBatch};
use ironrdp_rdpetxt::client::{ClientToServerChannel, ServerToClientChannel, TextInputClient};
use ironrdp_session::{ActiveStage, ActiveStageOutput, SessionResult};
use tracing::{debug, warn};

/// Registers both text input channels around one shared client.
pub(crate) fn attach(drdynvc: DrdynvcClient) -> DrdynvcClient {
    let client = TextInputClient::default().into_shared();
    let server_to_client = Arc::clone(&client);
    // The server may close and reopen the channels during a session, so each opening gets a new
    // processor sharing the client.
    drdynvc
        .with_dynamic_channel_factory(ironrdp_rdpetxt::SERVER_TO_CLIENT_CHANNEL_NAME, move || {
            ServerToClientChannel::new(Arc::clone(&server_to_client))
        })
        .with_dynamic_channel_factory(ironrdp_rdpetxt::CLIENT_TO_SERVER_CHANNEL_NAME, move || {
            ClientToServerChannel::new(Arc::clone(&client))
        })
}

/// Sends the queued replies with the iteration.
///
/// They become `dvc_batch` when the iteration has no other dynamic channel batch, and leave in a
/// frame of their own appended to `outputs` otherwise.
/// Returns whether `dvc_batch` now holds them, in which case they must be routed for the
/// client-to-server channel rather than for the request that produced the iteration.
pub(crate) fn flush(
    active_stage: &mut ActiveStage,
    outputs: &mut Vec<ActiveStageOutput>,
    dvc_batch: &mut Option<DvcMessageBatch>,
) -> SessionResult<bool> {
    // UNTESTED: no client test drives an ActiveStage through these paths.
    // The server waits for some replies, such as the version reply and text change
    // acknowledgements, so a change here must not drop or reorder them.
    if dvc_batch.as_ref().is_none_or(|batch| batch.messages().is_empty()) {
        let batch = pending_batch(active_stage);
        let installed = batch.is_some();
        if installed {
            *dvc_batch = batch;
        }
        return Ok(installed);
    }
    let Some(channel_id) = active_stage
        .get_dvc::<ClientToServerChannel>()
        .map(|channel| channel.channel_id())
    else {
        return Ok(false);
    };
    // Only the batch reaches a Soft-Sync tunnel, so tunneled replies wait for an iteration
    // without another batch.
    if active_stage.dvc_tunnel_for_channel(channel_id).is_some() {
        return Ok(false);
    }
    if let Some(batch) = pending_batch(active_stage) {
        let frame = active_stage.encode_dvc_messages(batch.into_messages())?;
        outputs.push(ActiveStageOutput::ResponseFrame(frame));
    }
    Ok(false)
}

/// Inserts `text` into the focused remote edit control.
pub(crate) fn commit_text(active_stage: &mut ActiveStage, text: &str) -> Option<DvcMessageBatch> {
    let Some(channel) = active_stage.get_dvc::<ClientToServerChannel>() else {
        debug!("Dropping committed text because the text input channel is not open");
        return None;
    };
    let committed = channel
        .processor()
        .client()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .commit_text(text);
    if let Err(error) = committed {
        debug!(%error, "Dropping committed text");
        return None;
    }
    pending_batch(active_stage)
}

fn pending_batch(active_stage: &mut ActiveStage) -> Option<DvcMessageBatch> {
    let mut channel = active_stage.get_dvc_mut::<ClientToServerChannel>()?;
    let channel_id = channel.channel_id();
    match channel
        .processor_mut()
        .pending_messages()
        .and_then(|messages| DvcMessageBatch::try_new(channel_id, messages))
    {
        Ok(batch) if !batch.messages().is_empty() => Some(batch),
        Ok(_) => None,
        Err(error) => {
            warn!(%error, "Unable to send text input PDUs");
            None
        }
    }
}
